//! Writes a note's `@meta{ icon: @doc.icon(name, pkg) }` -- the save-side
//! counterpart of `immermemo_tomet_render::meta_element`/`note_icon`
//! (`immermemo`'s `render::classify`), which only read it. Parses, mutates
//! the AST (inserting a `@meta{}` block if the note had none, or
//! replacing/adding its `icon` entry if it did), then re-prints the whole
//! document -- the same round-trip `crates/merge`'s `markers` module uses
//! to turn a constructed node back into source text.

use tomet_ast::{Block, Document, Element, ElementValue, Entry, Name, Placement, Sigil, Value};

/// `body` with its top-level `@meta{}`'s `icon` entry set to
/// `@doc.icon(name, pkg)`, constructing a `@meta{}` block first if `body`
/// had none. `None` only if `body` itself doesn't parse (a saved note's
/// own body always should; this isn't a format this function can produce
/// unparseable input for) -- same "shouldn't happen, but don't panic on a
/// caller's unchecked assumption" scope as the rest of this crate.
pub fn set_note_icon(body: &str, name: &str, pkg: &str) -> Option<String> {
    let mut doc: Document = tomet_parser::parse_document(body).ok()?;

    let icon_value = Value::Element(Box::new(Element {
        sigil: Sigil::Named(Name::namespaced("doc", "icon")),
        args: Some(Value::Map(vec![
            (String::new(), Value::String(name.to_owned())),
            ("pkg".to_owned(), Value::String(pkg.to_owned())),
        ])),
        ..Default::default()
    }));
    let icon_entry = Entry::Pair("icon".to_owned(), icon_value);

    let meta_index = doc
        .blocks
        .iter()
        .position(|block| matches!(block, Block::Element(el) if el.sigil.is_bare_named("meta")));

    match meta_index {
        Some(i) => {
            let Block::Element(meta) = &mut doc.blocks[i] else {
                unreachable!("meta_index only matches Block::Element");
            };
            let Some(ElementValue::Group(entries)) = &mut meta.value else {
                // `@meta` with a `+++` fence or `${...}` interp body
                // instead of a plain `{...}` group -- not a shape this
                // function knows how to add an entry to; leave it alone
                // rather than clobber whatever the note actually has.
                return None;
            };
            match entries
                .iter_mut()
                .find(|entry| matches!(entry, Entry::Pair(key, _) if key == "icon"))
            {
                Some(existing) => *existing = icon_entry,
                None => entries.push(icon_entry),
            }
        }
        None => {
            doc.blocks.insert(
                0,
                Block::Element(Element {
                    sigil: Sigil::named("meta"),
                    placement: Placement::Block,
                    value: Some(ElementValue::Group(vec![icon_entry])),
                    ..Default::default()
                }),
            );
        }
    }

    Some(tomet_printer::document_to_tm(&doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_a_meta_block_when_the_note_had_none() {
        let result = set_note_icon("Just a note.\n", "star", "tabler").unwrap();
        assert_eq!(
            result,
            "@meta{icon: @doc.icon(star, pkg: tabler)}\n\nJust a note.\n"
        );
    }

    #[test]
    fn adds_icon_to_an_existing_meta_block_that_has_other_keys() {
        let result = set_note_icon("@meta{ x: 1 }\n\nBody.\n", "star", "tabler").unwrap();
        assert_eq!(
            result,
            "@meta{x: 1, icon: @doc.icon(star, pkg: tabler)}\n\nBody.\n"
        );
    }

    #[test]
    fn replaces_an_existing_icon_rather_than_duplicating_it() {
        let src = "@meta{ icon: @doc.icon(\"flag\") }\n\nBody.\n";
        let result = set_note_icon(src, "star", "tabler").unwrap();
        assert_eq!(
            result,
            "@meta{icon: @doc.icon(star, pkg: tabler)}\n\nBody.\n"
        );
    }

    /// `set_note_icon`'s own output round-trips back through
    /// `immermemo_tomet_render::meta_element` (the read side) to the same
    /// `(name, pkg)` -- the thing that actually matters, more than the
    /// exact printed spelling the other tests above pin down.
    #[test]
    fn written_icon_reads_back_correctly() {
        let result = set_note_icon("Just a note.\n", "star", "tabler").unwrap();
        let (identity, args) = immermemo_tomet_render::meta_element(&result, "icon").unwrap();
        assert_eq!(identity.namespace.as_deref(), Some("doc"));
        assert_eq!(identity.name, "icon");
        assert_eq!(
            args,
            immermemo_tomet_render::ElementArgs::Named(vec![
                (String::new(), "star".to_owned()),
                ("pkg".to_owned(), "tabler".to_owned()),
            ])
        );
    }
}
