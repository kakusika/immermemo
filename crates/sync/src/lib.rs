//! Git-backed sync for a note vault, without putting `.git` anywhere the
//! user's cloud file provider can see it.
//!
//! # Why the repository is split from the working tree
//!
//! A vault's `.tmt` files can live anywhere the user wants them visible --
//! an iCloud Drive folder, a Google Drive folder, plain local storage. Git's
//! own metadata cannot live there too: `.git` is thousands of small loose
//! objects and a `.git` file/dir a file-provider sync layer was never
//! designed around, and on-device experience shows it fighting iCloud's and
//! Drive's own change detection (partial syncs, files quarantined as
//! "conflicted" copies by the *provider*, before git ever sees them).
//!
//! `libgit2`'s own `git init --separate-git-dir` equivalent
//! (`RepositoryInitOptions::workdir_path`) does not actually solve this --
//! it still writes a `.git` gitlink *file* into the working tree whenever
//! the workdir differs from the repo's natural location, because that file
//! is how the `git` CLI finds the real gitdir from inside the workdir.
//! `Vault` needs no such discovery: nothing outside this process ever runs
//! `git` inside the vault folder, so there is nothing to discover. Instead,
//! the repository here is opened **bare** (no workdir concept in libgit2 at
//! all), and "the working tree" is just an ordinary directory this crate
//! reads and writes with `std::fs` -- committing means staging file
//! contents directly into the index via `Index::add_frombuffer` (no file
//! has to exist relative to any libgit2-known workdir for that), and
//! checking out means walking a tree and writing blobs out by hand. The
//! vault folder never contains anything but the user's own files.
//!
//! # Sync cycle
//!
//! A sync is: commit whatever is dirty in the working tree, fetch the
//! remote, reconcile (fast-forward, or a real three-way merge when history
//! has diverged), push, and check out the result. When history has
//! diverged, the merge step never uses git's own (textual) merge driver --
//! [`sync`] computes the merge base itself, and for every `.tmt` file that
//! changed on both sides, reads all three blobs (base/local/remote),
//! parses them, and hands them to [`immermemo_merge::merge`] instead. The
//! result -- possibly containing `@mobile.conflict` markers -- becomes the
//! new tree, and the merge commit is recorded as a real two-parent git
//! commit, so history and blame stay intact even though the content-level
//! merge was ours rather than git's. A push that a concurrent push beat us
//! to is not an error: [`Vault::sync`] just fetches, reconciles and pushes
//! again.
//!
//! `git2` is built here with `vendored-libgit2` and `vendored-openssl`:
//! iOS and Android cannot be assumed to carry a system libgit2 or OpenSSL
//! to link against, so both are compiled from source and statically
//! linked into every target this crate ships to, desktop included, rather
//! than relying on the host toolchain only to have cross-compilation break
//! later.
//!
//! # What isn't decided yet
//!
//! Only `.tmt` files go through [`immermemo_merge::merge`]. A non-`.tmt`
//! file (an attachment) that both sides changed differently currently just
//! keeps `local`'s bytes -- there is no policy yet for attachments at all
//! (`docs/design.md` lists this as an open question: Git LFS is the
//! leading candidate, but nothing here implements it).
//!
//! A modify/delete conflict (one side edited a file, the other deleted it)
//! resolves by keeping the edit. This is a real, deliberate default --
//! preferring not to silently lose someone's edit over preferring a clean
//! deletion -- not an oversight, but it has not been discussed with the
//! app's author and should be treated as provisional.
//!
//! # Multiple vaults
//!
//! One [`Vault`] per folder the user has linked; each vault's private
//! gitdir is independent, keyed by the vault's id. Nothing here assumes a
//! single vault per app install.

pub mod tls;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use git2::{
    FetchOptions, IndexEntry, IndexTime, ObjectType, Oid, PushOptions, RemoteCallbacks,
    Repository, Signature, Tree, TreeWalkMode, TreeWalkResult,
};

const REMOTE_NAME: &str = "origin";
const BRANCH: &str = "main";
const MAX_PUSH_ATTEMPTS: u32 = 5;

