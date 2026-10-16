//! Small per-vault (remote URL, last-opened note) and per-install (editor
//! font size, theme, status bar visibility while editing) settings --
//! each just a trimmed-text file under the app-data root, read and
//! written independently of each other.

use std::path::{Path, PathBuf};

use super::{AppData, read_trimmed, write};

/// Declares per-install integer settings, each a trimmed-text file directly
/// under the app-data root: `load_*` is `None` until first saved (or if the
/// file is unreadable), `save_*` overwrites it.
macro_rules! int_setting {
    ($($(#[$doc:meta])* $file:literal, $load:ident, $save:ident;)*) => {$(
        $(#[$doc])*
        pub fn $load(&self) -> Option<i32> {
            read_trimmed(&self.base.join($file)).and_then(|s| s.parse().ok())
        }

        pub fn $save(&self, value: i32) -> anyhow::Result<()> {
            write(&self.base.join($file), &value.to_string())
        }
    )*};
}

/// [`int_setting!`] for free-form text.
macro_rules! text_setting {
    ($($(#[$doc:meta])* $file:literal, $load:ident, $save:ident;)*) => {$(
        $(#[$doc])*
        pub fn $load(&self) -> Option<String> {
            read_trimmed(&self.base.join($file))
        }

        pub fn $save(&self, value: &str) -> anyhow::Result<()> {
            write(&self.base.join($file), value)
        }
    )*};
}

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
        if in_tree.exists()
            && let Ok(repo) = git2::Repository::open(vault_dir)
            && let Ok(remote) = repo.find_remote("origin")
            && let Some(url) = remote.url().map(str::trim).filter(|s| !s.is_empty())
        {
            return Some(url.to_string());
        }
        None
    }

    pub fn save_remote(&self, vault_dir: &Path, url: &str) -> anyhow::Result<()> {
        write(&self.remote_file(vault_dir)?, url)?;
        let in_tree = vault_dir.join(".git");
        if in_tree.exists()
            && let Ok(repo) = git2::Repository::open(vault_dir)
        {
            if repo.find_remote("origin").is_ok() {
                let _ = repo.remote_set_url("origin", url);
            } else {
                let _ = repo.remote("origin", url);
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

    int_setting! {
        /// 0: small, 1: medium (default), 2: large.
        "font_size.txt", load_font_size, save_font_size;
        /// 0: follow the system, 1: light, 2: dark.
        "theme.txt", load_theme, save_theme;
        /// 0: show (default, even while editing), 1: hide while editing a note,
        /// 2: hide everywhere.
        "status_bar.txt", load_status_bar_choice, save_status_bar_choice;
        /// 0: system wallpaper / dynamic color, 1: custom color & style
        "color_source.txt", load_color_source, save_color_source;
        /// 0: TonalSpot, 1: Vivid, 2: Expressive, 3: FruitSalad, 4: Rainbow, 5: Spritz, 6: Monochrome
        "palette_style.txt", load_palette_style, save_palette_style;
        /// 0: Standard (0.0), 1: Medium (0.5), 2: High (1.0)
        "contrast_level.txt", load_contrast_level, save_contrast_level;
    }

    text_setting! {
        /// The comma-separated bottom navigation bar items, in order.
        "bottom_bar.txt", load_bottom_bar_config, save_bottom_bar_config;
        /// The accent seed color as `#RRGGBB`.
        "color_seed.txt", load_color_seed, save_color_seed;
    }
}
