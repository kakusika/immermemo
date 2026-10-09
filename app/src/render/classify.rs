//! Classifies a note body into the blocks the editor's view-mode display
//! draws. Nothing here touches a Slint-generated type, even though this
//! module lives in the Slint app crate now (folded in from
//! `immermemo-editor` -- see `render`'s own module doc for why): the
//! seam that actually does, `super`'s `to_rendered_block` and friends,
//! converts [`ClassifiedBlock`] into the Slint-generated `RenderedBlock`
//! model `EditorScreen`'s `rendered-blocks` property expects.
//!
//! `immermemo-tomet-render`'s `classify()` reports every named element's
//! `(namespace, name)` identity uniformly -- `@use`'d ones and Tomet's own
//! bare built-ins alike (`namespace` is `None` for the latter, including
//! `ruby`/`link` -- `tomet-render` doesn't treat those as a thing apart
//! from any other element either). Deciding what a given identity *means*,
//! visually, is left to whoever renders it. [`REGISTRY`] below is that
//! decision for this app: it maps a recognized identity to a [`Look`] (a
//! drawing primitive [`BlockShape`], a [`Tone`], and a label), so giving a
//! new vocabulary -- `@use`'d or bare built-in -- its own presentation is
//! "add one table entry," not "add a match arm in three different
//! places." Nothing vocabulary-specific belongs in [`BlockShape`]/[`Tone`]
//! themselves -- those are a closed set of drawing primitives (not a
//! vocabulary registry), the same way a design system has a fixed set of
//! component kinds that any number of use-sites can be mapped onto.
//!
//! An identity with no [`REGISTRY`] entry falls back differently depending
//! on where it came from:
//! - A `@use`'d element (`namespace: Some`) falls back to a generic
//!   [`BlockShape::Badge`] labeled from its source text -- its raw
//!   `@ns.name(...)` syntax would look like broken code left as plain
//!   text, so showing *something* beats nothing.
//! - A bare built-in (`namespace: None`, e.g. a list) falls back to
//!   [`BlockShape::PlainText`] showing its source exactly as written --
//!   its own syntax (e.g. `- item`) already reads fine unrendered, so
//!   nothing this app hasn't explicitly opted into should change how it
//!   looks.

use immermemo_tomet_render::{ElementArgs, ElementIdentity, RenderItem, TextStyle, classify};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockShape {
    /// A paragraph's plain content, shown as-is -- `style` may still mark
    /// the whole thing bold/italic/mark/strikeout if it was a single
    /// `em`/`strong`/`mark`/`strikeout`-wrapped block with nothing else
    /// sharing its line (see `immermemo_tomet_render`'s
    /// `classify_bare_character_element`).
    PlainText,
    /// A `@conflict(a: [...], b: [...])` -- `ClassifiedBlock::side_a`/
    /// `side_b` hold each side's own content, classified the same way the
    /// surrounding document's blocks are (see [`ConflictLeafBlock`]).
    /// `text`/`reading` are unused (empty), same as `Divider` used to be --
    /// this shape's look comes entirely from `side_a`/`side_b`.
    Conflict,
    /// A neutral pill with a label -- the fallback for anything
    /// namespaced that [`REGISTRY`] doesn't recognize.
    Badge,
    /// A `@ruby[base](rt:"reading")` standing as an entire block by
    /// itself -- `text` is the base, `reading` the annotation. Rarer than
    /// the inline case (`super::flow`), which is what a ruby annotation
    /// mid-sentence actually looks like. [`ruby_look`] is what gives the
    /// bare `(None, "ruby")` identity this shape.
    Ruby,
    /// A bare `@link`/`@std.link` standing as an entire block by itself
    /// -- `text` is its display content, or the target itself when there
    /// is no `[content]`. Static, same as every other shape here: this
    /// only shows where a link sits, it doesn't make it tappable.
    /// [`link_look`] is what gives the bare `(None, "link")` identity
    /// this shape.
    Link,
    /// A `@doc.icon(name, pkg)` -- `text` is `name` (the icon slug within
    /// its package, e.g. `"star"`), `reading` is `pkg` (e.g. `"tabler"`,
    /// defaulting to it when absent). Resolving that pair into an actual
    /// image is a Slint concern ([`crate::render::to_rendered_block`]),
    /// not this crate's -- this module stays free of `slint`/`origiri_icons`
    /// the same way it stays free of every other UI-framework type.
    /// [`icon_look`] is what gives `(Some("doc"), "icon")` this shape.
    Icon,
}

