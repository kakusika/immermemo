//! Block-level document classification and grouping.

use tomet::ast::{Block, Document, Element, Sigil};

use crate::inline::{
    classify_inline_seq, element_item, inline_content, style_wrapper, walk_inline_seq,
};
use crate::meta::is_metadata_element_ast;
use crate::types::{ParseFailed, RenderBlockKind, RenderItem, TextStyle, whole_source_span};

/// Bare built-in names that stand as a thing of their own.
pub(crate) const ATOMIC_BARE_ELEMENTS: &[&str] = &["ruby", "link", "conflict", "task"];

/// Whether `el`'s sigil is a bare (non-`@use`'d) name in [`ATOMIC_BARE_ELEMENTS`].
pub(crate) fn is_atomic_bare(el: &Element) -> bool {
    let Sigil::Named(name) = &el.sigil else {
        return false;
    };
    name.namespace.is_none() && ATOMIC_BARE_ELEMENTS.contains(&name.name.as_str())
}

/// Classifies `src`'s top-level blocks (and, for a [`Block::Paragraph`],
/// its inline content) into [`RenderItem`]s, in source order.
pub fn classify(src: &str) -> Vec<RenderItem> {
    let doc: Document = match tomet::parser::parse_document(src) {
        Ok(doc) => doc,
        Err(_) => {
            return vec![RenderItem::Text(
                whole_source_span(src),
                TextStyle::default(),
            )];
        }
    };
    doc.blocks
        .iter()
        .flat_map(|b| classify_block_groups(b, true))
        .flat_map(|(_, items)| items)
        .collect()
}

/// Keeps each source block's [`RenderItem`]s grouped (and tagged with [`RenderBlockKind`])
/// instead of flattening them into one document-wide list.
pub fn classify_blocks(src: &str) -> Result<Vec<(RenderBlockKind, Vec<RenderItem>)>, ParseFailed> {
    let doc: Document = tomet::parser::parse_document(src).map_err(|_| ParseFailed)?;
    Ok(doc
        .blocks
        .iter()
        .flat_map(|b| classify_block_groups(b, true))
        .collect())
}

/// `block`'s groups -- plural, because a [`Block::Section`] contributes
/// more than one.
pub(crate) fn classify_block_groups(
    block: &Block,
    skip_metadata: bool,
) -> Vec<(RenderBlockKind, Vec<RenderItem>)> {
    match block {
        Block::Paragraph(p) => vec![(RenderBlockKind::Paragraph, classify_inline_seq(&p.content))],
        Block::Element(el) => {
            if skip_metadata && is_metadata_element_ast(el) {
                return Vec::new();
            }
            let items = classify_bare_character_element(el).unwrap_or_else(|| {
                vec![
                    element_item(el)
                        .unwrap_or_else(|| RenderItem::Text(el.span, TextStyle::default())),
                ]
            });
            vec![(RenderBlockKind::Element, items)]
        }
        Block::Section(section) => {
            let mut groups = vec![(
                RenderBlockKind::Heading(section.level),
                classify_inline_seq(&section.title),
            )];
            groups.extend(
                section
                    .blocks
                    .iter()
                    .flat_map(|b| classify_block_groups(b, skip_metadata)),
            );
            groups
        }
        Block::List(list) => {
            let has_named = list
                .items
                .iter()
                .any(|item| matches!(item.element.sigil, Sigil::Named(_)));
            if !has_named {
                vec![(
                    RenderBlockKind::Paragraph,
                    vec![RenderItem::Text(list.span, TextStyle::default())],
                )]
            } else {
                let mut groups = Vec::new();
                for item in &list.items {
                    match &item.element.sigil {
                        Sigil::Named(_) => {
                            if skip_metadata && is_metadata_element_ast(&item.element) {
                                continue;
                            }
                            let items = classify_bare_character_element(&item.element)
                                .unwrap_or_else(|| {
                                    vec![element_item(&item.element).unwrap_or_else(|| {
                                        RenderItem::Text(item.element.span, TextStyle::default())
                                    })]
                                });
                            groups.push((RenderBlockKind::Element, items));
                        }
                        _ => {
                            groups.push((
                                RenderBlockKind::Paragraph,
                                vec![RenderItem::Text(item.element.span, TextStyle::default())],
                            ));
                        }
                    }
                    if let Some(sublist) = &item.sublist {
                        groups.extend(classify_block_groups(
                            &Block::List(sublist.clone()),
                            skip_metadata,
                        ));
                    }
                }
                groups
            }
        }
    }
}

/// `el`'s items if it's a bare `em`/`strong`/`mark`/`strikeout`, or one of [`ATOMIC_BARE_ELEMENTS`].
pub(crate) fn classify_bare_character_element(el: &Element) -> Option<Vec<RenderItem>> {
    if is_atomic_bare(el) {
        return Some(vec![element_item(el).expect(
            "is_atomic_bare already confirmed el.sigil is Sigil::Named",
        )]);
    }
    let wrapper = style_wrapper(el)?;
    let content = inline_content(el)?;
    let mut items = Vec::new();
    let mut run = None;
    walk_inline_seq(content, wrapper, &mut items, &mut run);
    if let Some((span, style)) = run {
        items.push(RenderItem::Text(span, style));
    }
    Some(items)
}

/// `blocks` classified into one flat `RenderItem` list.
pub(crate) fn classify_blocks_flat(blocks: &[Block]) -> Vec<RenderItem> {
    blocks
        .iter()
        .flat_map(|b| classify_block_groups(b, false))
        .flat_map(|(_, items)| items)
        .collect()
}
