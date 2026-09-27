mod history;
mod notes;
mod sync;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use history::{History, Restored};

slint::include_modules!();

/// What the window is currently editing. Lives on the UI thread only.
struct Session {
    vault_dir: PathBuf,
    notes: Vec<PathBuf>,
    conflicted: Vec<bool>,
    current: Option<usize>,
    /// Dropped whenever the note changes from outside (a sync) or another
    /// note is opened, so undo never crosses into different content.
    history: Option<History>,
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

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let vault_dir = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("usage: immermemo <notes-dir> [remote-url]"))?,
    );
    let remote = args.next();
    std::fs::create_dir_all(&vault_dir)?;

    let app = App::new()?;
    let session = Rc::new(RefCell::new(Session {
        vault_dir: vault_dir.clone(),
        notes: Vec::new(),
        conflicted: Vec::new(),
        current: None,
        history: None,
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

    app.on_sync({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let Some(remote) = remote.clone() else {
                weak.unwrap()
                    .set_status("No remote: pass a remote URL as the second argument".into());
                return;
            };
            let app = weak.unwrap();
            app.set_syncing(true);
            app.set_status("Syncing...".into());
            let dir = session.borrow().vault_dir.clone();
            let weak = weak.clone();
            // Vault is opened inside the thread; only plain data crosses.
            std::thread::spawn(move || {
                let result = sync::run(&dir, &remote).map_err(|e| format!("{e:#}"));
                let _ = slint::invoke_from_event_loop(move || {
                    let app = weak.unwrap();
                    let session = SESSION
                        .with(|s| s.borrow().clone())
                        .expect("session set in main");
                    finish_sync(&app, &session, result);
                });
            });
        }
    });

    app.run()?;
    Ok(())
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
            app.set_current_has_conflict(s.conflicted[index]);
            app.set_body(text.into());
            app.set_status(SharedString::new());
        }
        Err(e) => app.set_status(format!("Could not open {}: {e}", path.display()).into()),
    }
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
