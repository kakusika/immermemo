//! UI session state management and application action handlers.

pub mod conflict;
pub mod history;
pub mod icon_picker;
pub mod library;
pub mod navigation;
pub mod sync_ui;
pub mod util;
pub mod vault;

pub use conflict::*;
pub use history::*;
pub use icon_picker::*;
pub use library::*;
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

use crate::{
    App, BreadcrumbSegment, DirectoryEntry, NoteBodyItemView, NoteIconView, NotePropertyEntry,
    SelectionCheckState,
};

/// What the window is currently editing. Lives on the UI thread only.
pub struct Session {
    pub core: immermemo_core::Session,
    /// `history_preview_items`'s cache of rendered history-neighbor
    /// previews, keyed by vault-relative path.
    pub history_preview_cache: HashMap<String, (f32, ModelRc<NoteBodyItemView>)>,
    /// Whether directory folder entries need rebuilding when the Directory page is viewed.
    pub directory_views_dirty: bool,
    /// Whether Starred/Recent/History entries need rebuilding when a Library view is opened.
    pub library_views_dirty: bool,
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
            library_views_dirty: true,
            selection: origiri_selection::Selection::default(),
        }
    }
}

/// The UI thread's shared handle to the session, as every callback holds it.
pub type SessionRef = Rc<RefCell<Session>>;

thread_local! {
    // `invoke_from_event_loop` needs a `Send` closure, so a finished sync
    // finds the session again here rather than carrying an `Rc` across.
    pub static SESSION: RefCell<Option<SessionRef>> = const { RefCell::new(None) };
}

pub fn refresh_list(app: &App, session: &Rc<RefCell<Session>>) {
    let mut s = session.borrow_mut();
    let outcome = s.refresh_list_data();
    crate::platform::widget::export_recent_notes(&s.index);
    drop(s);
    app.set_conflict_count(outcome.conflict_count as i32);
    update_filtered_list(app, session);
}

/// Loads note list from index without walking the filesystem. Returns whether any notes were cached.
pub fn load_cached_list(app: &App, session: &Rc<RefCell<Session>>) -> bool {
    let mut s = session.borrow_mut();
    let outcome = s.load_cached_list_data();
    let has_notes = !s.notes.is_empty();
    crate::platform::widget::export_recent_notes(&s.index);
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

    let vault_dir = session.borrow().vault_dir.clone();
    let bytes = immermemo_core::compute_vault_size(&vault_dir);
    app.set_vault_size(immermemo_core::format_vault_size(bytes).into());

    session.borrow_mut().directory_views_dirty = true;
    if app.get_active_tab() == 2 && app.get_files_page() == 1 {
        refresh_directory_views(app, session);
    }
    mark_library_views_dirty(app, session);
}

