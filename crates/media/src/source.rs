//! Where the bytes behind a note's image reference come from.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, bail};
use tomet::links::VaultLinkIndex;

/// The most a remote image may weigh, in bytes.
pub const MAX_REMOTE_BYTES: usize = 8 * 1024 * 1024;

/// The most a file inside the vault may weigh to be read as an image, in bytes.
pub const MAX_LOCAL_BYTES: u64 = 32 * 1024 * 1024;

/// How old the file-name index of a vault may be before a name it does not
/// know is looked for again: files arrive through sync while the app runs.
const NAME_INDEX_MAX_AGE: Duration = Duration::from_secs(5);

/// The most files the file-name index of one vault holds.
const NAME_INDEX_MAX_FILES: usize = 200_000;

/// What a Git LFS pointer file says: the object that stands in for the real
/// file, which is kept on the LFS server rather than in the repository.
///
/// A vault whose `.gitattributes` sends images through LFS holds these in
/// place of the images wherever Git's LFS filter has not run -- and
/// libgit2, which the app syncs with, never runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LfsPointer {
    /// The object's SHA-256, as lowercase hex.
    pub oid: String,
    /// The real file's size in bytes.
    pub size: u64,
}

impl LfsPointer {
    /// `Some` if `bytes` is exactly a pointer file (a few short `key value`
    /// lines, the first being the spec version); `None` for anything else,
    /// including every real image.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        const SPEC: &str = "version https://git-lfs.github.com/spec/v1";
        // A pointer is ~130 bytes; the spec caps it at 1024.
        if bytes.len() > 1024 {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.lines();
        if lines.next()? != SPEC {
            return None;
        }
        let (mut oid, mut size) = (None, None);
        for line in lines {
            match line.split_once(' ')? {
                ("oid", value) => oid = value.strip_prefix("sha256:").map(str::to_owned),
                ("size", value) => size = value.parse().ok(),
                _ => {}
            }
        }
        Some(Self {
            oid: oid.filter(|oid| oid.len() == 64 && oid.bytes().all(|b| b.is_ascii_hexdigit()))?,
            size: size?,
        })
    }
}

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
    ///
    /// A bare file name (`+c14d6f.png`, optionally written `ref:+c14d6f.png`)
    /// that no path finds is looked up by name among every file in the vault,
    /// the way `@link(ref: ...)` names things in a Tomet vault (case does not
    /// matter); with several files of that name, the one closest to the note
    /// wins. Tomet's own `VaultLinkIndex` makes that decision -- see
    /// [`find_by_name`] for what this crate adds around it.
    pub fn resolve(reference: &str, vault_dir: &Path, note_rel: &Path) -> Option<Self> {
        let reference = reference.trim();
        if let Some(rest) = reference.strip_prefix("https://") {
            return (!rest.is_empty()).then(|| Self::Remote(reference.to_owned()));
        }
        // `http://`, `file:`, `data:` and every other scheme.
        if reference.contains("://") || reference.starts_with("data:") {
            return None;
        }

        let reference = reference.strip_prefix("ref:").unwrap_or(reference).trim();
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

        let by_path = candidates.into_iter().find_map(|candidate| {
            let resolved = candidate.canonicalize().ok()?;
            (resolved.starts_with(&vault) && resolved.is_file()).then_some(resolved)
        });
        let found = by_path.or_else(|| {
            if path.contains('/') {
                return None;
            }
            find_by_name(&vault, path, note_rel, NAME_INDEX_MAX_AGE)
        });
        found.map(Self::Vault)
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
                let bytes =
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
                if let Some(pointer) = LfsPointer::parse(&bytes) {
                    bail!(
                        "{} is a Git LFS pointer ({} bytes, sha256:{}), not the image itself",
                        path.display(),
                        pointer.size,
                        pointer.oid
                    );
                }
                Ok(bytes)
            }
        }
    }
}

