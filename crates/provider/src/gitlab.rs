//! GitLab Git hosting provider implementation.
//!
//! Supports both GitLab.com (`https://gitlab.com`) and self-hosted GitLab instances
//! using Personal Access Token (PAT) authentication and `/api/v4` REST endpoints.

use serde::{Deserialize, Serialize};

use crate::{AuthPollStatus, AuthSession, Provider, RepositoryInfo, UserInfo};

/// Default public GitLab instance.
pub const DEFAULT_GITLAB_INSTANCE: &str = "https://gitlab.com";

const USER_AGENT: &str = "immermemo/0.1.0";

/// GitLab Git hosting provider.
#[derive(Debug, Clone)]
pub struct GitLabProvider {
    instance_url: String,
    api_base_url: String,
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
        let base = normalize_instance_url(instance_url.unwrap_or(DEFAULT_GITLAB_INSTANCE));
        let api_base = format!("{base}/api/v4");
        Self {
            instance_url: base,
            api_base_url: api_base,
        }
    }

    /// For tests: creates a provider directly pointing to a custom mock API base URL.
    #[cfg(test)]
    pub fn with_custom_api_base(api_base_url: String) -> Self {
        Self {
            instance_url: api_base_url.clone(),
            api_base_url,
        }
    }

    /// Returns the normalized instance base URL (e.g. `https://gitlab.com`).
    pub fn instance_url(&self) -> &str {
        &self.instance_url
    }

    /// Checks if a remote URL looks like it points to a GitLab instance
    /// (either `gitlab.com` or a hostname containing `gitlab`).
    pub fn is_gitlab_remote(url: &str) -> bool {
        let trimmed = url.trim();
        if let Some((host, _, _)) = Self::parse_any_remote(trimmed) {
            host == "gitlab.com" || host.contains("gitlab")
        } else {
            false
        }
    }

    /// Extracts repository namespace (user or group) and repository name from a git remote URL
    /// given an instance base URL.
    /// Handles HTTPS and SSH formats, including subgroups
    /// (e.g. `https://gitlab.com/alice/notes.git` or `git@gitlab.com:team/subgroup/notes.git`).
    pub fn parse_remote(instance_base: &str, url: &str) -> Option<(String, String)> {
        let trimmed_url = url.trim();
        let normalized_base = normalize_instance_url(instance_base);
        let trimmed_base = normalized_base.trim_end_matches('/');

        let path = if let Some(rest) = trimmed_url.strip_prefix(trimmed_base) {
            rest
        } else {
            let host_only = trimmed_base
                .strip_prefix("https://")
                .or_else(|| trimmed_base.strip_prefix("http://"))
                .unwrap_or(trimmed_base)
                .split(['/', ':'])
                .next()?;

            if let Some(rest) = trimmed_url.strip_prefix(&format!("git@{host_only}:")) {
                rest
            } else {
                trimmed_url.strip_prefix(&format!("ssh://git@{host_only}/"))?
            }
        };

        let path = path.trim_start_matches('/').trim_end_matches(".git");
        let (namespace, repo) = path.rsplit_once('/')?;
        if !namespace.is_empty() && !repo.is_empty() {
            Some((namespace.to_owned(), repo.to_owned()))
        } else {
            None
        }
    }

    /// Extracts instance host, namespace, and repository name from a git remote URL
    /// across HTTPS and SSH formats (e.g. `https://gitlab.com/alice/notes.git` or `git@gitlab.com:team/subgroup/notes.git`).
    pub fn parse_any_remote(url: &str) -> Option<(String, String, String)> {
        let trimmed = url.trim();
        let (host, path) = if let Some(rest) = trimmed.strip_prefix("https://") {
            let mut parts = rest.splitn(2, '/');
            (parts.next()?, parts.next()?)
        } else if let Some(rest) = trimmed.strip_prefix("http://") {
            let mut parts = rest.splitn(2, '/');
            (parts.next()?, parts.next()?)
        } else if let Some(rest) = trimmed.strip_prefix("git@") {
            let mut parts = rest.splitn(2, ':');
            (parts.next()?, parts.next()?)
        } else {
            let rest = trimmed.strip_prefix("ssh://git@")?;
            let mut parts = rest.splitn(2, '/');
            (parts.next()?, parts.next()?)
        };

        if host.is_empty() {
            return None;
        }

        let path = path.trim_start_matches('/').trim_end_matches(".git");
        let (namespace, repo) = path.rsplit_once('/')?;
        if !namespace.is_empty() && !repo.is_empty() {
            Some((host.to_owned(), namespace.to_owned(), repo.to_owned()))
        } else {
            None
        }
    }
}

/// Normalizes user-provided instance URL by trimming whitespace, removing trailing slashes,
/// and defaulting to `https://` if no scheme was specified.
fn normalize_instance_url(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return DEFAULT_GITLAB_INSTANCE.to_owned();
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        format!("https://{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

#[derive(Deserialize)]
struct GitLabUserResponse {
    username: String,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Serialize)]
struct CreateGitLabProjectRequest<'a> {
    name: &'a str,
    visibility: &'a str,
    initialize_with_readme: bool,
}

#[derive(Deserialize)]
struct GitLabProjectResponse {
    name: String,
    path_with_namespace: String,
    http_url_to_repo: String,
    visibility: String,
    default_branch: Option<String>,
}

