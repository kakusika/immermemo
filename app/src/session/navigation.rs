//! Editor navigation: opening notes, back/forward history navigation, and tab switching.

use crate::session::SessionRef;
use std::path::Path;

use slint::SharedString;

use super::conflict::sync_conflict_sheet_state;
use super::history::{history_preview, show_history_state};
use super::util::{set_error, set_status};
use super::{clear_note_stats, update_note_stats, update_rendered_body};
use crate::App;

/// Opens `notes[index]` and records it in the back/forward stack.
pub fn open_note(app: &App, session: &SessionRef, index: usize) {
    if let Some(rel_path) = open_note_inner(app, session, index) {
        session.borrow_mut().note_nav.visit(rel_path);
        sync_nav_state(app, session);
    }
}

/// Switches the editor to `notes[index]`. Returns the opened note's
/// vault-relative path on success.
fn open_note_inner(app: &App, session: &SessionRef, index: usize) -> Option<String> {
    let outcome = session.borrow_mut().open_note(index)?;
    match outcome {
        Err(msg) => {
            set_error(app, msg);
            None
        }
        Ok(opened) => {
            app.set_current(opened.ui_current.map_or(-1, |i| i as i32));
            app.set_current_title(opened.title.into());
            app.set_current_note_path(opened.note_rel_path.clone().into());
            app.set_current_has_conflict(opened.has_conflict);
            app.set_can_undo(opened.can_undo);
            app.set_can_redo(opened.can_redo);
            update_rendered_body(app, &opened.body);
            update_note_stats(app, &opened.body);
            set_status(app, "");
            app.set_active_conflict_index(0);
            if opened.has_conflict {
                sync_conflict_sheet_state(app, session);
            } else {
                app.set_conflict_sheet_open(false);
                app.set_active_conflict_total(0);
                app.set_active_conflict_a(SharedString::new());
                app.set_active_conflict_b(SharedString::new());
            }
            Some(opened.note_rel_path)
        }
    }
}

/// Steps one note back in the open-note history, skipping over (and
/// dropping) any entries that no longer resolve to a live note.
pub fn navigate_back(app: &App, session: &SessionRef) {
    let rel_path = {
        let mut s = session.borrow_mut();
        let vault_dir = s.vault_dir.clone();
        let notes = s.notes.clone();
        s.note_nav
            .step_back(move |p| notes.iter().any(|n| *n == vault_dir.join(p)))
    };
    navigate_to(app, session, rel_path);
}

/// Mirrors `navigate_back`, stepping forward instead.
pub fn navigate_forward(app: &App, session: &SessionRef) {
    let rel_path = {
        let mut s = session.borrow_mut();
        let vault_dir = s.vault_dir.clone();
        let notes = s.notes.clone();
        s.note_nav
            .step_forward(move |p| notes.iter().any(|n| *n == vault_dir.join(p)))
    };
    navigate_to(app, session, rel_path);
}

fn navigate_to(app: &App, session: &SessionRef, rel_path: Option<String>) {
    let Some(rel_path) = rel_path else { return };
    let index = {
        let s = session.borrow();
        let abs_path = s.vault_dir.join(&rel_path);
        s.notes.iter().position(|p| *p == abs_path)
    };
    if let Some(index) = index {
        open_note_inner(app, session, index);
    }
    sync_nav_state(app, session);
}

pub fn sync_nav_state(app: &App, session: &SessionRef) {
    let (vault_dir, can_back, can_forward, back_path, forward_path) = {
        let s = session.borrow();
        (
            s.vault_dir.clone(),
            s.note_nav.can_back(),
            s.note_nav.can_forward(),
            s.note_nav.peek_back().map(str::to_string),
            s.note_nav.peek_forward().map(str::to_string),
        )
    };
    app.set_can_navigate_back(can_back);
    app.set_can_navigate_forward(can_forward);
    let max_width = app.get_body_content_width();
    let (back_title, back_items) = back_path
        .as_deref()
        .map(|p| history_preview(app, session, &vault_dir, p, max_width))
        .unwrap_or_default();
    let (forward_title, forward_items) = forward_path
        .as_deref()
        .map(|p| history_preview(app, session, &vault_dir, p, max_width))
        .unwrap_or_default();
    app.set_history_back_title(back_title.into());
    app.set_history_back_items(back_items);
    app.set_history_forward_title(forward_title.into());
    app.set_history_forward_items(forward_items);
}

/// Opens the note that was last open in `vault_dir`, or the first note in the
/// list if none was recorded (or if the recorded note is gone).
pub fn open_initial_or_last_note(app: &App, session: &SessionRef) {
    let index = {
        let s = session.borrow();
        immermemo_core::editor::initial_or_last_note_index(&s.app_data, &s.vault_dir, &s.notes)
    };
    if let Some(index) = index {
        open_note(app, session, index);
    }
}

/// Opens the note at `rel_path`, if it still exists in the current vault's note list.
pub fn open_note_by_rel_path(app: &App, session: &SessionRef, rel_path: &Path) -> bool {
    let index = {
        let s = session.borrow();
        let abs_path = s.vault_dir.join(rel_path);
        s.notes.iter().position(|p| *p == abs_path)
    };
    match index {
        Some(index) => {
            open_note(app, session, index);
            true
        }
        None => false,
    }
}

/// Shows the note list if the vault has no notes left, or reopens the initial/last note.
pub fn show_list_or_reopen(app: &App, session: &SessionRef, switch_to_files_tab: bool) {
    if session.borrow().notes.is_empty() {
        let s = session.borrow();
        s.app_data.clear_last_note(&s.vault_dir);
        drop(s);
        clear_note_stats(app);
        if switch_to_files_tab {
            app.set_active_tab(2);
        }
        app.set_list_open(true);
    } else {
        open_initial_or_last_note(app, session);
    }
}

/// Clears the currently-open note's UI state and shows the note list or reopens another note.
pub fn close_current_note_and_show_list_or_reopen(
    app: &App,
    session: &SessionRef,
    switch_to_files_tab: bool,
) {
    session.borrow_mut().editor.history = None;
    app.set_current_title(SharedString::new());
    show_history_state(app, &session.borrow());
    update_rendered_body(app, "");
    show_list_or_reopen(app, session, switch_to_files_tab);
}
