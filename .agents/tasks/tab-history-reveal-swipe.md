# Tab history (back/forward) + reveal-swipe UI

Plan: /home/tefla/.claude/plans/snazzy-imagining-dove.md (full rationale there)

Two phases, doing phase 1 first:
1. Rust history stack (Session) + mini-card reveal/commit swipe gesture.
2. Note editor sheet horizontal swipe + lightweight live-preview carousel.

## Phase 1 steps

- [x] 1. `immermemo/src/session.rs`: added `BackForwardStack` (vault-rel
      paths + cursor, not `NoteHistory` -- renamed to avoid clashing with
      `EditorState.history`/the git-commit "history" sheets), hooked into
      `open_note` (now split into `open_note` + inner `open_note_inner`)
      to push on normal opens, added `navigate_back`/`navigate_forward`
      that step the cursor via `BackForwardStack::step_back`/
      `step_forward` (pruning dead entries as they're found), resolve to
      a live index, and call `open_note_inner` directly (no re-push).
      Cleared on vault switch (`switch_vault`).
- [x] 2. Added `preview_line`/`note_preview` in `session.rs` (first
      non-empty line, up to 80 chars) -- query-less, distinct from
      `crates/index`'s `extract_snippet`.
- [x] 3. Wired `App`: `can-navigate-back`/`-forward`,
      `history-back-title`/`-preview`, `history-forward-title`/`-preview`
      (all `in property`, pushed from Rust via `sync_nav_state`), plus
      `navigate-back()`/`navigate-forward()` callbacks in `app.slint`,
      handled in `immermemo/src/lib.rs` via `session::navigate_back`/
      `navigate_forward`.
- [x] 4. `app.slint`: fixed chevron-left/chevron-right icons flanking
      `MiniNoteCard` inside `mini-card-gesture` (not independently
      tappable -- the swipe commits directly, see design Q&A in the
      plan); `mini-card-gesture` gained `handle-swipe-right`/`-left`
      (gated on `can-navigate-back`/`-forward`) alongside its existing
      `handle-swipe-up`, disambiguated once per gesture via
      `mini-drag-locked`/`mini-drag-horizontal`; drag clamps
      `mini-card-reveal-dx` (applied only to `MiniNoteCard`'s own x, the
      icons never move); release past threshold/velocity calls
      `navigate-back`/`-forward` immediately, else springs back to 0.
- [x] 5. Unit tests for `BackForwardStack` (visit/truncate-on-branch/
      step_back/step_forward/dead-entry-pruning/clear) in
      `session.rs`'s `back_forward_stack_tests` module -- one test's
      first assertion was actually wrong (expected forward history fully
      gone after pruning a dead middle entry; corrected once the
      implementation's actual, correct behavior made the bug in the test
      itself obvious).
- [x] 6. `cargo build` + `cargo test --workspace` clean throughout.
- [x] 7. `cargo run --example snap`: confirmed the mini-card fully covers
      both icons at rest, and (temporarily forcing `mini-card-reveal-dx`
      and `can-navigate-back` to verify, then reverting both) that a
      revealed back icon renders in accent color at the right position.
- [ ] 8. Ask the user to hand-test the clamped reveal-drag feel on
      device (can't simulate a real drag headlessly) before starting
      phase 2.

## Phase 2 (after phase 1 lands and is confirmed working)

Note: by the time phase 2 started, `86a8ad7` had already added `NavPill`
(tap-only back/forward) to the note sheet. Its own doc comment in
`components.slint` already flagged this swipe as still-unwired future
work ("Swiping the body itself to navigate isn't wired up yet"), so this
phase adds the swipe *alongside* NavPill rather than replacing it --
confirmed with the user before starting (NavPill stays, both pills
coexist).

- [x] 9. `editor.slint`: `header-gesture`/`body-gesture` switched from
      `SheetDragArea` to a new local `NoteSheetGestureArea` (inherits
      `SwipeGestureHandler` directly, same shape as `SheetDragArea` plus
      a second axis). `handle-swipe-down` stays fixed true in the
      component body; `handle-swipe-left`/`-right` are bound per
      instance to `nav-swipe-enabled && can-navigate-forward`/`-back`
      (disabled while `note-view == 1` or editing has focus, and
      whenever that direction has nowhere to go -- modeled on
      `app.slint`'s 5-tab swiper gating, not the mini-card's
      always-recognized-but-muted style). `moved`'s axis lock (dx vs dy
      compared once per gesture, `drag-locked`/`drag-horizontal`) is the
      same pattern `app.slint`'s `mini-card-gesture` already uses, since
      `SwipeGestureHandler`'s own direction resolution isn't available
      continuously during `moved`. No changes to the other full-screen
      sheets (`VaultSheet`/`HistorySheet`/etc.) -- they keep using the
      unmodified, down-only `SheetDragArea`.
- [x] 10. 3-slot horizontal carousel: new local `HistoryPreviewPanel`
      (title + preview line, no real rendering) at `-width + swipe-nav-dx`
      (back) and `width + swipe-nav-dx` (forward), `edit-scope` itself at
      `x: swipe-nav-dx` (center/real slot) -- all inside `body-container`,
      which gained `clip: true` so the side slots are invisible at rest.
      `history-back/-forward-title/-preview` (already pushed from Rust by
      phase 1's `sync_nav_state`) wired through from `app.slint`'s
      `pane := EditorScreen` instantiation (4 new property bindings, no
      Rust changes needed -- the data was already there).
- [x] 11. `commit-nav-swipe(dx)` function on `EditorScreen`: plain 40px
      distance threshold (no velocity/fast-flick case, unlike the
      sheet-dismiss/mini-card gestures -- deliberately matched to the
      5-tab swiper's own simpler model instead, per the plan's own
      framing of this as closer to flipping tabs than to a commit-or-
      cancel sheet drag), calls `navigate-back()`/`navigate-forward()`
      then resets `swipe-nav-dx` to 0 (animated, since `is-swiping-nav`
      is cleared first) -- the real content swaps in synchronously via
      the callback before the reset animation plays, same timing
      `sheet-drag-swiped`'s existing close/cancel handling already
      relies on.
- [x] 12. `cargo build`/`cargo test --workspace` green (via `nix
      develop`). Visually verified headlessly via a temporary one-off
      `cargo run --example snap` scenario (`swipe-nav-dx` made `in-out`
      and forwarded through a temporary `App.debug-swipe-nav-dx`
      property, both reverted after): confirmed the back/forward preview
      panels clip and position correctly during a drag (checked at a
      small 120px offset and a large 500px offset -- the panel's title
      only becomes visible once dragged far enough for its own
      left-aligned text to scroll into the visible slice, which is
      correct, not a bug), and that the normal at-rest editor screen has
      no regressions (NavPill/ViewSwitcher/FAB all render as before).
      **Not done**: on-device hand-test (see below) -- same as phase 1,
      the clamped-drag feel and live carousel motion can't be judged
      headlessly.

## Phase 2 redesign: lightweight preview replaced with real rendering

User hand-tested step 12's lightweight `HistoryPreviewPanel` (title +
`preview_line`) on-device and rejected it outright, for reasons that
exposed real gaps in how step 12 was verified, not just a taste call:

- **Huge empty margin either side.** The panel's content (2 short lines)
  sat pinned to the top of a full sheet-height `Rectangle` -- the rest
  was just blank background. The temporary debug screenshots in step 12
  *did* show this (visible in the captured frames) but it wasn't read
  correctly as a problem at the time -- a miss on this session's part,
  not something the screenshots hid.
- **Note appears from the wrong side / content pops mid-animation.**
  Root cause, confirmed by re-reading the code rather than guessing:
  `commit-nav-swipe` calls `navigate-back()`/`navigate-forward()` at the
  exact moment of commit, which synchronously flips
  `can-navigate-back`/`-forward` (and therefore which `HistoryPreviewPanel`
  `if` block even exists) and `edit-scope`'s own bound content, all
  *before* `swipe-nav-dx`'s reset-to-0 animation has finished playing.
  This session had actually noticed this risk while building step 11 (see
  its own note about timing) but filed it away as acceptable instead of
  flagging it as a real caveat before calling the phase done -- the
  session's own mistake, named directly when the user asked about it.
  **Not yet fixed as of this redesign** -- still applies to the
  real-rendering version below; tracked separately, see "Still open" at
  the end of this section.

Also: this session wrote its own throwaway `ppm2png.py` in `/tmp` during
step 12's verification instead of first checking for one in the repo --
`scripts/ppm2png.py` already existed and does the same job. Checked for
existing tooling before reusing the technique below.

Discussed with the user and settled on **real rendering** over a
lighter-weight title-only redesign, after correcting two unverified
performance claims this session made along the way (both checked against
the actual vendored Slint 1.18.1 source, not asserted from memory):

- "Layout cost lands every frame while a panel is mounted" was **wrong**.
  Slint's property system is dependency-tracked/dirty-based
  (`i-slint-core-1.18.1/properties.rs`: a binding only re-evaluates when
  its `dirty` flag is set), and both renderers back this with their own
  caching: the software renderer is an explicit partial renderer
  (`partial_renderer.rs`'s module doc: repaints only items whose bounding
  box changed or whose rendering `PropertyTracker` is dirty) and the Skia
  renderer (`i-slint-renderer-skia-1.18.1/lib.rs`) carries its own
  `dirty_region` tracking plus `ItemCache`s and a `text_layout_cache`.
  A mounted-but-unchanging panel costs nothing extra at rest; cost is
  only paid while something about it is actually changing (e.g. all
  three carousel slots' `x` during an active drag -- same as the
  existing 5-tab swiper already costs today).
- The one real, still-correct cost is `classify`/`flow` parsing
  (`render::note_body_items`) for each neighbor note. Mitigated with a
  browser-bfcache-style cache (see below) rather than accepted as a
  recurring cost.

User also caught a structural waste this session hadn't: back and
forward are never on screen at once, so there's no reason for both
`HistoryPreviewPanel`s to be mounted (and both bound to `swipe-nav-dx`)
whenever `can-navigate-back`/`-forward` is merely true (i.e. almost
always once there's history, even at rest). Fixed by also gating each
panel's `if` on `swipe-nav-dx`'s sign (`> 0px` for back, `< 0px` for
forward) -- only the panel actually being dragged toward ever exists,
and at true rest (`swipe-nav-dx == 0px`) neither does.

### What landed

- `editor.slint`'s `HistoryPreviewPanel`: now title + a real `NoteBodyView`
  (`items`/`text-font-size` properties, same structure `edit-scope`'s own
  view-box uses), not a plain preview string. `clip: true` added (no more
  giant blank background below two lines of text). Both `if` conditions
  gained the `swipe-nav-dx` sign gate described above.
- `history-back-preview`/`history-forward-preview` (`string`) replaced
  throughout (`editor.slint`, `app.slint`) with `history-back-items`/
  `history-forward-items` (`[NoteBodyItemView]`).
- `session.rs`: `note_preview`/`preview_line` replaced by
  `history_preview`, which runs the real `render::note_body_items`
  pipeline (same one `update_rendered_body` uses for the currently-open
  note) instead of extracting a plain first-line string. Results are
  cached in a new `Session::history_preview_cache: HashMap<String, (f32,
  ModelRc<NoteBodyItemView>)>` keyed by vault-relative path, value keyed
  alongside the `max_width` it was measured against (a resize/font-size
  change invalidates lazily on next lookup, not by eagerly walking the
  cache). `sync_nav_state` changed signature from `&Session` to
  `&Rc<RefCell<Session>>` since the cache lookup needs its own borrow;
  all three call sites (`open_note`, `navigate_to`, `switch_vault`)
  updated. Cache cleared wholesale on vault switch
  (`switch_vault`, alongside the existing `note_nav.clear()`), and a
  single path's entry dropped by `lib.rs`'s `on_edited` handler the
  moment that note's own content is written to disk (same spot
  `index.record_write` already runs) -- so a stale pre-edit preview can
  never be shown again once the real file has changed.
- Verified the same way as step 12 (temporary `in-out`/debug-property
  round trip on `swipe-nav-dx`, reverted after): both panels render real
  flowed content (including inline `@strong` formatting) at the correct
  clipped position; `cargo build`/`cargo test --workspace` green; normal
  at-rest `editor.ppm` screenshot unchanged (no regression). Used
  `scripts/ppm2png.py` this time, not a reinvented copy.

### Mid-animation content-swap bug: fixed

User explicitly asked for this fix ("直そうか。もちろん。" -- let's fix it,
of course) after the diagnosis above. `commit-nav-swipe` split into two
phases instead of one, in `editor.slint`:

1. Past the threshold, `swipe-nav-dx` now animates the rest of the way to
   a full `+-root.width` (not back to `0px`) over the existing 220ms ease.
   The already-correct `HistoryPreviewPanel` (real content, same as
   before this fix) finishes sliding fully into the center; `edit-scope`'s
   still-old content finishes sliding fully off-screen. Nothing about
   `can-navigate-*`/`edit-scope`'s bound content changes during this
   phase -- the actual bug's trigger -- so there's nothing left to flip
   mid-flight.
2. A new `nav-settle-timer` (`Timer`, 220ms interval -- matches the
   animation's own duration) fires `finish-nav-commit` once that
   animation is done: *then* `navigate-back()`/`navigate-forward()` is
   called (swapping `edit-scope`'s real content to what the preview panel
   was already showing -- same title + `NoteBodyView` structure in both,
   so this is visually a no-op) and `swipe-nav-dx` snaps back to `0px`
   *instantly* (new `nav-commit-instant` flag, 0ms duration, same idea as
   the existing `is-swiping-nav` flag but for this one deliberate jump)
   in the same instant. `can-navigate-*` flips at the exact moment the
   panels' `swipe-nav-dx > 0px`/`< 0px` gate goes false, so the panel
   that was covering the screen and `edit-scope` underneath it (now
   showing the same thing) swap with nothing visibly moving.

`pending-nav-direction` (`-1`/`0`/`1`) carries which direction was
committed from `commit-nav-swipe` through to `finish-nav-commit`, rather
than re-deriving it from `swipe-nav-dx`'s sign at settle time.

Verified: `cargo build`/`cargo test --workspace` green; `cargo run
--example snap`'s existing `editor.ppm` frame unchanged (no regression).
**Could not verify the actual timed settle/handoff headlessly** -- it
depends on `Timer`'s real wall-clock firing and a real touch-release
gesture, neither of which the headless `snap` harness can drive (no
synthetic `SwipeGestureHandler` events). The logic is traced through
above and in the code's own comments, but whether the handoff actually
*looks* seamless needs the same on-device hand-test already pending
below -- this isn't a new gap, just the same one the rest of this task's
gesture feel has always needed.

### Back-only fixed-behind redesign (forward still slides in)

Still from on-device hand-testing: real rendering fixed the emptiness,
but the user still found the title slow to become legible, because
the back panel had to slide a long way in from fully off-screen before
its own (left-aligned) title scrolled into view.

Proposed two alternatives, asked which to pursue: (1) stop moving the
*past* note at all -- sit it fixed behind the current note, which slides
away to reveal it, like peeling a card off a stack rather than something
sliding in -- or (2) show the title vertically in the seam between notes.
Recommended (1): it fixes both title *and* body reveal timing at once
(nothing to wait for -- the whole panel is already sitting there,
fully rendered, the moment any of it is uncovered), is simpler to
implement than a rotated-text seam label, and matches an established
native pattern (iOS/Android predictive-back). User agreed.

**First implementation attempt was wrong, caught before landing**: built
the fixed-behind treatment symmetrically for *both* back and forward,
reading the user's "don't move the past note" framing as a general
back/forward depth rule. User stopped this immediately: the framing was
specifically about the *past* -- a stack/depth metaphor only makes sense
for what's behind you; forward (something chronologically ahead) has no
reason to be "behind" the current note too. Asked which treatment
forward should get instead; user chose: leave forward exactly as it was
(the original off-screen slide-in from `adopt-std-conflict`-era "Phase 2
redesign" above) -- not touched by this change at all.

**Landed**: only the back `HistoryPreviewPanel` instantiation changed,
`editor.slint`'s `body-container`: `x: -parent.width + root.swipe-nav-dx`
-> fixed `x: 0px` (same resting rect `edit-scope` itself occupies).
`edit-scope` (declared after it, so it paints on top and fully covers it
at rest) is the only thing still moving for a back-direction drag --
dragging right slides it out of the way to reveal more of the
already-complete, already-rendered panel sitting underneath, instead of
something new arriving from off-screen. No changes to
`HistoryPreviewPanel` itself (its title is already left-aligned, which
is exactly correct for a reveal that grows from the left edge inward --
no directional-alignment property needed, unlike what a symmetric
forward treatment would have required). The existing two-phase
`commit-nav-swipe`/`nav-settle-timer`/`finish-nav-commit` handoff
(previous section) still applies unchanged and is, if anything, more
seamless now for back specifically: the panel it hands off to was never
sliding in the first place.

Verified: `cargo build`/`cargo test --workspace` green; `editor.ppm`
unchanged (no regression). Same headless-can't-drive-a-real-gesture
limitation as before -- the fixed-panel reveal and the settle handoff
both still need on-device confirmation.

### Forward also redesigned, plus a real overlap bug fixed

On seeing the back-only redesign, the user pushed back immediately on
two things in the same message (before any hand-test -- caught from
reading the description alone):

1. "forwardは、現在のノートを固定し、次のノートを手前にのせる形が良くない?"
   -- for forward, shouldn't the *current* note stay fixed, with the
   *next* note sliding on top of it, instead of leaving forward as the
   original off-screen slide-in? This generalizes cleanly from the
   back redesign's own logic once stated this way: in either direction,
   the *older* note of the pair being shown stays fixed, and the *newer*
   one is what moves -- back: current is newer (of the {current, past}
   pair), so *it* moves, away, to reveal the fixed older one. Forward:
   the target is newer (of the {current, next} pair), so *it* moves, in,
   over the fixed older (current) one. Previously thought of as "back
   gets the new treatment, forward keeps the old one" -- actually both
   fit one consistent rule once framed this way, they just needed two
   different implementations to express it (old-direction already fixed
   the *target*; forward instead needs to fix *current*, which is a
   different end of the pair).
2. "背景透明なままじゃない？コンテンツが重なっちゃっている" -- a real bug this
   session introduced and missed verifying for: `edit-scope`
   (`FocusScope`) and `AppScrollView` (`editor-scroll`) have never had
   their own opaque `background` -- fine before this task, since nothing
   was ever positioned directly underneath them at the same screen
   coordinates. The back-only fixed-behind panel broke that assumption
   (it now sits exactly at `edit-scope`'s own resting bounds), so
   whatever wasn't actually covered by `edit-scope`'s own text content
   (nearly the whole screen below the title) showed the back panel's
   content bleeding straight through.

Both fixed in `editor.slint`:

- `edit-scope`'s `x` changed from unconditional `root.swipe-nav-dx` to
  `max(0px, root.swipe-nav-dx)` -- moves normally for a back drag
  (`dx > 0`), stays at `0px` for a forward one (`dx < 0`). The forward
  `HistoryPreviewPanel` itself needed no position/z-order change --
  it already slid in via `x: parent.width + root.swipe-nav-dx` and was
  already declared (and therefore painted) after `edit-scope`, i.e.
  already on top -- fixing `current` in place for forward was the only
  change actually needed to get "next note slides on top of a fixed
  current" for free.
- `edit-scope` gained a plain opaque `Rectangle { background:
  Colors.page-background; }` as its first child, filling it -- moves
  with it (so it still properly occludes the back panel while covering
  it, and stops occluding exactly as it slides away).

Verified: `cargo build`/`cargo test --workspace` green; `editor.ppm`
unchanged (no regression). Also re-verified headlessly with the same
temporary debug-property technique as before, this time specifically at
a *small* drag distance (80px) in both directions: back showed the
neighbor's title legible immediately with no bleed-through into/from
`買い物リスト`'s own area; forward showed the current note's title
completely stationary (pixel-identical to rest) while the incoming
panel's edge slides in cleanly from the right. Screenshots confirmed
both fixes before reverting the debug properties.

### Depth shadow

Final request before wrap-up: a shadow to make the stacking itself read
clearly (which note is "on top" vs. "underneath"). Applied to whichever
layer is the moving/elevated one in each direction -- `edit-scope`'s own
new opaque backing `Rectangle` (back direction, gated on
`swipe-nav-dx > 0px` so it doesn't look like a stray shadow around the
note view the rest of the time) and the forward `HistoryPreviewPanel`
instantiation (unconditional -- that whole panel already only exists
while relevant). Both use `drop-shadow-offset-x: -6px * Metrics.ui-scale`
(leaning toward each one's own leading edge -- the boundary with
whatever's fixed underneath it) with the same blur/color convention
`SheetSurface` already uses elsewhere in this file, just on the x axis
instead of y since the travel here is horizontal.

**Could not visually confirm the shadow itself via `cargo run --example
snap`** -- the headless software renderer that tool uses doesn't appear
to render `drop-shadow-*` at all (checked: the FAB's own long-standing
shadow, several screens away from anything touched this task, is
likewise invisible in every snap screenshot this whole task produced).
Since the property usage is otherwise identical to multiple already-
shipped shadows elsewhere in this exact file, this is treated as a
renderer-fidelity gap in the headless tool, not a reason to doubt the
code -- but it does mean the shadow specifically needs the on-device
check more than anything else in this task did.

Build/test confirmed green as usual; no regression in `editor.ppm`.

## Done, pending on-device confirmation

Every planned step (both phases, the real-rendering redesign, the
settle-timer fix, the back/forward depth redesign, and this shadow) is
implemented, tested (`cargo build`/`cargo test --workspace` green
throughout), and headlessly verified wherever the snap tool's own
software renderer could show it. What's left is exactly what every
section above already flagged: a real on-device hand-test of the feel
(clamped reveal-drag, the real-rendering carousel, the settle-timer
handoff, back/forward's different fixed/sliding treatment, and now
whether the depth shadow actually reads as intended).

## Wrap-up

Fold anything worth keeping into doc comments or the commit message(s)
when each phase finishes, then delete this file (or the whole file once
phase 2 is done).
