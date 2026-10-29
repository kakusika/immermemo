//! Converts one paragraph's `immermemo_render_tomet::RenderItem`s (grouped
//! by `immermemo_render_tomet::classify_blocks`) into an
//! `origiri_richtext_flow::Block`, preserving the inline positions
//! [`super::classify::classify_body`] discards by flattening every item
//! into its own stacked row (see that module's doc comment, and the test
//! named `an_inline_merge_conflict_is_recognized_through_the_real_pipeline`
//! for the concrete case this exists to fix). Reuses `classify.rs`'s own
//! [`look_up`]/[`fallback_label`] so a recognized element gets the same
//! label/tone in both the flowing and the stacked presentation -- adding a
//! vocabulary entry never needs touching two places.
//!
//! A standalone `@conflict` marker between paragraphs is not flowed here:
//! inline flow only matters where an element sits *inside* running prose,
//! and `classify_body`'s stacked `RenderedBlockView` path already renders
//! a standalone block-level one correctly (with both sides, however long,
//! laid out as their own rows -- fine there since there's no running
//! sentence to break). An *inline* `@conflict` is different: it only
//! ever reaches here from `tomet::merge::merge`'s character-level text
//! diff (a structural mismatch -- a whole paragraph, an element -- always
//! narrows to a *block*-level `@conflict` instead, never an inline one),
//! so its two sides are always short runs, safe to expand right inline
//! rather than hiding behind a tap target (see [`push_conflict_side`]).
//!
//! A `Heading` group *is* flowed exactly like a `Paragraph` when it has
//! more than one item (a heading can carry a link or a bold word same as
//! any other prose -- see `immermemo_render_tomet`'s module doc) --
//! `RenderBlockKind::Element` is the only kind that never does, since it
//! always holds exactly one item.
//!
//! [`to_flow_paragraph`] delegates every item's look entirely to
//! [`super::classify::to_classified_block`] rather than re-deriving it --
//! the only thing specific to flowing is *where the result goes*
//! (`Inline::Text` directly, or a [`ClassifiedBlock`] parked in
//! `elements` and referenced by `Inline::Element { id }`), not *what it
//! looks like*.

use immermemo_render_tomet::{RenderBlockKind, RenderItem, TextStyle, classify_blocks};
use origiri_richtext_flow::{Block, Inline};

use super::classify::{
    BlockShape, ClassifiedBlock, ConflictLeafBlock, Tone, classify_body, image_label,
    to_classified_block,
};

/// `render-tomet`'s [`TextStyle`] (bold/italic/mark/strikeout -- the same
/// four flags, just a separate type since `render-tomet` has no
/// `origiri-richtext-flow` dependency to share one with) packed into the
/// `origiri_richtext_flow::TextStyle` convention `FlowView` decodes.
fn to_richtext_style(style: TextStyle) -> origiri_richtext_flow::TextStyle {
    origiri_richtext_flow::TextStyle {
        bold: style.bold,
        italic: style.italic,
        mark: style.mark,
        strikeout: style.strikeout,
    }
}

