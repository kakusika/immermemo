# Adopt `tomet`'s std `@conflict`, retire `@mobile.conflict`

Planning only -- not started. Parked until other in-flight work here
finishes; this file exists so it can resume cold.

## Why

`tomet` now has `@conflict(a: ..., b: ...)` built into `std` (bare, no
`@use`), with real `Vec<Block>` content in `a`/`b` (via `Value::Blocks`,
a new general `key: [...]` value grammar), and `tomet check` already
warns on one left unresolved (`Diagnostic::Conflict`). This repo's own
`vocab/mobile.vocabulary.tmt` doc comment explains exactly why
`@mobile.conflict` had to use three positional range markers
(`mine`/`theirs`/`end`) instead of real content: "tomet's syntax has no
way to bundle multiple blocks into one element's content except via
list items" -- that limitation is gone. This is the fix for the exact
gap this repo's own vocabulary file was written around.

## What exists today (read in full before touching anything)

- `vocab/mobile.vocabulary.tmt` -- declares `@element(conflict)` with
  `side`/`mine`/`theirs` params. The **only** element the `mobile`
  vocabulary declares -- nothing else lives in this namespace.
- `crates/merge/src/markers.rs` -- construction/detection of the three
  marker shapes: `@mobile.conflict(side)` (bare positional `"mine"` /
  `"theirs"` / `"end"`, block or inline placement) and
  `@mobile.conflict(mine: ..., theirs: ...)` (a `Value::Element`
  sitting where a scalar value goes). Also `use_mobile_block`/
  `is_use_mobile_block` -- `@mobile.conflict` needs `@use(mobile)` in
  scope, so `merge` injects it and `resolve` strips it once no markers
  remain.
- `crates/merge/src/merge.rs` -- three call sites that give up
  narrowing a conflict and embed a marker instead: `merge_block_seq`
  (whole blocks), `merge_inline_seq`/`merge_text` (inline runs, down to
  individual characters), `merge_value` (a leaf `{data}`/`(args)` value
  both sides changed differently).
- `crates/merge/src/find_conflicts.rs` -- scans a merged document for
  marker *triples* by position (find `mine`, then the next `theirs`,
  then the next `end`) to build `ConflictItem { index, mine, theirs }`
  for the resolution UI, plus `has_conflicts` for the post-sync report.
- `crates/merge/src/resolve.rs` -- the inverse: walks in the same order
  `merge` produced markers in, replaces each triple/value-marker with
  the picked side (`ConflictResolution::{Mine,Theirs,Both,Rewritten}`),
  then drops `@use(mobile)` once no markers remain.
- `crates/editor/src/conflicts.rs` -- the UI-facing layer:
  `resolve_conflict_step`/`resolve_active_conflict` (buttons call these),
  `conflict_sheet_state` (feeds `ConflictSheetState::Active { mine,
  theirs, .. }` to `immermemo/ui/sheets/conflict.slint`, not read yet --
  read it before touching UI copy).

## The one open design question -- decided

Settled on option 2 below. `crates/sync/src/lib.rs`'s own module doc
says the merge commit is "recorded as a real two-parent git commit, so
history and blame stay intact" -- which means "which side is mine" is
never a content guess, on any device, at any later time: it's "which
parent commit is an ancestor of (or equal to) my own branch tip",
answerable from git alone. That's what makes option 2 cheap here
specifically (not true in general for every conflict-tracking scheme) --
the merge step already keeps exactly the structure a live mine/theirs
re-derivation needs, it just isn't read for that today. Implementing
this needs `crates/editor/src/conflicts.rs` (or a new helper beside it)
to walk from the open note's current commit back to the merge commit
that introduced the still-unresolved `@conflict`, read its two parents,
and check each against the device's own branch history via `git2`
(`Commit::parents()`, ancestry via `graph_descendant_of` or walking
`revwalk`) -- not designed further than that yet; worth its own look at
implementation time rather than guessing the exact `git2` calls now.

## The one open design question (superseded by the decision above)

