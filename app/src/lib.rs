//! Application binary and platform entry points for immermemo.

pub mod cli;
#[cfg(target_os = "android")]
pub mod android;

pub use immermemo_shell::*;