/// One item of the note body's view-mode display, in source order: either
/// a classified block ([`ClassifiedBlock`], one shape per whole block --
/// what every block rendered as before this module existed) or a
/// paragraph with at least one inline element, kept together so it can
/// flow as one line instead of stacking each split fragment as its own
/// row (see the module doc).
#[derive(Debug, Clone, PartialEq)]
pub struct NoteBodyItem {
    pub depth: usize,
    pub prefix: String,
    pub heading_level: usize,
    pub source_span: Option<tomet::ast::Span>,
    pub content: NoteBodyContent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NoteBodyContent {
    Stacked(ClassifiedBlock),
    Flowed(FlowParagraph),
}

/// The single source of truth for the view-mode display: every top-level
/// block, in order, each already decided as [`NoteBodyContent::Stacked`] or
/// [`NoteBodyContent::Flowed`]. Supersedes calling [`classify_body`] directly
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
            .map(|block| NoteBodyItem {
                depth: 0,
                prefix: String::new(),
                heading_level: 0,
                source_span: None,
                content: NoteBodyContent::Stacked(block),
            })
            .collect();
    };
    groups
        .into_iter()
        // A group with no items is a heading with no title (`=[ ]`, or a bare
        // `=` line): there is nothing in it to show.
        .filter(|(_, items)| !items.is_empty())
        .map(|(kind, items)| {
            let (depth, prefix, heading_level) = match kind {
                RenderBlockKind::ListItem {
                    depth,
                    ordered,
                    index,
                } => {
                    let is_task = items.len() == 1
                        && matches!(&items[0], RenderItem::Element(el) if el.identity.name == "task");
                    let prefix = if is_task {
                        String::new()
                    } else if ordered {
                        format!("{index}.")
                    } else {
                        "•".to_string()
                    };
                    (depth, prefix, 0)
                }
                RenderBlockKind::Heading(level) => (0, String::new(), level),
                _ => (0, String::new(), 0),
            };

            let source_span = if let (Some(first), Some(last)) = (items.first(), items.last()) {
                let start = match first {
                    RenderItem::Text(s, _) => s.start,
                    RenderItem::Element(el) => el.span.start,
                };
                let end = match last {
                    RenderItem::Text(s, _) => s.end,
                    RenderItem::Element(el) => el.span.end,
                };
                Some(tomet::ast::Span::new(start, end))
            } else {
                None
            };

            let content = if items.len() > 1 {
                NoteBodyContent::Flowed(to_flow_paragraph(body, items))
            } else {
                let item = items
                    .into_iter()
                    .next()
                    .expect("groups without items were filtered out above");
                NoteBodyContent::Stacked(to_classified_block(body, item))
            };

            NoteBodyItem {
                depth,
                prefix,
                heading_level,
                source_span,
                content,
            }
        })
        .collect()
}

/// One paragraph's flowed content, plus the element payloads its
/// `Inline::Element { id }` entries index into. `id` is only an index into
/// `elements` -- see `origiri_richtext_flow::model::ElementId`'s doc for
/// why this crate, not `origiri-richtext-flow`, owns what an id means.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowParagraph {
    pub block: Block,
    pub elements: Vec<ClassifiedBlock>,
}

/// Flows every paragraph or heading in `body` that contains at least one
/// inline element. A paragraph/heading with none, and every `Element`
/// block, is left for [`super::classify::classify_body`] to render as
/// today -- only a group actually split by an inline marker needs the
/// flow engine (see [`note_body_items`]'s doc for why `items.len() > 1`
/// alone is enough to tell those apart).
pub fn flow_paragraphs(body: &str) -> Vec<FlowParagraph> {
    let Ok(groups) = classify_blocks(body) else {
        return Vec::new();
    };
    groups
        .into_iter()
        .filter(|(_kind, items)| items.len() > 1)
        .map(|(_, items)| to_flow_paragraph(body, items))
        .collect()
}

