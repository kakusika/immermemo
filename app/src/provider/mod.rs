//! Linking a vault to a Git hosting account: the Settings sheet's connect /
//! disconnect flows for GitHub (OAuth device flow, [`github`]) and for
//! Forgejo / Gitea and GitLab (pasted access token, [`token_flow`]).
//!
//! What the flows have in common lives here: how a freshly created repository
//! becomes the vault's remote and a registered account, how a vault is
//! disconnected again, and how the current remote URL is reflected in the
//! sheet. The domain half (what gets persisted) is
//! [`immermemo_core::Session::link_remote`] / `unlink_remote`.

mod github;
mod token_flow;

use std::cell::RefCell;
use std::rc::Rc;

use immermemo_core::Account;
use immermemo_provider::{ProviderId, RepositoryInfo, UserInfo, identify_remote};
use slint::{ComponentHandle, SharedString};
use token_flow::TokenFlowUi as _;

use crate::session::{Session, set_status, start_sync};
use crate::{App, SettingsAdapter};

/// Repository created when the user does not name one.
pub(crate) const DEFAULT_REPO_NAME: &str = "immermemo-notes";

/// Binds every provider's connect / disconnect callbacks to the Slint app.
pub fn bind_provider_callbacks(app: &App, session: &Rc<RefCell<Session>>) {
    github::bind(app, session);
    token_flow::bind::<token_flow::ForgejoUi>(app, session);
    token_flow::bind::<token_flow::GitLabUi>(app, session);
}

/// Reflects the vault's current remote URL in the Settings sheet: the
/// provider it belongs to shows as connected, the others as not.
pub fn update_provider_ui_state(app: &App, remote: Option<&str>) {
    let settings = app.global::<SettingsAdapter>();
    let identity = remote.and_then(identify_remote);
    let of = |provider| identity.as_ref().filter(|i| i.provider == provider);

    match of(ProviderId::GitHub) {
        Some(i) => {
            settings.set_github_connected(true);
            settings.set_github_username(i.owner.as_str().into());
            settings.set_github_repo_name(i.repo.as_str().into());
        }
        None => {
            settings.set_github_connected(false);
            settings.set_github_username(SharedString::new());
            settings.set_github_repo_name(SharedString::new());
        }
    }
    token_flow::ForgejoUi::show_remote(&settings, of(ProviderId::Forgejo));
    token_flow::GitLabUi::show_remote(&settings, of(ProviderId::GitLab));
}

/// The id an account is stored under. GitHub has a single instance, so its
/// ids stay `github:<user>`; self-hostable providers also carry the instance.
pub(crate) fn account_id(provider: ProviderId, instance_url: &str, username: &str) -> String {
    match provider {
        ProviderId::GitHub => format!("github:{username}"),
        _ => format!("{}:{instance_url}:{username}", provider.as_str()),
    }
}

/// `host` of a remote URL as an instance URL (remotes carry no scheme here).
pub(crate) fn instance_url_for_host(provider: ProviderId, host: &str) -> String {
    match provider {
        ProviderId::GitHub => "https://github.com".to_owned(),
        _ if host.starts_with("http://") || host.starts_with("https://") => host.to_owned(),
        _ => format!("https://{host}"),
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

/// What a connect flow has once it holds a repository for the account.
struct Linked {
    provider: ProviderId,
    instance_url: String,
    token: String,
    user: Option<UserInfo>,
    repo: RepositoryInfo,
}

impl Linked {
    /// The account's handle: the profile's login, or failing that the owner
    /// the new repository's clone URL names.
    fn username(&self) -> String {
        self.user
            .as_ref()
            .map(|u| u.login.clone())
            .or_else(|| identify_remote(&self.repo.clone_url).map(|i| i.owner))
            .unwrap_or_default()
    }

    /// Makes the repository the vault's remote and registers the account.
    /// On success, returns the account's username.
    fn persist(&self, session: &Rc<RefCell<Session>>) -> Result<String, String> {
        let username = self.username();
        let account = new_account(
            self.provider,
            &self.instance_url,
            &username,
            self.user.as_ref(),
        );
        session
            .borrow_mut()
            .link_remote(account, &self.repo.clone_url, &self.token)
            .map_err(|e| format!("{e:#}"))?;
        Ok(username)
    }

    /// The part of the Settings sheet every provider shares once linked,
    /// then the first sync.
    fn show_and_sync(&self, app: &App, session: &Rc<RefCell<Session>>) {
        let settings = app.global::<SettingsAdapter>();
        settings.set_url_draft(self.repo.clone_url.as_str().into());
        settings.set_has_token(true);
        crate::callbacks::refresh_accounts_ui(app, &session.borrow());
        app.set_remote_configured(true);
        set_status(
            app,
            format!(
                "{} account linked! Starting sync...",
                self.provider.display_name()
            ),
        );
        start_sync(app, session);
    }
}

/// Forgets the vault's remote and token, and clears the sheet.
fn disconnect(app: &App, session: &Rc<RefCell<Session>>, provider: ProviderId) {
    session.borrow_mut().unlink_remote();
    app.set_remote_configured(false);
    let settings = app.global::<SettingsAdapter>();
    settings.set_url_draft(SharedString::new());
    settings.set_token_draft(SharedString::new());
    settings.set_has_token(false);
    update_provider_ui_state(app, None);
    set_status(
        app,
        format!("Disconnected {} repository", provider.display_name()),
    );
}

/// The session, found again from inside an event-loop callback (which cannot
/// carry the `Rc` across threads).
fn current_session() -> Rc<RefCell<Session>> {
    crate::session::SESSION
        .with(|s| s.borrow().clone())
        .expect("session set in run")
}
