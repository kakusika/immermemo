//! Git hosting provider abstractions and API clients (e.g. GitHub, Forgejo).
//!
//! This crate encapsulates remote account authentication (OAuth / Device Authorization Flow
//! RFC 8628) and automated repository setup (provisioning and clone URL resolution),
//! isolating external network and HTTP dependencies from local vault storage.

pub mod forgejo;
pub mod github;

pub use forgejo::{DEFAULT_CODEBERG_INSTANCE, ForgejoProvider};
pub use github::GitHubProvider;

use serde::{Deserialize, Serialize};

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

/// Abstraction over a Git hosting provider (GitHub, Forgejo, GitLab, etc.).
pub trait Provider: Send + Sync {
    /// Unique machine identifier for this provider (`github`, `forgejo`, etc.).
    fn id(&self) -> &'static str;

    /// Human-readable display label (e.g. `GitHub`, `Forgejo`).
    fn display_name(&self) -> &str;

    /// Initiates a device authorization flow session (RFC 8628).
    fn start_auth(&self) -> anyhow::Result<AuthSession>;

    /// Polls the provider for the authorization grant status.
    fn poll_auth(&self, session: &AuthSession) -> anyhow::Result<AuthPollStatus>;

    /// Fetches profile information for the authenticated user.
    fn get_user(&self, token: &str) -> anyhow::Result<UserInfo>;

    /// Ensures that a synchronization repository with `repo_name` exists:
    /// searches for an existing one, or creates a new private repository if absent.
    fn setup_repository(&self, token: &str, repo_name: &str) -> anyhow::Result<RepositoryInfo>;

    /// Extracts repository owner/user and repository name from a remote URL if it belongs to this provider.
    fn parse_remote_url(&self, url: &str) -> Option<(String, String)>;
}
