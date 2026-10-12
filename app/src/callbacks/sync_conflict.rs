//! Sync triggering, sync error dismissal, conflict resolution, and history callbacks.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, SharedString};

use crate::App;
use crate::session::{
    self, Session, open_first_conflicted_note, resolve_conflict_step, start_sync,
    sync_conflict_sheet_state,
};

pub fn bind_sync_conflict_callbacks(app: &App, session: &Rc<RefCell<Session>>) {
    app.on_sync({
        let (weak, session) = (app.as_weak(), session.clone());
        move || start_sync(&weak.unwrap(), &session)
    });

    let jump_pos = Rc::new(RefCell::new(0usize));
    app.on_jump_conflict({
        let (weak, session, jump_pos) = (app.as_weak(), session.clone(), jump_pos);
        move || {
            let app = weak.unwrap();
            let s = session.borrow();
            let Some(path) = s.current_path() else {
                return;
            };
            let Ok(text) = std::fs::read_to_string(path) else {
                return;
            };
            let mut offset = *jump_pos.borrow();
            if offset >= text.len() {
                offset = 0;
            }
            let found = text[offset..]
                .find(tomet::merge::CONFLICT_MARKER)
                .map(|p| offset + p)
                .or_else(|| text.find(tomet::merge::CONFLICT_MARKER));
            if let Some(pos) = found {
                app.invoke_set_cursor(pos as i32);
                *jump_pos.borrow_mut() = pos + tomet::merge::CONFLICT_MARKER.len();
            }
        }
    });

    app.on_resolve_mine({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            resolve_conflict_step(&app, &session, 0);
        }
    });

    app.on_resolve_theirs({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            resolve_conflict_step(&app, &session, 1);
        }
    });

    app.on_open_conflict_sheet({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            sync_conflict_sheet_state(&app, &session);
            app.set_conflict_sheet_open(true);
        }
    });

    app.on_resolve_conflict_step({
        let (weak, session) = (app.as_weak(), session.clone());
        move |choice| {
            let app = weak.unwrap();
            resolve_conflict_step(&app, &session, choice);
        }
    });

    app.on_prev_conflict_step({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let idx = app.get_active_conflict_index();
            if idx > 0 {
                app.set_active_conflict_index(idx - 1);
                sync_conflict_sheet_state(&app, &session);
            }
        }
    });

    app.on_next_conflict_step({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let idx = app.get_active_conflict_index();
            let total = app.get_active_conflict_total();
            if idx + 1 < total {
                app.set_active_conflict_index(idx + 1);
                sync_conflict_sheet_state(&app, &session);
            }
        }
    });

    app.on_open_conflict_note({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            open_first_conflicted_note(&app, &session);
        }
    });

    app.on_copy_sync_error({
        let weak = app.as_weak();
        move || {
            let Some(app) = weak.upgrade() else { return };
            let err = app.get_last_sync_error();
            if !err.is_empty() {
                session::copy_to_clipboard(&app, err.as_str());
                app.set_sync_error_copied(true);
                let weak_timer = weak.clone();
                slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
                    if let Some(app) = weak_timer.upgrade() {
                        app.set_sync_error_copied(false);
                    }
                });
            }
        }
    });

    app.on_dismiss_sync_error({
        let weak = app.as_weak();
        move || {
            let Some(app) = weak.upgrade() else { return };
            app.set_last_sync_error(SharedString::new());
            app.set_sync_error_copied(false);
            if app.get_status_is_error() {
                app.set_status(SharedString::new());
                app.set_status_is_error(false);
            }
        }
    });

    app.on_open_history({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            session::open_note_history(&app, &session);
        }
    });

    app.on_restore_history_version({
        let (weak, session) = (app.as_weak(), session.clone());
        move |idx| {
            let app = weak.unwrap();
            session::restore_note_version(&app, &session, idx as usize);
        }
    });

    app.on_open_vault_history({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            session::open_vault_history(&app, &session);
        }
    });
}
