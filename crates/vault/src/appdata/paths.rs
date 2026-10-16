//! Where a given vault's own private files live under the app-data root
//! (`base/vaults/<key>/`), keyed by the vault's path so two linked folders
//! never collide.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::AppData;

impl AppData {
    /// The folder name of a vault's private data: its last path component
    /// (sanitized, to tell the folders apart at a glance) and 12 hex digits of
    /// the SHA-256 of its path (to tell them apart for certain). The path is
    /// the canonical one while the folder exists; a vault whose folder has
    /// gone missing (an unmounted drive) keeps the path it was registered
    /// with, which [`AppData::add_vault`] made the canonical one -- so its key
    /// does not change and its data can still be found, and wiped.
    pub(super) fn vault_key(vault_dir: &Path) -> String {
        let path = vault_dir
            .canonicalize()
            .unwrap_or_else(|_| vault_dir.to_owned());
        let digest = Sha256::digest(path.to_string_lossy().as_bytes());
        let hash: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
        let name: String = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(40)
            .collect();
        format!("{name}-{hash}")
    }

    pub(super) fn vault_data_dir(&self, vault_dir: &Path) -> PathBuf {
        self.base.join("vaults").join(Self::vault_key(vault_dir))
    }

    /// The git directory for the vault at `vault_dir`.
    ///
    /// If `vault_dir` contains an in-tree `.git` (e.g. from an external `git clone`
    /// or `git init`), that directory is used directly so external tools (VS Code,
    /// git CLI, Obsidian) see changes. Otherwise, falls back to this app's private
    /// bare git directory outside the notes folder.
    pub fn gitdir(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        let in_tree = vault_dir.join(".git");
        if in_tree.exists() {
            return Ok(in_tree);
        }
        Ok(self.vault_data_dir(vault_dir).join("git"))
    }

    /// Where that vault's access token lives -- the caller decides how
    /// (`credentials::PlainFileTokenStore` on desktop,
    /// `android_keystore::AndroidKeystoreTokenStore` on Android), this just
    /// says where.
    pub fn token_path(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.vault_data_dir(vault_dir).join("token"))
    }

    /// Where that vault's SQLite search and metadata index lives.
    pub fn index_path(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.vault_data_dir(vault_dir).join("index.db"))
    }
}
