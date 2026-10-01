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
//! reads and writes with `std::fs` -- committing stages file contents directly
//! into the index via `Index::add_frombuffer` while comparing metadata (mtime
//! and size) against existing entries to skip disk reads for untouched files,
//! and checking out computes a tree diff (`diff_tree_to_tree`) to write and
//! delete only changed files without rewriting unmodified files or disturbing
//! their timestamps. The vault folder never contains anything but the user's
//! own files.
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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use git2::{
    Buf, Delta, FetchOptions, IndexEntry, IndexTime, Indexer, ObjectType, Oid, PushOptions,
    RemoteCallbacks, Repository, Signature, Tree, TreeWalkMode, TreeWalkResult,
};

const REMOTE_NAME: &str = "origin";
const BRANCH: &str = "main";
const MAX_PUSH_ATTEMPTS: u32 = 5;

/// Shard directory inside `.git/objects/` sampled to decide whether auto-GC is needed.
const AUTO_GC_SAMPLE_SHARD: &str = "17";
/// If the sampled shard has at least this many files (~4 * 256 ≈ 1,000 loose objects total),
/// `should_auto_gc` returns true.
const AUTO_GC_SHARD_THRESHOLD: usize = 4;

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
    /// Relative paths of notes that contain unresolved `@mobile.conflict` markers.
    pub notes_needing_resolution: Vec<PathBuf>,
    /// Relative paths of notes written or updated in the working tree during this sync.
    pub updated_notes: Vec<PathBuf>,
    /// Relative paths of notes removed from the working tree during this sync.
    pub deleted_notes: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Number of objects packed into the new packfile.
    pub packed_objects: usize,
    /// Number of loose object files pruned from disk.
    pub pruned_objects: usize,
}

/// A single revision of a file from Git history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRevision {
    /// Full 40-character commit hex hash.
    pub commit_id: String,
    /// 7-character short commit hash.
    pub short_id: String,
    /// Unix timestamp in seconds.
    pub timestamp_secs: i64,
    /// Commit message summary (first line).
    pub summary: String,
    /// Full text content of the file at this revision.
    pub content: String,
}

/// A commit summary from the repository's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    /// Full 40-character commit hex hash.
    pub commit_id: String,
    /// 7-character short commit hash.
    pub short_id: String,
    /// Unix timestamp in seconds.
    pub timestamp_secs: i64,
    /// Commit message summary (first line).
    pub summary: String,
}

impl Vault {
    /// Opens an existing vault, or initializes one if `gitdir` is
    /// empty -- the first sync of a folder and every later one go through
    /// the same call. Supports both private bare repositories (outside the
    /// working tree) and standard in-tree repositories (`.git` inside the working tree).
    pub fn open(working_tree: &Path, gitdir: &Path) -> anyhow::Result<Self> {
        let repo = if gitdir.join("HEAD").exists() || (gitdir.is_file() && gitdir.exists()) {
            Repository::open(gitdir).or_else(|_| Repository::open_bare(gitdir))?
        } else if gitdir == working_tree.join(".git") {
            Repository::init(working_tree)?
        } else {
            Repository::init_bare(gitdir)?
        };
        // Associates the working tree in-memory with the repository so libgit2
        // can evaluate `.gitignore` rules, without creating any `.git` link file.
        repo.set_workdir(working_tree, false)?;
        let _ = repo.set_head(&format!("refs/heads/{BRANCH}"));
        let _ = repo.add_ignore_rule(".DS_Store\nThumbs.db\n");
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
        let initial_head_tree_oid = self.head_commit().map(|c| c.tree_id());

        for _ in 0..MAX_PUSH_ATTEMPTS {
            self.fetch(credentials)?;
            self.reconcile_with_remote()?;
            if self.push(credentials)? {
                let checkout = self.checkout_differential(initial_head_tree_oid)?;
                if self.should_auto_gc() {
                    let _ = self.gc();
                }
                return Ok(SyncReport {
                    notes_needing_resolution: self.conflicted_notes()?,
                    updated_notes: checkout.updated,
                    deleted_notes: checkout.deleted,
                });
            }
            // Rejected: another push landed between our fetch and this
            // push. Fetch, reconcile and try again -- ordinary git
            // behavior, just automated instead of surfaced to a user.
        }

        anyhow::bail!("sync did not converge after {MAX_PUSH_ATTEMPTS} push attempts")
    }

