//! A note's banner: the wide image across the top of the note.
//!
//! `@meta{ banner: "images/hero.jpg", banner-y: 30 }`, the keys `tomet-shelf`
//! uses. The value is an image reference in any of the forms an icon takes
//! (a path, a URL, `@embed(...)`); `banner-y` (or `banner_y`) is which
//! vertical position of the image stays in view when it is cropped to the
//! banner's shape, as a percentage from the top (0 is the top edge, 100 the
//! bottom, 50 the default).

use tomet::ast::Value;

use crate::icon::media_target;
use crate::meta::{element_identity_and_args, meta_pairs};

/// What a note declares as its banner.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteBanner {
    /// The image reference exactly as written: a vault path (relative to the
    /// note or the vault root) or a URL.
    pub reference: String,
    /// Which vertical position stays in view when the image is cropped, as a
    /// percentage (0 to 100) from the top. `None` means centred.
    pub focus_y: Option<f32>,
}

/// Reads `src`'s `@meta{ banner: ... }`, if it holds an image reference.
pub fn extract_note_banner(src: &str) -> Option<NoteBanner> {
    let pairs = meta_pairs(src);
    let value = |key: &str| {
        pairs
            .iter()
            .find_map(|(entry_key, value)| (entry_key == key).then_some(value))
    };

    let reference = match value("banner")? {
        Value::String(reference) => reference.trim().to_owned(),
        Value::Element(el) => {
            let (identity, args) = element_identity_and_args(el)?;
            match (identity.namespace.as_deref(), identity.name.as_str()) {
                (None | Some("std" | "doc"), "embed" | "link" | "ref") => media_target(&args)?,
                _ => return None,
            }
        }
        _ => return None,
    };
    if reference.is_empty() {
        return None;
    }

    let focus_y = value("banner-y")
        .or_else(|| value("banner_y"))
        .and_then(|y| match y {
            Value::Int(n) => Some(*n as f32),
            Value::Float(n) => Some(*n as f32),
            _ => None,
        })
        .map(|y| y.clamp(0.0, 100.0));

    Some(NoteBanner { reference, focus_y })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner_of(meta: &str) -> Option<NoteBanner> {
        extract_note_banner(&format!("@meta{{ {meta} }}\n\nBody.\n"))
    }

    fn banner(reference: &str, focus_y: Option<f32>) -> Option<NoteBanner> {
        Some(NoteBanner {
            reference: reference.to_owned(),
            focus_y,
        })
    }

    #[test]
    fn a_string_or_an_embed_is_a_banner_reference() {
        assert_eq!(
            banner_of("banner: \"images/hero.jpg\""),
            banner("images/hero.jpg", None)
        );
        assert_eq!(
            banner_of("banner: \"https://example.com/h.png\""),
            banner("https://example.com/h.png", None)
        );
        assert_eq!(
            banner_of("banner: @embed(path: \"hero.jpg\")"),
            banner("hero.jpg", None)
        );
        assert_eq!(
            banner_of("banner: @embed(\"hero.jpg\")"),
            banner("hero.jpg", None)
        );
    }

    #[test]
    fn banner_y_accepts_both_spellings_and_both_number_types_and_is_clamped() {
        assert_eq!(
            banner_of("banner: \"h.jpg\", banner-y: 30"),
            banner("h.jpg", Some(30.0))
        );
        assert_eq!(
            banner_of("banner: \"h.jpg\", banner_y: 12.5"),
            banner("h.jpg", Some(12.5))
        );
        assert_eq!(
            banner_of("banner: \"h.jpg\", banner-y: 250"),
            banner("h.jpg", Some(100.0))
        );
        assert_eq!(
            banner_of("banner: \"h.jpg\", banner-y: -5"),
            banner("h.jpg", Some(0.0))
        );
        assert_eq!(
            banner_of("banner: \"h.jpg\", banner-y: \"top\""),
            banner("h.jpg", None)
        );
    }

    #[test]
    fn a_link_by_file_name_is_a_banner_reference() {
        // The form a vault written with `@link(ref: ...)` uses everywhere: a
        // bare file name, resolved against the whole vault by `immermemo_media`.
        assert_eq!(
            banner_of("banner: @link(ref:+c14d6f0b.png), banner-y: 7"),
            banner("+c14d6f0b.png", Some(7.0))
        );
    }

    #[test]
    fn anything_else_is_no_banner() {
        assert_eq!(banner_of("banner: \"  \""), None);
        assert_eq!(banner_of("banner: 3"), None);
        assert_eq!(banner_of("banner: @callout(\"x\")"), None);
        assert_eq!(banner_of("banner-y: 30"), None);
        assert_eq!(extract_note_banner("Just a paragraph.\n"), None);
    }
}
