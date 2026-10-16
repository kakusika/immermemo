//! Filesystem scanning, metadata reconciliation, and incremental sync updates.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::params;
use tomet::merge::CONFLICT_MARKER;

use crate::schema::{IconColumns, UPSERT_NOTE_SQL};
use crate::{NoteIndex, display_name_from_rel};

/// Summary of changes detected during a filesystem reconciliation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
}

impl NoteIndex {
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
        let mut cached_notes: HashMap<PathBuf, (u64, u64, u32)> = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT path, size, mtime_secs, mtime_nanos FROM notes")?;
            let mut rows = stmt.query([])?;
            let mut to_remove: Vec<PathBuf> = Vec::new();

            while let Some(row) = rows.next()? {
                let path_str: String = row.get(0)?;
                let rel_path = PathBuf::from(&path_str);
                let cached_size: u64 = row.get(1)?;
                let cached_secs: u64 = row.get(2)?;
                let cached_nanos: u32 = row.get(3)?;
                match disk_meta.get(&rel_path) {
                    None => to_remove.push(rel_path),
                    Some((size, secs, nanos)) => {
                        if *size == cached_size && *secs == cached_secs && *nanos == cached_nanos {
                            report.unchanged += 1;
                        }
                        cached_notes.insert(rel_path, (cached_size, cached_secs, cached_nanos));
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
            let mut upsert_stmt = tx.prepare_cached(UPSERT_NOTE_SQL)?;

            for (rel_path, size, mtime_secs, mtime_nanos) in disk_files {
                let cached = cached_notes.get(&rel_path);
                let needs_read = match cached {
                    Some(&(cached_size, cached_sec, cached_nano)) => {
                        cached_size != size
                            || cached_sec != mtime_secs
                            || cached_nano != mtime_nanos
                    }
                    None => true,
                };

                if needs_read {
                    let path_str = rel_path.to_string_lossy().to_string();
                    let abs_path = vault_dir.join(&rel_path);
                    if let Ok(body) = std::fs::read_to_string(&abs_path) {
                        let has_conflict = body.contains(CONFLICT_MARKER);
                        let title = display_name_from_rel(&rel_path);
                        let is_new = cached.is_none();
                        let icon = IconColumns::of_note(&body);

                        upsert_stmt.execute(params![
                            &path_str,
                            &title,
                            mtime_secs,
                            mtime_nanos,
                            size,
                            if has_conflict { 1 } else { 0 },
                            &body,
                            icon.kind,
                            icon.value,
                            icon.pkg,
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
                    let icon = IconColumns::of_note(&body);

                    upsert_stmt.execute(params![
                        &path_str,
                        &title,
                        mtime_secs,
                        mtime_nanos,
                        size,
                        if has_conflict { 1 } else { 0 },
                        &body,
                        icon.kind,
                        icon.value,
                        icon.pkg,
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
}

pub(crate) fn mtime_parts(time: SystemTime) -> (u64, u32) {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => (d.as_secs(), d.subsec_nanos()),
        Err(_) => (0, 0),
    }
}

pub(crate) fn collect_tmt_files(root: &Path, found: &mut Vec<(PathBuf, u64, u64, u32)>) {
    immermemo_vault::notes::walk_tmt_files(root, &mut |path, meta| {
        if let Ok(rel) = path.strip_prefix(root) {
            let (secs, nanos) = mtime_parts(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
            found.push((rel.to_path_buf(), meta.len(), secs, nanos));
        }
    });
}
