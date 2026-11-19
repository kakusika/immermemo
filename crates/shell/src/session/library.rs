//! Library views: Starred, Recent, and Opened-Note History builders and actions.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use slint::{ModelRc, VecModel};

use super::{Session, SessionRef, lfs_remote, resolve_note_icon, set_error};
use crate::{App, NoteListEntry, SelectionCheckState};

/// Builds a `HashMap` from vault-relative path to canonical `note_index` in `s.notes`.
fn rel_path_index_map(s: &Session) -> HashMap<PathBuf, usize> {
    s.notes
        .iter()
        .enumerate()
        .filter_map(|(idx, abs)| {
            abs.strip_prefix(&s.vault_dir)
                .ok()
                .map(|rel| (rel.to_path_buf(), idx))
        })
        .collect()
}

/// Returns the parent folder display string for `rel_path`, or empty if at vault root.
fn folder_subtitle(rel_path: &Path) -> String {
    rel_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Formats `YYYY-MM-DD HH:MM` plus optional ` · <folder>` when not at vault root.
fn timestamp_folder_subtitle(epoch_secs: i64, rel_path: &Path) -> String {
    let ts = immermemo_core::format_ymd_hm(epoch_secs);
    let folder = folder_subtitle(rel_path);
    if folder.is_empty() {
        ts
    } else {
        format!("{ts} · {folder}")
    }
}

/// Returns the display title for a vault-relative note path (filename stem).
fn note_display_title(rel_path: &Path) -> String {
    rel_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| rel_path.to_string_lossy().into_owned())
}

fn build_note_list_entry(
    s: &Session,
    note_index: usize,
    subtitle: String,
    starred_set: &HashSet<&str>,
    lfs: Option<&immermemo_media::LfsRemote>,
) -> Option<NoteListEntry> {
    let abs_path = s.notes.get(note_index)?;
    let rel_path = abs_path.strip_prefix(&s.vault_dir).ok()?;
    let rel_str = rel_path.to_string_lossy();
    let title = note_display_title(rel_path);
    let has_conflict = s.conflicted.get(note_index).copied().unwrap_or(false);
    let is_starred = starred_set.contains(rel_str.as_ref());
    let icon = resolve_note_icon(s, abs_path, lfs);
    let path_str = abs_path.to_string_lossy().to_string();
    let selection_state = if s.selection.is_selected(&path_str) {
        SelectionCheckState::All
    } else {
        SelectionCheckState::None
    };
    Some(NoteListEntry {
        title: title.into(),
        subtitle: subtitle.into(),
        icon,
        has_conflict,
        is_starred,
        note_index: note_index as i32,
        path: path_str.into(),
        selection_state,
    })
}

/// Builds `NoteListEntry` items for the Starred view, preserving starred order.
pub fn build_starred_entries(s: &Session) -> Vec<NoteListEntry> {
    let starred = s.starred_paths();
    if starred.is_empty() {
        return Vec::new();
    }
    let starred_set: HashSet<&str> = starred.iter().map(String::as_str).collect();
    let index_map = rel_path_index_map(s);
    let lfs = lfs_remote(s);

    starred
        .iter()
        .filter_map(|rel_str| {
            let rel_path = Path::new(rel_str);
            let &note_index = index_map.get(rel_path)?;
            build_note_list_entry(
                s,
                note_index,
                folder_subtitle(rel_path),
                &starred_set,
                lfs.as_ref(),
            )
        })
        .collect()
}

/// Builds `NoteListEntry` items for the Recent view (up to 100 most recently modified notes).
pub fn build_recent_entries(s: &Session) -> Vec<NoteListEntry> {
    let recent = s.index.list_recent_with_mtime(100).unwrap_or_default();
    if recent.is_empty() {
        return Vec::new();
    }
    let starred = s.starred_paths();
    let starred_set: HashSet<&str> = starred.iter().map(String::as_str).collect();
    let index_map = rel_path_index_map(s);
    let lfs = lfs_remote(s);

    recent
        .iter()
        .filter_map(|entry| {
            let &note_index = index_map.get(&entry.note.path)?;
            let subtitle = timestamp_folder_subtitle(entry.mtime_secs, &entry.note.path);
            build_note_list_entry(s, note_index, subtitle, &starred_set, lfs.as_ref())
        })
        .collect()
}

