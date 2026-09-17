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
2. [x] Recurse into a both-changed region that aligns 1:1
   (`merge_one`/`merge_inline_seq`/`merge_text` in `lib.rs`): a
   `Paragraph`'s (or same-`sigil` `Element`'s) `content: Vec<Inline>`
   gets diffed inline-by-inline, and a single-`Text`-per-side inline
   conflict recurses once more into a character-level diff3. Two edits
   to different words in the same sentence now merge silently; a true
   same-word conflict embeds `@mobile.conflict` as inline markers inside
   the paragraph instead of duplicating the whole paragraph. Tested
   (`edits_to_different_words_in_one_sentence_merge_cleanly`,
   `edits_to_the_same_word_produce_an_inline_conflict`,
   `resolving_an_inline_conflict_leaves_one_plain_sentence`).

   **Still not narrowed**: an `Element`'s `args`/`value`/`children` --
   `merge_one` currently requires those three identical between `local`
   and `remote` before it will even attempt the `content` recursion
   (see the `mismatched_block_kinds_fall_back_to_block_level_markers`
   fallback path), so e.g. two concurrent edits to different `{data}`
   keys on the same element still fall back to a whole-block conflict
   rather than dissolving. Recursing into `ElementValue::Group` by key
   and `Value::Map`/`Value::Seq` is the next increment here, and needs
   its own decision on how to represent a conflict inside a `{data}`
   value (a value slot can't hold a marker block/inline the way
   `Document.blocks`/`Paragraph.content` can -- this is the same kind of
   representation question step 3 below already hit once).
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

Steps 1, 3, 4, 5 done and tested; step 2 done one level deep (paragraph/
inline/character). 16/16 tests passing (`cargo test -p immermemo-merge`).

Remaining: recursing into an `Element`'s `args`/`value`/`children`
(see step 2's note above) -- needs a design decision on representing a
conflict inside a `{data}` value slot before it can be implemented, not
just more code in the same shape as what's here.
