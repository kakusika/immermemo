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
//! 3. A single base block changed on *both* sides recurses one level,
//!    via [`merge_one`]: a `Paragraph`'s `content: Vec<Inline>` (and an
//!    `Element`'s, when both sides kept the same `args`/`value`/
//!    `children`) gets the same diff3 treatment as step 1, just over
//!    inline runs instead of blocks. Two edits to different words in the
//!    same paragraph merge silently this way. Deeper recursion --
//!    dissolving a conflict inside an `Element`'s `{data}` keys or
//!    `children`, rather than requiring them identical on both sides to
//!    even attempt the paragraph-content merge -- is not implemented yet
//!    (`.agents/tasks/merge-tree-diff.md`).
//! 4. A conflict [`merge_one`] can't narrow any further is recorded as
//!    three marker nodes around the disputed stretch:
//!    `@mobile.conflict(mine)`, then whatever `local` made of that
//!    stretch, then `@mobile.conflict(theirs)`, then what `remote` made
//!    of it, then `@mobile.conflict(end)`. At the `Document.blocks`
//!    level these are block markers; inside a paragraph's `content`
//!    they're the same element used inline (`vocab/mobile.vocabulary.tmt`
//!    leaves `display` unset for exactly this reason), so a word-level
//!    conflict shows up inside its paragraph instead of duplicating the
//!    whole paragraph.
//!
//! Markers, not a wrapping element, because tomet has no syntax for
//! "several blocks as one element's contents" outside of list items:
//! `[content]` is always `Vec<Inline>`, never `Vec<Block>`, so an
//! `Element` with block children is not something any `.tmt` source can
//! actually parse to today. Three plain marker nodes sidestep that
//! entirely -- each is just `@mobile.conflict(side)`, ordinary syntax,
//! and the disputed content between two markers is established by
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
            diff3::Region::Conflict { base, mine, theirs } => {
                let narrowed = match (base.as_slice(), mine.as_slice(), theirs.as_slice()) {
                    ([b], [m], [t]) => merge_one(b, m, t),
                    _ => None,
                };
                if let Some((block, sub_clean)) = narrowed {
                    blocks.push(block);
                    clean &= sub_clean;
                    continue;
                }
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

/// Attempts to narrow a conflict between one base block and the single
/// block each side made of it, instead of giving up on the whole block.
///
/// `None` means recursion can't help here -- the caller falls back to
/// wrapping the whole block in marker nodes. `Some((block, clean))`
/// means it produced a merged block; `clean` says whether that block
/// still had to embed its own (narrower) conflict markers.
fn merge_one(base: &Block, local: &Block, remote: &Block) -> Option<(Block, bool)> {
    match (base, local, remote) {
        (Block::Paragraph(b), Block::Paragraph(l), Block::Paragraph(r)) => {
            let (content, clean) = merge_inline_seq(&b.content, &l.content, &r.content);
            Some((Block::Paragraph(Paragraph::new(content, b.span)), clean))
        }
        (Block::Element(b), Block::Element(l), Block::Element(r))
            if b.sigil == l.sigil && l.sigil == r.sigil =>
        {
            // args/value/children aren't diffed yet (see the module
            // doc); only content can narrow, and only when neither side
            // touched anything else about this element.
            if l.args != r.args || l.value != r.value || l.children != r.children {
                return None;
            }
            let (content, clean) = match (&b.content, &l.content, &r.content) {
                (Some(bc), Some(lc), Some(rc)) => {
                    let (merged, clean) = merge_inline_seq(bc, lc, rc);
                    (Some(merged), clean)
                }
                (None, None, None) => (None, true),
                // One side added or removed [content] outright -- not a
                // shape recursion can narrow.
                _ => return None,
            };
            Some((
                Block::Element(Element {
                    content,
                    ..l.clone()
                }),
                clean,
            ))
        }
        _ => None,
    }
}

/// The inline-level counterpart of `merge`'s main loop: diffs one
/// `Vec<Inline>` three ways and always succeeds, embedding
/// `@mobile.conflict` markers (as inline nodes) around anything it can't
/// resolve rather than failing outward to a whole-block conflict.
///
/// A run of plain prose parses to one `Inline::Text` covering the whole
/// run, so without a further step, editing any word anywhere in a
/// sentence would make the entire sentence one non-matching item and
/// conflict outright. When a conflict region narrows to exactly one
/// `Inline::Text` per side, [`merge_text`] recurses once more, character
/// by character, which is what actually lets two edits to different
/// words in the same sentence merge silently.
fn merge_inline_seq(base: &[Inline], local: &[Inline], remote: &[Inline]) -> (Vec<Inline>, bool) {
    let mut out = Vec::new();
    let mut clean = true;
    for region in diff3::merge3(base, local, remote) {
        match region {
            diff3::Region::Same(items) => out.extend(items),
            diff3::Region::Conflict { base, mine, theirs } => {
                let narrowed = match (base.as_slice(), mine.as_slice(), theirs.as_slice()) {
                    ([Inline::Text(b)], [Inline::Text(l)], [Inline::Text(r)]) => {
                        Some(merge_text(&b.value, &l.value, &r.value))
                    }
                    _ => None,
                };
                if let Some((items, sub_clean)) = narrowed {
                    out.extend(items);
                    clean &= sub_clean;
                    continue;
                }
                clean = false;
                out.push(inline_conflict_marker("mine"));
                out.extend(mine);
                out.push(inline_conflict_marker("theirs"));
                out.extend(theirs);
                out.push(inline_conflict_marker("end"));
            }
        }
    }
    (coalesce_text(out), clean)
}

/// Character-level three-way merge of one `Inline::Text` run, the bottom
/// of the recursion: nothing below a character can narrow a conflict any
/// further.
fn merge_text(base: &str, local: &str, remote: &str) -> (Vec<Inline>, bool) {
    let (b, l, r): (Vec<char>, Vec<char>, Vec<char>) = (
        base.chars().collect(),
        local.chars().collect(),
        remote.chars().collect(),
    );

    let mut out = Vec::new();
    let mut clean = true;
    for region in diff3::merge3(&b, &l, &r) {
        match region {
            diff3::Region::Same(chars) => push_text(&mut out, chars),
            diff3::Region::Conflict { mine, theirs, .. } => {
                clean = false;
                out.push(inline_conflict_marker("mine"));
                push_text(&mut out, mine);
                out.push(inline_conflict_marker("theirs"));
                push_text(&mut out, theirs);
                out.push(inline_conflict_marker("end"));
            }
        }
    }
    (coalesce_text(out), clean)
}

fn push_text(out: &mut Vec<Inline>, chars: Vec<char>) {
    if !chars.is_empty() {
        out.push(Inline::Text(Text::from(chars.into_iter().collect::<String>())));
    }
}

/// Merges adjacent `Inline::Text` runs into one. Diffing and resolving
/// both build their output as separate pieces (a `Same` run here, a
/// `mine`/`theirs` pick there), which otherwise leaves needlessly
/// fragmented text behind even when nothing about the *content*
/// disagrees.
fn coalesce_text(items: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Inline::Text(t) => match out.last_mut() {
                Some(Inline::Text(prev)) => prev.value.push_str(&t.value),
                _ => out.push(Inline::Text(t)),
            },
            other => out.push(other),
        }
    }
    out
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
        if conflict_side(&document.blocks[i]) == Some("mine") {
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
            continue;
        }

        blocks.push(resolve_block_inline(&document.blocks[i], &mut resolutions));
        i += 1;
    }

    if !blocks.iter().any(document_or_block_has_conflict) {
        blocks.retain(|b| !is_use_mobile_block(b));
    }

    Document {
        blocks,
        span: document.span,
    }
}