    /// Checks whether loose objects in the repository have accumulated beyond
    /// the threshold where packing is recommended.
    ///
    /// Uses Git's standard heuristic of sampling a single 2-hex shard directory
    /// (`objects/17/`) rather than scanning all 256 shards, keeping this check
    /// sub-millisecond during regular sync cycles.
    pub fn should_auto_gc(&self) -> bool {
        let sample_dir = self.repo.path().join("objects").join(AUTO_GC_SAMPLE_SHARD);
        match std::fs::read_dir(sample_dir) {
            Ok(entries) => {
                let count = entries.flatten().filter(|e| e.path().is_file()).count();
                count >= AUTO_GC_SHARD_THRESHOLD
            }
            Err(_) => false,
        }
    }

    /// Packs all reachable objects into a single packfile in `objects/pack/`
    /// and prunes loose object files from disk.
    ///
    /// Safe to call at any time; if the repository has no objects or refs,
    /// returns an empty [`GcReport`] without writing files.
    pub fn gc(&self) -> anyhow::Result<GcReport> {
        let mut pb = self.repo.packbuilder()?;
        let mut revwalk = self.repo.revwalk()?;
        let _ = revwalk.push_glob("refs/*");
        pb.insert_walk(&mut revwalk)?;

        let object_count = pb.object_count();
        if object_count == 0 {
            return Ok(GcReport::default());
        }

        let mut buf = Buf::new();
        pb.write_buf(&mut buf)?;

        let odb = self.repo.odb()?;
        let pack_dir = self.repo.path().join("objects").join("pack");
        std::fs::create_dir_all(&pack_dir)?;
        let mut indexer = Indexer::new(Some(&odb), &pack_dir, 0o644, true)?;
        indexer.write_all(&buf)?;
        let _pack_name = indexer.commit()?;

        let pruned_objects = prune_loose_objects(&self.repo.path().join("objects"))?;

        Ok(GcReport {
            packed_objects: object_count,
            pruned_objects,
        })
    }

    /// Returns the commit history that modified `rel_path`, newest first.
    pub fn file_history(
        &self,
        rel_path: &Path,
        max_count: usize,
    ) -> anyhow::Result<Vec<FileRevision>> {
        let mut revwalk = match self.repo.revwalk() {
            Ok(rw) => rw,
            Err(_) => return Ok(Vec::new()),
        };
        if self.repo.head().is_err() {
            return Ok(Vec::new());
        }
        revwalk.push_head()?;
        revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let mut revisions = Vec::new();
        let path_str = rel_path.to_str().unwrap_or_default();

        for oid in revwalk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            let tree = commit.tree()?;

            let entry = tree.get_path(Path::new(path_str)).ok();
            let entry_id = entry.as_ref().map(|e| e.id());

            // Check if any parent had the exact same tree entry ID
            let changed = if commit.parent_count() == 0 {
                entry_id.is_some()
            } else {
                let mut parent_matches = false;
                for parent in commit.parents() {
                    let parent_tree = parent.tree()?;
                    let parent_entry = parent_tree.get_path(Path::new(path_str)).ok();
                    let parent_id = parent_entry.as_ref().map(|e| e.id());
                    if parent_id == entry_id {
                        parent_matches = true;
                        break;
                    }
                }
                !parent_matches && entry_id.is_some()
            };

            if changed {
                if let Some(entry) = entry {
                    let obj = entry.to_object(&self.repo)?;
                    if let Some(blob) = obj.as_blob() {
                        let content = String::from_utf8_lossy(blob.content()).into_owned();
                        let commit_id = oid.to_string();
                        let short_id = commit_id[..7.min(commit_id.len())].to_string();
                        let timestamp_secs = commit.time().seconds();
                        let summary = commit.summary().unwrap_or("").to_string();

                        revisions.push(FileRevision {
                            commit_id,
                            short_id,
                            timestamp_secs,
                            summary,
                            content,
                        });

                        if revisions.len() >= max_count {
                            break;
                        }
                    }
                }
            }
        }

