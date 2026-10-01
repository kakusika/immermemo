//! Three-way merge for `.tmt` notes, at the AST level rather than the text
//! level.
//!
//! A textual `diff3` treats a note as lines. Tomet documents are trees of
//! `@element(args)[content]{data}` nodes, and most real edits touch
//! different nodes -- editing one `{data}` key on a device while another
//! device edits an unrelated block's `[content]` is not a conflict in any
//! meaningful sense, even though the two edits may land on adjacent lines.
//! Diffing the tree instead of the text is what lets those edits merge
//! silently, which is the entire point of doing this instead of shelling
//! out to `git merge-file`.
//!
//! # Algorithm
//!
//! [`merge`] walks `base`, `local` and `remote` together, top-down:
//!
//! 1. At each level, align the child sequence of `base` against `local`
//!    and against `remote` with an LCS-style alignment (the same technique
//!    a line-based diff uses, applied to sibling nodes instead of lines).
//!    Alignment is structural -- two nodes are "the same" position if they
//!    are AST-equal, not by any synthetic identity tomet would have to
//!    invent and inject into the user's document.
//! 2. A child unchanged on one side and changed on the other resolves to
//!    the changed side. A child added on only one side is kept. A child
//!    removed on only one side is dropped.
//! 3. A single base block changed on *both* sides recurses via
//!    `merge_one`, into every part of it independently:
//!    - `content: Vec<Inline>` (a `Paragraph`'s, or an `Element`'s) gets
//!      the same diff3 treatment as step 1, just over inline runs; a
//!      single-`Text`-per-side inline conflict recurses once more,
//!      character by character (`merge_text`). This is what lets two
//!      edits to different words in the same sentence merge silently.
//!    - `children: Vec<Block>` recurses through this very same
//!      algorithm (`merge_block_seq`), so a conflict inside a
//!      container's children can dissolve exactly as it would at the
//!      top level.
//!    - `args`/`{data}` (`Element.value`'s `Group`) recurse by key
//!      (`merge_value`/`merge_value_map`): editing two different
//!      keys never conflicts, and a `Value::Map`/`Value::Seq` inside a
//!      value recurses further still. This only narrows a `{data}`
//!      group made of plain `key: value` pairs -- one holding a nested
//!      element (rather than only pairs) isn't narrowed
//!      (`.agents/tasks/merge-tree-diff.md`).
//!    Each part narrows independently; the recursion only gives up on
//!    the *whole* block (step 4) when at least one part's shape itself
//!    differs (e.g. `[content]` present on one side and absent on the
//!    other) rather than merely disagreeing in value.
//! 4. A conflict `merge_one` can't narrow any further is recorded
//!    in-place, using the same element in two different roles depending
//!    on where it sits:
//!    - **As a sequence of blocks/inlines**, it becomes three marker
//!      nodes around the disputed stretch: `@mobile.conflict(mine)`,
//!      then whatever `local` made of that stretch,
//!      `@mobile.conflict(theirs)`, then what `remote` made of it,
//!      `@mobile.conflict(end)`. Markers, not a wrapping element,
//!      because tomet has no syntax for "several blocks as one
//!      element's contents" outside of list items -- `[content]` is
//!      always `Vec<Inline>`, never `Vec<Block>`. At the
//!      `Document.blocks` level these are block markers; inside a
//!      paragraph's `content` they're the same element used inline
//!      (`vocab/mobile.vocabulary.tmt` leaves `display` unset for
//!      exactly this reason).
//!    - **As a single value** (a `{data}` key, or `args`), it becomes
//!      `@mobile.conflict(mine: ..., theirs: ...)` sitting where the
//!      value went, using named args rather than `{value}` -- a value
//!      slot only permits `(args)` on an embedded element, confirmed the
//!      hard way when a first draft that used `{value}` here failed to
//!      parse back.
//!
//! If the merged document ends up with at least one `@mobile.conflict`,
//! `merge` also ensures `@use(mobile)` is present in the preamble (adding
//! it if missing); [`resolve`] removes it again once no
//! `@mobile.conflict` remains, so a note that has never conflicted never
//! carries the `@use` line.
//!
//! # What this crate assumes
//!
//! A `base` is always available: `immermemo-sync` only calls into this
//! crate for an actual three-way git merge, where `git2` has already found
//! a merge base commit. Linking two vaults that do not share history is a
//! separate operation (vault adoption), not a merge, and is not this
//! crate's problem.
//!
//! # Module layout
//!
//! - [`merge`] (the module, via [`merge()`] the function): the merge half
//!   of the algorithm above.
//! - [`resolve`]: strips markers back out once a side has been picked.
//! - [`find_conflicts`]: reads markers back out without removing them, for
//!   showing the user what's in conflict.
//! - `markers`: the `@mobile.conflict` marker shape itself (construction
//!   and detection), shared by all three.
//! - `diff3`: the plain three-way sequence alignment every level of the
//!   tree reuses; has no idea it's being used for an AST.

mod diff3;
mod find_conflicts;
mod markers;
mod merge;
mod resolve;