/// A small, closed palette selector -- not vocabulary-specific, just
/// which of the app's existing semantic colors (see `Colors` in
/// `crates/origiri-mobile/ui/tokens.slint`) a [`Look`] should draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Warning,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedBlock {
    pub shape: BlockShape,
    pub tone: Tone,
    /// The paragraph text for `PlainText`, the base for `Ruby`, or a label
    /// for `Badge`. Unused (empty) for `Conflict` -- its look comes
    /// entirely from `side_a`/`side_b`.
    pub text: String,
    /// Meaningful only for `PlainText` -- see that variant's doc.
    /// `Default::default()` (no style) everywhere else.
    pub style: TextStyle,
    /// The reading annotation, meaningful only for `Ruby` -- copied
    /// straight from the matching [`Look::secondary_text`]. Empty
    /// everywhere else.
    pub reading: String,
    /// `@conflict`'s two sides, meaningful only for `Conflict` -- empty
    /// for every other shape. [`ConflictLeafBlock`], not `ClassifiedBlock`
    /// itself: a side's own content never gets a further `side_a`/`side_b`
    /// of its own, even if it happens to contain another, still-unresolved
    /// `@conflict` (see that type's doc for why, and `conflict_look` for
    /// where these get built).
    pub side_a: Vec<ConflictLeafBlock>,
    pub side_b: Vec<ConflictLeafBlock>,
}

/// A block inside one side of a `@conflict`, classified the same way any
/// other block is -- but never itself carrying `side_a`/`side_b`. Slint's
/// struct/component system can't express a type that nests inside itself
/// (confirmed against the real compiler: a struct field or a component
/// referencing its own type fails to resolve, with no `Loader`-style
/// dynamic-instantiation escape hatch the way QML has one) -- and a
/// `@conflict` only ever has exactly two sides in practice, never an
/// unbounded tree the UI needs to walk, so there was nothing to buy by
/// reaching for one anyway. If a side's content happens to itself contain
/// an unresolved `@conflict` (two unresolved merges stacked before either
/// was resolved), it renders flat/atomic here -- same as any other
/// not-specially-visualized element -- rather than as a second resolvable
/// nested chip. That loses nothing that worked before:
/// `immermemo_merge::find_conflicts`'s own block-sequence walk already
/// stops at the first `@conflict` it finds and never recurses into that
/// element's own `a`/`b` content looking for more, so a nested conflict
/// was never independently reachable from the resolution sheet until the
/// outer one resolves and its chosen side's content is spliced into the
/// document in its place -- same as always.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictLeafBlock {
    pub shape: BlockShape,
    pub tone: Tone,
    pub text: String,
    pub style: TextStyle,
    pub reading: String,
}

impl From<ClassifiedBlock> for ConflictLeafBlock {
    fn from(block: ClassifiedBlock) -> Self {
        ConflictLeafBlock {
            shape: block.shape,
            tone: block.tone,
            text: block.text,
            style: block.style,
            reading: block.reading,
        }
    }
}

