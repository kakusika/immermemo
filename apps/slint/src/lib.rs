mod appdata;
pub mod credentials;
mod history;
mod notes;
mod sync;
mod token_blob;

#[cfg(target_os = "android")]
mod android_cert;
#[cfg(target_os = "android")]
mod android_keystore;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use appdata::AppData;
use credentials::{TokenCredentials, TokenStore};
use history::{History, Restored};

slint::include_modules!();

/// Builds the token store for a given vault directory -- a plain file on
/// desktop, the Keystore on Android. Called once at startup and again
/// every time the current vault changes, since a different vault can need
/// a completely different store.
pub type TokenStoreFactory = Box<dyn Fn(&Path) -> anyhow::Result<Arc<dyn TokenStore>>>;

/// What the window is currently editing. Lives on the UI thread only.
struct Session {
    vault_dir: PathBuf,
    app_data: AppData,
    /// The current vault's token store. Rebuilt (via `token_store_for`)
    /// whenever the vault switches -- it can't be built once at startup
    /// the way it used to be, since a different vault can need a
    /// completely different store (Android: a different Keystore entry).
    token_store: Arc<dyn TokenStore>,
    token_store_for: TokenStoreFactory,
    /// Every vault folder the user has linked, in the order they were
    /// added. `vault_dir` is always one of these.
    known_vaults: Vec<PathBuf>,
    /// The remote URL, kept here once loaded so Sync doesn't re-read it from
    /// disk on every tap. Not secret -- the token lives in `token_store`.
    remote: Option<String>,
    notes: Vec<PathBuf>,
    conflicted: Vec<bool>,
    current: Option<usize>,
    /// Dropped whenever the note changes from outside (a sync) or another
    /// note is opened, so undo never crosses into different content.
    history: Option<History>,
    /// Which note the open rename/delete-confirm dialog acts on, if any --
    /// set when the dialog opens, read (and cleared) when it's confirmed.
    rename_target: Option<usize>,
    delete_target: Option<usize>,
}

thread_local! {
    // `invoke_from_event_loop` needs a `Send` closure, so a finished sync
    // finds the session again here rather than carrying an `Rc` across.
    static SESSION: RefCell<Option<Rc<RefCell<Session>>>> = const { RefCell::new(None) };
}

impl Session {
    fn current_path(&self) -> Option<&PathBuf> {
        self.current.and_then(|i| self.notes.get(i))
    }
}

/// Where a vault's access token would live under `app_data_base`, without
/// needing a running [`Session`] -- for a caller of [`run`] building its
/// token-store factory before the app (and its `AppData`) exists yet.
pub fn token_path_in(app_data_base: &Path, vault_dir: &Path) -> anyhow::Result<PathBuf> {
    AppData::new(app_data_base.to_owned()).token_path(vault_dir)
}

/// Desktop's app-data base: `$XDG_DATA_HOME/immermemo`, or
/// `$HOME/.local/share/immermemo`. `main.rs` passes this to [`run`]; Android
/// has neither variable and computes its own base in `android_main` instead.
pub fn desktop_app_data_dir() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| anyhow::anyhow!("neither XDG_DATA_HOME nor HOME is set"))?;
    Ok(base.join("immermemo"))
}