pub use find_conflicts::{ConflictItem, find_conflicts, has_conflicts};
pub use markers::CONFLICT_MARKER;
pub use merge::{MergeResult, merge};
pub use resolve::{ConflictResolution, resolve, resolve_all, resolve_single};

#[cfg(test)]
mod tests {
    use super::*;
    use tomet_ast::{Block, Document};
    use tomet_parser::parse_document;

    fn doc(src: &str) -> Document {
        parse_document(src).unwrap_or_else(|e| panic!("failed to parse {src:?}: {e}"))
    }

    #[test]
    fn disjoint_edits_merge_cleanly() {
        let base = doc("Alpha.\n\nBeta.\n");
        let local = doc("Alpha changed.\n\nBeta.\n");
        let remote = doc("Alpha.\n\nBeta changed.\n");

        let result = merge(&base, &local, &remote);

        assert!(result.clean);
        assert_eq!(result.document, doc("Alpha changed.\n\nBeta changed.\n"));
    }

    #[test]
    fn mismatched_block_kinds_fall_back_to_block_level_markers() {
        // local turns the block into a different kind of node entirely
        // (an element, not a paragraph) -- merge_one can't narrow across
        // a change in node kind, so this exercises the whole-block
        // marker fallback rather than the inline recursion.
        let base = doc("Original.\n");
        let local = doc("@meta{ x: 1 }\n");
        let remote = doc("Theirs.\n");

        let result = merge(&base, &local, &remote);

        assert!(!result.clean);
        assert_eq!(
            result.document,
            doc(concat!(
                "@use(mobile)\n\n",
                "@mobile.conflict(mine)\n\n",
                "@meta{ x: 1 }\n\n",
                "@mobile.conflict(theirs)\n\n",
                "Theirs.\n\n",
                "@mobile.conflict(end)\n",
            ))
        );
    }

    #[test]
    fn resolve_mine_drops_theirs_and_the_use_line() {
        let base = doc("Original.\n");
        let local = doc("Mine.\n");
        let remote = doc("Theirs.\n");
        let merged = merge(&base, &local, &remote).document;

        let resolved = resolve(&merged, &[ConflictResolution::Mine]);

        assert_eq!(resolved, doc("Mine.\n"));
    }

    #[test]
    fn resolve_theirs_drops_mine_and_the_use_line() {
        let base = doc("Original.\n");
        let local = doc("Mine.\n");
        let remote = doc("Theirs.\n");
        let merged = merge(&base, &local, &remote).document;

        let resolved = resolve(&merged, &[ConflictResolution::Theirs]);

        assert_eq!(resolved, doc("Theirs.\n"));
    }

    #[test]
    fn edits_to_different_words_in_one_sentence_merge_cleanly() {
        let base = doc("The quick fox jumps.\n");
        let local = doc("The slow fox jumps.\n");
        let remote = doc("The quick fox runs.\n");

        let result = merge(&base, &local, &remote);

        assert!(result.clean);
        assert_eq!(result.document, doc("The slow fox runs.\n"));
    }

    #[test]
    fn edits_to_the_same_word_produce_an_inline_conflict() {
        // The exact character split isn't asserted here -- that's an
        // implementation detail of the LCS alignment, not a contract --
        // only that it's flagged unclean and stays a single paragraph
        // (the point of narrowing to the inline level at all) rather
        // than duplicating the whole sentence as block-level markers.
        // `resolving_an_inline_conflict_leaves_one_plain_sentence` below
        // is what actually pins down correctness, by checking what
        // resolving each side produces.
        let base = doc("The quick fox jumps.\n");
        let local = doc("The slow fox jumps.\n");
        let remote = doc("The lazy fox jumps.\n");

        let result = merge(&base, &local, &remote);

        assert!(!result.clean);
        assert_eq!(
            result.document.blocks.len(),
            2,
            "@use(mobile) + one paragraph"
        );
        assert!(matches!(result.document.blocks[1], Block::Paragraph(_)));
    }

    #[test]
    fn resolving_an_inline_conflict_leaves_one_plain_sentence() {
        let base = doc("The quick fox jumps.\n");
        let local = doc("The slow fox jumps.\n");
        let remote = doc("The lazy fox jumps.\n");
        let merged = merge(&base, &local, &remote).document;

        let resolved = resolve(&merged, &[ConflictResolution::Theirs]);

        assert_eq!(resolved, doc("The lazy fox jumps.\n"));
    }

    #[test]
    fn resolve_all_resolves_every_conflict_and_drops_use_line() {
        let base = doc("Zzz.\n\nAnchor.\n\nQqq.\n");
        let local = doc("Mmm.\n\nAnchor.\n\nNnn.\n");
        let remote = doc("Ppp.\n\nAnchor.\n\nRrr.\n");
        let merged = merge(&base, &local, &remote).document;

        let resolved_mine = resolve_all(&merged, ConflictResolution::Mine);
        assert_eq!(resolved_mine, doc("Mmm.\n\nAnchor.\n\nNnn.\n"));

        let resolved_theirs = resolve_all(&merged, ConflictResolution::Theirs);
        assert_eq!(resolved_theirs, doc("Ppp.\n\nAnchor.\n\nRrr.\n"));
    }