/// Supplies credentials for a fetch or push, without this crate knowing
/// where they actually live. Implemented once per platform (Android:
/// `apps/slint/src/credentials.rs`, backed by the Android Keystore).
pub trait CredentialProvider {
    fn credentials(&self, remote_url: &str) -> anyhow::Result<git2::Cred>;
}

/// Decides whether to trust a server's TLS certificate, when libgit2's own
/// OS-level verification isn't available to fall back on. Only needed
/// where [`git2::init`]'s per-OS system CA bundle probing finds nothing to
/// check against at all -- Android is the only such target so far (see the
/// [`tls`] module doc for why `git2::opts::set_ssl_cert_file`/`_dir` can't
/// be used there either). Desktop leaves this unset and keeps using
/// libgit2's ordinary OS-provided verification.
pub trait CertificateVerifier {
    /// `cert_der`: the leaf certificate the server presented, in DER.
    fn trust(&self, host: &str, cert_der: &[u8]) -> bool;
}

/// One linked note folder: a working tree the user sees, backed by a git
/// repository that lives elsewhere.
pub struct Vault {
    /// The folder the user picked -- may be inside an iCloud/Drive-synced
    /// location. Holds only `.tmt` files (and whatever else the user put
    /// there). Never a `.git` of any kind.
    working_tree: PathBuf,
    /// Bare -- no workdir association in libgit2 at all. Lives in this
    /// app's private storage, opaque to the user.
    repo: Repository,
    certificate_verifier: Option<Box<dyn CertificateVerifier>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub notes_needing_resolution: Vec<PathBuf>,
}

impl Vault {
    /// Opens an existing vault, or initializes one if `private_gitdir` is
    /// empty -- the first sync of a folder and every later one go through
    /// the same call.
    pub fn open(working_tree: &Path, private_gitdir: &Path) -> anyhow::Result<Self> {
        let repo = if private_gitdir.join("HEAD").exists() {
            Repository::open_bare(private_gitdir)?
        } else {
            Repository::init_bare(private_gitdir)?
        };
        Ok(Self {
            working_tree: working_tree.to_owned(),
            repo,
            certificate_verifier: None,
        })
    }

    /// Sets (or clears) the certificate verifier used to decide whether to
    /// trust a fetch/push's TLS connection when libgit2's own OS-level
    /// verification isn't available. See [`CertificateVerifier`].
    pub fn set_certificate_verifier(&mut self, verifier: Option<Box<dyn CertificateVerifier>>) {
        self.certificate_verifier = verifier;
    }

    /// Links this vault to a remote (GitHub, Gitea, self-hosted). Takes a
    /// plain URL and nothing else -- there is no per-host special-casing
    /// anywhere in this crate, so which kind of host it is never matters
    /// here.
    ///
    /// Adopting a vault that already has content elsewhere (no shared
    /// history) is not a distinct operation: [`sync`](Self::sync) treats
    /// "remote has history and we don't" as an ordinary fast-forward.
    pub fn set_remote(&mut self, url: &str) -> anyhow::Result<()> {
        if self.repo.find_remote(REMOTE_NAME).is_ok() {
            self.repo.remote_set_url(REMOTE_NAME, url)?;
        } else {
            self.repo.remote(REMOTE_NAME, url)?;
        }
        Ok(())
    }

    /// Stages, commits, fetches, and reconciles with the remote. Returns
    /// which notes (relative paths) came back containing unresolved
    /// `@mobile.conflict` markers, so the caller can surface them.
    pub fn sync(&mut self, credentials: &dyn CredentialProvider) -> anyhow::Result<SyncReport> {
        self.commit_working_tree()?;

        for _ in 0..MAX_PUSH_ATTEMPTS {
            self.fetch(credentials)?;
            self.reconcile_with_remote()?;
            if self.push(credentials)? {
                self.checkout_head()?;
                return Ok(SyncReport {
                    notes_needing_resolution: self.conflicted_notes()?,
                });
            }
            // Rejected: another push landed between our fetch and this
            // push. Fetch, reconcile and try again -- ordinary git
            // behavior, just automated instead of surfaced to a user.
        }

        anyhow::bail!("sync did not converge after {MAX_PUSH_ATTEMPTS} push attempts")
    }