// `pub(super)`: `super::flow` also needs a recognized identity's
// shape/tone/text, for the same elements flowed inline instead of stacked.
pub(super) struct Look {
    pub(super) shape: BlockShape,
    pub(super) tone: Tone,
    pub(super) text: String,
    /// The reading annotation, meaningful only for [`BlockShape::Ruby`].
    /// Empty for every other shape.
    pub(super) secondary_text: String,
    /// `@conflict`'s two sides, meaningful only for [`BlockShape::Conflict`].
    /// Empty for every other shape, same convention as `secondary_text`.
    pub(super) side_a: Vec<ConflictLeafBlock>,
    pub(super) side_b: Vec<ConflictLeafBlock>,
}

/// What a [`REGISTRY`] entry's lookup function gets to decide a [`Look`]
/// from: `args` is `immermemo_tomet_render::RenderItem::Element::args`
/// unchanged, `content` is the same item's `content` span already
/// resolved against the note body (so a lookup function never needs to
/// know what a [`tomet::ast::Span`] is or where the body string lives).
/// `block_args`/`body` are for [`conflict_look`] alone -- a lookup
/// function that wants to classify a `key: [...]` arg's own content
/// (rather than just read it as a string, like `args` offers) needs both:
/// `immermemo_tomet_render::RenderItem::Element::block_args` unchanged,
/// and `body` to resolve its items' spans against, the same way
/// [`to_classified_block`] always has.
pub(super) struct LookInput<'a> {
    pub(super) args: &'a ElementArgs,
    pub(super) content: Option<&'a str>,
    pub(super) block_args: &'a [(String, Vec<RenderItem>)],
    pub(super) body: &'a str,
}

/// Which `(namespace, name)` pairs this app recognizes, and how each
/// should look. `namespace: None` registers a look for one of Tomet's own
/// bare built-in elements -- `conflict`/`ruby`/`link` are all three bare,
/// and ordinary entries here: `immermemo_tomet_render` doesn't treat any
/// of them as a thing apart from any other element (see its module doc),
/// and neither does this table. A future vocabulary -- `@use`'d or bare
/// built-in -- gets its own presentation by adding a row here, not by
/// touching [`look_up`] or anything downstream of it.
const REGISTRY: &[(Option<&str>, &str, fn(LookInput) -> Look)] = &[
    (None, "conflict", conflict_look),
    (None, "ruby", ruby_look),
    (None, "link", link_look),
    (Some("doc"), "icon", icon_look),
];

/// The icon package `@doc.icon(name)` resolves against when it omits
/// `pkg` -- the only one `origiri-icons` vendors today (see that crate's
/// `ui/icons/` -- one subdirectory per package, `tabler` the only one so
/// far).
const DEFAULT_ICON_PKG: &str = "tabler";

/// `@conflict(a: [...], b: [...])` -- `a`/`b` are plain structural names,
/// not "mine"/"theirs" (see `immermemo_merge::find_conflicts`'s own module
/// doc for why neither carries that meaning). Resolving the conflict is
/// the resolution sheet's job (`crates/editor/src/conflicts.rs`), not
/// this one's -- this only classifies each side's content for display,
/// via the same [`to_classified_block`] the surrounding document uses,
/// one level deep (see [`ConflictLeafBlock`]'s doc for why not further).
fn conflict_look(input: LookInput) -> Look {
    let side = |key: &str| -> Vec<ConflictLeafBlock> {
        input
            .block_args
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, items)| {
                items
                    .iter()
                    .cloned()
                    .map(|item| to_classified_block(input.body, item).into())
                    .collect()
            })
            .unwrap_or_default()
    };
    Look {
        shape: BlockShape::Conflict,
        tone: Tone::Neutral,
        text: String::new(),
        secondary_text: String::new(),
        side_a: side("a"),
        side_b: side("b"),
    }
}

/// `@ruby[base](rt:"reading")` -- `base` is `input.content`, `reading` is
/// the `rt` named arg (empty if missing or non-string, same
/// "検証は未実装" scope decision `tomet-render` already makes).
fn ruby_look(input: LookInput) -> Look {
    let reading = match input.args {
        ElementArgs::Named(pairs) => pairs
            .iter()
            .find(|(key, _)| key == "rt")
            .map(|(_, value)| value.clone())
            .unwrap_or_default(),
        _ => String::new(),
    };
    Look {
        shape: BlockShape::Ruby,
        tone: Tone::Neutral,
        text: input.content.unwrap_or_default().to_owned(),
        secondary_text: reading,
        side_a: Vec::new(),
        side_b: Vec::new(),
    }
}

