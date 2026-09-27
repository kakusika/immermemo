//! Undo/redo for one editing session, kept in a Loro document.
//!
//! Loro is used here for nothing but the operation history: the `.tmt` text
//! on disk stays the only source of truth, and the document is never
//! written anywhere. A [`History`] lives as long as one note stays open and
//! unchanged from outside; when a sync rewrites the note, the caller drops
//! it and starts a new one, so nothing from before the sync can be undone
//! into content that no longer matches the file.

use loro::{LoroDoc, LoroText, UndoManager};

/// The state the editor should show after an undo or redo.
#[derive(Debug, PartialEq, Eq)]
pub struct Restored {
    pub text: String,
    /// Byte offset just past the restored change, where the caret belongs.
    pub cursor: usize,
}

pub struct History {
    doc: LoroDoc,
    text: LoroText,
    undo: UndoManager,
    current: String,
}

impl History {
    /// Typing bursts closer together than this become one undo step.
    const MERGE_INTERVAL_MS: i64 = 500;

    pub fn new(initial: &str) -> Self {
        Self::with_merge_interval(initial, Self::MERGE_INTERVAL_MS)
    }

    pub fn with_merge_interval(initial: &str, interval_ms: i64) -> Self {
        let doc = LoroDoc::new();
        let text = doc.get_text("text");
        text.insert(0, initial).expect("insert into a fresh text");
        doc.commit();
        // Created after the initial insert, so loading the note is not
        // itself an undo step.
        let mut undo = UndoManager::new(&doc);
        undo.set_merge_interval(interval_ms);
        Self {
            doc,
            text,
            undo,
            current: initial.to_owned(),
        }
    }

    /// Records the editor's new full text as an edit on top of the last one.
    pub fn edit(&mut self, new_text: &str) {
        let (start, delete, insert) = diff(&self.current, new_text);
        if delete > 0 {
            self.text
                .delete(start, delete)
                .expect("delete within bounds");
        }
        if !insert.is_empty() {
            self.text
                .insert(start, insert)
                .expect("insert within bounds");
        }
        self.doc.commit();
        self.current = new_text.to_owned();
    }

    pub fn undo(&mut self) -> Option<Restored> {
        if !self.undo.undo().ok()? {
            return None;
        }
        Some(self.restored())
    }

    pub fn redo(&mut self) -> Option<Restored> {
        if !self.undo.redo().ok()? {
            return None;
        }
        Some(self.restored())
    }

    fn restored(&mut self) -> Restored {
        let text = self.text.to_string();
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
