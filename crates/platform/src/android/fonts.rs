//! Repairs the font fallbacks Slint builds on Android.
//!
//! Slint hands text layout to parley, which asks fontique for fallback fonts.
//! fontique's Android backend derives them from `/system/etc/fonts.xml`, and
//! what it reads leaves text blank in two ways:
//!
//! - **Emoji.** The generic emoji family is hard-wired to a font named "Noto
//!   Color Emoji". Since Android 13 that font is COLRv1, which the renderer
//!   cannot draw (see [`emoji_font`](crate::platform::emoji_font)), so every
//!   emoji came out as nothing; and a vendor that ships its own emoji font under
//!   another name left the family empty. So the emoji fonts on the device are
//!   looked at -- the files its own font list declares for emoji (`und-Zsye`),
//!   and any with "emoji" in the name, in the system font
//!   directories -- and the first the renderer can draw (bitmaps before COLRv0)
//!   is registered under a name of its own and made the generic emoji family,
//!   which is what parley falls back to for an emoji cluster. (A name of its
//!   own because Android's two Noto files share "Noto Color Emoji", and a family
//!   of that name would merge them.) Android 13 and later ship a bitmap build
//!   next to the COLRv1 one, `NotoColorEmojiLegacy.ttf`, which this finds.
//! - **Symbols.** `NotoSansSymbols-Regular-Subsetted.ttf` carries the common
//!   geometric and decorative symbols (`◕`, `✧`, ...) that kaomoji lean on. It
//!   is listed in `fonts.xml` as a family with neither `lang` nor
//!   `postScriptName`, which fontique reads only into the fallbacks that have
//!   one, so the glyphs were unreachable. It is appended to the generic
//!   families Slint falls back through, after the UI font and the CJK fonts.
//!
//! What was found and chosen is kept as text ([`report`]) and shown on the About
//! page: a phone cannot be looked into from the desk, and "emoji do not show"
//! has more than one cause.

use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use slint::fontique_011::fontique::{Blob, FontInfoOverride, GenericFamily};

use crate::emoji_font::{EmojiFormat, classify, declared_emoji_files};

/// Where Android keeps system fonts: the base image, then the partitions a
/// vendor adds to.
const FONT_DIRS: [&str; 4] = [
    "/system/fonts",
    "/product/fonts",
    "/system_ext/fonts",
    "/vendor/fonts",
];

/// The system font lists: `fonts.xml`, and `font_fallback.xml` that replaced it
/// in Android 12.
const FONT_LISTS: [&str; 2] = ["/system/etc/fonts.xml", "/system/etc/font_fallback.xml"];

/// The family name the chosen emoji font is registered under. Not its own:
/// see the module doc.
const EMOJI_FAMILY: &str = "Immermemo Emoji";

/// The system family holding the symbol fonts.
const SYMBOLS_FAMILY: &str = "Noto Sans Symbols";

static REPORT: OnceLock<String> = OnceLock::new();

/// What [`repair_fallbacks`] found and chose for emoji, in one line.
pub fn report() -> String {
    REPORT.get().cloned().unwrap_or_default()
}

/// An emoji font file found on the device.
struct Found {
    path: PathBuf,
    format: EmojiFormat,
    /// The file's size: a full emoji font is megabytes, a flags-only one is not.
    size: u64,
}

impl Found {
    fn name(&self) -> String {
        self.path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }

    fn describe(&self) -> String {
        format!("{} ({})", self.name(), self.format.label())
    }
}

