//! UI session state management and application action handlers.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use immermemo_editor::EditorState;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::credentials::TokenCredentials;
use crate::directory;
use crate::sync;
use crate::{App, BreadcrumbSegment, DirectoryEntry, TokenStoreFactory};
use immermemo_editor::History;
use immermemo_index::NoteIndex;
use immermemo_sync::Vault;
use immermemo_vault::appdata::AppData;
use immermemo_vault::credentials::TokenStore;
use immermemo_vault::notes;

#[cfg(target_os = "android")]
use crate::android::cert as android_cert;

/// What the window is currently editing. Lives on the UI thread only.
pub struct Session {
    pub vault_dir: PathBuf,
    pub app_data: AppData,
    /// The current vault's token store. Rebuilt (via `token_store_for`)
    /// whenever the vault switches -- it can't be built once at startup
    /// the way it used to be, since a different vault can need a
    /// completely different store (Android: a different Keystore entry).
    pub token_store: Arc<dyn TokenStore>,
    pub token_store_for: TokenStoreFactory,
    /// Every vault folder the user has linked, in the order they were
    /// added. `vault_dir` is always one of these.
    pub known_vaults: Vec<PathBuf>,
    /// The remote URL, kept here once loaded so Sync doesn't re-read it from
    /// disk on every tap. Not secret -- the token lives in `token_store`.
    pub remote: Option<String>,
    pub index: NoteIndex,
    pub notes: Vec<PathBuf>,
    pub conflicted: Vec<bool>,
    pub search_query: String,
    pub filtered_results: Vec<notes::SearchResult>,
    /// What the editor is showing right now -- see `immermemo-editor`'s
    /// module doc for why this is its own type rather than more fields
    /// here.
    pub editor: EditorState,
    /// Which note the open rename/delete-confirm dialog acts on, if any --
    /// set when the dialog opens, read (and cleared) when it's confirmed.
    pub rename_target: Option<usize>,
    pub delete_target: Option<usize>,
    pub delete_vault_target: Option<usize>,
    pub pending_auto_sync: bool,
    pub last_synced_at: Option<std::time::SystemTime>,
    /// The folder the Directory page (List and Grid alike) is currently
    /// browsing, keyed by its full slash-joined title (e.g. `work/sub`).
    /// `""` means the vault root. Ephemeral UI state, not persisted.
    pub current_folder: String,
    /// `(display title, index into notes/conflicted, has_conflict)` for
    /// whatever's currently filtered into view, sorted alphabetically by
    /// title -- the cache [`refresh_directory_views`] rebuilds
    /// `tree_rows`/`grid_entries` from on every toggle/navigate, without
    /// re-running `index.search`.
    pub directory_source: Vec<(String, i32, bool)>,
}

thread_local! {
    // `invoke_from_event_loop` needs a `Send` closure, so a finished sync
    // finds the session again here rather than carrying an `Rc` across.
    pub static SESSION: RefCell<Option<Rc<RefCell<Session>>>> = const { RefCell::new(None) };
    static AUTO_SYNC_TIMER: RefCell<slint::Timer> = RefCell::new(slint::Timer::default());
}

impl Session {
    pub fn current_path(&self) -> Option<PathBuf> {
        immermemo_editor::current_path(&self.editor, &self.notes)
    }
}

