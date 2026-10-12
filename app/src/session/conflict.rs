//! Conflict resolution handling, step navigation, and conflict sheet synchronization.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};

use crate::App;
use super::util::set_status;
use super::{
    Session, open_note, refresh_list, show_history_state, update_note_stats, update_rendered_body,
};

#[allow(dead_code)]
pub fn resolve_active_conflict(
    app: &App,
    session: &Rc<RefCell<Session>>,
    resolution: tomet::merge::ConflictResolution,
) {
    let outcome = {
        let mut guard = session.borrow_mut();
        let s = &mut guard.core;
        let vault_dir = s.vault_dir.clone();
        immermemo_editor::resolve_active_conflict(
            &mut s.editor,
            &vault_dir,
            &s.notes,
            &mut s.conflicted,
            &mut s.index,
            resolution,
        )
    };
    let Some(outcome) = outcome else { return };
    match outcome {
        Err(msg) => set_status(app, msg),
        Ok(resolved) => {
            show_history_state(app, &session.borrow());
            update_rendered_body(app, &resolved.body);
            update_note_stats(app, &resolved.body);
            let (conflicted_snapshot, conflict_count) = {
                let s = session.borrow();
                (
                    s.conflicted.clone(),
                    s.conflicted.iter().filter(|&&c| c).count(),
                )
            };
            app.set_conflicted(ModelRc::new(VecModel::from(conflicted_snapshot)));
            app.set_conflict_count(conflict_count as i32);
            app.set_current_has_conflict(false);
            app.set_conflict_sheet_open(false);
            refresh_list(app, session);
            set_status(app, resolved.status);
        }
    }
}

/// `(header label, button label)` for one side of a conflict, from
/// what `immermemo_sync::Vault::conflict_authorship` could determine.
fn conflict_side_labels(side: immermemo_sync::ConflictSide, letter: &str) -> (String, String) {
    match side {
        immermemo_sync::ConflictSide::Mine => {
            ("📱 Your changes".to_owned(), "Keep Mine".to_owned())
        }
        immermemo_sync::ConflictSide::NotMine => {
            ("☁️ Their changes".to_owned(), "Keep Theirs".to_owned())
        }
        immermemo_sync::ConflictSide::Unknown => {
            (format!("Version {letter}"), format!("Keep {letter}"))
        }
    }
}

pub fn sync_conflict_sheet_state(app: &App, session: &Rc<RefCell<Session>>) {
    let state = {
        let s = session.borrow();
        immermemo_editor::conflict_sheet_state(
            &s.editor,
            &s.notes,
            app.get_active_conflict_index(),
            &s.app_data,
            &s.vault_dir,
        )
    };
    match state {
        immermemo_editor::ConflictSheetState::NoActiveNote => {
            app.set_active_conflict_total(0);
            app.set_conflict_sheet_open(false);
        }
        immermemo_editor::ConflictSheetState::NoConflicts => {
            app.set_active_conflict_total(0);
            app.set_conflict_sheet_open(false);
            app.set_current_has_conflict(false);
            app.set_active_conflict_a(SharedString::new());
            app.set_active_conflict_b(SharedString::new());
        }
        immermemo_editor::ConflictSheetState::Active {
            total,
            index,
            a,
            b,
            a_side,
            b_side,
        } => {
            let (a_label, a_button_label) = conflict_side_labels(a_side, "A");
            let (b_label, b_button_label) = conflict_side_labels(b_side, "B");
            app.set_active_conflict_total(total as i32);
            app.set_active_conflict_index(index as i32);
            app.set_active_conflict_a(a.into());
            app.set_active_conflict_b(b.into());
            app.set_active_conflict_a_label(a_label.into());
            app.set_active_conflict_b_label(b_label.into());
            app.set_active_conflict_a_button_label(a_button_label.into());
            app.set_active_conflict_b_button_label(b_button_label.into());
        }
    }
}

pub fn resolve_conflict_step(app: &App, session: &Rc<RefCell<Session>>, choice: i32) {
    let active_conflict_index = app.get_active_conflict_index();
    let outcome = {
        let mut guard = session.borrow_mut();
        let s = &mut guard.core;
        let vault_dir = s.vault_dir.clone();
        immermemo_editor::resolve_conflict_step(
            &mut s.editor,
            &vault_dir,
            &s.notes,
            &mut s.conflicted,
            &mut s.index,
            active_conflict_index,
            choice,
        )
    };
    let Some(outcome) = outcome else { return };
    match outcome {
        Err(msg) => set_status(app, msg),
        Ok(resolved) => {
            show_history_state(app, &session.borrow());
            update_rendered_body(app, &resolved.body);
            update_note_stats(app, &resolved.body);
            let (conflicted_snapshot, conflict_count) = {
                let s = session.borrow();
                (
                    s.conflicted.clone(),
                    s.conflicted.iter().filter(|&&c| c).count(),
                )
            };
            app.set_conflicted(ModelRc::new(VecModel::from(conflicted_snapshot)));
            app.set_conflict_count(conflict_count as i32);
            app.set_current_has_conflict(resolved.has_conflict);
            sync_conflict_sheet_state(app, session);
            refresh_list(app, session);
            set_status(app, resolved.status);
        }
    }
}

pub fn open_first_conflicted_note(app: &App, session: &Rc<RefCell<Session>>) {
    let target_idx = {
        let s = session.borrow();
        immermemo_editor::first_conflicted_note_index(&s.editor, &s.conflicted)
    };
    if let Some(idx) = target_idx {
        open_note(app, session, idx);
        app.set_note_sheet_open(true);
        sync_conflict_sheet_state(app, session);
        app.set_conflict_sheet_open(true);
    }
}
