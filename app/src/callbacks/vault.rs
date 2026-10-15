//! Vault management, settings sheet, and app preferences callback bindings.

#[cfg(target_os = "android")]
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, SharedString};

use crate::{AccountData, App, SettingsAdapter};
use crate::haptic;
use crate::render;
use crate::update_provider_ui_state;
use crate::session::{
    Session, refresh_vault_list, set_status, start_sync, switch_vault, vault_display_name,
    vault_path_from_input,
};

/// Migrates any legacy remote URL on known vaults into registered accounts if not yet present.
pub fn migrate_legacy_remotes(session: &Session) {
    for vault in &session.known_vaults {
        if session.app_data.load_vault_account(vault).is_some() {
            continue;
        }
        let Some(remote_url) = session.app_data.load_remote(vault) else {
            continue;
        };
        let trimmed = remote_url.trim();
        if trimmed.is_empty() {
            continue;
        }

        let maybe_account = if let Some((user, _)) = immermemo_provider::GitHubProvider::parse_remote(trimmed) {
            Some(immermemo_core::Account {
                id: format!("github:{user}"),
                provider: "github".to_string(),
                instance_url: "https://github.com".to_string(),
                username: user,
                display_name: None,
                avatar_url: None,
            })
        } else if let Some((host, user, _)) = immermemo_provider::ForgejoProvider::parse_any_remote(trimmed) {
            let instance = if host.starts_with("http://") || host.starts_with("https://") {
                host
            } else {
                format!("https://{host}")
            };
            Some(immermemo_core::Account {
                id: format!("forgejo:{instance}:{user}"),
                provider: "forgejo".to_string(),
                instance_url: instance,
                username: user,
                display_name: None,
                avatar_url: None,
            })
        } else {
            None
        };

        if let Some(acc) = maybe_account {
            let acc_id = acc.id.clone();
            let _ = session.app_data.add_account(acc);
            let _ = session.app_data.save_vault_account(vault, &acc_id);
            if let Ok(token) = session.token_store.load() {
                if let Some(t) = token {
                    if let Ok(token_path) = session.app_data.account_token_path(&acc_id) {
                        if let Ok(acc_store) = (session.token_store_for)(&token_path) {
                            let _ = acc_store.save(&t);
                        }
                    }
                }
            }
        }
    }
}

/// Refreshes the list of provider accounts and vault account link in the Settings sheet.
pub fn refresh_accounts_ui(app: &App, session: &Session) {
    migrate_legacy_remotes(session);
    let settings = app.global::<SettingsAdapter>();
    let accounts = session.app_data.load_accounts();
    let ui_accounts: Vec<AccountData> = accounts
        .into_iter()
        .map(|a| AccountData {
            id: a.id.into(),
            provider: a.provider.into(),
            instance_url: a.instance_url.into(),
            username: a.username.into(),
            display_name: a.display_name.unwrap_or_default().into(),
        })
        .collect();
    settings.set_accounts(slint::ModelRc::new(slint::VecModel::from(ui_accounts)));

    if let Some(acc_id) = session.app_data.load_vault_account(&session.vault_dir) {
        settings.set_current_vault_account_id(acc_id.clone().into());
        if let Some(acc) = session.app_data.get_account(&acc_id) {
            settings.set_current_vault_account_label(
                format!("{} (@{})", acc.provider, acc.username).into(),
            );
        } else {
            settings.set_current_vault_account_label(acc_id.into());
        }
    } else {
        settings.set_current_vault_account_id(SharedString::new());
        settings.set_current_vault_account_label(SharedString::new());
    }
}

#[cfg(target_os = "android")]
pub fn status_bar_visible_for(choice: i32, note_sheet_open: bool) -> bool {
    match choice {
        1 => !note_sheet_open,
        2 => false,
        _ => true,
    }
}

