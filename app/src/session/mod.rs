//! UI session state management and application action handlers.

pub mod conflict;
pub mod history;
pub mod icon_picker;
pub mod navigation;
pub mod sync_ui;
pub mod util;
pub mod vault;

pub use conflict::*;
pub use history::*;
pub use icon_picker::*;
pub use navigation::*;
pub use sync_ui::*;
pub use util::*;
pub use vault::*;

pub use immermemo_core::vault_display_name;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use immermemo_core::directory;
use slint::{ModelRc, SharedString, VecModel};

use crate::{App, BreadcrumbSegment, DirectoryEntry, NoteBodyItemView};

/// What the window is currently editing. Lives on the UI thread only.
pub struct Session {
    pub core: immermemo_core::Session,
    /// `history_preview_items`'s cache of rendered history-neighbor
    /// previews, keyed by vault-relative path.
    pub history_preview_cache: HashMap<String, (f32, ModelRc<NoteBodyItemView>)>,
}

impl std::ops::Deref for Session {
    type Target = immermemo_core::Session;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl std::ops::DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl Session {
    pub fn new(core: immermemo_core::Session) -> Self {
        Self {
            core,
            history_preview_cache: HashMap::new(),
        }
    }
}

thread_local! {
    // `invoke_from_event_loop` needs a `Send` closure, so a finished sync
    // finds the session again here rather than carrying an `Rc` across.
    pub static SESSION: RefCell<Option<Rc<RefCell<Session>>>> = const { RefCell::new(None) };
}

pub fn refresh_list(app: &App, session: &Rc<RefCell<Session>>) {
    let mut s = session.borrow_mut();
    let outcome = s.refresh_list_data();
    crate::widget::export_recent_notes(&s.index);
    drop(s);
    app.set_conflict_count(outcome.conflict_count as i32);
    update_filtered_list(app, session);
}

pub fn update_filtered_list(app: &App, session: &Rc<RefCell<Session>>) {
    let outcome = session.borrow_mut().update_filtered_list_data();
    let names: Vec<SharedString> = outcome.names.into_iter().map(SharedString::from).collect();
    let snippets: Vec<SharedString> = outcome
        .snippets
        .into_iter()
        .map(SharedString::from)
        .collect();

    app.set_notes(ModelRc::new(VecModel::from(names)));
    app.set_conflicted(ModelRc::new(VecModel::from(outcome.conflicted)));
    app.set_snippets(ModelRc::new(VecModel::from(snippets)));
    app.set_current(outcome.ui_current.map_or(-1, |i| i as i32));
    refresh_directory_views(app, session);
}

/// Rebuilds the Directory page's List/Grid contents from `Session`'s
/// cached `directory_source` plus whichever folder is currently browsed.
pub fn refresh_directory_views(app: &App, session: &Rc<RefCell<Session>>) {
    let s = session.borrow();
    let source: Vec<directory::TitledNote> = s
        .directory_source
        .iter()
        .map(|(title, idx, conflict)| (title.as_str(), *idx, *conflict))
        .collect();
    let icon_by_path: std::collections::HashMap<PathBuf, slint::Image> = s
        .index
        .bodies()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(rel_path, body)| {
            let (name, pkg) = crate::render::classify::note_icon(&body)?;
            Some((
                s.vault_dir.join(rel_path),
                crate::render::icon_image(&name, &pkg),
            ))
        })
        .collect();
    let folder_entries: Vec<DirectoryEntry> =
        directory::build_folder_entries(&source, &s.current_folder)
            .into_iter()
            .map(|e| to_entry_view(e, &s.notes, &icon_by_path))
            .collect();
    let breadcrumb: Vec<BreadcrumbSegment> = directory::build_breadcrumb(&s.current_folder)
        .into_iter()
        .map(|seg| BreadcrumbSegment {
            key: seg.key.into(),
            display: seg.display.into(),
        })
        .collect();
    drop(s);
    app.set_directory_folder_entries(ModelRc::new(VecModel::from(folder_entries)));
    app.set_directory_breadcrumb(ModelRc::new(VecModel::from(breadcrumb)));
}

fn to_entry_view(
    e: directory::DirectoryEntry,
    notes: &[PathBuf],
    icon_by_path: &std::collections::HashMap<PathBuf, slint::Image>,
) -> DirectoryEntry {
    let icon_image = usize::try_from(e.note_index)
        .ok()
        .and_then(|i| notes.get(i))
        .and_then(|path| icon_by_path.get(path))
        .cloned()
        .unwrap_or_default();
    DirectoryEntry {
        is_folder: e.is_folder,
        key: e.key.into(),
        display: e.display.into(),
        note_index: e.note_index,
        has_conflict: e.has_conflict,
        icon_image,
    }
}

pub fn navigate_directory_folder(app: &App, session: &Rc<RefCell<Session>>, key: &str) {
    session.borrow_mut().current_folder = key.to_string();
    refresh_directory_views(app, session);
}

/// Sets `body` and recomputes `note-body-items` in the same step.
pub fn update_rendered_body(app: &App, text: &str) {
    app.set_body(text.into());
    let max_width = app.get_body_content_width();
    app.set_note_body_items(crate::render::note_body_items(text, app, max_width));
}

pub fn update_note_stats(app: &App, text: &str) {
    let stats = immermemo_editor::note_stats(text);
    app.set_char_count(stats.char_count);
    app.set_word_count(stats.word_count);
    app.set_line_count(stats.line_count);
}

pub fn clear_note_stats(app: &App) {
    app.set_current_note_path(SharedString::new());
    app.set_char_count(0);
    app.set_word_count(0);
    app.set_line_count(0);
}
