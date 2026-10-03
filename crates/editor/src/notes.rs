use std::path::{Path, PathBuf};

use immermemo_vault::appdata::AppData;
use immermemo_vault::history::{History, Restored};
use immermemo_vault::notes::{self, SearchResult};

use crate::state::{EditorState, current_path};

pub struct OpenedNote {
    /// Index into the filtered list the Files/Home screens show, if the
    /// opened note is currently in it.
    pub ui_current: Option<usize>,
    pub title: String,
    pub note_rel_path: String,
    pub has_conflict: bool,
    pub body: String,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Opens `notes[index]`, replacing whatever `state` had open. Returns
/// `None` for a bad index (nothing to show, nothing to report -- matches
/// how a stale UI index is silently ignored elsewhere), `Some(Err(_))`
/// with a user-facing message if the note couldn't be read off disk.
pub fn open_note(
    state: &mut EditorState,
    app_data: &AppData,
    vault_dir: &Path,
    notes: &[PathBuf],
    conflicted: &[bool],
    filtered_results: &[SearchResult],
    index: usize,
) -> Option<Result<OpenedNote, String>> {
    let path = notes.get(index)?.clone();
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            state.current = Some(index);
            state.history = Some(History::new(&text));
            let _ = app_data.save_last_note(vault_dir, &path);
            let ui_current = filtered_results.iter().position(|r| r.note_index == index);
            let note_rel_path = path
                .strip_prefix(vault_dir)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            Some(Ok(OpenedNote {
                ui_current,
                title: notes::display_name(vault_dir, &path),
                note_rel_path,
                has_conflict: conflicted.get(index).copied().unwrap_or(false),
                body: text,
                can_undo: state.history.as_ref().is_some_and(History::can_undo),
                can_redo: state.history.as_ref().is_some_and(History::can_redo),
            }))
        }
        Err(e) => Some(Err(format!("Could not open {}: {e}", path.display()))),
    }
}

/// The note that was last open in this vault, or the first note in the
/// list if none was recorded (or if the recorded note is gone). `None` if
/// the vault has no notes at all.
pub fn initial_or_last_note_index(
    app_data: &AppData,
    vault_dir: &Path,
    notes: &[PathBuf],
) -> Option<usize> {
    if notes.is_empty() {
        return None;
    }
    let last_note = app_data.load_last_note(vault_dir);
    Some(
        last_note
            .and_then(|p| notes.iter().position(|n| *n == p))
            .unwrap_or(0),
    )
}

/// The currently-open conflicted note, or the first conflicted note in
/// the vault if the open one (if any) isn't conflicted.
pub fn first_conflicted_note_index(state: &EditorState, conflicted: &[bool]) -> Option<usize> {
    if let Some(cur) = state.current
        && conflicted.get(cur).copied().unwrap_or(false)
    {
        Some(cur)
    } else {
        conflicted.iter().position(|&c| c)
    }
}

pub struct RestoredNote {
    pub body: String,
    pub cursor: usize,
    pub has_conflict: bool,
    /// A "Save failed: ..." message, if writing the restored text back
    /// to disk failed.
    pub write_error: Option<String>,
}

/// Writes an undo/redo step's restored text back to disk and reports what
/// changed. `restored` is `History::undo`/`redo`'s own return value --
/// callers that got `None` from those have nothing to apply.
pub fn apply_restored(state: &EditorState, notes: &[PathBuf], restored: Restored) -> RestoredNote {
    let Restored { text, cursor } = restored;
    let write_error = current_path(state, notes).and_then(|path| {
        std::fs::write(&path, &text)
            .err()
            .map(|e| format!("Save failed: {e}"))
    });
    let has_conflict = text.contains(immermemo_merge::CONFLICT_MARKER);
    RestoredNote {
        body: text,
        cursor,
        has_conflict,
        write_error,
    }
}