    #[test]
    fn resolve_leaves_the_use_line_while_another_conflict_remains() {
        // An unchanged paragraph in the middle gives the LCS an anchor,
        // so this is two independent conflicts rather than one hunk
        // spanning the whole document. The two conflicting paragraphs
        // share no characters with each other (besides the trailing
        // "."), on purpose -- so the character-level recursion can't
        // find a partial anchor and muddle the two together, which kept
        // this fixture's first draft (reusing "One"/"Two" inside the
        // replacement text) producing a garbled cross-conflict result.
        let base = doc("Zzz.\n\nAnchor.\n\nQqq.\n");
        let local = doc("Mmm.\n\nAnchor.\n\nNnn.\n");
        let remote = doc("Ppp.\n\nAnchor.\n\nRrr.\n");
        let merged = merge(&base, &local, &remote).document;

        let resolved = resolve(
            &merged,
            &[ConflictResolution::Mine, ConflictResolution::Theirs],
        );

        assert_eq!(resolved, doc("Mmm.\n\nAnchor.\n\nRrr.\n"));
    }

    #[test]
    fn edits_to_different_data_keys_merge_cleanly() {
        let base = doc("@meta{ x: 1, y: 2 }\n");
        let local = doc("@meta{ x: 10, y: 2 }\n");
        let remote = doc("@meta{ x: 1, y: 20 }\n");

        let result = merge(&base, &local, &remote);

        assert!(result.clean);
        assert_eq!(result.document, doc("@meta{ x: 10, y: 20 }\n"));
    }

    #[test]
    fn editing_the_same_data_key_produces_a_value_conflict() {
        let base = doc("@meta{ x: 1 }\n");
        let local = doc("@meta{ x: 2 }\n");
        let remote = doc("@meta{ x: 3 }\n");

        let result = merge(&base, &local, &remote);

        assert!(!result.clean);
        assert_eq!(
            result.document,
            doc("@use(mobile)\n\n@meta{ x: @mobile.conflict(mine: 2, theirs: 3) }\n")
        );
    }

    #[test]
    fn resolving_a_data_key_conflict_leaves_a_plain_value() {
        let base = doc("@meta{ x: 1 }\n");
        let local = doc("@meta{ x: 2 }\n");
        let remote = doc("@meta{ x: 3 }\n");
        let merged = merge(&base, &local, &remote).document;

        let resolved = resolve(&merged, &[ConflictResolution::Theirs]);

        assert_eq!(resolved, doc("@meta{ x: 3 }\n"));
    }

    #[test]
    fn a_key_deleted_on_one_side_and_untouched_on_the_other_is_dropped() {
        let base = doc("@meta{ x: 1, y: 2 }\n");
        let local = doc("@meta{ y: 2 }\n"); // local deleted x
        let remote = doc("@meta{ x: 1, y: 2 }\n"); // remote left it alone

        let result = merge(&base, &local, &remote);

        assert!(result.clean);
        assert_eq!(result.document, doc("@meta{ y: 2 }\n"));
    }

    #[test]
    fn a_key_deleted_on_one_side_and_modified_on_the_other_keeps_the_edit() {
        let base = doc("@meta{ x: 1 }\n");
        let local = doc("@meta{}\n"); // local deleted x
        let remote = doc("@meta{ x: 9 }\n"); // remote modified it

        let result = merge(&base, &local, &remote);

        assert!(result.clean);
        assert_eq!(result.document, doc("@meta{ x: 9 }\n"));
    }

    #[test]
    fn find_conflicts_and_resolve_single_and_both() {
        let base = doc("Alpha.\n\nKeep.\n\nBeta.\n");
        let local = doc("Alpha local.\n\nKeep.\n\nBeta local.\n");
        let remote = doc("Alpha remote.\n\nKeep.\n\nBeta remote.\n");

        let merged = merge(&base, &local, &remote);
        assert!(!merged.clean);

        let conflicts = find_conflicts(&merged.document);
        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0].mine, "l");
        assert_eq!(conflicts[0].theirs, " remote");
        assert_eq!(conflicts[1].mine, "l");
        assert_eq!(conflicts[1].theirs, " remote");

        // Resolve only first conflict with Mine
        let resolved_first = resolve_single(&merged.document, 0, ConflictResolution::Mine);
        let remaining = find_conflicts(&resolved_first);
        assert_eq!(remaining.len(), 1);
        let text_after_first = tomet_printer::document_to_tm(&resolved_first);
        assert!(text_after_first.contains("Alpha local."));
        assert!(!text_after_first.contains("Alpha remote."));

        // Resolve remaining conflict with Both
        let resolved_all = resolve_single(&resolved_first, 0, ConflictResolution::Both);
        assert!(find_conflicts(&resolved_all).is_empty());
        let final_text = tomet_printer::document_to_tm(&resolved_all);
        assert!(final_text.contains("Alpha local."));
        assert!(final_text.contains("Keep."));
        assert!(final_text.contains("Beta local remote."));
    }
}
