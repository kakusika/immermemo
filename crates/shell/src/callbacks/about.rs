//! The About page: the buttons that copy the build and the system details.

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::session::{copy_to_clipboard, set_status};
use crate::{App, SettingsAdapter, SystemInfoRow, system_info};

/// Fills the System card and binds the About page's copy buttons.
pub fn bind_about_callbacks(app: &App) {
    let settings = app.global::<SettingsAdapter>();
    let rows = system_info::rows();
    settings.set_app_version(env!("CARGO_PKG_VERSION").into());
    settings.set_build_id(immermemo_buildinfo::label().into());
    settings.set_system_info(ModelRc::new(VecModel::from(
        rows.iter()
            .map(|row| SystemInfoRow {
                label: row.label.into(),
                value: row.value.as_str().into(),
            })
            .collect::<Vec<_>>(),
    )));

    settings.on_copy_build_id({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            copy_to_clipboard(&app, settings.get_build_id().as_str());
            settings.set_build_copied(true);
            set_status(&app, "Build copied to clipboard");
        }
    });

    settings.on_copy_system_info({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            let settings = app.global::<SettingsAdapter>();
            copy_to_clipboard(
                &app,
                &system_info::report(
                    settings.get_app_version().as_str(),
                    settings.get_build_id().as_str(),
                    &rows,
                ),
            );
            settings.set_system_info_copied(true);
            set_status(&app, "System details copied to clipboard");
        }
    });
}
