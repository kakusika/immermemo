//! Setting a vault up: its name and place, and how (whether) it syncs.
//!
//! Accounts and vaults are independent. An [`Account`] is a hosting service
//! login registered on its own ([`Session::add_account`]); a vault chooses,
//! separately, how it syncs ([`VaultSync`]): through an account, to a remote
//! typed in, or not at all. Nothing here talks to a network: creating or
//! choosing the repository an account-synced vault points at is the caller's
//! job, and arrives here as a finished `remote_url`.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::session::Session;
use crate::{Account, TokenStore};

/// How a vault syncs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultSync {
    /// Through a registered account: the account's token authenticates.
    Account {
        account_id: String,
        remote_url: String,
    },
    /// To a remote the user typed in, authenticated by `token`, which is kept
    /// in the vault's own store (an empty one keeps what is already there).
    Custom { remote_url: String, token: String },
}

impl VaultSync {
    /// The remote this syncs to.
    pub fn remote_url(&self) -> &str {
        match self {
            Self::Account { remote_url, .. } | Self::Custom { remote_url, .. } => remote_url,
        }
    }
}

/// A vault to add.
#[derive(Debug, Clone)]
pub struct NewVault {
    /// Its folder (created if missing).
    pub dir: PathBuf,
    /// How it syncs, if it does.
    pub sync: Option<VaultSync>,
}

/// The most characters a vault's name may have.
const MAX_NAME_CHARS: usize = 100;

/// A vault's name as typed, checked: it becomes a folder name, so it must be
/// a single path component. Surrounding whitespace is dropped.
pub fn parse_vault_name(input: &str) -> anyhow::Result<String> {
    let name = input.trim();
    anyhow::ensure!(!name.is_empty(), "the name can't be empty");
    anyhow::ensure!(
        !name.contains(['/', '\\', '\0']),
        "the name can't contain a path separator"
    );
    anyhow::ensure!(name != "." && name != "..", "that is not a usable name");
    anyhow::ensure!(
        name.chars().count() <= MAX_NAME_CHARS,
        "the name can be at most {MAX_NAME_CHARS} characters"
    );
    Ok(name.to_owned())
}

/// The folder a vault called `name` gets inside `parent`.
pub fn vault_dir_in(parent: &Path, name: &str) -> anyhow::Result<PathBuf> {
    Ok(parent.join(parse_vault_name(name)?))
}

/// Whether two paths are one folder: equal, or both existing and the same
/// once symlinks are resolved.
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

impl Session {
    /// Registers `account` and keeps its `token`, replacing what an account
    /// with the same id had. Touches no vault.
    pub fn add_account(&self, account: Account, token: &str) -> anyhow::Result<()> {
        let path = self.app_data.account_token_path(&account.id)?;
        (self.token_store_for)(&path)?
            .save(token)
            .context("Could not save the account's access token")?;
        self.app_data
            .add_account(account)
            .context("Could not save the account")
    }

    /// Sets how the vault at `dir` syncs (any known vault, not only the open
    /// one). The open vault's remote and token store follow at once.
    ///
    /// Re-pointing a vault at another account only changes this vault: the
    /// previous account and its token stay, other vaults may use them.
    pub fn set_vault_sync(&mut self, dir: &Path, sync: &VaultSync) -> anyhow::Result<()> {
        match sync {
            VaultSync::Account {
                account_id,
                remote_url,
            } => {
                anyhow::ensure!(
                    self.app_data.get_account(account_id).is_some(),
                    "that account is not registered"
                );
                self.app_data
                    .save_remote(dir, remote_url)
                    .context("Could not save the remote")?;
                self.app_data
                    .save_vault_account(dir, account_id)
                    .context("Could not link the account")?;
            }
            VaultSync::Custom { remote_url, token } => {
                self.app_data
                    .save_remote(dir, remote_url)
                    .context("Could not save the remote")?;
                // A typed-in remote is not tied to an account: dropping the
                // link is what makes sync use the token saved below.
                self.app_data.clear_vault_account(dir);
                if !token.is_empty() {
                    (self.token_store_for)(dir)?
                        .save(token)
                        .context("Could not save the access token")?;
                }
            }
        }
        if same_folder(dir, &self.vault_dir) {
            self.remote = Some(sync.remote_url().to_owned());
            self.token_store = self.open_token_store(dir)?;
        }
        Ok(())
    }

    /// Stops the vault at `dir` from syncing: forgets its remote, its own
    /// token and its account link. Registered accounts and their tokens stay.
    pub fn clear_vault_sync(&mut self, dir: &Path) {
        let _ = self.app_data.save_remote(dir, "");
        self.app_data.clear_vault_account(dir);
        let own: Option<std::sync::Arc<dyn TokenStore>> = (self.token_store_for)(dir).ok();
        if let Some(own) = &own {
            let _ = own.save("");
        }
        if same_folder(dir, &self.vault_dir) {
            self.remote = None;
            if let Some(own) = own {
                self.token_store = own;
            }
        }
    }

    /// Adds a vault: creates its folder if need be, registers it, and sets
    /// how it syncs. Does not switch to it. Returns the folder as registered
    /// (canonical), which is what every other call about it takes.
    pub fn add_vault(&mut self, vault: &NewVault) -> anyhow::Result<PathBuf> {
        let known_before = self.app_data.known_vaults();
        self.register_vault(&vault.dir)?;
        let dir = vault.dir.canonicalize()?;
        anyhow::ensure!(
            !known_before.iter().any(|known| same_folder(known, &dir)),
            "that folder is already a vault"
        );
        if let Some(sync) = &vault.sync
            && let Err(error) = self.set_vault_sync(&dir, sync)
        {
            // Do not leave a half-set-up vault behind.
            let _ = self.forget_vault(&dir);
            return Err(error);
        }
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vault_name_is_one_path_component() {
        assert_eq!(parse_vault_name("  notes  ").unwrap(), "notes");
        assert_eq!(parse_vault_name("日記 2026").unwrap(), "日記 2026");
        for bad in [
            "",
            "   ",
            "a/b",
            "a\\b",
            ".",
            "..",
            "x\0y",
            &"n".repeat(101),
        ] {
            assert!(parse_vault_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            vault_dir_in(Path::new("/data/folders"), " work ").unwrap(),
            PathBuf::from("/data/folders/work")
        );
    }
}
