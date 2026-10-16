//! The small slice of HTTP every provider's REST client needs: the
//! per-host auth header, JSON `GET`/`POST`, and "404 means absent".

use serde::Serialize;
use serde::de::DeserializeOwned;

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
        let mut request = ureq::get(format!("{}{path}", self.base));
        for (name, value) in self.headers(token) {
            request = request.header(name, value);
        }
        Ok(request.call()?.into_body().read_json()?)
    }

    /// Like [`get`](Self::get), but a `404 Not Found` is `Ok(None)`.
    pub(crate) fn get_optional<T: DeserializeOwned>(
        &self,
        path: &str,
        token: &str,
    ) -> anyhow::Result<Option<T>> {
        match self.get(path, token) {
            Ok(value) => Ok(Some(value)),
            Err(e) => match e.downcast_ref::<ureq::Error>() {
                Some(ureq::Error::StatusCode(404)) => Ok(None),
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
        let mut request = ureq::post(format!("{}{path}", self.base));
        for (name, value) in self.headers(token) {
            request = request.header(name, value);
        }
        Ok(request.send_json(body)?.into_body().read_json()?)
    }
}

/// `POST` to an unauthenticated OAuth endpoint (GitHub's device flow), which
/// answers with JSON even for the "not yet" states.
pub(crate) fn post_oauth<B: Serialize, T: DeserializeOwned>(
    url: &str,
    body: &B,
) -> anyhow::Result<T> {
    Ok(ureq::post(url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .send_json(body)?
        .into_body()
        .read_json()?)
}
