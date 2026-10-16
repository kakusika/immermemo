//! Prints this tree's build label: `cargo run --release -p immermemo-buildinfo
//! --example label` (or `just build-id`), to compare with what the app shows.
fn main() {
    println!("{}", immermemo_buildinfo::label());
}
