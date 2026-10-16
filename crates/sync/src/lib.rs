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
//! [`Vault::sync`] computes the merge base itself, and for every `.tmt` file
//! that changed on both sides, reads all three blobs (base/local/remote),
//! parses them, and hands them to [`tomet::merge::merge`] instead. The
//! result -- possibly containing `@conflict` markers -- becomes the
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
//! Only `.tmt` files go through [`tomet::merge::merge`]. A non-`.tmt`
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
//!
//! # Module layout
//!
//! - [`remote`]: fetch and push over whatever transport the remote URL implies.
//! - [`reconcile`]: fast-forward/adopt/three-way-merge once a fetch lands.
//! - [`checkout`]: writing the merged tree back into the working tree, and
//!   reading back which notes still have a conflict.
//! - [`gc`]: packing loose objects, and the heuristic that decides when to.
//! - [`history`]: reading a vault's git history back out.
//! - `fs`: staging the working tree into the git index.

mod checkout;
mod fs;
mod gc;
mod history;
mod reconcile;
mod remote;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod tls;

use std::path::{Path, PathBuf};

use git2::{Repository, Signature, Tree};

pub use gc::GcReport;
pub use history::{CommitSummary, FileRevision};

const REMOTE_NAME: &str = "origin";
const BRANCH: &str = "main";
const MAX_PUSH_ATTEMPTS: u32 = 5;

