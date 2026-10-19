//! Global action definitions, metadata, and configuration persistence models.

pub mod config;
pub mod id;
pub mod registry;

pub use config::{
    BottomBarConfig, DEFAULT_HOME_ACTIONS, HomeQuickActionsConfig,
};
pub use id::{ActionId, ActionKind};
pub use registry::{ActionMeta, get_action_meta};
