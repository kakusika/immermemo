//! Device-to-device transfer: QR export, URI import, clipboard payload handling.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, SharedString};

use crate::session::{self, Session};
use crate::update_provider_ui_state;
use crate::{App, SettingsAdapter};

thread_local! {
    static APP_WEAK: RefCell<Option<slint::Weak<App>>> = const { RefCell::new(None) };
    static PENDING_IMPORT_PAYLOAD: RefCell<Option<Rc<RefCell<Option<immermemo_core::transfer::TransferPayload>>>>> = const { RefCell::new(None) };
}

/// Populates and displays the transfer import preview in SettingsSheet given a transfer URI or JSON.
pub(crate) fn open_import_preview(
    app: &App,
    pending_payload: &Rc<RefCell<Option<immermemo_core::transfer::TransferPayload>>>,
    input: &str,
) {
    let settings = app.global::<SettingsAdapter>();
    let trimmed = input.trim();
    settings.set_import_draft(trimmed.into());
    match immermemo_core::transfer::decode_payload(trimmed) {
        Ok(payload) => {
            settings.set_import_status_message(SharedString::new());
            settings.set_import_status_is_error(false);
            settings.set_preview_has_interface(payload.interface.is_some());
            let has_remote = payload
                .sync
                .as_ref()
                .and_then(|s| s.remote_url.as_ref())
                .is_some();
            let remote_url = payload
                .sync
                .as_ref()
                .and_then(|s| s.remote_url.clone())
                .unwrap_or_default();
            let has_token = payload
                .sync
                .as_ref()
                .and_then(|s| s.token.as_ref())
                .is_some();
            settings.set_preview_has_remote(has_remote);
            settings.set_preview_remote_url(remote_url.into());
            settings.set_preview_has_token(has_token);
            settings.set_import_preview_ready(true);
            *pending_payload.borrow_mut() = Some(payload);
        }
        Err(e) => {
            settings.set_import_preview_ready(false);
            settings.set_import_status_is_error(true);
            settings.set_import_status_message(format!("Could not read transfer data: {e}").into());
        }
    }
    // Sub-page 5: Transfer & Share, Tab 1: Import
    app.set_settings_page(5);
    settings.set_transfer_tab(1);
    app.set_settings_open(true);
}

/// Called when a deep-link URI (`immermemo://...`) is received either at launch or during runtime.
pub fn handle_incoming_import_uri(uri: &str) {
    let weak = APP_WEAK.with(|a| a.borrow().clone());
    let pending = PENDING_IMPORT_PAYLOAD.with(|p| p.borrow().clone());
    if let (Some(weak), Some(pending)) = (weak, pending)
        && let Some(app) = weak.upgrade()
    {
        open_import_preview(&app, &pending, uri);
    }
}

