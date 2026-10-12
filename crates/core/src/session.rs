//! Core session state management and note operations, independent of GUI frameworks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use immermemo_editor::EditorState;
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
        immermemo_editor::current_path(&self.editor, &self.notes)
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

    /// Creates a new note in the vault and indexes it.
    pub fn create_note(&mut self) -> anyhow::Result<PathBuf> {
        let path = notes::create(&self.vault_dir)?;
        if let Ok(rel) = path.strip_prefix(&self.vault_dir) {
            let _ = self.index.record_write(&self.vault_dir, rel, "");
        }
        let _ = self.refresh_list_data();
        let _ = self.update_filtered_list_data();
        Ok(path)
    }

    /// Renames the note at `note_index` to `new_title`.
    pub fn rename_note(&mut self, note_index: usize, new_title: &str) -> anyhow::Result<PathBuf> {
        let current_path = self.notes.get(note_index).ok_or_else(|| {
            anyhow::anyhow!("note index out of bounds")
        })?.clone();
        let new_path = notes::rename(&current_path, new_title)?;
        if let (Ok(old_rel), Ok(new_rel)) = (
            current_path.strip_prefix(&self.vault_dir),
            new_path.strip_prefix(&self.vault_dir),
        ) {
            let _ = self.index.record_rename(old_rel, new_rel);
        }
        let _ = self.refresh_list_data();
        let _ = self.update_filtered_list_data();
        Ok(new_path)
    }

    /// Deletes the note at `note_index`.
    pub fn delete_note(&mut self, note_index: usize) -> anyhow::Result<()> {
        let path = self.notes.get(note_index).ok_or_else(|| {
            anyhow::anyhow!("note index out of bounds")
        })?.clone();
        notes::delete(&path)?;
        if let Ok(rel) = path.strip_prefix(&self.vault_dir) {
            let _ = self.index.record_delete(rel);
        }
        let _ = self.refresh_list_data();
        let _ = self.update_filtered_list_data();
        Ok(())
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
