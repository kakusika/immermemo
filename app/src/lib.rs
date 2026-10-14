mod callbacks;
pub mod cert;
pub mod credentials;
pub mod forgejo;
pub mod github;
pub mod gitlab;
mod haptic;
pub mod qr;
pub mod render;
mod session;
mod sync;
pub mod transfer;
mod widget;

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
use std::cell::Cell;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::ComponentHandle;

pub use forgejo::update_forgejo_ui_state;
pub use github::update_github_ui_state;
pub use gitlab::update_gitlab_ui_state;
use immermemo_core::AppData;
pub use immermemo_core::{TokenStore, TokenStoreFactory};
pub use transfer::handle_incoming_import_uri;
#[cfg(test)]
pub(crate) use transfer::open_import_preview;

/// Updates UI connection state for all supported Git hosting providers (GitHub, Forgejo/Gitea, GitLab).
pub fn update_provider_ui_state(app: &App, remote: Option<&str>) {
    github::update_github_ui_state(app, remote);
    forgejo::update_forgejo_ui_state(app, remote);
    gitlab::update_gitlab_ui_state(app, remote);
}

/// Applies comma-separated bottom bar configuration keys to the SettingsAdapter.
pub fn apply_bottom_bar_config(settings: &SettingsAdapter, config: &str) {
    let raw_items: Vec<&str> = config
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let set: std::collections::HashSet<&str> = raw_items.iter().copied().collect();
    settings.set_nav_show_home(set.contains("home"));
    settings.set_nav_show_search(set.contains("search"));
    settings.set_nav_show_library(set.contains("library"));
    settings.set_nav_show_directory(set.contains("directory"));
    settings.set_nav_show_daily(set.contains("daily"));
    settings.set_nav_show_starred(set.contains("starred"));
    settings.set_nav_show_recent(set.contains("recent"));
    settings.set_nav_show_graph(set.contains("graph"));
    settings.set_nav_show_quick(set.contains("quick"));

    let key_to_id = |k: &str| -> Option<i32> {
        match k {
            "home" => Some(0),
            "search" => Some(1),
            "library" => Some(2),
            "directory" => Some(3),
            "daily" => Some(4),
            "starred" => Some(5),
            "recent" => Some(6),
            "graph" => Some(7),
            "quick" => Some(8),
            _ => None,
        }
    };
    let mut order: Vec<i32> = Vec::new();
    for key in raw_items {
        if let Some(id) = key_to_id(key) {
            if !order.contains(&id) {
                order.push(id);
            }
        }
    }
    for default_id in 0..9 {
        if !order.contains(&default_id) {
            order.push(default_id);
        }
    }
    settings.set_nav_order(std::rc::Rc::new(slint::VecModel::from(order)).into());
}

