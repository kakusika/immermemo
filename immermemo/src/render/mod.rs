//! Converts [`flow::note_body_items`]'s plain-data items into the
//! Slint-generated `NoteBodyItemView` model `ui/screens/editor.slint`'s
//! view-mode display draws (see `ui/screens/note_body.slint`'s
//! `NoteBodyView`). The classification decision -- including which
//! identity looks like what -- lives in [`classify`] (its own `REGISTRY`),
//! which knows nothing about Slint; this module is just the seam between
//! that and the generated Slint types. `classify`/`flow` are folded into
//! this crate (not a separate shared one) because nothing else needs
//! them: the common "add a new `@xxx`'s look" change always touches both
//! this seam and `classify`'s `REGISTRY` together, so a crate boundary
//! between them buys nothing for that workflow, only for a rarer
//! logic-only refactor of `classify`/`flow` alone -- see this
//! repository's history around the `apps/slint` -> `immermemo` move for
//! the fuller reasoning. This module itself has to live here regardless:
//! `NoteBodyItemView`/`RenderedBlock`/etc. are only real Rust types inside
//! whichever crate's `slint::include_modules!()` call compiled a tree
//! that imports them, and this crate is the only one that does
//! (`ui/screens/` is source checked directly into it, same as every
//! other screen).
//!
//! A [`flow::NoteBodyItem::Flowed`] paragraph's content goes through
//! `origami_richtext_flow::layout_block` and comes back out as
//! `origami-richtext`'s generic `RichTextFragment`s; an element fragment's
//! widget is a `RenderedBlockView` instantiated at runtime via
//! `slint::ComponentFactory` -- the same component a stacked
//! [`classify::ClassifiedBlock`] renders as its own row, just embedded
//! inline instead.
//!
//! `#![allow(deprecated)]`: `ComponentFactory` is `slint`'s own escape
//! hatch for this (`ComponentContainer`'s `component-factory` property),
//! but the crate marks it `#[deprecated(note = "Experimental type was made
//! public by mistake")]` and `#[doc(hidden)]`, and `build.rs` has to opt
//! the compiler into treating `ComponentContainer`/`component-factory` as
//! real `.slint` syntax at all (`SLINT_ENABLE_EXPERIMENTAL_FEATURES`,
//! since Slint's normal compiler path removes both). Known, accepted
//! risk, not an oversight.
#![allow(deprecated)]

pub mod classify;
pub mod flow;

use classify::{BlockShape, ClassifiedBlock, Tone};
use flow::{FlowParagraph, NoteBodyItem, note_body_items as classify_note_body_items};
use origami_richtext_flow::{Fragment, Measure, layout_block};
use slint::{ComponentFactory, ModelRc, VecModel};

use crate::{
    App, FlowElementWidget, NoteBodyItemView, RenderedBlock, RenderedBlockShape, RenderedBlockTone,
    RichTextFragment, RichTextLine,
};

/// Classifies/flows `body` and converts the result into the model
/// `App`/`EditorScreen`'s `note-body-items` property expects, wrapping
/// each [`NoteBodyItem::Flowed`] paragraph against `max_width` (logical
/// pixels -- `App`'s own `body-content-width`, see `editor.slint`'s doc
/// comment on it). Callers should set the result alongside `body` every
/// time `body` *or* `max_width` changes -- see `session::update_rendered_body`,
/// `lib.rs`'s `on_edited` handler, and `lib.rs`'s `reflow-note-body`
/// handler, the three places that happens.
pub fn note_body_items(body: &str, app: &App, max_width: f32) -> ModelRc<NoteBodyItemView> {
    let items: Vec<NoteBodyItemView> = classify_note_body_items(body)
        .into_iter()
        .map(|item| to_note_body_item_view(item, app, max_width))
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn to_note_body_item_view(item: NoteBodyItem, app: &App, max_width: f32) -> NoteBodyItemView {
    match item {
        NoteBodyItem::Stacked(block) => NoteBodyItemView {
            is_flow: false,
            block: to_rendered_block(&block),
            lines: ModelRc::default(),
        },
        NoteBodyItem::Flowed(paragraph) => NoteBodyItemView {
            is_flow: true,
            block: RenderedBlock::default(),
            lines: to_rich_text_lines(&paragraph, app, max_width),
        },
    }
}

/// Measures against `app`'s hidden probe elements (`app.slint`'s
/// `measure-probe-text-item`/`measure-probe-block-item`): set a probe
/// property, read the matching `preferred-width`-derived property right
/// back, synchronously. Same technique `screens/editor.slint`'s
/// `LineNumberGutter` uses for line-height, just parameterized via
/// properties instead of a fixed probe string. `font_size` and `elements`
/// are fixed for the lifetime of one `layout_block` call (one paragraph,
/// one font size), so they're captured once at construction rather than
/// threaded through every `Measure` call.
struct RealMeasure<'a> {
    app: &'a App,
    font_size: f32,
    elements: &'a [ClassifiedBlock],
}

