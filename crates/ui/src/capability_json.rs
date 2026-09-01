//! Bounded, incremental JSON rendering for the CapabilityStatement page.
//!
//! CapabilityStatements are server-owned but can be tens of thousands of
//! lines. Rendering one as a complete highlighted DOM is substantially more
//! expensive than returning its JSON. This adapter keeps that exceptional
//! case local to the CapabilityStatement page: small subtrees still use the
//! shared JSON renderer, while large containers expose one bounded level at a
//! time through validated JSON Pointer requests.

use crate::json_view;
use helios_fhir::FhirVersion;
use serde_json::Value;

pub(crate) const DEFAULT_PAGE_SIZE: usize = 100;
pub(crate) const MAX_PAGE_SIZE: usize = 100;
pub(crate) const MAX_FRAGMENT_HTML_BYTES: usize = 1024 * 1024;

const MAX_POINTER_BYTES: usize = 1024;
const MAX_POINTER_DEPTH: usize = 32;
const FULL_MAX_LINES: usize = 1_000;
const FULL_MAX_ESTIMATED_HTML_BYTES: usize = MAX_FRAGMENT_HTML_BYTES;
const MAX_DISPLAY_KEY_CHARS: usize = 128;
const MAX_DISPLAY_STRING_CHARS: usize = 256;

pub(crate) enum View {
    Full(Vec<json_view::JsonLine>),
    Outline(Outline),
}

pub(crate) struct Outline {
    pub(crate) rows: Vec<Row>,
    pub(crate) opening: &'static str,
    pub(crate) closing: &'static str,
    pub(crate) has_previous: bool,
    pub(crate) previous_url: String,
    pub(crate) has_next: bool,
    pub(crate) next_url: String,
    pub(crate) first_item: usize,
    pub(crate) last_item: usize,
    pub(crate) total_items: usize,
}

pub(crate) struct Row {
    pub(crate) prefix: String,
    pub(crate) tokens: Vec<json_view::Token>,
    pub(crate) is_container: bool,
    pub(crate) summary: String,
    pub(crate) fragment_url: String,
    pub(crate) expandable: bool,
    pub(crate) truncated: bool,
    pub(crate) comma: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidPointer,
    InvalidPage,
    NotFound,
}

pub(crate) fn plan(
    document: &Value,
    pointer: &str,
    offset: usize,
    limit: usize,
    version: FhirVersion,
) -> Result<View, Error> {
    validate_pointer(pointer)?;
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(Error::InvalidPage);
    }

    let value = if pointer.is_empty() {
        document
    } else {
        document.pointer(pointer).ok_or(Error::NotFound)?
    };

    if offset == 0
        && container_len(value).is_none_or(|length| length <= MAX_PAGE_SIZE)
        && let Ok(lines) = json_view::try_lines(
            value,
            json_view::RenderOptions {
                include_paths: false,
                budget: Some(json_view::RenderBudget {
                    max_lines: FULL_MAX_LINES,
                    max_estimated_html_bytes: FULL_MAX_ESTIMATED_HTML_BYTES,
                }),
            },
        )
    {
        return Ok(View::Full(lines));
    }

    outline(value, pointer, offset, limit, version).map(View::Outline)
}

