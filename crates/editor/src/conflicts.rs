use std::path::{Path, PathBuf};

use immermemo_index::NoteIndex;
use immermemo_merge::ConflictResolution;

use crate::state::{EditorState, current_path};

pub struct ConflictResolved {
    pub body: String,
    /// Whether the note still has unresolved conflicts after this step.
    pub has_conflict: bool,
    pub status: String,
}

/// Resolves every conflict in the current note at once, in favor of one
/// side. No UI currently calls this -- the editor/properties "Keep
/// Mine"/"Keep Theirs" bar resolves one conflict at a time via
/// [`resolve_conflict_step`] instead, so that a same-looking button means
/// the same thing on every screen. Kept around (and covered by this
/// crate's tests) for a future bulk-resolve feature.
#[allow(dead_code)]
pub fn resolve_active_conflict(
    state: &mut EditorState,
    vault_dir: &Path,
    notes: &[PathBuf],
    conflicted: &mut [bool],
    index: &mut NoteIndex,
    resolution: ConflictResolution,
) -> Option<Result<ConflictResolved, String>> {
    let path = current_path(state, notes)?;
    let current_text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return Some(Err(format!("Could not read note: {e}"))),
    };
    let doc = match tomet_parser::parse_document(&current_text) {
        Ok(d) => d,
        Err(e) => {
            return Some(Err(format!(
                "Could not parse note for conflict resolution: {e}"
            )));
        }
    };
    let side_name = match &resolution {
        ConflictResolution::A => "local",
        ConflictResolution::B => "remote",
        _ => "custom",
    };
    let resolved_doc = immermemo_merge::resolve_all(&doc, resolution);
    let resolved_text = tomet_printer::document_to_tm(&resolved_doc);

    if let Err(e) = std::fs::write(&path, &resolved_text) {
        return Some(Err(format!("Save failed: {e}")));
    }
    if let Some(history) = state.history.as_mut() {
        history.edit(&resolved_text);
    }
    if let Ok(rel) = path.strip_prefix(vault_dir) {
        let _ = index.record_write(vault_dir, rel, &resolved_text);
        let _ = index.set_conflict(rel, false);
    }
    if let Some(cur) = state.current
        && cur < conflicted.len()
    {
        conflicted[cur] = false;
    }

    Some(Ok(ConflictResolved {
        body: resolved_text,
        has_conflict: false,
        status: format!("Resolved conflict (kept {side_name} version)"),
    }))
}

/// Resolves only the conflict at `active_conflict_index`, one at a time --
/// the editor/properties quick-resolve bar and the full `ConflictSheet`
/// both walk conflicts this way, one step per tap.
pub fn resolve_conflict_step(
    state: &mut EditorState,
    vault_dir: &Path,
    notes: &[PathBuf],
    conflicted: &mut [bool],
    index: &mut NoteIndex,
    active_conflict_index: i32,
    choice: i32,
) -> Option<Result<ConflictResolved, String>> {
    let path = current_path(state, notes)?;
    let current_text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return Some(Err(format!("Could not read note: {e}"))),
    };
    let doc = match tomet_parser::parse_document(&current_text) {
        Ok(d) => d,
        Err(e) => {
            return Some(Err(format!(
                "Could not parse note for conflict resolution: {e}"
            )));
        }
    };
    let target_idx = active_conflict_index.max(0) as usize;
    let resolution = match choice {
        0 => ConflictResolution::A,
        1 => ConflictResolution::B,
        _ => ConflictResolution::Both,
    };
    let choice_name = match choice {
        0 => "kept local",
        1 => "kept remote",
        _ => "kept both",
    };

    let resolved_doc = immermemo_merge::resolve_single(&doc, target_idx, resolution);
    let resolved_text = tomet_printer::document_to_tm(&resolved_doc);

    if let Err(e) = std::fs::write(&path, &resolved_text) {
        return Some(Err(format!("Save failed: {e}")));
    }
    if let Some(history) = state.history.as_mut() {
        history.edit(&resolved_text);
    }

    let remaining_conflicts = immermemo_merge::find_conflicts(&resolved_doc);
    let has_conflict = !remaining_conflicts.is_empty();
    if let Ok(rel) = path.strip_prefix(vault_dir) {
        let _ = index.record_write(vault_dir, rel, &resolved_text);
        let _ = index.set_conflict(rel, has_conflict);
    }
    if let Some(cur) = state.current
        && cur < conflicted.len()
    {
        conflicted[cur] = has_conflict;
    }

    let status = if has_conflict {
        format!("Resolved conflict ({choice_name})")
    } else {
        "All conflicts resolved".to_owned()
    };
    Some(Ok(ConflictResolved {
        body: resolved_text,
        has_conflict,
        status,
    }))
}

pub enum ConflictSheetState {
    /// No note is open, or its content couldn't be read/parsed -- nothing
    /// to show, and whether it "has a conflict" is left untouched.
    NoActiveNote,
    /// The open note parsed cleanly and has no conflicts.
    NoConflicts,
    Active {
        total: usize,
        index: usize,
        mine: String,
        theirs: String,
    },
}

/// Recomputes the `ConflictSheet`'s state for the currently open note,
/// clamping `requested_index` (the sheet's own `active-conflict-index`)
/// into range.
pub fn conflict_sheet_state(
    state: &EditorState,
    notes: &[PathBuf],
    requested_index: i32,
) -> ConflictSheetState {
    let Some(path) = current_path(state, notes) else {
        return ConflictSheetState::NoActiveNote;
    };
    let Ok(current_text) = std::fs::read_to_string(&path) else {
        return ConflictSheetState::NoActiveNote;
    };
    let Ok(doc) = tomet_parser::parse_document(&current_text) else {
        return ConflictSheetState::NoActiveNote;
    };
    let items = immermemo_merge::find_conflicts(&doc);
    if items.is_empty() {
        return ConflictSheetState::NoConflicts;
    }
    let total = items.len();
    let index = requested_index.max(0).min(total as i32 - 1) as usize;
    let item = &items[index];
    ConflictSheetState::Active {
        total,
        index,
        // `immermemo_merge::merge` always puts the device that ran it
        // into `a` and the incoming side into `b` -- correct for the
        // common case (resolving right after the sync that produced
        // this conflict), but not re-derived here for a conflict opened
        // cold (app restarted) or on a different device than the one
        // that merged it. See `.agents/tasks/adopt-std-conflict.md`'s
        // "decided" section for the git-ancestry-based fix that would
        // make this correct in every case, not implemented yet.
        mine: item.a.clone(),
        theirs: item.b.clone(),
    }
}