/// Resolves the `NoteIconView` for a note at `abs_path` using the index and thumbnail cache.
pub(crate) fn resolve_note_icon(
    s: &Session,
    abs_path: &std::path::Path,
    lfs: Option<&immermemo_media::LfsRemote>,
) -> NoteIconView {
    let Ok(rel_path) = abs_path.strip_prefix(&s.vault_dir) else {
        return NoteIconView::default();
    };
    let Ok(Some(icon)) = s.index.icon_for(rel_path) else {
        return NoteIconView::default();
    };
    let base = crate::thumbs::MediaBase::new(&s.vault_dir, rel_path).with_lfs(lfs.cloned());
    crate::note_icon::resolve_view(Some(&icon), &base)
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
    let lfs = lfs_remote(&s);
    let mut icon_by_path: std::collections::HashMap<PathBuf, NoteIconView> =
        std::collections::HashMap::new();
    for entry in &folder_entries_raw {
        if !entry.is_folder
            && let Ok(idx) = usize::try_from(entry.note_index)
            && let Some(abs_path) = s.notes.get(idx)
        {
            let resolved = resolve_note_icon(&s, abs_path, lfs.as_ref());
            if resolved != NoteIconView::default() {
                icon_by_path.insert(abs_path.clone(), resolved);
            }
        }
    }

    let folder_entries: Vec<DirectoryEntry> = folder_entries_raw
        .into_iter()
        .map(|e| to_entry_view(e, &s.directory_source, &s.notes, &icon_by_path, &s.selection))
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

/// Collects the full path strings of all notes in `directory_source` that live
/// under `folder_key` (including nested subfolders). When `folder_key` is empty,
/// returns all notes in `directory_source`.
pub(crate) fn descendant_note_paths(
    directory_source: &[(String, i32, bool)],
    notes: &[PathBuf],
    folder_key: &str,
) -> Vec<String> {
    let prefix = if folder_key.is_empty() {
        String::new()
    } else {
        format!("{folder_key}/")
    };
    directory_source
        .iter()
        .filter(|(title, _, _)| title.starts_with(&prefix))
        .filter_map(|(_, note_index, _)| {
            usize::try_from(*note_index)
                .ok()
                .and_then(|idx| notes.get(idx))
                .map(|p| p.to_string_lossy().to_string())
        })
        .collect()
}

/// Derives the 3-state selection status for a folder from all of its descendant notes.
pub(crate) fn derive_folder_selection_state(
    directory_source: &[(String, i32, bool)],
    notes: &[PathBuf],
    folder_key: &str,
    selection: &origiri_selection::Selection,
) -> SelectionCheckState {
    let prefix = if folder_key.is_empty() {
        String::new()
    } else {
        format!("{folder_key}/")
    };
    let mut total = 0usize;
    let mut selected = 0usize;
    for (title, note_index, _) in directory_source {
        if !title.starts_with(&prefix) {
            continue;
        }
        let Some(path) = usize::try_from(*note_index)
            .ok()
            .and_then(|idx| notes.get(idx))
        else {
            continue;
        };
        total += 1;
        let path_str = path.to_string_lossy();
        if selection.is_selected(path_str.as_ref()) {
            selected += 1;
        }
    }
    if total == 0 || selected == 0 {
        SelectionCheckState::None
    } else if selected == total {
        SelectionCheckState::All
    } else {
        SelectionCheckState::Partial
    }
}

fn to_entry_view(
    e: directory::DirectoryEntry,
    directory_source: &[(String, i32, bool)],
    notes: &[PathBuf],
    icon_by_path: &std::collections::HashMap<PathBuf, NoteIconView>,
    selection: &origiri_selection::Selection,
) -> DirectoryEntry {
    let (icon, path_str) = match usize::try_from(e.note_index)
        .ok()
        .and_then(|i| notes.get(i))
    {
        Some(path) => {
            let p_str = path.to_string_lossy().to_string();
            let icon = icon_by_path.get(path).cloned().unwrap_or_default();
            (icon, p_str)
        }
        None => (NoteIconView::default(), String::new()),
    };
    let selection_state = if e.is_folder {
        derive_folder_selection_state(directory_source, notes, &e.key, selection)
    } else if !path_str.is_empty() && selection.is_selected(&path_str) {
        SelectionCheckState::All
    } else {
        SelectionCheckState::None
    };
    DirectoryEntry {
        is_folder: e.is_folder,
        key: e.key.into(),
        display: e.display.into(),
        note_index: e.note_index,
        has_conflict: e.has_conflict,
        icon,
        path: path_str.into(),
        selection_state,
    }
}

/// Syncs the current `origiri_selection::Selection` count, mode, and paths to the `SelectionAdapter`
/// global in Slint and refreshes directory and library views.
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
    mark_library_views_dirty(app, session);
}

pub fn navigate_directory_folder(app: &App, session: &Rc<RefCell<Session>>, key: &str) {
    session.borrow_mut().current_folder = key.to_string();
    refresh_directory_views(app, session);
}

/// The Git LFS credentials of a vault's linked account: the token sync uses,
/// in the user name convention of the remote's host. Read when a download is
/// actually made, on the worker thread that makes it.
struct TokenAuth {
    remote: String,
    store: std::sync::Arc<dyn immermemo_core::TokenStore>,
}

impl immermemo_media::LfsAuth for TokenAuth {
    fn basic_auth(&self) -> Option<(String, String)> {
        let token = self.store.load().ok().flatten()?;
        Some(immermemo_provider::https_basic_auth_for_remote(
            &self.remote,
            &token,
        ))
    }
}

/// The Git LFS server of the vault `session` has open, if it has a remote
/// one can be found for (see `immermemo_media::LfsRemote`).
fn lfs_remote(session: &Session) -> Option<immermemo_media::LfsRemote> {
    let remote = session.remote.clone()?;
    let auth = TokenAuth {
        remote: remote.clone(),
        store: session.sync_token_store(),
    };
    immermemo_media::LfsRemote::new(&remote, std::sync::Arc::new(auth))
}

/// Where the open note's relative image references resolve: the vault, and
/// the note (`current-note-path`) they were written in -- and the vault's Git
/// LFS server, for images that are pointers to it.
pub fn media_base(app: &App, session: &SessionRef) -> crate::thumbs::MediaBase {
    let s = session.borrow();
    crate::thumbs::MediaBase::new(
        &s.vault_dir,
        std::path::Path::new(app.get_current_note_path().as_str()),
    )
    .with_lfs(lfs_remote(&s))
}

/// Sets `body` and recomputes `note-body-items` in the same step.
pub fn update_rendered_body(app: &App, session: &SessionRef, text: &str) {
    app.set_body(text.into());
    let max_width = app.get_body_content_width();
    if max_width > 0.0 {
        let base = media_base(app, session);
        app.set_note_body_items(crate::render::note_body_items(text, app, max_width, &base));
    } else {
        app.set_note_body_items(ModelRc::default());
    }
}

