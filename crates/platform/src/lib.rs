//! What differs by operating system, behind one cross-platform surface:
//! the Android entry point and JNI bridges ([`android`]), haptic feedback,
//! the home-screen widget's data, and the TLS roots mobile builds bring along.
//! Each is a graceful no-op where the platform has nothing to do.

#[cfg(target_os = "android")]
pub mod android;
pub mod cert;
pub mod haptic;
pub mod storage;
pub mod widget;
