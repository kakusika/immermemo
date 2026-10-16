//! Where the bytes behind a note's image reference come from.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, bail};

/// The most a remote image may weigh, in bytes.
pub const MAX_REMOTE_BYTES: usize = 8 * 1024 * 1024;

/// The most a file inside the vault may weigh to be read as an image, in bytes.
pub const MAX_LOCAL_BYTES: u64 = 32 * 1024 * 1024;

/// An HTTP client, supplied by the app.
pub trait Fetcher: Send + Sync {
    /// GETs `url` and returns its body, failing if the body is larger than
    /// `max_bytes` (so a hostile server cannot make the app buffer more).
    fn fetch(&self, url: &str, max_bytes: usize) -> anyhow::Result<Vec<u8>>;
}

/// What a note's image reference points at.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MediaSource {
    /// An `https://` URL. Plain `http://` is not accepted.
    Remote(String),
    /// A file inside the vault, as a canonical absolute path.
    Vault(PathBuf),
}

impl MediaSource {
    /// Resolves `reference`, as written in the note at `note_rel` (relative to
    /// the vault), to a source.
    ///
    /// A path is looked up next to the note first, then from the vault root (a
    /// leading `/` means the vault root only). The file must exist and, once
    /// symlinks and `..` are resolved, still lie inside `vault_dir`: a note
    /// cannot make the app read anything outside the vault. A query string or
    /// fragment on a path is ignored.
    pub fn resolve(reference: &str, vault_dir: &Path, note_rel: &Path) -> Option<Self> {
        let reference = reference.trim();
        if let Some(rest) = reference.strip_prefix("https://") {
            return (!rest.is_empty()).then(|| Self::Remote(reference.to_owned()));
        }
        // `http://`, `file:`, `data:` and every other scheme.
        if reference.contains("://") || reference.starts_with("data:") {
            return None;
        }

        let path = reference.split(['?', '#']).next().unwrap_or(reference);
        if path.is_empty() {
            return None;
        }
        let vault = vault_dir.canonicalize().ok()?;
        let from_root = path.trim_start_matches('/');
        let mut candidates = Vec::with_capacity(2);
        if !path.starts_with('/')
            && let Some(note_dir) = note_rel.parent()
        {
            candidates.push(vault.join(note_dir).join(path));
        }
        candidates.push(vault.join(from_root));

        candidates.into_iter().find_map(|candidate| {
            let resolved = candidate.canonicalize().ok()?;
            (resolved.starts_with(&vault) && resolved.is_file()).then_some(Self::Vault(resolved))
        })
    }

    /// What distinguishes this source's *current* content, for the cache key:
    /// the URL, or a vault file's path with its size and modification time.
    /// `None` if the file cannot be stat'ed.
    pub fn identity(&self) -> Option<String> {
        match self {
            Self::Remote(url) => Some(format!("remote:{url}")),
            Self::Vault(path) => {
                let meta = path.metadata().ok()?;
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                Some(format!("vault:{}:{}:{mtime}", path.display(), meta.len()))
            }
        }
    }

    /// Reads the original bytes, within the size limits.
    pub(crate) fn read(&self, fetcher: &dyn Fetcher) -> anyhow::Result<Vec<u8>> {
        match self {
            Self::Remote(url) => fetcher
                .fetch(url, MAX_REMOTE_BYTES)
                .with_context(|| format!("fetching {url}")),
            Self::Vault(path) => {
                let len = path.metadata()?.len();
                if len > MAX_LOCAL_BYTES {
                    bail!(
                        "{} is {len} bytes, over the {MAX_LOCAL_BYTES} byte limit",
                        path.display()
                    );
                }
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A vault with `a/note.tmt`'s neighbours: `a/near.png`, `far.png`, and a
    /// secret file one level above the vault.
    fn vault() -> (TempDir, PathBuf) {
        let root = TempDir::new().unwrap();
        let vault = root.path().join("vault");
        std::fs::create_dir_all(vault.join("a")).unwrap();
        std::fs::write(vault.join("a/near.png"), b"near").unwrap();
        std::fs::write(vault.join("far.png"), b"far").unwrap();
        std::fs::write(root.path().join("secret.png"), b"secret").unwrap();
        (root, vault)
    }

    fn resolved(vault: &Path, reference: &str) -> Option<PathBuf> {
        match MediaSource::resolve(reference, vault, Path::new("a/note.tmt"))? {
            MediaSource::Vault(path) => Some(
                path.strip_prefix(vault.canonicalize().unwrap())
                    .unwrap()
                    .to_owned(),
            ),
            MediaSource::Remote(_) => None,
        }
    }

    #[test]
    fn a_path_is_found_next_to_the_note_then_from_the_vault_root() {
        let (_root, vault) = vault();
        assert_eq!(
            resolved(&vault, "near.png"),
            Some(PathBuf::from("a/near.png"))
        );
        assert_eq!(resolved(&vault, "far.png"), Some(PathBuf::from("far.png")));
        assert_eq!(resolved(&vault, "/far.png"), Some(PathBuf::from("far.png")));
        // A leading slash means the vault root only, not the note's folder.
        assert_eq!(resolved(&vault, "/near.png"), None);
        assert_eq!(
            resolved(&vault, "../far.png"),
            Some(PathBuf::from("far.png"))
        );
        assert_eq!(
            resolved(&vault, "near.png?v=2#x"),
            Some(PathBuf::from("a/near.png"))
        );
        assert_eq!(resolved(&vault, "missing.png"), None);
        assert_eq!(resolved(&vault, ""), None);
    }

    #[test]
    fn a_path_cannot_leave_the_vault() {
        let (_root, vault) = vault();
        assert_eq!(resolved(&vault, "../../secret.png"), None);
        assert_eq!(resolved(&vault, "/../secret.png"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_cannot_lead_out_of_the_vault() {
        let (root, vault) = vault();
        std::os::unix::fs::symlink(root.path().join("secret.png"), vault.join("link.png")).unwrap();
        assert_eq!(resolved(&vault, "link.png"), None);
    }

    #[test]
    fn only_https_urls_are_remote_sources() {
        let (_root, vault) = vault();
        let note = Path::new("a/note.tmt");
        assert_eq!(
            MediaSource::resolve(" https://example.com/a.png?x=1 ", &vault, note),
            Some(MediaSource::Remote(
                "https://example.com/a.png?x=1".to_owned()
            ))
        );
        for rejected in [
            "http://example.com/a.png",
            "file:///etc/passwd",
            "data:image/png;base64,AAAA",
            "ftp://example.com/a.png",
            "https://",
        ] {
            assert_eq!(
                MediaSource::resolve(rejected, &vault, note),
                None,
                "{rejected}"
            );
        }
    }

    #[test]
    fn identity_changes_when_a_vault_file_does() {
        let (_root, vault) = vault();
        let source = MediaSource::resolve("far.png", &vault, Path::new("n.tmt")).unwrap();
        let before = source.identity().unwrap();
        std::fs::write(vault.join("far.png"), b"a longer replacement").unwrap();
        assert_ne!(source.identity().unwrap(), before);
        assert_eq!(
            MediaSource::Remote("https://e.com/a.png".into())
                .identity()
                .as_deref(),
            Some("remote:https://e.com/a.png")
        );
    }
}
