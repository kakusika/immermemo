pub mod credentials;
mod sync;
mod haptic;
mod session;

#[cfg(target_os = "android")]
mod android;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use immermemo_vault::appdata::AppData;
pub use immermemo_vault::credentials::{TokenStore, TokenStoreFactory};
use immermemo_vault::history::History;
use immermemo_vault::notes;

use session::{
    apply_restored, open_initial_or_last_note, open_note, refresh_list, refresh_vault_list,
    resolve_active_conflict, set_status, show_history_state, start_sync, switch_vault,
    update_filtered_list, vault_display_name, vault_path_from_input, Session, SESSION,
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
    let session = Rc::new(RefCell::new(Session {
        vault_dir,
        app_data,
        token_store,
        token_store_for,
        known_vaults,
        remote,
        notes: Vec::new(),
        conflicted: Vec::new(),
        search_query: String::new(),
        filtered_results: Vec::new(),
        current: None,
        history: None,
        rename_target: None,
        delete_target: None,
        delete_vault_target: None,
    }));

    SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
    refresh_list(&app, &session);
    refresh_vault_list(&app, &session);
    if session.borrow().notes.is_empty() {
        app.set_list_open(true);
    } else {
        app.set_list_open(false);
        open_initial_or_last_note(&app, &session);
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

    app.on_edited({
        let (weak, session) = (app.as_weak(), session.clone());
        move |text| {
            let app = weak.unwrap();
            let mut s = session.borrow_mut();
            if let Some(history) = s.history.as_mut() {
                history.edit(&text);
            }
            show_history_state(&app, &s);
            let has_conflict = text.contains("@mobile.conflict");
            if app.get_current_has_conflict() != has_conflict {
                app.set_current_has_conflict(has_conflict);
                if let Some(idx) = s.current
                    && idx < s.conflicted.len()
                {
                    s.conflicted[idx] = has_conflict;
                    let conflicted: Vec<bool> = s
                        .filtered_results
                        .iter()
                        .map(|r| s.conflicted[r.note_index])
                        .collect();
                    app.set_conflicted(ModelRc::new(VecModel::from(conflicted)));
                }
            }
            if let Some(path) = s.current_path() {
                match std::fs::write(path, text.as_str()) {
                    Ok(_) => {
                        if app.get_status_is_error() && app.get_status().starts_with("Save failed")
                        {
                            set_status(&app, "");
                        }
                    }
                    Err(e) => set_status(&app, format!("Save failed: {e}")),
                }
            }
        }
    });

    app.on_undo({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let restored = session
                .borrow_mut()
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
                .history
                .as_mut()
                .and_then(History::redo);
            apply_restored(&weak.unwrap(), &session, restored);
        }
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
    app.on_close_remote({
        let weak = app.as_weak();
        move || weak.unwrap().set_remote_open(false)
    });
    app.on_trigger_haptic(|| {
        haptic::perform_haptic();
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
            let was_current = session.borrow().current_path() == Some(&path);
            match notes::rename(&path, &new_name) {
                Ok(new_path) => {
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
            if let Err(e) = notes::delete(&path) {
                set_status(&app, format!("Could not delete the note: {e}"));
                return;
            }
            let was_current = session.borrow().current_path() == Some(&path);
            refresh_list(&app, &session);
            if was_current {
                session.borrow_mut().history = None;
                app.set_current_title(SharedString::new());
                show_history_state(&app, &session.borrow());
                app.set_body(SharedString::new());
                if session.borrow().notes.is_empty() {
                    let s = session.borrow();
                    s.app_data.clear_last_note(&s.vault_dir);
                    app.set_list_open(true);
                } else {
                    open_initial_or_last_note(&app, &session);
                }
            } else if session.borrow().notes.is_empty() {
                let s = session.borrow();
                s.app_data.clear_last_note(&s.vault_dir);
                app.set_list_open(true);
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
            app.set_remote_open(false);
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
            const MARKER: &str = "@mobile.conflict";
            let mut offset = *jump_pos.borrow();
            if offset >= text.len() {
                offset = 0;
            }
            let found = text[offset..]
                .find(MARKER)
                .map(|p| offset + p)
                .or_else(|| text.find(MARKER));
            if let Some(pos) = found {
                app.invoke_set_cursor(pos as i32);
                *jump_pos.borrow_mut() = pos + MARKER.len();
            }
        }
    });

    app.on_resolve_mine({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            resolve_active_conflict(&app, &session, immermemo_merge::ConflictResolution::Mine);
        }
    });

    app.on_resolve_theirs({
        let (weak, session) = (app.as_weak(), session.clone());
        move || {
            let app = weak.unwrap();
            resolve_active_conflict(&app, &session, immermemo_merge::ConflictResolution::Theirs);
        }
    });

    app.run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use slint::Model;

    #[test]
    fn ui_helpers_and_conflict_resolution() {
        let app = App::new().unwrap();

        // 1. Status classification
        set_status(&app, "Save failed: permission denied");
        assert!(app.get_status_is_error());
        assert_eq!(app.get_status(), "Save failed: permission denied");

        set_status(&app, "Syncing...");
        assert!(!app.get_status_is_error());
        assert_eq!(app.get_status(), "Syncing...");

        set_status(&app, "Sync failed: network timeout");
        assert!(app.get_status_is_error());

        set_status(&app, "Synced");
        assert!(!app.get_status_is_error());

        // 2. Conflict resolution helper
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
            notes: vec![note_path.clone()],
            conflicted: vec![true],
            search_query: String::new(),
            filtered_results: vec![notes::SearchResult {
                note_index: 0,
                snippet: None,
            }],
            current: Some(0),
            history: Some(History::new(&conflicted_text)),
            rename_target: None,
            delete_target: None,
            delete_vault_target: None,
        }));

        app.set_current_has_conflict(true);
        app.set_body(conflicted_text.into());

        resolve_active_conflict(&app, &session, immermemo_merge::ConflictResolution::Mine);

        assert!(!app.get_current_has_conflict());
        assert_eq!(app.get_body(), "Mine.\n");
        let disk_text = std::fs::read_to_string(&note_path).unwrap();
        assert_eq!(disk_text, "Mine.\n");
        assert!(!notes::has_conflict_marker(&note_path));
        assert!(app.get_status().contains("Resolved conflict"));

        // 3. Search filtering and UI model update
        let n1 = vault_dir.join("roadmap.tmt");
        std::fs::write(&n1, "Project roadmap and milk supply\n").unwrap();
        session.borrow_mut().notes.push(n1);
        session.borrow_mut().conflicted.push(false);

        // Search query empty -> all notes
        refresh_list(&app, &session);
        assert_eq!(app.get_notes().row_count(), 2);

        // Filter for "milk" (present in roadmap content)
        session.borrow_mut().search_query = "milk".to_string();
        update_filtered_list(&app, &session);
        assert_eq!(app.get_notes().row_count(), 1);
        assert_eq!(app.get_snippets().row_count(), 1);
        assert_eq!(app.get_notes().row_data(0).unwrap(), "roadmap");

        // Filter for "test" (matches test.tmt title)
        session.borrow_mut().search_query = "test".to_string();
        update_filtered_list(&app, &session);
        assert_eq!(app.get_notes().row_count(), 1);
        assert_eq!(app.get_notes().row_data(0).unwrap(), "test");

        // Filter for nonexistent query
        session.borrow_mut().search_query = "nonexistent".to_string();
        update_filtered_list(&app, &session);
        assert_eq!(app.get_notes().row_count(), 0);
    }
}
