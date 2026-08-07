//! Discriminator → slice-match translation.
//!
//! A `value`/`pattern` discriminator at path `P` becomes a partial-match
//! pattern built from the `fixed[x]`/`pattern[x]` constant found at `P`
//! inside the slice's subtree (`$this` meaning the item root). Discriminator
//! types we cannot evaluate yet (`type`, `profile`, `exists`, and any path
//! containing `resolve()` or `extension(...)`) produce a slice with **no
//! match and no minimum**: it can never produce false cardinality errors,
//! its constraints simply stay dormant until the matcher lands (Phase 7),
//! and the generator surfaces a warning.
//!
//! Under `ordered: true` each slice also gets its declaration ordinal as
//! `order` — FHIR's ordered slicing means "matched items appear in the order
//! the slices are declared", and the engine's check needs a number to compare.
//! Emitted only when the slicing is ordered, matching upstream's generated
//! schemas (an unordered slicing carries no `order`).

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
    let is_ordered = ordered == Some(true);
    let mut out: IndexMap<String, Slice> = IndexMap::new();
    for (position, (name, slice_node)) in slices.into_iter().enumerate() {
        let SliceNode {
            node,
            min,
            max,
            extension_profile: _,
        } = slice_node;

        let match_ = build_match(&node, discriminators);
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
                // Ordinals are relative — only their sequence is compared —
                // so gaps from lifted extension-sugar slices are harmless.
                order: is_ordered.then_some(position as u64),
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

/// Build a pattern match from `value`/`pattern` discriminators, reading the
/// constant at each discriminator path out of the slice subtree.
fn build_match(node: &super::tree::Node, discriminators: &[EdDiscriminator]) -> Option<Match> {
    if discriminators.is_empty() {
        return None;
    }

    let mut this_constant: Option<Value> = None;
    let mut pattern = Map::new();

    for disc in discriminators {
        if !matches!(disc.type_.as_str(), "value" | "pattern") {
            return None;
        }
        if disc.path.contains("resolve()") || disc.path.contains("extension(") {
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

/// The fixed/pattern constant at `path` within the slice subtree
/// (`$this` → the subtree root itself).
fn constant_at(node: &super::tree::Node, path: &str) -> Option<Value> {
    let target = if path == "$this" {
        node
    } else {
        let mut current = node;
        for segment in path.split('.') {
            current = current.children.get(segment)?;
        }
        current
    };
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
