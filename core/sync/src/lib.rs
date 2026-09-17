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
//! `libgit2` (via `git2`) supports opening a repository whose object
//! database and whose working tree are two unrelated paths -- normally
//! surfaced as `git init --separate-git-dir`, which still leaves a `.git`
//! *pointer file* behind in the working tree so the `git` CLI can find the
//! real gitdir. `Vault` skips even that: nothing outside this process ever
//! runs `git` inside the vault folder, so there is no discovery problem to
//! solve, and therefore no pointer file to write. The vault folder holds
//! exactly the user's own files. The real repository -- object database,
//! refs, index -- lives under this app's private storage, keyed by vault
//! id, and is opened by passing both paths to `git2::Repository::open_ext`
//! explicitly every time.
//!
//! # Sync cycle
//!
//! A sync is: stage everything dirty in the working tree, commit, fetch the
//! remote, and fast-forward or merge. The merge step never uses git's own
//! (textual) merge driver -- when history has diverged, `Vault::sync`
//! computes the merge base itself, reads all three blobs (base/local/
//! remote) for every note that changed on both sides, and hands them to
//! [`immermemo_merge::merge`] instead. The result -- possibly containing
//! `@mobile.conflict` markers -- is written back to the working tree and
//! staged, and the merge commit is recorded as a real two-parent git
//! commit, so history and blame stay intact even though the content-level
//! merge was ours rather than git's.
//!
//! # Credentials
//!
//! Fetch/push authentication is not this crate's business beyond an
//! injection point: [`CredentialProvider`] is implemented once per
//! platform in `bindings/mobile` (iOS Keychain, Android Keystore) and
//! passed in, so this crate never touches OS-specific secret storage.
//!
//! `git2` is built here with `vendored-libgit2` and `vendored-openssl`:
//! iOS and Android cannot be assumed to carry a system libgit2 or OpenSSL
//! to link against, so both are compiled from source and statically
//! linked into every target this crate ships to, desktop included, rather
//! than relying on the host toolchain only to have cross-compilation break
//! later.
//!
//! # Multiple vaults
//!
//! One [`Vault`] per folder the user has linked; each vault's private
//! gitdir is independent, keyed by the vault's id. Nothing here assumes a
//! single vault per app install.

use std::path::{Path, PathBuf};

/// Supplies credentials for a fetch or push, without this crate knowing
/// where they actually live.
pub trait CredentialProvider {
    fn credentials(&self, remote_url: &str) -> anyhow::Result<git2::Cred>;
}

/// One linked note folder: a working tree the user sees, backed by a git
/// repository that lives elsewhere.
pub struct Vault {
    /// The folder the user picked -- may be inside an iCloud/Drive-synced
    /// location. Holds only `.tmt` files and whatever else the user put
    /// there. Never a `.git` of any kind.
    working_tree: PathBuf,
    /// This app's private storage for this vault's object database, refs
    /// and index. Opaque to the user.
    private_gitdir: PathBuf,
}

impl Vault {
    /// Opens an existing vault, or initializes one if `private_gitdir` is
    /// empty -- the first sync of a folder and every later one go through
    /// the same call.
    pub fn open(working_tree: &Path, private_gitdir: &Path) -> anyhow::Result<Self> {
        let _ = (working_tree, private_gitdir);
        todo!()
    }

    /// Links this vault to a remote (GitHub, Gitea, self-hosted). Adopting
    /// a vault that already has content elsewhere -- no shared history --
    /// is a distinct, explicit step from an ordinary sync and is handled
    /// here, not inside `immermemo-merge`.
    pub fn set_remote(&mut self, url: &str) -> anyhow::Result<()> {
        let _ = url;
        todo!()
    }

    /// Stages, commits, fetches, and reconciles with the remote. Returns
    /// which notes (relative paths) came back containing unresolved
    /// `@mobile.conflict` markers, so the caller can surface them.
    pub fn sync(&mut self, credentials: &dyn CredentialProvider) -> anyhow::Result<SyncReport> {
        let _ = credentials;
        todo!()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub notes_needing_resolution: Vec<PathBuf>,
}
