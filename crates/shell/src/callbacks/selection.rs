//! Callback wiring for multi-selection adapter and bulk operations.

use crate::session::SessionRef;
use std::path::PathBuf;

use slint::ComponentHandle;

use crate::session::{
    close_current_note_and_show_list_or_reopen, refresh_list, show_list_or_reopen,
    sync_selection_to_ui,
};
use crate::{App, SelectionAdapter};

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

    // Select all note items in the current view (or toggle off if already all selected)
    adapter.on_select_all_in_view({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let Some(app) = weak.upgrade() else { return };
            let mut s = session.borrow_mut();
            let current_folder = s.current_folder.clone();
            let source: Vec<immermemo_core::directory::TitledNote> = s
                .directory_source
                .iter()
                .map(|(title, idx, conflict)| (title.as_str(), *idx, *conflict))
                .collect();
            let entries = immermemo_core::directory::build_folder_entries(&source, &current_folder);

            let view_paths: Vec<String> = entries
                .into_iter()
                .filter(|e| !e.is_folder)
                .filter_map(|e| {
                    usize::try_from(e.note_index)
                        .ok()
                        .and_then(|idx| s.notes.get(idx))
                        .map(|p| p.to_string_lossy().to_string())
                })
                .collect();

            let all_selected =
                !view_paths.is_empty() && view_paths.iter().all(|p| s.selection.is_selected(p));
            if all_selected {
                for p in &view_paths {
                    s.selection.toggle(p);
                }
            } else {
                s.selection.select_all(view_paths);
            }
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
