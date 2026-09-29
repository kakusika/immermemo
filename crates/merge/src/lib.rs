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
//! 3. A single base block changed on *both* sides recurses via
//!    [`merge_one`], into every part of it independently:
//!    - `content: Vec<Inline>` (a `Paragraph`'s, or an `Element`'s) gets
//!      the same diff3 treatment as step 1, just over inline runs; a
//!      single-`Text`-per-side inline conflict recurses once more,
//!      character by character ([`merge_text`]). This is what lets two
//!      edits to different words in the same sentence merge silently.
//!    - `children: Vec<Block>` recurses through this very same
//!      algorithm ([`merge_block_seq`]), so a conflict inside a
//!      container's children can dissolve exactly as it would at the
//!      top level.
//!    - `args`/`{data}` (`Element.value`'s `Group`) recurse by key
//!      ([`merge_value`]/[`merge_value_map`]): editing two different
//!      keys never conflicts, and a `Value::Map`/`Value::Seq` inside a
//!      value recurses further still. This only narrows a `{data}`
//!      group made of plain `key: value` pairs -- one holding a nested
//!      element (rather than only pairs) isn't narrowed
//!      (`.agents/tasks/merge-tree-diff.md`).
//!    Each part narrows independently; the recursion only gives up on
//!    the *whole* block (step 4) when at least one part's shape itself
//!    differs (e.g. `[content]` present on one side and absent on the
//!    other) rather than merely disagreeing in value.
//! 4. A conflict [`merge_one`] can't narrow any further is recorded
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

mod diff3;

use tomet_ast::{
    Block, Document, Element, ElementValue, Entry, Inline, Name, Paragraph, Placement, Sigil,
    Text, Value,
};

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
    let (mut blocks, clean) = merge_block_seq(&base.blocks, &local.blocks, &remote.blocks);

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

/// Diffs one `Vec<Block>` three ways, narrowing each conflict via
/// [`merge_one`] before falling back to whole-block marker nodes. Used
/// both for `Document.blocks` (top level) and an `Element`'s `children`
/// (recursively, from inside `merge_one` itself).
fn merge_block_seq(base: &[Block], local: &[Block], remote: &[Block]) -> (Vec<Block>, bool) {
    let mut blocks = Vec::new();
    let mut clean = true;
    for region in diff3::merge3(base, local, remote) {
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
    (blocks, clean)
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
            // `connects` isn't diffed -- bail rather than silently keep
            // `local`'s and drop a real difference on the floor.
            if l.connects != r.connects {
                return None;
            }

            let (args, args_clean) = match (&b.args, &l.args, &r.args) {
                (Some(ba), Some(la), Some(ra)) => {
                    let (merged, clean) = merge_value(Some(ba), la, ra);
                    (Some(merged), clean)
                }
                (None, None, None) => (None, true),
                _ => return None,
            };
            let (content, content_clean) = match (&b.content, &l.content, &r.content) {
                (Some(bc), Some(lc), Some(rc)) => {
                    let (merged, clean) = merge_inline_seq(bc, lc, rc);
                    (Some(merged), clean)
                }
                (None, None, None) => (None, true),
                // One side added or removed [content] outright -- not a
                // shape recursion can narrow.
                _ => return None,
            };
            let (children, children_clean) = match (&b.children, &l.children, &r.children) {
                (Some(bc), Some(lc), Some(rc)) => {
                    let (merged, clean) = merge_block_seq(bc, lc, rc);
                    (Some(merged), clean)
                }
                (None, None, None) => (None, true),
                _ => return None,
            };
            let (value, value_clean) = match (&b.value, &l.value, &r.value) {
                (Some(bv), Some(lv), Some(rv)) => {
                    let (merged, clean) = merge_element_value(bv, lv, rv)?;
                    (Some(merged), clean)
                }
                (None, None, None) => (None, true),
                _ => return None,
            };

            Some((
                Block::Element(Element {
                    args,
                    content,
                    children,
                    value,
                    ..l.clone()
                }),
                args_clean && content_clean && children_clean && value_clean,
            ))
        }
        _ => None,
    }
}

