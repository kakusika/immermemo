//! Prints what `merge`/`resolve` actually produce, as real `.tmt` text,
//! for two scenarios: an edit to two different notes' content that
//! merges silently, and a real same-sentence conflict that gets resolved.
//!
//! Run with: `cargo run -p immermemo-merge --example demo`

use immermemo_merge::{ConflictResolution, merge, resolve};
use tomet_parser::parse_document;
use tomet_printer::document_to_tm;

fn doc(src: &str) -> tomet_ast::Document {
    parse_document(src).unwrap()
}

fn section(title: &str) {
    println!("\n== {title} ==");
}

fn main() {
    section("Scenario 1: two phones edit different parts of the same note");

    let base = doc("会議は10時から。\n\n持ち物：ノートPC。\n");
    let phone_a = doc("会議は11時から。\n\n持ち物：ノートPC。\n");
    let phone_b = doc("会議は10時から。\n\n持ち物：ノートPCと資料。\n");

    println!(
        "--- base (last synced version) ---\n{}",
        document_to_tm(&base)
    );
    println!("--- phone A wrote ---\n{}", document_to_tm(&phone_a));
    println!("--- phone B wrote ---\n{}", document_to_tm(&phone_b));

    let result = merge(&base, &phone_a, &phone_b);
    println!("clean = {}", result.clean);
    println!(
        "--- merged automatically, no user action needed ---\n{}",
        document_to_tm(&result.document)
    );

    section("Scenario 2: both phones edit the same sentence");

    let base = doc("持ち物：ノートPC。\n");
    let phone_a = doc("持ち物：ノートPCと充電器。\n");
    let phone_b = doc("持ち物：ノートPCと資料。\n");

    println!("--- base ---\n{}", document_to_tm(&base));
    println!("--- phone A wrote ---\n{}", document_to_tm(&phone_a));
    println!("--- phone B wrote ---\n{}", document_to_tm(&phone_b));

    let result = merge(&base, &phone_a, &phone_b);
    println!("clean = {}", result.clean);
    println!(
        "--- merged: the shared part merged, only the disputed part is marked ---\n{}",
        document_to_tm(&result.document)
    );

    let resolved = resolve(&result.document, &[ConflictResolution::Theirs]);
    println!(
        "--- after the user picks phone B's version ---\n{}",
        document_to_tm(&resolved)
    );
}
