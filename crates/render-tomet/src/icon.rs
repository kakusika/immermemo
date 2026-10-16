//! A note's icon: what `@meta{ icon: ... }` says, and how a text icon is laid out.
//!
//! The same three shapes `tomet-book` accepts for a document icon:
//!
//! - `icon: @doc.icon("star")` -- a named glyph from a vendored icon package;
//! - `icon: "🎂"` -- a short text or emoji, laid out per [`text_icon_cells`];
//! - `icon: "cover.png"`, `icon: "https://example.com/a.png"` or
//!   `icon: @embed("cover.png")` -- an image, as the reference was written
//!   (resolving it to bytes is not this crate's job).

use tomet::ast::Value;
use unicode_segmentation::UnicodeSegmentation;

use crate::meta::{element_identity_and_args, meta_value};
use crate::types::ElementArgs;

/// The icon package `@doc.icon(name)` resolves against when it omits `pkg`.
pub const DEFAULT_ICON_PKG: &str = "tabler";

/// What a note declares as its icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteIcon {
    /// A named glyph of an icon package (`@doc.icon("star", pkg: "tabler")`).
    Glyph { name: String, pkg: String },
    /// Text or emoji, trimmed and non-empty.
    Text(String),
    /// An image reference exactly as written in the note: a vault path
    /// (relative to the note or to the vault root) or a URL.
    Image(String),
}

