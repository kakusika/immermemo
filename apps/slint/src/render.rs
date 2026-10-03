//! Converts `immermemo_editor::note_body_items`'s plain-data items into the
//! Slint-generated `NoteBodyItemView` model `ui/screens/editor.slint`'s
//! view-mode display draws (see `ui/screens/note_body.slint`'s
//! `NoteBodyView`). The classification decision -- including which
//! identity looks like what -- lives in `immermemo-editor`
//! (`crates/editor/src/classify.rs`'s `REGISTRY`), which knows nothing
//! about Slint; this module is just the seam between that and the
//! generated Slint types. It has to live in `apps/slint` because those
//! types are only real Rust types inside whichever crate's
//! `slint::include_modules!()` call compiled a tree that imports them, and
//! `apps/slint` is the only crate that does so (`ui/screens/` is source
//! checked directly into this crate, same as every other screen -- see
//! `.agents/tasks/fold-editor-slint-into-apps-slint.md` for why
//! `crates/editor-slint` no longer exists as a separate crate).
//!
//! A [`NoteBodyItem::Flowed`] paragraph's content goes through
//! `origami_richtext_flow::layout_block` and comes back out as
//! `origami-richtext`'s generic `RichTextFragment`s; an element fragment's
//! widget is a `RenderedBlockView` instantiated at runtime via
//! `slint::ComponentFactory` -- the same component a stacked
//! [`ClassifiedBlock`] renders as its own row, just embedded inline
//! instead (see `.agents/tasks/richtext-flow-inline-conflict.md`).
//!
//! `#![allow(deprecated)]`: `ComponentFactory` is `slint`'s own escape
//! hatch for this (`ComponentContainer`'s `component-factory` property),
//! but the crate marks it `#[deprecated(note = "Experimental type was made
//! public by mistake")]` and `#[doc(hidden)]`, and `apps/slint/build.rs`
//! has to opt the compiler into treating `ComponentContainer`/
//! `component-factory` as real `.slint` syntax at all
//! (`SLINT_ENABLE_EXPERIMENTAL_FEATURES`, since Slint's normal compiler
//! path removes both). Known, accepted risk, not an oversight -- see
//! `.agents/tasks/richtext-flow-inline-conflict.md`.
#![allow(deprecated)]

use immermemo_editor::{
    BlockShape, ClassifiedBlock, FlowParagraph, NoteBodyItem, Tone,
    note_body_items as classify_note_body_items,
};
use origami_richtext_flow::{Fragment, Measure, layout_block};
use slint::{ComponentFactory, ModelRc, VecModel};

use crate::{
    FlowElementWidget, NoteBodyItemView, RenderedBlock, RenderedBlockShape, RenderedBlockTone,
    RichTextFragment, RichTextLine,
};

