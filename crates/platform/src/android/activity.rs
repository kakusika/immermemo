//! The process-wide handle to the JVM and the running `MainActivity`, and the
//! one place that attaches a thread to call into it.
//!
//! Every JNI bridge in this app (haptics, splash, status bar, the widget,
//! launch intents) used to keep its own copy of this pair; they share it now.

use std::sync::OnceLock;

use jni::errors::Result as JniResult;
use jni::objects::{GlobalRef, JObject};
use jni::{JNIEnv, JavaVM};

struct Handle {
    vm: JavaVM,
    activity: GlobalRef,
}

static HANDLE: OnceLock<Handle> = OnceLock::new();

/// Captures the JVM and the activity for the process's lifetime. Must run
/// once, from `android_main`, before any other function here is useful.
///
/// # Safety
/// `vm_ptr` must be a valid `JavaVM*` and `activity_ptr` a valid `jobject`
/// for the process.
pub unsafe fn init(vm_ptr: *mut std::ffi::c_void, activity_ptr: *mut std::ffi::c_void) {
    let Ok(vm) = (unsafe { JavaVM::from_raw(vm_ptr.cast()) }) else {
        return;
    };
    let Ok(activity) = (|| -> JniResult<_> {
        let env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(activity_ptr.cast()) };
        env.new_global_ref(activity)
    })() else {
        return;
    };
    let _ = HANDLE.set(Handle { vm, activity });
}

/// Runs `f` with a JNI environment attached to the current thread and the
/// activity. `None` if [`init`] has not succeeded, the thread cannot be
/// attached, or `f` failed -- callers that only want a best-effort answer
/// (a getter with a sensible default) use this.
pub fn call<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> JniResult<R>) -> Option<R> {
    let handle = HANDLE.get()?;
    let mut env = handle.vm.attach_current_thread().ok()?;
    f(&mut env, handle.activity.as_obj()).ok()
}

/// Like [`call`], but reports a failure of `f` on stderr as
/// "failed to {what}" -- for fire-and-forget actions where silence would
/// hide a broken bridge.
pub fn call_logged<R>(
    what: &str,
    f: impl FnOnce(&mut JNIEnv, &JObject) -> JniResult<R>,
) -> Option<R> {
    let handle = HANDLE.get()?;
    let mut env = handle.vm.attach_current_thread().ok()?;
    match f(&mut env, handle.activity.as_obj()) {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("immermemo: failed to {what}: {e}");
            None
        }
    }
}

/// Reads a `String` out of a possibly-null `java.lang.String` object.
pub fn optional_string(env: &mut JNIEnv, value: JObject) -> JniResult<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let string = jni::objects::JString::from(value);
    Ok(Some(env.get_string(&string)?.into()))
}
