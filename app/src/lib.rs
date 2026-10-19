//! Application binary and platform entry points for immermemo.

#[cfg(target_os = "android")]
pub mod android;
pub mod cli;

pub use immermemo_shell::*;
