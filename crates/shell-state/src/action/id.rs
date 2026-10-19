//! Action identifier and kind definitions.

/// Identifies a globally dispatchable action in the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ActionId {
    Home = 0,
    Search = 1,
    Library = 2,
    Directory = 3,
    Daily = 4,
    Starred = 5,
    Recent = 6,
    Graph = 7,
    NewNote = 8,
    Sync = 9,
    SwitchVault = 10,
    Settings = 11,
}

impl ActionId {
    /// All defined actions in canonical default order.
    pub const ALL: [ActionId; 12] = [
        ActionId::Home,
        ActionId::Search,
        ActionId::NewNote,
        ActionId::Library,
        ActionId::Sync,
        ActionId::SwitchVault,
        ActionId::Settings,
        ActionId::Directory,
        ActionId::Daily,
        ActionId::Starred,
        ActionId::Recent,
        ActionId::Graph,
    ];

    /// Returns the persistent string key for this action.
    pub const fn as_str(&self) -> &'static str {
        match self {
            ActionId::Home => "home",
            ActionId::Search => "search",
            ActionId::Library => "library",
            ActionId::Directory => "directory",
            ActionId::Daily => "daily",
            ActionId::Starred => "starred",
            ActionId::Recent => "recent",
            ActionId::Graph => "graph",
            ActionId::NewNote => "new_note",
            ActionId::Sync => "sync",
            ActionId::SwitchVault => "switch_vault",
            ActionId::Settings => "settings",
        }
    }

    /// Parses an action from its persistent string key.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "home" => Some(ActionId::Home),
            "search" => Some(ActionId::Search),
            "library" => Some(ActionId::Library),
            "directory" => Some(ActionId::Directory),
            "daily" => Some(ActionId::Daily),
            "starred" => Some(ActionId::Starred),
            "recent" => Some(ActionId::Recent),
            "graph" => Some(ActionId::Graph),
            "new_note" => Some(ActionId::NewNote),
            "sync" => Some(ActionId::Sync),
            "switch_vault" => Some(ActionId::SwitchVault),
            "settings" => Some(ActionId::Settings),
            _ => None,
        }
    }

    /// Converts from an integer ID used in Slint.
    pub fn from_i32(val: i32) -> Option<Self> {
        match val {
            0 => Some(ActionId::Home),
            1 => Some(ActionId::Search),
            2 => Some(ActionId::Library),
            3 => Some(ActionId::Directory),
            4 => Some(ActionId::Daily),
            5 => Some(ActionId::Starred),
            6 => Some(ActionId::Recent),
            7 => Some(ActionId::Graph),
            8 => Some(ActionId::NewNote),
            9 => Some(ActionId::Sync),
            10 => Some(ActionId::SwitchVault),
            11 => Some(ActionId::Settings),
            _ => None,
        }
    }

    /// Returns the integer ID for Slint.
    pub const fn to_i32(self) -> i32 {
        self as i32
    }

    /// Returns the behavioral kind of this action.
    pub const fn kind(&self) -> ActionKind {
        match self {
            ActionId::Home
            | ActionId::Search
            | ActionId::Library
            | ActionId::Directory
            | ActionId::Daily
            | ActionId::Starred
            | ActionId::Recent
            | ActionId::Graph => ActionKind::Navigation,
            ActionId::SwitchVault | ActionId::Settings => ActionKind::Sheet,
            ActionId::NewNote | ActionId::Sync => ActionKind::Execute,
        }
    }
}

/// The behavioral category of an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// Switches the main viewport to a destination page. Has active state.
    Navigation,
    /// Opens a slide-up or modal sheet.
    Sheet,
    /// Instantly executes an operation without retaining persistent navigation state.
    Execute,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_id_roundtrips() {
        assert_eq!(ActionId::ALL.len(), 12);
        for action in ActionId::ALL {
            let key = action.as_str();
            assert_eq!(ActionId::from_key(key), Some(action));
            let i = action.to_i32();
            assert_eq!(ActionId::from_i32(i), Some(action));
        }
    }

    #[test]
    fn test_action_kinds() {
        assert_eq!(ActionId::Home.kind(), ActionKind::Navigation);
        assert_eq!(ActionId::Search.kind(), ActionKind::Navigation);
        assert_eq!(ActionId::Library.kind(), ActionKind::Navigation);
        assert_eq!(ActionId::NewNote.kind(), ActionKind::Execute);
        assert_eq!(ActionId::Sync.kind(), ActionKind::Execute);
        assert_eq!(ActionId::SwitchVault.kind(), ActionKind::Sheet);
        assert_eq!(ActionId::Settings.kind(), ActionKind::Sheet);
    }
}
