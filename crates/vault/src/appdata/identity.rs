//! Where this device's own identity file lives -- an app-level concern
//! (one identity per install, used for every vault this app ever syncs),
//! not a per-vault one, same as [`super::registry`]'s `known_vaults`/
//! `current_vault`. The identity's own shape (`device_id`/
//! `display_name`, generation, validation) lives in the separate
//! `immermemo-identity` crate, which only ever takes a plain path -- this
//! is that path.

use std::path::PathBuf;

use super::AppData;

impl AppData {
    /// Where this device's `immermemo_identity::DeviceIdentity` is
    /// persisted. Not keyed by vault: switching vaults never changes
    /// which device is running this install.
    pub fn identity_path(&self) -> PathBuf {
        self.base.join("identity.txt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_path_is_app_level_not_per_vault() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.identity_path(), dir.path().join("identity.txt"));
        // Same path regardless of which vault is current -- there is no
        // vault_dir parameter to vary it by, unlike `gitdir`/`token_path`.
    }
}
