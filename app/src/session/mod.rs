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
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{App, BreadcrumbSegment, DirectoryEntry, NoteBodyItemView, NotePropertyEntry};

/// What the window is currently editing. Lives on the UI thread only.
pub struct Session {
    pub core: immermemo_core::Session,
    /// `history_preview_items`'s cache of rendered history-neighbor
    /// previews, keyed by vault-relative path.
    pub history_preview_cache: HashMap<String, (f32, ModelRc<NoteBodyItemView>)>,
    /// Whether directory folder entries need rebuilding when the Directory page is viewed.
    pub directory_views_dirty: bool,
    /// Multi-selection state for notes across the app.
    pub selection: origiri_selection::Selection,
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
            directory_views_dirty: true,
            selection: origiri_selection::Selection::default(),
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

/// Loads note list from index without walking the filesystem. Returns whether any notes were cached.
pub fn load_cached_list(app: &App, session: &Rc<RefCell<Session>>) -> bool {
    let mut s = session.borrow_mut();
    let outcome = s.load_cached_list_data();
    let has_notes = !s.notes.is_empty();
    crate::widget::export_recent_notes(&s.index);
    drop(s);
    app.set_conflict_count(outcome.conflict_count as i32);
    update_filtered_list(app, session);
    has_notes
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

    session.borrow_mut().directory_views_dirty = true;
    if app.get_active_tab() == 2 && app.get_files_page() == 1 {
        refresh_directory_views(app, session);
    }
}

/// Ensures Directory page contents are refreshed if they have been marked dirty.
pub fn ensure_directory_views(app: &App, session: &Rc<RefCell<Session>>) {
    if session.borrow().directory_views_dirty {
        refresh_directory_views(app, session);
    }
}

/// Rebuilds the Directory page's List/Grid contents from `Session`'s
/// cached `directory_source` plus whichever folder is currently browsed.
pub fn refresh_directory_views(app: &App, session: &Rc<RefCell<Session>>) {
    let mut s = session.borrow_mut();
    s.directory_views_dirty = false;
    let current_folder = s.current_folder.clone();
    let source: Vec<directory::TitledNote> = s
        .directory_source
        .iter()
        .map(|(title, idx, conflict)| (title.as_str(), *idx, *conflict))
        .collect();
    let folder_entries_raw = directory::build_folder_entries(&source, &current_folder);

    // Only resolve note icon metadata for notes actually present in this folder
    let mut icon_by_path: std::collections::HashMap<PathBuf, slint::Image> =
        std::collections::HashMap::new();
    for entry in &folder_entries_raw {
        if !entry.is_folder {
            if let Ok(idx) = usize::try_from(entry.note_index) {
                if let Some(abs_path) = s.notes.get(idx) {
                    if let Ok(rel_path) = abs_path.strip_prefix(&s.vault_dir) {
                        if let Ok(Some((name, pkg))) = s.index.icon_for(rel_path) {
                            icon_by_path
                                .insert(abs_path.clone(), crate::render::icon_image(&name, &pkg));
                        }
                    }
                }
            }
        }
    }

    let folder_entries: Vec<DirectoryEntry> = folder_entries_raw
        .into_iter()
        .map(|e| to_entry_view(e, &s.notes, &icon_by_path, &s.selection))
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
    selection: &origiri_selection::Selection,
) -> DirectoryEntry {
    let (icon_image, path_str) = match usize::try_from(e.note_index)
        .ok()
        .and_then(|i| notes.get(i))
    {
        Some(path) => {
            let p_str = path.to_string_lossy().to_string();
            let icon = icon_by_path.get(path).cloned().unwrap_or_default();
            (icon, p_str)
        }
        None => (slint::Image::default(), String::new()),
    };
    let is_selected = !e.is_folder && !path_str.is_empty() && selection.is_selected(&path_str);
    DirectoryEntry {
        is_folder: e.is_folder,
        key: e.key.into(),
        display: e.display.into(),
        note_index: e.note_index,
        has_conflict: e.has_conflict,
        icon_image,
        path: path_str.into(),
        is_selected,
    }
}

/// Syncs the current `origiri_selection::Selection` count, mode, and paths to the `SelectionAdapter`
/// global in Slint and refreshes directory views.
pub fn sync_selection_to_ui(app: &App, session: &Rc<RefCell<Session>>) {
    let sel = session.borrow();
    let count = sel.selection.len() as i32;
    let mode = count > 0;
    let paths: Vec<SharedString> = sel
        .selection
        .paths()
        .into_iter()
        .map(SharedString::from)
        .collect();
    drop(sel);

    let adapter = app.global::<crate::SelectionAdapter>();
    adapter.set_selection_count(count);
    adapter.set_selection_mode(mode);
    adapter.set_selected_paths(ModelRc::new(VecModel::from(paths)));

    refresh_directory_views(app, session);
}

pub fn navigate_directory_folder(app: &App, session: &Rc<RefCell<Session>>, key: &str) {
    session.borrow_mut().current_folder = key.to_string();
    refresh_directory_views(app, session);
}

/// Sets `body` and recomputes `note-body-items` in the same step.
pub fn update_rendered_body(app: &App, text: &str) {
    app.set_body(text.into());
    let max_width = app.get_body_content_width();
    if max_width > 0.0 {
        app.set_note_body_items(crate::render::note_body_items(text, app, max_width));
    } else {
        app.set_note_body_items(ModelRc::default());
    }
}

pub fn update_note_stats(app: &App, text: &str) {
    let stats = immermemo_core::note_stats(text);
    app.set_char_count(stats.char_count);
    app.set_word_count(stats.word_count);
    app.set_line_count(stats.line_count);
    let kind = immermemo_render_tomet::extract_note_kind(text).unwrap_or_default();
    app.set_note_kind(kind.into());
    let metadata_entries: Vec<NotePropertyEntry> =
        immermemo_render_tomet::extract_note_metadata(text)
            .into_iter()
            .map(|(k, v)| NotePropertyEntry {
                key: k.into(),
                value: v.into(),
            })
            .collect();
    app.set_note_metadata(Rc::new(VecModel::from(metadata_entries)).into());
}

pub fn clear_note_stats(app: &App) {
    app.set_current_note_path(SharedString::new());
    app.set_char_count(0);
    app.set_word_count(0);
    app.set_line_count(0);
    app.set_note_kind(SharedString::new());
    app.set_note_metadata(Rc::new(VecModel::from(Vec::<NotePropertyEntry>::new())).into());
}
