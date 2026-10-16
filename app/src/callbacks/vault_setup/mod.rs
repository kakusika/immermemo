//! The vault setup sheet: adding a vault (name, where it lives, how it
//! syncs) and setting up how the open vault syncs.
//!
//! The form's state is `AddVaultAdapter`'s. [`form`] says, as plain data,
//! whether it can be submitted and what submitting does; this module reads
//! the sheet into that, shows the verdict back, loads an account's
//! repositories and creates a new one off the UI thread, and finally acts on
//! the plan through [`immermemo_core::Session`].

mod form;

use std::cell::RefCell;
use std::path::PathBuf;

use crate::platform::storage;
use immermemo_core::{NewVault, VaultSync};
use immermemo_provider::{ProviderId, RepositoryInfo, identify_remote, provider_for};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::session::{
    SessionRef, on_ui_thread, refresh_vault_list, set_status, start_sync, switch_vault,
};
use crate::{AddVaultAdapter, App, RepoRow, SettingsAdapter};
use form::{Env, Form, Plan, Storage, SyncChoice};

/// The most repositories the list shows at once (the rest are a search away).
const MAX_LISTED_REPOSITORIES: usize = 50;

/// What the sheet remembers besides the form itself.
#[derive(Default)]
struct Setup {
    /// The chosen account's repositories, once loaded.
    repositories: Vec<RepositoryInfo>,
    /// Raised whenever the sheet opens or the account changes, so a slow
    /// answer meant for an earlier state is dropped.
    generation: u64,
    /// The repository name last filled in from the vault's name. While the
    /// field still holds it, the user has not chosen a name of their own.
    offered_repository_name: String,
    /// The folder of the vault being set up, when setting up an existing one.
    target: Option<PathBuf>,
}

thread_local! {
    static SETUP: RefCell<Setup> = RefCell::default();
}

fn setup<R>(f: impl FnOnce(&mut Setup) -> R) -> R {
    SETUP.with(|s| f(&mut s.borrow_mut()))
}

/// The sheet's state as the form module sees it.
fn read_form(adapter: &AddVaultAdapter) -> Form {
    let account_id = adapter.get_account_id().to_string();
    let sync = if !adapter.get_sync() {
        SyncChoice::None
    } else if adapter.get_sync_kind() == 1 {
        SyncChoice::Custom {
            url: adapter.get_custom_url().to_string(),
            token: adapter.get_custom_token().to_string(),
        }
    } else if adapter.get_repo_mode() == 1 {
        SyncChoice::ExistingRepository {
            account_id,
            full_name: adapter.get_repo_full_name().to_string(),
        }
    } else {
        SyncChoice::NewRepository {
            account_id,
            name: adapter.get_repo_name().to_string(),
        }
    };
    Form {
        adding: adapter.get_mode() == 0,
        name: adapter.get_name().to_string(),
        storage: if adapter.get_storage() == 1 {
            Storage::Local
        } else {
            Storage::App
        },
        folder: adapter.get_folder().to_string(),
        sync,
    }
}

/// Checks the sheet's form against the session: the plan, or what is wrong.
fn check(adapter: &AddVaultAdapter, session: &SessionRef) -> (Form, Result<Plan, String>) {
    let form = read_form(adapter);
    let s = session.borrow();
    let app_folders = s.app_data.base().join("folders");
    let account_ids: Vec<String> = s
        .app_data
        .load_accounts()
        .into_iter()
        .map(|a| a.id)
        .collect();
    let repositories = setup(|st| st.repositories.clone());
    let env = Env {
        app_folders: &app_folders,
        known_vaults: &s.known_vaults,
        account_ids: &account_ids,
        repositories: &repositories,
    };
    let plan = form::plan(&form, &env);
    (form, plan)
}

