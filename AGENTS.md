# Immermemo

## What this is

A mobile note app that writes Tomet (`.tmt`) notes and syncs them over
git, with the git repository kept out of the folder the user's files are
visible in. Read `README.md` first for the shape of the project, then
`docs/design.md` for how the whole system fits together (the sync
protocol, what goes into git and what doesn't, the conflict pipeline).
Below that level, each crate's own `//!` module doc is the design record
for that crate's internals -- don't duplicate a crate's own design back
into `docs/design.md`, and don't let `docs/design.md`'s system-level
account drift out of sync with what the crates actually do.

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

Write all code comments in English. Do not use Japanese in code.

**Exception:** user-facing strings in `app/ui/*.slint` go through
Slint's i18n (`@tr("...")`, English as the source string, same as any
other code) and are translated in per-language catalogs under
`app/translations/<lang>/LC_MESSAGES/immermemo.po` (wired
up in `app/build.rs` via `with_bundled_translations`, selected at
startup in `app/src/lib.rs`'s `select_system_translation`). Those
`.po` files are expected to hold non-English text -- that's the whole
point of them -- and aren't covered by "no Japanese in code" above. When
adding or changing a `@tr(...)` string, regenerate the template with
`slint-tr-extractor` (`cargo install slint-tr-extractor`, then `find
app/ui -name '*.slint' | xargs slint-tr-extractor -o /tmp/x.pot`)
rather than hand-editing a `.po` file out of sync with the source.

`docs/design.md` is written in plain Japanese, deliberately -- it's the
account of the system that gets discussed and revised in conversation
with the app's author, in the language that conversation happens in, not
a spec for other engineers. Crate-level module docs stay English, same
as the code they sit beside.

## UI & Slint conventions

- **Vertical centering in `HorizontalLayout`**:
  Slint's `HorizontalLayout` has no cross-axis alignment (`align-items: center`).
  Children without explicit/fixed heights stretch across the full row height.
  `Text` elements vertically center their text glyphs within their stretched box
  (`vertical-alignment: center`), but fixed-height children (such as `Icon`, badges,
  or buttons) are positioned at `y = 0` (top edge). To vertically center any
  fixed-height element inside a `HorizontalLayout`, wrap it in:
  ```slint
  VerticalLayout {
      alignment: center;
      Icon { ... }
  }
  ```
- **Theme colors**:
  Never hardcode hex colors for surfaces, backgrounds, or borders. Use the Slint
  `Palette` tokens and derive contrast dynamically (e.g.
  `Palette.color-scheme == ColorScheme.dark ? Palette.background.darker(...) : ...`),
  respecting system light/dark modes and user device settings.


## Verifying changes

```bash
cargo build
cargo test --workspace
```

There is no GUI to click through in `crates/`; verify the merge and sync
logic with unit tests against real `.tmt` fixtures, not by hand-tracing.
`immermemo` is the one app, built for every platform (desktop, Android)
from the same Slint UI and switching backends per target (see its
`Cargo.toml`); its own verification story (`cargo run --example snap`,
`just apk`/`just run`) belongs in its own directory, not here. If iOS
needs more than that -- a native wrapper this repository doesn't have
yet -- its verification story will too, but nothing today decides where
that would live.

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
