fn main() {
    let is_android = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android");
    let default_style = if is_android { "material" } else { "fluent" };
    let style = std::env::var("SLINT_STYLE").unwrap_or_else(|_| default_style.to_string());

    let config = slint_build::CompilerConfiguration::new().with_style(style);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
