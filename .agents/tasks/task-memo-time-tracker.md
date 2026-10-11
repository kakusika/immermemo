# Task memo + time tracker (not started -- planning only)

Created because this session's context was too low to safely carry an
implementation. Nothing below has been implemented, no files outside
this one were touched. A future session should be able to resume from
this file alone.

## Origin

User request (paraphrased): now that `tomet` has landed its recent
large-scale changes, finally build the long-wanted task-memo + time-tracker
feature on top of `-@task()`.

**Update (2026-10-11):** user's actual plan is to get `@task` promoted
to `tomet`'s own *official* vocabulary (std), the same path `@conflict`
took (was `@mobile.conflict`, immermemo-owned, then became bare std
`@conflict`). That is upstream `tomet` work, not something to declare
in `vocab/mobile.vocabulary.tmt` here. This file is now scoped to: (a)
what immermemo needs to do once `tomet` ships std `@task`, and (b) an
estimate of that footprint, asked for explicitly instead of
implementation (session context was/is constrained). Nothing in this
file is started.

## What "recent large-scale changes" actually means here

Checked against `../tomet` directly rather than assumed:

- There is **no builtin `@task` element** in `tomet`. The only `@task`
  references in `../tomet` are in
  `crates/tomet-semantics/src/positional.rs` test names/doc-comments,
  used purely as a *hypothetical* custom-element example (a `title`/
  `priority` positional pair) to illustrate the old, now-removed
  `@settings{ elements: {...} }` positional-inference mechanism. It was
  never a shipped element and must not be treated as one.
- The real recent capability is **`@param(name){ type: ..., positional:
  true }` inside a vault's own `@vocabulary` declaration**, which *is*
  implemented (confirmed in
  `../tomet/tmtroot/docs/spec/elements/std-directive/settings.tmt`:
  "`@args` の `@param` は実装済み"). This is what finally lets
  immermemo declare its own `@task(title, priority)`-shaped element with
  real positional args, via its own vocabulary file, instead of a
  parser-sentinel workaround.
- Shape of the declaration (from that same spec file):
  ```tm
  @kind(vocabulary)
  @vocabulary(mobile){ version: "1.0.0" }

  @element(task){
    display: block
    @args{
      @param(title){ type: string, positional: true }[ タスクの見出し。 ]
      @param(priority){ type: string, positional: true }[ 優先度。 ]
    }
  }[
    タスクメモ一件。
  ]
  ```
  This is a sketch to validate against the real `../tomet` checkout, not
  a final design -- see open questions below.

## Important pre-existing-state findings (don't re-derive these)

- **`vocab/mobile.vocabulary.tmt` does not exist yet.** `README.md` and
  `CONTRIBUTING.md` both describe it as already holding
  `@mobile.conflict`, but `git ls-files` shows it was never committed.
  README/CONTRIBUTING are stale on this point.
- **`@mobile.conflict` itself was retired.** Conflict marking now uses a
  bare `@conflict` (`crates/merge/src/markers.rs`: `CONFLICT_MARKER =
  "@conflict"`), which became `tomet`'s own std element, not a
  namespaced one `immermemo` owns. So there is currently *no* existing
  content that `vocab/mobile.vocabulary.tmt` needs to hold -- a new
  `@task`/`@mobile.task` element would be the file's first real
  resident, not an addition alongside conflict markup.
- `docs/design.md` also does not exist in this checkout; the source is
  `tmtroot/design.tmt`, exported via `just docs`. Any new design
  rationale for this feature belongs in `tmtroot/design.tmt`, never
  edited as `.md` directly (`just docs-check` enforces this).
- `README.md`'s description of `vocab/mobile.vocabulary.tmt` holding
  `@mobile.conflict` is out of date regardless of this feature -- worth
  a separate small correction, not bundled into this task.

## Merge: already-implemented 3-way merge is sufficient, no new code expected

Corrected from an earlier version of this file, which wrongly claimed
the time-tracker's entry list would need a bespoke union/append merge
rule. Checked against `crates/merge/src/merge.rs` directly:
`merge_value` (the three-way merge of one `Value`) recurses into
`Value::Seq` via `merge_value_seq`, which runs the *same*
`diff3::merge3` LCS-style three-way diff that `merge_block_seq` uses
for block/children sequences -- not a naive positional/overwrite
comparison. Two devices independently appending different entries to
the same list is exactly the classic "both sides inserted at/near EOF"
diff3 case, which already merges cleanly without a conflict marker.

So: as long as each time-tracker entry is one element of a
`Value::Seq` (or a block-level child), the existing generic merge
already handles concurrent appends. `@task` is also just an ordinary
content element (like `@card`/`@callout`), not a structurally special
one the merge algorithm itself produces/consumes (unlike `@conflict`,
which *is* the conflict representation) -- so `crates/merge` likely
needs **zero changes** to support it. Only revisit this if `tomet`'s
shipped `@task` shape turns out to need semantics the generic
map/seq-by-key/position merge can't express (e.g. a field that should
sum/max across sides rather than pick one) -- not assumed, just a
check once the real shape is known.

## File-change footprint estimate, using the @conflict precedent

`@mobile.conflict` -> std `@conflict` is the one precedent in this repo
for "a tomet-std vocabulary promotion changes what immermemo consumes."
Actual diffstat, for calibration:

- `c45a737` (data-model/merge pipeline): 11 real files --
  `crates/merge/{lib.rs,markers.rs,merge.rs,resolve.rs,
  find_conflicts.rs,examples/demo.rs}`, `crates/tomet-render/src/lib.rs`,
  `crates/editor/src/conflicts.rs`, `crates/sync/src/lib.rs`, plus
  deleting `vocab/mobile.vocabulary.tmt` (39 lines, since the
  namespaced declaration was no longer needed).
