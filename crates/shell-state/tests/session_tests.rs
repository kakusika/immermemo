use std::sync::Arc;
use tempfile::tempdir;

use immermemo_core::{AppData, PlainFileTokenStore, StartVault, TokenStore};
use immermemo_shell_state::*;

#[test]
fn test_shell_session_initial_state() {
    let tmp = tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    std::fs::create_dir_all(&vault_dir).unwrap();
    let app_data = AppData::new(tmp.path().join("app_data"));
    let token_store = Arc::new(PlainFileTokenStore::new(tmp.path().join("token")));

    let core = immermemo_core::Session::start(
        app_data,
        Box::new(move |_| Ok(token_store.clone() as Arc<dyn TokenStore>)),
        StartVault {
            first_run: vault_dir.clone(),
            open: Some(vault_dir),
        },
    )
    .unwrap();

    let mut session = ShellSession::new(core);

    assert_eq!(session.active_tab, ShellTab::Home);
    assert_eq!(session.files_page, FilesPage::Library);
    assert_eq!(session.active_sheet, ShellSheet::None);
    assert!(!session.has_selection());

    // Selection tests
    session.toggle_selection("note1.tmt");
    assert!(session.has_selection());
    assert!(session.selection.is_selected("note1.tmt"));

    session.select_only("note2.tmt");
    assert!(!session.selection.is_selected("note1.tmt"));
    assert!(session.selection.is_selected("note2.tmt"));

    session.clear_selection();
    assert!(!session.has_selection());

    // Select all in view
    let paths = vec!["note1.tmt".to_string(), "note2.tmt".to_string()];
    session.select_all_in_view(&paths);
    assert!(session.selection.is_selected("note1.tmt"));
    assert!(session.selection.is_selected("note2.tmt"));

    // Toggling select all in view clears if all are selected
    session.select_all_in_view(&paths);
    assert!(!session.has_selection());
}

#[test]
fn test_shell_session_icon_picker_flow() {
    let tmp = tempdir().unwrap();
    let vault_dir = tmp.path().join("vault");
    std::fs::create_dir_all(&vault_dir).unwrap();
    let note_path = vault_dir.join("test.tmt");
    std::fs::write(&note_path, "Hello world\n").unwrap();

    let app_data = AppData::new(tmp.path().join("app_data"));
    let token_store = Arc::new(PlainFileTokenStore::new(tmp.path().join("token")));

    let mut core = immermemo_core::Session::start(
        app_data,
        Box::new(move |_| Ok(token_store.clone() as Arc<dyn TokenStore>)),
        StartVault {
            first_run: vault_dir.clone(),
            open: Some(vault_dir),
        },
    )
    .unwrap();

    core.refresh_list_data();
    let mut session = ShellSession::new(core);
    assert_eq!(session.notes.len(), 1);

    session.icon_picker.open(0);
    assert!(session.icon_picker.is_open);
    assert_eq!(session.icon_picker.target_note_index, Some(0));

    let updated = session.apply_icon_selection("star").unwrap();
    assert_eq!(updated, Some(note_path.clone()));

    let new_content = std::fs::read_to_string(&note_path).unwrap();
    assert!(new_content.contains("::meta{icon: @doc.icon(star, pkg: tabler)}"));
    assert!(session.directory_view.is_dirty);
}
