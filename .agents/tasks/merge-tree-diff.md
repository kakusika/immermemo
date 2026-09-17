# core/merge: implement the tree-diff3 algorithm

Design reference: `core/merge/src/lib.rs` module doc, and the chat
discussion recorded in `docs/design.md`.

## Steps

1. [x] Generic slice-level diff3 (`core/merge/src/diff3.rs`, `merge3`).
   Tested against plain `Vec<char>` fixtures, independent of `tomet_ast`.
   Found and fixed one real bug during testing: adjacent-but-disjoint
   hunks (one ending exactly where another begins) were being merged
   into a false conflict by the interval-grouping step; fixed by
   `touches()`, which only joins on a true overlap or two insertions at
   the exact same empty gap.
2. [ ] Recurse into a both-changed region that aligns 1:1 to narrow the
   conflict below whole-block granularity (`Block::Element` ->
   `args`/`value`/`content`/`children`, `ElementValue::Group` entries by
   key, `Value::Map` by key, `Value::Seq` positionally, bottoming out at
   leaf `Value`/`Text`). **Not started.** `merge()` currently only diffs
   `Document.blocks` as one flat sequence, so every conflict today spans
   whole blocks.
3. [x] Resolved, and it changed the vocabulary: `@mobile.conflict` cannot
   wrap multiple blocks as `[content]` or as element children, because
   no `.tmt` syntax outside of list items ever populates
   `Element.children`, and `[content]` always parses to `Vec<Inline>`,
   never `Vec<Block>` -- confirmed by grepping `tomet-syntax-parser` for
   every site that sets `children` (only `list.rs`). Switched to three
   plain marker blocks -- `@mobile.conflict(mine)`, `(theirs)`, `(end)`
   -- with the disputed content living between them by position rather
   than by nesting. `vocab/mobile.vocabulary.tmt` and the module doc are
   updated to match.
4. [x] `merge()` wired at the `Document.blocks` level: non-conflicting
   regions merge silently, conflicting regions get the three-marker
   treatment, `@use(mobile)` is added to the preamble when needed.
5. [x] `resolve()` implemented: strips a `mine`/`theirs`/`end` triple,
   keeping the chosen side (or a rewritten replacement); drops
   `@use(mobile)` once no triple remains.
6. [x] Tests for steps 1, 3, 4, 5, using `tomet_parser::parse_document`
   against real `.tmt` source (not hand-built AST) so the fixtures are
   real syntax, not just whatever shape happened to compile. 13/13
   passing (`cargo test -p immermemo-merge`).

## Status

Steps 1, 3, 4, 5 done and tested. Step 2 (recursing past whole-block
conflicts) is the remaining work -- pick this back up before relying on
conflicts being as narrow as the module doc's end-state description
promises.
