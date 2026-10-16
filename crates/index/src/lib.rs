//! Note index database for fast listing, metadata caching, and full-text search.
//!
//! Stores note metadata (`path`, `title`, `mtime`, `size`, `has_conflict`) and provides
//! fast full-text search powered by SQLite and FTS5 trigram indexing.
//!
//! Designed to scale to tens of thousands of notes without performing full filesystem
//! walks or per-file disk reads on every UI refresh or keystroke search.

mod reconcile;
mod schema;
mod search;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::{Connection, params};
use tomet::merge::CONFLICT_MARKER;

pub use immermemo_render_tomet::NoteIcon;
pub use reconcile::ReconcileReport;
use reconcile::mtime_parts;
use schema::{IconColumns, UPSERT_NOTE_SQL};
pub use search::{SearchHit, extract_snippet};

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

/// SQLite-backed index for a vault.
pub struct NoteIndex {
    pub(crate) conn: Connection,
}

impl NoteIndex {
    /// Opens or creates an index database at `db_path`.
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        schema::init_connection(&conn)?;
        Ok(Self { conn })
    }

    /// Opens an in-memory index database (primarily for testing).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::init_connection(&conn)?;
        Ok(Self { conn })
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
        let icon = IconColumns::of_note(content);

        self.conn.execute(
            UPSERT_NOTE_SQL,
            params![
                &path_str,
                &title,
                mtime_secs,
                mtime_nanos,
                meta.len(),
                if has_conflict { 1 } else { 0 },
                content,
                icon.kind,
                icon.value,
                icon.pkg,
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
        let notes = stmt
            .query_map([], indexed_note_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(notes)
    }

    /// Every indexed note's relative path alongside its cached body (the
    /// same text already stored for FTS search, not a fresh disk read) --
    /// for a caller that needs to derive something from a note's content
    /// without paying for a full `reconcile_filesystem` walk.
    pub fn bodies(&self) -> anyhow::Result<Vec<(PathBuf, String)>> {
        let mut stmt = self.conn.prepare("SELECT path, body FROM notes")?;
        let bodies = stmt
            .query_map([], |row| {
                Ok((PathBuf::from(row.get::<_, String>(0)?), row.get(1)?))
            })?
            .collect::<Result<_, _>>()?;
        Ok(bodies)
    }

    /// Returns the cached body of the note at `rel_path`, if indexed.
    pub fn body_for(&self, rel_path: &Path) -> anyhow::Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT body FROM notes WHERE path = ?1")?;
        let path_str = rel_path.to_string_lossy();
        let mut rows = stmt.query(params![path_str])?;
        if let Some(row) = rows.next()? {
            let body: String = row.get(0)?;
            Ok(Some(body))
        } else {
            Ok(None)
        }
    }

    /// Returns the cached icon the note at `rel_path` declares, if any.
    pub fn icon_for(&self, rel_path: &Path) -> anyhow::Result<Option<NoteIcon>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT icon_kind, icon_value, icon_pkg FROM notes WHERE path = ?1")?;
        let path_str = rel_path.to_string_lossy();
        let mut rows = stmt.query(params![path_str])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(IconColumns {
            kind: row.get(0)?,
            value: row.get(1)?,
            pkg: row.get(2)?,
        }
        .into_icon())
    }

    /// Returns up to `limit` most recently modified notes, newest first.
    pub fn list_recent(&self, limit: usize) -> anyhow::Result<Vec<IndexedNote>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, title, has_conflict FROM notes
             ORDER BY mtime_secs DESC, mtime_nanos DESC
             LIMIT ?1",
        )?;
        let notes = stmt
            .query_map(params![limit as i64], indexed_note_from_row)?
            .collect::<Result<_, _>>()?;
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
}

fn indexed_note_from_row(row: &rusqlite::Row) -> rusqlite::Result<IndexedNote> {
    Ok(IndexedNote {
        path: PathBuf::from(row.get::<_, String>(0)?),
        title: row.get(1)?,
        has_conflict: row.get::<_, i32>(2)? != 0,
    })
}

