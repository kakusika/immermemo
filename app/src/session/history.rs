//! History sheet views: note revisions, vault commits, and history previews.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use immermemo_core::{History, format_timestamp, notes};
use slint::{ModelRc, SharedString, VecModel};

use super::util::set_status;
use super::{Session, schedule_auto_sync, update_note_stats, update_rendered_body};
use crate::{App, NoteBodyItemView};

/// Title + real rendered body items for `rel_path`'s history-neighbor preview.
pub fn history_preview(
    app: &App,
    session: &Rc<RefCell<Session>>,
    vault_dir: &Path,
    rel_path: &str,
    max_width: f32,
) -> (String, ModelRc<NoteBodyItemView>) {
    let abs_path = vault_dir.join(rel_path);
    let title = notes::display_name(vault_dir, &abs_path);
    if let Some((cached_width, items)) = session.borrow().history_preview_cache.get(rel_path)
        && *cached_width == max_width
    {
        return (title, items.clone());
    }
    let Ok(text) = std::fs::read_to_string(&abs_path) else {
        return (String::new(), ModelRc::default());
    };
    let items = crate::render::note_body_items(&text, app, max_width);
    session
        .borrow_mut()
        .history_preview_cache
        .insert(rel_path.to_string(), (max_width, items.clone()));
    (title, items)
}

/// Tells the window whether the undo and redo buttons have anything to do.
pub fn show_history_state(app: &App, session: &Session) {
    let history = session.editor.history.as_ref();
    app.set_can_undo(history.is_some_and(History::can_undo));
    app.set_can_redo(history.is_some_and(History::can_redo));
}

pub fn apply_restored(
    app: &App,
    session: &Rc<RefCell<Session>>,
    restored: Option<immermemo_core::Restored>,
) {
    let Some(restored) = restored else {
        return;
    };
    let result = session.borrow_mut().apply_restored(restored);
    match &result.write_error {
        Some(e) => set_status(app, e.clone()),
        None => {
            if app.get_status_is_error() && app.get_status().starts_with("Save failed") {
                set_status(app, "");
            }
        }
    }
    app.set_current_has_conflict(result.has_conflict);
    {
        let s = session.borrow();
        if let Some(idx) = s.editor.current
            && idx < s.conflicted.len()
        {
            app.set_conflicted(ModelRc::new(VecModel::from(s.conflicted.clone())));
        }
    }
    update_rendered_body(app, &result.body);
    update_note_stats(app, &result.body);
    app.invoke_set_cursor(result.cursor as i32);
    show_history_state(app, &session.borrow());
}

pub fn open_note_history(app: &App, session: &Rc<RefCell<Session>>) {
    let revisions = session.borrow_mut().load_note_history(50);
    let Some(revisions) = revisions else { return };

    let mut commit_ids = Vec::with_capacity(revisions.len());
    let mut dates = Vec::with_capacity(revisions.len());
    let mut previews = Vec::with_capacity(revisions.len());

    for rev in &revisions {
        commit_ids.push(SharedString::from(format!(
            "{} - {}",
            rev.short_id, rev.summary
        )));
        dates.push(SharedString::from(format_timestamp(rev.timestamp_secs)));
        previews.push(SharedString::from(&rev.content));
    }

    app.set_history_commits(ModelRc::new(VecModel::from(commit_ids)));
    app.set_history_dates(ModelRc::new(VecModel::from(dates)));
    app.set_history_previews(ModelRc::new(VecModel::from(previews)));
    app.set_history_selected_index(0);
    app.set_history_sheet_open(true);
}

pub fn restore_note_version(app: &App, session: &Rc<RefCell<Session>>, rev_idx: usize) {
    let outcome = session.borrow_mut().restore_note_version(rev_idx);
    let Some(outcome) = outcome else { return };
    match outcome {
        Err(msg) => set_status(app, msg),
        Ok(restored) => {
            update_rendered_body(app, &restored.body);
            app.set_can_undo(restored.can_undo);
            app.set_can_redo(restored.can_redo);
            update_note_stats(app, &restored.body);
            app.set_history_sheet_open(false);
            set_status(app, "Restored note to selected revision");
            schedule_auto_sync(app, std::time::Duration::from_secs(5));
        }
    }
}

pub fn open_vault_history(app: &App, session: &Rc<RefCell<Session>>) {
    let commits = session.borrow().vault_history(50);

    let mut commit_summaries = Vec::with_capacity(commits.len());
    let mut dates = Vec::with_capacity(commits.len());

    for c in commits {
        commit_summaries.push(SharedString::from(format!(
            "{} - {}",
            c.short_id, c.summary
        )));
        dates.push(SharedString::from(format_timestamp(c.timestamp_secs)));
    }

    app.set_vault_history_commits(ModelRc::new(VecModel::from(commit_summaries)));
    app.set_vault_history_dates(ModelRc::new(VecModel::from(dates)));
    app.set_vault_history_open(true);
}
