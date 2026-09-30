//! Where a remote's access token comes from, turned into git credentials.
//!
//! GitHub, Gitea and Forgejo all accept a personal access token presented as
//! the password of an HTTPS basic auth, with the username left blank --
//! nothing here special-cases which of them a vault's remote points at.
//! An `ssh://` remote goes through the desktop's ssh-agent instead; Android
//! has no agent to ask, so an ssh remote there is not supported yet.

use immermemo_sync::CredentialProvider;
pub use immermemo_vault::credentials::{PlainFileTokenStore, TokenStore, TokenStoreFactory};

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
