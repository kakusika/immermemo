//! Persistence and configuration parsing for bottom bar actions and quick actions.

use std::collections::HashSet;

use super::id::ActionId;
use super::registry::get_action_meta;

/// Configuration for bottom bar actions, defining which actions are enabled and their display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BottomBarConfig {
    pub enabled: HashSet<ActionId>,
    pub order: Vec<ActionId>,
}

impl Default for BottomBarConfig {
    fn default() -> Self {
        Self::parse("")
    }
}

impl BottomBarConfig {
    /// Parses a comma-separated action configuration string.
    ///
    /// Handles legacy "quick" composite action, unknown actions, and empty configs gracefully.
    pub fn parse(config: &str) -> Self {
        let raw_items: Vec<&str> = config
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        let is_empty_config = raw_items.is_empty();
        let set: HashSet<&str> = raw_items.iter().copied().collect();
        let has_legacy_quick = set.contains("quick");

        let mut enabled = HashSet::new();
        for action in ActionId::ALL {
            let is_on = if is_empty_config {
                get_action_meta(action).default_enabled
            } else if has_legacy_quick
                && matches!(
                    action,
                    ActionId::NewNote | ActionId::Sync | ActionId::SwitchVault | ActionId::Settings
                )
            {
                true
            } else {
                set.contains(action.as_str())
            };
            if is_on {
                enabled.insert(action);
            }
        }

        let mut order: Vec<ActionId> = Vec::new();
        for key in raw_items {
            if key == "quick" {
                for legacy_action in [
                    ActionId::NewNote,
                    ActionId::Sync,
                    ActionId::SwitchVault,
                    ActionId::Settings,
                ] {
                    if !order.contains(&legacy_action) {
                        order.push(legacy_action);
                    }
                }
            } else if let Some(action) = ActionId::from_key(key) {
                if !order.contains(&action) {
                    order.push(action);
                }
            }
        }
        for default_action in ActionId::ALL {
            if !order.contains(&default_action) {
                order.push(default_action);
            }
        }

        Self { enabled, order }
    }

    /// Checks if a given action is enabled.
    pub fn is_enabled(&self, action: ActionId) -> bool {
        self.enabled.contains(&action)
    }

    /// Sets whether an action is enabled.
    pub fn set_enabled(&mut self, action: ActionId, enabled: bool) {
        if enabled {
            self.enabled.insert(action);
        } else {
            self.enabled.remove(&action);
        }
    }

    /// Serializes the currently enabled and ordered actions into a comma-separated string.
    pub fn serialize(&self) -> String {
        let mut items = Vec::new();
        for action in &self.order {
            if self.is_enabled(*action) {
                items.push(action.as_str());
            }
        }

        if items.is_empty() {
            items.push(ActionId::Home.as_str());
        }

        items.join(",")
    }
}

/// Default order of quick actions on the home page (excluding Home itself).
pub const DEFAULT_HOME_ACTIONS: [ActionId; 11] = [
    ActionId::NewNote,
    ActionId::Search,
    ActionId::Daily,
    ActionId::Directory,
    ActionId::Starred,
    ActionId::Recent,
    ActionId::Graph,
    ActionId::Library,
    ActionId::Sync,
    ActionId::SwitchVault,
    ActionId::Settings,
];

/// Configuration for Home page quick actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeQuickActionsConfig {
    pub enabled: HashSet<ActionId>,
    pub order: Vec<ActionId>,
}

impl Default for HomeQuickActionsConfig {
    fn default() -> Self {
        Self::parse("")
    }
}

