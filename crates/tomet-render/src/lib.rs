//! Classifies a note's top-level blocks into plain text and recognized
//! elements, so the editor can hide `@element(...)` syntax behind a
//! block-like presentation without parsing anything itself.
//!
//! This crate only answers "what is this span of the document" -- it never
//! renders anything. The actual cards/chips are Slint components living in
//! `apps/slint`, the same split `crates/vault`/`crates/merge` already draw
//! between UI-framework-agnostic domain logic and the UI that uses it: a
//! crate with no Slint dependency can be unit-tested against `.tmt`
//! fixtures directly, which a crate wired into a widget tree cannot.
//!
//! # Scope
//!
//! [`tomet_ast::Block::Element`] is classified directly, and so is a
//! [`tomet_ast::Block::Paragraph`]'s inline content -- a recognized
//! [`tomet_ast::Inline::Element`] becomes its own [`RenderItem::Element`],
//! splitting the surrounding plain-text runs around it the same way a
//! block-level marker splits the blocks around it. This only covers
//! *view-mode* rendering (a read-only display with no cursor or selection
//! to keep coherent); the original worry about inline elements -- cursor
//! movement and selection mixing with surrounding text runs -- is strictly
//! an *editing* concern and doesn't apply here. Editing still shows raw
//! source text unconditionally; that's a caller decision (apps/slint's
//! edit-mode `TextInput`), not something this crate restricts.
//!
//! [`tomet_ast::Section`]'s own inline content (its title) is *not* walked
//! -- conflicts there are rarer than in a paragraph (the one concrete
//! motivation for this crate), and every other block kind (bare built-in
//! elements, a `Section`'s nested blocks) still classifies as one opaque
//! [`RenderItem::Text`] span, same as before.
//!
//! # Parse failures fall back to one plain-text span
//!
//! [`tomet_parser::parse_document`] has no error recovery: it either
//! parses the whole document or fails outright, so a user mid-keystroke on
//! something like `@mobile.conflict(` can turn the *entire* note into an
//! unparseable one. [`classify`] treats that the same as "nothing
//! recognized" -- one [`RenderItem::Text`] spanning the source -- which is
//! exactly today's always-plain-text experience, not a regression. Once
//! [`tomet_parser`] gains partial recovery, this crate can classify
//! further into whatever it did manage to parse.
//!
//! # Every named element gets an identity -- block and inline differ in
//! # what they do with it
//!
//! [`ElementIdentity::namespace`] is `Some` for an element that came in
//! through `@use`, `None` for one of Tomet's own bare built-in vocabulary
//! (`ol`/`ul` and the like) -- both are reported, at both block and inline
//! level: [`classify`] itself never decides that a bare identity is
//! uninteresting, because that decision depends on what the *caller* wants
//! to do with it (give it a registered look, or leave it exactly as
//! written), not on anything `classify` can know from syntax alone. That
//! keeps this crate's only dependency `tomet-ast` itself -- deciding "is
//! `ol` really structural" would be `tomet-semantics`'s `classify_std*`
//! job, and this crate never needs to ask that question: identity here is
//! read straight off [`tomet_ast::Sigil::Named`]'s `Name`, nothing more.
//!
//! What *does* still differ between block and inline is splitting: a
//! block-level element is always its own top-level item regardless of its
//! identity (blocks were never merged with their neighbors to begin
//! with), but inline splitting only pulls a *namespaced* element out of
//! its surrounding prose -- a bare inline element (if Tomet's vocabulary
//! ever has one) stays merged into the running text span around it,
//! because splitting prose on every lightweight inline marker would
//! fragment a sentence a reader expects to flow as one run. The first and
//! only namespaced element this crate's caller currently cares about is
//! `immermemo_merge`'s `@mobile.conflict` marker, but [`classify`] does
//! not special-case it: it reports every element's [`ElementIdentity`]
//! uniformly, and leaves "do I recognize this `(namespace, name)` pair,
//! or should it fall back to something generic" to whatever renders
//! [`RenderItem`]s (today: `immermemo-editor`'s `REGISTRY`).

use tomet_ast::{
    Block, Document, Element, ElementValue, Entry, Inline, Position, Sigil, Span, Value,
};