/// `@link`/`@std.link` -- `text` is `input.content` (the display text) if
/// present, else the target itself. The target only reads two of
/// `@link`'s several forms correctly: the bare positional string and the
/// explicit `target:` named arg; the typed shortcuts (`url:`/`file:`/
/// `tm:`/`id:`/`ref:`) are normalized into a prefixed `target` string by
/// `tomet-semantics`, which this crate deliberately does not depend on
/// (same reasoning as `immermemo_tomet_render`'s module doc) -- so those
/// are read back as whatever raw value sits under the key, unprefixed.
fn link_look(input: LookInput) -> Look {
    let target = match input.args {
        ElementArgs::Positional(target) => target.clone(),
        ElementArgs::Named(pairs) => pairs
            .iter()
            .find(|(key, _)| {
                matches!(
                    key.as_str(),
                    "target" | "url" | "file" | "tm" | "id" | "ref"
                )
            })
            .map(|(_, value)| value.clone())
            .unwrap_or_default(),
        ElementArgs::None => String::new(),
    };
    Look {
        shape: BlockShape::Link,
        tone: Tone::Accent,
        text: input.content.map(str::to_owned).unwrap_or(target),
        secondary_text: String::new(),
        side_a: Vec::new(),
        side_b: Vec::new(),
    }
}

/// `@doc.icon(name, pkg)`'s `(name, pkg)` -- `name` is positional and
/// required, `pkg` named and optional (`tomet-semantics`' own `doc.icon`
/// declaration), defaulting to [`DEFAULT_ICON_PKG`] when absent. A
/// positional arg mixed with named ones lands under the empty-string key
/// in `tomet`'s AST (confirmed against the real parser, not assumed), so
/// `@doc.icon("star", pkg: "tabler")` and the `pkg`-less `@doc.icon("star")`
/// take different [`ElementArgs`] shapes and both need handling here.
///
/// `pub(crate)`: `directory`'s note-list icon (`@meta{ icon: @doc.icon(...)
/// }`) shares this with [`icon_look`]'s body-inline one rather than
/// re-deriving the same `(name, pkg)` reading twice.
pub(crate) fn icon_name_pkg(args: &ElementArgs) -> (String, String) {
    match args {
        ElementArgs::Positional(name) => (name.clone(), DEFAULT_ICON_PKG.to_owned()),
        ElementArgs::Named(pairs) => {
            let name = pairs
                .iter()
                .find(|(key, _)| key.is_empty())
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            let pkg = pairs
                .iter()
                .find(|(key, _)| key == "pkg")
                .map(|(_, value)| value.clone())
                .unwrap_or_else(|| DEFAULT_ICON_PKG.to_owned());
            (name, pkg)
        }
        ElementArgs::None => (String::new(), DEFAULT_ICON_PKG.to_owned()),
    }
}

fn icon_look(input: LookInput) -> Look {
    let (name, pkg) = icon_name_pkg(input.args);
    Look {
        shape: BlockShape::Icon,
        tone: Tone::Neutral,
        text: name,
        secondary_text: pkg,
        side_a: Vec::new(),
        side_b: Vec::new(),
    }
}

/// `body`'s `@meta{ icon: @doc.icon(name, pkg) }`, if present -- the same
/// `(name, pkg)` shape [`icon_name_pkg`] returns for a body-inline
/// occurrence, so a note's list-row icon and an inline `@doc.icon` in its
/// own body resolve through the same path downstream
/// (`super::icon_image`). `None` for no such meta entry, or one whose
/// value isn't specifically a `(doc, icon)` element (`meta_element`
/// already filters to "is the value an element at all"; this narrows
/// further).
pub(crate) fn note_icon(body: &str) -> Option<(String, String)> {
    let (identity, args) = immermemo_tomet_render::meta_element(body, "icon")?;
    (identity.namespace.as_deref() == Some("doc") && identity.name == "icon")
        .then(|| icon_name_pkg(&args))
}

