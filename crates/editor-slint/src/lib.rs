//! The editor's Slint UI (`ui/editor.slint`'s `EditorScreen`, `ui/rendered_block.slint`'s
//! `RenderedBlock`/`RenderedBlockView`) -- source only, same as `crates/origami-mobile`.
//!
//! Like that crate, this one declares no `slint` dependency and does no
//! compilation of its own: `build.rs` only reports this crate's `ui/`
//! directory (via the `links` mechanism), for a consumer's own
//! `slint_build` call to register as the `@editor-slint/...` import
//! prefix. The generated Rust types (`EditorScreen`, `RenderedBlock`,
//! `RenderedBlockKind`) only exist inside whichever crate actually runs
//! `slint::include_modules!()` over a tree that imports this source --
//! today that's `apps/slint` alone (`EditorScreen` is composed inside
//! `app.slint`'s own window, not run standalone), so the Rust-side
//! `ClassifiedBlock` -> `RenderedBlock` adapter stays there too (see
//! `apps/slint/src/render.rs`), not here.
//!
//! This is a temporary home: the plan is to move `ui/` into
//! `origami-frameworks/origami-kit/views/tomet-editor` once that project
//! is ready for it, the same way `crates/origami-mobile` is headed for
//! `origami-frameworks/origami` -- see that crate's own doc comment.