/// An element's identity: the `(namespace, name)` pair from its
/// [`tomet_ast::Sigil::Named`] name. `namespace` is `None` for one of
/// Tomet's own bare built-in elements (came in with no `@use`), `Some`
/// for one brought in through `@use`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementIdentity {
    pub namespace: Option<String>,
    pub name: String,
}

/// One classified region of a document's top-level blocks.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderItem {
    /// Plain text -- display exactly as encountered in the source.
    Text(Span),
    /// A named element, identified at the source-text level only.
    /// `args_summary`/`data_summary` are a generic fallback description of
    /// its `(args)`/`{value}` (see [`summarize_args`]/[`summarize_data`])
    /// for a caller that doesn't recognize `identity` and wants something
    /// better than nothing to show.
    Element {
        identity: ElementIdentity,
        span: Span,
        args_summary: String,
        data_summary: String,
    },
}

/// Classifies `src`'s top-level blocks (and, for a [`Block::Paragraph`],
/// its inline content) into [`RenderItem`]s, in source order. See the
/// module doc for the parse-failure fallback and the namespace-gating
/// rule.
pub fn classify(src: &str) -> Vec<RenderItem> {
    let doc: Document = match tomet_parser::parse_document(src) {
        Ok(doc) => doc,
        Err(_) => return vec![RenderItem::Text(whole_source_span(src))],
    };
    doc.blocks.iter().flat_map(classify_block).collect()
}

fn classify_block(block: &Block) -> Vec<RenderItem> {
    match block {
        Block::Element(el) => vec![element_item(el).unwrap_or_else(|| RenderItem::Text(el.span))],
        Block::Paragraph(p) => classify_inline_seq(&p.content),
        Block::Section(_) => vec![RenderItem::Text(block.span())],
    }
}

/// Which kind of source block a [`classify_blocks`] group came from.
/// [`classify`]'s flat `Vec<RenderItem>` loses this: a `Block::Paragraph`
/// split around inline elements and a run of independent `Block::Element`s
/// look identical once flattened, but only the former should ever be laid
/// out as one flowing line (see `immermemo-editor`'s `flow` module, which
/// needs this distinction to group inline items back into one block).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBlockKind {
    Paragraph,
    Element,
    Section,
}

fn block_kind(block: &Block) -> RenderBlockKind {
    match block {
        Block::Element(_) => RenderBlockKind::Element,
        Block::Paragraph(_) => RenderBlockKind::Paragraph,
        Block::Section(_) => RenderBlockKind::Section,
    }
}

/// `src` failed to parse -- see the module doc's "Parse failures fall back
/// to one plain-text span" section. [`classify`] absorbs this into a
/// single [`RenderItem::Text`]; [`classify_blocks`] has no sensible single
/// group to fall back to without lying about [`RenderBlockKind`], so it
/// reports the failure instead and leaves the fallback decision to the
/// caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseFailed;

/// Like [`classify`], but keeps each source block's [`RenderItem`]s
/// grouped (and tagged with [`RenderBlockKind`]) instead of flattening
/// them into one document-wide list. A [`RenderBlockKind::Paragraph`]
/// group may hold several items (prose split around inline elements -- see
/// [`classify_inline_seq`]); every other kind holds exactly one.
pub fn classify_blocks(src: &str) -> Result<Vec<(RenderBlockKind, Vec<RenderItem>)>, ParseFailed> {
    let doc: Document = tomet_parser::parse_document(src).map_err(|_| ParseFailed)?;
    Ok(doc
        .blocks
        .iter()
        .map(|block| (block_kind(block), classify_block(block)))
        .collect())
}

