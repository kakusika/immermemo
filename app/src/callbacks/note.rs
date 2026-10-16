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

    app.on_action_executed({
        let _session = session.clone();
        move |_id| {
            // Action executed hook
        }
    });

    app.on_navigate_directory_folder({
        let (weak, session) = (app.as_weak(), session.clone());
        move |key| {
            let app = weak.unwrap();
            session::navigate_directory_folder(&app, &session, key.as_str());
        }
    });

    app.on_ensure_directory_views({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            session::ensure_directory_views(&app, &session);
        }
    });

    app.on_edited({
        let (weak, session) = (app.as_weak(), session.clone());
        move |text| {
            let app = weak.unwrap();
            handle_edit(&app, &session, text.as_str());
        }
    });

    app.on_insert_symbol({
        let (weak, session) = (app.as_weak(), session.clone());
        move |sym, cursor_offset, anchor_offset| {
            let app = weak.unwrap();
            let body = app.get_body();
            let c_pos = if cursor_offset < 0 {
                body.len()
            } else {
                (cursor_offset as usize).min(body.len())
            };
            let a_pos = if anchor_offset < 0 {
                body.len()
            } else {
                (anchor_offset as usize).min(body.len())
            };

            let (new_body, new_cursor) =
                apply_symbol_insertion(body.as_str(), sym.as_str(), c_pos, a_pos);
            app.set_body(new_body.clone().into());
            handle_edit(&app, &session, &new_body);
            app.invoke_set_cursor(new_cursor as i32);
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

    app.on_link_clicked({
        let (weak, session) = (app.as_weak(), session.clone());
        move |target| {
            let app = weak.unwrap();
            handle_link_clicked(&app, &session, target.as_str());
        }
    });

    app.on_task_toggled({
        let (weak, session) = (app.as_weak(), session.clone());
        move |item_idx| {
            let app = weak.unwrap();
            handle_task_toggled(&app, &session, item_idx as usize);
        }
    });
}

fn clamp_to_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn apply_symbol_insertion(body: &str, sym: &str, cursor: usize, anchor: usize) -> (String, usize) {
    let c = clamp_to_char_boundary(body, cursor);
    let a = clamp_to_char_boundary(body, anchor);
    let (sel_start, sel_end) = if c <= a { (c, a) } else { (a, c) };

    if sym == "- " || sym == "= " || sym == "# " {
        // Line-based insertion: insert at the beginning of the line containing sel_start
        let line_start = body[..sel_start].rfind('\n').map_or(0, |idx| idx + 1);
        let mut new_body = String::with_capacity(body.len() + sym.len());
        new_body.push_str(&body[..line_start]);
        new_body.push_str(sym);
        new_body.push_str(&body[line_start..]);
        let new_cursor = c + sym.len();
        (new_body, new_cursor)
    } else {
        // Inline insertion: insert at cursor or replace selection
        let mut new_body = String::with_capacity(body.len() + sym.len());
        new_body.push_str(&body[..sel_start]);
        new_body.push_str(sym);
        new_body.push_str(&body[sel_end..]);
        let new_cursor = sel_start + sym.len();
        (new_body, new_cursor)
    }
}

fn handle_edit(app: &App, session: &std::rc::Rc<std::cell::RefCell<Session>>, text: &str) {
    let max_width = app.get_body_content_width();
    app.set_note_body_items(render::note_body_items(text, app, max_width));
    session::update_note_stats(app, text);
    {
        let mut s = session.borrow_mut();
        if let Some(history) = s.editor.history.as_mut() {
            history.edit(text);
        }
        show_history_state(app, &s);
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
            match std::fs::write(&path, text) {
                Ok(_) => {
                    let vault_dir = s.vault_dir.clone();
                    if let Ok(rel) = path.strip_prefix(&vault_dir) {
                        let _ = s.index.record_write(&vault_dir, rel, text);
                        s.history_preview_cache.remove(&rel.display().to_string());
                    }
                    if app.get_status_is_error() && app.get_status().starts_with("Save failed") {
                        set_status(app, "");
                    }
                }
                Err(e) => set_status(app, format!("Save failed: {e}")),
            }
        }
    }
    session::schedule_auto_sync(app, std::time::Duration::from_secs(5));
}

fn handle_link_clicked(app: &App, session: &std::rc::Rc<std::cell::RefCell<Session>>, target: &str) {
    let target = target.trim();
    if target.is_empty() {
        return;
    }

    let (scheme, rest) = tomet::semantics::target_scheme(target);
    match scheme {
        tomet::semantics::TargetScheme::Url => {
            let url = if rest.starts_with("http://") || rest.starts_with("https://") {
                rest
            } else if target.starts_with("http://") || target.starts_with("https://") {
                target
            } else {
                rest
            };
            if let Err(e) = webbrowser::open(url) {
                set_status(app, format!("Failed to open link: {e}"));
            }
        }
        tomet::semantics::TargetScheme::Ref => {
            let note_query = rest.split('#').next().unwrap_or(rest).trim();
            if !navigate_to_note(app, session, note_query) {
                create_and_open_linked_note(app, session, note_query);
            }
        }
        tomet::semantics::TargetScheme::Document
        | tomet::semantics::TargetScheme::File
        | tomet::semantics::TargetScheme::Internal
        | tomet::semantics::TargetScheme::Unresolved
        | tomet::semantics::TargetScheme::Dir => {
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
            {
                if let Err(e) = webbrowser::open(target) {
                    set_status(app, format!("Failed to open link: {e}"));
                }
                return;
            }
            let note_query = rest.split('#').next().unwrap_or(rest).trim();
            if !navigate_to_note(app, session, note_query) {
                create_and_open_linked_note(app, session, note_query);
            }
        }
    }
}

fn navigate_to_note(app: &App, session: &std::rc::Rc<std::cell::RefCell<Session>>, query: &str) -> bool {
    let s = session.borrow();
    let vault_dir = s.vault_dir.clone();
    let query_trimmed = query.trim_start_matches('@');
    let query_path = std::path::Path::new(query_trimmed);

    // 1. Exact path match or relative path match
    for (i, note_path) in s.notes.iter().enumerate() {
        if let Ok(rel) = note_path.strip_prefix(&vault_dir) {
            if rel == query_path
                || rel.with_extension("") == query_path
                || rel.to_string_lossy() == query
                || rel.to_string_lossy() == query_trimmed
                || rel.with_extension("").to_string_lossy() == query
                || rel.with_extension("").to_string_lossy() == query_trimmed
            {
                drop(s);
                open_note(app, session, i);
                return true;
            }
        }
    }

    // 2. File stem match (case-insensitive)
    for (i, note_path) in s.notes.iter().enumerate() {
        if let Some(stem) = note_path.file_stem().and_then(|st| st.to_str()) {
            if stem.eq_ignore_ascii_case(query)
                || stem.eq_ignore_ascii_case(query_trimmed)
                || stem.trim_start_matches('@').eq_ignore_ascii_case(query_trimmed)
            {
                drop(s);
                open_note(app, session, i);
                return true;
            }
        }
    }

    false
}

fn create_and_open_linked_note(
    app: &App,
    session: &std::rc::Rc<std::cell::RefCell<Session>>,
    note_name: &str,
) {
    let clean_name = note_name.trim().trim_start_matches('@').trim_end_matches(".tmt");
    if clean_name.is_empty() {
        return;
    }
    let initial_content = format!("= {clean_name}\n\n");
    match session.borrow_mut().create_named_note(clean_name, &initial_content) {
        Ok(path) => {
            app.set_search_query(slint::SharedString::new());
            refresh_list(app, session);
            let index = session.borrow().notes.iter().position(|p| *p == path);
            if let Some(index) = index {
                open_note(app, session, index);
                set_status(app, format!("Created note \"{clean_name}\""));
            }
        }
        Err(e) => set_status(app, format!("Could not create note: {e}")),
    }
}

fn handle_task_toggled(
    app: &App,
    session: &std::rc::Rc<std::cell::RefCell<Session>>,
    item_idx: usize,
) {
    let body = app.get_body();
    let items = render::classify_note_body_items(body.as_str());
    let Some(item) = items.get(item_idx) else {
        return;
    };
    let Some(span) = item.source_span else {
        return;
    };

    if let Some(new_body) = toggle_task_in_text(body.as_str(), span.start.offset) {
        app.set_body(new_body.clone().into());
        handle_edit(app, session, &new_body);
    }
}

pub fn toggle_task_in_text(body: &str, span_offset: usize) -> Option<String> {
    let offset = clamp_to_char_boundary(body, span_offset);
    let line_start = body[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line_end = body[offset..].find('\n').map_or(body.len(), |i| offset + i);
    let line = &body[line_start..line_end];

    let toggled_line = if line.contains("@task(x)") {
        line.replacen("@task(x)", "@task()", 1)
    } else if line.contains("@task(done)") {
        line.replacen("@task(done)", "@task()", 1)
    } else if line.contains("@task()") {
        line.replacen("@task()", "@task(x)", 1)
    } else if line.contains("- (x)") {
        line.replacen("- (x)", "- ()", 1)
    } else if line.contains("- ()") {
        line.replacen("- ()", "- (x)", 1)
    } else if line.contains("- [x]") {
        line.replacen("- [x]", "- [ ]", 1)
    } else if line.contains("- [ ]") {
        line.replacen("- [ ]", "- [x]", 1)
    } else {
        return None;
    };

    let mut result = String::with_capacity(body.len() + 2);
    result.push_str(&body[..line_start]);
    result.push_str(&toggled_line);
    result.push_str(&body[line_end..]);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_symbol_inserts_at_line_start() {
        let body = "Hello world";
        let (new_body, cursor) = apply_symbol_insertion(body, "= ", 5, 5);
        assert_eq!(new_body, "= Hello world");
        assert_eq!(cursor, 7);
    }

    #[test]
    fn list_symbol_inserts_at_line_start() {
        let body = "First line\nSecond line";
        let (new_body, cursor) = apply_symbol_insertion(body, "- ", 15, 15);
        assert_eq!(new_body, "First line\n- Second line");
        assert_eq!(cursor, 17);
    }

    #[test]
    fn inline_symbol_inserts_at_cursor() {
        let body = "Hello world";
        let (new_body, cursor) = apply_symbol_insertion(body, "@", 5, 5);
        assert_eq!(new_body, "Hello@ world");
        assert_eq!(cursor, 6);
    }

    #[test]
    fn toggle_task_in_text_flips_states() {
        // Bare element
        let src = "@task()[Open task]\n";
        let toggled = toggle_task_in_text(src, 3).unwrap();
        assert_eq!(toggled, "@task(x)[Open task]\n");
        let back = toggle_task_in_text(&toggled, 3).unwrap();
        assert_eq!(back, "@task()[Open task]\n");

        // Combine list notation
        let src = "-@task(x) Completed item\n-@task() Pending item\n";
        let offset_item2 = src.find("-@task()").unwrap();
        let toggled = toggle_task_in_text(src, offset_item2).unwrap();
        assert_eq!(
            toggled,
            "-@task(x) Completed item\n-@task(x) Pending item\n"
        );

        // First item toggled
        let toggled_first = toggle_task_in_text(&toggled, 0).unwrap();
        assert_eq!(
            toggled_first,
            "-@task() Completed item\n-@task(x) Pending item\n"
        );

        // Value notation
        let src_val = "- (x) Done\n- () Todo\n";
        let offset_todo = src_val.find("- ()").unwrap();
        let toggled_val = toggle_task_in_text(src_val, offset_todo).unwrap();
        assert_eq!(toggled_val, "- (x) Done\n- (x) Todo\n");
    }
}
