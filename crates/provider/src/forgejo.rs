//! Forgejo / Gitea Git hosting provider implementation.
//!
//! Supports both Codeberg (`https://codeberg.org`) and self-hosted Forgejo/Gitea instances
//! using Personal Access Token (PAT) authentication and `/api/v1` REST endpoints.

use serde::{Deserialize, Serialize};

use crate::{AuthPollStatus, AuthSession, Provider, RepositoryInfo, UserInfo};

/// Default Forgejo public instance (Codeberg).
pub const DEFAULT_CODEBERG_INSTANCE: &str = "https://codeberg.org";

const USER_AGENT: &str = "immermemo/0.1.0";

/// Forgejo and Gitea Git hosting provider.
#[derive(Debug, Clone)]
pub struct ForgejoProvider {
    instance_url: String,
    api_base_url: String,
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
        let base = normalize_instance_url(instance_url.unwrap_or(DEFAULT_CODEBERG_INSTANCE));
        let api_base = format!("{base}/api/v1");
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

    /// Returns the normalized instance base URL (e.g. `https://codeberg.org`).
    pub fn instance_url(&self) -> &str {
        &self.instance_url
    }

    /// Extracts repository owner and repository name from a git remote URL given an instance base URL.
    /// Handles HTTPS and SSH formats (e.g. `https://codeberg.org/alice/notes.git` or `git@codeberg.org:alice/notes.git`).
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
            } else if let Some(rest) = trimmed_url.strip_prefix(&format!("ssh://git@{host_only}/")) {
                rest
            } else {
                return None;
            }
        };

        let path = path.trim_start_matches('/').trim_end_matches(".git");
        let mut parts = path.split('/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        if parts.next().is_none() && !owner.is_empty() && !repo.is_empty() {
            Some((owner.to_owned(), repo.to_owned()))
        } else {
            None
        }
    }

    /// Extracts instance host, repository owner, and repository name from a git remote URL
    /// across HTTPS and SSH formats (e.g. `https://codeberg.org/alice/notes.git` or `git@codeberg.org:alice/notes.git`).
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
        } else if let Some(rest) = trimmed.strip_prefix("ssh://git@") {
            let mut parts = rest.splitn(2, '/');
            (parts.next()?, parts.next()?)
        } else {
            return None;
        };

        if host.is_empty() {
            return None;
        }

        let path = path.trim_start_matches('/').trim_end_matches(".git");
        let mut parts = path.split('/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        if parts.next().is_none() && !owner.is_empty() && !repo.is_empty() {
            Some((host.to_owned(), owner.to_owned(), repo.to_owned()))
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
        return DEFAULT_CODEBERG_INSTANCE.to_owned();
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        format!("https://{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

#[derive(Deserialize)]
struct ForgejoUserResponse {
    login: Option<String>,
    username: Option<String>,
    full_name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Serialize)]
struct CreateRepoRequest<'a> {
    name: &'a str,
    description: &'a str,
    private: bool,
    auto_init: bool,
}

#[derive(Deserialize)]
struct ForgejoRepoResponse {
    name: String,
    full_name: String,
    clone_url: String,
    private: bool,
    default_branch: Option<String>,
}

