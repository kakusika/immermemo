pub mod credentials;
mod directory;
mod haptic;
pub mod render;
mod session;
mod sync;
mod widget;

#[cfg(target_os = "android")]
mod android;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use immermemo_editor::History;
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

/// Desktop's app-data base: `$XDG_DATA_HOME/immermemo`, or
/// `$HOME/.local/share/immermemo`. `main.rs` passes this to [`run`]; Android
/// has neither variable and computes its own base in `android_main` instead.
pub fn desktop_app_data_dir() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| anyhow::anyhow!("neither XDG_DATA_HOME nor HOME is set"))?;
    Ok(base.join("immermemo"))
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
    app.set_current_vault_name(vault_display_name(&vault_dir).into());
    app.set_current_vault_path(vault_dir.display().to_string().into());
    app.set_remote_configured(remote.is_some());
    let index = app_data
        .index_path(&vault_dir)
        .ok()
        .and_then(|p| immermemo_index::NoteIndex::open(&p).ok())
        .unwrap_or_else(|| immermemo_index::NoteIndex::open_in_memory().unwrap());
    let session = Rc::new(RefCell::new(Session {
        vault_dir,
        app_data,
        token_store,
        token_store_for,
        known_vaults,
        remote,
        index,
        notes: Vec::new(),
        conflicted: Vec::new(),
        search_query: String::new(),
        filtered_results: Vec::new(),
        editor: immermemo_editor::EditorState::default(),
        rename_target: None,
        icon_picker_target: None,
        delete_target: None,
        delete_vault_target: None,
        pending_auto_sync: false,
        last_synced_at: None,
        current_folder: String::new(),
        directory_source: Vec::new(),
        note_nav: session::BackForwardStack::default(),
    }));

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
                let has_conflict = text.contains(immermemo_merge::CONFLICT_MARKER);
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
            let _ = session.borrow().app_data.save_font_size(choice);
        }
    });
    app.on_theme_changed({
        let session = session.clone();
        move |choice| {
            let _ = session.borrow().app_data.save_theme(choice);
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
            app.set_settings_open(false);
            start_sync(&app, &session);
        }
    });

    app.on_sync({
        let (weak, session) = (app.as_weak(), session.clone());
        move || start_sync(&weak.unwrap(), &session)
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
                .find(immermemo_merge::CONFLICT_MARKER)
                .map(|p| offset + p)
                .or_else(|| text.find(immermemo_merge::CONFLICT_MARKER));
            if let Some(pos) = found {
                app.invoke_set_cursor(pos as i32);
                *jump_pos.borrow_mut() = pos + immermemo_merge::CONFLICT_MARKER.len();
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

        // `render::seam_tests` needs the same shared `app` for the same
        // one-event-loop-per-process reason -- see that module's doc.
        render::seam_tests::stacked_items_convert_into_the_generated_model(&app);
        render::seam_tests::a_wide_inline_conflict_paragraph_flows_onto_one_line(&app);
        render::seam_tests::a_narrow_width_wraps_an_inline_conflict_paragraph_onto_several_lines(
            &app,
        );
        render::seam_tests::a_doc_icon_resolves_to_a_real_image_and_an_unknown_one_to_empty(&app);
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
        let base_doc = tomet_parser::parse_document("Original.\n").unwrap();
        let local_doc = tomet_parser::parse_document("Mine.\n").unwrap();
        let remote_doc = tomet_parser::parse_document("Theirs.\n").unwrap();
        let merged_doc = immermemo_merge::merge(&base_doc, &local_doc, &remote_doc).document;
        let conflicted_text = tomet_printer::document_to_tm(&merged_doc);
        std::fs::write(&note_path, &conflicted_text).unwrap();

        let app_data = AppData::new(tmp.path().join("data"));
        let token_store: Arc<dyn TokenStore> = Arc::new(credentials::PlainFileTokenStore::new(
            tmp.path().join("token"),
        ));
        let session = Rc::new(RefCell::new(Session {
            vault_dir: vault_dir.clone(),
            app_data,
            token_store: token_store.clone(),
            token_store_for: Box::new(move |_| Ok(token_store.clone())),
            known_vaults: vec![vault_dir.clone()],
            remote: None,
            index: immermemo_index::NoteIndex::open_in_memory().unwrap(),
            notes: vec![note_path.clone()],
            conflicted: vec![true],
            search_query: String::new(),
            filtered_results: vec![notes::SearchResult {
                note_index: 0,
                snippet: None,
            }],
            editor: immermemo_editor::EditorState {
                current: Some(0),
                history: Some(History::new(&conflicted_text)),
                note_history_revisions: Vec::new(),
            },
            rename_target: None,
        icon_picker_target: None,
            delete_target: None,
            delete_vault_target: None,
            pending_auto_sync: false,
            last_synced_at: None,
            current_folder: String::new(),
            directory_source: Vec::new(),
            note_nav: session::BackForwardStack::default(),
        }));

        app.set_current_has_conflict(true);
        app.set_body(conflicted_text.into());

        (tmp, session, vault_dir, note_path)
    }

    fn resolving_the_conflict_clears_the_marker_and_status(
        app: &App,
        session: &Rc<RefCell<Session>>,
        note_path: &Path,
    ) {
        resolve_active_conflict(app, session, immermemo_merge::ConflictResolution::A);

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
        std::fs::write(&iconed, "@meta{ icon: @doc.icon(\"star\") }\n\nStarred note.\n").unwrap();
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
        assert!(results.row_count() > 0, "\"star\" should match at least tabler's own star icon");
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
            tomet_parser::parse_document("Alpha base.\n\nMiddle untouched.\n\nBeta base.\n")
                .unwrap();
        let local_doc =
            tomet_parser::parse_document("Alpha mine.\n\nMiddle untouched.\n\nBeta mine.\n")
                .unwrap();
        let remote_doc =
            tomet_parser::parse_document("Alpha theirs.\n\nMiddle untouched.\n\nBeta theirs.\n")
                .unwrap();
        let merged_doc = immermemo_merge::merge(&base_doc, &local_doc, &remote_doc).document;
        let conflicted_text = tomet_printer::document_to_tm(&merged_doc);
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
        assert!(!disk_text.contains(immermemo_merge::CONFLICT_MARKER));
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
            tomet_parser::parse_document("Alpha base.\n\nMiddle untouched.\n\nBeta base.\n")
                .unwrap();
        let local_doc =
            tomet_parser::parse_document("Alpha mine.\n\nMiddle untouched.\n\nBeta mine.\n")
                .unwrap();
        let remote_doc =
            tomet_parser::parse_document("Alpha theirs.\n\nMiddle untouched.\n\nBeta theirs.\n")
                .unwrap();
        let merged_doc = immermemo_merge::merge(&base_doc, &local_doc, &remote_doc).document;
        let conflicted_text = tomet_printer::document_to_tm(&merged_doc);
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
        assert!(disk_text.contains(immermemo_merge::CONFLICT_MARKER));
        assert!(disk_text.contains("Alpha min."));

        // `resolve_conflict_step` re-clamps `active-conflict-index` to the
        // first remaining conflict, so pressing the same button again walks
        // to the next conflict instead of repeating the first.
        resolve_conflict_step(app, session, 0);
        assert!(!app.get_current_has_conflict());
        let disk_text = std::fs::read_to_string(&note_path).unwrap();
        assert!(!disk_text.contains(immermemo_merge::CONFLICT_MARKER));
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
}
