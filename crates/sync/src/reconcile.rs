//! Brings the local branch up to date with what was just fetched: adopts,
//! fast-forwards, or performs a real three-way merge over `.tmt` content
//! (see the crate's module doc, "Sync cycle").

use std::collections::BTreeSet;

use git2::{Oid, Repository, Tree, TreeWalkMode, TreeWalkResult};

use crate::{BRANCH, REMOTE_NAME, Vault};

impl Vault {
    /// Brings the local branch up to date with what was just fetched:
    /// adopts the remote's history if there was none locally, fast-forwards
    /// if the remote is a strict descendant, does nothing if local is
    /// already ahead (the next push will carry it), and otherwise performs
    /// a real three-way merge (treating unrelated histories with no merge base
    /// as having an empty base tree).
    pub(crate) fn reconcile_with_remote(&self) -> anyhow::Result<()> {
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
    /// [`reconcile_with_remote`](Self::reconcile_with_remote), which both
    /// do exactly this with only the log message differing.
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
                crate::fs::stage_bytes(&mut index, &path, &bytes, (0, 0))?;
            }
        }

        let tree_oid = index.write_tree_to(&self.repo)?;
        let tree = self.repo.find_tree(tree_oid)?;
        self.commit_tree("merge", &tree, &[&local_commit, &remote_commit])
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

fn union_of_paths(trees: &[&Tree<'_>]) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for tree in trees {
        let _ = tree.walk(TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                paths.insert(format!("{root}{}", entry.name().unwrap_or_default()));
            }
            TreeWalkResult::Ok
        });
    }
    paths
}

fn blob_at(repo: &Repository, tree: &Tree<'_>, path: &str) -> Option<Vec<u8>> {
    let entry = tree.get_path(std::path::Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(blob.content().to_vec())
}
