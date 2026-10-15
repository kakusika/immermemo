//! Classifies a note's top-level blocks into plain text and recognized
//! elements, so the editor can hide `@element(...)` syntax behind a
//! block-like presentation without parsing anything itself.
//!
//! This crate only answers "what is this span of the document" -- it never
//! renders anything. The actual cards/chips are Slint components living in
//! `immermemo`, the same split `crates/vault`/`crates/merge` already draw
//! between UI-framework-agnostic domain logic and the UI that uses it: a
//! crate with no Slint dependency can be unit-tested against `.tmt`
//! fixtures directly, which a crate wired into a widget tree cannot.

mod block;
mod inline;
mod meta;
mod types;

pub use block::{classify, classify_blocks};
pub use meta::{
    extract_note_icon, extract_note_kind, extract_note_metadata, is_metadata_element,
    is_metadata_render_item, meta_element,
};
pub use types::{
    ElementArgs, ElementIdentity, ParseFailed, RenderBlockKind, RenderItem, TextStyle,
};

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
    fn meta_element_reads_an_element_valued_key() {
        let src = "@meta{ icon: @doc.icon(\"star\", pkg: \"tabler\") }\n\nBody.\n";
        let (identity, args) = meta_element(src, "icon").expect("icon is a meta entry");
        assert_eq!(identity.namespace.as_deref(), Some("doc"));
        assert_eq!(identity.name, "icon");
        assert_eq!(
            args,
            ElementArgs::Named(vec![
                (String::new(), "star".to_owned()),
                ("pkg".to_owned(), "tabler".to_owned()),
            ])
        );
    }

    #[test]
    fn meta_element_is_none_for_a_missing_key_a_non_element_value_or_no_meta_at_all() {
        assert!(meta_element("@meta{ title: \"x\" }\n", "icon").is_none());
        assert!(meta_element("@meta{}\n", "icon").is_none());
        assert!(meta_element("Just a paragraph.\n", "icon").is_none());
    }

    #[test]
    fn extract_note_kind_reads_various_syntax_shapes() {
        assert_eq!(
            extract_note_kind("@kind(note)\n\nBody text.\n"),
            Some("note".to_string())
        );
        assert_eq!(
            extract_note_kind("@kind(\"meeting\")\n\nBody text.\n"),
            Some("meeting".to_string())
        );
        assert_eq!(
            extract_note_kind("@kind(type: \"journal\")\n\nBody text.\n"),
            Some("journal".to_string())
        );
        assert_eq!(
            extract_note_kind("@doc.kind(reference)\n\nBody text.\n"),
            Some("reference".to_string())
        );
        assert_eq!(extract_note_kind("Just plain text with no kind.\n"), None);
    }

    #[test]
    fn extract_note_metadata_reads_meta_and_config_entries() {
        let src = "@config(export: { type: commonmark })\n\
                   @meta{\n  id: doc-123,\n  lang: en,\n  tags: [rust, notes]\n}\n\n\
                   Body content.\n";
        let meta = extract_note_metadata(src);
        assert_eq!(
            meta,
            vec![
                ("config.export".to_string(), "type: commonmark".to_string()),
                ("id".to_string(), "doc-123".to_string()),
                ("lang".to_string(), "en".to_string()),
                ("tags".to_string(), "rust, notes".to_string()),
            ]
        );
    }

    #[test]
    fn metadata_elements_are_filtered_from_render_items() {
        let src = "@kind(note)\n\
                   @meta{ id: 123 }\n\
                   @use(doc)\n\n\
                   Visible body text.\n";
        let items = classify(src);
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Text(span, _) => {
                assert_eq!(
                    &src[span.start.offset..span.end.offset],
                    "Visible body text."
                );
            }
            _ => panic!("expected a Text item"),
        }

        let blocks = classify_blocks(src).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, RenderBlockKind::Paragraph);
    }

    #[test]
    fn a_bare_builtin_element_is_identified_with_no_namespace() {
        // `@task(x)[Todo]` is a bare (un-namespaced) built-in element.
        // It's reported as an `Element` with namespace None.
        let items = classify("@task(x)[Todo]\n");
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Element { identity, args, .. } => {
                assert_eq!(identity.namespace, None);
                assert_eq!(identity.name, "task");
                assert_eq!(*args, ElementArgs::Positional("x".to_string()));
            }
            _ => panic!("expected an Element item"),
        }

        // Combine list notation `-@task(x) Todo` is also classified as `@task`.
        let items = classify("-@task(x) Todo\n");
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Element { identity, args, .. } => {
                assert_eq!(identity.namespace, None);
                assert_eq!(identity.name, "task");
                assert_eq!(*args, ElementArgs::Positional("x".to_string()));
            }
            _ => panic!("expected an Element item"),
        }
    }

    #[test]
    fn repeated_namespaced_elements_are_each_identified() {
        // Generic: three occurrences of the same namespaced identity are
        // each found and split correctly, not just the first one. Any
        // namespaced name would do here -- this isn't about `@conflict`
        // (which is bare `std` now, not namespaced; see the `conflict`-
        // specific tests below).
        let src = "Before.\n\n\
                   @deck.marker(x)\n\n\
                   X text.\n\n\
                   @deck.marker(y)\n\n\
                   Y text.\n\n\
                   @deck.marker(z)\n\n\
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
            assert_eq!(identity.namespace.as_deref(), Some("deck"));
            assert_eq!(identity.name, "marker");
        }

        let args_summaries: Vec<&str> = items
            .iter()
            .filter_map(|item| match item {
                RenderItem::Element { args_summary, .. } => Some(args_summary.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(args_summaries, vec!["x", "y", "z"]);
    }

    #[test]
    fn a_bare_conflict_element_is_identified_with_both_sides_classified() {
        // `@conflict` is `tomet`'s own bare `std` element (not namespaced),
        // and its `a`/`b` live in `(args)` as `Value::Blocks` rather than
        // in `[content]` -- `block_args` is what exposes them, recursively
        // classified the same way top-level blocks are.
        let items = classify("@conflict(a: [Mine text.], b: [Their text.])\n");
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Element {
                identity,
                content,
                block_args,
                ..
            } => {
                assert_eq!(identity.namespace, None);
                assert_eq!(identity.name, "conflict");
                assert_eq!(
                    *content, None,
                    "conflict's payload is in (args), not [content]"
                );
                assert_eq!(block_args.len(), 2);
                assert_eq!(block_args[0].0, "a");
                assert_eq!(block_args[1].0, "b");
                assert_eq!(block_args[0].1.len(), 1);
                assert_eq!(block_args[1].1.len(), 1);
            }
            other => panic!("expected an Element item, got {other:?}"),
        }
    }

    #[test]
    fn an_inline_conflict_splits_the_paragraph_around_it() {
        // The common conflict shape: both sides edited the same sentence,
        // which `tomet::merge` narrows to one inline
        // `@conflict` (never a whole-block marker) sitting where the
        // disputed word did.
        let src = "Before @conflict(a: [mine text], b: [their text]) after.\n";
        let items = classify(src);

        let kinds: Vec<&str> = items
            .iter()
            .map(|item| match item {
                RenderItem::Text(_, _) => "text",
                RenderItem::Element { .. } => "element",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "element", "text"]);

        let RenderItem::Text(first, _) = &items[0] else {
            unreachable!()
        };
        assert_eq!(&src[first.start.offset..first.end.offset], "Before ");

        let RenderItem::Text(last, _) = items.last().unwrap() else {
            unreachable!()
        };
        assert_eq!(&src[last.start.offset..last.end.offset], " after.");

        let RenderItem::Element { block_args, .. } = &items[1] else {
            unreachable!()
        };
        assert_eq!(block_args.len(), 2);
        assert_eq!(block_args[0].0, "a");
        assert_eq!(block_args[1].0, "b");
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
        let src = "Fine paragraph.\n\n@conflict(\n";
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
        // Same fixture `immermemo`'s render/classify.rs proves goes through
        // the real merge pipeline -- here just checking `classify_blocks`
        // keeps all of a paragraph's split items in one group, unlike
        // `classify`'s flat list. `@conflict` needs no `@use` (it's bare
        // `std`), so the whole document is one `Block::Paragraph`.
        let src = "The @conflict(a: [slow], b: [lazy]) fox jumps.\n";
        let groups = classify_blocks(src).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, RenderBlockKind::Paragraph);
        // "The ", the @conflict element, " fox jumps."
        assert_eq!(groups[0].1.len(), 3);
    }

    #[test]
    fn classify_blocks_reports_parse_failure_instead_of_guessing_a_kind() {
        let src = "Fine paragraph.\n\n@conflict(\n";
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
    fn a_bare_ruby_element_splits_out_as_an_atomic_bare_element() {
        let src = "plain @ruby[漢字](rt:\"かんじ\") plain\n";
        let items = classify(src);
        assert_eq!(items.len(), 3);
        match &items[1] {
            RenderItem::Element {
                identity,
                content,
                args,
                ..
            } => {
                assert_eq!(identity.namespace, None);
                assert_eq!(identity.name, "ruby");
                let content = content.expect("expected a content span");
                assert_eq!(&src[content.start.offset..content.end.offset], "漢字");
                assert_eq!(
                    args,
                    &ElementArgs::Named(vec![("rt".to_owned(), "かんじ".to_owned())])
                );
            }
            other => panic!("expected an Element item, got {other:?}"),
        }
    }

    #[test]
    fn a_ruby_element_with_a_missing_reading_arg_has_no_named_args() {
        let items = classify("plain @ruby[漢字] plain\n");
        match &items[1] {
            RenderItem::Element { args, .. } => assert_eq!(args, &ElementArgs::None),
            other => panic!("expected an Element item, got {other:?}"),
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
            RenderItem::Element {
                identity, content, ..
            } => {
                assert_eq!(identity.name, "ruby");
                let content = content.expect("expected a content span");
                assert_eq!(&src[content.start.offset..content.end.offset], "漢字");
            }
            other => panic!("expected an Element item, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_link_with_a_positional_target_splits_out_inline() {
        let src = "plain @link(\"https://example.com\")[see here] plain\n";
        let items = classify(src);
        assert_eq!(items.len(), 3);
        match &items[1] {
            RenderItem::Element {
                identity,
                content,
                args,
                ..
            } => {
                assert_eq!(identity.namespace, None);
                assert_eq!(identity.name, "link");
                assert_eq!(
                    args,
                    &ElementArgs::Positional("https://example.com".to_owned())
                );
                let content = content.expect("expected a content span");
                assert_eq!(&src[content.start.offset..content.end.offset], "see here");
            }
            other => panic!("expected an Element item, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_link_with_a_named_target_arg_is_read_correctly() {
        let items = classify("plain @link(target:\"file:a/b.tmt\")[the file] plain\n");
        match &items[1] {
            RenderItem::Element { args, .. } => assert_eq!(
                args,
                &ElementArgs::Named(vec![("target".to_owned(), "file:a/b.tmt".to_owned())])
            ),
            other => panic!("expected an Element item, got {other:?}"),
        }
    }

    #[test]
    fn a_link_with_no_display_content_has_no_content_span() {
        let items = classify("plain @link(target:\"ref:note\") plain\n");
        match &items[1] {
            RenderItem::Element { content, .. } => assert_eq!(*content, None),
            other => panic!("expected an Element item, got {other:?}"),
        }
    }

    #[test]
    fn a_link_alone_on_its_own_line_still_splits_out() {
        let items = classify("@link(\"https://example.com\")[see here]\n");
        assert_eq!(items.len(), 1);
        match &items[0] {
            RenderItem::Element { identity, .. } => assert_eq!(identity.name, "link"),
            other => panic!("expected an Element item, got {other:?}"),
        }
    }

    #[test]
    fn a_section_heading_flows_like_a_paragraph_and_its_blocks_classify_recursively() {
        let src = "= A @strong[bold] heading\n\nInside.\n\n@conflict(a: 1, b: 2)\n\nMine.\n";
        let groups = classify_blocks(src).unwrap();

        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0].0, RenderBlockKind::Heading(1));
        // "A ", bold "bold", " heading" -- same split classify_inline_seq
        // gives any other prose containing a style wrapper.
        assert_eq!(groups[0].1.len(), 3);
        assert_eq!(groups[1].0, RenderBlockKind::Paragraph);
        assert_eq!(groups[2].0, RenderBlockKind::Element);
        assert_eq!(groups[3].0, RenderBlockKind::Paragraph);
    }

    #[test]
    fn nested_sub_sections_classify_recursively_too() {
        let src = "= Outer\n\n== Inner\n\nInside the inner section.\n";
        let groups = classify_blocks(src).unwrap();
        assert_eq!(
            groups.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            vec![
                RenderBlockKind::Heading(1),
                RenderBlockKind::Heading(2),
                RenderBlockKind::Paragraph,
            ]
        );
    }
}
