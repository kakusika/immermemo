# Media: free-form note icons and cached thumbnails

Workspace: `.agents/workspaces/media-icons-thumbs`

Scope agreed with the user: **stage 1 and 2 only**. Stage 3 (banner, images
inside the note body) starts only after the user has confirmed 1 and 2.

## Decisions already made (do not re-litigate)

- Where an original image lives is decided by the *reference written in the
  note* (a remote `https://` URL, or a path inside the vault). The app has
  **no** "image storage location" setting, and no way to add/paste/upload an
  image yet. A future "where files go" schema file is the point to revisit
  attachments (`tmtroot/architecture.tmt` keeps attachments out of git;
  that stays true, we only *read* what the user put in the vault).
- The derived cache is never part of the vault or git. App-managed,
  not user-configurable.
- Reference syntax follows `../tomet-book` (`crates/tmtbook-render/src/document/meta/icon.rs`):
  `icon: @doc.icon("star")` (glyph), `icon: "🎂"` (text/emoji), `icon: "x.png"`
  / `icon: "https://..."` / `icon: @embed("x.png")` (image).
- Text icon layout follows tomet-book: 1 grapheme = one big glyph, 2 = side
  by side, 3 = 2x2 with the third centred on the bottom row, 4 = 2x2,
  5+ = first two graphemes.
