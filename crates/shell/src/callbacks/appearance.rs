//! Appearance, bottom-bar and device-feedback callbacks.

#[cfg(target_os = "android")]
use std::cell::Cell;
#[cfg(target_os = "android")]
use std::rc::Rc;

use slint::{ComponentHandle, Model};

use crate::platform::haptic;
use crate::render;
use crate::session::SessionRef;
use crate::{App, SettingsAdapter};

#[cfg(target_os = "android")]
pub fn status_bar_visible_for(choice: i32, note_sheet_open: bool) -> bool {
    match choice {
        1 => !note_sheet_open,
        2 => false,
        _ => true,
    }
}

/// Binds the Settings controls that change how the app looks and feels.
pub fn bind_appearance_callbacks(
    app: &App,
    session: &SessionRef,
    #[cfg(target_os = "android")] status_bar_choice_cell: Rc<Cell<i32>>,
) {
    let settings = app.global::<SettingsAdapter>();

    settings.on_font_size_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |choice| {
            let app = weak.unwrap();
            app.set_editor_font_size(crate::theme::editor_font_px(choice));
            let body = app.get_body();
            let max_width = app.get_body_content_width();
            let base = crate::session::media_base(&app, &session);
            app.set_note_body_items(render::note_body_items(
                body.as_str(),
                &app,
                max_width,
                &base,
            ));
            let _ = session.borrow().app_data.save_font_size(choice);
        }
    });

    settings.on_theme_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |choice| {
            let session_borrow = session.borrow();
            let _ = session_borrow.app_data.save_theme(choice);
            let app = weak.unwrap();
            crate::theme::apply_theme(&app, &session_borrow.app_data);
            let body = app.get_body();
            let max_width = app.get_body_content_width();
            let base = crate::session::media_base(&app, &session);
            app.set_note_body_items(render::note_body_items(
                body.as_str(),
                &app,
                max_width,
                &base,
            ));
        }
    });

    settings.on_color_source_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |source| {
            let session_borrow = session.borrow();
            let _ = session_borrow.app_data.save_color_source(source);
            let app = weak.unwrap();
            crate::theme::apply_theme(&app, &session_borrow.app_data);
        }
    });

    settings.on_color_seed_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |seed| {
            let session_borrow = session.borrow();
            let _ = session_borrow.app_data.save_color_seed(seed.as_str());
            let app = weak.unwrap();
            crate::theme::apply_theme(&app, &session_borrow.app_data);
        }
    });

    settings.on_palette_style_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |style| {
            let session_borrow = session.borrow();
            let _ = session_borrow.app_data.save_palette_style(style);
            let app = weak.unwrap();
            crate::theme::apply_theme(&app, &session_borrow.app_data);
        }
    });

    settings.on_contrast_level_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |level| {
            let session_borrow = session.borrow();
            let _ = session_borrow.app_data.save_contrast_level(level);
            let app = weak.unwrap();
            crate::theme::apply_theme(&app, &session_borrow.app_data);
        }
    });

    settings.on_status_bar_changed({
        let session = session.clone();
        #[cfg(target_os = "android")]
        let (weak, status_bar_choice_cell) = (app.as_weak(), status_bar_choice_cell.clone());
        move |choice| {
            let _ = session.borrow().app_data.save_status_bar_choice(choice);
            #[cfg(target_os = "android")]
            {
                status_bar_choice_cell.set(choice);
                let app = weak.unwrap();
                crate::platform::android::system_ui::set_status_bar_visible(
                    status_bar_visible_for(choice, app.get_note_sheet_open()),
                );
            }
        }
    });

    let save_bottom_bar = |session: &std::rc::Rc<std::cell::RefCell<crate::session::Session>>,
                           settings: &SettingsAdapter| {
        let serialized = crate::action::serialize_bottom_bar_config(settings);
        let _ = session
            .borrow()
            .app_data
            .save_bottom_bar_config(&serialized);
    };

    settings.on_bottom_bar_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            save_bottom_bar(&session, &settings);
        }
    });

    settings.on_move_nav_item({
        let (weak, session) = (app.as_weak(), session.clone());
        move |from_idx, to_idx| {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let mut order: Vec<i32> = settings.get_nav_order().iter().collect();
            let len = order.len() as i32;
            if from_idx >= 0 && from_idx < len && to_idx >= 0 && to_idx < len && from_idx != to_idx
            {
                let item = order.remove(from_idx as usize);
                order.insert(to_idx as usize, item);
                settings.set_nav_order(std::rc::Rc::new(slint::VecModel::from(order)).into());
                save_bottom_bar(&session, &settings);
            }
        }
    });

    let save_home_actions = |session: &std::rc::Rc<std::cell::RefCell<crate::session::Session>>,
                             settings: &SettingsAdapter| {
        crate::action::update_home_enabled_actions(settings);
        let serialized = crate::action::serialize_home_actions_config(settings);
        let _ = session
            .borrow()
            .app_data
            .save_home_quick_actions_config(&serialized);
    };

    settings.on_home_actions_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            save_home_actions(&session, &settings);
        }
    });

    settings.on_move_home_action_item({
        let (weak, session) = (app.as_weak(), session.clone());
        move |from_idx, to_idx| {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            let mut order: Vec<i32> = settings.get_home_actions_order().iter().collect();
            let len = order.len() as i32;
            if from_idx >= 0 && from_idx < len && to_idx >= 0 && to_idx < len && from_idx != to_idx
            {
                let item = order.remove(from_idx as usize);
                order.insert(to_idx as usize, item);
                settings.set_home_actions_order(std::rc::Rc::new(slint::VecModel::from(order)).into());
                save_home_actions(&session, &settings);
            }
        }
    });

    #[cfg(target_os = "android")]
    app.on_note_sheet_open_changed({
        let status_bar_choice_cell = status_bar_choice_cell.clone();
        move |open| {
            crate::platform::android::system_ui::set_status_bar_visible(status_bar_visible_for(
                status_bar_choice_cell.get(),
                open,
            ));
        }
    });

    app.on_trigger_haptic(|| {
        haptic::perform_haptic();
    });

    app.on_trigger_heavy_haptic(|| {
        haptic::perform_heavy_haptic();
    });
}