- `e1d8c64` (view-mode UI rendering): 7 real files --
  `immermemo/src/render/{classify.rs,flow.rs,mod.rs}`,
  `immermemo/src/lib.rs`, `immermemo/examples/snap.rs`,
  `immermemo/ui/screens/rendered_block.slint`,
  `immermemo/ui/sheets/settings.slint`.
- `d9e6f53` (small UI follow-up): 2 files.

~20 files total, but that commit's `crates/merge` weight came
specifically from `@conflict` being the merge algorithm's own special
element (three-marker scheme torn out and rebuilt). `@task` is an
ordinary element, so that whole `crates/merge` side is expected to
shrink to ~0 files (see above). The realistic remaining surface, once
`tomet` ships std `@task`:

- `crates/tomet-render/src/lib.rs` -- 1 file, optional: classify
  `@task` into its own `RenderItem` case instead of falling back to
  generic unknown-element handling. Skippable at first; only buys a
  nicer chip, not functionality.
- `immermemo/src/render/{classify.rs,flow.rs,mod.rs}` +
  `immermemo/ui/screens/*.slint` (new component) -- the bulk of the
  work, comparable in size to `e1d8c64` (5-8 files). This is new
  feature code (the task card, the time-tracker UI), not "catching up"
  to tomet -- it would be the same size regardless of whether `@task`
  is std or immermemo's own namespaced element.
- `crates/index` -- only if a cross-vault "all tasks" listing is
  wanted (see open question below); otherwise 0 files.
- No `vocab/mobile.vocabulary.tmt` work at all, since the element
  would ship from `tomet` itself -- this removes what used to be this
  file's step 3.

Exact count still depends on `tomet`'s actual shipped `@task` shape
(field names, whether time entries are part of std's own design or an
immermemo-only data convention layered on top) -- not decided upstream
yet, so this is a calibrated estimate, not a final number.

## Open design questions (resolve once tomet's @task shape is known)

1. **Where does the time-tracker's own data live?** `tomet`'s std
   `@task` is unlikely to define time-tracking fields itself (that's
   app-specific, not a general note-taking primitive) -- most likely
   immermemo stores time entries under `@task`'s own `{data}` using
   keys it defines itself, same way any app layers extra `{data}` onto
   a std element. Confirm once `@task`'s actual shape ships; don't
   assume tomet will define this part.
2. **In-progress timer state**: live in the note's `{data}` (visible/
   synced across devices, relies on the merge behavior confirmed
   above) vs. device-local (`crates/vault`/appdata, only a *completed*
   entry gets written to the note). The device-local option forgoes a
   live cross-device "currently running" indicator but is simpler and
   matches `crates/identity`'s existing device-local-state precedent.
   Not decided -- ask the user once this is picked back up.
3. **Status model**: open/done only, or richer? Decide once `@task`'s
   own std fields are known, so immermemo's own `{data}` additions
   don't duplicate something tomet already provides.
4. **Where "all tasks" are listed**: a dedicated view, or folded into
   existing search/index? Only matters if a cross-vault listing (not
   just per-note display) is wanted -- determines whether
   `crates/index` needs schema changes at all.

## Status

**Blocked on upstream**: waiting for `tomet` to actually ship `@task`
as std vocabulary. Nothing here starts until that lands and its real
shape (field names, args) is known -- the estimate above is for
scoping/planning only, not a green light to start guessing at the
shape and building against it.

## Discrete steps (once tomet ships std @task)

1. [ ] Re-read this file; confirm the footprint estimate above still
   holds against `tomet`'s actual shipped `@task` (field names, args) --
   update the open design questions if the real shape changes the
   calculus (e.g. if `tomet` *does* define time-tracking-shaped fields
   itself, question 1 below is answered for free).
2. [ ] Settle the remaining open design questions with the user.
3. [ ] Create `.agents/workspaces/task-memo-time-tracker` via `jj
   workspace add` (non-trivial task, per `AGENTS.md`).
4. [ ] Bump the `tomet` git dependency to the commit that ships std
   `@task` (same step `c45a737` took for std `@conflict`).
5. [ ] `crates/merge`: per the analysis above, expect no changes --
   verify with a real `.tmt` fixture round-trip (two devices each
   appending a different task/time-entry, confirm a clean merge)
   before concluding no code is needed, rather than assuming.
6. [ ] `crates/tomet-render/src/lib.rs`: classify `@task` into its own
   `RenderItem` case (optional first pass, see estimate above).
7. [ ] `immermemo/src/render/{classify.rs,flow.rs,mod.rs}` + new
   Slint component under `immermemo/ui/`: task-card rendering and
   time-tracker UI -- `Palette` tokens only (no hardcoded hex), correct
   `VerticalLayout { alignment: center; }` wrapping for any
   fixed-height child placed in a `HorizontalLayout` (per `AGENTS.md`'s
   UI conventions; the latter is a recurring mistake per memory --
   match the sibling's real span, not just wrap and assume).
8. [ ] `crates/index`: only if the "all tasks" listing question below
   calls for cross-vault queries -- add the minimal schema/query
   needed.
9. [ ] `cargo build && cargo test --workspace`; exercise the UI with
   `just run`/the app's own verification story before calling this done
   (per `AGENTS.md`, UI changes need to actually be clicked through).
10. [ ] `just docs` / `just docs-check` if `tmtroot/design.tmt` changed.
11. [ ] Fold anything worth keeping into crate `//!` docs or
    `tmtroot/design.tmt`, then delete this file. Do not delete without
    folding -- that's how the decisions above get lost.
12. [ ] `jj workspace forget task-memo-time-tracker` and remove the
    workspace directory.
