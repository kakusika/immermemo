//! GitLab Git hosting provider implementation.
//!
//! Supports both GitLab.com (`https://gitlab.com`) and self-hosted GitLab instances
//! using Personal Access Token (PAT) authentication and `/api/v4` REST endpoints.

use serde::{Deserialize, Serialize};

use crate::api::{Api, Auth};
use crate::remote_url::{normalize_instance_url, parse_any_remote, parse_remote_for_instance};
use crate::{AuthPollStatus, AuthSession, Provider, ProviderId, RepositoryInfo, UserInfo};

/// Default public GitLab instance.
pub const DEFAULT_GITLAB_INSTANCE: &str = "https://gitlab.com";

/// GitLab Git hosting provider.
#[derive(Debug, Clone)]
pub struct GitLabProvider {
    instance_url: String,
    api: Api,
}

impl Default for GitLabProvider {
    fn default() -> Self {
        Self::new(None)
    }
}

impl GitLabProvider {
    /// Creates a new [`GitLabProvider`] pointing to the specified instance URL
    /// (e.g. `https://gitlab.com` or `https://gitlab.example.com`).
    /// If `None` is provided, defaults to [`DEFAULT_GITLAB_INSTANCE`].
    pub fn new(instance_url: Option<&str>) -> Self {
        let base = normalize_instance_url(
            instance_url.unwrap_or(DEFAULT_GITLAB_INSTANCE),
            DEFAULT_GITLAB_INSTANCE,
        );
        let api = Api::new(
            format!("{base}/api/v4"),
            Auth::PrivateToken,
            "application/json",
        );
        Self {
            instance_url: base,
            api,
        }
    }

    /// For tests: creates a provider directly pointing to a custom mock API base URL.
    #[cfg(test)]
    pub fn with_custom_api_base(api_base_url: String) -> Self {
        Self {
            api: Api::new(api_base_url.clone(), Auth::PrivateToken, "application/json"),
            instance_url: api_base_url,
        }
    }

    /// Returns the normalized instance base URL (e.g. `https://gitlab.com`).
    pub fn instance_url(&self) -> &str {
        &self.instance_url
    }

    /// Checks if a remote URL looks like it points to a GitLab instance
    /// (either `gitlab.com` or a hostname containing `gitlab`).
    pub fn is_gitlab_remote(url: &str) -> bool {
        Self::parse_any_remote(url).is_some_and(|(host, _, _)| is_gitlab_host(&host))
    }

    /// Extracts repository namespace (user or group) and repository name from a git remote URL
    /// given an instance base URL.
    /// Handles HTTPS and SSH formats, including subgroups
    /// (e.g. `https://gitlab.com/alice/notes.git` or `git@gitlab.com:team/subgroup/notes.git`).
    pub fn parse_remote(instance_base: &str, url: &str) -> Option<(String, String)> {
        let base = normalize_instance_url(instance_base, DEFAULT_GITLAB_INSTANCE);
        parse_remote_for_instance(&base, url, true)
    }

    /// Extracts instance host, namespace, and repository name from a git remote URL
    /// across HTTPS and SSH formats (e.g. `https://gitlab.com/alice/notes.git` or `git@gitlab.com:team/subgroup/notes.git`).
    pub fn parse_any_remote(url: &str) -> Option<(String, String, String)> {
        parse_any_remote(url, true)
    }
}

/// Whether `host` (port allowed) is gitlab.com or looks like a self-hosted GitLab.
pub(crate) fn is_gitlab_host(host: &str) -> bool {
    host.contains("gitlab")
}

#[derive(Deserialize)]
struct GitLabUser {
    username: String,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Serialize)]
struct CreateProject<'a> {
    name: &'a str,
    visibility: &'a str,
    initialize_with_readme: bool,
}

#[derive(Deserialize)]
struct Project {
    name: String,
    path_with_namespace: String,
    http_url_to_repo: String,
    visibility: String,
    default_branch: Option<String>,
}

