//! UI session state management and application action handlers.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::credentials::TokenCredentials;
use crate::sync;
use crate::{App, TokenStoreFactory};
use immermemo_vault::appdata::AppData;
use immermemo_vault::credentials::TokenStore;
use immermemo_vault::history::{History, Restored};
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
    pub notes: Vec<PathBuf>,
    pub conflicted: Vec<bool>,
    pub search_query: String,
    pub filtered_results: Vec<notes::SearchResult>,
    pub current: Option<usize>,
    /// Dropped whenever the note changes from outside (a sync) or another
    /// note is opened, so undo never crosses into different content.
    pub history: Option<History>,
    /// Which note the open rename/delete-confirm dialog acts on, if any --
    /// set when the dialog opens, read (and cleared) when it's confirmed.
    pub rename_target: Option<usize>,
    pub delete_target: Option<usize>,
    pub delete_vault_target: Option<usize>,
}

thread_local! {
    // `invoke_from_event_loop` needs a `Send` closure, so a finished sync
    // finds the session again here rather than carrying an `Rc` across.
    pub static SESSION: RefCell<Option<Rc<RefCell<Session>>>> = const { RefCell::new(None) };
}

impl Session {
    pub fn current_path(&self) -> Option<&PathBuf> {
        self.current.and_then(|i| self.notes.get(i))
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
    let previous = s.current_path().cloned();
    s.notes = notes::scan(&s.vault_dir);
    s.conflicted = s
        .notes
        .iter()
        .map(|p| notes::has_conflict_marker(p))
        .collect();
    let conflict_count = s.conflicted.iter().filter(|&&c| c).count();
    s.current = previous.and_then(|p| s.notes.iter().position(|n| *n == p));
    drop(s);
    app.set_conflict_count(conflict_count as i32);
    update_filtered_list(app, session);
}

pub fn update_filtered_list(app: &App, session: &Rc<RefCell<Session>>) {
    let mut s = session.borrow_mut();
    s.filtered_results = notes::search(&s.vault_dir, &s.notes, &s.search_query);

    let names: Vec<SharedString> = s
        .filtered_results
        .iter()
        .map(|r| notes::display_name(&s.vault_dir, &s.notes[r.note_index]).into())
        .collect();
    let conflicted: Vec<bool> = s
        .filtered_results
        .iter()
        .map(|r| s.conflicted[r.note_index])
        .collect();
    let snippets: Vec<SharedString> = s
        .filtered_results
        .iter()
        .map(|r| r.snippet.clone().unwrap_or_default().into())
        .collect();

    let ui_current = s
        .current
        .and_then(|cur| s.filtered_results.iter().position(|r| r.note_index == cur));

    drop(s);
    app.set_notes(ModelRc::new(VecModel::from(names)));
    app.set_conflicted(ModelRc::new(VecModel::from(conflicted)));
    app.set_snippets(ModelRc::new(VecModel::from(snippets)));
    app.set_current(ui_current.map_or(-1, |i| i as i32));
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
    s.vault_dir = vault_dir;
    s.token_store = token_store;
    s.remote = remote.clone();
    s.search_query.clear();
    s.current = None;
    s.history = None;
    drop(s);

    app.set_search_query(SharedString::new());
    app.set_remote_configured(remote.is_some());
    app.set_current_title(SharedString::new());
    app.set_body(SharedString::new());
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
}

pub fn update_note_stats(app: &App, text: &str) {
    let char_count = text.chars().count() as i32;
    let word_count = text.split_whitespace().count() as i32;
    let line_count = if text.is_empty() {
        0
    } else {
        text.lines().count() as i32
    };
    app.set_char_count(char_count);
    app.set_word_count(word_count);
    app.set_line_count(line_count);
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
    let mut s = session.borrow_mut();
    let Some(path) = s.notes.get(index).cloned() else {
        return;
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            s.current = Some(index);
            s.history = Some(History::new(&text));
            let _ = s.app_data.save_last_note(&s.vault_dir, &path);
            let ui_current = s.filtered_results.iter().position(|r| r.note_index == index);
            app.set_current(ui_current.map_or(-1, |i| i as i32));
            app.set_current_title(notes::display_name(&s.vault_dir, &path).into());
            let note_rel_path = path
                .strip_prefix(&s.vault_dir)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            app.set_current_note_path(note_rel_path.into());
            app.set_current_has_conflict(s.conflicted[index]);
            show_history_state(app, &s);
            app.set_body(text.clone().into());
            update_note_stats(app, &text);
            set_status(app, "");
        }
        Err(e) => set_status(app, format!("Could not open {}: {e}", path.display())),
    }
}

/// Opens the note that was last open in `vault_dir`, or the first note in the
/// list if none was recorded (or if the recorded note is gone). If the vault
/// has no notes at all, does nothing.
pub fn open_initial_or_last_note(app: &App, session: &Rc<RefCell<Session>>) {
    let s = session.borrow();
    if s.notes.is_empty() {
        return;
    }
    let last_note = s.app_data.load_last_note(&s.vault_dir);
    let index = last_note
        .and_then(|p| s.notes.iter().position(|n| *n == p))
        .unwrap_or(0);
    drop(s);
    open_note(app, session, index);
}