/// Spawns a sync in the background, unless no remote is linked yet -- then
/// it just opens the sheet to ask for one instead of failing.
pub fn start_sync(app: &App, session: &Rc<RefCell<Session>>) {
    let (vault_dir, gitdir, remote, token_store) = {
        let s = session.borrow();
        let Some(remote) = s.remote.clone() else {
            set_status(app, "No remote linked yet");
            app.set_remote_url_draft(SharedString::new());
            app.set_remote_token_draft(SharedString::new());
            app.set_remote_has_token(false);
            app.set_current_vault_name(vault_display_name(&s.vault_dir).into());
            app.set_current_vault_path(s.vault_dir.display().to_string().into());
            app.set_settings_open(true);
            return;
        };
        let gitdir = match s.app_data.gitdir(&s.vault_dir) {
            Ok(g) => g,
            Err(e) => {
                drop(s);
                set_status(app, format!("Could not prepare the vault: {e}"));
                return;
            }
        };
        (s.vault_dir.clone(), gitdir, remote, s.token_store.clone())
    };

    app.set_syncing(true);
    app.set_last_sync_error(SharedString::new());
    app.set_sync_error_copied(false);
    set_status(app, "Syncing...");
    let weak = app.as_weak();
    // Vault is opened inside the thread; only plain data and the (Send +
    // Sync) token store cross.
    std::thread::spawn(move || {
        let credentials = TokenCredentials::new(token_store);
        // Desktop's OpenSSL can still find an OS-provided CA bundle itself
        // (see `git2::init`'s path probing); only Android needs a verifier
        // supplied here at all.
        #[cfg(target_os = "android")]
        let verifier = android_cert::verifier().map(Some);
        #[cfg(not(target_os = "android"))]
        let verifier: anyhow::Result<Option<Box<dyn immermemo_sync::CertificateVerifier>>> =
            Ok(None);
        let result = verifier
            .and_then(|verifier| sync::run(&vault_dir, &gitdir, &remote, &credentials, verifier))
            .map_err(|e| format!("{e:#}"));
        let _ = slint::invoke_from_event_loop(move || {
            let app = weak.unwrap();
            let session = SESSION
                .with(|s| s.borrow().clone())
                .expect("session set in run");
            finish_sync(&app, &session, result);
        });
    });
}

pub fn refresh_list(app: &App, session: &Rc<RefCell<Session>>) {
    let mut s = session.borrow_mut();
    let previous = s.current_path();
    let vault_dir = s.vault_dir.clone();
    let _ = s.index.reconcile_filesystem(&vault_dir);
    let all = s.index.list_all().unwrap_or_default();
    let vault_dir = s.vault_dir.clone();
    s.notes = all.iter().map(|n| vault_dir.join(&n.path)).collect();
    s.conflicted = all.iter().map(|n| n.has_conflict).collect();
    let conflict_count = s.conflicted.iter().filter(|&&c| c).count();
    s.editor.current = previous.and_then(|p| s.notes.iter().position(|n| *n == p));
    drop(s);
    app.set_conflict_count(conflict_count as i32);
    update_filtered_list(app, session);
}

pub fn update_filtered_list(app: &App, session: &Rc<RefCell<Session>>) {
    let mut s = session.borrow_mut();
    let hits = s.index.search(&s.search_query).unwrap_or_default();
    let vault_dir = s.vault_dir.clone();

    let mut filtered_results = Vec::with_capacity(hits.len());
    let mut names = Vec::with_capacity(hits.len());
    let mut conflicted = Vec::with_capacity(hits.len());
    let mut snippets = Vec::with_capacity(hits.len());
    // Mirrors `names`/`filtered_results`/`conflicted` above but sorted by
    // title -- `hits` is ranked by search relevance, not path, while the
    // Directory Tree/Grid views need a folder's notes to appear as a
    // contiguous run. See [`refresh_directory_views`].
    let mut directory_source = Vec::with_capacity(hits.len());

    for hit in hits {
        let abs_path = vault_dir.join(&hit.path);
        if let Some(index) = s.notes.iter().position(|p| *p == abs_path) {
            directory_source.push((hit.title.clone(), index as i32, hit.has_conflict));
            filtered_results.push(notes::SearchResult {
                note_index: index,
                snippet: hit.snippet.clone(),
            });
            names.push(SharedString::from(hit.title));
            conflicted.push(hit.has_conflict);
            snippets.push(SharedString::from(hit.snippet.unwrap_or_default()));
        }
    }
    directory_source.sort_by(|a: &(String, i32, bool), b| a.0.cmp(&b.0));

    s.filtered_results = filtered_results;
    s.directory_source = directory_source;
    let ui_current = s
        .editor
        .current
        .and_then(|cur| s.filtered_results.iter().position(|r| r.note_index == cur));

    drop(s);
    app.set_notes(ModelRc::new(VecModel::from(names)));
    app.set_conflicted(ModelRc::new(VecModel::from(conflicted)));
    app.set_snippets(ModelRc::new(VecModel::from(snippets)));
    app.set_current(ui_current.map_or(-1, |i| i as i32));
    refresh_directory_views(app, session);
}

