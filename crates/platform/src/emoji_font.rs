//! What a colour emoji font is made of, read from its table directory, to tell
//! the ones the renderer can draw from the ones it cannot.
//!
//! Slint draws colour glyphs through swash, which handles bitmap fonts (`CBDT`
//! with `CBLC`, or Apple's `sbix`) and COLRv0 -- but not COLRv1, which is what
//! Android's own emoji font is made of since Android 13. A COLRv1 font draws as
//! nothing at all, so choosing an emoji font by its name is not enough: the
//! file has to be looked at. This lives apart from `android/fonts.rs`, which
//! does the choosing, so that the reading can be tested on any host.

use std::io::{self, Read, Seek, SeekFrom};

/// How a font stores its colour glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmojiFormat {
    /// Colour bitmaps (`CBDT`/`CBLC` or `sbix`). Drawable.
    Bitmap,
    /// Layered colour outlines, COLR version 0. Drawable.
    ColorV0,
    /// COLR version 1 (gradients, transforms). Not drawable.
    ColorV1,
    /// Neither: no colour tables, or not a font this reads.
    Other,
}

impl EmojiFormat {
    /// Whether the renderer can draw glyphs stored this way.
    pub fn drawable(self) -> bool {
        matches!(self, Self::Bitmap | Self::ColorV0)
    }

    /// Bitmaps first: the safest to draw, and what Android's legacy emoji font is.
    pub fn preference(self) -> u8 {
        match self {
            Self::Bitmap => 0,
            Self::ColorV0 => 1,
            Self::ColorV1 | Self::Other => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bitmap => "bitmap",
            Self::ColorV0 => "COLRv0",
            Self::ColorV1 => "COLRv1",
            Self::Other => "not a colour font",
        }
    }
}

/// Reads the table directory of the font in `source`, and the version of its
/// `COLR` table if it has one. Reads a few hundred bytes, not the file.
pub fn classify<R: Read + Seek>(mut source: R) -> io::Result<EmojiFormat> {
    let mut head = [0u8; 12];
    source.read_exact(&mut head)?;
    // 0x00010000 (TrueType), `OTTO` (CFF) and `true` (Apple); a `ttcf`
    // collection is not an emoji font anyone ships.
    if !matches!(&head[..4], [0, 1, 0, 0] | b"OTTO" | b"true") {
        return Ok(EmojiFormat::Other);
    }
    let tables = u16::from_be_bytes([head[4], head[5]]) as usize;
    let mut directory = vec![0u8; tables * 16];
    source.read_exact(&mut directory)?;

    let find = |tag: &[u8; 4]| {
        (0..tables)
            .map(|i| &directory[i * 16..(i + 1) * 16])
            .find(|record| &record[..4] == tag)
            .map(|record| u32::from_be_bytes([record[8], record[9], record[10], record[11]]))
    };

    if (find(b"CBDT").is_some() && find(b"CBLC").is_some()) || find(b"sbix").is_some() {
        return Ok(EmojiFormat::Bitmap);
    }
    let Some(colr) = find(b"COLR") else {
        return Ok(EmojiFormat::Other);
    };
    source.seek(SeekFrom::Start(u64::from(colr)))?;
    let mut version = [0u8; 2];
    source.read_exact(&mut version)?;
    Ok(if u16::from_be_bytes(version) == 0 {
        EmojiFormat::ColorV0
    } else {
        EmojiFormat::ColorV1
    })
}

/// The emoji font files a system font list (`fonts.xml`, or
/// `font_fallback.xml` since Android 12) declares: the fonts of every family
/// whose `lang` includes `und-Zsye`, the script tag for emoji. The device names
/// its own emoji font here, whatever the file is called -- unlike a search for
/// "emoji" in file names -- and a family marked `ignore="true"` (Android's
/// bitmap build) is included: it is the one the renderer can draw.
///
/// Reads the text as it comes, not as XML: the lists are machine-written and
/// regular, and the two attributes and the font file names are all it needs.
pub fn declared_emoji_files(xml: &str) -> Vec<String> {
    let mut files = Vec::new();
    for family in xml.split("<family").skip(1) {
        let (tag, body) = family.split_once('>').unwrap_or((family, ""));
        let body = body.split("</family>").next().unwrap_or("");
        if !attribute(tag, "lang")
            .is_some_and(|lang| lang.split(',').any(|l| l.trim() == "und-Zsye"))
        {
            continue;
        }
        for font in body.split("<font").skip(1) {
            let Some((_, rest)) = font.split_once('>') else {
                continue;
            };
            let name = rest.split("</font>").next().unwrap_or("").trim();
            if !name.is_empty() && !files.iter().any(|f| f == name) {
                files.push(name.to_owned());
            }
        }
    }
    files
}