/// Opens the window and runs until it is closed.
///
/// `default_vault_dir` only matters the very first time the app ever
/// runs (nothing in `app_data_base` yet): it becomes the first known
/// vault, so a fresh desktop CLI invocation or a fresh Android install
/// still opens straight onto a vault the way a single-vault build used
/// to. After that, which vaults exist and which one is current come
/// entirely from `app_data_base`.
///
/// `app_data_base` is this platform's private storage for everything that
/// isn't a note: each vault's own private gitdir, remote URL and (outside
/// Android, which keeps its own token behind the Keystore) access token,
/// plus the list of known vaults itself. `token_store_for` builds the
/// token store for a given vault directory -- platform-specific (a plain
/// file on desktop, the Keystore on Android), and called again every time
/// the current vault changes, since a different vault can need a
/// completely different store.
pub fn run(
    default_vault_dir: PathBuf,
    app_data_base: PathBuf,
    token_store_for: TokenStoreFactory,
) -> anyhow::Result<()> {
    let app_data = AppData::new(app_data_base);
    let mut known_vaults = app_data.known_vaults();
    if known_vaults.is_empty() {
        std::fs::create_dir_all(&default_vault_dir)?;
        app_data.add_vault(&default_vault_dir)?;
        known_vaults.push(default_vault_dir);
    }
    let vault_dir = app_data
        .load_current_vault()
        .filter(|v| known_vaults.contains(v))
        .unwrap_or_else(|| known_vaults[0].clone());
    std::fs::create_dir_all(&vault_dir)?;

    let token_store = token_store_for(&vault_dir)?;
    let remote = app_data.load_remote(&vault_dir);

    let app = App::new()?;
    app.set_remote_configured(remote.is_some());
    let session = Rc::new(RefCell::new(Session {
        vault_dir,
        app_data,
        token_store,
        token_store_for,
        known_vaults,
        remote,
        notes: Vec::new(),
        conflicted: Vec::new(),
        current: None,
        history: None,
        rename_target: None,
        delete_target: None,
    }));

    SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
    refresh_list(&app, &session);
    refresh_vault_list(&app, &session);

    app.on_select({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            open_note(&app, &session, index as usize);
        }
    });

    app.on_new_note({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let dir = session.borrow().vault_dir.clone();
            match notes::create(&dir) {
                Ok(path) => {
                    refresh_list(&app, &session);
                    let index = session.borrow().notes.iter().position(|p| *p == path);
                    if let Some(index) = index {
                        open_note(&app, &session, index);
                    }
                }
                Err(e) => app.set_status(format!("Could not create a note: {e}").into()),
            }
        }
    });

    app.on_edited({
        let (weak, session) = (app.as_weak(), session.clone());
        move |text| {
            let app = weak.unwrap();
            let mut s = session.borrow_mut();
            if let Some(history) = s.history.as_mut() {
                history.edit(&text);
            }
            show_history_state(&app, &s);
            if let Some(path) = s.current_path() {
                if let Err(e) = std::fs::write(path, text.as_str()) {
                    app.set_status(format!("Save failed: {e}").into());
                }
            }
        }
    });

    app.on_undo({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let restored = session
                .borrow_mut()
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
                .history
                .as_mut()
                .and_then(History::redo);
            apply_restored(&weak.unwrap(), &session, restored);
        }
    });

    app.on_link({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let s = session.borrow();
            app.set_remote_url_draft(s.remote.clone().unwrap_or_default().into());
            app.set_remote_token_draft(SharedString::new());
            app.set_remote_has_token(s.token_store.load().unwrap_or_default().is_some());
            app.set_remote_open(true);
        }
    });
    app.on_close_remote({
        let weak = app.as_weak();
        move || weak.unwrap().set_remote_open(false)
    });

    app.on_rename_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            let mut s = session.borrow_mut();
            let Some(path) = s.notes.get(index as usize).cloned() else {
                return;
            };
            s.rename_target = Some(index as usize);
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
            let was_current = session.borrow().current_path() == Some(&path);
            match notes::rename(&path, &new_name) {
                Ok(new_path) => {
                    refresh_list(&app, &session);
                    if was_current {
                        let index = session.borrow().notes.iter().position(|p| *p == new_path);
                        if let Some(index) = index {
                            open_note(&app, &session, index);
                        }
                    }
                }
                Err(e) => app.set_status(format!("Could not rename the note: {e}").into()),
            }
        }
    });

    app.on_delete_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            session.borrow_mut().delete_target = Some(index as usize);
            app.set_delete_confirm_open(true);
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
            if let Err(e) = notes::delete(&path) {
                app.set_status(format!("Could not delete the note: {e}").into());
                return;
            }
            let was_current = session.borrow().current_path() == Some(&path);
            refresh_list(&app, &session);
            if was_current {
                session.borrow_mut().history = None;
                app.set_current_title(SharedString::new());
                show_history_state(&app, &session.borrow());
                app.set_body(SharedString::new());
            }
        }
    });

    app.on_open_vault_sheet({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            refresh_vault_list(&app, &session);
            app.set_vault_sheet_open(true);
        }
    });
    app.on_switch_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            app.set_vault_sheet_open(false);
            let Some(vault_dir) = session.borrow().known_vaults.get(index as usize).cloned() else {
                return;
            };
            switch_vault(&app, &session, vault_dir);
        }
    });
    app.on_add_vault_requested({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            app.set_add_vault_draft(SharedString::new());
            app.set_add_vault_open(true);
        }
    });
    app.on_confirm_add_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move |input| {
            let app = weak.unwrap();
            app.set_add_vault_open(false);
            let base = session.borrow().app_data.base().to_owned();
            let new_vault = match vault_path_from_input(&base, &input) {
                Ok(p) => p,
                Err(e) => {
                    app.set_status(format!("Could not add the vault: {e}").into());
                    return;
                }
            };
            if let Err(e) = std::fs::create_dir_all(&new_vault) {
                app.set_status(format!("Could not add the vault: {e}").into());
                return;
            }
            let mut s = session.borrow_mut();
            if let Err(e) = s.app_data.add_vault(&new_vault) {
                drop(s);
                app.set_status(format!("Could not add the vault: {e}").into());
                return;
            }
            s.known_vaults = s.app_data.known_vaults();
            drop(s);
            switch_vault(&app, &session, new_vault);
        }
    });

    app.on_save_remote({
        let (weak, session) = (app.as_weak(), session.clone());
        move |url, token| {
            let app = weak.unwrap();
            let url = url.trim();
            if url.is_empty() {
                app.set_status("Enter a remote URL first".into());
                return;
            }
            let mut s = session.borrow_mut();
            if let Err(e) = s.app_data.save_remote(&s.vault_dir, url) {
                drop(s);
                app.set_status(format!("Could not save the remote: {e}").into());
                return;
            }
            // An empty token field keeps whatever token is already saved --
            // the field starts blank on every open, so the user only retypes
            // it when actually changing it.
            if !token.is_empty()
                && let Err(e) = s.token_store.save(&token)
            {
                drop(s);
                app.set_status(format!("Could not save the access token: {e}").into());
                return;
            }
            s.remote = Some(url.to_owned());
            drop(s);
            app.set_remote_configured(true);
            app.set_remote_open(false);
            start_sync(&app, &session);
        }
    });

    app.on_sync({
        let (weak, session) = (app.as_weak(), session.clone());
        move || start_sync(&weak.unwrap(), &session)
    });

    app.run()?;
    Ok(())
}

