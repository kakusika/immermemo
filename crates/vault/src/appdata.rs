//! Where a vault's private files live: outside the notes folder (which the
//! user's file provider may sync), keyed by the vault's own path so more
//! than one linked folder never collides. Everything under here is this
//! app's own data.
//!
//! Each vault's directory (`base/vaults/<key>/`) holds three things that
//! belong to that vault specifically, and nothing else: its bare git
//! directory, its remote URL, and its access token -- switching vaults
//! switches all three together, since a different vault may point at a
//! completely different host. Which vaults exist at all, and which one is
//! current, are the only things kept at the top level instead: they're
//! properties of the *app*, not of any one vault.

use std::path::{Path, PathBuf};

pub struct AppData {
    base: PathBuf,
}

impl AppData {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    /// The app-data root itself -- for a platform that needs to compute a
    /// new vault's path relative to it (Android: a subfolder name).
    pub fn base(&self) -> &Path {
        &self.base
    }

    fn vault_key(vault_dir: &Path) -> anyhow::Result<String> {
        let canonical = vault_dir.canonicalize()?;
        Ok(canonical.to_string_lossy().replace(['/', '\\'], "_"))
    }

    fn vault_data_dir(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
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

    fn remote_file(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.vault_data_dir(vault_dir)?.join("remote.txt"))
    }

    /// The remote URL isn't secret -- it's kept as plain text, separately
    /// from wherever the access token lives.
    ///
    /// If no URL has been saved in appdata yet, but `vault_dir` has an in-tree
    /// git repository with an `origin` remote, returns that URL.
    pub fn load_remote(&self, vault_dir: &Path) -> Option<String> {
        if let Some(url) = self.remote_file(vault_dir).ok().and_then(|p| read_trimmed(&p)) {
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

    fn vault_list_file(&self) -> PathBuf {
        self.base.join("vaults.txt")
    }

    /// Every vault folder the user has linked, in the order they were
    /// added. Not a property of any one vault -- kept at the app level.
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
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn write(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_url_round_trips_and_starts_unset_per_vault() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        assert_eq!(data.load_remote(a.path()), None);

        data.save_remote(a.path(), "https://example.invalid/a.git")
            .unwrap();
        assert_eq!(
            data.load_remote(a.path()),
            Some("https://example.invalid/a.git".to_owned())
        );
        // A different vault's remote is independent.
        assert_eq!(data.load_remote(b.path()), None);
    }

    #[test]
    fn each_vault_gets_its_own_gitdir_and_token_path() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let a = data.gitdir(one.path()).unwrap();
        let b = data.gitdir(two.path()).unwrap();
        assert_ne!(a, b);
        assert_eq!(a, data.gitdir(one.path()).unwrap());
        assert!(a.starts_with(dir.path().join("appdata")));

        assert_ne!(
            data.token_path(one.path()).unwrap(),
            data.token_path(two.path()).unwrap()
        );
        // Not nested inside the bare gitdir: that directory belongs to
        // libgit2, nothing else should write into it.
        assert!(!data.token_path(one.path()).unwrap().starts_with(&a));
    }

    #[test]
    fn known_vaults_starts_empty_and_add_vault_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.known_vaults(), Vec::<PathBuf>::new());

        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        data.add_vault(a.path()).unwrap();
        data.add_vault(b.path()).unwrap();
        data.add_vault(a.path()).unwrap(); // already known, no duplicate
        assert_eq!(
            data.known_vaults(),
            vec![a.path().to_owned(), b.path().to_owned()]
        );
    }

    #[test]
    fn remove_vault_drops_from_list_and_wipes_private_data() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        data.add_vault(a.path()).unwrap();
        data.add_vault(b.path()).unwrap();
        data.save_remote(a.path(), "https://example.invalid/a.git")
            .unwrap();
        data.save_current_vault(a.path()).unwrap();

        let a_data_dir = data.vault_data_dir(a.path()).unwrap();
        assert!(a_data_dir.exists());

        data.remove_vault(a.path()).unwrap();
        assert_eq!(data.known_vaults(), vec![b.path().to_owned()]);
        assert!(!a_data_dir.exists());
        // Current vault switched to b.
        assert_eq!(data.load_current_vault(), Some(b.path().to_owned()));
        // The notes directory itself remains untouched.
        assert!(a.path().exists());
    }

    #[test]
    fn current_vault_round_trips_and_starts_unset() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_current_vault(), None);
        let vault = tempfile::tempdir().unwrap();
        data.save_current_vault(vault.path()).unwrap();
        assert_eq!(data.load_current_vault(), Some(vault.path().to_owned()));
    }

    #[test]
    fn last_note_round_trips_and_starts_unset() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let vault = tempfile::tempdir().unwrap();
        assert_eq!(data.load_last_note(vault.path()), None);

        let note = vault.path().join("my-note.tmt");
        data.save_last_note(vault.path(), &note).unwrap();
        assert_eq!(data.load_last_note(vault.path()), Some(note));
    }

    #[test]
    fn last_note_can_be_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let vault = tempfile::tempdir().unwrap();
        let note = vault.path().join("my-note.tmt");
        data.save_last_note(vault.path(), &note).unwrap();
        assert_eq!(data.load_last_note(vault.path()), Some(note));

        data.clear_last_note(vault.path());
        assert_eq!(data.load_last_note(vault.path()), None);
    }

    #[test]
    fn font_size_and_theme_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_font_size(), None);
        assert_eq!(data.load_theme(), None);

        data.save_font_size(2).unwrap();
        data.save_theme(1).unwrap();
        assert_eq!(data.load_font_size(), Some(2));
        assert_eq!(data.load_theme(), Some(1));
    }

    #[test]
    fn in_tree_gitdir_is_used_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        // Without .git, gitdir points inside appdata.
        let default_gitdir = data.gitdir(vault.path()).unwrap();
        assert!(default_gitdir.starts_with(dir.path().join("appdata")));

        // With in-tree .git, gitdir points to vault's .git.
        let in_tree_git = vault.path().join(".git");
        std::fs::create_dir(&in_tree_git).unwrap();
        assert_eq!(data.gitdir(vault.path()).unwrap(), in_tree_git);
    }

    #[test]
    fn in_tree_remote_is_detected_and_updated() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        // Initialize a standard git repo inside vault
        let repo = git2::Repository::init(vault.path()).unwrap();
        repo.remote("origin", "https://github.com/example/notes.git")
            .unwrap();

        // Automatically detected without remote.txt
        assert_eq!(
            data.load_remote(vault.path()),
            Some("https://github.com/example/notes.git".to_string())
        );

        // Updating via save_remote updates both remote.txt and .git config
        data.save_remote(vault.path(), "https://github.com/example/updated.git")
            .unwrap();
        assert_eq!(
            data.load_remote(vault.path()),
            Some("https://github.com/example/updated.git".to_string())
        );
        let updated_repo = git2::Repository::open(vault.path()).unwrap();
        let remote = updated_repo.find_remote("origin").unwrap();
        assert_eq!(remote.url(), Some("https://github.com/example/updated.git"));
    }
}
