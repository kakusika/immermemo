//! GitHub provider implementation using the Device Authorization Flow (RFC 8628).

use serde::{Deserialize, Serialize};

use crate::api::{Api, Auth, post_oauth};
use crate::remote_url::parse_remote_for_instance;
use crate::{
    AuthPollStatus, AuthSession, CreateGitStyleRepo, GitStyleRepo, Provider, ProviderId,
    RepositoryInfo, UserInfo,
};

/// Default client ID registered for Immermemo desktop and mobile apps.
/// Device authorization flow public clients do not require a client secret.
pub const DEFAULT_GITHUB_CLIENT_ID: &str = "Ov23lik7gNqKqL1k1P2R";

const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_API_BASE: &str = "https://api.github.com";
const GITHUB_INSTANCE: &str = "https://github.com";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";

/// GitHub Git hosting provider.
#[derive(Debug, Clone)]
pub struct GitHubProvider {
    client_id: String,
    device_code_url: String,
    access_token_url: String,
    api: Api,
}

impl Default for GitHubProvider {
    fn default() -> Self {
        Self::new(None)
    }
}

impl GitHubProvider {
    /// Creates a new [`GitHubProvider`] with an optional custom client ID.
    pub fn new(client_id: Option<String>) -> Self {
        Self {
            client_id: client_id.unwrap_or_else(|| DEFAULT_GITHUB_CLIENT_ID.to_owned()),
            device_code_url: GITHUB_DEVICE_CODE_URL.to_owned(),
            access_token_url: GITHUB_ACCESS_TOKEN_URL.to_owned(),
            api: Api::new(GITHUB_API_BASE, Auth::Bearer, GITHUB_ACCEPT),
        }
    }

    /// For tests: overrides endpoints to point at mock HTTP servers.
    #[cfg(test)]
    pub fn with_custom_endpoints(
        client_id: String,
        device_code_url: String,
        access_token_url: String,
        api_base_url: String,
    ) -> Self {
        Self {
            client_id,
            device_code_url,
            access_token_url,
            api: Api::new(api_base_url, Auth::Bearer, GITHUB_ACCEPT),
        }
    }

    /// Extracts repository owner and repository name from a git remote URL if it belongs to GitHub.
    /// Handles HTTPS and SSH formats (e.g. `https://github.com/alice/immermemo-notes.git`).
    pub fn parse_remote(url: &str) -> Option<(String, String)> {
        parse_remote_for_instance(GITHUB_INSTANCE, url, false)
    }
}

#[derive(Serialize)]
struct DeviceCodeRequest<'a> {
    client_id: &'a str,
    scope: &'a str,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Serialize)]
struct TokenPollRequest<'a> {
    client_id: &'a str,
    device_code: &'a str,
    grant_type: &'a str,
}

#[derive(Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct GitHubUser {
    login: String,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

impl Provider for GitHubProvider {
    fn id(&self) -> ProviderId {
        ProviderId::GitHub
    }

    fn instance_url(&self) -> &str {
        GITHUB_INSTANCE
    }

    fn start_auth(&self) -> anyhow::Result<AuthSession> {
        let data: DeviceCodeResponse = post_oauth(
            &self.device_code_url,
            &DeviceCodeRequest {
                client_id: &self.client_id,
                scope: "repo read:user user:email",
            },
        )?;

        Ok(AuthSession {
            device_code: data.device_code,
            user_code: data.user_code,
            verification_uri: data.verification_uri,
            expires_in: data.expires_in,
            interval: data.interval.unwrap_or(5),
        })
    }

    fn poll_auth(&self, session: &AuthSession) -> anyhow::Result<AuthPollStatus> {
        let data: TokenPollResponse = post_oauth(
            &self.access_token_url,
            &TokenPollRequest {
                client_id: &self.client_id,
                device_code: &session.device_code,
                grant_type: "urn:ietf:params:oauth:grant-type:device_code",
            },
        )?;

        if let Some(token) = data.access_token {
            return Ok(AuthPollStatus::Granted {
                access_token: token,
                token_type: data.token_type.unwrap_or_else(|| "bearer".to_owned()),
                scope: data.scope.unwrap_or_default(),
            });
        }

        match data.error.as_deref() {
            Some("authorization_pending") => Ok(AuthPollStatus::Pending),
            Some("slow_down") => Ok(AuthPollStatus::SlowDown),
            Some("expired_token") => Ok(AuthPollStatus::Expired),
            Some("access_denied") => Ok(AuthPollStatus::AccessDenied),
            Some(unknown) => anyhow::bail!("GitHub OAuth error: {unknown}"),
            None => anyhow::bail!("GitHub OAuth returned unexpected empty payload"),
        }
    }

