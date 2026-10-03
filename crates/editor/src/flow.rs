//! Converts one paragraph's `immermemo_tomet_render::RenderItem`s (grouped
//! by `immermemo_tomet_render::classify_blocks`) into an
//! `origami_richtext_flow::Block`, preserving the inline positions
//! [`crate::classify::classify_body`] discards by flattening every item
//! into its own stacked row (see that module's doc comment, and the test
//! named `an_inline_merge_conflict_is_recognized_through_the_real_pipeline`
//! for the concrete case this exists to fix). Reuses `classify.rs`'s own
//! [`look_up`]/[`fallback_label`] so a recognized element gets the same
//! label/tone in both the flowing and the stacked presentation -- adding a
//! vocabulary entry never needs touching two places.
//!
//! Non-paragraph blocks (a standalone `@mobile.conflict` marker between
//! paragraphs, a `Section` heading) are not flowed here: inline flow only
//! matters where an element sits *inside* running prose, and
//! `classify_body`'s stacked `RenderedBlockView` path already renders a
//! standalone block-level marker correctly.

use immermemo_tomet_render::{RenderBlockKind, RenderItem, classify_blocks};
use origami_richtext_flow::{Block, Inline};

use crate::classify::{
    BlockShape, ClassifiedBlock, Tone, classify_body, fallback_label, look_up, to_classified_block,
};

/// One item of the note body's view-mode display, in source order: either
/// a classified block ([`ClassifiedBlock`], one shape per whole block --
/// what every block rendered as before this module existed) or a
/// paragraph with at least one inline element, kept together so it can
/// flow as one line instead of stacking each split fragment as its own
/// row (see the module doc).
#[derive(Debug, Clone, PartialEq)]
pub enum NoteBodyItem {
    Stacked(ClassifiedBlock),
    Flowed(FlowParagraph),
}

/// The single source of truth for the view-mode display: every top-level
/// block, in order, each already decided as [`NoteBodyItem::Stacked`] or
/// [`NoteBodyItem::Flowed`]. Supersedes calling [`classify_body`] directly
/// for this purpose -- that function is still correct (and still used by
/// [`flow_paragraphs`]'s own fallback) for a caller that only wants the
/// stacked presentation.
pub fn note_body_items(body: &str) -> Vec<NoteBodyItem> {
    let Ok(groups) = classify_blocks(body) else {
        // Same fallback `classify_body` uses: a note mid-keystroke on
        // something like `@mobile.conflict(` is unparseable, and gets one
        // plain-text span over the whole body rather than an empty list.
        return classify_body(body)
            .into_iter()
            .map(NoteBodyItem::Stacked)
            .collect();
    };
    groups
        .into_iter()
        .map(|(kind, items)| {
            if kind == RenderBlockKind::Paragraph && items.len() > 1 {
                NoteBodyItem::Flowed(to_flow_paragraph(body, items))
            } else {
                // Every other case -- an `Element`/`Section` group, or a
                // `Paragraph` with no inline element -- always has exactly
                // one item (see `tomet-render`'s `classify_block`).
                let item = items
                    .into_iter()
                    .next()
                    .expect("classify_block always returns at least one item");
                NoteBodyItem::Stacked(to_classified_block(body, item))
            }
        })
        .collect()
}

/// One paragraph's flowed content, plus the element payloads its
/// `Inline::Element { id }` entries index into. `id` is only an index into
/// `elements` -- see `origami_richtext_flow::model::ElementId`'s doc for
/// why this crate, not `origami-richtext-flow`, owns what an id means.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowParagraph {
    pub block: Block,
    pub elements: Vec<ClassifiedBlock>,
}

/// Flows every paragraph in `body` that contains at least one inline
/// element. A paragraph with none, and every non-paragraph block, is left
/// for [`crate::classify::classify_body`] to render as today -- only a
/// paragraph actually split by an inline marker needs the flow engine.
pub fn flow_paragraphs(body: &str) -> Vec<FlowParagraph> {
    let Ok(groups) = classify_blocks(body) else {
        return Vec::new();
    };
    groups
        .into_iter()
        .filter(|(kind, items)| *kind == RenderBlockKind::Paragraph && items.len() > 1)
        .map(|(_, items)| to_flow_paragraph(body, items))
        .collect()
}

