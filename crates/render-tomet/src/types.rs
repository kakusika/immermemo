//! Fundamental types and data structures for document element classification.

use tomet::ast::{Position, Span};

/// An element's identity: the `(namespace, name)` pair from its
/// [`tomet::ast::Sigil::Named`] name. `namespace` is `None` for one of
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
    pub fn union(self, other: TextStyle) -> TextStyle {
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
    /// A named element, identified at the source-text level only --
    /// `@use`'d (`identity.namespace: Some`) or one of
    /// [`crate::block::ATOMIC_BARE_ELEMENTS`] (`identity.namespace: None`) alike.
    /// Boxed: an element carries far more than a text run, and most items
    /// are text.
    Element(Box<ElementItem>),
}

/// What [`RenderItem::Element`] knows about one named element.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementItem {
    pub identity: ElementIdentity,
    pub span: Span,
    /// The element's own `[content]` span, if it has one -- `None`
    /// for an element with no bracketed content at all.
    pub content: Option<Span>,
    /// A generic fallback description of `(args)`/`{value}`.
    pub args_summary: String,
    pub data_summary: String,
    /// Every string-valued entry of `(args)`, structured instead of flattened.
    pub args: ElementArgs,
    /// Every `key: [...]` (`Value::Blocks`) entry of `(args)`,
    /// recursively classified the same way this element's own `[content]` is.
    pub block_args: Vec<(String, Vec<RenderItem>)>,
}

/// [`ElementItem::args`]'s shape -- string-valued args only, read
/// generically off `(args)` without knowing what any key means.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ElementArgs {
    #[default]
    None,
    /// `@element("positional string")`.
    Positional(String),
    /// `@element(key: "string", ...)` -- non-string values are dropped.
    Named(Vec<(String, String)>),
}

/// Which kind of source block a group came from.
/// `Heading`'s `usize` is the section's nesting level (`=` is 1, `==` is 2, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBlockKind {
    Paragraph,
    Element,
    Heading(usize),
    ListItem {
        depth: usize,
        ordered: bool,
        index: usize,
    },
}

/// `src` failed to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseFailed;

/// A [`Span`] covering all of `src`, for [`crate::block::classify`]'s parse-failure fallback.
pub(crate) fn whole_source_span(src: &str) -> Span {
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
