//! Forgejo / Gitea (Codeberg and self-hosted) Git hosting provider.
//!
//! Authenticates with a Personal Access Token (these hosts have no device
//! authorization flow) against the `/api/v1` REST endpoints.

use serde::Deserialize;

use crate::api::{Api, Auth};
use crate::remote_url::{normalize_instance_url, parse_any_remote, parse_remote_for_instance};
use crate::{
    AuthPollStatus, AuthSession, CreateGitStyleRepo, GitStyleRepo, Provider, ProviderId,
    RepositoryInfo, UserInfo,
};

/// Default public Forgejo instance.
pub const DEFAULT_CODEBERG_INSTANCE: &str = "https://codeberg.org";

/// Forgejo / Gitea Git hosting provider.
#[derive(Debug, Clone)]
pub struct ForgejoProvider {
    instance_url: String,
    api: Api,
}

impl Default for ForgejoProvider {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ForgejoProvider {
    /// Creates a new [`ForgejoProvider`] pointing to the specified instance URL
    /// (e.g. `https://codeberg.org` or `https://git.example.com`).
    /// If `None` is provided, defaults to [`DEFAULT_CODEBERG_INSTANCE`].
    pub fn new(instance_url: Option<&str>) -> Self {
        let base = normalize_instance_url(
            instance_url.unwrap_or(DEFAULT_CODEBERG_INSTANCE),
            DEFAULT_CODEBERG_INSTANCE,
        );
        let api = Api::new(format!("{base}/api/v1"), Auth::Token, "application/json");
        Self {
            instance_url: base,
            api,
        }
    }

    /// For tests: creates a provider directly pointing to a custom mock API base URL.
    #[cfg(test)]
    pub fn with_custom_api_base(api_base_url: String) -> Self {
        Self {
            api: Api::new(api_base_url.clone(), Auth::Token, "application/json"),
            instance_url: api_base_url,
        }
    }

    /// Returns the normalized instance base URL (e.g. `https://codeberg.org`).
    pub fn instance_url(&self) -> &str {
        &self.instance_url
    }

    /// Extracts repository owner and repository name from a git remote URL given an instance base URL.
    /// Handles HTTPS and SSH formats (e.g. `https://codeberg.org/alice/notes.git` or `git@codeberg.org:alice/notes.git`).
    pub fn parse_remote(instance_base: &str, url: &str) -> Option<(String, String)> {
        let base = normalize_instance_url(instance_base, DEFAULT_CODEBERG_INSTANCE);
        parse_remote_for_instance(&base, url, false)
    }

    /// Extracts instance host, repository owner, and repository name from a git remote URL
    /// across HTTPS and SSH formats (e.g. `https://codeberg.org/alice/notes.git` or `git@codeberg.org:alice/notes.git`).
    pub fn parse_any_remote(url: &str) -> Option<(String, String, String)> {
        parse_any_remote(url, false)
    }
}

#[derive(Deserialize)]
struct ForgejoUser {
    // Older Gitea builds only fill `username`.
    login: Option<String>,
    username: Option<String>,
    full_name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

impl Provider for ForgejoProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Forgejo
    }

    fn instance_url(&self) -> &str {
        &self.instance_url
    }

    fn start_auth(&self) -> anyhow::Result<AuthSession> {
        anyhow::bail!(
            "Device authorization flow is not supported by Forgejo / Gitea. Please authenticate with a Personal Access Token."
        )
    }

    fn poll_auth(&self, _session: &AuthSession) -> anyhow::Result<AuthPollStatus> {
        anyhow::bail!("Device authorization flow is not supported by Forgejo / Gitea.")
    }

    fn get_user(&self, token: &str) -> anyhow::Result<UserInfo> {
        let user: ForgejoUser = self.api.get("/user", token)?;
        let login = user
            .login
            .or(user.username)
            .ok_or_else(|| anyhow::anyhow!("Forgejo user response missing login or username"))?;
        Ok(UserInfo {
            login,
            name: user.full_name,
            email: user.email,
            avatar_url: user.avatar_url,
        })
    }

    fn find_repository(
        &self,
        token: &str,
        owner: &str,
        repo_name: &str,
    ) -> anyhow::Result<Option<RepositoryInfo>> {
        let repo: Option<GitStyleRepo> = self
            .api
            .get_optional(&format!("/repos/{owner}/{repo_name}"), token)?;
        Ok(repo.map(Into::into))
    }

    fn list_repositories(&self, token: &str) -> anyhow::Result<Vec<RepositoryInfo>> {
        let repos: Vec<GitStyleRepo> = self.api.get("/user/repos", token)?;
        Ok(repos.into_iter().map(Into::into).collect())
    }