/// What the search for emoji fonts looked at and found.
struct Search {
    /// Best to draw first.
    found: Vec<Found>,
    /// The file names the system's font list declares for emoji.
    declared: Vec<String>,
    /// How many files each font directory held (`None`: unreadable).
    directories: Vec<(&'static str, Option<usize>)>,
}

/// Emoji font files in the system font directories: those the font list
/// declares for emoji, and those with "emoji" in their name.
fn search_emoji_fonts() -> Search {
    let declared: Vec<String> = FONT_LISTS
        .iter()
        .filter_map(|list| std::fs::read_to_string(list).ok())
        .flat_map(|xml| declared_emoji_files(&xml))
        .collect();

    let mut directories = Vec::new();
    let mut found: Vec<Found> = Vec::new();
    for dir in FONT_DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            directories.push((dir, None));
            continue;
        };
        let mut count = 0;
        for entry in entries.flatten() {
            count += 1;
            let file = entry.file_name().to_string_lossy().into_owned();
            let lower = file.to_lowercase();
            // A flags-only font covers a few dozen glyphs, not emoji -- even
            // when the system lists it among its emoji fonts, as Android's
            // own does.
            if lower.contains("flag") {
                continue;
            }
            if !lower.contains("emoji") && !declared.contains(&file) {
                continue;
            }
            let path = entry.path();
            if found.iter().any(|f| f.path == path) {
                continue;
            }
            let size = entry.metadata().map_or(0, |meta| meta.len());
            if let Some(format) = File::open(&path).ok().and_then(|f| classify(f).ok()) {
                found.push(Found { path, format, size });
            }
        }
        directories.push((dir, Some(count)));
    }
    // Drawable first, then the biggest: the font with the most glyphs.
    found.sort_by_key(|font| {
        (
            font.format.preference(),
            std::cmp::Reverse(font.size),
            font.name(),
        )
    });
    Search {
        found,
        declared,
        directories,
    }
}

/// Applies the repairs. Call once the Slint backend is initialised; they apply
/// to the whole process.
pub fn repair_fallbacks() {
    let mut collection = slint::fontique_011::shared_collection();

    let search = search_emoji_fonts();
    let chosen = search.found.iter().find(|font| font.format.drawable());
    let mut report = match chosen {
        Some(font) => match register_emoji_font(&mut collection, font) {
            Ok(()) => font.describe(),
            Err(error) => format!("{} could not be loaded: {error}", font.describe()),
        },
        None if search.found.is_empty() => "no emoji font file found".to_owned(),
        None => format!(
            "none the renderer can draw ({})",
            search
                .found
                .iter()
                .map(Found::describe)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    if chosen.is_none() {
        // Nothing usable: say where it looked, so the next step needs no
        // guessing from the desk.
        report.push_str(&format!(
            "; the system lists {}; searched {}",
            if search.declared.is_empty() {
                "none".to_owned()
            } else {
                search.declared.join(", ")
            },
            search
                .directories
                .iter()
                .map(|(dir, count)| match count {
                    Some(n) => format!("{dir} ({n} files)"),
                    None => format!("{dir} (unreadable)"),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let _ = REPORT.set(report);

    if let Some(symbols) = collection.family_id(SYMBOLS_FAMILY) {
        // The same pair Slint appends to every font stack.
        for generic in [GenericFamily::SansSerif, GenericFamily::SystemUi] {
            collection.append_generic_families(generic, [symbols].into_iter());
        }
    }
}

/// Registers `font` and makes it the generic emoji family.
///
/// The file can be ~9 MB and fontique keeps it in memory for the process's
/// life; that is the price of drawing emoji at all on these devices.
fn register_emoji_font(
    collection: &mut slint::fontique_011::fontique::Collection,
    font: &Found,
) -> std::io::Result<()> {
    let bytes = std::fs::read(&font.path)?;
    let registered = collection.register_fonts(
        Blob::new(Arc::new(bytes)),
        Some(FontInfoOverride {
            family_name: Some(EMOJI_FAMILY),
            ..Default::default()
        }),
    );
    if registered.is_empty() {
        return Err(std::io::Error::other("no font in the file"));
    }
    collection.set_generic_families(
        GenericFamily::Emoji,
        registered.iter().map(|(family, _)| *family),
    );
    Ok(())
}
