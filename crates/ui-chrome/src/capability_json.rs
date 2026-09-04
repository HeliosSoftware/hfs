//! Bounded, incremental JSON rendering for the CapabilityStatement page
//! (#808, generalized from HFS's original #798 work).
//!
//! CapabilityStatements are server-owned but can be tens of thousands of
//! lines — HTS's grows with every loaded code system, ~1,975
//! `capabilitystatement-supported-system` extensions and 422 KB against the
//! bundled seed set. Rendering one as a complete highlighted DOM is
//! substantially more expensive than returning its JSON. This adapter keeps
//! that exceptional case local to the CapabilityStatement page: small
//! subtrees still use the shared JSON renderer ([`crate::json_view`]), while
//! large containers expose one bounded level at a time through validated
//! JSON Pointer requests.
//!
//! HFS and HTS mount the fragment endpoint at different paths and echo back
//! different version codes, so every function here takes a
//! [`FragmentEndpoint`] rather than hard-coding either.

use crate::ChromeLabels;
use crate::json_view;
use askama::Template;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const DEFAULT_PAGE_SIZE: usize = 100;
pub const MAX_PAGE_SIZE: usize = 100;
pub const MAX_FRAGMENT_HTML_BYTES: usize = 1024 * 1024;
pub const MAX_EXPANDED_ROWS: usize = 1_000;
pub const MAX_PAGE_DESCRIPTORS: usize = 64;
pub const MAX_PAGE_STATE_FORM_BYTES: usize = 256 * 1024;

const MAX_POINTER_BYTES: usize = 1024;
const MAX_POINTER_DEPTH: usize = 32;
const FULL_MAX_LINES: usize = 1_000;
const FULL_MAX_ESTIMATED_HTML_BYTES: usize = MAX_FRAGMENT_HTML_BYTES;
const MAX_DISPLAY_KEY_CHARS: usize = 128;
const MAX_DISPLAY_STRING_CHARS: usize = 256;

/// Where the fragment endpoint lives and which release code to echo back on
/// every paginated link — the two bits of context only the caller knows.
/// HFS mounts at `/ui/capability-statement/json-fragment` and echoes a
/// `FhirVersion`; HTS mounts at `/ui/hts/capability-statement/json-fragment`
/// and echoes the release the binary was built for.
#[derive(Clone, Copy)]
pub struct FragmentEndpoint<'a> {
    pub base_path: &'a str,
    pub version: &'a str,
    /// Extra, already-percent-encoded query parameters appended to every
    /// fragment URL (e.g. `system=…&code=…&target=response` for an HTS
    /// workbench that must re-issue the operation on each fragment GET).
    /// Empty for endpoints that need no context beyond the path itself,
    /// like the CapabilityStatement page.
    pub extra_query: &'a str,
}

pub enum View {
    Full(Vec<json_view::JsonLine>),
    Outline(Outline),
}

pub struct Outline {
    pub rows: Vec<Row>,
    pub pointer: String,
    pub offset: usize,
    pub limit: usize,
    pub opening: &'static str,
    pub closing: &'static str,
    pub has_previous: bool,
    pub previous_url: String,
    pub has_next: bool,
    pub next_url: String,
    pub first_item: usize,
    pub last_item: usize,
    pub total_items: usize,
}

pub struct Row {
    pub prefix: String,
    pub tokens: Vec<json_view::Token>,
    pub is_container: bool,
    pub summary: String,
    pub fragment_url: String,
    pub expandable: bool,
    pub truncated: bool,
    pub comma: bool,
}

/// One visible page that an aggregate expansion should preserve.
///
/// Callers collect these from the rendered `data-path`, `data-offset`, and
/// `data-limit` attributes. The planner validates every descriptor against
/// the current document before it starts selecting containers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageDescriptor {
    pub pointer: String,
    pub offset: usize,
    pub limit: usize,
}

impl PageDescriptor {
    pub fn new(pointer: impl Into<String>, offset: usize, limit: usize) -> Self {
        Self {
            pointer: pointer.into(),
            offset,
            limit,
        }
    }
}

