//! Where a vault's private files live: outside the notes folder (which the
//! user's file provider may sync), keyed by the vault's own path so more
//! than one linked folder never collides. Everything under here is this
//! app's own data.

use std::path::{Path, PathBuf};

pub struct AppData {
    base: PathBuf,
}

impl AppData {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    /// The private (bare) git directory for the vault at `vault_dir`.
    pub fn gitdir(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        let canonical = vault_dir.canonicalize()?;
        let key = canonical.to_string_lossy().replace(['/', '\\'], "_");
        Ok(self.base.join("vaults").join(key))
    }

    fn remote_file(&self) -> PathBuf {
        self.base.join("remote.txt")
    }

    /// The remote URL isn't secret -- it's kept as plain text, separately
    /// from wherever the access token lives.
    pub fn load_remote(&self) -> Option<String> {
        read_trimmed(&self.remote_file())
    }

    pub fn save_remote(&self, url: &str) -> anyhow::Result<()> {
        write(&self.remote_file(), url)
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
    fn remote_url_round_trips_and_starts_unset() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_remote(), None);
        data.save_remote("https://example.invalid/notes.git")
            .unwrap();
        assert_eq!(
            data.load_remote(),
            Some("https://example.invalid/notes.git".to_owned())
        );
    }

    #[test]
    fn each_vault_gets_its_own_gitdir() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let a = data.gitdir(one.path()).unwrap();
        let b = data.gitdir(two.path()).unwrap();
        assert_ne!(a, b);
        assert_eq!(a, data.gitdir(one.path()).unwrap());
        assert!(a.starts_with(dir.path().join("appdata")));
    }
}
