//! The note files of a vault folder, as the list in the window shows them.

use std::path::{Path, PathBuf};

const EXTENSION: &str = "tmt";

/// Every `.tmt` file under `dir`, sorted by path. Hidden entries are skipped.
pub fn scan(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk_tmt_files(dir, &mut |path, _meta| found.push(path.to_path_buf()));
    found.sort();
    found
}

/// Walks every `.tmt` file under `dir`, recursively, skipping hidden
/// entries (dotfiles and dotdirs, including `.git`). Calls `visit(path,
/// meta)` for each one found, in filesystem order (unspecified -- sort
/// the collected results if order matters).
///
/// The canonical `.tmt` walk other crates (`immermemo-index`,
/// `immermemo-sync`) build on, instead of each keeping its own copy of
/// this recursion with its own slightly different rules for what counts
/// as a note file. An unreadable directory is treated as empty rather
/// than failing the whole walk.
pub fn walk_tmt_files(dir: &Path, visit: &mut impl FnMut(&Path, &std::fs::Metadata)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk_tmt_files(&path, visit);
        } else if path.extension().is_some_and(|e| e == EXTENSION) {
            if let Ok(meta) = entry.metadata() {
                visit(&path, &meta);
            }
        }
    }
}

pub fn display_name(vault_dir: &Path, path: &Path) -> String {
    path.strip_prefix(vault_dir)
        .unwrap_or(path)
        .with_extension("")
        .display()
        .to_string()
}

/// A cheap textual check for the list badge; the sync report is what says
/// which notes the merge itself flagged.
pub fn has_conflict_marker(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| t.contains(immermemo_merge::CONFLICT_MARKER))
}

/// Deletes a note file outright. Not recoverable from the UI -- the
/// caller is responsible for confirming with the user first.
pub fn delete(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}

/// Renames a note to `new_name` (no extension, no path separators),
/// keeping it in the same directory -- this can't move a note to a
/// different folder, only change its file name. Fails if a note with
/// that name already exists there.
pub fn rename(path: &Path, new_name: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!new_name.is_empty(), "the name can't be empty");
    anyhow::ensure!(
        !new_name.contains(['/', '\\']),
        "the name can't contain a path separator"
    );
    let new_path = path.with_file_name(format!("{new_name}.{EXTENSION}"));
    anyhow::ensure!(
        !new_path.exists(),
        "a note named \"{new_name}\" already exists here"
    );
    std::fs::rename(path, &new_path)?;
    Ok(new_path)
}

/// Creates an empty `untitled-N.tmt` and returns its path.
pub fn create(dir: &Path) -> std::io::Result<PathBuf> {
    for n in 1.. {
        let path = dir.join(format!("untitled-{n}.{EXTENSION}"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub note_index: usize,
    pub snippet: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_nested_tmt_files_and_skips_hidden_and_others() {
        let dir = std::env::temp_dir().join(format!("immermemo-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("a.tmt"), "").unwrap();
        std::fs::write(dir.join("sub/b.tmt"), "").unwrap();
        std::fs::write(dir.join("c.png"), "").unwrap();
        std::fs::write(dir.join(".hidden/d.tmt"), "").unwrap();

        let names: Vec<_> = scan(&dir).iter().map(|p| display_name(&dir, p)).collect();
        assert_eq!(names, ["a", "sub/b"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("immermemo-create-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = create(&dir).unwrap();
        let second = create(&dir).unwrap();
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_the_file() {
        let dir = std::env::temp_dir().join(format!("immermemo-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.tmt");
        std::fs::write(&path, "").unwrap();

        delete(&path).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_keeps_the_note_in_the_same_directory() {
        let dir = std::env::temp_dir().join(format!("immermemo-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let path = dir.join("sub/a.tmt");
        std::fs::write(&path, "content").unwrap();

        let new_path = rename(&path, "b").unwrap();
        assert_eq!(new_path, dir.join("sub/b.tmt"));
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "content");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_rejects_an_empty_name_a_path_separator_or_an_existing_note() {
        let dir =
            std::env::temp_dir().join(format!("immermemo-rename-reject-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.tmt");
        std::fs::write(&path, "").unwrap();
        std::fs::write(dir.join("b.tmt"), "").unwrap();

        assert!(rename(&path, "").is_err());
        assert!(rename(&path, "sub/b").is_err());
        assert!(rename(&path, "b").is_err());
        assert!(path.exists(), "a rejected rename must not touch the file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