/// Rebuilds the Directory page's List/Grid contents from `Session`'s
/// cached `directory_source` plus whichever folder is currently browsed --
/// called after [`update_filtered_list`] rebuilds that cache, and again
/// (cheaply, with no new `index.search`) from [`navigate_directory_folder`]
/// below.
pub fn refresh_directory_views(app: &App, session: &Rc<RefCell<Session>>) {
    let s = session.borrow();
    let source: Vec<directory::TitledNote> = s
        .directory_source
        .iter()
        .map(|(title, idx, conflict)| (title.as_str(), *idx, *conflict))
        .collect();
    let folder_entries: Vec<DirectoryEntry> =
        directory::build_folder_entries(&source, &s.current_folder)
            .into_iter()
            .map(to_entry_view)
            .collect();
    let breadcrumb: Vec<BreadcrumbSegment> = directory::build_breadcrumb(&s.current_folder)
        .into_iter()
        .map(|(key, display)| BreadcrumbSegment {
            key: key.into(),
            display: display.into(),
        })
        .collect();
    drop(s);
    app.set_directory_folder_entries(ModelRc::new(VecModel::from(folder_entries)));
    app.set_directory_breadcrumb(ModelRc::new(VecModel::from(breadcrumb)));
}

fn to_entry_view(e: directory::DirectoryEntry) -> DirectoryEntry {
    DirectoryEntry {
        is_folder: e.is_folder,
        key: e.key.into(),
        display: e.display.into(),
        note_index: e.note_index,
        has_conflict: e.has_conflict,
    }
}

/// Navigates the Directory page (List and Grid alike) to `key` (a folder's
/// full slash-joined title, or `""` for the vault root) -- driven by a
/// folder tile/row, a breadcrumb segment, the × root-reset, or the
/// hardware back action popping one level.
pub fn navigate_directory_folder(app: &App, session: &Rc<RefCell<Session>>, key: &str) {
    session.borrow_mut().current_folder = key.to_string();
    refresh_directory_views(app, session);
}

pub fn refresh_vault_list(app: &App, session: &Rc<RefCell<Session>>) {
    let s = session.borrow();
    let names: Vec<SharedString> = s
        .known_vaults
        .iter()
        .map(|p| vault_display_name(p).into())
        .collect();
    let current = s.known_vaults.iter().position(|v| *v == s.vault_dir);
    drop(s);
    app.set_vaults(ModelRc::new(VecModel::from(names)));
    app.set_current_vault(current.map_or(-1, |i| i as i32));
}

pub fn vault_display_name(vault_dir: &std::path::Path) -> String {
    vault_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| vault_dir.display().to_string())
}

/// Switches to `vault_dir`, reloading everything that's per-vault: the
/// token store, the remote URL, the note list, and the open editor
/// (there's no note from the old vault that still makes sense to show).
pub fn switch_vault(app: &App, session: &Rc<RefCell<Session>>, vault_dir: PathBuf) {
    if let Err(e) = std::fs::create_dir_all(&vault_dir) {
        set_status(app, format!("Could not open this vault: {e}"));
        return;
    }
    let token_store = {
        let s = session.borrow();
        match (s.token_store_for)(&vault_dir) {
            Ok(t) => t,
            Err(e) => {
                drop(s);
                set_status(app, format!("Could not open this vault: {e}"));
                return;
            }
        }
    };

    let mut s = session.borrow_mut();
    let remote = s.app_data.load_remote(&vault_dir);
    app.set_current_vault_name(vault_display_name(&vault_dir).into());
    app.set_current_vault_path(vault_dir.display().to_string().into());
    let _ = s.app_data.save_current_vault(&vault_dir);
    let index = s
        .app_data
        .index_path(&vault_dir)
        .ok()
        .and_then(|p| NoteIndex::open(&p).ok())
        .unwrap_or_else(|| NoteIndex::open_in_memory().unwrap());
    s.vault_dir = vault_dir;
    s.token_store = token_store;
    s.remote = remote.clone();
    s.index = index;
    s.search_query.clear();
    s.editor = EditorState::default();
    s.pending_auto_sync = false;
    s.last_synced_at = None;
    drop(s);

    app.set_search_query(SharedString::new());
    app.set_remote_configured(remote.is_some());
    app.set_current_title(SharedString::new());
    update_rendered_body(app, "");
    app.set_last_sync_error(SharedString::new());
    app.set_sync_error_copied(false);
    app.set_last_synced_at(SharedString::new());
    clear_note_stats(app);
    show_history_state(app, &session.borrow());
    refresh_list(app, session);
    refresh_vault_list(app, session);
    if session.borrow().notes.is_empty() {
        app.set_list_open(true);
    } else {
        app.set_list_open(false);
        open_initial_or_last_note(app, session);
    }
    if remote.is_some() {
        schedule_auto_sync(app, std::time::Duration::from_millis(500));
    }
}

