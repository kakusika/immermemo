//! Global Action System.
//!
//! Provides first-class, dispatchable actions unifying page navigation,
//! modal sheets, and instant operations (such as New Note and Sync).

pub mod config;
pub mod dispatch;
pub mod id;
pub mod registry;

pub use config::{
    apply_bottom_bar_config, is_action_enabled, serialize_bottom_bar_config, set_action_enabled,
};
pub use dispatch::dispatch_action;
pub use id::{ActionId, ActionKind};
pub use registry::{ActionMeta, get_action_meta};
