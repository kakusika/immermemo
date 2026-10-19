//! Headless UI session combining core domain state with presentation state.

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;

use immermemo_core::ConflictResolution;
use immermemo_core::editor::{ConflictResolved, OpenedNote};
use immermemo_core::session::{FilteredListOutcome, RefreshListOutcome};
use origiri_selection::Selection;

use crate::view_state::{
    DirectoryViewState, FilesPage, IconPickerState, SearchState, ShellSheet, ShellTab, StatusState,
};

/// Combined application and presentation session state.
pub struct ShellSession {
    pub core: immermemo_core::Session,
    pub selection: Selection,
    pub directory_view: DirectoryViewState,
    pub search: SearchState,
    pub icon_picker: IconPickerState,
    pub status: StatusState,
    pub active_tab: ShellTab,
    pub files_page: FilesPage,
    pub active_sheet: ShellSheet,
}

impl Deref for ShellSession {
    type Target = immermemo_core::Session;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for ShellSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl ShellSession {
    /// Creates a new `ShellSession` wrapping the given domain `Session`.
    pub fn new(core: immermemo_core::Session) -> Self {
        Self {
            core,
            selection: Selection::default(),
            directory_view: DirectoryViewState {
                current_folder: String::new(),
                view_mode: Default::default(),
                is_dirty: true,
            },
            search: SearchState::default(),
            icon_picker: IconPickerState::default(),
            status: StatusState::default(),
            active_tab: ShellTab::Home,
            files_page: FilesPage::Library,
            active_sheet: ShellSheet::None,
        }
    }

    /// Returns whether any notes are currently selected.
    pub fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    /// Toggles a note's selection by path.
    pub fn toggle_selection(&mut self, path: &str) {
        self.selection.toggle(path);
    }

    /// Clears selection and selects only this note path.
    pub fn select_only(&mut self, path: &str) {
        self.selection.select_only(path);
    }

    /// Clears all selections.
    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Selects all notes in the given list, or toggles off if all are already selected.
    pub fn select_all_in_view(&mut self, paths: &[String]) {
        let all_selected = !paths.is_empty() && paths.iter().all(|p| self.selection.is_selected(p));
        if all_selected {
            self.selection.clear();
        } else {
            for p in paths {
                self.selection.toggle_on(p);
            }
        }
    }

    /// Opens the note at `index`, updating editor state and navigation history.
    pub fn open_note(&mut self, index: usize) -> Option<Result<OpenedNote, String>> {
        let outcome = self.core.open_note(index)?;
        if let Ok(ref opened) = outcome {
            self.core.note_nav.visit(opened.note_rel_path.clone());
        }
        Some(outcome)
    }

    /// Refreshes the vault note list and reconciles cached index state.
    pub fn refresh_list_data(&mut self) -> RefreshListOutcome {
        let outcome = self.core.refresh_list_data();
        self.directory_view.is_dirty = true;
        outcome
    }

    /// Loads the note list from index without disk scanning.
    pub fn load_cached_list_data(&mut self) -> RefreshListOutcome {
        let outcome = self.core.load_cached_list_data();
        self.directory_view.is_dirty = true;
        outcome
    }

    /// Updates filtered search list data based on current query.
    pub fn update_filtered_list_data(&mut self) -> FilteredListOutcome {
        let outcome = self.core.update_filtered_list_data();
        self.directory_view.is_dirty = true;
        outcome
    }

    /// Applies `@meta{ icon: @doc.icon(name, pkg) }` to the target note in `icon_picker`.
    pub fn apply_icon_selection(&mut self, icon_name: &str) -> anyhow::Result<Option<PathBuf>> {
        let Some(note_index) = self.icon_picker.target_note_index.take() else {
            return Ok(None);
        };
        let Some(path) = self.core.notes.get(note_index).cloned() else {
            return Ok(None);
        };
        let body = std::fs::read_to_string(&path)?;
        let Some(new_body) = immermemo_core::set_note_icon(&body, icon_name, IconPickerState::DEFAULT_PKG) else {
            anyhow::bail!("Could not update note icon");
        };
        std::fs::write(&path, &new_body)?;

        self.directory_view.is_dirty = true;
        Ok(Some(path))
    }

    /// Resolves the active conflict in the open note.
    pub fn resolve_active_conflict(
        &mut self,
        resolution: ConflictResolution,
    ) -> Option<Result<ConflictResolved, String>> {
        self.core.resolve_active_conflict(resolution)
    }
}