/// Sets `body` and recomputes `note-body-items` (editor.slint's view-mode
/// display) from it in the same step, so the two never drift apart. Every
/// place `body` changes from the Rust side should go through this instead
/// of `app.set_body` directly -- the one exception is `lib.rs`'s
/// `on_edited` handler, where `body` has already changed via the
/// `TextInput`'s own two-way binding before Rust ever sees the edit, and
/// only `note-body-items` needs recomputing.
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

/// Turns what the user typed in the "Add vault" dialog into an actual
/// vault directory -- desktop takes it as a path outright; Android has no
/// external folder access in this build, so it names a subfolder of the
/// same private storage the app already uses.
#[cfg(target_os = "android")]
pub fn vault_path_from_input(base: &std::path::Path, input: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!input.is_empty(), "the name can't be empty");
    anyhow::ensure!(
        !input.contains(['/', '\\']),
        "the name can't contain a path separator"
    );
    anyhow::ensure!(
        input != "vaults" && input != "vaults.txt" && input != "current_vault.txt",
        "that name is reserved for the app's own data"
    );
    Ok(base.join(input))
}

#[cfg(not(target_os = "android"))]
pub fn vault_path_from_input(_base: &std::path::Path, input: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!input.is_empty(), "the path can't be empty");
    Ok(PathBuf::from(input))
}

pub fn open_note(app: &App, session: &Rc<RefCell<Session>>, index: usize) {
    let outcome = {
        let mut guard = session.borrow_mut();
        let s = &mut *guard;
        let vault_dir = s.vault_dir.clone();
        immermemo_editor::open_note(
            &mut s.editor,
            &s.app_data,
            &vault_dir,
            &s.notes,
            &s.conflicted,
            &s.filtered_results,
            index,
        )
    };
    let Some(outcome) = outcome else { return };
    match outcome {
        Err(msg) => set_status(app, msg),
        Ok(opened) => {
            app.set_current(opened.ui_current.map_or(-1, |i| i as i32));
            app.set_current_title(opened.title.into());
            app.set_current_note_path(opened.note_rel_path.into());
            app.set_current_has_conflict(opened.has_conflict);
            app.set_can_undo(opened.can_undo);
            app.set_can_redo(opened.can_redo);
            update_rendered_body(app, &opened.body);
            update_note_stats(app, &opened.body);
            set_status(app, "");
            app.set_active_conflict_index(0);
            if opened.has_conflict {
                sync_conflict_sheet_state(app, session);
            } else {
                app.set_conflict_sheet_open(false);
                app.set_active_conflict_total(0);
                app.set_active_conflict_mine(SharedString::new());
                app.set_active_conflict_theirs(SharedString::new());
            }
        }
    }
}

/// Opens the note that was last open in `vault_dir`, or the first note in the
/// list if none was recorded (or if the recorded note is gone). If the vault
/// has no notes at all, does nothing.
pub fn open_initial_or_last_note(app: &App, session: &Rc<RefCell<Session>>) {
    let index = {
        let s = session.borrow();
        immermemo_editor::initial_or_last_note_index(&s.app_data, &s.vault_dir, &s.notes)
    };
    if let Some(index) = index {
        open_note(app, session, index);
    }
}

/// Sets the status message on the window, classifying it as an error or
/// normal status so both FilesScreen and EditorScreen can present it appropriately.
pub fn set_status(app: &App, msg: impl Into<SharedString>) {
    let s: SharedString = msg.into();
    let text = s.as_str();
    let is_error = text.starts_with("Save failed")
        || text.starts_with("Sync failed")
        || text.starts_with("Could not")
        || text.starts_with("Enter a remote")
        || text.contains("failed")
        || text.contains("error")
        || text.contains("Error");
    app.set_status(s);
    app.set_status_is_error(is_error);
}

