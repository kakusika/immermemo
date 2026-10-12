//! Core session state management and note operations, independent of GUI frameworks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::editor::EditorState;
use immermemo_index::NoteIndex;
use immermemo_vault::appdata::AppData;
use immermemo_vault::credentials::TokenStore;
use immermemo_vault::notes;

use crate::navigation::BackForwardStack;

/// Factory for creating vault-specific token stores.
pub type TokenStoreFactory =
    Box<dyn Fn(&Path) -> anyhow::Result<Arc<dyn TokenStore>> + 'static>;

/// Results of updating the filtered list according to search queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilteredListOutcome {
    pub names: Vec<String>,
    pub conflicted: Vec<bool>,
    pub snippets: Vec<String>,
    pub ui_current: Option<usize>,
}

/// Results of refreshing the vault note list from filesystem and index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshListOutcome {
    pub conflict_count: usize,
}

/// What the application is currently editing and browsing.
pub struct Session {
    pub vault_dir: PathBuf,
    pub app_data: AppData,
    pub token_store: Arc<dyn TokenStore>,
    pub token_store_for: TokenStoreFactory,
    pub known_vaults: Vec<PathBuf>,
    pub remote: Option<String>,
    pub index: NoteIndex,
    pub notes: Vec<PathBuf>,
    pub conflicted: Vec<bool>,
    pub search_query: String,
    pub filtered_results: Vec<notes::SearchResult>,
    pub editor: EditorState,
    pub rename_target: Option<usize>,
    pub delete_target: Option<usize>,
    pub delete_vault_target: Option<usize>,
    pub icon_picker_target: Option<usize>,
    pub pending_auto_sync: bool,
    pub last_synced_at: Option<std::time::SystemTime>,
    pub current_folder: String,
    pub directory_source: Vec<(String, i32, bool)>,
    pub note_nav: BackForwardStack,
}

impl Session {
    /// Constructs a new Session with initialized defaults.
    pub fn new(
        vault_dir: PathBuf,
        app_data: AppData,
        token_store: Arc<dyn TokenStore>,
        token_store_for: TokenStoreFactory,
        known_vaults: Vec<PathBuf>,
        remote: Option<String>,
        index: NoteIndex,
    ) -> Self {
        Self {
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
            editor: EditorState::default(),
            rename_target: None,
            delete_target: None,
            delete_vault_target: None,
            icon_picker_target: None,
            pending_auto_sync: false,
            last_synced_at: None,
            current_folder: String::new(),
            directory_source: Vec::new(),
            note_nav: BackForwardStack::default(),
        }
    }

    /// Returns the absolute path of the currently open note, if any.
    pub fn current_path(&self) -> Option<PathBuf> {
        crate::editor::current_path(&self.editor, &self.notes)
    }

    /// Opens the note at `index` in `self.notes`.
    pub fn open_note(&mut self, index: usize) -> Option<Result<crate::editor::OpenedNote, String>> {
        let vault_dir = self.vault_dir.clone();
        crate::editor::open_note(
            &mut self.editor,
            &self.app_data,
            &vault_dir,
            &self.notes,
            &self.conflicted,
            &self.filtered_results,
            index,
        )
    }

    /// Selects the first note to open on launch (either last opened or first in list).
    pub fn open_initial_or_last_note(&mut self) -> Option<Result<crate::editor::OpenedNote, String>> {
        let idx = crate::editor::initial_or_last_note_index(
            &self.app_data,
            &self.vault_dir,
            &self.notes,
        )?;
        self.open_note(idx)
    }

    /// Opens the first note containing an unresolved conflict marker, if any.
    pub fn open_first_conflicted_note(&mut self) -> Option<Result<crate::editor::OpenedNote, String>> {
        let idx = crate::editor::first_conflicted_note_index(
            &self.editor,
            &self.conflicted,
        )?;
        self.open_note(idx)
    }

    /// Opens a note by its vault-relative path.
    pub fn open_note_by_rel_path(&mut self, rel_path: &Path) -> Option<Result<crate::editor::OpenedNote, String>> {
        let target = self.vault_dir.join(rel_path);
        let idx = self.notes.iter().position(|p| *p == target)?;
        self.open_note(idx)
    }

    /// Creates a new empty note and updates index.
    pub fn create_note(&mut self) -> anyhow::Result<PathBuf> {
        let dir = self.vault_dir.clone();
        let path = notes::create(&dir)?;
        if let Ok(rel) = path.strip_prefix(&dir) {
            let _ = self.index.record_write(&dir, rel, "");
        }
        self.search_query.clear();
        let _ = self.refresh_list_data();
        let _ = self.update_filtered_list_data();
        Ok(path)
    }