impl Measure for RealMeasure<'_> {
    fn text_width(&self, content: &str, style: u32) -> f32 {
        let style = origami_richtext_flow::TextStyle::from_style_id(style);
        self.app.set_measure_probe_text(content.into());
        self.app.set_measure_probe_font_size(self.font_size);
        self.app
            .set_measure_probe_font_weight(if style.bold { 700 } else { 400 });
        self.app.set_measure_probe_font_italic(style.italic);
        self.app.get_measure_probe_text_width()
    }

    fn element_width(&self, id: u64) -> f32 {
        let rendered = to_rendered_block(&self.elements[id as usize]);
        self.app.set_measure_probe_block(rendered);
        self.app.get_measure_probe_block_width()
    }
}

fn to_rich_text_lines(
    paragraph: &FlowParagraph,
    app: &App,
    max_width: f32,
) -> ModelRc<RichTextLine> {
    let measure = RealMeasure {
        app,
        font_size: app.get_editor_font_size(),
        elements: &paragraph.elements,
    };
    let lines: Vec<RichTextLine> = layout_block(&paragraph.block, max_width, &measure)
        .into_iter()
        .map(|line| RichTextLine {
            fragments: ModelRc::new(VecModel::from(
                line.fragments
                    .into_iter()
                    .map(|fragment| to_rich_text_fragment(fragment, &paragraph.elements))
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    ModelRc::new(VecModel::from(lines))
}

fn to_rich_text_fragment(fragment: Fragment, elements: &[ClassifiedBlock]) -> RichTextFragment {
    match fragment {
        Fragment::Text { content, style, .. } => {
            let style = origami_richtext_flow::TextStyle::from_style_id(style);
            RichTextFragment {
                is_element: false,
                text: content.into(),
                factory: ComponentFactory::default(),
                bold: style.bold,
                italic: style.italic,
                mark: style.mark,
                strikeout: style.strikeout,
            }
        }
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
                bold: false,
                italic: false,
                mark: false,
                strikeout: false,
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
            BlockShape::Ruby => RenderedBlockShape::Ruby,
            BlockShape::Link => RenderedBlockShape::Link,
            BlockShape::Icon => RenderedBlockShape::Icon,
        },
        tone: match block.tone {
            Tone::Accent => RenderedBlockTone::Accent,
            Tone::Warning => RenderedBlockTone::Warning,
            Tone::Neutral => RenderedBlockTone::Neutral,
        },
        text: block.text.clone().into(),
        bold: block.style.bold,
        italic: block.style.italic,
        mark: block.style.mark,
        strikeout: block.style.strikeout,
        reading: block.reading.clone().into(),
        icon_image: if block.shape == BlockShape::Icon {
            icon_image(&block.text, &block.reading)
        } else {
            slint::Image::default()
        },
    }
}

/// `pkg`'s `slug` icon as a Slint image, or a blank one if `origami_icons`
/// doesn't vendor that pair (an unknown/typo'd icon name -- nothing in
/// `tomet-semantics` validates `@doc.icon`'s `name` against what's actually
/// vendored, so this has to degrade quietly rather than panic).
fn icon_image(slug: &str, pkg: &str) -> slint::Image {
    origami_icons::icon_svg(pkg, slug)
        .and_then(|svg| slint::Image::load_from_svg_data(&svg).ok())
        .unwrap_or_default()
}

// The substantial classification-decision tests (conflict markers,
// unrecognized elements, the real merge pipeline, inline flow grouping)
// live in `classify`/`flow`'s own tests now. These just check the seam:
// that converting into the generated model actually round-trips
// shape/tone/text, and that a flowed paragraph produces lines with the
// right fragment count.
//
// Plain functions, not `#[test]`s: winit only allows one event loop per
// process, and `cargo test` runs every `#[test]` fn in a package's `lib`
// target in one process (by default, concurrently), so a second
// independent `App::new()` call anywhere in this crate's test binary
// fails with "EventLoop can't be recreated" the moment it races the
// first. `lib.rs`'s `tests::ui_helpers_and_conflict_resolution` already
// owns the one `App::new()` this whole binary gets; it calls these.
#[cfg(test)]
pub(crate) mod seam_tests {
    use super::*;
    use slint::Model;

    pub(crate) fn stacked_items_convert_into_the_generated_model(app: &App) {
        let model = note_body_items("Before.\n\n@mobile.conflict(mine)\n\nMine.\n", app, 1000.0);
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

    /// `icon_image` is the one field `to_rendered_block` computes instead
    /// of just converting -- the only thing this needs to prove beyond
    /// `classify`'s own `icon_look` tests (which stop at the plain-data
    /// `name`/`pkg`) is that a *vendored* pair actually resolves to a
    /// non-empty image, and an unvendored one degrades to an empty one
    /// instead of panicking.
    pub(crate) fn a_doc_icon_resolves_to_a_real_image_and_an_unknown_one_to_empty(app: &App) {
        let model = note_body_items("@doc.icon(\"star\")\n", app, 1000.0);
        assert_eq!(model.row_count(), 1);
        let item = model.row_data(0).unwrap();
        assert_eq!(item.block.shape, RenderedBlockShape::Icon);
        assert!(item.block.icon_image.size().width > 0);

        let model = note_body_items("@doc.icon(\"not-a-real-icon-slug\")\n", app, 1000.0);
        assert_eq!(model.row_count(), 1);
        let item = model.row_data(0).unwrap();
        assert_eq!(item.block.icon_image.size().width, 0);
    }

    pub(crate) fn a_wide_inline_conflict_paragraph_flows_onto_one_line(app: &App) {
        let src = "The @mobile.conflict(mine)slow@mobile.conflict(theirs)lazy@mobile.conflict(end) fox jumps.\n";
        let model = note_body_items(src, app, 10_000.0);
        assert_eq!(model.row_count(), 1);
        let item = model.row_data(0).unwrap();
        assert!(item.is_flow);
        assert_eq!(item.lines.row_count(), 1);
        let line = item.lines.row_data(0).unwrap();
        // "The " / Mine-chip / "slow" / Theirs-chip / "lazy" / " " /
        // "fox " / "jumps." -- 8, not the paragraph's own 6 `Inline`s: the
        // "end" divider still renders as nothing inline (see
        // `flow::to_flow_paragraph`'s doc comment), but
        // `layout_block` additionally splits each text run into its own
        // word tokens (`origami_richtext_flow::layout::words`), and
        // " fox jumps." is 3 words, including its own leading space (not
        // dropped here -- that only happens at an actual line start, and
        // this whole paragraph fits on one line).
        assert_eq!(line.fragments.row_count(), 8);
        assert!(!line.fragments.row_data(0).unwrap().is_element);
        assert!(line.fragments.row_data(1).unwrap().is_element);
        assert!(line.fragments.row_data(3).unwrap().is_element);
    }

    pub(crate) fn a_narrow_width_wraps_an_inline_conflict_paragraph_onto_several_lines(app: &App) {
        let src = "The @mobile.conflict(mine)slow@mobile.conflict(theirs)lazy@mobile.conflict(end) fox jumps.\n";
        // Real font metrics, not a fake fixed-width measure -- too narrow
        // for the whole sentence (and its two chips) to fit on one line.
        let model = note_body_items(src, app, 80.0);
        let item = model.row_data(0).unwrap();
        assert!(item.is_flow);
        assert!(
            item.lines.row_count() > 1,
            "expected a narrow width to wrap onto more than one line, got {}",
            item.lines.row_count()
        );
    }
}