impl Provider for ForgejoProvider {
    fn id(&self) -> &'static str {
        "forgejo"
    }

    fn display_name(&self) -> &str {
        "Forgejo / Gitea"
    }

    fn start_auth(&self) -> anyhow::Result<AuthSession> {
        anyhow::bail!(
            "Device authorization flow (RFC 8628) is not supported by Forgejo/Gitea. Please authenticate with a Personal Access Token."
        )
    }

    fn poll_auth(&self, _session: &AuthSession) -> anyhow::Result<AuthPollStatus> {
        anyhow::bail!("Device authorization flow is not supported by Forgejo/Gitea.")
    }

    fn get_user(&self, token: &str) -> anyhow::Result<UserInfo> {
        let url = format!("{}/user", self.api_base_url);
        let response = ureq::get(&url)
            .header("Authorization", &format!("token {token}"))
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT)
            .call()?;

        let user: ForgejoUserResponse = response.into_body().read_json()?;
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

    fn setup_repository(&self, token: &str, repo_name: &str) -> anyhow::Result<RepositoryInfo> {
        let user = self.get_user(token)?;
        let check_url = format!("{}/repos/{}/{}", self.api_base_url, user.login, repo_name);

        match ureq::get(&check_url)
            .header("Authorization", &format!("token {token}"))
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT)
            .call()
        {
            Ok(response) => {
                let repo: ForgejoRepoResponse = response.into_body().read_json()?;
                return Ok(RepositoryInfo {
                    name: repo.name,
                    full_name: repo.full_name,
                    clone_url: repo.clone_url,
                    is_private: repo.private,
                    default_branch: repo.default_branch.unwrap_or_else(|| "main".to_owned()),
                });
            }
            Err(ureq::Error::StatusCode(404)) => {
                // Repository does not exist; proceed to creation.
            }
            Err(e) => return Err(e.into()),
        }

        let create_url = format!("{}/user/repos", self.api_base_url);
        let body = CreateRepoRequest {
            name: repo_name,
            description: "Immermemo notes vault",
            private: true,
            auto_init: false,
        };

        let response = ureq::post(&create_url)
            .header("Authorization", &format!("token {token}"))
            .header("Accept", "application/json")
            .header("User-Agent", USER_AGENT)
            .send_json(&body)?;

        let repo: ForgejoRepoResponse = response.into_body().read_json()?;

        Ok(RepositoryInfo {
            name: repo.name,
            full_name: repo.full_name,
            clone_url: repo.clone_url,
            is_private: repo.private,
            default_branch: repo.default_branch.unwrap_or_else(|| "main".to_owned()),
        })
    }

    fn parse_remote_url(&self, url: &str) -> Option<(String, String)> {
        Self::parse_remote(&self.instance_url, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn mock_single_request(
        response_body: &'static str,
        status_line: &'static str,
    ) -> (String, std::thread::JoinHandle<()>) {
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
        assert_eq!(normalize_instance_url("codeberg.org"), "https://codeberg.org");
        assert_eq!(
            normalize_instance_url("https://codeberg.org/"),
            "https://codeberg.org"
        );
        assert_eq!(
            normalize_instance_url("http://git.local:3000/"),
            "http://git.local:3000"
        );
        assert_eq!(normalize_instance_url(""), DEFAULT_CODEBERG_INSTANCE);
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
    }

    #[test]
    fn test_parse_any_remote() {
        assert_eq!(
            ForgejoProvider::parse_any_remote("https://codeberg.org/alice/notes.git"),
            Some(("codeberg.org".to_owned(), "alice".to_owned(), "notes".to_owned()))
        );
        assert_eq!(
            ForgejoProvider::parse_any_remote("git@codeberg.org:alice/notes.git"),
            Some(("codeberg.org".to_owned(), "alice".to_owned(), "notes".to_owned()))
        );
        assert_eq!(
            ForgejoProvider::parse_any_remote("https://git.example.com:3000/bob/notes"),
            Some(("git.example.com:3000".to_owned(), "bob".to_owned(), "notes".to_owned()))
        );
        assert_eq!(ForgejoProvider::parse_any_remote("invalid-url"), None);
    }

    #[test]
    fn test_get_user() {
        let json = r#"{
            "login": "codeberg_user",
            "full_name": "Codeberg User",
            "email": "user@codeberg.org",
            "avatar_url": "https://codeberg.org/avatar.png"
        }"#;
        let (url, server) = mock_single_request(json, "HTTP/1.1 200 OK");
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
        // Multi-request mock server:
        // Request 1: GET /user
        // Request 2: GET /repos/alice/notes
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            // First request: /user
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = r#"{"login":"alice","full_name":"Alice"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            // Second request: /repos/alice/notes
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = r#"{"name":"notes","full_name":"alice/notes","clone_url":"https://codeberg.org/alice/notes.git","private":true,"default_branch":"main"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let provider = ForgejoProvider::with_custom_api_base(format!("http://127.0.0.1:{port}"));
        let repo = provider.setup_repository("pat123", "notes").unwrap();
        handle.join().unwrap();

        assert_eq!(repo.name, "notes");
        assert_eq!(repo.full_name, "alice/notes");
        assert_eq!(repo.clone_url, "https://codeberg.org/alice/notes.git");
        assert!(repo.is_private);
    }
}
