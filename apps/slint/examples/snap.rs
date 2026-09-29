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
    app.set_body("牛乳\nパン\n卵\n\nあとで郵便局へ。".into());
    app.set_can_undo(true);
    app.show().unwrap();

    // `changed` handlers (which decide `narrow`) run before a frame, so the layout
    // settles on the second one.
    render(&window, size, "/dev/null");
    render(&window, size, &format!("{prefix}-list.ppm"));
    app.set_list_open(false);
    render(&window, size, &format!("{prefix}-editor.ppm"));
    app.set_current_has_conflict(true);
    render(&window, size, &format!("{prefix}-conflict.ppm"));
    app.set_current_has_conflict(false);

    app.set_status("Save failed: permission denied".into());
    app.set_status_is_error(true);
    render(&window, size, &format!("{prefix}-error.ppm"));
    app.set_status("".into());
    app.set_status_is_error(false);

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

    app.set_rename_open(true);
    app.set_rename_draft("ideas".into());
    render(&window, size, &format!("{prefix}-rename.ppm"));
    app.set_rename_open(false);

    app.set_delete_confirm_open(true);
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
}
