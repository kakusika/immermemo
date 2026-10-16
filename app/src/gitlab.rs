//! GitLab authentication, personal access token verification, and repository setup.

use std::cell::RefCell;
use std::rc::Rc;

use immermemo_provider::{GitLabProvider, Provider};
use slint::{ComponentHandle, SharedString};

use crate::session::{SESSION, Session, set_status, start_sync};
use crate::{App, SettingsAdapter};

/// Updates GitLab connected/account state in the UI based on the current remote URL.
pub fn update_gitlab_ui_state(app: &App, remote: Option<&str>) {
    let settings = app.global::<SettingsAdapter>();
    if let Some(url) = remote {
        if GitLabProvider::is_gitlab_remote(url) {
            if let Some((host, user, repo)) = GitLabProvider::parse_any_remote(url) {
                settings.set_gitlab_connected(true);
                settings.set_gitlab_username(user.into());
                settings.set_gitlab_instance(host.into());
                settings.set_gitlab_repo_name(repo.into());
                return;
            }
        }
    }
    settings.set_gitlab_connected(false);
    settings.set_gitlab_username(SharedString::new());
    settings.set_gitlab_instance(SharedString::new());
    settings.set_gitlab_repo_name(SharedString::new());
}

/// Binds GitLab connect and disconnect callbacks to the Slint App.
pub fn bind_gitlab_callbacks(app: &App, session: &Rc<RefCell<Session>>) {
    let settings = app.global::<SettingsAdapter>();

    settings.on_disconnect_gitlab({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let mut s = session.borrow_mut();
            let vault_dir = s.vault_dir.clone();
            let _ = s.app_data.save_remote(&vault_dir, "");
            let _ = s.app_data.save_provider(&vault_dir, "");
            let _ = s.token_store.save("");
            s.remote = None;
            drop(s);
            app.set_remote_configured(false);
            let settings = app.global::<SettingsAdapter>();
            settings.set_url_draft(SharedString::new());
            settings.set_token_draft(SharedString::new());
            settings.set_has_token(false);
            update_gitlab_ui_state(&app, None);
            set_status(&app, "Disconnected GitLab repository");
        }
    });

    settings.on_connect_gitlab({
        let weak = app.as_weak();
        move |instance_url, token, repo_name| {
            let Some(app) = weak.upgrade() else { return };
            let token_str = token.trim().to_string();
            let instance_str = instance_url.trim().to_string();
            let repo_str = if repo_name.trim().is_empty() {
                "immermemo-notes".to_string()
            } else {
                repo_name.trim().to_string()
            };

            let settings = app.global::<SettingsAdapter>();
            if token_str.is_empty() {
                settings.set_gitlab_status_is_error(true);
                settings.set_gitlab_status_message("Personal access token cannot be empty.".into());
                return;
            }

            settings.set_gitlab_connecting(true);
            settings.set_gitlab_status_is_error(false);
            settings.set_gitlab_status_message(
                "Connecting and setting up private repository...".into(),
            );

            let thread_weak = weak.clone();
            std::thread::spawn(move || {
                let provider = GitLabProvider::new(if instance_str.is_empty() {
                    None
                } else {
                    Some(&instance_str)
                });

                let repo_info = match provider.setup_repository(&token_str, &repo_str) {
                    Ok(info) => info,
                    Err(e) => {
                        let weak = thread_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = weak.upgrade() {
                                let settings = app.global::<SettingsAdapter>();
                                settings.set_gitlab_connecting(false);
                                settings.set_gitlab_status_is_error(true);
                                settings.set_gitlab_status_message(
                                    format!("Repository setup failed: {e}").into(),
                                );
                            }
                        });
                        return;
                    }
                };

                let user_info = provider.get_user(&token_str).ok();
                let username = user_info
                    .as_ref()
                    .map(|u| u.login.clone())
                    .or_else(|| {
                        GitLabProvider::parse_any_remote(&repo_info.clone_url).map(|(_, u, _)| u)
                    })
                    .unwrap_or_default();
                let instance_display = provider.instance_url().to_string();

                let weak_done = thread_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak_done.upgrade() {
                        let session = SESSION
                            .with(|s| s.borrow().clone())
                            .expect("session set in run");
                        let mut s = session.borrow_mut();
                        let vault_dir = s.vault_dir.clone();
                        let clone_url = repo_info.clone_url.clone();
                        let settings = app.global::<SettingsAdapter>();
                        if let Err(e) = s.app_data.save_remote(&vault_dir, &clone_url) {
                            drop(s);
                            settings.set_gitlab_connecting(false);
                            settings.set_gitlab_status_is_error(true);
                            settings.set_gitlab_status_message(
                                format!("Could not save remote: {e}").into(),
                            );
                            return;
                        }
                        let _ = s.app_data.save_provider(&vault_dir, "gitlab");
                        if let Err(e) = s.token_store.save(&token_str) {
                            drop(s);
                            settings.set_gitlab_connecting(false);
                            settings.set_gitlab_status_is_error(true);
                            settings.set_gitlab_status_message(
                                format!("Could not save access token: {e}").into(),
                            );
                            return;
                        }
                        s.remote = Some(clone_url.clone());
                        let account_id = format!("gitlab:{instance_display}:{username}");
                        let account = immermemo_core::Account {
                            id: account_id.clone(),
                            provider: "gitlab".to_string(),
                            instance_url: instance_display.clone(),
                            username: username.clone(),
                            display_name: user_info.as_ref().and_then(|u| u.name.clone()),
                            avatar_url: user_info.as_ref().and_then(|u| u.avatar_url.clone()),
                        };
                        let _ = s.app_data.add_account(account);
                        let _ = s.app_data.save_vault_account(&vault_dir, &account_id);
                        if let Ok(path) = s.app_data.account_token_path(&account_id) {
                            if let Ok(store) = (s.token_store_for)(&path) {
                                let _ = store.save(&token_str);
                            }
                        }

                        settings.set_gitlab_connecting(false);
                        settings.set_gitlab_connected(true);
                        settings.set_gitlab_username(username.into());
                        settings.set_gitlab_instance(instance_display.into());
                        settings.set_gitlab_repo_name(repo_info.name.into());
                        settings.set_gitlab_status_message(SharedString::new());
                        settings.set_gitlab_status_is_error(false);
                        settings.set_url_draft(clone_url.into());
                        settings.set_has_token(true);

                        crate::callbacks::refresh_accounts_ui(&app, &s);
                        drop(s);

                        app.set_remote_configured(true);
                        set_status(&app, "GitLab account linked! Starting sync...");
                        start_sync(&app, &session);
                    }
                });
            });
        }
    });
}
