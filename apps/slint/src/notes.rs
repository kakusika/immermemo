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

/// Searches `notes` for `query` case-insensitively across both display name and file content.
///
/// Returns matching note indices into `notes`, with an optional snippet from the first
/// matching line of content. Title matches are ranked before body-only matches.
pub fn search(vault_dir: &Path, notes: &[PathBuf], query: &str) -> Vec<SearchResult> {
    let query = query.trim();
    if query.is_empty() {
        return notes
            .iter()
            .enumerate()
            .map(|(i, _)| SearchResult {
                note_index: i,
                snippet: None,
            })
            .collect();
    }

    let mut title_matches = Vec::new();
    let mut body_only_matches = Vec::new();

    for (index, path) in notes.iter().enumerate() {
        let name = display_name(vault_dir, path);
        let title_hit = contains_case_insensitive(&name, query);

        let content = std::fs::read_to_string(path).unwrap_or_default();
        let snippet = extract_snippet(&content, query);

        if title_hit {
            title_matches.push(SearchResult {
                note_index: index,
                snippet,
            });
        } else if let Some(snip) = snippet {
            body_only_matches.push(SearchResult {
                note_index: index,
                snippet: Some(snip),
            });
        }
    }

    title_matches.extend(body_only_matches);
    title_matches
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    find_case_insensitive_char_idx(haystack, needle).is_some()
}

fn extract_snippet(content: &str, query: &str) -> Option<String> {
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

    #[test]
    fn search_empty_query_returns_all_notes() {
        let dir = std::env::temp_dir().join(format!("immermemo-search-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.tmt");
        let b = dir.join("b.tmt");
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let notes = vec![a, b];

        let results = search(&dir, &notes, "");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].note_index, 0);
        assert_eq!(results[0].snippet, None);
        assert_eq!(results[1].note_index, 1);
        assert_eq!(results[1].snippet, None);

        let whitespace_results = search(&dir, &notes, "   ");
        assert_eq!(whitespace_results.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_matches_title_and_body_with_ranking_and_snippets() {
        let dir = std::env::temp_dir().join(format!("immermemo-search-ranking-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let n0 = dir.join("rust-notes.tmt");
        let n1 = dir.join("todo.tmt");
        let n2 = dir.join("learning.tmt");
        std::fs::write(&n0, "A guide to programming in rust").unwrap();
        std::fs::write(&n1, "Buy groceries\nLearn Rust async runtime\nClean up").unwrap();
        std::fs::write(&n2, "Python basics").unwrap();
        let notes = vec![n0, n1, n2];

        // Search for "rust" (case-insensitive)
        let results = search(&dir, &notes, "RUST");
        // Title match: n0 (rust-notes) comes first
        // Body match only: n1 (todo) comes second
        // n2 does not match
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].note_index, 0);
        assert_eq!(results[0].snippet, Some("A guide to programming in rust".into()));
        assert_eq!(results[1].note_index, 1);
        assert_eq!(results[1].snippet, Some("Learn Rust async runtime".into()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_japanese_content_and_long_line_snippet() {
        let dir = std::env::temp_dir().join(format!("immermemo-search-jp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let n0 = dir.join("買い物.tmt");
        let long_line = "今日の予定はとても忙しいです。朝から牛乳を買いに行って、それから銀行へ行き、その後にプログラミングの勉強をします。夜はゆっくり休みます。";
        std::fs::write(&n0, long_line).unwrap();
        let notes = vec![n0];

        let results = search(&dir, &notes, "牛乳");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note_index, 0);
        assert!(results[0].snippet.is_some());
        let snippet = results[0].snippet.as_ref().unwrap();
        assert!(snippet.contains("牛乳"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
