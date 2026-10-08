//! Platform haptic feedback implementation.
//!
//! - On Android, this calls `window.getDecorView().performHapticFeedback(...)`
//!   via JNI, which provides tactile response without extra permissions.
//! - On iOS, this will invoke UIKit's `UIImpactFeedbackGenerator`.
//! - On desktop Linux, this is a graceful no-op.

#[cfg(target_os = "android")]
static ANDROID_APP_VM: std::sync::OnceLock<(jni::JavaVM, jni::objects::GlobalRef)> =
    std::sync::OnceLock::new();

/// Initializes Android haptics with the application's JavaVM and NativeActivity.
///
/// # Safety
/// `vm_ptr` must be valid `JavaVM*` and `activity_ptr` must be valid `jobject` for the process.
#[cfg(target_os = "android")]
pub unsafe fn init_android_haptics(
    vm_ptr: *mut std::ffi::c_void,
    activity_ptr: *mut std::ffi::c_void,
) {
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(vm_ptr.cast()) }) else {
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

/// Android HapticFeedbackConstants types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HapticType {
    /// HapticFeedbackConstants.CLOCK_TICK (4): Light, crisp tactile tick for tab switching and navigation.
    Light,
    /// HapticFeedbackConstants.LONG_PRESS (0): Standard heavy tactile buzz for long-press actions.
    Heavy,
}

/// Triggers a light haptic feedback (tactile tick) for tab switching and navigation.
pub fn perform_haptic() {
    perform_haptic_with_type(HapticType::Light);
}

/// Triggers a heavier haptic feedback for long-press actions.
pub fn perform_heavy_haptic() {
    perform_haptic_with_type(HapticType::Heavy);
}

/// Triggers haptic feedback of the given type.
pub fn perform_haptic_with_type(_haptic_type: HapticType) {
    #[cfg(target_os = "android")]
    {
        let haptic_type = _haptic_type;
        let Some((vm, activity_ref)) = ANDROID_APP_VM.get() else {
            return;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let activity = activity_ref.as_obj();

        // On Android:
        // 4 is HapticFeedbackConstants.CLOCK_TICK (light, crisp tick)
        // 0 is HapticFeedbackConstants.LONG_PRESS (heavy buzz)
        let constant: i32 = match haptic_type {
            HapticType::Light => 4,
            HapticType::Heavy => 0,
        };

        let res: jni::errors::Result<()> = (|| {
            let window = env
                .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
                .l()?;
            let decor_view = env
                .call_method(window, "getDecorView", "()Landroid/view/View;", &[])?
                .l()?;
            env.call_method(
                decor_view,
                "performHapticFeedback",
                "(I)Z",
                &[jni::objects::JValue::Int(constant)],
            )?;
            Ok(())
        })();
        if let Err(e) = res {
            eprintln!("immermemo: failed to trigger Android haptic feedback: {e}");
        }
    }
    #[cfg(target_os = "ios")]
    {
        // TODO(ios): Trigger UIImpactFeedbackGenerator style (Light / Heavy) via UIKit
        let _ = _haptic_type;
    }
}