/// Narrows a conflict inside an element's `{data}` group, by key rather
/// than by position (so reordered-but-otherwise-untouched keys never
/// register as a conflict). `None` when either side's group holds a
/// nested element rather than only `key: value` pairs -- narrowing a
/// conflict *between* interleaved pairs and nested elements would need
/// its own representation for "a conflict sitting where an element
/// could be", which is a fourth placement context for `@mobile.conflict`
/// and hasn't been designed.
fn merge_element_value(
    base: &ElementValue,
    local: &ElementValue,
    remote: &ElementValue,
) -> Option<(ElementValue, bool)> {
    let (ElementValue::Group(b), ElementValue::Group(l), ElementValue::Group(r)) =
        (base, local, remote)
    else {
        return None;
    };

    let only_pairs = |entries: &[Entry]| -> Option<Vec<(String, Value)>> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Pair(k, v) => Some((k.clone(), v.clone())),
                Entry::Element(_) => None,
            })
            .collect()
    };
    let (base_pairs, local_pairs, remote_pairs) = (only_pairs(b)?, only_pairs(l)?, only_pairs(r)?);

    let (merged, clean) = merge_value_map(&base_pairs, &local_pairs, &remote_pairs);
    let Value::Map(entries) = merged else {
        unreachable!("merge_value_map always returns a Value::Map")
    };
    Some((
        ElementValue::Group(entries.into_iter().map(|(k, v)| Entry::Pair(k, v)).collect()),
        clean,
    ))
}

/// Three-way merge of one value, recursing into `Value::Map` (by key) and
/// `Value::Seq` (positionally); a leaf-level (or shape-mismatched)
/// disagreement embeds `@mobile.conflict(mine: ..., theirs: ...)` as
/// the value itself, using `Value::Element` -- a value slot can hold an
/// arbitrary element, so the same element name used as a block/inline
/// marker elsewhere does double duty here as data instead.
///
/// `base: None` means there is nothing to compare against (the key, or
/// this whole value, was added independently by both sides) -- treated
/// as an empty `Map`/`Seq` rather than failing, so two sides adding the
/// same key with compatible sub-structure can still merge below this
/// level instead of conflicting outright.
fn merge_value(base: Option<&Value>, local: &Value, remote: &Value) -> (Value, bool) {
    if local == remote {
        return (local.clone(), true);
    }
    if base == Some(local) {
        return (remote.clone(), true);
    }
    if base == Some(remote) {
        return (local.clone(), true);
    }

    match (local, remote) {
        (Value::Map(l), Value::Map(r)) => {
            let b = match base {
                Some(Value::Map(b)) => b.as_slice(),
                _ => &[],
            };
            merge_value_map(b, l, r)
        }
        (Value::Seq(l), Value::Seq(r)) => {
            let b = match base {
                Some(Value::Seq(b)) => b.as_slice(),
                _ => &[],
            };
            merge_value_seq(b, l, r)
        }
        _ => (value_conflict(local.clone(), remote.clone()), false),
    }
}

fn merge_value_map(
    base: &[(String, Value)],
    local: &[(String, Value)],
    remote: &[(String, Value)],
) -> (Value, bool) {
    fn lookup<'a>(entries: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
        entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    // Local's key order first, then any key remote alone introduced --
    // an arbitrary but deterministic choice; `{data}` order isn't
    // semantically load-bearing the way block order is.
    let mut order: Vec<&str> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (k, _) in local.iter().chain(remote.iter()) {
        if seen.insert(k.as_str()) {
            order.push(k.as_str());
        }
    }

    let mut out = Vec::new();
    let mut clean = true;
    for key in order {
        let (b, l, r) = (lookup(base, key), lookup(local, key), lookup(remote, key));
        match (l, r) {
            (Some(lv), None) => {
                // remote deleted `key`; keep it only if local actually
                // changed it -- an unmodified key gives way to the
                // other side's deletion instead of resurrecting it.
                if b != Some(lv) {
                    out.push((key.to_string(), lv.clone()));
                }
            }
            (None, Some(rv)) => {
                if b != Some(rv) {
                    out.push((key.to_string(), rv.clone()));
                }
            }
            (None, None) => unreachable!("key came from local or remote"),
            (Some(lv), Some(rv)) => {
                let (merged, sub_clean) = merge_value(b, lv, rv);
                clean &= sub_clean;
                out.push((key.to_string(), merged));
            }
        }
    }
    (Value::Map(out), clean)
}

fn merge_value_seq(base: &[Value], local: &[Value], remote: &[Value]) -> (Value, bool) {
    let mut out = Vec::new();
    let mut clean = true;
    for region in diff3::merge3(base, local, remote) {
        match region {
            diff3::Region::Same(items) => out.extend(items),
            diff3::Region::Conflict { base, mine, theirs } => {
                if let ([b], [l], [r]) = (base.as_slice(), mine.as_slice(), theirs.as_slice()) {
                    let (merged, sub_clean) = merge_value(Some(b), l, r);
                    out.push(merged);
                    clean &= sub_clean;
                    continue;
                }
                clean = false;
                out.push(value_conflict(Value::Seq(mine), Value::Seq(theirs)));
            }
        }
    }
    (Value::Seq(out), clean)
}

