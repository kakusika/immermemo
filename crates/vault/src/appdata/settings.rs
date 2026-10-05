//! Small per-vault (remote URL, last-opened note) and per-install (editor
//! font size, theme, status bar visibility while editing) settings --
//! each just a trimmed-text file under the app-data root, read and
//! written independently of each other.

use std::path::{Path, PathBuf};

use super::{AppData, read_trimmed, write};

impl AppData {
    fn remote_file(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.vault_data_dir(vault_dir)?.join("remote.txt"))
    }

    /// The remote URL isn't secret -- it's kept as plain text, separately
    /// from wherever the access token lives.
    ///
    /// If no URL has been saved in appdata yet, but `vault_dir` has an in-tree
    /// git repository with an `origin` remote, returns that URL.
    pub fn load_remote(&self, vault_dir: &Path) -> Option<String> {
        if let Some(url) = self
            .remote_file(vault_dir)
            .ok()
            .and_then(|p| read_trimmed(&p))
        {
            return Some(url);
        }
        let in_tree = vault_dir.join(".git");
        if in_tree.exists() {
            if let Ok(repo) = git2::Repository::open(vault_dir) {
                if let Ok(remote) = repo.find_remote("origin") {
                    if let Some(url) = remote.url().map(str::trim).filter(|s| !s.is_empty()) {
                        return Some(url.to_string());
                    }
                }
            }
        }
        None
    }

    pub fn save_remote(&self, vault_dir: &Path, url: &str) -> anyhow::Result<()> {
        write(&self.remote_file(vault_dir)?, url)?;
        let in_tree = vault_dir.join(".git");
        if in_tree.exists() {
            if let Ok(repo) = git2::Repository::open(vault_dir) {
                if repo.find_remote("origin").is_ok() {
                    let _ = repo.remote_set_url("origin", url);
                } else {
                    let _ = repo.remote("origin", url);
                }
            }
        }
        Ok(())
    }

    fn last_note_file(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.vault_data_dir(vault_dir)?.join("last_note.txt"))
    }

    pub fn load_last_note(&self, vault_dir: &Path) -> Option<PathBuf> {
        self.last_note_file(vault_dir)
            .ok()
            .and_then(|p| read_trimmed(&p))
            .map(PathBuf::from)
    }

    pub fn save_last_note(&self, vault_dir: &Path, note_path: &Path) -> anyhow::Result<()> {
        write(
            &self.last_note_file(vault_dir)?,
            &note_path.display().to_string(),
        )
    }

    pub fn clear_last_note(&self, vault_dir: &Path) {
        if let Ok(file) = self.last_note_file(vault_dir) {
            let _ = std::fs::remove_file(file);
        }
    }

    fn font_size_file(&self) -> PathBuf {
        self.base.join("font_size.txt")
    }

    pub fn load_font_size(&self) -> Option<i32> {
        read_trimmed(&self.font_size_file()).and_then(|s| s.parse().ok())
    }

    pub fn save_font_size(&self, choice: i32) -> anyhow::Result<()> {
        write(&self.font_size_file(), &choice.to_string())
    }

    fn theme_file(&self) -> PathBuf {
        self.base.join("theme.txt")
    }

    pub fn load_theme(&self) -> Option<i32> {
        read_trimmed(&self.theme_file()).and_then(|s| s.parse().ok())
    }

    pub fn save_theme(&self, choice: i32) -> anyhow::Result<()> {
        write(&self.theme_file(), &choice.to_string())
    }

    fn status_bar_file(&self) -> PathBuf {
        self.base.join("status_bar.txt")
    }

    /// 0: show (default, even while editing), 1: hide while editing a note.
    pub fn load_status_bar_choice(&self) -> Option<i32> {
        read_trimmed(&self.status_bar_file()).and_then(|s| s.parse().ok())
    }

    pub fn save_status_bar_choice(&self, choice: i32) -> anyhow::Result<()> {
        write(&self.status_bar_file(), &choice.to_string())
    }
}
