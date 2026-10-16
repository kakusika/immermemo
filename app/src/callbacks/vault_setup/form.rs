//! What the vault setup sheet's form asks for, checked and turned into a plan.
//!
//! Free of Slint: [`Form`] is the sheet's state as plain data, [`plan`] says
//! whether it can be submitted and, if so, what submitting does. The wiring
//! that reads the sheet and acts on the plan is the parent module's.

use std::path::{Path, PathBuf};

use immermemo_core::{parse_vault_name, vault_dir_in};
use immermemo_provider::{RepositoryInfo, validate_repository_name};

/// Where a new vault's folder goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    /// Inside the app's own storage.
    App,
    /// Inside a folder the user names (local storage).
    Local,
}

/// How a vault syncs, as chosen on the sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncChoice {
    /// Not at all.
    None,
    /// Through an account, to a new private repository.
    NewRepository { account_id: String, name: String },
    /// Through an account, to one of its existing repositories
    /// (`full_name` as the account's list shows it).
    ExistingRepository {
        account_id: String,
        full_name: String,
    },
    /// To a remote URL typed in.
    Custom { url: String, token: String },
}

/// The sheet's state.
#[derive(Debug, Clone)]
pub struct Form {
    /// Adding a vault (as opposed to setting up the sync of an existing one):
    /// the name and storage count only then.
    pub adding: bool,
    pub name: String,
    pub storage: Storage,
    /// Local storage: the folder the vault's own folder goes in.
    pub folder: String,
    pub sync: SyncChoice,
}

/// What the form is checked against.
pub struct Env<'a> {
    /// The app's folder for vaults kept in the app.
    pub app_folders: &'a Path,
    pub known_vaults: &'a [PathBuf],
    pub account_ids: &'a [String],
    /// The chosen account's repositories, if they have been loaded.
    pub repositories: &'a [RepositoryInfo],
}

/// What submitting does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The folder of the vault to add; `None` when setting up an existing
    /// vault.
    pub new_vault_dir: Option<PathBuf>,
    pub sync: SyncChoice,
}

/// Where the vault being added would be created, if the name and storage
/// say so well enough yet.
pub fn resulting_path(form: &Form, env: &Env) -> Option<PathBuf> {
    if !form.adding {
        return None;
    }
    let parent = match form.storage {
        Storage::App => env.app_folders.to_owned(),
        Storage::Local => {
            let folder = immermemo_core::vault_path_from_input(&form.folder);
            (!form.folder.trim().is_empty()).then_some(folder)?
        }
    };
    vault_dir_in(&parent, &form.name).ok()
}

