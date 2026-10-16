//! Core session state management and note operations, independent of GUI frameworks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;

use crate::editor::EditorState;
use immermemo_index::NoteIndex;
use immermemo_vault::appdata::{Account, AppData};
use immermemo_vault::credentials::TokenStore;
use immermemo_vault::notes;

use crate::navigation::BackForwardStack;

/// Factory for creating vault-specific token stores.
pub type TokenStoreFactory = Box<dyn Fn(&Path) -> anyhow::Result<Arc<dyn TokenStore>> + 'static>;

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
    pub fn open_initial_or_last_note(
        &mut self,
    ) -> Option<Result<crate::editor::OpenedNote, String>> {
        let idx = crate::editor::initial_or_last_note_index(
            &self.app_data,
            &self.vault_dir,
            &self.notes,
        )?;
        self.open_note(idx)
    }

    /// Opens the first note containing an unresolved conflict marker, if any.
    pub fn open_first_conflicted_note(
        &mut self,
    ) -> Option<Result<crate::editor::OpenedNote, String>> {
        let idx = crate::editor::first_conflicted_note_index(&self.editor, &self.conflicted)?;
        self.open_note(idx)
    }

    /// Opens a note by its vault-relative path.
    pub fn open_note_by_rel_path(
        &mut self,
        rel_path: &Path,
    ) -> Option<Result<crate::editor::OpenedNote, String>> {
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

    /// Creates a new note with the specified name and initial content, and updates index.
    pub fn create_named_note(&mut self, name: &str, content: &str) -> anyhow::Result<PathBuf> {
        let dir = self.vault_dir.clone();
        let sanitized = name
            .trim()
            .trim_end_matches(".tmt")
            .replace(['/', '\\'], "_");
        anyhow::ensure!(!sanitized.is_empty(), "note name cannot be empty");
        let path = dir.join(format!("{sanitized}.tmt"));
        if !path.exists() {
            std::fs::write(&path, content)?;
        }
        if let Ok(rel) = path.strip_prefix(&dir) {
            let _ = self.index.record_write(&dir, rel, content);
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
        let revisions =
            crate::editor::load_note_history(&self.app_data, &self.vault_dir, current_path, limit)?;
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

    /// Loads cached note lists from SQLite index without walking the filesystem.
    pub fn load_cached_list_data(&mut self) -> RefreshListOutcome {
        let previous = self.current_path();
        let all = self.index.list_all().unwrap_or_default();
        let vault_dir = self.vault_dir.clone();
        self.notes = all.iter().map(|n| vault_dir.join(&n.path)).collect();
        self.conflicted = all.iter().map(|n| n.has_conflict).collect();
        let conflict_count = self.conflicted.iter().filter(|&&c| c).count();
        self.editor.current = previous.and_then(|p| self.notes.iter().position(|n| *n == p));
        RefreshListOutcome { conflict_count }
    }

    /// Links this vault to `remote_url`, authenticated by `token` and
    /// registered under `account` (the remote and the token are the part
    /// that must succeed; the account bookkeeping is best-effort, since sync
    /// itself only needs the vault's own remote and token).
    ///
    /// Everything a provider's connect flow does after it has a repository,
    /// whichever way it authenticated -- OAuth device flow or a pasted token.
    pub fn link_remote(
        &mut self,
        account: Account,
        remote_url: &str,
        token: &str,
    ) -> anyhow::Result<()> {
        self.app_data
            .save_remote(&self.vault_dir, remote_url)
            .context("Could not save remote")?;
        self.token_store
            .save(token)
            .context("Could not save access token")?;
        self.remote = Some(remote_url.to_owned());

        let account_id = account.id.clone();
        let _ = self.app_data.add_account(account);
        let _ = self
            .app_data
            .save_vault_account(&self.vault_dir, &account_id);
        if let Ok(path) = self.app_data.account_token_path(&account_id)
            && let Ok(store) = (self.token_store_for)(&path)
        {
            let _ = store.save(token);
        }
        Ok(())
    }

    /// Forgets this vault's remote and token (the registered account, if
    /// any, stays available to other vaults).
    pub fn unlink_remote(&mut self) {
        let _ = self.app_data.save_remote(&self.vault_dir, "");
        let _ = self.token_store.save("");
        self.remote = None;
    }

    /// Performs search filtering and prepares directory source cache.
    pub fn update_filtered_list_data(&mut self) -> FilteredListOutcome {
        let hits = self.index.search(&self.search_query).unwrap_or_default();
        let vault_dir = self.vault_dir.clone();

        let note_indices: HashMap<&Path, usize> = self
            .notes
            .iter()
            .enumerate()
            .map(|(idx, path)| (path.as_path(), idx))
            .collect();

        let mut filtered_results = Vec::with_capacity(hits.len());
        let mut names = Vec::with_capacity(hits.len());
        let mut conflicted = Vec::with_capacity(hits.len());
        let mut snippets = Vec::with_capacity(hits.len());
        let mut directory_source = Vec::with_capacity(hits.len());

        for hit in hits {
            let abs_path = vault_dir.join(&hit.path);
            if let Some(&index) = note_indices.get(abs_path.as_path()) {
                directory_source.push((hit.title.clone(), index as i32, hit.has_conflict));
                filtered_results.push(notes::SearchResult {
                    note_index: index,
                    snippet: hit.snippet.clone(),
                });
                names.push(notes::display_name(&vault_dir, &abs_path));
                conflicted.push(hit.has_conflict);
                snippets.push(hit.snippet.unwrap_or_default());
            }
        }
        directory_source.sort_by(|a: &(String, i32, bool), b| a.0.cmp(&b.0));

        self.filtered_results = filtered_results;
        self.directory_source = directory_source;
        let ui_current = self.editor.current.and_then(|cur| {
            self.filtered_results
                .iter()
                .position(|r| r.note_index == cur)
        });

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
    if let Some(rest) = trimmed.strip_prefix("~/")
        && let Some(home) = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
    {
        return home.join(rest);
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
    fn test_create_named_note() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_dir = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        let app_data = AppData::new(tmp.path().join("data"));

        let mut session = test_session(vault_dir, app_data);
        let path = session
            .create_named_note("My Custom Note", "= My Custom Note\n\nContent\n")
            .unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "My Custom Note.tmt");
        let read = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read, "= My Custom Note\n\nContent\n");
        assert_eq!(session.notes.len(), 1);
    }

    #[test]
    fn test_vault_display_name() {
        assert_eq!(
            vault_display_name(Path::new("/home/user/my-vault")),
            "my-vault"
        );
        assert_eq!(
            vault_display_name(Path::new("relative-vault")),
            "relative-vault"
        );
    }

    /// A token store that remembers what was saved, shared by the vault and
    /// the per-account factory so a test can see both writes.
    #[derive(Default)]
    struct MemoryTokenStore(std::sync::Mutex<Option<String>>);
    impl TokenStore for MemoryTokenStore {
        fn load(&self) -> anyhow::Result<Option<String>> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn save(&self, token: &str) -> anyhow::Result<()> {
            *self.0.lock().unwrap() = Some(token.to_owned());
            Ok(())
        }
    }

    #[test]
    fn linking_a_remote_persists_remote_token_and_account_and_unlinking_clears_them() {
        let tmp = TempDir::new().unwrap();
        let vault_dir = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        let app_data = AppData::new(tmp.path().join("data"));
        let vault_store = Arc::new(MemoryTokenStore::default());
        let account_store = Arc::new(MemoryTokenStore::default());
        let mut session = Session::new(
            vault_dir.clone(),
            app_data.clone(),
            vault_store.clone(),
            {
                let account_store = account_store.clone();
                Box::new(move |_| Ok(account_store.clone() as Arc<dyn TokenStore>))
            },
            vec![vault_dir.clone()],
            None,
            NoteIndex::open_in_memory().unwrap(),
        );
        let account = Account {
            id: "github:octocat".to_owned(),
            provider: "github".to_owned(),
            instance_url: "https://github.com".to_owned(),
            username: "octocat".to_owned(),
            display_name: None,
            avatar_url: None,
        };

        session
            .link_remote(
                account.clone(),
                "https://github.com/octocat/notes.git",
                "tok",
            )
            .unwrap();

        assert_eq!(
            session.remote.as_deref(),
            Some("https://github.com/octocat/notes.git")
        );
        assert_eq!(
            app_data.load_remote(&vault_dir).as_deref(),
            Some("https://github.com/octocat/notes.git")
        );
        assert_eq!(vault_store.load().unwrap().as_deref(), Some("tok"));
        assert_eq!(account_store.load().unwrap().as_deref(), Some("tok"));
        assert_eq!(app_data.get_account("github:octocat"), Some(account));
        assert_eq!(
            app_data.load_vault_account(&vault_dir).as_deref(),
            Some("github:octocat")
        );

        session.unlink_remote();

        assert_eq!(session.remote, None);
        assert_eq!(app_data.load_remote(&vault_dir), None);
        assert_eq!(vault_store.load().unwrap().as_deref(), Some(""));
        assert!(
            app_data.get_account("github:octocat").is_some(),
            "the account itself stays registered for other vaults"
        );
    }
}