    fn head_commit(&self) -> Option<git2::Commit<'_>> {
        self.repo
            .find_reference(&format!("refs/heads/{BRANCH}"))
            .ok()
            .and_then(|r| r.peel_to_commit().ok())
    }

    /// Stages the working tree's current contents and commits them onto
    /// the vault's local history. A no-op if nothing changed since HEAD.
    fn commit_working_tree(&self) -> anyhow::Result<()> {
        let mut index = self.repo.index()?;
        index.clear()?;
        stage_dir(&mut index, &self.working_tree, &self.working_tree)?;

        let parent = self.head_commit();
        if parent.is_none() && index.len() == 0 {
            // A freshly linked vault, working tree still empty because
            // nothing has been fetched into it yet: there is no history
            // to compare against and nothing to commit. Committing here
            // anyway would create an empty root commit sharing no
            // history with whatever the remote already has, and the
            // next merge_base lookup would fail outright.
            return Ok(());
        }

        let tree_oid = index.write_tree_to(&self.repo)?;
        if let Some(parent) = &parent {
            if parent.tree_id() == tree_oid {
                return Ok(());
            }
        }

        let tree = self.repo.find_tree(tree_oid)?;
        let sig = Signature::now("immermemo", "immermemo@local")?;
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        self.repo.commit(
            Some(&format!("refs/heads/{BRANCH}")),
            &sig,
            &sig,
            "sync",
            &tree,
            &parents,
        )?;
        Ok(())
    }

    /// Registers the certificate-check callback, if a verifier was set,
    /// onto `callbacks`. Shared between [`fetch`](Self::fetch) and
    /// [`push`](Self::push): both open their own TLS connection.
    fn install_certificate_check<'a>(&'a self, callbacks: &mut RemoteCallbacks<'a>) {
        let Some(verifier) = &self.certificate_verifier else {
            return;
        };
        callbacks.certificate_check(move |cert, host| {
            let trusted = cert
                .as_x509()
                .is_some_and(|x509| verifier.trust(host, x509.data()));
            if trusted {
                Ok(git2::CertificateCheckStatus::CertificateOk)
            } else {
                Err(git2::Error::from_str(&format!(
                    "certificate for {host} is not trusted"
                )))
            }
        });
    }

    fn fetch(&self, credentials: &dyn CredentialProvider) -> anyhow::Result<()> {
        let mut remote = self.repo.find_remote(REMOTE_NAME)?;
        let url = remote.url().unwrap_or_default().to_string();
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(move |_url, _username, _allowed| {
            credentials
                .credentials(&url)
                .map_err(|e| git2::Error::from_str(&e.to_string()))
        });
        self.install_certificate_check(&mut callbacks);
        let mut opts = FetchOptions::new();
        opts.remote_callbacks(callbacks);
        remote.fetch(
            &[format!(
                "+refs/heads/{BRANCH}:refs/remotes/{REMOTE_NAME}/{BRANCH}"
            )],
            Some(&mut opts),
            None,
        )?;
        Ok(())
    }

    /// Brings the local branch up to date with what was just fetched:
    /// adopts the remote's history if there was none locally, fast-forwards
    /// if the remote is a strict descendant, does nothing if local is
    /// already ahead (the next push will carry it), and otherwise performs
    /// a real three-way merge.
    fn reconcile_with_remote(&self) -> anyhow::Result<()> {
        let Ok(remote_ref) = self
            .repo
            .find_reference(&format!("refs/remotes/{REMOTE_NAME}/{BRANCH}"))
        else {
            return Ok(()); // remote has no history yet
        };
        let Some(remote_oid) = remote_ref.target() else {
            return Ok(());
        };

        match self.head_commit() {
            None => {
                self.repo.reference(
                    &format!("refs/heads/{BRANCH}"),
                    remote_oid,
                    true,
                    "adopt remote history",
                )?;
            }
            Some(local) if local.id() == remote_oid => {}
            Some(local) => {
                let base_oid = self.repo.merge_base(local.id(), remote_oid)?;
                if base_oid == local.id() {
                    self.repo.reference(
                        &format!("refs/heads/{BRANCH}"),
                        remote_oid,
                        true,
                        "fast-forward",
                    )?;
                } else if base_oid == remote_oid {
                    // Local is already ahead; nothing to reconcile.
                } else {
                    self.merge_histories(base_oid, local.id(), remote_oid)?;
                }
            }
        }
        Ok(())
    }

    fn merge_histories(&self, base_oid: Oid, local_oid: Oid, remote_oid: Oid) -> anyhow::Result<()> {
        let base_tree = self.repo.find_commit(base_oid)?.tree()?;
        let local_commit = self.repo.find_commit(local_oid)?;
        let remote_commit = self.repo.find_commit(remote_oid)?;
        let local_tree = local_commit.tree()?;
        let remote_tree = remote_commit.tree()?;

        let paths = union_of_paths(&[&base_tree, &local_tree, &remote_tree]);

        let mut index = self.repo.index()?;
        index.clear()?;
        for path in paths {
            let base_blob = blob_at(&self.repo, &base_tree, &path);
            let local_blob = blob_at(&self.repo, &local_tree, &path);
            let remote_blob = blob_at(&self.repo, &remote_tree, &path);

            let merged = match (base_blob, local_blob, remote_blob) {
                // Deleted on both sides, or never existed on either --
                // nothing to write.
                (_, None, None) => None,
                // Added on exactly one side: no disagreement.
                (None, Some(l), None) => Some(l),
                (None, None, Some(r)) => Some(r),
                // Unchanged on one side: take whichever side is not base.
                (b, Some(l), Some(r)) if b.as_deref() == Some(l.as_slice()) => Some(r),
                (b, Some(l), Some(r)) if b.as_deref() == Some(r.as_slice()) => Some(l),
                // Deleted on one side, present (possibly changed) on the
                // other: keep the edit. See the module doc's "what isn't
                // decided yet" -- this default hasn't been discussed.
                (_, None, Some(r)) => Some(r),
                (_, Some(l), None) => Some(l),
                // Present, and different, on both sides: a real 3-way
                // merge for .tmt content; anything else keeps `local`
                // (also undecided -- see the module doc).
                (b, Some(l), Some(r)) => Some(merge_bytes(&path, b, l, r)),
            };

            if let Some(bytes) = merged {
                stage_bytes(&mut index, &path, &bytes)?;
            }
        }

        let tree_oid = index.write_tree_to(&self.repo)?;
        let tree = self.repo.find_tree(tree_oid)?;
        let sig = Signature::now("immermemo", "immermemo@local")?;
        self.repo.commit(
            Some(&format!("refs/heads/{BRANCH}")),
            &sig,
            &sig,
            "merge",
            &tree,
            &[&local_commit, &remote_commit],
        )?;
        Ok(())
    }

    /// Pushes the local branch. Returns `Ok(true)` if it was accepted,
    /// `Ok(false)` if the remote rejected it (moved since our last fetch)
    /// so the caller can fetch, reconcile and retry.
    fn push(&self, credentials: &dyn CredentialProvider) -> anyhow::Result<bool> {
        let mut remote = self.repo.find_remote(REMOTE_NAME)?;
        let url = remote.url().unwrap_or_default().to_string();
        let rejected: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        let rejected_in_callback = Rc::clone(&rejected);

        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(move |_url, _username, _allowed| {
            credentials
                .credentials(&url)
                .map_err(|e| git2::Error::from_str(&e.to_string()))
        });
        callbacks.push_update_reference(move |_refname, status| {
            if status.is_some() {
                *rejected_in_callback.borrow_mut() = true;
            }
            Ok(())
        });
        self.install_certificate_check(&mut callbacks);

        let mut opts = PushOptions::new();
        opts.remote_callbacks(callbacks);
        remote.push(
            &[format!("refs/heads/{BRANCH}:refs/heads/{BRANCH}")],
            Some(&mut opts),
        )?;
        Ok(!*rejected.borrow())
    }

    /// Writes HEAD's tree into the working tree, and removes working-tree
    /// files the new tree no longer has.
    fn checkout_head(&self) -> anyhow::Result<()> {
        let Some(commit) = self.head_commit() else {
            return Ok(());
        };
        let tree = commit.tree()?;

        let mut wanted = BTreeSet::new();
        tree.walk(TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() == Some(ObjectType::Blob) {
                let rel = format!("{root}{}", entry.name().unwrap_or_default());
                if let Ok(blob) = self.repo.find_blob(entry.id()) {
                    let path = self.working_tree.join(&rel);
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&path, blob.content());
                }
                wanted.insert(rel);
            }
            TreeWalkResult::Ok
        })?;

        remove_unwanted(&self.working_tree, &self.working_tree, &wanted)?;
        Ok(())
    }

    /// Which `.tmt` notes in the working tree still contain an unresolved
    /// `@mobile.conflict`, after a sync's checkout.
    fn conflicted_notes(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut found = Vec::new();
        find_tmt_files(&self.working_tree, &self.working_tree, &mut found)?;
        Ok(found
            .into_iter()
            .filter(|rel| {
                std::fs::read_to_string(self.working_tree.join(rel))
                    .ok()
                    .and_then(|src| tomet_parser::parse_document(&src).ok())
                    .is_some_and(|doc| immermemo_merge::has_conflicts(&doc))
            })
            .collect())
    }
}

