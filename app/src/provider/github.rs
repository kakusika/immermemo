//! Connecting GitHub through the OAuth device flow: show the user a code,
//! open the browser, poll until they approve, then register the account.

use crate::session::SessionRef;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use immermemo_provider::{AuthPollStatus, GitHubProvider, Provider, ProviderId};
use slint::{ComponentHandle, SharedString};

use super::{Connected, current_session};
use crate::session::on_ui_thread;
use crate::{App, SettingsAdapter};

/// Sets how far along the sign-in is; `pending` false ends the flow's UI.
fn show_auth_status(app: &App, pending: bool, status: impl Into<SharedString>) {
    let settings = app.global::<SettingsAdapter>();
    settings.set_github_auth_pending(pending);
    settings.set_github_auth_status(status.into());
}

/// Ends the flow's UI on `weak`'s window with `message`, from any thread.
fn fail(weak: &slint::Weak<App>, message: String) {
    on_ui_thread(weak, move |app| show_auth_status(app, false, message));
}

pub(super) fn bind(app: &App, _session: &SessionRef) {
    let settings = app.global::<SettingsAdapter>();
    // Set while a sign-in is in flight; a new attempt or a cancel raises it.
    let active_auth_cancel = Rc::new(RefCell::new(None::<Arc<AtomicBool>>));

    settings.on_open_github_browser({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                let uri = app
                    .global::<SettingsAdapter>()
                    .get_github_verification_uri();
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
                show_auth_status(&app, false, "");
                let settings = app.global::<SettingsAdapter>();
                settings.set_github_user_code(SharedString::new());
                settings.set_github_verification_uri(SharedString::new());
            }
        }
    });

    settings.on_connect_github({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            if let Some(flag) = active_auth_cancel.borrow_mut().take() {
                flag.store(true, Ordering::SeqCst);
            }
            let cancel = Arc::new(AtomicBool::new(false));
            *active_auth_cancel.borrow_mut() = Some(cancel.clone());

            let settings = app.global::<SettingsAdapter>();
            settings.set_github_user_code(SharedString::new());
            settings.set_github_verification_uri(SharedString::new());
            show_auth_status(&app, true, "Connecting to GitHub...");

            let weak = weak.clone();
            std::thread::spawn(move || sign_in(&weak, &cancel));
        }
    });
}

/// The whole sign-in, on its own thread. Returns quietly if `cancel` is raised.
fn sign_in(weak: &slint::Weak<App>, cancel: &AtomicBool) {
    let provider = GitHubProvider::default();
    let auth = match provider.start_auth() {
        Ok(auth) => auth,
        Err(e) => return fail(weak, format!("Connection failed: {e}")),
    };

    let (user_code, uri) = (auth.user_code.clone(), auth.verification_uri.clone());
    on_ui_thread(weak, move |app| {
        let settings = app.global::<SettingsAdapter>();
        settings.set_github_user_code(user_code.into());
        settings.set_github_verification_uri(uri.into());
        settings.set_github_auth_status("Waiting for browser authorization...".into());
    });
    let _ = webbrowser::open(&auth.verification_uri);

    let mut interval = Duration::from_secs(auth.interval.max(5));
    let token = loop {
        std::thread::sleep(interval);
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        match provider.poll_auth(&auth) {
            Ok(AuthPollStatus::Granted { access_token, .. }) => break access_token,
            Ok(AuthPollStatus::Pending) => {}
            Ok(AuthPollStatus::SlowDown) => interval += Duration::from_secs(5),
            Ok(AuthPollStatus::Expired) => {
                return fail(
                    weak,
                    "Authorization session expired. Please try again.".to_owned(),
                );
            }
            Ok(AuthPollStatus::AccessDenied) => {
                return fail(
                    weak,
                    "Authorization was cancelled or denied on GitHub.".to_owned(),
                );
            }
            Err(e) => return fail(weak, format!("Auth error: {e}")),
        }
    };
    if cancel.load(Ordering::SeqCst) {
        return;
    }

    on_ui_thread(weak, |app| {
        show_auth_status(app, true, "Reading your GitHub profile...");
    });
    let user = match provider.get_user(&token) {
        Ok(user) => user,
        Err(e) => return fail(weak, format!("Could not read your GitHub profile: {e}")),
    };
    let connected = Connected {
        provider: ProviderId::GitHub,
        instance_url: provider.instance_url().to_owned(),
        token,
        user,
    };
    on_ui_thread(weak, move |app| finish(app, &connected));
}

/// Back on the UI thread with the credentials accepted: register the account.
fn finish(app: &App, connected: &Connected) {
    let session = current_session();
    let account = match connected.register(&session) {
        Ok(account) => account,
        Err(e) => {
            show_auth_status(app, false, e.as_str());
            crate::session::set_error(app, e);
            return;
        }
    };
    let settings = app.global::<SettingsAdapter>();
    settings.set_github_user_code(SharedString::new());
    settings.set_github_verification_uri(SharedString::new());
    show_auth_status(app, false, "");
    settings.set_adding_account(false);
    crate::callbacks::refresh_accounts_ui(app, &session.borrow());
    crate::session::set_status(app, format!("GitHub account @{} added", account.username));
}
