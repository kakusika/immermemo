//! Three-way merge for `.tmt` notes, at the AST level rather than the text
//! level.
//!
//! A textual `diff3` treats a note as lines. Tomet documents are trees of
//! `@element(args)[content]{data}` nodes, and most real edits touch
//! different nodes -- editing one `{data}` key on a device while another
//! device edits an unrelated block's `[content]` is not a conflict in any
//! meaningful sense, even though the two edits may land on adjacent lines.
//! Diffing the tree instead of the text is what lets those edits merge
//! silently, which is the entire point of doing this instead of shelling
//! out to `git merge-file`.
//!
//! # Algorithm
//!
//! `merge` walks `base`, `local` and `remote` together, top-down:
//!
//! 1. At each level, align the child sequence of `base` against `local`
//!    and against `remote` with an LCS-style alignment (the same technique
//!    a line-based diff uses, applied to sibling nodes instead of lines).
//!    Alignment is structural -- two nodes are "the same" position if they
//!    are AST-equal, not by any synthetic identity tomet would have to
//!    invent and inject into the user's document.
//! 2. A child unchanged on one side and changed on the other resolves to
//!    the changed side. A child added on only one side is kept. A child
//!    removed on only one side is dropped.
//! 3. A child changed on *both* sides recurses: if it is itself a
//!    container (has children), the conflict may dissolve further down --
//!    two edits to different keys of the same `@element`'s `{data}` are
//!    still non-conflicting. Recursion bottoms out at leaf values (a
//!    `Value` with no children), where "changed on both sides, to
//!    different things" cannot be narrowed any further.
//! 4. A true leaf conflict is wrapped in a pair of sibling
//!    `@mobile.conflict(mine)[...]` / `@mobile.conflict(theirs)[...]`
//!    elements (see `vocab/mobile.vocabulary.tmt`), replacing the
//!    contested node in place. The rest of the document merges normally
//!    around it -- a conflict never widens past the smallest node that
//!    actually disagrees.
//!
//! `@mobile.conflict` deliberately leaves `display` unset in its
//! vocabulary declaration, so it can stand wherever the node it replaces
//! could stand -- block or inline. The merge never needs to choose a
//! coarser replacement point to stay syntactically legal.
//!
//! If the merged document ends up with at least one `@mobile.conflict`,
//! `merge` also ensures `@use(mobile)` is present in the preamble (adding
//! it if missing); [`resolve`] removes it again once no
//! `@mobile.conflict` remains, so a note that has never conflicted never
//! carries the `@use` line.
//!
//! # What this crate assumes
//!
//! A `base` is always available: `immermemo-sync` only calls into this
//! crate for an actual three-way git merge, where `git2` has already found
//! a merge base commit. Linking two vaults that do not share history is a
//! separate operation (vault adoption), not a merge, and is not this
//! crate's problem.

use tomet_ast::Document;

/// The outcome of merging one note across two divergent versions of a
/// vault.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    /// The merged document. Always a structurally valid Tomet document --
    /// conflicts are represented as `@mobile.conflict` elements, never as
    /// malformed or partial output.
    pub document: Document,
    /// Where merging succeeded without leaving a conflict marker behind.
    pub clean: bool,
}

/// Merges `local` and `remote`, both descendants of `base`, into one
/// document.
///
/// Never fails: a merge that cannot be resolved automatically produces a
/// document containing `@mobile.conflict` markers rather than an error.
/// The caller (`immermemo-sync`) always has something to commit.
pub fn merge(base: &Document, local: &Document, remote: &Document) -> MergeResult {
    let _ = (base, local, remote);
    todo!("tree alignment per module doc -- see .agents/tasks/ for the current step")
}

/// Removes every `@mobile.conflict` pair from `document`, keeping the
/// `side` the caller picked for each, and drops the `@use(mobile)`
/// preamble line if none remain.
///
/// `resolutions` maps each conflict's position (in document order) to the
/// side that was kept. A position with no entry is left unresolved.
pub fn resolve(document: &Document, resolutions: &[ConflictResolution]) -> Document {
    let _ = (document, resolutions);
    todo!()
}

/// Which side of one `@mobile.conflict` pair a user picked, or that they
/// edited a fresh replacement by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    Mine,
    Theirs,
    /// The user wrote something new that is neither side verbatim.
    Rewritten(String),
}