/// Parses the parallel repeated fields sent by the progressively enhanced
/// Raw CapabilityStatement toolbar.
///
/// HTML form encoding represents every visible page as one `path`, `offset`,
/// and `limit` occurrence. Axum's ordinary `Form` extractor cannot retain
/// repeated keys, so both HFS and HTS hand the bounded body to this shared
/// parser before calling [`plan_expanded`].
pub fn parse_page_descriptors(body: &[u8]) -> Result<Vec<PageDescriptor>, Error> {
    if body.len() > MAX_PAGE_STATE_FORM_BYTES {
        return Err(Error::InvalidPage);
    }

    let mut paths = Vec::new();
    let mut offsets = Vec::new();
    let mut limits = Vec::new();
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "path" => paths.push(value.into_owned()),
            "offset" => offsets.push(value.parse::<usize>().map_err(|_| Error::InvalidPage)?),
            "limit" => limits.push(value.parse::<usize>().map_err(|_| Error::InvalidPage)?),
            _ => return Err(Error::InvalidPage),
        }
        if paths.len() > MAX_PAGE_DESCRIPTORS
            || offsets.len() > MAX_PAGE_DESCRIPTORS
            || limits.len() > MAX_PAGE_DESCRIPTORS
        {
            return Err(Error::InvalidPage);
        }
    }
    if paths.len() != offsets.len() || paths.len() != limits.len() {
        return Err(Error::InvalidPage);
    }

    Ok(paths
        .into_iter()
        .zip(offsets)
        .zip(limits)
        .map(|((pointer, offset), limit)| PageDescriptor::new(pointer, offset, limit))
        .collect())
}

/// Whether the aggregate response contains every item in the document.
///
/// A paginated container makes the response partial even when the planner
/// expands every item on its visible page. The response never follows a page
/// link on the user's behalf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpansionState {
    Complete,
    Partial,
}

impl ExpansionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
        }
    }
}

/// A fully planned aggregate expansion, ready for one Askama render.
pub struct Expanded {
    root: PageFrame,
    events: Vec<ExpandedEvent>,
    state: ExpansionState,
    row_count: usize,
}

impl Expanded {
    pub fn state(&self) -> ExpansionState {
        self.state
    }

    pub fn row_count(&self) -> usize {
        self.row_count
    }
}

struct PageFrame {
    pointer: String,
    offset: usize,
    limit: usize,
    item_count: usize,
    opening: &'static str,
    closing: &'static str,
    has_previous: bool,
    previous_url: String,
    has_next: bool,
    next_url: String,
    first_item: usize,
    last_item: usize,
    total_items: usize,
}

struct ExpandedContainer {
    row: Row,
    page: PageFrame,
}

enum ExpandedEvent {
    Scalar(Row),
    Collapsed(Row),
    Open(ExpandedContainer),
    Close(PageFrame),
}

/// Rendering can fail at template time or at the final byte guard.
#[derive(Debug)]
pub enum ExpandedRenderError {
    Template(askama::Error),
    TooLarge,
}

impl From<askama::Error> for ExpandedRenderError {
    fn from(error: askama::Error) -> Self {
        Self::Template(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidPointer,
    InvalidPage,
    NotFound,
}

pub fn plan(
    document: &Value,
    pointer: &str,
    offset: usize,
    limit: usize,
    endpoint: FragmentEndpoint,
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

    // The root is always an outline. That gives every CapabilityStatement the
    // same initial shape: root braces and first-level keys, with containers
    // closed. Small nested values still use the denser shared renderer.
    if !pointer.is_empty()
        && offset == 0
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

    outline(value, pointer, offset, limit, endpoint).map(View::Outline)
}

fn outline(
    value: &Value,
    pointer: &str,
    offset: usize,
    limit: usize,
    endpoint: FragmentEndpoint,
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
                    endpoint,
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
                    endpoint,
                ));
            }
        }
        _ => return Err(Error::InvalidPointer),
    }

    Ok(Outline {
        rows,
        pointer: pointer.to_string(),
        offset,
        limit,
        opening: if value.is_object() { "{" } else { "[" },
        closing: if value.is_object() { "}" } else { "]" },
        has_previous: offset > 0,
        previous_url: fragment_url(endpoint, pointer, offset.saturating_sub(limit), limit),
        has_next: end < total_items,
        next_url: fragment_url(endpoint, pointer, end, limit),
        first_item: if total_items == 0 { 0 } else { offset + 1 },
        last_item: end,
        total_items,
    })
}

#[derive(Clone, Copy)]
struct ExpansionBudget {
    max_rows: usize,
    max_estimated_html_bytes: usize,
}

struct Selection {
    expanded: BTreeSet<String>,
    pages: BTreeMap<String, (usize, usize)>,
    state: ExpansionState,
    row_count: usize,
}

/// Plans one bounded, aggregate expansion of the CapabilityStatement.
///
/// Selection is breadth-first so a large early branch cannot starve its
/// siblings. Rendering later walks the selected tree depth-first, preserving
/// document order and balanced markup. Only the supplied page for each
/// container is considered; containers without a descriptor use their first
/// page. No next/previous link is followed automatically.
pub fn plan_expanded(
    document: &Value,
    page_descriptors: &[PageDescriptor],
    endpoint: FragmentEndpoint,
) -> Result<Expanded, Error> {
    plan_expanded_with_budget(
        document,
        page_descriptors,
        endpoint,
        ExpansionBudget {
            max_rows: MAX_EXPANDED_ROWS,
            max_estimated_html_bytes: MAX_FRAGMENT_HTML_BYTES,
        },
    )
}