/// `None` if `identity` has no [`REGISTRY`] entry -- callers decide the
/// fallback themselves, since it differs by whether `identity` came in
/// through `@use` (see the module doc).
///
/// `pub(super)`: `super::flow` calls this too, for the same lookup against
/// an inline (rather than block-level) occurrence of the identity.
pub(super) fn look_up(identity: &ElementIdentity, input: LookInput) -> Option<Look> {
    REGISTRY
        .iter()
        .find(|(namespace, name, _)| {
            *namespace == identity.namespace.as_deref() && *name == identity.name
        })
        .map(|(_, _, look)| look(input))
}

/// A short description of an unrecognized `@use`'d element, for
/// [`BlockShape::Badge`]. Not meant to round-trip -- just enough to show
/// something.
///
/// `pub(super)`: `super::flow` reuses this for the same fallback, inline.
pub(super) fn fallback_label(namespace: &str, name: &str, args_summary: &str) -> String {
    if args_summary.is_empty() {
        format!("{namespace}.{name}")
    } else {
        format!("{namespace}.{name}: {args_summary}")
    }
}

/// Classifies `body`'s top-level blocks for the editor's view-mode display.
pub fn classify_body(body: &str) -> Vec<ClassifiedBlock> {
    classify(body)
        .into_iter()
        .map(|item| to_classified_block(body, item))
        .collect()
}

