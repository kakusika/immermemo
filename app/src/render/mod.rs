//! Converts [`flow::note_body_items`]'s plain-data items into the
//! Slint-generated `NoteBodyItemView` model `ui/pages/editor/editor.slint`'s
//! view-mode display draws (see `ui/render/note_body.slint`'s
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
//! (`ui/` is source checked directly into it, same as every
//! other page).
//!
//! A [`flow::NoteBodyItem::Flowed`] paragraph's content goes through
//! `origiri_richtext_flow::layout_block` and comes back out as
//! `RichTextFragment`s; an element fragment directly carries its
//! `RenderedBlock` data into `flow_view.slint`'s `RenderedBlockView`.

pub mod classify;
pub mod flow;

use classify::{BlockShape, ClassifiedBlock, ConflictLeafBlock, Tone};
pub use flow::note_body_items as classify_note_body_items;
use flow::{FlowParagraph, NoteBodyContent, NoteBodyItem};
use origiri_richtext_flow::{Fragment, Measure, layout_block};
use slint::{ModelRc, VecModel};

use crate::{
    App, ConflictSideBlock, NoteBodyItemView, RenderedBlock, RenderedBlockShape, RenderedBlockTone,
    RichTextFragment, RichTextLine,
};

/// Classifies/flows `body` and converts the result into the model
/// `App`/`EditorScreen`'s `note-body-items` property expects, wrapping
/// each [`NoteBodyItem`] paragraph against `max_width` (logical
/// pixels -- `App`'s own `body-content-width`, see `editor.slint`'s doc
/// comment on it). Callers should set the result alongside `body` every
/// time `body` *or* `max_width` changes -- see `session::update_rendered_body`,
/// `callbacks::note`'s `on_edited` and `on_reflow_note_body` handlers, the
/// three places that happens.
pub fn note_body_items(body: &str, app: &App, max_width: f32) -> ModelRc<NoteBodyItemView> {
    let items: Vec<NoteBodyItemView> = classify_note_body_items(body)
        .into_iter()
        .map(|item| to_note_body_item_view(item, app, max_width))
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn heading_font_size(base_size: f32, level: usize) -> f32 {
    match level {
        1 => base_size * 1.5,
        2 => base_size * 1.3,
        3 => base_size * 1.15,
        _ if level > 3 => base_size * 1.05,
        _ => base_size,
    }
}

fn to_note_body_item_view(item: NoteBodyItem, app: &App, max_width: f32) -> NoteBodyItemView {
    let indent_px = item.depth as f32 * 16.0;
    let prefix_px = if !item.prefix.is_empty() { 20.0 } else { 0.0 };
    let flow_max_width = (max_width - indent_px - prefix_px).max(40.0);
    let heading_level = item.heading_level as i32;

    match item.content {
        NoteBodyContent::Stacked(block) => NoteBodyItemView {
            is_flow: false,
            heading_level,
            depth: item.depth as i32,
            prefix: item.prefix.into(),
            block: to_rendered_block(&block),
            lines: ModelRc::default(),
        },
        NoteBodyContent::Flowed(paragraph) => NoteBodyItemView {
            is_flow: true,
            heading_level,
            depth: item.depth as i32,
            prefix: item.prefix.into(),
            block: RenderedBlock::default(),
            lines: to_rich_text_lines(&paragraph, app, flow_max_width, item.heading_level),
        },
    }
}

/// Measures against `app`'s hidden probe elements (`app.slint`'s
/// `measure-probe-text-item`/`measure-probe-block-item`): set a probe
/// property, read the matching `preferred-width`-derived property right
/// back, synchronously. Same technique `pages/editor/editor.slint`'s
/// `LineNumberGutter` uses for line-height, just parameterized via
/// properties instead of a fixed probe string. `font_size` and `elements`
/// are fixed for the lifetime of one `layout_block` call (one paragraph,
/// one font size), so they're captured once at construction rather than
/// threaded through every `Measure` call.
struct RealMeasure<'a> {
    app: &'a App,
    font_size: f32,
    is_heading: bool,
    elements: &'a [ClassifiedBlock],
}