impl From<Project> for RepositoryInfo {
    fn from(project: Project) -> Self {
        Self {
            name: project.name,
            full_name: project.path_with_namespace,
            clone_url: project.http_url_to_repo,
            is_private: project.visibility != "public",
            default_branch: project.default_branch.unwrap_or_else(|| "main".to_owned()),
        }
    }
}

impl Provider for GitLabProvider {
    fn id(&self) -> ProviderId {
        ProviderId::GitLab
    }

    fn instance_url(&self) -> &str {
        &self.instance_url
    }

    fn start_auth(&self) -> anyhow::Result<AuthSession> {
        anyhow::bail!(
            "Device authorization flow is not supported by GitLab. Please authenticate with a Personal Access Token."
        )
    }

    fn poll_auth(&self, _session: &AuthSession) -> anyhow::Result<AuthPollStatus> {
        anyhow::bail!("Device authorization flow is not supported by GitLab.")
    }

    fn get_user(&self, token: &str) -> anyhow::Result<UserInfo> {
        let user: GitLabUser = self.api.get("/user", token)?;
        Ok(UserInfo {
            login: user.username,
            name: user.name,
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
        // GitLab addresses a project by its URL-encoded `namespace/name`.
        let project: Option<Project> = self
            .api
            .get_optional(&format!("/projects/{owner}%2F{repo_name}"), token)?;
        Ok(project.map(Into::into))
    }

    fn list_repositories(&self, token: &str) -> anyhow::Result<Vec<RepositoryInfo>> {
        let projects: Vec<Project> = self.api.get(
            "/projects?membership=true&order_by=updated_at&per_page=100",
            token,
        )?;
        Ok(projects.into_iter().map(Into::into).collect())
    }

    fn create_repository(
        &self,
        token: &str,
        repo_name: &str,
        is_private: bool,
    ) -> anyhow::Result<RepositoryInfo> {
        let body = CreateProject {
            name: repo_name,
            visibility: if is_private { "private" } else { "public" },
            initialize_with_readme: false,
        };
        let project: Project = self.api.post("/projects", token, &body)?;
        Ok(project.into())
    }

    fn parse_remote_url(&self, url: &str) -> Option<(String, String)> {
        Self::parse_remote(&self.instance_url, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CREATED, NOT_FOUND, OK, serve, serve_once};

    const PROJECT: &str = r#"{"name":"notes","path_with_namespace":"alice/notes","http_url_to_repo":"https://gitlab.com/alice/notes.git","visibility":"private","default_branch":"main"}"#;

    #[test]
    fn an_empty_instance_falls_back_to_gitlab_com() {
        assert_eq!(
            GitLabProvider::new(Some("")).instance_url(),
            DEFAULT_GITLAB_INSTANCE
        );
        assert_eq!(
            GitLabProvider::new(Some("gitlab.local:8080/")).instance_url(),
            "https://gitlab.local:8080"
        );
    }

    #[test]
    fn test_parse_remote() {
        assert_eq!(
            GitLabProvider::parse_remote(
                "https://gitlab.com",
                "https://gitlab.com/alice/immermemo-notes.git"
            ),
            Some(("alice".to_owned(), "immermemo-notes".to_owned()))
        );
        assert_eq!(
            GitLabProvider::parse_remote(
                "https://gitlab.com",
                "git@gitlab.com:alice/immermemo-notes.git"
            ),
            Some(("alice".to_owned(), "immermemo-notes".to_owned()))
        );
        assert_eq!(
            GitLabProvider::parse_remote(
                "https://gitlab.com",
                "git@gitlab.com:team/subgroup/notes.git"
            ),
            Some(("team/subgroup".to_owned(), "notes".to_owned()))
        );
        assert_eq!(
            GitLabProvider::parse_remote(
                "https://gitlab.com",
                "ssh://git@gitlab.com/team/subgroup/notes.git"
            ),
            Some(("team/subgroup".to_owned(), "notes".to_owned()))
        );
        assert_eq!(
            GitLabProvider::parse_remote(
                "https://gitlab.example.com:8443",
                "https://gitlab.example.com:8443/bob/notes"
            ),
            Some(("bob".to_owned(), "notes".to_owned()))
        );
        assert_eq!(
            GitLabProvider::parse_remote(
                "https://gitlab.com",
                "https://github.com/alice/notes.git"
            ),
            None
        );
    }

    #[test]
    fn test_parse_any_remote() {
        assert_eq!(
            GitLabProvider::parse_any_remote("https://gitlab.com/alice/notes.git"),
            Some((
                "gitlab.com".to_owned(),
                "alice".to_owned(),
                "notes".to_owned()
            ))
        );
        assert_eq!(
            GitLabProvider::parse_any_remote("git@gitlab.com:team/subgroup/notes.git"),
            Some((
                "gitlab.com".to_owned(),
                "team/subgroup".to_owned(),
                "notes".to_owned()
            ))
        );
        assert_eq!(
            GitLabProvider::parse_any_remote("https://gitlab.example.com:8443/bob/notes"),
            Some((
                "gitlab.example.com:8443".to_owned(),
                "bob".to_owned(),
                "notes".to_owned()
            ))
        );
        assert_eq!(GitLabProvider::parse_any_remote("invalid-url"), None);
    }

    #[test]
    fn test_is_gitlab_remote() {
        assert!(GitLabProvider::is_gitlab_remote(
            "https://gitlab.com/alice/notes.git"
        ));
        assert!(GitLabProvider::is_gitlab_remote(
            "git@gitlab.com:alice/notes.git"
        ));
        assert!(GitLabProvider::is_gitlab_remote(
            "https://gitlab.corp.net/team/notes.git"
        ));
        assert!(!GitLabProvider::is_gitlab_remote(
            "https://github.com/alice/notes.git"
        ));
        assert!(!GitLabProvider::is_gitlab_remote(
            "https://codeberg.org/alice/notes.git"
        ));
    }

    #[test]
    fn test_get_user() {
        let (url, server) = serve_once(
            OK,
            r#"{
                "username": "gitlab_user",
                "name": "GitLab User",
                "email": "user@gitlab.com",
                "avatar_url": "https://gitlab.com/uploads/user.png"
            }"#,
        );
        let provider = GitLabProvider::with_custom_api_base(url);

        let user = provider.get_user("glpat_secret").unwrap();
        server.join().unwrap();

        assert_eq!(user.login, "gitlab_user");
        assert_eq!(user.name, Some("GitLab User".to_owned()));
        assert_eq!(user.email, Some("user@gitlab.com".to_owned()));
    }

    #[test]
    fn test_device_auth_unsupported() {
        let provider = GitLabProvider::default();
        assert!(provider.start_auth().is_err());
    }

    #[test]
    fn test_setup_repository_existing() {
        let (url, server) = serve(&[
            (OK, r#"{"username":"alice","name":"Alice"}"#),
            (OK, PROJECT),
        ]);
        let provider = GitLabProvider::with_custom_api_base(url);

        let repo = provider.setup_repository("glpat123", "notes").unwrap();
        server.join().unwrap();

        assert_eq!(repo.name, "notes");
        assert_eq!(repo.full_name, "alice/notes");
        assert_eq!(repo.clone_url, "https://gitlab.com/alice/notes.git");
        assert!(repo.is_private);
    }

    #[test]
    fn test_setup_repository_create_new() {
        let (url, server) = serve(&[
            (OK, r#"{"username":"alice","name":"Alice"}"#),
            (NOT_FOUND, ""),
            (CREATED, PROJECT),
        ]);
        let provider = GitLabProvider::with_custom_api_base(url);

        let repo = provider.setup_repository("glpat123", "notes").unwrap();
        server.join().unwrap();

        assert_eq!(repo.name, "notes");
        assert_eq!(repo.full_name, "alice/notes");
        assert_eq!(repo.clone_url, "https://gitlab.com/alice/notes.git");
        assert!(repo.is_private);
    }
}