// `pub(super)`: `super::flow::note_body_items` reuses this for the groups
// it isn't flowing (see that module's doc).
pub(super) fn to_classified_block(body: &str, item: RenderItem) -> ClassifiedBlock {
    match item {
        RenderItem::Text(span, style) => ClassifiedBlock {
            shape: BlockShape::PlainText,
            tone: Tone::Neutral,
            text: body[span.start.offset..span.end.offset].into(),
            style,
            reading: String::new(),
            side_a: Vec::new(),
            side_b: Vec::new(),
        },
        RenderItem::Element {
            identity,
            span,
            content,
            args_summary,
            args,
            block_args,
            ..
        } => {
            let content_text = content.map(|span| &body[span.start.offset..span.end.offset]);
            let input = LookInput {
                args: &args,
                content: content_text,
                block_args: &block_args,
                body,
            };
            match look_up(&identity, input) {
                Some(look) => ClassifiedBlock {
                    shape: look.shape,
                    tone: look.tone,
                    text: look.text,
                    style: TextStyle::default(),
                    reading: look.secondary_text,
                    side_a: look.side_a,
                    side_b: look.side_b,
                },
                None => match &identity.namespace {
                    // A `@use`'d marker nobody's given a look to yet --
                    // show *something* rather than raw `@ns.name(...)`
                    // syntax.
                    Some(namespace) => ClassifiedBlock {
                        shape: BlockShape::Badge,
                        tone: Tone::Neutral,
                        text: fallback_label(namespace, &identity.name, &args_summary),
                        style: TextStyle::default(),
                        reading: String::new(),
                        side_a: Vec::new(),
                        side_b: Vec::new(),
                    },
                    // A bare built-in nobody's registered a look for --
                    // its own syntax already reads fine unrendered (e.g.
                    // a list's `- item` lines), so show it exactly as
                    // written, same as if it had never been classified
                    // as an element at all.
                    None => ClassifiedBlock {
                        shape: BlockShape::PlainText,
                        tone: Tone::Neutral,
                        text: body[span.start.offset..span.end.offset].into(),
                        style: TextStyle::default(),
                        reading: String::new(),
                        side_a: Vec::new(),
                        side_b: Vec::new(),
                    },
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(body: &str) -> Vec<(BlockShape, Tone, String)> {
        classify_body(body)
            .into_iter()
            .map(|b| (b.shape, b.tone, b.text))
            .collect()
    }

    #[test]
    fn plain_text_round_trips_verbatim() {
        // A block's span covers its own content only, not the blank-line
        // separator after it -- hence no trailing "\n" here.
        let blocks = classify_body("Just a paragraph.\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::PlainText);
        assert_eq!(blocks[0].text, "Just a paragraph.");
    }

    #[test]
    fn a_conflict_block_classifies_both_sides() {
        let src = "Before.\n\n\
                   @conflict(a: [Mine text.], b: [Theirs text.])\n\n\
                   After.\n";
        assert_eq!(
            summary(src),
            vec![
                (BlockShape::PlainText, Tone::Neutral, "Before.".to_owned()),
                (BlockShape::Conflict, Tone::Neutral, String::new()),
                (BlockShape::PlainText, Tone::Neutral, "After.".to_owned()),
            ]
        );
        let blocks = classify_body(src);
        let leaf = |text: &str| ConflictLeafBlock {
            shape: BlockShape::PlainText,
            tone: Tone::Neutral,
            text: text.to_owned(),
            style: TextStyle::default(),
            reading: String::new(),
        };
        assert_eq!(blocks[1].side_a, vec![leaf("Mine text.")]);
        assert_eq!(blocks[1].side_b, vec![leaf("Theirs text.")]);
    }

    // `ConflictLeafBlock` has no `side_a`/`side_b` fields of its own (see
    // its doc for why) -- a `@conflict` nested inside another, still-
    // unresolved one's side still gets `BlockShape::Conflict`, but its own
    // sides are simply not representable one level down, by construction,
    // not by a special case in `conflict_look`.
    #[test]
    fn a_nested_conflict_renders_flat_inside_a_side() {
        let src = "@conflict(a: [@conflict(a: [Deep A.], b: [Deep B.])], b: [Theirs text.])\n";
        let blocks = classify_body(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Conflict);
        assert_eq!(blocks[0].side_a.len(), 1);
        assert_eq!(blocks[0].side_a[0].shape, BlockShape::Conflict);
    }

    #[test]
    fn an_unknown_namespaced_element_gets_a_labeled_fallback() {
        let blocks = classify_body("@deck.bookmark(label: \"Chapter 1\")\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Badge);
        assert_eq!(blocks[0].tone, Tone::Neutral);
        assert_eq!(blocks[0].text, "deck.bookmark: label");
    }

    // `immermemo-tomet-render` now identifies bare built-in elements too
    // (not just `@use`'d ones -- see its module doc), so the registry
    // mechanism above can already be extended to give one of them a
    // custom look by adding a `REGISTRY` row. Nothing has opted in yet,
    // so an unregistered bare built-in (a list) must still show exactly
    // as written, not some generic badge -- this is the no-regression
    // guarantee that extension relies on.
    #[test]
    fn an_unregistered_bare_builtin_element_shows_exactly_as_written() {
        let src = "- one\n- two\n";
        let blocks = classify_body(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::PlainText);
        assert_eq!(blocks[0].tone, Tone::Neutral);
        assert_eq!(blocks[0].text, src);
    }

    #[test]
    fn empty_body_has_no_blocks() {
        assert!(classify_body("").is_empty());
    }

    // `immermemo_merge::merge_one` always recurses into a
    // Paragraph/Paragraph/Paragraph triple (`merge_inline_seq` never
    // fails), so two sides editing the *same* paragraph differently --
    // the common conflict -- narrows to an *inline* `@conflict`. Only a
    // structural mismatch (here: local turns the block into a different
    // kind of node, which `merge_one` can't narrow across) produces a
    // block-level one instead. This test proves the real
    // `immermemo_merge::merge` -> `tomet::parser::parse_document` ->
    // `classify` pipeline actually reaches `BlockShape::Conflict` for
    // that case, not just a hand-written `@conflict(...)` fixture.
    #[test]
    fn an_inline_merge_conflict_is_recognized_through_the_real_pipeline() {
        let base = tomet::parser::parse_document("The quick fox jumps.\n").unwrap();
        let local = tomet::parser::parse_document("The slow fox jumps.\n").unwrap();
        let remote = tomet::parser::parse_document("The lazy fox jumps.\n").unwrap();
        let merged = immermemo_merge::merge(&base, &local, &remote).document;
        let text = tomet::printer::document_to_tm(&merged);

        let blocks = classify_body(&text);
        assert_eq!(
            blocks.iter().map(|b| b.shape).collect::<Vec<_>>(),
            vec![
                BlockShape::PlainText, // "The "
                BlockShape::Conflict,
                BlockShape::PlainText, // " fox jumps."
            ]
        );
        let conflict = blocks.into_iter().nth(1).unwrap();
        assert_eq!(
            conflict.side_a.iter().map(|b| &b.text).collect::<Vec<_>>(),
            vec!["slow"]
        );
        assert_eq!(
            conflict.side_b.iter().map(|b| &b.text).collect::<Vec<_>>(),
            vec!["lazy"]
        );
    }

    #[test]
    fn a_structural_merge_conflict_is_recognized_through_the_real_pipeline() {
        let base = tomet::parser::parse_document("Original.\n").unwrap();
        let local = tomet::parser::parse_document("@meta{ x: 1 }\n").unwrap();
        let remote = tomet::parser::parse_document("Theirs.\n").unwrap();
        let merged = immermemo_merge::merge(&base, &local, &remote).document;
        let text = tomet::printer::document_to_tm(&merged);

        let blocks = classify_body(&text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Conflict);
        assert_eq!(blocks[0].side_a.len(), 1);
        assert_eq!(blocks[0].side_b.len(), 1);
        assert_eq!(blocks[0].side_b[0].text, "Theirs.");
    }

    #[test]
    fn a_whole_paragraph_wrapped_in_strong_is_plain_text_tagged_bold() {
        // Nothing else shares this line, so it's its own `Block::Element`
        // -- the rarer path, still handled (see `classify_block`'s own
        // test coverage in `immermemo-tomet-render`).
        let blocks = classify_body("@strong[whole line]\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::PlainText);
        assert_eq!(blocks[0].text, "whole line");
        assert!(blocks[0].style.bold);
    }

    #[test]
    fn a_ruby_block_becomes_its_own_shape_with_base_and_reading() {
        let blocks = classify_body("@ruby[漢字](rt:\"かんじ\")\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Ruby);
        assert_eq!(blocks[0].text, "漢字");
        assert_eq!(blocks[0].reading, "かんじ");
    }

    #[test]
    fn a_link_block_with_display_content_shows_the_display_text() {
        let blocks = classify_body("@link(\"https://example.com\")[see here]\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Link);
        assert_eq!(blocks[0].tone, Tone::Accent);
        assert_eq!(blocks[0].text, "see here");
    }

    #[test]
    fn a_link_block_with_no_display_content_falls_back_to_its_target() {
        let blocks = classify_body("@link(target:\"ref:note\")\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Link);
        assert_eq!(blocks[0].text, "ref:note");
    }

    #[test]
    fn a_doc_icon_block_with_pkg_carries_name_and_pkg() {
        let blocks = classify_body("@doc.icon(\"star\", pkg: \"tabler\")\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Icon);
        assert_eq!(blocks[0].text, "star");
        assert_eq!(blocks[0].reading, "tabler");
    }

    #[test]
    fn a_doc_icon_block_without_pkg_defaults_to_tabler() {
        let blocks = classify_body("@doc.icon(\"star\")\n");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].shape, BlockShape::Icon);
        assert_eq!(blocks[0].text, "star");
        assert_eq!(blocks[0].reading, "tabler");
    }
}
