//! Connecting with a pasted Personal Access Token: Forgejo / Gitea and
//! GitLab. Their flows are identical; only the Slint properties and
//! callbacks they read and write (`forgejo-*` / `gitlab-*`) differ, which
//! [`TokenFlowUi`] abstracts.

use std::cell::RefCell;
use std::rc::Rc;

use immermemo_provider::{ProviderId, RemoteIdentity, provider_for};
use slint::{ComponentHandle, SharedString};

use super::{DEFAULT_REPO_NAME, Linked, current_session, disconnect};
use crate::session::{Session, on_ui_thread};
use crate::{App, SettingsAdapter};

/// A linked account as the Settings sheet displays it.
pub(super) struct AccountView<'a> {
    pub username: &'a str,
    pub instance: &'a str,
    pub repo: &'a str,
}

/// One token-based provider's slice of `SettingsAdapter`.
pub(super) trait TokenFlowUi: 'static {
    const PROVIDER: ProviderId;
    const EMPTY_TOKEN_MESSAGE: &'static str;

    fn set_account(settings: &SettingsAdapter, account: Option<AccountView>);
    fn set_connecting(settings: &SettingsAdapter, connecting: bool);
    fn set_status(settings: &SettingsAdapter, message: &str, is_error: bool);
    fn on_connect(
        settings: &SettingsAdapter,
        handler: impl Fn(SharedString, SharedString, SharedString) + 'static,
    );
    fn on_disconnect(settings: &SettingsAdapter, handler: impl Fn() + 'static);

    /// Shows `remote` as this provider's connected account, or clears it.
    /// The instance shown is the remote's host, as written in its URL.
    fn show_remote(settings: &SettingsAdapter, remote: Option<&RemoteIdentity>) {
        Self::set_account(
            settings,
            remote.map(|r| AccountView {
                username: &r.owner,
                instance: &r.host,
                repo: &r.repo,
            }),
        );
    }
}

pub(super) struct ForgejoUi;

impl TokenFlowUi for ForgejoUi {
    const PROVIDER: ProviderId = ProviderId::Forgejo;
    const EMPTY_TOKEN_MESSAGE: &'static str = "Access token cannot be empty.";

    fn set_account(settings: &SettingsAdapter, account: Option<AccountView>) {
        settings.set_forgejo_connected(account.is_some());
        let account = account.unwrap_or(AccountView {
            username: "",
            instance: "",
            repo: "",
        });
        settings.set_forgejo_username(account.username.into());
        settings.set_forgejo_instance(account.instance.into());
        settings.set_forgejo_repo_name(account.repo.into());
    }

    fn set_connecting(settings: &SettingsAdapter, connecting: bool) {
        settings.set_forgejo_connecting(connecting);
    }

    fn set_status(settings: &SettingsAdapter, message: &str, is_error: bool) {
        settings.set_forgejo_status_message(message.into());
        settings.set_forgejo_status_is_error(is_error);
    }

    fn on_connect(
        settings: &SettingsAdapter,
        handler: impl Fn(SharedString, SharedString, SharedString) + 'static,
    ) {
        settings.on_connect_forgejo(handler);
    }

    fn on_disconnect(settings: &SettingsAdapter, handler: impl Fn() + 'static) {
        settings.on_disconnect_forgejo(handler);
    }
}

pub(super) struct GitLabUi;

impl TokenFlowUi for GitLabUi {
    const PROVIDER: ProviderId = ProviderId::GitLab;
    const EMPTY_TOKEN_MESSAGE: &'static str = "Personal access token cannot be empty.";

    fn set_account(settings: &SettingsAdapter, account: Option<AccountView>) {
        settings.set_gitlab_connected(account.is_some());
        let account = account.unwrap_or(AccountView {
            username: "",
            instance: "",
            repo: "",
        });
        settings.set_gitlab_username(account.username.into());
        settings.set_gitlab_instance(account.instance.into());
        settings.set_gitlab_repo_name(account.repo.into());
    }

    fn set_connecting(settings: &SettingsAdapter, connecting: bool) {
        settings.set_gitlab_connecting(connecting);
    }

    fn set_status(settings: &SettingsAdapter, message: &str, is_error: bool) {
        settings.set_gitlab_status_message(message.into());
        settings.set_gitlab_status_is_error(is_error);
    }

    fn on_connect(
        settings: &SettingsAdapter,
        handler: impl Fn(SharedString, SharedString, SharedString) + 'static,
    ) {
        settings.on_connect_gitlab(handler);
    }

    fn on_disconnect(settings: &SettingsAdapter, handler: impl Fn() + 'static) {
        settings.on_disconnect_gitlab(handler);
    }
}

pub(super) fn bind<P: TokenFlowUi>(app: &App, session: &Rc<RefCell<Session>>) {
    let settings = app.global::<SettingsAdapter>();

    P::on_disconnect(&settings, {
        let (weak, session) = (app.as_weak(), session.clone());
        move || disconnect(&weak.unwrap(), &session, P::PROVIDER)
    });

    P::on_connect(&settings, {
        let weak = app.as_weak();
        move |instance, token, repo| {
            let Some(app) = weak.upgrade() else { return };
            connect::<P>(&app, instance.trim(), token.trim(), repo.trim());
        }
    });
}

fn connect<P: TokenFlowUi>(app: &App, instance: &str, token: &str, repo_name: &str) {
    let settings = app.global::<SettingsAdapter>();
    if token.is_empty() {
        P::set_status(&settings, P::EMPTY_TOKEN_MESSAGE, true);
        return;
    }
    P::set_connecting(&settings, true);
    P::set_status(
        &settings,
        "Connecting and setting up private repository...",
        false,
    );

    let instance = (!instance.is_empty()).then(|| instance.to_owned());
    let token = token.to_owned();
    let repo_name = if repo_name.is_empty() {
        DEFAULT_REPO_NAME.to_owned()
    } else {
        repo_name.to_owned()
    };
    let weak = app.as_weak();
    std::thread::spawn(move || {
        let provider = provider_for(P::PROVIDER, instance.as_deref());
        let repo = match provider.setup_repository(&token, &repo_name) {
            Ok(repo) => repo,
            Err(e) => {
                report_failure::<P>(&weak, format!("Repository setup failed: {e}"));
                return;
            }
        };
        let linked = Linked {
            provider: P::PROVIDER,
            instance_url: provider.instance_url().to_owned(),
            user: provider.get_user(&token).ok(),
            token,
            repo,
        };
        on_ui_thread(&weak, move |app| finish::<P>(app, &linked));
    });
}

/// Back on the UI thread with the repository in hand: persist, show, sync.
fn finish<P: TokenFlowUi>(app: &App, linked: &Linked) {
    let session = current_session();
    let settings = app.global::<SettingsAdapter>();
    let username = match linked.persist(&session) {
        Ok(username) => username,
        Err(e) => {
            P::set_connecting(&settings, false);
            P::set_status(&settings, &e, true);
            return;
        }
    };
    P::set_connecting(&settings, false);
    P::set_account(
        &settings,
        Some(AccountView {
            username: &username,
            instance: &linked.instance_url,
            repo: &linked.repo.name,
        }),
    );
    P::set_status(&settings, "", false);
    linked.show_and_sync(app, &session);
}

fn report_failure<P: TokenFlowUi>(weak: &slint::Weak<App>, message: String) {
    on_ui_thread(weak, move |app| {
        let settings = app.global::<SettingsAdapter>();
        P::set_connecting(&settings, false);
        P::set_status(&settings, &message, true);
    });
}
