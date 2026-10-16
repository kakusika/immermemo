//! Git hosting provider abstractions and API clients (e.g. GitHub, Forgejo).
//!
//! This crate encapsulates remote account authentication (OAuth / Device Authorization Flow
//! RFC 8628) and automated repository setup (provisioning and clone URL resolution),
//! isolating external network and HTTP dependencies from local vault storage.

mod api;
pub mod forgejo;
pub mod github;
pub mod gitlab;
mod remote_url;
#[cfg(test)]
mod test_support;

pub use forgejo::{DEFAULT_CODEBERG_INSTANCE, ForgejoProvider};
pub use github::GitHubProvider;
pub use gitlab::{DEFAULT_GITLAB_INSTANCE, GitLabProvider};

use serde::{Deserialize, Serialize};

/// The Git hosting services this crate can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderId {
    GitHub,
    Forgejo,
    GitLab,
}

impl ProviderId {
    /// Every provider, in the order the UI lists them.
    pub const ALL: [ProviderId; 3] = [Self::GitHub, Self::Forgejo, Self::GitLab];

    /// The stable machine identifier persisted with an account (`"github"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Forgejo => "forgejo",
            Self::GitLab => "gitlab",
        }
    }

    /// Inverse of [`as_str`](Self::as_str); `None` for an unknown identifier.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == id)
    }

    /// The `(username, password)` this host expects for HTTPS basic auth
    /// carrying the access token `token`: GitLab wants the fixed username
    /// `oauth2` with the token as the password; GitHub, Gitea and Forgejo
    /// take the token as the username and an empty password.
    pub fn https_basic_auth(self, token: &str) -> (String, String) {
        match self {
            Self::GitLab => ("oauth2".to_owned(), token.to_owned()),
            Self::GitHub | Self::Forgejo => (token.to_owned(), String::new()),
        }
    }

    /// Human-readable name for status messages.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::Forgejo => "Forgejo / Gitea",
            Self::GitLab => "GitLab",
        }
    }
}

/// Builds the client for `id`. `instance_url` selects a self-hosted
/// instance (Forgejo, GitLab); `None` means the public default, and GitHub
/// has no instances.
pub fn provider_for(id: ProviderId, instance_url: Option<&str>) -> Box<dyn Provider> {
    match id {
        ProviderId::GitHub => Box::new(GitHubProvider::default()),
        ProviderId::Forgejo => Box::new(ForgejoProvider::new(instance_url)),
        ProviderId::GitLab => Box::new(GitLabProvider::new(instance_url)),
    }
}

/// The HTTPS basic-auth pair for `token` against `remote_url`'s host. A
/// remote that is not recognized as any provider's (a plain self-hosted git
/// server) gets the common convention: the token as the username.
pub fn https_basic_auth_for_remote(remote_url: &str, token: &str) -> (String, String) {
    match identify_remote(remote_url) {
        Some(remote) => remote.provider.https_basic_auth(token),
        None => (token.to_owned(), String::new()),
    }
}

/// Which provider a git remote URL belongs to, and where in it the
/// repository lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteIdentity {
    pub provider: ProviderId,
    /// Host as written in the URL (with the port for HTTP(S) remotes).
    pub host: String,
    /// User, organization or (GitLab) `group/subgroup`.
    pub owner: String,
    pub repo: String,
}

/// Classifies `url`: `github.com` is GitHub, a host containing `gitlab` is
/// GitLab, and any other HTTPS/SSH remote is taken to be Forgejo / Gitea --
/// there is no way to tell a self-hosted instance apart from its URL alone.
pub fn identify_remote(url: &str) -> Option<RemoteIdentity> {
    if let Some((owner, repo)) = GitHubProvider::parse_remote(url) {
        return Some(RemoteIdentity {
            provider: ProviderId::GitHub,
            host: "github.com".to_owned(),
            owner,
            repo,
        });
    }
    if let Some((host, owner, repo)) = GitLabProvider::parse_any_remote(url)
        && gitlab::is_gitlab_host(&host)
    {
        return Some(RemoteIdentity {
            provider: ProviderId::GitLab,
            host,
            owner,
            repo,
        });
    }
    let (host, owner, repo) = ForgejoProvider::parse_any_remote(url)?;
    Some(RemoteIdentity {
        provider: ProviderId::Forgejo,
        host,
        owner,
        repo,
    })
}

/// An in-progress device authorization flow session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSession {
    /// Device verification code used by the client for polling.
    pub device_code: String,
    /// Human-friendly user verification code shown on screen (e.g. `WDAS-5821`).
    pub user_code: String,
    /// Web URL the user visits to enter or confirm the code.
    pub verification_uri: String,
    /// Lifetime of the verification code in seconds.
    pub expires_in: u64,
    /// Minimum polling interval in seconds.
    pub interval: u64,
}

/// The status returned when polling the authorization server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthPollStatus {
    /// The user has not yet authorized or rejected the request; continue polling.
    Pending,
    /// The client is polling too frequently; increase the polling interval.
    SlowDown,
    /// The user successfully granted authorization.
    Granted {
        access_token: String,
        token_type: String,
        scope: String,
    },
    /// The device code has expired without authorization.
    Expired,
    /// The user explicitly denied the authorization request.
    AccessDenied,
}

/// Profile information of the authenticated user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInfo {
    /// Username or account handle (e.g. `octocat`).
    pub login: String,
    /// Display name if configured.
    pub name: Option<String>,
    /// Primary email if available.
    pub email: Option<String>,
    /// Web avatar image URL.
    pub avatar_url: Option<String>,
}

