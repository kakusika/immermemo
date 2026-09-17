# Immermemo

## What this is

A mobile note app that writes Tomet (`.tmt`) notes and syncs them over
git, with the git repository kept out of the folder the user's files are
visible in. Read `README.md` first for the shape of the project, then
each crate's own `//!` module doc for why it's built the way it is --
that doc is the design record; do not duplicate it into a separate
architecture file that will drift from the code.

## Relationship to `tomet`

The sibling `../tomet` repository is a dependency (via the git
dependencies in the workspace `Cargo.toml`), never a target. Nothing in
this repository edits `tomet`'s source. A gap in what `tomet` exposes
(the AST, the vocabulary resolver, `tomet-load`) is a feature request
against that repository, filed and waited on -- not worked around here.

`vocab/mobile.vocabulary.tmt` is this repository's own vocabulary,
declared the way any `tomet` vault declares one (see `tomet`'s
`docs/spec/vocabulary.tmt`). It does not change what `tomet` itself
accepts.

## Language

Write all code comments and documentation in English. Do not use
Japanese in code. (Design discussion in chat/PRs can be whatever
language the thinking happened in.)

## Verifying changes

```bash
cargo build
cargo test --workspace
```

There is no GUI to click through in `core/`; verify the merge and sync
logic with unit tests against real `.tmt` fixtures, not by hand-tracing.
Once `apps/ios` and `apps/android` exist, their own verification story
belongs in their own directories, not here.

## Task tracking

Before starting implementation on any non-trivial task, create a file
under `.agents/tasks/` (one per task) breaking the work into discrete
steps, and keep it updated as steps complete -- accurate enough that the
work can be resumed from that file alone. Fold anything worth keeping
into a doc comment or commit message when the task finishes, then delete
the file.

## Commits

Do not put `Claude-Session:` or `Co-Authored-By: Claude` trailers in
commit messages.

Never push to a remote without being asked.
