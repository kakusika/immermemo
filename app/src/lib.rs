pub mod cert;
pub mod credentials;
mod haptic;
pub mod qr;
pub mod render;
mod session;
mod sync;
mod widget;

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
use std::cell::Cell;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use immermemo_editor::History;
use immermemo_provider::{AuthPollStatus, GitHubProvider, Provider};
use immermemo_vault::appdata::AppData;
pub use immermemo_vault::credentials::{TokenStore, TokenStoreFactory};
use immermemo_vault::notes;

use session::{
    SESSION, Session, apply_icon_selection, apply_restored, filter_icon_picker,
    open_first_conflicted_note, open_icon_picker, open_initial_or_last_note, open_note,
    open_note_by_rel_path, refresh_list, refresh_vault_list, resolve_conflict_step, set_status,
    show_history_state, start_sync, switch_vault, sync_conflict_sheet_state, update_filtered_list,
    vault_display_name, vault_path_from_input,
};

slint::include_modules!();

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

/// Whether the status bar should be visible, given the Settings choice
/// (0: always show, 1: hide while editing a note, 2: always hide) and
/// whether the note editor sheet is currently open.
#[cfg(target_os = "android")]
fn status_bar_visible_for(choice: i32, note_sheet_open: bool) -> bool {
    match choice {
        1 => !note_sheet_open,
        2 => false,
        _ => true,
    }
}

/// Updates GitHub connected/account state in the UI based on the current remote URL.
pub(crate) fn update_github_ui_state(app: &App, remote: Option<&str>) {
    if let Some(url) = remote {
        if let Some((user, repo)) = GitHubProvider::parse_remote(url) {
            app.set_github_connected(true);
            app.set_github_username(user.into());
            app.set_github_repo_name(repo.into());
            return;
        }
    }
    app.set_github_connected(false);
    app.set_github_username(SharedString::new());
    app.set_github_repo_name(SharedString::new());
}

thread_local! {
    static APP_WEAK: RefCell<Option<slint::Weak<App>>> = const { RefCell::new(None) };
    static PENDING_IMPORT_PAYLOAD: RefCell<Option<Rc<RefCell<Option<immermemo_vault::transfer::TransferPayload>>>>> = const { RefCell::new(None) };
}

/// Populates and displays the transfer import preview in SettingsSheet given a transfer URI or JSON.
pub(crate) fn open_import_preview(
    app: &App,
    pending_payload: &Rc<RefCell<Option<immermemo_vault::transfer::TransferPayload>>>,
    input: &str,
) {
    let trimmed = input.trim();
    app.set_import_draft(trimmed.into());
    match immermemo_vault::transfer::decode_payload(trimmed) {
        Ok(payload) => {
            app.set_import_status_message(SharedString::new());
            app.set_import_status_is_error(false);
            app.set_preview_has_interface(payload.interface.is_some());
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
            app.set_preview_has_remote(has_remote);
            app.set_preview_remote_url(remote_url.into());
            app.set_preview_has_token(has_token);
            app.set_import_preview_ready(true);
            *pending_payload.borrow_mut() = Some(payload);
        }
        Err(e) => {
            app.set_import_preview_ready(false);
            app.set_import_status_is_error(true);
            app.set_import_status_message(format!("Could not read transfer data: {e}").into());
        }
    }
    // Sub-page 5: Transfer & Share, Tab 1: Import
    app.set_settings_page(5);
    app.set_settings_transfer_tab(1);
    app.set_settings_open(true);
}

/// Called when a deep-link URI (`immermemo://...`) is received either at launch or during runtime.
pub fn handle_incoming_import_uri(uri: &str) {
    let weak = APP_WEAK.with(|a| a.borrow().clone());
    let pending = PENDING_IMPORT_PAYLOAD.with(|p| p.borrow().clone());
    if let (Some(weak), Some(pending)) = (weak, pending) {
        if let Some(app) = weak.upgrade() {
            open_import_preview(&app, &pending, uri);
        }
    }
}

