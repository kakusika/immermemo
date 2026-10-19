//! Keeping a vault in the device's shared storage (Android), and what that
//! asks of the platform.
//!
//! A vault is a plain folder the app reads with `std::fs` and syncs with
//! libgit2, so it needs a *path*. Android's folder picker answers with a tree
//! URI instead (`content://com.android.externalstorage.documents/tree/
//! primary%3ADocuments`); [`path_from_tree_uri`] turns the ones that name a
//! folder on the device's own storage into the path the folder has under
//! `/storage`. Using that path takes the "all files access" permission, which
//! [`android::storage_access`](super::android) asks about and requests.
//!
//! The URI conversion is plain string work and runs (and is tested) on every
//! platform; only the JNI calls are Android's.

// The URI conversion is only called on Android (the picker's answer), but it is
// plain string work that is built and tested everywhere.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::path::{Component, Path, PathBuf};

/// Whether this device offers a folder picker (and needs the permission
/// below): Android does; on desktop a folder is typed as a path.
pub const CAN_PICK_FOLDER: bool = cfg!(target_os = "android");

#[cfg(target_os = "android")]
pub use super::android::storage_access::{
    has_all_files_access, pick_folder, request_all_files_access,
};

/// Whether the app may use shared storage by path: always, off Android.
#[cfg(not(target_os = "android"))]
pub fn has_all_files_access() -> bool {
    true
}

#[cfg(not(target_os = "android"))]
pub fn request_all_files_access() {}

#[cfg(not(target_os = "android"))]
pub fn pick_folder() {}

/// The system's own storage provider: the only one whose folders are folders
/// on the device (Google Drive, Downloads and the like are not).
const EXTERNAL_STORAGE_AUTHORITY: &str = "com.android.externalstorage.documents";

/// The primary volume (the phone's internal "shared storage") as a path.
const PRIMARY_VOLUME: &str = "/storage/emulated/0";

/// Decodes `%XX` escapes (UTF-8). `None` for a malformed escape or bytes that
/// are not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The path of the folder a picker's tree URI names, if it names a folder on
/// the device's own storage (the phone's shared storage or an SD card).
///
/// `None` for another provider's URI, a malformed one, or a path that tries
/// to climb out with `..`.
pub fn path_from_tree_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("content://")?;
    let (authority, path) = rest.split_once('/')?;
    if authority != EXTERNAL_STORAGE_AUTHORITY {
        return None;
    }
    // `tree/<id>` for the folder picked; a subfolder reached from it adds
    // `/document/<id>`, which is the more specific one.
    let mut segments = path.split('/');
    let mut id = None;
    while let Some(kind) = segments.next() {
        match (kind, segments.next()) {
            ("tree" | "document", Some(value)) => id = Some(value),
            _ => return None,
        }
    }
    let decoded = percent_decode(id?)?;
    let (volume, relative) = decoded.split_once(':')?;

    let mut path = match volume {
        "primary" => PathBuf::from(PRIMARY_VOLUME),
        // The "Documents" shortcut the system lists as a root of its own.
        "home" => Path::new(PRIMARY_VOLUME).join("Documents"),
        // An SD card or USB drive: its volume id, like `1234-ABCD`.
        id if !id.is_empty() && !id.contains(['/', '\\']) && id != "raw" => {
            Path::new("/storage").join(id)
        }
        _ => return None,
    };
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            // `..`, a root or a prefix: not a folder inside this volume.
            _ => return None,
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "content://com.android.externalstorage.documents";

    fn path(uri: &str) -> Option<String> {
        path_from_tree_uri(uri).map(|p| p.display().to_string())
    }

    #[test]
    fn a_folder_on_the_phones_shared_storage_is_a_path_under_storage_emulated() {
        assert_eq!(
            path(&format!("{BASE}/tree/primary%3ADocuments%2FNotes")).as_deref(),
            Some("/storage/emulated/0/Documents/Notes")
        );
        // The root of the shared storage.
        assert_eq!(
            path(&format!("{BASE}/tree/primary%3A")).as_deref(),
            Some("/storage/emulated/0")
        );
        // Unencoded colon and slash are accepted too.
        assert_eq!(
            path(&format!("{BASE}/tree/primary:Documents/Notes")).as_deref(),
            None,
            "a raw slash would be another URI segment, not part of the id"
        );
        assert_eq!(
            path(&format!("{BASE}/tree/primary:Notes")).as_deref(),
            Some("/storage/emulated/0/Notes")
        );
    }

    #[test]
    fn spaces_and_non_ascii_names_survive() {
        assert_eq!(
            path(&format!(
                "{BASE}/tree/primary%3ADocuments%2FMy%20notes%2F%E6%97%A5%E8%A8%98"
            ))
            .as_deref(),
            Some("/storage/emulated/0/Documents/My notes/日記")
        );
    }

    #[test]
    fn a_folder_on_an_sd_card_is_a_path_under_its_volume() {
        assert_eq!(
            path(&format!("{BASE}/tree/1234-ABCD%3ANotes")).as_deref(),
            Some("/storage/1234-ABCD/Notes")
        );
        assert_eq!(
            path(&format!("{BASE}/tree/1234-ABCD%3A")).as_deref(),
            Some("/storage/1234-ABCD")
        );
    }

    #[test]
    fn the_documents_shortcut_is_the_documents_folder() {
        assert_eq!(
            path(&format!("{BASE}/tree/home%3Anotes")).as_deref(),
            Some("/storage/emulated/0/Documents/notes")
        );
    }

    #[test]
    fn a_subfolder_reached_from_the_picked_one_uses_the_more_specific_id() {
        assert_eq!(
            path(&format!(
                "{BASE}/tree/primary%3ADocuments/document/primary%3ADocuments%2FNotes%2F2026"
            ))
            .as_deref(),
            Some("/storage/emulated/0/Documents/Notes/2026")
        );
    }

    #[test]
    fn what_is_not_a_folder_on_the_device_is_refused() {
        for uri in [
            "content://com.android.providers.downloads.documents/tree/downloads",
            "content://com.google.android.apps.docs.storage/tree/abc",
            "file:///storage/emulated/0/Notes",
            "https://example.com/tree/primary%3ANotes",
            "",
            "content://com.android.externalstorage.documents",
            "content://com.android.externalstorage.documents/tree/",
            "content://com.android.externalstorage.documents/tree/noColon",
            "content://com.android.externalstorage.documents/tree/raw%3A%2Fdata",
            "content://com.android.externalstorage.documents/tree/primary%3A..%2F..%2Fdata",
            "content://com.android.externalstorage.documents/tree/primary%3A%2Fabs",
            "content://com.android.externalstorage.documents/tree/primary%3A%ZZ",
            "content://com.android.externalstorage.documents/tree/%2E%2E%2Fx%3Ay",
        ] {
            assert_eq!(path_from_tree_uri(uri), None, "{uri:?}");
        }
    }
}
