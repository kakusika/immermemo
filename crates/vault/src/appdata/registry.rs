//! Which vault folders the user has linked, and which one is current --
//! properties of the *app*, not of any one vault, so these live at the
//! app-data root rather than under any vault's own directory.

use std::path::{Path, PathBuf};

use super::{AppData, read_trimmed, write};

/// Whether `a` and `b` are the same vault folder: the same path, or two paths
/// that both exist and lead to one folder. Two paths that no longer exist are
/// not the same vault merely because neither can be resolved.
fn same_vault(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize(), b.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        )
}

impl AppData {
    fn vault_list_file(&self) -> PathBuf {
        self.base.join("vaults.txt")
    }

    /// Every vault folder the user has linked, in the order they were
    /// added.
    pub fn known_vaults(&self) -> Vec<PathBuf> {
        std::fs::read_to_string(self.vault_list_file())
            .map(|s| s.lines().map(PathBuf::from).collect())
            .unwrap_or_default()
    }

    /// Adds `vault_dir` to the known list, if it isn't there already -- as
    /// its canonical path when the folder exists, so that one folder is one
    /// spelling however it was reached (a symlink, a `..`), and its private
    /// files keep the same key if the folder later goes missing.
    pub fn add_vault(&self, vault_dir: &Path) -> anyhow::Result<()> {
        let vault_dir = vault_dir
            .canonicalize()
            .unwrap_or_else(|_| vault_dir.to_owned());
        let mut vaults = self.known_vaults();
        if vaults.iter().any(|v| same_vault(v, &vault_dir)) {
            return Ok(());
        }
        vaults.push(vault_dir);
        let content = vaults
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        write(&self.vault_list_file(), &content)
    }

    /// Removes `vault_dir` from the known list, wipes its private data
    /// directory, and updates `current_vault.txt` if needed. Does not touch
    /// the user's note files.
    pub fn remove_vault(&self, vault_dir: &Path) -> anyhow::Result<()> {
        let mut vaults = self.known_vaults();
        let len_before = vaults.len();
        vaults.retain(|v| !same_vault(v, vault_dir));
        if vaults.len() == len_before {
            return Ok(());
        }
        let content = vaults
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        write(&self.vault_list_file(), &content)?;

        // Wipe that vault's private data (git, remote.txt, token)
        let _ = std::fs::remove_dir_all(self.vault_data_dir(vault_dir));

        // If the removed vault was the current vault, update or clear current_vault.txt
        if self
            .load_current_vault()
            .is_some_and(|current| same_vault(&current, vault_dir))
        {
            if let Some(first) = vaults.first() {
                let _ = self.save_current_vault(first);
            } else {
                let _ = std::fs::remove_file(self.current_vault_file());
            }
        }
        Ok(())
    }

    fn current_vault_file(&self) -> PathBuf {
        self.base.join("current_vault.txt")
    }

    pub fn load_current_vault(&self) -> Option<PathBuf> {
        read_trimmed(&self.current_vault_file()).map(PathBuf::from)
    }

    pub fn save_current_vault(&self, vault_dir: &Path) -> anyhow::Result<()> {
        write(&self.current_vault_file(), &vault_dir.display().to_string())
    }
}
