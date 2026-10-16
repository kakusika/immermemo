//! Decoding an original and shrinking it to a thumbnail.

use std::io::Cursor;

use anyhow::{Context, bail};
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};

/// The most an SVG may weigh, in bytes. It is not rasterized here, only
/// bounded, so a note cannot make the UI toolkit parse a huge document.
pub const MAX_SVG_BYTES: usize = 512 * 1024;

/// The widest or tallest an image may be, in pixels, before the decoder
/// refuses it.
pub const DECODE_MAX_DIMENSION: u32 = 8192;

/// The most memory the decoder may allocate for one image, in bytes.
pub const DECODE_MAX_ALLOC: u64 = 256 * 1024 * 1024;

/// How small an image is made, by where it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    /// A note's icon in a list, a tile or the mini card: 256px on the long
    /// edge, enough for a ~100px tile at 2.5x density.
    Thumb,
    /// A note's banner across the top of the note: 1280px on the long edge.
    Banner,
    /// An image embedded in a note's body: 1280px on the long edge.
    Body,
}

impl Preset {
    /// The longest edge, in pixels, an image of this preset has.
    pub fn max_edge(self) -> u32 {
        match self {
            Self::Thumb => 256,
            Self::Banner | Self::Body => 1280,
        }
    }

    pub(crate) fn dir_name(self) -> &'static str {
        match self {
            Self::Thumb => "thumb",
            Self::Banner => "banner",
            Self::Body => "body",
        }
    }
}

/// A produced thumbnail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Thumbnail {
    /// Decoded straight RGBA, `width * height * 4` bytes, upright (EXIF
    /// orientation already applied).
    Raster {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    /// An SVG document, as written.
    Svg(Vec<u8>),
}

/// How a thumbnail is stored on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Jpeg,
    Png,
    Svg,
}

impl Format {
    pub(crate) const ALL: [Format; 3] = [Format::Jpeg, Format::Png, Format::Svg];

    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Svg => "svg",
        }
    }
}

impl Thumbnail {
    /// The bytes to store, and how they are encoded: JPEG for an opaque
    /// raster, PNG for one with transparency, the document itself for SVG.
    pub(crate) fn encode(&self) -> anyhow::Result<(Format, Vec<u8>)> {
        match self {
            Self::Svg(doc) => Ok((Format::Svg, doc.clone())),
            Self::Raster {
                width,
                height,
                rgba,
            } => {
                let image = image::RgbaImage::from_raw(*width, *height, rgba.clone())
                    .context("pixel buffer does not match its dimensions")?;
                let mut out = Vec::new();
                // Every fourth byte is an alpha value.
                if rgba.iter().skip(3).step_by(4).any(|&alpha| alpha != 255) {
                    DynamicImage::ImageRgba8(image)
                        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)?;
                    Ok((Format::Png, out))
                } else {
                    let rgb = DynamicImage::ImageRgba8(image).to_rgb8();
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
                        .encode_image(&rgb)?;
                    Ok((Format::Jpeg, out))
                }
            }
        }
    }

    /// Reads back what [`Thumbnail::encode`] stored.
    pub(crate) fn decode(format: Format, bytes: &[u8]) -> anyhow::Result<Self> {
        match format {
            Format::Svg => Ok(Self::Svg(bytes.to_vec())),
            Format::Jpeg | Format::Png => {
                let rgba = image::load_from_memory(bytes)?.to_rgba8();
                Ok(Self::Raster {
                    width: rgba.width(),
                    height: rgba.height(),
                    rgba: rgba.into_raw(),
                })
            }
        }
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let head = head.trim_ascii_start();
    head.starts_with(b"<svg")
        || (head.starts_with(b"<?xml") && head.windows(4).any(|w| w == b"<svg"))
}