impl Provider for GitLabProvider {
    fn id(&self) -> &'static str {
        "gitlab"
    }

    fn display_name(&self) -> &str {
        "GitLab"
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
        let url = format!("{}/user", self.api_base_url);
        let response = ureq::get(&url)
            .header("PRIVATE-TOKEN", token)
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT)
            .call()?;

        let user: GitLabUserResponse = response.into_body().read_json()?;

        Ok(UserInfo {
            login: user.username,
            name: user.name,
            email: user.email,
            avatar_url: user.avatar_url,
        })
    }

    fn setup_repository(&self, token: &str, repo_name: &str) -> anyhow::Result<RepositoryInfo> {
        let user = self.get_user(token)?;
        // In GitLab API v4, projects are retrieved by URL-encoded path `namespace%2Fproject_name`
        let check_url = format!(
            "{}/projects/{}%2F{}",
            self.api_base_url, user.login, repo_name
        );

        match ureq::get(&check_url)
            .header("PRIVATE-TOKEN", token)
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT)
            .call()
        {
            Ok(response) => {
                let repo: GitLabProjectResponse = response.into_body().read_json()?;
                return Ok(RepositoryInfo {
                    name: repo.name,
                    full_name: repo.path_with_namespace,
                    clone_url: repo.http_url_to_repo,
                    is_private: repo.visibility != "public",
                    default_branch: repo.default_branch.unwrap_or_else(|| "main".to_owned()),
                });
            }
            Err(ureq::Error::StatusCode(404)) => {}
            Err(e) => return Err(e.into()),
        }

        self.create_repository(token, repo_name, true)
    }

    fn list_repositories(&self, token: &str) -> anyhow::Result<Vec<RepositoryInfo>> {
        let url = format!(
            "{}/projects?membership=true&order_by=updated_at&per_page=100",
            self.api_base_url
        );
        let response = ureq::get(&url)
            .header("PRIVATE-TOKEN", token)
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT)
            .call()?;

        let repos: Vec<GitLabProjectResponse> = response.into_body().read_json()?;
        Ok(repos
            .into_iter()
            .map(|repo| RepositoryInfo {
                name: repo.name,
                full_name: repo.path_with_namespace,
                clone_url: repo.http_url_to_repo,
                is_private: repo.visibility != "public",
                default_branch: repo.default_branch.unwrap_or_else(|| "main".to_owned()),
            })
            .collect())
    }

    fn create_repository(
        &self,
        token: &str,
        repo_name: &str,
        is_private: bool,
    ) -> anyhow::Result<RepositoryInfo> {
        let create_url = format!("{}/projects", self.api_base_url);
        let body = CreateGitLabProjectRequest {
            name: repo_name,
            visibility: if is_private { "private" } else { "public" },
            initialize_with_readme: false,
        };

        let response = ureq::post(&create_url)
            .header("PRIVATE-TOKEN", token)
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT)
            .send_json(&body)?;

        let repo: GitLabProjectResponse = response.into_body().read_json()?;

        Ok(RepositoryInfo {
            name: repo.name,
            full_name: repo.path_with_namespace,
            clone_url: repo.http_url_to_repo,
            is_private: repo.visibility != "public",
            default_branch: repo.default_branch.unwrap_or_else(|| "main".to_owned()),
        })
    }

    fn parse_remote_url(&self, url: &str) -> Option<(String, String)> {
        Self::parse_remote(&self.instance_url, url)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    use super::*;

    fn mock_single_request(
        response_body: &'static str,
        status_line: &'static str,
    ) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    #[test]
    fn test_normalize_instance_url() {
        assert_eq!(
            normalize_instance_url("https://gitlab.com/"),
            "https://gitlab.com"
        );
        assert_eq!(
            normalize_instance_url("gitlab.example.com"),
            "https://gitlab.example.com"
        );
        assert_eq!(
            normalize_instance_url("http://gitlab.local:8080/"),
            "http://gitlab.local:8080"
        );
        assert_eq!(normalize_instance_url(""), DEFAULT_GITLAB_INSTANCE);
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
        let json = r#"{
            "username": "gitlab_user",
            "name": "GitLab User",
            "email": "user@gitlab.com",
            "avatar_url": "https://gitlab.com/uploads/user.png"
        }"#;
        let (url, server) = mock_single_request(json, "HTTP/1.1 200 OK");
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            // First request: /user
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = r#"{"username":"alice","name":"Alice"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            // Second request: /projects/alice%2Fnotes
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = r#"{"name":"notes","path_with_namespace":"alice/notes","http_url_to_repo":"https://gitlab.com/alice/notes.git","visibility":"private","default_branch":"main"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let provider = GitLabProvider::with_custom_api_base(format!("http://127.0.0.1:{port}"));
        let repo = provider.setup_repository("glpat123", "notes").unwrap();
        handle.join().unwrap();

        assert_eq!(repo.name, "notes");
        assert_eq!(repo.full_name, "alice/notes");
        assert_eq!(repo.clone_url, "https://gitlab.com/alice/notes.git");
        assert!(repo.is_private);
    }

    #[test]
    fn test_setup_repository_create_new() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            // First request: /user
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = r#"{"username":"alice","name":"Alice"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            // Second request: /projects/alice%2Fnotes -> 404
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let resp =
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
            }
            // Third request: POST /projects -> 201 Created
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = r#"{"name":"notes","path_with_namespace":"alice/notes","http_url_to_repo":"https://gitlab.com/alice/notes.git","visibility":"private","default_branch":"main"}"#;
                let resp = format!(
                    "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let provider = GitLabProvider::with_custom_api_base(format!("http://127.0.0.1:{port}"));
        let repo = provider.setup_repository("glpat123", "notes").unwrap();
        handle.join().unwrap();

        assert_eq!(repo.name, "notes");
        assert_eq!(repo.full_name, "alice/notes");
        assert_eq!(repo.clone_url, "https://gitlab.com/alice/notes.git");
        assert!(repo.is_private);
    }
}