/// Merges one file's three versions. `.tmt` files go through
/// `immermemo_merge`; anything else keeps `local` (no policy decided yet,
/// see the module doc).
fn merge_bytes(path: &str, base: Option<Vec<u8>>, local: Vec<u8>, remote: Vec<u8>) -> Vec<u8> {
    if !path.ends_with(".tmt") {
        return local;
    }

    let parse = |bytes: &[u8]| -> tomet_ast::Document {
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| tomet_parser::parse_document(s).ok())
            .unwrap_or_default()
    };
    let base_doc = base.as_deref().map(parse).unwrap_or_default();
    let local_doc = parse(&local);
    let remote_doc = parse(&remote);
    let result = immermemo_merge::merge(&base_doc, &local_doc, &remote_doc);
    tomet_printer::document_to_tm(&result.document).into_bytes()
}

fn stage_dir(index: &mut git2::Index, root: &Path, dir: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            stage_dir(index, root, &path)?;
        } else {
            let data = std::fs::read(&path)?;
            let rel = relative_posix_path(root, &path)?;
            stage_bytes(index, &rel, &data)?;
        }
    }
    Ok(())
}

fn stage_bytes(index: &mut git2::Index, path: &str, data: &[u8]) -> anyhow::Result<()> {
    let entry = IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o100644,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: Oid::zero(),
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    };
    index.add_frombuffer(&entry, data)?;
    Ok(())
}