/// Decodes `bytes` (PNG, JPEG, WebP or SVG) and returns it fitted to
/// `preset`, never enlarged. A photograph's EXIF orientation is applied.
///
/// Fails on bytes that are not a supported image, an image over the decode
/// limits ([`DECODE_MAX_DIMENSION`], [`DECODE_MAX_ALLOC`]) and an SVG over
/// [`MAX_SVG_BYTES`].
pub fn make_thumbnail(bytes: &[u8], preset: Preset) -> anyhow::Result<Thumbnail> {
    if looks_like_svg(bytes) {
        if bytes.len() > MAX_SVG_BYTES {
            bail!(
                "SVG is {} bytes, over the {MAX_SVG_BYTES} byte limit",
                bytes.len()
            );
        }
        return Ok(Thumbnail::Svg(bytes.to_vec()));
    }

    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits({
        let mut limits = Limits::default();
        limits.max_image_width = Some(DECODE_MAX_DIMENSION);
        limits.max_image_height = Some(DECODE_MAX_DIMENSION);
        limits.max_alloc = Some(DECODE_MAX_ALLOC);
        limits
    });
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);

    let edge = preset.max_edge();
    if image.width() > edge || image.height() > edge {
        image = image.thumbnail(edge, edge);
    }
    let rgba = image.to_rgba8();
    Ok(Thumbnail::Raster {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, alpha: u8) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, alpha]));
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn size(thumb: &Thumbnail) -> (u32, u32) {
        match thumb {
            Thumbnail::Raster { width, height, .. } => (*width, *height),
            Thumbnail::Svg(_) => panic!("expected a raster"),
        }
    }

    #[test]
    fn a_large_image_is_fitted_to_the_preset_keeping_its_aspect_ratio() {
        let thumb = make_thumbnail(&png(1000, 500, 255), Preset::Thumb).unwrap();
        assert_eq!(size(&thumb), (256, 128));
        let thumb = make_thumbnail(&png(300, 900, 255), Preset::Thumb).unwrap();
        assert_eq!(size(&thumb), (85, 256));
    }

    #[test]
    fn the_wide_presets_keep_far_more_detail_than_a_thumbnail() {
        for preset in [Preset::Banner, Preset::Body] {
            let thumb = make_thumbnail(&png(3000, 1500, 255), preset).unwrap();
            assert_eq!(size(&thumb), (1280, 640), "{preset:?}");
        }
    }

    #[test]
    fn each_preset_is_cached_apart() {
        let dirs: std::collections::HashSet<_> = [Preset::Thumb, Preset::Banner, Preset::Body]
            .into_iter()
            .map(Preset::dir_name)
            .collect();
        assert_eq!(dirs.len(), 3);
    }

    #[test]
    fn a_small_image_is_never_enlarged() {
        let thumb = make_thumbnail(&png(40, 30, 255), Preset::Thumb).unwrap();
        assert_eq!(size(&thumb), (40, 30));
    }

    #[test]
    fn an_opaque_thumbnail_is_stored_as_jpeg_and_a_transparent_one_as_png() {
        let opaque = make_thumbnail(&png(64, 64, 255), Preset::Thumb).unwrap();
        let (format, bytes) = opaque.encode().unwrap();
        assert_eq!(format, Format::Jpeg);
        assert_eq!(&bytes[..2], [0xFF, 0xD8]);

        let clear = make_thumbnail(&png(64, 64, 100), Preset::Thumb).unwrap();
        let (format, bytes) = clear.encode().unwrap();
        assert_eq!(format, Format::Png);
        assert_eq!(&bytes[..4], [0x89, b'P', b'N', b'G']);
    }

    #[test]
    fn a_stored_thumbnail_decodes_back_to_its_dimensions() {
        let thumb = make_thumbnail(&png(500, 250, 255), Preset::Thumb).unwrap();
        let (format, bytes) = thumb.encode().unwrap();
        assert_eq!(
            size(&Thumbnail::decode(format, &bytes).unwrap()),
            (256, 128)
        );
    }

    #[test]
    fn svg_is_passed_through_unrasterized() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"/>"#;
        assert_eq!(
            make_thumbnail(svg, Preset::Thumb).unwrap(),
            Thumbnail::Svg(svg.to_vec())
        );
        let xml = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#;
        assert!(matches!(
            make_thumbnail(xml, Preset::Thumb),
            Ok(Thumbnail::Svg(_))
        ));
    }

    #[test]
    fn an_oversized_svg_is_refused() {
        let mut svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\">".to_vec();
        svg.resize(MAX_SVG_BYTES + 1, b' ');
        assert!(make_thumbnail(&svg, Preset::Thumb).is_err());
    }

    #[test]
    fn bytes_that_are_not_an_image_are_refused() {
        assert!(make_thumbnail(b"not an image at all", Preset::Thumb).is_err());
        assert!(make_thumbnail(b"", Preset::Thumb).is_err());
        assert!(make_thumbnail(b"<html><svg/></html>", Preset::Thumb).is_err());
    }

    #[test]
    fn an_image_over_the_decode_limits_is_refused() {
        let wide = png(DECODE_MAX_DIMENSION + 1, 1, 255);
        assert!(make_thumbnail(&wide, Preset::Thumb).is_err());
        let tall = png(1, DECODE_MAX_DIMENSION + 1, 255);
        assert!(make_thumbnail(&tall, Preset::Thumb).is_err());
    }
}
