//! Connecting a Git hosting account: the Settings sheet's add-account flows
//! for GitHub (OAuth device flow, [`github`]) and for Forgejo / Gitea and
//! GitLab (pasted access token, [`token_flow`]).
//!
//! A connect flow only *authenticates*. It ends with the account registered
//! and its token kept ([`Connected::register`], then
//! [`immermemo_core::Session::add_account`]): no repository is created and no
//! vault is touched. Which account and repository a vault syncs through is a
//! separate, per-vault choice (`immermemo_core::VaultSync`).

mod github;
mod token_flow;

use crate::session::SessionRef;

use immermemo_core::Account;
use immermemo_provider::{ProviderId, UserInfo};

use crate::App;

/// Binds every provider's add-account callbacks to the Slint app.
pub fn bind_provider_callbacks(app: &App, session: &SessionRef) {
    github::bind(app, session);
    token_flow::bind::<token_flow::ForgejoUi>(app, session);
    token_flow::bind::<token_flow::GitLabUi>(app, session);
}

/// The id an account is stored under. GitHub has a single instance, so its
/// ids stay `github:<user>`; self-hostable providers also carry the instance.
pub(crate) fn account_id(provider: ProviderId, instance_url: &str, username: &str) -> String {
    match provider {
        ProviderId::GitHub => format!("github:{username}"),
        _ => format!("{}:{instance_url}:{username}", provider.as_str()),
    }
}

pub(crate) fn new_account(
    provider: ProviderId,
    instance_url: &str,
    username: &str,
    user: Option<&UserInfo>,
) -> Account {
    Account {
        id: account_id(provider, instance_url, username),
        provider: provider.as_str().to_owned(),
        instance_url: instance_url.to_owned(),
        username: username.to_owned(),
        display_name: user.and_then(|u| u.name.clone()),
        avatar_url: user.and_then(|u| u.avatar_url.clone()),
    }
}

/// What a connect flow has once the host has accepted the credentials.
pub(crate) struct Connected {
    pub provider: ProviderId,
    pub instance_url: String,
    pub token: String,
    pub user: UserInfo,
}

impl Connected {
    /// Registers the account (its token with it) and returns it. A second
    /// connection of the same account replaces the first's token.
    pub(crate) fn register(&self, session: &SessionRef) -> Result<Account, String> {
        let account = new_account(
            self.provider,
            &self.instance_url,
            &self.user.login,
            Some(&self.user),
        );
        session
            .borrow()
            .add_account(account.clone(), &self.token)
            .map_err(|e| format!("{e:#}"))?;
        Ok(account)
    }
}

/// The session, found again from inside an event-loop callback (which cannot
/// carry the `Rc` across threads).
pub(super) fn current_session() -> SessionRef {
    crate::session::SESSION
        .with(|s| s.borrow().clone())
        .expect("session set in run")
}
