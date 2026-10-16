//! Git remote URL handling shared by every provider: normalizing a
//! user-typed instance URL, and splitting an HTTPS/SSH remote into
//! `(host, namespace, repository)`.
//!
//! The only difference between hosts is whether a repository may live
//! under nested namespaces (GitLab subgroups: `team/subgroup/notes`) or
//! is always exactly `owner/repo` (GitHub, Forgejo/Gitea) -- the `nested`
//! flag threaded through below.

/// Normalizes a user-provided instance URL: trims whitespace and trailing
/// slashes, defaults to `https://` when no scheme was typed, and falls back
/// to `default` for empty input.
pub(crate) fn normalize_instance_url(input: &str, default: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return default.to_owned();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    }
}

/// Splits a remote URL into `(host, path)`. The host keeps its port for
/// HTTP(S) remotes; SSH forms (`git@host:path`, `ssh://git@host/path`)
/// never carry one in the scp-like variant.
fn split_remote(url: &str) -> Option<(&str, &str)> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://git@"))
    {
        rest.split_once('/')?
    } else {
        url.strip_prefix("git@")?.split_once(':')?
    };
    (!host.is_empty()).then_some((host, path))
}

/// Splits `path` into `(namespace, repo)`. Without `nested`, the path must
/// be exactly two segments.
fn split_namespace(path: &str, nested: bool) -> Option<(&str, &str)> {
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (namespace, repo) = path.rsplit_once('/')?;
    if namespace.is_empty() || repo.is_empty() || (!nested && namespace.contains('/')) {
        return None;
    }
    Some((namespace, repo))
}

/// `(host, namespace, repo)` of any HTTPS or SSH remote, whatever host it is.
pub(crate) fn parse_any_remote(url: &str, nested: bool) -> Option<(String, String, String)> {
    let (host, path) = split_remote(url)?;
    let (namespace, repo) = split_namespace(path, nested)?;
    Some((host.to_owned(), namespace.to_owned(), repo.to_owned()))
}

/// `(namespace, repo)` of `url` if it belongs to the instance at
/// `instance_base` (e.g. `https://codeberg.org`), over HTTP(S) -- also with
/// the opposite scheme to the one `instance_base` was typed with -- or SSH.
pub(crate) fn parse_remote_for_instance(
    instance_base: &str,
    url: &str,
    nested: bool,
) -> Option<(String, String)> {
    let url = url.trim();
    let base = instance_base.trim_end_matches('/');
    let host_only = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .unwrap_or(base);
    let flipped = if let Some(rest) = base.strip_prefix("https://") {
        format!("http://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        base.to_owned()
    };

    let path = url
        .strip_prefix(base)
        .or_else(|| url.strip_prefix(flipped.as_str()))
        // `https://host.evil.com/..` must not match the instance `https://host`.
        .filter(|rest| rest.starts_with('/'))
        .or_else(|| {
            // The scp-like and `ssh://` forms never carry the HTTP port.
            let ssh_host = host_only.split(['/', ':']).next()?;
            url.strip_prefix(&format!("git@{ssh_host}:"))
                .or_else(|| url.strip_prefix(&format!("ssh://git@{ssh_host}/")))
        })?;
    let (namespace, repo) = split_namespace(path, nested)?;
    Some((namespace.to_owned(), repo.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_defaults_scheme_and_falls_back() {
        let default = "https://example.org";
        assert_eq!(
            normalize_instance_url("gitlab.example.com", default),
            "https://gitlab.example.com"
        );
        assert_eq!(
            normalize_instance_url("https://codeberg.org/", default),
            "https://codeberg.org"
        );
        assert_eq!(
            normalize_instance_url(" http://git.local:3000/ ", default),
            "http://git.local:3000"
        );
        assert_eq!(normalize_instance_url("", default), default);
        assert_eq!(normalize_instance_url("  / ", default), default);
    }

    #[test]
    fn exactly_two_segments_unless_nested() {
        assert_eq!(
            parse_any_remote("https://h/a/b/c.git", false),
            None,
            "a flat host must reject a nested path"
        );
        assert_eq!(
            parse_any_remote("https://h/a/b/c.git", true),
            Some(("h".into(), "a/b".into(), "c".into()))
        );
    }

    #[test]
    fn every_remote_shape_is_split() {
        let want = Some(("h.org".to_owned(), "alice".to_owned(), "notes".to_owned()));
        for url in [
            "https://h.org/alice/notes.git",
            "http://h.org/alice/notes",
            "git@h.org:alice/notes.git",
            "ssh://git@h.org/alice/notes.git",
            " https://h.org/alice/notes/ ",
        ] {
            assert_eq!(parse_any_remote(url, false), want, "{url}");
        }
        assert_eq!(parse_any_remote("invalid-url", false), None);
        assert_eq!(parse_any_remote("https://h.org/solo", false), None);
        assert_eq!(parse_any_remote("https:///a/b", false), None);
    }

    #[test]
    fn a_dot_git_suffix_is_removed_once_not_repeatedly() {
        assert_eq!(
            parse_any_remote("https://h/a/x.git.git", false),
            Some(("h".into(), "a".into(), "x.git".into()))
        );
    }

    #[test]
    fn instance_match_accepts_both_schemes_and_ssh_but_not_other_hosts() {
        let base = "https://git.example.com:3000";
        let want = Some(("bob".to_owned(), "notes".to_owned()));
        assert_eq!(
            parse_remote_for_instance(base, "https://git.example.com:3000/bob/notes", false),
            want
        );
        assert_eq!(
            parse_remote_for_instance(base, "http://git.example.com:3000/bob/notes", false),
            want
        );
        assert_eq!(
            parse_remote_for_instance(base, "git@git.example.com:bob/notes.git", false),
            want
        );
        assert_eq!(
            parse_remote_for_instance(base, "ssh://git@git.example.com/bob/notes.git", false),
            want
        );
        assert_eq!(
            parse_remote_for_instance(base, "https://github.com/bob/notes.git", false),
            None
        );
        assert_eq!(
            parse_remote_for_instance("https://gitlab.com", "https://gitlab.com.evil.io/a/b", true),
            None,
            "a longer hostname sharing the instance as a prefix is a different host"
        );
    }
}
