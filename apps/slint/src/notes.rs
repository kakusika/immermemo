//! The note files of a vault folder, as the list in the window shows them.

use std::path::{Path, PathBuf};

const EXTENSION: &str = "tmt";
const CONFLICT_MARKER: &str = "@mobile.conflict";

/// Every `.tmt` file under `dir`, sorted by path. Hidden entries are skipped.
pub fn scan(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(dir, &mut found);
    found.sort();
    found
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk(&path, found);
        } else if path.extension().is_some_and(|e| e == EXTENSION) {
            found.push(path);
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
    std::fs::read_to_string(path).is_ok_and(|t| t.contains(CONFLICT_MARKER))
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
}
