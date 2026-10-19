//! Shell state, action management, and UI presentation controller logic for immermemo.
//!
//! Independent of Slint or GUI frameworks, enabling headless testing and fast compilation.

pub mod action;
pub mod session;
pub mod view_state;

pub use action::*;
pub use session::ShellSession;
pub use view_state::*;
