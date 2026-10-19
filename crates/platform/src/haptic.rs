//! Platform haptic feedback implementation.
//!
//! - On Android, this calls `window.getDecorView().performHapticFeedback(...)`
//!   via JNI, which provides tactile response without extra permissions.
//! - On iOS, this will invoke UIKit's `UIImpactFeedbackGenerator`.
//! - On desktop Linux, this is a graceful no-op.

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
        // HapticFeedbackConstants: CLOCK_TICK (4) is a light, crisp tick;
        // LONG_PRESS (0) is the heavy buzz.
        let constant: i32 = match _haptic_type {
            HapticType::Light => 4,
            HapticType::Heavy => 0,
        };
        crate::android::activity::call_logged(
            "trigger Android haptic feedback",
            |env, activity| {
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
                )
                .map(|_| ())
            },
        );
    }
    #[cfg(target_os = "ios")]
    {
        // TODO(ios): Trigger UIImpactFeedbackGenerator style (Light / Heavy) via UIKit
        let _ = _haptic_type;
    }
}
