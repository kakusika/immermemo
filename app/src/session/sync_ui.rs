//! Background sync runner, completion handler, and debounced auto-sync scheduler.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, SharedString};

use crate::credentials::TokenCredentials;
use crate::sync;
use crate::App;
#[cfg(any(target_os = "android", target_os = "ios"))]
use crate::cert;

use super::util::{format_time_only, set_status};
use super::{
    SESSION, Session, close_current_note_and_show_list_or_reopen, open_note, refresh_list,
    show_list_or_reopen, vault_display_name,
};

thread_local! {
    static AUTO_SYNC_TIMER: RefCell<slint::Timer> = RefCell::new(slint::Timer::default());
}

/// Spawns a sync in the background, unless no remote is linked yet -- then
/// it just opens the sheet to ask for one instead of failing.
pub fn start_sync(app: &App, session: &Rc<RefCell<Session>>) {
    if app.get_syncing() {
        return;
    }
    let (vault_dir, gitdir, remote, token_store, identity) = {
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
        let identity =
            immermemo_identity::DeviceIdentity::load_or_init(&s.app_data.identity_path()).ok();
        (
            s.vault_dir.clone(),
            gitdir,
            remote,
            s.token_store.clone(),
            identity,
        )
    };

    app.set_syncing(true);
    app.set_last_sync_error(SharedString::new());
    app.set_sync_error_copied(false);
    set_status(app, "Syncing...");
    let weak = app.as_weak();
    std::thread::spawn(move || {
        let credentials = TokenCredentials::new(token_store);
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let verifier = cert::verifier().map(Some);
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let verifier: anyhow::Result<Option<Box<dyn immermemo_sync::CertificateVerifier>>> =
            Ok(None);
        let result = verifier
            .and_then(|verifier| {
                sync::run(
                    &vault_dir,
                    &gitdir,
                    &remote,
                    &credentials,
                    verifier,
                    identity,
                )
            })
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