/// Information about a remote repository used for vault synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInfo {
    /// Repository name (e.g. `immermemo-notes`).
    pub name: String,
    /// Full path or identifier (e.g. `octocat/immermemo-notes`).
    pub full_name: String,
    /// HTTPS clone URL used by git.
    pub clone_url: String,
    /// Whether the repository is private.
    pub is_private: bool,
    /// Default branch name (e.g. `main`).
    pub default_branch: String,
}

/// The `{name, full_name, clone_url, private, default_branch}` repository
/// object GitHub and Forgejo / Gitea share.
#[derive(Deserialize)]
pub(crate) struct GitStyleRepo {
    name: String,
    full_name: String,
    clone_url: String,
    private: bool,
    default_branch: Option<String>,
}

impl From<GitStyleRepo> for RepositoryInfo {
    fn from(repo: GitStyleRepo) -> Self {
        Self {
            name: repo.name,
            full_name: repo.full_name,
            clone_url: repo.clone_url,
            is_private: repo.private,
            default_branch: repo.default_branch.unwrap_or_else(|| "main".to_owned()),
        }
    }
}

/// Body of the "create a user repository" request GitHub and Forgejo /
/// Gitea share.
#[derive(Serialize)]
pub(crate) struct CreateGitStyleRepo<'a> {
    name: &'a str,
    description: &'static str,
    private: bool,
    auto_init: bool,
}

impl<'a> CreateGitStyleRepo<'a> {
    pub(crate) fn new(name: &'a str, private: bool) -> Self {
        Self {
            name,
            description: "Immermemo notes vault",
            private,
            auto_init: false,
        }
    }
}

/// Abstraction over a Git hosting provider (GitHub, Forgejo, GitLab, etc.).
pub trait Provider: Send + Sync {
    /// Which hosting service this is.
    fn id(&self) -> ProviderId;

    /// Human-readable display label (e.g. `GitHub`).
    fn display_name(&self) -> &str {
        self.id().display_name()
    }

    /// Base URL of the instance this client talks to (`https://github.com`,
    /// `https://codeberg.org`, ...).
    fn instance_url(&self) -> &str;

    /// Initiates a device authorization flow session (RFC 8628).
    fn start_auth(&self) -> anyhow::Result<AuthSession>;

    /// Polls the provider for the authorization grant status.
    fn poll_auth(&self, session: &AuthSession) -> anyhow::Result<AuthPollStatus>;

    /// Fetches profile information for the authenticated user.
    fn get_user(&self, token: &str) -> anyhow::Result<UserInfo>;

    /// Looks up the repository `owner/repo_name`; `None` if it does not exist.
    fn find_repository(
        &self,
        token: &str,
        owner: &str,
        repo_name: &str,
    ) -> anyhow::Result<Option<RepositoryInfo>>;

    /// Ensures that a synchronization repository with `repo_name` exists:
    /// finds the user's existing one, or creates a new private repository.
    fn setup_repository(&self, token: &str, repo_name: &str) -> anyhow::Result<RepositoryInfo> {
        let user = self.get_user(token)?;
        match self.find_repository(token, &user.login, repo_name)? {
            Some(existing) => Ok(existing),
            None => self.create_repository(token, repo_name, true),
        }
    }

    /// Lists remote repositories accessible with the provided token.
    fn list_repositories(&self, token: &str) -> anyhow::Result<Vec<RepositoryInfo>>;

    /// Creates a new remote repository with the specified name and visibility.
    fn create_repository(
        &self,
        token: &str,
        repo_name: &str,
        is_private: bool,
    ) -> anyhow::Result<RepositoryInfo>;

    /// Extracts repository owner/user and repository name from a remote URL if it belongs to this provider.
    fn parse_remote_url(&self, url: &str) -> Option<(String, String)>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_round_trip_through_their_stored_string() {
        for id in ProviderId::ALL {
            assert_eq!(ProviderId::from_id(id.as_str()), Some(id));
        }
        assert_eq!(ProviderId::from_id("bitbucket"), None);
    }

    #[test]
    fn remotes_are_attributed_to_the_right_provider() {
        let who = |url| identify_remote(url).map(|r| (r.provider, r.host, r.owner, r.repo));
        assert_eq!(
            who("https://github.com/octocat/notes.git"),
            Some((
                ProviderId::GitHub,
                "github.com".into(),
                "octocat".into(),
                "notes".into()
            ))
        );
        assert_eq!(
            who("git@gitlab.com:team/sub/notes.git"),
            Some((
                ProviderId::GitLab,
                "gitlab.com".into(),
                "team/sub".into(),
                "notes".into()
            ))
        );
        assert_eq!(
            who("https://git.example.com:3000/bob/notes"),
            Some((
                ProviderId::Forgejo,
                "git.example.com:3000".into(),
                "bob".into(),
                "notes".into()
            ))
        );
        assert_eq!(who("not a url"), None);
    }

    #[test]
    fn each_host_gets_the_basic_auth_pair_it_expects() {
        let auth = |url| https_basic_auth_for_remote(url, "tok");
        assert_eq!(
            auth("https://gitlab.com/a/b.git"),
            ("oauth2".into(), "tok".into())
        );
        assert_eq!(
            auth("https://gitlab.corp.net/team/sub/b.git"),
            ("oauth2".into(), "tok".into())
        );
        assert_eq!(
            auth("https://github.com/a/b.git"),
            ("tok".into(), String::new())
        );
        assert_eq!(
            auth("https://codeberg.org/a/b.git"),
            ("tok".into(), String::new())
        );
        assert_eq!(
            auth("https://git.example.com/some/deep/path.git"),
            ("tok".into(), String::new()),
            "an unrecognized host takes the common convention"
        );
    }

    #[test]
    fn a_nested_path_on_a_non_gitlab_host_is_not_a_forgejo_remote() {
        assert_eq!(identify_remote("https://git.example.com/a/b/c"), None);
    }
}