/// Spawns a sync in the background, unless no remote is linked yet -- then
/// it just opens the sheet to ask for one instead of failing.
fn start_sync(app: &App, session: &Rc<RefCell<Session>>) {
    let (vault_dir, gitdir, remote, token_store) = {
        let s = session.borrow();
        let Some(remote) = s.remote.clone() else {
            app.set_status("No remote linked yet".into());
            app.set_remote_url_draft(SharedString::new());
            app.set_remote_token_draft(SharedString::new());
            app.set_remote_has_token(false);
            app.set_remote_open(true);
            return;
        };
        let gitdir = match s.app_data.gitdir(&s.vault_dir) {
            Ok(g) => g,
            Err(e) => {
                drop(s);
                app.set_status(format!("Could not prepare the vault: {e}").into());
                return;
            }
        };
        (s.vault_dir.clone(), gitdir, remote, s.token_store.clone())
    };

    app.set_syncing(true);
    app.set_status("Syncing...".into());
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

fn refresh_list(app: &App, session: &Rc<RefCell<Session>>) {
    let mut s = session.borrow_mut();
    let previous = s.current_path().cloned();
    s.notes = notes::scan(&s.vault_dir);
    s.conflicted = s
        .notes
        .iter()
        .map(|p| notes::has_conflict_marker(p))
        .collect();
    s.current = previous.and_then(|p| s.notes.iter().position(|n| *n == p));

    let names: Vec<SharedString> = s
        .notes
        .iter()
        .map(|p| notes::display_name(&s.vault_dir, p).into())
        .collect();
    app.set_notes(ModelRc::new(VecModel::from(names)));
    app.set_conflicted(ModelRc::new(VecModel::from(s.conflicted.clone())));
    app.set_current(s.current.map_or(-1, |i| i as i32));
}

fn refresh_vault_list(app: &App, session: &Rc<RefCell<Session>>) {
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

fn vault_display_name(vault_dir: &std::path::Path) -> String {
    vault_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| vault_dir.display().to_string())
}

/// Switches to `vault_dir`, reloading everything that's per-vault: the
/// token store, the remote URL, the note list, and the open editor
/// (there's no note from the old vault that still makes sense to show).
fn switch_vault(app: &App, session: &Rc<RefCell<Session>>, vault_dir: PathBuf) {
    if let Err(e) = std::fs::create_dir_all(&vault_dir) {
        app.set_status(format!("Could not open this vault: {e}").into());
        return;
    }
    let token_store = {
        let s = session.borrow();
        match (s.token_store_for)(&vault_dir) {
            Ok(t) => t,
            Err(e) => {
                drop(s);
                app.set_status(format!("Could not open this vault: {e}").into());
                return;
            }
        }
    };

    let mut s = session.borrow_mut();
    let remote = s.app_data.load_remote(&vault_dir);
    let _ = s.app_data.save_current_vault(&vault_dir);
    s.vault_dir = vault_dir;
    s.token_store = token_store;
    s.remote = remote.clone();
    s.current = None;
    s.history = None;
    drop(s);

    app.set_remote_configured(remote.is_some());
    app.set_current_title(SharedString::new());
    app.set_body(SharedString::new());
    show_history_state(app, &session.borrow());
    app.set_list_open(true);
    refresh_list(app, session);
    refresh_vault_list(app, session);
}

/// Turns what the user typed in the "Add vault" dialog into an actual
/// vault directory -- desktop takes it as a path outright; Android has no
/// external folder access in this build, so it names a subfolder of the
/// same private storage the app already uses.
#[cfg(target_os = "android")]
fn vault_path_from_input(base: &std::path::Path, input: &str) -> anyhow::Result<PathBuf> {
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
fn vault_path_from_input(_base: &std::path::Path, input: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!input.is_empty(), "the path can't be empty");
    Ok(PathBuf::from(input))
}

fn open_note(app: &App, session: &Rc<RefCell<Session>>, index: usize) {
    let mut s = session.borrow_mut();
    let Some(path) = s.notes.get(index).cloned() else {
        return;
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            s.current = Some(index);
            s.history = Some(History::new(&text));
            app.set_current(index as i32);
            app.set_current_title(notes::display_name(&s.vault_dir, &path).into());
            app.set_current_has_conflict(s.conflicted[index]);
            show_history_state(app, &s);
            app.set_body(text.into());
            app.set_status(SharedString::new());
        }
        Err(e) => app.set_status(format!("Could not open {}: {e}", path.display()).into()),
    }
}

