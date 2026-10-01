//! Reading a vault's git history back out: either one file's revisions, or
//! the vault's commit log as a whole.

use std::path::Path;

use crate::{BRANCH, Vault};

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
    /// A revwalk from HEAD, newest first -- the setup [`file_history`](Self::file_history)
    /// and [`vault_history`](Self::vault_history) both need before they can
    /// diverge into "did this commit touch `rel_path`" vs. "take every commit".
    /// `None` if there's no HEAD to walk from yet (a freshly linked vault).
    fn history_revwalk(&self) -> Option<git2::Revwalk<'_>> {
        let mut revwalk = self.repo.revwalk().ok()?;
        if self.repo.head().is_err() {
            return None;
        }
        revwalk.push_head().ok()?;
        revwalk
            .set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
            .ok()?;
        Some(revwalk)
    }

    /// Returns the commit history that modified `rel_path`, newest first.
    pub fn file_history(
        &self,
        rel_path: &Path,
        max_count: usize,
    ) -> anyhow::Result<Vec<FileRevision>> {
        let Some(revwalk) = self.history_revwalk() else {
            return Ok(Vec::new());
        };

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
        let Some(revwalk) = self.history_revwalk() else {
            return Ok(Vec::new());
        };

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

    pub(crate) fn head_commit(&self) -> Option<git2::Commit<'_>> {
        self.repo
            .find_reference(&format!("refs/heads/{BRANCH}"))
            .ok()
            .and_then(|r| r.peel_to_commit().ok())
    }
}
