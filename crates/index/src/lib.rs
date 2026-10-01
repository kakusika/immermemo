//! Note index database for fast listing, metadata caching, and full-text search.
//!
//! Stores note metadata (`path`, `title`, `mtime`, `size`, `has_conflict`) and provides
//! fast full-text search powered by SQLite and FTS5 trigram indexing.
//!
//! Designed to scale to tens of thousands of notes without performing full filesystem
//! walks or per-file disk reads on every UI refresh or keystroke search.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use immermemo_merge::CONFLICT_MARKER;
use rusqlite::{Connection, Row, params};

/// Shared by every place that writes a note row: insert it fresh, or
/// overwrite every column if the path already exists.
const UPSERT_NOTE_SQL: &str =
    "INSERT INTO notes (path, title, mtime_secs, mtime_nanos, size, has_conflict, body)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
     ON CONFLICT(path) DO UPDATE SET
        title = excluded.title,
        mtime_secs = excluded.mtime_secs,
        mtime_nanos = excluded.mtime_nanos,
        size = excluded.size,
        has_conflict = excluded.has_conflict,
        body = excluded.body";

/// An indexed note in the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedNote {
    /// Relative path from the vault root (e.g., `grocery.tmt` or `work/todo.tmt`).
    pub path: PathBuf,
    /// Display title (e.g., `grocery` or `work/todo`).
    pub title: String,
    /// Whether this note currently contains `@mobile.conflict` markers.
    pub has_conflict: bool,
}

/// A search result hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Relative path from the vault root.
    pub path: PathBuf,
    /// Display title.
    pub title: String,
    /// Whether this note currently contains `@mobile.conflict` markers.
    pub has_conflict: bool,
    /// Optional contextual snippet from the note's content.
    pub snippet: Option<String>,
}

/// Summary of changes detected during a filesystem reconciliation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
}

/// SQLite-backed index for a vault.
pub struct NoteIndex {
    conn: Connection,
}

