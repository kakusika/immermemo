//! Where a remote's access token comes from, turned into git credentials.
//!
//! Which username/password pair a host wants the token in is
//! `immermemo_provider`'s knowledge (`https_basic_auth_for_remote`); this
//! module only fetches the token and hands it over.
//!
//! An `ssh://` remote uses the in-app Ed25519 keypair if present, falling
//! back to desktop ssh-agent when available.

use immermemo_credentials::{SshKeyStore, TokenStore};
use immermemo_provider::https_basic_auth_for_remote;
use immermemo_sync::CredentialProvider;
use immermemo_vault::appdata::AppData;

/// Credentials backed by whatever [`TokenStore`] the platform provides,
/// plus optional in-app SSH keys stored in [`AppData`].
pub struct TokenCredentials<S> {
    store: S,
    ssh_store: Option<SshKeyStore>,
}

impl<S: TokenStore> TokenCredentials<S> {
    pub fn new(store: S) -> Self {
        Self {
            store,
            ssh_store: None,
        }
    }

    pub fn with_app_data(store: S, app_data: AppData) -> Self {
        Self {
            store,
            ssh_store: Some(app_data.ssh_store()),
        }
    }

    pub fn with_ssh_store(store: S, ssh_store: SshKeyStore) -> Self {
        Self {
            store,
            ssh_store: Some(ssh_store),
        }
    }
}

impl<S: TokenStore> CredentialProvider for TokenCredentials<S> {
    fn credentials(&self, remote_url: &str) -> anyhow::Result<git2::Cred> {
        let is_ssh = remote_url.starts_with("ssh://")
            || (!remote_url.starts_with("http") && remote_url.contains('@'));

        if is_ssh {
            // 1. Prefer in-app SSH key from AppData if available
            if let Some(ssh_store) = &self.ssh_store
                && let Some(priv_key) = ssh_store.load_private_key()
            {
                let pub_key = ssh_store.load_public_key();
                return Ok(git2::Cred::ssh_key_from_memory(
                    "git",
                    pub_key.as_deref(),
                    &priv_key,
                    None,
                )?);
            }

            // 2. Fall back to desktop ssh-agent if available
            #[cfg(not(target_os = "android"))]
            {
                return Ok(git2::Cred::ssh_key_from_agent("git")?);
            }
            #[cfg(target_os = "android")]
            {
                anyhow::bail!(
                    "No in-app SSH key generated. Please generate an SSH key in Settings to sync via SSH."
                );
            }
        }

        let token = self
            .store
            .load()?
            .ok_or_else(|| anyhow::anyhow!("no access token saved for this vault's remote"))?;

        let (username, password) = https_basic_auth_for_remote(remote_url, &token);
        Ok(git2::Cred::userpass_plaintext(&username, &password)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use immermemo_credentials::PlainFileTokenStore;

    #[test]
    fn ssh_credentials_uses_in_app_key_when_available() {
        let dir = tempfile::tempdir().unwrap();
        let app_data = AppData::new(dir.path().to_owned());
        let token_store = PlainFileTokenStore::new(dir.path().join("token"));

        let creds = TokenCredentials::with_app_data(token_store, app_data.clone());
        assert!(!app_data.has_ssh_key());

        let pubkey = app_data.ensure_ssh_key().unwrap();
        assert!(pubkey.starts_with("ssh-ed25519 "));

        let cred = creds.credentials("git@github.com:alice/notes.git");
        assert!(cred.is_ok());

        let cred_ssh_url = creds.credentials("ssh://git@codeberg.org/alice/notes.git");
        assert!(cred_ssh_url.is_ok());
    }

    #[test]
    fn https_credentials_uses_token_store() {
        let dir = tempfile::tempdir().unwrap();
        let app_data = AppData::new(dir.path().to_owned());
        let token_store = PlainFileTokenStore::new(dir.path().join("token"));
        token_store.save("my_pat_secret").unwrap();

        let creds = TokenCredentials::with_app_data(token_store, app_data);
        assert!(
            creds
                .credentials("https://github.com/alice/notes.git")
                .is_ok()
        );
        assert!(
            creds
                .credentials("https://gitlab.com/alice/notes.git")
                .is_ok()
        );
    }
}
