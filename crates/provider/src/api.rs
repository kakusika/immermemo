//! The small slice of HTTP every provider's REST client needs: the
//! per-host auth header, JSON `GET`/`POST`, and "404 means absent".

use std::sync::OnceLock;

use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

/// The TLS settings every HTTP client in the app uses.
///
/// This workspace compiles ureq with `native-tls` only, so the platform's TLS
/// is the one to ask for: ureq's own defaults are rustls (not compiled in)
/// and Mozilla's roots (also not compiled in), and either one panics on the
/// first `https` request.
pub fn tls_config() -> TlsConfig {
    TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(RootCerts::PlatformVerifier)
        .build()
}

/// The app's HTTP client, sharing one connection pool. Clone-cheap.
pub fn http_agent() -> ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            ureq::Agent::config_builder()
                .tls_config(tls_config())
                .build()
                .into()
        })
        .clone()
}

/// The client for a host's REST API: like [`http_agent`], but a 4xx/5xx is
/// handed back as a response, so the reason the host gives can be read
/// ([`HttpStatus`]) instead of being thrown away with the status code.
fn api_agent() -> ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT
        .get_or_init(|| {
            ureq::Agent::config_builder()
                .tls_config(tls_config())
                .http_status_as_error(false)
                .build()
                .into()
        })
        .clone()
}

/// A host's answer that is not a success: its status, and the reason it
/// gave (`"message"` of its JSON error, or the start of its body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpStatus {
    pub code: u16,
    pub message: String,
}

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "http status {}", self.code)
        } else {
            write!(f, "http status {}: {}", self.code, self.message)
        }
    }
}

impl std::error::Error for HttpStatus {}

/// The reason in an error body: the usual JSON fields, else the first line
/// of text, shortened.
fn error_message(body: &str) -> String {
    let from_json = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| {
            ["message", "error_description", "error"]
                .iter()
                .find_map(|key| json.get(*key).and_then(|v| v.as_str()).map(str::to_owned))
        });
    let text = from_json.unwrap_or_else(|| body.trim().lines().next().unwrap_or("").to_owned());
    text.chars().take(200).collect()
}

/// `response` itself if it succeeded, otherwise the [`HttpStatus`] it is.
fn successful(
    response: ureq::http::Response<ureq::Body>,
) -> anyhow::Result<ureq::http::Response<ureq::Body>> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.into_body().read_to_string().unwrap_or_default();
    Err(HttpStatus {
        code: status.as_u16(),
        message: error_message(&body),
    }
    .into())
}

pub(crate) const USER_AGENT: &str = concat!("immermemo/", env!("CARGO_PKG_VERSION"));

/// How a host expects the access token to be presented.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Auth {
    /// `Authorization: Bearer <token>` (GitHub).
    Bearer,
    /// `Authorization: token <token>` (Forgejo / Gitea).
    Token,
    /// `PRIVATE-TOKEN: <token>` (GitLab).
    PrivateToken,
}

/// A REST API root plus the conventions needed to call it.
#[derive(Debug, Clone)]
pub(crate) struct Api {
    base: String,
    auth: Auth,
    accept: &'static str,
}

impl Api {
    pub(crate) fn new(base: impl Into<String>, auth: Auth, accept: &'static str) -> Self {
        Self {
            base: base.into(),
            auth,
            accept,
        }
    }

