//! UI presentation and view models (tabs, sheets, directories, search, icon picker).

use origiri_icons::icon_names;

/// Active top-level navigation tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum ShellTab {
    #[default]
    Home = 0,
    Search = 1,
    Files = 2,
}

impl ShellTab {
    pub fn from_i32(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Home),
            1 => Some(Self::Search),
            2 => Some(Self::Files),
            _ => None,
        }
    }

    pub const fn to_i32(self) -> i32 {
        self as i32
    }
}

/// Active sub-page within the Files/Library screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum FilesPage {
    #[default]
    Library = 0,
    Directory = 1,
    Daily = 2,
    Starred = 3,
    Recent = 4,
    Graph = 5,
}

impl FilesPage {
    pub fn from_i32(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Library),
            1 => Some(Self::Directory),
            2 => Some(Self::Daily),
            3 => Some(Self::Starred),
            4 => Some(Self::Recent),
            5 => Some(Self::Graph),
            _ => None,
        }
    }

    pub const fn to_i32(self) -> i32 {
        self as i32
    }
}

/// Directory browsing display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DirectoryViewMode {
    #[default]
    List,
    Grid,
}

/// Currently active slide-up or modal sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellSheet {
    #[default]
    None,
    Vault,
    VaultSetup,
    Settings,
    Conflict,
    History,
    VaultHistory,
    IconPicker,
}

/// Status and toast notification state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusState {
    pub message: String,
    pub is_error: bool,
    pub sync_error: Option<String>,
}

impl StatusState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.message = msg.into();
        self.is_error = false;
    }

    pub fn set_error(&mut self, err: impl Into<String>) {
        self.message = err.into();
        self.is_error = true;
    }

    pub fn clear(&mut self) {
        self.message.clear();
        self.is_error = false;
    }
}

/// Search filter and input focus state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchState {
    pub query: String,
    pub is_focused: bool,
}

/// Icon picker filter and target note state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IconPickerState {
    pub target_note_index: Option<usize>,
    pub query: String,
    pub is_open: bool,
}

impl IconPickerState {
    pub const DEFAULT_PKG: &'static str = "tabler";
    pub const MAX_RESULTS: usize = 60;

    /// Filters icon names by the current query substring (case-insensitive).
    pub fn filter_icons(&self, max_results: usize) -> Vec<&'static str> {
        let query = self.query.to_lowercase();
        icon_names(Self::DEFAULT_PKG)
            .iter()
            .copied()
            .filter(|name| name.contains(&query))
            .take(max_results)
            .collect()
    }

    pub fn open(&mut self, target_note_index: usize) {
        self.target_note_index = Some(target_note_index);
        self.query.clear();
        self.is_open = true;
    }

    pub fn close(&mut self) {
        self.target_note_index = None;
        self.query.clear();
        self.is_open = false;
    }
}

/// Directory viewing state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirectoryViewState {
    pub current_folder: String,
    pub view_mode: DirectoryViewMode,
    pub is_dirty: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_tab_conversions() {
        assert_eq!(ShellTab::from_i32(0), Some(ShellTab::Home));
        assert_eq!(ShellTab::from_i32(1), Some(ShellTab::Search));
        assert_eq!(ShellTab::from_i32(2), Some(ShellTab::Files));
        assert_eq!(ShellTab::from_i32(99), None);
        assert_eq!(ShellTab::Home.to_i32(), 0);
    }

    #[test]
    fn test_files_page_conversions() {
        assert_eq!(FilesPage::from_i32(0), Some(FilesPage::Library));
        assert_eq!(FilesPage::from_i32(1), Some(FilesPage::Directory));
        assert_eq!(FilesPage::from_i32(5), Some(FilesPage::Graph));
        assert_eq!(FilesPage::from_i32(6), None);
    }

    #[test]
    fn test_status_state() {
        let mut status = StatusState::new();
        status.set_status("All good");
        assert_eq!(status.message, "All good");
        assert!(!status.is_error);

        status.set_error("Failed to load");
        assert_eq!(status.message, "Failed to load");
        assert!(status.is_error);

        status.clear();
        assert_eq!(status.message, "");
        assert!(!status.is_error);
    }

    #[test]
    fn test_icon_picker_filter() {
        let mut picker = IconPickerState::default();
        picker.open(3);
        assert_eq!(picker.target_note_index, Some(3));
        assert!(picker.is_open);

        picker.query = "star".to_string();
        let matches = picker.filter_icons(10);
        assert!(!matches.is_empty());
        for name in &matches {
            assert!(name.contains("star"));
        }

        picker.close();
        assert_eq!(picker.target_note_index, None);
        assert!(!picker.is_open);
    }
}
