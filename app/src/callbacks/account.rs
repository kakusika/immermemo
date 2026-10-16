//! The Settings sheet's accounts: opening the sheet, the SSH key, and the
//! registered hosting accounts.

use crate::session::SessionRef;

use slint::{ComponentHandle, SharedString};

use crate::session::{Session, refresh_vault_list, set_error, set_status, show_vault_identity};
use crate::{AccountData, App, SettingsAdapter};

/// Refreshes the list of provider accounts, and the open vault's account
/// shown on Home.
pub fn refresh_accounts_ui(app: &App, session: &Session) {
    let settings = app.global::<SettingsAdapter>();
    let ui_accounts: Vec<AccountData> = session
        .app_data
        .load_accounts()
        .into_iter()
        .map(|a| {
            let (avatar, has_avatar) =
                crate::avatar::load_or_fetch_avatar(&session.app_data, &a, app.as_weak());
            let vault_count = session
                .known_vaults
                .iter()
                .filter(|vault| {
                    session.app_data.load_vault_account(vault).as_deref() == Some(&a.id)
                })
                .count();
            AccountData {
                id: a.id.into(),
                provider: a.provider.into(),
                instance_url: a.instance_url.into(),
                username: a.username.into(),
                display_name: a.display_name.unwrap_or_default().into(),
                avatar,
                has_avatar,
                vault_count: vault_count as i32,
            }
        })
        .collect();
    settings.set_accounts(slint::ModelRc::new(slint::VecModel::from(ui_accounts)));

    let account = session
        .app_data
        .load_vault_account(&session.vault_dir)
        .and_then(|id| session.app_data.get_account(&id));
    match account {
        Some(account) => {
            let (avatar, has_avatar) =
                crate::avatar::load_or_fetch_avatar(&session.app_data, &account, app.as_weak());
            settings.set_current_vault_has_account(true);
            settings.set_current_vault_account_provider(account.provider.into());
            settings.set_current_vault_account_avatar(avatar);
            settings.set_current_vault_account_has_avatar(has_avatar);
        }
        None => {
            settings.set_current_vault_has_account(false);
            settings.set_current_vault_account_provider(SharedString::new());
            settings.set_current_vault_account_avatar(slint::Image::default());
            settings.set_current_vault_account_has_avatar(false);
        }
    }
}

/// Binds opening and closing the Settings sheet, the SSH key and the accounts.
pub fn bind_account_callbacks(app: &App, session: &SessionRef) {
    let settings = app.global::<SettingsAdapter>();

    app.on_open_settings({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let s = session.borrow();
            show_vault_identity(&app, &s.vault_dir);
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

    settings.on_delete_account({
        let (weak, session) = (app.as_weak(), session.clone());
        move |account_id| {
            let app = weak.unwrap();
            // Vaults that sync through the account can no longer authenticate:
            // they stop syncing (their notes stay).
            let affected: Vec<std::path::PathBuf> = {
                let s = session.borrow();
                s.known_vaults
                    .iter()
                    .filter(|vault| {
                        s.app_data.load_vault_account(vault).as_deref() == Some(account_id.as_str())
                    })
                    .cloned()
                    .collect()
            };
            {
                let mut s = session.borrow_mut();
                for vault in &affected {
                    s.clear_vault_sync(vault);
                }
                let _ = s.app_data.remove_account(&account_id);
                app.set_remote_configured(s.remote.is_some());
            }
            refresh_accounts_ui(&app, &session.borrow());
            refresh_vault_list(&app, &session);
            set_status(
                &app,
                match affected.len() {
                    0 => "Account removed".to_owned(),
                    1 => "Account removed; 1 vault no longer syncs".to_owned(),
                    n => format!("Account removed; {n} vaults no longer sync"),
                },
            );
        }
    });
}
