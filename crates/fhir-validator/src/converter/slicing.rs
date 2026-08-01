//! Discriminator → slice-match translation.
//!
//! A `value`/`pattern` discriminator at path `P` becomes a partial-match
//! pattern built from the `fixed[x]`/`pattern[x]` constant found at `P`
//! inside the slice's subtree (`$this` meaning the item root).
//!
//! `type`, `profile`, and `binding` discriminators become typed [`Match`]
//! values evaluated at runtime by the engine. Paths containing `resolve()`
//! or `extension(...)` remain unsupported (slice kept without match/min).

use super::EdDiscriminator;
use super::tree::{SliceNode, finalize};
use crate::schema::{Match, Slice, Slicing};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::sync::Arc;

pub(super) fn build_slicing(
    slices: IndexMap<String, SliceNode>,
    discriminators: &[EdDiscriminator],
    rules: Option<String>,
    ordered: Option<bool>,
    warnings: &mut Vec<String>,
) -> Option<Slicing> {
    let mut out: IndexMap<String, Slice> = IndexMap::new();
    for (name, slice_node) in slices {
        let SliceNode {
            node,
            min,
            max,
            extension_profile,
        } = slice_node;

        let match_ = build_match(&node, discriminators, extension_profile.as_deref());
        if match_.is_none() {
            warnings.push(format!(
                "slice '{name}': discriminator(s) {:?} not translatable to a match; \
                 slice kept without match or min",
                discriminators
                    .iter()
                    .map(|d| format!("{}@{}", d.type_, d.path))
                    .collect::<Vec<_>>()
            ));
        }

        let schema = finalize(node, warnings);
        out.insert(
            name,
            Slice {
                // Without a matcher the slice must not enforce a minimum —
                // nothing can ever match it.
                min: if match_.is_some() { min } else { None },
                max,
                match_,
                order: None,
                reslice: None,
                slice_is_constraining: None,
                schema: schema.map(Arc::new),
            },
        );
    }

    if out.is_empty() {
        return None;
    }
    Some(Slicing {
        slices: out,
        rules,
        ordered,
    })
}

/// Build a match from discriminators, reading constants / type / binding /
/// profile metadata out of the slice subtree.
fn build_match(
    node: &super::tree::Node,
    discriminators: &[EdDiscriminator],
    extension_profile: Option<&str>,
) -> Option<Match> {
    if discriminators.is_empty() {
        return None;
    }

    if discriminators
        .iter()
        .any(|d| d.path.contains("resolve()") || d.path.contains("extension("))
    {
        return None;
    }

    let kinds: Vec<&str> = discriminators.iter().map(|d| d.type_.as_str()).collect();
    let all_pattern = kinds
        .iter()
        .all(|k| matches!(*k, "value" | "pattern"));
    if all_pattern {
        return build_pattern_match(node, discriminators);
    }

    // Single non-pattern discriminator (homogeneous set of one kind).
    if kinds.iter().all(|k| *k == "type") && discriminators.len() == 1 {
        let disc = &discriminators[0];
        let target = node_at(node, &disc.path)?;
        let type_code = target.schema.type_.as_ref()?;
        return Some(Match {
            type_: Some("type".to_string()),
            value: Some(Value::String(type_code.clone())),
            resolve_ref: None,
        });
    }
    if kinds.iter().all(|k| *k == "profile") && discriminators.len() == 1 {
        let disc = &discriminators[0];
        let target = node_at(node, &disc.path)?;
        let profile = extension_profile
            .map(str::to_string)
            .or_else(|| target.type_profiles.first().cloned())
            .or_else(|| target.schema.url.clone())
            .or_else(|| {
                target
                    .schema
                    .refers
                    .as_ref()
                    .and_then(|r| r.first().cloned())
            })?;
        return Some(Match {
            type_: Some("profile".to_string()),
            value: Some(Value::String(profile)),
            resolve_ref: None,
        });
    }
    if kinds.iter().all(|k| *k == "binding") && discriminators.len() == 1 {
        let disc = &discriminators[0];
        let target = node_at(node, &disc.path)?;
        let vs = target.schema.binding.as_ref()?.value_set.clone();
        return Some(Match {
            type_: Some("binding".to_string()),
            value: Some(Value::String(vs)),
            resolve_ref: None,
        });
    }

    None
}

fn build_pattern_match(
    node: &super::tree::Node,
    discriminators: &[EdDiscriminator],
) -> Option<Match> {
    let mut this_constant: Option<Value> = None;
    let mut pattern = Map::new();

    for disc in discriminators {
        if !matches!(disc.type_.as_str(), "value" | "pattern") {
            return None;
        }
        let constant = constant_at(node, &disc.path)?;
        if disc.path == "$this" {
            if discriminators.len() > 1 {
                return None; // $this plus siblings — ambiguous
            }
            this_constant = Some(constant);
        } else {
            insert_nested(&mut pattern, &disc.path, constant);
        }
    }

    let value = match this_constant {
        Some(v) => v,
        None if !pattern.is_empty() => Value::Object(pattern),
        None => return None,
    };
    Some(Match {
        type_: Some("pattern".to_string()),
        value: Some(value),
        resolve_ref: None,
    })
}

fn node_at<'a>(node: &'a super::tree::Node, path: &str) -> Option<&'a super::tree::Node> {
    if path == "$this" {
        return Some(node);
    }
    let mut current = node;
    for segment in path.split('.') {
        current = current.children.get(segment)?;
    }
    Some(current)
}

/// The fixed/pattern constant at `path` within the slice subtree
/// (`$this` → the subtree root itself).
fn constant_at(node: &super::tree::Node, path: &str) -> Option<Value> {
    let target = node_at(node, path)?;
    target
        .schema
        .fixed
        .clone()
        .or_else(|| target.schema.pattern.clone())
}

/// `insert_nested(map, "a.b", v)` → `{a: {b: v}}` (merging siblings).
fn insert_nested(map: &mut Map<String, Value>, path: &str, value: Value) {
    let mut segments = path.split('.').peekable();
    let mut current = map;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            current.insert(segment.to_string(), value);
            return;
        }
        current = current
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("nested pattern segment is an object");
    }
}
