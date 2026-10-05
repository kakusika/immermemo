//! Wires up TLS certificate verification for sync on Android.
//!
//! Re-exports the shared mobile certificate verifier backed by the
//! vendored Mozilla root bundle.

pub use crate::cert::verifier;
