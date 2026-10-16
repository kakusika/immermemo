//! GitHub authentication, device-flow OAuth browser interaction, and repo setup.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use immermemo_provider::{AuthPollStatus, GitHubProvider, Provider};
use slint::{ComponentHandle, SharedString};

use crate::session::{SESSION, Session, set_status, start_sync};
use crate::{App, SettingsAdapter};

/// Updates GitHub connected/account state in the UI based on the current remote URL.
pub fn update_github_ui_state(app: &App, remote: Option<&str>) {
    let settings = app.global::<SettingsAdapter>();
    if let Some(url) = remote
        && let Some((user, repo)) = GitHubProvider::parse_remote(url)
    {
        settings.set_github_connected(true);
        settings.set_github_username(user.into());
        settings.set_github_repo_name(repo.into());
        return;
    }
    settings.set_github_connected(false);
    settings.set_github_username(SharedString::new());
    settings.set_github_repo_name(SharedString::new());
}

/// Binds GitHub connect, disconnect, cancel, and browser callbacks to the Slint App.
pub fn bind_github_callbacks(app: &App, session: &Rc<RefCell<Session>>) {
    let settings = app.global::<SettingsAdapter>();
    let active_auth_cancel = Rc::new(RefCell::new(None::<Arc<AtomicBool>>));

    settings.on_open_github_browser({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                let settings = app.global::<SettingsAdapter>();
                let uri = settings.get_github_verification_uri();
                if !uri.is_empty() {
                    let _ = webbrowser::open(uri.as_str());
                }
            }
        }
    });

    settings.on_cancel_github_auth({
        let (weak, cancel_flag) = (app.as_weak(), active_auth_cancel.clone());
        move || {
            if let Some(flag) = cancel_flag.borrow_mut().take() {
                flag.store(true, Ordering::SeqCst);
            }
            if let Some(app) = weak.upgrade() {
                let settings = app.global::<SettingsAdapter>();
                settings.set_github_auth_pending(false);
                settings.set_github_user_code(SharedString::new());
                settings.set_github_verification_uri(SharedString::new());
                settings.set_github_auth_status(SharedString::new());
            }
        }
    });

    settings.on_disconnect_github({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let mut s = session.borrow_mut();
            let vault_dir = s.vault_dir.clone();
            let _ = s.app_data.save_remote(&vault_dir, "");
            let _ = s.token_store.save("");
            s.remote = None;
            drop(s);
            app.set_remote_configured(false);
            let settings = app.global::<SettingsAdapter>();
            settings.set_url_draft(SharedString::new());
            settings.set_token_draft(SharedString::new());
            settings.set_has_token(false);
            update_github_ui_state(&app, None);
            set_status(&app, "Disconnected GitHub repository");
        }
    });

    settings.on_connect_github({
        let (weak, active_auth_cancel) = (app.as_weak(), active_auth_cancel);
        move || {
            let app = weak.unwrap();
            if let Some(flag) = active_auth_cancel.borrow_mut().take() {
                flag.store(true, Ordering::SeqCst);
            }
            let cancel_flag = Arc::new(AtomicBool::new(false));
            *active_auth_cancel.borrow_mut() = Some(cancel_flag.clone());

            let settings = app.global::<SettingsAdapter>();
            settings.set_github_auth_pending(true);
            settings.set_github_user_code(SharedString::new());
            settings.set_github_verification_uri(SharedString::new());
            settings.set_github_auth_status("Connecting to GitHub...".into());

            let thread_weak = weak.clone();
            std::thread::spawn(move || {
                let provider = GitHubProvider::default();
                let auth_session = match provider.start_auth() {
                    Ok(sess) => sess,
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = thread_weak.upgrade() {
                                let settings = app.global::<SettingsAdapter>();
                                settings.set_github_auth_pending(false);
                                settings.set_github_auth_status(
                                    format!("Connection failed: {e}").into(),
                                );
                            }
                        });
                        return;
                    }
                };

                let user_code = auth_session.user_code.clone();
                let uri = auth_session.verification_uri.clone();
                let weak_init = thread_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak_init.upgrade() {
                        let settings = app.global::<SettingsAdapter>();
                        settings.set_github_user_code(user_code.into());
                        settings.set_github_verification_uri(uri.clone().into());
                        settings
                            .set_github_auth_status("Waiting for browser authorization...".into());
                    }
                });

                let _ = webbrowser::open(&auth_session.verification_uri);

                let mut interval = std::time::Duration::from_secs(auth_session.interval.max(5));
                let token = loop {
                    std::thread::sleep(interval);
                    if cancel_flag.load(Ordering::SeqCst) {
                        return;
                    }

                    match provider.poll_auth(&auth_session) {
                        Ok(AuthPollStatus::Granted { access_token, .. }) => {
                            break access_token;
                        }
                        Ok(AuthPollStatus::Pending) => {}
                        Ok(AuthPollStatus::SlowDown) => {
                            interval += std::time::Duration::from_secs(5);
                        }
                        Ok(AuthPollStatus::Expired) => {
                            let weak = thread_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = weak.upgrade() {
                                    let settings = app.global::<SettingsAdapter>();
                                    settings.set_github_auth_pending(false);
                                    settings.set_github_auth_status(
                                        "Authorization session expired. Please try again.".into(),
                                    );
                                }
                            });
                            return;
                        }
                        Ok(AuthPollStatus::AccessDenied) => {
                            let weak = thread_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = weak.upgrade() {
                                    let settings = app.global::<SettingsAdapter>();
                                    settings.set_github_auth_pending(false);
                                    settings.set_github_auth_status(
                                        "Authorization was cancelled or denied on GitHub.".into(),
                                    );
                                }
                            });
                            return;
                        }
                        Err(e) => {
                            let weak = thread_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = weak.upgrade() {
                                    let settings = app.global::<SettingsAdapter>();
                                    settings.set_github_auth_pending(false);
                                    settings
                                        .set_github_auth_status(format!("Auth error: {e}").into());
                                }
                            });
                            return;
                        }
                    }
                };

                if cancel_flag.load(Ordering::SeqCst) {
                    return;
                }

                let weak_status = thread_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak_status.upgrade() {
                        let settings = app.global::<SettingsAdapter>();
                        settings.set_github_auth_status("Setting up private repository...".into());
                    }
                });

                let repo_info = match provider.setup_repository(&token, "immermemo-notes") {
                    Ok(info) => info,
                    Err(e) => {
                        let weak = thread_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = weak.upgrade() {
                                let settings = app.global::<SettingsAdapter>();
                                settings.set_github_auth_pending(false);
                                settings.set_github_auth_status(
                                    format!("Repository setup failed: {e}").into(),
                                );
                            }
                        });
                        return;
                    }
                };

                let user_info = provider.get_user(&token).ok();
                let username = user_info
                    .as_ref()
                    .map(|u| u.login.clone())
                    .or_else(|| GitHubProvider::parse_remote(&repo_info.clone_url).map(|(u, _)| u))
                    .unwrap_or_default();

                let weak_done = thread_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak_done.upgrade() {
                        let session = SESSION
                            .with(|s| s.borrow().clone())
                            .expect("session set in run");
                        let mut s = session.borrow_mut();
                        let vault_dir = s.vault_dir.clone();
                        let clone_url = repo_info.clone_url.clone();
                        if let Err(e) = s.app_data.save_remote(&vault_dir, &clone_url) {
                            drop(s);
                            set_status(&app, format!("Could not save remote: {e}"));
                            return;
                        }
                        if let Err(e) = s.token_store.save(&token) {
                            drop(s);
                            set_status(&app, format!("Could not save access token: {e}"));
                            return;
                        }
                        s.remote = Some(clone_url.clone());
                        let account_id = format!("github:{username}");
                        let account = immermemo_core::Account {
                            id: account_id.clone(),
                            provider: "github".to_string(),
                            instance_url: "https://github.com".to_string(),
                            username: username.clone(),
                            display_name: user_info.as_ref().and_then(|u| u.name.clone()),
                            avatar_url: user_info.as_ref().and_then(|u| u.avatar_url.clone()),
                        };
                        let _ = s.app_data.add_account(account);
                        let _ = s.app_data.save_vault_account(&vault_dir, &account_id);
                        if let Ok(path) = s.app_data.account_token_path(&account_id)
                            && let Ok(store) = (s.token_store_for)(&path)
                        {
                            let _ = store.save(&token);
                        }

                        let settings = app.global::<SettingsAdapter>();
                        settings.set_github_auth_pending(false);
                        settings.set_github_connected(true);
                        settings.set_github_username(username.into());
                        settings.set_github_repo_name(repo_info.name.into());
                        settings.set_url_draft(clone_url.into());
                        settings.set_has_token(true);

                        crate::callbacks::refresh_accounts_ui(&app, &s);
                        drop(s);

                        app.set_remote_configured(true);
                        set_status(&app, "GitHub account linked! Starting sync...");
                        start_sync(&app, &session);
                    }
                });
            });
        }
    });
}
