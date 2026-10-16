//! Fetching a Git LFS object by the pointer that stands in for it.
//!
//! The protocol is two requests (see git-lfs's `docs/api/batch.md`): a JSON
//! POST to `<lfs endpoint>/objects/batch` asking where `oid` can be
//! downloaded, then a GET of the `href` the server answers with, carrying the
//! headers it lists. What comes back is checked against the pointer's SHA-256
//! and size before anyone uses it, so a wrong or truncated body never becomes
//! an image.

use std::sync::Arc;

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

use crate::source::{Fetcher, HttpRequest, LfsPointer, Method};

/// The most an LFS object may weigh to be fetched, in bytes. Checked against
/// the pointer's own size, so an oversized object is refused before a single
/// request is made.
pub const MAX_LFS_BYTES: u64 = 32 * 1024 * 1024;

/// The most the batch response may weigh, in bytes.
const MAX_BATCH_RESPONSE_BYTES: usize = 1024 * 1024;

const LFS_JSON: &str = "application/vnd.git-lfs+json";

/// Where credentials for an LFS server come from. Asked when a download is
/// actually about to be made, on whatever thread makes it, so the caller can
/// read a secret store lazily.
pub trait LfsAuth: Send + Sync {
    /// `(username, password)` for HTTP basic auth, if there is a token.
    fn basic_auth(&self) -> Option<(String, String)>;
}

/// A vault's LFS server: where it is, and how to authenticate to it.
#[derive(Clone)]
pub struct LfsRemote {
    endpoint: String,
    auth: Arc<dyn LfsAuth>,
}

impl std::fmt::Debug for LfsRemote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LfsRemote")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl LfsRemote {
    /// The LFS server of the Git remote `remote_url`, or `None` if that URL
    /// is not one an HTTPS LFS endpoint can be derived from.
    pub fn new(remote_url: &str, auth: Arc<dyn LfsAuth>) -> Option<Self> {
        Some(Self {
            endpoint: lfs_endpoint(remote_url)?,
            auth,
        })
    }

    /// The LFS API root, e.g. `https://github.com/owner/repo.git/info/lfs`.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// git-lfs's rule for finding an LFS server from a Git remote: the remote's
/// HTTPS URL with `.git` ensured, then `/info/lfs`. An SSH remote
/// (`git@host:owner/repo.git`, `ssh://git@host[:port]/owner/repo.git`) maps
/// to the HTTPS URL of the same host and path. Plain `http://` is refused.
fn lfs_endpoint(remote_url: &str) -> Option<String> {
    let remote = remote_url.trim().trim_end_matches('/');
    let (host, path) = if let Some(rest) = remote.strip_prefix("https://") {
        rest.split_once('/')?
    } else if let Some(rest) = remote.strip_prefix("ssh://") {
        let (host, path) = rest.split_once('/')?;
        let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
        (host.split(':').next()?, path)
    } else if !remote.contains("://") {
        let (host, path) = remote.split_once(':')?;
        (host.rsplit_once('@').map_or(host, |(_, host)| host), path)
    } else {
        return None;
    };
    let path = path.trim_matches('/');
    if host.is_empty() || path.is_empty() {
        return None;
    }
    let path = path.strip_suffix(".git").unwrap_or(path);
    Some(format!("https://{host}/{path}.git/info/lfs"))
}