/// Opens the window and runs until it is closed.
///
/// `default_vault_dir` only matters the very first time the app ever
/// runs (nothing in `app_data_base` yet): it becomes the first known
/// vault, so a fresh desktop CLI invocation or a fresh Android install
/// still opens straight onto a vault the way a single-vault build used
/// to. After that, which vaults exist and which one is current come
/// entirely from `app_data_base`.
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
    default_vault_dir: PathBuf,
    app_data_base: PathBuf,
    token_store_for: TokenStoreFactory,
    initial_note_path: Option<PathBuf>,
    initial_import_uri: Option<String>,
) -> anyhow::Result<()> {
    let app_data = AppData::new(app_data_base);
    let mut known_vaults = app_data.known_vaults();
    if known_vaults.is_empty() {
        std::fs::create_dir_all(&default_vault_dir)?;
        app_data.add_vault(&default_vault_dir)?;
        known_vaults.push(default_vault_dir);
    }
    let vault_dir = app_data
        .load_current_vault()
        .filter(|v| known_vaults.contains(v))
        .unwrap_or_else(|| known_vaults[0].clone());
    std::fs::create_dir_all(&vault_dir)?;

    let token_store = token_store_for(&vault_dir)?;
    let remote = app_data.load_remote(&vault_dir);

    let app = App::new()?;
    #[cfg(target_os = "android")]
    android::splash::notify_on_first_frame(&app);
    select_system_translation();
    let font_size_choice = app_data.load_font_size().unwrap_or(1);
    let theme_choice = app_data.load_theme().unwrap_or(0);
    app.set_font_size_choice(font_size_choice);
    app.set_editor_font_size(match font_size_choice {
        0 => 14.0,
        2 => 20.0,
        _ => 17.0,
    });
    app.set_theme_choice(theme_choice);
    let status_bar_choice = app_data.load_status_bar_choice().unwrap_or(0);
    app.set_status_bar_choice(status_bar_choice);
    #[cfg(target_os = "android")]
    let status_bar_choice_cell = Rc::new(Cell::new(status_bar_choice));
    // 2 (always hidden) takes effect immediately; 0/1 don't need an
    // initial call -- the window starts visible either way (AppTheme's
    // own default), and 1 only hides once a note is actually opened.
    #[cfg(target_os = "android")]
    if status_bar_choice == 2 {
        android::system_ui::set_status_bar_visible(false);
    }
    app.set_current_vault_name(vault_display_name(&vault_dir).into());
    app.set_current_vault_path(vault_dir.display().to_string().into());
    app.set_remote_configured(remote.is_some());
    update_github_ui_state(&app, remote.as_deref());
    let index = app_data
        .index_path(&vault_dir)
        .ok()
        .and_then(|p| immermemo_index::NoteIndex::open(&p).ok())
        .unwrap_or_else(|| immermemo_index::NoteIndex::open_in_memory().unwrap());
    let core = immermemo_core::Session::new(
        vault_dir,
        app_data,
        token_store,
        token_store_for,
        known_vaults,
        remote,
        index,
    );
    let session = Rc::new(RefCell::new(Session::new(core)));

    SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
    refresh_list(&app, &session);
    refresh_vault_list(&app, &session);
    if session.borrow().notes.is_empty() {
        app.set_list_open(true);
    } else {
        app.set_list_open(false);
        let opened_from_widget = initial_note_path
            .as_deref()
            .is_some_and(|rel| open_note_by_rel_path(&app, &session, rel));
        if !opened_from_widget {
            open_initial_or_last_note(&app, &session);
        }
    }
    if session.borrow().remote.is_some() {
        session::schedule_auto_sync(&app, std::time::Duration::from_millis(500));
    }

    app.on_select({
        let (weak, session) = (app.as_weak(), session.clone());
        move |ui_index| {
            let app = weak.unwrap();
            let note_index = session
                .borrow()
                .filtered_results
                .get(ui_index as usize)
                .map(|r| r.note_index);
            if let Some(index) = note_index {
                open_note(&app, &session, index);
            }
        }
    });

    app.on_new_note({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let dir = session.borrow().vault_dir.clone();
            match notes::create(&dir) {
                Ok(path) => {
                    if let Ok(rel) = path.strip_prefix(&dir) {
                        let _ = session.borrow_mut().index.record_write(&dir, rel, "");
                    }
                    session.borrow_mut().search_query.clear();
                    app.set_search_query(SharedString::new());
                    refresh_list(&app, &session);
                    let index = session.borrow().notes.iter().position(|p| *p == path);
                    if let Some(index) = index {
                        open_note(&app, &session, index);
                    }
                }
                Err(e) => set_status(&app, format!("Could not create a note: {e}")),
            }
        }
    });

    app.on_navigate_directory_folder({
        let (weak, session) = (app.as_weak(), session.clone());
        move |key| {
            let app = weak.unwrap();
            session::navigate_directory_folder(&app, &session, key.as_str());
        }
    });

    app.on_edited({
        let (weak, session) = (app.as_weak(), session.clone());
        move |text| {
            let app = weak.unwrap();
            // `body` already changed via the TextInput's own two-way
            // binding -- only `note-body-items` needs recomputing here.
            let max_width = app.get_body_content_width();
            app.set_note_body_items(render::note_body_items(text.as_str(), &app, max_width));
            session::update_note_stats(&app, text.as_str());
            {
                let mut s = session.borrow_mut();
                if let Some(history) = s.editor.history.as_mut() {
                    history.edit(&text);
                }
                show_history_state(&app, &s);
                let has_conflict = text.contains(tomet::merge::CONFLICT_MARKER);
                if app.get_current_has_conflict() != has_conflict {
                    app.set_current_has_conflict(has_conflict);
                    if let Some(idx) = s.editor.current
                        && idx < s.conflicted.len()
                    {
                        s.conflicted[idx] = has_conflict;
                        let conflicted: Vec<bool> = s
                            .filtered_results
                            .iter()
                            .map(|r| s.conflicted[r.note_index])
                            .collect();
                        app.set_conflicted(ModelRc::new(VecModel::from(conflicted)));
                        let conflict_count = s.conflicted.iter().filter(|&&c| c).count();
                        app.set_conflict_count(conflict_count as i32);
                    }
                }
                if let Some(path) = s.current_path() {
                    match std::fs::write(&path, text.as_str()) {
                        Ok(_) => {
                            let vault_dir = s.vault_dir.clone();
                            if let Ok(rel) = path.strip_prefix(&vault_dir) {
                                let _ = s.index.record_write(&vault_dir, rel, text.as_str());
                                // Drop any cached history-neighbor preview
                                // for this note -- it's now stale (see
                                // session.rs's `history_preview_items`).
                                s.history_preview_cache.remove(&rel.display().to_string());
                            }
                            if app.get_status_is_error()
                                && app.get_status().starts_with("Save failed")
                            {
                                set_status(&app, "");
                            }
                        }
                        Err(e) => set_status(&app, format!("Save failed: {e}")),
                    }
                }
            }
            session::schedule_auto_sync(&app, std::time::Duration::from_secs(5));
        }
    });

    app.on_reflow_note_body({
        let weak = app.as_weak();
        move |max_width: f32| {
            let app = weak.unwrap();
            // The view-mode width changed (window resize, UI-scale
            // change, sidebar toggle, ...) -- `body` itself didn't, but a
            // flowed paragraph's line breaks depend on both.
            let body = app.get_body();
            app.set_note_body_items(render::note_body_items(body.as_str(), &app, max_width));
        }
    });

    app.on_undo({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let restored = session
                .borrow_mut()
                .editor
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
                .editor
                .history
                .as_mut()
                .and_then(History::redo);
            apply_restored(&weak.unwrap(), &session, restored);
        }
    });

    app.on_navigate_back({
        let (weak, session) = (app.as_weak(), session.clone());
        move || session::navigate_back(&weak.unwrap(), &session)
    });
    app.on_navigate_forward({
        let (weak, session) = (app.as_weak(), session.clone());
        move || session::navigate_forward(&weak.unwrap(), &session)
    });

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
            update_github_ui_state(&app, s.remote.as_deref());
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
                android::system_ui::set_status_bar_visible(status_bar_visible_for(
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
            android::system_ui::set_status_bar_visible(status_bar_visible_for(
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
            update_github_ui_state(&app, s.remote.as_deref());
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

    app.on_search_query_changed({
        let (weak, session) = (app.as_weak(), session.clone());
        move |query| {
            let app = weak.unwrap();
            session.borrow_mut().search_query = query.to_string();
            update_filtered_list(&app, &session);
        }
    });

    app.on_rename_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |ui_index| {
            let app = weak.unwrap();
            let mut s = session.borrow_mut();
            let Some(res) = s.filtered_results.get(ui_index as usize) else {
                return;
            };
            let note_index = res.note_index;
            let Some(path) = s.notes.get(note_index).cloned() else {
                return;
            };
            s.rename_target = Some(note_index);
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
            let was_current = session.borrow().current_path() == Some(path.clone());
            match notes::rename(&path, &new_name) {
                Ok(new_path) => {
                    let vault_dir = session.borrow().vault_dir.clone();
                    if let (Ok(old_rel), Ok(new_rel)) = (
                        path.strip_prefix(&vault_dir),
                        new_path.strip_prefix(&vault_dir),
                    ) {
                        let _ = session.borrow_mut().index.record_rename(old_rel, new_rel);
                    }
                    refresh_list(&app, &session);
                    if was_current {
                        let index = session.borrow().notes.iter().position(|p| *p == new_path);
                        if let Some(index) = index {
                            open_note(&app, &session, index);
                        }
                    }
                }
                Err(e) => set_status(&app, format!("Could not rename the note: {e}")),
            }
        }
    });

    app.on_icon_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |ui_index| {
            let app = weak.unwrap();
            let s = session.borrow();
            let Some(res) = s.filtered_results.get(ui_index as usize) else {
                return;
            };
            let note_index = res.note_index;
            drop(s);
            open_icon_picker(&app, &session, note_index);
        }
    });
    app.on_icon_picker_query_changed({
        let weak = app.as_weak();
        move |query| {
            let app = weak.unwrap();
            filter_icon_picker(&app, &query);
        }
    });
    app.on_icon_picker_select({
        let (weak, session) = (app.as_weak(), session.clone());
        move |name| {
            let app = weak.unwrap();
            apply_icon_selection(&app, &session, &name);
        }
    });

    app.on_delete_requested({
        let (weak, session) = (app.as_weak(), session.clone());
        move |ui_index| {
            let app = weak.unwrap();
            let note_index = session
                .borrow()
                .filtered_results
                .get(ui_index as usize)
                .map(|r| r.note_index);
            let Some(index) = note_index else {
                return;
            };
            session.borrow_mut().delete_target = Some(index);
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
            let vault_dir = session.borrow().vault_dir.clone();
            if let Err(e) = notes::delete(&path) {
                set_status(&app, format!("Could not delete the note: {e}"));
                return;
            }
            if let Ok(rel) = path.strip_prefix(&vault_dir) {
                let _ = session.borrow_mut().index.record_delete(rel);
            }
            let was_current = session.borrow().current_path() == Some(path.clone());
            refresh_list(&app, &session);
            if was_current {
                session::close_current_note_and_show_list_or_reopen(&app, &session, true);
            } else if session.borrow().notes.is_empty() {
                session::show_list_or_reopen(&app, &session, true);
            }
        }
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
            // An empty token field keeps whatever token is already saved --
            // the field starts blank on every open, so the user only retypes
            // it when actually changing it.
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
            update_github_ui_state(&app, Some(url));
            app.set_settings_open(false);
            start_sync(&app, &session);
        }
    });

    app.on_sync({
        let (weak, session) = (app.as_weak(), session.clone());
        move || start_sync(&weak.unwrap(), &session)
    });

    let active_auth_cancel = Rc::new(RefCell::new(None::<Arc<AtomicBool>>));

    app.on_open_github_browser({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                let uri = app.get_github_verification_uri();
                if !uri.is_empty() {
                    let _ = webbrowser::open(uri.as_str());
                }
            }
        }
    });

    app.on_cancel_github_auth({
        let (weak, cancel_flag) = (app.as_weak(), active_auth_cancel.clone());
        move || {
            if let Some(flag) = cancel_flag.borrow_mut().take() {
                flag.store(true, Ordering::SeqCst);
            }
            if let Some(app) = weak.upgrade() {
                app.set_github_auth_pending(false);
                app.set_github_user_code(SharedString::new());
                app.set_github_verification_uri(SharedString::new());
                app.set_github_auth_status(SharedString::new());
            }
        }
    });

    app.on_disconnect_github({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let mut s = session.borrow_mut();
            let vault_dir = s.vault_dir.clone();
            let _ = s.app_data.save_remote(&vault_dir, "");
            let _ = s.token_store.save("");
            s.remote = None;
            drop(s);
            app.set_remote_configured(false);
            app.set_remote_url_draft(SharedString::new());
            app.set_remote_token_draft(SharedString::new());
            app.set_remote_has_token(false);
            update_github_ui_state(&app, None);
            set_status(&app, "Disconnected GitHub repository");
        }
    });

    app.on_connect_github({
        let (weak, active_auth_cancel) = (app.as_weak(), active_auth_cancel.clone());
        move || {
            let app = weak.unwrap();
            if let Some(flag) = active_auth_cancel.borrow_mut().take() {
                flag.store(true, Ordering::SeqCst);
            }
            let cancel_flag = Arc::new(AtomicBool::new(false));
            *active_auth_cancel.borrow_mut() = Some(cancel_flag.clone());

            app.set_github_auth_pending(true);
            app.set_github_user_code(SharedString::new());
            app.set_github_verification_uri(SharedString::new());
            app.set_github_auth_status("Connecting to GitHub...".into());

            let thread_weak = weak.clone();
            std::thread::spawn(move || {
                let provider = GitHubProvider::default();
                let auth_session = match provider.start_auth() {
                    Ok(sess) => sess,
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = thread_weak.upgrade() {
                                app.set_github_auth_pending(false);
                                app.set_github_auth_status(
                                    format!("Connection failed: {e}").into(),
                                );
                            }
                        });
                        return;
                    }
                };

                let user_code = auth_session.user_code.clone();
                let uri = auth_session.verification_uri.clone();
                let weak_init = thread_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak_init.upgrade() {
                        app.set_github_user_code(user_code.into());
                        app.set_github_verification_uri(uri.clone().into());
                        app.set_github_auth_status("Waiting for browser authorization...".into());
                    }
                });

                let _ = webbrowser::open(&auth_session.verification_uri);

                let mut interval = std::time::Duration::from_secs(auth_session.interval.max(5));
                let token = loop {
                    std::thread::sleep(interval);
                    if cancel_flag.load(Ordering::SeqCst) {
                        return;
                    }

                    match provider.poll_auth(&auth_session) {
                        Ok(AuthPollStatus::Granted { access_token, .. }) => {
                            break access_token;
                        }
                        Ok(AuthPollStatus::Pending) => {}
                        Ok(AuthPollStatus::SlowDown) => {
                            interval += std::time::Duration::from_secs(5);
                        }
                        Ok(AuthPollStatus::Expired) => {
                            let weak = thread_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = weak.upgrade() {
                                    app.set_github_auth_pending(false);
                                    app.set_github_auth_status(
                                        "Authorization session expired. Please try again.".into(),
                                    );
                                }
                            });
                            return;
                        }
                        Ok(AuthPollStatus::AccessDenied) => {
                            let weak = thread_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = weak.upgrade() {
                                    app.set_github_auth_pending(false);
                                    app.set_github_auth_status(
                                        "Authorization was cancelled or denied on GitHub.".into(),
                                    );
                                }
                            });
                            return;
                        }
                        Err(e) => {
                            let weak = thread_weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = weak.upgrade() {
                                    app.set_github_auth_pending(false);
                                    app.set_github_auth_status(format!("Auth error: {e}").into());
                                }
                            });
                            return;
                        }
                    }
                };

                if cancel_flag.load(Ordering::SeqCst) {
                    return;
                }

                let weak_status = thread_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak_status.upgrade() {
                        app.set_github_auth_status("Setting up private repository...".into());
                    }
                });

                let repo_info = match provider.setup_repository(&token, "immermemo-notes") {
                    Ok(info) => info,
                    Err(e) => {
                        let weak = thread_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(app) = weak.upgrade() {
                                app.set_github_auth_pending(false);
                                app.set_github_auth_status(
                                    format!("Repository setup failed: {e}").into(),
                                );
                            }
                        });
                        return;
                    }
                };

                let user_info = provider.get_user(&token).ok();
                let username = user_info
                    .map(|u| u.login)
                    .or_else(|| GitHubProvider::parse_remote(&repo_info.clone_url).map(|(u, _)| u))
                    .unwrap_or_default();

                let weak_done = thread_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak_done.upgrade() {
                        let session = SESSION
                            .with(|s| s.borrow().clone())
                            .expect("session set in run");
                        let mut s = session.borrow_mut();
                        let vault_dir = s.vault_dir.clone();
                        let clone_url = repo_info.clone_url.clone();
                        if let Err(e) = s.app_data.save_remote(&vault_dir, &clone_url) {
                            drop(s);
                            set_status(&app, format!("Could not save remote: {e}"));
                            return;
                        }
                        if let Err(e) = s.token_store.save(&token) {
                            drop(s);
                            set_status(&app, format!("Could not save access token: {e}"));
                            return;
                        }
                        s.remote = Some(clone_url.clone());
                        drop(s);

                        app.set_github_auth_pending(false);
                        app.set_github_connected(true);
                        app.set_github_username(username.into());
                        app.set_github_repo_name(repo_info.name.into());
                        app.set_remote_configured(true);
                        app.set_remote_url_draft(clone_url.into());
                        app.set_remote_has_token(true);
                        set_status(&app, "GitHub account linked! Starting sync...");
                        start_sync(&app, &session);
                    }
                });
            });
        }
    });

    let cached_export_uri = Rc::new(RefCell::new(None::<String>));
    let pending_imported_payload = Rc::new(RefCell::new(
        None::<immermemo_vault::transfer::TransferPayload>,
    ));
    APP_WEAK.with(|a| *a.borrow_mut() = Some(app.as_weak()));
    PENDING_IMPORT_PAYLOAD.with(|p| *p.borrow_mut() = Some(pending_imported_payload.clone()));

    let update_transfer_qr = {
        let cached_export_uri = cached_export_uri.clone();
        move |app: &App, session: &Rc<RefCell<Session>>| {
            let options = immermemo_vault::transfer::ExportOptions {
                include_interface: app.get_export_include_interface(),
                include_remote: app.get_export_include_remote(),
                include_token: app.get_export_include_token(),
            };
            let s = session.borrow();
            let vault_name = Some(session::vault_display_name(&s.vault_dir));
            let ui_scale = Some(app.get_ui_scale());
            let payload_res = immermemo_vault::transfer::build_payload(
                &s.app_data,
                &s.vault_dir,
                &*s.token_store,
                &options,
                vault_name,
                ui_scale,
            );
            drop(s);
            match payload_res {
                Ok(payload) => match immermemo_vault::transfer::encode_payload(&payload) {
                    Ok(uri) => {
                        *cached_export_uri.borrow_mut() = Some(uri.clone());
                        match crate::qr::generate_qr_image(&uri, 260) {
                            Ok(img) => app.set_qr_image(img),
                            Err(_) => app.set_qr_image(slint::Image::default()),
                        }
                    }
                    Err(_) => app.set_qr_image(slint::Image::default()),
                },
                Err(_) => app.set_qr_image(slint::Image::default()),
            }
        }
    };

    app.on_refresh_transfer_qr({
        let (weak, session, update_qr) =
            (app.as_weak(), session.clone(), update_transfer_qr.clone());
        move || {
            let app = weak.unwrap();
            app.set_transfer_copied(false);
            update_qr(&app, &session);
        }
    });

    app.on_export_options_changed({
        let (weak, session, update_qr) = (app.as_weak(), session.clone(), update_transfer_qr);
        move |include_interface, include_remote, include_token| {
            let app = weak.unwrap();
            app.set_export_include_interface(include_interface);
            app.set_export_include_remote(include_remote);
            app.set_export_include_token(include_token);
            app.set_transfer_copied(false);
            update_qr(&app, &session);
        }
    });

    app.on_copy_transfer_link({
        let (weak, cached_export_uri) = (app.as_weak(), cached_export_uri.clone());
        move || {
            let app = weak.unwrap();
            if let Some(uri) = cached_export_uri.borrow().as_deref() {
                session::copy_to_clipboard(&app, uri);
                app.set_transfer_copied(true);
            }
        }
    });

    app.on_paste_import_clipboard({
        let weak = app.as_weak();
        move || {
            let app = weak.unwrap();
            if let Some(text) = session::read_from_clipboard(&app) {
                app.set_import_draft(text.trim().into());
            }
        }
    });

    app.on_review_import({
        let (weak, pending_payload) = (app.as_weak(), pending_imported_payload.clone());
        move |input| {
            let app = weak.unwrap();
            open_import_preview(&app, &pending_payload, &input);
        }
    });

    app.on_cancel_import_preview({
        let (weak, pending_payload) = (app.as_weak(), pending_imported_payload.clone());
        move || {
            let app = weak.unwrap();
            app.set_import_preview_ready(false);
            *pending_payload.borrow_mut() = None;
        }
    });

    app.on_apply_import({
        let (weak, session, pending_payload) = (
            app.as_weak(),
            session.clone(),
            pending_imported_payload.clone(),
        );
        move || {
            let app = weak.unwrap();
            let payload = pending_payload.borrow_mut().take();
            let Some(payload) = payload else {
                return;
            };
            let mut s = session.borrow_mut();
            match immermemo_vault::transfer::apply_payload(
                &s.app_data,
                &s.vault_dir,
                &*s.token_store,
                &payload,
            ) {
                Ok(result) => {
                    app.set_import_preview_ready(false);
                    app.set_import_status_is_error(false);
                    app.set_import_status_message("Settings applied successfully!".into());
                    if result.interface_updated {
                        if let Some(theme) = s.app_data.load_theme() {
                            app.set_theme_choice(theme);
                        }
                        if let Some(font_choice) = s.app_data.load_font_size() {
                            app.set_font_size_choice(font_choice);
                            app.set_editor_font_size(match font_choice {
                                0 => 14.0,
                                2 => 20.0,
                                _ => 17.0,
                            });
                        }
                        if let Some(status_bar) = s.app_data.load_status_bar_choice() {
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
                        update_github_ui_state(&app, s.remote.as_deref());
                        app.set_remote_url_draft(s.remote.clone().unwrap_or_default().into());
                    }
                    if result.token_updated {
                        app.set_remote_has_token(true);
                    }
                }
                Err(e) => {
                    app.set_import_status_is_error(true);
                    app.set_import_status_message(format!("Failed to apply settings: {e}").into());
                }
            }
        }
    });

    let jump_pos = Rc::new(RefCell::new(0usize));
    app.on_jump_conflict({
        let (weak, session, jump_pos) = (app.as_weak(), session.clone(), jump_pos.clone());
        move || {
            let app = weak.unwrap();
            let s = session.borrow();
            let Some(path) = s.current_path() else {
                return;
            };
            let Ok(text) = std::fs::read_to_string(path) else {
                return;
            };
            let mut offset = *jump_pos.borrow();
            if offset >= text.len() {
                offset = 0;
            }
            let found = text[offset..]
                .find(tomet::merge::CONFLICT_MARKER)
                .map(|p| offset + p)
                .or_else(|| text.find(tomet::merge::CONFLICT_MARKER));
            if let Some(pos) = found {
                app.invoke_set_cursor(pos as i32);
                *jump_pos.borrow_mut() = pos + tomet::merge::CONFLICT_MARKER.len();
            }
        }
    });

    app.on_resolve_mine({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            resolve_conflict_step(&app, &session, 0);
        }
    });

    app.on_resolve_theirs({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            resolve_conflict_step(&app, &session, 1);
        }
    });

    app.on_open_conflict_sheet({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            sync_conflict_sheet_state(&app, &session);
            app.set_conflict_sheet_open(true);
        }
    });

    app.on_resolve_conflict_step({
        let (weak, session) = (app.as_weak(), session.clone());
        move |choice| {
            let app = weak.unwrap();
            resolve_conflict_step(&app, &session, choice);
        }
    });

    app.on_prev_conflict_step({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let idx = app.get_active_conflict_index();
            if idx > 0 {
                app.set_active_conflict_index(idx - 1);
                sync_conflict_sheet_state(&app, &session);
            }
        }
    });

    app.on_next_conflict_step({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            let idx = app.get_active_conflict_index();
            let total = app.get_active_conflict_total();
            if idx + 1 < total {
                app.set_active_conflict_index(idx + 1);
                sync_conflict_sheet_state(&app, &session);
            }
        }
    });

    app.on_open_conflict_note({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            open_first_conflicted_note(&app, &session);
        }
    });

    app.on_copy_sync_error({
        let weak = app.as_weak();
        move || {
            let Some(app) = weak.upgrade() else { return };
            let err = app.get_last_sync_error();
            if !err.is_empty() {
                session::copy_to_clipboard(&app, err.as_str());
                app.set_sync_error_copied(true);
                let weak_timer = weak.clone();
                slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
                    if let Some(app) = weak_timer.upgrade() {
                        app.set_sync_error_copied(false);
                    }
                });
            }
        }
    });

    app.on_dismiss_sync_error({
        let weak = app.as_weak();
        move || {
            let Some(app) = weak.upgrade() else { return };
            app.set_last_sync_error(SharedString::new());
            app.set_sync_error_copied(false);
            if app.get_status_is_error() {
                app.set_status(SharedString::new());
                app.set_status_is_error(false);
            }
        }
    });

    app.on_open_history({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            session::open_note_history(&app, &session);
        }
    });

    app.on_restore_history_version({
        let (weak, session) = (app.as_weak(), session.clone());
        move |idx| {
            let app = weak.unwrap();
            session::restore_note_version(&app, &session, idx as usize);
        }
    });

    app.on_open_vault_history({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            session::open_vault_history(&app, &session);
        }
    });

    if let Some(uri) = initial_import_uri {
        open_import_preview(&app, &pending_imported_payload, &uri);
    }

    app.run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{copy_to_clipboard, finish_sync, resolve_active_conflict};
    use slint::Model;
    use std::sync::Arc;

    // Slint's winit/femtovg backend can only ever initialize one event
    // loop per process, so `App::new()` can only be called once across
    // this whole test binary -- that's why this stays one #[test]
    // function instead of several. Each numbered scenario from the
    // original single function is now its own named helper below,
    // called in sequence, so a failing assertion's backtrace at least
    // names the scenario it broke in.
    #[test]
    fn ui_helpers_and_conflict_resolution() {
        let app = App::new().unwrap();

        status_classification_tracks_is_error(&app);

        let (_tmp, session, vault_dir, note_path) = set_up_session_with_one_conflicted_note(&app);
        resolving_the_conflict_clears_the_marker_and_status(&app, &session, &note_path);
        search_filtering_updates_the_notes_and_snippets_models(&app, &session, &vault_dir);
        a_note_s_meta_icon_reaches_its_directory_entry(&app, &session, &vault_dir);
        icon_picker_selection_saves_and_is_reflected_in_the_note_list(&app, &session, &vault_dir);
        let multi_note_path =
            conflict_sheet_steps_through_and_resolves_every_conflict(&app, &session, &vault_dir);
        sync_error_is_tracked_copied_and_dismissed(&app, &session);
        version_history_restore_and_auto_sync_queueing(&app, &session, &multi_note_path);
        editor_quick_resolve_buttons_walk_conflicts_one_at_a_time(&app, &session, &vault_dir);
        editing_while_a_sync_is_running_does_not_panic_on_reentrant_borrow(&app, &session);
        bundled_japanese_translation_is_discoverable_at_runtime();
        desktop_app_data_dir_resolves_sensible_path();
        deep_link_import_preview_opens_settings_sheet(&app);

        // `render::seam_tests` needs the same shared `app` for the same
        // one-event-loop-per-process reason -- see that module's doc.
        render::seam_tests::stacked_items_convert_into_the_generated_model(&app);
        render::seam_tests::a_wide_inline_conflict_paragraph_flows_onto_one_line(&app);
        render::seam_tests::a_narrow_width_wraps_an_inline_conflict_paragraph_onto_several_lines(
            &app,
        );
        render::seam_tests::a_doc_icon_resolves_to_a_real_image_and_an_unknown_one_to_empty(&app);
        render::seam_tests::task_items_convert_into_the_generated_model(&app);
        render::seam_tests::inline_conflict_maintains_identity_and_order_across_resizes(&app);
    }

    fn status_classification_tracks_is_error(app: &App) {
        set_status(app, "Save failed: permission denied");
        assert!(app.get_status_is_error());
        assert_eq!(app.get_status(), "Save failed: permission denied");

        set_status(app, "Syncing...");
        assert!(!app.get_status_is_error());
        assert_eq!(app.get_status(), "Syncing...");

        set_status(app, "Sync failed: network timeout");
        assert!(app.get_status_is_error());

        set_status(app, "Synced");
        assert!(!app.get_status_is_error());
    }

    fn set_up_session_with_one_conflicted_note(
        app: &App,
    ) -> (tempfile::TempDir, Rc<RefCell<Session>>, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let vault_dir = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        let note_path = vault_dir.join("test.tmt");
        let base_doc = tomet::parser::parse_document("Original.\n").unwrap();
        let local_doc = tomet::parser::parse_document("Mine.\n").unwrap();
        let remote_doc = tomet::parser::parse_document("Theirs.\n").unwrap();
        let merged_doc = tomet::merge::merge(&base_doc, &local_doc, &remote_doc).document;
        let conflicted_text = tomet::printer::document_to_tm(&merged_doc);
        std::fs::write(&note_path, &conflicted_text).unwrap();

        let app_data = AppData::new(tmp.path().join("data"));
        let token_store: Arc<dyn TokenStore> = Arc::new(credentials::PlainFileTokenStore::new(
            tmp.path().join("token"),
        ));
        let mut core = immermemo_core::Session::new(
            vault_dir.clone(),
            app_data,
            token_store.clone(),
            Box::new(move |_| Ok(token_store.clone())),
            vec![vault_dir.clone()],
            None,
            immermemo_index::NoteIndex::open_in_memory().unwrap(),
        );
        core.notes = vec![note_path.clone()];
        core.conflicted = vec![true];
        core.filtered_results = vec![notes::SearchResult {
            note_index: 0,
            snippet: None,
        }];
        core.editor = immermemo_editor::EditorState {
            current: Some(0),
            history: Some(History::new(&conflicted_text)),
            note_history_revisions: Vec::new(),
        };
        let session = Rc::new(RefCell::new(Session::new(core)));

        app.set_current_has_conflict(true);
        app.set_body(conflicted_text.into());

        (tmp, session, vault_dir, note_path)
    }

    fn resolving_the_conflict_clears_the_marker_and_status(
        app: &App,
        session: &Rc<RefCell<Session>>,
        note_path: &Path,
    ) {
        resolve_active_conflict(app, session, tomet::merge::ConflictResolution::A);

        assert!(!app.get_current_has_conflict());
        assert_eq!(app.get_body(), "Mine.\n");
        let disk_text = std::fs::read_to_string(note_path).unwrap();
        assert_eq!(disk_text, "Mine.\n");
        assert!(!notes::has_conflict_marker(note_path));
        assert!(app.get_status().contains("Resolved conflict"));
    }

    fn search_filtering_updates_the_notes_and_snippets_models(
        app: &App,
        session: &Rc<RefCell<Session>>,
        vault_dir: &Path,
    ) {
        let n1 = vault_dir.join("roadmap.tmt");
        std::fs::write(&n1, "Project roadmap and milk supply\n").unwrap();
        session.borrow_mut().notes.push(n1);
        session.borrow_mut().conflicted.push(false);

        // Search query empty -> all notes
        refresh_list(app, session);
        assert_eq!(app.get_notes().row_count(), 2);

        // Filter for "milk" (present in roadmap content)
        session.borrow_mut().search_query = "milk".to_string();
        update_filtered_list(app, session);
        assert_eq!(app.get_notes().row_count(), 1);
        assert_eq!(app.get_snippets().row_count(), 1);
        assert_eq!(app.get_notes().row_data(0).unwrap(), "roadmap");

        // Filter for "test" (matches test.tmt title)
        session.borrow_mut().search_query = "test".to_string();
        update_filtered_list(app, session);
        assert_eq!(app.get_notes().row_count(), 1);
        assert_eq!(app.get_notes().row_data(0).unwrap(), "test");

        // Filter for nonexistent query
        session.borrow_mut().search_query = "nonexistent".to_string();
        update_filtered_list(app, session);
        assert_eq!(app.get_notes().row_count(), 0);
    }

    /// `refresh_list` -> `update_filtered_list` -> `refresh_directory_views`
    /// is the real, full path a note's `@meta{ icon: @doc.icon(...) }`
    /// travels to reach `app.directory_folder_entries` -- `render::
    /// seam_tests`' own icon test stops at `to_rendered_block` (a body-
    /// inline occurrence), so this is the one place proving the *note-list*
    /// path (`session::to_entry_view`, `NoteIndex::bodies`) actually wires
    /// up end to end, not just each piece in isolation.
    fn a_note_s_meta_icon_reaches_its_directory_entry(
        app: &App,
        session: &Rc<RefCell<Session>>,
        vault_dir: &Path,
    ) {
        let iconed = vault_dir.join("starred.tmt");
        std::fs::write(
            &iconed,
            "@meta{ icon: @doc.icon(\"star\") }\n\nStarred note.\n",
        )
        .unwrap();
        session.borrow_mut().search_query = String::new();
        refresh_list(app, session);

        let entries = app.get_directory_folder_entries();
        let starred = (0..entries.row_count())
            .map(|i| entries.row_data(i).unwrap())
            .find(|e| e.display == "starred")
            .expect("starred.tmt is in the root folder's entries");
        assert!(starred.icon_image.size().width > 0);

        let unstarred = (0..entries.row_count())
            .map(|i| entries.row_data(i).unwrap())
            .find(|e| e.display == "test")
            .expect("test.tmt (no meta icon) is also in the root folder's entries");
        assert_eq!(unstarred.icon_image.size().width, 0);
    }

    /// The icon picker's *write* side -- `open_icon_picker` ->
    /// `filter_icon_picker` -> `apply_icon_selection` -- saves a real
    /// `@meta{ icon: ... }` onto disk and the index picks it up on the next
    /// `refresh_list` the same save already triggers. The *read* side
    /// (`a_note_s_meta_icon_reaches_its_directory_entry` above) only ever
    /// exercised a hand-written fixture; this is the one place proving a
    /// picker selection actually produces a body `meta_element` can read
    /// back, not just that `immermemo_editor::set_note_icon` can in
    /// isolation (that's `crates/editor`'s own unit tests' job).
    fn icon_picker_selection_saves_and_is_reflected_in_the_note_list(
        app: &App,
        session: &Rc<RefCell<Session>>,
        vault_dir: &Path,
    ) {
        let plain = vault_dir.join("plain.tmt");
        std::fs::write(&plain, "No icon yet.\n").unwrap();
        session.borrow_mut().search_query = String::new();
        refresh_list(app, session);

        let note_index = session
            .borrow()
            .notes
            .iter()
            .position(|p| *p == plain)
            .expect("plain.tmt is in Session.notes after refresh_list");

        open_icon_picker(app, session, note_index);
        assert!(app.get_icon_picker_open());
        assert!(
            app.get_icon_picker_results().row_count() > 0,
            "opening with an empty query should still show a first page of results"
        );

        filter_icon_picker(app, "star");
        let results = app.get_icon_picker_results();
        assert!(
            results.row_count() > 0,
            "\"star\" should match at least tabler's own star icon"
        );
        let star = (0..results.row_count())
            .map(|i| results.row_data(i).unwrap())
            .find(|r| r.name == "star")
            .expect("tabler vendors a plain \"star\" icon");
        assert!(star.image.size().width > 0);

        apply_icon_selection(app, session, "star");
        assert!(
            session.borrow().icon_picker_target.is_none(),
            "apply_icon_selection should clear the target it consumed"
        );

        let saved = std::fs::read_to_string(&plain).unwrap();
        let (identity, _) = immermemo_tomet_render::meta_element(&saved, "icon")
            .expect("the saved note now has a meta icon entry");
        assert_eq!(identity.namespace.as_deref(), Some("doc"));
        assert_eq!(identity.name, "icon");

        // refresh_list already ran inside apply_icon_selection -- the
        // Directory entries should already reflect it without a further
        // manual refresh here.
        let entries = app.get_directory_folder_entries();
        let row = (0..entries.row_count())
            .map(|i| entries.row_data(i).unwrap())
            .find(|e| e.display == "plain")
            .expect("plain.tmt is still in the root folder's entries");
        assert!(row.icon_image.size().width > 0);
    }

    fn conflict_sheet_steps_through_and_resolves_every_conflict(
        app: &App,
        session: &Rc<RefCell<Session>>,
        vault_dir: &Path,
    ) -> PathBuf {
        let multi_note_path = vault_dir.join("multi_conflict.tmt");
        let base_doc =
            tomet::parser::parse_document("Alpha base.\n\nMiddle untouched.\n\nBeta base.\n")
                .unwrap();
        let local_doc =
            tomet::parser::parse_document("Alpha mine.\n\nMiddle untouched.\n\nBeta mine.\n")
                .unwrap();
        let remote_doc =
            tomet::parser::parse_document("Alpha theirs.\n\nMiddle untouched.\n\nBeta theirs.\n")
                .unwrap();
        let merged_doc = tomet::merge::merge(&base_doc, &local_doc, &remote_doc).document;
        let conflicted_text = tomet::printer::document_to_tm(&merged_doc);
        std::fs::write(&multi_note_path, &conflicted_text).unwrap();

        let new_idx = session.borrow().notes.len();
        let mut s = session.borrow_mut();
        s.notes.push(multi_note_path.clone());
        s.conflicted.push(true);
        s.editor.current = Some(new_idx);
        s.editor.history = Some(History::new(&conflicted_text));
        drop(s);
        app.set_current_has_conflict(true);
        app.set_body(conflicted_text.clone().into());

        // Test open_first_conflicted_note from Home tab
        app.set_active_tab(0);
        app.set_note_sheet_open(false);
        app.set_conflict_sheet_open(false);
        open_first_conflicted_note(app, session);
        assert_eq!(app.get_active_tab(), 0);
        assert!(app.get_note_sheet_open());
        assert!(app.get_conflict_sheet_open());
        assert_eq!(app.get_active_conflict_total(), 2);
        assert_eq!(app.get_active_conflict_a(), "min");
        assert_eq!(app.get_active_conflict_b(), "theirs");

        // Initial state sync
        sync_conflict_sheet_state(app, session);
        assert_eq!(app.get_active_conflict_total(), 2);
        assert_eq!(app.get_active_conflict_index(), 0);
        assert_eq!(app.get_active_conflict_a(), "min");
        assert_eq!(app.get_active_conflict_b(), "theirs");

        // Navigate to next conflict
        app.set_active_conflict_index(1);
        sync_conflict_sheet_state(app, session);
        assert_eq!(app.get_active_conflict_index(), 1);
        assert_eq!(app.get_active_conflict_a(), "min");
        assert_eq!(app.get_active_conflict_b(), "theirs");

        // Resolve conflict at index 1 with Keep Both (choice 2)
        resolve_conflict_step(app, session, 2);
        assert_eq!(app.get_active_conflict_total(), 1);
        assert!(app.get_current_has_conflict());
        assert_eq!(app.get_active_conflict_index(), 0);

        // Resolve the remaining conflict (index 0) with Keep Mine (choice 0)
        resolve_conflict_step(app, session, 0);
        assert_eq!(app.get_active_conflict_total(), 0);
        assert!(!app.get_current_has_conflict());
        assert!(!app.get_conflict_sheet_open());
        assert_eq!(app.get_status(), "All conflicts resolved");

        let disk_text = std::fs::read_to_string(&multi_note_path).unwrap();
        assert!(!disk_text.contains(tomet::merge::CONFLICT_MARKER));
        assert!(disk_text.contains("Alpha min."));
        assert!(disk_text.contains("Middle untouched."));
        assert!(disk_text.contains("Beta mintheirs."));

        multi_note_path
    }

    /// Editor/properties' "Keep Mine"/"Keep Theirs" bar wires to
    /// `resolve_conflict_step`, not `resolve_active_conflict` (see
    /// `app.on_resolve_mine`/`app.on_resolve_theirs` in `lib.rs`) -- it must
    /// resolve only the conflict at `active-conflict-index`, one at a time,
    /// even when the full ConflictSheet was never opened.
    fn editor_quick_resolve_buttons_walk_conflicts_one_at_a_time(
        app: &App,
        session: &Rc<RefCell<Session>>,
        vault_dir: &Path,
    ) {
        let note_path = vault_dir.join("editor_quick_resolve.tmt");
        let base_doc =
            tomet::parser::parse_document("Alpha base.\n\nMiddle untouched.\n\nBeta base.\n")
                .unwrap();
        let local_doc =
            tomet::parser::parse_document("Alpha mine.\n\nMiddle untouched.\n\nBeta mine.\n")
                .unwrap();
        let remote_doc =
            tomet::parser::parse_document("Alpha theirs.\n\nMiddle untouched.\n\nBeta theirs.\n")
                .unwrap();
        let merged_doc = tomet::merge::merge(&base_doc, &local_doc, &remote_doc).document;
        let conflicted_text = tomet::printer::document_to_tm(&merged_doc);
        std::fs::write(&note_path, &conflicted_text).unwrap();

        let new_idx = session.borrow().notes.len();
        let mut s = session.borrow_mut();
        s.notes.push(note_path.clone());
        s.conflicted.push(true);
        s.editor.current = Some(new_idx);
        s.editor.history = Some(History::new(&conflicted_text));
        drop(s);
        app.set_current_has_conflict(true);
        app.set_body(conflicted_text.clone().into());

        // Never opened the ConflictSheet -- `active-conflict-index` sits at
        // its default, pointing at the first conflict in the note.
        app.set_active_conflict_index(0);

        resolve_conflict_step(app, session, 0);
        assert!(app.get_current_has_conflict());
        let disk_text = std::fs::read_to_string(&note_path).unwrap();
        assert!(disk_text.contains(tomet::merge::CONFLICT_MARKER));
        assert!(disk_text.contains("Alpha min."));

        // `resolve_conflict_step` re-clamps `active-conflict-index` to the
        // first remaining conflict, so pressing the same button again walks
        // to the next conflict instead of repeating the first.
        resolve_conflict_step(app, session, 0);
        assert!(!app.get_current_has_conflict());
        let disk_text = std::fs::read_to_string(&note_path).unwrap();
        assert!(!disk_text.contains(tomet::merge::CONFLICT_MARKER));
        assert!(disk_text.contains("Alpha min."));
        assert!(disk_text.contains("Beta min."));
    }

    fn sync_error_is_tracked_copied_and_dismissed(app: &App, session: &Rc<RefCell<Session>>) {
        let err_text = "Sync failed: no merge base found; class=Merge (22)";
        finish_sync(
            app,
            session,
            Err("no merge base found; class=Merge (22)".to_string()),
        );
        assert_eq!(app.get_last_sync_error(), err_text);
        assert!(app.get_status_is_error());
        assert_eq!(app.get_status(), err_text);
        assert!(!app.get_sync_error_copied());

        // Copy error simulation
        copy_to_clipboard(app, err_text);
        app.set_sync_error_copied(true);
        assert!(app.get_sync_error_copied());

        // Dismiss error
        app.set_last_sync_error(SharedString::new());
        app.set_sync_error_copied(false);
        app.set_status(SharedString::new());
        app.set_status_is_error(false);
        assert_eq!(app.get_last_sync_error(), "");
        assert!(!app.get_status_is_error());

        // Successful sync clears error and updates last_synced_at
        finish_sync(app, session, Err("some error".to_string()));
        assert_eq!(app.get_last_sync_error(), "Sync failed: some error");
        finish_sync(app, session, Ok(immermemo_sync::SyncReport::default()));
        assert_eq!(app.get_last_sync_error(), "");
        assert_eq!(app.get_status(), "Synced");
        assert!(!app.get_status_is_error());
        assert!(!app.get_last_synced_at().is_empty());
    }

    fn version_history_restore_and_auto_sync_queueing(
        app: &App,
        session: &Rc<RefCell<Session>>,
        multi_note_path: &Path,
    ) {
        let ts = session::format_timestamp(1700000000);
        assert!(ts.contains("2023-11-14"));

        let rev1 = immermemo_sync::FileRevision {
            commit_id: "1111111111111111111111111111111111111111".to_string(),
            short_id: "1111111".to_string(),
            timestamp_secs: 1700000000,
            summary: "Initial commit".to_string(),
            content: "Historical content v1.\n".to_string(),
        };
        session.borrow_mut().editor.note_history_revisions = vec![rev1];

        session::restore_note_version(app, session, 0);
        assert_eq!(app.get_body(), "Historical content v1.\n");
        let disk_text = std::fs::read_to_string(multi_note_path).unwrap();
        assert_eq!(disk_text, "Historical content v1.\n");
        assert!(!app.get_history_sheet_open());
        assert!(app.get_status().contains("Restored note"));

        // Auto-sync pending queue when already syncing
        app.set_syncing(true);
        session.borrow_mut().remote = Some("https://example.com/repo.git".to_string());
        session::trigger_auto_sync(app, session);
        assert!(session.borrow().pending_auto_sync);

        // When sync finishes, pending_auto_sync is cleared and next sync is scheduled
        finish_sync(app, session, Ok(immermemo_sync::SyncReport::default()));
        assert!(app.get_syncing());
        finish_sync(app, session, Ok(immermemo_sync::SyncReport::default()));
        assert!(!app.get_syncing());
    }

    fn editing_while_a_sync_is_running_does_not_panic_on_reentrant_borrow(
        app: &App,
        session: &Rc<RefCell<Session>>,
    ) {
        SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
        app.set_syncing(true);
        session::schedule_auto_sync(app, std::time::Duration::from_millis(50));
        session::trigger_auto_sync(app, session);
        assert!(session.borrow().pending_auto_sync);
        finish_sync(app, session, Ok(immermemo_sync::SyncReport::default()));
        assert!(app.get_syncing());
        finish_sync(app, session, Ok(immermemo_sync::SyncReport::default()));
        assert!(!app.get_syncing());
    }

    // The bundled `ja` translation catalog is discoverable at runtime (a
    // stale/missing .po file would show up here, not just at compile time,
    // since the directory is read again when selecting a language).
    fn bundled_japanese_translation_is_discoverable_at_runtime() {
        assert!(slint::select_bundled_translation("ja").is_ok());
        let _ = slint::select_bundled_translation("en");
    }

    fn desktop_app_data_dir_resolves_sensible_path() {
        let dir = desktop_app_data_dir().expect("desktop app data dir should resolve");
        assert!(dir.ends_with("immermemo"));
    }

    fn deep_link_import_preview_opens_settings_sheet(app: &App) {
        let pending = Rc::new(RefCell::new(None));
        let valid_payload = immermemo_vault::transfer::TransferPayload {
            v: 1,
            app: "immermemo".to_string(),
            interface: Some(immermemo_vault::transfer::InterfaceTransfer {
                font_size: Some(2),
                theme: Some(1),
                status_bar: Some(0),
                ui_scale: Some(1.2),
            }),
            sync: Some(immermemo_vault::transfer::SyncTransfer {
                vault_name: Some("test-vault".to_string()),
                remote_url: Some("https://example.com/notes.git".to_string()),
                token: Some("secret123".to_string()),
            }),
        };
        let uri = immermemo_vault::transfer::encode_payload(&valid_payload).unwrap();
        open_import_preview(app, &pending, &uri);

        assert!(app.get_settings_sheet_open());
        assert_eq!(app.get_settings_page(), 5);
        assert_eq!(app.get_settings_transfer_tab(), 1);
        assert!(app.get_import_preview_ready());
        assert!(app.get_preview_has_interface());
        assert!(app.get_preview_has_remote());
        assert_eq!(
            app.get_preview_remote_url(),
            "https://example.com/notes.git"
        );
        assert!(app.get_preview_has_token());
        assert!(pending.borrow().is_some());
    }
}