/// A repository name to offer for a vault called `vault_name`: the name with
/// everything a host would refuse turned into `-`.
pub fn default_repository_name(vault_name: &str) -> String {
    let name: String = vault_name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches(['-', '.']).to_owned();
    if name.is_empty() {
        "immermemo-notes".to_owned()
    } else {
        name
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// Checks `form` against `env`: the plan if it can be submitted, otherwise
/// what is wrong, in the words to show.
pub fn plan(form: &Form, env: &Env) -> Result<Plan, String> {
    let new_vault_dir = if form.adding {
        parse_vault_name(&form.name).map_err(|e| format!("{e}"))?;
        if form.storage == Storage::Local && form.folder.trim().is_empty() {
            return Err("Choose the folder to keep the vault in".to_owned());
        }
        let dir = resulting_path(form, env)
            .ok_or_else(|| "The name and folder do not make a path".to_owned())?;
        if env
            .known_vaults
            .iter()
            .any(|known| same_folder(known, &dir))
        {
            return Err("That folder is already a vault".to_owned());
        }
        Some(dir)
    } else {
        None
    };

    let sync = match &form.sync {
        SyncChoice::None => SyncChoice::None,
        SyncChoice::NewRepository { account_id, name } => {
            require_account(account_id, env)?;
            SyncChoice::NewRepository {
                account_id: account_id.clone(),
                name: validate_repository_name(name)?.to_owned(),
            }
        }
        SyncChoice::ExistingRepository {
            account_id,
            full_name,
        } => {
            require_account(account_id, env)?;
            if full_name.is_empty() || !env.repositories.iter().any(|r| &r.full_name == full_name) {
                return Err("Choose one of the account's repositories".to_owned());
            }
            SyncChoice::ExistingRepository {
                account_id: account_id.clone(),
                full_name: full_name.clone(),
            }
        }
        SyncChoice::Custom { url, token } => {
            let url = url.trim();
            if url.is_empty() {
                return Err("Enter the remote repository URL".to_owned());
            }
            SyncChoice::Custom {
                url: url.to_owned(),
                token: token.trim().to_owned(),
            }
        }
    };
    Ok(Plan {
        new_vault_dir,
        sync,
    })
}

fn require_account(account_id: &str, env: &Env) -> Result<(), String> {
    if account_id.is_empty() {
        return Err("Choose an account to sync with".to_owned());
    }
    if !env.account_ids.iter().any(|id| id == account_id) {
        return Err("That account is no longer registered".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(full_name: &str) -> RepositoryInfo {
        RepositoryInfo {
            name: full_name.rsplit('/').next().unwrap().to_owned(),
            full_name: full_name.to_owned(),
            clone_url: format!("https://github.com/{full_name}.git"),
            is_private: true,
            default_branch: "main".to_owned(),
        }
    }

    struct Fixture {
        known: Vec<PathBuf>,
        accounts: Vec<String>,
        repos: Vec<RepositoryInfo>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                known: vec![PathBuf::from("/data/notes")],
                accounts: vec!["github:a".to_owned()],
                repos: vec![repo("a/notes"), repo("a/ideas")],
            }
        }

        fn env(&self) -> Env<'_> {
            Env {
                app_folders: Path::new("/data/folders"),
                known_vaults: &self.known,
                account_ids: &self.accounts,
                repositories: &self.repos,
            }
        }
    }

    fn form(name: &str) -> Form {
        Form {
            adding: true,
            name: name.to_owned(),
            storage: Storage::App,
            folder: String::new(),
            sync: SyncChoice::None,
        }
    }

    #[test]
    fn a_vault_in_the_app_goes_in_the_apps_folders() {
        let f = Fixture::new();
        let plan = plan(&form(" Work "), &f.env()).unwrap();
        assert_eq!(
            plan.new_vault_dir,
            Some(PathBuf::from("/data/folders/Work"))
        );
        assert_eq!(plan.sync, SyncChoice::None);
    }

    #[test]
    fn a_vault_in_local_storage_goes_in_the_chosen_folder() {
        let f = Fixture::new();
        let mut local = form("Work");
        local.storage = Storage::Local;
        assert!(plan(&local, &f.env()).is_err(), "no folder yet");
        assert_eq!(resulting_path(&local, &f.env()), None);

        local.folder = "/storage/emulated/0/Documents".to_owned();
        assert_eq!(
            plan(&local, &f.env()).unwrap().new_vault_dir,
            Some(PathBuf::from("/storage/emulated/0/Documents/Work"))
        );
    }

    #[test]
    fn the_name_and_the_folder_must_make_a_new_vault() {
        let f = Fixture::new();
        for bad in ["", "a/b", ".."] {
            assert!(plan(&form(bad), &f.env()).is_err(), "{bad:?}");
        }
        // The folder of a vault already known.
        let mut taken = form("notes");
        taken.storage = Storage::Local;
        taken.folder = "/data".to_owned();
        let error = plan(&taken, &f.env()).unwrap_err();
        assert!(error.contains("already a vault"), "{error}");
    }

    #[test]
    fn syncing_through_an_account_needs_a_registered_account_and_a_repository() {
        let f = Fixture::new();
        let mut with = form("Work");
        with.sync = SyncChoice::NewRepository {
            account_id: String::new(),
            name: "work".to_owned(),
        };
        assert!(
            plan(&with, &f.env())
                .unwrap_err()
                .contains("Choose an account")
        );

        with.sync = SyncChoice::NewRepository {
            account_id: "github:gone".to_owned(),
            name: "work".to_owned(),
        };
        assert!(
            plan(&with, &f.env())
                .unwrap_err()
                .contains("no longer registered")
        );

        with.sync = SyncChoice::NewRepository {
            account_id: "github:a".to_owned(),
            name: "my work".to_owned(),
        };
        assert!(plan(&with, &f.env()).is_err(), "a name a host would refuse");

        with.sync = SyncChoice::NewRepository {
            account_id: "github:a".to_owned(),
            name: " work ".to_owned(),
        };
        assert_eq!(
            plan(&with, &f.env()).unwrap().sync,
            SyncChoice::NewRepository {
                account_id: "github:a".to_owned(),
                name: "work".to_owned()
            }
        );

        with.sync = SyncChoice::ExistingRepository {
            account_id: "github:a".to_owned(),
            full_name: "a/ideas".to_owned(),
        };
        assert!(plan(&with, &f.env()).is_ok());
        with.sync = SyncChoice::ExistingRepository {
            account_id: "github:a".to_owned(),
            full_name: "a/missing".to_owned(),
        };
        assert!(
            plan(&with, &f.env())
                .unwrap_err()
                .contains("account's repositories")
        );
    }

    #[test]
    fn a_typed_remote_needs_a_url_and_keeps_its_token() {
        let f = Fixture::new();
        let mut with = form("Work");
        with.sync = SyncChoice::Custom {
            url: "  ".to_owned(),
            token: String::new(),
        };
        assert!(plan(&with, &f.env()).is_err());
        with.sync = SyncChoice::Custom {
            url: " https://git.example.com/me/n.git ".to_owned(),
            token: " tok ".to_owned(),
        };
        assert_eq!(
            plan(&with, &f.env()).unwrap().sync,
            SyncChoice::Custom {
                url: "https://git.example.com/me/n.git".to_owned(),
                token: "tok".to_owned()
            }
        );
    }

    #[test]
    fn setting_up_an_existing_vault_ignores_the_name_and_storage() {
        let f = Fixture::new();
        let mut existing = form("");
        existing.adding = false;
        existing.sync = SyncChoice::ExistingRepository {
            account_id: "github:a".to_owned(),
            full_name: "a/notes".to_owned(),
        };
        let plan = plan(&existing, &f.env()).unwrap();
        assert_eq!(plan.new_vault_dir, None);
        assert_eq!(resulting_path(&existing, &f.env()), None);
    }

    #[test]
    fn the_offered_repository_name_is_the_vault_name_a_host_would_accept() {
        assert_eq!(default_repository_name("Work"), "Work");
        assert_eq!(default_repository_name("my notes"), "my-notes");
        assert_eq!(default_repository_name("日記"), "immermemo-notes");
        assert_eq!(default_repository_name("  .hidden "), "hidden");
        for name in ["Work", "my notes", "日記 2026", "a/b", "---"] {
            assert!(
                validate_repository_name(&default_repository_name(name)).is_ok(),
                "{name}"
            );
        }
    }
}
