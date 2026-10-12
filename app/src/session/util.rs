use slint::{ComponentHandle, SharedString};
use crate::App;

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

fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1029 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = (mp as i64) + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m, d as i64)
}

pub fn format_timestamp(epoch_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(epoch_secs);
    let rel = if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    };

    let secs_per_day = 86400;
    let days = epoch_secs.div_euclid(secs_per_day);
    let time_of_day = epoch_secs.rem_euclid(secs_per_day);
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02} ({rel})")
}

pub fn format_time_only(epoch_secs: i64) -> String {
    let secs_per_day = 86400;
    let days = epoch_secs.div_euclid(secs_per_day);
    let time_of_day = epoch_secs.rem_euclid(secs_per_day);
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let (_year, month, day) = days_to_ymd(days);
    format!("{month:02}/{day:02} {hours:02}:{minutes:02}")
}
