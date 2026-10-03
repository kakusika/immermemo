//! Generic, app-agnostic mobile Slint widgets (`ui/tokens.slint`,
//! `ui/icons.slint`, `ui/components.slint`), kept independent of
//! `apps/slint`'s own domain-specific components (note cards, the bottom
//! nav bar, the editor's rendered-block view).
//!
//! This crate is a temporary home. It is deliberately laid out and named
//! the same way `origami-frameworks`' own `origami` crate is (same
//! `links` + `cargo:UI_DIR=...` build-script convention, same `ui/`
//! file split) so that once that project's `origami` crate is mature
//! enough to depend on directly, this crate's `ui/` contents can move
//! there with no restructuring -- see `apps/slint/build.rs`'s
//! `with_library_paths` wiring for the `@origami-mobile/...` import
//! prefix this crate provides today.