    /// Renames the note at `target_idx` to `new_name` and updates the index.
    pub fn rename_note(&mut self, target_idx: usize, new_name: &str) -> anyhow::Result<PathBuf> {
        let old_path = self
            .notes
            .get(target_idx)
            .ok_or_else(|| anyhow::anyhow!("invalid note index"))?
            .clone();
        let new_path = notes::rename(&old_path, new_name)?;
        if let (Ok(old_rel), Ok(new_rel)) = (
            old_path.strip_prefix(&self.vault_dir),
            new_path.strip_prefix(&self.vault_dir),
        ) {
            let _ = self.index.record_rename(old_rel, new_rel);
        }
        let _ = self.refresh_list_data();
        let _ = self.update_filtered_list_data();
        Ok(new_path)
    }

    /// Deletes the note at `target_idx` and removes it from the index.
    pub fn delete_note(&mut self, target_idx: usize) -> anyhow::Result<PathBuf> {
        let path = self
            .notes
            .get(target_idx)
            .ok_or_else(|| anyhow::anyhow!("invalid note index"))?
            .clone();
        notes::delete(&path)?;
        if let Ok(rel) = path.strip_prefix(&self.vault_dir) {
            let _ = self.index.record_delete(rel);
        }
        let _ = self.refresh_list_data();
        let _ = self.update_filtered_list_data();
        Ok(path)
    }

    /// Resolves one conflict step at `step_idx` choosing `side` (0: base, 1: local, 2: remote).
    pub fn resolve_conflict_step(
        &mut self,
        step_idx: i32,
        side: i32,
    ) -> Option<Result<crate::editor::ConflictResolved, String>> {
        let vault_dir = self.vault_dir.clone();
        crate::editor::resolve_conflict_step(
            &mut self.editor,
            &vault_dir,
            &self.notes,
            &mut self.conflicted,
            &mut self.index,
            step_idx,
            side,
        )
    }

    /// Resolves the active conflict using the specified resolution.
    pub fn resolve_active_conflict(
        &mut self,
        resolution: tomet::merge::ConflictResolution,
    ) -> Option<Result<crate::editor::ConflictResolved, String>> {
        let vault_dir = self.vault_dir.clone();
        crate::editor::resolve_active_conflict(
            &mut self.editor,
            &vault_dir,
            &self.notes,
            &mut self.conflicted,
            &mut self.index,
            resolution,
        )
    }

    /// Computes conflict resolution sheet state for the open note.
    pub fn conflict_sheet_state(&self, requested_index: i32) -> crate::editor::ConflictSheetState {
        crate::editor::conflict_sheet_state(
            &self.editor,
            &self.notes,
            requested_index,
            &self.app_data,
            &self.vault_dir,
        )
    }

    /// Sets or updates the note icon meta attribute for `self.icon_picker_target`.
    pub fn set_note_icon(&mut self, icon_name: &str, pkg: &str) -> anyhow::Result<bool> {
        let Some(target) = self.icon_picker_target else {
            return Ok(false);
        };
        let Some(path) = self.notes.get(target).cloned() else {
            return Ok(false);
        };
        let body = std::fs::read_to_string(&path)?;
        let Some(new_body) = crate::editor::set_note_icon(&body, icon_name, pkg) else {
            return Ok(false);
        };
        std::fs::write(&path, &new_body)?;
        if let Ok(rel) = path.strip_prefix(&self.vault_dir) {
            let _ = self.index.record_write(&self.vault_dir, rel, &new_body);
        }
        Ok(true)
    }

    /// Loads Git revision history for the open note up to `limit` entries.
    pub fn load_note_history(&mut self, limit: usize) -> Option<Vec<immermemo_sync::FileRevision>> {
        let current_path = self.current_path();
        let revisions = crate::editor::load_note_history(
            &self.app_data,
            &self.vault_dir,
            current_path,
            limit,
        )?;
        self.editor.note_history_revisions = revisions.clone();
        Some(revisions)
    }

    /// Restores a historical revision for the open note.
    pub fn restore_note_version(
        &mut self,
        version_idx: usize,
    ) -> Option<Result<crate::editor::RestoredVersion, String>> {
        let vault_dir = self.vault_dir.clone();
        crate::editor::restore_note_version(
            &mut self.editor,
            &vault_dir,
            &self.notes,
            &mut self.index,
            version_idx,
        )
    }

    /// Applies an undo/redo restored state, writing file changes and updating conflict tracking.
    pub fn apply_restored(
        &mut self,
        restored: crate::editor::Restored,
    ) -> crate::editor::RestoredNote {
        let result = crate::editor::apply_restored(&self.editor, &self.notes, restored);
        if let Some(idx) = self.editor.current
            && idx < self.conflicted.len()
        {
            self.conflicted[idx] = result.has_conflict;
        }
        result
    }

