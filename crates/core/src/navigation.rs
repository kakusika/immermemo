//! Back/forward navigation stack between previously-opened notes.
//!
//! Distinct from `EditorState.history` (per-note text undo/redo) and the
//! git-commit history sheets. Holds vault-relative paths, not note indices,
//! since those shift under rename/create/delete/filtering.

/// Back/forward navigation stack between previously-opened notes.
/// `entries[cursor]` is always the note open right now.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BackForwardStack {
    entries: Vec<String>,
    cursor: Option<usize>,
}

impl BackForwardStack {
    /// Records a newly-opened note as "here", discarding any forward
    /// entries past the cursor (browser semantics: navigating to a new
    /// note after going back drops the abandoned forward branch). A
    /// no-op if `rel_path` is already "here" (re-opening the same note
    /// doesn't duplicate it).
    pub fn visit(&mut self, rel_path: String) {
        if self.cursor.is_some_and(|c| self.entries[c] == rel_path) {
            return;
        }
        let next = self.cursor.map_or(0, |c| c + 1);
        self.entries.truncate(next);
        self.entries.push(rel_path);
        self.cursor = Some(next);
    }

    pub fn can_back(&self) -> bool {
        self.cursor.is_some_and(|c| c > 0)
    }

    pub fn can_forward(&self) -> bool {
        self.cursor.is_some_and(|c| c + 1 < self.entries.len())
    }

    /// The note one step back from "here", for a lightweight preview.
    pub fn peek_back(&self) -> Option<&str> {
        let c = self.cursor?;
        (c > 0).then(|| self.entries[c - 1].as_str())
    }

    /// The note one step forward from "here", for a lightweight preview.
    pub fn peek_forward(&self) -> Option<&str> {
        self.entries.get(self.cursor? + 1).map(String::as_str)
    }

    /// Moves back one step, dropping entries `is_live` rejects (renamed
    /// or deleted since they were visited) before landing on one that's
    /// still real. Returns the path now "here", if any.
    pub fn step_back(&mut self, is_live: impl Fn(&str) -> bool) -> Option<String> {
        while self.can_back() {
            let c = self.cursor.unwrap();
            if is_live(&self.entries[c - 1]) {
                self.cursor = Some(c - 1);
                return Some(self.entries[c - 1].clone());
            }
            self.entries.remove(c - 1);
            self.cursor = Some(c - 1);
        }
        None
    }

    /// Mirrors `step_back`, moving forward instead.
    pub fn step_forward(&mut self, is_live: impl Fn(&str) -> bool) -> Option<String> {
        while self.can_forward() {
            let c = self.cursor.unwrap();
            if is_live(&self.entries[c + 1]) {
                self.cursor = Some(c + 1);
                return Some(self.entries[c + 1].clone());
            }
            self.entries.remove(c + 1);
        }
        None
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = None;
    }

    pub fn current(&self) -> Option<&str> {
        self.cursor
            .and_then(|c| self.entries.get(c))
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_forward_traversal_and_branch_truncation() {
        let mut nav = BackForwardStack::default();
        assert!(!nav.can_back());
        assert!(!nav.can_forward());

        nav.visit("note1.tmt".into());
        assert_eq!(nav.current(), Some("note1.tmt"));
        assert!(!nav.can_back());

        nav.visit("note2.tmt".into());
        assert_eq!(nav.current(), Some("note2.tmt"));
        assert!(nav.can_back());
        assert_eq!(nav.peek_back(), Some("note1.tmt"));

        // Step back to note1
        let prev = nav.step_back(|_| true);
        assert_eq!(prev, Some("note1.tmt".into()));
        assert_eq!(nav.current(), Some("note1.tmt"));
        assert!(nav.can_forward());
        assert_eq!(nav.peek_forward(), Some("note2.tmt"));

        // Visiting note3 drops forward branch (note2)
        nav.visit("note3.tmt".into());
        assert_eq!(nav.current(), Some("note3.tmt"));
        assert!(!nav.can_forward());
        assert!(nav.can_back());
        assert_eq!(nav.peek_back(), Some("note1.tmt"));
    }

    #[test]
    fn dead_entries_are_skipped() {
        let mut nav = BackForwardStack::default();
        nav.visit("a.tmt".into());
        nav.visit("deleted.tmt".into());
        nav.visit("b.tmt".into());

        // Step back skips "deleted.tmt"
        let landed = nav.step_back(|p| p != "deleted.tmt");
        assert_eq!(landed, Some("a.tmt".into()));
        assert_eq!(nav.current(), Some("a.tmt"));
    }
}
