fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:UI_DIR={manifest_dir}/ui");
    println!("cargo:rerun-if-changed=ui");
}
