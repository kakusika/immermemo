//! Search facilities: title matching, SQLite FTS5 trigram queries, LIKE fallback, and snippet extraction.

use std::collections::HashSet;
use std::path::PathBuf;

use rusqlite::{Row, params};

use crate::NoteIndex;

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

impl NoteIndex {
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
pub(crate) fn row_to_note_fields(row: &Row) -> rusqlite::Result<(PathBuf, String, bool, String)> {
    let path_str: String = row.get(0)?;
    let title: String = row.get(1)?;
    let conflict_int: i32 = row.get(2)?;
    let body: String = row.get(3)?;
    Ok((PathBuf::from(path_str), title, conflict_int != 0, body))
}

pub(crate) fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
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

pub(crate) fn find_case_insensitive_char_idx(haystack: &str, needle: &str) -> Option<usize> {
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