/// Re-derives what the sheet shows from the form: the path the vault will be
/// created at, the repository name on offer, whether it can be submitted.
fn refresh(app: &App, session: &SessionRef) {
    let adapter = app.global::<AddVaultAdapter>();
    offer_repository_name(&adapter);

    let (form, plan) = check(&adapter, session);
    let s = session.borrow();
    let app_folders = s.app_data.base().join("folders");
    let env = Env {
        app_folders: &app_folders,
        known_vaults: &s.known_vaults,
        account_ids: &[],
        repositories: &[],
    };
    let path = form::resulting_path(&form, &env);
    adapter.set_resulting_path(
        path.map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
    adapter.set_can_submit(plan.is_ok());
    adapter.set_storage_hint(storage_hint(&form).into());
    adapter.set_error(SharedString::new());
}

/// What to tell the user about the chosen storage, if anything.
fn storage_hint(form: &Form) -> &'static str {
    if form.adding
        && form.storage == Storage::Local
        && storage::CAN_PICK_FOLDER
        && !storage::has_all_files_access()
    {
        "Allow access to all files to keep a vault in local storage."
    } else {
        ""
    }
}

/// Fills the repository name field from the vault's name, until the user
/// types a name of their own.
///
/// The field is written *after* the sheet's state is released: writing it
/// runs the sheet's own `edited` handler, which borrows that state again.
fn offer_repository_name(adapter: &AddVaultAdapter) {
    let vault_name = if adapter.get_mode() == 0 {
        adapter.get_name().to_string()
    } else {
        adapter.get_target_name().to_string()
    };
    let offer = form::default_repository_name(&vault_name);
    let current = adapter.get_repo_name().to_string();
    let replace = setup(|st| {
        let untouched = current.is_empty() || current == st.offered_repository_name;
        if untouched {
            st.offered_repository_name = offer.clone();
        }
        untouched && current != offer
    });
    if replace {
        adapter.set_repo_name(offer.into());
    }
}

fn reset_adapter(adapter: &AddVaultAdapter) {
    adapter.set_page(0);
    adapter.set_name(SharedString::new());
    adapter.set_storage(0);
    adapter.set_folder(SharedString::new());
    adapter.set_resulting_path(SharedString::new());
    adapter.set_can_pick_folder(storage::CAN_PICK_FOLDER);
    adapter.set_storage_hint(SharedString::new());
    adapter.set_sync(false);
    adapter.set_sync_kind(0);
    adapter.set_account_id(SharedString::new());
    adapter.set_repo_mode(0);
    adapter.set_repo_name(SharedString::new());
    adapter.set_repo_query(SharedString::new());
    adapter.set_repos(ModelRc::default());
    adapter.set_repo_full_name(SharedString::new());
    adapter.set_repos_loading(false);
    adapter.set_repos_error(SharedString::new());
    adapter.set_custom_url(SharedString::new());
    adapter.set_custom_token(SharedString::new());
    adapter.set_busy(false);
    adapter.set_error(SharedString::new());
    adapter.set_can_submit(false);
}

fn forget_loaded(target: Option<PathBuf>) {
    setup(|st| {
        st.repositories.clear();
        st.generation += 1;
        st.offered_repository_name.clear();
        st.target = target;
    });
}

/// Opens the sheet to add a vault; `account` (an account id) starts it
/// syncing through that account.
fn open_add(app: &App, session: &SessionRef, account: Option<&str>) {
    let adapter = app.global::<AddVaultAdapter>();
    reset_adapter(&adapter);
    forget_loaded(None);
    adapter.set_mode(0);
    adapter.set_target_name(SharedString::new());
    if let Some(account) = account {
        adapter.set_sync(true);
        adapter.set_account_id(account.into());
    }
    crate::callbacks::refresh_accounts_ui(app, &session.borrow());
    refresh(app, session);
    app.set_vault_setup_open(true);
}