impl NoteIndex {
    /// Opens or creates an index database at `db_path`.
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        Self::init(conn)
    }

    /// Opens an in-memory index database (primarily for testing).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS notes (
                path TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL,
                size INTEGER NOT NULL,
                has_conflict INTEGER NOT NULL DEFAULT 0,
                body TEXT NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                path UNINDEXED,
                title,
                body,
                tokenize = 'trigram'
            );

            CREATE TRIGGER IF NOT EXISTS notes_ai AFTER INSERT ON notes BEGIN
                INSERT INTO notes_fts (path, title, body) VALUES (new.path, new.title, new.body);
            END;

            CREATE TRIGGER IF NOT EXISTS notes_ad AFTER DELETE ON notes BEGIN
                DELETE FROM notes_fts WHERE path = old.path;
            END;

            CREATE TRIGGER IF NOT EXISTS notes_au AFTER UPDATE ON notes BEGIN
                DELETE FROM notes_fts WHERE path = old.path;
                INSERT INTO notes_fts (path, title, body) VALUES (new.path, new.title, new.body);
            END;
            ",
        )?;
        Ok(Self { conn })
    }

    /// Reconciles the database with the filesystem under `vault_dir`.
    ///
    /// Walks the directory tree for `.tmt` files. Compares file size and modification time
    /// against cached values in the database. Unchanged files skip disk content reads entirely.
    /// Notes that no longer exist on disk are removed from the index.
    pub fn reconcile_filesystem(&mut self, vault_dir: &Path) -> anyhow::Result<ReconcileReport> {
        let mut report = ReconcileReport::default();
        let mut disk_files: Vec<(PathBuf, u64, u64, u32)> = Vec::new(); // (rel_path, size, mtime_secs, mtime_nanos)

        collect_tmt_files(vault_dir, &mut disk_files);

        // Looked up once per cached row below rather than linearly
        // scanning `disk_files` each time -- this is the only place a
        // vault with many thousands of notes would otherwise pay O(n*m).
        let disk_meta: HashMap<PathBuf, (u64, u64, u32)> = disk_files
            .iter()
            .map(|(path, size, secs, nanos)| (path.clone(), (*size, *secs, *nanos)))
            .collect();

        let tx = self.conn.transaction()?;

        // Retrieve existing paths and their cached metadata.
        let mut cached_notes: HashSet<PathBuf> = HashSet::new();
        {
            let mut stmt = tx.prepare("SELECT path, size, mtime_secs, mtime_nanos FROM notes")?;
            let mut rows = stmt.query([])?;
            let mut to_remove: Vec<PathBuf> = Vec::new();

            while let Some(row) = rows.next()? {
                let path_str: String = row.get(0)?;
                let rel_path = PathBuf::from(&path_str);
                match disk_meta.get(&rel_path) {
                    None => to_remove.push(rel_path),
                    Some((size, secs, nanos)) => {
                        let cached_size: u64 = row.get(1)?;
                        let cached_secs: u64 = row.get(2)?;
                        let cached_nanos: u32 = row.get(3)?;
                        if *size == cached_size && *secs == cached_secs && *nanos == cached_nanos {
                            report.unchanged += 1;
                        }
                        cached_notes.insert(rel_path);
                    }
                }
            }

            // Remove files from database that are no longer on disk
            let mut del_stmt = tx.prepare_cached("DELETE FROM notes WHERE path = ?1")?;
            for path in to_remove {
                del_stmt.execute(params![path.to_string_lossy()])?;
                report.removed += 1;
            }
        }

        // Process disk files: insert new, update modified
        {
            let mut check_stmt = tx.prepare_cached(
                "SELECT size, mtime_secs, mtime_nanos FROM notes WHERE path = ?1",
            )?;
            let mut upsert_stmt = tx.prepare_cached(UPSERT_NOTE_SQL)?;

            for (rel_path, size, mtime_secs, mtime_nanos) in disk_files {
                let mut needs_read = true;
                let path_str = rel_path.to_string_lossy().to_string();

                let existing = check_stmt.query_row(params![&path_str], |row| {
                    let s: u64 = row.get(0)?;
                    let sec: u64 = row.get(1)?;
                    let nano: u32 = row.get(2)?;
                    Ok((s, sec, nano))
                });

                if let Ok((cached_size, cached_sec, cached_nano)) = existing {
                    if cached_size == size && cached_sec == mtime_secs && cached_nano == mtime_nanos
                    {
                        needs_read = false;
                    }
                }

                if needs_read {
                    let abs_path = vault_dir.join(&rel_path);
                    if let Ok(body) = std::fs::read_to_string(&abs_path) {
                        let has_conflict = body.contains(CONFLICT_MARKER);
                        let title = display_name_from_rel(&rel_path);
                        let is_new = !cached_notes.contains(&rel_path);

                        upsert_stmt.execute(params![
                            &path_str,
                            &title,
                            mtime_secs,
                            mtime_nanos,
                            size,
                            if has_conflict { 1 } else { 0 },
                            &body
                        ])?;

                        if is_new {
                            report.added += 1;
                        } else {
                            report.updated += 1;
                        }
                    }
                }
            }
        }

        tx.commit()?;
        Ok(report)
    }

    /// Incrementally updates the database based on paths from a sync operation.
    ///
    /// Updates only `updated_notes` and deletes `deleted_notes` without scanning the filesystem.
    pub fn apply_sync_report(
        &mut self,
        vault_dir: &Path,
        updated_notes: &[PathBuf],
        deleted_notes: &[PathBuf],
        notes_needing_resolution: &[PathBuf],
    ) -> anyhow::Result<()> {
        let conflict_set: HashSet<&Path> = notes_needing_resolution
            .iter()
            .map(|p| p.as_path())
            .collect();
        let tx = self.conn.transaction()?;

        {
            let mut del_stmt = tx.prepare_cached("DELETE FROM notes WHERE path = ?1")?;
            for rel_path in deleted_notes {
                del_stmt.execute(params![rel_path.to_string_lossy()])?;
            }
        }

        {
            let mut upsert_stmt = tx.prepare_cached(UPSERT_NOTE_SQL)?;

            for rel_path in updated_notes {
                let abs_path = vault_dir.join(rel_path);
                if let Ok(meta) = abs_path.metadata() {
                    let size = meta.len();
                    let (mtime_secs, mtime_nanos) =
                        mtime_parts(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
                    let body = std::fs::read_to_string(&abs_path).unwrap_or_default();
                    let has_conflict =
                        conflict_set.contains(rel_path.as_path()) || body.contains(CONFLICT_MARKER);
                    let title = display_name_from_rel(rel_path);
                    let path_str = rel_path.to_string_lossy().to_string();

                    upsert_stmt.execute(params![
                        &path_str,
                        &title,
                        mtime_secs,
                        mtime_nanos,
                        size,
                        if has_conflict { 1 } else { 0 },
                        &body
                    ])?;
                }
            }
        }

        // Ensure conflict markers for any other notes needing resolution are recorded
        {
            let mut conflict_stmt =
                tx.prepare_cached("UPDATE notes SET has_conflict = 1 WHERE path = ?1")?;
            for rel_path in notes_needing_resolution {
                conflict_stmt.execute(params![rel_path.to_string_lossy()])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Records or updates a note directly in the index (e.g. after a local save).
    pub fn record_write(
        &mut self,
        vault_dir: &Path,
        rel_path: &Path,
        content: &str,
    ) -> anyhow::Result<()> {
        let abs_path = vault_dir.join(rel_path);
        let meta = abs_path.metadata()?;
        let (mtime_secs, mtime_nanos) =
            mtime_parts(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        let title = display_name_from_rel(rel_path);
        let has_conflict = content.contains(CONFLICT_MARKER);
        let path_str = rel_path.to_string_lossy().to_string();

        self.conn.execute(
            UPSERT_NOTE_SQL,
            params![
                &path_str,
                &title,
                mtime_secs,
                mtime_nanos,
                meta.len(),
                if has_conflict { 1 } else { 0 },
                content
            ],
        )?;
        Ok(())
    }

    /// Renames a note in the index.
    pub fn record_rename(
        &mut self,
        old_rel_path: &Path,
        new_rel_path: &Path,
    ) -> anyhow::Result<()> {
        let old_str = old_rel_path.to_string_lossy().to_string();
        let new_str = new_rel_path.to_string_lossy().to_string();
        let new_title = display_name_from_rel(new_rel_path);

        self.conn.execute(
            "UPDATE notes SET path = ?1, title = ?2 WHERE path = ?3",
            params![&new_str, &new_title, &old_str],
        )?;
        Ok(())
    }

    /// Deletes a note from the index.
    pub fn record_delete(&mut self, rel_path: &Path) -> anyhow::Result<()> {
        let path_str = rel_path.to_string_lossy().to_string();
        self.conn
            .execute("DELETE FROM notes WHERE path = ?1", params![&path_str])?;
        Ok(())
    }

    /// Sets or clears the conflict status of a note.
    pub fn set_conflict(&mut self, rel_path: &Path, has_conflict: bool) -> anyhow::Result<()> {
        let path_str = rel_path.to_string_lossy().to_string();
        self.conn.execute(
            "UPDATE notes SET has_conflict = ?1 WHERE path = ?2",
            params![if has_conflict { 1 } else { 0 }, &path_str],
        )?;
        Ok(())
    }

    /// Returns all indexed notes sorted by relative path.
    pub fn list_all(&self) -> anyhow::Result<Vec<IndexedNote>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, title, has_conflict FROM notes ORDER BY path ASC")?;
        let rows = stmt.query_map([], |row| {
            let path_str: String = row.get(0)?;
            let title: String = row.get(1)?;
            let conflict_int: i32 = row.get(2)?;
            Ok(IndexedNote {
                path: PathBuf::from(path_str),
                title,
                has_conflict: conflict_int != 0,
            })
        })?;

        let mut notes = Vec::new();
        for r in rows {
            notes.push(r?);
        }
        Ok(notes)
    }

    /// Returns the number of notes currently having unresolved conflicts.
    pub fn conflict_count(&self) -> anyhow::Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM notes WHERE has_conflict != 0",
            [],
            |r| r.get(0),
        )?;
        Ok(count as usize)
    }

    /// Searches notes by query case-insensitively across title and body content.
    ///
    /// Notes matching in the title are ranked before body-only matches.
    /// Body matches utilize SQLite FTS5 trigram full-text indexing with BM25 ranking,
    /// falling back cleanly to substring matching for short queries (< 3 chars).
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>> {
        let query = query.trim();
        if query.is_empty() {
            let all = self.list_all()?;
            return Ok(all
                .into_iter()
                .map(|n| SearchHit {
                    path: n.path,
                    title: n.title,
                    has_conflict: n.has_conflict,
                    snippet: None,
                })
                .collect());
        }

        let mut hits = Vec::new();
        let mut seen_paths = HashSet::new();

        // 1. Title matches (ranked first)
        {
            let escaped = escape_like(query);
            let pattern = format!("%{escaped}%");
            let mut stmt = self.conn.prepare(
                "SELECT path, title, has_conflict, body FROM notes
                 WHERE title LIKE ?1 ESCAPE '\\'
                 ORDER BY path ASC",
            )?;

            let rows = stmt.query_map(params![&pattern], row_to_note_fields)?;

            for r in rows {
                let (path, title, has_conflict, body) = r?;
                let snippet = extract_snippet(&body, query);
                seen_paths.insert(path.clone());
                hits.push(SearchHit {
                    path,
                    title,
                    has_conflict,
                    snippet,
                });
            }
        }

        // 2. Body-only matches
        let query_char_count = query.chars().count();
        let mut body_hits = Vec::new();

        if query_char_count >= 3 {
            // Attempt FTS5 search with BM25 ranking
            let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
            let fts_result = self.conn.prepare(
                "SELECT notes.path, notes.title, notes.has_conflict, notes.body
                 FROM notes_fts
                 JOIN notes ON notes.path = notes_fts.path
                 WHERE notes_fts MATCH ?1
                 ORDER BY notes_fts.rank",
            );

            if let Ok(mut stmt) = fts_result {
                if let Ok(rows) = stmt.query_map(params![&fts_query], row_to_note_fields) {
                    for r in rows.flatten() {
                        let (path, title, has_conflict, body) = r;
                        if !seen_paths.contains(&path) {
                            if let Some(snippet) = extract_snippet(&body, query) {
                                seen_paths.insert(path.clone());
                                body_hits.push(SearchHit {
                                    path,
                                    title,
                                    has_conflict,
                                    snippet: Some(snippet),
                                });
                            }
                        }
                    }
                }
            }
        }

        // If query < 3 characters or FTS didn't catch everything, fall back to LIKE on body
        if body_hits.is_empty() || query_char_count < 3 {
            let escaped = escape_like(query);
            let pattern = format!("%{escaped}%");
            let mut stmt = self.conn.prepare(
                "SELECT path, title, has_conflict, body FROM notes
                 WHERE body LIKE ?1 ESCAPE '\\'
                 ORDER BY path ASC",
            )?;

            let rows = stmt.query_map(params![&pattern], row_to_note_fields)?;

            for r in rows.flatten() {
                let (path, title, has_conflict, body) = r;
                if !seen_paths.contains(&path) {
                    if let Some(snippet) = extract_snippet(&body, query) {
                        seen_paths.insert(path.clone());
                        body_hits.push(SearchHit {
                            path,
                            title,
                            has_conflict,
                            snippet: Some(snippet),
                        });
                    }
                }
            }
        }

        hits.extend(body_hits);
        Ok(hits)
    }
}

/// The `(path, title, has_conflict, body)` shape every `search()` query
/// reads a row into, shared by its title-match, FTS and LIKE-fallback
/// passes -- they differ only in which `WHERE` clause found the row.
fn row_to_note_fields(row: &Row) -> rusqlite::Result<(PathBuf, String, bool, String)> {
    let path_str: String = row.get(0)?;
    let title: String = row.get(1)?;
    let conflict_int: i32 = row.get(2)?;
    let body: String = row.get(3)?;
    Ok((PathBuf::from(path_str), title, conflict_int != 0, body))
}

fn display_name_from_rel(rel_path: &Path) -> String {
    rel_path.with_extension("").to_string_lossy().to_string()
}

fn mtime_parts(time: SystemTime) -> (u64, u32) {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => (d.as_secs(), d.subsec_nanos()),
        Err(_) => (0, 0),
    }
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn collect_tmt_files(root: &Path, found: &mut Vec<(PathBuf, u64, u64, u32)>) {
    immermemo_vault::notes::walk_tmt_files(root, &mut |path, meta| {
        if let Ok(rel) = path.strip_prefix(root) {
            let (secs, nanos) = mtime_parts(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
            found.push((rel.to_path_buf(), meta.len(), secs, nanos));
        }
    });
}

/// Contextual snippet extraction matching Immermemo's display formatting.
pub fn extract_snippet(content: &str, query: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(char_idx) = find_case_insensitive_char_idx(trimmed, query) {
            let chars: Vec<char> = trimmed.chars().collect();
            let total_chars = chars.len();
            const MAX_CHARS: usize = 60;
            const CONTEXT_BEFORE: usize = 20;

            if total_chars <= MAX_CHARS {
                return Some(trimmed.to_string());
            }

            let start_char = char_idx.saturating_sub(CONTEXT_BEFORE);
            let end_char = (start_char + MAX_CHARS).min(total_chars);
            let start_char = if end_char == total_chars {
                total_chars.saturating_sub(MAX_CHARS)
            } else {
                start_char
            };

            let mut snippet = String::new();
            if start_char > 0 {
                snippet.push_str("...");
            }
            snippet.extend(&chars[start_char..end_char]);
            if end_char < total_chars {
                snippet.push_str("...");
            }
            return Some(snippet);
        }
    }
    None
}

fn find_case_insensitive_char_idx(haystack: &str, needle: &str) -> Option<usize> {
    let needle_lower: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
    if needle_lower.is_empty() {
        return None;
    }
    let haystack_chars: Vec<char> = haystack.chars().collect();
    if haystack_chars.is_empty() {
        return None;
    }

    for i in 0..haystack_chars.len() {
        let mut matched = true;
        let mut h_idx = i;
        let mut n_idx = 0;
        while n_idx < needle_lower.len() {
            if h_idx >= haystack_chars.len() {
                matched = false;
                break;
            }
            let h_lowered: Vec<char> = haystack_chars[h_idx].to_lowercase().collect();
            if h_lowered.is_empty() || !needle_lower[n_idx..].starts_with(&h_lowered) {
                matched = false;
                break;
            }
            n_idx += h_lowered.len();
            h_idx += 1;
        }
        if matched && n_idx == needle_lower.len() {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn reconcile_filesystem_adds_skips_unchanged_and_removes_deleted() {
        let vault = TempDir::new().unwrap();
        let sub = vault.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        let a = vault.path().join("a.tmt");
        let b = sub.join("b.tmt");
        let hidden = vault.path().join(".hidden.tmt");
        let non_tmt = vault.path().join("image.png");

        std::fs::write(&a, "Alpha note").unwrap();
        std::fs::write(&b, "Beta note").unwrap();
        std::fs::write(&hidden, "Hidden").unwrap();
        std::fs::write(&non_tmt, "Png data").unwrap();

        let mut index = NoteIndex::open_in_memory().unwrap();

        // Initial reconciliation
        let report = index.reconcile_filesystem(vault.path()).unwrap();
        assert_eq!(report.added, 2);
        assert_eq!(report.updated, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.unchanged, 0);

        let notes = index.list_all().unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].path, PathBuf::from("a.tmt"));
        assert_eq!(notes[0].title, "a");
        assert_eq!(notes[1].path, PathBuf::from("sub/b.tmt"));
        assert_eq!(notes[1].title, "sub/b");

        // Second reconciliation without changes should find 2 unchanged
        let report2 = index.reconcile_filesystem(vault.path()).unwrap();
        assert_eq!(report2.added, 0);
        assert_eq!(report2.updated, 0);
        assert_eq!(report2.removed, 0);
        assert_eq!(report2.unchanged, 2);

        // Delete a.tmt from disk, reconcile should remove it
        std::fs::remove_file(&a).unwrap();
        let report3 = index.reconcile_filesystem(vault.path()).unwrap();
        assert_eq!(report3.removed, 1);
        assert_eq!(report3.unchanged, 1);

        let notes_after = index.list_all().unwrap();
        assert_eq!(notes_after.len(), 1);
        assert_eq!(notes_after[0].path, PathBuf::from("sub/b.tmt"));
    }

    #[test]
    fn apply_sync_report_updates_and_deletes_without_full_scan() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();

        let n1 = vault.path().join("n1.tmt");
        let n2 = vault.path().join("n2.tmt");
        std::fs::write(&n1, "Note 1 original").unwrap();
        std::fs::write(&n2, "Note 2 to be deleted").unwrap();

        index.reconcile_filesystem(vault.path()).unwrap();
        assert_eq!(index.list_all().unwrap().len(), 2);

        // Simulate incoming sync: n1 updated with conflict, n2 deleted, n3 added
        let n3 = vault.path().join("n3.tmt");
        std::fs::write(&n1, "Note 1 merged\n@mobile.conflict\nmine\n===\ntheirs").unwrap();
        std::fs::remove_file(&n2).unwrap();
        std::fs::write(&n3, "Note 3 newly received").unwrap();

        index
            .apply_sync_report(
                vault.path(),
                &[PathBuf::from("n1.tmt"), PathBuf::from("n3.tmt")],
                &[PathBuf::from("n2.tmt")],
                &[PathBuf::from("n1.tmt")],
            )
            .unwrap();

        let notes = index.list_all().unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].path, PathBuf::from("n1.tmt"));
        assert!(notes[0].has_conflict);
        assert_eq!(notes[1].path, PathBuf::from("n3.tmt"));
        assert!(!notes[1].has_conflict);

        assert_eq!(index.conflict_count().unwrap(), 1);
    }

    #[test]
    fn search_ranking_and_snippets_across_japanese_and_english() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();

        let n0 = vault.path().join("rust-guide.tmt");
        let n1 = vault.path().join("todo.tmt");
        let n2 = vault.path().join("買い物.tmt");

        std::fs::write(&n0, "A complete guide to programming in rust").unwrap();
        std::fs::write(&n1, "Buy milk\nLearn Rust async runtime\nClean up").unwrap();
        std::fs::write(&n2, "今日の予定はとても忙しいです。朝から牛乳を買いに行って、それからプログラミングの勉強をします。").unwrap();

        index.reconcile_filesystem(vault.path()).unwrap();

        // 1. Search for "rust": title match (n0) comes before body-only match (n1)
        let results = index.search("RUST").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].path, PathBuf::from("rust-guide.tmt"));
        assert_eq!(
            results[0].snippet,
            Some("A complete guide to programming in rust".into())
        );
        assert_eq!(results[1].path, PathBuf::from("todo.tmt"));
        assert_eq!(results[1].snippet, Some("Learn Rust async runtime".into()));

        // 2. Short Japanese search: 2 chars "牛乳"
        let results_jp = index.search("牛乳").unwrap();
        assert_eq!(results_jp.len(), 1);
        assert_eq!(results_jp[0].path, PathBuf::from("買い物.tmt"));
        assert!(results_jp[0].snippet.as_ref().unwrap().contains("牛乳"));

        // 3. 1 char search: "牛"
        let results_single = index.search("牛").unwrap();
        assert_eq!(results_single.len(), 1);
        assert_eq!(results_single[0].path, PathBuf::from("買い物.tmt"));

        // 4. Multi-char Japanese search: "プログラミング"
        let results_prog = index.search("プログラミング").unwrap();
        assert_eq!(results_prog.len(), 1);
        assert_eq!(results_prog[0].path, PathBuf::from("買い物.tmt"));

        // 5. Empty search returns all notes
        let all = index.search("").unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn record_crud_operations() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();

        let path = PathBuf::from("draft.tmt");
        let abs_path = vault.path().join(&path);
        std::fs::write(&abs_path, "Initial draft content").unwrap();

        index
            .record_write(vault.path(), &path, "Initial draft content")
            .unwrap();
        assert_eq!(index.list_all().unwrap().len(), 1);
        assert_eq!(index.list_all().unwrap()[0].title, "draft");

        // Rename
        let new_path = PathBuf::from("final.tmt");
        let new_abs = vault.path().join(&new_path);
        std::fs::rename(&abs_path, &new_abs).unwrap();

        index.record_rename(&path, &new_path).unwrap();
        assert_eq!(
            index.list_all().unwrap()[0].path,
            PathBuf::from("final.tmt")
        );
        assert_eq!(index.list_all().unwrap()[0].title, "final");

        // Delete
        index.record_delete(&new_path).unwrap();
        assert_eq!(index.list_all().unwrap().len(), 0);
    }

    #[test]
    fn search_handles_special_characters_without_crashing() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();

        let n1 = vault.path().join("code.tmt");
        std::fs::write(
            &n1,
            "Programming in C++ and C# with 100% \"test_coverage\" (awesome)!",
        )
        .unwrap();

        index.reconcile_filesystem(vault.path()).unwrap();

        // Testing queries that would crash unescaped FTS5
        let res_plus = index.search("C++").unwrap();
        assert_eq!(res_plus.len(), 1);

        let res_percent = index.search("100%").unwrap();
        assert_eq!(res_percent.len(), 1);

        let res_paren = index.search("(awesome)").unwrap();
        assert_eq!(res_paren.len(), 1);

        let res_quotes = index.search("\"test_coverage\"").unwrap();
        assert_eq!(res_quotes.len(), 1);

        let res_no_quotes = index.search("test_coverage").unwrap();
        assert_eq!(res_no_quotes.len(), 1);
    }

    #[test]
    fn set_conflict_and_conflict_count_round_trip() {
        let mut index = NoteIndex::open_in_memory().unwrap();
        let p1 = PathBuf::from("a.tmt");
        let p2 = PathBuf::from("b.tmt");

        index
            .conn
            .execute(
                "INSERT INTO notes (path, title, mtime_secs, mtime_nanos, size, has_conflict, body)
             VALUES ('a.tmt', 'a', 0, 0, 0, 0, 'content a'),
                    ('b.tmt', 'b', 0, 0, 0, 0, 'content b')",
                [],
            )
            .unwrap();

        assert_eq!(index.conflict_count().unwrap(), 0);

        index.set_conflict(&p1, true).unwrap();
        assert_eq!(index.conflict_count().unwrap(), 1);

        let notes = index.list_all().unwrap();
        assert!(notes.iter().find(|n| n.path == p1).unwrap().has_conflict);
        assert!(!notes.iter().find(|n| n.path == p2).unwrap().has_conflict);

        index.set_conflict(&p1, false).unwrap();
        assert_eq!(index.conflict_count().unwrap(), 0);
    }

    #[test]
    fn reconcile_detects_modified_files() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();

        let note = vault.path().join("note.tmt");
        std::fs::write(&note, "Version 1").unwrap();

        let rep1 = index.reconcile_filesystem(vault.path()).unwrap();
        assert_eq!(rep1.added, 1);

        // Modify file with new content
        std::fs::write(&note, "Version 2 with more words").unwrap();

        let rep2 = index.reconcile_filesystem(vault.path()).unwrap();
        assert_eq!(rep2.updated, 1);
        assert_eq!(rep2.added, 0);
        assert_eq!(rep2.unchanged, 0);

        let hits = index.search("Version 2").unwrap();
        assert_eq!(hits.len(), 1);
    }
}
