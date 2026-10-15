//! Note metadata and directives extraction (`@meta`, `@kind`, `@config`, `@use`).

use tomet::ast::{Block, Document, Element, ElementValue, Entry, Inline, Sigil, Value};

use crate::inline::inline_content;
use crate::types::{ElementArgs, ElementIdentity, RenderItem};

/// Reads `src`'s top-level `@meta{ ... }` group's `key` entry, if present
/// *and* its value is itself an element (e.g. `@meta{ icon: @doc.icon(...) }`).
pub fn meta_element(src: &str, key: &str) -> Option<(ElementIdentity, ElementArgs)> {
    let doc: Document = tomet::parser::parse_document(src).ok()?;
    let meta = doc.blocks.iter().find_map(|block| match block {
        Block::Element(el) => {
            let Sigil::Named(name) = &el.sigil else {
                return None;
            };
            (name.namespace.is_none() && name.name == "meta").then_some(el)
        }
        _ => None,
    })?;
    let ElementValue::Group(entries) = meta.value.as_ref()? else {
        return None;
    };
    entries.iter().find_map(|entry| match entry {
        Entry::Pair(entry_key, Value::Element(el)) if entry_key == key => {
            let Sigil::Named(name) = &el.sigil else {
                return None;
            };
            Some((
                ElementIdentity {
                    namespace: name.namespace.clone(),
                    name: name.name.clone(),
                },
                crate::inline::element_args(el.args.as_ref()),
            ))
        }
        _ => None,
    })
}

/// Extracts `(icon_name, icon_pkg)` from a note's `@meta{ icon: @doc.icon(...) }`, if present.
pub fn extract_note_icon(src: &str) -> Option<(String, String)> {
    let (identity, args) = meta_element(src, "icon")?;
    if identity.namespace.as_deref() == Some("doc") && identity.name == "icon" {
        let (name, pkg) = match &args {
            ElementArgs::Positional(name) => (name.clone(), "tabler".to_string()),
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
                    .unwrap_or_else(|| "tabler".to_string());
                (name, pkg)
            }
            ElementArgs::None => (String::new(), "tabler".to_string()),
        };
        Some((name, pkg))
    } else {
        None
    }
}

/// Returns true if an element identity represents document-level metadata
/// or directives (`@meta`, `@kind`, `@config`, `@use`) that belong in note
/// properties rather than the rendered reading view.
pub fn is_metadata_element(identity: &ElementIdentity) -> bool {
    let is_ns = match identity.namespace.as_deref() {
        None | Some("std") | Some("doc") => true,
        _ => false,
    };
    is_ns && matches!(identity.name.as_str(), "meta" | "kind" | "config" | "use")
}

/// Returns true if a classified `RenderItem` is a metadata element.
pub fn is_metadata_render_item(item: &RenderItem) -> bool {
    match item {
        RenderItem::Element { identity, .. } => is_metadata_element(identity),
        _ => false,
    }
}

/// Helper for AST elements before full RenderItem conversion.
pub(crate) fn is_metadata_element_ast(el: &Element) -> bool {
    let Sigil::Named(name) = &el.sigil else {
        return false;
    };
    let is_ns = match name.namespace.as_deref() {
        None | Some("std") | Some("doc") => true,
        _ => false,
    };
    is_ns && matches!(name.name.as_str(), "meta" | "kind" | "config" | "use")
}

/// Formats an AST [`Value`] into a human-readable display string for metadata view.
pub(crate) fn format_meta_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::Call(name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                let formatted_args: Vec<_> = args.iter().map(format_meta_value).collect();
                format!("{name}({})", formatted_args.join(", "))
            }
        }
        Value::Seq(items) => {
            let formatted: Vec<_> = items.iter().map(format_meta_value).collect();
            formatted.join(", ")
        }
        Value::Map(entries) => {
            let formatted: Vec<_> = entries
                .iter()
                .map(|(k, v)| format!("{k}: {}", format_meta_value(v)))
                .collect();
            formatted.join(", ")
        }
        Value::Element(el) => {
            let name = el
                .sigil
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "element".to_string());
            if let Some(args) = &el.args {
                format!("@{name}({})", format_meta_value(args))
            } else {
                format!("@{name}")
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

/// Extracts the note's kind from `@kind(...)`, `@doc.kind(...)`, or `@std.kind(...)`
/// if present at document level.
pub fn extract_note_kind(src: &str) -> Option<String> {
    let doc: Document = tomet::parser::parse_document(src).ok()?;
    for block in &doc.blocks {
        if let Block::Element(el) = block {
            let Sigil::Named(name) = &el.sigil else {
                continue;
            };
            if name.name == "kind"
                && matches!(name.namespace.as_deref(), None | Some("std") | Some("doc"))
            {
                if let Some(args) = &el.args {
                    match args {
                        Value::String(s) => return Some(s.clone()),
                        Value::Call(ident, _) => return Some(ident.clone()),
                        Value::Map(entries) => {
                            for (k, v) in entries {
                                if k.is_empty() || k == "type" || k == "name" || k == "kind" {
                                    match v {
                                        Value::String(s) => return Some(s.clone()),
                                        Value::Call(ident, _) => return Some(ident.clone()),
                                        _ => {}
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(content) = inline_content(el) {
                    let mut s = String::new();
                    for inline in content {
                        let span = inline.span();
                        s.push_str(&src[span.start.offset..span.end.offset]);
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

/// Extracts metadata key-value pairs from top-level `@meta{ ... }` and `@config(...)`
/// blocks in `src`. Returns `(key, formatted_value)` pairs.
pub fn extract_note_metadata(src: &str) -> Vec<(String, String)> {
    let doc: Document = match tomet::parser::parse_document(src) {
        Ok(doc) => doc,
        Err(_) => return Vec::new(),
    };
    let mut entries = Vec::new();
    for block in &doc.blocks {
        if let Block::Element(el) = block {
            let Sigil::Named(name) = &el.sigil else {
                continue;
            };
            if !matches!(name.namespace.as_deref(), None | Some("std") | Some("doc")) {
                continue;
            }
            if name.name == "meta" {
                if let Some(ElementValue::Group(group_entries)) = &el.value {
                    for entry in group_entries {
                        if let Entry::Pair(k, v) = entry {
                            entries.push((k.clone(), format_meta_value(v)));
                        }
                    }
                }
                if let Some(Value::Map(map_entries)) = &el.args {
                    for (k, v) in map_entries {
                        if !k.is_empty() {
                            entries.push((k.clone(), format_meta_value(v)));
                        }
                    }
                }
            } else if name.name == "config" {
                if let Some(Value::Map(map_entries)) = &el.args {
                    for (k, v) in map_entries {
                        if !k.is_empty() {
                            entries.push((format!("config.{}", k), format_meta_value(v)));
                        }
                    }
                }
                if let Some(ElementValue::Group(group_entries)) = &el.value {
                    for entry in group_entries {
                        if let Entry::Pair(k, v) = entry {
                            entries.push((format!("config.{}", k), format_meta_value(v)));
                        }
                    }
                }
            }
        }
    }
    entries
}
