//! Note metadata and directives extraction (`@meta`, `@kind`, `@config`, `@use`).

use tomet::ast::{Block, Element, ElementView, File, Inline, Tuple, Value};
use tomet_scalar::Scalar;

use crate::inline::inline_content;
use crate::types::{ElementArgs, ElementIdentity, RenderItem};

/// Every `key: value` entry of `src`'s top-level `::meta(...)` or `@meta{ ... }`
/// group, in source order. Empty if there is no such group.
pub(crate) fn meta_pairs(src: &str) -> Vec<(String, Value)> {
    let Ok(doc) = tomet::parse_file(src) else {
        return Vec::new();
    };
    if let Some(decl) = doc
        .header
        .iter()
        .find(|d| d.name.namespace.is_none() && d.name.name == "meta")
    {
        if let Some(args) = &decl.data.args {
            return args
                .entries()
                .filter_map(|e| e.name().map(|k| (k.to_owned(), e.value().clone())))
                .collect();
        }
        if let Some(group) = &decl.data.value {
            return group.pairs().map(|(k, v)| (k.clone(), v.clone())).collect();
        }
    }
    let meta = doc.body.iter().find_map(|block| match block {
        Block::Element(el) => (el.name.namespace.is_none() && el.name.name == "meta").then_some(el),
        _ => None,
    });
    if let Some(el) = meta {
        if let Some(group) = &el.data.value {
            return group.pairs().map(|(k, v)| (k.clone(), v.clone())).collect();
        }
        if let Some(args) = &el.data.args {
            return args
                .entries()
                .filter_map(|e| e.name().map(|k| (k.to_owned(), e.value().clone())))
                .collect();
        }
    }
    Vec::new()
}

/// `src`'s top-level `@meta{ ... }` group's `key` entry value, whatever its
/// shape (a string, an element, ...), if present.
pub(crate) fn meta_value(src: &str, key: &str) -> Option<Value> {
    meta_pairs(src)
        .into_iter()
        .find_map(|(entry_key, value)| (entry_key == key).then_some(value))
}

/// Reads `src`'s top-level `@meta{ ... }` group's `key` entry, if present
/// *and* its value is itself an element (e.g. `@meta{ icon: @doc.icon(...) }`).
pub fn meta_element(src: &str, key: &str) -> Option<(ElementIdentity, ElementArgs)> {
    let Value::Element(el) = meta_value(src, key)? else {
        return None;
    };
    element_identity_and_args(&el)
}

/// A named element's identity and string args.
pub(crate) fn element_identity_and_args(el: &Element) -> Option<(ElementIdentity, ElementArgs)> {
    Some((
        ElementIdentity {
            namespace: el.name.namespace.clone(),
            name: el.name.name.clone(),
        },
        crate::inline::element_args(el.data.args.as_ref()),
    ))
}

/// Returns true if an element identity represents document-level metadata
/// or directives (`@meta`, `@kind`, `@config`, `@use`) that belong in note
/// properties rather than the rendered reading view.
pub fn is_metadata_element(identity: &ElementIdentity) -> bool {
    let is_ns = matches!(
        identity.namespace.as_deref(),
        None | Some("std") | Some("doc")
    );
    is_ns && matches!(identity.name.as_str(), "meta" | "kind" | "config" | "use")
}

/// Returns true if a classified `RenderItem` is a metadata element.
pub fn is_metadata_render_item(item: &RenderItem) -> bool {
    match item {
        RenderItem::Element(el) => is_metadata_element(&el.identity),
        _ => false,
    }
}

/// Helper for AST elements before full RenderItem conversion.
pub(crate) fn is_metadata_element_ast(el: ElementView<'_>) -> bool {
    let is_ns = matches!(
        el.name.namespace.as_deref(),
        None | Some("std") | Some("doc")
    );
    is_ns && matches!(el.name.name.as_str(), "meta" | "kind" | "config" | "use")
}