/// The value of attribute `name` in the text of an opening tag.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = &tag[at..];
    Some(&rest[..rest.find('"')?])
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// A font with only a table directory and, at the end, `colr_version` if
    /// `tags` has a `COLR` table.
    fn font(tags: &[&[u8; 4]], colr_version: u16) -> Vec<u8> {
        let mut bytes = vec![0, 1, 0, 0];
        bytes.extend((tags.len() as u16).to_be_bytes());
        bytes.extend([0u8; 6]);
        let tables_end = 12 + 16 * tags.len();
        for tag in tags {
            bytes.extend(*tag);
            bytes.extend([0u8; 4]); // checksum
            bytes.extend((tables_end as u32).to_be_bytes()); // offset
            bytes.extend([0u8; 4]); // length
        }
        bytes.extend(colr_version.to_be_bytes());
        bytes
    }

    fn classified(bytes: Vec<u8>) -> EmojiFormat {
        classify(Cursor::new(bytes)).unwrap()
    }

    #[test]
    fn colour_bitmaps_are_drawable() {
        assert_eq!(
            classified(font(&[b"CBDT", b"CBLC", b"cmap"], 0)),
            EmojiFormat::Bitmap
        );
        assert_eq!(
            classified(font(&[b"sbix", b"cmap"], 0)),
            EmojiFormat::Bitmap
        );
        // Half a bitmap font (data without its locations) is not one.
        assert_eq!(classified(font(&[b"CBDT", b"cmap"], 0)), EmojiFormat::Other);
    }

    #[test]
    fn colr_is_drawable_only_at_version_zero() {
        assert_eq!(
            classified(font(&[b"COLR", b"CPAL"], 0)),
            EmojiFormat::ColorV0
        );
        assert_eq!(
            classified(font(&[b"COLR", b"CPAL"], 1)),
            EmojiFormat::ColorV1
        );
        assert!(EmojiFormat::ColorV0.drawable());
        assert!(!EmojiFormat::ColorV1.drawable());
    }

    #[test]
    fn a_bitmap_font_wins_over_a_colr_one_in_the_same_file() {
        assert_eq!(
            classified(font(&[b"COLR", b"CBDT", b"CBLC"], 1)),
            EmojiFormat::Bitmap
        );
    }

    #[test]
    fn a_plain_font_or_something_else_is_other() {
        assert_eq!(classified(font(&[b"glyf", b"cmap"], 0)), EmojiFormat::Other);
        let mut not_a_font = vec![b'<', b'h', b't', b'm', b'l'];
        not_a_font.resize(64, 0);
        assert_eq!(classified(not_a_font), EmojiFormat::Other);
    }

    #[test]
    fn a_file_too_short_to_hold_its_directory_is_an_error() {
        let mut truncated = font(&[b"CBDT", b"CBLC"], 0);
        truncated.truncate(20);
        assert!(classify(Cursor::new(truncated)).is_err());
    }

    #[test]
    fn each_format_is_named_for_the_about_page() {
        assert_eq!(EmojiFormat::Bitmap.label(), "bitmap");
        assert_eq!(EmojiFormat::ColorV0.label(), "COLRv0");
        assert_eq!(EmojiFormat::ColorV1.label(), "COLRv1");
        assert_eq!(EmojiFormat::Other.label(), "not a colour font");
    }

    #[test]
    fn bitmaps_are_preferred_over_colr_v0() {
        assert!(EmojiFormat::Bitmap.preference() < EmojiFormat::ColorV0.preference());
        assert!(EmojiFormat::ColorV0.preference() < EmojiFormat::ColorV1.preference());
    }

    /// Lines from an Android 14 `fonts.xml`.
    const ANDROID_14_FONTS_XML: &str = r#"
    <family lang="ko">
        <font weight="400" style="normal" index="1" postScriptName="NotoSansCJKjp-Regular">
            NotoSansCJK-Regular.ttc
        </font>
    </family>
    <family lang="und-Zsye" ignore="true">
        <font weight="400" style="normal">NotoColorEmojiLegacy.ttf</font>
    </family>
    <family lang="und-Zsye">
        <font weight="400" style="normal">NotoColorEmoji.ttf</font>
    </family>
    <family lang="und-Zsye">
        <font weight="400" style="normal">NotoColorEmojiFlags.ttf</font>
    </family>
    <family lang="und-Zsym">
        <font weight="400" style="normal">NotoSansSymbols-Regular-Subsetted2.ttf</font>
    </family>
    "#;

    #[test]
    fn the_emoji_fonts_a_system_declares_are_read_from_its_font_list() {
        assert_eq!(
            declared_emoji_files(ANDROID_14_FONTS_XML),
            [
                "NotoColorEmojiLegacy.ttf",
                "NotoColorEmoji.ttf",
                "NotoColorEmojiFlags.ttf"
            ]
        );
    }

    #[test]
    fn a_vendors_own_emoji_font_is_found_whatever_it_is_called() {
        let xml = r#"<familyset>
            <family name="sans-serif"><font>Roboto.ttf</font></family>
            <family lang="ja,und-Zsye"><font weight="400">VendorGlyphs.ttf</font></family>
        </familyset>"#;
        assert_eq!(declared_emoji_files(xml), ["VendorGlyphs.ttf"]);
        assert!(declared_emoji_files("<familyset/>").is_empty());
        assert!(declared_emoji_files("not xml at all").is_empty());
    }

    /// The real files from an Android 14 system image, when they are around:
    /// the system's own emoji font is COLRv1, the legacy one is bitmaps.
    #[test]
    fn android_14s_two_emoji_fonts_classify_as_expected() {
        let Some(dir) = std::env::var_os("IMMERMEMO_ANDROID_FONTS") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let read = |name: &str| classify(std::fs::File::open(dir.join(name)).unwrap()).unwrap();
        assert_eq!(read("NotoColorEmoji.ttf"), EmojiFormat::ColorV1);
        assert_eq!(read("NotoColorEmojiLegacy.ttf"), EmojiFormat::Bitmap);
    }
}
