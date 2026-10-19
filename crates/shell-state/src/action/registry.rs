//! Action metadata and registry.

use super::id::{ActionId, ActionKind};

/// Metadata defining a global action.
#[derive(Debug, Clone)]
pub struct ActionMeta {
    pub id: ActionId,
    pub title: &'static str,
    pub kind: ActionKind,
    pub default_enabled: bool,
}

impl ActionMeta {
    pub const fn new(id: ActionId, title: &'static str, default_enabled: bool) -> Self {
        Self {
            kind: id.kind(),
            id,
            title,
            default_enabled,
        }
    }
}

/// Returns the static metadata for a given action ID.
pub fn get_action_meta(id: ActionId) -> ActionMeta {
    match id {
        ActionId::Home => ActionMeta::new(id, "Home", true),
        ActionId::Search => ActionMeta::new(id, "Search", true),
        ActionId::NewNote => ActionMeta::new(id, "New Note", true),
        ActionId::Library => ActionMeta::new(id, "Library", true),
        ActionId::Sync => ActionMeta::new(id, "Sync", true),
        ActionId::SwitchVault => ActionMeta::new(id, "Switch Vault", true),
        ActionId::Settings => ActionMeta::new(id, "Settings", true),
        ActionId::Directory => ActionMeta::new(id, "Directory", false),
        ActionId::Daily => ActionMeta::new(id, "Daily Notes", false),
        ActionId::Starred => ActionMeta::new(id, "Starred", false),
        ActionId::Recent => ActionMeta::new(id, "Recent", false),
        ActionId::Graph => ActionMeta::new(id, "Graph View", false),
    }
}