fn value_conflict(mine: Value, theirs: Value) -> Value {
    // A value-embedded element may only carry `(args)` -- no `{value}`
    // and no `[content]` -- so `mine`/`theirs` live as named args
    // (`(mine: ..., theirs: ...)`), which parse to the same `Value::Map`
    // shape `{data}` would have used if it were available here.
    Value::Element(Box::new(Element {
        sigil: Sigil::Named(Name::namespaced("mobile", "conflict")),
        args: Some(Value::Map(vec![
            ("mine".to_string(), mine),
            ("theirs".to_string(), theirs),
        ])),
        ..Default::default()
    }))
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
    let mut resolutions = resolutions.iter();
    let mut blocks = resolve_block_seq(&document.blocks, &mut resolutions);

    let resolved = Document {
        blocks: blocks.clone(),
        span: document.span,
    };
    if !has_conflicts(&resolved) {
        blocks.retain(|b| !is_use_mobile_block(b));
    }

    Document {
        blocks,
        span: document.span,
    }
}

/// Removes every `@mobile.conflict` marker from `document`, resolving all conflicts
/// to the specified side (`ConflictResolution::Mine` or `ConflictResolution::Theirs`),
/// and drops the `@use(mobile)` preamble line once none remain.
pub fn resolve_all(document: &Document, resolution: ConflictResolution) -> Document {
    let mut resolutions = std::iter::repeat(&resolution);
    let mut blocks = resolve_block_seq(&document.blocks, &mut resolutions);

    let resolved = Document {
        blocks: blocks.clone(),
        span: document.span,
    };
    if !has_conflicts(&resolved) {
        blocks.retain(|b| !is_use_mobile_block(b));
    }

    Document {
        blocks,
        span: document.span,
    }
}

/// The block-sequence counterpart of [`merge_block_seq`]: strips
/// `mine`/`theirs`/`end` marker triples, keeping the chosen side, and
/// recurses into every other block via [`resolve_block`]. Used for
/// `Document.blocks` and for an `Element`'s `children`, matching how
/// `merge_block_seq` is used in both places -- resolving must walk the
/// tree in the same order merging did, or a resolution would apply to
/// the wrong conflict.
fn resolve_block_seq<'a>(
    blocks: &[Block],
    resolutions: &mut impl Iterator<Item = &'a ConflictResolution>,
) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        if conflict_side(&blocks[i]) == Some("mine") {
            let theirs_at = (i + 1..blocks.len())
                .find(|&j| conflict_side(&blocks[j]) == Some("theirs"))
                .expect("a mine marker is always followed by a theirs marker");
            let end_at = (theirs_at + 1..blocks.len())
                .find(|&j| conflict_side(&blocks[j]) == Some("end"))
                .expect("a theirs marker is always followed by an end marker");

            let mine_blocks = &blocks[i + 1..theirs_at];
            let theirs_blocks = &blocks[theirs_at + 1..end_at];

            match resolutions.next() {
                Some(ConflictResolution::Mine) => out.extend(mine_blocks.iter().cloned()),
                Some(ConflictResolution::Theirs) => out.extend(theirs_blocks.iter().cloned()),
                Some(ConflictResolution::Rewritten(text)) => out.push(Block::Paragraph(
                    Paragraph::new(vec![Inline::Text(Text::from(text.as_str()))], Default::default()),
                )),
                None => out.extend(blocks[i..=end_at].iter().cloned()),
            }
            i = end_at + 1;
            continue;
        }

        out.push(resolve_block(&blocks[i], resolutions));
        i += 1;
    }
    out
}

/// Resolves every kind of `@mobile.conflict` a single block might carry:
/// inline markers in a `Paragraph`'s or `Element`'s content, a marker
/// triple among an `Element`'s children, and a
/// `@mobile.conflict(mine: ..., theirs: ...)` value sitting in `args` or
/// `{data}`. Visits
/// args/content/children/value in that order -- the same order
/// `merge_one` recurses in -- so resolutions line up with the conflicts
/// `merge` produced.
fn resolve_block<'a>(
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
            if let Some(args) = &e.args {
                e.args = Some(resolve_value(args, resolutions));
            }
            if let Some(content) = &e.content {
                e.content = Some(resolve_inline_seq(content, resolutions));
            }
            if let Some(children) = &e.children {
                e.children = Some(resolve_block_seq(children, resolutions));
            }
            if let Some(value) = &e.value {
                e.value = Some(resolve_element_value(value, resolutions));
            }
            Block::Element(e)
        }
    }
}