fn plan_expanded_with_budget(
    document: &Value,
    page_descriptors: &[PageDescriptor],
    endpoint: FragmentEndpoint,
    budget: ExpansionBudget,
) -> Result<Expanded, Error> {
    let pages = validate_page_descriptors(document, page_descriptors)?;
    let root_value = document;
    container_len(root_value).ok_or(Error::InvalidPointer)?;
    let (root_offset, root_limit) = page_for(&pages, "");
    let root_frame = page_frame(root_value, "", root_offset, root_limit, endpoint)?;

    let mut state = if root_frame.has_previous || root_frame.has_next {
        ExpansionState::Partial
    } else {
        ExpansionState::Complete
    };
    // Opening and closing delimiters are visible rows and consume the same
    // global DOM budget as entries do.
    let mut row_count = root_frame.item_count.saturating_add(2);
    let mut estimated_bytes = estimate_page(root_value, "", root_offset, root_limit, endpoint)?;
    let mut expanded = BTreeSet::new();
    let mut queue = VecDeque::new();
    enqueue_containers(
        root_value,
        "",
        root_offset,
        root_limit,
        &mut queue,
        &mut state,
    )?;

    while let Some(pointer) = queue.pop_front() {
        let value = document.pointer(&pointer).ok_or(Error::NotFound)?;
        let (offset, limit) = page_for(&pages, &pointer);
        let total_items = container_len(value).ok_or(Error::InvalidPage)?;
        if offset > total_items || limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(Error::InvalidPage);
        }
        let added_rows = total_items
            .min(offset.saturating_add(limit))
            .saturating_sub(offset)
            .saturating_add(2);
        let added_bytes = estimate_page(value, &pointer, offset, limit, endpoint)?
            .saturating_add(ESTIMATED_EXPANDED_CONTAINER_BYTES);

        if row_count.saturating_add(added_rows) > budget.max_rows
            || estimated_bytes.saturating_add(added_bytes) > budget.max_estimated_html_bytes
        {
            state = ExpansionState::Partial;
            continue;
        }

        let frame = page_frame(value, &pointer, offset, limit, endpoint)?;
        row_count += added_rows;
        estimated_bytes = estimated_bytes.saturating_add(added_bytes);
        if frame.has_previous || frame.has_next {
            state = ExpansionState::Partial;
        }
        expanded.insert(pointer.clone());
        enqueue_containers(value, &pointer, offset, limit, &mut queue, &mut state)?;
    }

    let selection = Selection {
        expanded,
        pages,
        state,
        row_count,
    };
    let mut events = Vec::with_capacity(selection.row_count.saturating_mul(2));
    emit_page(
        document,
        document,
        "",
        root_offset,
        root_limit,
        endpoint,
        &selection,
        &mut events,
    )?;

    Ok(Expanded {
        root: root_frame,
        events,
        state: selection.state,
        row_count: selection.row_count,
    })
}

fn validate_page_descriptors(
    document: &Value,
    descriptors: &[PageDescriptor],
) -> Result<BTreeMap<String, (usize, usize)>, Error> {
    if descriptors.len() > MAX_PAGE_DESCRIPTORS {
        return Err(Error::InvalidPage);
    }
    let mut pages = BTreeMap::new();
    for descriptor in descriptors {
        validate_pointer(&descriptor.pointer)?;
        if descriptor.limit == 0 || descriptor.limit > MAX_PAGE_SIZE {
            return Err(Error::InvalidPage);
        }
        let value = if descriptor.pointer.is_empty() {
            document
        } else {
            document
                .pointer(&descriptor.pointer)
                .ok_or(Error::NotFound)?
        };
        let length = container_len(value).ok_or(Error::InvalidPage)?;
        if descriptor.offset > length
            || pages
                .insert(
                    descriptor.pointer.clone(),
                    (descriptor.offset, descriptor.limit),
                )
                .is_some()
        {
            return Err(Error::InvalidPage);
        }
    }
    Ok(pages)
}

fn page_for(pages: &BTreeMap<String, (usize, usize)>, pointer: &str) -> (usize, usize) {
    pages
        .get(pointer)
        .copied()
        .unwrap_or((0, DEFAULT_PAGE_SIZE))
}

fn page_frame(
    value: &Value,
    pointer: &str,
    offset: usize,
    limit: usize,
    endpoint: FragmentEndpoint,
) -> Result<PageFrame, Error> {
    let total_items = container_len(value).ok_or(Error::InvalidPage)?;
    if offset > total_items || limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(Error::InvalidPage);
    }
    let end = offset.saturating_add(limit).min(total_items);
    Ok(PageFrame {
        pointer: pointer.to_string(),
        offset,
        limit,
        item_count: end.saturating_sub(offset),
        opening: if value.is_object() { "{" } else { "[" },
        closing: if value.is_object() { "}" } else { "]" },
        has_previous: offset > 0,
        previous_url: fragment_url(endpoint, pointer, offset.saturating_sub(limit), limit),
        has_next: end < total_items,
        next_url: fragment_url(endpoint, pointer, end, limit),
        first_item: if total_items == 0 { 0 } else { offset + 1 },
        last_item: end,
        total_items,
    })
}