use session::{
    SESSION, Session, open_initial_or_last_note, open_note_by_rel_path, refresh_list,
    refresh_vault_list, vault_display_name,
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
    let settings = app.global::<SettingsAdapter>();
    let font_size_choice = app_data.load_font_size().unwrap_or(1);
    let theme_choice = app_data.load_theme().unwrap_or(0);
    settings.set_font_size_choice(font_size_choice);
    app.set_font_size_choice(font_size_choice);
    app.set_editor_font_size(match font_size_choice {
        0 => 14.0,
        2 => 20.0,
        _ => 17.0,
    });
    settings.set_theme_choice(theme_choice);
    app.set_theme_choice(theme_choice);
    let status_bar_choice = app_data.load_status_bar_choice().unwrap_or(0);
    settings.set_status_bar_choice(status_bar_choice);
    app.set_status_bar_choice(status_bar_choice);
    let bottom_bar_config = app_data
        .load_bottom_bar_config()
        .unwrap_or_else(|| "home,search,library,quick".to_string());
    apply_bottom_bar_config(&settings, &bottom_bar_config);
    #[cfg(target_os = "android")]
    let status_bar_choice_cell = Rc::new(Cell::new(status_bar_choice));
    // 2 (always hidden) takes effect immediately; 0/1 don't need an
    // initial call -- the window starts visible either way (AppTheme's
    // own default), and 1 only hides once a note is actually opened.
    #[cfg(target_os = "android")]
    if status_bar_choice == 2 {
        android::system_ui::set_status_bar_visible(false);
    }
    settings.set_current_vault_name(vault_display_name(&vault_dir).into());
    settings.set_current_vault_path(vault_dir.display().to_string().into());
    app.set_current_vault_name(vault_display_name(&vault_dir).into());
    app.set_current_vault_path(vault_dir.display().to_string().into());
    app.set_remote_configured(remote.is_some());
    update_provider_ui_state(&app, remote.as_deref());
    let ssh_pub = app_data.load_ssh_public_key();
    settings.set_ssh_key_present(ssh_pub.is_some());
    settings.set_ssh_public_key(ssh_pub.unwrap_or_default().into());
    settings.set_ssh_key_copied(false);
    let index = app_data
        .index_path(&vault_dir)
        .ok()
        .and_then(|p| immermemo_core::NoteIndex::open(&p).ok())
        .unwrap_or_else(|| immermemo_core::NoteIndex::open_in_memory().unwrap());
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

    callbacks::bind_note_callbacks(&app, &session);
    callbacks::bind_vault_callbacks(
        &app,
        &session,
        #[cfg(target_os = "android")]
        status_bar_choice_cell,
    );
    callbacks::bind_sync_conflict_callbacks(&app, &session);
    github::bind_github_callbacks(&app, &session);
    forgejo::bind_forgejo_callbacks(&app, &session);
    gitlab::bind_gitlab_callbacks(&app, &session);
    transfer::bind_transfer_callbacks(&app, &session, initial_import_uri);

    app.run()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        apply_icon_selection, copy_to_clipboard, filter_icon_picker, finish_sync,
        open_first_conflicted_note, open_icon_picker, resolve_active_conflict,
        resolve_conflict_step, set_status, sync_conflict_sheet_state, update_filtered_list,
    };
    use immermemo_core::{History, notes};
    use slint::{Model, SharedString};
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
        provider_ui_state_tracks_github_forgejo_and_gitlab(&app);
        bottom_bar_config_applies_to_settings(&app);

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
            immermemo_core::NoteIndex::open_in_memory().unwrap(),
        );
        core.notes = vec![note_path.clone()];
        core.conflicted = vec![true];
        core.filtered_results = vec![notes::SearchResult {
            note_index: 0,
            snippet: None,
        }];
        core.editor = immermemo_core::EditorState {
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
        resolve_active_conflict(app, session, immermemo_core::ConflictResolution::A);

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
    /// back, not just that `immermemo_core::set_note_icon` can in
    /// isolation (that's `crates/core`'s own unit tests' job).
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
        let (identity, _) = immermemo_render_tomet::meta_element(&saved, "icon")
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
        finish_sync(app, session, Ok(immermemo_core::SyncReport::default()));
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

        let rev1 = immermemo_core::FileRevision {
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
        finish_sync(app, session, Ok(immermemo_core::SyncReport::default()));
        assert!(app.get_syncing());
        finish_sync(app, session, Ok(immermemo_core::SyncReport::default()));
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
        finish_sync(app, session, Ok(immermemo_core::SyncReport::default()));
        assert!(app.get_syncing());
        finish_sync(app, session, Ok(immermemo_core::SyncReport::default()));
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
        let settings = app.global::<SettingsAdapter>();
        let pending = Rc::new(RefCell::new(None));
        let valid_payload = immermemo_core::transfer::TransferPayload {
            v: 1,
            app: "immermemo".to_string(),
            interface: Some(immermemo_core::transfer::InterfaceTransfer {
                font_size: Some(2),
                theme: Some(1),
                status_bar: Some(0),
                ui_scale: Some(1.2),
            }),
            sync: Some(immermemo_core::transfer::SyncTransfer {
                vault_name: Some("test-vault".to_string()),
                remote_url: Some("https://example.com/notes.git".to_string()),
                token: Some("secret123".to_string()),
            }),
        };
        let uri = immermemo_core::transfer::encode_payload(&valid_payload).unwrap();
        open_import_preview(app, &pending, &uri);

        assert!(app.get_settings_sheet_open());
        assert_eq!(app.get_settings_page(), 5);
        assert_eq!(settings.get_transfer_tab(), 1);
        assert!(settings.get_import_preview_ready());
        assert!(settings.get_preview_has_interface());
        assert!(settings.get_preview_has_remote());
        assert_eq!(
            settings.get_preview_remote_url(),
            "https://example.com/notes.git"
        );
        assert!(settings.get_preview_has_token());
        assert!(pending.borrow().is_some());
    }

    fn provider_ui_state_tracks_github_forgejo_and_gitlab(app: &App) {
        let settings = app.global::<SettingsAdapter>();
        // GitHub remote
        update_provider_ui_state(app, Some("https://github.com/octocat/notes.git"));
        assert!(settings.get_github_connected());
        assert_eq!(settings.get_github_username(), "octocat");
        assert_eq!(settings.get_github_repo_name(), "notes");
        assert!(!settings.get_forgejo_connected());
        assert!(!settings.get_gitlab_connected());

        // Forgejo / Codeberg remote
        update_provider_ui_state(app, Some("https://codeberg.org/alice/immermemo-vault.git"));
        assert!(!settings.get_github_connected());
        assert!(settings.get_forgejo_connected());
        assert_eq!(settings.get_forgejo_username(), "alice");
        assert_eq!(settings.get_forgejo_instance(), "codeberg.org");
        assert_eq!(settings.get_forgejo_repo_name(), "immermemo-vault");
        assert!(!settings.get_gitlab_connected());

        // Custom self-hosted Forgejo remote
        update_provider_ui_state(app, Some("https://git.example.com:3000/bob/my-notes"));
        assert!(!settings.get_github_connected());
        assert!(settings.get_forgejo_connected());
        assert_eq!(settings.get_forgejo_username(), "bob");
        assert_eq!(settings.get_forgejo_instance(), "git.example.com:3000");
        assert_eq!(settings.get_forgejo_repo_name(), "my-notes");
        assert!(!settings.get_gitlab_connected());

        // GitLab.com remote
        update_provider_ui_state(app, Some("https://gitlab.com/charlie/notes.git"));
        assert!(!settings.get_github_connected());
        assert!(!settings.get_forgejo_connected());
        assert!(settings.get_gitlab_connected());
        assert_eq!(settings.get_gitlab_username(), "charlie");
        assert_eq!(settings.get_gitlab_instance(), "gitlab.com");
        assert_eq!(settings.get_gitlab_repo_name(), "notes");

        // Self-hosted GitLab remote with nested groups
        update_provider_ui_state(app, Some("git@gitlab.company.org:subgroup/team/notes.git"));
        assert!(!settings.get_github_connected());
        assert!(!settings.get_forgejo_connected());
        assert!(settings.get_gitlab_connected());
        assert_eq!(settings.get_gitlab_username(), "subgroup/team");
        assert_eq!(settings.get_gitlab_instance(), "gitlab.company.org");
        assert_eq!(settings.get_gitlab_repo_name(), "notes");

        // Disconnected
        update_provider_ui_state(app, None);
        assert!(!settings.get_github_connected());
        assert!(!settings.get_forgejo_connected());
        assert!(!settings.get_gitlab_connected());
    }

    fn bottom_bar_config_applies_to_settings(app: &App) {
        let settings = app.global::<SettingsAdapter>();

        apply_bottom_bar_config(&settings, "home,search,library,quick");
        assert!(settings.get_nav_show_home());
        assert!(settings.get_nav_show_search());
        assert!(settings.get_nav_show_library());
        assert!(!settings.get_nav_show_directory());
        assert!(!settings.get_nav_show_daily());
        assert!(!settings.get_nav_show_starred());
        assert!(!settings.get_nav_show_recent());
        assert!(!settings.get_nav_show_graph());
        assert!(settings.get_nav_show_quick());

        apply_bottom_bar_config(&settings, "home,daily,starred,recent,graph");
        assert!(settings.get_nav_show_home());
        assert!(!settings.get_nav_show_search());
        assert!(!settings.get_nav_show_library());
        assert!(settings.get_nav_show_daily());
        assert!(settings.get_nav_show_starred());
        assert!(settings.get_nav_show_recent());
        assert!(settings.get_nav_show_graph());
        assert!(!settings.get_nav_show_quick());

        // Restore default
        apply_bottom_bar_config(&settings, "home,search,library,quick");
    }
}