/// Copies the given text to the system clipboard using Slint's platform abstraction.
pub fn copy_to_clipboard(app: &App, text: &str) {
    slint::private_unstable_api::re_exports::WindowInner::from_pub(app.window())
        .context()
        .platform()
        .set_clipboard_text(text, slint::platform::Clipboard::DefaultClipboard);
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
    restored: Option<immermemo_editor::Restored>,
) {
    let Some(restored) = restored else {
        return;
    };
    let result = {
        let s = session.borrow();
        immermemo_editor::apply_restored(&s.editor, &s.notes, restored)
    };
    {
        let mut s = session.borrow_mut();
        if let Some(idx) = s.editor.current
            && idx < s.conflicted.len()
        {
            s.conflicted[idx] = result.has_conflict;
        }
    }
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

/// Resolves every conflict in the current note at once, in favor of one
/// side. No UI currently calls this -- the editor/properties "Keep
/// Mine"/"Keep Theirs" bar resolves one conflict at a time via
/// [`resolve_conflict_step`] instead, so that a same-looking button means
/// the same thing on every screen. Kept around (and covered by
/// `resolving_the_conflict_clears_the_marker_and_status` in
/// `src/lib.rs`'s tests) for a future bulk-resolve feature.
#[allow(dead_code)]
pub fn resolve_active_conflict(
    app: &App,
    session: &Rc<RefCell<Session>>,
    resolution: immermemo_merge::ConflictResolution,
) {
    let outcome = {
        let mut guard = session.borrow_mut();
        let s = &mut *guard;
        let vault_dir = s.vault_dir.clone();
        immermemo_editor::resolve_active_conflict(
            &mut s.editor,
            &vault_dir,
            &s.notes,
            &mut s.conflicted,
            &mut s.index,
            resolution,
        )
    };
    let Some(outcome) = outcome else { return };
    match outcome {
        Err(msg) => set_status(app, msg),
        Ok(resolved) => {
            show_history_state(app, &session.borrow());
            update_rendered_body(app, &resolved.body);
            update_note_stats(app, &resolved.body);
            let (conflicted_snapshot, conflict_count) = {
                let s = session.borrow();
                (
                    s.conflicted.clone(),
                    s.conflicted.iter().filter(|&&c| c).count(),
                )
            };
            app.set_conflicted(ModelRc::new(VecModel::from(conflicted_snapshot)));
            app.set_conflict_count(conflict_count as i32);
            app.set_current_has_conflict(false);
            app.set_conflict_sheet_open(false);
            refresh_list(app, session);
            set_status(app, resolved.status);
        }
    }
}

pub fn sync_conflict_sheet_state(app: &App, session: &Rc<RefCell<Session>>) {
    let state = {
        let s = session.borrow();
        immermemo_editor::conflict_sheet_state(&s.editor, &s.notes, app.get_active_conflict_index())
    };
    match state {
        immermemo_editor::ConflictSheetState::NoActiveNote => {
            app.set_active_conflict_total(0);
            app.set_conflict_sheet_open(false);
        }
        immermemo_editor::ConflictSheetState::NoConflicts => {
            app.set_active_conflict_total(0);
            app.set_conflict_sheet_open(false);
            app.set_current_has_conflict(false);
            app.set_active_conflict_mine(SharedString::new());
            app.set_active_conflict_theirs(SharedString::new());
        }
        immermemo_editor::ConflictSheetState::Active {
            total,
            index,
            mine,
            theirs,
        } => {
            app.set_active_conflict_total(total as i32);
            app.set_active_conflict_index(index as i32);
            app.set_active_conflict_mine(mine.into());
            app.set_active_conflict_theirs(theirs.into());
        }
    }
}

pub fn resolve_conflict_step(app: &App, session: &Rc<RefCell<Session>>, choice: i32) {
    let active_conflict_index = app.get_active_conflict_index();
    let outcome = {
        let mut guard = session.borrow_mut();
        let s = &mut *guard;
        let vault_dir = s.vault_dir.clone();
        immermemo_editor::resolve_conflict_step(
            &mut s.editor,
            &vault_dir,
            &s.notes,
            &mut s.conflicted,
            &mut s.index,
            active_conflict_index,
            choice,
        )
    };
    let Some(outcome) = outcome else { return };
    match outcome {
        Err(msg) => set_status(app, msg),
        Ok(resolved) => {
            show_history_state(app, &session.borrow());
            update_rendered_body(app, &resolved.body);
            update_note_stats(app, &resolved.body);
            let (conflicted_snapshot, conflict_count) = {
                let s = session.borrow();
                (
                    s.conflicted.clone(),
                    s.conflicted.iter().filter(|&&c| c).count(),
                )
            };
            app.set_conflicted(ModelRc::new(VecModel::from(conflicted_snapshot)));
            app.set_conflict_count(conflict_count as i32);
            app.set_current_has_conflict(resolved.has_conflict);
            sync_conflict_sheet_state(app, session);
            refresh_list(app, session);
            set_status(app, resolved.status);
        }
    }
}

/// Shows the note list if the vault has no notes left (clearing the
/// remembered "last open note" and the Properties stats so neither
/// lingers for a note that's gone), or reopens the initial/last note
/// otherwise. Shared tail of [`finish_sync`] and `src/lib.rs`'s
/// `on_confirm_delete` handler, both of which need this once whatever
/// was open stops existing (a sync's merge rewrote it away, or the user
/// just deleted it) -- `switch_to_files_tab` matches an explicit delete
/// jumping the user to the list; a background sync doing the same
/// wouldn't make sense, since the user didn't take an action here.
pub fn show_list_or_reopen(app: &App, session: &Rc<RefCell<Session>>, switch_to_files_tab: bool) {
    if session.borrow().notes.is_empty() {
        let s = session.borrow();
        s.app_data.clear_last_note(&s.vault_dir);
        drop(s);
        clear_note_stats(app);
        if switch_to_files_tab {
            app.set_active_tab(1);
        }
        app.set_list_open(true);
    } else {
        open_initial_or_last_note(app, session);
    }
}

/// Clears the currently-open note's UI state (title, body, history) and
/// then either shows the note list or reopens another note, via
/// [`show_list_or_reopen`] -- the common tail [`finish_sync`] and
/// `on_confirm_delete` both need once the note they had open stops
/// existing.
pub fn close_current_note_and_show_list_or_reopen(
    app: &App,
    session: &Rc<RefCell<Session>>,
    switch_to_files_tab: bool,
) {
    session.borrow_mut().editor.history = None;
    app.set_current_title(SharedString::new());
    show_history_state(app, &session.borrow());
    update_rendered_body(app, "");
    show_list_or_reopen(app, session, switch_to_files_tab);
}

pub fn finish_sync(
    app: &App,
    session: &Rc<RefCell<Session>>,
    result: Result<immermemo_sync::SyncReport, String>,
) {
    app.set_syncing(false);
    let mut should_sync_again = false;
    match result {
        Ok(report) => {
            app.set_last_sync_error(SharedString::new());
            app.set_sync_error_copied(false);
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            app.set_last_synced_at(format_time_only(now_secs).into());
            {
                let mut s = session.borrow_mut();
                s.last_synced_at = Some(std::time::SystemTime::now());
                if s.pending_auto_sync {
                    s.pending_auto_sync = false;
                    should_sync_again = true;
                }
                let vault_dir = s.vault_dir.clone();
                let _ = s.index.apply_sync_report(
                    &vault_dir,
                    &report.updated_notes,
                    &report.deleted_notes,
                    &report.notes_needing_resolution,
                );
            }
            let reopen = session.borrow().current_path();
            refresh_list(app, session);
            // The open note may have been rewritten by the merge: reload it
            // from disk and start a fresh history.
            if let Some(path) = reopen {
                let index = session.borrow().notes.iter().position(|p| *p == path);
                match index {
                    Some(index) => open_note(app, session, index),
                    None => close_current_note_and_show_list_or_reopen(app, session, false),
                }
            } else {
                show_list_or_reopen(app, session, false);
            }
            set_status(
                app,
                if report.notes_needing_resolution.is_empty() {
                    "Synced".to_owned()
                } else {
                    format!(
                        "Synced; {} note(s) need resolution",
                        report.notes_needing_resolution.len()
                    )
                },
            );
        }
        Err(e) => {
            let error_msg = format!("Sync failed: {e}");
            app.set_last_sync_error(SharedString::from(&error_msg));
            app.set_sync_error_copied(false);
            set_status(app, error_msg);
        }
    }
    if should_sync_again {
        trigger_auto_sync(app, session);
    }
}

fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1029 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = (mp as i64) + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    (y, m, d as i64)
}