impl HomeQuickActionsConfig {
    /// Parses a comma-separated action configuration string for Home quick actions.
    pub fn parse(config: &str) -> Self {
        let raw_items: Vec<&str> = config
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        let is_empty_config = raw_items.is_empty();
        let set: HashSet<&str> = raw_items.iter().copied().collect();

        let mut enabled = HashSet::new();
        for action in DEFAULT_HOME_ACTIONS {
            let is_on = if is_empty_config {
                action == ActionId::NewNote
            } else {
                set.contains(action.as_str())
            };
            if is_on {
                enabled.insert(action);
            }
        }

        let mut order: Vec<ActionId> = Vec::new();
        for key in raw_items {
            if let Some(action) = ActionId::from_key(key)
                && action != ActionId::Home
                && !order.contains(&action)
            {
                order.push(action);
            }
        }
        for default_action in DEFAULT_HOME_ACTIONS {
            if !order.contains(&default_action) {
                order.push(default_action);
            }
        }

        Self { enabled, order }
    }

    /// Checks if a quick action is enabled on Home.
    pub fn is_enabled(&self, action: ActionId) -> bool {
        if action == ActionId::Home {
            return false;
        }
        self.enabled.contains(&action)
    }

    /// Sets whether an action is enabled on Home.
    pub fn set_enabled(&mut self, action: ActionId, enabled: bool) {
        if action == ActionId::Home {
            return;
        }
        if enabled {
            self.enabled.insert(action);
        } else {
            self.enabled.remove(&action);
        }
    }

    /// Returns the enabled actions in their configured order.
    pub fn enabled_actions(&self) -> Vec<ActionId> {
        self.order
            .iter()
            .copied()
            .filter(|action| self.is_enabled(*action))
            .collect()
    }

    /// Chunks enabled actions for row layouts (e.g., 3 per row), returning (items, empty_slots).
    pub fn chunked_enabled_actions(&self, chunk_size: usize) -> Vec<(Vec<ActionId>, usize)> {
        let enabled = self.enabled_actions();
        let mut rows = Vec::new();
        for chunk in enabled.chunks(chunk_size) {
            let empty_slots = chunk_size - chunk.len();
            rows.push((chunk.to_vec(), empty_slots));
        }
        rows
    }

    /// Serializes the currently enabled and ordered quick actions into a comma-separated string.
    pub fn serialize(&self) -> String {
        let items: Vec<&'static str> = self
            .enabled_actions()
            .into_iter()
            .map(|a| a.as_str())
            .collect();
        items.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bottom_bar_config_defaults() {
        let config = BottomBarConfig::parse("");
        assert!(config.is_enabled(ActionId::Home));
        assert!(config.is_enabled(ActionId::Search));
        assert!(config.is_enabled(ActionId::NewNote));
        assert!(!config.is_enabled(ActionId::Directory));
    }

    #[test]
    fn test_bottom_bar_legacy_quick_expansion() {
        let config = BottomBarConfig::parse("home,quick");
        assert!(config.is_enabled(ActionId::Home));
        assert!(config.is_enabled(ActionId::NewNote));
        assert!(config.is_enabled(ActionId::Sync));
        assert!(config.is_enabled(ActionId::SwitchVault));
        assert!(config.is_enabled(ActionId::Settings));
    }

    #[test]
    fn test_bottom_bar_serialization_roundtrip() {
        let mut config = BottomBarConfig::parse("home,search,library");
        config.set_enabled(ActionId::Search, false);
        config.set_enabled(ActionId::Settings, true);
        let serialized = config.serialize();
        let reloaded = BottomBarConfig::parse(&serialized);
        assert_eq!(config.enabled, reloaded.enabled);
    }

    #[test]
    fn test_home_quick_actions_config() {
        let config = HomeQuickActionsConfig::parse("new_note,sync");
        assert!(config.is_enabled(ActionId::NewNote));
        assert!(config.is_enabled(ActionId::Sync));
        assert!(!config.is_enabled(ActionId::Search));
        assert_eq!(config.enabled_actions(), vec![ActionId::NewNote, ActionId::Sync]);

        let chunks = config.chunked_enabled_actions(3);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0, vec![ActionId::NewNote, ActionId::Sync]);
        assert_eq!(chunks[0].1, 1); // 1 empty slot
    }
}
