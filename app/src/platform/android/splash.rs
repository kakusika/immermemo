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

use std::sync::atomic::{AtomicBool, Ordering};

static NOTIFIED: AtomicBool = AtomicBool::new(false);

/// Registers a rendering notifier on `app`'s window that calls
/// `MainActivity.markFirstFrameRendered()` over JNI the first time
/// Slint's renderer reaches [`slint::RenderingState::AfterRendering`],
/// releasing the splash held open by `MainActivity`.
pub fn notify_on_first_frame(app: &crate::App) {
    use slint::ComponentHandle;
    let _ = app.window().set_rendering_notifier(|state, _| {
        if !matches!(state, slint::RenderingState::AfterRendering) {
            return;
        }
        if NOTIFIED.swap(true, Ordering::Relaxed) {
            return;
        }
        super::activity::call_logged("release the Android splash screen", |env, activity| {
            env.call_method(activity, "markFirstFrameRendered", "()V", &[])
                .map(|_| ())
        });
    });
}
