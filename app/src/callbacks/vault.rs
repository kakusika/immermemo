//! Vault management, settings sheet, and app preferences callback bindings.

#[cfg(target_os = "android")]
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, SharedString};

use crate::App;
use crate::haptic;
use crate::render;
use crate::update_provider_ui_state;
use crate::session::{
    Session, refresh_vault_list, set_status, start_sync, switch_vault, vault_display_name,
    vault_path_from_input,
};

#[cfg(target_os = "android")]
pub fn status_bar_visible_for(choice: i32, note_sheet_open: bool) -> bool {
    match choice {
        1 => !note_sheet_open,
        2 => false,
        _ => true,
    }
}

pub fn bind_vault_callbacks(
    app: &App,
    session: &Rc<RefCell<Session>>,
    #[cfg(target_os = "android")] status_bar_choice_cell: Rc<Cell<i32>>,
) {
    app.on_open_settings({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let s = session.borrow();
            app.set_remote_url_draft(s.remote.clone().unwrap_or_default().into());
            app.set_remote_token_draft(SharedString::new());
            app.set_remote_has_token(s.token_store.load().unwrap_or_default().is_some());
            app.set_current_vault_name(vault_display_name(&s.vault_dir).into());
            app.set_current_vault_path(s.vault_dir.display().to_string().into());
            update_provider_ui_state(&app, s.remote.as_deref());
            app.set_settings_page(0);
            app.set_settings_open(true);
        }
    });
    app.on_close_settings({
        let weak = app.as_weak();
        move || weak.unwrap().set_settings_open(false)
    });

    app.on_font_size_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |choice| {
            let app = weak.unwrap();
            let px = match choice {
                0 => 14.0,
                2 => 20.0,
                _ => 17.0,
            };
            app.set_editor_font_size(px);
            let body = app.get_body();
            let max_width = app.get_body_content_width();
            app.set_note_body_items(render::note_body_items(body.as_str(), &app, max_width));
            let _ = session.borrow().app_data.save_font_size(choice);
        }
    });
    app.on_theme_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |choice| {
            let _ = session.borrow().app_data.save_theme(choice);
            let app = weak.unwrap();
            let body = app.get_body();
            let max_width = app.get_body_content_width();
            app.set_note_body_items(render::note_body_items(body.as_str(), &app, max_width));
        }
    });
    app.on_status_bar_changed({
        let session = session.clone();
        #[cfg(target_os = "android")]
        let (weak, status_bar_choice_cell) = (app.as_weak(), status_bar_choice_cell.clone());
        move |choice| {
            let _ = session.borrow().app_data.save_status_bar_choice(choice);
            #[cfg(target_os = "android")]
            {
                status_bar_choice_cell.set(choice);
                let app = weak.unwrap();
                crate::android::system_ui::set_status_bar_visible(status_bar_visible_for(
                    choice,
                    app.get_note_sheet_open(),
                ));
            }
        }
    });
    #[cfg(target_os = "android")]
    app.on_note_sheet_open_changed({
        let status_bar_choice_cell = status_bar_choice_cell.clone();
        move |open| {
            crate::android::system_ui::set_status_bar_visible(status_bar_visible_for(
                status_bar_choice_cell.get(),
                open,
            ));
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
            app.set_current_vault_name(vault_display_name(&s.vault_dir).into());
            app.set_current_vault_path(s.vault_dir.display().to_string().into());
            update_provider_ui_state(&app, s.remote.as_deref());
            app.set_settings_page(2);
            app.set_settings_open(true);
        }
    });
    app.on_trigger_haptic(|| {
        haptic::perform_haptic();
    });
    app.on_trigger_heavy_haptic(|| {
        haptic::perform_heavy_haptic();
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
                    set_status(&app, format!("Could not add the vault: {e}"));
                    return;
                }
            };
            if let Err(e) = std::fs::create_dir_all(&new_vault) {
                set_status(&app, format!("Could not add the vault: {e}"));
                return;
            }
            let mut s = session.borrow_mut();
            if let Err(e) = s.app_data.add_vault(&new_vault) {
                drop(s);
                set_status(&app, format!("Could not add the vault: {e}"));
                return;
            }
            s.known_vaults = s.app_data.known_vaults();
            drop(s);
            switch_vault(&app, &session, new_vault);
        }
    });
    app.on_delete_vault_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |index| {
            let app = weak.unwrap();
            let s = session.borrow();
            if s.known_vaults.len() <= 1 {
                return;
            }
            drop(s);
            session.borrow_mut().delete_vault_target = Some(index as usize);
            app.set_delete_vault_confirm_open(true);
        }
    });
    app.on_confirm_delete_vault({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            app.set_delete_vault_confirm_open(false);
            let Some(index) = session.borrow_mut().delete_vault_target.take() else {
                return;
            };
            let Some(vault_dir) = session.borrow().known_vaults.get(index).cloned() else {
                return;
            };
            let mut s = session.borrow_mut();
            if s.known_vaults.len() <= 1 {
                return;
            }
            let is_current = s.vault_dir == vault_dir;
            if let Err(e) = s.app_data.remove_vault(&vault_dir) {
                drop(s);
                set_status(&app, format!("Could not remove the vault: {e}"));
                return;
            }
            s.known_vaults = s.app_data.known_vaults();
            let next_vault = if is_current {
                s.known_vaults.first().cloned()
            } else {
                None
            };
            drop(s);
            if let Some(next) = next_vault {
                switch_vault(&app, &session, next);
            } else {
                refresh_vault_list(&app, &session);
            }
        }
    });

    app.on_save_remote({
        let (weak, session) = (app.as_weak(), session.clone());
        move |url, token| {
            let app = weak.unwrap();
            let url = url.trim();
            if url.is_empty() {
                set_status(&app, "Enter a remote URL first");
                return;
            }
            let mut s = session.borrow_mut();
            if let Err(e) = s.app_data.save_remote(&s.vault_dir, url) {
                drop(s);
                set_status(&app, format!("Could not save the remote: {e}"));
                return;
            }
            if !token.is_empty()
                && let Err(e) = s.token_store.save(&token)
            {
                drop(s);
                set_status(&app, format!("Could not save the access token: {e}"));
                return;
            }
            s.remote = Some(url.to_owned());
            drop(s);
            app.set_remote_configured(true);
            update_provider_ui_state(&app, Some(url));
            app.set_settings_open(false);
            start_sync(&app, &session);
        }
    });
}
