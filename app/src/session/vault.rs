//! Vault management: switching vaults, listing vaults, and validating vault paths.

use crate::session::SessionRef;
use std::path::{Path, PathBuf};

use immermemo_core::{AppData, EditorState};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::util::set_error;
use super::{
    clear_note_stats, open_initial_or_last_note, refresh_list, schedule_auto_sync,
    show_history_state, show_vault_identity, sync_nav_state, update_rendered_body,
    vault_display_name,
};
use crate::{App, SettingsAdapter, VaultData};

fn vault_account_info(
    app_data: &AppData,
    vault_dir: &Path,
    app_weak: slint::Weak<App>,
) -> (bool, SharedString, slint::Image, bool) {
    if let Some(acc_id) = app_data.load_vault_account(vault_dir) {
        if let Some(acc) = app_data.get_account(&acc_id) {
            let (avatar, has_avatar) =
                crate::avatar::load_or_fetch_avatar(app_data, &acc, app_weak);
            (true, acc.provider.into(), avatar, has_avatar)
        } else {
            (false, SharedString::new(), slint::Image::default(), false)
        }
    } else {
        (false, SharedString::new(), slint::Image::default(), false)
    }
}

pub fn refresh_vault_list(app: &App, session: &SessionRef) {
    let s = session.borrow();
    let names: Vec<SharedString> = s
        .known_vaults
        .iter()
        .map(|p| vault_display_name(p).into())
        .collect();
    let subtitles: Vec<SharedString> = s
        .known_vaults
        .iter()
        .map(|p| {
            if let Some(acc_id) = s.app_data.load_vault_account(p) {
                if let Some(acc) = s.app_data.get_account(&acc_id) {
                    format!("{} (@{})", acc.provider, acc.username).into()
                } else {
                    acc_id.into()
                }
            } else if let Some(url) = s.app_data.load_remote(p) {
                url.into()
            } else {
                "This device only".into()
            }
        })
        .collect();
    let current = s.known_vaults.iter().position(|v| *v == s.vault_dir);

    // Build SettingsAdapter vault list: active vault first, followed by other vaults
    let settings = app.global::<SettingsAdapter>();
    let mut ui_vaults = Vec::new();
    if let Some(curr_idx) = current {
        let p = &s.known_vaults[curr_idx];
        let subtitle = subtitles
            .get(curr_idx)
            .cloned()
            .unwrap_or_else(|| "This device only".into());
        let (has_account, account_provider, account_avatar, account_has_avatar) =
            vault_account_info(&s.app_data, p, app.as_weak());
        ui_vaults.push(VaultData {
            name: names[curr_idx].clone(),
            path: p.display().to_string().into(),
            subtitle,
            is_current: true,
            original_index: curr_idx as i32,
            has_account,
            syncs: s.app_data.load_remote(p).is_some(),
            account_provider,
            account_avatar,
            account_has_avatar,
        });
    }

    for (idx, p) in s.known_vaults.iter().enumerate() {
        if Some(idx) == current {
            continue;
        }
        let subtitle = subtitles
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "This device only".into());
        let (has_account, account_provider, account_avatar, account_has_avatar) =
            vault_account_info(&s.app_data, p, app.as_weak());
        ui_vaults.push(VaultData {
            name: names[idx].clone(),
            path: p.display().to_string().into(),
            subtitle,
            is_current: false,
            original_index: idx as i32,
            has_account,
            syncs: s.app_data.load_remote(p).is_some(),
            account_provider,
            account_avatar,
            account_has_avatar,
        });
    }
    drop(s);

    app.set_vaults(ModelRc::new(VecModel::from(names)));
    app.set_vault_subtitles(ModelRc::new(VecModel::from(subtitles)));
    app.set_current_vault(current.map_or(-1, |i| i as i32));
    settings.set_vaults(ModelRc::new(VecModel::from(ui_vaults)));
}

/// Switches to `vault_dir`, reloading everything that's per-vault: the
/// token store, the remote URL, the note list, and the open editor.
pub fn switch_vault(app: &App, session: &SessionRef, vault_dir: PathBuf) {
    if let Err(e) = std::fs::create_dir_all(&vault_dir) {
        set_error(app, format!("Could not open this vault: {e}"));
        return;
    }
    let token_store = {
        let s = session.borrow();
        let store_res = s.open_token_store(&vault_dir);
        match store_res {
            Ok(t) => t,
            Err(e) => {
                drop(s);
                set_error(app, format!("Could not open this vault: {e}"));
                return;
            }
        }
    };

    let mut s = session.borrow_mut();
    let remote = s.app_data.load_remote(&vault_dir);
    show_vault_identity(app, &vault_dir);
    let _ = s.app_data.save_current_vault(&vault_dir);
    let index = immermemo_core::Session::open_index(&s.app_data, &vault_dir);
    s.vault_dir = vault_dir;
    s.token_store = token_store;
    s.remote = remote.clone();
    s.index = index;
    s.search_query.clear();
    s.editor = EditorState::default();
    s.note_nav.clear();
    s.history_preview_cache.clear();
    s.selection.clear();
    s.pending_auto_sync = false;
    s.last_synced_at = None;
    drop(s);

    sync_nav_state(app, session);
    super::sync_selection_to_ui(app, session);
    app.set_search_query(SharedString::new());
    app.set_remote_configured(remote.is_some());
    app.set_current_title(SharedString::new());
    update_rendered_body(app, session, "");
    app.set_last_sync_error(SharedString::new());
    app.set_sync_error_copied(false);
    app.set_last_synced_at(SharedString::new());
    clear_note_stats(app);
    show_history_state(app, &session.borrow());
    refresh_list(app, session);
    refresh_vault_list(app, session);
    crate::callbacks::refresh_accounts_ui(app, &session.borrow());
    if session.borrow().notes.is_empty() {
        app.set_list_open(true);
    } else {
        app.set_list_open(false);
        open_initial_or_last_note(app, session);
    }
    if remote.is_some() {
        schedule_auto_sync(app, std::time::Duration::from_millis(500));
    }
}
