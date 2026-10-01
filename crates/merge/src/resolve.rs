//! Strips `@mobile.conflict` markers back out of a merged document, once
//! the user (or an automatic policy) has picked a side for each one. The
//! counterpart to [`crate::merge`]: it walks the tree in the exact same
//! order merging produced markers in, so resolutions line up with the
//! right conflict.

use tomet_ast::{Block, Document, ElementValue, Entry, Inline, Paragraph, Text, Value};

use crate::markers::{
    coalesce_text, conflict_side, inline_conflict_side, is_use_mobile_block, value_conflict_sides,
};

/// Which side of one `@mobile.conflict` triple a user picked, or that
/// they edited a fresh replacement by hand, or kept both versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    Mine,
    Theirs,
    Both,
    /// The user wrote something new that is neither side verbatim.
    Rewritten(String),
}

/// Removes every `@mobile.conflict` marker triple from `document`,
/// keeping the side the caller picked for each, and drops the
/// `@use(mobile)` preamble line once none remain.
///
/// `resolutions` are consumed in document order, one per `mine`/`theirs`/
/// `end` triple. A triple with no corresponding entry (`resolutions` ran
/// out) is left in the document untouched.
pub fn resolve(document: &Document, resolutions: &[ConflictResolution]) -> Document {
    apply_resolutions(document, &mut resolutions.iter().map(Some))
}

/// Removes every `@mobile.conflict` marker from `document`, resolving all conflicts
/// to the specified side (`ConflictResolution::Mine` or `ConflictResolution::Theirs`),
/// and drops the `@use(mobile)` preamble line once none remain.
pub fn resolve_all(document: &Document, resolution: ConflictResolution) -> Document {
    apply_resolutions(document, &mut std::iter::repeat(Some(&resolution)))
}

/// Resolves a single conflict at `target_index` to the given `resolution`,
/// leaving all other conflicts untouched.
pub fn resolve_single(
    document: &Document,
    target_index: usize,
    resolution: ConflictResolution,
) -> Document {
    let mut idx = 0;
    apply_resolutions(
        document,
        &mut std::iter::from_fn(|| {
            let res = if idx == target_index {
                Some(&resolution)
            } else {
                None
            };
            idx += 1;
            Some(res)
        }),
    )
}

/// Shared implementation for [`resolve`], [`resolve_all`] and [`resolve_single`]:
/// walks the document applying each resolution from `resolutions` in order, then
/// strips `@use(mobile)` if no conflict markers remain.
fn apply_resolutions<'a>(
    document: &Document,
    resolutions: &mut impl Iterator<Item = Option<&'a ConflictResolution>>,
) -> Document {
    let mut blocks = resolve_block_seq(&document.blocks, resolutions);

    let resolved = Document {
        blocks: blocks.clone(),
        span: document.span,
    };
    if !crate::find_conflicts::has_conflicts(&resolved) {
        blocks.retain(|b| !is_use_mobile_block(b));
    }

    Document {
        blocks,
        span: document.span,
    }
}

/// The block-sequence counterpart of `merge_block_seq`: strips
/// `mine`/`theirs`/`end` marker triples, keeping the chosen side, and
/// recurses into every other block via [`resolve_block`]. Used for
/// `Document.blocks` and for an `Element`'s `children`, matching how
/// `merge_block_seq` is used in both places -- resolving must walk the
/// tree in the same order merging did, or a resolution would apply to
/// the wrong conflict.
fn resolve_block_seq<'a>(
    blocks: &[Block],
    resolutions: &mut impl Iterator<Item = Option<&'a ConflictResolution>>,
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
                Some(Some(ConflictResolution::Mine)) => out.extend(mine_blocks.iter().cloned()),
                Some(Some(ConflictResolution::Theirs)) => out.extend(theirs_blocks.iter().cloned()),
                Some(Some(ConflictResolution::Both)) => {
                    out.extend(mine_blocks.iter().cloned());
                    out.extend(theirs_blocks.iter().cloned());
                }
                Some(Some(ConflictResolution::Rewritten(text))) => {
                    out.push(Block::Paragraph(Paragraph::new(
                        vec![Inline::Text(Text::from(text.as_str()))],
                        Default::default(),
                    )))
                }
                Some(None) | None => out.extend(blocks[i..=end_at].iter().cloned()),
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
    resolutions: &mut impl Iterator<Item = Option<&'a ConflictResolution>>,
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
        Block::Section(sec) => {
            let mut s = sec.clone();
            s.title = resolve_inline_seq(&s.title, resolutions);
            if let Some(args) = &s.args {
                s.args = Some(resolve_value(args, resolutions));
            }
            s.blocks = resolve_block_seq(&s.blocks, resolutions);
            if let Some(value) = &s.value {
                s.value = Some(resolve_element_value(value, resolutions));
            }
            Block::Section(s)
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
    resolutions: &mut impl Iterator<Item = Option<&'a ConflictResolution>>,
) -> Value {
    if let Some((mine, theirs)) = value_conflict_sides(value) {
        return match resolutions.next() {
            Some(Some(ConflictResolution::Mine)) => mine.clone(),
            Some(Some(ConflictResolution::Theirs)) => theirs.clone(),
            Some(Some(ConflictResolution::Both)) => match (mine, theirs) {
                (Value::String(m), Value::String(t)) => Value::String(format!("{m} / {t}")),
                _ => mine.clone(),
            },
            Some(Some(ConflictResolution::Rewritten(text))) => Value::String(text.clone()),
            Some(None) | None => value.clone(),
        };
    }
    match value {
        Value::Map(entries) => Value::Map(
            entries
                .iter()
                .map(|(k, v)| (k.clone(), resolve_value(v, resolutions)))
                .collect(),
        ),
        Value::Seq(items) => Value::Seq(
            items
                .iter()
                .map(|v| resolve_value(v, resolutions))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn resolve_element_value<'a>(
    value: &ElementValue,
    resolutions: &mut impl Iterator<Item = Option<&'a ConflictResolution>>,
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

/// The inline-level counterpart of `resolve`'s main loop.
fn resolve_inline_seq<'a>(
    content: &[Inline],
    resolutions: &mut impl Iterator<Item = Option<&'a ConflictResolution>>,
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
            Some(Some(ConflictResolution::Mine)) => out.extend(mine_run.iter().cloned()),
            Some(Some(ConflictResolution::Theirs)) => out.extend(theirs_run.iter().cloned()),
            Some(Some(ConflictResolution::Both)) => {
                out.extend(mine_run.iter().cloned());
                out.extend(theirs_run.iter().cloned());
            }
            Some(Some(ConflictResolution::Rewritten(text))) => {
                out.push(Inline::Text(Text::from(text.as_str())))
            }
            Some(None) | None => out.extend(content[i..=end_at].iter().cloned()),
        }
        i = end_at + 1;
    }
    coalesce_text(out)
}
