//! What the About page's System card lists: the facts about the device and the
//! app's build of it that a bug report needs, one `label: value` row each.
//!
//! - **OS**: Android's release and API level (from the system properties), or
//!   the desktop OS's name.
//! - **Device**: the maker and model (Android only; a desktop has no such thing
//!   worth saying).
//! - **Architecture**: what the app was compiled for.
//! - **Renderer**: the one this build draws with.
//! - **Emoji font**: which font the device offered for emoji and which was used
//!   (Android only; see `platform/android/fonts.rs`).
//!
//! The same rows, under the build's label, make the text the card's copy button
//! puts on the clipboard, so a report is pasted rather than retyped.

/// One row of the System card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
}

/// The rows for this device.
pub fn rows() -> Vec<Row> {
    let mut rows = vec![Row {
        label: "OS",
        value: os(),
    }];
    if let Some(device) = device() {
        rows.push(Row {
            label: "Device",
            value: device,
        });
    }
    rows.push(Row {
        label: "Architecture",
        value: std::env::consts::ARCH.to_owned(),
    });
    rows.push(Row {
        label: "Renderer",
        value: "FemtoVG".to_owned(),
    });
    #[cfg(target_os = "android")]
    {
        let emoji = crate::platform::android::fonts::report();
        if !emoji.is_empty() {
            rows.push(Row {
                label: "Emoji font",
                value: emoji,
            });
        }
    }
    rows
}

/// The text copied from the System card: the app and its build, then a row per
/// line.
pub fn report(version: &str, build: &str, rows: &[Row]) -> String {
    let mut text = format!("Immermemo v{version}\nBuild: {build}");
    for row in rows {
        text.push_str(&format!("\n{}: {}", row.label, row.value));
    }
    text
}

fn os() -> String {
    #[cfg(target_os = "android")]
    {
        if let (Some(release), Some(sdk)) = (
            system_property("ro.build.version.release"),
            system_property("ro.build.version.sdk"),
        ) {
            return format_android(&release, &sdk);
        }
    }
    desktop_os_name(std::env::consts::OS)
}

fn device() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        return format_device(
            &system_property("ro.product.manufacturer").unwrap_or_default(),
            &system_property("ro.product.model").unwrap_or_default(),
        );
    }
    #[cfg(not(target_os = "android"))]
    None
}

#[cfg(any(target_os = "android", test))]
fn format_android(release: &str, sdk: &str) -> String {
    format!("Android {release} (API {sdk})")
}

#[cfg(any(target_os = "android", test))]
/// `Google Pixel 8`; a model that already starts with the maker's name (as
/// some do) is not prefixed with it again.
fn format_device(manufacturer: &str, model: &str) -> Option<String> {
    let (manufacturer, model) = (manufacturer.trim(), model.trim());
    if model.is_empty() {
        return (!manufacturer.is_empty()).then(|| manufacturer.to_owned());
    }
    if manufacturer.is_empty()
        || model
            .to_lowercase()
            .starts_with(&manufacturer.to_lowercase())
    {
        return Some(model.to_owned());
    }
    Some(format!("{manufacturer} {model}"))
}

fn desktop_os_name(os: &str) -> String {
    match os {
        "linux" => "Linux",
        "macos" => "macOS",
        "windows" => "Windows",
        "ios" => "iOS",
        other => return other.to_owned(),
    }
    .to_owned()
}

/// A value of an Android system property, if it is set.
#[cfg(target_os = "android")]
fn system_property(name: &str) -> Option<String> {
    use std::ffi::{CString, c_char, c_int};

    // `PROP_VALUE_MAX` in `<sys/system_properties.h>`: values are at most 91
    // bytes plus the terminator.
    const PROP_VALUE_MAX: usize = 92;
    unsafe extern "C" {
        fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int;
    }
    let name = CString::new(name).ok()?;
    let mut value = [0 as c_char; PROP_VALUE_MAX];
    // Safety: `name` is a NUL-terminated string and `value` has the room
    // `__system_property_get` documents for any value.
    let length = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if length <= 0 {
        return None;
    }
    let bytes: Vec<u8> = value[..length as usize].iter().map(|&c| c as u8).collect();
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_is_named_with_its_release_and_api_level() {
        assert_eq!(format_android("14", "34"), "Android 14 (API 34)");
    }

    #[test]
    fn a_device_is_its_maker_and_model_without_repeating_the_maker() {
        assert_eq!(
            format_device("Google", "Pixel 8").as_deref(),
            Some("Google Pixel 8")
        );
        assert_eq!(
            format_device("samsung", "samsung SM-S911B").as_deref(),
            Some("samsung SM-S911B")
        );
        assert_eq!(format_device("", "Pixel 8").as_deref(), Some("Pixel 8"));
        assert_eq!(format_device("Google", "").as_deref(), Some("Google"));
        assert_eq!(format_device("", ""), None);
    }

    #[test]
    fn a_desktop_os_gets_its_proper_name() {
        assert_eq!(desktop_os_name("linux"), "Linux");
        assert_eq!(desktop_os_name("macos"), "macOS");
        assert_eq!(desktop_os_name("freebsd"), "freebsd");
    }

    #[test]
    fn the_report_is_the_build_then_a_line_per_row() {
        let rows = [
            Row {
                label: "OS",
                value: "Android 14 (API 34)".to_owned(),
            },
            Row {
                label: "Emoji font",
                value: "NotoColorEmojiLegacy.ttf (bitmap)".to_owned(),
            },
        ];
        assert_eq!(
            report("0.1.0", "0b9bc064+386bcd88 · 2026-10-16 15:49 UTC", &rows),
            "Immermemo v0.1.0\nBuild: 0b9bc064+386bcd88 · 2026-10-16 15:49 UTC\n\
             OS: Android 14 (API 34)\nEmoji font: NotoColorEmojiLegacy.ttf (bitmap)"
        );
    }

    #[test]
    fn every_device_lists_its_os_and_architecture_and_renderer() {
        let labels: Vec<_> = rows().iter().map(|row| row.label).collect();
        for needed in ["OS", "Architecture", "Renderer"] {
            assert!(labels.contains(&needed), "{labels:?}");
        }
    }
}