        Ok(revisions)
    }

    /// Returns the recent commit summaries across the entire vault, newest first.
    pub fn vault_history(&self, max_count: usize) -> anyhow::Result<Vec<CommitSummary>> {
        let mut revwalk = match self.repo.revwalk() {
            Ok(rw) => rw,
            Err(_) => return Ok(Vec::new()),
        };
        if self.repo.head().is_err() {
            return Ok(Vec::new());
        }
        revwalk.push_head()?;
        revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let mut commits = Vec::new();
        for oid in revwalk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            let commit_id = oid.to_string();
            let short_id = commit_id[..7.min(commit_id.len())].to_string();
            let timestamp_secs = commit.time().seconds();
            let summary = commit.summary().unwrap_or("").to_string();

            commits.push(CommitSummary {
                commit_id,
                short_id,
                timestamp_secs,
                summary,
            });

            if commits.len() >= max_count {
                break;
            }
        }
        Ok(commits)
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
        stage_dir(
            &self.repo,
            &mut index,
            &self.working_tree,
            &self.working_tree,
        )?;

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
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        self.commit_tree("sync", &tree, &parents)?;
        let _ = index.write();
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
    /// a real three-way merge (treating unrelated histories with no merge base
    /// as having an empty base tree).
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
            None => self.set_branch_tip(remote_oid, "adopt remote history")?,
            Some(local) if local.id() == remote_oid => {}
            Some(local) => {
                let base_oid = match self.repo.merge_base(local.id(), remote_oid) {
                    Ok(oid) => Some(oid),
                    Err(e) if e.class() == git2::ErrorClass::Merge => None,
                    Err(e) => return Err(e.into()),
                };
                if base_oid == Some(local.id()) {
                    self.set_branch_tip(remote_oid, "fast-forward")?;
                } else if base_oid == Some(remote_oid) {
                    // Local is already ahead; nothing to reconcile.
                } else {
                    self.merge_histories(base_oid, local.id(), remote_oid)?;
                }
            }
        }
        Ok(())
    }

    /// Force-sets `refs/heads/{BRANCH}` to `oid`. Used for both the
    /// "adopt remote history" and "fast-forward" cases in
    /// [`reconcile_with_remote`], which both do exactly this with only the
    /// log message differing.
    fn set_branch_tip(&self, oid: Oid, reason: &str) -> anyhow::Result<()> {
        self.repo
            .reference(&format!("refs/heads/{BRANCH}"), oid, true, reason)?;
        Ok(())
    }

    /// Merges `local_oid` and `remote_oid` into a new merge commit.
    /// If `base_oid` is `None` (unrelated histories), an empty base tree is used.
    fn merge_histories(
        &self,
        base_oid: Option<Oid>,
        local_oid: Oid,
        remote_oid: Oid,
    ) -> anyhow::Result<()> {
        let base_tree = match base_oid {
            Some(oid) => Some(self.repo.find_commit(oid)?.tree()?),
            None => None,
        };
        let local_commit = self.repo.find_commit(local_oid)?;
        let remote_commit = self.repo.find_commit(remote_oid)?;
        let local_tree = local_commit.tree()?;
        let remote_tree = remote_commit.tree()?;

        let mut trees: Vec<&Tree<'_>> = vec![&local_tree, &remote_tree];
        if let Some(ref bt) = base_tree {
            trees.push(bt);
        }
        let paths = union_of_paths(&trees);

        let mut index = self.repo.index()?;
        index.clear()?;
        for path in paths {
            let base_blob = base_tree
                .as_ref()
                .and_then(|t| blob_at(&self.repo, t, &path));
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
                stage_bytes(&mut index, &path, &bytes, (0, 0))?;
            }
        }

        let tree_oid = index.write_tree_to(&self.repo)?;
        let tree = self.repo.find_tree(tree_oid)?;
        self.commit_tree("merge", &tree, &[&local_commit, &remote_commit])
    }

    /// Creates a commit on `refs/heads/{BRANCH}` pointing at `tree`, with the
    /// given `message` and `parents`. Deduplicates the `Signature::now` +
    /// `repo.commit` boilerplate that would otherwise appear in every place a
    /// commit is written.
    fn commit_tree(
        &self,
        message: &str,
        tree: &Tree,
        parents: &[&git2::Commit],
    ) -> anyhow::Result<()> {
        let sig = Signature::now("immermemo", "immermemo@local")?;
        self.repo.commit(
            Some(&format!("refs/heads/{BRANCH}")),
            &sig,
            &sig,
            message,
            tree,
            parents,
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

    /// Checks out changes between `old_tree_oid` and HEAD into the working tree.
    ///
    /// Computes a tree-to-tree diff and applies only added, modified, and deleted files,
    /// leaving unmodified files and their timestamps untouched on disk. If `old_tree_oid`
    /// is None, writes all files in HEAD (initial checkout).
    fn checkout_differential(&self, old_tree_oid: Option<Oid>) -> anyhow::Result<CheckoutReport> {
        let Some(commit) = self.head_commit() else {
            return Ok(CheckoutReport::default());
        };
        let new_tree = commit.tree()?;

        if Some(new_tree.id()) == old_tree_oid {
            return Ok(CheckoutReport::default());
        }

        let old_tree = match old_tree_oid {
            Some(oid) => self.repo.find_tree(oid).ok(),
            None => None,
        };

        let diff = self
            .repo
            .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;

        let mut updated = Vec::new();
        let mut deleted = Vec::new();

        for delta in diff.deltas() {
            match delta.status() {
                Delta::Deleted => {
                    if let Some(old_file) = delta.old_file().path() {
                        let path = self.working_tree.join(old_file);
                        if path.exists() {
                            let _ = std::fs::remove_file(&path);
                            remove_empty_parent_dirs(&self.working_tree, &path);
                        }
                        deleted.push(old_file.to_owned());
                    }
                }
                Delta::Added | Delta::Modified | Delta::Copied | Delta::Typechange => {
                    if let Some(new_file) = delta.new_file().path() {
                        if let Ok(blob) = self.repo.find_blob(delta.new_file().id()) {
                            let path = self.working_tree.join(new_file);
                            if let Some(parent) = path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            std::fs::write(&path, blob.content())?;
                            updated.push(new_file.to_owned());
                        }
                    }
                }
                Delta::Renamed => {
                    if let Some(old_file) = delta.old_file().path() {
                        let path = self.working_tree.join(old_file);
                        if path.exists() {
                            let _ = std::fs::remove_file(&path);
                            remove_empty_parent_dirs(&self.working_tree, &path);
                        }
                        deleted.push(old_file.to_owned());
                    }
                    if let Some(new_file) = delta.new_file().path() {
                        if let Ok(blob) = self.repo.find_blob(delta.new_file().id()) {
                            let path = self.working_tree.join(new_file);
                            if let Some(parent) = path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            std::fs::write(&path, blob.content())?;
                            updated.push(new_file.to_owned());
                        }
                    }
                }
                _ => {}
            }
        }

        if let Ok(mut index) = self.repo.index() {
            let _ = index.read_tree(&new_tree);
            let _ = index.write();
        }

        Ok(CheckoutReport { updated, deleted })
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CheckoutReport {
    updated: Vec<PathBuf>,
    deleted: Vec<PathBuf>,
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

/// Walks every non-ignored file under `dir` (relative to `root`), calling
/// `on_file(abs_path, posix_rel)` for each. Skips `.git` entries and
/// anything the repository's `.gitignore` rules would ignore. Used by
/// [`stage_dir`] to avoid repeating the `read_dir` + gitignore-filter
/// skeleton; `remove_unwanted` and `find_tmt_files` keep their own loops
/// because they need to handle directory-level operations or lack a
/// `Repository` reference.
fn walk_working_tree<F>(
    repo: &Repository,
    root: &Path,
    dir: &Path,
    on_file: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&Path, &str) -> anyhow::Result<()>,
{
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy() == ".git" {
            continue;
        }
        let rel_path = path.strip_prefix(root)?;
        if repo.status_should_ignore(rel_path)? {
            continue;
        }
        if path.is_dir() {
            walk_working_tree(repo, root, &path, on_file)?;
        } else {
            let rel = relative_posix_path(root, &path)?;
            on_file(&path, &rel)?;
        }
    }
    Ok(())
}

fn stage_dir(
    repo: &Repository,
    index: &mut git2::Index,
    root: &Path,
    dir: &Path,
) -> anyhow::Result<()> {
    let mut seen_paths = BTreeSet::new();

    walk_working_tree(repo, root, dir, &mut |path, rel| {
        seen_paths.insert(rel.to_string());

        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_) => return Ok(()),
        };
        let file_size = meta.len() as u32;
        let mtime = meta
            .modified()
            .map(system_time_to_index_time)
            .unwrap_or((0, 0));

        // Check if existing index entry matches mtime and size
        if let Some(entry) = index.get_path(Path::new(rel), 0) {
            if entry.file_size == file_size
                && entry.mtime.seconds() == mtime.0
                && entry.mtime.nanoseconds() == mtime.1
                && entry.id != Oid::zero()
            {
                // Unchanged: stat cache matches, no need to read file contents or re-hash
                return Ok(());
            }
        }

        let data = std::fs::read(path)?;
        stage_bytes(index, rel, &data, mtime)
    })?;

    // Remove any entries from index that no longer exist in working tree
    let mut paths_to_remove = Vec::new();
    for i in 0..index.len() {
        if let Some(entry) = index.get(i) {
            if let Ok(path_str) = std::str::from_utf8(&entry.path) {
                if !seen_paths.contains(path_str) {
                    paths_to_remove.push(PathBuf::from(path_str));
                }
            }
        }
    }
    for p in paths_to_remove {
        let _ = index.remove_path(&p);
    }

    Ok(())
}

fn stage_bytes(
    index: &mut git2::Index,
    path: &str,
    data: &[u8],
    mtime: (i32, u32),
) -> anyhow::Result<()> {
    let entry = IndexEntry {
        ctime: IndexTime::new(mtime.0, mtime.1),
        mtime: IndexTime::new(mtime.0, mtime.1),
        dev: 0,
        ino: 0,
        mode: 0o100644,
        uid: 0,
        gid: 0,
        file_size: data.len() as u32,
        id: Oid::zero(),
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    };
    index.add_frombuffer(&entry, data)?;
    Ok(())
}

fn system_time_to_index_time(t: std::time::SystemTime) -> (i32, u32) {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i32, d.subsec_nanos()),
        Err(e) => (
            -(e.duration().as_secs() as i32),
            e.duration().subsec_nanos(),
        ),
    }
}

