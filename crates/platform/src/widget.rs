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

#[cfg(any(target_os = "android", test))]
use immermemo_core::IndexedNote;
use immermemo_core::NoteIndex;

/// Where the snapshot file is written -- the private-storage base `AppData`
/// uses -- set once by [`init_android_widget_bridge`].
#[cfg(target_os = "android")]
static SNAPSHOT_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Tells the widget bridge which directory the snapshot file goes into.
#[cfg(target_os = "android")]
pub fn init_android_widget_bridge(base: &std::path::Path) {
    let _ = SNAPSHOT_PATH.set(base.join("widget_recent_notes.json"));
}

/// Writes the recent-notes snapshot and asks the widget to redraw now.
/// No-op if the bridge wasn't initialized (e.g. on desktop, or if init
/// failed).
pub fn export_recent_notes(_index: &NoteIndex) {
    #[cfg(target_os = "android")]
    {
        let Some(snapshot_path) = SNAPSHOT_PATH.get() else {
            return;
        };
        let notes = match _index.list_recent(10) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("immermemo: widget list_recent failed: {e}");
                return;
            }
        };
        let json = notes_to_json(&notes);

        std::thread::spawn(move || {
            if let Err(e) = std::fs::write(snapshot_path, json) {
                eprintln!("immermemo: failed to write widget snapshot: {e}");
                return;
            }
            crate::android::activity::call_logged("notify widget", |env, activity| {
                // `android_main` runs on a plain native pthread, not one the
                // framework attached with the app's classloader as context, so
                // `JNIEnv::find_class` resolves against the bootstrap
                // classloader and can't see `dev.immermemo.app.widget.*`: it
                // throws `ClassNotFoundException` and, left unhandled, the next
                // JNI call aborts the process ("No pending exception
                // expected"). The activity's own loader sees our classes.
                let class = env
                    .call_method(activity, "getClass", "()Ljava/lang/Class;", &[])?
                    .l()?;
                let loader = env
                    .call_method(&class, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
                    .l()?;
                // `loadClass` wants the dotted binary name, unlike `find_class`'s
                // slash-separated one.
                let name = env.new_string("dev.immermemo.app.widget.NoteWidgetProvider")?;
                let class_obj = env
                    .call_method(
                        &loader,
                        "loadClass",
                        "(Ljava/lang/String;)Ljava/lang/Class;",
                        &[jni::objects::JValue::Object(&name)],
                    )?
                    .l()?;
                env.call_static_method(
                    jni::objects::JClass::from(class_obj),
                    "requestUpdate",
                    "(Landroid/content/Context;)V",
                    &[jni::objects::JValue::Object(activity)],
                )
                .map(|_| ())
            });
        });
    }
}

/// Reads the `"note_path"` string extra off the `NativeActivity`'s launch
/// `Intent`, if present -- set by the home-screen widget's `PendingIntent`
/// when a specific note was tapped (see `app/android`'s
/// `NoteWidgetProvider`/`NoteWidgetFactory`). `None` on any failure (no
/// extra set, JNI error, ...): same "fall back to the normal startup note"
/// behavior either way, so nothing here is worth surfacing as an error.
#[cfg(target_os = "android")]
pub fn read_launch_note_path() -> Option<std::path::PathBuf> {
    crate::android::activity::call(|env, activity| {
        let intent = env
            .call_method(activity, "getIntent", "()Landroid/content/Intent;", &[])?
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
        crate::android::activity::optional_string(env, value)
    })
    .flatten()
    .map(std::path::PathBuf::from)
}

/// Hand-rolled JSON array of `{"path": ..., "title": ...}` objects -- this
/// is a tiny, fixed shape read by one Kotlin file in this repo, not a public
/// format, so it isn't worth a `serde_json` dependency for. Only called from
/// Android-only code above, but kept free of that `cfg` itself (`test` only)
/// so it's unit-testable on desktop.
#[cfg(any(target_os = "android", test))]
fn notes_to_json(notes: &[IndexedNote]) -> String {
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
        let notes = [IndexedNote {
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