pub(crate) fn display_name_from_rel(rel_path: &Path) -> String {
    rel_path.with_extension("").to_string_lossy().to_string()
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
    fn bodies_returns_every_note_s_cached_content_by_path() {
        let vault = TempDir::new().unwrap();
        std::fs::write(vault.path().join("a.tmt"), "Alpha content").unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();
        index.reconcile_filesystem(vault.path()).unwrap();

        let bodies = index.bodies().unwrap();
        assert_eq!(
            bodies,
            vec![(PathBuf::from("a.tmt"), "Alpha content".to_owned())]
        );
    }

    #[test]
    fn list_recent_orders_by_mtime_descending_and_respects_limit() {
        let vault = TempDir::new().unwrap();
        let oldest = vault.path().join("oldest.tmt");
        let middle = vault.path().join("middle.tmt");
        let newest = vault.path().join("newest.tmt");

        for (path, age_secs) in [(&oldest, 2000), (&middle, 1000), (&newest, 0)] {
            std::fs::write(path, "content").unwrap();
            let mtime = SystemTime::now() - std::time::Duration::from_secs(age_secs);
            std::fs::File::open(path)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
        }

        let mut index = NoteIndex::open_in_memory().unwrap();
        index.reconcile_filesystem(vault.path()).unwrap();

        let recent = index.list_recent(2).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].path, PathBuf::from("newest.tmt"));
        assert_eq!(recent[1].path, PathBuf::from("middle.tmt"));

        assert_eq!(index.list_recent(10).unwrap().len(), 3);
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

    #[test]
    fn body_for_returns_cached_content() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();
        let note = vault.path().join("test.tmt");
        std::fs::write(&note, "Sample note body").unwrap();
        index.reconcile_filesystem(vault.path()).unwrap();

        let body = index.body_for(Path::new("test.tmt")).unwrap();
        assert_eq!(body.as_deref(), Some("Sample note body"));

        let nonexistent = index.body_for(Path::new("missing.tmt")).unwrap();
        assert_eq!(nonexistent, None);
    }

    #[test]
    fn icon_for_returns_persisted_icon_metadata() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();
        let note = vault.path().join("starred.tmt");
        std::fs::write(
            &note,
            "@meta{ icon: @doc.icon(\"star\") }\n\nStarred note.\n",
        )
        .unwrap();
        index.reconcile_filesystem(vault.path()).unwrap();

        let icon = index.icon_for(Path::new("starred.tmt")).unwrap();
        assert_eq!(
            icon,
            Some(NoteIcon::Glyph {
                name: "star".to_owned(),
                pkg: "tabler".to_owned()
            })
        );

        let plain = vault.path().join("plain.tmt");
        std::fs::write(&plain, "Plain note without icon.\n").unwrap();
        index.reconcile_filesystem(vault.path()).unwrap();

        let plain_icon = index.icon_for(Path::new("plain.tmt")).unwrap();
        assert_eq!(plain_icon, None);
    }

    #[test]
    fn icon_for_round_trips_a_text_and_an_image_icon() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();
        std::fs::write(
            vault.path().join("cake.tmt"),
            "@meta{ icon: \"🎂\" }\n\nCake.\n",
        )
        .unwrap();
        std::fs::write(
            vault.path().join("cover.tmt"),
            "@meta{ icon: @embed(\"images/cover.png\") }\n\nCover.\n",
        )
        .unwrap();
        index.reconcile_filesystem(vault.path()).unwrap();

        assert_eq!(
            index.icon_for(Path::new("cake.tmt")).unwrap(),
            Some(NoteIcon::Text("🎂".to_owned()))
        );
        assert_eq!(
            index.icon_for(Path::new("cover.tmt")).unwrap(),
            Some(NoteIcon::Image("images/cover.png".to_owned()))
        );
    }

    #[test]
    fn changing_a_notes_icon_kind_replaces_every_icon_column() {
        let vault = TempDir::new().unwrap();
        let mut index = NoteIndex::open_in_memory().unwrap();
        let rel = Path::new("n.tmt");
        std::fs::write(
            vault.path().join(rel),
            "@meta{ icon: @doc.icon(\"star\") }\n\nx\n",
        )
        .unwrap();
        index.reconcile_filesystem(vault.path()).unwrap();

        let rewritten = "@meta{ icon: \"🎂\" }\n\nx\n";
        std::fs::write(vault.path().join(rel), rewritten).unwrap();
        index.record_write(vault.path(), rel, rewritten).unwrap();

        assert_eq!(
            index.icon_for(rel).unwrap(),
            Some(NoteIcon::Text("🎂".to_owned()))
        );
        let pkg: Option<String> = index
            .conn
            .query_row("SELECT icon_pkg FROM notes WHERE path = 'n.tmt'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pkg, None);
    }

    /// The index only caches the vault, so a database from another schema
    /// version is rebuilt rather than migrated: it opens at the current
    /// schema, empty, and the next reconcile fills it from the vault.
    #[test]
    fn a_database_from_another_schema_version_is_rebuilt() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(
            vault.join("a.tmt"),
            "@meta{ icon: @doc.icon(\"star\") }\n\nx\n",
        )
        .unwrap();
        let db = dir.path().join("index.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE notes (
                    path TEXT PRIMARY KEY, title TEXT NOT NULL,
                    mtime_secs INTEGER NOT NULL, mtime_nanos INTEGER NOT NULL,
                    size INTEGER NOT NULL, has_conflict INTEGER NOT NULL DEFAULT 0,
                    body TEXT NOT NULL, icon_name TEXT, icon_pkg TEXT);
                INSERT INTO notes VALUES ('gone.tmt', 'gone', 0, 0, 1, 0, 'x', 'star', 'tabler');
                PRAGMA user_version = 2;",
            )
            .unwrap();
        }
        let mut index = NoteIndex::open(&db).unwrap();
        assert_eq!(index.icon_for(Path::new("gone.tmt")).unwrap(), None);

        index.reconcile_filesystem(&vault).unwrap();
        assert_eq!(
            index.icon_for(Path::new("a.tmt")).unwrap(),
            Some(NoteIcon::Glyph {
                name: "star".to_owned(),
                pkg: "tabler".to_owned()
            })
        );
        let version: u32 = index
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 3);
    }
}
