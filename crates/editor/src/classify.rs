//! Classifies a note body into the blocks the editor's view-mode display
//! draws, without knowing anything about Slint -- `crates/editor-slint`'s
//! `render.rs` (in `apps/slint`, where the generated Slint types actually
//! live -- see that crate's doc comment) converts [`ClassifiedBlock`] into
//! the Slint-generated `RenderedBlock` model `EditorScreen`'s
//! `rendered-blocks` property expects.
//!
//! `immermemo-tomet-render`'s `classify()` reports every named element's
//! `(namespace, name)` identity uniformly -- `@use`'d ones and Tomet's own
//! bare built-ins alike (`namespace` is `None` for the latter). Deciding
//! what a given identity *means*, visually, is left to whoever renders
//! it. [`REGISTRY`] below is that decision for this app: it maps a
//! recognized identity to a [`Look`] (a drawing primitive [`BlockShape`],
//! a [`Tone`], and a label), so giving a new vocabulary -- `@use`'d or
//! bare built-in -- its own presentation is "add one table entry," not
//! "add a match arm in three different places." Nothing vocabulary-
//! specific belongs in [`BlockShape`]/[`Tone`] themselves -- those are a
//! closed set of drawing primitives (not a vocabulary registry), the same
//! way a design system has a fixed set of component kinds that any number
//! of use-sites can be mapped onto.
//!
//! An identity with no [`REGISTRY`] entry falls back differently depending
//! on where it came from:
//! - A `@use`'d element (`namespace: Some`) falls back to a generic
//!   [`BlockShape::Badge`] labeled from its source text -- its raw
//!   `@ns.name(...)` syntax would look like broken code left as plain
//!   text, so showing *something* beats nothing.
//! - A bare built-in (`namespace: None`, e.g. a list) falls back to
//!   [`BlockShape::PlainText`] showing its source exactly as written --
//!   its own syntax (e.g. `- item`) already reads fine unrendered, so
//!   nothing this app hasn't explicitly opted into should change how it
//!   looks.

use immermemo_tomet_render::{ElementIdentity, RenderItem, classify};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockShape {
    /// A paragraph's plain content, shown as-is.
    PlainText,
    /// A small colored pill with a label (a `mobile.conflict` side marker).
    Chip,
    /// A hairline divider (a `mobile.conflict` end marker).
    Divider,
    /// A neutral pill with a label -- the fallback for anything
    /// namespaced that [`REGISTRY`] doesn't recognize.
    Badge,
}

/// A small, closed palette selector -- not vocabulary-specific, just
/// which of the app's existing semantic colors (see `Colors` in
/// `crates/origami-mobile/ui/tokens.slint`) a [`Look`] should draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Warning,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedBlock {
    pub shape: BlockShape,
    pub tone: Tone,
    /// The paragraph text for `PlainText`, or a label for `Chip`/`Badge`.
    /// Unused (empty) for `Divider` -- its look comes entirely from
    /// `shape`.
    pub text: String,
}

struct Look {
    shape: BlockShape,
    tone: Tone,
    text: String,
}

/// Which `(namespace, name)` pairs this app recognizes, and how each
/// should look. `namespace: None` registers a look for one of Tomet's own
/// bare built-in elements; the only entry today is `mobile.conflict`
/// (`immermemo_merge::CONFLICT_MARKER`). A future vocabulary -- `@use`'d
/// or bare built-in -- gets its own presentation by adding a row here,
/// not by touching [`look_up`] or anything downstream of it.
const REGISTRY: &[(Option<&str>, &str, fn(&str) -> Look)] =
    &[(Some("mobile"), "conflict", mobile_conflict_look)];

/// `@mobile.conflict(side)` -- `side` is the one positional arg
/// `classify` reports as `args_summary`.
fn mobile_conflict_look(args_summary: &str) -> Look {
    match args_summary {
        "mine" => Look {
            shape: BlockShape::Chip,
            tone: Tone::Accent,
            text: "Mine".to_owned(),
        },
        "theirs" => Look {
            shape: BlockShape::Chip,
            tone: Tone::Warning,
            text: "Theirs".to_owned(),
        },
        // "end", or anything else this marker might someday carry -- the
        // end marker has nothing worth labeling, just a divider, same as
        // an unrecognized `side`.
        _ => Look {
            shape: BlockShape::Divider,
            tone: Tone::Neutral,
            text: String::new(),
        },
    }
}

