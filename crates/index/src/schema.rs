//! Database connection initialization, schema creation, migrations, and shared SQL statements.

use rusqlite::Connection;

/// Shared by every place that writes a note row: insert it fresh, or
/// overwrite every column if the path already exists.
pub(crate) const UPSERT_NOTE_SQL: &str =
    "INSERT INTO notes (path, title, mtime_secs, mtime_nanos, size, has_conflict, body, icon_name, icon_pkg)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
     ON CONFLICT(path) DO UPDATE SET
        title = excluded.title,
        mtime_secs = excluded.mtime_secs,
        mtime_nanos = excluded.mtime_nanos,
        size = excluded.size,
        has_conflict = excluded.has_conflict,
        body = excluded.body,
        icon_name = excluded.icon_name,
        icon_pkg = excluded.icon_pkg";

/// Initializes pragmas, creates tables, indexes, triggers, and applies schema migrations.
pub(crate) fn init_connection(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;",
    )?;

    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 0 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notes (
                path TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL,
                size INTEGER NOT NULL,
                has_conflict INTEGER NOT NULL DEFAULT 0,
                body TEXT NOT NULL,
                icon_name TEXT,
                icon_pkg TEXT
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

            PRAGMA user_version = 2;
            ",
        )?;
    } else if version == 1 {
        let _ = conn.execute("ALTER TABLE notes ADD COLUMN icon_name TEXT;", []);
        let _ = conn.execute("ALTER TABLE notes ADD COLUMN icon_pkg TEXT;", []);
        let _ = conn.execute("PRAGMA user_version = 2;", []);
    }
    Ok(())
}