/// The host part of an `https://` URL.
fn https_host(url: &str) -> Option<&str> {
    url.strip_prefix("https://")?.split('/').next()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Downloads the object `pointer` names from `remote`, verified.
///
/// Blocking: call it off the UI thread.
pub fn download(
    remote: &LfsRemote,
    fetcher: &dyn Fetcher,
    pointer: &LfsPointer,
) -> anyhow::Result<Vec<u8>> {
    if pointer.size > MAX_LFS_BYTES {
        bail!(
            "the LFS object is {} bytes, over the {MAX_LFS_BYTES} byte limit",
            pointer.size
        );
    }
    let auth = remote.auth.basic_auth();

    let batch = serde_json::json!({
        "operation": "download",
        "transfers": ["basic"],
        "objects": [{ "oid": pointer.oid, "size": pointer.size }],
    });
    let response = fetcher
        .request(&HttpRequest {
            method: Method::Post,
            url: format!("{}/objects/batch", remote.endpoint),
            headers: vec![
                ("Accept".to_owned(), LFS_JSON.to_owned()),
                ("Content-Type".to_owned(), LFS_JSON.to_owned()),
            ],
            basic_auth: auth.clone(),
            body: batch.to_string().into_bytes(),
            max_bytes: MAX_BATCH_RESPONSE_BYTES,
        })
        .context("asking the LFS server where to download the object")?;
    let response: serde_json::Value =
        serde_json::from_slice(&response).context("the LFS server's answer is not JSON")?;
    let object = &response["objects"][0];
    if let Some(error) = object.get("error") {
        bail!(
            "the LFS server refused the object: {} ({})",
            error["message"].as_str().unwrap_or("no message"),
            error["code"]
        );
    }
    let download = &object["actions"]["download"];
    let href = download["href"]
        .as_str()
        .context("the LFS server offered no download for the object")?;
    // Never follow a downgrade to plain HTTP: the object is fetched with
    // credentials.
    let Some(href_host) = https_host(href) else {
        bail!("the LFS server's download address is not https");
    };
    let headers: Vec<(String, String)> = download["header"]
        .as_object()
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    // Our own credentials go only where they were given: to the LFS host,
    // and only when the server did not hand over an `Authorization` of its
    // own (as a pre-signed storage URL does).
    let own_auth = headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
    let same_host = https_host(&remote.endpoint) == Some(href_host);

    let bytes = fetcher
        .request(&HttpRequest {
            method: Method::Get,
            url: href.to_owned(),
            headers,
            basic_auth: if own_auth || !same_host { None } else { auth },
            body: Vec::new(),
            max_bytes: pointer.size as usize,
        })
        .context("downloading the LFS object")?;

    if bytes.len() as u64 != pointer.size {
        bail!(
            "the LFS object came back {} bytes, expected {}",
            bytes.len(),
            pointer.size
        );
    }
    let digest = hex(&Sha256::digest(&bytes));
    if digest != pointer.oid {
        bail!("the LFS object does not match its pointer (sha256 {digest})");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FixedAuth(Option<(&'static str, &'static str)>);

    impl LfsAuth for FixedAuth {
        fn basic_auth(&self) -> Option<(String, String)> {
            self.0.map(|(u, p)| (u.to_owned(), p.to_owned()))
        }
    }

    fn remote(auth: Option<(&'static str, &'static str)>) -> LfsRemote {
        LfsRemote::new("git@github.com:owner/notes.git", Arc::new(FixedAuth(auth))).unwrap()
    }

    fn pointer_for(content: &[u8]) -> LfsPointer {
        LfsPointer {
            oid: hex(&Sha256::digest(content)),
            size: content.len() as u64,
        }
    }

    /// Answers the batch POST and then the GET, recording every request.
    struct Server {
        batch: String,
        object: Vec<u8>,
        seen: Mutex<Vec<HttpRequest>>,
    }

    impl Server {
        fn serving(object: &[u8], batch: impl FnOnce(&str) -> serde_json::Value) -> Self {
            let pointer = pointer_for(object);
            Self {
                batch: batch(&pointer.oid).to_string(),
                object: object.to_vec(),
                seen: Mutex::default(),
            }
        }

        fn requests(&self) -> Vec<HttpRequest> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Fetcher for Server {
        fn fetch(&self, _: &str, _: usize) -> anyhow::Result<Vec<u8>> {
            unreachable!("LFS uses request()")
        }

        fn request(&self, request: &HttpRequest) -> anyhow::Result<Vec<u8>> {
            self.seen.lock().unwrap().push(request.clone());
            Ok(match request.method {
                Method::Post => self.batch.clone().into_bytes(),
                Method::Get => self.object.clone(),
            })
        }
    }

    fn offering(
        href: &'static str,
        header: serde_json::Value,
    ) -> impl FnOnce(&str) -> serde_json::Value {
        move |oid| {
            serde_json::json!({
                "objects": [{
                    "oid": oid,
                    "actions": { "download": { "href": href, "header": header } }
                }]
            })
        }
    }

    #[test]
    fn the_endpoint_follows_git_lfs_for_every_kind_of_remote() {
        let expected = Some("https://github.com/owner/notes.git/info/lfs".to_owned());
        for url in [
            "https://github.com/owner/notes.git",
            "https://github.com/owner/notes",
            "https://github.com/owner/notes/",
            "git@github.com:owner/notes.git",
            "ssh://git@github.com/owner/notes.git",
            "ssh://git@github.com:22/owner/notes.git",
        ] {
            assert_eq!(lfs_endpoint(url), expected, "{url}");
        }
        assert_eq!(
            lfs_endpoint("https://git.example.com/team/sub/notes.git").as_deref(),
            Some("https://git.example.com/team/sub/notes.git/info/lfs")
        );
        for refused in [
            "http://github.com/o/r.git",
            "file:///srv/r.git",
            "",
            "https://host",
        ] {
            assert_eq!(lfs_endpoint(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn an_object_is_downloaded_through_the_batch_api_and_verified() {
        let object = b"the real image bytes".as_slice();
        let server = Server::serving(
            object,
            offering(
                "https://github-cloud.example.com/objects/abc",
                serde_json::json!({ "Authorization": "RemoteAuth zzz" }),
            ),
        );
        let bytes = download(&remote(Some(("tok", ""))), &server, &pointer_for(object)).unwrap();
        assert_eq!(bytes, object);

        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        let batch = &requests[0];
        assert_eq!(batch.method, Method::Post);
        assert_eq!(
            batch.url,
            "https://github.com/owner/notes.git/info/lfs/objects/batch"
        );
        assert_eq!(batch.basic_auth, Some(("tok".to_owned(), String::new())));
        assert!(
            batch
                .headers
                .contains(&("Accept".to_owned(), LFS_JSON.to_owned()))
        );
        let body: serde_json::Value = serde_json::from_slice(&batch.body).unwrap();
        assert_eq!(body["operation"], "download");
        assert_eq!(body["objects"][0]["oid"], pointer_for(object).oid);
        assert_eq!(body["objects"][0]["size"], object.len());

        let get = &requests[1];
        assert_eq!(get.method, Method::Get);
        assert_eq!(get.url, "https://github-cloud.example.com/objects/abc");
        assert_eq!(
            get.headers,
            vec![("Authorization".to_owned(), "RemoteAuth zzz".to_owned())]
        );
        assert_eq!(
            get.basic_auth, None,
            "the server's own Authorization is used instead"
        );
        assert_eq!(get.max_bytes, object.len());
    }

    #[test]
    fn our_credentials_go_only_to_the_lfs_host() {
        let object = b"x".as_slice();
        let same = Server::serving(
            object,
            offering("https://github.com/lfs/obj", serde_json::json!({})),
        );
        download(&remote(Some(("tok", ""))), &same, &pointer_for(object)).unwrap();
        assert_eq!(
            same.requests()[1].basic_auth,
            Some(("tok".to_owned(), String::new()))
        );

        let other = Server::serving(
            object,
            offering("https://cdn.example.com/obj", serde_json::json!({})),
        );
        download(&remote(Some(("tok", ""))), &other, &pointer_for(object)).unwrap();
        assert_eq!(other.requests()[1].basic_auth, None);
    }

    #[test]
    fn a_download_that_is_not_https_is_refused() {
        let object = b"x".as_slice();
        let server = Server::serving(
            object,
            offering("http://github.com/lfs/obj", serde_json::json!({})),
        );
        let error = download(&remote(None), &server, &pointer_for(object)).unwrap_err();
        assert!(error.to_string().contains("not https"), "{error}");
        assert_eq!(server.requests().len(), 1, "no GET was made");
    }

    #[test]
    fn bytes_that_do_not_match_the_pointer_are_refused() {
        let wanted = pointer_for(b"the real image bytes");
        // Same length, different content.
        let server = Server::serving(
            b"the fake image bytes",
            offering("https://github.com/lfs/obj", serde_json::json!({})),
        );
        let error = download(&remote(None), &server, &wanted).unwrap_err();
        assert!(error.to_string().contains("does not match"), "{error}");

        // Different length.
        let short = Server::serving(
            b"short",
            offering("https://github.com/lfs/obj", serde_json::json!({})),
        );
        let error = download(&remote(None), &short, &wanted).unwrap_err();
        assert!(error.to_string().contains("expected"), "{error}");
    }

    #[test]
    fn an_error_from_the_server_is_reported_with_its_message() {
        let object = b"x".as_slice();
        let server = Server::serving(object, |oid| {
            serde_json::json!({
                "objects": [{ "oid": oid, "error": { "code": 404, "message": "Object does not exist" } }]
            })
        });
        let error = download(&remote(None), &server, &pointer_for(object)).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("Object does not exist") && message.contains("404"),
            "{message}"
        );

        let none = Server::serving(
            object,
            |oid| serde_json::json!({ "objects": [{ "oid": oid }] }),
        );
        assert!(download(&remote(None), &none, &pointer_for(object)).is_err());
    }

    #[test]
    fn an_object_over_the_limit_is_refused_before_any_request() {
        let server = Server::serving(
            b"x",
            offering("https://github.com/lfs/obj", serde_json::json!({})),
        );
        let huge = LfsPointer {
            oid: "0".repeat(64),
            size: MAX_LFS_BYTES + 1,
        };
        assert!(download(&remote(None), &server, &huge).is_err());
        assert!(server.requests().is_empty());
    }

    #[test]
    fn a_client_that_cannot_make_lfs_requests_fails_cleanly() {
        struct PlainGet;
        impl Fetcher for PlainGet {
            fn fetch(&self, _: &str, _: usize) -> anyhow::Result<Vec<u8>> {
                unreachable!()
            }
        }
        let error = download(&remote(None), &PlainGet, &pointer_for(b"x")).unwrap_err();
        assert!(
            format!("{error:#}").contains("cannot make Git LFS requests"),
            "{error:#}"
        );
    }
}
