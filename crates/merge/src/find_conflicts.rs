//! Reads `@mobile.conflict` markers back out of a document without
//! removing them -- for showing the user what's in conflict (the
//! resolution UI) and for `immermemo-sync`'s post-sync conflict report.

use tomet_ast::{Block, Document, ElementValue, Entry, Inline, Value};

use crate::markers::{
    conflict_side, inline_conflict_side, inlines_to_text, is_conflict_block, value_conflict_sides,
    value_to_text,
};

/// An individual conflict region discovered within a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictItem {
    pub index: usize,
    pub mine: String,
    pub theirs: String,
}

/// Finds and extracts all conflict regions present in `document`.
pub fn find_conflicts(document: &Document) -> Vec<ConflictItem> {
    let mut items = Vec::new();
    find_conflicts_block_seq(&document.blocks, &mut items);
    items
}

fn find_conflicts_block_seq(blocks: &[Block], items: &mut Vec<ConflictItem>) {
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

            let mine_doc = Document {
                blocks: mine_blocks.to_vec(),
                span: Default::default(),
            };
            let theirs_doc = Document {
                blocks: theirs_blocks.to_vec(),
                span: Default::default(),
            };

            items.push(ConflictItem {
                index: items.len(),
                mine: tomet_printer::document_to_tm(&mine_doc).trim().to_string(),
                theirs: tomet_printer::document_to_tm(&theirs_doc)
                    .trim()
                    .to_string(),
            });

            i = end_at + 1;
            continue;
        }

        find_conflicts_block(&blocks[i], items);
        i += 1;
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
                find_conflicts_inline_seq(content, items);
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
    let mut i = 0;
    while i < content.len() {
        if inline_conflict_side(&content[i]) == Some("mine") {
            let theirs_at = (i + 1..content.len())
                .find(|&j| inline_conflict_side(&content[j]) == Some("theirs"))
                .expect("a mine marker is always followed by a theirs marker");
            let end_at = (theirs_at + 1..content.len())
                .find(|&j| inline_conflict_side(&content[j]) == Some("end"))
                .expect("a theirs marker is always followed by an end marker");

            let mine_run = &content[i + 1..theirs_at];
            let theirs_run = &content[theirs_at + 1..end_at];

            items.push(ConflictItem {
                index: items.len(),
                mine: inlines_to_text(mine_run),
                theirs: inlines_to_text(theirs_run),
            });

            i = end_at + 1;
            continue;
        }
        i += 1;
    }
}

fn find_conflicts_value(value: &Value, items: &mut Vec<ConflictItem>) {
    if let Some((mine, theirs)) = value_conflict_sides(value) {
        items.push(ConflictItem {
            index: items.len(),
            mine: value_to_text(mine),
            theirs: value_to_text(theirs),
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
                || e.content
                    .as_ref()
                    .is_some_and(|c| inline_seq_has_conflict(c))
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
