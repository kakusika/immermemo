fn main() {
    // `@richtext/flow_view.slint` uses `ComponentContainer`/`component-factory`
    // to embed a real widget per inline element. Slint 1.18 keeps both out
    // of its normal compiler path (`i-slint-compiler`'s
    // `TypeRegister::builtin()` explicitly removes them; only
    // `builtin_experimental()` -- "Do not use in production code!", its own
    // doc comment says -- keeps them), gated behind this env var. The Rust
    // side's `ComponentFactory` is similarly `#[deprecated(note =
    // "Experimental type was made public by mistake")]` and `#[doc(hidden)]`
    // in the `slint` crate. Known, accepted risk -- see
    // `.agents/tasks/richtext-flow-inline-conflict.md`.
    // SAFETY: single-threaded build script, set before any other code reads
    // the environment.
    unsafe {
        std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1");
    }

    let is_android = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android");
    let default_style = if is_android { "material" } else { "fluent" };
    let style = std::env::var("SLINT_STYLE").unwrap_or_else(|_| default_style.to_string());

    // `origami-mobile` (from `origami-frameworks`, migrated out of this
    // repo's own `crates/origami-mobile`) reports its own `ui/` directory
    // through this `links`-propagated env var (see its `build.rs`); this is
    // what `@origami-mobile/...` resolves against below. `editor.slint`/
    // `rendered_block.slint` used to come from a separate `crates/
    // editor-slint` the same way, but that crate had no other consumer and
    // no Rust code of its own, so its `ui/` moved into this crate's own
    // `ui/screens/` directly (see
    // `.agents/tasks/fold-editor-slint-into-apps-slint.md`).
    let origami_mobile_ui_dir = std::env::var("DEP_ORIGAMI_MOBILE_UI_DIR").unwrap();
    // `origami-mobile`'s own `.slint` now unconditionally imports
    // `@origami-icons` (the icon set it shares with `origami-frameworks`'s
    // desktop `origami` crate), so this app needs it wired too whenever it
    // needs `origami-mobile`.
    let origami_icons_ui_dir = std::env::var("DEP_ORIGAMI_ICONS_UI_DIR").unwrap();
    // `origami-richtext` (from `origami-frameworks`) reports its own `ui/`
    // the same way; this is what `@richtext/...` resolves against below.
    let richtext_ui_dir = std::env::var("DEP_ORIGAMI_RICHTEXT_UI_DIR").unwrap();
    let mut library_paths = std::collections::HashMap::new();
    library_paths.insert(
        "origami-mobile".to_string(),
        std::path::PathBuf::from(origami_mobile_ui_dir),
    );
    library_paths.insert(
        "origami-icons".to_string(),
        std::path::PathBuf::from(origami_icons_ui_dir),
    );
    library_paths.insert(
        "richtext".to_string(),
        std::path::PathBuf::from(richtext_ui_dir),
    );

    let config = slint_build::CompilerConfiguration::new()
        .with_style(style)
        .with_bundled_translations("translations")
        .with_library_paths(library_paths);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
