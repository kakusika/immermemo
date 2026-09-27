//! Where a remote's access token comes from, turned into git credentials.
//!
//! GitHub, Gitea and Forgejo all accept a personal access token presented as
//! the password of an HTTPS basic auth, with the username left blank --
//! nothing here special-cases which of them a vault's remote points at.
//! An `ssh://` remote goes through the desktop's ssh-agent instead; Android
//! has no agent to ask, so an ssh remote there is not supported yet.

use std::path::PathBuf;
use std::sync::Arc;

use immermemo_sync::CredentialProvider;

pub trait TokenStore: Send + Sync {
    fn load(&self) -> anyhow::Result<Option<String>>;
    fn save(&self, token: &str) -> anyhow::Result<()>;
}

/// So an `Arc<dyn TokenStore>` -- how the window shares one store between the
/// UI thread and a sync running on its own thread -- is itself a `TokenStore`.
impl<T: TokenStore + ?Sized> TokenStore for Arc<T> {
    fn load(&self) -> anyhow::Result<Option<String>> {
        (**self).load()
    }

    fn save(&self, token: &str) -> anyhow::Result<()> {
        (**self).save(token)
    }
}

/// Credentials backed by whatever [`TokenStore`] the platform provides.
pub struct TokenCredentials<S> {
    store: S,
}

impl<S: TokenStore> TokenCredentials<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S: TokenStore> CredentialProvider for TokenCredentials<S> {
    fn credentials(&self, remote_url: &str) -> anyhow::Result<git2::Cred> {
        if remote_url.starts_with("ssh://")
            || (!remote_url.starts_with("http") && remote_url.contains('@'))
        {
            return Ok(git2::Cred::ssh_key_from_agent("git")?);
        }
        let token = self
            .store
            .load()?
            .ok_or_else(|| anyhow::anyhow!("no access token saved for this vault's remote"))?;
        Ok(git2::Cred::userpass_plaintext(&token, "")?)
    }
}

/// A token kept in a plain file. Used on desktop, where nothing wires this
/// crate up to the OS keychain yet -- fine for trying the sync loop out,
/// not for anything real. Android uses [`crate::android_keystore`] instead.
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