fn outline(
    value: &Value,
    pointer: &str,
    offset: usize,
    limit: usize,
    version: FhirVersion,
) -> Result<Outline, Error> {
    let total_items = container_len(value).ok_or(Error::InvalidPointer)?;
    if offset > total_items {
        return Err(Error::InvalidPage);
    }
    let end = offset.saturating_add(limit).min(total_items);

    let mut rows = Vec::with_capacity(end.saturating_sub(offset));
    match value {
        Value::Object(map) => {
            for (index, (key, child)) in map.iter().enumerate().skip(offset).take(limit) {
                rows.push(row(
                    Some(key),
                    child,
                    child_pointer(pointer, key),
                    index + 1 < total_items,
                    version,
                ));
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate().skip(offset).take(limit) {
                rows.push(row(
                    None,
                    child,
                    child_pointer(pointer, &index.to_string()),
                    index + 1 < total_items,
                    version,
                ));
            }
        }
        _ => return Err(Error::InvalidPointer),
    }

    Ok(Outline {
        rows,
        opening: if value.is_object() { "{" } else { "[" },
        closing: if value.is_object() { "}" } else { "]" },
        has_previous: offset > 0,
        previous_url: fragment_url(version, pointer, offset.saturating_sub(limit), limit),
        has_next: end < total_items,
        next_url: fragment_url(version, pointer, end, limit),
        first_item: if total_items == 0 { 0 } else { offset + 1 },
        last_item: end,
        total_items,
    })
}

fn row(
    key: Option<&str>,
    value: &Value,
    pointer: Option<String>,
    comma: bool,
    version: FhirVersion,
) -> Row {
    let (display_key, key_truncated) = key
        .map(|key| truncate(key, MAX_DISPLAY_KEY_CHARS))
        .unwrap_or_else(|| (String::new(), false));
    let prefix = key
        .map(|_| format!("{}: ", json_string(&display_key)))
        .unwrap_or_default();

    if let Some(length) = container_len(value) {
        let summary = if value.is_object() {
            format!("{{ {length} }}")
        } else {
            format!("[ {length} ]")
        };
        let expandable = pointer
            .as_deref()
            .is_some_and(|pointer| validate_pointer(pointer).is_ok());
        let fragment_url = pointer
            .as_deref()
            .filter(|_| expandable)
            .map(|pointer| fragment_url(version, pointer, 0, DEFAULT_PAGE_SIZE))
            .unwrap_or_default();
        return Row {
            prefix,
            tokens: Vec::new(),
            is_container: true,
            summary,
            fragment_url,
            expandable,
            truncated: key_truncated,
            comma,
        };
    }

    let (display_value, value_truncated) = match value {
        Value::String(text) => {
            let (text, truncated) = truncate(text, MAX_DISPLAY_STRING_CHARS);
            (Value::String(text), truncated)
        }
        _ => (value.clone(), false),
    };
    let tokens = json_view::try_lines(
        &display_value,
        json_view::RenderOptions {
            include_paths: false,
            budget: Some(json_view::RenderBudget {
                max_lines: 1,
                max_estimated_html_bytes: 16 * 1024,
            }),
        },
    )
    .ok()
    .and_then(|mut lines| lines.pop())
    .map(|line| line.tokens)
    .unwrap_or_default();

    Row {
        prefix,
        tokens,
        is_container: false,
        summary: String::new(),
        fragment_url: String::new(),
        expandable: false,
        truncated: key_truncated || value_truncated,
        comma,
    }
}

fn container_len(value: &Value) -> Option<usize> {
    match value {
        Value::Object(map) => Some(map.len()),
        Value::Array(items) => Some(items.len()),
        _ => None,
    }
}

fn child_pointer(parent: &str, segment: &str) -> Option<String> {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    let pointer = format!("{parent}/{escaped}");
    (pointer.len() <= MAX_POINTER_BYTES).then_some(pointer)
}

fn fragment_url(version: FhirVersion, pointer: &str, offset: usize, limit: usize) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("version", version.as_str());
    query.append_pair("path", pointer);
    query.append_pair("offset", &offset.to_string());
    query.append_pair("limit", &limit.to_string());
    format!("/ui/capability-statement/json-fragment?{}", query.finish())
}

pub(crate) fn root_fragment_url(version: FhirVersion) -> String {
    fragment_url(version, "", 0, DEFAULT_PAGE_SIZE)
}

fn validate_pointer(pointer: &str) -> Result<(), Error> {
    if pointer.len() > MAX_POINTER_BYTES {
        return Err(Error::InvalidPointer);
    }
    if pointer.is_empty() {
        return Ok(());
    }
    if !pointer.starts_with('/') || pointer.split('/').skip(1).count() > MAX_POINTER_DEPTH {
        return Err(Error::InvalidPointer);
    }
    for segment in pointer.split('/').skip(1) {
        let bytes = segment.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'~' {
                if index + 1 >= bytes.len() || !matches!(bytes[index + 1], b'0' | b'1') {
                    return Err(Error::InvalidPointer);
                }
                index += 2;
            } else {
                index += 1;
            }
        }
    }
    Ok(())
}