pub fn format_timestamp(epoch_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(epoch_secs);
    let rel = if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    };

    let secs_per_day = 86400;
    let days = epoch_secs.div_euclid(secs_per_day);
    let time_of_day = epoch_secs.rem_euclid(secs_per_day);
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02} ({rel})")
}

pub fn format_time_only(epoch_secs: i64) -> String {
    let secs_per_day = 86400;
    let days = epoch_secs.div_euclid(secs_per_day);
    let time_of_day = epoch_secs.rem_euclid(secs_per_day);
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let (_year, month, day) = days_to_ymd(days);
    format!("{month:02}/{day:02} {hours:02}:{minutes:02}")
}

/// Triggers an automatic sync in the background if a remote is configured and no sync is running.
/// If a sync is already running, sets `pending_auto_sync` so another sync will run after it completes.
pub fn trigger_auto_sync(app: &App, session: &Rc<RefCell<Session>>) {
    let has_remote = {
        let Ok(s) = session.try_borrow() else {
            // Contested borrow: retry in 200ms
            schedule_auto_sync(app, std::time::Duration::from_millis(200));
            return;
        };
        s.remote.is_some()
    };
    if !has_remote {
        return;
    }

    if app.get_syncing() {
        if let Ok(mut s) = session.try_borrow_mut() {
            s.pending_auto_sync = true;
        } else {
            schedule_auto_sync(app, std::time::Duration::from_millis(200));
        }
        return;
    }

    if let Ok(mut s) = session.try_borrow_mut() {
        s.pending_auto_sync = false;
    }
    start_sync(app, session);
}

