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
//! `merge` walks `base`, `local` and `remote` together, top-down:
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
//! 3. A child changed on *both* sides recurses: if it is itself a
//!    container, the conflict may dissolve further down -- two edits to
//!    different keys of the same `@element`'s `{data}` are still
//!    non-conflicting. Recursion bottoms out at leaf values, where
//!    "changed on both sides, to different things" cannot be narrowed
//!    any further. **Not implemented yet** -- see
//!    `.agents/tasks/merge-tree-diff.md`. Today, `merge` only diffs
//!    `Document.blocks` as one flat sequence, so a conflict always spans
//!    whole blocks even when a deeper diff could have narrowed it.
//! 4. A conflict is recorded as three marker blocks around the disputed
//!    stretch: `@mobile.conflict(mine)`, then the blocks `local` made of
//!    that stretch, then `@mobile.conflict(theirs)`, then what `remote`
//!    made of it, then `@mobile.conflict(end)`. See
//!    `vocab/mobile.vocabulary.tmt`.
//!
//! Markers, not a wrapping element, because tomet has no syntax for
//! "several blocks as one element's contents" outside of list items:
//! `[content]` is always `Vec<Inline>`, never `Vec<Block>`, so an
//! `Element` with block children is not something any `.tmt` source can
//! actually parse to today. Three plain, childless marker blocks sidestep
//! that entirely -- each is just `@mobile.conflict(side)`, ordinary
//! syntax, and the disputed content between two markers is established by
//! position instead of by nesting.
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

mod diff3;

use tomet_ast::{Block, Document, Element, Inline, Name, Paragraph, Placement, Sigil, Text, Value};

/// The outcome of merging one note across two divergent versions of a
/// vault.
#[derive(Debug, Clone, PartialEq)]
pub struct MergeResult {
    /// The merged document. Always a structurally valid Tomet document --
    /// conflicts are represented as `@mobile.conflict` markers, never as
    /// malformed or partial output.
    pub document: Document,
    /// Where merging succeeded without leaving a conflict marker behind.
    pub clean: bool,
}

/// Merges `local` and `remote`, both descendants of `base`, into one
/// document.
///
/// Never fails: a merge that cannot be resolved automatically produces a
/// document containing `@mobile.conflict` markers rather than an error.
/// The caller (`immermemo-sync`) always has something to commit.
pub fn merge(base: &Document, local: &Document, remote: &Document) -> MergeResult {
    let regions = diff3::merge3(&base.blocks, &local.blocks, &remote.blocks);

    let mut blocks = Vec::new();
    let mut clean = true;
    for region in regions {
        match region {
            diff3::Region::Same(items) => blocks.extend(items),
            diff3::Region::Conflict { mine, theirs, .. } => {
                clean = false;
                blocks.push(conflict_marker("mine"));
                blocks.extend(mine);
                blocks.push(conflict_marker("theirs"));
                blocks.extend(theirs);
                blocks.push(conflict_marker("end"));
            }
        }
    }

    if !clean && !blocks.iter().any(is_use_mobile_block) {
        blocks.insert(0, use_mobile_block());
    }

    MergeResult {
        document: Document {
            blocks,
            span: base.span,
        },
        clean,
    }
}

