//! Where a given vault's own private files live under the app-data root
//! (`base/vaults/<key>/`), keyed by the vault's canonicalized path so two
//! linked folders never collide.

use std::path::{Path, PathBuf};

use super::AppData;

impl AppData {
    pub(super) fn vault_key(vault_dir: &Path) -> anyhow::Result<String> {
        let canonical = vault_dir.canonicalize()?;
        Ok(canonical.to_string_lossy().replace(['/', '\\'], "_"))
    }

    pub(super) fn vault_data_dir(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.base.join("vaults").join(Self::vault_key(vault_dir)?))
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
        Ok(self.vault_data_dir(vault_dir)?.join("git"))
    }

    /// Where that vault's access token lives -- the caller decides how
    /// (`credentials::PlainFileTokenStore` on desktop,
    /// `android_keystore::AndroidKeystoreTokenStore` on Android), this just
    /// says where.
    pub fn token_path(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.vault_data_dir(vault_dir)?.join("token"))
    }

    /// Where that vault's SQLite search and metadata index lives.
    pub fn index_path(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.vault_data_dir(vault_dir)?.join("index.db"))
    }
}
