//! Starting the app: choosing what to open ([`immermemo_core::Session::start`]),
//! showing the saved preferences and the first notes, and binding every
//! callback before the window runs.

#[cfg(target_os = "android")]
use std::cell::Cell;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::ComponentHandle;

use crate::session::{
    SESSION, Session, SessionRef, load_cached_list, open_initial_or_last_note,
    open_note_by_rel_path, refresh_list, refresh_vault_list,
};
use crate::{
    App, SettingsAdapter, TokenStoreFactory, action, callbacks, provider, session, theme, transfer,
    update_provider_ui_state,
};
use immermemo_core::{AppData, StartVault};

/// Where a vault's access token would live under `app_data_base`, without
/// needing a running [`Session`] -- for a caller of [`run`] building its
/// token-store factory before the app (and its `AppData`) exists yet.
pub fn token_path_in(app_data_base: &Path, vault_dir: &Path) -> anyhow::Result<PathBuf> {
    AppData::new(app_data_base.to_owned()).token_path(vault_dir)
}

/// Development desktop app-data base: standard data directory on Linux
/// (`$XDG_DATA_HOME/immermemo`). `main.rs` passes this to [`run`]; mobile
/// platforms (Android, iOS) have their own private sandboxes and compute
/// their base in their own entry points.
pub fn desktop_app_data_dir() -> anyhow::Result<PathBuf> {
    let proj_dirs = directories::ProjectDirs::from("dev", "immermemo", "immermemo")
        .ok_or_else(|| anyhow::anyhow!("could not determine desktop data directory"))?;
    Ok(proj_dirs.data_dir().to_path_buf())
}

/// Switches the UI to whichever bundled translation (see
/// `translations/<lang>/LC_MESSAGES/immermemo.po`) matches the
/// system's locale, silently keeping the English source strings if none
/// matches -- there is no language picker yet, so this is the only way a
/// non-English translation ever gets used. Must run after `App::new()`
/// (the Slint translation machinery isn't ready any earlier).
fn select_system_translation() {
    let Some(locale) = sys_locale::get_locale() else {
        return;
    };
    let lang = locale.split(['-', '_']).next().unwrap_or(&locale);
    let _ = slint::select_bundled_translation(lang);
}

/// Opens the window and runs until it is closed.
///
/// Which vaults exist and which one is current come from `app_data_base`
/// (see [`immermemo_core::Session::start`]); `start` only adds to that: where
/// a vault is created when none can be opened (a fresh install), and a folder
/// the user asked to open. If the vault the app was on could not be opened,
/// the status line says so.
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
    start: StartVault,
    app_data_base: PathBuf,
    token_store_for: TokenStoreFactory,
    initial_note_path: Option<PathBuf>,
    initial_import_uri: Option<String>,
) -> anyhow::Result<()> {
    let core = immermemo_core::Session::start(AppData::new(app_data_base), token_store_for, start)?;

    let app = App::new()?;
    #[cfg(target_os = "android")]
    crate::platform::android::splash::notify_on_first_frame(&app);
    select_system_translation();
    let appearance = theme::Appearance::load(&core.app_data);
    show_saved_settings(&app, &core, &appearance);
    #[cfg(target_os = "android")]
    let status_bar_choice_cell = Rc::new(Cell::new(appearance.status_bar_choice));
    // 2 (always hidden) takes effect immediately; 0/1 don't need an
    // initial call -- the window starts visible either way (AppTheme's
    // own default), and 1 only hides once a note is actually opened.
    #[cfg(target_os = "android")]
    if appearance.status_bar_choice == 2 {
        crate::platform::android::system_ui::set_status_bar_visible(false);
    }

    crate::thumbs::init(&app, core.app_data.media_cache_dir());
    let session = Rc::new(RefCell::new(Session::new(core)));
    SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
    show_initial_notes(&app, &session, initial_note_path.as_deref());
    if let Some(notice) = session.borrow().startup_notice.clone() {
        session::set_error(&app, notice);
    }

    callbacks::bind_note_callbacks(&app, &session);
    callbacks::bind_appearance_callbacks(
        &app,
        &session,
        #[cfg(target_os = "android")]
        status_bar_choice_cell,
    );
    callbacks::bind_vault_callbacks(&app, &session);
    callbacks::bind_account_callbacks(&app, &session);
    callbacks::bind_sync_conflict_callbacks(&app, &session);
    callbacks::bind_selection_callbacks(&app, &session);
    provider::bind_provider_callbacks(&app, &session);
    transfer::bind_transfer_callbacks(&app, &session, initial_import_uri);

    app.run()?;
    Ok(())
}

/// Shows everything saved about how the app looks and what it is linked
/// to: appearance, the bottom bar, the vault and its remote, the SSH key.
fn show_saved_settings(app: &App, core: &immermemo_core::Session, appearance: &theme::Appearance) {
    let settings = app.global::<SettingsAdapter>();
    appearance.show(app);
    let bottom_bar_config = core
        .app_data
        .load_bottom_bar_config()
        .unwrap_or_else(|| "home,search,library,quick".to_string());
    action::apply_bottom_bar_config(&settings, &bottom_bar_config);
    let home_actions_config = core
        .app_data
        .load_home_quick_actions_config()
        .unwrap_or_else(|| "new_note".to_string());
    action::apply_home_actions_config(&settings, &home_actions_config);
    session::show_vault_identity(app, &core.vault_dir);
    app.set_remote_configured(core.remote.is_some());
    update_provider_ui_state(app, core.remote.as_deref());
    let ssh_pub = core.app_data.load_ssh_public_key();
    settings.set_ssh_key_present(ssh_pub.is_some());
    settings.set_ssh_public_key(ssh_pub.unwrap_or_default().into());
    settings.set_ssh_key_copied(false);
}

/// Lists the vault's notes (from the index first, so the window is useful
/// at once, then reconciled against the files) and opens one -- the one a
/// widget tap named, else the one open last time -- unless the vault is
/// empty. Schedules the first sync when a remote is linked.
fn show_initial_notes(app: &App, session: &SessionRef, initial_note_path: Option<&Path>) {
    let had_cached_notes = load_cached_list(app, session);
    if !had_cached_notes {
        refresh_list(app, session);
    }
    refresh_vault_list(app, session);
    if session.borrow().notes.is_empty() {
        app.set_list_open(true);
    } else {
        app.set_list_open(false);
        let opened_from_widget =
            initial_note_path.is_some_and(|rel| open_note_by_rel_path(app, session, rel));
        if !opened_from_widget {
            open_initial_or_last_note(app, session);
        }
    }
    if had_cached_notes {
        let (weak, session) = (app.as_weak(), session.clone());
        slint::Timer::single_shot(std::time::Duration::from_millis(50), move || {
            if let Some(app) = weak.upgrade() {
                refresh_list(&app, &session);
            }
        });
    }
    if session.borrow().remote.is_some() {
        session::schedule_auto_sync(app, std::time::Duration::from_millis(500));
    }
}