/// Schedules an auto-sync after `duration`. Any subsequent call before the timer fires
/// resets the countdown (debounce). Completely independent of `Session` borrows.
pub fn schedule_auto_sync(app: &App, duration: std::time::Duration) {
    let weak = app.as_weak();
    AUTO_SYNC_TIMER.with(|timer| {
        timer
            .borrow_mut()
            .start(slint::TimerMode::SingleShot, duration, move || {
                let Some(app) = weak.upgrade() else { return };
                let session = SESSION.with(|cell| cell.borrow().clone());
                let Some(session) = session else { return };
                trigger_auto_sync(&app, &session);
            });
    });
}

pub fn open_note_history(app: &App, session: &Rc<RefCell<Session>>) {
    let revisions = {
        let s = session.borrow();
        let current_path = s.current_path();
        immermemo_editor::load_note_history(&s.app_data, &s.vault_dir, current_path, 50)
    };
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

    session.borrow_mut().editor.note_history_revisions = revisions;

    app.set_history_commits(ModelRc::new(VecModel::from(commit_ids)));
    app.set_history_dates(ModelRc::new(VecModel::from(dates)));
    app.set_history_previews(ModelRc::new(VecModel::from(previews)));
    app.set_history_selected_index(0);
    app.set_history_sheet_open(true);
}

pub fn restore_note_version(app: &App, session: &Rc<RefCell<Session>>, rev_idx: usize) {
    let outcome = {
        let mut guard = session.borrow_mut();
        let s = &mut *guard;
        let vault_dir = s.vault_dir.clone();
        immermemo_editor::restore_note_version(
            &mut s.editor,
            &vault_dir,
            &s.notes,
            &mut s.index,
            rev_idx,
        )
    };
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
    let s = session.borrow();
    let vault_dir = s.vault_dir.clone();
    let Ok(gitdir) = s.app_data.gitdir(&vault_dir) else {
        return;
    };
    drop(s);

    let commits = match Vault::open(&vault_dir, &gitdir) {
        Ok(vault) => vault.vault_history(50).unwrap_or_default(),
        Err(_) => Vec::new(),
    };

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

/// Opens the first note in the vault that has an unresolved conflict, switches to
/// the editor tab, and opens the conflict resolution sheet.
pub fn open_first_conflicted_note(app: &App, session: &Rc<RefCell<Session>>) {
    let target_idx = {
        let s = session.borrow();
        immermemo_editor::first_conflicted_note_index(&s.editor, &s.conflicted)
    };
    if let Some(idx) = target_idx {
        open_note(app, session, idx);
        app.set_note_sheet_open(true);
        sync_conflict_sheet_state(app, session);
        app.set_conflict_sheet_open(true);
    }
}
