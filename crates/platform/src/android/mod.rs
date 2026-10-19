//! Android platform integration: Activity JNI calls, keystore, fonts,
//! splash handoff, system UI, and shared-storage access.

pub mod activity;
pub mod fonts;
pub mod keystore;
pub mod splash;
pub mod storage_access;
pub mod system_ui;

/// Reads the deep-link import URI (`immermemo://...`) off `MainActivity`'s intent, if present.
pub fn read_launch_import_uri() -> Option<String> {
    activity::call(|env, activity| {
        let value = env
            .call_method(activity, "consumeImportUri", "()Ljava/lang/String;", &[])?
            .l()?;
        activity::optional_string(env, value)
    })
    .flatten()
}