/// Builds `NoteListEntry` items for the Opened-Note History view, most-recent-first.
pub fn build_history_entries(s: &Session) -> Vec<NoteListEntry> {
    let history = s.history_entries();
    if history.is_empty() {
        return Vec::new();
    }
    let starred = s.starred_paths();
    let starred_set: HashSet<&str> = starred.iter().map(String::as_str).collect();
    let index_map = rel_path_index_map(s);
    let lfs = lfs_remote(s);

    history
        .iter()
        .filter_map(|entry| {
            let rel_path = Path::new(&entry.path);
            let &note_index = index_map.get(rel_path)?;
            let subtitle = timestamp_folder_subtitle(entry.opened_at_secs as i64, rel_path);
            build_note_list_entry(s, note_index, subtitle, &starred_set, lfs.as_ref())
        })
        .collect()
}

/// Marks library views (Starred, Recent, History) as dirty and refreshes them if one is currently visible.
pub fn mark_library_views_dirty(app: &App, session: &SessionRef) {
    session.borrow_mut().library_views_dirty = true;
    if app.get_active_tab() == 2 && matches!(app.get_files_page(), 3 | 4 | 8) {
        refresh_library_views(app, session);
    }
}

/// Refreshes whichever active Library sub-page (Directory, Starred, Recent, History) is dirty.
pub fn ensure_library_views(app: &App, session: &SessionRef) {
    let page = app.get_files_page();
    if page == 1 {
        super::ensure_directory_views(app, session);
    } else if matches!(page, 3 | 4 | 8) && session.borrow().library_views_dirty {
        refresh_library_views(app, session);
    }
}

/// Rebuilds Starred, Recent, and History list models and pushes them to Slint.
pub fn refresh_library_views(app: &App, session: &SessionRef) {
    let (starred, recent, history) = {
        let mut s = session.borrow_mut();
        s.library_views_dirty = false;
        (
            build_starred_entries(&s),
            build_recent_entries(&s),
            build_history_entries(&s),
        )
    };
    app.set_starred_entries(ModelRc::new(VecModel::from(starred)));
    app.set_recent_entries(ModelRc::new(VecModel::from(recent)));
    app.set_history_entries(ModelRc::new(VecModel::from(history)));
}

/// Syncs `current-note-index` and `current-note-starred` to the Slint UI.
pub fn sync_current_note_starred(app: &App, session: &SessionRef) {
    let s = session.borrow();
    let current_idx = s.editor.current;
    let is_starred = current_idx
        .and_then(|idx| s.notes.get(idx))
        .and_then(|abs| abs.strip_prefix(&s.vault_dir).ok())
        .is_some_and(|rel| s.is_starred(&rel.to_string_lossy()));
    drop(s);
    app.set_current_note_index(current_idx.map_or(-1, |i| i as i32));
    app.set_current_note_starred(is_starred);
}

/// Toggles the starred state of `notes[note_index]` and refreshes UI state.
pub fn toggle_note_star(app: &App, session: &SessionRef, note_index: usize) {
    let (rel_str, is_current) = {
        let s = session.borrow();
        let Some(abs_path) = s.notes.get(note_index) else {
            return;
        };
        let Ok(rel_path) = abs_path.strip_prefix(&s.vault_dir) else {
            return;
        };
        (
            rel_path.to_string_lossy().into_owned(),
            s.editor.current == Some(note_index),
        )
    };
    if let Err(e) = session.borrow().toggle_star(&rel_str) {
        set_error(app, format!("Could not update star: {e}"));
        return;
    }
    if is_current {
        sync_current_note_starred(app, session);
    }
    mark_library_views_dirty(app, session);
}

/// Toggles the starred state of the currently open note.
pub fn toggle_current_star(app: &App, session: &SessionRef) {
    let current = session.borrow().editor.current;
    if let Some(index) = current {
        toggle_note_star(app, session, index);
    }
}

/// Clears the opened-note history for the active vault and refreshes library views.
pub fn clear_history(app: &App, session: &SessionRef) {
    if let Err(e) = session.borrow().clear_history() {
        set_error(app, format!("Could not clear history: {e}"));
        return;
    }
    mark_library_views_dirty(app, session);
}

/// Prepares and opens the note action sheet for canonical `note_index`.
pub fn open_note_menu(app: &App, session: &SessionRef, note_index: usize) {
    let info = {
        let s = session.borrow();
        s.notes
            .get(note_index)
            .and_then(|abs| abs.strip_prefix(&s.vault_dir).ok())
            .map(|rel| {
                let title = note_display_title(rel);
                let starred = s.is_starred(&rel.to_string_lossy());
                (title, starred)
            })
    };
    let Some((title, starred)) = info else {
        return;
    };
    app.set_note_menu_title(title.into());
    app.set_note_menu_starred(starred);
    app.set_note_menu_index(note_index as i32);
}