/// Refreshes everything derived from the open note's text: counts, kind,
/// properties and the icon on the mini card.
pub fn update_note_stats(app: &App, session: &SessionRef, text: &str) {
    let stats = immermemo_core::note_stats(text);
    app.set_char_count(stats.char_count);
    app.set_word_count(stats.word_count);
    app.set_line_count(stats.line_count);
    let kind = immermemo_render_tomet::extract_note_kind(text).unwrap_or_default();
    app.set_note_kind(kind.into());
    update_current_header(app, session, text);
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

/// Sets what the open note shows of its own header from its `text`: the
/// mini card's icon and the banner above the title. The note's path
/// (`current-note-path`) is what image references are resolved against.
pub fn update_current_header(app: &App, session: &SessionRef, text: &str) {
    let icon = immermemo_render_tomet::extract_note_icon(text);
    let banner = immermemo_render_tomet::extract_note_banner(text);
    let base = media_base(app, session);
    app.set_current_icon(crate::note_icon::resolve_view(icon.as_ref(), &base));
    app.set_current_banner(crate::note_icon::banner_view(banner.as_ref(), &base));
}

pub fn clear_note_stats(app: &App) {
    app.set_current_note_index(-1);
    app.set_current_note_starred(false);
    app.set_current_note_path(SharedString::new());
    app.set_char_count(0);
    app.set_word_count(0);
    app.set_line_count(0);
    app.set_note_kind(SharedString::new());
    app.set_current_icon(NoteIconView::default());
    app.set_current_banner(crate::NoteBannerView::default());
    app.set_note_metadata(Rc::new(VecModel::from(Vec::<NotePropertyEntry>::new())).into());
}

#[cfg(test)]
mod lfs_auth_tests {
    use super::*;
    use immermemo_media::LfsAuth;

    fn auth_for(remote: &str, token: Option<&str>) -> Option<(String, String)> {
        let dir = tempfile::tempdir().unwrap();
        let store = immermemo_core::PlainFileTokenStore::new(dir.path().join("token"));
        if let Some(token) = token {
            immermemo_core::TokenStore::save(&store, token).unwrap();
        }
        TokenAuth {
            remote: remote.to_owned(),
            store: std::sync::Arc::new(store),
        }
        .basic_auth()
    }

    #[test]
    fn the_token_is_sent_the_way_each_host_expects_it() {
        assert_eq!(
            auth_for("git@github.com:owner/notes.git", Some("ghp_x")),
            Some(("ghp_x".to_owned(), String::new()))
        );
        assert_eq!(
            auth_for("https://gitlab.com/team/notes.git", Some("glpat_x")),
            Some(("oauth2".to_owned(), "glpat_x".to_owned()))
        );
    }

    #[test]
    fn with_no_saved_token_there_are_no_credentials() {
        assert_eq!(auth_for("https://github.com/owner/notes.git", None), None);
    }
}

#[cfg(test)]
mod folder_selection_tests {
    use super::*;

    fn sample_source() -> (Vec<(String, i32, bool)>, Vec<PathBuf>) {
        let notes = vec![
            PathBuf::from("/vault/work/a.tmt"),
            PathBuf::from("/vault/work/sub/b.tmt"),
            PathBuf::from("/vault/work2/c.tmt"),
        ];
        let source = vec![
            ("work/a".to_string(), 0, false),
            ("work/sub/b".to_string(), 1, false),
            ("work2/c".to_string(), 2, false),
        ];
        (source, notes)
    }

    #[test]
    fn derives_none_partial_and_all_across_nested_subfolders() {
        let (source, notes) = sample_source();
        let mut sel = origiri_selection::Selection::default();

        assert_eq!(
            derive_folder_selection_state(&source, &notes, "work", &sel),
            SelectionCheckState::None
        );
        assert_eq!(
            derive_folder_selection_state(&source, &notes, "empty", &sel),
            SelectionCheckState::None
        );

        // Selecting a note in work2 must not affect "work"
        sel.toggle("/vault/work2/c.tmt");
        assert_eq!(
            derive_folder_selection_state(&source, &notes, "work", &sel),
            SelectionCheckState::None
        );

        // Selecting nested note makes "work/sub" All and "work" Partial
        sel.toggle("/vault/work/sub/b.tmt");
        assert_eq!(
            derive_folder_selection_state(&source, &notes, "work/sub", &sel),
            SelectionCheckState::All
        );
        assert_eq!(
            derive_folder_selection_state(&source, &notes, "work", &sel),
            SelectionCheckState::Partial
        );

        // Selecting remaining note in "work" makes "work" All
        sel.toggle("/vault/work/a.tmt");
        assert_eq!(
            derive_folder_selection_state(&source, &notes, "work", &sel),
            SelectionCheckState::All
        );
    }

    #[test]
    fn descendant_note_paths_respects_folder_prefix_boundary() {
        let (source, notes) = sample_source();
        assert_eq!(
            descendant_note_paths(&source, &notes, "work"),
            vec![
                "/vault/work/a.tmt".to_string(),
                "/vault/work/sub/b.tmt".to_string(),
            ]
        );
        assert_eq!(
            descendant_note_paths(&source, &notes, ""),
            vec![
                "/vault/work/a.tmt".to_string(),
                "/vault/work/sub/b.tmt".to_string(),
                "/vault/work2/c.tmt".to_string(),
            ]
        );
    }
}
