//! Construction and detection of `@mobile.conflict` marker nodes, and the
//! small text-building helpers shared by [`crate::merge`], [`crate::resolve`]
//! and [`crate::find_conflicts`]. Nothing here walks a whole document --
//! that's each of those modules' own job -- this only knows what a single
//! marker or value looks like.

use tomet_ast::{Block, Document, Element, Inline, Name, Placement, Sigil, Text, Value};

/// The raw-text substring other crates can look for without parsing a
/// document -- a cheap "does this note have a conflict in it at all"
/// check over file content read straight off disk.
pub const CONFLICT_MARKER: &str = "@mobile.conflict";

/// Returns `Name` for `@mobile.conflict`, used in every place this crate
/// creates or inspects a conflict marker element.
pub(crate) fn conflict_name() -> Name {
    Name::namespaced("mobile", "conflict")
}

/// The `side` argument of an `@mobile.conflict(side)` element, extracted
/// from the `Element` directly. Used by both the block-level and the
/// inline-level helpers below.
pub(crate) fn element_conflict_side(el: &Element) -> Option<&str> {
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

pub(crate) fn conflict_marker(side: &str) -> Block {
    Block::Element(Element {
        sigil: Sigil::Named(conflict_name()),
        placement: Placement::Block,
        args: Some(Value::String(side.to_string())),
        ..Default::default()
    })
}

pub(crate) fn use_mobile_block() -> Block {
    Block::Element(Element {
        sigil: Sigil::named("use"),
        placement: Placement::Block,
        args: Some(Value::String("mobile".to_string())),
        ..Default::default()
    })
}

/// This block's `side` argument, if it is a `@mobile.conflict(side)` marker.
pub(crate) fn conflict_side(block: &Block) -> Option<&str> {
    let Block::Element(el) = block else {
        return None;
    };
    element_conflict_side(el)
}

pub(crate) fn is_conflict_block(block: &Block) -> bool {
    conflict_side(block).is_some()
}

pub(crate) fn inline_conflict_marker(side: &str) -> Inline {
    Inline::Element(Element {
        sigil: Sigil::Named(conflict_name()),
        placement: Placement::Inline,
        args: Some(Value::String(side.to_string())),
        ..Default::default()
    })
}

/// This inline node's `side` argument, if it is a
/// `@mobile.conflict(side)` marker.
pub(crate) fn inline_conflict_side(inline: &Inline) -> Option<&str> {
    let Inline::Element(el) = inline else {
        return None;
    };
    element_conflict_side(el)
}

pub(crate) fn is_use_mobile_block(block: &Block) -> bool {
    let Block::Element(el) = block else {
        return false;
    };
    el.sigil.is_bare_named("use") && matches!(&el.args, Some(Value::String(ns)) if ns == "mobile")
}

/// This value's `mine`/`theirs` sides, if it is a
/// `@mobile.conflict(mine: ..., theirs: ...)` marker.
///
/// Note: this is a different shape from the block/inline markers
/// (`@mobile.conflict(mine)` with a plain string arg). Those are detected
/// by [`element_conflict_side`]; this one carries a `Value::Map` in `args`.
pub(crate) fn value_conflict_sides(value: &Value) -> Option<(&Value, &Value)> {
    let Value::Element(el) = value else {
        return None;
    };
    let Sigil::Named(name) = &el.sigil else {
        return None;
    };
    if name != &conflict_name() {
        return None;
    }
    let Value::Map(entries) = el.args.as_ref()? else {
        return None;
    };
    let get = |key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    Some((get("mine")?, get("theirs")?))
}

pub(crate) fn value_conflict(mine: Value, theirs: Value) -> Value {
    // A value-embedded element may only carry `(args)` -- no `{value}`
    // and no `[content]` -- so `mine`/`theirs` live as named args
    // (`(mine: ..., theirs: ...)`), which parse to the same `Value::Map`
    // shape `{data}` would have used if it were available here.
    Value::Element(Box::new(Element {
        sigil: Sigil::Named(conflict_name()),
        args: Some(Value::Map(vec![
            ("mine".to_string(), mine),
            ("theirs".to_string(), theirs),
        ])),
        ..Default::default()
    }))
}

pub(crate) fn push_text(out: &mut Vec<Inline>, chars: Vec<char>) {
    if !chars.is_empty() {
        out.push(Inline::Text(Text::from(
            chars.into_iter().collect::<String>(),
        )));
    }
}

/// Merges adjacent `Inline::Text` runs into one. Diffing and resolving
/// both build their output as separate pieces (a `Same` run here, a
/// `mine`/`theirs` pick there), which otherwise leaves needlessly
/// fragmented text behind even when nothing about the *content*
/// disagrees.
pub(crate) fn coalesce_text(items: Vec<Inline>) -> Vec<Inline> {
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

pub(crate) fn inlines_to_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) => s.push_str(&t.value),
            Inline::Raw(r) => s.push_str(&r.value),
            Inline::SoftBreak(_) => s.push(' '),
            Inline::LineBreak(_) => s.push('\n'),
            Inline::Element(el) => {
                let doc = Document {
                    blocks: vec![Block::Element(el.clone())],
                    span: Default::default(),
                };
                s.push_str(tomet_printer::document_to_tm(&doc).trim());
            }
        }
    }
    s
}

pub(crate) fn value_to_text(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Null => "null".to_string(),
        _ => format!("{val:?}"),
    }
}