/// Splits a run of [`Inline`]s into [`RenderItem`]s: a *namespaced*
/// element becomes its own [`RenderItem::Element`], and everything else
/// (plain prose, and a bare inline element if Tomet's vocabulary ever has
/// one -- see the module doc) is coalesced into one [`RenderItem::Text`]
/// run via [`Span::union`], the same way adjacent prose reads as one run
/// rather than one item per word.
fn classify_inline_seq(content: &[Inline]) -> Vec<RenderItem> {
    let mut items = Vec::new();
    let mut run: Option<Span> = None;
    for inline in content {
        let element = match inline {
            Inline::Element(el) => element_item(el).filter(is_namespaced),
            Inline::Text(_) | Inline::SoftBreak(_) | Inline::LineBreak(_) | Inline::Raw(_) => None,
        };
        match element {
            Some(item) => {
                if let Some(span) = run.take() {
                    items.push(RenderItem::Text(span));
                }
                items.push(item);
            }
            None => {
                let span = inline.span();
                run = Some(match run {
                    Some(existing) => existing.union(&span),
                    None => span,
                });
            }
        }
    }
    if let Some(span) = run {
        items.push(RenderItem::Text(span));
    }
    items
}

/// `el`'s [`RenderItem::Element`], for any named element -- `None` only
/// for [`Sigil::Bare`]/`Dollar`/`Caret`, which have no [`tomet_ast::Name`]
/// at all and so carry no identity to report. `identity.namespace` is
/// `None` for one of Tomet's own bare built-in elements; see the module
/// doc for how block and inline classification treat that differently.
fn element_item(el: &Element) -> Option<RenderItem> {
    let Sigil::Named(name) = &el.sigil else {
        return None;
    };
    Some(RenderItem::Element {
        identity: ElementIdentity {
            namespace: name.namespace.clone(),
            name: name.name.clone(),
        },
        span: el.span,
        args_summary: summarize_args(el.args.as_ref()),
        data_summary: summarize_data(el.value.as_ref()),
    })
}

/// Whether `item` is a [`RenderItem::Element`] that came in through
/// `@use` (has a namespace) -- see [`classify_inline_seq`].
fn is_namespaced(item: &RenderItem) -> bool {
    matches!(item, RenderItem::Element { identity, .. } if identity.namespace.is_some())
}

/// A short description of an element's `(args)`, for a caller that falls
/// back to a generic card. Not meant to round-trip -- just enough to show
/// something.
fn summarize_args(args: Option<&Value>) -> String {
    match args {
        None => String::new(),
        Some(value) => summarize_value(value),
    }
}

/// A short description of an element's `{value}` group: its pair keys,
/// joined, skipping nested elements. Not meant to round-trip, same as
/// [`summarize_args`].
fn summarize_data(value: Option<&ElementValue>) -> String {
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
        Some(ElementValue::Raw(_)) => "raw".to_string(),
        Some(ElementValue::Interp(_)) => "interp".to_string(),
    }
}

fn summarize_value(value: &Value) -> String {
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
    }
}

