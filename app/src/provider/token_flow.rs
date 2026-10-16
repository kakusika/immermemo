//! Connecting with a pasted Personal Access Token: Forgejo / Gitea and
//! GitLab. Their flows are identical; only the Slint properties and
//! callbacks they read and write (`forgejo-*` / `gitlab-*`) differ, which
//! [`TokenFlowUi`] abstracts. The token is checked by reading the profile it
//! belongs to; that profile is the account that gets registered.

use crate::session::SessionRef;

use immermemo_provider::{ProviderId, provider_for};
use slint::{ComponentHandle, SharedString};

use super::{Connected, current_session};
use crate::session::on_ui_thread;
use crate::{App, SettingsAdapter};

/// One token-based provider's slice of `SettingsAdapter`.
pub(super) trait TokenFlowUi: 'static {
    const PROVIDER: ProviderId;
    const EMPTY_TOKEN_MESSAGE: &'static str;

    fn set_connecting(settings: &SettingsAdapter, connecting: bool);
    fn set_status(settings: &SettingsAdapter, message: &str, is_error: bool);
    fn on_connect(
        settings: &SettingsAdapter,
        handler: impl Fn(SharedString, SharedString) + 'static,
    );
}

pub(super) struct ForgejoUi;

impl TokenFlowUi for ForgejoUi {
    const PROVIDER: ProviderId = ProviderId::Forgejo;
    const EMPTY_TOKEN_MESSAGE: &'static str = "Access token cannot be empty.";

    fn set_connecting(settings: &SettingsAdapter, connecting: bool) {
        settings.set_forgejo_connecting(connecting);
    }

    fn set_status(settings: &SettingsAdapter, message: &str, is_error: bool) {
        settings.set_forgejo_status_message(message.into());
        settings.set_forgejo_status_is_error(is_error);
    }

    fn on_connect(
        settings: &SettingsAdapter,
        handler: impl Fn(SharedString, SharedString) + 'static,
    ) {
        settings.on_connect_forgejo(handler);
    }
}

pub(super) struct GitLabUi;

impl TokenFlowUi for GitLabUi {
    const PROVIDER: ProviderId = ProviderId::GitLab;
    const EMPTY_TOKEN_MESSAGE: &'static str = "Personal access token cannot be empty.";

    fn set_connecting(settings: &SettingsAdapter, connecting: bool) {
        settings.set_gitlab_connecting(connecting);
    }

    fn set_status(settings: &SettingsAdapter, message: &str, is_error: bool) {
        settings.set_gitlab_status_message(message.into());
        settings.set_gitlab_status_is_error(is_error);
    }

    fn on_connect(
        settings: &SettingsAdapter,
        handler: impl Fn(SharedString, SharedString) + 'static,
    ) {
        settings.on_connect_gitlab(handler);
    }
}

pub(super) fn bind<P: TokenFlowUi>(app: &App, _session: &SessionRef) {
    let settings = app.global::<SettingsAdapter>();

    P::on_connect(&settings, {
        let weak = app.as_weak();
        move |instance, token| {
            let Some(app) = weak.upgrade() else { return };
            connect::<P>(&app, instance.trim(), token.trim());
        }
    });
}

fn connect<P: TokenFlowUi>(app: &App, instance: &str, token: &str) {
    let settings = app.global::<SettingsAdapter>();
    if token.is_empty() {
        P::set_status(&settings, P::EMPTY_TOKEN_MESSAGE, true);
        return;
    }
    P::set_connecting(&settings, true);
    P::set_status(&settings, "Connecting...", false);

    let instance = (!instance.is_empty()).then(|| instance.to_owned());
    let token = token.to_owned();
    let weak = app.as_weak();
    std::thread::spawn(move || {
        let provider = provider_for(P::PROVIDER, instance.as_deref());
        let user = match provider.get_user(&token) {
            Ok(user) => user,
            Err(e) => {
                report_failure::<P>(&weak, format!("Could not read the account's profile: {e}"));
                return;
            }
        };
        let connected = Connected {
            provider: P::PROVIDER,
            instance_url: provider.instance_url().to_owned(),
            token,
            user,
        };
        on_ui_thread(&weak, move |app| finish::<P>(app, &connected));
    });
}

/// Back on the UI thread with the credentials accepted: register the account.
fn finish<P: TokenFlowUi>(app: &App, connected: &Connected) {
    let session = current_session();
    let settings = app.global::<SettingsAdapter>();
    P::set_connecting(&settings, false);
    let account = match connected.register(&session) {
        Ok(account) => account,
        Err(e) => {
            P::set_status(&settings, &e, true);
            return;
        }
    };
    P::set_status(&settings, "", false);
    settings.set_adding_account(false);
    crate::callbacks::refresh_accounts_ui(app, &session.borrow());
    crate::callbacks::vault_setup::account_added(app, &account.id);
    crate::session::set_status(
        app,
        format!(
            "{} account @{} added",
            P::PROVIDER.display_name(),
            account.username
        ),
    );
}

fn report_failure<P: TokenFlowUi>(weak: &slint::Weak<App>, message: String) {
    on_ui_thread(weak, move |app| {
        let settings = app.global::<SettingsAdapter>();
        P::set_connecting(&settings, false);
        P::set_status(&settings, &message, true);
    });
}