fn truncate(value: &str, max_chars: usize) -> (String, bool) {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        (format!("{prefix}…"), true)
    } else {
        (prefix, false)
    }
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn expect_outline(view: View) -> Outline {
        match view {
            View::Outline(outline) => outline,
            View::Full(_) => panic!("expected an outline"),
        }
    }

    fn expect_full(view: View) -> Vec<json_view::JsonLine> {
        match view {
            View::Full(lines) => lines,
            View::Outline(_) => panic!("expected the shared renderer"),
        }
    }

    #[test]
    fn rejects_invalid_or_excessively_deep_pointers() {
        let version = FhirVersion::default_enabled();
        assert_eq!(
            plan(&json!({}), "not-a-pointer", 0, 10, version).err(),
            Some(Error::InvalidPointer)
        );
        assert_eq!(
            plan(&json!({}), "/bad~2escape", 0, 10, version).err(),
            Some(Error::InvalidPointer)
        );
        let deep = format!("/{}", vec!["x"; MAX_POINTER_DEPTH + 1].join("/"));
        assert_eq!(
            plan(&json!({}), &deep, 0, 10, version).err(),
            Some(Error::InvalidPointer)
        );
        let overlong = format!("/{}", "x".repeat(MAX_POINTER_BYTES));
        assert_eq!(
            plan(&json!({}), &overlong, 0, 10, version).err(),
            Some(Error::InvalidPointer)
        );
        assert_eq!(
            plan(&json!(true), "", 1, 10, version).err(),
            Some(Error::InvalidPointer)
        );
    }

    #[test]
    fn large_arrays_are_paged_and_keep_pointer_escaping() {
        let version = FhirVersion::default_enabled();
        let document = json!({"a/b~c": (0..205).collect::<Vec<_>>()});
        let first = expect_outline(plan(&document, "/a~1b~0c", 0, MAX_PAGE_SIZE, version).unwrap());
        assert_eq!(first.rows.len(), 100);
        assert_eq!((first.first_item, first.last_item), (1, 100));
        assert!(first.has_next);
        assert!(first.next_url.contains("offset=100"));

        let last =
            expect_outline(plan(&document, "/a~1b~0c", 200, MAX_PAGE_SIZE, version).unwrap());
        assert_eq!(last.rows.len(), 5);
        assert_eq!((last.first_item, last.last_item), (201, 205));
        assert!(!last.has_next);
        assert!(last.has_previous);
    }

    #[test]
    fn small_values_keep_using_the_shared_renderer() {
        let lines = expect_full(
            plan(
                &json!({"ok": true}),
                "",
                0,
                100,
                FhirVersion::default_enabled(),
            )
            .unwrap(),
        );
        assert!(
            lines
                .iter()
                .any(|line| line.tokens.iter().any(|token| token.kind == "key"))
        );
    }

    #[test]
    fn object_rows_and_long_scalars_expose_bounded_summaries() {
        let version = FhirVersion::default_enabled();
        let object = row(
            Some("nested"),
            &json!({"value": true}),
            Some("/nested".to_string()),
            false,
            version,
        );
        assert_eq!(object.summary, "{ 1 }");
        assert!(object.expandable);

        assert_eq!(truncate("abcdef", 3), ("abc…".to_string(), true));
    }

    #[test]
    #[should_panic(expected = "expected an outline")]
    fn outline_test_helper_rejects_full_views() {
        let view = plan(
            &json!({"ok": true}),
            "",
            0,
            100,
            FhirVersion::default_enabled(),
        )
        .unwrap();
        expect_outline(view);
    }

    #[test]
    #[should_panic(expected = "expected the shared renderer")]
    fn full_test_helper_rejects_outlines() {
        let view = plan(
            &json!((0..101).collect::<Vec<_>>()),
            "",
            0,
            100,
            FhirVersion::default_enabled(),
        )
        .unwrap();
        expect_full(view);
    }
}
