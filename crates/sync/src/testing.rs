//! Fixtures for tests of this crate and of crates built on it (enable the
//! `test-support` feature from a `[dev-dependencies]` entry).

use tempfile::TempDir;

use crate::CredentialProvider;

/// Credentials for a local-path remote, which never asks for any: calling
/// it means a test accidentally went through a network transport.
pub struct NoCredentials;

impl CredentialProvider for NoCredentials {
    fn credentials(&self, _remote_url: &str) -> anyhow::Result<git2::Cred> {
        anyhow::bail!("not expected to be called for a local-path transport")
    }
}

/// A bare repository standing in for a hosted git remote (GitHub, Gitea,
/// self-hosted): real fetch and push over git's own local transport, with no
/// network or hosting account involved.
pub fn shared_remote() -> TempDir {
    let dir = TempDir::new().unwrap();
    git2::Repository::init_bare(dir.path()).unwrap();
    dir
}
