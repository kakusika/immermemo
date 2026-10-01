//! Renders the window headlessly at a phone's size and density, to look at the
//! layout without a device: `snap <out-prefix> [width] [height] [scale]`.
use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};
slint::include_modules!();

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
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
    slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
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

    // Note Editor Sheet
    app.set_note_sheet_open(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    render(&window, size, &format!("{prefix}-editor.ppm"));
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

    app.set_remote_open(true);
    app.set_remote_has_token(true);
    app.set_remote_url_draft("https://github.com/you/notes.git".into());
    render(&window, size, &format!("{prefix}-remote.ppm"));
    app.set_remote_open(false);

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