    fn create_repository(
        &self,
        token: &str,
        repo_name: &str,
        is_private: bool,
    ) -> anyhow::Result<RepositoryInfo> {
        let repo: GitStyleRepo = self.api.post(
            "/user/repos",
            token,
            &CreateGitStyleRepo::new(repo_name, is_private),
        )?;
        Ok(repo.into())
    }

    fn parse_remote_url(&self, url: &str) -> Option<(String, String)> {
        Self::parse_remote(&self.instance_url, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{OK, serve, serve_once};

    #[test]
    fn an_empty_instance_falls_back_to_codeberg() {
        assert_eq!(
            ForgejoProvider::new(Some("")).instance_url(),
            DEFAULT_CODEBERG_INSTANCE
        );
        assert_eq!(
            ForgejoProvider::new(Some("git.local:3000/")).instance_url(),
            "https://git.local:3000"
        );
    }

    #[test]
    fn test_parse_remote() {
        assert_eq!(
            ForgejoProvider::parse_remote(
                "https://codeberg.org",
                "https://codeberg.org/alice/immermemo-notes.git"
            ),
            Some(("alice".to_owned(), "immermemo-notes".to_owned()))
        );
        assert_eq!(
            ForgejoProvider::parse_remote(
                "https://codeberg.org",
                "git@codeberg.org:alice/immermemo-notes.git"
            ),
            Some(("alice".to_owned(), "immermemo-notes".to_owned()))
        );
        assert_eq!(
            ForgejoProvider::parse_remote(
                "https://git.example.com:3000",
                "https://git.example.com:3000/bob/notes"
            ),
            Some(("bob".to_owned(), "notes".to_owned()))
        );
        assert_eq!(
            ForgejoProvider::parse_remote(
                "https://codeberg.org",
                "https://github.com/alice/notes.git"
            ),
            None
        );
        assert_eq!(
            ForgejoProvider::parse_remote(
                "https://codeberg.org",
                "https://codeberg.org/team/sub/notes.git"
            ),
            None,
            "Forgejo has no nested namespaces"
        );
    }

    #[test]
    fn test_parse_any_remote() {
        assert_eq!(
            ForgejoProvider::parse_any_remote("https://codeberg.org/alice/notes.git"),
            Some((
                "codeberg.org".to_owned(),
                "alice".to_owned(),
                "notes".to_owned()
            ))
        );
        assert_eq!(
            ForgejoProvider::parse_any_remote("git@codeberg.org:alice/notes.git"),
            Some((
                "codeberg.org".to_owned(),
                "alice".to_owned(),
                "notes".to_owned()
            ))
        );
        assert_eq!(
            ForgejoProvider::parse_any_remote("https://git.example.com:3000/bob/notes"),
            Some((
                "git.example.com:3000".to_owned(),
                "bob".to_owned(),
                "notes".to_owned()
            ))
        );
        assert_eq!(ForgejoProvider::parse_any_remote("invalid-url"), None);
    }

    #[test]
    fn test_get_user() {
        let (url, server) = serve_once(
            OK,
            r#"{
                "login": "codeberg_user",
                "full_name": "Codeberg User",
                "email": "user@codeberg.org",
                "avatar_url": "https://codeberg.org/avatar.png"
            }"#,
        );
        let provider = ForgejoProvider::with_custom_api_base(url);

        let user = provider.get_user("pat_secret").unwrap();
        server.join().unwrap();

        assert_eq!(user.login, "codeberg_user");
        assert_eq!(user.name, Some("Codeberg User".to_owned()));
        assert_eq!(user.email, Some("user@codeberg.org".to_owned()));
    }

    #[test]
    fn test_device_auth_unsupported() {
        let provider = ForgejoProvider::default();
        assert!(provider.start_auth().is_err());
    }

    #[test]
    fn test_setup_repository_existing() {
        let (url, server) = serve(&[
            (OK, r#"{"login":"alice","full_name":"Alice"}"#),
            (
                OK,
                r#"{"name":"notes","full_name":"alice/notes","clone_url":"https://codeberg.org/alice/notes.git","private":true,"default_branch":"main"}"#,
            ),
        ]);
        let provider = ForgejoProvider::with_custom_api_base(url);

        let repo = provider.setup_repository("pat123", "notes").unwrap();
        server.join().unwrap();

        assert_eq!(repo.name, "notes");
        assert_eq!(repo.full_name, "alice/notes");
        assert_eq!(repo.clone_url, "https://codeberg.org/alice/notes.git");
        assert!(repo.is_private);
    }
}
