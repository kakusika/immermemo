//! Reads `@conflict` markers back out of a document without removing
//! them -- for showing the user what's in conflict (the resolution UI)
//! and for `immermemo-sync`'s post-sync conflict report.

use tomet_ast::{Block, Document, ElementValue, Entry, Inline, Value};

use crate::markers::{
    blocks_to_text, conflict_block_sides, conflict_inline_sides, inline_is_conflict,
    inlines_to_text, is_conflict_block, value_conflict_sides, value_to_text,
};

/// An individual conflict region discovered within a document. `a`/`b`
/// are the same structural, non-perspectival names `@conflict` itself
/// uses -- neither is "mine": a conflict found cold (the app reopened,
/// or a different device than the one that ran the merge) has no
/// stored way to tell which side was whose. A caller that wants a
/// mine/theirs label derives it itself, live, from something `@conflict`
/// doesn't carry (see `immermemo-editor`'s `conflicts` module).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictItem {
    pub index: usize,
    pub a: String,
    pub b: String,
}

/// Finds and extracts all conflict regions present in `document`.
pub fn find_conflicts(document: &Document) -> Vec<ConflictItem> {
    let mut items = Vec::new();
    find_conflicts_block_seq(&document.blocks, &mut items);
    items
}

fn find_conflicts_block_seq(blocks: &[Block], items: &mut Vec<ConflictItem>) {
    for block in blocks {
        if let Block::Element(el) = block
            && let Some((a, b)) = conflict_block_sides(el)
        {
            items.push(ConflictItem {
                index: items.len(),
                a: blocks_to_text(&a),
                b: blocks_to_text(&b),
            });
            continue;
        }
        find_conflicts_block(block, items);
    }
}

fn find_conflicts_block(block: &Block, items: &mut Vec<ConflictItem>) {
    match block {
        Block::Paragraph(p) => find_conflicts_inline_seq(&p.content, items),
        Block::Element(e) => {
            if let Some(args) = &e.args {
                find_conflicts_value(args, items);
            }
            if let Some(content) = &e.content {
                find_conflicts_block_seq(content, items);
            }
            if let Some(children) = &e.children {
                find_conflicts_block_seq(children, items);
            }
            if let Some(value) = &e.value {
                find_conflicts_element_value(value, items);
            }
        }
        Block::Section(s) => {
            find_conflicts_inline_seq(&s.title, items);
            if let Some(args) = &s.args {
                find_conflicts_value(args, items);
            }
            find_conflicts_block_seq(&s.blocks, items);
            if let Some(value) = &s.value {
                find_conflicts_element_value(value, items);
            }
        }
    }
}

fn find_conflicts_inline_seq(content: &[Inline], items: &mut Vec<ConflictItem>) {
    for inline in content {
        if let Inline::Element(el) = inline
            && let Some((a, b)) = conflict_inline_sides(el)
        {
            items.push(ConflictItem {
                index: items.len(),
                a: inlines_to_text(&a),
                b: inlines_to_text(&b),
            });
        }
    }
}

fn find_conflicts_value(value: &Value, items: &mut Vec<ConflictItem>) {
    if let Some((a, b)) = value_conflict_sides(value) {
        items.push(ConflictItem {
            index: items.len(),
            a: value_to_text(a),
            b: value_to_text(b),
        });
        return;
    }
    match value {
        Value::Map(entries) => {
            for (_, v) in entries {
                find_conflicts_value(v, items);
            }
        }
        Value::Seq(seq) => {
            for v in seq {
                find_conflicts_value(v, items);
            }
        }
        _ => {}
    }
}

fn find_conflicts_element_value(value: &ElementValue, items: &mut Vec<ConflictItem>) {
    if let ElementValue::Group(entries) = value {
        for entry in entries {
            if let Entry::Pair(_, v) = entry {
                find_conflicts_value(v, items);
            }
        }
    }
}

/// Whether `document` still contains an unresolved `@conflict`, at the
/// block level or nested in a paragraph's inline content. `immermemo-
/// sync` uses this to report which notes need the user's attention
/// after a sync.
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
                || e.content
                    .as_ref()
                    .is_some_and(|c| c.iter().any(document_or_block_has_conflict))
                || e.children
                    .as_ref()
                    .is_some_and(|c| c.iter().any(document_or_block_has_conflict))
                || e.value.as_ref().is_some_and(element_value_has_conflict)
        }
        Block::Section(s) => {
            inline_seq_has_conflict(&s.title)
                || s.args.as_ref().is_some_and(value_has_conflict)
                || s.blocks.iter().any(document_or_block_has_conflict)
                || s.value.as_ref().is_some_and(element_value_has_conflict)
        }
    }
}

fn inline_seq_has_conflict(content: &[Inline]) -> bool {
    content.iter().any(|i| {
        inline_is_conflict(i)
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