/// Removes every `@mobile.conflict` marker triple from `document`,
/// keeping the side the caller picked for each, and drops the
/// `@use(mobile)` preamble line once none remain.
///
/// `resolutions` are consumed in document order, one per `mine`/`theirs`/
/// `end` triple. A triple with no corresponding entry (`resolutions` ran
/// out) is left in the document untouched.
pub fn resolve(document: &Document, resolutions: &[ConflictResolution]) -> Document {
    let mut blocks = Vec::new();
    let mut resolutions = resolutions.iter();
    let mut i = 0;
    while i < document.blocks.len() {
        if conflict_side(&document.blocks[i]) != Some("mine") {
            blocks.push(document.blocks[i].clone());
            i += 1;
            continue;
        }

        let theirs_at = (i + 1..document.blocks.len())
            .find(|&j| conflict_side(&document.blocks[j]) == Some("theirs"))
            .expect("a mine marker is always followed by a theirs marker");
        let end_at = (theirs_at + 1..document.blocks.len())
            .find(|&j| conflict_side(&document.blocks[j]) == Some("end"))
            .expect("a theirs marker is always followed by an end marker");

        let mine_blocks = &document.blocks[i + 1..theirs_at];
        let theirs_blocks = &document.blocks[theirs_at + 1..end_at];

        match resolutions.next() {
            Some(ConflictResolution::Mine) => blocks.extend(mine_blocks.iter().cloned()),
            Some(ConflictResolution::Theirs) => blocks.extend(theirs_blocks.iter().cloned()),
            Some(ConflictResolution::Rewritten(text)) => blocks.push(Block::Paragraph(
                Paragraph::new(vec![Inline::Text(Text::from(text.as_str()))], Default::default()),
            )),
            None => blocks.extend(document.blocks[i..=end_at].iter().cloned()),
        }
        i = end_at + 1;
    }

    if !blocks.iter().any(is_conflict_block) {
        blocks.retain(|b| !is_use_mobile_block(b));
    }

    Document {
        blocks,
        span: document.span,
    }
}

fn conflict_marker(side: &str) -> Block {
    Block::Element(Element {
        sigil: Sigil::Named(Name::namespaced("mobile", "conflict")),
        placement: Placement::Block,
        args: Some(Value::String(side.to_string())),
        ..Default::default()
    })
}

fn use_mobile_block() -> Block {
    Block::Element(Element {
        sigil: Sigil::named("use"),
        placement: Placement::Block,
        args: Some(Value::String("mobile".to_string())),
        ..Default::default()
    })
}

/// This block's `side` argument, if it is a `@mobile.conflict(side)`
/// marker.
fn conflict_side(block: &Block) -> Option<&str> {
    let Block::Element(el) = block else {
        return None;
    };
    let Sigil::Named(name) = &el.sigil else {
        return None;
    };
    if name.namespace.as_deref() != Some("mobile") || name.name != "conflict" {
        return None;
    }
    match &el.args {
        Some(Value::String(side)) => Some(side.as_str()),
        _ => None,
    }
}

fn is_conflict_block(block: &Block) -> bool {
    conflict_side(block).is_some()
}

fn is_use_mobile_block(block: &Block) -> bool {
    let Block::Element(el) = block else {
        return false;
    };
    el.sigil.is_bare_named("use") && matches!(&el.args, Some(Value::String(ns)) if ns == "mobile")
}

/// Which side of one `@mobile.conflict` triple a user picked, or that
/// they edited a fresh replacement by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    Mine,
    Theirs,
    /// The user wrote something new that is neither side verbatim.
    Rewritten(String),
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(
            result.document,
            doc("Alpha changed.\n\nBeta changed.\n")
        );
    }

    #[test]
    fn same_paragraph_conflict_produces_markers() {
        let base = doc("Original.\n");
        let local = doc("Mine.\n");
        let remote = doc("Theirs.\n");

        let result = merge(&base, &local, &remote);

        assert!(!result.clean);
        assert_eq!(
            result.document,
            doc(concat!(
                "@use(mobile)\n\n",
                "@mobile.conflict(mine)\n\n",
                "Mine.\n\n",
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
    fn resolve_leaves_the_use_line_while_another_conflict_remains() {
        // An unchanged paragraph in the middle gives the LCS an anchor,
        // so this is two independent conflicts rather than one hunk
        // spanning the whole document.
        let base = doc("One.\n\nAnchor.\n\nTwo.\n");
        let local = doc("Mine one.\n\nAnchor.\n\nMine two.\n");
        let remote = doc("Theirs one.\n\nAnchor.\n\nTheirs two.\n");
        let merged = merge(&base, &local, &remote).document;

        let resolved = resolve(
            &merged,
            &[ConflictResolution::Mine, ConflictResolution::Theirs],
        );

        assert_eq!(resolved, doc("Mine one.\n\nAnchor.\n\nTheirs two.\n"));
    }
}