fn remove_empty_parent_dirs(root: &Path, file_path: &Path) {
    let mut cur = file_path.parent();
    while let Some(parent) = cur {
        if parent == root || !parent.starts_with(root) {
            break;
        }
        // std::fs::remove_dir succeeds only when the directory is empty.
        if std::fs::remove_dir(parent).is_err() {
            break;
        }
        cur = parent.parent();
    }
}

fn prune_loose_objects(objects_dir: &Path) -> anyhow::Result<usize> {
    let mut pruned = 0;
    if !objects_dir.exists() {
        return Ok(0);
    }
    for entry in std::fs::read_dir(objects_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.len() == 2
            && name_str.chars().all(|c| c.is_ascii_hexdigit())
            && entry.path().is_dir()
        {
            let shard_dir = entry.path();
            if let Ok(shard_entries) = std::fs::read_dir(&shard_dir) {
                for file_entry in shard_entries.flatten() {
                    let file_path = file_entry.path();
                    if file_path.is_file() {
                        if std::fs::remove_file(&file_path).is_ok() {
                            pruned += 1;
                        }
                    }
                }
            }
            let _ = std::fs::remove_dir(&shard_dir);
        }
    }
    Ok(pruned)
}

fn relative_posix_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    Ok(path
        .strip_prefix(root)?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}