/// Supplies credentials for a fetch or push, without this crate knowing
/// where they actually live. Implemented once per platform (Android:
/// `app/src/credentials.rs`, backed by the Android Keystore).
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
    /// This device's own identity, used to sign every commit this
    /// `Vault` makes (see [`set_identity`](Self::set_identity)) and to
    /// decide authorship when labeling a conflict's sides (see
    /// [`conflict_authorship`](Self::conflict_authorship)). `None` (the
    /// default, same as a `Vault` nobody ever calls `set_identity` on)
    /// signs with the generic `"immermemo"` signature every device used
    /// before this existed, and never claims a conflict side as "mine" --
    /// the correct, honest degradation, not a bug.
    identity: Option<immermemo_identity::DeviceIdentity>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Relative paths of notes that contain unresolved `@conflict` markers.
    pub notes_needing_resolution: Vec<PathBuf>,
    /// Relative paths of notes written or updated in the working tree during this sync.
    pub updated_notes: Vec<PathBuf>,
    /// Relative paths of notes removed from the working tree during this sync.
    pub deleted_notes: Vec<PathBuf>,
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
            identity: None,
        })
    }

    /// Sets (or clears) the certificate verifier used to decide whether to
    /// trust a fetch/push's TLS connection when libgit2's own OS-level
    /// verification isn't available. See [`CertificateVerifier`].
    pub fn set_certificate_verifier(&mut self, verifier: Option<Box<dyn CertificateVerifier>>) {
        self.certificate_verifier = verifier;
    }

    /// Sets (or clears) this device's own identity -- see the `identity`
    /// field's own doc for what changes when it's set vs. left `None`.
    pub fn set_identity(&mut self, identity: Option<immermemo_identity::DeviceIdentity>) {
        self.identity = identity;
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
    /// `@conflict` markers, so the caller can surface them.
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
                    notes_needing_resolution: self.conflicted_among(&checkout.updated),
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

    /// Stages the working tree's current contents and commits them onto
    /// the vault's local history. A no-op if nothing changed since HEAD.
    fn commit_working_tree(&self) -> anyhow::Result<()> {
        let mut index = self.repo.index()?;
        let initial_index_len = index.len();
        fs::stage_dir(
            &self.repo,
            &mut index,
            &self.working_tree,
            &self.working_tree,
        )?;

        let parent = self.head_commit();
        if parent.is_none() && index.is_empty() {
            // A freshly linked vault, working tree still empty because
            // nothing has been fetched into it yet: there is no history
            // to compare against and nothing to commit. Committing here
            // anyway would create an empty root commit sharing no
            // history with whatever the remote already has, and the
            // next merge_base lookup would fail outright.
            return Ok(());
        }

        // Safety Guard: Empty Vault Wipeout Protection
        // Protect against accidental wipeout of the remote vault when syncing
        // from an empty or unhydrated local directory.
        if let Some(parent) = &parent
            && let Ok(parent_tree) = parent.tree()
            && !parent_tree.is_empty()
        {
            // 1. If index had 0 entries before staging, the working tree was
            //    never hydrated/checked out on this device. Do not commit an empty tree!
            if initial_index_len == 0 && index.is_empty() {
                return Ok(());
            }

            // 2. Total Wipeout Protection: If HEAD has multiple notes (>= 2),
            //    but the working tree now has 0 files, refuse to commit a total wipeout.
            //    Wiping out an entire multi-file vault in a single sync is almost
            //    always an unhydrated folder, unmounted drive, or accidental clear.
            if parent_tree.len() >= 2 && index.is_empty() {
                return Ok(());
            }
        }

        let tree_oid = index.write_tree_to(&self.repo)?;
        if let Some(parent) = &parent
            && parent.tree_id() == tree_oid
        {
            return Ok(());
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
    fn install_certificate_check<'a>(&'a self, callbacks: &mut git2::RemoteCallbacks<'a>) {
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
        let sig = self.signature()?;
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

    /// The signature every commit this `Vault` makes is signed with --
    /// `self.identity`'s `display_name` (cosmetic, defaults to the
    /// generic `"immermemo"` every device used before per-device identity
    /// existed) as the author *name*, `device_id` as the author *email*
    /// (`<device_id>@immermemo.local`) -- the part [`conflict_authorship`]
    /// actually compares. `self.identity: None` keeps today's fully
    /// generic signature, same as before this existed.
    fn signature(&self) -> anyhow::Result<Signature<'static>> {
        let (name, email) = match &self.identity {
            Some(identity) => (
                identity
                    .display_name
                    .clone()
                    .unwrap_or_else(|| "immermemo".to_owned()),
                format!("{}@immermemo.local", identity.device_id),
            ),
            None => ("immermemo".to_owned(), "immermemo@local".to_owned()),
        };
        Ok(Signature::now(&name, &email)?)
    }

    /// Whether `email` (an author's, from a commit) is *this* device's
    /// own, per `self.identity`. `false` whenever `self.identity` is
    /// unset -- a `Vault` with no identity never claims a conflict side
    /// as its own (see the `identity` field's own doc).
    fn email_is_this_device(&self, email: &str) -> bool {
        self.identity
            .as_ref()
            .is_some_and(|identity| email == format!("{}@immermemo.local", identity.device_id))
    }

    /// Who made each side of a `@conflict` still sitting in `note_rel_path`
    /// at `current_text` -- for labeling the resolution sheet honestly
    /// instead of assuming `a` is always "mine". It only ever answers
    /// for the *immediate* case: nothing committed since the merge that
    /// produced the conflict, never a deeper history walk.
    ///
    /// Reads `HEAD`: if it isn't a two-parent (merge) commit, or its tree
    /// entry for `note_rel_path` doesn't match `current_text` exactly
    /// (something changed since -- a later sync, an edit around the
    /// conflict), returns [`ConflictAuthorship::Unknown`] for both sides.
    /// Otherwise checks `HEAD`'s two parents' author emails against this
    /// device's own identity: `merge_histories`'s own fixed parent order
    /// is `[local_commit, remote_commit]`, and `markers.rs`/`merge.rs`'s
    /// own fixed convention is "`local` becomes `a`, `remote` becomes
    /// `b`" -- so `HEAD.parent(0)` authored `a`, `HEAD.parent(1)`
    /// authored `b`.
    pub fn conflict_authorship(
        &self,
        note_rel_path: &Path,
        current_text: &str,
    ) -> ConflictAuthorship {
        let unknown = ConflictAuthorship {
            a: ConflictSide::Unknown,
            b: ConflictSide::Unknown,
        };
        // No identity at all means this device can't judge authorship
        // one way or the other -- `Unknown`, not `NotMine`: without an
        // identity to compare against, "not mine" would be true of every
        // commit, including ones this device itself made, which isn't a
        // useful signal for a caller to act on.
        if self.identity.is_none() {
            return unknown;
        }
        let Some(head) = self.head_commit() else {
            return unknown;
        };
        if head.parent_count() != 2 {
            return unknown;
        }
        let Ok(tree) = head.tree() else {
            return unknown;
        };
        let Some(entry) = tree.get_path(note_rel_path).ok() else {
            return unknown;
        };
        let Ok(blob) = self.repo.find_blob(entry.id()) else {
            return unknown;
        };
        if blob.content() != current_text.as_bytes() {
            return unknown;
        };
        let side = |parent_index: usize| -> ConflictSide {
            let Ok(parent) = head.parent(parent_index) else {
                return ConflictSide::Unknown;
            };
            match parent.author().email() {
                Some(email) if self.email_is_this_device(email) => ConflictSide::Mine,
                Some(_) => ConflictSide::NotMine,
                None => ConflictSide::Unknown,
            }
        };
        ConflictAuthorship {
            a: side(0),
            b: side(1),
        }
    }
}

/// [`Vault::conflict_authorship`]'s answer for one side of a `@conflict`.
/// Three states, not a `bool`/`Option<bool>`: "I know this isn't mine"
/// (`NotMine`) is a real, useful answer (a UI can still show "Theirs"
/// confidently), distinct from genuinely not knowing either way
/// (`Unknown` -- no usable record at all, e.g. a third device's commit,
/// or the immediate-case check above didn't apply).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSide {
    Mine,
    NotMine,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictAuthorship {
    pub a: ConflictSide,
    pub b: ConflictSide,
}

#[cfg(test)]
mod tests;
