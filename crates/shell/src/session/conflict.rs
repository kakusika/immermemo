//! Conflict resolution handling, step navigation, and conflict sheet synchronization.

use crate::session::SessionRef;

use slint::{ModelRc, SharedString, VecModel};

use super::util::{set_error, set_status};
use super::{open_note, refresh_list, show_history_state, update_note_stats, update_rendered_body};
use crate::App;

#[allow(dead_code)]
pub fn resolve_active_conflict(
    app: &App,
    session: &SessionRef,
    resolution: immermemo_core::ConflictResolution,
) {
    let outcome = session.borrow_mut().resolve_active_conflict(resolution);
    let Some(outcome) = outcome else { return };
    match outcome {
        Err(msg) => set_error(app, msg),
        Ok(resolved) => {
            show_history_state(app, &session.borrow());
            update_rendered_body(app, session, &resolved.body);
            update_note_stats(app, session, &resolved.body);
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
fn conflict_side_labels(side: immermemo_core::ConflictSide, letter: &str) -> (String, String) {
    match side {
        immermemo_core::ConflictSide::Mine => {
            ("📱 Your changes".to_owned(), "Keep Mine".to_owned())
        }
        immermemo_core::ConflictSide::NotMine => {
            ("☁️ Their changes".to_owned(), "Keep Theirs".to_owned())
        }
        immermemo_core::ConflictSide::Unknown => {
            (format!("Version {letter}"), format!("Keep {letter}"))
        }
    }
}

pub fn sync_conflict_sheet_state(app: &App, session: &SessionRef) {
    let state = session
        .borrow()
        .conflict_sheet_state(app.get_active_conflict_index());
    match state {
        immermemo_core::ConflictSheetState::NoActiveNote => {
            app.set_active_conflict_total(0);
            app.set_conflict_sheet_open(false);
        }
        immermemo_core::ConflictSheetState::NoConflicts => {
            app.set_active_conflict_total(0);
            app.set_conflict_sheet_open(false);
            app.set_current_has_conflict(false);
            app.set_active_conflict_a(SharedString::new());
            app.set_active_conflict_b(SharedString::new());
        }
        immermemo_core::ConflictSheetState::Active {
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

pub fn resolve_conflict_step(app: &App, session: &SessionRef, choice: i32) {
    let active_conflict_index = app.get_active_conflict_index();
    let outcome = session
        .borrow_mut()
        .resolve_conflict_step(active_conflict_index, choice);
    let Some(outcome) = outcome else { return };
    match outcome {
        Err(msg) => set_error(app, msg),
        Ok(resolved) => {
            show_history_state(app, &session.borrow());
            update_rendered_body(app, session, &resolved.body);
            update_note_stats(app, session, &resolved.body);
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

pub fn open_first_conflicted_note(app: &App, session: &SessionRef) {
    let target_idx = {
        let s = session.borrow();
        immermemo_core::editor::first_conflicted_note_index(&s.editor, &s.conflicted)
    };
    if let Some(idx) = target_idx {
        open_note(app, session, idx);
        app.set_note_sheet_open(true);
        sync_conflict_sheet_state(app, session);
        app.set_conflict_sheet_open(true);
    }
}
