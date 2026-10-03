use std::path::{Path, PathBuf};

use immermemo_index::NoteIndex;
use immermemo_sync::{FileRevision, Vault};
use immermemo_vault::appdata::AppData;
use immermemo_vault::history::History;

use crate::state::{EditorState, current_path};

/// The current note's revision history (newest-first, up to `limit`
/// entries), or `None` if there's no current note / its vault location
/// can't be resolved. A git error while reading history (no repo yet,
/// corrupt gitdir) is reported as an empty list, not `None` -- the
/// history sheet still opens, just with nothing in it.
pub fn load_note_history(
    app_data: &AppData,
    vault_dir: &Path,
    current_path: Option<PathBuf>,
    limit: usize,
) -> Option<Vec<FileRevision>> {
    let current_path = current_path?;
    let rel_path = current_path.strip_prefix(vault_dir).ok()?;
    let gitdir = app_data.gitdir(vault_dir).ok()?;
    Some(match Vault::open(vault_dir, &gitdir) {
        Ok(vault) => vault.file_history(rel_path, limit).unwrap_or_default(),
        Err(_) => Vec::new(),
    })
}

pub struct RestoredVersion {
    pub body: String,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Restores the current note to `state.note_history_revisions[rev_idx]`,
/// writing it to disk and recording the edit in both the search index and
/// the undo history. `None` if there's no current note or no such
/// revision; `Some(Err(_))` with a user-facing message if the write
/// failed.
pub fn restore_note_version(
    state: &mut EditorState,
    vault_dir: &Path,
    notes: &[PathBuf],
    index: &mut NoteIndex,
    rev_idx: usize,
) -> Option<Result<RestoredVersion, String>> {
    let path = current_path(state, notes)?;
    let content = state.note_history_revisions.get(rev_idx)?.content.clone();

    if let Err(e) = std::fs::write(&path, &content) {
        return Some(Err(format!("Failed to restore: {e}")));
    }
    if let Ok(rel_path) = path.strip_prefix(vault_dir) {
        let _ = index.record_write(vault_dir, rel_path, &content);
    }
    if let Some(h) = state.history.as_mut() {
        h.edit(&content);
    }

    Some(Ok(RestoredVersion {
        body: content,
        can_undo: state.history.as_ref().is_some_and(History::can_undo),
        can_redo: state.history.as_ref().is_some_and(History::can_redo),
    }))
}
