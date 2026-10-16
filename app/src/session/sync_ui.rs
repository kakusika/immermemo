//! Background sync runner, completion handler, and debounced auto-sync scheduler.

use crate::session::SessionRef;
use std::cell::RefCell;

use slint::{ComponentHandle, SharedString};

use crate::App;
#[cfg(any(target_os = "android", target_os = "ios"))]
use crate::platform::cert;
use immermemo_core::format_time_only;

use super::util::{set_error, set_status};
use super::{
    SESSION, close_current_note_and_show_list_or_reopen, open_note, refresh_list,
    show_list_or_reopen,
};

thread_local! {
    static AUTO_SYNC_TIMER: RefCell<slint::Timer> = RefCell::new(slint::Timer::default());
}

/// Spawns a sync in the background, unless no remote is linked yet -- then
/// it just opens the sheet to ask for one instead of failing.
pub fn start_sync(app: &App, session: &SessionRef) {
    if app.get_syncing() {
        return;
    }
    let job = {
        let s = session.borrow();
        match s.prepare_sync() {
            Ok(Some(job)) => job,
            Ok(None) => {
                set_status(app, "This vault does not sync yet");
                // Offer to set that up.
                app.invoke_open_sync_settings();
                return;
            }
            Err(e) => {
                drop(s);
                set_error(app, format!("Could not prepare the vault: {e}"));
                return;
            }
        }
    };

    app.set_syncing(true);
    app.set_last_sync_error(SharedString::new());
    app.set_sync_error_copied(false);
    set_status(app, "Syncing...");
    let weak = app.as_weak();
    std::thread::spawn(move || {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        let verifier = cert::verifier().map(Some);
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        let verifier: anyhow::Result<Option<Box<dyn immermemo_core::CertificateVerifier>>> =
            Ok(None);
        let result = verifier
            .and_then(|verifier| job.run(verifier))
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
    session: &SessionRef,
    result: Result<immermemo_core::SyncReport, String>,
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
            set_error(app, error_msg);
        }
    }
    if should_sync_again {
        trigger_auto_sync(app, session);
    }
}

/// Triggers an automatic sync in the background if a remote is configured and no sync is running.
/// If a sync is already running, sets `pending_auto_sync` so another sync will run after it completes.
pub fn trigger_auto_sync(app: &App, session: &SessionRef) {
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
