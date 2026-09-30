//! Authentication token storage abstraction and plain-file implementation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub trait TokenStore: Send + Sync {
    fn load(&self) -> anyhow::Result<Option<String>>;
    fn save(&self, token: &str) -> anyhow::Result<()>;
}

/// So an `Arc<dyn TokenStore>` -- how the app shares one store between threads
/// -- is itself a `TokenStore`.
impl<T: TokenStore + ?Sized> TokenStore for Arc<T> {
    fn load(&self) -> anyhow::Result<Option<String>> {
        (**self).load()
    }

    fn save(&self, token: &str) -> anyhow::Result<()> {
        (**self).save(token)
    }
}

pub type TokenStoreFactory = Box<dyn Fn(&Path) -> anyhow::Result<Arc<dyn TokenStore>>>;

/// A token kept in a plain file. Used on desktop, where nothing wires this
/// crate up to the OS keychain yet -- fine for trying the sync loop out,
/// not for anything real. Android uses the Android Keystore instead.
pub struct PlainFileTokenStore {
    path: PathBuf,
}

impl PlainFileTokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
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