/// `None` if `identity` has no [`REGISTRY`] entry -- callers decide the
/// fallback themselves, since it differs by whether `identity` came in
/// through `@use` (see the module doc).
fn look_up(identity: &ElementIdentity, args_summary: &str) -> Option<Look> {
    REGISTRY
        .iter()
        .find(|(namespace, name, _)| {
            *namespace == identity.namespace.as_deref() && *name == identity.name
        })
        .map(|(_, _, look)| look(args_summary))
}

/// A short description of an unrecognized `@use`'d element, for
/// [`BlockShape::Badge`]. Not meant to round-trip -- just enough to show
/// something.
fn fallback_label(namespace: &str, name: &str, args_summary: &str) -> String {
    if args_summary.is_empty() {
        format!("{namespace}.{name}")
    } else {
        format!("{namespace}.{name}: {args_summary}")
    }
}

/// Classifies `body`'s top-level blocks for the editor's view-mode display.
pub fn classify_body(body: &str) -> Vec<ClassifiedBlock> {
    classify(body)
        .into_iter()
        .map(|item| to_classified_block(body, item))
        .collect()
}

fn to_classified_block(body: &str, item: RenderItem) -> ClassifiedBlock {
    match item {
        RenderItem::Text(span) => ClassifiedBlock {
            shape: BlockShape::PlainText,
            tone: Tone::Neutral,
            text: body[span.start.offset..span.end.offset].into(),
        },
        RenderItem::Element {
            identity,
            span,
            args_summary,
            ..
        } => match look_up(&identity, &args_summary) {
            Some(look) => ClassifiedBlock {
                shape: look.shape,
                tone: look.tone,
                text: look.text,
            },
            None => match &identity.namespace {
                // A `@use`'d marker nobody's given a look to yet -- show
                // *something* rather than raw `@ns.name(...)` syntax.
                Some(namespace) => ClassifiedBlock {
                    shape: BlockShape::Badge,
                    tone: Tone::Neutral,
                    text: fallback_label(namespace, &identity.name, &args_summary),
                },
                // A bare built-in nobody's registered a look for -- its
                // own syntax already reads fine unrendered (e.g. a list's
                // `- item` lines), so show it exactly as written, same as
                // if it had never been classified as an element at all.
                None => ClassifiedBlock {
                    shape: BlockShape::PlainText,
                    tone: Tone::Neutral,
                    text: body[span.start.offset..span.end.offset].into(),
                },
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes(body: &str) -> Vec<BlockShape> {
        classify_body(body).into_iter().map(|b| b.shape).collect()
    }

    fn summary(body: &str) -> Vec<(BlockShape, Tone, String)> {
        classify_body(body)
            .into_iter()
            .map(|b| (b.shape, b.tone, b.text))
            .collect()
    }

    #[test]
    fn plain_text_round_trips_verbatim() {
        // A block's span covers its own content only, not the blank-line
        // separator after it -- hence no trailing "\n" here.
        let blocks = classify_body("Just a paragraph.\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::PlainText);
        assert_eq!(blocks[0].text, "Just a paragraph.");
    }

    #[test]
    fn a_conflict_triple_becomes_mine_theirs_end() {
        let src = "Before.\n\n\
                   @mobile.conflict(mine)\n\n\
                   Mine text.\n\n\
                   @mobile.conflict(theirs)\n\n\
                   Theirs text.\n\n\
                   @mobile.conflict(end)\n\n\
                   After.\n";
        assert_eq!(
            summary(src)
                .into_iter()
                .map(|(shape, tone, text)| (shape, tone, text))
                .collect::<Vec<_>>(),
            vec![
                (BlockShape::PlainText, Tone::Neutral, "Before.".to_owned()),
                (BlockShape::Chip, Tone::Accent, "Mine".to_owned()),
                (BlockShape::PlainText, Tone::Neutral, "Mine text.".to_owned()),
                (BlockShape::Chip, Tone::Warning, "Theirs".to_owned()),
                (
                    BlockShape::PlainText,
                    Tone::Neutral,
                    "Theirs text.".to_owned()
                ),
                (BlockShape::Divider, Tone::Neutral, String::new()),
                (BlockShape::PlainText, Tone::Neutral, "After.".to_owned()),
            ]
        );
    }

    #[test]
    fn an_unknown_namespaced_element_gets_a_labeled_fallback() {
        let blocks = classify_body("@deck.bookmark(label: \"Chapter 1\")\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Badge);
        assert_eq!(blocks[0].tone, Tone::Neutral);
        assert_eq!(blocks[0].text, "deck.bookmark: label");
    }

    // `immermemo-tomet-render` now identifies bare built-in elements too
    // (not just `@use`'d ones -- see its module doc), so the registry
    // mechanism above can already be extended to give one of them a
    // custom look by adding a `REGISTRY` row. Nothing has opted in yet,
    // so an unregistered bare built-in (a list) must still show exactly
    // as written, not some generic badge -- this is the no-regression
    // guarantee that extension relies on.
    #[test]
    fn an_unregistered_bare_builtin_element_shows_exactly_as_written() {
        let src = "- one\n- two\n";
        let blocks = classify_body(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::PlainText);
        assert_eq!(blocks[0].tone, Tone::Neutral);
        assert_eq!(blocks[0].text, src);
    }

    #[test]
    fn empty_body_has_no_blocks() {
        assert!(classify_body("").is_empty());
    }

    // `immermemo_merge::merge_one` always recurses into a
    // Paragraph/Paragraph/Paragraph triple (`merge_inline_seq` never
    // fails), so two sides editing the *same* paragraph differently --
    // the common conflict -- narrows to *inline* conflict markers, which
    // this crate deliberately does not touch (see the module doc). Only a
    // structural mismatch (here: local turns the block into a different
    // kind of node, which `merge_one` can't narrow across) produces the
    // whole-block markers `classify_body` recognizes. This test proves
    // the real `immermemo_merge::merge` -> `tomet_parser::parse_document`
    // -> `classify` pipeline actually reaches `Chip`/`Chip`/`Divider` for
    // that case, not just a hand-written `@mobile.conflict(...)` fixture.
    #[test]
    fn an_inline_merge_conflict_is_recognized_through_the_real_pipeline() {
        let base = tomet_parser::parse_document("The quick fox jumps.\n").unwrap();
        let local = tomet_parser::parse_document("The slow fox jumps.\n").unwrap();
        let remote = tomet_parser::parse_document("The lazy fox jumps.\n").unwrap();
        let merged = immermemo_merge::merge(&base, &local, &remote).document;
        let text = tomet_printer::document_to_tm(&merged);

        assert_eq!(
            shapes(&text),
            vec![
                BlockShape::PlainText, // "@use(mobile)" -- bare, not namespaced
                BlockShape::PlainText, // "The "
                BlockShape::Chip,
                BlockShape::PlainText, // "slow"
                BlockShape::Chip,
                BlockShape::PlainText, // "lazy"
                BlockShape::Divider,
                BlockShape::PlainText, // " fox jumps."
            ]
        );
    }

    #[test]
    fn a_structural_merge_conflict_is_recognized_through_the_real_pipeline() {
        let base = tomet_parser::parse_document("Original.\n").unwrap();
        let local = tomet_parser::parse_document("@meta{ x: 1 }\n").unwrap();
        let remote = tomet_parser::parse_document("Theirs.\n").unwrap();
        let merged = immermemo_merge::merge(&base, &local, &remote).document;
        let text = tomet_printer::document_to_tm(&merged);

        assert_eq!(
            shapes(&text),
            vec![
                BlockShape::PlainText, // "@use(mobile)" -- bare, not namespaced
                BlockShape::Chip,
                BlockShape::PlainText, // "@meta{ x: 1 }" -- bare too
                BlockShape::Chip,
                BlockShape::PlainText, // "Theirs."
                BlockShape::Divider,
            ]
        );
    }
}
