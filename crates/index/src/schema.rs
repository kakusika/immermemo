//! Database connection initialization, schema creation, migrations, and shared SQL statements.

use immermemo_render_tomet::NoteIcon;
use rusqlite::Connection;

/// The schema version [`init_connection`] leaves a database at.
const SCHEMA_VERSION: u32 = 3;

/// A [`NoteIcon`] as the three `notes` columns it is stored in: `icon_kind`
/// (`glyph`/`text`/`image`), `icon_value` (the glyph name, the text, or the
/// image reference) and `icon_pkg` (glyphs only). All `None` is "no icon".
#[derive(Debug, Default)]
pub(crate) struct IconColumns {
    pub kind: Option<String>,
    pub value: Option<String>,
    pub pkg: Option<String>,
}

impl IconColumns {
    /// The columns for whatever icon the note source `content` declares.
    pub fn of_note(content: &str) -> Self {
        immermemo_render_tomet::extract_note_icon(content).map_or_else(Self::default, Self::from)
    }

    /// Reads an icon back from its columns; an unknown kind (written by a
    /// newer version) or a missing value is "no icon".
    pub fn into_icon(self) -> Option<NoteIcon> {
        let value = self.value.filter(|v| !v.is_empty())?;
        match self.kind?.as_str() {
            "glyph" => Some(NoteIcon::Glyph {
                name: value,
                pkg: self
                    .pkg
                    .unwrap_or_else(|| immermemo_render_tomet::DEFAULT_ICON_PKG.to_owned()),
            }),
            "text" => Some(NoteIcon::Text(value)),
            "image" => Some(NoteIcon::Image(value)),
            _ => None,
        }
    }
}

impl From<NoteIcon> for IconColumns {
    fn from(icon: NoteIcon) -> Self {
        match icon {
            NoteIcon::Glyph { name, pkg } => Self {
                kind: Some("glyph".to_owned()),
                value: Some(name),
                pkg: Some(pkg),
            },
            NoteIcon::Text(text) => Self {
                kind: Some("text".to_owned()),
                value: Some(text),
                pkg: None,
            },
            NoteIcon::Image(reference) => Self {
                kind: Some("image".to_owned()),
                value: Some(reference),
                pkg: None,
            },
        }
    }
}

/// Shared by every place that writes a note row: insert it fresh, or
/// overwrite every column if the path already exists.
pub(crate) const UPSERT_NOTE_SQL: &str =
    "INSERT INTO notes (path, title, mtime_secs, mtime_nanos, size, has_conflict, body, icon_kind, icon_value, icon_pkg)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
     ON CONFLICT(path) DO UPDATE SET
        title = excluded.title,
        mtime_secs = excluded.mtime_secs,
        mtime_nanos = excluded.mtime_nanos,
        size = excluded.size,
        has_conflict = excluded.has_conflict,
        body = excluded.body,
        icon_kind = excluded.icon_kind,
        icon_value = excluded.icon_value,
        icon_pkg = excluded.icon_pkg";

/// Initializes pragmas and creates the tables, indexes and triggers.
///
/// The index only caches what the vault's files say, so there are no
/// migrations: a database left by any other schema version is dropped and
/// created afresh, and the next reconcile fills it from the vault again.
pub(crate) fn init_connection(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;",
    )?;

    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    conn.execute_batch(
        "DROP TABLE IF EXISTS notes_fts;
        DROP TABLE IF EXISTS notes;

        CREATE TABLE notes (
            path TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            mtime_secs INTEGER NOT NULL,
            mtime_nanos INTEGER NOT NULL,
            size INTEGER NOT NULL,
            has_conflict INTEGER NOT NULL DEFAULT 0,
            body TEXT NOT NULL,
            icon_kind TEXT,
            icon_value TEXT,
            icon_pkg TEXT
        );

        CREATE VIRTUAL TABLE notes_fts USING fts5(
            path UNINDEXED,
            title,
            body,
            tokenize = 'trigram'
        );

        CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN
            INSERT INTO notes_fts (path, title, body) VALUES (new.path, new.title, new.body);
        END;

        CREATE TRIGGER notes_ad AFTER DELETE ON notes BEGIN
            DELETE FROM notes_fts WHERE path = old.path;
        END;

        CREATE TRIGGER notes_au AFTER UPDATE ON notes BEGIN
            DELETE FROM notes_fts WHERE path = old.path;
            INSERT INTO notes_fts (path, title, body) VALUES (new.path, new.title, new.body);
        END;",
    )?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}