    fn get_user(&self, token: &str) -> anyhow::Result<UserInfo> {
        let user: GitHubUser = self.api.get("/user", token)?;
        Ok(UserInfo {
            login: user.login,
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
        let repo: Option<GitStyleRepo> = self
            .api
            .get_optional(&format!("/repos/{owner}/{repo_name}"), token)?;
        Ok(repo.map(Into::into))
    }

    fn list_repositories(&self, token: &str) -> anyhow::Result<Vec<RepositoryInfo>> {
        let repos: Vec<GitStyleRepo> = self.api.get(
            "/user/repos?sort=updated&per_page=100&affiliation=owner,collaborator",
            token,
        )?;
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
        Self::parse_remote(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{CREATED, OK, serve_once};

    #[test]
    fn test_start_auth() {
        let json = r#"{
            "device_code": "dev123",
            "user_code": "WDAS-5821",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900,
            "interval": 5
        }"#;
        let (url, server) = serve_once(OK, json);
        let provider = GitHubProvider::with_custom_endpoints(
            "test_client".to_owned(),
            url.clone(),
            url.clone(),
            url,
        );

        let session = provider.start_auth().unwrap();
        server.join().unwrap();

        assert_eq!(session.device_code, "dev123");
        assert_eq!(session.user_code, "WDAS-5821");
        assert_eq!(session.verification_uri, "https://github.com/login/device");
        assert_eq!(session.expires_in, 900);
        assert_eq!(session.interval, 5);
    }

    #[test]
    fn test_poll_auth_pending() {
        let json = r#"{"error": "authorization_pending"}"#;
        let (url, server) = serve_once(OK, json);
        let provider = GitHubProvider::with_custom_endpoints(
            "test_client".to_owned(),
            url.clone(),
            url.clone(),
            url,
        );
        let session = AuthSession {
            device_code: "dev123".to_owned(),
            user_code: "WDAS-5821".to_owned(),
            verification_uri: "https://github.com/login/device".to_owned(),
            expires_in: 900,
            interval: 5,
        };

        let status = provider.poll_auth(&session).unwrap();
        server.join().unwrap();

        assert_eq!(status, AuthPollStatus::Pending);
    }

    #[test]
    fn test_poll_auth_granted() {
        let json = r#"{
            "access_token": "ghu_token123",
            "token_type": "bearer",
            "scope": "repo,read:user"
        }"#;
        let (url, server) = serve_once(OK, json);
        let provider = GitHubProvider::with_custom_endpoints(
            "test_client".to_owned(),
            url.clone(),
            url.clone(),
            url,
        );
        let session = AuthSession {
            device_code: "dev123".to_owned(),
            user_code: "WDAS-5821".to_owned(),
            verification_uri: "https://github.com/login/device".to_owned(),
            expires_in: 900,
            interval: 5,
        };

        let status = provider.poll_auth(&session).unwrap();
        server.join().unwrap();

        match status {
            AuthPollStatus::Granted {
                access_token,
                token_type,
                scope,
            } => {
                assert_eq!(access_token, "ghu_token123");
                assert_eq!(token_type, "bearer");
                assert_eq!(scope, "repo,read:user");
            }
            _ => panic!("expected Granted status"),
        }
    }

    #[test]
    fn test_get_user() {
        let json = r#"{
            "login": "octocat",
            "name": "Mona Lisa Octocat",
            "email": "octocat@github.com",
            "avatar_url": "https://avatars.githubusercontent.com/u/583231"
        }"#;
        let (url, server) = serve_once(OK, json);
        let provider = GitHubProvider::with_custom_endpoints(
            "test_client".to_owned(),
            url.clone(),
            url.clone(),
            url,
        );

        let user = provider.get_user("ghu_token123").unwrap();
        server.join().unwrap();

        assert_eq!(user.login, "octocat");
        assert_eq!(user.name, Some("Mona Lisa Octocat".to_owned()));
        assert_eq!(user.email, Some("octocat@github.com".to_owned()));
    }

    #[test]
    fn test_parse_remote() {
        assert_eq!(
            GitHubProvider::parse_remote("https://github.com/octocat/immermemo-notes.git"),
            Some(("octocat".to_owned(), "immermemo-notes".to_owned()))
        );
        assert_eq!(
            GitHubProvider::parse_remote("https://github.com/octocat/immermemo-notes"),
            Some(("octocat".to_owned(), "immermemo-notes".to_owned()))
        );
        assert_eq!(
            GitHubProvider::parse_remote("git@github.com:octocat/immermemo-notes.git"),
            Some(("octocat".to_owned(), "immermemo-notes".to_owned()))
        );
        assert_eq!(
            GitHubProvider::parse_remote("https://gitlab.com/octocat/immermemo-notes.git"),
            None
        );
        assert_eq!(GitHubProvider::parse_remote(""), None);
    }

    #[test]
    fn test_list_repositories() {
        let json = r#"[
            {
                "name": "notes-personal",
                "full_name": "octocat/notes-personal",
                "clone_url": "https://github.com/octocat/notes-personal.git",
                "private": true,
                "default_branch": "main"
            },
            {
                "name": "work-vault",
                "full_name": "octocat/work-vault",
                "clone_url": "https://github.com/octocat/work-vault.git",
                "private": false,
                "default_branch": "master"
            }
        ]"#;
        let (url, server) = serve_once(OK, json);
        let provider = GitHubProvider::with_custom_endpoints(
            "test_client".to_owned(),
            url.clone(),
            url.clone(),
            url,
        );

        let repos = provider.list_repositories("ghu_token123").unwrap();
        server.join().unwrap();

        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "notes-personal");
        assert!(repos[0].is_private);
        assert_eq!(repos[1].name, "work-vault");
        assert!(!repos[1].is_private);
    }

    #[test]
    fn test_create_repository() {
        let json = r#"{
            "name": "my-new-vault",
            "full_name": "octocat/my-new-vault",
            "clone_url": "https://github.com/octocat/my-new-vault.git",
            "private": true,
            "default_branch": "main"
        }"#;
        let (url, server) = serve_once(CREATED, json);
        let provider = GitHubProvider::with_custom_endpoints(
            "test_client".to_owned(),
            url.clone(),
            url.clone(),
            url,
        );

        let repo = provider
            .create_repository("ghu_token123", "my-new-vault", true)
            .unwrap();
        server.join().unwrap();

        assert_eq!(repo.name, "my-new-vault");
        assert_eq!(repo.full_name, "octocat/my-new-vault");
        assert!(repo.is_private);
    }
}