/// Tells the window whether the undo and redo buttons have anything to do.
fn show_history_state(app: &App, session: &Session) {
    let history = session.history.as_ref();
    app.set_can_undo(history.is_some_and(History::can_undo));
    app.set_can_redo(history.is_some_and(History::can_redo));
}

fn apply_restored(app: &App, session: &Rc<RefCell<Session>>, restored: Option<Restored>) {
    let Some(Restored { text, cursor }) = restored else {
        return;
    };
    if let Some(path) = session.borrow().current_path() {
        if let Err(e) = std::fs::write(path, &text) {
            app.set_status(format!("Save failed: {e}").into());
        }
    }
    app.set_body(text.into());
    app.invoke_set_cursor(cursor as i32);
    show_history_state(app, &session.borrow());
}

fn finish_sync(app: &App, session: &Rc<RefCell<Session>>, result: Result<Vec<PathBuf>, String>) {
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
                    }
                }
            }
            app.set_status(
                if needing_resolution.is_empty() {
                    "Synced".to_owned()
                } else {
                    format!(
                        "Synced; {} note(s) need resolution",
                        needing_resolution.len()
                    )
                }
                .into(),
            );
        }
        Err(e) => app.set_status(format!("Sync failed: {e}").into()),
    }
}

/// Android starts the app here (the activity loads this library), not at
/// `main`. Everything -- notes, the private gitdir, the encrypted token --
/// lives under the app's private storage; Android has no `HOME` or
/// `XDG_DATA_HOME` for the desktop paths to fall back on.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    let base = app
        .internal_data_path()
        .expect("the app has private storage");
    let default_vault_dir = base.join("notes");
    let app_data = AppData::new(base.clone());
    let vm_ptr = app.vm_as_ptr();
    let token_store_for: TokenStoreFactory = Box::new(move |vault_dir| {
        let path = app_data.token_path(vault_dir)?;
        // Safety: `vm_ptr` is `app.vm_as_ptr()`, valid for the process's
        // lifetime; this closure doesn't outlive it.
        let store = unsafe { android_keystore::AndroidKeystoreTokenStore::new(vm_ptr, path) }?;
        Ok(Arc::new(store) as Arc<dyn TokenStore>)
    });
    slint::android::init(app).expect("initialize the Android backend");
    run(default_vault_dir, base, token_store_for).expect("run the app");
}
