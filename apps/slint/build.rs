fn main() {
    let is_android = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android");
    let default_style = if is_android { "material" } else { "fluent" };
    let style = std::env::var("SLINT_STYLE").unwrap_or_else(|_| default_style.to_string());

    // `crates/origami-mobile` and `crates/editor-slint` each report their
    // own `ui/` directory through this `links`-propagated env var (see
    // their `build.rs`); this is what `@origami-mobile/...` and
    // `@editor-slint/...` resolve against below.
    let origami_mobile_ui_dir = std::env::var("DEP_IMMERMEMO_ORIGAMI_MOBILE_UI_DIR").unwrap();
    let editor_slint_ui_dir = std::env::var("DEP_IMMERMEMO_EDITOR_SLINT_UI_DIR").unwrap();
    let mut library_paths = std::collections::HashMap::new();
    library_paths.insert(
        "origami-mobile".to_string(),
        std::path::PathBuf::from(origami_mobile_ui_dir),
    );
    library_paths.insert(
        "editor-slint".to_string(),
        std::path::PathBuf::from(editor_slint_ui_dir),
    );

    let config = slint_build::CompilerConfiguration::new()
        .with_style(style)
        .with_bundled_translations("translations")
        .with_library_paths(library_paths);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