    fn headers(&self, token: &str) -> [(&'static str, String); 3] {
        let (name, value) = match self.auth {
            Auth::Bearer => ("Authorization", format!("Bearer {token}")),
            Auth::Token => ("Authorization", format!("token {token}")),
            Auth::PrivateToken => ("PRIVATE-TOKEN", token.to_owned()),
        };
        [
            (name, value),
            ("Accept", self.accept.to_owned()),
            ("User-Agent", USER_AGENT.to_owned()),
        ]
    }

    pub(crate) fn get<T: DeserializeOwned>(&self, path: &str, token: &str) -> anyhow::Result<T> {
        let mut request = api_agent().get(format!("{}{path}", self.base));
        for (name, value) in self.headers(token) {
            request = request.header(name, value);
        }
        Ok(successful(request.call()?)?.into_body().read_json()?)
    }

    /// Like [`get`](Self::get), but a `404 Not Found` is `Ok(None)`.
    pub(crate) fn get_optional<T: DeserializeOwned>(
        &self,
        path: &str,
        token: &str,
    ) -> anyhow::Result<Option<T>> {
        match self.get(path, token) {
            Ok(value) => Ok(Some(value)),
            Err(e) => match e.downcast_ref::<HttpStatus>() {
                Some(HttpStatus { code: 404, .. }) => Ok(None),
                _ => Err(e),
            },
        }
    }

    pub(crate) fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        token: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let mut request = api_agent().post(format!("{}{path}", self.base));
        for (name, value) in self.headers(token) {
            request = request.header(name, value);
        }
        Ok(successful(request.send_json(body)?)?
            .into_body()
            .read_json()?)
    }
}

/// `POST` to an unauthenticated OAuth endpoint (GitHub's device flow), which
/// answers with JSON even for the "not yet" states.
pub(crate) fn post_oauth<B: Serialize, T: DeserializeOwned>(
    url: &str,
    body: &B,
) -> anyhow::Result<T> {
    Ok(api_agent()
        .post(url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .send_json(body)?
        .into_body()
        .read_json()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local socket that accepts a connection and hangs up: an `https`
    /// peer that never speaks TLS.
    fn silent_peer() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let _ = listener.accept();
        });
        format!("https://127.0.0.1:{port}/")
    }

    /// The regression: with ureq's defaults an `https` request *panicked*
    /// ("provider is Rustls but feature is not enabled") as soon as it
    /// connected. Now it reaches the TLS handshake and fails as an error.
    #[test]
    fn an_https_request_fails_as_an_error_not_a_panic() {
        let result = http_agent().get(silent_peer()).call();
        assert!(result.is_err());
    }

    #[test]
    fn a_refusal_carries_the_reason_the_host_gave() {
        let (base, server) = crate::test_support::serve_once(
            "HTTP/1.1 403 Forbidden",
            r#"{"message":"token does not have at least one of required scope(s): [read:user]","url":"x"}"#,
        );
        let api = Api::new(base, Auth::Token, "application/json");
        let error = api.get::<serde_json::Value>("/user", "tok").unwrap_err();
        server.join().unwrap();
        let status = error.downcast_ref::<HttpStatus>().expect("an HttpStatus");
        assert_eq!(status.code, 403);
        assert!(status.message.contains("read:user"), "{status}");
        assert_eq!(
            error.to_string(),
            "http status 403: token does not have at least one of required scope(s): [read:user]"
        );
    }

    #[test]
    fn a_missing_resource_is_none_and_any_other_refusal_is_an_error() {
        let (base, server) = crate::test_support::serve(&[
            (crate::test_support::NOT_FOUND, r#"{"message":"not found"}"#),
            ("HTTP/1.1 500 Internal Server Error", "boom\nsecond line"),
        ]);
        let api = Api::new(base, Auth::Bearer, "application/json");
        assert_eq!(
            api.get_optional::<serde_json::Value>("/a", "t").unwrap(),
            None
        );
        let error = api.get::<serde_json::Value>("/b", "t").unwrap_err();
        server.join().unwrap();
        assert_eq!(error.to_string(), "http status 500: boom");
    }

    #[test]
    fn an_error_body_is_summarized_to_its_reason() {
        assert_eq!(
            error_message(r#"{"message":"Bad credentials"}"#),
            "Bad credentials"
        );
        assert_eq!(
            error_message(r#"{"error":"invalid_grant"}"#),
            "invalid_grant"
        );
        assert_eq!(error_message("plain\ntext"), "plain");
        assert_eq!(error_message(""), "");
        assert_eq!(error_message(&"x".repeat(500)).chars().count(), 200);
    }

    #[test]
    fn the_tls_settings_are_the_platforms() {
        let config = tls_config();
        assert_eq!(config.provider(), TlsProvider::NativeTls);
        assert!(matches!(config.root_certs(), RootCerts::PlatformVerifier));
    }
}
