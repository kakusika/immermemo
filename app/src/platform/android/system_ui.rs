//! Toggles the Android status bar's visibility over JNI -- used to hide
//! system chrome while a note is being edited, when the user opts into
//! that in Settings (see `MainActivity.kt`'s `setStatusBarVisible`) -- and
//! reads the system theme the Material colors follow.

use jni::objects::JValue;

use super::activity::{call, call_logged};

pub fn set_status_bar_visible(visible: bool) {
    call_logged("toggle the Android status bar", |env, activity| {
        env.call_method(
            activity,
            "setStatusBarVisible",
            "(Z)V",
            &[JValue::Bool(visible as u8)],
        )
        .map(|_| ())
    });
}

pub fn get_wallpaper_seed_color() -> Option<u32> {
    let color = call(|env, activity| {
        env.call_method(activity, "getWallpaperSeedColor", "()I", &[])?
            .i()
    })? as u32;
    (color != 0).then_some(color)
}

pub fn is_system_in_dark_theme() -> bool {
    call(|env, activity| {
        env.call_method(activity, "isSystemInDarkTheme", "()Z", &[])?
            .z()
    })
    .unwrap_or(false)
}

#[allow(dead_code)]
pub fn get_system_contrast() -> f64 {
    call(|env, activity| {
        env.call_method(activity, "getSystemContrast", "()F", &[])?
            .f()
    })
    .unwrap_or(0.0) as f64
}
