//! Toggles the Android status bar's visibility over JNI -- used to hide
//! system chrome while a note is being edited, when the user opts into
//! that in Settings (see `MainActivity.kt`'s `setStatusBarVisible`).

use std::sync::OnceLock;

use jni::JavaVM;
use jni::objects::{GlobalRef, JValue};

static ANDROID_APP_VM: OnceLock<(JavaVM, GlobalRef)> = OnceLock::new();

/// # Safety
/// `vm_ptr` must be valid `JavaVM*` and `activity_ptr` must be valid `jobject` for the process.
pub unsafe fn init(vm_ptr: *mut std::ffi::c_void, activity_ptr: *mut std::ffi::c_void) {
    let Ok(vm) = (unsafe { JavaVM::from_raw(vm_ptr.cast()) }) else {
        return;
    };
    let Ok(global_ref) = (|| -> jni::errors::Result<_> {
        let env = vm.attach_current_thread()?;
        let activity = unsafe { jni::objects::JObject::from_raw(activity_ptr.cast()) };
        env.new_global_ref(activity)
    })() else {
        return;
    };
    let _ = ANDROID_APP_VM.set((vm, global_ref));
}

pub fn set_status_bar_visible(visible: bool) {
    let Some((vm, activity_ref)) = ANDROID_APP_VM.get() else {
        return;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return;
    };
    let res = env.call_method(
        activity_ref.as_obj(),
        "setStatusBarVisible",
        "(Z)V",
        &[JValue::Bool(visible as u8)],
    );
    if let Err(e) = res {
        eprintln!("immermemo: failed to toggle the Android status bar: {e}");
    }
}

pub fn get_wallpaper_seed_color() -> Option<u32> {
    let (vm, activity_ref) = ANDROID_APP_VM.get()?;
    let mut env = vm.attach_current_thread().ok()?;
    let val = env
        .call_method(activity_ref.as_obj(), "getWallpaperSeedColor", "()I", &[])
        .ok()?;
    let color = val.i().ok()? as u32;
    if color != 0 { Some(color) } else { None }
}

pub fn is_system_in_dark_theme() -> bool {
    let Some((vm, activity_ref)) = ANDROID_APP_VM.get() else {
        return false;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return false;
    };
    let Ok(val) = env.call_method(activity_ref.as_obj(), "isSystemInDarkTheme", "()Z", &[]) else {
        return false;
    };
    val.z().unwrap_or(false)
}

pub fn get_system_contrast() -> f64 {
    let Some((vm, activity_ref)) = ANDROID_APP_VM.get() else {
        return 0.0;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return 0.0;
    };
    let Ok(val) = env.call_method(activity_ref.as_obj(), "getSystemContrast", "()F", &[]) else {
        return 0.0;
    };
    val.f().unwrap_or(0.0) as f64
}