fn to_flow_paragraph(body: &str, items: Vec<RenderItem>) -> FlowParagraph {
    let mut content = Vec::with_capacity(items.len());
    let mut elements = Vec::new();
    for item in items {
        match item {
            RenderItem::Text(span, style) => content.push(Inline::Text {
                content: body[span.start..span.end].to_string(),
                style: to_richtext_style(style).to_style_id(),
            }),
            item @ RenderItem::Element(_) => {
                let classified = to_classified_block(body, item);
                if classified.shape == BlockShape::Conflict {
                    push_conflict_side(
                        &mut content,
                        &mut elements,
                        classified.side_a,
                        Tone::Accent,
                    );
                    content.push(Inline::Text {
                        content: " / ".to_string(),
                        style: to_richtext_style(TextStyle::default()).to_style_id(),
                    });
                    push_conflict_side(
                        &mut content,
                        &mut elements,
                        classified.side_b,
                        Tone::Warning,
                    );
                } else {
                    let id = elements.len() as u64;
                    elements.push(without_picture(classified));
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

/// An image in the middle of a line has no room to draw its picture: it
/// stands as a labeled pill, like any other element nobody has a look for.
fn without_picture(mut block: ClassifiedBlock) -> ClassifiedBlock {
    if block.shape == BlockShape::Image {
        block.text = image_label(&block.text, &block.target);
        block.target = String::new();
        block.shape = BlockShape::Badge;
    }
    block
}

/// One side of an inline `@conflict`, flattened into
/// `elements`/`content` the same way every other item in
/// [`to_flow_paragraph`] is -- but every leaf becomes its own
/// `Inline::Element`, even a plain-text one (which elsewhere in this
/// function flows as bare `Inline::Text`): `Inline::Text` carries no
/// color of its own (only bold/italic/mark/strikeout, see
/// [`to_richtext_style`]), and `side_tone` is the whole point here -- the
/// two sides need to read as visually distinct runs. `side_tone`
/// overrides a plain-text leaf's own (always `Tone::Neutral`) tone with
/// the same accent/warning colors the retired `@mobile.conflict` scheme
/// used for its "Mine"/"Theirs" chips, without reintroducing a
/// mine/theirs *label* (`a`/`b` are `@conflict`'s structural names and
/// carry no such meaning). A non-plain-text leaf (rare: a nested element inside a
/// conflicting run) keeps its own tone -- overriding, say, a nested
/// link's accent color to match its side would fight the look that
/// element already has everywhere else it appears.
fn push_conflict_side(
    content: &mut Vec<Inline>,
    elements: &mut Vec<ClassifiedBlock>,
    side: Vec<ConflictLeafBlock>,
    side_tone: Tone,
) {
    for leaf in side {
        let tone = if leaf.shape == BlockShape::PlainText {
            side_tone
        } else {
            leaf.tone
        };
        let id = elements.len() as u64;
        elements.push(without_picture(ClassifiedBlock {
            shape: leaf.shape,
            tone,
            text: leaf.text,
            target: leaf.target,
            style: leaf.style,
            reading: leaf.reading,
            checked: leaf.checked,
            side_a: Vec::new(),
            side_b: Vec::new(),
        }));
        content.push(Inline::Element { id });
    }
}

#[cfg(test)]
mod tests {
    use super::super::classify::Tone;
    use super::*;

    #[test]
    fn an_image_in_the_middle_of_a_line_flows_as_a_labeled_pill() {
        let paragraphs = flow_paragraphs("See @embed[A cat](path: \"img/cat.png\") here.\n");
        assert_eq!(paragraphs.len(), 1);
        let element = &paragraphs[0].elements[0];
        assert_eq!(element.shape, BlockShape::Badge);
        assert_eq!(element.text, "A cat");
        assert_eq!(element.target, "");

        let paragraphs = flow_paragraphs("See @embed(path: \"img/cat.png\") here.\n");
        assert_eq!(paragraphs[0].elements[0].text, "cat.png");
    }

    /// A heading with no title -- `====[  ]`, or a bare `=` line -- has no
    /// items; it used to panic, and a note with one could not be opened.
    #[test]
    fn a_heading_with_no_title_is_skipped_instead_of_panicking() {
        let src = "Before.\n\n====[  ]\n\nAfter.\n\n=\n";
        let items = note_body_items(src);
        let texts: Vec<String> = items
            .iter()
            .filter_map(|item| match &item.content {
                NoteBodyContent::Stacked(block) => Some(block.text.clone()),
                NoteBodyContent::Flowed(_) => None,
            })
            .collect();
        assert_eq!(texts, ["Before.", "After."]);
    }

    #[test]
    fn note_body_items_mixes_stacked_and_flowed_items_in_source_order() {
        let src = "Before.\n\n\
                   @conflict(a: [Mine.], b: [Theirs.])\n\n\
                   The @conflict(a: [slow], b: [lazy]) fox jumps.\n";
        let items = note_body_items(src);
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0].content, NoteBodyContent::Stacked(_)));
        assert!(matches!(items[1].content, NoteBodyContent::Stacked(_)));
        match &items[2].content {
            NoteBodyContent::Flowed(paragraph) => {
                // "The ", side_a's "slow", " / ", side_b's "lazy",
                // " fox jumps." -- see `push_conflict_side`'s doc.
                assert_eq!(paragraph.block.content.len(), 5);
                assert_eq!(paragraph.elements.len(), 2);
            }
            NoteBodyContent::Stacked(_) => panic!("expected the inline-split paragraph to flow"),
        }
    }

    #[test]
    fn note_body_items_populates_list_items_with_depth_and_prefix() {
        let src = "- Bullet 1\n  - Nested\n-. Ordered\n-@task(x) Done\n";
        let items = note_body_items(src);
        assert_eq!(items.len(), 4);

        assert_eq!(items[0].depth, 0);
        assert_eq!(items[0].prefix, "•");

        assert_eq!(items[1].depth, 1);
        assert_eq!(items[1].prefix, "•");

        assert_eq!(items[2].depth, 0);
        assert_eq!(items[2].prefix, "1.");

        assert_eq!(items[3].depth, 0);
        assert_eq!(items[3].prefix, "");
        if let NoteBodyContent::Stacked(block) = &items[3].content {
            assert_eq!(block.shape, BlockShape::Task);
            assert!(block.checked);
        } else {
            panic!("expected task to be stacked");
        }
    }

    #[test]
    fn a_plain_paragraph_has_no_flow_paragraphs() {
        // Only one `RenderItem` (no inline split) -- nothing for the flow
        // engine to do that `classify_body` doesn't already render fine.
        assert!(flow_paragraphs("Just a paragraph.\n").is_empty());
    }

    #[test]
    fn an_inline_conflict_flows_both_sides_with_distinct_tones() {
        let src = "The @conflict(a: [slow], b: [lazy]) fox jumps.\n";
        let paragraphs = flow_paragraphs(src);
        assert_eq!(paragraphs.len(), 1);

        let paragraph = &paragraphs[0];
        assert_eq!(paragraph.block.kind, "paragraph");
        // "The ", side_a's "slow", " / ", side_b's "lazy", " fox jumps.".
        assert_eq!(paragraph.block.content.len(), 5);
        assert_eq!(paragraph.elements.len(), 2);

        match &paragraph.block.content[0] {
            Inline::Text { content, .. } => assert_eq!(content, "The "),
            Inline::Element { .. } => panic!("expected text"),
        }
        match &paragraph.block.content[1] {
            Inline::Element { id } => {
                assert_eq!(paragraph.elements[*id as usize].text, "slow");
                assert_eq!(
                    paragraph.elements[*id as usize].shape,
                    BlockShape::PlainText
                );
                assert_eq!(paragraph.elements[*id as usize].tone, Tone::Accent);
            }
            Inline::Text { .. } => panic!("expected an element"),
        }
        match &paragraph.block.content[2] {
            Inline::Text { content, .. } => assert_eq!(content, " / "),
            Inline::Element { .. } => panic!("expected text"),
        }
        match &paragraph.block.content[3] {
            Inline::Element { id } => {
                assert_eq!(paragraph.elements[*id as usize].text, "lazy");
                assert_eq!(
                    paragraph.elements[*id as usize].shape,
                    BlockShape::PlainText
                );
                assert_eq!(paragraph.elements[*id as usize].tone, Tone::Warning);
            }
            Inline::Text { .. } => panic!("expected an element"),
        }
        match &paragraph.block.content[4] {
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

    #[test]
    fn an_inline_strong_run_carries_a_real_style_id_not_zero() {
        let paragraphs = flow_paragraphs("plain @strong[bold] plain\n");
        assert_eq!(paragraphs.len(), 1);
        let Inline::Text { style, .. } = paragraphs[0].block.content[1] else {
            panic!("expected the bold run at index 1");
        };
        assert_eq!(
            style,
            origiri_richtext_flow::TextStyle {
                bold: true,
                ..Default::default()
            }
            .to_style_id()
        );
    }

    #[test]
    fn an_inline_ruby_element_flows_as_an_atomic_element() {
        let src = "plain @ruby[漢字](rt:\"かんじ\") plain\n";
        let paragraphs = flow_paragraphs(src);
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].elements.len(), 1);
        assert_eq!(paragraphs[0].elements[0].shape, BlockShape::Ruby);
        assert_eq!(paragraphs[0].elements[0].text, "漢字");
        assert_eq!(paragraphs[0].elements[0].reading, "かんじ");
    }

    #[test]
    fn an_inline_link_element_flows_as_an_atomic_element() {
        let src = "plain @link(\"https://example.com\")[see here] plain\n";
        let paragraphs = flow_paragraphs(src);
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].elements.len(), 1);
        assert_eq!(paragraphs[0].elements[0].shape, BlockShape::Link);
        assert_eq!(paragraphs[0].elements[0].tone, Tone::Accent);
        assert_eq!(paragraphs[0].elements[0].text, "see here");
    }

    // Regression test for the item-dropping bug: before this fix, only
    // `RenderBlockKind::Paragraph` groups with >1 item were flowed, so a
    // `Heading` group split by an inline element (here, `@strong`) fell
    // into `note_body_items`'s stacked branch and silently lost every
    // item after the first via `items.into_iter().next()`.
    #[test]
    fn a_heading_split_by_an_inline_element_flows_instead_of_dropping_items() {
        let src = "= A @strong[bold] heading\n\nBody.\n";
        let paragraphs = flow_paragraphs(src);
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].block.content.len(), 3);
        match &paragraphs[0].block.content[2] {
            Inline::Text { content, .. } => assert_eq!(content, " heading"),
            Inline::Element { .. } => panic!("expected the trailing text run"),
        }

        let items = note_body_items(src);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].content, NoteBodyContent::Flowed(_)));
    }

    #[test]
    fn an_inline_conflict_at_paragraph_start_with_trailing_text_flows() {
        let src = "@conflict(a: [うんこ], b: [うんち])あいうえお\n";
        let paragraphs = flow_paragraphs(src);
        assert_eq!(paragraphs.len(), 1);
        let p = &paragraphs[0];
        // side_a, " / ", side_b, "あいうえお"
        assert_eq!(p.block.content.len(), 4);
        assert_eq!(p.elements.len(), 2);
        assert_eq!(p.elements[0].text, "うんこ");
        assert_eq!(p.elements[0].tone, Tone::Accent);
        assert_eq!(p.elements[1].text, "うんち");
        assert_eq!(p.elements[1].tone, Tone::Warning);
        match &p.block.content[1] {
            Inline::Text { content, .. } => assert_eq!(content, " / "),
            Inline::Element { .. } => panic!("expected separator text"),
        }
        match &p.block.content[3] {
            Inline::Text { content, .. } => assert_eq!(content, "あいうえお"),
            Inline::Element { .. } => panic!("expected trailing text"),
        }
    }

    #[test]
    fn note_body_items_omits_metadata_elements() {
        let src = "@kind(note)\n\
                   @config(export: { type: commonmark })\n\
                   @meta{ id: 123, author: Alice }\n\n\
                   Body paragraph.\n";
        let items = note_body_items(src);
        assert_eq!(items.len(), 1);
        match &items[0].content {
            NoteBodyContent::Stacked(block) => {
                assert_eq!(block.shape, BlockShape::PlainText);
                assert_eq!(block.text, "Body paragraph.");
            }
            _ => panic!("expected stacked plain text"),
        }
    }

    #[test]
    fn headings_populate_heading_level_and_source_span() {
        let src = "= Main Title\n\n== Section 1\n\n=== Subsection\n\nPlain text.\n";
        let items = note_body_items(src);
        assert_eq!(items.len(), 4);

        assert_eq!(items[0].heading_level, 1);
        assert_eq!(items[0].depth, 0);
        assert!(items[0].source_span.is_some());

        assert_eq!(items[1].heading_level, 2);
        assert_eq!(items[1].depth, 0);
        assert!(items[1].source_span.is_some());

        assert_eq!(items[2].heading_level, 3);
        assert_eq!(items[2].depth, 0);
        assert!(items[2].source_span.is_some());

        assert_eq!(items[3].heading_level, 0);
        assert_eq!(items[3].depth, 0);
        assert!(items[3].source_span.is_some());
    }
}
