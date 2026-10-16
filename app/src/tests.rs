use super::*;
use crate::session::SessionRef;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::session::{SESSION, Session, refresh_list};
use crate::session::{
    apply_icon_selection, copy_to_clipboard, filter_icon_picker, finish_sync,
    open_first_conflicted_note, open_icon_picker, resolve_active_conflict, resolve_conflict_step,
    set_error, set_status, sync_conflict_sheet_state, update_current_icon, update_filtered_list,
};
use immermemo_core::{AppData, History, notes};
use slint::ComponentHandle;
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
    a_note_s_image_icon_appears_once_its_thumbnail_is_ready(&app, &session, &vault_dir);
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
    home_actions_config_applies_to_settings(&app);

    // `render::seam_tests` needs the same shared `app` for the same
    // one-event-loop-per-process reason -- see that module's doc.
    render::seam_tests::stacked_items_convert_into_the_generated_model(&app);
    render::seam_tests::a_wide_inline_conflict_paragraph_flows_onto_one_line(&app);
    render::seam_tests::a_narrow_width_wraps_an_inline_conflict_paragraph_onto_several_lines(&app);
    render::seam_tests::a_doc_icon_resolves_to_a_real_image_and_an_unknown_one_to_empty(&app);
    render::seam_tests::task_items_convert_into_the_generated_model(&app);
    render::seam_tests::inline_conflict_maintains_identity_and_order_across_resizes(&app);
    render::seam_tests::list_items_carry_depth_prefix_and_links(&app);
    render::seam_tests::heading_items_carry_level_in_generated_model(&app);
    render::seam_tests::code_and_callout_items_convert_into_the_generated_model(&app);
}

fn status_classification_tracks_is_error(app: &App) {
    set_error(app, "Save failed: permission denied");
    assert!(app.get_status_is_error());
    assert_eq!(app.get_status(), "Save failed: permission denied");

    set_status(app, "Syncing...");
    assert!(!app.get_status_is_error());
    assert_eq!(app.get_status(), "Syncing...");

    set_error(app, "Sync failed: network timeout");
    assert!(app.get_status_is_error());

    // What the message says no longer decides how it is shown.
    set_status(app, "Created note \"error handling\"");
    assert!(!app.get_status_is_error());

    set_status(app, "Synced");
    assert!(!app.get_status_is_error());
}