/// Classifies/flows `body` and converts the result into the model
/// `App`/`EditorScreen`'s `note-body-items` property expects. Callers
/// should set this alongside `body` every time `body` changes -- see
/// `session::update_rendered_body` and `lib.rs`'s `on_edited` handler,
/// the two places that happens.
pub fn note_body_items(body: &str) -> ModelRc<NoteBodyItemView> {
    let items: Vec<NoteBodyItemView> = classify_note_body_items(body)
        .into_iter()
        .map(to_note_body_item_view)
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn to_note_body_item_view(item: NoteBodyItem) -> NoteBodyItemView {
    match item {
        NoteBodyItem::Stacked(block) => NoteBodyItemView {
            is_flow: false,
            block: to_rendered_block(&block),
            line: RichTextLine::default(),
        },
        NoteBodyItem::Flowed(paragraph) => NoteBodyItemView {
            is_flow: true,
            block: RenderedBlock::default(),
            line: to_rich_text_line(&paragraph),
        },
    }
}

/// `layout_block`'s wrap decision needs real font-metrics measurement (see
/// `origami-richtext-flow`'s README); `FlowView` positions a resolved
/// line's fragments with its own `HorizontalLayout`, not `Fragment::x`, so
/// nothing downstream actually reads what this reports yet. A real
/// `Measure` only matters once `layout_block` does more than one line.
struct NoMeasure;

impl Measure for NoMeasure {
    fn text_width(&self, _content: &str, _style: u32) -> f32 {
        0.0
    }

    fn element_width(&self, _id: u64) -> f32 {
        0.0
    }
}

fn to_rich_text_line(paragraph: &FlowParagraph) -> RichTextLine {
    let fragments: Vec<RichTextFragment> = layout_block(&paragraph.block, f32::MAX, &NoMeasure)
        .into_iter()
        .next()
        .map(|line| {
            line.fragments
                .into_iter()
                .map(|fragment| to_rich_text_fragment(fragment, &paragraph.elements))
                .collect()
        })
        .unwrap_or_default();
    RichTextLine {
        fragments: ModelRc::new(VecModel::from(fragments)),
    }
}

fn to_rich_text_fragment(fragment: Fragment, elements: &[ClassifiedBlock]) -> RichTextFragment {
    match fragment {
        Fragment::Text { content, .. } => RichTextFragment {
            is_element: false,
            text: content.into(),
            factory: ComponentFactory::default(),
        },
        Fragment::Element { id, .. } => {
            let rendered = to_rendered_block(&elements[id as usize]);
            let factory = ComponentFactory::new(move |_| {
                let widget = FlowElementWidget::new().ok()?;
                widget.set_block(rendered.clone());
                Some(widget)
            });
            RichTextFragment {
                is_element: true,
                text: Default::default(),
                factory,
            }
        }
    }
}

fn to_rendered_block(block: &ClassifiedBlock) -> RenderedBlock {
    RenderedBlock {
        shape: match block.shape {
            BlockShape::PlainText => RenderedBlockShape::PlainText,
            BlockShape::Chip => RenderedBlockShape::Chip,
            BlockShape::Divider => RenderedBlockShape::Divider,
            BlockShape::Badge => RenderedBlockShape::Badge,
        },
        tone: match block.tone {
            Tone::Accent => RenderedBlockTone::Accent,
            Tone::Warning => RenderedBlockTone::Warning,
            Tone::Neutral => RenderedBlockTone::Neutral,
        },
        text: block.text.clone().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    // The substantial classification-decision tests (conflict markers,
    // unrecognized elements, the real merge pipeline, inline flow
    // grouping) live in `immermemo-editor`'s own tests now. This just
    // checks the seam: that converting into the generated model actually
    // round-trips shape/tone/text, and that a flowed paragraph produces a
    // line with the right fragment count.
    #[test]
    fn stacked_items_convert_into_the_generated_model() {
        let model = note_body_items("Before.\n\n@mobile.conflict(mine)\n\nMine.\n");
        assert_eq!(model.row_count(), 3);
        let first = model.row_data(0).unwrap();
        assert!(!first.is_flow);
        assert_eq!(first.block.shape, RenderedBlockShape::PlainText);
        assert_eq!(first.block.text, "Before.");
        let second = model.row_data(1).unwrap();
        assert_eq!(second.block.shape, RenderedBlockShape::Chip);
        assert_eq!(second.block.tone, RenderedBlockTone::Accent);
        assert_eq!(second.block.text, "Mine");
    }

    #[test]
    fn an_inline_conflict_paragraph_flows_with_text_and_element_fragments() {
        let src = "The @mobile.conflict(mine)slow@mobile.conflict(theirs)lazy@mobile.conflict(end) fox jumps.\n";
        let model = note_body_items(src);
        assert_eq!(model.row_count(), 1);
        let item = model.row_data(0).unwrap();
        assert!(item.is_flow);
        // 6, not 7: the "end" divider renders as nothing inline (see
        // `immermemo_editor::flow::to_flow_paragraph`'s doc comment).
        assert_eq!(item.line.fragments.row_count(), 6);
        assert!(!item.line.fragments.row_data(0).unwrap().is_element);
        assert!(item.line.fragments.row_data(1).unwrap().is_element);
    }
}