fn relative_posix_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    Ok(path
        .strip_prefix(root)?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}

fn remove_unwanted(root: &Path, dir: &Path, wanted: &BTreeSet<String>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            remove_unwanted(root, &path, wanted)?;
        } else {
            let rel = relative_posix_path(root, &path)?;
            if !wanted.contains(&rel) {
                std::fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

fn find_tmt_files(root: &Path, dir: &Path, found: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            find_tmt_files(root, &path, found)?;
        } else if path.extension().is_some_and(|e| e == "tmt") {
            found.push(path.strip_prefix(root)?.to_owned());
        }
    }
    Ok(())
}

fn union_of_paths(trees: &[&Tree<'_>]) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for tree in trees {
        let _ = tree.walk(TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() == Some(ObjectType::Blob) {
                paths.insert(format!("{root}{}", entry.name().unwrap_or_default()));
            }
            TreeWalkResult::Ok
        });
    }
    paths
}

fn blob_at(repo: &Repository, tree: &Tree<'_>, path: &str) -> Option<Vec<u8>> {
    let entry = tree.get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(blob.content().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Local transport (a plain filesystem path) never invokes the
    /// credentials callback, so these tests never need real credentials.
    struct NoCredentials;
    impl CredentialProvider for NoCredentials {
        fn credentials(&self, _remote_url: &str) -> anyhow::Result<git2::Cred> {
            anyhow::bail!("not expected to be called for a local transport")
        }
    }

    /// A vault plus its own temp directories, kept alive for the test's
    /// duration (the `Vault` only borrows their paths).
    struct TestVault {
        working_tree: TempDir,
        _gitdir: TempDir,
        vault: Vault,
    }

    fn open_vault() -> TestVault {
        let working_tree = TempDir::new().unwrap();
        let gitdir = TempDir::new().unwrap();
        let vault = Vault::open(working_tree.path(), gitdir.path()).unwrap();
        TestVault {
            working_tree,
            _gitdir: gitdir,
            vault,
        }
    }

    fn write_note(vault: &TestVault, name: &str, content: &str) {
        std::fs::write(vault.working_tree.path().join(name), content).unwrap();
    }

    fn read_note(vault: &TestVault, name: &str) -> String {
        std::fs::read_to_string(vault.working_tree.path().join(name)).unwrap()
    }

    /// A bare repo standing in for a hosted git remote (GitHub, Gitea,
    /// self-hosted) -- real fetch/push over git's own local transport,
    /// no network or hosting account needed.
    fn shared_remote() -> TempDir {
        let dir = TempDir::new().unwrap();
        Repository::init_bare(dir.path()).unwrap();
        dir
    }

    #[test]
    fn push_then_pull_round_trip() {
        let remote = shared_remote();

        let mut a = open_vault();
        a.vault
            .set_remote(remote.path().to_str().unwrap())
            .unwrap();
        write_note(&a, "note.tmt", "Hello.\n");
        a.vault.sync(&NoCredentials).unwrap();

        let mut b = open_vault();
        b.vault
            .set_remote(remote.path().to_str().unwrap())
            .unwrap();
        b.vault.sync(&NoCredentials).unwrap();

        assert_eq!(read_note(&b, "note.tmt"), "Hello.\n");
    }

    #[test]
    fn edits_to_different_notes_merge_without_a_conflict() {
        let remote = shared_remote();

        let mut a = open_vault();
        a.vault
            .set_remote(remote.path().to_str().unwrap())
            .unwrap();
        write_note(&a, "one.tmt", "One.\n");
        write_note(&a, "two.tmt", "Two.\n");
        a.vault.sync(&NoCredentials).unwrap();

        let mut b = open_vault();
        b.vault
            .set_remote(remote.path().to_str().unwrap())
            .unwrap();
        b.vault.sync(&NoCredentials).unwrap();

        write_note(&a, "one.tmt", "One changed by A.\n");
        a.vault.sync(&NoCredentials).unwrap();

        write_note(&b, "two.tmt", "Two changed by B.\n");
        let report = b.vault.sync(&NoCredentials).unwrap();

        assert!(report.notes_needing_resolution.is_empty());
        assert_eq!(read_note(&b, "one.tmt"), "One changed by A.\n");
        assert_eq!(read_note(&b, "two.tmt"), "Two changed by B.\n");

        // A's next sync should pick up B's merge too.
        let report = a.vault.sync(&NoCredentials).unwrap();
        assert!(report.notes_needing_resolution.is_empty());
        assert_eq!(read_note(&a, "two.tmt"), "Two changed by B.\n");
    }

    #[test]
    fn editing_the_same_word_is_reported_as_needing_resolution() {
        let remote = shared_remote();

        let mut a = open_vault();
        a.vault
            .set_remote(remote.path().to_str().unwrap())
            .unwrap();
        write_note(&a, "note.tmt", "Bring a laptop.\n");
        a.vault.sync(&NoCredentials).unwrap();

        let mut b = open_vault();
        b.vault
            .set_remote(remote.path().to_str().unwrap())
            .unwrap();
        b.vault.sync(&NoCredentials).unwrap();

        write_note(&a, "note.tmt", "Bring a charger.\n");
        a.vault.sync(&NoCredentials).unwrap();

        write_note(&b, "note.tmt", "Bring a notebook.\n");
        let report = b.vault.sync(&NoCredentials).unwrap();

        assert_eq!(
            report.notes_needing_resolution,
            vec![PathBuf::from("note.tmt")]
        );
        let merged = read_note(&b, "note.tmt");
        assert!(merged.contains("@use(mobile)"));
        assert!(merged.contains("@mobile.conflict(mine)"));
        assert!(merged.contains("charger"));
        assert!(merged.contains("@mobile.conflict(theirs)"));
        assert!(merged.contains("notebook"));
    }
}