/// Resolves a `@mobile.conflict(mine, theirs)` sitting where a value
/// goes, recursing into `Value::Map`/`Value::Seq` to find one nested
/// arbitrarily deep. The order this walks a `Map`'s entries matches the
/// order `merge_value_map` built them in, since it visits the *merged*
/// document's own entries in the order they already sit in.
fn resolve_value<'a>(
    value: &Value,
    resolutions: &mut impl Iterator<Item = &'a ConflictResolution>,
) -> Value {
    if let Some((mine, theirs)) = value_conflict_sides(value) {
        return match resolutions.next() {
            Some(ConflictResolution::Mine) => mine.clone(),
            Some(ConflictResolution::Theirs) => theirs.clone(),
            Some(ConflictResolution::Rewritten(text)) => Value::String(text.clone()),
            None => value.clone(),
        };
    }
    match value {
        Value::Map(entries) => Value::Map(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), resolve_value(v, resolutions)))
                .collect(),
        ),
        Value::Seq(items) => {
            Value::Seq(items.iter().map(|v| resolve_value(v, resolutions)).collect())
        }
        other => other.clone(),
    }
}

fn resolve_element_value<'a>(
    value: &ElementValue,
    resolutions: &mut impl Iterator<Item = &'a ConflictResolution>,
) -> ElementValue {
    let ElementValue::Group(entries) = value else {
        return value.clone();
    };
    ElementValue::Group(
        entries
            .iter()
            .map(|e| match e {
                Entry::Pair(k, v) => Entry::Pair(k.clone(), resolve_value(v, resolutions)),
                // Groups mixing pairs with nested elements were never
                // narrowed by merge_element_value, so there is nothing
                // here for a resolution to apply to.
                Entry::Element(el) => Entry::Element(el.clone()),
            })
            .collect(),
    )
}

/// This value's `mine`/`theirs` sides, if it is a
/// `@mobile.conflict(mine: ..., theirs: ...)` marker.
fn value_conflict_sides(value: &Value) -> Option<(&Value, &Value)> {
    let Value::Element(el) = value else {
        return None;
    };
    let Sigil::Named(name) = &el.sigil else {
        return None;
    };
    if name.namespace.as_deref() != Some("mobile") || name.name != "conflict" {
        return None;
    }
    let Value::Map(entries) = el.args.as_ref()? else {
        return None;
    };
    let get = |key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    Some((get("mine")?, get("theirs")?))
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

/// Whether `document` still contains an unresolved `@mobile.conflict`,
/// at the block level or nested in a paragraph's inline content.
/// `immermemo-sync` uses this to report which notes need the user's
/// attention after a sync.
pub fn has_conflicts(document: &Document) -> bool {
    document.blocks.iter().any(document_or_block_has_conflict)
}

/// Whether this block still carries a conflict, at the block level or
/// nested in its own inline content.
fn document_or_block_has_conflict(block: &Block) -> bool {
    if is_conflict_block(block) {
        return true;
    }
    match block {
        Block::Paragraph(p) => inline_seq_has_conflict(&p.content),
        Block::Element(e) => {
            e.args.as_ref().is_some_and(value_has_conflict)
                || e.content.as_ref().is_some_and(|c| inline_seq_has_conflict(c))
                || e.children
                    .as_ref()
                    .is_some_and(|c| c.iter().any(document_or_block_has_conflict))
                || e.value.as_ref().is_some_and(element_value_has_conflict)
        }
    }
}

fn inline_seq_has_conflict(content: &[Inline]) -> bool {
    content.iter().any(|i| {
        inline_conflict_side(i).is_some()
            || matches!(i, Inline::Element(el) if el.value.as_ref().is_some_and(element_value_has_conflict)
                || el.args.as_ref().is_some_and(value_has_conflict))
    })
}

fn element_value_has_conflict(value: &ElementValue) -> bool {
    let ElementValue::Group(entries) = value else {
        return false;
    };
    entries.iter().any(|e| match e {
        Entry::Pair(_, v) => value_has_conflict(v),
        Entry::Element(el) => {
            el.args.as_ref().is_some_and(value_has_conflict)
                || el.value.as_ref().is_some_and(element_value_has_conflict)
        }
    })
}

fn value_has_conflict(value: &Value) -> bool {
    if value_conflict_sides(value).is_some() {
        return true;
    }
    match value {
        Value::Map(entries) => entries.iter().any(|(_, v)| value_has_conflict(v)),
        Value::Seq(items) => items.iter().any(value_has_conflict),
        _ => false,
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
}