/// Opens the sheet to set up how the open vault syncs, starting from how it
/// does now.
fn open_sync(app: &App, session: &SessionRef) {
    let adapter = app.global::<AddVaultAdapter>();
    reset_adapter(&adapter);
    let (dir, remote, account) = {
        let s = session.borrow();
        (
            s.vault_dir.clone(),
            s.remote.clone(),
            s.app_data.load_vault_account(&s.vault_dir),
        )
    };
    forget_loaded(Some(dir.clone()));
    adapter.set_mode(1);
    adapter.set_target_name(crate::session::vault_display_name(&dir).into());
    crate::callbacks::refresh_accounts_ui(app, &session.borrow());

    if let Some(remote) = remote {
        adapter.set_sync(true);
        match account {
            Some(account_id) => {
                adapter.set_sync_kind(0);
                adapter.set_account_id(account_id.into());
                // The repository it uses now, to pick again from the list.
                if let Some(identity) = identify_remote(&remote) {
                    adapter.set_repo_mode(1);
                    adapter
                        .set_repo_full_name(format!("{}/{}", identity.owner, identity.repo).into());
                }
            }
            None => {
                adapter.set_sync_kind(1);
                adapter.set_custom_url(remote.into());
            }
        }
    }
    refresh(app, session);
    if adapter.get_sync() && adapter.get_sync_kind() == 0 && adapter.get_repo_mode() == 1 {
        load_repositories(app, session);
    }
    app.set_vault_setup_open(true);
}

/// The account `account_id`'s provider client and token.
fn account_client(
    session: &SessionRef,
    account_id: &str,
) -> Result<(Box<dyn immermemo_provider::Provider>, String), String> {
    let s = session.borrow();
    let account = s
        .app_data
        .get_account(account_id)
        .ok_or("That account is no longer registered")?;
    let provider_id = ProviderId::from_id(&account.provider)
        .ok_or_else(|| format!("Unsupported provider: {}", account.provider))?;
    let path = s
        .app_data
        .account_token_path(account_id)
        .map_err(|e| format!("{e}"))?;
    let token = (s.token_store_for)(&path)
        .and_then(|store| store.load())
        .map_err(|e| format!("Could not read the account's token: {e}"))?
        .ok_or("No token is saved for this account")?;
    Ok((
        provider_for(provider_id, Some(&account.instance_url)),
        token,
    ))
}

/// Starts loading the chosen account's repositories.
fn load_repositories(app: &App, session: &SessionRef) {
    let adapter = app.global::<AddVaultAdapter>();
    let account_id = adapter.get_account_id().to_string();
    if account_id.is_empty() {
        return;
    }
    let (provider, token) = match account_client(session, &account_id) {
        Ok(client) => client,
        Err(message) => {
            adapter.set_repos_error(message.into());
            return;
        }
    };
    adapter.set_repos_loading(true);
    adapter.set_repos_error(SharedString::new());
    let generation = setup(|st| {
        st.generation += 1;
        st.generation
    });
    let weak = app.as_weak();
    std::thread::spawn(move || {
        let result = provider
            .list_repositories(&token)
            .map_err(|e| format!("Could not list the repositories: {e}"));
        on_ui_thread(&weak, move |app| {
            if setup(|st| st.generation) != generation {
                return;
            }
            let adapter = app.global::<AddVaultAdapter>();
            adapter.set_repos_loading(false);
            match result {
                Ok(repositories) => setup(|st| st.repositories = repositories),
                Err(message) => adapter.set_repos_error(message.into()),
            }
            show_repositories(&adapter);
            let session = crate::session::SESSION
                .with(|s| s.borrow().clone())
                .expect("session set in run");
            refresh(app, &session);
        });
    });
}

/// Shows the loaded repositories that match the search, most relevant first.
fn show_repositories(adapter: &AddVaultAdapter) {
    let query = adapter.get_repo_query().to_lowercase();
    let mut rows: Vec<RepoRow> = setup(|st| {
        st.repositories
            .iter()
            .filter(|r| r.full_name.to_lowercase().contains(&query))
            .map(|r| RepoRow {
                name: r.name.clone().into(),
                full_name: r.full_name.clone().into(),
                is_private: r.is_private,
            })
            .collect()
    });
    rows.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    rows.truncate(MAX_LISTED_REPOSITORIES);
    adapter.set_repos(ModelRc::new(VecModel::from(rows)));
}

