use std::path::PathBuf;

use immermemo_sync::FileRevision;
use immermemo_vault::history::History;

/// What the editor is currently showing, independent of which vault it's
/// in or what that vault's note list/sync state look like -- those stay
/// in `apps/slint`'s own `Session` (see `crates/editor`'s module doc).
pub struct EditorState {
    /// Index into the caller's `notes: &[PathBuf]`.
    pub current: Option<usize>,
    /// Dropped whenever the note changes from outside (a sync) or another
    /// note is opened, so undo never crosses into different content.
    pub history: Option<History>,
    pub note_history_revisions: Vec<FileRevision>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            current: None,
            history: None,
            note_history_revisions: Vec::new(),
        }
    }
}

/// The path of the currently open note, if any -- `state.current` is an
/// index into `notes`, which is the caller's (vault-level) note list.
pub fn current_path(state: &EditorState, notes: &[PathBuf]) -> Option<PathBuf> {
    state.current.and_then(|i| notes.get(i).cloned())
}