/// A vault's files, as Tomet's own link index, built by walking the vault.
/// Symlinks and dot-folders (`.git`, ...) are not entered, so nothing outside
/// the vault is ever indexed -- Tomet's `VaultLinkIndex` decides which file a
/// name means, but which files it is told about is this crate's call.
struct NameIndex {
    built: Instant,
    links: VaultLinkIndex,
}

impl NameIndex {
    fn build(vault: &Path) -> Self {
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut dirs = vec![vault.to_owned()];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if kind.is_dir() {
                    if !name.starts_with('.') {
                        dirs.push(entry.path());
                    }
                } else if kind.is_file()
                    && paths.len() < NAME_INDEX_MAX_FILES
                    && let Ok(relative) = entry.path().strip_prefix(vault)
                {
                    paths.push(relative.to_owned());
                }
            }
        }
        Self {
            built: Instant::now(),
            links: VaultLinkIndex::from_paths(paths),
        }
    }
}

static NAME_INDEXES: Mutex<Option<HashMap<PathBuf, NameIndex>>> = Mutex::new(None);

/// The file `name` means anywhere in `vault` (a canonical path), as seen from
/// the note at `note_rel` (relative to the vault): Tomet's `VaultLinkIndex`
/// picks it, the one closest to the note when several share the name. An
/// index built within `max_age` that does not know `name` is believed; an
/// older one is rebuilt first, in case the file has just arrived.
fn find_by_name(vault: &Path, name: &str, note_rel: &Path, max_age: Duration) -> Option<PathBuf> {
    let mut guard = NAME_INDEXES.lock().ok()?;
    let indexes = guard.get_or_insert_with(HashMap::new);
    let lookup = |index: &NameIndex| {
        let found = vault.join(index.links.resolve_ref(name, Some(note_rel))?);
        found.is_file().then_some(found)
    };

    if let Some(found) = indexes.get(vault).and_then(&lookup) {
        return Some(found);
    }
    // A miss, or a hit that has since vanished: believe it only if the index
    // is fresh.
    if indexes
        .get(vault)
        .is_some_and(|index| index.built.elapsed() < max_age)
    {
        return None;
    }
    let index = NameIndex::build(vault);
    let found = lookup(&index);
    indexes.insert(vault.to_owned(), index);
    found
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

    /// A vault shaped like a real one: a note with its attachments in a `-`
    /// folder beside it, and another note elsewhere with a same-named file.
    fn attachments_vault() -> (TempDir, PathBuf) {
        let root = TempDir::new().unwrap();
        let vault = root.path().join("vault");
        for dir in ["people/fauna/-", "people/mumei/-", ".git"] {
            std::fs::create_dir_all(vault.join(dir)).unwrap();
        }
        std::fs::write(vault.join("people/fauna/-/+a.png"), b"fauna").unwrap();
        std::fs::write(vault.join("people/mumei/-/+a.png"), b"mumei").unwrap();
        std::fs::write(vault.join("people/mumei/-/+b.png"), b"b").unwrap();
        std::fs::write(vault.join(".git/+hidden.png"), b"h").unwrap();
        (root, vault)
    }

    fn by_name(vault: &Path, reference: &str, note: &str) -> Option<String> {
        match MediaSource::resolve(reference, vault, Path::new(note))? {
            MediaSource::Vault(path) => Some(std::fs::read_to_string(path).unwrap()),
            MediaSource::Remote(_) => None,
        }
    }

    #[test]
    fn a_bare_file_name_is_found_anywhere_in_the_vault() {
        let (_root, vault) = attachments_vault();
        // The note and the picture are in different folders: only a name
        // lookup can connect them.
        assert_eq!(
            by_name(&vault, "+b.png", "people/fauna/n.tmt").as_deref(),
            Some("b")
        );
        assert_eq!(
            by_name(&vault, "ref:+b.png", "people/fauna/n.tmt").as_deref(),
            Some("b")
        );
        assert_eq!(by_name(&vault, "+nope.png", "people/fauna/n.tmt"), None);
        // A name with a folder in it is a path, never searched for.
        assert_eq!(
            by_name(&vault, "elsewhere/+b.png", "people/fauna/n.tmt"),
            None
        );
    }

    #[test]
    fn with_several_files_of_one_name_the_one_nearest_the_note_wins() {
        let (_root, vault) = attachments_vault();
        assert_eq!(
            by_name(&vault, "+a.png", "people/fauna/n.tmt").as_deref(),
            Some("fauna")
        );
        assert_eq!(
            by_name(&vault, "+a.png", "people/mumei/n.tmt").as_deref(),
            Some("mumei")
        );
    }

    #[test]
    fn dot_folders_are_not_searched() {
        let (_root, vault) = attachments_vault();
        assert_eq!(by_name(&vault, "+hidden.png", "n.tmt"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_not_found_by_name() {
        let (root, vault) = attachments_vault();
        std::fs::write(root.path().join("secret.png"), b"secret").unwrap();
        std::os::unix::fs::symlink(root.path().join("secret.png"), vault.join("+secret.png"))
            .unwrap();
        assert_eq!(by_name(&vault, "+secret.png", "n.tmt"), None);
    }

    #[test]
    fn a_file_that_arrives_later_is_found_once_the_index_is_old_enough() {
        let (_root, vault) = attachments_vault();
        let canonical = vault.canonicalize().unwrap();
        let note = Path::new("people/fauna/n.tmt");
        assert_eq!(
            find_by_name(&canonical, "+late.png", note, Duration::from_secs(3600)),
            None
        );
        std::fs::write(canonical.join("people/fauna/-/+late.png"), b"late").unwrap();
        // Still within the age limit: the miss is believed.
        assert_eq!(
            find_by_name(&canonical, "+late.png", note, Duration::from_secs(3600)),
            None
        );
        // Old enough: the index is rebuilt and the file is there.
        assert!(find_by_name(&canonical, "+late.png", note, Duration::ZERO).is_some());
    }

    const POINTER: &str = "version https://git-lfs.github.com/spec/v1\n\
        oid sha256:00d9507e56ba0a5ad49053dead913ee2a377c74c1571293fe1eeb76b53a2886f\n\
        size 563672\n";

    #[test]
    fn an_lfs_pointer_is_recognized_and_a_real_file_is_not() {
        assert_eq!(
            LfsPointer::parse(POINTER.as_bytes()),
            Some(LfsPointer {
                oid: "00d9507e56ba0a5ad49053dead913ee2a377c74c1571293fe1eeb76b53a2886f".to_owned(),
                size: 563_672
            })
        );
        assert_eq!(LfsPointer::parse(b"\x89PNG\r\n\x1a\n"), None);
        assert_eq!(LfsPointer::parse(b""), None);
        // A pointer missing its oid, or with a malformed one, is not one.
        assert_eq!(
            LfsPointer::parse(b"version https://git-lfs.github.com/spec/v1\nsize 5\n"),
            None
        );
        let short = POINTER.replace("00d9507e56ba0a5a", "zz");
        assert_eq!(LfsPointer::parse(short.as_bytes()), None);
    }

    #[test]
    fn a_vault_file_that_is_an_lfs_pointer_fails_with_a_clear_reason() {
        let (_root, vault) = vault();
        std::fs::write(vault.join("far.png"), POINTER).unwrap();
        let source = MediaSource::resolve("far.png", &vault, Path::new("n.tmt")).unwrap();
        struct NoNet;
        impl Fetcher for NoNet {
            fn fetch(&self, _: &str, _: usize) -> anyhow::Result<Vec<u8>> {
                unreachable!("a vault file is not fetched")
            }
        }
        let error = source.read(&NoNet).unwrap_err().to_string();
        assert!(error.contains("Git LFS pointer"), "{error}");
        assert!(error.contains("563672"), "{error}");
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
