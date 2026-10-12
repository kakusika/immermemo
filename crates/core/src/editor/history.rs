//! Undo/redo for one editing session, kept as a lightweight in-memory snapshot stack.
//!
//! A [`History`] lives as long as one note stays open and unchanged from outside;
//! when a sync rewrites the note, the caller drops it and starts a new one, so nothing
//! from before the sync can be undone into content that no longer matches the file.

use std::time::{Duration, Instant};

/// The state the editor should show after an undo or redo.
#[derive(Debug, PartialEq, Eq)]
pub struct Restored {
    pub text: String,
    /// Byte offset just past the restored change, where the caret belongs.
    pub cursor: usize,
}

pub struct History {
    /// Past text states that can be restored via `undo()`.
    undo_stack: Vec<String>,
    /// Future text states that can be restored via `redo()`.
    redo_stack: Vec<String>,
    /// The current document text.
    current: String,
    /// Minimum pause required between edits to form a new undo checkpoint.
    merge_interval: Duration,
    /// When the last edit occurred.
    last_edit_time: Option<Instant>,
    /// Whether the current edit burst has already saved an undo checkpoint.
    in_merge_burst: bool,
}

impl History {
    /// Typing bursts closer together than this become one undo step.
    const MERGE_INTERVAL_MS: u64 = 500;
    /// Maximum undo steps kept in memory.
    const MAX_STACK_DEPTH: usize = 1000;

    pub fn new(initial: &str) -> Self {
        Self::with_merge_interval(initial, Self::MERGE_INTERVAL_MS as i64)
    }

    pub fn with_merge_interval(initial: &str, interval_ms: i64) -> Self {
        let merge_interval = if interval_ms > 0 {
            Duration::from_millis(interval_ms as u64)
        } else {
            Duration::ZERO
        };
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current: initial.to_owned(),
            merge_interval,
            last_edit_time: None,
            in_merge_burst: false,
        }
    }

    /// Records the editor's new full text as an edit on top of the last one.
    pub fn edit(&mut self, new_text: &str) {
        if new_text == self.current {
            return;
        }
        let now = Instant::now();
        let is_burst = !self.merge_interval.is_zero()
            && self.in_merge_burst
            && self
                .last_edit_time
                .is_some_and(|t| now.duration_since(t) < self.merge_interval);

        if !is_burst {
            if self.undo_stack.len() >= Self::MAX_STACK_DEPTH {
                self.undo_stack.remove(0);
            }
            self.undo_stack.push(self.current.clone());
            self.in_merge_burst = true;
        }
        self.redo_stack.clear();
        self.last_edit_time = Some(now);
        self.current = new_text.to_owned();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) -> Option<Restored> {
        let prev = self.undo_stack.pop()?;
        self.redo_stack.push(self.current.clone());
        self.in_merge_burst = false;
        self.last_edit_time = None;
        Some(self.restored_to(prev))
    }

    pub fn redo(&mut self) -> Option<Restored> {
        let next = self.redo_stack.pop()?;
        self.undo_stack.push(self.current.clone());
        self.in_merge_burst = false;
        self.last_edit_time = None;
        Some(self.restored_to(next))
    }

    fn restored_to(&mut self, text: String) -> Restored {
        let (start, _, insert) = diff(&self.current, &text);
        let cursor = byte_offset(&text, start + insert.chars().count());
        self.current = text.clone();
        Restored { text, cursor }
    }
}

/// The smallest single replacement turning `old` into `new`, as
/// (char index, chars to delete, text to insert).
fn diff<'a>(old: &str, new: &'a str) -> (usize, usize, &'a str) {
    let old: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();
    let prefix = old
        .iter()
        .zip(&new_chars)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new_chars[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let delete = old.len() - prefix - suffix;
    let insert_chars = new_chars.len() - prefix - suffix;
    let from = byte_offset(new, prefix);
    let to = byte_offset(new, prefix + insert_chars);
    (prefix, delete, &new[from..to])
}

fn byte_offset(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_finds_the_middle_replacement() {
        assert_eq!(diff("abcdef", "abXYef"), (2, 2, "XY"));
        assert_eq!(diff("abc", "abc"), (3, 0, ""));
        assert_eq!(diff("", "あい"), (0, 0, "あい"));
        assert_eq!(diff("あいう", "あう"), (1, 1, ""));
    }

    #[test]
    fn undo_and_redo_step_through_edits() {
        let mut h = History::with_merge_interval("", 0);
        h.edit("a");
        h.edit("ab");
        h.edit("abc");
        assert_eq!(h.undo().unwrap().text, "ab");
        assert_eq!(h.undo().unwrap().text, "a");
        assert_eq!(h.redo().unwrap().text, "ab");
    }

    #[test]
    fn can_undo_and_can_redo_follow_the_history() {
        let mut h = History::with_merge_interval("", 0);
        assert!(!h.can_undo() && !h.can_redo());
        h.edit("a");
        assert!(h.can_undo() && !h.can_redo());
        h.undo();
        assert!(!h.can_undo() && h.can_redo());
    }

    #[test]
    fn loading_the_note_is_not_an_undo_step() {
        let mut h = History::with_merge_interval("既存の文章", 0);
        assert_eq!(h.undo(), None);
        h.edit("既存の文章です");
        assert_eq!(h.undo().unwrap().text, "既存の文章");
        assert_eq!(h.undo(), None);
    }

    #[test]
    fn caret_lands_after_the_restored_change() {
        let mut h = History::with_merge_interval("あいう", 0);
        h.edit("あいXう");
        let r = h.undo().unwrap();
        assert_eq!(r.text, "あいう");
        // After undoing an insert, the caret sits where the insert was.
        assert_eq!(r.cursor, "あい".len());
    }

    #[test]
    fn rapid_typing_merges_into_one_step() {
        let mut h = History::new("");
        h.edit("a");
        h.edit("ab");
        h.edit("abc");
        assert_eq!(h.undo().unwrap().text, "");
    }
}
