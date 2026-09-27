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
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use appdata::AppData;
use credentials::{TokenCredentials, TokenStore};
use history::{History, Restored};

slint::include_modules!();

/// What the window is currently editing. Lives on the UI thread only.
struct Session {
    vault_dir: PathBuf,
    app_data: AppData,
    token_store: Arc<dyn TokenStore>,
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

/// Opens the window on `vault_dir` and runs until it is closed.
///
/// `app_data_base` is this platform's private storage for everything that
/// isn't a note: the private gitdir, the remote URL, and (outside Android,
/// which keeps its own token behind the Keystore) the access token.
/// `token_store` is how the saved access token is read and written --
/// platform-specific, so the caller builds it.
pub fn run(
    vault_dir: PathBuf,
    app_data_base: PathBuf,
    token_store: Arc<dyn TokenStore>,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(&vault_dir)?;
    let app_data = AppData::new(app_data_base);
    let remote = app_data.load_remote();

    let app = App::new()?;
    app.set_remote_configured(remote.is_some());
    let session = Rc::new(RefCell::new(Session {
        vault_dir: vault_dir.clone(),
        app_data,
        token_store,
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
            if let Err(e) = s.app_data.save_remote(url) {
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
    let vault_dir = base.join("notes");
    let token_store: Arc<dyn TokenStore> = Arc::new(
        // Safety: `app.vm_as_ptr()` is exactly the pointer this constructor
        // requires, valid for the process's lifetime.
        unsafe {
            android_keystore::AndroidKeystoreTokenStore::new(app.vm_as_ptr(), base.join("token"))
        }
        .expect("open the Android Keystore"),
    );
    slint::android::init(app).expect("initialize the Android backend");
    run(vault_dir, base, token_store).expect("run the app");
}
