use immermemo_credentials::SshKeyStore;

use super::AppData;

impl AppData {
    /// Returns the SSH key store managed within this app data directory.
    pub fn ssh_store(&self) -> SshKeyStore {
        SshKeyStore::new(self.base.join("ssh"))
    }

    /// Loads the stored SSH public key if present.
    pub fn load_ssh_public_key(&self) -> Option<String> {
        self.ssh_store().load_public_key()
    }

    /// Loads the stored SSH private key if present.
    pub fn load_ssh_private_key(&self) -> Option<String> {
        self.ssh_store().load_private_key()
    }

    /// Generates a new SSH Ed25519 keypair, overwriting any existing key, and returns the public key.
    pub fn generate_ssh_key(&self, comment: Option<&str>) -> anyhow::Result<String> {
        self.ssh_store().generate_key(comment)
    }

    /// Returns the existing public key, or generates a new keypair if not present.
    pub fn ensure_ssh_key(&self) -> anyhow::Result<String> {
        self.ssh_store().ensure_key()
    }

    /// Returns true if an SSH keypair is present.
    pub fn has_ssh_key(&self) -> bool {
        self.ssh_store().has_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appdata_ssh_delegation() {
        let dir = tempfile::tempdir().unwrap();
        let app_data = AppData::new(dir.path().to_owned());

        assert!(!app_data.has_ssh_key());
        assert!(app_data.load_ssh_public_key().is_none());
        assert!(app_data.load_ssh_private_key().is_none());

        let pubkey = app_data.ensure_ssh_key().unwrap();
        assert!(app_data.has_ssh_key());
        assert_eq!(app_data.load_ssh_public_key(), Some(pubkey.clone()));
        assert!(app_data.load_ssh_private_key().is_some());

        let regenerated = app_data.generate_ssh_key(Some("custom-comment")).unwrap();
        assert_ne!(regenerated, pubkey);
        assert!(regenerated.ends_with(" custom-comment"));
        assert_eq!(app_data.load_ssh_public_key(), Some(regenerated));
    }
}
