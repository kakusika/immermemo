//! Which vault folders the user has linked, and which one is current --
//! properties of the *app*, not of any one vault, so these live at the
//! app-data root rather than under any vault's own directory.

use std::path::{Path, PathBuf};

use super::{AppData, read_trimmed, write};

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

    /// Adds `vault_dir` to the known list, if it isn't there already.
    /// `vault_dir` must already exist -- its canonical form is what
    /// dedupes and keys its private files, and canonicalizing a path that
    /// isn't there yet fails.
    pub fn add_vault(&self, vault_dir: &Path) -> anyhow::Result<()> {
        let mut vaults = self.known_vaults();
        if vaults.iter().any(|v| v == vault_dir) {
            return Ok(());
        }
        vaults.push(vault_dir.to_owned());
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
        vaults.retain(|v| v != vault_dir && v.canonicalize().ok() != vault_dir.canonicalize().ok());
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
        if let Ok(data_dir) = self.vault_data_dir(vault_dir) {
            let _ = std::fs::remove_dir_all(&data_dir);
        }

        // If the removed vault was the current vault, update or clear current_vault.txt
        if self.load_current_vault().as_deref() == Some(vault_dir)
            || self
                .load_current_vault()
                .and_then(|v| v.canonicalize().ok())
                == vault_dir.canonicalize().ok()
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
