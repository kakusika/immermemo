use std::path::Path;

use crate::{App, SettingsAdapter};
use slint::{ComponentHandle, SharedString};

/// Shows `msg` in the status line as plain information.
pub fn set_status(app: &App, msg: impl Into<SharedString>) {
    app.set_status(msg.into());
    app.set_status_is_error(false);
}

/// Shows `msg` in the status line as an error.
pub fn set_error(app: &App, msg: impl Into<SharedString>) {
    app.set_status(msg.into());
    app.set_status_is_error(true);
}

/// Copies the given text to the system clipboard using Slint's platform abstraction.
pub fn copy_to_clipboard(app: &App, text: &str) {
    slint::private_unstable_api::re_exports::WindowInner::from_pub(app.window())
        .context()
        .platform()
        .set_clipboard_text(text, slint::platform::Clipboard::DefaultClipboard);
}

/// Reads text from the system clipboard using Slint's platform abstraction.
pub fn read_from_clipboard(app: &App) -> Option<String> {
    slint::private_unstable_api::re_exports::WindowInner::from_pub(app.window())
        .context()
        .platform()
        .clipboard_text(slint::platform::Clipboard::DefaultClipboard)
}

/// Runs `f` against the app on the UI thread, from any thread -- a no-op if
/// the window is already gone by the time the event loop gets to it.
pub fn on_ui_thread(weak: &slint::Weak<App>, f: impl FnOnce(&App) + Send + 'static) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = weak.upgrade() {
            f(&app);
        }
    });
}

/// Shows `vault_dir`'s name and path wherever the UI displays the current
/// vault (the main window and the Settings sheet keep separate copies).
pub fn show_vault_identity(app: &App, vault_dir: &Path) {
    let name: SharedString = super::vault_display_name(vault_dir).into();
    let path: SharedString = vault_dir.display().to_string().into();
    let settings = app.global::<SettingsAdapter>();
    settings.set_current_vault_name(name.clone());
    settings.set_current_vault_path(path.clone());
    app.set_current_vault_name(name);
    app.set_current_vault_path(path);

    let bytes = immermemo_core::compute_vault_size(vault_dir);
    let size_str = immermemo_core::format_vault_size(bytes);
    app.set_vault_size(size_str.into());
}
