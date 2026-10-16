//! The Settings sheet's accounts and remote: opening the sheet, SSH key,
//! the manual remote, and the registered hosting accounts.

use crate::session::SessionRef;

use slint::{ComponentHandle, SharedString};

use crate::provider;
use crate::session::{
    Session, on_ui_thread, refresh_vault_list, set_error, set_status, show_vault_identity,
    start_sync, switch_vault,
};
use crate::update_provider_ui_state;
use crate::{AccountData, App, SettingsAdapter};

/// Refreshes the list of provider accounts and vault account link in the Settings sheet.
pub fn refresh_accounts_ui(app: &App, session: &Session) {
    provider::migrate_legacy_remotes(session);
    let settings = app.global::<SettingsAdapter>();
    let accounts = session.app_data.load_accounts();
    let ui_accounts: Vec<AccountData> = accounts
        .into_iter()
        .map(|a| {
            let (avatar, has_avatar) =
                crate::avatar::load_or_fetch_avatar(&session.app_data, &a, app.as_weak());
            AccountData {
                id: a.id.into(),
                provider: a.provider.into(),
                instance_url: a.instance_url.into(),
                username: a.username.into(),
                display_name: a.display_name.unwrap_or_default().into(),
                avatar,
                has_avatar,
            }
        })
        .collect();
    settings.set_accounts(slint::ModelRc::new(slint::VecModel::from(ui_accounts)));

    if let Some(acc_id) = session.app_data.load_vault_account(&session.vault_dir) {
        settings.set_current_vault_account_id(acc_id.clone().into());
        if let Some(acc) = session.app_data.get_account(&acc_id) {
            settings.set_current_vault_account_label(
                format!("{} (@{})", acc.provider, acc.username).into(),
            );
            settings.set_current_vault_has_account(true);
            settings.set_current_vault_account_provider(acc.provider.clone().into());
            let (avatar, has_avatar) =
                crate::avatar::load_or_fetch_avatar(&session.app_data, &acc, app.as_weak());
            settings.set_current_vault_account_avatar(avatar);
            settings.set_current_vault_account_has_avatar(has_avatar);
        } else {
            settings.set_current_vault_account_label(acc_id.into());
            settings.set_current_vault_has_account(false);
            settings.set_current_vault_account_provider(SharedString::new());
            settings.set_current_vault_account_avatar(slint::Image::default());
            settings.set_current_vault_account_has_avatar(false);
        }
    } else {
        settings.set_current_vault_account_id(SharedString::new());
        settings.set_current_vault_account_label(SharedString::new());
        settings.set_current_vault_has_account(false);
        settings.set_current_vault_account_provider(SharedString::new());
        settings.set_current_vault_account_avatar(slint::Image::default());
        settings.set_current_vault_account_has_avatar(false);
    }
}

