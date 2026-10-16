//! The vault list: switching, adding, removing and copying a vault's path.
//!
//! The vault sheet (`App`'s callbacks) and the Settings sheet's vault page
//! (`SettingsAdapter`'s) offer the same actions, each through its own pair of
//! callbacks; both pairs end up in the functions below.

use crate::session::SessionRef;

use slint::{ComponentHandle, SharedString};

use crate::session::{
    copy_to_clipboard, refresh_vault_list, set_error, set_status, switch_vault,
    vault_path_from_input,
};
use crate::{App, SettingsAdapter};

fn open_vault_sheet(app: &App, session: &SessionRef) {
    refresh_vault_list(app, session);
    app.set_vault_sheet_open(true);
}

fn request_add_vault(app: &App) {
    app.set_add_vault_draft(SharedString::new());
    app.set_add_vault_open(true);
}

/// Asks for confirmation before removing the vault at `index` -- unless it
/// is the only one, which cannot be removed.
fn request_delete_vault(app: &App, session: &SessionRef, index: i32) {
    if session.borrow().known_vaults.len() <= 1 {
        return;
    }
    session.borrow_mut().delete_vault_target = Some(index as usize);
    app.set_delete_vault_confirm_open(true);
}

fn known_vault(session: &SessionRef, index: i32) -> Option<std::path::PathBuf> {
    session.borrow().known_vaults.get(index as usize).cloned()
}

fn add_vault(app: &App, session: &SessionRef, input: &str) {
    let base = session.borrow().app_data.base().to_owned();
    let result = vault_path_from_input(&base, input).and_then(|dir| {
        session.borrow_mut().register_vault(&dir)?;
        Ok(dir)
    });
    match result {
        Ok(dir) => switch_vault(app, session, dir),
        Err(e) => set_error(app, format!("Could not add the vault: {e}")),
    }
}

fn delete_confirmed_vault(app: &App, session: &SessionRef) {
    let Some(index) = session.borrow_mut().delete_vault_target.take() else {
        return;
    };
    let Some(dir) = known_vault(session, index as i32) else {
        return;
    };
    let result = session.borrow_mut().forget_vault(&dir);
    match result {
        Ok(Some(next)) => switch_vault(app, session, next),
        Ok(None) => refresh_vault_list(app, session),
        // The only vault was asked to go: nothing happens.
        Err(_) if session.borrow().known_vaults.len() <= 1 => {}
        Err(e) => set_error(app, format!("Could not remove the vault: {e}")),
    }
}

/// Binds the vault sheet and the Settings sheet's vault list.
pub fn bind_vault_callbacks(app: &App, session: &SessionRef) {
    let settings = app.global::<SettingsAdapter>();

    app.on_copy_vault_path({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let Some(app) = weak.upgrade() else { return };
            let path = session.borrow().vault_dir.display().to_string();
            copy_to_clipboard(&app, &path);
            set_status(&app, "Vault path copied to clipboard");
        }
    });

    // The Settings sheet's vault page.
    settings.on_open_vaults({
        let (weak, session) = (app.as_weak(), session.clone());
        move || open_vault_sheet(&weak.unwrap(), &session)
    });
    settings.on_switch_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            if let Some(dir) = known_vault(&session, index) {
                switch_vault(&app, &session, dir);
                refresh_vault_list(&app, &session);
            }
        }
    });
    settings.on_add_vault({
        let weak = app.as_weak();
        move || request_add_vault(&weak.unwrap())
    });
    settings.on_delete_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| request_delete_vault(&weak.unwrap(), &session, index)
    });

    // The vault sheet.
    app.on_open_vault_sheet({
        let (weak, session) = (app.as_weak(), session.clone());
        move || open_vault_sheet(&weak.unwrap(), &session)
    });
    app.on_switch_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            app.set_vault_sheet_open(false);
            if let Some(dir) = known_vault(&session, index) {
                switch_vault(&app, &session, dir);
            }
        }
    });
    app.on_add_vault_requested({
        let weak = app.as_weak();
        move || request_add_vault(&weak.unwrap())
    });
    app.on_delete_vault_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| request_delete_vault(&weak.unwrap(), &session, index)
    });

    // Both sheets' confirmation dialogs.
    app.on_confirm_add_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move |input| {
            let app = weak.unwrap();
            app.set_add_vault_open(false);
            add_vault(&app, &session, &input);
        }
    });
    app.on_confirm_delete_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            app.set_delete_vault_confirm_open(false);
            delete_confirmed_vault(&app, &session);
        }
    });
}
