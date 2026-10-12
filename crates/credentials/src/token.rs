//! Authentication token storage abstraction and plain-file implementation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Storage abstraction for personal access tokens.
pub trait TokenStore: Send + Sync {
    /// Loads the stored token string if present.
    fn load(&self) -> anyhow::Result<Option<String>>;

    /// Persists the provided token string.
    fn save(&self, token: &str) -> anyhow::Result<()>;
}

impl<T: TokenStore + ?Sized> TokenStore for Arc<T> {
    fn load(&self) -> anyhow::Result<Option<String>> {
        (**self).load()
    }

    fn save(&self, token: &str) -> anyhow::Result<()> {
        (**self).save(token)
    }
}

/// A factory for creating a [`TokenStore`] for a given vault path.
pub type TokenStoreFactory =
    Box<dyn Fn(&Path) -> anyhow::Result<Arc<dyn TokenStore>> + Send + Sync>;

/// A token kept in a plain text file.
/// Used on desktop or non-keystore environments.
#[derive(Debug, Clone)]
pub struct PlainFileTokenStore {
    path: PathBuf,
}

impl PlainFileTokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl TokenStore for PlainFileTokenStore {
    fn load(&self) -> anyhow::Result<Option<String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => Ok(Some(s.trim().to_owned()).filter(|s| !s.is_empty())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, token: &str) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, token)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_file_round_trips_and_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlainFileTokenStore::new(dir.path().join("token"));
        assert_eq!(store.load().unwrap(), None);
        store.save("ghp_example").unwrap();
        assert_eq!(store.load().unwrap(), Some("ghp_example".to_owned()));
    }
}
