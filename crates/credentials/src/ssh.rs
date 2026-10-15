//! In-app SSH key generation and storage (Ed25519).
//!
//! Stores and manages an app-level OpenSSH Ed25519 key pair:
//! - `id_ed25519`: private key (OpenSSH format, 0600 file permission on unix)
//! - `id_ed25519.pub`: public key (OpenSSH format, 0644 file permission on unix)

use std::path::{Path, PathBuf};

/// Manages an in-app Ed25519 SSH keypair stored on disk.
#[derive(Debug, Clone)]
pub struct SshKeyStore {
    dir: PathBuf,
}

impl SshKeyStore {
    /// Creates a new [`SshKeyStore`] operating on the specified directory.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The directory where the key files are stored.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Path to the OpenSSH private key file (`id_ed25519`).
    pub fn private_key_path(&self) -> PathBuf {
        self.dir.join("id_ed25519")
    }

    /// Path to the OpenSSH public key file (`id_ed25519.pub`).
    pub fn public_key_path(&self) -> PathBuf {
        self.dir.join("id_ed25519.pub")
    }

    /// Returns `true` if both the private and public key files exist.
    pub fn has_key(&self) -> bool {
        self.private_key_path().exists() && self.public_key_path().exists()
    }

    /// Reads and returns the public key string if present.
    pub fn load_public_key(&self) -> Option<String> {
        std::fs::read_to_string(self.public_key_path())
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    }

    /// Reads and returns the private key string if present.
    pub fn load_private_key(&self) -> Option<String> {
        std::fs::read_to_string(self.private_key_path())
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    }

    /// Generates a new Ed25519 keypair, writes both keys to disk with secure permissions,
    /// and returns the OpenSSH public key string.
    pub fn generate_key(&self, comment: Option<&str>) -> anyhow::Result<String> {
        let comment_str = comment.unwrap_or("immermemo");
        let key = ssh_key::PrivateKey::random(
            &mut ssh_key::rand_core::OsRng,
            ssh_key::Algorithm::Ed25519,
        )?;
        let priv_openssh = (*key.to_openssh(ssh_key::LineEnding::LF)?).to_owned();
        let mut pub_key = key.public_key().clone();
        pub_key.set_comment(comment_str);
        let pub_openssh = pub_key.to_openssh()?;

        let priv_path = self.private_key_path();
        let pub_path = self.public_key_path();

        if let Some(parent) = priv_path.parent() {
            std::fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }
        }

        std::fs::write(&priv_path, priv_openssh.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&priv_path, std::fs::Permissions::from_mode(0o600));
        }

        std::fs::write(&pub_path, pub_openssh.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&pub_path, std::fs::Permissions::from_mode(0o644));
        }

        Ok(pub_openssh)
    }

    /// Returns the existing public key, or generates a new keypair if not present.
    pub fn ensure_key(&self) -> anyhow::Result<String> {
        if let Some(pubkey) = self.load_public_key() {
            if self.private_key_path().exists() {
                return Ok(pubkey);
            }
        }
        self.generate_key(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_key_store_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SshKeyStore::new(tmp.path().join("ssh"));

        assert!(!store.has_key());
        assert!(store.load_public_key().is_none());
        assert!(store.load_private_key().is_none());

        let pubkey = store.ensure_key().unwrap();
        assert!(store.has_key());
        assert!(pubkey.starts_with("ssh-ed25519 "));
        assert!(pubkey.ends_with(" immermemo"));

        let loaded_pub = store.load_public_key().unwrap();
        assert_eq!(loaded_pub, pubkey);

        let privkey = store.load_private_key().unwrap();
        assert!(privkey.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(privkey.ends_with("-----END OPENSSH PRIVATE KEY-----"));

        // ensure_key returns the same key on second call without regenerating
        let pubkey_again = store.ensure_key().unwrap();
        assert_eq!(pubkey_again, pubkey);
    }
}