/// Resolves any `@mobile.conflict` markers embedded in a block's own
/// inline content (a `Paragraph`, or an `Element` that carries
/// `[content]`); leaves every other block untouched.
fn resolve_block_inline<'a>(
    block: &Block,
    resolutions: &mut impl Iterator<Item = &'a ConflictResolution>,
) -> Block {
    match block {
        Block::Paragraph(p) => Block::Paragraph(Paragraph::new(
            resolve_inline_seq(&p.content, resolutions),
            p.span,
        )),
        Block::Element(e) => {
            let mut e = e.clone();
            if let Some(content) = &e.content {
                e.content = Some(resolve_inline_seq(content, resolutions));
            }
            Block::Element(e)
        }
    }
}

/// The inline-level counterpart of `resolve`'s main loop.
fn resolve_inline_seq<'a>(
    content: &[Inline],
    resolutions: &mut impl Iterator<Item = &'a ConflictResolution>,
) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < content.len() {
        if inline_conflict_side(&content[i]) != Some("mine") {
            out.push(content[i].clone());
            i += 1;
            continue;
        }

        let theirs_at = (i + 1..content.len())
            .find(|&j| inline_conflict_side(&content[j]) == Some("theirs"))
            .expect("a mine marker is always followed by a theirs marker");
        let end_at = (theirs_at + 1..content.len())
            .find(|&j| inline_conflict_side(&content[j]) == Some("end"))
            .expect("a theirs marker is always followed by an end marker");

        let mine_run = &content[i + 1..theirs_at];
        let theirs_run = &content[theirs_at + 1..end_at];

        match resolutions.next() {
            Some(ConflictResolution::Mine) => out.extend(mine_run.iter().cloned()),
            Some(ConflictResolution::Theirs) => out.extend(theirs_run.iter().cloned()),
            Some(ConflictResolution::Rewritten(text)) => {
                out.push(Inline::Text(Text::from(text.as_str())))
            }
            None => out.extend(content[i..=end_at].iter().cloned()),
        }
        i = end_at + 1;
    }
    coalesce_text(out)
}

/// Whether this block still carries a conflict, at the block level or
/// nested in its own inline content.
fn document_or_block_has_conflict(block: &Block) -> bool {
    if is_conflict_block(block) {
        return true;
    }
    let content = match block {
        Block::Paragraph(p) => Some(&p.content),
        Block::Element(e) => e.content.as_ref(),
    };
    content.is_some_and(|c| c.iter().any(|i| inline_conflict_side(i).is_some()))
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

fn inline_conflict_marker(side: &str) -> Inline {
    Inline::Element(Element {
        sigil: Sigil::Named(Name::namespaced("mobile", "conflict")),
        placement: Placement::Inline,
        args: Some(Value::String(side.to_string())),
        ..Default::default()
    })
}

/// This inline node's `side` argument, if it is a
/// `@mobile.conflict(side)` marker.
fn inline_conflict_side(inline: &Inline) -> Option<&str> {
    let Inline::Element(el) = inline else {
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
        assert_eq!(result.document.blocks.len(), 2, "@use(mobile) + one paragraph");
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
}