fn to_flow_paragraph(body: &str, items: Vec<RenderItem>) -> FlowParagraph {
    let mut content = Vec::with_capacity(items.len());
    let mut elements = Vec::new();
    for item in items {
        match item {
            RenderItem::Text(span) => content.push(Inline::Text {
                content: body[span.start.offset..span.end.offset].to_string(),
                style: 0,
            }),
            RenderItem::Element {
                identity,
                args_summary,
                ..
            } => {
                // `classify_inline_seq` (`immermemo-tomet-render`) only
                // ever reports a *namespaced* inline element as its own
                // `RenderItem::Element` -- a bare one stays merged into
                // the surrounding text run -- so `identity.namespace` is
                // always `Some` here.
                let classified = match look_up(&identity, &args_summary) {
                    Some(look) => ClassifiedBlock {
                        shape: look.shape,
                        tone: look.tone,
                        text: look.text,
                    },
                    None => ClassifiedBlock {
                        shape: BlockShape::Badge,
                        tone: Tone::Neutral,
                        text: fallback_label(
                            identity.namespace.as_deref().unwrap_or_default(),
                            &identity.name,
                            &args_summary,
                        ),
                    },
                };
                // A divider marks where a conflict region ends -- a full-
                // width hairline is the right cue between stacked rows
                // (the reader is scanning top to bottom and needs an
                // explicit boundary), but inline, within one running line,
                // the boundary is already implicit the moment no more
                // Mine/Theirs chips follow: a rule slicing through a
                // sentence would break the text without adding
                // information. So it renders as nothing here, not a
                // shrunk-down sliver.
                if classified.shape != BlockShape::Divider {
                    let id = elements.len() as u64;
                    elements.push(classified);
                    content.push(Inline::Element { id });
                }
            }
        }
    }
    FlowParagraph {
        block: Block {
            kind: "paragraph",
            content,
        },
        elements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_body_items_mixes_stacked_and_flowed_items_in_source_order() {
        let src = "Before.\n\n\
                   @mobile.conflict(mine)\n\n\
                   The @mobile.conflict(theirs)slow@mobile.conflict(end) fox jumps.\n";
        let items = note_body_items(src);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], NoteBodyItem::Stacked(_)));
        assert!(matches!(items[1], NoteBodyItem::Stacked(_)));
        match &items[2] {
            NoteBodyItem::Flowed(paragraph) => {
                // "The ", theirs-chip, "slow", " fox jumps." -- the "end"
                // divider renders as nothing inline (see `to_flow_paragraph`).
                assert_eq!(paragraph.block.content.len(), 4);
                assert_eq!(paragraph.elements.len(), 1);
            }
            NoteBodyItem::Stacked(_) => panic!("expected the inline-split paragraph to flow"),
        }
    }

    #[test]
    fn a_plain_paragraph_has_no_flow_paragraphs() {
        // Only one `RenderItem` (no inline split) -- nothing for the flow
        // engine to do that `classify_body` doesn't already render fine.
        assert!(flow_paragraphs("Just a paragraph.\n").is_empty());
    }

    #[test]
    fn an_inline_conflict_flows_with_the_end_divider_rendering_as_nothing() {
        let src = "@use(mobile)\nThe @mobile.conflict(mine)slow@mobile.conflict(theirs)lazy@mobile.conflict(end) fox jumps.\n";
        let paragraphs = flow_paragraphs(src);
        assert_eq!(paragraphs.len(), 1);

        let paragraph = &paragraphs[0];
        assert_eq!(paragraph.block.kind, "paragraph");
        // "The ", mine-chip, "slow", theirs-chip, "lazy", " fox jumps." --
        // the "end" divider contributes no `Inline` at all (see
        // `to_flow_paragraph`'s doc comment), so only 2 elements remain
        // even though the source has 3 `@mobile.conflict(...)` markers.
        assert_eq!(paragraph.block.content.len(), 6);
        assert_eq!(paragraph.elements.len(), 2);
        assert!(
            paragraph
                .elements
                .iter()
                .all(|e| e.shape != BlockShape::Divider),
            "a Divider-shaped element must never reach the flowed content"
        );

        match &paragraph.block.content[0] {
            Inline::Text { content, .. } => assert_eq!(content, "The "),
            Inline::Element { .. } => panic!("expected text"),
        }
        match &paragraph.block.content[1] {
            Inline::Element { id } => {
                assert_eq!(paragraph.elements[*id as usize].text, "Mine");
                assert_eq!(paragraph.elements[*id as usize].shape, BlockShape::Chip);
                assert_eq!(paragraph.elements[*id as usize].tone, Tone::Accent);
            }
            Inline::Text { .. } => panic!("expected an element"),
        }
        match &paragraph.block.content[4] {
            Inline::Text { content, .. } => assert_eq!(content, "lazy"),
            Inline::Element { .. } => panic!("expected text"),
        }
        match &paragraph.block.content[5] {
            Inline::Text { content, .. } => assert_eq!(content, " fox jumps."),
            Inline::Element { .. } => panic!("expected text"),
        }
    }

    #[test]
    fn an_unregistered_namespaced_inline_element_gets_a_badge_fallback() {
        let src = "Before @deck.bookmark(label: \"x\") after.\n";
        let paragraphs = flow_paragraphs(src);
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].elements.len(), 1);
        assert_eq!(paragraphs[0].elements[0].shape, BlockShape::Badge);
        assert_eq!(paragraphs[0].elements[0].text, "deck.bookmark: label");
    }
}
