//! Writes a note's `@meta{ icon: @doc.icon(name, pkg) }` -- the save-side
//! counterpart of `immermemo_tomet_render::meta_element`/`note_icon`
//! (`immermemo`'s `render::classify`), which only read it.
//!
//! Uses `tomet::edit` to mutate only the `@meta{}` block while preserving 100% of
//! layout trivia, comments, and unedited blocks byte-identically.

use tomet::edit::{
    AttrGroup, ChildItem, EditDoc, EditOp, Node, NodeKind, export_doc, import_source,
};

/// `body` with its top-level `@meta{}`'s `icon` entry set to
/// `@doc.icon(name, pkg)`, constructing a `@meta{}` block first if `body`
/// had none. Preserves comments, whitespace, and layout trivia in the rest of
/// the document byte-identically.
pub fn set_note_icon(body: &str, name: &str, pkg: &str) -> Option<String> {
    let mut doc: EditDoc = import_source(body);

    let meta_node_id = doc
        .root_children()
        .iter()
        .filter_map(|item| item.as_node_id())
        .find(|id| {
            matches!(
                doc.get(*id).map(|n| &n.kind),
                Some(NodeKind::BlockElement { name, .. }) if name == "meta"
            )
        });

    let icon_val = if pkg.is_empty() {
        format!("@doc.icon({name})")
    } else {
        format!("@doc.icon({name}, pkg: {pkg})")
    };

    match meta_node_id {
        Some(id) => {
            doc.set_attribute(id, AttrGroup::Data, "icon", Some(icon_val))
                .ok()?;
        }
        None => {
            let new_id = doc.alloc_id();
            let mut new_node = Node::new(
                new_id,
                NodeKind::BlockElement {
                    name: "meta".to_string(),
                    id_attr: None,
                    args: Vec::new(),
                    data: vec![("icon".to_string(), icon_val)],
                    content: None,
                },
                String::new(),
            );
            new_node.mark_edited();
            doc.apply_op(EditOp::Insert {
                parent: None,
                index: 0,
                node: new_node,
            })
            .ok()?;

            // Insert spacing trivia between @meta and the rest of the body
            if doc.root_children().len() > 1 {
                doc.root_children_mut()
                    .insert(1, ChildItem::Trivia("\n\n".to_string()));
            } else {
                doc.root_children_mut()
                    .insert(1, ChildItem::Trivia("\n".to_string()));
            }
        }
    }

    Some(export_doc(&doc))
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

    #[test]
    fn preserves_existing_comments_and_blank_lines_outside_meta() {
        let src = "// Leading comment\n@meta{ x: 1 }\n\n\nBody paragraph with *markup*.\n\n// Trailing comment\n";
        let result = set_note_icon(src, "star", "tabler").unwrap();
        assert!(result.starts_with(
            "// Leading comment\n@meta{x: 1, icon: @doc.icon(star, pkg: tabler)}\n\n\n"
        ));
        assert!(result.contains("Body paragraph with *markup*.\n\n// Trailing comment\n"));
    }
}
