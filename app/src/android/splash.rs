//! Signals the system splash screen (held open by
//! `android/app/src/main/kotlin/dev/immermemo/app/MainActivity.kt`'s
//! `setKeepOnScreenCondition`) to release once Slint's first frame has
//! actually reached the native surface.
//!
//! `NativeActivity.onCreate` hands the window's `Surface` straight to
//! native code (`takeSurface`) before any native code runs, so without
//! this the system splash is dismissed immediately on activity start
//! and the empty surface briefly shows whatever is behind it (the
//! launcher) until Slint's first frame lands.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use jni::JavaVM;
use jni::objects::GlobalRef;

static ANDROID_APP_VM: OnceLock<(JavaVM, GlobalRef)> = OnceLock::new();
static NOTIFIED: AtomicBool = AtomicBool::new(false);

/// Initializes the JNI handle `notify_on_first_frame`'s rendering
/// notifier calls back through. Must run before [`notify_on_first_frame`]
/// is registered.
///
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

/// Registers a rendering notifier on `app`'s window that calls
/// `MainActivity.markFirstFrameRendered()` over JNI the first time
/// Slint's renderer reaches [`slint::RenderingState::AfterRendering`],
/// releasing the splash held open in [`init`]'s caller.
pub fn notify_on_first_frame(app: &crate::App) {
    use slint::ComponentHandle;
    let _ = app.window().set_rendering_notifier(|state, _| {
        if !matches!(state, slint::RenderingState::AfterRendering) {
            return;
        }
        if NOTIFIED.swap(true, Ordering::Relaxed) {
            return;
        }
        let Some((vm, activity_ref)) = ANDROID_APP_VM.get() else {
            return;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let res = env.call_method(activity_ref.as_obj(), "markFirstFrameRendered", "()V", &[]);
        if let Err(e) = res {
            eprintln!("immermemo: failed to release the Android splash screen: {e}");
        }
    });
}
