//! Persistence and configuration parsing for bottom bar actions.

use std::collections::HashSet;
use std::rc::Rc;
use slint::{Model, VecModel};

use crate::SettingsAdapter;
use super::id::ActionId;
use super::registry::get_action_meta;

/// Applies a comma-separated action configuration string to the `SettingsAdapter`.
///
/// Missing or unrecognized action keys are handled gracefully. If `config` is
/// empty or yields no valid actions, defaults from `ActionId::ALL` are applied.
pub fn apply_bottom_bar_config(settings: &SettingsAdapter, config: &str) {
    let raw_items: Vec<&str> = config
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let is_empty_config = raw_items.is_empty();
    let set: HashSet<&str> = raw_items.iter().copied().collect();
    let has_legacy_quick = set.contains("quick");

    // Set visibility per action
    for action in ActionId::ALL {
        let enabled = if is_empty_config {
            get_action_meta(action).default_enabled
        } else if has_legacy_quick
            && matches!(
                action,
                ActionId::NewNote
                    | ActionId::Sync
                    | ActionId::SwitchVault
                    | ActionId::Settings
            )
        {
            true
        } else {
            set.contains(action.as_str())
        };
        set_action_enabled(settings, action, enabled);
    }

    // Determine ordering
    let mut order: Vec<i32> = Vec::new();
    for key in raw_items {
        if key == "quick" {
            // Expand legacy "quick" item into its underlying actions
            for legacy_action in [
                ActionId::NewNote,
                ActionId::Sync,
                ActionId::SwitchVault,
                ActionId::Settings,
            ] {
                let id = legacy_action.to_i32();
                if !order.contains(&id) {
                    order.push(id);
                }
            }
        } else if let Some(action) = ActionId::from_key(key) {
            let id = action.to_i32();
            if !order.contains(&id) {
                order.push(id);
            }
        }
    }
    for default_action in ActionId::ALL {
        let id = default_action.to_i32();
        if !order.contains(&id) {
            order.push(id);
        }
    }

    settings.set_nav_order(Rc::new(VecModel::from(order)).into());
}

/// Serializes the currently enabled and ordered actions into a comma-separated string.
pub fn serialize_bottom_bar_config(settings: &SettingsAdapter) -> String {
    let order: Vec<i32> = settings.get_nav_order().iter().collect();
    let mut items = Vec::new();

    for id_val in order {
        if let Some(action) = ActionId::from_i32(id_val) {
            if is_action_enabled(settings, action) {
                items.push(action.as_str());
            }
        }
    }

    if items.is_empty() {
        items.push(ActionId::Home.as_str());
        set_action_enabled(settings, ActionId::Home, true);
    }

    items.join(",")
}

/// Checks whether an action is enabled in `SettingsAdapter`.
pub fn is_action_enabled(settings: &SettingsAdapter, action: ActionId) -> bool {
    match action {
        ActionId::Home => settings.get_nav_show_home(),
        ActionId::Search => settings.get_nav_show_search(),
        ActionId::Library => settings.get_nav_show_library(),
        ActionId::Directory => settings.get_nav_show_directory(),
        ActionId::Daily => settings.get_nav_show_daily(),
        ActionId::Starred => settings.get_nav_show_starred(),
        ActionId::Recent => settings.get_nav_show_recent(),
        ActionId::Graph => settings.get_nav_show_graph(),
        ActionId::NewNote => settings.get_nav_show_new_note(),
        ActionId::Sync => settings.get_nav_show_sync(),
        ActionId::SwitchVault => settings.get_nav_show_switch_vault(),
        ActionId::Settings => settings.get_nav_show_settings(),
    }
}

/// Sets whether an action is enabled in `SettingsAdapter`.
pub fn set_action_enabled(settings: &SettingsAdapter, action: ActionId, enabled: bool) {
    match action {
        ActionId::Home => settings.set_nav_show_home(enabled),
        ActionId::Search => settings.set_nav_show_search(enabled),
        ActionId::Library => settings.set_nav_show_library(enabled),
        ActionId::Directory => settings.set_nav_show_directory(enabled),
        ActionId::Daily => settings.set_nav_show_daily(enabled),
        ActionId::Starred => settings.set_nav_show_starred(enabled),
        ActionId::Recent => settings.set_nav_show_recent(enabled),
        ActionId::Graph => settings.set_nav_show_graph(enabled),
        ActionId::NewNote => settings.set_nav_show_new_note(enabled),
        ActionId::Sync => settings.set_nav_show_sync(enabled),
        ActionId::SwitchVault => settings.set_nav_show_switch_vault(enabled),
        ActionId::Settings => settings.set_nav_show_settings(enabled),
    }
}
