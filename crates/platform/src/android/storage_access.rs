//! Shared-storage access over JNI: whether the app may use it, asking for
//! that, and the system's folder picker. See `crate::storage` for
//! what it is for and for the URI-to-path conversion.

use super::activity;

/// Whether the app may read and write the device's shared storage by path.
pub fn has_all_files_access() -> bool {
    activity::call(|env, activity| {
        env.call_method(activity, "hasAllFilesAccess", "()Z", &[])?
            .z()
    })
    .unwrap_or(false)
}

/// Opens the system screen (or prompt) for granting that access.
pub fn request_all_files_access() {
    activity::call_logged("ask for storage access", |env, activity| {
        env.call_method(activity, "requestAllFilesAccess", "()V", &[])?;
        Ok(())
    });
}

/// Opens the system's folder picker. The answer arrives later, through
/// `MainActivity.onActivityResult` -> `Java_dev_immermemo_app_MainActivity_notifyFolderPicked`.
pub fn pick_folder() {
    activity::call_logged("open the folder picker", |env, activity| {
        env.call_method(activity, "pickFolder", "()V", &[])?;
        Ok(())
    });
}