pub fn bind_vault_callbacks(
    app: &App,
    session: &Rc<RefCell<Session>>,
    #[cfg(target_os = "android")] status_bar_choice_cell: Rc<Cell<i32>>,
) {
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
            settings.set_current_vault_name(vault_display_name(&s.vault_dir).into());
            settings.set_current_vault_path(s.vault_dir.display().to_string().into());
            update_provider_ui_state(&app, s.remote.as_deref());
            refresh_accounts_ui(&app, &s);
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
                    set_status(&app, format!("Failed to generate SSH key: {e}"));
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

    settings.on_font_size_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |choice| {
            let app = weak.unwrap();
            let px = match choice {
                0 => 14.0,
                2 => 20.0,
                _ => 17.0,
            };
            app.set_editor_font_size(px);
            let body = app.get_body();
            let max_width = app.get_body_content_width();
            app.set_note_body_items(render::note_body_items(body.as_str(), &app, max_width));
            let _ = session.borrow().app_data.save_font_size(choice);
        }
    });
    settings.on_theme_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |choice| {
            let _ = session.borrow().app_data.save_theme(choice);
            let app = weak.unwrap();
            let body = app.get_body();
            let max_width = app.get_body_content_width();
            app.set_note_body_items(render::note_body_items(body.as_str(), &app, max_width));
        }
    });
    settings.on_status_bar_changed({
        let session = session.clone();
        #[cfg(target_os = "android")]
        let (weak, status_bar_choice_cell) = (app.as_weak(), status_bar_choice_cell.clone());
        move |choice| {
            let _ = session.borrow().app_data.save_status_bar_choice(choice);
            #[cfg(target_os = "android")]
            {
                status_bar_choice_cell.set(choice);
                let app = weak.unwrap();
                crate::android::system_ui::set_status_bar_visible(status_bar_visible_for(
                    choice,
                    app.get_note_sheet_open(),
                ));
            }
        }
    });
    let save_bottom_bar = |session: &std::rc::Rc<std::cell::RefCell<crate::session::Session>>,
                           settings: &SettingsAdapter| {
        let order: Vec<i32> = settings.get_nav_order().iter().collect();
        let id_to_key = |id: i32| -> Option<&'static str> {
            match id {
                0 if settings.get_nav_show_home() => Some("home"),
                1 if settings.get_nav_show_search() => Some("search"),
                2 if settings.get_nav_show_library() => Some("library"),
                3 if settings.get_nav_show_directory() => Some("directory"),
                4 if settings.get_nav_show_daily() => Some("daily"),
                5 if settings.get_nav_show_starred() => Some("starred"),
                6 if settings.get_nav_show_recent() => Some("recent"),
                7 if settings.get_nav_show_graph() => Some("graph"),
                8 if settings.get_nav_show_quick() => Some("quick"),
                _ => None,
            }
        };
        let mut items = Vec::new();
        for id in &order {
            if let Some(key) = id_to_key(*id) {
                items.push(key);
            }
        }
        if items.is_empty() {
            items.push("home");
            settings.set_nav_show_home(true);
        }
        let _ = session
            .borrow()
            .app_data
            .save_bottom_bar_config(&items.join(","));
    };

    settings.on_bottom_bar_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            save_bottom_bar(&session, &settings);
        }
    });
    settings.on_move_nav_item({
        let (weak, session) = (app.as_weak(), session.clone());
        move |from_idx, to_idx| {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let mut order: Vec<i32> = settings.get_nav_order().iter().collect();
            let len = order.len() as i32;
            if from_idx >= 0 && from_idx < len && to_idx >= 0 && to_idx < len && from_idx != to_idx {
                let item = order.remove(from_idx as usize);
                order.insert(to_idx as usize, item);
                settings.set_nav_order(std::rc::Rc::new(slint::VecModel::from(order)).into());
                save_bottom_bar(&session, &settings);
            }
        }
    });
    settings.on_open_vaults({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            refresh_vault_list(&app, &session);
            app.set_vault_sheet_open(true);
        }
    });
    #[cfg(target_os = "android")]
    app.on_note_sheet_open_changed({
        let status_bar_choice_cell = status_bar_choice_cell.clone();
        move |open| {
            crate::android::system_ui::set_status_bar_visible(status_bar_visible_for(
                status_bar_choice_cell.get(),
                open,
            ));
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
            settings.set_current_vault_name(vault_display_name(&s.vault_dir).into());
            settings.set_current_vault_path(s.vault_dir.display().to_string().into());
            update_provider_ui_state(&app, s.remote.as_deref());
            app.set_settings_page(2);
            app.set_settings_open(true);
        }
    });
    app.on_trigger_haptic(|| {
        haptic::perform_haptic();
    });
    app.on_trigger_heavy_haptic(|| {
        haptic::perform_heavy_haptic();
    });

    app.on_open_vault_sheet({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            refresh_vault_list(&app, &session);
            app.set_vault_sheet_open(true);
        }
    });
    app.on_switch_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            app.set_vault_sheet_open(false);
            let Some(vault_dir) = session.borrow().known_vaults.get(index as usize).cloned() else {
                return;
            };
            switch_vault(&app, &session, vault_dir);
        }
    });
    app.on_add_vault_requested({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            app.set_add_vault_draft(SharedString::new());
            app.set_add_vault_open(true);
        }
    });
    app.on_confirm_add_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move |input| {
            let app = weak.unwrap();
            app.set_add_vault_open(false);
            let base = session.borrow().app_data.base().to_owned();
            let new_vault = match vault_path_from_input(&base, &input) {
                Ok(p) => p,
                Err(e) => {
                    set_status(&app, format!("Could not add the vault: {e}"));
                    return;
                }
            };
            if let Err(e) = std::fs::create_dir_all(&new_vault) {
                set_status(&app, format!("Could not add the vault: {e}"));
                return;
            }
            let mut s = session.borrow_mut();
            if let Err(e) = s.app_data.add_vault(&new_vault) {
                drop(s);
                set_status(&app, format!("Could not add the vault: {e}"));
                return;
            }
            s.known_vaults = s.app_data.known_vaults();
            drop(s);
            switch_vault(&app, &session, new_vault);
        }
    });
    app.on_delete_vault_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            let s = session.borrow();
            if s.known_vaults.len() <= 1 {
                return;
            }
            drop(s);
            session.borrow_mut().delete_vault_target = Some(index as usize);
            app.set_delete_vault_confirm_open(true);
        }
    });
    app.on_confirm_delete_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            app.set_delete_vault_confirm_open(false);
            let Some(index) = session.borrow_mut().delete_vault_target.take() else {
                return;
            };
            let Some(vault_dir) = session.borrow().known_vaults.get(index).cloned() else {
                return;
            };
            let mut s = session.borrow_mut();
            if s.known_vaults.len() <= 1 {
                return;
            }
            let is_current = s.vault_dir == vault_dir;
            if let Err(e) = s.app_data.remove_vault(&vault_dir) {
                drop(s);
                set_status(&app, format!("Could not remove the vault: {e}"));
                return;
            }
            s.known_vaults = s.app_data.known_vaults();
            let next_vault = if is_current {
                s.known_vaults.first().cloned()
            } else {
                None
            };
            drop(s);
            if let Some(next) = next_vault {
                switch_vault(&app, &session, next);
            } else {
                refresh_vault_list(&app, &session);
            }
        }
    });

    settings.on_save_remote({
        let (weak, session) = (app.as_weak(), session.clone());
        move |url, token| {
            let app = weak.unwrap();
            let url = url.trim();
            if url.is_empty() {
                set_status(&app, "Enter a remote URL first");
                return;
            }
            let mut s = session.borrow_mut();
            if let Err(e) = s.app_data.save_remote(&s.vault_dir, url) {
                drop(s);
                set_status(&app, format!("Could not save the remote: {e}"));
                return;
            }
            if !token.is_empty()
                && let Err(e) = s.token_store.save(&token)
            {
                drop(s);
                set_status(&app, format!("Could not save the access token: {e}"));
                return;
            }
            s.remote = Some(url.to_owned());
            drop(s);
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
                set_status(&app, "Account not found");
                return;
            };
            let token_path = match s.app_data.account_token_path(&account_id) {
                Ok(p) => p,
                Err(e) => {
                    set_status(&app, format!("Could not get token path: {e}"));
                    return;
                }
            };
            let token_store = match (s.token_store_for)(&token_path) {
                Ok(t) => t,
                Err(e) => {
                    set_status(&app, format!("Could not load token store: {e}"));
                    return;
                }
            };
            let token = match token_store.load() {
                Ok(Some(t)) => t,
                _ => {
                    set_status(&app, "No token found for this account");
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
                let provider: Box<dyn immermemo_provider::Provider> = match provider_id.as_str() {
                    "github" => Box::new(immermemo_provider::GitHubProvider::default()),
                    "forgejo" => Box::new(immermemo_provider::ForgejoProvider::new(Some(&instance_url))),
                    "gitlab" => Box::new(immermemo_provider::GitLabProvider::new(Some(&instance_url))),
                    _ => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = weak_thread.upgrade() {
                                set_status(&app, format!("Unsupported provider: {provider_id}"));
                            }
                        });
                        return;
                    }
                };

                let repo_name = default_repo_name.as_str();
                let repo_info = match provider.create_repository(&token, repo_name, true) {
                    Ok(info) => info,
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = weak_thread.upgrade() {
                                set_status(&app, format!("Failed to create repository: {e}"));
                            }
                        });
                        return;
                    }
                };

                let vault_dir = base.join("vaults_repos").join(&repo_info.name);
                let _ = std::fs::create_dir_all(&vault_dir);

                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak_thread.upgrade() {
                        let session = crate::session::SESSION
                            .with(|s| s.borrow().clone())
                            .expect("session set");
                        let mut s = session.borrow_mut();
                        let _ = s.app_data.add_vault(&vault_dir);
                        let _ = s.app_data.save_vault_account(&vault_dir, &account_id_clone);
                        let _ = s.app_data.save_remote(&vault_dir, &repo_info.clone_url);
                        s.known_vaults = s.app_data.known_vaults();
                        drop(s);

                        switch_vault(&app, &session, vault_dir);
                        app.set_settings_open(false);
                        set_status(&app, format!("Created and switched to vault '{}'", repo_info.name));
                    }
                });
            });
        }
    });
}