/// Acts on the checked form.
fn submit(app: &App, session: &SessionRef) {
    let adapter = app.global::<AddVaultAdapter>();
    let (_, plan) = check(&adapter, session);
    let plan = match plan {
        Ok(plan) => plan,
        Err(message) => {
            adapter.set_error(message.into());
            return;
        }
    };
    adapter.set_busy(true);
    adapter.set_error(SharedString::new());

    match plan.sync.clone() {
        SyncChoice::NewRepository { account_id, name } => {
            let (provider, token) = match account_client(session, &account_id) {
                Ok(client) => client,
                Err(message) => return fail(app, message),
            };
            let weak = app.as_weak();
            std::thread::spawn(move || {
                let created = provider
                    .create_repository(&token, &name, true)
                    .map_err(|e| format!("Could not create the repository: {e}"));
                on_ui_thread(&weak, move |app| {
                    let session = crate::session::SESSION
                        .with(|s| s.borrow().clone())
                        .expect("session set in run");
                    match created {
                        Ok(repo) => finish(
                            app,
                            &session,
                            plan.new_vault_dir,
                            Some(VaultSync::Account {
                                account_id,
                                remote_url: repo.clone_url,
                            }),
                        ),
                        Err(message) => fail(app, message),
                    }
                });
            });
        }
        SyncChoice::ExistingRepository {
            account_id,
            full_name,
        } => {
            let clone_url = setup(|st| {
                st.repositories
                    .iter()
                    .find(|r| r.full_name == full_name)
                    .map(|r| r.clone_url.clone())
            });
            match clone_url {
                Some(remote_url) => finish(
                    app,
                    session,
                    plan.new_vault_dir,
                    Some(VaultSync::Account {
                        account_id,
                        remote_url,
                    }),
                ),
                None => fail(app, "Choose one of the account's repositories".to_owned()),
            }
        }
        SyncChoice::Custom { url, token } => finish(
            app,
            session,
            plan.new_vault_dir,
            Some(VaultSync::Custom {
                remote_url: url,
                token,
            }),
        ),
        SyncChoice::None => finish(app, session, plan.new_vault_dir, None),
    }
}

fn fail(app: &App, message: String) {
    let adapter = app.global::<AddVaultAdapter>();
    adapter.set_busy(false);
    adapter.set_error(message.into());
}

/// The vault and its sync are decided: add the vault, or set the open
/// vault's sync, and close the sheet.
fn finish(
    app: &App,
    session: &SessionRef,
    new_vault_dir: Option<PathBuf>,
    sync: Option<VaultSync>,
) {
    let syncs = sync.is_some();
    let result: Result<String, String> = match new_vault_dir {
        Some(dir) => {
            let added = session
                .borrow_mut()
                .add_vault(&NewVault { dir, sync })
                .map_err(|e| format!("{e:#}"));
            added.map(|dir| {
                let name = crate::session::vault_display_name(&dir);
                switch_vault(app, session, dir);
                format!("Added the vault \"{name}\"")
            })
        }
        None => {
            let target = setup(|st| st.target.clone());
            let Some(dir) = target else {
                return fail(app, "There is no vault to set up".to_owned());
            };
            let mut s = session.borrow_mut();
            let applied = match &sync {
                Some(sync) => s.set_vault_sync(&dir, sync).map_err(|e| format!("{e:#}")),
                None => {
                    s.clear_vault_sync(&dir);
                    Ok(())
                }
            };
            let remote_configured = s.remote.is_some();
            drop(s);
            app.set_remote_configured(remote_configured);
            applied.map(|()| {
                if syncs {
                    "Sync is set up".to_owned()
                } else {
                    "This vault no longer syncs".to_owned()
                }
            })
        }
    };
    match result {
        Ok(message) => {
            app.global::<AddVaultAdapter>().set_busy(false);
            app.set_vault_setup_open(false);
            crate::callbacks::refresh_accounts_ui(app, &session.borrow());
            refresh_vault_list(app, session);
            set_status(app, message);
            if syncs {
                start_sync(app, session);
            }
        }
        Err(message) => fail(app, message),
    }
}

