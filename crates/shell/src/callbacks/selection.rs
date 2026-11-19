//! Callback wiring for multi-selection adapter and bulk operations.

use crate::session::SessionRef;
use std::path::PathBuf;

use slint::ComponentHandle;

use crate::session::{
    build_history_entries, build_recent_entries, build_starred_entries,
    close_current_note_and_show_list_or_reopen, descendant_note_paths, refresh_list,
    show_list_or_reopen, sync_selection_to_ui,
};
use crate::{App, SelectionAdapter};

/// If all `paths` are already selected, deselects them; otherwise selects all of them.
fn toggle_paths_group(selection: &mut origiri_selection::Selection, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let all_selected = paths.iter().all(|p| selection.is_selected(p));
    if all_selected {
        for p in &paths {
            selection.toggle(p);
        }
    } else {
        selection.select_all(paths);
    }
}

/// Binds UI callbacks for [`SelectionAdapter`].
pub fn bind_selection_callbacks(app: &App, session: &SessionRef) {
    let adapter = app.global::<SelectionAdapter>();

    // Toggle a note's selection by path
    adapter.on_toggle({
        let (weak, session) = (app.as_weak(), session.clone());
        move |path| {
            let Some(app) = weak.upgrade() else { return };
            session.borrow_mut().selection.toggle(path.as_str());
            sync_selection_to_ui(&app, &session);
        }
    });

    // Select only this note (e.g. entering selection mode via long-press)
    adapter.on_select_only({
        let (weak, session) = (app.as_weak(), session.clone());
        move |path| {
            let Some(app) = weak.upgrade() else { return };
            session.borrow_mut().selection.select_only(path.as_str());
            sync_selection_to_ui(&app, &session);
        }
    });

    // Toggle all descendant notes of a folder by folder key
    adapter.on_toggle_folder({
        let (weak, session) = (app.as_weak(), session.clone());
        move |key| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = session.borrow_mut();
            let paths = descendant_note_paths(&s.directory_source, &s.notes, key.as_str());
            toggle_paths_group(&mut s.selection, paths);
            drop(s);
            sync_selection_to_ui(&app, &session);
        }
    });

    // Clear selection and select all descendant notes of a folder by folder key
    adapter.on_select_only_folder({
        let (weak, session) = (app.as_weak(), session.clone());
        move |key| {
            let Some(app) = weak.upgrade() else { return };
            let mut s = session.borrow_mut();
            let paths = descendant_note_paths(&s.directory_source, &s.notes, key.as_str());
            s.selection.clear();
            if !paths.is_empty() {
                s.selection.select_all(paths);
            }
            drop(s);
            sync_selection_to_ui(&app, &session);
        }
    });

    // Select all note items in the current view (or toggle off if already all selected)
    adapter.on_select_all_in_view({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let Some(app) = weak.upgrade() else { return };
            let files_page = app.get_files_page();
            let mut s = session.borrow_mut();
            let view_paths: Vec<String> = match files_page {
                3 => build_starred_entries(&s)
                    .into_iter()
                    .map(|e| e.path.to_string())
                    .collect(),
                4 => build_recent_entries(&s)
                    .into_iter()
                    .map(|e| e.path.to_string())
                    .collect(),
                8 => build_history_entries(&s)
                    .into_iter()
                    .map(|e| e.path.to_string())
                    .collect(),
                _ => {
                    let current_folder = s.current_folder.clone();
                    descendant_note_paths(&s.directory_source, &s.notes, &current_folder)
                }
            };

            toggle_paths_group(&mut s.selection, view_paths);
            drop(s);
            sync_selection_to_ui(&app, &session);
        }
    });

    // Clear selection and exit selection mode
    adapter.on_clear({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let Some(app) = weak.upgrade() else { return };
            session.borrow_mut().selection.clear();
            sync_selection_to_ui(&app, &session);
        }
    });

    // Delete all selected notes in bulk
    adapter.on_delete_selected({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let Some(app) = weak.upgrade() else { return };
            let mut s = session.borrow_mut();
            let selected_paths = s.selection.paths();
            if selected_paths.is_empty() {
                return;
            }

            let current_path = s.current_path().map(|p| p.to_string_lossy().to_string());
            let current_deleted = current_path
                .as_ref()
                .is_some_and(|cp| selected_paths.contains(cp));

            for path_str in &selected_paths {
                let p = PathBuf::from(path_str);
                let _ = immermemo_core::notes::delete(&p);
                if let Ok(rel) = p.strip_prefix(&s.vault_dir) {
                    let _ = s.index.record_delete(rel);
                }
            }

            s.selection.clear();
            s.directory_views_dirty = true;
            let _ = s.refresh_list_data();
            let _ = s.update_filtered_list_data();
            drop(s);

            refresh_list(&app, &session);
            sync_selection_to_ui(&app, &session);

            if current_deleted {
                close_current_note_and_show_list_or_reopen(&app, &session, true);
            } else if session.borrow().notes.is_empty() {
                show_list_or_reopen(&app, &session, true);
            }
        }
    });

    // Action menu / custom action trigger placeholder
    adapter.on_request_action({
        let weak = app.as_weak();
        move || {
            let Some(app) = weak.upgrade() else { return };
            app.set_delete_selected_confirm_open(true);
        }
    });
}