fn set_up_session_with_one_conflicted_note(
    app: &App,
) -> (tempfile::TempDir, SessionRef, PathBuf, PathBuf) {
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
    let token_store: Arc<dyn TokenStore> = Arc::new(immermemo_core::PlainFileTokenStore::new(
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
    session: &SessionRef,
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
    session: &SessionRef,
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
    session: &SessionRef,
    vault_dir: &Path,
) {
    app.set_active_tab(2);
    app.set_files_page(1);
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
    assert_eq!(starred.icon.kind, NoteIconKind::Glyph);
    assert!(starred.icon.glyph.size().width > 0);

    let unstarred = (0..entries.row_count())
        .map(|i| entries.row_data(i).unwrap())
        .find(|e| e.display == "test")
        .expect("test.tmt (no meta icon) is also in the root folder's entries");
    assert_eq!(unstarred.icon.kind, NoteIconKind::Empty);

    let cake = vault_dir.join("cake.tmt");
    std::fs::write(&cake, "@meta{ icon: \"🐾🩵🔥\" }\n\nCake.\n").unwrap();
    refresh_list(app, session);
    let entries = app.get_directory_folder_entries();
    let cake = (0..entries.row_count())
        .map(|i| entries.row_data(i).unwrap())
        .find(|e| e.display == "cake")
        .expect("cake.tmt is in the root folder's entries");
    assert_eq!(cake.icon.kind, NoteIconKind::Text);
    assert_eq!(cake.icon.cells.row_count(), 3);
}

/// An image icon takes the same path as every other kind, but its picture
/// comes from `thumbs`: empty (so the fallback glyph shows) until the worker
/// has produced the thumbnail, then the real, downscaled image on the next
/// refresh. A reference that leads nowhere readable stays empty.
fn a_note_s_image_icon_appears_once_its_thumbnail_is_ready(
    app: &App,
    session: &SessionRef,
    vault_dir: &Path,
) {
    let thumbs = crate::thumbs::tests::install_for_tests(vault_dir.join("../thumb-cache"));
    app.set_active_tab(2);
    app.set_files_page(1);
    std::fs::create_dir_all(vault_dir.join("images")).unwrap();
    std::fs::write(
        vault_dir.join("images/cover.png"),
        crate::thumbs::tests::png(800, 400),
    )
    .unwrap();
    std::fs::write(
        vault_dir.join("covered.tmt"),
        "@meta{ icon: \"images/cover.png\" }\n\nHas a cover.\n",
    )
    .unwrap();
    std::fs::write(
        vault_dir.join("lost.tmt"),
        "@meta{ icon: \"images/missing.png\" }\n\nCover is gone.\n",
    )
    .unwrap();
    session.borrow_mut().search_query = String::new();

    let entry = |name: &str| {
        let entries = app.get_directory_folder_entries();
        (0..entries.row_count())
            .map(|i| entries.row_data(i).unwrap())
            .find(|e| e.display == name)
            .unwrap_or_else(|| panic!("{name}.tmt is in the root folder's entries"))
    };

    refresh_list(app, session);
    let covered = entry("covered");
    assert_eq!(covered.icon.kind, NoteIconKind::Picture);
    assert_eq!(covered.icon.picture.size().width, 0, "not produced yet");

    thumbs.settle();
    refresh_list(app, session);
    let covered = entry("covered");
    assert_eq!(covered.icon.kind, NoteIconKind::Picture);
    assert_eq!(
        (
            covered.icon.picture.size().width,
            covered.icon.picture.size().height
        ),
        (256, 128)
    );

    let lost = entry("lost");
    assert_eq!(lost.icon.kind, NoteIconKind::Picture);
    assert_eq!(lost.icon.picture.size().width, 0);

    // The mini card resolves the same reference against the open note.
    app.set_current_note_path("covered.tmt".into());
    update_current_icon(
        app,
        session,
        "@meta{ icon: \"images/cover.png\" }\n\nHas a cover.\n",
    );
    assert_eq!(app.get_current_icon().kind, NoteIconKind::Picture);
    assert_eq!(app.get_current_icon().picture.size().width, 256);
    app.set_current_note_path("".into());
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
    session: &SessionRef,
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
    assert_eq!(row.icon.kind, NoteIconKind::Glyph);
    assert!(row.icon.glyph.size().width > 0);
}

fn conflict_sheet_steps_through_and_resolves_every_conflict(
    app: &App,
    session: &SessionRef,
    vault_dir: &Path,
) -> PathBuf {
    let multi_note_path = vault_dir.join("multi_conflict.tmt");
    let base_doc =
        tomet::parser::parse_document("Alpha base.\n\nMiddle untouched.\n\nBeta base.\n").unwrap();
    let local_doc =
        tomet::parser::parse_document("Alpha mine.\n\nMiddle untouched.\n\nBeta mine.\n").unwrap();
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
    session: &SessionRef,
    vault_dir: &Path,
) {
    let note_path = vault_dir.join("editor_quick_resolve.tmt");
    let base_doc =
        tomet::parser::parse_document("Alpha base.\n\nMiddle untouched.\n\nBeta base.\n").unwrap();
    let local_doc =
        tomet::parser::parse_document("Alpha mine.\n\nMiddle untouched.\n\nBeta mine.\n").unwrap();
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

fn sync_error_is_tracked_copied_and_dismissed(app: &App, session: &SessionRef) {
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
    session: &SessionRef,
    multi_note_path: &Path,
) {
    let ts = immermemo_core::format_timestamp(1700000000);
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
    session: &SessionRef,
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

    apply_bottom_bar_config(&settings, "home,search,new_note,library");
    assert!(settings.get_nav_show_home());
    assert!(settings.get_nav_show_search());
    assert!(settings.get_nav_show_new_note());
    assert!(settings.get_nav_show_library());
    assert!(!settings.get_nav_show_directory());
    assert!(!settings.get_nav_show_daily());
    assert!(!settings.get_nav_show_starred());
    assert!(!settings.get_nav_show_recent());
    assert!(!settings.get_nav_show_graph());
    assert!(!settings.get_nav_show_sync());

    apply_bottom_bar_config(&settings, "home,daily,starred,recent,graph,sync");
    assert!(settings.get_nav_show_home());
    assert!(!settings.get_nav_show_search());
    assert!(!settings.get_nav_show_new_note());
    assert!(!settings.get_nav_show_library());
    assert!(settings.get_nav_show_daily());
    assert!(settings.get_nav_show_starred());
    assert!(settings.get_nav_show_recent());
    assert!(settings.get_nav_show_graph());
    assert!(settings.get_nav_show_sync());

    // Test legacy "quick" migration: expands into new_note, sync, switch_vault, settings
    apply_bottom_bar_config(&settings, "home,search,library,quick");
    assert!(settings.get_nav_show_home());
    assert!(settings.get_nav_show_search());
    assert!(settings.get_nav_show_library());
    assert!(settings.get_nav_show_new_note());
    assert!(settings.get_nav_show_sync());
    assert!(settings.get_nav_show_switch_vault());
    assert!(settings.get_nav_show_settings());
    assert!(!settings.get_nav_show_directory());

    let serialized = action::serialize_bottom_bar_config(&settings);
    assert_eq!(
        serialized,
        "home,search,library,new_note,sync,switch_vault,settings"
    );

    // Restore default
    apply_bottom_bar_config(&settings, "home,search,new_note,library");
}

fn home_actions_config_applies_to_settings(app: &App) {
    let settings = app.global::<SettingsAdapter>();

    action::apply_home_actions_config(&settings, "new_note,search,daily");
    assert!(settings.get_home_show_new_note());
    assert!(settings.get_home_show_search());
    assert!(settings.get_home_show_daily());
    assert!(!settings.get_home_show_library());
    assert!(!settings.get_home_show_directory());
    assert!(!settings.get_home_show_starred());
    assert!(!settings.get_home_show_recent());
    assert!(!settings.get_home_show_graph());
    assert!(!settings.get_home_show_sync());

    let enabled: Vec<i32> = settings.get_home_enabled_actions().iter().collect();
    assert_eq!(
        enabled,
        vec![
            action::ActionId::NewNote.to_i32(),
            action::ActionId::Search.to_i32(),
            action::ActionId::Daily.to_i32(),
        ]
    );

    let rows: Vec<HomeQuickActionRow> = settings.get_home_quick_action_rows().iter().collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].empty_slots, 0);
    assert_eq!(rows[0].items.iter().count(), 3);

    // 5 items across 2 rows
    action::apply_home_actions_config(&settings, "new_note,search,daily,directory,starred");
    let rows5: Vec<HomeQuickActionRow> = settings.get_home_quick_action_rows().iter().collect();
    assert_eq!(rows5.len(), 2);
    assert_eq!(rows5[0].items.iter().count(), 3);
    assert_eq!(rows5[0].empty_slots, 0);
    assert_eq!(rows5[1].items.iter().count(), 2);
    assert_eq!(rows5[1].empty_slots, 1);

    let serialized = action::serialize_home_actions_config(&settings);
    assert_eq!(serialized, "new_note,search,daily,directory,starred");

    // Empty config defaults to NewNote
    action::apply_home_actions_config(&settings, "");
    assert!(settings.get_home_show_new_note());
    assert!(!settings.get_home_show_search());
    let enabled_default: Vec<i32> = settings.get_home_enabled_actions().iter().collect();
    assert_eq!(enabled_default, vec![action::ActionId::NewNote.to_i32()]);
    let rows_default: Vec<HomeQuickActionRow> = settings.get_home_quick_action_rows().iter().collect();
    assert_eq!(rows_default.len(), 1);
    assert_eq!(rows_default[0].items.iter().count(), 1);
    assert_eq!(rows_default[0].empty_slots, 2);

    // Restore
    action::apply_home_actions_config(&settings, "new_note");
}