fn format_tuple(tuple: &Tuple) -> String {
    tuple
        .entries()
        .map(|entry| match entry.name() {
            Some(k) => format!("{k}: {}", format_meta_value(entry.value())),
            None => format_meta_value(entry.value()),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Formats an AST [`Value`] into a human-readable display string for metadata view.
pub(crate) fn format_meta_value(value: &Value) -> String {
    match value {
        Value::Scalar(Scalar::None) => "none".to_string(),
        Value::Scalar(Scalar::Auto) => "auto".to_string(),
        Value::Scalar(Scalar::Bool(b)) => b.to_string(),
        Value::Scalar(Scalar::Int(i)) => i.to_string(),
        Value::Scalar(Scalar::Float(f)) => f.to_string(),
        Value::Scalar(Scalar::Number(s) | Scalar::Text(s)) => s.clone(),
        Value::Call(name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                format!("{name}({})", format_tuple(args))
            }
        }
        Value::Tuple(tuple) => format_tuple(tuple),
        Value::Interp(_) => "interp".to_string(),
        Value::Hole(_) => "hole".to_string(),
        Value::TypeSeat(_) => "type".to_string(),
        Value::Element(el) => {
            if let Some(args) = &el.data.args {
                format!("@{}({})", el.name, format_tuple(args))
            } else {
                format!("@{}", el.name)
            }
        }
        Value::Blocks(blocks) => {
            let mut parts = Vec::new();
            for block in blocks {
                if let Block::Paragraph(p) = block {
                    let mut p_text = String::new();
                    for inline in &p.content {
                        if let Inline::Text(t) = inline {
                            p_text.push_str(&t.value);
                        }
                    }
                    if !p_text.trim().is_empty() {
                        parts.push(p_text.trim().to_string());
                    }
                }
            }
            if parts.is_empty() {
                format!("[{} block(s)]", blocks.len())
            } else {
                parts.join(", ")
            }
        }
    }
}

fn kind_from_args(args: &Tuple) -> Option<String> {
    if args.len() == 1 {
        if let Some(entry) = args.entries().next() {
            if entry.name().is_none() {
                match entry.value() {
                    Value::Scalar(Scalar::Text(s)) => return Some(s.clone()),
                    Value::Call(ident, _) => return Some(ident.clone()),
                    _ => {}
                }
            }
        }
    }
    for entry in args.entries() {
        let k = entry.name().unwrap_or("");
        if k.is_empty() || k == "type" || k == "name" || k == "kind" {
            match entry.value() {
                Value::Scalar(Scalar::Text(s)) => return Some(s.clone()),
                Value::Call(ident, _) => return Some(ident.clone()),
                _ => {}
            }
        }
    }
    None
}

/// Extracts the note's kind from `::kind(...)`, `@kind(...)`, `@doc.kind(...)`, or `@std.kind(...)`
/// if present at document level.
pub fn extract_note_kind(src: &str) -> Option<String> {
    let doc: File = tomet::parse_file(src).ok()?;
    for decl in &doc.header {
        if decl.name.name == "kind"
            && matches!(
                decl.name.namespace.as_deref(),
                None | Some("std") | Some("doc")
            )
        {
            if let Some(args) = &decl.data.args {
                if let Some(k) = kind_from_args(args) {
                    return Some(k);
                }
            }
        }
    }
    for block in &doc.body {
        if let Block::Element(el) = block {
            if el.name.name == "kind"
                && matches!(
                    el.name.namespace.as_deref(),
                    None | Some("std") | Some("doc")
                )
            {
                if let Some(args) = &el.data.args {
                    if let Some(k) = kind_from_args(args) {
                        return Some(k);
                    }
                }
                if let Some(content) = inline_content(el.as_view()) {
                    let mut s = String::new();
                    for inline in content {
                        let span = inline.span();
                        s.push_str(&src[span.start..span.end]);
                    }
                    if !s.trim().is_empty() {
                        return Some(s.trim().to_string());
                    }
                }
            }
        }
    }
    None
}

/// Extracts metadata key-value pairs from top-level `::meta(...)` / `@meta{ ... }` and
/// `::config(...)` / `@config(...)` blocks in `src`. Returns `(key, formatted_value)` pairs.
pub fn extract_note_metadata(src: &str) -> Vec<(String, String)> {
    let doc: File = match tomet::parse_file(src) {
        Ok(doc) => doc,
        Err(_) => return Vec::new(),
    };
    let mut entries = Vec::new();
    for decl in &doc.header {
        if !matches!(
            decl.name.namespace.as_deref(),
            None | Some("std") | Some("doc")
        ) {
            continue;
        }
        let prefix = match decl.name.name.as_str() {
            "meta" => None,
            "config" => Some("config."),
            _ => continue,
        };
        if let Some(args) = &decl.data.args {
            for entry in args.entries() {
                if let Some(k) = entry.name() {
                    let key = match prefix {
                        Some(p) => format!("{p}{k}"),
                        None => k.to_owned(),
                    };
                    entries.push((key, format_meta_value(entry.value())));
                }
            }
        }
        if let Some(group) = &decl.data.value {
            for (k, v) in group.pairs() {
                let key = match prefix {
                    Some(p) => format!("{p}{k}"),
                    None => k.clone(),
                };
                entries.push((key, format_meta_value(v)));
            }
        }
    }
    for block in &doc.body {
        if let Block::Element(el) = block {
            if !matches!(
                el.name.namespace.as_deref(),
                None | Some("std") | Some("doc")
            ) {
                continue;
            }
            let prefix = match el.name.name.as_str() {
                "meta" => None,
                "config" => Some("config."),
                _ => continue,
            };
            if let Some(group) = &el.data.value {
                for (k, v) in group.pairs() {
                    let key = match prefix {
                        Some(p) => format!("{p}{k}"),
                        None => k.clone(),
                    };
                    entries.push((key, format_meta_value(v)));
                }
            }
            if let Some(args) = &el.data.args {
                for entry in args.entries() {
                    if let Some(k) = entry.name() {
                        let key = match prefix {
                            Some(p) => format!("{p}{k}"),
                            None => k.to_owned(),
                        };
                        entries.push((key, format_meta_value(entry.value())));
                    }
                }
            }
        }
    }
    entries
}
