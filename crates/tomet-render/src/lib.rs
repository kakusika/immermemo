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
//! its surrounding prose -- an unrecognized bare inline element stays
//! merged into the running text span around it, because splitting prose
//! on every lightweight inline marker would fragment a sentence a reader
//! expects to flow as one run. The first and only namespaced element this
//! crate's caller currently cares about is `immermemo_merge`'s
//! `@mobile.conflict` marker, but [`classify`] does not special-case it:
//! it reports every element's [`ElementIdentity`] uniformly, and leaves
//! "do I recognize this `(namespace, name)` pair, or should it fall back
//! to something generic" to whatever renders [`RenderItem`]s (today:
//! `immermemo-editor`'s `REGISTRY`).
//!
//! Tomet's five bare built-in "character" elements (`em`/`strong`/`mark`/
//! `strikeout`/`ruby` -- `docs/spec/builtin-elements.tmt`'s `[ 文字 ]`
//! section) are the exception to "bare stays merged": `em`/`strong`/
//! `mark`/`strikeout` recurse into their own `content` and tag the
//! [`RenderItem::Text`] runs inside with a [`TextStyle`] flag instead of
//! splitting them out -- they're a style on surrounding prose, not an
//! atomic thing of their own, and nesting composes (`@strong[@em[x]]` is
//! bold *and* italic). `ruby` is different again: a reading annotation
//! can't be expressed as a style flag on a text run, so it becomes its own
//! [`RenderItem::Ruby`], the same way a namespaced element splits out.

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

/// Which of Tomet's four bare inline style wrappers (`em`/`strong`/`mark`/
/// `strikeout`) are active over a [`RenderItem::Text`] run. More than one
/// bit can be set -- the wrappers nest (`@strong[@em[x]]`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
    pub mark: bool,
    pub strikeout: bool,
}

impl TextStyle {
    /// The style with every flag `self` or `other` sets -- how nested
    /// wrappers combine (`@strong[@em[x]]` is `self.bold || other.italic`
    /// and vice versa, applied as the walk descends).
    fn union(self, other: TextStyle) -> TextStyle {
        TextStyle {
            bold: self.bold || other.bold,
            italic: self.italic || other.italic,
            mark: self.mark || other.mark,
            strikeout: self.strikeout || other.strikeout,
        }
    }
}

/// One classified region of a document's top-level blocks.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderItem {
    /// Plain text -- display exactly as encountered in the source, with
    /// whatever `em`/`strong`/`mark`/`strikeout` wrappers it was nested
    /// inside flattened into `style`.
    Text(Span, TextStyle),
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
    /// A bare `@ruby[base](rt:"reading")` -- `base` is `content`'s own
    /// span (not the whole element's, which would include the `(rt:...)`
    /// part too). `reading` is empty if `rt` was missing or not a string
    /// (`docs/spec/builtin-elements.tmt`: "検証は未実装", not validated,
    /// same scope decision as elsewhere in this crate).
    Ruby { base: Span, reading: String },
}

/// Classifies `src`'s top-level blocks (and, for a [`Block::Paragraph`],
/// its inline content) into [`RenderItem`]s, in source order. See the
/// module doc for the parse-failure fallback and the namespace-gating
/// rule.
pub fn classify(src: &str) -> Vec<RenderItem> {
    let doc: Document = match tomet_parser::parse_document(src) {
        Ok(doc) => doc,
        Err(_) => {
            return vec![RenderItem::Text(
                whole_source_span(src),
                TextStyle::default(),
            )];
        }
    };
    doc.blocks.iter().flat_map(classify_block).collect()
}

fn classify_block(block: &Block) -> Vec<RenderItem> {
    match block {
        Block::Element(el) => classify_bare_character_element(el).unwrap_or_else(|| {
            vec![
                element_item(el).unwrap_or_else(|| RenderItem::Text(el.span, TextStyle::default())),
            ]
        }),
        Block::Paragraph(p) => classify_inline_seq(&p.content),
        Block::Section(_) => vec![RenderItem::Text(block.span(), TextStyle::default())],
    }
}

