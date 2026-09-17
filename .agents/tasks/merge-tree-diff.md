# core/merge: implement the tree-diff3 algorithm

Design reference: `core/merge/src/lib.rs` module doc, and the chat
discussion recorded in `docs/design.md`.

## Steps

1. [x] Generic slice-level diff3 (`diff3.rs`, `merge3`). Tested against
   plain `Vec<char>` fixtures. Found and fixed a real bug: adjacent-but-
   disjoint hunks were merged into a false conflict; fixed by `touches()`.
2. [x] Recurse into a both-changed region that aligns 1:1, into *every*
   part of the node independently (`merge_one` in `lib.rs`):
   - `content: Vec<Inline>` -- diffed inline-by-inline
     (`merge_inline_seq`); a single-`Text`-per-side conflict recurses
     once more into a character-level diff3 (`merge_text`). Two edits to
     different words in one sentence now merge silently.
   - `children: Vec<Block>` -- recurses through `merge_block_seq`, the
     same function the top level uses, so a conflict inside a
     container's children dissolves exactly as it would at the top.
   - `args` / `{data}` (`Element.value`'s `Group`) -- recurse by key
     (`merge_value`/`merge_value_map`/`merge_value_seq`): editing two
     different keys never conflicts, and nested `Value::Map`/`Value::Seq`
     recurse further. Only narrows a `{data}` group made of plain
     `key: value` pairs; one holding a nested element isn't narrowed
     (would need a fourth placement context for a conflict marker, not
     designed).
   `connects` is explicitly guarded (bails to whole-block conflict if it
   differs) rather than silently kept from `local` -- an early version of
   this recursion did exactly that and would have silently dropped a real
   difference.
3. [x] Resolved (see the design correction below): `@mobile.conflict`
   cannot wrap multiple blocks as `[content]` or as element children,
   because no `.tmt` syntax outside list items ever populates
   `Element.children`, and `[content]` always parses to `Vec<Inline>`,
   never `Vec<Block>`. Switched to three plain marker nodes --
   `@mobile.conflict(mine)`, `(theirs)`, `(end)` -- with the disputed
   content living between them by position rather than by nesting.

   **Second correction, found while testing step 2's `{data}` work**: a
   value-level conflict was first designed as
   `@mobile.conflict{ mine: ..., theirs: ... }` (using `{value}`), which
   fails to parse -- an element embedded in a value position may only
   carry `(args)`, confirmed by the parser's own error message. Switched
   to `@mobile.conflict(mine: ..., theirs: ...)` (named args, which parse
   to the same `Value::Map` shape `{data}` would have used). Both
   corrections came from testing round-trips through the real
   `tomet_parser`/`tomet_printer` rather than trusting hand-built AST
   shapes -- worth repeating for any future placement of this element.
4. [x] `merge()` wired at the `Document.blocks` level (`merge_block_seq`,
   reused for `children` too), `@use(mobile)` added to the preamble when
   needed.
5. [x] `resolve()` implemented, symmetric to `merge`: `resolve_block_seq`
   (blocks/children), `resolve_block` (visits args/content/children/value
   in the same order `merge_one` recurses in, so resolutions line up),
   `resolve_inline_seq`, `resolve_value`/`resolve_element_value`. Also
   extended `has_conflicts` to recurse into args/value/children, not just
   block/inline markers -- needed so `resolve` doesn't strip `@use(mobile)`
   while a value-level conflict is still sitting unresolved.
6. [x] Tests using `tomet_parser::parse_document` against real `.tmt`
   source throughout. 21/21 passing in `immermemo-merge`
   (`cargo test -p immermemo-merge`), 24/24 across the workspace.

## Status

Done, to the depth described in the module doc. Remaining, explicitly
out of scope for now: narrowing a `{data}` group that mixes `key: value`
pairs with nested elements (rather than only pairs).
