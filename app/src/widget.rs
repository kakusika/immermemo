//! Home-screen widget data bridge.
//!
//! Writes a small JSON snapshot of the most recently modified notes for the
//! Kotlin-side `RemoteViewsService` (`app/android`) to read, then pokes
//! the widget to redraw immediately. Android's own `AppWidgetProviderInfo`
//! update interval has a 30-minute floor, far too slow to feel right right
//! after an edit, so freshness instead rides on [`export_recent_notes`]
//! being called whenever the note list changes (see `session::refresh_list`).
//!
//! On other platforms (e.g. desktop), this is a graceful no-op.

use immermemo_index::NoteIndex;

#[cfg(target_os = "android")]
static ANDROID_APP_VM: std::sync::OnceLock<(
    jni::JavaVM,
    jni::objects::GlobalRef, // activity
    jni::objects::GlobalRef, // the app's own ClassLoader -- see its use below
    std::path::PathBuf,
)> = std::sync::OnceLock::new();

/// Initializes the widget bridge with the application's JavaVM/NativeActivity
/// and the directory the snapshot file should be written into (the same
/// private-storage base `AppData` uses).
///
/// Also captures the activity's `ClassLoader`. `android_main` runs on a
/// plain native pthread, not one the framework attached with the app's
/// classloader as context, so `JNIEnv::find_class` on it resolves against
/// the bootstrap classloader and can't see `dev.immermemo.app.widget.*` --
/// it throws `ClassNotFoundException` and, left unhandled, the next JNI call
/// aborts the process ("No pending exception expected"). Loading our own
/// classes through this captured loader's `loadClass` instead sidesteps the
/// whole problem; see its use in [`export_recent_notes`].
///
/// # Safety
/// `vm_ptr` must be valid `JavaVM*` and `activity_ptr` must be valid `jobject` for the process.
#[cfg(target_os = "android")]
pub unsafe fn init_android_widget_bridge(
    vm_ptr: *mut std::ffi::c_void,
    activity_ptr: *mut std::ffi::c_void,
    base: &std::path::Path,
) {
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(vm_ptr.cast()) }) else {
        return;
    };
    let Ok((activity_ref, class_loader_ref)) = (|| -> jni::errors::Result<_> {
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { jni::objects::JObject::from_raw(activity_ptr.cast()) };
        let class = env
            .call_method(&activity, "getClass", "()Ljava/lang/Class;", &[])?
            .l()?;
        let loader = env
            .call_method(&class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
            .l()?;
        Ok((env.new_global_ref(activity)?, env.new_global_ref(loader)?))
    })() else {
        return;
    };
    let _ = ANDROID_APP_VM.set((
        vm,
        activity_ref,
        class_loader_ref,
        base.join("widget_recent_notes.json"),
    ));
}

/// Writes the recent-notes snapshot and asks the widget to redraw now.
/// No-op if the bridge wasn't initialized (e.g. on desktop, or if init
/// failed).
pub fn export_recent_notes(_index: &NoteIndex) {
    #[cfg(target_os = "android")]
    {
        let index = _index;
        let Some((vm, activity_ref, class_loader_ref, snapshot_path)) = ANDROID_APP_VM.get() else {
            return;
        };
        let notes = match index.list_recent(10) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("immermemo: widget list_recent failed: {e}");
                return;
            }
        };
        let json = notes_to_json(&notes);
        if let Err(e) = std::fs::write(snapshot_path, json) {
            eprintln!("immermemo: failed to write widget snapshot: {e}");
            return;
        }

        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let activity = activity_ref.as_obj();
        let res: jni::errors::Result<()> = (|| {
            // `loadClass` wants the dotted binary name, unlike `find_class`'s
            // slash-separated one.
            let name = env.new_string("dev.immermemo.app.widget.NoteWidgetProvider")?;
            let class_obj = env
                .call_method(
                    class_loader_ref.as_obj(),
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[jni::objects::JValue::Object(&name)],
                )?
                .l()?;
            let class = jni::objects::JClass::from(class_obj);
            env.call_static_method(
                class,
                "requestUpdate",
                "(Landroid/content/Context;)V",
                &[jni::objects::JValue::Object(activity)],
            )?;
            Ok(())
        })();
        if let Err(e) = res {
            eprintln!("immermemo: failed to notify widget: {e}");
        }
    }
}

/// Reads the `"note_path"` string extra off the `NativeActivity`'s launch
/// `Intent`, if present -- set by the home-screen widget's `PendingIntent`
/// when a specific note was tapped (see `app/android`'s
/// `NoteWidgetProvider`/`NoteWidgetFactory`). `None` on any failure (no
/// extra set, JNI error, ...): same "fall back to the normal startup note"
/// behavior either way, so nothing here is worth surfacing as an error.
///
/// # Safety
/// Same contract as [`init_android_widget_bridge`].
#[cfg(target_os = "android")]
pub unsafe fn read_launch_note_path(
    vm_ptr: *mut std::ffi::c_void,
    activity_ptr: *mut std::ffi::c_void,
) -> Option<std::path::PathBuf> {
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr.cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { jni::objects::JObject::from_raw(activity_ptr.cast()) };
    let result: jni::errors::Result<Option<String>> = (|| {
        let intent = env
            .call_method(&activity, "getIntent", "()Landroid/content/Intent;", &[])?
            .l()?;
        let key = env.new_string("note_path")?;
        let value = env
            .call_method(
                &intent,
                "getStringExtra",
                "(Ljava/lang/String;)Ljava/lang/String;",
                &[jni::objects::JValue::Object(&key)],
            )?
            .l()?;
        if value.is_null() {
            return Ok(None);
        }
        let jstring = jni::objects::JString::from(value);
        Ok(Some(env.get_string(&jstring)?.into()))
    })();
    result.ok().flatten().map(std::path::PathBuf::from)
}

/// Hand-rolled JSON array of `{"path": ..., "title": ...}` objects -- this
/// is a tiny, fixed shape read by one Kotlin file in this repo, not a public
/// format, so it isn't worth a `serde_json` dependency for. Only called from
/// Android-only code above, but kept free of that `cfg` itself (`test` only)
/// so it's unit-testable on desktop.
#[cfg(any(target_os = "android", test))]
fn notes_to_json(notes: &[immermemo_index::IndexedNote]) -> String {
    let mut out = String::from("[");
    for (i, note) in notes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"path\":\"");
        json_escape_into(&note.path.to_string_lossy(), &mut out);
        out.push_str("\",\"title\":\"");
        json_escape_into(&note.title, &mut out);
        out.push_str("\"}");
    }
    out.push(']');
    out
}

#[cfg(any(target_os = "android", test))]
fn json_escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn escapes_quotes_backslashes_and_unicode() {
        let notes = [immermemo_index::IndexedNote {
            path: PathBuf::from("a\"b\\c.tmt"),
            title: "日本語\ttitle".to_string(),
            has_conflict: false,
        }];
        let json = notes_to_json(&notes);
        assert_eq!(
            json,
            "[{\"path\":\"a\\\"b\\\\c.tmt\",\"title\":\"日本語\\ttitle\"}]"
        );
    }
}
