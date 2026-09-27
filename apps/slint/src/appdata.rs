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

    /// The private (bare) git directory for the vault at `vault_dir`.
    pub fn gitdir(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
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
    pub fn load_remote(&self, vault_dir: &Path) -> Option<String> {
        self.remote_file(vault_dir)
            .ok()
            .and_then(|p| read_trimmed(&p))
    }

    pub fn save_remote(&self, vault_dir: &Path, url: &str) -> anyhow::Result<()> {
        write(&self.remote_file(vault_dir)?, url)
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
    fn current_vault_round_trips_and_starts_unset() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_current_vault(), None);
        let vault = tempfile::tempdir().unwrap();
        data.save_current_vault(vault.path()).unwrap();
        assert_eq!(data.load_current_vault(), Some(vault.path().to_owned()));
    }
}
