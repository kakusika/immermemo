//! Note editing, creation, renaming, deleting, search, and navigation callbacks.

use std::cell::RefCell;
use std::rc::Rc;

use immermemo_core::History;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::session::{
    self, Session, apply_icon_selection, apply_restored, filter_icon_picker, open_icon_picker,
    open_note, refresh_list, set_status, show_history_state, update_filtered_list,
};
use crate::{App, render};

pub fn bind_note_callbacks(app: &App, session: &Rc<RefCell<Session>>) {
    app.on_select({
        let (weak, session) = (app.as_weak(), session.clone());
        move |ui_index| {
            let app = weak.unwrap();
            let note_index = session
                .borrow()
                .filtered_results
                .get(ui_index as usize)
                .map(|r| r.note_index);
            if let Some(index) = note_index {
                open_note(&app, &session, index);
            }
        }
    });

    app.on_new_note({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            match session.borrow_mut().create_note() {
                Ok(path) => {
                    app.set_search_query(SharedString::new());
                    refresh_list(&app, &session);
                    let index = session.borrow().notes.iter().position(|p| *p == path);
                    if let Some(index) = index {
                        open_note(&app, &session, index);
                    }
                }
                Err(e) => set_status(&app, format!("Could not create a note: {e}")),
            }
        }
    });

    app.on_navigate_directory_folder({
        let (weak, session) = (app.as_weak(), session.clone());
        move |key| {
            let app = weak.unwrap();
            session::navigate_directory_folder(&app, &session, key.as_str());
        }
    });

    app.on_edited({
        let (weak, session) = (app.as_weak(), session.clone());
        move |text| {
            let app = weak.unwrap();
            let max_width = app.get_body_content_width();
            app.set_note_body_items(render::note_body_items(text.as_str(), &app, max_width));
            session::update_note_stats(&app, text.as_str());
            {
                let mut s = session.borrow_mut();
                if let Some(history) = s.editor.history.as_mut() {
                    history.edit(&text);
                }
                show_history_state(&app, &s);
                let has_conflict = text.contains(immermemo_core::CONFLICT_MARKER);
                if app.get_current_has_conflict() != has_conflict {
                    app.set_current_has_conflict(has_conflict);
                    if let Some(idx) = s.editor.current
                        && idx < s.conflicted.len()
                    {
                        s.conflicted[idx] = has_conflict;
                        let conflicted: Vec<bool> = s
                            .filtered_results
                            .iter()
                            .map(|r| s.conflicted[r.note_index])
                            .collect();
                        app.set_conflicted(ModelRc::new(VecModel::from(conflicted)));
                        let conflict_count = s.conflicted.iter().filter(|&&c| c).count();
                        app.set_conflict_count(conflict_count as i32);
                    }
                }
                if let Some(path) = s.current_path() {
                    match std::fs::write(&path, text.as_str()) {
                        Ok(_) => {
                            let vault_dir = s.vault_dir.clone();
                            if let Ok(rel) = path.strip_prefix(&vault_dir) {
                                let _ = s.index.record_write(&vault_dir, rel, text.as_str());
                                s.history_preview_cache.remove(&rel.display().to_string());
                            }
                            if app.get_status_is_error()
                                && app.get_status().starts_with("Save failed")
                            {
                                set_status(&app, "");
                            }
                        }
                        Err(e) => set_status(&app, format!("Save failed: {e}")),
                    }
                }
            }
            session::schedule_auto_sync(&app, std::time::Duration::from_secs(5));
        }
    });

    app.on_reflow_note_body({
        let weak = app.as_weak();
        move |max_width: f32| {
            let app = weak.unwrap();
            let body = app.get_body();
            app.set_note_body_items(render::note_body_items(body.as_str(), &app, max_width));
        }
    });

    app.on_undo({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let restored = session
                .borrow_mut()
                .editor
                .history
                .as_mut()
                .and_then(History::undo);
            apply_restored(&weak.unwrap(), &session, restored);
        }
    });
    app.on_redo({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let restored = session
                .borrow_mut()
                .editor
                .history
                .as_mut()
                .and_then(History::redo);
            apply_restored(&weak.unwrap(), &session, restored);
        }
    });

    app.on_navigate_back({
        let (weak, session) = (app.as_weak(), session.clone());
        move || session::navigate_back(&weak.unwrap(), &session)
    });
    app.on_navigate_forward({
        let (weak, session) = (app.as_weak(), session.clone());
        move || session::navigate_forward(&weak.unwrap(), &session)
    });

    app.on_search_query_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |query| {
            let app = weak.unwrap();
            session.borrow_mut().search_query = query.to_string();
            update_filtered_list(&app, &session);
        }
    });

    app.on_rename_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |ui_index| {
            let app = weak.unwrap();
            let mut s = session.borrow_mut();
            let Some(res) = s.filtered_results.get(ui_index as usize) else {
                return;
            };
            let note_index = res.note_index;
            let Some(path) = s.notes.get(note_index).cloned() else {
                return;
            };
            s.rename_target = Some(note_index);
            drop(s);
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            app.set_rename_draft(stem.into());
            app.set_rename_open(true);
        }
    });
    app.on_confirm_rename({
        let (weak, session) = (app.as_weak(), session.clone());
        move |new_name| {
            let app = weak.unwrap();
            app.set_rename_open(false);
            let Some(index) = session.borrow_mut().rename_target.take() else {
                return;
            };
            let Some(path) = session.borrow().notes.get(index).cloned() else {
                return;
            };
            let was_current = session.borrow().current_path() == Some(path);
            match session.borrow_mut().rename_note(index, &new_name) {
                Ok(new_path) => {
                    refresh_list(&app, &session);
                    if was_current {
                        let index = session.borrow().notes.iter().position(|p| *p == new_path);
                        if let Some(index) = index {
                            open_note(&app, &session, index);
                        }
                    }
                }
                Err(e) => set_status(&app, format!("Could not rename the note: {e}")),
            }
        }
    });

    app.on_icon_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |ui_index| {
            let app = weak.unwrap();
            let s = session.borrow();
            let Some(res) = s.filtered_results.get(ui_index as usize) else {
                return;
            };
            let note_index = res.note_index;
            drop(s);
            open_icon_picker(&app, &session, note_index);
        }
    });
    app.on_icon_picker_query_changed({
        let weak = app.as_weak();
        move |query| {
            let app = weak.unwrap();
            filter_icon_picker(&app, &query);
        }
    });
    app.on_icon_picker_select({
        let (weak, session) = (app.as_weak(), session.clone());
        move |name| {
            let app = weak.unwrap();
            apply_icon_selection(&app, &session, &name);
        }
    });

    app.on_delete_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |ui_index| {
            let app = weak.unwrap();
            let note_index = session
                .borrow()
                .filtered_results
                .get(ui_index as usize)
                .map(|r| r.note_index);
            if let Some(index) = note_index {
                session.borrow_mut().delete_target = Some(index);
                app.set_delete_confirm_open(true);
            }
        }
    });
    app.on_confirm_delete({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            app.set_delete_confirm_open(false);
            let Some(index) = session.borrow_mut().delete_target.take() else {
                return;
            };
            let Some(path) = session.borrow().notes.get(index).cloned() else {
                return;
            };
            let was_current = session.borrow().current_path() == Some(path);
            match session.borrow_mut().delete_note(index) {
                Ok(_) => {
                    refresh_list(&app, &session);
                    if was_current {
                        session::close_current_note_and_show_list_or_reopen(&app, &session, true);
                    } else if session.borrow().notes.is_empty() {
                        session::show_list_or_reopen(&app, &session, true);
                    }
                }
                Err(e) => set_status(&app, format!("Could not delete the note: {e}")),
            }
        }
    });
}
