//! Inline elements classification, styling wrapper aggregation, and text span splitting.

use tomet::ast::{Block, ElementView, Group, Inline, Tuple, Value};
use tomet_scalar::Scalar;
use tomet_span::Span;

use crate::block::{classify_blocks_flat, is_atomic_bare};
use crate::types::{ElementArgs, ElementIdentity, ElementItem, RenderItem, TextStyle};

/// `el`'s own `[content]` as a flat run of `Inline`s.
pub(crate) fn inline_content<'a>(el: ElementView<'a>) -> Option<&'a [Inline]> {
    match el.content? {
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
            let view = el.as_view();
            if let Some(wrapper) = style_wrapper(view) {
                if let Some(nested) = inline_content(view) {
                    walk_inline_seq(nested, style.union(wrapper), items, run);
                }
                continue;
            }
            if is_atomic_bare(view) {
                flush_run(items, run);
                items.push(element_item(view));
                continue;
            }
            let item = element_item(view);
            if is_namespaced(&item) {
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
pub(crate) fn style_wrapper(el: ElementView<'_>) -> Option<TextStyle> {
    if el.name.namespace.is_some() {
        return None;
    }
    match el.name.name.as_str() {
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
pub(crate) fn element_item(el: ElementView<'_>) -> RenderItem {
    RenderItem::Element(Box::new(ElementItem {
        identity: ElementIdentity {
            namespace: el.name.namespace.clone(),
            name: el.name.name.clone(),
        },
        span: el.span,
        content: el
            .content
            .and_then(|blocks| blocks.iter().map(Block::span).reduce(|a, b| a.union(&b))),
        args_summary: summarize_args(el.args()),
        data_summary: summarize_data(el.value()),
        args: element_args(el.args()),
        block_args: block_args(el.args()),
    }))
}

/// [`ElementItem::block_args`] -- recursively classified.
pub(crate) fn block_args(args: Option<&Tuple>) -> Vec<(String, Vec<RenderItem>)> {
    let Some(tuple) = args else {
        return Vec::new();
    };
    tuple
        .entries()
        .filter_map(|entry| match (entry.name(), entry.value()) {
            (Some(key), Value::Blocks(blocks)) if !key.is_empty() => {
                Some((key.to_owned(), classify_blocks_flat(blocks)))
            }
            _ => None,
        })
        .collect()
}

/// `el`'s [`RenderItem::Element::args`] -- every string-valued entry of `(args)`.
pub(crate) fn element_args(args: Option<&Tuple>) -> ElementArgs {
    let Some(tuple) = args else {
        return ElementArgs::None;
    };
    if tuple.len() == 1 {
        if let Some(entry) = tuple.entries().next() {
            if entry.name().is_none() {
                if let Value::Scalar(Scalar::Text(s)) = entry.value() {
                    return ElementArgs::Positional(s.clone());
                }
            }
        }
    }
    ElementArgs::Named(
        tuple
            .entries()
            .filter_map(|entry| match entry.value() {
                Value::Scalar(Scalar::Text(s)) => {
                    Some((entry.name().unwrap_or("").to_owned(), s.clone()))
                }
                _ => None,
            })
            .collect(),
    )
}

/// Whether `item` is a [`RenderItem::Element`] that came in through `@use` (has a namespace).
pub(crate) fn is_namespaced(item: &RenderItem) -> bool {
    matches!(item, RenderItem::Element(el) if el.identity.namespace.is_some())
}

/// A short description of an element's `(args)`, for a caller that falls
/// back to a generic card.
pub(crate) fn summarize_args(args: Option<&Tuple>) -> String {
    match args {
        None => String::new(),
        Some(tuple) => summarize_tuple(tuple),
    }
}

/// A short description of an element's `{value}` group: its pair keys, joined.
pub(crate) fn summarize_data(value: Option<&Group>) -> String {
    match value {
        None => String::new(),
        Some(group) => group
            .pairs()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

pub(crate) fn summarize_tuple(tuple: &Tuple) -> String {
    if tuple.len() == 1 {
        if let Some(entry) = tuple.entries().next() {
            if entry.name().is_none() {
                return summarize_value(entry.value());
            }
        }
    }
    tuple
        .entries()
        .filter_map(|entry| entry.name().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn summarize_scalar(scalar: &Scalar) -> String {
    match scalar {
        Scalar::None => "none".to_string(),
        Scalar::Auto => "auto".to_string(),
        Scalar::Bool(b) => b.to_string(),
        Scalar::Int(i) => i.to_string(),
        Scalar::Float(f) => f.to_string(),
        Scalar::Number(s) | Scalar::Text(s) => s.clone(),
    }
}

pub(crate) fn summarize_value(value: &Value) -> String {
    match value {
        Value::Scalar(s) => summarize_scalar(s),
        Value::Tuple(tuple) => summarize_tuple(tuple),
        Value::Call(name, args) => format!("{name}({} arg(s))", args.len()),
        Value::Interp(_) => "interp".to_string(),
        Value::Hole(_) => "hole".to_string(),
        Value::TypeSeat(_) => "type".to_string(),
        Value::Element(el) => el.name.to_string(),
        Value::Blocks(blocks) => format!("[{} block(s)]", blocks.len()),
    }
}
