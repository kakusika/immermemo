//! Per-vault app-private library metadata: starred note paths (`starred.json`)
//! and opened-note history (`history.json`), stored under `vault_data_dir`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{AppData, read_json_vec, write_json_atomic};

/// Maximum number of entries retained in the per-vault opened-note history.
pub const MAX_HISTORY_ENTRIES: usize = 100;

/// One entry in a vault's opened-note history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub path: String,
    pub opened_at_secs: u64,
}

impl AppData {
    fn starred_file(&self, vault_dir: &Path) -> PathBuf {
        self.vault_data_dir(vault_dir).join("starred.json")
    }

    fn history_file(&self, vault_dir: &Path) -> PathBuf {
        self.vault_data_dir(vault_dir).join("history.json")
    }

    /// Loads the ordered list of starred vault-relative note paths for `vault_dir`.
    /// Returns an empty list if missing or unreadable/corrupt.
    pub fn load_starred(&self, vault_dir: &Path) -> Vec<String> {
        read_json_vec(&self.starred_file(vault_dir))
    }

    /// Returns whether `rel_path` is currently starred in `vault_dir`.
    pub fn is_starred(&self, vault_dir: &Path, rel_path: &str) -> bool {
        self.load_starred(vault_dir).iter().any(|p| p == rel_path)
    }

    /// Toggles the starred state of `rel_path` in `vault_dir`, returning the new state.
    pub fn toggle_starred(&self, vault_dir: &Path, rel_path: &str) -> anyhow::Result<bool> {
        let mut starred = self.load_starred(vault_dir);
        let new_state = if let Some(pos) = starred.iter().position(|p| p == rel_path) {
            starred.remove(pos);
            false
        } else {
            starred.push(rel_path.to_owned());
            true
        };
        write_json_atomic(&self.starred_file(vault_dir), &starred)?;
        Ok(new_state)
    }

    /// Updates starred paths when a note is renamed from `old_rel_path` to `new_rel_path`.
    pub fn rename_starred_path(
        &self,
        vault_dir: &Path,
        old_rel_path: &str,
        new_rel_path: &str,
    ) -> anyhow::Result<()> {
        let mut starred = self.load_starred(vault_dir);
        if !starred.iter().any(|p| p == old_rel_path) {
            return Ok(());
        }
        for path in &mut starred {
            if path == old_rel_path {
                *path = new_rel_path.to_owned();
            }
        }
        dedup_preserve_order(&mut starred);
        write_json_atomic(&self.starred_file(vault_dir), &starred)
    }

    /// Removes `rel_path` from starred paths (e.g., when a note is deleted).
    pub fn remove_starred_path(&self, vault_dir: &Path, rel_path: &str) -> anyhow::Result<()> {
        let mut starred = self.load_starred(vault_dir);
        let len_before = starred.len();
        starred.retain(|p| p != rel_path);
        if starred.len() == len_before {
            return Ok(());
        }
        write_json_atomic(&self.starred_file(vault_dir), &starred)
    }

    /// Loads the opened-note history for `vault_dir` (most-recent-first).
    /// Returns an empty list if missing or unreadable/corrupt.
    pub fn load_history(&self, vault_dir: &Path) -> Vec<HistoryEntry> {
        read_json_vec(&self.history_file(vault_dir))
    }

    /// Records `rel_path` as opened now in `vault_dir`, deduplicating and capping at
    /// [`MAX_HISTORY_ENTRIES`].
    pub fn record_history(&self, vault_dir: &Path, rel_path: &str) -> anyhow::Result<()> {
        let opened_at_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.record_history_at(vault_dir, rel_path, opened_at_secs)
    }

    /// Records `rel_path` as opened at `opened_at_secs` in `vault_dir`, deduplicating by
    /// path, keeping most-recent-first, and capping at [`MAX_HISTORY_ENTRIES`].
    pub fn record_history_at(
        &self,
        vault_dir: &Path,
        rel_path: &str,
        opened_at_secs: u64,
    ) -> anyhow::Result<()> {
        let mut history = self.load_history(vault_dir);
        history.retain(|entry| entry.path != rel_path);
        history.insert(
            0,
            HistoryEntry {
                path: rel_path.to_owned(),
                opened_at_secs,
            },
        );
        history.truncate(MAX_HISTORY_ENTRIES);
        write_json_atomic(&self.history_file(vault_dir), &history)
    }

    /// Updates history entries when a note is renamed from `old_rel_path` to `new_rel_path`.
    pub fn rename_history_path(
        &self,
        vault_dir: &Path,
        old_rel_path: &str,
        new_rel_path: &str,
    ) -> anyhow::Result<()> {
        let mut history = self.load_history(vault_dir);
        if !history.iter().any(|e| e.path == old_rel_path) {
            return Ok(());
        }
        for entry in &mut history {
            if entry.path == old_rel_path {
                entry.path = new_rel_path.to_owned();
            }
        }
        let mut seen = HashSet::new();
        history.retain(|entry| seen.insert(entry.path.clone()));
        write_json_atomic(&self.history_file(vault_dir), &history)
    }

