//! Global Action System.
//!
//! Provides first-class, dispatchable actions unifying page navigation,
//! modal sheets, and instant operations (such as New Note and Sync).

pub mod config;
pub mod dispatch;
pub mod id;
pub mod registry;

pub use config::{
    apply_bottom_bar_config, apply_home_actions_config, is_action_enabled, is_home_action_enabled,
    serialize_bottom_bar_config, serialize_home_actions_config, set_action_enabled,
    set_home_action_enabled, update_home_enabled_actions,
};
pub use dispatch::dispatch_action;
pub use id::{ActionId, ActionKind};
pub use registry::{ActionMeta, get_action_meta};