    /// Loads vault-wide Git revision history.
    pub fn vault_history(&self, limit: usize) -> Vec<immermemo_sync::CommitSummary> {
        let Ok(gitdir) = self.app_data.gitdir(&self.vault_dir) else {
            return Vec::new();
        };
        match immermemo_sync::Vault::open(&self.vault_dir, &gitdir) {
            Ok(v) => v.vault_history(limit).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// Reconciles filesystem notes with the SQLite index and updates internal note lists.
    pub fn refresh_list_data(&mut self) -> RefreshListOutcome {
        let previous = self.current_path();
        let vault_dir = self.vault_dir.clone();
        let _ = self.index.reconcile_filesystem(&vault_dir);
        let all = self.index.list_all().unwrap_or_default();
        let vault_dir = self.vault_dir.clone();
        self.notes = all.iter().map(|n| vault_dir.join(&n.path)).collect();
        self.conflicted = all.iter().map(|n| n.has_conflict).collect();
        let conflict_count = self.conflicted.iter().filter(|&&c| c).count();
        self.editor.current = previous.and_then(|p| self.notes.iter().position(|n| *n == p));
        RefreshListOutcome { conflict_count }
    }

    /// Performs search filtering and prepares directory source cache.
    pub fn update_filtered_list_data(&mut self) -> FilteredListOutcome {
        let hits = self.index.search(&self.search_query).unwrap_or_default();
        let vault_dir = self.vault_dir.clone();

        let mut filtered_results = Vec::with_capacity(hits.len());
        let mut names = Vec::with_capacity(hits.len());
        let mut conflicted = Vec::with_capacity(hits.len());
        let mut snippets = Vec::with_capacity(hits.len());
        let mut directory_source = Vec::with_capacity(hits.len());

        for hit in hits {
            let abs_path = vault_dir.join(&hit.path);
            if let Some(index) = self.notes.iter().position(|p| *p == abs_path) {
                directory_source.push((hit.title.clone(), index as i32, hit.has_conflict));
                filtered_results.push(notes::SearchResult {
                    note_index: index,
                    snippet: hit.snippet.clone(),
                });
                names.push(hit.title);
                conflicted.push(hit.has_conflict);
                snippets.push(hit.snippet.unwrap_or_default());
            }
        }
        directory_source.sort_by(|a: &(String, i32, bool), b| a.0.cmp(&b.0));

        self.filtered_results = filtered_results;
        self.directory_source = directory_source;
        let ui_current = self
            .editor
            .current
            .and_then(|cur| self.filtered_results.iter().position(|r| r.note_index == cur));

        FilteredListOutcome {
            names,
            conflicted,
            snippets,
            ui_current,
        }
    }
}

/// Derives a clean display name for a vault directory.
pub fn vault_display_name(vault_dir: &Path) -> String {
    vault_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| vault_dir.display().to_string())
}

/// Resolves a user-typed vault path input (expanding `~`).
pub fn vault_path_from_input(input: &str) -> PathBuf {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()) {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct DummyTokenStore;
    impl TokenStore for DummyTokenStore {
        fn load(&self) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
        fn save(&self, _token: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn test_session(vault_dir: PathBuf, app_data: AppData) -> Session {
        let token_store: Arc<dyn TokenStore> = Arc::new(DummyTokenStore);
        Session::new(
            vault_dir.clone(),
            app_data,
            token_store.clone(),
            Box::new(move |_| Ok(token_store.clone())),
            vec![vault_dir],
            None,
            NoteIndex::open_in_memory().unwrap(),
        )
    }

    #[test]
    fn session_note_crud_and_refresh() {
        let tmp = TempDir::new().unwrap();
        let vault_dir = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        let app_data = AppData::new(tmp.path().join("data"));

        let mut session = test_session(vault_dir, app_data);

        // 1. Create notes
        let path1 = session.create_note().unwrap();
        assert!(path1.exists());
        let path2 = session.create_note().unwrap();
        assert!(path2.exists());

        let outcome = session.refresh_list_data();
        assert_eq!(session.notes.len(), 2);
        assert_eq!(outcome.conflict_count, 0);

        let filtered = session.update_filtered_list_data();
        assert_eq!(filtered.names.len(), 2);

        // 2. Rename note
        let renamed = session.rename_note(0, "Renamed Note").unwrap();
        assert!(renamed.exists());

        // 3. Delete note
        session.delete_note(1).unwrap();
        assert_eq!(session.notes.len(), 1);
    }

    #[test]
    fn test_vault_display_name() {
        assert_eq!(vault_display_name(Path::new("/home/user/my-vault")), "my-vault");
        assert_eq!(vault_display_name(Path::new("relative-vault")), "relative-vault");
    }
}