fn find_tmt_files(root: &Path, dir: &Path, found: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    // Plain recursive scan without gitignore filtering: this function is
    // called after checkout, so the vault only contains user files. No
    // Repository reference is available here, and the vault layout has no
    // nested .git dirs that would need to be skipped beyond the top-level
    // check below.
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy() == ".git" {
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

    #[test]
    fn gitignore_is_preserved_and_rules_are_respected() {
        let remote = shared_remote();

        let mut a = open_vault();
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();

        // Write a .gitignore, a normal note, and an ignored file.
        write_note(&a, ".gitignore", "*.tmp\nignored_dir/\n");
        write_note(&a, "note.tmt", "Hello.\n");
        write_note(&a, "scratch.tmp", "Temporary content.\n");
        std::fs::create_dir_all(a.working_tree.path().join("ignored_dir")).unwrap();
        write_note(&a, "ignored_dir/sub.tmt", "Ignored note.\n");

        a.vault.sync(&NoCredentials).unwrap();

        // Verify with another client: .gitignore and note.tmt arrive, but not scratch.tmp or ignored_dir.
        let mut b = open_vault();
        b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        b.vault.sync(&NoCredentials).unwrap();

        assert_eq!(read_note(&b, ".gitignore"), "*.tmp\nignored_dir/\n");
        assert_eq!(read_note(&b, "note.tmt"), "Hello.\n");
        assert!(!b.working_tree.path().join("scratch.tmp").exists());
        assert!(!b.working_tree.path().join("ignored_dir").exists());

        // Perform another sync on client a without changes: .gitignore must NOT be deleted.
        a.vault.sync(&NoCredentials).unwrap();
        assert_eq!(read_note(&a, ".gitignore"), "*.tmp\nignored_dir/\n");
        // And local ignored files on client a must NOT have been removed by remove_unwanted.
        assert!(a.working_tree.path().join("scratch.tmp").exists());
        assert!(a.working_tree.path().join("ignored_dir/sub.tmt").exists());

        // B syncs again and verifies .gitignore is still present in the git tree.
        b.vault.sync(&NoCredentials).unwrap();
        assert_eq!(read_note(&b, ".gitignore"), "*.tmp\nignored_dir/\n");
    }

    fn write_note(vault: &TestVault, name: &str, content: &str) {
        let path = vault.working_tree.path().join(name);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, content).unwrap();
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
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        write_note(&a, "note.tmt", "Hello.\n");
        a.vault.sync(&NoCredentials).unwrap();

        let mut b = open_vault();
        b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        b.vault.sync(&NoCredentials).unwrap();

        assert_eq!(read_note(&b, "note.tmt"), "Hello.\n");
    }

    #[test]
    fn edits_to_different_notes_merge_without_a_conflict() {
        let remote = shared_remote();

        let mut a = open_vault();
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        write_note(&a, "one.tmt", "One.\n");
        write_note(&a, "two.tmt", "Two.\n");
        a.vault.sync(&NoCredentials).unwrap();

        let mut b = open_vault();
        b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
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
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        write_note(&a, "note.tmt", "Bring a laptop.\n");
        a.vault.sync(&NoCredentials).unwrap();

        let mut b = open_vault();
        b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
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

    #[test]
    fn unrelated_histories_merge_cleanly() {
        let remote = shared_remote();

        // Client A initializes vault, writes note_a.tmt, syncs to remote (creates root commit on remote).
        let mut a = open_vault();
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        write_note(&a, "note_a.tmt", "Note A from Client A.\n");
        a.vault.sync(&NoCredentials).unwrap();

        // Client B initializes vault independently, writes note_b.tmt (creates local root commit before syncing).
        let mut b = open_vault();
        b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        write_note(&b, "note_b.tmt", "Note B from Client B.\n");

        // Client B syncs with remote. Local and remote have unrelated histories (no merge base).
        // This should not fail with "no merge base found", but produce a merge commit.
        let report = b.vault.sync(&NoCredentials).unwrap();
        assert!(report.notes_needing_resolution.is_empty());
        assert_eq!(read_note(&b, "note_a.tmt"), "Note A from Client A.\n");
        assert_eq!(read_note(&b, "note_b.tmt"), "Note B from Client B.\n");

        // Client A syncs again and receives note_b.tmt via fast-forward.
        let report = a.vault.sync(&NoCredentials).unwrap();
        assert!(report.notes_needing_resolution.is_empty());
        assert_eq!(read_note(&a, "note_a.tmt"), "Note A from Client A.\n");
        assert_eq!(read_note(&a, "note_b.tmt"), "Note B from Client B.\n");
    }

    #[test]
    fn in_tree_git_repository_syncs_and_interoperates_with_external_git() {
        let remote = shared_remote();

        // Device A: PC setup with an in-tree git repository (e.g. git clone or git init).
        let notes_dir = tempfile::tempdir().unwrap();
        let in_tree_repo = Repository::init(notes_dir.path()).unwrap();
        let in_tree_gitdir = notes_dir.path().join(".git");

        let mut a = Vault::open(notes_dir.path(), &in_tree_gitdir).unwrap();
        a.set_remote(remote.path().to_str().unwrap()).unwrap();

        // Write a note from PC
        std::fs::write(notes_dir.path().join("pc_note.tmt"), "Written on PC\n").unwrap();
        let report = a.sync(&NoCredentials).unwrap();
        assert!(report.notes_needing_resolution.is_empty());

        // Verify git commit is visible to the in-tree repo and external git commands
        let head = in_tree_repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.summary(), Some("sync"));

        // Verify working tree is clean for external git tools
        let statuses = in_tree_repo.statuses(None).unwrap();
        assert!(
            statuses.is_empty(),
            "Working tree should be clean in in-tree repo"
        );

        // Device B: Mobile / cloud setup using private bare repo
        let mut b = open_vault();
        b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        let report = b.vault.sync(&NoCredentials).unwrap();
        assert!(report.notes_needing_resolution.is_empty());
        assert_eq!(read_note(&b, "pc_note.tmt"), "Written on PC\n");

        // Device B writes mobile_note.tmt and syncs
        write_note(&b, "mobile_note.tmt", "Written on Mobile\n");
        b.vault.sync(&NoCredentials).unwrap();

        // Device A syncs and receives mobile_note.tmt
        let report = a.sync(&NoCredentials).unwrap();
        assert!(report.notes_needing_resolution.is_empty());
        assert_eq!(
            std::fs::read_to_string(notes_dir.path().join("mobile_note.tmt")).unwrap(),
            "Written on Mobile\n"
        );

        // In-tree repo continues to see working tree as clean
        let statuses = in_tree_repo.statuses(None).unwrap();
        assert!(
            statuses.is_empty(),
            "Working tree should remain clean after receiving remote changes"
        );
    }

    #[test]
    fn unchanged_notes_preserve_mtime_across_sync() {
        let remote = shared_remote();

        let mut a = open_vault();
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        write_note(&a, "note_a.tmt", "Note A initial.\n");
        let report = a.vault.sync(&NoCredentials).unwrap();
        // Client A already has note_a.tmt on disk; local push requires no checkout disk writes.
        assert!(report.updated_notes.is_empty());

        let note_a_path = a.working_tree.path().join("note_a.tmt");
        let initial_mtime = std::fs::metadata(&note_a_path).unwrap().modified().unwrap();

        // Client B syncs and receives note_a.tmt as an incoming update
        let mut b = open_vault();
        b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        let report_b = b.vault.sync(&NoCredentials).unwrap();
        assert_eq!(report_b.updated_notes, vec![PathBuf::from("note_a.tmt")]);

        // Sleep briefly to ensure filesystem timestamp ticks
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Client B adds note_b.tmt and syncs
        write_note(&b, "note_b.tmt", "Note B from B.\n");
        b.vault.sync(&NoCredentials).unwrap();

        // Client A syncs: receives note_b.tmt, note_a.tmt is unmodified
        let report = a.vault.sync(&NoCredentials).unwrap();
        assert_eq!(report.updated_notes, vec![PathBuf::from("note_b.tmt")]);
        assert!(report.deleted_notes.is_empty());

        // Note A must preserve its initial mtime (differential checkout did not touch it)
        let current_mtime = std::fs::metadata(&note_a_path).unwrap().modified().unwrap();
        assert_eq!(
            initial_mtime, current_mtime,
            "Untouched note mtime should be preserved"
        );

        // Sync again with no changes on either side
        let report = a.vault.sync(&NoCredentials).unwrap();
        assert!(report.updated_notes.is_empty());
        assert!(report.deleted_notes.is_empty());
    }

    #[test]
    fn deleted_remote_notes_are_removed_and_reported() {
        let remote = shared_remote();

        let mut a = open_vault();
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        write_note(&a, "folder/nested.tmt", "Nested note content.\n");
        a.vault.sync(&NoCredentials).unwrap();

        let mut b = open_vault();
        b.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        let report = b.vault.sync(&NoCredentials).unwrap();
        assert_eq!(
            report.updated_notes,
            vec![PathBuf::from("folder/nested.tmt")]
        );
        assert!(b.working_tree.path().join("folder/nested.tmt").exists());

        // Client A deletes nested.tmt and syncs
        std::fs::remove_file(a.working_tree.path().join("folder/nested.tmt")).unwrap();
        a.vault.sync(&NoCredentials).unwrap();

        // Client B syncs and receives the deletion
        let report = b.vault.sync(&NoCredentials).unwrap();
        assert_eq!(
            report.deleted_notes,
            vec![PathBuf::from("folder/nested.tmt")]
        );
        assert!(!b.working_tree.path().join("folder/nested.tmt").exists());
        assert!(
            !b.working_tree.path().join("folder").exists(),
            "Empty directory should be removed"
        );
    }

    #[test]
    fn gc_packs_loose_objects_and_preserves_repository_integrity() {
        let remote = shared_remote();
        let mut a = open_vault();
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();

        // Write a note and sync to create loose objects
        write_note(&a, "test.tmt", "Pack test content\n");
        a.vault.sync(&NoCredentials).unwrap();

        let objects_dir = a.vault.repo.path().join("objects");
        // Count loose object files before GC
        let mut loose_count_before = 0;
        for entry in std::fs::read_dir(&objects_dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.len() == 2 && name.chars().all(|c| c.is_ascii_hexdigit()) {
                if let Ok(shard_entries) = std::fs::read_dir(entry.path()) {
                    loose_count_before += shard_entries.flatten().count();
                }
            }
        }
        assert!(
            loose_count_before > 0,
            "Should have loose objects before GC"
        );

        // Run GC explicitly
        let gc_report = a.vault.gc().unwrap();
        assert!(gc_report.packed_objects > 0);
        assert_eq!(gc_report.pruned_objects, loose_count_before);

        // Pack directory should have .pack and .idx
        let pack_dir = objects_dir.join("pack");
        assert!(pack_dir.exists());
        let pack_files: Vec<_> = std::fs::read_dir(&pack_dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(pack_files.iter().any(|f| f.ends_with(".pack")));
        assert!(pack_files.iter().any(|f| f.ends_with(".idx")));

        // All loose shards should be gone
        let mut loose_count_after = 0;
        for entry in std::fs::read_dir(&objects_dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.len() == 2 && name.chars().all(|c| c.is_ascii_hexdigit()) {
                if let Ok(shard_entries) = std::fs::read_dir(entry.path()) {
                    loose_count_after += shard_entries.flatten().count();
                }
            }
        }
        assert_eq!(loose_count_after, 0, "All loose objects should be pruned");

        // Existing notes should be read cleanly from the packfile
        assert_eq!(read_note(&a, "test.tmt"), "Pack test content\n");

        // Subsequent commits and syncs continue to work seamlessly
        write_note(&a, "another.tmt", "Another note after GC\n");
        let report = a.vault.sync(&NoCredentials).unwrap();
        assert!(report.notes_needing_resolution.is_empty());
        assert_eq!(read_note(&a, "another.tmt"), "Another note after GC\n");
    }

    #[test]
    fn should_auto_gc_detects_shard_threshold() {
        let a = open_vault();
        let sample_shard = a
            .vault
            .repo
            .path()
            .join("objects")
            .join(AUTO_GC_SAMPLE_SHARD);
        std::fs::create_dir_all(&sample_shard).unwrap();

        // Below threshold
        assert!(!a.vault.should_auto_gc());

        // Create dummy loose object files in the sample shard to reach threshold
        for i in 0..AUTO_GC_SHARD_THRESHOLD {
            std::fs::write(sample_shard.join(format!("dummy_{i}")), b"dummy").unwrap();
        }
        assert!(
            a.vault.should_auto_gc(),
            "Should trigger auto-GC when threshold is met"
        );

        // Clean up dummy files
        for i in 0..AUTO_GC_SHARD_THRESHOLD {
            let _ = std::fs::remove_file(sample_shard.join(format!("dummy_{i}")));
        }
        assert!(
            !a.vault.should_auto_gc(),
            "Should not trigger after cleanup"
        );
    }

    #[test]
    fn auto_gc_runs_automatically_during_sync_when_threshold_met() {
        let remote = shared_remote();
        let mut a = open_vault();
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();
        write_note(&a, "note.tmt", "Initial content\n");

        // Populate sample shard to force auto-GC trigger on next sync
        let sample_shard = a
            .vault
            .repo
            .path()
            .join("objects")
            .join(AUTO_GC_SAMPLE_SHARD);
        std::fs::create_dir_all(&sample_shard).unwrap();
        for i in 0..AUTO_GC_SHARD_THRESHOLD {
            std::fs::write(sample_shard.join(format!("dummy_{i}")), b"dummy").unwrap();
        }
        assert!(a.vault.should_auto_gc());

        // Sync triggers auto-GC automatically
        let report = a.vault.sync(&NoCredentials).unwrap();
        assert!(report.notes_needing_resolution.is_empty());

        // Auto-GC should have run, creating pack and clearing loose objects
        let pack_dir = a.vault.repo.path().join("objects").join("pack");
        assert!(pack_dir.exists());
        assert!(
            !a.vault.should_auto_gc(),
            "Auto-GC should reset loose object condition"
        );
    }

    #[test]
    fn file_history_and_vault_history_track_revisions_and_content() {
        let remote = shared_remote();
        let mut a = open_vault();
        a.vault.set_remote(remote.path().to_str().unwrap()).unwrap();

        // Commit 1: create note1 and note2
        write_note(&a, "note1.tmt", "Version 1 of note 1\n");
        write_note(&a, "note2.tmt", "Version 1 of note 2\n");
        a.vault.sync(&NoCredentials).unwrap();

        // Commit 2: modify note1 only
        write_note(&a, "note1.tmt", "Version 2 of note 1\n");
        a.vault.sync(&NoCredentials).unwrap();

        // Commit 3: modify note2 only
        write_note(&a, "note2.tmt", "Version 2 of note 2\n");
        a.vault.sync(&NoCredentials).unwrap();

        // Check file_history for note1.tmt (should only have 2 revisions: V2 and V1, newest first)
        let hist1 = a.vault.file_history(Path::new("note1.tmt"), 10).unwrap();
        assert_eq!(hist1.len(), 2);
        assert_eq!(hist1[0].content, "Version 2 of note 1\n");
        assert_eq!(hist1[1].content, "Version 1 of note 1\n");
        assert_eq!(hist1[0].short_id.len(), 7);

        // Check file_history for note2.tmt
        let hist2 = a.vault.file_history(Path::new("note2.tmt"), 10).unwrap();
        assert_eq!(hist2.len(), 2);
        assert_eq!(hist2[0].content, "Version 2 of note 2\n");
        assert_eq!(hist2[1].content, "Version 1 of note 2\n");

        // Check vault_history (total 3 commits)
        let vault_hist = a.vault.vault_history(10).unwrap();
        assert_eq!(vault_hist.len(), 3);
    }
}
