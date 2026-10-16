//! Renders the window headlessly at a phone's size and density, to look at the
//! layout without a device: `snap <out-prefix> [width] [height] [scale]`.

use std::cell::Cell;
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

use immermemo::render::to_note_body_item_view;
use immermemo::{
    AccountData, AddVaultAdapter, App, BreadcrumbSegment, DirectoryEntry, MediaBase,
    NoteBannerView, NoteIconKind, NoteIconView, PickerIconEntry, RepoRow, SettingsAdapter,
};

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

    // Tab 2: Files / Directory -- List view (default mode) at the vault
    // root: a folder, a second folder, and two root-level notes.
    app.set_active_tab(2);
    app.set_files_page(1);
    let text_icon = |text: &str| {
        let cells: Vec<slint::SharedString> = text
            .chars()
            .map(|c| slint::SharedString::from(c.to_string()))
            .collect();
        NoteIconView {
            kind: NoteIconKind::Text,
            cells: ModelRc::new(VecModel::from(cells)),
            ..NoteIconView::default()
        }
    };
    // A stand-in for a decoded thumbnail: a diagonal gradient, so a
    // `cover` crop that squashes or tints it is visible.
    let picture_icon = || {
        let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(96, 64);
        let (w, h) = (buf.width(), buf.height());
        for (i, px) in buf.make_mut_slice().iter_mut().enumerate() {
            let (x, y) = ((i as u32) % w, (i as u32) / w);
            *px = slint::Rgba8Pixel::new((x * 255 / w) as u8, (y * 255 / h) as u8, 160, 255);
        }
        NoteIconView {
            kind: NoteIconKind::Picture,
            picture: slint::Image::from_rgba8(buf),
            ..NoteIconView::default()
        }
    };
    let root_entries = || {
        ModelRc::new(VecModel::from(vec![
            DirectoryEntry {
                is_folder: true,
                key: "meeting".into(),
                display: "meeting".into(),
                note_index: -1,
                has_conflict: false,
                icon: NoteIconView::default(),
                path: "".into(),
                is_selected: false,
            },
            DirectoryEntry {
                is_folder: true,
                key: "ideas".into(),
                display: "ideas".into(),
                note_index: -1,
                has_conflict: false,
                icon: NoteIconView::default(),
                path: "".into(),
                is_selected: false,
            },
            DirectoryEntry {
                is_folder: false,
                key: "".into(),
                display: "買い物リスト".into(),
                note_index: 0,
                has_conflict: false,
                icon: text_icon("🛒"),
                path: "".into(),
                is_selected: false,
            },
            DirectoryEntry {
                is_folder: false,
                key: "".into(),
                display: "untitled-1".into(),
                note_index: 4,
                has_conflict: false,
                icon: NoteIconView::default(),
                path: "".into(),
                is_selected: false,
            },
            DirectoryEntry {
                is_folder: false,
                key: "".into(),
                display: "two cells".into(),
                note_index: 5,
                has_conflict: false,
                icon: text_icon("あい"),
                path: "".into(),
                is_selected: false,
            },
            DirectoryEntry {
                is_folder: false,
                key: "".into(),
                display: "three cells".into(),
                note_index: 6,
                has_conflict: false,
                icon: text_icon("ABC"),
                path: "".into(),
                is_selected: false,
            },
            DirectoryEntry {
                is_folder: false,
                key: "".into(),
                display: "four cells".into(),
                note_index: 7,
                has_conflict: false,
                icon: text_icon("ABCD"),
                path: "".into(),
                is_selected: false,
            },
            DirectoryEntry {
                is_folder: false,
                key: "".into(),
                display: "picture".into(),
                note_index: 8,
                has_conflict: false,
                icon: picture_icon(),
                path: "".into(),
                is_selected: false,
            },
        ]))
    };
    app.set_directory_folder_entries(root_entries());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-directory-list.ppm"));

    // Directory -- List view, descended into "meeting" (breadcrumb shown,
    // one conflicted note). The root crumb (empty key/display, rendered
    // as a home icon) is always first -- it's the only way back to the
    // vault root now that there's no separate always-present root button.
    let root_crumb = || BreadcrumbSegment {
        key: "".into(),
        display: "".into(),
    };
    app.set_directory_breadcrumb(ModelRc::new(VecModel::from(vec![
        root_crumb(),
        BreadcrumbSegment {
            key: "meeting".into(),
            display: "meeting".into(),
        },
    ])));
    app.set_directory_folder_entries(ModelRc::new(VecModel::from(vec![DirectoryEntry {
        is_folder: false,
        key: "".into(),
        display: "2026-09-27".into(),
        note_index: 1,
        has_conflict: true,
        icon: NoteIconView::default(),
        path: "".into(),
        is_selected: false,
    }])));
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(
        &window,
        size,
        &format!("{prefix}-directory-list-inside-folder.ppm"),
    );

    // Directory -- List view, two levels deep (multi-segment breadcrumb
    // -- the case that exposed the "spread across the row instead of
    // packed to the left" layout bug).
    app.set_directory_breadcrumb(ModelRc::new(VecModel::from(vec![
        root_crumb(),
        BreadcrumbSegment {
            key: "meeting".into(),
            display: "meeting".into(),
        },
        BreadcrumbSegment {
            key: "meeting/2026".into(),
            display: "2026".into(),
        },
    ])));
    app.set_directory_folder_entries(ModelRc::new(VecModel::from(vec![DirectoryEntry {
        is_folder: false,
        key: "".into(),
        display: "09-27".into(),
        note_index: 1,
        has_conflict: true,
        icon: NoteIconView::default(),
        path: "".into(),
        is_selected: false,
    }])));
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(
        &window,
        size,
        &format!("{prefix}-directory-list-breadcrumb-deep.ppm"),
    );

    // Directory -- Grid view, back at the vault root.
    app.set_directory_mode(1);
    app.set_directory_breadcrumb(ModelRc::new(VecModel::default()));
    app.set_directory_folder_entries(root_entries());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-directory-grid.ppm"));

    // The mini card shows the open note's own icon.
    app.set_current_icon(text_icon("🐾🩵"));
    render(&window, size, &format!("{prefix}-mini-card-text-icon.ppm"));
    app.set_current_icon(picture_icon());
    render(
        &window,
        size,
        &format!("{prefix}-mini-card-picture-icon.ppm"),
    );
    app.set_current_icon(NoteIconView::default());

    // Directory -- Grid view, descended into "meeting" too.
    app.set_directory_breadcrumb(ModelRc::new(VecModel::from(vec![
        root_crumb(),
        BreadcrumbSegment {
            key: "meeting".into(),
            display: "meeting".into(),
        },
    ])));
    app.set_directory_folder_entries(ModelRc::new(VecModel::from(vec![DirectoryEntry {
        is_folder: false,
        key: "".into(),
        display: "2026-09-27".into(),
        note_index: 1,
        has_conflict: true,
        icon: NoteIconView::default(),
        path: "".into(),
        is_selected: false,
    }])));
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(
        &window,
        size,
        &format!("{prefix}-directory-grid-inside-folder.ppm"),
    );
    app.set_directory_mode(0);
    app.set_directory_folder_entries(ModelRc::new(VecModel::default()));
    app.set_directory_breadcrumb(ModelRc::new(VecModel::default()));

    // Tab 2: Files / Daily Notes (collapsed)
    app.set_files_page(2);
    app.set_calendar_show_month_picker(false);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-daily-notes.ppm"));

    // Daily Notes (month picker expanded)
    app.set_calendar_show_month_picker(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-daily-notes-picker.ppm"));
    app.set_calendar_show_month_picker(false);
    app.set_files_page(0);

    // Tab 1: Search query with snippet preview
    app.set_active_tab(1);
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

    // View mode via the real classify/flow pipeline: a block-level
    // `@conflict`, an unrecognized namespaced element, and -- the common
    // case that motivated `origiri-richtext`/`origiri-richtext-flow` in
    // the first place -- both sides editing the same sentence, which
    // narrows to an *inline* `@conflict` mid-paragraph rather than a
    // whole-block one (see `src/render/classify.rs`'s
    // `an_inline_merge_conflict_is_recognized_through_the_real_pipeline`).
    // That last paragraph now flows as one line instead of stacking each
    // split fragment as its own row. Items come from `render::note_body_items`'s
    // classify/flow step and go through the library's own
    // `to_note_body_item_view`, so what is snapped is what the app draws.
    let body = "買い物リストの変更について。\n\n\
                @conflict(a: [牛乳（低脂肪）], b: [牛乳（特濃）])\n\n\
                @deck.bookmark(label: しおり)\n\n\
                続きはここから。\n\n\
                The @conflict(a: [slow], b: [lazy]) fox jumps.\n\n\
                Some @strong[bold] and @em[italic] and @mark[marked] and @strikeout[struck] text, with @ruby[漢字](rt:\"かんじ\") inline.\n\n\
                Starred @doc.icon(\"star\") and flagged @doc.icon(\"flag\", pkg:\"tabler\") inline.";
    app.set_body(body.into());
    let max_width = app.get_body_content_width();
    app.set_note_body_items(ModelRc::new(VecModel::from(
        immermemo::render::flow::note_body_items(body)
            .into_iter()
            .map(|item| to_note_body_item_view(item, &app, max_width, &MediaBase::none()))
            .collect::<Vec<_>>(),
    )));
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-editor-note-body.ppm"));

    // A banner above the title and a body image, as they look once loaded
    // (a gradient stands in for the decoded pictures, no network or vault
    // here): first the banner at two crop positions, then a body image that
    // is still loading next to one that has arrived.
    let gradient = |w: u32, h: u32| {
        let mut buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
        for (i, px) in buf.make_mut_slice().iter_mut().enumerate() {
            let (x, y) = ((i as u32) % w, (i as u32) / w);
            *px = slint::Rgba8Pixel::new((x * 255 / w) as u8, (y * 255 / h) as u8, 160, 255);
        }
        slint::Image::from_rgba8(buf)
    };
    let body = "写真つきのメモ。\n\n@embed[A cat](path: \"cat.png\")\n\n@embed[Still loading](path: \"later.png\")\n\nおわり。";
    let mut rows = immermemo::render::flow::note_body_items(body)
        .into_iter()
        .map(|item| to_note_body_item_view(item, &app, max_width, &MediaBase::none()))
        .collect::<Vec<_>>();
    if let Some(cat) = rows.iter_mut().find(|row| row.block.text == "A cat") {
        cat.block.picture = gradient(600, 300);
    }
    app.set_body(body.into());
    app.set_note_body_items(ModelRc::new(VecModel::from(rows)));
    app.set_current_banner(NoteBannerView {
        declared: true,
        picture: gradient(1280, 640),
        focus_y: 0.25,
    });
    render(&window, size, &format!("{prefix}-editor-banner.ppm"));
    app.set_current_banner(NoteBannerView {
        declared: true,
        picture: slint::Image::default(),
        focus_y: 0.5,
    });
    render(
        &window,
        size,
        &format!("{prefix}-editor-banner-loading.ppm"),
    );
    app.set_current_banner(NoteBannerView::default());

    // Verify headings, code blocks, callouts, list items, and clickable links
    let list_links_body = "= 見出しレベル 1 (大見出し)\n\n\
                           == 見出しレベル 2 (中見出し)\n\n\
                           === 見出しレベル 3 (小見出し)\n\n\
                           リンクの確認:\n\
                           - 外部URL: @link(\"https://example.com\")[Example]\n\
                           - ノートリンク: @link(target:\"ref:@ReGLOSS\")\n\
                           - 別の記法: @link(\"ref:Shopping\")\n\n\
                           タスクリスト (チェック可能):\n\
                           -@task(x) 完了したタスク\n\
                           -@task() 未完了のタスク\n\n\
                           コードブロック:\n\
                           ```rust\n\
                           fn main() {\n\
                               println!(\"Hello, Tomet!\");\n\
                           }\n\
                           ```\n\n\
                           コールアウト表示:\n\
                           @callout(info, title: \"お知らせ\")[インフォメーションのコールアウトです。]\n\n\
                           @callout(warning)[注意: この操作は取り消せません。]\n\n\
                           箇条書きリスト:\n\
                           - 項目 1: 通常のテキスト\n\
                             - ネストされた項目 A (@strong[太字]含む)\n\
                             - ネストされた項目 B\n\
                           - 項目 2\n\n\
                           番号付きリスト:\n\
                           -. 最初のステップ\n\
                           -. 次のステップ\n\
                           -. 最後のステップ\n";
    app.set_body(list_links_body.into());
    let max_width = app.get_body_content_width();
    app.set_note_body_items(ModelRc::new(VecModel::from(
        immermemo::render::flow::note_body_items(list_links_body)
            .into_iter()
            .map(|item| to_note_body_item_view(item, &app, max_width, &MediaBase::none()))
            .collect::<Vec<_>>(),
    )));
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-editor-lists-links.ppm"));

    // User bug scenario verification: inline conflict across multiple window widths
    let user_body = "テスト: @conflict(a: [うんこ], b: [うんち]) です。";
    app.set_body(user_body.into());
    for &(label_w, test_w) in &[("narrow", 280.0), ("mid", 412.0), ("wide", 600.0)] {
        let test_size = PhysicalSize::new((test_w * scale) as u32, (h * scale) as u32);
        window.set_size(test_size);
        let max_w = app.get_body_content_width();
        app.set_note_body_items(ModelRc::new(VecModel::from(
            immermemo::render::flow::note_body_items(user_body)
                .into_iter()
                .map(|item| to_note_body_item_view(item, &app, max_w, &MediaBase::none()))
                .collect::<Vec<_>>(),
        )));
        std::thread::sleep(std::time::Duration::from_millis(100));
        render(
            &window,
            test_size,
            &format!("{prefix}-resize-{label_w}.ppm"),
        );
    }
    // Restore original size and body
    window.set_size(size);
    app.set_body(body.into());
    let max_width = app.get_body_content_width();
    app.set_note_body_items(ModelRc::new(VecModel::from(
        immermemo::render::flow::note_body_items(body)
            .into_iter()
            .map(|item| to_note_body_item_view(item, &app, max_width, &MediaBase::none()))
            .collect::<Vec<_>>(),
    )));

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
    app.set_active_conflict_a("牛乳（低脂肪）".into());
    app.set_active_conflict_b("牛乳（特濃）".into());
    app.set_conflict_sheet_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-conflict-sheet.ppm"));
    app.set_conflict_sheet_open(false);

    app.set_current_has_conflict(false);
    app.set_note_sheet_open(false);

    // Properties no longer has its own tab -- it's the note sheet's own
    // "Properties" view (EditorScreen's note-view, see ViewSwitcher in
    // components.slint), not reachable here without simulating a tap
    // (note-view is deliberately not exposed to Rust, see editor.slint's
    // doc comment on it).
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

    // Settings Sheet
    app.set_settings_sheet_open(true);
    app.set_current_vault_name("notes".into());
    app.set_current_vault_path("/home/user/notes".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-settings.ppm"));

    app.set_settings_page(1);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-settings-interface.ppm"));

    app.set_settings_page(11);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-settings-bottom-nav.ppm"));

    // Every other Settings page and the states within them, so a layout
    // refactor of the sheet can be diffed screen by screen.
    let settings = app.global::<SettingsAdapter>();
    let snap_settings = |name: &str| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        render(&window, size, &format!("{prefix}-settings-{name}.ppm"));
    };
    settings.set_color_source(1);
    app.set_settings_page(1);
    snap_settings("interface-custom-color");
    settings.set_color_source(0);
    for (page, name) in [
        (3, "vaults"),
        (4, "about"),
        (5, "transfer-share"),
        (6, "advanced"),
        (7, "navigation"),
        (12, "directory"),
        (13, "daily-notes"),
        (14, "starred"),
        (15, "recent"),
        (16, "graph"),
    ] {
        app.set_settings_page(page);
        snap_settings(name);
    }
    app.set_settings_page(5);
    settings.set_transfer_tab(1);
    snap_settings("transfer-import");
    settings.set_transfer_tab(0);
    // Accounts: the list, then the add-account form's every tab and state.
    app.set_settings_page(2);
    let account = |id: &str, provider: &str, user: &str| AccountData {
        id: id.into(),
        provider: provider.into(),
        instance_url: "https://example.org".into(),
        username: user.into(),
        display_name: "".into(),
        avatar: slint::Image::default(),
        has_avatar: false,
        vault_count: if user == "octocat" { 2 } else { 0 },
    };
    settings.set_accounts(ModelRc::new(VecModel::from(vec![
        account("github:octocat", "github", "octocat"),
        account("gitlab:https://gitlab.com:alice", "gitlab", "alice"),
    ])));
    snap_settings("accounts-list");
    settings.set_adding_account(true);
    settings.set_sync_provider_tab(0);
    snap_settings("accounts-add-github");
    settings.set_github_auth_pending(true);
    settings.set_github_user_code("WDAS-5821".into());
    settings.set_github_verification_uri("https://github.com/login/device".into());
    settings.set_github_auth_status("Waiting for browser authorization...".into());
    snap_settings("accounts-add-github-pending");
    settings.set_github_auth_pending(false);
    settings.set_sync_provider_tab(1);
    snap_settings("accounts-add-forgejo");
    settings.set_forgejo_status_message(
        "Could not read the account's profile: 401 Unauthorized".into(),
    );
    settings.set_forgejo_status_is_error(true);
    snap_settings("accounts-add-forgejo-error");
    settings.set_forgejo_connecting(true);
    snap_settings("accounts-add-forgejo-connecting");
    settings.set_forgejo_connecting(false);
    settings.set_sync_provider_tab(2);
    snap_settings("accounts-add-gitlab");
    settings.set_gitlab_connecting(true);
    settings.set_gitlab_status_message("Connecting...".into());
    snap_settings("accounts-add-gitlab-connecting");
    settings.set_gitlab_connecting(false);
    settings.set_adding_account(false);

    app.set_settings_page(0);
    app.set_settings_sheet_open(false);

    app.set_note_menu_index(2);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-note-menu.ppm"));
    app.set_note_menu_index(-1);

    app.set_more_menu_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-more-menu.ppm"));
    app.set_more_menu_open(false);

    // Mini card swipe gesture with history preview bubble (Back)
    app.set_can_navigate_back(true);
    app.set_history_back_title("前回のメモ: アイデア出し".into());
    app.set_is_dragging_mini_reveal(true);
    app.set_mini_card_reveal_dx(42.0);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(
        &window,
        size,
        &format!("{prefix}-mini-card-history-bubble.ppm"),
    );
    app.set_mini_card_reveal_dx(0.0);
    app.set_is_dragging_mini_reveal(false);

    // Mini card swipe gesture with history preview bubble (Forward)
    app.set_can_navigate_forward(true);
    app.set_history_forward_title("次のメモ: 買い物リスト".into());
    app.set_is_dragging_mini_reveal(true);
    app.set_mini_card_reveal_dx(-42.0);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(
        &window,
        size,
        &format!("{prefix}-mini-card-forward-bubble.ppm"),
    );
    app.set_mini_card_reveal_dx(0.0);
    app.set_is_dragging_mini_reveal(false);

    app.set_rename_open(true);
    app.set_rename_draft("ideas".into());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-rename.ppm"));
    app.set_rename_open(false);

    // Icon picker -- real resolved images (origiri_icons::icon_svg), not
    // blank placeholders, so a layout bug in the grid itself would
    // actually show up here.
    let picker_names = ["star", "flag", "heart", "home", "bell", "bookmark"];
    app.set_icon_picker_results(ModelRc::new(VecModel::from(
        picker_names
            .iter()
            .map(|&name| {
                let image = origiri_icons::icon_svg("tabler", name)
                    .and_then(|svg| slint::Image::load_from_svg_data(&svg).ok())
                    .unwrap_or_default();
                PickerIconEntry {
                    name: name.into(),
                    image,
                }
            })
            .collect::<Vec<_>>(),
    )));
    app.set_icon_picker_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-icon-picker.ppm"));
    app.set_icon_picker_open(false);

    // The vault setup sheet in each of its states.
    let setup = app.global::<AddVaultAdapter>();
    let snap_setup = |name: &str| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        render(&window, size, &format!("{prefix}-vault-setup-{name}.ppm"));
    };
    settings.set_accounts(ModelRc::new(VecModel::from(vec![
        account("github:octocat", "github", "octocat"),
        account("gitlab:https://gitlab.com:alice", "gitlab", "alice"),
    ])));
    app.set_vault_setup_open(true);
    setup.set_mode(0);
    setup.set_page(0);
    setup.set_name("".into());
    snap_setup("add-empty");
    setup.set_name("Work notes".into());
    setup.set_resulting_path("/data/folders/Work notes".into());
    setup.set_can_submit(true);
    snap_setup("add-in-app");
    setup.set_storage(1);
    setup.set_folder("/storage/emulated/0/Documents".into());
    setup.set_can_pick_folder(true);
    setup.set_storage_hint("Allow access to all files to keep a vault in local storage.".into());
    setup.set_resulting_path("/storage/emulated/0/Documents/Work notes".into());
    snap_setup("add-local-storage");
    setup.set_storage(0);
    setup.set_storage_hint("".into());
    setup.set_sync(true);
    snap_setup("sync-no-account-chosen");
    setup.set_account_id("github:octocat".into());
    setup.set_repo_mode(0);
    setup.set_repo_name("Work-notes".into());
    snap_setup("sync-new-repository");
    setup.set_repo_mode(1);
    setup.set_repos(ModelRc::new(VecModel::from(vec![
        RepoRow {
            name: "notes".into(),
            full_name: "octocat/notes".into(),
            is_private: true,
        },
        RepoRow {
            name: "ideas".into(),
            full_name: "octocat/ideas".into(),
            is_private: false,
        },
    ])));
    setup.set_repo_full_name("octocat/ideas".into());
    snap_setup("sync-existing-repository");
    setup.set_repos(ModelRc::default());
    setup.set_repos_loading(true);
    snap_setup("sync-loading-repositories");
    setup.set_repos_loading(false);
    setup.set_repos_error("Could not list the repositories: 401 Unauthorized".into());
    snap_setup("sync-repositories-error");
    setup.set_repos_error("".into());
    setup.set_sync_kind(1);
    setup.set_custom_url("https://git.example.com/me/notes.git".into());
    snap_setup("sync-remote-url");
    setup.set_error("Could not create the repository: name already exists".into());
    setup.set_busy(false);
    snap_setup("error");
    setup.set_error("".into());
    setup.set_busy(true);
    snap_setup("busy");
    setup.set_busy(false);
    setup.set_page(1);
    snap_setup("add-account-page");
    setup.set_page(0);
    setup.set_mode(1);
    setup.set_target_name("Work notes".into());
    setup.set_sync_kind(0);
    snap_setup("sync-mode");
    app.set_vault_setup_open(false);

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
    std::thread::sleep(std::time::Duration::from_millis(300));
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
    app.set_active_tab(2);
    app.set_directory_folder_entries(root_entries());
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-directory-dark.ppm"));
    app.set_directory_folder_entries(ModelRc::new(VecModel::default()));
    let max_width = app.get_body_content_width();
    app.set_note_body_items(ModelRc::new(VecModel::from(
        immermemo::render::flow::note_body_items(body)
            .into_iter()
            .map(|item| to_note_body_item_view(item, &app, max_width, &MediaBase::none()))
            .collect::<Vec<_>>(),
    )));
    app.set_note_sheet_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-editor-dark.ppm"));
    app.set_note_sheet_open(false);
    app.set_active_tab(0);
    app.set_theme_choice(0);
}