`mine`/`theirs` are assigned by whichever device runs `merge()` (it
always knows which input was its own local note), and that label is
then **written into the file**. If a merge commit reaches a *second*
device before anyone resolves it (plausible: this is a git-sync app,
nothing stops two devices pulling the same merge commit), that device's
UI would show the first device's "mine" as if it were its own --
exactly backwards. `tomet`'s own design doc for `@conflict` names this
exact failure mode as the reason `a`/`b` carry no mine/theirs meaning:
" 誰から見るかによって完全に逆転可能" (completely reversible depending on
who's looking). Adopting `a`/`b` as the *stored* representation fixes
this for free. It does not, by itself, decide what the **UI** should
show instead of today's "Keep Mine"/"Keep Theirs" buttons:

1. Relabel the UI to something neutral too (`A`/`B`, or less dry --
   "Version 1"/"Version 2", a timestamp, a device name if sync tracks
   one) and accept that "which one is mine" is no longer answerable
   from the file alone, on any device, ever. Simplest, matches `tomet`'s
   own stance, but a bigger UX change than it looks (button copy,
   probably `ConflictResolution::{Mine,Theirs}` naming too, for the same
   reason the stored side shouldn't be called that).
2. Keep "mine"/"theirs" in the UI, but derive it live at resolution
   time instead of from storage: diff `a`/`b` against *this device's*
   current/last-known-local copy of the note (if one is still
   reachable -- git log, a cached pre-merge snapshot) to figure out
   which one is "what I had" on *this* device, freshly, every time the
   sheet opens. Keeps the UX, fixes the correctness bug, costs a real
   design (what if neither side matches, e.g. a third device's edit is
   what's being viewed?).
3. Some hybrid: label by mine/theirs only in the merge that just
   happened in this same session (session-local, never stored, never
   shown again after app restart), falling back to neutral labels for
   a conflict found cold (app reopened, or a different device).

Not deciding this is fine for now -- it's the thing to settle before
writing any code, not before writing this plan.

## Rough step shape (resolve the question above first)

0. Bump the `tomet` git dependency pin to a commit including this
   session's work (`Value::Blocks`, `ElementKind::Conflict`,
   `Diagnostic::Conflict`) -- check `Cargo.toml`'s `[workspace.
   dependencies]` `git = "https://github.com/tomet-lang/tomet.git"`
   entries; `cargo update -p tomet-ast` (and the other `tomet-*`
   members) once a tag/commit exists to pin to.
1. Delete `vocab/mobile.vocabulary.tmt`'s `@element(conflict)` entirely
   (it's the only thing in that file) -- `@conflict` is bare `std` now,
   needs no vocabulary declaration and no `@use`. Check whether the
   `mobile` vocabulary file/namespace is referenced anywhere else before
   deleting the file itself outright (a repo-wide grep for `mobile.
   conflict`/`conflict_name`/`is_use_mobile_block`/`@use(mobile)`/
   `mobile.vocabulary.tmt` specifically -- plain `mobile` alone also
   matches this app being a *mobile* app in lots of unrelated files, so
   grep for the narrower strings, not the bare word).
2. `markers.rs`: replace the three marker-shape builders with one
   `conflict_element(a: Vec<Block>, b: Vec<Block>) -> Element`
   (`Value::Blocks` in `args`, see `tomet-syntax-ast::Value::Blocks`)
   for block/inline placement, and one `value_conflict(a: Value, b:
   Value) -> Value` (plain scalar `a`/`b`, same shape `@meta{x: @conflict(a:
   2, b: 3)}` uses) for the value-slot case. Detection becomes "is this
   `classify_std(el) == Ok(ElementKind::Conflict)`" (one check, not
   three string-matched shapes) plus reading `a`/`b` back out via
   `tomet_semantics::normalized_element_args` (resolves the positional
   shorthand too, for free). `use_mobile_block`/`is_use_mobile_block`
   and every call site of either are deleted outright.
3. `merge.rs`: the three conflict-embedding call sites
   (`merge_block_seq`, `merge_inline_seq`/`merge_text`, `merge_value`)
   each build one `conflict_element`/`value_conflict` instead of a
   three-marker sequence. `merge`'s own `@use(mobile)` injection (lines
   35-37) is deleted.
4. `find_conflicts.rs`: the position-scanning triple-finder
   (`find_conflicts_block_seq`, `find_conflicts_inline_seq`) collapses
   to "find an element classifying as `Conflict`, read `a`/`b`
   straight out of it" -- no more hunting for a matching `theirs`/`end`
   by position. `has_conflicts` similarly simplifies; consider whether
   it should keep its own tree walk or delegate to
   `tomet_semantics_validator::validate_document`'s own
   `Diagnostic::Conflict` (weigh: one less parallel implementation to
   keep in sync with `tomet`'s own classification vs. pulling in the
   validator crate as a new dependency and whatever else it checks).
5. `resolve.rs`: same collapse -- no triple position-scanning, just
   replace the found `@conflict` element with `a`'s or `b`'s content
   (or both, or the rewritten text) directly. `@use(mobile)`-stripping
   step deleted (nothing injected it any more after step 3).
6. `crates/editor/src/conflicts.rs` + `immermemo/ui/sheets/conflict.slint`
   (read the `.slint` file before touching it -- not read yet): apply
   whatever the open design question above settled on. `ConflictItem`'s
   `mine`/`theirs` fields, `ConflictResolution::{Mine,Theirs}`'s names,
   and the button copy all flow from that decision together -- don't
   rename half of them.
7. Test fixtures: this crate's own tests almost certainly assert against
   literal `.tmt` text containing the old three-marker shape (not
   checked yet -- `crates/merge`'s own test modules will have them).
   All of those need rewriting to the new single-element shape; this is
   probably the single largest mechanical chunk of the whole migration
   by line count, even though each individual change is small.
8. `cargo build && cargo test --workspace` (this repo's own
   `AGENTS.md`-mandated verification -- no GUI to click through, the
   merge/sync logic is unit-tested against real `.tmt` fixtures).

## Status

Steps 0-5 committed (`c45a737`, already pushed). Steps 6-8 -- the
view-mode UI rendering (`immermemo/src/render/*`, the Slint files) --
done in a later session and detailed further down ("View-mode design,
decided" section): data model, `conflict_look`, the new `.slint` struct/
component, `flow.rs`'s inline treatment, every stale fixture rewritten,
`cargo test --workspace` green. Not yet committed -- holding for the
user's review (and the UI hasn't been visually checked on a real screen
yet, see that section's last note).

- Step 0: `tomet` pushed to `origin` (3 commits it was behind), `cargo
  update` bumped every `tomet-*`/`tove` dependency to `f444178b`.
  **Bigger than this file originally scoped**: the pin had drifted far
  enough behind that `Element.content`'s `Vec<Inline>` -> `Vec<Block>`
  change (an *earlier* step in this same tomet session, not part of the
  `@conflict` work at all) had never been caught up to either. Fixed as
  its own pass, in `immermemo-tomet-render` and `immermemo-merge`: both
  crates' `*_inline_seq` helpers that used to read `el.content` directly
  as `&[Inline]` now go through `el.content` as `Vec<Block>` -- routed
  through the existing `*_block_seq` functions where one already existed
  (`merge`/`resolve`/`find_conflicts` already had block-level
  counterparts for `children`, since `content` now means the same thing
  `children` always did), or a new `inline_content(el)` helper
  (`tomet-render`) for the handful of inline-only kinds (`em`/`strong`/
  `mark`/`strikeout`/`ruby`/`link`) it still needs flat `Inline`s from.
  `tomet_ast::Section` also gained an `id` field since this pin was last
  bumped (unrelated to `@conflict`); `merge_one`'s `Section` arm needed
  it added to both the constructed value and the `l.connects != r.
  connects`-style bail guard (now also checking `id`).
- Step 1: `vocab/mobile.vocabulary.tmt` deleted outright (it declared
  only `conflict`, nothing else survives in the `mobile` namespace).
  Confirmed nothing references the file path or the vocabulary name
  anywhere else in the repo first.
- Step 2: `markers.rs` rewritten around `conflict_block`/`conflict_
  inline`/`value_conflict` (construction) and `conflict_block_sides`/
  `conflict_inline_sides`/`value_conflict_sides` (reading), all using
  `Value::Blocks` for `a`/`b` at the block/inline level, plain scalars
  at the value level -- exactly the split the task's own research
  established. `is_use_mobile_block`/`use_mobile_block` deleted outright
  (nothing to inject or strip any more). `CONFLICT_MARKER` is now
  `"@conflict"` -- every one of its many callers (`crates/index`,
  `crates/vault`, `crates/editor/src/notes.rs`, `immermemo/src/lib.rs`,
  `immermemo/src/sync.rs`) needed zero code changes, since they only
  ever compared against the constant, never the literal string.
- Step 3: `merge.rs`'s three embedding call sites (block/inline/value)
  switched to the new builders; `@use(mobile)` injection deleted.
- Step 4: `find_conflicts.rs`/`resolve.rs` rewritten around "is this
  element a `@conflict`, read `a`/`b` straight out of it" instead of
  scanning for a `mine`/`theirs`/`end` triple by position.
  `ConflictItem { index, mine, theirs }` -> `{ index, a, b }`;
  `ConflictResolution::{Mine,Theirs}` -> `{A,B}` -- both now live fully
  neutral in `immermemo-merge`, as planned.
- Step 6 (partial): `crates/editor/src/conflicts.rs` recompiles against
  the renamed enum/struct, but **still assumes `a` = mine, `b` = theirs
  unconditionally** (a comment at `conflict_sheet_state`'s construction
  site says so explicitly) -- correct for resolving right after the
  merge that produced the conflict (the overwhelming common case,
  and exactly what the old code also assumed, so no regression), but
  not yet the git-ancestry re-derivation this file's "decided" section
  above calls for. That's real, not-yet-designed-in-detail work (walk
  from the open note back to the merge commit that introduced the
  still-unresolved `@conflict`, read its two parents via `git2`, check
  each against the device's own branch history) -- still pending.
- Step 6: done -- see "View-mode design, decided" below for the full
  account (`classify.rs`'s `conflict_look`/`ConflictLeafBlock`,
  `flow.rs`'s `push_conflict_side`, the new `.slint` struct/component,
  `render/mod.rs`'s and `examples/snap.rs`'s conversions).
  `immermemo/ui/sheets/conflict.slint` (the resolution sheet itself)
  remains untouched -- nothing about the `a`/`b` rename changes its UI.
- Step 7: done -- every fixture that spelled the old 3-marker/
  `@use(mobile)` shape is rewritten (see "View-mode design, decided"
  for the full list).
- Step 8: `cargo build && cargo test --workspace` green for every crate,
  including the UI crate itself and `examples/snap.rs` -- confirmed via
  `nix develop` (this sandbox has no system `fontconfig`/etc. outside
  that shell, same limitation noted when steps 0-5 landed).

Not committed yet -- holding for the user's review. Nothing in this
session has been pushed or committed.

## View-mode design, decided (option 1: show both sides inline)

`immermemo-tomet-render`'s `RenderItem::Element` gained a new, fully
generic field: `block_args: Vec<(String, Vec<RenderItem>)>` -- every
`key: [...]` (`Value::Blocks`) entry of `(args)`, recursively classified
the same way the element's own `[content]` would be (new helper
`classify_blocks_flat`). Nothing `@conflict`-specific lives in
`tomet-render` itself -- this is the same "generic capability, not a
special case" fix this session already gave `tomet`'s own converters,
just one layer up. `"conflict"` joined `ATOMIC_BARE_ELEMENTS` (`ruby`,
`link`) so an inline (mid-sentence) `@conflict` splits out as its own
`RenderItem::Element` too, not just a block-level one (which every
`Block::Element` already got regardless of that list). Fully tested
(24/24, `cargo test -p immermemo-tomet-render`), including new tests for
a bare `@conflict`'s identity + `block_args`, and a rewrite of every
fixture that used to spell the old 3-marker shape as "some namespaced
element" example text (`a_mobile_conflict_marker_triple_is_identified`
-> `repeated_namespaced_elements_are_each_identified`, now using
`@deck.marker` so it's not confusable with testing real `@conflict`
behavior).

**Not done yet, and genuinely large**: the `immermemo` app crate's own
side. **Correction after discussing this with the user**: the `REGISTRY`
mechanism itself is *not* broken by `@conflict` and does not need a
bypass -- `(namespace, name) -> one Look-building fn` stays exactly the
pattern it already is for `ruby`/`link`/`icon`. What's missing is purely
in the *data* `Look`/`ClassifiedBlock` can carry: today they're flat
(`shape, tone, text, secondary_text`), enough for every existing shape
(`ruby`'s `secondary_text`/`reading` is the one field already meaningful
for only one shape, empty for the rest -- the same pattern `@conflict`
needs two more of). The fix is additive, not a dispatch bypass:
- Add `side_a`/`side_b: Vec<ClassifiedBlock>` to both `Look` and
  `ClassifiedBlock` (empty for every shape but `Conflict`, same
  "meaningful for one shape only" convention `reading` already set).
- Add `block_args: &[(String, Vec<RenderItem>)]` and `body: &str` to
  `LookInput`, so a `conflict_look(input: LookInput) -> Look` (replacing
  `mobile_conflict_look`) can recursively call `to_classified_block(body,
  item.clone())` per item on each side and populate `side_a`/`side_b`.
  `conflict_look` is then an ordinary `REGISTRY` row
  (`(None, "conflict", conflict_look)`), same shape as every other
  entry -- no special-cased dispatch ahead of `look_up` needed after
  all.
- **Verified, resolved**: neither a self-referential Slint struct
  (`RenderedBlock { side_a: [RenderedBlock], ... }`) nor a
  self-referential Slint component (one that instantiates itself inside
  its own body, e.g. for a recursive `Repeater`) compiles. Checked
  against the real Slint 1.18.1 compiler in an isolated scratch project
  (outside this repo, via `nix develop` from here to get the same
  toolchain) -- both fail at the exact self-reference with `error:
  Unknown type 'RenderedBlock'` / `error: Unknown element 'TreeNode'`
  respectively: Slint compiles to a static component tree and can't
  resolve a name against its own not-yet-finished definition, with no
  `Loader`-style dynamic-instantiation escape hatch the way QML has one.
  A generic recursive tree was never actually needed, though: a
  `@conflict` only ever has exactly two sides, never an unbounded
  nesting depth the UI needs to walk. `RenderedBlock.side_a`/`side_b`
  are `[ConflictSideBlock]`, a new, deliberately non-recursive struct
  (same flat fields as today's shapes -- `shape, tone, text,
  secondary_text` -- but no `side_a`/`side_b` of its own). Mirror this
  bound on the Rust side too, so the Rust -> Slint conversion in
  `render/mod.rs`'s `to_rendered_block` is a plain field-for-field copy,
  not a lossy flatten: `Look`/`ClassifiedBlock`'s `side_a`/`side_b` are a
  new flat leaf type (name TBD), not `Vec<ClassifiedBlock>` recursively.
  If a side's content happens to itself contain an unresolved
  `@conflict` (e.g. two unresolved merges stacked before either was
  resolved) it renders flat/atomic inside that `ConflictSideBlock`,
  same as any other not-specially-visualized element, rather than as a
  second resolvable nested chip. That loses nothing that worked before:
  `crates/merge/src/find_conflicts.rs`'s `find_conflicts_block_seq` and
  `document_or_block_has_conflict` already stop at the first `@conflict`
  found and never recurse into its own `a`/`b` content looking for more
  -- a nested conflict was never independently reachable from the
  resolution sheet until the outer one resolves and its chosen side's
  content is spliced into the document in its place, same as always.
- A new `RenderedBlockShape.conflict` branch in `rendered_block.slint`'s
  `RenderedBlockView` -- still the one existing component, one more
  branch inside it (same as every other shape), laying out both sides
  with a plain, non-recursive `Repeater` over `[ConflictSideBlock]` (not
  a recursive call to `RenderedBlockView` itself -- confirmed unbuildable
  above). Real layout work, not a one-line enum addition, but not a new
  component either.

**Done.** All of the above is implemented and tested (steps 6/7 of the
rough shape above, plus the view-mode design). What actually landed,
differing from the sketch in small ways discovered while building it:

- **`flow.rs`'s inline decision, settled: expand both sides inline**, not
  a compact chip. Reasoning (discussed with the user): an inline
  `@conflict` only ever reaches `flow.rs` from `merge_text`'s character-
  level diff -- a structural mismatch (a whole paragraph, an element)
  always narrows to a *block*-level `@conflict` instead (`merge_one`
  returns `None`, `merge_block_seq` embeds the marker), never an inline
  one. So an inline `@conflict`'s two sides are always short runs by
  construction, not the "could be arbitrarily long" case the sketch
  above worried about -- safe to expand right in the sentence. Hiding
  that content behind a tap target would also have been a real
  regression from the retired `@mobile.conflict` scheme, which already
  showed real Mine/Theirs text inline, just with chip labels. Landed as
  `flow::push_conflict_side`: each side's leaves become their own
  `Inline::Element`s (even a plain-text leaf, which elsewhere in
  `to_flow_paragraph` flows as bare `Inline::Text` -- `Inline::Text` has
  no color of its own), tinted `Tone::Accent`/`Tone::Warning`
  respectively (the same colors the retired chips used, without a
  mine/theirs *label*), joined by a plain `" / "` text run. Needed one
  small addition to `rendered_block.slint`'s `plain-text` branch, which
  used to hardcode `Palette.foreground` ignoring `tone` entirely (every
  other caller always passed `Tone::Neutral`, so this is additive, not a
  behavior change for them).
- **`ConflictSideBlock`/`ConflictSideBlockView`, Rust and Slint sides,
  as sketched**, with one correction found only by trying to compile it:
  `ConflictSideBlockView` could **not** delegate to `RenderedBlockView`
  by wrapping a `ConflictSideBlock` into an ad-hoc `RenderedBlock` value
  and instantiating `RenderedBlockView` inside its own body. That is not
  the self-reference already ruled out (two distinct, non-recursive
  component names), but Slint still rejects it with the same `Unknown
  element` error: name resolution is strictly top-down within a file,
  so `RenderedBlockView`'s own `conflict` branch (which needs
  `ConflictSideBlockView` to render each side) and `ConflictSideBlockView`
  (which would have needed `RenderedBlockView` to already exist) can't
  reference each other in either file order. Fixed by making
  `ConflictSideBlockView` fully self-contained instead (the same five
  shape branches `RenderedBlockView` has, minus `conflict` itself,
  duplicated rather than shared) and placing it *before*
  `RenderedBlockView` in the file, so the one-directional reference
  (`RenderedBlockView` -> `ConflictSideBlockView`) is the only one that
  has to resolve.
- A nested, still-unresolved `@conflict` inside a side (the edge case
  the design above flags) is normalized to a `Badge`-shaped leaf
  (`to_conflict_side_block`/`to_rendered_block_shape`, both `render/
  mod.rs` and its `examples/snap.rs` duplicate) rather than rendering
  blank -- "show *something*", same convention the rest of this module
  already uses for an unrecognized element.
- `ClassifiedBlock`/`Look`/`LookInput` all landed exactly as sketched
  (`side_a`/`side_b: Vec<ConflictLeafBlock>`, `block_args`/`body` added
  to `LookInput`, `conflict_look` as an ordinary `REGISTRY` row).
  `BlockShape::Chip`/`Divider` deleted outright (nothing but the retired
  marker scheme ever used them) in favor of `BlockShape::Conflict`.
- Every fixture that spelled the old 3-marker/`@use(mobile)` shape is
  rewritten: `classify.rs` (the hand-written triple test ->
  `a_conflict_block_classifies_both_sides`, both "real pipeline" tests,
  plus a new `a_nested_conflict_renders_flat_inside_a_side`), `flow.rs`
  (`note_body_items_mixes_stacked_and_flowed_items_in_source_order`,
  `an_inline_conflict_flows_with_the_end_divider_rendering_as_nothing` ->
  `an_inline_conflict_flows_both_sides_with_distinct_tones`), `mod.rs`'s
  `seam_tests` (all 4; the two layout-engine ones now assert only
  row/line counts, not exact fragment counts -- that detail is `flow.rs`'s
  own tests' job, not the Slint-seam ones'), and `examples/snap.rs`'s
  own duplicate of the `render/mod.rs` seam (demo body text + its own
  `to_rendered_block`/new `to_conflict_side_block`/`icon_image`).
- Found and fixed one pre-existing, unrelated break while getting
  `cargo test --workspace` green: `immermemo/src/lib.rs`'s
  `resolving_the_conflict_clears_the_marker_and_status` still referenced
  `ConflictResolution::Mine`, a leftover from the `A`/`B` rename in the
  commit that landed steps 0-5 (c45a737) that nothing had caught since
  (fixed to `::A`, matching the test's own fixture: `merge()`'s `local`
  is always `@conflict`'s `a` side, and `local` here is the "Mine." note).
- `immermemo/ui/sheets/settings.slint`'s one `@tr(...)` string naming
  `@mobile.conflict` updated to `@conflict`, and its `.po` msgid/msgstr
  hand-edited to match (not run through `slint-tr-extractor` -- not
  installed in this sandbox, and installing a new global cargo tool for
  one string felt like overkill; if a fuller translation-catalog
  refresh is wanted later, `AGENTS.md`'s own instructions cover it).
- `cargo build && cargo test --workspace` green end to end, every crate
  (confirmed via `nix develop` -- see the verification note below),
  including `examples/snap.rs` building clean.

**Verification note, resolved**: this crate (`immermemo`, the Slint UI
binary) would not build in this sandbox at all at first --
`yeslogic-fontconfig-sys`'s build script couldn't find `fontconfig`/
`freetype2`/etc. via `pkg-config`. Chasing each missing `.pc` by hand via
`PKG_CONFIG_PATH` turned into a cascade (fontconfig -> freetype2 -> zlib/
bzip2/libpng/libbrotlidec -> ...) not worth pursuing -- the repo's own
`flake.nix` already solves this. Run everything that touches this crate
through `nix develop --command bash -c '...'`; confirmed working
end to end (`cargo test --workspace` succeeds, ~a few minutes cold).

**Not done**: the view-mode binary itself hasn't been *looked at* --
`cargo test`/`cargo check` prove the data flows and the `.slint` compiles,
not that the layout reads well on a real screen. Worth a real run
(`just run`/`cargo run --example snap`) before calling the UI side done,
same as any other visual change in this crate.

## Git-ancestry mine/theirs labeling, detailed design (not implemented)

Refines the "decided: option 2" section above into something concrete
enough to build. **Simpler than a live git-ancestry walk at resolution
time**: don't reconstruct "which parent was mine" from history after
the fact at all -- record it locally, once, at the moment it's actually
known (merge time), and treat "no record" as the honest "don't know,
show neutral" case rather than a guess.

- `immermemo-sync`'s `Vault::sync` is the one place that ever calls
  `immermemo_merge::merge(base, local, remote)` and then commits the
  result as a real two-parent git commit (`crates/sync/src/lib.rs`'s own
  module doc already says so). `merge()`'s `local` always becomes
  `@conflict`'s `a` side (this crate's own fixed convention, see
  `markers.rs`/`merge.rs` above) -- so at the exact moment `Vault::sync`
  makes that commit, "is `a` mine" is simply true, by construction, no
  inference needed.
- Record that fact right there, keyed by note path, in a small *local-
  only* store -- `crates/vault/src/appdata.rs` already keeps per-vault
  private data outside the synced git tree (same file `has_conflict_
  marker`/`CONFLICT_MARKER` lives beside); add a table/row there:
  `{ note_path, merge_commit_oid }`, written only when that sync's own
  merge actually produced at least one `@conflict` (`MergeResult.clean
  == false`) for that path, overwritten on every subsequent sync of the
  same path (only the most recent merge's labeling matters -- an older
  one is moot the moment a newer merge touches the same note again).
- At resolution time (`crates/editor/src/conflicts.rs`'s
  `conflict_sheet_state`/`resolve_conflict_step`), look up the open
  note's path in that store. Two outcomes:
  - **Found, and its `merge_commit_oid` is still the note's current
    content's source** (sanity check: this device hasn't since pulled
    *another* merge for the same note, which would make the cached
    labeling stale) -- `a` is mine, `b` is theirs, confidently.
  - **Not found, or stale** (app restarted and the store was somehow
    cleared, a different device opened the note before resolving, or a
    newer merge superseded the cached one) -- show neutral labels
    (`A`/`B`) instead of guessing. This is the fix for the actual bug
    this design exists to prevent: a *wrong* mine/theirs claim is worse
    than an honest "don't know."
  - The sanity check needs a cheap way to tell "is this still the same
    merge" -- simplest: also cache the merged note's own text (or a
    hash of it) alongside `merge_commit_oid`, and compare against the
    note's current on-disk content at resolution time. If they differ
    (the user edited around the conflict, say), treat as stale -> show
    neutral labels. Exact-text comparison, not something cleverer: this
    only ever needs to answer "is this the same unresolved conflict I
    just created," not survive arbitrary edits.
- `crates/editor/src/conflicts.rs`'s existing hard-coded "`a` = mine, `b`
  = theirs" comment (added this session, see above) becomes this lookup
  instead -- same two call sites (`resolve_active_conflict`'s
  `side_name`, `conflict_sheet_state`'s `mine`/`theirs` fields), both
  already isolated behind that one assumption, so this is a contained
  change once the store exists.
- Not designed further: the store's exact schema/location within
  `appdata.rs` (new sqlite table? a new small file, like `token_blob`'s
  own pattern?), and whether `resolve_conflict_step`'s per-conflict
  (not per-note) indexing needs the cache keyed more precisely than
  "note path" once a note can have more than one conflict at a time
  (today's `ConflictItem.index` is positional within one `find_
  conflicts` call, not a stable id across edits -- the sanity-check
  text comparison above should cover this, since any edit invalidates
  the whole cached entry rather than trying to track individual
  conflicts' identity across a resolve).