/// Reads `src`'s `@meta{ icon: ... }`, if it holds a recognizable icon.
pub fn extract_note_icon(src: &str) -> Option<NoteIcon> {
    match meta_value(src, "icon")? {
        Value::String(s) => icon_from_string(&s),
        Value::Element(el) => {
            let (identity, args) = element_identity_and_args(&el)?;
            match (identity.namespace.as_deref(), identity.name.as_str()) {
                (Some("doc"), "icon") => glyph_from_args(&args),
                (None | Some("std" | "doc"), "embed" | "link" | "ref") => {
                    media_target(&args).map(NoteIcon::Image)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn icon_from_string(s: &str) -> Option<NoteIcon> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else if is_image_reference(s) {
        Some(NoteIcon::Image(s.to_owned()))
    } else {
        Some(NoteIcon::Text(s.to_owned()))
    }
}

fn glyph_from_args(args: &ElementArgs) -> Option<NoteIcon> {
    let (name, pkg) = match args {
        ElementArgs::Positional(name) => (name.clone(), None),
        ElementArgs::Named(pairs) => (
            pairs
                .iter()
                .find(|(key, _)| key.is_empty())
                .map(|(_, value)| value.clone())
                .unwrap_or_default(),
            pairs
                .iter()
                .find(|(key, _)| key == "pkg")
                .map(|(_, value)| value.clone()),
        ),
        ElementArgs::None => return None,
    };
    (!name.is_empty()).then(|| NoteIcon::Glyph {
        name,
        pkg: pkg.unwrap_or_else(|| DEFAULT_ICON_PKG.to_owned()),
    })
}

/// The reference an `@embed`/`@link`/`@ref` points at: its positional
/// string, or the first of the target-like named args (`path`, `ref`,
/// `src`, `url`, ...).
pub fn media_target(args: &ElementArgs) -> Option<String> {
    let target = match args {
        ElementArgs::Positional(target) => target.clone(),
        ElementArgs::Named(pairs) => pairs
            .iter()
            .find(|(key, _)| {
                matches!(
                    key.as_str(),
                    "" | "target" | "ref" | "path" | "src" | "url" | "href"
                )
            })
            .map(|(_, value)| value.clone())?,
        ElementArgs::None => return None,
    };
    let target = target.trim();
    (!target.is_empty()).then(|| target.to_owned())
}

/// Whether a plain string names an image rather than being display text:
/// an `http(s)` URL, or a path whose extension is an image's.
pub(crate) fn is_image_reference(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        return true;
    }
    let path = s.split(['?', '#']).next().unwrap_or(s);
    let ext = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    matches!(
        ext.as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "ico")
    )
}

/// The cells a text icon is drawn from, one grapheme each (so an emoji with
/// a skin tone or a ZWJ sequence stays whole). Mirrors `tomet-book`'s
/// avatar rule: 1 grapheme is one large glyph, 2 sit side by side, 3 fill a
/// 2x2 grid with the third centred on the bottom row, 4 fill the grid, and
/// 5 or more are abbreviated to their first two. The caller picks the
/// arrangement from `cells.len()`.
pub fn text_icon_cells(text: &str) -> Vec<String> {
    let cells = text.graphemes(true).filter(|g| !g.trim().is_empty());
    let cells: Vec<String> = cells.map(str::to_owned).collect();
    if cells.len() >= 5 {
        cells[..2].to_vec()
    } else {
        cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon_of(meta_entry: &str) -> Option<NoteIcon> {
        extract_note_icon(&format!("@meta{{ {meta_entry} }}\n\nBody.\n"))
    }

    fn glyph(name: &str, pkg: &str) -> Option<NoteIcon> {
        Some(NoteIcon::Glyph {
            name: name.to_owned(),
            pkg: pkg.to_owned(),
        })
    }

    #[test]
    fn a_doc_icon_is_a_glyph_in_the_default_package_unless_one_is_named() {
        assert_eq!(
            icon_of("icon: @doc.icon(\"star\")"),
            glyph("star", "tabler")
        );
        assert_eq!(
            icon_of("icon: @doc.icon(\"star\", pkg: \"lucide\")"),
            glyph("star", "lucide")
        );
    }

    #[test]
    fn a_doc_icon_without_a_name_is_no_icon() {
        assert_eq!(icon_of("icon: @doc.icon(\"\")"), None);
        assert_eq!(icon_of("icon: @doc.icon()"), None);
    }

    #[test]
    fn a_plain_string_is_text_unless_it_names_an_image() {
        assert_eq!(
            icon_of("icon: \"🎂\""),
            Some(NoteIcon::Text("🎂".to_owned()))
        );
        assert_eq!(
            icon_of("icon: \"  Design \""),
            Some(NoteIcon::Text("Design".to_owned()))
        );
        assert_eq!(
            icon_of("icon: \"images/cover.PNG\""),
            Some(NoteIcon::Image("images/cover.PNG".to_owned()))
        );
        assert_eq!(
            icon_of("icon: \"https://example.com/a?x=1\""),
            Some(NoteIcon::Image("https://example.com/a?x=1".to_owned()))
        );
        assert_eq!(
            icon_of("icon: \"cover.png?v=2\""),
            Some(NoteIcon::Image("cover.png?v=2".to_owned()))
        );
    }

    #[test]
    fn a_blank_string_is_no_icon() {
        assert_eq!(icon_of("icon: \"   \""), None);
    }

    #[test]
    fn embed_link_and_ref_elements_are_images() {
        for element in ["embed", "link", "ref"] {
            assert_eq!(
                icon_of(&format!("icon: @{element}(\"cover.png\")")),
                Some(NoteIcon::Image("cover.png".to_owned())),
                "@{element}"
            );
        }
        assert_eq!(
            icon_of("icon: @embed(src: \"cover.png\")"),
            Some(NoteIcon::Image("cover.png".to_owned()))
        );
    }

    #[test]
    fn anything_else_is_no_icon() {
        assert_eq!(icon_of("icon: 3"), None);
        assert_eq!(icon_of("icon: @callout(\"x\")"), None);
        assert_eq!(icon_of("title: \"x\""), None);
        assert_eq!(extract_note_icon("Just a paragraph.\n"), None);
    }

    #[test]
    fn text_icon_cells_follow_the_avatar_rule() {
        assert_eq!(text_icon_cells("🎂"), ["🎂"]);
        assert_eq!(text_icon_cells("🐾🩵"), ["🐾", "🩵"]);
        assert_eq!(text_icon_cells("🐾🩵🔥"), ["🐾", "🩵", "🔥"]);
        assert_eq!(text_icon_cells("🐾🩵🦀🔥"), ["🐾", "🩵", "🦀", "🔥"]);
        assert_eq!(text_icon_cells("Design"), ["D", "e"]);
    }

    #[test]
    fn text_icon_cells_keep_a_grapheme_cluster_whole_and_skip_whitespace() {
        // A family emoji is one grapheme made of several scalar values, and a
        // flag is a pair of regional indicators.
        assert_eq!(text_icon_cells("👨‍👩‍👧"), ["👨‍👩‍👧"]);
        assert_eq!(text_icon_cells("🇯🇵 a"), ["🇯🇵", "a"]);
        assert!(text_icon_cells("  ").is_empty());
    }
}
