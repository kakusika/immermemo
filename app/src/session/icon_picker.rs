//! Icon picker sheet management: query filtering and `@meta{ icon: ... }` note decoration.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};

use crate::{App, PickerIconEntry};
use super::util::set_status;
use super::{Session, refresh_list, schedule_auto_sync};

const ICON_PICKER_MAX_RESULTS: usize = 60;
const ICON_PICKER_DEFAULT_PKG: &str = "tabler";

/// Opens the icon picker for `notes[note_index]`. Resolves an initial (query-less) page of
/// results so the sheet never opens empty.
pub fn open_icon_picker(app: &App, session: &Rc<RefCell<Session>>, note_index: usize) {
    session.borrow_mut().icon_picker_target = Some(note_index);
    app.set_icon_picker_query(SharedString::new());
    filter_icon_picker(app, "");
    app.set_icon_picker_open(true);
}

/// Filters `origiri_icons::icon_names(ICON_PICKER_DEFAULT_PKG)` by `query`
/// (substring, case-insensitive), resolves up to `ICON_PICKER_MAX_RESULTS`
/// matches into real images, and sets `app.icon-picker-results`.
pub fn filter_icon_picker(app: &App, query: &str) {
    let query = query.to_lowercase();
    let results: Vec<PickerIconEntry> = origiri_icons::icon_names(ICON_PICKER_DEFAULT_PKG)
        .iter()
        .filter(|name| name.contains(&query))
        .take(ICON_PICKER_MAX_RESULTS)
        .map(|&name| PickerIconEntry {
            name: name.into(),
            image: crate::render::icon_image(name, ICON_PICKER_DEFAULT_PKG),
        })
        .collect();
    app.set_icon_picker_results(ModelRc::new(VecModel::from(results)));
}

/// Sets `notes[icon_picker_target]`'s `@meta{ icon: @doc.icon(name, pkg) }`
/// and saves. Refreshes the editor's visible body too, if the target note happens
/// to be currently open.
pub fn apply_icon_selection(app: &App, session: &Rc<RefCell<Session>>, name: &str) {
    let Some(note_index) = session.borrow_mut().icon_picker_target.take() else {
        return;
    };
    let Some(path) = session.borrow().notes.get(note_index).cloned() else {
        return;
    };
    let Ok(body) = std::fs::read_to_string(&path) else {
        set_status(app, "Could not read this note");
        return;
    };
    let Some(new_body) = immermemo_editor::set_note_icon(&body, name, ICON_PICKER_DEFAULT_PKG)
    else {
        set_status(app, "Could not update this note's icon");
        return;
    };
    match std::fs::write(&path, &new_body) {
        Ok(()) => {
            let is_current = {
                let mut s = session.borrow_mut();
                let vault_dir = s.vault_dir.clone();
                if let Ok(rel) = path.strip_prefix(&vault_dir) {
                    let _ = s.index.record_write(&vault_dir, rel, &new_body);
                }
                s.current_path() == Some(path.clone())
            };
            refresh_list(app, session);
            if is_current {
                let max_width = app.get_body_content_width();
                app.set_body(new_body.clone().into());
                app.set_note_body_items(crate::render::note_body_items(&new_body, app, max_width));
            }
            schedule_auto_sync(app, std::time::Duration::from_secs(5));
        }
        Err(e) => set_status(app, format!("Could not save this note's icon: {e}")),
    }
}
