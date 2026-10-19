//! Global Action System (Slint Shell Adapter).

pub mod config;
pub mod dispatch;

pub use immermemo_shell_state::action::*;

pub use config::{
    apply_bottom_bar_config, apply_home_actions_config, is_action_enabled, is_home_action_enabled,
    serialize_bottom_bar_config, serialize_home_actions_config, set_action_enabled,
    set_home_action_enabled, update_home_enabled_actions,
};
pub use dispatch::dispatch_action;