thread_local! {
    static APP: RefCell<Option<slint::Weak<App>>> = const { RefCell::new(None) };
}

/// An account was just added (from this sheet's own Add Account page): back
/// to the form, with that account chosen for sync.
pub fn account_added(app: &App, account_id: &str) {
    if !app.get_vault_setup_open() {
        return;
    }
    let adapter = app.global::<AddVaultAdapter>();
    if adapter.get_page() != 1 {
        return;
    }
    adapter.set_page(0);
    adapter.set_sync(true);
    adapter.set_sync_kind(0);
    adapter.set_account_id(account_id.into());
    // Choosing it is what loads its repositories and re-checks the form.
    adapter.invoke_account_chosen(account_id.into());
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
/// The folder picker answered with `tree_uri`: it becomes the folder the
/// vault is created in, if it is one this app can use.
pub fn folder_picked(tree_uri: &str) {
    let Some(app) = APP.with(|a| a.borrow().as_ref().and_then(|w| w.upgrade())) else {
        return;
    };
    let session = crate::session::SESSION
        .with(|s| s.borrow().clone())
        .expect("session set in run");
    let adapter = app.global::<AddVaultAdapter>();
    match storage::path_from_tree_uri(tree_uri) {
        Some(path) => {
            adapter.set_folder(path.display().to_string().into());
            refresh(&app, &session);
        }
        None => adapter.set_error(
            "That folder can't be used. Choose one on this device or on an SD card.".into(),
        ),
    }
}

/// "Choose folder": asks for the storage permission first if it is missing,
/// then opens the picker.
fn choose_folder(app: &App) {
    if storage::has_all_files_access() {
        storage::pick_folder();
    } else {
        storage::request_all_files_access();
        // Say what to do on coming back.
        app.global::<AddVaultAdapter>().set_storage_hint(
            "Allow access to all files in the screen that opened, then choose the folder again."
                .into(),
        );
    }
}

/// Binds the sheet's callbacks and every way of opening it.
pub fn bind_vault_setup_callbacks(app: &App, session: &SessionRef) {
    APP.with(|a| *a.borrow_mut() = Some(app.as_weak()));
    let adapter = app.global::<AddVaultAdapter>();

    adapter.on_edited({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            show_repositories(&app.global::<AddVaultAdapter>());
            refresh(&app, &session);
        }
    });
    adapter.on_account_chosen({
        let (weak, session) = (app.as_weak(), session.clone());
        move |_account_id| {
            let app = weak.unwrap();
            let adapter = app.global::<AddVaultAdapter>();
            setup(|st| {
                st.repositories.clear();
                st.generation += 1;
            });
            adapter.set_repo_full_name(SharedString::new());
            adapter.set_repos(ModelRc::default());
            adapter.set_repos_error(SharedString::new());
            if adapter.get_repo_mode() == 1 {
                load_repositories(&app, &session);
            }
            refresh(&app, &session);
        }
    });
    adapter.on_load_repos({
        let (weak, session) = (app.as_weak(), session.clone());
        move || load_repositories(&weak.unwrap(), &session)
    });
    adapter.on_pick_folder({
        let weak = app.as_weak();
        move || choose_folder(&weak.unwrap())
    });
    adapter.on_submit({
        let (weak, session) = (app.as_weak(), session.clone());
        move || submit(&weak.unwrap(), &session)
    });
    adapter.on_close({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            setup(|st| st.generation += 1);
            app.set_vault_setup_open(false);
        }
    });

    app.on_add_vault_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move || open_add(&weak.unwrap(), &session, None)
    });
    app.on_open_sync_settings({
        let (weak, session) = (app.as_weak(), session.clone());
        move || open_sync(&weak.unwrap(), &session)
    });
    let settings = app.global::<SettingsAdapter>();
    settings.on_add_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move || open_add(&weak.unwrap(), &session, None)
    });
    settings.on_new_vault_with_account({
        let (weak, session) = (app.as_weak(), session.clone());
        move |account_id| open_add(&weak.unwrap(), &session, Some(account_id.as_str()))
    });
}
