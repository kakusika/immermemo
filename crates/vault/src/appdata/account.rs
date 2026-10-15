//! Provider accounts management: persisting authenticated hosting provider accounts
//! (GitHub, Forgejo, GitLab) and associating vaults with accounts.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{AppData, read_trimmed, write};

/// An authenticated Git hosting provider account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Unique identifier for this account (e.g. `github:octocat`, `gitlab:gitlab.com:alice`, or a UUID).
    pub id: String,
    /// Provider kind (`github`, `forgejo`, `gitlab`).
    pub provider: String,
    /// Hosting instance URL (e.g. `https://github.com`, `https://codeberg.org`, `https://gitlab.example.com`).
    pub instance_url: String,
    /// Account username or handle.
    pub username: String,
    /// Optional human-readable display name.
    pub display_name: Option<String>,
    /// Optional web avatar image URL.
    pub avatar_url: Option<String>,
}

impl AppData {
    fn accounts_file(&self) -> PathBuf {
        self.base.join("accounts.json")
    }

    /// Loads all registered provider accounts.
    pub fn load_accounts(&self) -> Vec<Account> {
        std::fs::read_to_string(self.accounts_file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persists the list of provider accounts.
    pub fn save_accounts(&self, accounts: &[Account]) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(accounts)?;
        write(&self.accounts_file(), &json)
    }

    /// Adds or updates a provider account by its `id`.
    pub fn add_account(&self, account: Account) -> anyhow::Result<()> {
        let mut accounts = self.load_accounts();
        if let Some(existing) = accounts.iter_mut().find(|a| a.id == account.id) {
            *existing = account;
        } else {
            accounts.push(account);
        }
        self.save_accounts(&accounts)
    }

    /// Removes an account by `id` and deletes its private data directory (including saved tokens).
    pub fn remove_account(&self, account_id: &str) -> anyhow::Result<()> {
        let mut accounts = self.load_accounts();
        accounts.retain(|a| a.id != account_id);
        self.save_accounts(&accounts)?;
        if let Ok(dir) = self.account_dir(account_id) {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(())
    }

    /// Finds an account by `id`.
    pub fn get_account(&self, account_id: &str) -> Option<Account> {
        self.load_accounts()
            .into_iter()
            .find(|a| a.id == account_id)
    }

    /// Directory where private credentials and data for this account live.
    pub fn account_dir(&self, account_id: &str) -> anyhow::Result<PathBuf> {
        let sanitized = account_id.replace(['/', '\\', ':', '@', ' '], "_");
        Ok(self.base.join("accounts").join(sanitized))
    }

    /// Path to the authentication token file for this account.
    pub fn account_token_path(&self, account_id: &str) -> anyhow::Result<PathBuf> {
        Ok(self.account_dir(account_id)?.join("token"))
    }

    fn vault_account_file(&self, vault_dir: &Path) -> anyhow::Result<PathBuf> {
        Ok(self.vault_data_dir(vault_dir)?.join("account_id.txt"))
    }

    /// Returns the account ID linked to this vault, if any.
    pub fn load_vault_account(&self, vault_dir: &Path) -> Option<String> {
        self.vault_account_file(vault_dir)
            .ok()
            .and_then(|p| read_trimmed(&p))
    }

    /// Links this vault to an account ID.
    pub fn save_vault_account(&self, vault_dir: &Path, account_id: &str) -> anyhow::Result<()> {
        write(&self.vault_account_file(vault_dir)?, account_id)
    }

    /// Clears any account link on this vault.
    pub fn clear_vault_account(&self, vault_dir: &Path) {
        if let Ok(file) = self.vault_account_file(vault_dir) {
            let _ = std::fs::remove_file(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_crud_and_token_path() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        assert_eq!(data.load_accounts(), vec![]);

        let acc1 = Account {
            id: "github:octocat".to_string(),
            provider: "github".to_string(),
            instance_url: "https://github.com".to_string(),
            username: "octocat".to_string(),
            display_name: Some("The Octocat".to_string()),
            avatar_url: None,
        };
        let acc2 = Account {
            id: "gitlab:gitlab.com:alice".to_string(),
            provider: "gitlab".to_string(),
            instance_url: "https://gitlab.com".to_string(),
            username: "alice".to_string(),
            display_name: None,
            avatar_url: None,
        };

        data.add_account(acc1.clone()).unwrap();
        data.add_account(acc2.clone()).unwrap();
        assert_eq!(data.load_accounts(), vec![acc1.clone(), acc2.clone()]);
        assert_eq!(data.get_account("github:octocat"), Some(acc1));

        let token_path = data.account_token_path("github:octocat").unwrap();
        assert!(token_path.starts_with(dir.path().join("accounts")));

        // Update account
        let updated_acc1 = Account {
            display_name: Some("Updated Cat".to_string()),
            ..data.get_account("github:octocat").unwrap()
        };
        data.add_account(updated_acc1.clone()).unwrap();
        assert_eq!(
            data.get_account("github:octocat").unwrap().display_name,
            Some("Updated Cat".to_string())
        );

        // Remove account
        data.remove_account("github:octocat").unwrap();
        assert_eq!(data.load_accounts(), vec![acc2]);
        assert_eq!(data.get_account("github:octocat"), None);
    }

    #[test]
    fn vault_account_association() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().to_owned());
        let vault = tempfile::tempdir().unwrap();

        assert_eq!(data.load_vault_account(vault.path()), None);
        data.save_vault_account(vault.path(), "github:octocat")
            .unwrap();
        assert_eq!(
            data.load_vault_account(vault.path()),
            Some("github:octocat".to_string())
        );

        data.clear_vault_account(vault.path());
        assert_eq!(data.load_vault_account(vault.path()), None);
    }
}
