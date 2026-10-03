//! Renders the window headlessly at a phone's size and density, to look at the
//! layout without a device: `snap <out-prefix> [width] [height] [scale]`.
//!
//! `#![allow(deprecated)]`: this file's `to_rich_text_fragment` uses
//! `slint::ComponentFactory`, same known, accepted risk as
//! `src/render/mod.rs`'s doc comment describes.
#![allow(deprecated)]

use std::cell::Cell;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentFactory, ComponentHandle, ModelRc, PhysicalSize, VecModel};
slint::include_modules!();

use immermemo::render::classify::{BlockShape, ClassifiedBlock, Tone};
use immermemo::render::flow::{FlowParagraph, NoteBodyItem};
use origami_richtext_flow::{Fragment, Measure, layout_block};

struct Headless {
    main: Rc<MinimalSoftwareWindow>,
    main_claimed: Cell<bool>,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        // The first call is `App`'s own window -- the one this file
        // actually renders/snapshots, so it must be `main`. A later call
        // (e.g. constructing a `ComponentFactory` product that inherits
        // `Window`, like `FlowElementWidget` -- see
        // `src/render/mod.rs`) also goes through this: its
        // `ComponentHandle::new()` calls `WindowInner::set_component`,
        // which rebinds whichever adapter it's given to render *that*
        // component instead ("Further event handling and rendering, etc.
        // will be done with that component" -- i-slint-core's own doc
        // comment). Returning `main` again there would silently steal the
        // window away from `App` and corrupt every snapshot taken after
        // it (discovered exactly this way while adding the first
        // `ComponentFactory` user to this app). A real backend (winit)
        // never has this problem -- each call creates a genuinely
        // separate native window -- so each additional call here gets its
        // own independent, unused `MinimalSoftwareWindow` instead.
        if !self.main_claimed.replace(true) {
            Ok(self.main.clone())
        } else {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }
}

fn render(window: &MinimalSoftwareWindow, size: PhysicalSize, path: &str) {
    slint::platform::update_timers_and_animations();
    window.window().request_redraw();
    let (w, h) = (size.width as usize, size.height as usize);
    let mut buf = vec![PremultipliedRgbaColor::default(); w * h];
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buf, w);
    });
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in &buf {
        out.extend_from_slice(&[p.red, p.green, p.blue]);
    }
    std::fs::write(path, out).unwrap();
}