/// Binds opening and closing the Settings sheet, the SSH key, the remote and the accounts.
pub fn bind_account_callbacks(app: &App, session: &SessionRef) {
    let settings = app.global::<SettingsAdapter>();

    app.on_open_settings({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let s = session.borrow();
            settings.set_url_draft(s.remote.clone().unwrap_or_default().into());
            settings.set_token_draft(SharedString::new());
            settings.set_has_token(s.token_store.load().unwrap_or_default().is_some());
            show_vault_identity(&app, &s.vault_dir);
            update_provider_ui_state(&app, s.remote.as_deref());
            refresh_accounts_ui(&app, &s);
            drop(s);
            refresh_vault_list(&app, &session);
            let s = session.borrow();
            let ssh_pub = s.app_data.load_ssh_public_key();
            settings.set_ssh_key_present(ssh_pub.is_some());
            settings.set_ssh_public_key(ssh_pub.unwrap_or_default().into());
            settings.set_ssh_key_copied(false);
            app.set_settings_page(0);
            app.set_settings_open(true);
        }
    });

    app.on_close_settings({
        let weak = app.as_weak();
        move || weak.unwrap().set_settings_open(false)
    });

    settings.on_generate_ssh_key({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let s = session.borrow();
            match s.app_data.generate_ssh_key(None) {
                Ok(pubkey) => {
                    settings.set_ssh_key_present(true);
                    settings.set_ssh_public_key(pubkey.into());
                    settings.set_ssh_key_copied(false);
                    set_status(&app, "Generated new Ed25519 SSH key");
                }
                Err(e) => {
                    set_error(&app, format!("Failed to generate SSH key: {e}"));
                }
            }
        }
    });

    settings.on_copy_ssh_public_key({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let pubkey = settings.get_ssh_public_key();
            if !pubkey.is_empty() {
                crate::session::copy_to_clipboard(&app, pubkey.as_str());
                settings.set_ssh_key_copied(true);
                set_status(&app, "SSH public key copied to clipboard");
            }
        }
    });

    app.on_link({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let s = session.borrow();
            settings.set_url_draft(s.remote.clone().unwrap_or_default().into());
            settings.set_token_draft(SharedString::new());
            settings.set_has_token(s.token_store.load().unwrap_or_default().is_some());
            show_vault_identity(&app, &s.vault_dir);
            update_provider_ui_state(&app, s.remote.as_deref());
            app.set_settings_page(2);
            app.set_settings_open(true);
        }
    });

    settings.on_save_remote({
        let (weak, session) = (app.as_weak(), session.clone());
        move |url, token| {
            let app = weak.unwrap();
            let url = url.trim();
            if url.is_empty() {
                set_error(&app, "Enter a remote URL first");
                return;
            }
            let result = session.borrow_mut().set_manual_remote(url, &token);
            if let Err(e) = result {
                set_error(&app, format!("{e:#}"));
                return;
            }
            crate::callbacks::refresh_accounts_ui(&app, &session.borrow());
            app.set_remote_configured(true);
            update_provider_ui_state(&app, Some(url));
            app.set_settings_open(false);
            start_sync(&app, &session);
        }
    });

    settings.on_delete_account({
        let (weak, session) = (app.as_weak(), session.clone());
        move |account_id| {
            let app = weak.unwrap();
            let s = session.borrow();
            let _ = s.app_data.remove_account(&account_id);
            if s.app_data.load_vault_account(&s.vault_dir).as_deref() == Some(&account_id) {
                s.app_data.clear_vault_account(&s.vault_dir);
            }
            refresh_accounts_ui(&app, &s);
            drop(s);
            refresh_vault_list(&app, &session);
            set_status(&app, "Account removed");
        }
    });

    settings.on_create_vault_from_account({
        let (weak, session) = (app.as_weak(), session.clone());
        move |account_id, default_repo_name| {
            let app = weak.unwrap();
            let s = session.borrow();
            let Some(account) = s.app_data.get_account(&account_id) else {
                set_error(&app, "Account not found");
                return;
            };
            let token_path = match s.app_data.account_token_path(&account_id) {
                Ok(p) => p,
                Err(e) => {
                    set_error(&app, format!("Could not get token path: {e}"));
                    return;
                }
            };
            let token_store = match (s.token_store_for)(&token_path) {
                Ok(t) => t,
                Err(e) => {
                    set_error(&app, format!("Could not load token store: {e}"));
                    return;
                }
            };
            let token = match token_store.load() {
                Ok(Some(t)) => t,
                _ => {
                    set_error(&app, "No token found for this account");
                    return;
                }
            };
            let base = s.app_data.base().to_owned();
            drop(s);

            let provider_id = account.provider.clone();
            let instance_url = account.instance_url.clone();
            let account_id_clone = account_id.clone();
            let weak_thread = app.as_weak();
            set_status(&app, format!("Creating vault for {}...", account.username));

            std::thread::spawn(move || {
                let Some(provider_kind) = immermemo_provider::ProviderId::from_id(&provider_id)
                else {
                    on_ui_thread(&weak_thread, move |app| {
                        set_error(app, format!("Unsupported provider: {provider_id}"));
                    });
                    return;
                };
                let provider = immermemo_provider::provider_for(provider_kind, Some(&instance_url));

                let repo_name = default_repo_name.as_str();
                let repo_info = match provider.create_repository(&token, repo_name, true) {
                    Ok(info) => info,
                    Err(e) => {
                        on_ui_thread(&weak_thread, move |app| {
                            set_error(app, format!("Failed to create repository: {e}"));
                        });
                        return;
                    }
                };

                let vault_dir = base.join("vaults_repos").join(&repo_info.name);
                let _ = std::fs::create_dir_all(&vault_dir);

                on_ui_thread(&weak_thread, move |app| {
                    let session = crate::session::SESSION
                        .with(|s| s.borrow().clone())
                        .expect("session set");
                    let mut s = session.borrow_mut();
                    let _ = s.app_data.add_vault(&vault_dir);
                    let _ = s.app_data.save_vault_account(&vault_dir, &account_id_clone);
                    let _ = s.app_data.save_remote(&vault_dir, &repo_info.clone_url);
                    s.known_vaults = s.app_data.known_vaults();
                    drop(s);

                    switch_vault(app, &session, vault_dir);
                    app.set_settings_open(false);
                    set_status(
                        app,
                        format!("Created and switched to vault '{}'", repo_info.name),
                    );
                });
            });
        }
    });
}
