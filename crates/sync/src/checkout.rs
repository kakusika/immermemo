//! Writing a fetched/merged tree back into the working tree, and reading
//! back which notes still carry an unresolved conflict afterward.

use git2::{Delta, Oid};
use std::path::{Path, PathBuf};

use crate::Vault;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CheckoutReport {
    pub(crate) updated: Vec<PathBuf>,
    pub(crate) deleted: Vec<PathBuf>,
}

impl Vault {
    /// Checks out changes between `old_tree_oid` and HEAD into the working tree.
    ///
    /// Computes a tree-to-tree diff and applies only added, modified, and deleted files,
    /// leaving unmodified files and their timestamps untouched on disk. If `old_tree_oid`
    /// is None, writes all files in HEAD (initial checkout).
    pub(crate) fn checkout_differential(
        &self,
        old_tree_oid: Option<Oid>,
    ) -> anyhow::Result<CheckoutReport> {
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

    /// Which of `updated` (the notes this sync's checkout just wrote --
    /// see [`checkout_differential`](Self::checkout_differential)) still
    /// contain an unresolved `@mobile.conflict`.
    ///
    /// Only checks files this sync actually touched rather than
    /// re-walking and re-parsing the whole working tree: a note this
    /// sync didn't write can't have gained a *new* conflict, and one
    /// left over from an earlier sync the user still hasn't resolved is
    /// already reflected in `immermemo-index`'s persisted `has_conflict`
    /// column (see `NoteIndex::apply_sync_report`), which is what the
    /// app's "unresolved conflicts" count actually reads -- this field
    /// is only the one-time "here's what this sync just did" report.
    pub(crate) fn conflicted_among(&self, updated: &[PathBuf]) -> Vec<PathBuf> {
        updated
            .iter()
            .filter(|rel| {
                std::fs::read_to_string(self.working_tree.join(rel))
                    .ok()
                    .and_then(|src| tomet::parser::parse_document(&src).ok())
                    .is_some_and(|doc| immermemo_merge::has_conflicts(&doc))
            })
            .cloned()
            .collect()
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
