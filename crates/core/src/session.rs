//! Core session state management and note operations, independent of GUI frameworks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;

use crate::editor::EditorState;
use immermemo_credentials::{TokenStore, TokenStoreFactory};
use immermemo_index::NoteIndex;
use immermemo_vault::appdata::{Account, AppData};
use immermemo_vault::notes;

use crate::navigation::BackForwardStack;

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

/// The note list's conflict flags after an edit changed the open note's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictFlags {
    /// One flag per row of the currently filtered list.
    pub filtered: Vec<bool>,
    /// How many notes in the whole vault have one.
    pub count: usize,
}

/// What an edit to the open note changed that the UI mirrors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOutcome {
    /// Whether the edited text still contains an unresolved `@conflict`.
    pub has_conflict: bool,
    /// Set only when this edit flipped the note's stored conflict flag.
    pub conflict_flags: Option<ConflictFlags>,
    /// The note's vault-relative path, once its new text was written.
    pub saved: Option<PathBuf>,
    /// Why the new text could not be written to disk, if it could not.
    pub write_error: Option<String>,
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

    /// Opens the session the app starts with: the vault that was current last
    /// time (or the first known one), its token store, remote and note index.
    ///
    /// `default_vault_dir` only matters the very first time the app runs
    /// (no vault known yet): it becomes the first known vault, so a fresh
    /// install still opens straight onto a vault.
    pub fn start(
        app_data: AppData,
        token_store_for: TokenStoreFactory,
        default_vault_dir: PathBuf,
    ) -> anyhow::Result<Self> {
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
        let index = Self::open_index(&app_data, &vault_dir);
        Ok(Self::new(
            vault_dir,
            app_data,
            token_store,
            token_store_for,
            known_vaults,
            remote,
            index,
        ))
    }

    /// The note index for `vault_dir`, kept on disk -- or, if that database
    /// cannot be opened, an in-memory one (the notes stay usable; the index
    /// is rebuilt from the files on the next launch).
    pub fn open_index(app_data: &AppData, vault_dir: &Path) -> NoteIndex {
        app_data
            .index_path(vault_dir)
            .ok()
            .and_then(|path| NoteIndex::open(&path).ok())
            .unwrap_or_else(|| NoteIndex::open_in_memory().expect("an in-memory database opens"))
    }

    /// Makes `dir` a known vault (creating the folder), without switching to it.
    pub fn register_vault(&mut self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        self.app_data.add_vault(dir)?;
        self.known_vaults = self.app_data.known_vaults();
        Ok(())
    }

    /// Forgets the known vault `dir` and wipes its private app data (never
    /// the notes folder). If it was the current vault, returns the one to
    /// switch to instead. The last remaining vault cannot be forgotten.
    pub fn forget_vault(&mut self, dir: &Path) -> anyhow::Result<Option<PathBuf>> {
        anyhow::ensure!(
            self.known_vaults.len() > 1,
            "the last vault cannot be removed"
        );
        let was_current = self.vault_dir == dir;
        self.app_data.remove_vault(dir)?;
        self.known_vaults = self.app_data.known_vaults();
        Ok(if was_current {
            self.known_vaults.first().cloned()
        } else {
            None
        })
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
        let vault_dir = self.vault_dir.clone();
        let _ = self.index.reconcile_filesystem(&vault_dir);
        self.load_cached_list_data()
    }

    /// Loads cached note lists from SQLite index without walking the filesystem.
    pub fn load_cached_list_data(&mut self) -> RefreshListOutcome {
        let previous = self.current_path();
        let all = self.index.list_all().unwrap_or_default();
        self.notes = all.iter().map(|n| self.vault_dir.join(&n.path)).collect();
        self.conflicted = all.iter().map(|n| n.has_conflict).collect();
        let conflict_count = self.conflicted.iter().filter(|&&c| c).count();
        self.editor.current = previous.and_then(|p| self.notes.iter().position(|n| *n == p));
        RefreshListOutcome { conflict_count }
    }

    /// Applies the editor's new text for the open note: records it in the
    /// undo history, keeps the note's conflict flag in step with whether the
    /// text still holds an `@conflict`, and writes it to disk and the index.
    pub fn apply_edit(&mut self, text: &str) -> EditOutcome {
        if let Some(history) = self.editor.history.as_mut() {
            history.edit(text);
        }

        let has_conflict = text.contains(crate::CONFLICT_MARKER);
        let conflict_flags = self
            .editor
            .current
            .filter(|&i| self.conflicted.get(i).is_some_and(|&c| c != has_conflict))
            .map(|i| {
                self.conflicted[i] = has_conflict;
                ConflictFlags {
                    filtered: self
                        .filtered_results
                        .iter()
                        .map(|r| self.conflicted[r.note_index])
                        .collect(),
                    count: self.conflicted.iter().filter(|&&c| c).count(),
                }
            });

        let mut saved = None;
        let mut write_error = None;
        if let Some(path) = self.current_path() {
            match std::fs::write(&path, text) {
                Ok(()) => {
                    if let Ok(rel) = path.strip_prefix(&self.vault_dir) {
                        let _ = self.index.record_write(&self.vault_dir, rel, text);
                        saved = Some(rel.to_owned());
                    }
                }
                Err(e) => write_error = Some(e.to_string()),
            }
        }
        EditOutcome {
            has_conflict,
            conflict_flags,
            saved,
            write_error,
        }
    }

    /// The index in `notes` of the note a link's target names: matched
    /// first as a vault-relative path (with or without the `.tmt`
    /// extension, `@` prefix optional), then by file name alone, ignoring
    /// case.
    pub fn find_note_for_link(&self, query: &str) -> Option<usize> {
        let trimmed = query.trim_start_matches('@');
        let by_path = |rel: &Path| {
            let without_extension = rel.with_extension("");
            [rel, without_extension.as_path()].iter().any(|candidate| {
                *candidate == Path::new(trimmed)
                    || candidate.to_string_lossy() == query
                    || candidate.to_string_lossy() == trimmed
            })
        };
        let by_stem = |note: &Path| {
            note.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| {
                    stem.eq_ignore_ascii_case(query)
                        || stem.eq_ignore_ascii_case(trimmed)
                        || stem.trim_start_matches('@').eq_ignore_ascii_case(trimmed)
                })
        };
        self.notes
            .iter()
            .position(|note| note.strip_prefix(&self.vault_dir).is_ok_and(by_path))
            .or_else(|| self.notes.iter().position(|note| by_stem(note)))
    }

    /// The vault's own token store -- never the shared account's.
    fn own_token_store(&self) -> anyhow::Result<Arc<dyn TokenStore>> {
        (self.token_store_for)(&self.vault_dir)
    }

    /// Links this vault to `remote_url`, authenticated by `token` and
    /// registered under `account`. The remote and the vault's own copy of
    /// the token must be saved; the account bookkeeping is best-effort,
    /// since sync can fall back on the vault's own token.
    ///
    /// Everything a provider's connect flow does after it has a repository,
    /// whichever way it authenticated -- OAuth device flow or a pasted token.
    /// Linking a different account than before only re-points this vault:
    /// the previous account's token is left alone, other vaults may use it.
    pub fn link_remote(
        &mut self,
        account: Account,
        remote_url: &str,
        token: &str,
    ) -> anyhow::Result<()> {
        self.app_data
            .save_remote(&self.vault_dir, remote_url)
            .context("Could not save remote")?;
        self.own_token_store()
            .and_then(|store| store.save(token))
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
        // From here on the vault authenticates through the account.
        if let Ok(store) = self.open_token_store(&self.vault_dir) {
            self.token_store = store;
        }
        Ok(())
    }

    /// Points this vault at a remote the user typed in, with `token` (kept
    /// if empty). Such a remote is not tied to a hosting account, so any
    /// account link is dropped -- otherwise sync would keep using the
    /// account's token and ignore the one typed here.
    pub fn set_manual_remote(&mut self, remote_url: &str, token: &str) -> anyhow::Result<()> {
        self.app_data
            .save_remote(&self.vault_dir, remote_url)
            .context("Could not save the remote")?;
        self.app_data.clear_vault_account(&self.vault_dir);
        self.token_store = self.own_token_store()?;
        if !token.is_empty() {
            self.token_store
                .save(token)
                .context("Could not save the access token")?;
        }
        self.remote = Some(remote_url.to_owned());
        Ok(())
    }

    /// Gathers what a sync of this vault needs into a job that can run on
    /// another thread. `None` when no remote is linked yet.
    pub fn prepare_sync(&self) -> anyhow::Result<Option<crate::sync::SyncJob>> {
        let Some(remote) = self.remote.clone() else {
            return Ok(None);
        };
        Ok(Some(crate::sync::SyncJob {
            gitdir: self.app_data.gitdir(&self.vault_dir)?,
            vault_dir: self.vault_dir.clone(),
            remote,
            token_store: self.sync_token_store(),
            app_data: self.app_data.clone(),
            identity: immermemo_identity::DeviceIdentity::load_or_init(
                &self.app_data.identity_path(),
            )
            .ok(),
        }))
    }

    /// The token file of the account `vault_dir` is linked to, if any.
    fn account_token_path(&self, vault_dir: &Path) -> Option<PathBuf> {
        let account_id = self.app_data.load_vault_account(vault_dir)?;
        self.app_data.account_token_path(&account_id).ok()
    }

    /// Opens the token store `vault_dir` authenticates through: its linked
    /// account's, or else one of its own.
    pub fn open_token_store(&self, vault_dir: &Path) -> anyhow::Result<Arc<dyn TokenStore>> {
        let path = self
            .account_token_path(vault_dir)
            .unwrap_or_else(|| vault_dir.to_owned());
        (self.token_store_for)(&path)
    }

    /// The token store a sync of the current vault uses: the linked
    /// account's if it can be opened, otherwise the vault's own.
    pub fn sync_token_store(&self) -> Arc<dyn TokenStore> {
        self.account_token_path(&self.vault_dir)
            .and_then(|path| (self.token_store_for)(&path).ok())
            .unwrap_or_else(|| self.token_store.clone())
    }

    /// Disconnects this vault: forgets its remote, its own token and its
    /// account link. The registered account and its token stay, other vaults
    /// may use them.
    pub fn unlink_remote(&mut self) {
        let _ = self.app_data.save_remote(&self.vault_dir, "");
        self.app_data.clear_vault_account(&self.vault_dir);
        if let Ok(own) = self.own_token_store() {
            let _ = own.save("");
            self.token_store = own;
        }
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

/// Resolves a user-typed vault path (trimming it and expanding a leading `~/`).
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

    /// A token store that remembers what was saved.
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

    /// One store per path, like the real factory: the vault's own and each
    /// account's can be told apart.
    #[derive(Clone, Default)]
    struct Stores(Arc<std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<MemoryTokenStore>>>>);
    impl Stores {
        fn at(&self, path: &Path) -> Arc<MemoryTokenStore> {
            self.0
                .lock()
                .unwrap()
                .entry(path.to_owned())
                .or_default()
                .clone()
        }
        fn token(&self, path: &Path) -> Option<String> {
            self.at(path).load().unwrap()
        }
    }

    fn account(user: &str) -> Account {
        Account {
            id: format!("github:{user}"),
            provider: "github".to_owned(),
            instance_url: "https://github.com".to_owned(),
            username: user.to_owned(),
            display_name: None,
            avatar_url: None,
        }
    }

    #[test]
    fn linking_keeps_each_accounts_token_apart_and_disconnecting_only_clears_this_vault() {
        let tmp = TempDir::new().unwrap();
        let vault_dir = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        let app_data = AppData::new(tmp.path().join("data"));
        let stores = Stores::default();
        let mut session = Session::new(
            vault_dir.clone(),
            app_data.clone(),
            stores.at(&vault_dir),
            {
                let stores = stores.clone();
                Box::new(move |path| Ok(stores.at(path) as Arc<dyn TokenStore>))
            },
            vec![vault_dir.clone()],
            None,
            NoteIndex::open_in_memory().unwrap(),
        );
        let token_of = |id: &str| stores.token(&app_data.account_token_path(id).unwrap());

        // Before linking, a sync authenticates through the vault's own store.
        session.sync_token_store().save("vault-only").unwrap();
        assert_eq!(stores.token(&vault_dir).as_deref(), Some("vault-only"));

        // Linking account A.
        session
            .link_remote(account("a"), "https://github.com/a/notes.git", "tok-a")
            .unwrap();
        assert_eq!(
            session.remote.as_deref(),
            Some("https://github.com/a/notes.git")
        );
        assert_eq!(
            app_data.load_remote(&vault_dir).as_deref(),
            Some("https://github.com/a/notes.git")
        );
        assert_eq!(stores.token(&vault_dir).as_deref(), Some("tok-a"));
        assert_eq!(token_of("github:a").as_deref(), Some("tok-a"));
        assert_eq!(app_data.get_account("github:a"), Some(account("a")));
        assert_eq!(
            app_data.load_vault_account(&vault_dir).as_deref(),
            Some("github:a")
        );
        // ...and from then on the vault authenticates through that account.
        session.token_store.save("refreshed-a").unwrap();
        assert_eq!(token_of("github:a").as_deref(), Some("refreshed-a"));

        // Linking account B must not clobber A, which other vaults may use.
        session
            .link_remote(account("b"), "https://github.com/b/notes.git", "tok-b")
            .unwrap();
        assert_eq!(token_of("github:a").as_deref(), Some("refreshed-a"));
        assert_eq!(token_of("github:b").as_deref(), Some("tok-b"));
        assert_eq!(
            app_data.load_vault_account(&vault_dir).as_deref(),
            Some("github:b")
        );

        // A typed-in remote drops the account link and uses its own token.
        session
            .set_manual_remote("https://git.example.com/me/notes.git", "typed")
            .unwrap();
        assert_eq!(app_data.load_vault_account(&vault_dir), None);
        assert_eq!(stores.token(&vault_dir).as_deref(), Some("typed"));
        assert_eq!(
            token_of("github:b").as_deref(),
            Some("tok-b"),
            "B untouched"
        );
        session.sync_token_store().save("typed-again").unwrap();
        assert_eq!(stores.token(&vault_dir).as_deref(), Some("typed-again"));
        session
            .set_manual_remote("https://git.example.com/me/other.git", "")
            .unwrap();
        assert_eq!(
            stores.token(&vault_dir).as_deref(),
            Some("typed-again"),
            "an empty token keeps the saved one"
        );

        // Disconnecting clears this vault's link, not the account.
        session
            .link_remote(account("a"), "https://github.com/a/notes.git", "tok-a2")
            .unwrap();
        session.unlink_remote();
        assert_eq!(session.remote, None);
        assert_eq!(app_data.load_remote(&vault_dir), None);
        assert_eq!(app_data.load_vault_account(&vault_dir), None);
        assert_eq!(stores.token(&vault_dir).as_deref(), Some(""));
        assert_eq!(
            token_of("github:a").as_deref(),
            Some("tok-a2"),
            "the account stays usable"
        );
        assert!(app_data.get_account("github:a").is_some());
    }

    #[test]
    fn a_typed_vault_path_is_trimmed_and_an_absolute_one_is_kept() {
        assert_eq!(
            vault_path_from_input("  /srv/notes \n"),
            PathBuf::from("/srv/notes")
        );
        assert_eq!(
            vault_path_from_input("rel/notes"),
            PathBuf::from("rel/notes")
        );
    }

    #[test]
    fn editing_writes_the_note_and_tracks_its_conflict_flag() {
        let tmp = TempDir::new().unwrap();
        let vault_dir = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        let mut session = test_session(vault_dir.clone(), AppData::new(tmp.path().join("data")));
        let note = session.create_note().unwrap();
        session.refresh_list_data();
        session.update_filtered_list_data();
        session.open_note(0).unwrap().unwrap();

        let plain = session.apply_edit("hello\n");
        assert!(!plain.has_conflict);
        assert_eq!(plain.conflict_flags, None, "nothing flipped");
        assert_eq!(plain.write_error, None);
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "hello\n");
        assert_eq!(plain.saved.as_deref(), note.strip_prefix(&vault_dir).ok());

        let conflicted = session.apply_edit(&format!("{}\n", crate::CONFLICT_MARKER));
        assert!(conflicted.has_conflict);
        assert_eq!(
            conflicted.conflict_flags,
            Some(ConflictFlags {
                filtered: vec![true],
                count: 1
            })
        );
        assert_eq!(
            session
                .apply_edit(&format!("{} still\n", crate::CONFLICT_MARKER))
                .conflict_flags,
            None,
            "still conflicted, nothing flipped"
        );
        assert_eq!(
            session
                .apply_edit("fixed\n")
                .conflict_flags
                .map(|f| f.count),
            Some(0)
        );
    }

    #[test]
    fn a_link_target_finds_the_note_by_path_then_by_file_name() {
        let tmp = TempDir::new().unwrap();
        let vault_dir = tmp.path().join("vault");
        std::fs::create_dir_all(vault_dir.join("work")).unwrap();
        for rel in ["inbox.tmt", "work/Todo.tmt", "Todo.tmt"] {
            std::fs::write(vault_dir.join(rel), "x\n").unwrap();
        }
        let mut session = test_session(vault_dir.clone(), AppData::new(tmp.path().join("data")));
        session.refresh_list_data();
        let name_at = |i: Option<usize>| {
            i.map(|i| {
                session.notes[i]
                    .strip_prefix(&vault_dir)
                    .unwrap()
                    .to_owned()
            })
        };

        let find = |q| name_at(session.find_note_for_link(q));
        assert_eq!(find("inbox"), Some("inbox.tmt".into()));
        assert_eq!(find("inbox.tmt"), Some("inbox.tmt".into()));
        assert_eq!(find("@inbox"), Some("inbox.tmt".into()));
        assert_eq!(find("work/Todo"), Some("work/Todo.tmt".into()));
        assert_eq!(
            find("TODO"),
            Some("Todo.tmt".into()),
            "file name, ignoring case"
        );
        assert_eq!(find("missing"), None);
    }

    #[test]
    fn starting_opens_the_default_vault_first_and_the_saved_current_one_after() {
        let tmp = TempDir::new().unwrap();
        let data = || AppData::new(tmp.path().join("data"));
        let factory = || -> TokenStoreFactory {
            Box::new(|_| Ok(Arc::new(DummyTokenStore) as Arc<dyn TokenStore>))
        };
        let (first, second) = (tmp.path().join("first"), tmp.path().join("second"));

        let session = Session::start(data(), factory(), first.clone()).unwrap();
        assert_eq!(session.vault_dir, first);
        assert_eq!(session.known_vaults, vec![first.clone()]);
        assert!(first.is_dir(), "the vault folder is created");

        // A later launch ignores the default and honors the saved choice.
        let app_data = data();
        std::fs::create_dir_all(&second).unwrap();
        app_data.add_vault(&second).unwrap();
        app_data.save_current_vault(&second).unwrap();
        let session = Session::start(data(), factory(), first.clone()).unwrap();
        assert_eq!(session.vault_dir, second);
        assert_eq!(session.known_vaults.len(), 2);

        // A saved vault that is no longer known falls back to the first one.
        app_data
            .save_current_vault(&tmp.path().join("gone"))
            .unwrap();
        let session = Session::start(data(), factory(), first.clone()).unwrap();
        assert_eq!(session.vault_dir, first);
    }

    #[test]
    fn vaults_are_registered_and_forgotten_but_never_the_last_one() {
        let tmp = TempDir::new().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        let mut session = test_session(a.clone(), AppData::new(tmp.path().join("data")));
        session.register_vault(&a).unwrap();

        assert!(session.forget_vault(&a).is_err(), "the only vault stays");

        session.register_vault(&b).unwrap();
        assert!(b.is_dir(), "registering creates the folder");
        assert_eq!(session.known_vaults.len(), 2);

        assert_eq!(session.forget_vault(&b).unwrap(), None, "b was not current");
        assert_eq!(session.known_vaults, vec![a.clone()]);

        session.register_vault(&b).unwrap();
        assert_eq!(
            session.forget_vault(&a).unwrap(),
            Some(b.clone()),
            "removing the current vault names its replacement"
        );
    }

    #[test]
    fn a_prepared_sync_job_pushes_the_vault_to_its_remote() {
        let tmp = TempDir::new().unwrap();
        let vault_dir = tmp.path().join("vault");
        std::fs::create_dir_all(&vault_dir).unwrap();
        std::fs::write(vault_dir.join("note.tmt"), "Hello.\n").unwrap();
        let remote = immermemo_sync::testing::shared_remote();
        let mut session = test_session(vault_dir, AppData::new(tmp.path().join("data")));

        assert!(
            session.prepare_sync().unwrap().is_none(),
            "nothing to sync before a remote is linked"
        );

        session.remote = Some(remote.path().to_str().unwrap().to_owned());
        let job = session.prepare_sync().unwrap().expect("a remote is linked");
        // The job owns its data: it runs without the session.
        drop(session);
        job.run(None).unwrap();

        let pushed = git2::Repository::open_bare(remote.path()).unwrap();
        assert!(pushed.find_reference("refs/heads/main").is_ok());
    }
}
