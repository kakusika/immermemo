# Immermemo

A mobile note app for writing [Tomet](https://github.com/tomet-lang/tomet)
(`.tmt`) notes, synced over git without git ever touching the folder the
user sees.

## Why this exists

Git sync on mobile runs into two problems: cloud file providers
(iCloud Drive, Google Drive) don't get along with `.git`'s pile of small
files, and a raw text merge on conflict forces the user into a resolution
UI no phone screen has room for.

Immermemo's answer to both: the git repository lives in the app's private
storage, entirely separate from the folder the user's notes are visible
in (`core/sync`); and conflicts are resolved at the AST level, not the
text level, so most concurrent edits never conflict at all -- the ones
that do get written back into the note itself as a `@mobile.conflict`
element instead of a duplicated file or a merge-marker mess
(`core/merge`, `vocab/mobile.vocabulary.tmt`).

## Layout

```
immermemo/
├── core/
│   ├── sync/    # git2-backed vault sync -- see its module doc
│   └── merge/   # AST-level three-way merge -- see its module doc
├── vocab/
│   └── mobile.vocabulary.tmt   # the @mobile.conflict vocabulary
├── bindings/mobile/            # UniFFI: core/* -> Kotlin + Swift
└── apps/
    ├── ios/
    └── android/
```

`tomet` itself is never modified by this project -- it's consumed as an
ordinary git dependency (see the workspace `Cargo.toml`). This repository
is independent of the `tomet` repository.

## Finding your way around

What a crate is and why it exists: that crate's `//!` module doc
(`core/sync/src/lib.rs`, `core/merge/src/lib.rs`). What the conflict
vocabulary means: `vocab/mobile.vocabulary.tmt`, written against
`tomet`'s own `docs/spec/vocabulary.tmt`.

## Building

```bash
cargo build
cargo test --workspace
```

`core/sync` and `core/merge` are currently scaffolded with their full
public API and module-level design docs; the bodies are `todo!()` pending
implementation.
