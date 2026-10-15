//! Inline elements classification, styling wrapper aggregation, and text span splitting.

use tomet::ast::{Block, Element, ElementValue, Entry, Inline, Sigil, Span, Value};

use crate::block::{classify_blocks_flat, is_atomic_bare};
use crate::types::{ElementArgs, ElementIdentity, RenderItem, TextStyle};

/// `el`'s own `[content]` as a flat run of `Inline`s.
pub(crate) fn inline_content(el: &Element) -> Option<&[Inline]> {
    match el.content.as_deref()? {
        [Block::Paragraph(p)] => Some(&p.content),
        _ => None,
    }
}

/// Splits a run of [`Inline`]s into [`RenderItem`]s: a *namespaced*
/// element or one of atomic bare elements becomes its own
/// [`RenderItem::Element`], a bare `em`/`strong`/`mark`/`strikeout`
/// recurses into its own content with that style flag added, and everything else
/// is coalesced into one [`RenderItem::Text`] run.
pub(crate) fn classify_inline_seq(content: &[Inline]) -> Vec<RenderItem> {
    let mut items = Vec::new();
    let mut run: Option<(Span, TextStyle)> = None;
    walk_inline_seq(content, TextStyle::default(), &mut items, &mut run);
    if let Some((span, style)) = run {
        items.push(RenderItem::Text(span, style));
    }
    items
}

pub(crate) fn walk_inline_seq(
    content: &[Inline],
    style: TextStyle,
    items: &mut Vec<RenderItem>,
    run: &mut Option<(Span, TextStyle)>,
) {
    for inline in content {
        if let Inline::Element(el) = inline {
            if let Some(wrapper) = style_wrapper(el) {
                if let Some(nested) = inline_content(el) {
                    walk_inline_seq(nested, style.union(wrapper), items, run);
                }
                continue;
            }
            if is_atomic_bare(el) {
                flush_run(items, run);
                items.push(
                    element_item(el)
                        .expect("is_atomic_bare already confirmed el.sigil is Sigil::Named"),
                );
                continue;
            }
            if let Some(item) = element_item(el).filter(is_namespaced) {
                flush_run(items, run);
                items.push(item);
                continue;
            }
        }
        extend_run(items, run, inline.span(), style);
    }
}

/// Appends `span` to `run`, flushing it first if it was accumulating under
/// a different [`TextStyle`].
pub(crate) fn extend_run(
    items: &mut Vec<RenderItem>,
    run: &mut Option<(Span, TextStyle)>,
    span: Span,
    style: TextStyle,
) {
    match run {
        Some((existing_span, existing_style)) if *existing_style == style => {
            *existing_span = existing_span.union(&span);
        }
        _ => {
            flush_run(items, run);
            *run = Some((span, style));
        }
    }
}

pub(crate) fn flush_run(items: &mut Vec<RenderItem>, run: &mut Option<(Span, TextStyle)>) {
    if let Some((span, style)) = run.take() {
        items.push(RenderItem::Text(span, style));
    }
}

/// `el`'s [`TextStyle`] flag if it's a bare (non-`@use`'d) `em`/`strong`/
/// `mark`/`strikeout` wrapper, else `None`.
pub(crate) fn style_wrapper(el: &Element) -> Option<TextStyle> {
    let Sigil::Named(name) = &el.sigil else {
        return None;
    };
    if name.namespace.is_some() {
        return None;
    }
    match name.name.as_str() {
        "em" => Some(TextStyle {
            italic: true,
            ..TextStyle::default()
        }),
        "strong" => Some(TextStyle {
            bold: true,
            ..TextStyle::default()
        }),
        "mark" => Some(TextStyle {
            mark: true,
            ..TextStyle::default()
        }),
        "strikeout" => Some(TextStyle {
            strikeout: true,
            ..TextStyle::default()
        }),
        _ => None,
    }
}

/// `el`'s [`RenderItem::Element`], for any named element.
pub(crate) fn element_item(el: &Element) -> Option<RenderItem> {
    let Sigil::Named(name) = &el.sigil else {
        return None;
    };
    Some(RenderItem::Element {
        identity: ElementIdentity {
            namespace: name.namespace.clone(),
            name: name.name.clone(),
        },
        span: el.span,
        content: el
            .content
            .as_ref()
            .and_then(|blocks| blocks.iter().map(Block::span).reduce(|a, b| a.union(&b))),
        args_summary: summarize_args(el.args.as_ref()),
        data_summary: summarize_data(el.value.as_ref()),
        args: element_args(el.args.as_ref()),
        block_args: block_args(el.args.as_ref()),
    })
}

/// [`RenderItem::Element::block_args`] -- recursively classified.
pub(crate) fn block_args(args: Option<&Value>) -> Vec<(String, Vec<RenderItem>)> {
    let Some(Value::Map(entries)) = args else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(key, value)| match value {
            Value::Blocks(blocks) if !key.is_empty() => {
                Some((key.clone(), classify_blocks_flat(blocks)))
            }
            _ => None,
        })
        .collect()
}

/// `el`'s [`RenderItem::Element::args`] -- every string-valued entry of `(args)`.
pub(crate) fn element_args(args: Option<&Value>) -> ElementArgs {
    match args {
        Some(Value::String(s)) => ElementArgs::Positional(s.clone()),
        Some(Value::Map(entries)) => ElementArgs::Named(
            entries
                .iter()
                .filter_map(|(key, value)| match value {
                    Value::String(s) => Some((key.clone(), s.clone())),
                    _ => None,
                })
                .collect(),
        ),
        _ => ElementArgs::None,
    }
}

/// Whether `item` is a [`RenderItem::Element`] that came in through `@use` (has a namespace).
pub(crate) fn is_namespaced(item: &RenderItem) -> bool {
    matches!(item, RenderItem::Element { identity, .. } if identity.namespace.is_some())
}

/// A short description of an element's `(args)`, for a caller that falls
/// back to a generic card.
pub(crate) fn summarize_args(args: Option<&Value>) -> String {
    match args {
        None => String::new(),
        Some(value) => summarize_value(value),
    }
}

/// A short description of an element's `{value}` group: its pair keys, joined.
pub(crate) fn summarize_data(value: Option<&ElementValue>) -> String {
    match value {
        None => String::new(),
        Some(ElementValue::Group(entries)) => entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Pair(key, _) => Some(key.clone()),
                Entry::Element(_) => None,
            })
            .collect::<Vec<_>>()
            .join(", "),
        Some(ElementValue::Interp(_)) => "interp".to_string(),
    }
}

pub(crate) fn summarize_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::Seq(items) => format!("[{} item(s)]", items.len()),
        Value::Map(entries) => entries
            .iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Call(name, args) => format!("{name}({} arg(s))", args.len()),
        Value::Element(el) => el.sigil.name().map(|n| n.to_string()).unwrap_or_default(),
        Value::Blocks(blocks) => format!("[{} block(s)]", blocks.len()),
    }
}
