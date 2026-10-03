# Migrate `apps/slint` from origami-mobile's old `Shapes`/Path icons to `Icons`/SVG

`origami-frameworks` unified desktop (`origami`) and mobile
(`origami-mobile`) icons into one shared `origami-icons` crate, standardizing
on the SVG method. `origami-mobile`'s `Shapes` global (Path stroke-command
strings) is gone, replaced by `Icons` (image values) from `@origami-icons`.
`IconButton`/`BubbleButton`'s `shape: string` property is now `icon: image`;
`BubbleChip`'s `icon: string = ""` is now `icon: image` (presence check
`!= ""` -> `.width > 0`).

Name mapping (old `Shapes.x` -> new `Icons.y`, same icon unless noted):
`back`->`chevron-left`, `forward`->`chevron-right`, `down`->`chevron-down`,
`up`->`chevron-up`, `plus`->`plus`, `undo`->`arrow-back-up`,
`redo`->`arrow-forward-up`, `sync`->`refresh`, `link`->`link`,
`menu`->`menu-2`, `close`->`x`, `settings`->`settings`, `search`->`search`,
`home`->`home`, `files`->`files`, `note`->`note`, `properties`->`info-circle`,
`copy`->`copy`, `check`->`check`, `history`->`history`, `cloud`->`cloud`,
`alert`->`alert-triangle`.

## Files with `Shapes.*` (58 call sites total)

- `apps/slint/ui/components.slint` (10)
- `apps/slint/ui/screens/home.slint` (12)
- `apps/slint/ui/screens/search.slint` (4)
- `apps/slint/ui/screens/files.slint` (2)
- `apps/slint/ui/screens/editor.slint` (8)
- `apps/slint/ui/sheets/history.slint` (8)
- `apps/slint/ui/sheets/vault_history.slint` (3)
- `apps/slint/ui/sheets/conflict.slint` (1)
- `apps/slint/ui/sheets/vault.slint` (1)
- `apps/slint/ui/sheets/settings.slint` (7)

## Steps

- [x] Updated `Cargo.lock` (`cargo update -p origami-mobile -p
      origami-icons`) to pull the pushed `origami-frameworks` commit
      (`a99a660`).
- [x] Also found `apps/slint/ui/screens/properties.slint` importing
      `Shapes` unused (dead import, not in the original 10-file count) --
      11 files total, not 10.
- [x] Renamed every `Shapes.x` reference per the mapping table (60+ call
      sites across 11 files) and fixed every import line to `import {
      Icons } from "@origami-icons/icons.slint";` (keeping `Icon` in the
      import where a file also instantiates it directly).
- [x] `IconButton`/`BubbleButton` call sites: `shape:` -> `icon:`.
      3 local wrapper components (`components.slint`'s `NavItem`,
      `sheets/settings.slint`'s `CategoryRow`, `screens/home.slint`'s
      `QuickActionTile`) had their own `in property <string> icon`
      forwarding to `shape:` -- retyped to `<image>` and their `shape:`
      forward became `icon:` too, same rename.
- [x] Correction while implementing: the blanket `shape:` -> `icon:`
      rename was wrong for 25 *raw* `Icon { ... }` instantiations (not
      through `IconButton`/`BubbleButton`) across 9 files -- those needed
      `icon:` -> `source:` and `tint:` -> `color:` (plus `animate tint`
      -> `animate color`) instead, matching the shared `Icon` component's
      actual property names (`source: image`, `color: brush`). Caught by
      a real build failure ("Unknown property icon in Icon"), not
      foreseen in the plan. Fixed with a brace-depth-aware awk script so
      only lines inside a bare `Icon {}` block were touched, leaving
      `IconButton`/`BubbleButton`/`ActionRow` call sites' own `icon:`/
      `tint:` properties (which really are named that) untouched.
- [x] `apps/slint/build.rs`: added `DEP_ORIGAMI_ICONS_UI_DIR` ->
      `@origami-icons` wiring, alongside the existing manual
      `@origami-mobile`/`@richtext` wiring (confirmed needed: it doesn't
      go through `origami_build::configure()`).
- [x] Root `Cargo.toml` / `apps/slint/Cargo.toml`: added `origami-icons`
      git dependency.
- [x] `cargo build -p immermemo-slint` / `cargo test --workspace` pass --
      checked the real exit code directly (not through `| tail`, which
      masked the `crates/origami-icons` missing-`src/lib.rs` and raw
      `@image-url` failures on the `origami-frameworks` side of this same
      session).
- [x] Ran the built binary; it stayed up without error for the duration
      of a timeout-bounded foreground run (backgrounded `&` runs exited
      immediately with no output, seemingly a terminal/display
      attachment quirk of backgrounding in this sandbox, not a real
      crash -- the foreground run is the trustworthy signal here).
- [ ] Commit (no `Co-Authored-By`, per this repo's `AGENTS.md`). Push
      pending confirmation.
