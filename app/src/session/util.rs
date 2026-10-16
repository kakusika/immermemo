use crate::App;
use slint::{ComponentHandle, SharedString};

pub fn set_status(app: &App, msg: impl Into<SharedString>) {
    let s: SharedString = msg.into();
    let text = s.as_str();
    let is_error = text.starts_with("Save failed")
        || text.starts_with("Sync failed")
        || text.starts_with("Could not")
        || text.starts_with("Enter a remote")
        || text.contains("failed")
        || text.contains("error")
        || text.contains("Error");
    app.set_status(s);
    app.set_status_is_error(is_error);
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