/// `el`'s items if it's a bare `em`/`strong`/`mark`/`strikeout`/`ruby` --
/// the same five characters [`classify_inline_seq`] recognizes inline, for
/// the rarer case where one stands as an entire paragraph-less block by
/// itself (nothing else shares its line, so it parses as `Block::Element`
/// rather than `Block::Paragraph` -- see the module doc).
fn classify_bare_character_element(el: &Element) -> Option<Vec<RenderItem>> {
    if is_bare_named(el, "ruby") {
        return Some(vec![ruby_item(el)]);
    }
    let wrapper = style_wrapper(el)?;
    let content = el.content.as_ref()?;
    let mut items = Vec::new();
    let mut run = None;
    walk_inline_seq(content, wrapper, &mut items, &mut run);
    if let Some((span, style)) = run {
        items.push(RenderItem::Text(span, style));
    }
    Some(items)
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
/// element becomes its own [`RenderItem::Element`], a bare `ruby` becomes
/// its own [`RenderItem::Ruby`], a bare `em`/`strong`/`mark`/`strikeout`
/// recurses into its own content with that style flag added (see the
/// module doc), and everything else (plain prose, and any other
/// unrecognized bare inline element) is coalesced into one
/// [`RenderItem::Text`] run per contiguous same-[`TextStyle`] stretch via
/// [`Span::union`], the same way adjacent prose reads as one run rather
/// than one item per word.
fn classify_inline_seq(content: &[Inline]) -> Vec<RenderItem> {
    let mut items = Vec::new();
    let mut run: Option<(Span, TextStyle)> = None;
    walk_inline_seq(content, TextStyle::default(), &mut items, &mut run);
    if let Some((span, style)) = run {
        items.push(RenderItem::Text(span, style));
    }
    items
}

fn walk_inline_seq(
    content: &[Inline],
    style: TextStyle,
    items: &mut Vec<RenderItem>,
    run: &mut Option<(Span, TextStyle)>,
) {
    for inline in content {
        if let Inline::Element(el) = inline {
            if let Some(wrapper) = style_wrapper(el) {
                if let Some(nested) = &el.content {
                    walk_inline_seq(nested, style.union(wrapper), items, run);
                }
                continue;
            }
            if is_bare_named(el, "ruby") {
                flush_run(items, run);
                items.push(ruby_item(el));
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
/// a different [`TextStyle`] -- a style change always starts a new
/// [`RenderItem::Text`], the same way splitting out an element does.
fn extend_run(
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

fn flush_run(items: &mut Vec<RenderItem>, run: &mut Option<(Span, TextStyle)>) {
    if let Some((span, style)) = run.take() {
        items.push(RenderItem::Text(span, style));
    }
}

/// `el`'s [`TextStyle`] flag if it's a bare (non-`@use`'d) `em`/`strong`/
/// `mark`/`strikeout` wrapper, else `None`.
fn style_wrapper(el: &Element) -> Option<TextStyle> {
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

/// Whether `el`'s sigil is the bare (non-`@use`'d) built-in named `name`.
fn is_bare_named(el: &Element, name: &str) -> bool {
    matches!(&el.sigil, Sigil::Named(n) if n.namespace.is_none() && n.name == name)
}

/// `el`'s [`RenderItem::Ruby`] -- see that variant's doc for `base`/`reading`.
fn ruby_item(el: &Element) -> RenderItem {
    let base = el
        .content
        .as_ref()
        .and_then(|inlines| inlines.iter().map(Inline::span).reduce(|a, b| a.union(&b)))
        .unwrap_or(el.span);
    let reading = match &el.args {
        Some(Value::Map(entries)) => entries
            .iter()
            .find_map(|(key, value)| match (key.as_str(), value) {
                ("rt", Value::String(s)) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default(),
        _ => String::new(),
    };
    RenderItem::Ruby { base, reading }
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
        assert!(matches!(items[0], RenderItem::Text(_, _)));
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
            _ => panic!("expected an Element item"),
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
                _ => None,
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
                _ => None,
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
                RenderItem::Text(_, _) => "text",
                RenderItem::Element { .. } => "element",
                RenderItem::Ruby { .. } => "ruby",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "text", "element", "text", "element", "text", "element", "text"
            ]
        );

        let RenderItem::Text(first, _) = &items[0] else {
            unreachable!()
        };
        assert_eq!(&src[first.start.offset..first.end.offset], "Before ");

        let RenderItem::Text(last, _) = items.last().unwrap() else {
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
            _ => panic!("expected an Element item"),
        }
    }

    #[test]
    fn unparseable_source_falls_back_to_one_text_item_spanning_everything() {
        let src = "Fine paragraph.\n\n@mobile.conflict(\n";
        let items = classify(src);
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Text(span, _) => {
                assert_eq!(span.start.offset, 0);
                assert_eq!(span.end.offset, src.len());
            }
            _ => panic!("expected a Text fallback item"),
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

    fn text_items(src: &str) -> Vec<(String, TextStyle)> {
        classify(src)
            .into_iter()
            .map(|item| match item {
                RenderItem::Text(span, style) => {
                    (src[span.start.offset..span.end.offset].to_owned(), style)
                }
                other => panic!("expected only Text items, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_bare_strong_wrapper_tags_its_content_bold_without_splitting_it_out() {
        let items = text_items("plain @strong[bold] plain\n");
        assert_eq!(
            items,
            vec![
                ("plain ".to_owned(), TextStyle::default()),
                (
                    "bold".to_owned(),
                    TextStyle {
                        bold: true,
                        ..TextStyle::default()
                    }
                ),
                (" plain".to_owned(), TextStyle::default()),
            ]
        );
    }

    #[test]
    fn nested_style_wrappers_combine_their_flags() {
        // A line that's *only* `@element[...]` parses as its own
        // top-level `Block::Element`, not a `Paragraph` -- same rule a
        // standalone `@mobile.conflict(...)` marker line follows (see the
        // module doc). Surrounding prose on the same line is what forces
        // `Block::Paragraph` and so the inline-walk path this test means
        // to exercise.
        let items = text_items("plain @strong[@em[both]] plain\n");
        assert_eq!(
            items,
            vec![
                ("plain ".to_owned(), TextStyle::default()),
                (
                    "both".to_owned(),
                    TextStyle {
                        bold: true,
                        italic: true,
                        ..TextStyle::default()
                    }
                ),
                (" plain".to_owned(), TextStyle::default()),
            ]
        );
    }

    #[test]
    fn a_namespaced_element_nested_inside_a_style_wrapper_still_splits_out() {
        let src = "plain @strong[before @deck.bookmark(label: x) after] plain\n";
        let items = classify(src);
        let kinds: Vec<&str> = items
            .iter()
            .map(|item| match item {
                RenderItem::Text(_, _) => "text",
                RenderItem::Element { .. } => "element",
                RenderItem::Ruby { .. } => "ruby",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "text", "element", "text", "text"]);
        let RenderItem::Text(inside, inside_style) = &items[1] else {
            unreachable!()
        };
        assert_eq!(&src[inside.start.offset..inside.end.offset], "before ");
        assert!(inside_style.bold);
    }

    #[test]
    fn a_bare_ruby_element_splits_out_with_its_base_and_reading() {
        let src = "plain @ruby[漢字](rt:\"かんじ\") plain\n";
        let items = classify(src);
        assert_eq!(items.len(), 3);
        match &items[1] {
            RenderItem::Ruby { base, reading } => {
                assert_eq!(&src[base.start.offset..base.end.offset], "漢字");
                assert_eq!(reading, "かんじ");
            }
            other => panic!("expected a Ruby item, got {other:?}"),
        }
    }

    #[test]
    fn a_ruby_element_with_a_missing_or_non_string_reading_falls_back_to_empty() {
        let items = classify("plain @ruby[漢字] plain\n");
        match &items[1] {
            RenderItem::Ruby { reading, .. } => assert_eq!(reading, ""),
            other => panic!("expected a Ruby item, got {other:?}"),
        }
    }

    #[test]
    fn a_style_wrapper_alone_on_its_own_line_is_still_tagged_not_just_shown_raw() {
        // Nothing else shares this line, so it parses as its own
        // `Block::Element`, not a `Block::Paragraph` -- the rarer path
        // `classify_bare_character_element` exists for.
        let items = text_items("@strong[whole line]\n");
        assert_eq!(
            items,
            vec![(
                "whole line".to_owned(),
                TextStyle {
                    bold: true,
                    ..TextStyle::default()
                }
            )]
        );
    }

    #[test]
    fn a_ruby_element_alone_on_its_own_line_still_splits_out() {
        let src = "@ruby[漢字](rt:\"かんじ\")\n";
        let items = classify(src);
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Ruby { base, reading } => {
                assert_eq!(&src[base.start.offset..base.end.offset], "漢字");
                assert_eq!(reading, "かんじ");
            }
            other => panic!("expected a Ruby item, got {other:?}"),
        }
    }
}
