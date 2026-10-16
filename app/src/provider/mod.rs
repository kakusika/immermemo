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

/// Registers an account for every known vault that has a remote URL but no
/// account yet -- vaults linked before accounts existed, or whose remote was
/// typed in by hand or came from an in-tree git repository. Each vault's own
/// token becomes its account's token.
///
/// Idempotent and cheap (a vault already linked to an account is skipped), so
/// it runs whenever the account or vault lists are rebuilt.
pub(crate) fn migrate_legacy_remotes(session: &Session) {
    for vault in &session.known_vaults {
        if session.app_data.load_vault_account(vault).is_some() {
            continue;
        }
        let Some(remote) = session
            .app_data
            .load_remote(vault)
            .and_then(|url| identify_remote(url.trim()))
        else {
            continue;
        };
        let instance_url = instance_url_for_host(remote.provider, &remote.host);
        let account = new_account(remote.provider, &instance_url, &remote.owner, None);
        let account_id = account.id.clone();
        // Read before linking: once the vault has an account, its token store
        // *is* the account's.
        let token = session
            .open_token_store(vault)
            .ok()
            .and_then(|store| store.load().ok().flatten());
        let _ = session.app_data.add_account(account);
        let _ = session.app_data.save_vault_account(vault, &account_id);
        if let Some(token) = token
            && let Ok(token_path) = session.app_data.account_token_path(&account_id)
            && let Ok(account_store) = (session.token_store_for)(&token_path)
        {
            let _ = account_store.save(&token);
        }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use immermemo_core::{AppData, NoteIndex, TokenStore};

    use super::*;

    #[derive(Default)]
    struct MemoryTokenStore(Mutex<Option<String>>);
    impl TokenStore for MemoryTokenStore {
        fn load(&self) -> anyhow::Result<Option<String>> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, token: &str) -> anyhow::Result<()> {
            *self.0.lock().unwrap() = Some(token.to_owned());
            Ok(())
        }
    }

    #[test]
    fn legacy_remotes_become_accounts_each_with_its_own_vaults_token() {
        let tmp = tempfile::TempDir::new().unwrap();
        let app_data = AppData::new(tmp.path().join("data"));
        let (github_vault, gitlab_vault) = (tmp.path().join("a"), tmp.path().join("b"));
        for (vault, remote) in [
            (&github_vault, "https://github.com/octocat/notes.git"),
            (&gitlab_vault, "git@gitlab.com:team/sub/notes.git"),
        ] {
            std::fs::create_dir_all(vault).unwrap();
            app_data.save_remote(vault, remote).unwrap();
        }

        // One token store per path, as the real factory hands out.
        let stores: Arc<Mutex<HashMap<PathBuf, Arc<MemoryTokenStore>>>> = Arc::default();
        let store_at = {
            let stores = stores.clone();
            move |path: &Path| -> Arc<MemoryTokenStore> {
                stores
                    .lock()
                    .unwrap()
                    .entry(path.to_owned())
                    .or_default()
                    .clone()
            }
        };
        store_at(&github_vault).save("github-token").unwrap();
        store_at(&gitlab_vault).save("gitlab-token").unwrap();

        let session = Session::new(immermemo_core::Session::new(
            github_vault.clone(),
            app_data.clone(),
            store_at(&github_vault),
            {
                let store_at = store_at.clone();
                Box::new(move |path| Ok(store_at(path) as Arc<dyn TokenStore>))
            },
            vec![github_vault.clone(), gitlab_vault.clone()],
            None,
            NoteIndex::open_in_memory().unwrap(),
        ));

        migrate_legacy_remotes(&session);
        migrate_legacy_remotes(&session); // idempotent

        assert_eq!(app_data.load_accounts().len(), 2);
        for (vault, account_id, token) in [
            (&github_vault, "github:octocat", "github-token"),
            (
                &gitlab_vault,
                "gitlab:https://gitlab.com:team/sub",
                "gitlab-token",
            ),
        ] {
            assert_eq!(
                app_data.load_vault_account(vault).as_deref(),
                Some(account_id)
            );
            let account_token_path = app_data.account_token_path(account_id).unwrap();
            assert_eq!(
                store_at(&account_token_path).load().unwrap().as_deref(),
                Some(token),
                "{account_id} must carry its own vault's token"
            );
        }
    }
}