/// Binds transfer QR generation, link copying, import preview, and payload application callbacks.
pub fn bind_transfer_callbacks(
    app: &App,
    session: &Rc<RefCell<Session>>,
    initial_import_uri: Option<String>,
) {
    let cached_export_uri = Rc::new(RefCell::new(None::<String>));
    let pending_imported_payload = Rc::new(RefCell::new(
        None::<immermemo_core::transfer::TransferPayload>,
    ));
    APP_WEAK.with(|a| *a.borrow_mut() = Some(app.as_weak()));
    PENDING_IMPORT_PAYLOAD.with(|p| *p.borrow_mut() = Some(pending_imported_payload.clone()));

    let update_transfer_qr = {
        let cached_export_uri = cached_export_uri.clone();
        move |app: &App, session: &Rc<RefCell<Session>>| {
            let settings = app.global::<SettingsAdapter>();
            let options = immermemo_core::transfer::ExportOptions {
                include_interface: settings.get_export_include_interface(),
                include_remote: settings.get_export_include_remote(),
                include_token: settings.get_export_include_token(),
            };
            let s = session.borrow();
            let vault_name = Some(session::vault_display_name(&s.vault_dir));
            let ui_scale = Some(app.get_ui_scale());
            let payload_res = immermemo_core::transfer::build_payload(
                &s.app_data,
                &s.vault_dir,
                &*s.token_store,
                &options,
                vault_name,
                ui_scale,
            );
            drop(s);
            match payload_res {
                Ok(payload) => match immermemo_core::transfer::encode_payload(&payload) {
                    Ok(uri) => {
                        *cached_export_uri.borrow_mut() = Some(uri.clone());
                        match crate::qr::generate_qr_image(&uri, 260) {
                            Ok(img) => settings.set_qr_image(img),
                            Err(_) => settings.set_qr_image(slint::Image::default()),
                        }
                    }
                    Err(_) => settings.set_qr_image(slint::Image::default()),
                },
                Err(_) => settings.set_qr_image(slint::Image::default()),
            }
        }
    };

    let settings = app.global::<SettingsAdapter>();

    settings.on_refresh_transfer_qr({
        let (weak, session, update_qr) =
            (app.as_weak(), session.clone(), update_transfer_qr.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            settings.set_transfer_copied(false);
            update_qr(&app, &session);
        }
    });

    settings.on_export_options_changed({
        let (weak, session, update_qr) = (app.as_weak(), session.clone(), update_transfer_qr);
        move |include_interface, include_remote, include_token| {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            settings.set_export_include_interface(include_interface);
            settings.set_export_include_remote(include_remote);
            settings.set_export_include_token(include_token);
            settings.set_transfer_copied(false);
            update_qr(&app, &session);
        }
    });

    settings.on_copy_transfer_link({
        let (weak, cached_export_uri) = (app.as_weak(), cached_export_uri);
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            if let Some(uri) = cached_export_uri.borrow().as_deref() {
                session::copy_to_clipboard(&app, uri);
                settings.set_transfer_copied(true);
            }
        }
    });

    settings.on_paste_import_clipboard({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            if let Some(text) = session::read_from_clipboard(&app) {
                settings.set_import_draft(text.trim().into());
            }
        }
    });

    settings.on_review_import({
        let (weak, pending_payload) = (app.as_weak(), pending_imported_payload.clone());
        move |input| {
            let app = weak.unwrap();
            open_import_preview(&app, &pending_payload, &input);
        }
    });

    settings.on_cancel_import_preview({
        let (weak, pending_payload) = (app.as_weak(), pending_imported_payload.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            settings.set_import_preview_ready(false);
            *pending_payload.borrow_mut() = None;
        }
    });

    settings.on_apply_import({
        let (weak, session, pending_payload) = (
            app.as_weak(),
            session.clone(),
            pending_imported_payload.clone(),
        );
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let payload = pending_payload.borrow_mut().take();
            let Some(payload) = payload else {
                return;
            };
            let mut s = session.borrow_mut();
            match immermemo_core::transfer::apply_payload(
                &s.app_data,
                &s.vault_dir,
                &*s.token_store,
                &payload,
            ) {
                Ok(result) => {
                    settings.set_import_preview_ready(false);
                    settings.set_import_status_is_error(false);
                    settings.set_import_status_message("Settings applied successfully!".into());
                    if result.interface_updated {
                        if let Some(theme) = s.app_data.load_theme() {
                            settings.set_theme_choice(theme);
                            app.set_theme_choice(theme);
                        }
                        if let Some(font_choice) = s.app_data.load_font_size() {
                            settings.set_font_size_choice(font_choice);
                            app.set_font_size_choice(font_choice);
                            app.set_editor_font_size(match font_choice {
                                0 => 14.0,
                                2 => 20.0,
                                _ => 17.0,
                            });
                        }
                        if let Some(status_bar) = s.app_data.load_status_bar_choice() {
                            settings.set_status_bar_choice(status_bar);
                            app.set_status_bar_choice(status_bar);
                        }
                        if let Some(interface) = &payload.interface
                            && let Some(scale) = interface.ui_scale
                        {
                            app.set_ui_scale(scale);
                        }
                    }
                    if result.remote_updated {
                        s.remote = s.app_data.load_remote(&s.vault_dir);
                        app.set_remote_configured(s.remote.is_some());
                        update_provider_ui_state(&app, s.remote.as_deref());
                        settings.set_url_draft(s.remote.clone().unwrap_or_default().into());
                    }
                    if result.token_updated {
                        settings.set_has_token(true);
                    }
                }
                Err(e) => {
                    settings.set_import_status_is_error(true);
                    settings
                        .set_import_status_message(format!("Failed to apply settings: {e}").into());
                }
            }
        }
    });

    if let Some(uri) = initial_import_uri {
        open_import_preview(app, &pending_imported_payload, &uri);
    }
}
