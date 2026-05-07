//! Shared JSON path utilities for profile validation.
//!
//! Profile rules use FHIR **`ElementDefinition.path`** strings (dotted, with optional `[x]` for
//! choices). Instance data is **serde JSON**: repeating FHIR elements become JSON arrays, and
//! polymorphic `…[x]` definitions become the union of concrete property names (`valueString`,
//! `valueCode`, …). These helpers unify traversal for:
//!
//! - [`crate::profile::cardinality`] (counts along relative paths),
//! - [`crate::profile::slicing`] (discriminator paths under repeated parents),
//! - [`crate::profile::element_bounds`] (scoped bound checks with human-readable `instance_path`s).
//!
//! # Choice elements (`[x]`)
//!
//! When a path segment ends with `[x]`, all object keys that **start with** the same prefix before
//! `[x]` are considered matches (e.g. path `value[x]` matches JSON keys `valueString`, `valueBoolean`).
//!
//! # Arrays
//!
//! Arrays are flattened: each index is visited and contributes separately to cardinality and to
//! slicing classification.

use serde_json::Value;

/// Parse `ElementDefinition.max` for slice rules: `*` → unbounded ([`None`]), else parse as `usize`.
pub(crate) fn parse_slice_max(max: &str) -> Option<usize> {
    if max == "*" {
        return None;
    }
    max.parse::<usize>().ok()
}

/// Resolve all JSON values at a relative dotted path from the given root.
///
/// Arrays are traversed transparently so repeated elements yield multiple values.
pub(crate) fn get_values_at_relative_path<'a>(
    root: &'a Value,
    relative_path: &str,
) -> Vec<&'a Value> {
    if relative_path.is_empty() {
        return vec![root];
    }

    let segments: Vec<&str> = relative_path.split('.').collect();
    collect_values_at_segments(root, &segments)
}

/// Resolve all JSON values at a relative dotted path, together with a readable
/// instance path suitable for reporting validation issues.
pub(crate) fn get_values_with_paths_at_relative_path<'a>(
    root: &'a Value,
    resource_type: &str,
    relative_path: &str,
) -> Vec<(&'a Value, String)> {
    if relative_path.is_empty() {
        return vec![(root, resource_type.to_string())];
    }

    let segments: Vec<&str> = relative_path.split('.').collect();
    collect_values_with_paths_at_segments(root, &segments, resource_type.to_string())
}

/// Recursive implementation for collecting all values that match a path split
/// into segments.
fn collect_values_at_segments<'a>(current: &'a Value, segments: &[&str]) -> Vec<&'a Value> {
    if segments.is_empty() {
        return match current {
            Value::Array(items) => items.iter().collect(),
            _ => vec![current],
        };
    }

    match current {
        Value::Object(map) => {
            let segment = segments[0];
            if let Some(choice_prefix) = choice_segment_prefix(segment) {
                map.iter()
                    .filter(|(key, _)| key.starts_with(choice_prefix))
                    .flat_map(|(_, next)| collect_values_at_segments(next, &segments[1..]))
                    .collect()
            } else {
                map.get(segment)
                    .map(|next| collect_values_at_segments(next, &segments[1..]))
                    .unwrap_or_default()
            }
        }
        Value::Array(items) => items
            .iter()
            .flat_map(|item| collect_values_at_segments(item, segments))
            .collect(),
        _ => Vec::new(),
    }
}

/// Recursive implementation for collecting values plus instance paths from a
/// path split into segments.
fn collect_values_with_paths_at_segments<'a>(
    current: &'a Value,
    segments: &[&str],
    current_path: String,
) -> Vec<(&'a Value, String)> {
    if segments.is_empty() {
        return match current {
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(idx, item)| (item, format!("{}[{}]", current_path, idx)))
                .collect(),
            _ => vec![(current, current_path)],
        };
    }

    match current {
        Value::Object(map) => {
            let segment = segments[0];
            if let Some(choice_prefix) = choice_segment_prefix(segment) {
                map.iter()
                    .filter(|(key, _)| key.starts_with(choice_prefix))
                    .flat_map(|(key, next)| {
                        collect_values_with_paths_at_segments(
                            next,
                            &segments[1..],
                            format!("{}.{}", current_path, key),
                        )
                    })
                    .collect()
            } else {
                map.get(segment)
                    .map(|next| {
                        collect_values_with_paths_at_segments(
                            next,
                            &segments[1..],
                            format!("{}.{}", current_path, segment),
                        )
                    })
                    .unwrap_or_default()
            }
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .flat_map(|(idx, item)| {
                collect_values_with_paths_at_segments(
                    item,
                    segments,
                    format!("{}[{}]", current_path, idx),
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn choice_segment_prefix(segment: &str) -> Option<&str> {
    segment.strip_suffix("[x]")
}

/// Infer **FHIR type code candidates** from a runtime JSON value for **type** slicing.
///
/// JSON is ambiguous: a string could be `string`, `code`, `uri`, etc.; a number may satisfy
/// `integer`, `decimal`, … This returns **all plausible** primitive type names that might
/// match `ElementDefinition.type.code` for discriminator matching in
/// [`crate::profile::slicing`]. Objects add `resourceType` when present and heuristics for
/// `Reference` / `CodeableReference` shapes.
pub(crate) fn json_type_codes(value: &Value) -> Vec<String> {
    match value {
        Value::String(_) => vec![
            "string".to_string(),
            "uri".to_string(),
            "url".to_string(),
            "canonical".to_string(),
            "code".to_string(),
            "markdown".to_string(),
            "id".to_string(),
            "oid".to_string(),
            "uuid".to_string(),
        ],
        Value::Bool(_) => vec!["boolean".to_string()],
        Value::Number(n) => {
            let mut out = Vec::new();
            if n.is_i64() {
                out.push("integer".to_string());
                out.push("decimal".to_string());
                if n.as_i64().is_some_and(|v| v >= 0) {
                    out.push("unsignedInt".to_string());
                }
                if n.as_i64().is_some_and(|v| v > 0) {
                    out.push("positiveInt".to_string());
                }
            } else if n.is_u64() {
                out.push("integer".to_string());
                out.push("decimal".to_string());
                out.push("unsignedInt".to_string());
                if n.as_u64().is_some_and(|v| v > 0) {
                    out.push("positiveInt".to_string());
                }
            } else {
                out.push("decimal".to_string());
            }
            out
        }
        Value::Object(map) => {
            let mut out = Vec::new();

            if let Some(resource_type) = map.get("resourceType").and_then(Value::as_str) {
                out.push(resource_type.to_string());
            }

            let has_reference_shape = map.contains_key("reference")
                || map.contains_key("identifier")
                || map.contains_key("display")
                || map.contains_key("type");

            let has_codeable_reference_shape =
                map.contains_key("concept") || map.contains_key("reference");

            if has_reference_shape {
                out.push("Reference".to_string());
            }

            if has_codeable_reference_shape {
                out.push("CodeableReference".to_string());
            }

            out
        }
        Value::Array(_) | Value::Null => Vec::new(),
    }
}
