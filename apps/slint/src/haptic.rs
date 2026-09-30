//! Platform haptic feedback implementation.
//!
//! On Android, this calls `window.getDecorView().performHapticFeedback(LONG_PRESS)`
//! via JNI, which provides the system-standard tactile response on long press
//! without requiring extra permissions.
//! On other platforms (e.g. desktop), this is a graceful no-op.

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

/// Triggers a brief haptic feedback (tactile tick) for long-press actions.
pub fn perform_haptic() {
    #[cfg(target_os = "android")]
    {
        let Some((vm, activity_ref)) = ANDROID_APP_VM.get() else {
            return;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let activity = activity_ref.as_obj();

        // 0 is HapticFeedbackConstants.LONG_PRESS
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
                &[jni::objects::JValue::Int(0)],
            )?;
            Ok(())
        })();
        if let Err(e) = res {
            eprintln!("immermemo: failed to trigger Android haptic feedback: {e}");
        }
    }
}