fn main() {
    let mut args = std::env::args().skip(1);
    let prefix = args.next().unwrap();
    let mut num = |default: f32| args.next().map_or(default, |a| a.parse().unwrap());
    let (w, h, scale) = (num(412.0), num(892.0), num(2.0));

    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless {
        main: window.clone(),
        main_claimed: Cell::new(false),
    }))
    .unwrap();
    let size = PhysicalSize::new((w * scale) as u32, (h * scale) as u32);
    window.set_size(size);
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });

    let app = App::new().unwrap();
    // A phone's status and navigation bars, which `safe-area-insets` reports on a device.
    app.set_extra_inset_top(36.0);
    app.set_extra_inset_bottom(24.0);
    let names = [
        "買い物リスト",
        "meeting/2026-09-27",
        "ideas",
        "旅行の計画",
        "untitled-1",
    ];
    app.set_notes(ModelRc::new(VecModel::from(
        names
            .iter()
            .map(|n| (*n).into())
            .collect::<Vec<slint::SharedString>>(),
    )));
    app.set_conflicted(ModelRc::new(VecModel::from(vec![
        false, false, true, false, false,
    ])));
    app.set_current(0);
    app.set_current_title("買い物リスト".into());
    app.set_current_note_path("買い物リスト.tmt".into());
    app.set_char_count(19);
    app.set_word_count(4);
    app.set_line_count(5);
    app.set_conflict_count(1);
    app.set_body("牛乳\nパン\n卵\n\nあとで郵便局へ。".into());
    app.set_can_undo(true);
    app.show().unwrap();

    // Tab 0: Home
    app.set_active_tab(0);
    app.set_current_vault_name("notes".into());
    app.set_remote_configured(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, "/dev/null");
    render(&window, size, &format!("{prefix}-home.ppm"));

    // Home with sync error
    app.set_last_sync_error("Sync failed: no merge base found; class=Merge (22)".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-home-sync-error.ppm"));
    app.set_last_sync_error("".into());

    // Tab 1: Files / List
    app.set_active_tab(1);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-list.ppm"));

    // Tab 2: Search query with snippet preview
    app.set_active_tab(2);
    app.set_search_query("計画".into());
    app.set_notes(ModelRc::new(VecModel::from(vec!["旅行の計画".into()])));
    app.set_conflicted(ModelRc::new(VecModel::from(vec![false])));
    app.set_snippets(ModelRc::new(VecModel::from(vec![
        "...来月の旅行の計画について話し合う...".into(),
    ])));
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-search.ppm"));

    // Search query with no matching notes (empty state)
    app.set_search_query("宇宙旅行".into());
    app.set_notes(ModelRc::new(VecModel::default()));
    app.set_snippets(ModelRc::new(VecModel::default()));
    render(&window, size, &format!("{prefix}-search-empty.ppm"));

    // Restore note list
    app.set_search_query("".into());
    app.set_notes(ModelRc::new(VecModel::from(
        names
            .iter()
            .map(|n| (*n).into())
            .collect::<Vec<slint::SharedString>>(),
    )));
    app.set_conflicted(ModelRc::new(VecModel::from(vec![
        false, false, true, false, false,
    ])));

    // Note Editor Sheet Transition (50% progress gesture tracking)
    app.set_is_dragging_sheet(true);
    app.set_drag_sheet_progress(0.5);
    render(&window, size, &format!("{prefix}-transition-half.ppm"));
    app.set_is_dragging_sheet(false);
    app.set_drag_sheet_progress(0.0);

    // Note Editor Sheet (View Mode)
    app.set_note_sheet_open(true);
    app.set_editor_edit_mode(false);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-editor.ppm"));

    // View mode via the real classify/flow pipeline: a `@mobile.conflict`
    // triple, an unrecognized namespaced element, and -- the common case
    // that motivated `origami-richtext`/`origami-richtext-flow` in the
    // first place -- both sides editing the same sentence, which narrows
    // to an *inline* conflict mid-paragraph rather than a whole-block one
    // (see `src/render/classify.rs`'s
    // `an_inline_merge_conflict_is_recognized_through_the_real_pipeline`).
    // That last paragraph now flows as one line instead of stacking each
    // split fragment as its own row. Goes through
    // `immermemo_editor::note_body_items` directly rather than
    // `immermemo::render::note_body_items`: this file's own
    // `include_modules!()` generates its own `NoteBodyItemView`/
    // `RenderedBlock`/`RichTextLine` types, distinct from the library
    // crate's (see `src/render/mod.rs`'s doc comment for why), so
    // the small seam (`to_note_body_item_view` and friends, below) has to
    // be repeated here too.
    let body = "買い物リストの変更について。\n\n\
                @mobile.conflict(mine)\n\n\
                牛乳（低脂肪）\n\n\
                @mobile.conflict(theirs)\n\n\
                牛乳（特濃）\n\n\
                @mobile.conflict(end)\n\n\
                @deck.bookmark(label: しおり)\n\n\
                続きはここから。\n\n\
                The @mobile.conflict(mine)slow@mobile.conflict(theirs)lazy@mobile.conflict(end) fox jumps.\n\n\
                Some @strong[bold] and @em[italic] and @mark[marked] and @strikeout[struck] text, with @ruby[漢字](rt:\"かんじ\") inline.";
    app.set_body(body.into());
    let max_width = app.get_body_content_width();
    app.set_note_body_items(ModelRc::new(VecModel::from(
        immermemo::render::flow::note_body_items(body)
            .into_iter()
            .map(|item| to_note_body_item_view(item, &app, max_width))
            .collect::<Vec<_>>(),
    )));
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-editor-note-body.ppm"));

    // Note Editor Sheet (Edit Mode)
    app.set_editor_edit_mode(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-editor-edit.ppm"));
    app.set_editor_edit_mode(false);

    app.set_current_has_conflict(true);
    render(&window, size, &format!("{prefix}-conflict.ppm"));

    // Conflict Sheet overlay
    app.set_active_conflict_index(0);
    app.set_active_conflict_total(2);
    app.set_active_conflict_mine("牛乳（低脂肪）".into());
    app.set_active_conflict_theirs("牛乳（特濃）".into());
    app.set_conflict_sheet_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-conflict-sheet.ppm"));
    app.set_conflict_sheet_open(false);

    app.set_current_has_conflict(false);
    app.set_note_sheet_open(false);

    // Tab 3: Details / Properties
    app.set_active_tab(3);
    app.set_current_has_conflict(true);
    app.set_active_conflict_total(2);
    app.set_active_conflict_mine("牛乳（低脂肪）".into());
    app.set_active_conflict_theirs("牛乳（特濃）".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-properties-conflict.ppm"));
    app.set_current_has_conflict(false);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-properties.ppm"));
    app.set_note_sheet_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));

    app.set_status("Save failed: permission denied".into());
    app.set_status_is_error(true);
    render(&window, size, &format!("{prefix}-error.ppm"));
    app.set_status("".into());
    app.set_status_is_error(false);
    app.set_note_sheet_open(false);

    app.set_current(-1);
    app.set_notes(ModelRc::new(VecModel::default()));
    render(&window, size, &format!("{prefix}-empty.ppm"));
    app.set_notes(ModelRc::new(VecModel::from(
        names
            .iter()
            .map(|n| (*n).into())
            .collect::<Vec<slint::SharedString>>(),
    )));
    app.set_current(0);

    // Tab 4: Settings
    app.set_active_tab(4);
    app.set_current_vault_name("notes".into());
    app.set_current_vault_path("/home/user/notes".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-settings.ppm"));
    app.set_active_tab(0);

    app.set_note_menu_index(2);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-note-menu.ppm"));
    app.set_note_menu_index(-1);

    app.set_rename_open(true);
    app.set_rename_draft("ideas".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-rename.ppm"));
    app.set_rename_open(false);

    app.set_delete_confirm_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-delete-confirm.ppm"));
    app.set_delete_confirm_open(false);

    app.set_vaults(ModelRc::new(VecModel::from(
        ["notes", "work"]
            .iter()
            .map(|n| (*n).into())
            .collect::<Vec<slint::SharedString>>(),
    )));
    app.set_current_vault(0);
    app.set_vault_sheet_open(true);
    render(&window, size, &format!("{prefix}-vaults.ppm"));
    app.set_vault_sheet_open(false);

    // Verify runtime UI scaling (1.25x)
    app.set_active_tab(0);
    app.set_ui_scale(1.25);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-home-scaled-125.ppm"));
    app.set_ui_scale(1.0);

    // Verify dark mode contrast & elevation
    app.set_theme_choice(2); // Dark theme
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-home-dark.ppm"));
    app.set_active_tab(1);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-list-dark.ppm"));
    app.set_note_sheet_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-editor-dark.ppm"));
    app.set_note_sheet_open(false);
    app.set_active_tab(0);
    app.set_theme_choice(0);
}

// A copy of `src/render/mod.rs`'s seam, targeting this file's own
// `include_modules!()`-generated types instead of the library crate's (see
// the call site above for why it can't just call that module directly).

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

/// Same technique `src/render/mod.rs`'s `RealMeasure` uses: the
/// off-screen probe elements live in `app.slint` itself, so they're
/// available here too (this file's own compiled `App`).
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
    }
}