/// Sets the status message on the window, classifying it as an error or
/// normal status so both ListPane and EditorPane can present it appropriately.
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

/// Tells the window whether the undo and redo buttons have anything to do.
pub fn show_history_state(app: &App, session: &Session) {
    let history = session.history.as_ref();
    app.set_can_undo(history.is_some_and(History::can_undo));
    app.set_can_redo(history.is_some_and(History::can_redo));
}

pub fn apply_restored(app: &App, session: &Rc<RefCell<Session>>, restored: Option<Restored>) {
    let Some(Restored { text, cursor }) = restored else {
        return;
    };
    if let Some(path) = session.borrow().current_path() {
        match std::fs::write(path, &text) {
            Ok(_) => {
                if app.get_status_is_error() && app.get_status().starts_with("Save failed") {
                    set_status(app, "");
                }
            }
            Err(e) => set_status(app, format!("Save failed: {e}")),
        }
    }
    let has_conflict = text.contains("@mobile.conflict");
    app.set_current_has_conflict(has_conflict);
    if let Some(idx) = session.borrow().current {
        let mut s = session.borrow_mut();
        if idx < s.conflicted.len() {
            s.conflicted[idx] = has_conflict;
            app.set_conflicted(ModelRc::new(VecModel::from(s.conflicted.clone())));
        }
    }
    app.set_body(text.clone().into());
    update_note_stats(app, &text);
    app.invoke_set_cursor(cursor as i32);
    show_history_state(app, &session.borrow());
}

pub fn resolve_active_conflict(
    app: &App,
    session: &Rc<RefCell<Session>>,
    resolution: immermemo_merge::ConflictResolution,
) {
    let mut s = session.borrow_mut();
    let Some(path) = s.current_path().cloned() else {
        return;
    };
    let current_text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            set_status(app, format!("Could not read note: {e}"));
            return;
        }
    };
    let doc = match tomet_parser::parse_document(&current_text) {
        Ok(d) => d,
        Err(e) => {
            set_status(
                app,
                format!("Could not parse note for conflict resolution: {e}"),
            );
            return;
        }
    };
    let side_name = match &resolution {
        immermemo_merge::ConflictResolution::Mine => "local",
        immermemo_merge::ConflictResolution::Theirs => "remote",
        _ => "custom",
    };
    let resolved_doc = immermemo_merge::resolve_all(&doc, resolution);
    let resolved_text = tomet_printer::document_to_tm(&resolved_doc);

    if let Err(e) = std::fs::write(&path, &resolved_text) {
        set_status(app, format!("Save failed: {e}"));
        return;
    }
    if let Some(history) = s.history.as_mut() {
        history.edit(&resolved_text);
    }
    show_history_state(app, &s);
    app.set_body(resolved_text.clone().into());
    update_note_stats(app, &resolved_text);

    let index = s.current.unwrap_or(0);
    if index < s.conflicted.len() {
        s.conflicted[index] = false;
        app.set_conflicted(ModelRc::new(VecModel::from(s.conflicted.clone())));
    }
    let conflict_count = s.conflicted.iter().filter(|&&c| c).count();
    drop(s);

    app.set_conflict_count(conflict_count as i32);
    app.set_current_has_conflict(false);
    refresh_list(app, session);
    set_status(app, format!("Resolved conflict (kept {side_name} version)"));
}

pub fn finish_sync(app: &App, session: &Rc<RefCell<Session>>, result: Result<Vec<PathBuf>, String>) {
    app.set_syncing(false);
    match result {
        Ok(needing_resolution) => {
            let reopen = {
                let s = session.borrow();
                s.current_path().cloned()
            };
            refresh_list(app, session);
            // The open note may have been rewritten by the merge: reload it
            // from disk and start a fresh history.
            if let Some(path) = reopen {
                let index = session.borrow().notes.iter().position(|p| *p == path);
                match index {
                    Some(index) => open_note(app, session, index),
                    None => {
                        session.borrow_mut().history = None;
                        app.set_current_title(SharedString::new());
                        show_history_state(app, &session.borrow());
                        app.set_body(SharedString::new());
                        if session.borrow().notes.is_empty() {
                            let s = session.borrow();
                            s.app_data.clear_last_note(&s.vault_dir);
                            app.set_list_open(true);
                        } else {
                            open_initial_or_last_note(app, session);
                        }
                    }
                }
            } else if session.borrow().notes.is_empty() {
                let s = session.borrow();
                s.app_data.clear_last_note(&s.vault_dir);
                app.set_list_open(true);
            } else {
                open_initial_or_last_note(app, session);
            }
            set_status(
                app,
                if needing_resolution.is_empty() {
                    "Synced".to_owned()
                } else {
                    format!(
                        "Synced; {} note(s) need resolution",
                        needing_resolution.len()
                    )
                },
            );
        }
        Err(e) => set_status(app, format!("Sync failed: {e}")),
    }
}