fn enqueue_containers(
    value: &Value,
    pointer: &str,
    offset: usize,
    limit: usize,
    queue: &mut VecDeque<String>,
    state: &mut ExpansionState,
) -> Result<(), Error> {
    let total = container_len(value).ok_or(Error::InvalidPage)?;
    if offset > total || limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(Error::InvalidPage);
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter().skip(offset).take(limit) {
                if child.is_object() || child.is_array() {
                    if let Some(child) = child_pointer(pointer, key) {
                        queue.push_back(child);
                    } else {
                        *state = ExpansionState::Partial;
                    }
                }
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate().skip(offset).take(limit) {
                if child.is_object() || child.is_array() {
                    if let Some(child) = child_pointer(pointer, &index.to_string()) {
                        queue.push_back(child);
                    } else {
                        *state = ExpansionState::Partial;
                    }
                }
            }
        }
        _ => return Err(Error::InvalidPage),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_page(
    document: &Value,
    value: &Value,
    pointer: &str,
    offset: usize,
    limit: usize,
    endpoint: FragmentEndpoint,
    selection: &Selection,
    events: &mut Vec<ExpandedEvent>,
) -> Result<(), Error> {
    let total_items = container_len(value).ok_or(Error::InvalidPage)?;
    match value {
        Value::Object(map) => {
            for (index, (key, child)) in map.iter().enumerate().skip(offset).take(limit) {
                emit_node(
                    document,
                    Some(key),
                    child,
                    child_pointer(pointer, key),
                    index + 1 < total_items,
                    endpoint,
                    selection,
                    events,
                )?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate().skip(offset).take(limit) {
                emit_node(
                    document,
                    None,
                    child,
                    child_pointer(pointer, &index.to_string()),
                    index + 1 < total_items,
                    endpoint,
                    selection,
                    events,
                )?;
            }
        }
        _ => return Err(Error::InvalidPage),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_node(
    document: &Value,
    key: Option<&str>,
    value: &Value,
    pointer: Option<String>,
    comma: bool,
    endpoint: FragmentEndpoint,
    selection: &Selection,
    events: &mut Vec<ExpandedEvent>,
) -> Result<(), Error> {
    let rendered_row = row(key, value, pointer.clone(), comma, endpoint);
    if !(value.is_object() || value.is_array()) {
        events.push(ExpandedEvent::Scalar(rendered_row));
        return Ok(());
    }
    let Some(pointer) = pointer else {
        events.push(ExpandedEvent::Collapsed(rendered_row));
        return Ok(());
    };
    if !selection.expanded.contains(&pointer) {
        events.push(ExpandedEvent::Collapsed(rendered_row));
        return Ok(());
    }

    let child = document.pointer(&pointer).ok_or(Error::NotFound)?;
    let (offset, limit) = page_for(&selection.pages, &pointer);
    let frame = page_frame(child, &pointer, offset, limit, endpoint)?;
    events.push(ExpandedEvent::Open(ExpandedContainer {
        row: rendered_row,
        page: frame,
    }));
    emit_page(
        document,
        document.pointer(&pointer).ok_or(Error::NotFound)?,
        &pointer,
        offset,
        limit,
        endpoint,
        selection,
        events,
    )?;
    events.push(ExpandedEvent::Close(page_frame(
        child, &pointer, offset, limit, endpoint,
    )?));
    Ok(())
}

const ESTIMATED_PAGE_BYTES: usize = 2_048;
const ESTIMATED_ROW_BYTES: usize = 1_024;
const ESTIMATED_EXPANDED_CONTAINER_BYTES: usize = 2_048;
const MAX_ESCAPED_BYTES_PER_SOURCE_BYTE: usize = 6;

fn estimate_page(
    value: &Value,
    pointer: &str,
    offset: usize,
    limit: usize,
    endpoint: FragmentEndpoint,
) -> Result<usize, Error> {
    let total = container_len(value).ok_or(Error::InvalidPage)?;
    if offset > total || limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(Error::InvalidPage);
    }
    let mut bytes =
        ESTIMATED_PAGE_BYTES.saturating_add(estimate_url_bytes(endpoint, pointer, offset, limit));
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter().skip(offset).take(limit) {
                let child_pointer = child_pointer(pointer, key);
                bytes = bytes.saturating_add(estimate_row_bytes(
                    Some(key),
                    child,
                    child_pointer.as_deref().unwrap_or(pointer),
                    endpoint,
                ));
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate().skip(offset).take(limit) {
                let index = index.to_string();
                let child_pointer = child_pointer(pointer, &index);
                bytes = bytes.saturating_add(estimate_row_bytes(
                    None,
                    child,
                    child_pointer.as_deref().unwrap_or(pointer),
                    endpoint,
                ));
            }
        }
        _ => return Err(Error::InvalidPage),
    }
    Ok(bytes)
}

fn estimate_row_bytes(
    key: Option<&str>,
    value: &Value,
    pointer: &str,
    endpoint: FragmentEndpoint,
) -> usize {
    let key_bytes = key
        .map(|key| capped_utf8_bytes(key, MAX_DISPLAY_KEY_CHARS))
        .unwrap_or(0);
    let value_bytes = match value {
        Value::String(text) => capped_utf8_bytes(text, MAX_DISPLAY_STRING_CHARS),
        _ => 64,
    };
    let url_bytes = if value.is_object() || value.is_array() {
        estimate_url_bytes(endpoint, pointer, 0, DEFAULT_PAGE_SIZE)
    } else {
        0
    };
    ESTIMATED_ROW_BYTES
        .saturating_add(
            key_bytes
                .saturating_add(value_bytes)
                .saturating_mul(MAX_ESCAPED_BYTES_PER_SOURCE_BYTE),
        )
        .saturating_add(url_bytes)
}

fn capped_utf8_bytes(value: &str, max_chars: usize) -> usize {
    value
        .chars()
        .take(max_chars)
        .map(char::len_utf8)
        .fold(0usize, usize::saturating_add)
}

fn estimate_url_bytes(
    endpoint: FragmentEndpoint,
    pointer: &str,
    _offset: usize,
    _limit: usize,
) -> usize {
    endpoint
        .base_path
        .len()
        .saturating_add(endpoint.version.len())
        .saturating_add(pointer.len().saturating_mul(3))
        .saturating_add(96)
        .saturating_mul(MAX_ESCAPED_BYTES_PER_SOURCE_BYTE)
}

fn row(
    key: Option<&str>,
    value: &Value,
    pointer: Option<String>,
    comma: bool,
    endpoint: FragmentEndpoint,
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
            .map(|pointer| fragment_url(endpoint, pointer, 0, DEFAULT_PAGE_SIZE))
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
    validate_pointer(&pointer).is_ok().then_some(pointer)
}

fn fragment_url(endpoint: FragmentEndpoint, pointer: &str, offset: usize, limit: usize) -> String {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("version", endpoint.version);
    query.append_pair("path", pointer);
    query.append_pair("offset", &offset.to_string());
    query.append_pair("limit", &limit.to_string());
    if endpoint.extra_query.is_empty() {
        format!("{}?{}", endpoint.base_path, query.finish())
    } else {
        format!(
            "{}?{}&{}",
            endpoint.base_path,
            query.finish(),
            endpoint.extra_query
        )
    }
}

pub fn root_fragment_url(endpoint: FragmentEndpoint) -> String {
    fragment_url(endpoint, "", 0, DEFAULT_PAGE_SIZE)
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

// ── Fragment templates ──────────────────────────────────────────────────────

/// A small CapabilityStatement (or subtree) rendered by the shared JSON
/// highlighter.
#[derive(Template)]
#[template(path = "partials/capability-json-full.html")]
struct FullFragmentTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    json_lines: Vec<json_view::JsonLine>,
    json_view_id: &'a str,
    /// Always false here: `plan()` calls `json_view::try_lines` with
    /// `include_paths: false`, so every [`json_view::JsonLine::path`] this
    /// template would key off is already empty.
    json_view_paths: bool,
}

/// One bounded level of a large CapabilityStatement.
#[derive(Template)]
#[template(path = "partials/capability-json-outline.html")]
struct OutlineFragmentTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    outline: &'a Outline,
}

/// One aggregate response selected breadth-first and emitted depth-first.
#[derive(Template)]
#[template(path = "partials/capability-json-expanded.html")]
struct ExpandedFragmentTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    expanded: &'a Expanded,
}