- Disk cache format is **not WebP**: `image` 0.25's WebP encoder is
  lossless-only (`image-webp` 0.2.4 `encoder.rs`: "Only supports VP8L
  lossless encoding"). Opaque images are stored as JPEG, images with alpha as
  PNG. SVG is passed through unrasterised (Slint renders it).
- `slint::Image` holds `VRc`s, so it does not cross threads; workers return
  decoded pixel data and the UI thread builds the `Image`.
- Network fetching is injected (`Fetcher` trait) so `crates/media` is
  testable without network and does not duplicate the app's TLS/cert setup
  (`app/src/platform/cert.rs`).

## Known risks / deviations to tell the user before calling this done

- Local-image cache key is `(vault-relative path, size, mtime)`, not a
  content hash (hashing means reading every original on every refresh). A
  git checkout rewriting mtimes costs one redundant re-thumbnail, never a
  wrong image. This deviates from the earlier "content hash" suggestion.
- Remote images are fetched automatically once a note with one is listed or
  opened (https only, size/time/pixel limits). There is no privacy toggle in
  this change.
- `app/src/avatar.rs` keeps its own separate cache; folding it into
  `crates/media` is deliberately left out.
- No in-app UI to author a text/image icon (the icon picker still only sets
  tabler glyphs). Text/image icons are written in the `.tmt` by hand.
- `lucide:` / `simple:` string prefixes from tomet-book are not supported
  (`origiri-icons` vendors tabler only).
- Non-`.tmt` conflict policy in `crates/sync` is untouched ("local wins").

## Stage 1 -- icon model

- [x] 1.1 `render-tomet`: `NoteIcon { Glyph{name,pkg}, Text, Image }`,
      `extract_note_icon` returns it (string / `@embed` / `@link` / `@ref` /
      `@doc.icon`), tests for every form
- [x] 1.2 `render-tomet`: `text_icon_cells` (grapheme segmentation, tomet-book
      layout rules), tests
- [x] 1.3 `index`: schema v3 (`icon_kind`, `icon_value`, `icon_pkg`); no migrations: any other
      schema version is dropped and rebuilt from the vault; `icon_for -> Option<NoteIcon>`, tests incl.
      the rebuild
- [x] 1.4 `core`/`app`: adapt callers (`session/mod.rs`, icon picker)
- [x] 1.5 Slint: `NoteIconKind`, `NoteIconView`, shared `NoteIconBadge`;
      use it in `DirectoryRow`, grid tile, `MiniNoteCard` (mini card now shows
      the current note's icon instead of the fixed note glyph)
- [x] 1.6 `cargo test --workspace`, `cargo run --example snap` look

Stage 1 notes: the Directory grid uses an app-local `DirectoryTile`
(`app/ui/widgets/directory_tile.slint`), not the framework's `IconTile`,
because `IconTile` tints whatever image it gets. `snap` (software renderer)
draws a picture icon with square corners: its `combine_clip` ignores the
border radius, while femtovg's `visit_clip` honours it. Every shipped target
(Linux, Android, iOS) uses femtovg, so this is a `snap`-only artifact.

## Stage 2 -- `crates/media` + async thumbnails

- [x] 2.1 New crate `immermemo-media`: `MediaSource`, `resolve` (note-relative,
      then vault-root-relative; reject escaping the vault), `Preset::Thumb`
- [x] 2.2 Thumbnail production with `image::Limits`, JPEG/PNG/SVG, tests with
      generated fixtures
- [x] 2.3 Disk cache: key = sha256(identity + preset), touch on hit, size cap
      with LRU sweep, negative cache for failed fetches, tests
- [x] 2.4 (`Fetcher` trait done in crates/media; remaining: `ureq` implementation in the app (https only,
      byte cap, timeout), fake-fetcher tests (no refetch on hit)
- [x] 2.5 App: `thumbs` loader (in-flight dedupe, bounded in-memory LRU,
      worker thread, coalesced UI refresh on completion)
- [x] 2.6 Wire into directory list/grid and mini card
- [x] 2.7 `tmtroot/architecture.tmt` section (Japanese), `just docs`,
      `just docs-check`; crate `//!` doc is the design record
- [x] 2.8 Full verification, report risks above

## Status

Stages 1 and 2 are implemented and verified here: `cargo test --workspace`
(all green), `cargo clippy --workspace --all-targets` (clean),
`cargo fmt --all --check`, `just docs-check` (the one warning, in
`tmtroot/dev/design/roots.tmt`, predates this work), and `cargo run
--example snap` (text icons 1-4 cells, picture icon on the list, grid and
mini card). NOT verified: Android/iOS builds, and a real device (rounded
picture corners are femtovg-only, see Stage 1 notes).

Stages 1 and 2 were confirmed on a device by the user and committed
(`feat: free-form note icons and cached image thumbnails`).

## Stage 3 -- banner and images in the note body (in progress)

Defaults chosen (change only if the user objects):

- Banner: `@meta{ banner: "images/hero.jpg", banner-y: 30 }` (string path/URL
  or `@embed(...)`; `banner-y` is a 0-100 vertical focal percent, same as
  tomet-book's `object-position`). Drawn full-bleed above the title in the
  editor's scrolling content, ~160dp tall, `cover`, view mode only. No index
  column (banners are not shown in lists), so no schema change.
- Body image: a block-level `@embed[alt](path: "x.png")` (tomet's own form;
  `@embed(ref: name)` name lookup is NOT supported) drawn full width at its
  own aspect ratio, rounded. Not inline in a paragraph, no tap-to-zoom.
- `Preset::Banner` and `Preset::Body`, both 1280px long edge, with their own
  tiny decoded cache (8) apart from the 96-entry thumbnail one.
- `MediaBase { vault_dir, note_rel }` is passed explicitly to
  `note_body_items` (no hidden global); history previews stay image-free.

- [x] 3.1 `media`: `Preset::Banner`/`Body`, tests
- [x] 3.2 `render-tomet`: `NoteBanner` + `extract_note_banner`; `embed` becomes
      an atomic bare element; shared media-target extraction
- [x] 3.3 app `classify`: `BlockShape::Image`; `RenderedBlock.picture`
- [x] 3.4 `thumbs`: presets, separate wide LRU
- [x] 3.5 thread `MediaBase` through `note_body_items`/`update_rendered_body`
- [x] 3.6 Slint: image block, banner above the title; `current-banner`
- [x] 3.7 refresh on ready, tests, snap, architecture.tmt, clippy, docs-check

### After stage 3: name resolution and Git LFS

- `ref:` names (`@link(ref:+c14d6f.png)`, the form the user's vault uses) are
  resolved through `tomet::links::VaultLinkIndex` (tomet commit `925b859`,
  pushed: the `links` feature, no SQLite). `Cargo.lock` now pins tomet there.
  immermemo's own walk only supplies the file list (no dot-folders, no
  symlinks).
- Found on the user's real vault (`/mnt/hd-main/appdata/public/arcana`): the
  images are **Git LFS pointer files** (131 bytes of text), because
  `.gitattributes` sends `*.png` etc. through LFS and libgit2 never runs the
  filter. So nothing can be shown even though the path now resolves.
  `LfsPointer::parse` detects it and the failure says so. OPEN DECISION for
  the user: implement fetching the LFS object (batch API with the provider
  token, cache keyed by the pointer's sha256) or leave images in LFS vaults
  unsupported.

### Stage 3 status

Implemented and verified (tests, clippy, docs-check, snap). The banner is an
inset rounded card above the title (not full-bleed: `content-layout` has
gutters), view mode only. Awaiting the user's check on a device, then the
task can be closed: fold nothing further (decisions live in `crates/media`'s
`//!` doc and `architecture.tmt`), commit, and delete this file and the
workspace.

## When done

Fold the decisions above into `crates/media`'s `//!` doc and
`tmtroot/architecture.tmt`, then delete this file.