    /// Removes `rel_path` from history (e.g., when a note is deleted).
    pub fn remove_history_path(&self, vault_dir: &Path, rel_path: &str) -> anyhow::Result<()> {
        let mut history = self.load_history(vault_dir);
        let len_before = history.len();
        history.retain(|e| e.path != rel_path);
        if history.len() == len_before {
            return Ok(());
        }
        write_json_atomic(&self.history_file(vault_dir), &history)
    }

    /// Clears all opened-note history for `vault_dir`.
    pub fn clear_history(&self, vault_dir: &Path) -> anyhow::Result<()> {
        match std::fs::remove_file(self.history_file(vault_dir)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn dedup_preserve_order(items: &mut Vec<String>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starred_starts_empty_and_toggles_preserving_order() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        assert!(data.load_starred(vault.path()).is_empty());
        assert!(!data.is_starred(vault.path(), "a.tmt"));

        assert!(data.toggle_starred(vault.path(), "a.tmt").unwrap());
        assert!(data.toggle_starred(vault.path(), "dir/b.tmt").unwrap());
        assert!(data.is_starred(vault.path(), "a.tmt"));
        assert_eq!(data.load_starred(vault.path()), vec!["a.tmt", "dir/b.tmt"]);

        assert!(!data.toggle_starred(vault.path(), "a.tmt").unwrap());
        assert!(!data.is_starred(vault.path(), "a.tmt"));
        assert_eq!(data.load_starred(vault.path()), vec!["dir/b.tmt"]);
    }

    #[test]
    fn starred_rename_and_remove_update_list() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        data.toggle_starred(vault.path(), "a.tmt").unwrap();
        data.toggle_starred(vault.path(), "b.tmt").unwrap();
        data.rename_starred_path(vault.path(), "a.tmt", "renamed.tmt")
            .unwrap();
        assert_eq!(
            data.load_starred(vault.path()),
            vec!["renamed.tmt", "b.tmt"]
        );

        // Renaming to an already-starred path deduplicates rather than duplicating.
        data.rename_starred_path(vault.path(), "renamed.tmt", "b.tmt")
            .unwrap();
        assert_eq!(data.load_starred(vault.path()), vec!["b.tmt"]);

        data.remove_starred_path(vault.path(), "b.tmt").unwrap();
        assert!(data.load_starred(vault.path()).is_empty());
    }

    #[test]
    fn corrupt_json_files_degrade_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        let vault_data = data.vault_data_dir(vault.path());
        std::fs::create_dir_all(&vault_data).unwrap();
        std::fs::write(vault_data.join("starred.json"), "{not json").unwrap();
        std::fs::write(vault_data.join("history.json"), "corrupt").unwrap();

        assert!(data.load_starred(vault.path()).is_empty());
        assert!(data.load_history(vault.path()).is_empty());
    }

    #[test]
    fn history_records_most_recent_first_and_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        data.record_history_at(vault.path(), "a.tmt", 100).unwrap();
        data.record_history_at(vault.path(), "b.tmt", 200).unwrap();
        data.record_history_at(vault.path(), "a.tmt", 300).unwrap();

        assert_eq!(
            data.load_history(vault.path()),
            vec![
                HistoryEntry {
                    path: "a.tmt".to_owned(),
                    opened_at_secs: 300,
                },
                HistoryEntry {
                    path: "b.tmt".to_owned(),
                    opened_at_secs: 200,
                },
            ]
        );
    }

    #[test]
    fn history_caps_at_max_entries() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        for i in 0..(MAX_HISTORY_ENTRIES + 15) {
            data.record_history_at(vault.path(), &format!("note-{i}.tmt"), i as u64)
                .unwrap();
        }

        let history = data.load_history(vault.path());
        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(history[0].path, format!("note-{}.tmt", MAX_HISTORY_ENTRIES + 14));
        assert_eq!(history[MAX_HISTORY_ENTRIES - 1].path, "note-15.tmt");
    }

    #[test]
    fn history_rename_remove_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let data = AppData::new(dir.path().join("appdata"));
        let vault = tempfile::tempdir().unwrap();

        data.record_history_at(vault.path(), "one.tmt", 10).unwrap();
        data.record_history_at(vault.path(), "two.tmt", 20).unwrap();

        data.rename_history_path(vault.path(), "one.tmt", "one-renamed.tmt")
            .unwrap();
        assert_eq!(
            data.load_history(vault.path()),
            vec![
                HistoryEntry {
                    path: "two.tmt".to_owned(),
                    opened_at_secs: 20,
                },
                HistoryEntry {
                    path: "one-renamed.tmt".to_owned(),
                    opened_at_secs: 10,
                },
            ]
        );

        data.remove_history_path(vault.path(), "two.tmt").unwrap();
        assert_eq!(
            data.load_history(vault.path()),
            vec![HistoryEntry {
                path: "one-renamed.tmt".to_owned(),
                opened_at_secs: 10,
            }]
        );

        data.clear_history(vault.path()).unwrap();
        assert!(data.load_history(vault.path()).is_empty());
        // Clearing when already empty is a no-op.
        data.clear_history(vault.path()).unwrap();
    }
}