/// A [`Span`] covering all of `src`, for [`classify`]'s parse-failure
/// fallback -- [`tomet_parser::parse_document`] gives up before producing a
/// [`Document::span`] to reuse, so this walks `src` itself to find the end
/// line/column (1-indexed, matching [`Position`]'s own convention).
fn whole_source_span(src: &str) -> Span {
    let mut line = 1;
    let mut column = 1;
    for ch in src.chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    Span::new(
        Position::new(1, 1, 0),
        Position::new(line, column, src.len()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_paragraph_is_one_text_item() {
        let items = classify("Just a plain paragraph.\n");
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], RenderItem::Text(_)));
    }

    #[test]
    fn a_bare_builtin_element_is_identified_with_no_namespace() {
        // A list is `Block::Element` with a bare (un-namespaced) name --
        // see the module doc. It's still reported as an `Element`, not
        // silently folded into `Text`: a caller that wants to give a bare
        // built-in its own look later needs this identity to key a lookup
        // on, even though today's caller (`immermemo-editor`) falls back
        // to showing it exactly as written, same as before this existed.
        let items = classify("- one\n- two\n");
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Element { identity, .. } => {
                assert_eq!(identity.namespace, None);
            }
            RenderItem::Text(_) => panic!("expected an Element item"),
        }
    }

    #[test]
    fn a_mobile_conflict_marker_triple_is_identified() {
        let src = "Before.\n\n\
                   @mobile.conflict(mine)\n\n\
                   Mine text.\n\n\
                   @mobile.conflict(theirs)\n\n\
                   Theirs text.\n\n\
                   @mobile.conflict(end)\n\n\
                   After.\n";
        let items = classify(src);

        let identities: Vec<&ElementIdentity> = items
            .iter()
            .filter_map(|item| match item {
                RenderItem::Element { identity, .. } => Some(identity),
                RenderItem::Text(_) => None,
            })
            .collect();
        assert_eq!(identities.len(), 3);
        for identity in identities {
            assert_eq!(identity.namespace.as_deref(), Some("mobile"));
            assert_eq!(identity.name, "conflict");
        }

        let args_summaries: Vec<&str> = items
            .iter()
            .filter_map(|item| match item {
                RenderItem::Element { args_summary, .. } => Some(args_summary.as_str()),
                RenderItem::Text(_) => None,
            })
            .collect();
        assert_eq!(args_summaries, vec!["mine", "theirs", "end"]);
    }

    #[test]
    fn an_inline_conflict_marker_splits_the_paragraph_around_it() {
        // The common conflict shape: both sides edited the same sentence,
        // which `immermemo_merge::merge_one` always narrows to inline
        // markers (never a whole-block marker) for a Paragraph/Paragraph/
        // Paragraph triple. This is the case the module doc's "Scope"
        // section added coverage for.
        let src = "Before @mobile.conflict(mine)mine text@mobile.conflict(theirs)their text@mobile.conflict(end) after.\n";
        let items = classify(src);

        let kinds: Vec<&str> = items
            .iter()
            .map(|item| match item {
                RenderItem::Text(_) => "text",
                RenderItem::Element { .. } => "element",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "text", "element", "text", "element", "text", "element", "text"
            ]
        );

        let RenderItem::Text(first) = &items[0] else {
            unreachable!()
        };
        assert_eq!(&src[first.start.offset..first.end.offset], "Before ");

        let RenderItem::Text(last) = items.last().unwrap() else {
            unreachable!()
        };
        assert_eq!(&src[last.start.offset..last.end.offset], " after.");
    }

    #[test]
    fn an_unknown_namespaced_element_gets_a_generic_summary() {
        let items = classify("@deck.bookmark(label: \"Chapter 1\")\n");
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Element {
                identity,
                args_summary,
                data_summary,
                ..
            } => {
                assert_eq!(identity.namespace.as_deref(), Some("deck"));
                assert_eq!(identity.name, "bookmark");
                assert_eq!(args_summary, "label");
                assert_eq!(data_summary, "");
            }
            RenderItem::Text(_) => panic!("expected an Element item"),
        }
    }

    #[test]
    fn unparseable_source_falls_back_to_one_text_item_spanning_everything() {
        let src = "Fine paragraph.\n\n@mobile.conflict(\n";
        let items = classify(src);
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Text(span) => {
                assert_eq!(span.start.offset, 0);
                assert_eq!(span.end.offset, src.len());
            }
            RenderItem::Element { .. } => panic!("expected a Text fallback item"),
        }
    }

    #[test]
    fn classify_blocks_groups_inline_conflict_items_under_one_paragraph() {
        // Same fixture `immermemo-editor`'s classify.rs proves goes through
        // the real merge pipeline -- here just checking `classify_blocks`
        // keeps all of a paragraph's split items in one group, unlike
        // `classify`'s flat list.
        let src = "@use(mobile)\nThe @mobile.conflict(mine)slow@mobile.conflict(theirs)lazy@mobile.conflict(end) fox jumps.\n";
        let groups = classify_blocks(src).unwrap();

        // `@use(mobile)` is its own bare `Block::Element`, then the
        // sentence is one `Block::Paragraph` holding every inline item.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, RenderBlockKind::Element);
        assert_eq!(groups[0].1.len(), 1);
        assert_eq!(groups[1].0, RenderBlockKind::Paragraph);
        assert_eq!(groups[1].1.len(), 7);
    }

    #[test]
    fn classify_blocks_reports_parse_failure_instead_of_guessing_a_kind() {
        let src = "Fine paragraph.\n\n@mobile.conflict(\n";
        assert_eq!(classify_blocks(src), Err(ParseFailed));
    }
}