/// Renders a [`View::Full`] payload. `is_root` selects the DOM id the page's
/// own fold controls key off (`#capability-json`); a nested fragment gets no
/// id of its own.
pub fn render_full(
    i18n: &dyn ChromeLabels,
    json_lines: Vec<json_view::JsonLine>,
    is_root: bool,
) -> Result<String, askama::Error> {
    FullFragmentTemplate {
        i18n,
        json_lines,
        json_view_id: if is_root { "capability-json" } else { "" },
        json_view_paths: false,
    }
    .render()
}

/// Renders a [`View::Outline`] page.
pub fn render_outline(i18n: &dyn ChromeLabels, outline: &Outline) -> Result<String, askama::Error> {
    OutlineFragmentTemplate { i18n, outline }.render()
}

/// Renders an aggregate expansion and enforces the final response-size cap.
///
/// The planner applies a conservative estimate before it adds a page. This
/// final check covers template fixed costs and future markup changes.
pub fn render_expanded(
    i18n: &dyn ChromeLabels,
    expanded: &Expanded,
) -> Result<String, ExpandedRenderError> {
    let html = ExpandedFragmentTemplate { i18n, expanded }.render()?;
    if html.len() > MAX_FRAGMENT_HTML_BYTES {
        return Err(ExpandedRenderError::TooLarge);
    }
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ENDPOINT: FragmentEndpoint<'static> = FragmentEndpoint {
        base_path: "/ui/capability-statement/json-fragment",
        version: "R4",
        extra_query: "",
    };

    struct Labels;

    impl ChromeLabels for Labels {
        fn lang(&self) -> String {
            "en".to_string()
        }

        fn t(&self, key: &str) -> String {
            format!("[{key}]")
        }
    }

    fn open_paths(expanded: &Expanded) -> Vec<&str> {
        expanded
            .events
            .iter()
            .filter_map(|event| match event {
                ExpandedEvent::Open(node) => Some(node.page.pointer.as_str()),
                _ => None,
            })
            .collect()
    }

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
        assert_eq!(
            plan(&json!({}), "not-a-pointer", 0, 10, ENDPOINT).err(),
            Some(Error::InvalidPointer)
        );
        assert_eq!(
            plan(&json!({}), "/bad~2escape", 0, 10, ENDPOINT).err(),
            Some(Error::InvalidPointer)
        );
        let deep = format!("/{}", vec!["x"; MAX_POINTER_DEPTH + 1].join("/"));
        assert_eq!(
            plan(&json!({}), &deep, 0, 10, ENDPOINT).err(),
            Some(Error::InvalidPointer)
        );
        let overlong = format!("/{}", "x".repeat(MAX_POINTER_BYTES));
        assert_eq!(
            plan(&json!({}), &overlong, 0, 10, ENDPOINT).err(),
            Some(Error::InvalidPointer)
        );
        assert_eq!(
            plan(&json!(true), "", 1, 10, ENDPOINT).err(),
            Some(Error::InvalidPointer)
        );
    }

    #[test]
    fn large_arrays_are_paged_and_keep_pointer_escaping() {
        let document = json!({"a/b~c": (0..205).collect::<Vec<_>>()});
        let first =
            expect_outline(plan(&document, "/a~1b~0c", 0, MAX_PAGE_SIZE, ENDPOINT).unwrap());
        assert_eq!(first.rows.len(), 100);
        assert_eq!((first.first_item, first.last_item), (1, 100));
        assert!(first.has_next);
        assert!(first.next_url.contains("offset=100"));

        let last =
            expect_outline(plan(&document, "/a~1b~0c", 200, MAX_PAGE_SIZE, ENDPOINT).unwrap());
        assert_eq!(last.rows.len(), 5);
        assert_eq!((last.first_item, last.last_item), (201, 205));
        assert!(!last.has_next);
        assert!(last.has_previous);
    }

    #[test]
    fn a_small_root_is_an_outline_with_only_its_first_level() {
        let outline =
            expect_outline(plan(&json!({"nested": {"ok": true}}), "", 0, 100, ENDPOINT).unwrap());
        assert_eq!(outline.rows.len(), 1);
        assert!(outline.rows[0].is_container);
        assert_eq!(outline.rows[0].summary, "{ 1 }");
        assert_eq!(outline.pointer, "");
    }

    #[test]
    fn small_nested_values_keep_using_the_shared_renderer() {
        let document = json!({"nested": {"ok": true}});
        let lines = expect_full(plan(&document, "/nested", 0, 100, ENDPOINT).unwrap());
        assert!(
            lines
                .iter()
                .any(|line| line.tokens.iter().any(|token| token.kind == "key"))
        );
    }

    #[test]
    fn object_rows_and_long_scalars_expose_bounded_summaries() {
        let object = row(
            Some("nested"),
            &json!({"value": true}),
            Some("/nested".to_string()),
            false,
            ENDPOINT,
        );
        assert_eq!(object.summary, "{ 1 }");
        assert!(object.expandable);

        assert_eq!(truncate("abcdef", 3), ("abc…".to_string(), true));
    }

    #[test]
    #[should_panic(expected = "expected an outline")]
    fn outline_test_helper_rejects_full_views() {
        let document = json!({"nested": {"ok": true}});
        let view = plan(&document, "/nested", 0, 100, ENDPOINT).unwrap();
        expect_outline(view);
    }

    #[test]
    #[should_panic(expected = "expected the shared renderer")]
    fn full_test_helper_rejects_outlines() {
        let view = plan(&json!((0..101).collect::<Vec<_>>()), "", 0, 100, ENDPOINT).unwrap();
        expect_full(view);
    }

    #[test]
    fn fragment_urls_carry_the_callers_own_base_path_and_version() {
        let hts_endpoint = FragmentEndpoint {
            base_path: "/ui/hts/capability-statement/json-fragment",
            version: "R4",
            extra_query: "",
        };
        let url = root_fragment_url(hts_endpoint);
        assert!(url.starts_with("/ui/hts/capability-statement/json-fragment?"));
        assert!(url.contains("version=R4"));
    }

    #[test]
    fn aggregate_selection_is_breadth_first_and_fair_to_root_siblings() {
        let document = json!({
            "a": {"a1": {"value": 1}},
            "b": {"b1": {"value": 2}}
        });
        let expanded = plan_expanded_with_budget(
            &document,
            &[],
            ENDPOINT,
            ExpansionBudget {
                max_rows: 10,
                max_estimated_html_bytes: usize::MAX,
            },
        )
        .unwrap();

        assert_eq!(open_paths(&expanded), vec!["/a", "/b"]);
        assert_eq!(expanded.row_count(), 10);
        assert_eq!(expanded.state(), ExpansionState::Partial);
    }

    #[test]
    fn aggregate_row_and_byte_budgets_are_inclusive_at_the_boundary() {
        let document = json!({"a": {"value": 1}});
        let root_bytes = estimate_page(&document, "", 0, 100, ENDPOINT).unwrap();
        let child = document.pointer("/a").unwrap();
        let full_bytes = root_bytes
            + estimate_page(child, "/a", 0, 100, ENDPOINT).unwrap()
            + ESTIMATED_EXPANDED_CONTAINER_BYTES;

        let exact = plan_expanded_with_budget(
            &document,
            &[],
            ENDPOINT,
            ExpansionBudget {
                max_rows: 6,
                max_estimated_html_bytes: full_bytes,
            },
        )
        .unwrap();
        assert_eq!(exact.state(), ExpansionState::Complete);
        assert_eq!(open_paths(&exact), vec!["/a"]);

        let one_byte_short = plan_expanded_with_budget(
            &document,
            &[],
            ENDPOINT,
            ExpansionBudget {
                max_rows: 6,
                max_estimated_html_bytes: full_bytes - 1,
            },
        )
        .unwrap();
        assert_eq!(one_byte_short.state(), ExpansionState::Partial);
        assert!(open_paths(&one_byte_short).is_empty());

        let one_row_short = plan_expanded_with_budget(
            &document,
            &[],
            ENDPOINT,
            ExpansionBudget {
                max_rows: 3,
                max_estimated_html_bytes: usize::MAX,
            },
        )
        .unwrap();
        assert_eq!(one_row_short.state(), ExpansionState::Partial);
        assert_eq!(one_row_short.row_count(), 3);
    }

    #[test]
    fn aggregate_render_is_deterministic_balanced_and_marked() {
        let document = json!({
            "a": {"nested": {"value": true}},
            "b": [1, 2]
        });
        let first = plan_expanded(&document, &[], ENDPOINT).unwrap();
        let second = plan_expanded(&document, &[], ENDPOINT).unwrap();
        let first_html = render_expanded(&Labels, &first).unwrap();
        let second_html = render_expanded(&Labels, &second).unwrap();

        assert_eq!(first_html, second_html);
        assert_eq!(
            first_html.matches("<details").count(),
            first_html.matches("</details>").count()
        );
        assert!(first_html.contains(r#"data-expansion-state="complete""#));
        assert!(first_html.contains(r#"data-path="/a/nested""#));
        assert!(first_html.contains(r#"aria-expanded"#) || first_html.contains(" open>"));
    }

    #[test]
    fn aggregate_never_follows_a_later_page() {
        let items: Vec<Value> = (0..205)
            .map(|index| json!({"marker": format!("page-{index}")}))
            .collect();
        let document = json!({"items": items});
        let expanded = plan_expanded(&document, &[], ENDPOINT).unwrap();
        let html = render_expanded(&Labels, &expanded).unwrap();

        assert_eq!(expanded.state(), ExpansionState::Partial);
        assert!(html.contains("page-99"));
        assert!(!html.contains("page-100"));
        assert!(html.contains("1–100 / 205"));
        assert!(html.contains("offset=100"));
    }

    #[test]
    fn aggregate_preserves_a_valid_visible_page_offset() {
        let items: Vec<Value> = (0..205)
            .map(|index| json!({"marker": format!("page-{index}")}))
            .collect();
        let document = json!({"items": items});
        let pages = [PageDescriptor::new("/items", 100, 100)];
        let expanded = plan_expanded(&document, &pages, ENDPOINT).unwrap();
        let html = render_expanded(&Labels, &expanded).unwrap();

        assert!(html.contains("page-100"));
        assert!(!html.contains("page-0"));
        assert!(html.contains("101–200 / 205"));
        assert!(html.contains(r#"data-offset="100""#));
    }

    #[test]
    fn aggregate_rejects_duplicate_excessive_and_invalid_page_state() {
        let document = json!({"items": [1, 2], "scalar": true});
        assert_eq!(
            plan_expanded(
                &document,
                &[
                    PageDescriptor::new("/items", 0, 100),
                    PageDescriptor::new("/items", 0, 100),
                ],
                ENDPOINT,
            )
            .err(),
            Some(Error::InvalidPage)
        );
        assert_eq!(
            plan_expanded(
                &document,
                &[PageDescriptor::new("/items", 3, 100)],
                ENDPOINT,
            )
            .err(),
            Some(Error::InvalidPage)
        );
        assert_eq!(
            plan_expanded(
                &document,
                &[PageDescriptor::new("/items", 0, 101)],
                ENDPOINT,
            )
            .err(),
            Some(Error::InvalidPage)
        );
        assert_eq!(
            plan_expanded(
                &document,
                &[PageDescriptor::new("/scalar", 0, 100)],
                ENDPOINT,
            )
            .err(),
            Some(Error::InvalidPage)
        );
        let too_many: Vec<_> = (0..=MAX_PAGE_DESCRIPTORS)
            .map(|index| PageDescriptor::new(format!("/missing-{index}"), 0, 100))
            .collect();
        assert_eq!(
            plan_expanded(&document, &too_many, ENDPOINT).err(),
            Some(Error::InvalidPage)
        );
    }

    #[test]
    fn aggregate_form_parser_preserves_parallel_visible_page_state() {
        let pages =
            parse_page_descriptors(b"path=&offset=0&limit=100&path=%2Fitems&offset=100&limit=50")
                .unwrap();

        assert_eq!(
            pages,
            vec![
                PageDescriptor::new("", 0, 100),
                PageDescriptor::new("/items", 100, 50),
            ]
        );
    }

    #[test]
    fn aggregate_form_parser_rejects_mismatched_unknown_and_oversized_state() {
        for body in [
            b"path=&offset=0".as_slice(),
            b"path=&offset=nope&limit=100".as_slice(),
            b"path=&offset=0&limit=100&extra=true".as_slice(),
        ] {
            assert_eq!(parse_page_descriptors(body), Err(Error::InvalidPage));
        }
        assert_eq!(
            parse_page_descriptors(&vec![b'x'; MAX_PAGE_STATE_FORM_BYTES + 1]),
            Err(Error::InvalidPage)
        );
    }

    #[test]
    fn aggregate_escapes_hostile_keys_values_and_urls() {
        let document = json!({
            "<script>&/": {"value": "</span><img src=x onerror=alert(1)>"}
        });
        let expanded = plan_expanded(&document, &[], ENDPOINT).unwrap();
        let html = render_expanded(&Labels, &expanded).unwrap();

        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&#60;script&#62;"));
        assert!(html.contains("%3Cscript%3E%26%7E1"), "{html}");
    }

    #[test]
    fn aggregate_stops_at_the_pointer_depth_limit() {
        let mut document = json!(true);
        for _ in 0..=MAX_POINTER_DEPTH + 1 {
            document = json!({"x": document});
        }
        let expanded = plan_expanded(&document, &[], ENDPOINT).unwrap();

        assert_eq!(expanded.state(), ExpansionState::Partial);
        assert!(open_paths(&expanded).len() <= MAX_POINTER_DEPTH);
        let html = render_expanded(&Labels, &expanded).unwrap();
        assert!(html.contains("[cap-json-path-too-deep]"));
    }

    #[test]
    fn aggregate_public_plan_and_final_render_stay_within_the_html_cap() {
        let branches: Vec<Value> = (0..100)
            .map(|index| {
                json!({
                    "name": format!("branch-{index}"),
                    "payload": "<&>\"".repeat(300)
                })
            })
            .collect();
        let document = json!({"branches": branches});
        let expanded = plan_expanded(&document, &[], ENDPOINT).unwrap();
        let html = render_expanded(&Labels, &expanded).unwrap();

        assert!(html.len() <= MAX_FRAGMENT_HTML_BYTES);
        assert!(expanded.row_count() <= MAX_EXPANDED_ROWS);
    }

    #[test]
    fn final_render_guard_rejects_oversized_template_output() {
        let document = json!({"value": true});
        let mut expanded = plan_expanded(&document, &[], ENDPOINT).unwrap();
        let row = expanded
            .events
            .iter_mut()
            .find_map(|event| match event {
                ExpandedEvent::Scalar(row) => Some(row),
                _ => None,
            })
            .unwrap();
        row.prefix = "x".repeat(MAX_FRAGMENT_HTML_BYTES + 1);

        assert!(matches!(
            render_expanded(&Labels, &expanded),
            Err(ExpandedRenderError::TooLarge)
        ));
    }
}