impl Measure for RealMeasure<'_> {
    fn text_width(&self, content: &str, style: u32) -> f32 {
        let style = origiri_richtext_flow::TextStyle::from_style_id(style);
        self.app.set_measure_probe_text(content.into());
        self.app.set_measure_probe_font_size(self.font_size);
        self.app
            .set_measure_probe_font_weight(if self.is_heading || style.bold {
                700
            } else {
                400
            });
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
    heading_level: usize,
) -> ModelRc<RichTextLine> {
    let font_size = heading_font_size(app.get_editor_font_size(), heading_level);
    let measure = RealMeasure {
        app,
        font_size,
        is_heading: heading_level > 0,
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
            let style = origiri_richtext_flow::TextStyle::from_style_id(style);
            RichTextFragment {
                is_element: false,
                text: content.into(),
                block: RenderedBlock::default(),
                bold: style.bold,
                italic: style.italic,
                mark: style.mark,
                strikeout: style.strikeout,
            }
        }
        Fragment::Element { id, .. } => {
            let rendered = to_rendered_block(&elements[id as usize]);
            RichTextFragment {
                is_element: true,
                text: Default::default(),
                block: rendered,
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
        shape: to_rendered_block_shape(block.shape),
        tone: to_rendered_block_tone(block.tone),
        text: block.text.clone().into(),
        target: block.target.clone().into(),
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
        checked: block.checked,
        side_a: ModelRc::new(VecModel::from(
            block
                .side_a
                .iter()
                .map(to_conflict_side_block)
                .collect::<Vec<_>>(),
        )),
        side_b: ModelRc::new(VecModel::from(
            block
                .side_b
                .iter()
                .map(to_conflict_side_block)
                .collect::<Vec<_>>(),
        )),
    }
}

fn to_rendered_block_shape(shape: BlockShape) -> RenderedBlockShape {
    match shape {
        BlockShape::PlainText => RenderedBlockShape::PlainText,
        BlockShape::Conflict => RenderedBlockShape::Conflict,
        BlockShape::Badge => RenderedBlockShape::Badge,
        BlockShape::Ruby => RenderedBlockShape::Ruby,
        BlockShape::Link => RenderedBlockShape::Link,
        BlockShape::Icon => RenderedBlockShape::Icon,
        BlockShape::Task => RenderedBlockShape::Task,
        BlockShape::Code => RenderedBlockShape::Code,
        BlockShape::Callout => RenderedBlockShape::Callout,
    }
}

fn to_rendered_block_tone(tone: Tone) -> RenderedBlockTone {
    match tone {
        Tone::Accent => RenderedBlockTone::Accent,
        Tone::Warning => RenderedBlockTone::Warning,
        Tone::Neutral => RenderedBlockTone::Neutral,
    }
}

/// One leaf of a `@conflict`'s side, converted the same way
/// [`to_rendered_block`] converts a top-level [`ClassifiedBlock`] --
/// except `shape` can never carry `Conflict` through to the Slint side:
/// [`ConflictSideBlock`] has no `side_a`/`side_b` fields to hold a nested
/// one's content at all (see that struct's own doc), so a nested,
/// still-unresolved `@conflict` here is normalized to the same `Badge`
/// look an unrecognized `@use`'d element gets -- "show *something*"
/// rather than a silently blank leaf.
fn to_conflict_side_block(leaf: &ConflictLeafBlock) -> ConflictSideBlock {
    let shape = if leaf.shape == BlockShape::Conflict {
        BlockShape::Badge
    } else {
        leaf.shape
    };
    let text = if leaf.shape == BlockShape::Conflict {
        "conflict".to_owned()
    } else {
        leaf.text.clone()
    };
    ConflictSideBlock {
        shape: to_rendered_block_shape(shape),
        tone: to_rendered_block_tone(leaf.tone),
        text: text.into(),
        target: leaf.target.clone().into(),
        bold: leaf.style.bold,
        italic: leaf.style.italic,
        mark: leaf.style.mark,
        strikeout: leaf.style.strikeout,
        reading: leaf.reading.clone().into(),
        icon_image: if leaf.shape == BlockShape::Icon {
            icon_image(&leaf.text, &leaf.reading)
        } else {
            slint::Image::default()
        },
        checked: leaf.checked,
    }
}

/// `pkg`'s `slug` icon as a Slint image, or a blank one if `origiri_icons`
/// doesn't vendor that pair (an unknown/typo'd icon name -- nothing in
/// `tomet-semantics` validates `@doc.icon`'s `name` against what's actually
/// vendored, so this has to degrade quietly rather than panic).
///
/// `pub(crate)`: `session`'s note-list icon (`@meta{ icon: @doc.icon(...)
/// }`, resolved via `NoteIndex::icon_for`) shares this with a body-inline
/// `@doc.icon`'s resolution above, rather than re-deriving the same
/// svg-bytes-to-`slint::Image` conversion twice.
pub(crate) fn icon_image(slug: &str, pkg: &str) -> slint::Image {
    origiri_icons::icon_svg(pkg, slug)
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
// first. `tests.rs`'s `tests::ui_helpers_and_conflict_resolution` already
// owns the one `App::new()` this whole binary gets; it calls these.
#[cfg(test)]
pub(crate) mod seam_tests {
    use super::*;
    use slint::Model;

    pub(crate) fn stacked_items_convert_into_the_generated_model(app: &App) {
        let model = note_body_items(
            "Before.\n\n@conflict(a: [Mine.], b: [Theirs.])\n",
            app,
            1000.0,
        );
        assert_eq!(model.row_count(), 2);
        let first = model.row_data(0).unwrap();
        assert!(!first.is_flow);
        assert_eq!(first.block.shape, RenderedBlockShape::PlainText);
        assert_eq!(first.block.text, "Before.");
        let second = model.row_data(1).unwrap();
        assert!(!second.is_flow);
        assert_eq!(second.block.shape, RenderedBlockShape::Conflict);
        assert_eq!(second.block.side_a.row_count(), 1);
        assert_eq!(second.block.side_a.row_data(0).unwrap().text, "Mine.");
        assert_eq!(second.block.side_b.row_count(), 1);
        assert_eq!(second.block.side_b.row_data(0).unwrap().text, "Theirs.");
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

    // The exact fragment/element count a `@conflict`'s two sides produce
    // is `flow`'s own `to_flow_paragraph`/`push_conflict_side` tests'
    // job now (plain data, no `App`/Slint layout engine needed to check
    // it) -- this only proves the seam: that flowing and wrapping still
    // work end to end through the real layout engine once a conflict is
    // in the mix.
    pub(crate) fn a_wide_inline_conflict_paragraph_flows_onto_one_line(app: &App) {
        let src = "The @conflict(a: [slow], b: [lazy]) fox jumps.\n";
        let model = note_body_items(src, app, 10_000.0);
        assert_eq!(model.row_count(), 1);
        let item = model.row_data(0).unwrap();
        assert!(item.is_flow);
        assert_eq!(item.lines.row_count(), 1);
    }

    pub(crate) fn a_narrow_width_wraps_an_inline_conflict_paragraph_onto_several_lines(app: &App) {
        let src = "The @conflict(a: [slow], b: [lazy]) fox jumps.\n";
        // Real font metrics, not a fake fixed-width measure -- too narrow
        // for the whole sentence (and its two colored runs) to fit on one
        // line.
        let model = note_body_items(src, app, 80.0);
        let item = model.row_data(0).unwrap();
        assert!(item.is_flow);
        assert!(
            item.lines.row_count() > 1,
            "expected a narrow width to wrap onto more than one line, got {}",
            item.lines.row_count()
        );
    }

    pub(crate) fn task_items_convert_into_the_generated_model(app: &App) {
        let src = "@task()[Open task]\n\n@task(x)[Done task]\n";
        let model = note_body_items(src, app, 1000.0);
        assert_eq!(model.row_count(), 2);

        let first = model.row_data(0).unwrap();
        assert!(!first.is_flow);
        assert_eq!(first.block.shape, RenderedBlockShape::Task);
        assert_eq!(first.block.text, "Open task");
        assert!(!first.block.checked);

        let second = model.row_data(1).unwrap();
        assert!(!second.is_flow);
        assert_eq!(second.block.shape, RenderedBlockShape::Task);
        assert_eq!(second.block.text, "Done task");
        assert!(second.block.checked);
    }

    pub(crate) fn inline_conflict_maintains_identity_and_order_across_resizes(app: &App) {
        let src = "まえ @conflict(a: [うんこ], b: [うんち]) うしろ\n";
        for width in [10_000.0, 500.0, 300.0, 180.0, 100.0, 60.0] {
            let model = note_body_items(src, app, width);
            assert_eq!(model.row_count(), 1);
            let item = model.row_data(0).unwrap();
            assert!(item.is_flow);

            let mut element_blocks = Vec::new();
            for l in 0..item.lines.row_count() {
                let line = item.lines.row_data(l).unwrap();
                for f in 0..line.fragments.row_count() {
                    let frag = line.fragments.row_data(f).unwrap();
                    if frag.is_element {
                        element_blocks.push((frag.block.text.to_string(), frag.block.tone));
                    }
                }
            }

            assert_eq!(
                element_blocks.len(),
                2,
                "At width {width}, expected exactly 2 elements, got {:?}",
                element_blocks
            );
            assert_eq!(
                element_blocks[0],
                ("うんこ".to_string(), RenderedBlockTone::Accent),
                "At width {width}, side A must be 'うんこ' with Accent tone"
            );
            assert_eq!(
                element_blocks[1],
                ("うんち".to_string(), RenderedBlockTone::Warning),
                "At width {width}, side B must be 'うんち' with Warning tone"
            );
        }
    }

    pub(crate) fn list_items_carry_depth_prefix_and_links(app: &App) {
        let src = "- An item with @link(\"https://example.com\")[a link]\n  - Nested item\n-. Numbered item\n";
        let model = note_body_items(src, app, 1000.0);
        assert_eq!(model.row_count(), 3);

        let item0 = model.row_data(0).unwrap();
        assert_eq!(item0.depth, 0);
        assert_eq!(item0.prefix, "•");
        assert!(item0.is_flow);
        let mut has_link = false;
        for l in 0..item0.lines.row_count() {
            let line = item0.lines.row_data(l).unwrap();
            for f in 0..line.fragments.row_count() {
                let frag = line.fragments.row_data(f).unwrap();
                if frag.is_element && frag.block.shape == RenderedBlockShape::Link {
                    assert_eq!(frag.block.target, "https://example.com");
                    assert_eq!(frag.block.text, "a link");
                    has_link = true;
                }
            }
        }
        assert!(has_link, "expected link fragment in list item");

        let item1 = model.row_data(1).unwrap();
        assert_eq!(item1.depth, 1);
        assert_eq!(item1.prefix, "•");

        let item2 = model.row_data(2).unwrap();
        assert_eq!(item2.depth, 0);
        assert_eq!(item2.prefix, "1.");
    }

    pub(crate) fn heading_items_carry_level_in_generated_model(app: &App) {
        let src = "= Heading 1\n\n== Heading 2\n\n=== Heading 3\n";
        let model = note_body_items(src, app, 1000.0);
        assert_eq!(model.row_count(), 3);

        let item0 = model.row_data(0).unwrap();
        assert_eq!(item0.heading_level, 1);

        let item1 = model.row_data(1).unwrap();
        assert_eq!(item1.heading_level, 2);

        let item2 = model.row_data(2).unwrap();
        assert_eq!(item2.heading_level, 3);
    }

    pub(crate) fn code_and_callout_items_convert_into_the_generated_model(app: &App) {
        let src = "```rust\nfn main() {}\n```\n\n@callout(warning)[Watch out!]\n";
        let model = note_body_items(src, app, 1000.0);
        assert_eq!(model.row_count(), 2);

        let item0 = model.row_data(0).unwrap();
        assert_eq!(item0.block.shape, RenderedBlockShape::Code);
        assert_eq!(item0.block.text, "fn main() {}");
        assert_eq!(item0.block.reading, "rust");

        let item1 = model.row_data(1).unwrap();
        assert_eq!(item1.block.shape, RenderedBlockShape::Callout);
        assert_eq!(item1.block.text, "Watch out!");
        assert_eq!(item1.block.tone, RenderedBlockTone::Warning);
    }
}
