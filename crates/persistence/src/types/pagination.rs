//! Pagination types for search results.
//!
//! This module defines types for handling pagination in FHIR search results,
//! supporting both cursor-based and offset-based pagination.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::SearchError;

/// Pagination configuration for a search request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    /// Maximum number of results to return.
    pub count: u32,

    /// The pagination mode.
    pub mode: PaginationMode,
}

/// The pagination mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PaginationMode {
    /// Cursor-based pagination (recommended).
    Cursor(Option<PageCursor>),

    /// Offset-based pagination (for compatibility).
    Offset(u32),
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            count: 20,
            mode: PaginationMode::Cursor(None),
        }
    }
}

impl Pagination {
    /// Creates pagination with cursor mode and the specified count.
    pub fn new(count: u32) -> Self {
        Self {
            count,
            mode: PaginationMode::Cursor(None),
        }
    }

    /// Creates pagination with cursor mode and default count.
    pub fn cursor() -> Self {
        Self::default()
    }

    /// Creates pagination with a cursor string and specified count.
    pub fn with_cursor(count: u32, cursor: String) -> Self {
        match PageCursor::decode(&cursor) {
            Ok(page_cursor) => Self {
                count,
                mode: PaginationMode::Cursor(Some(page_cursor)),
            },
            Err(_) => Self {
                count,
                mode: PaginationMode::Cursor(None),
            },
        }
    }

    /// Creates pagination with offset mode.
    pub fn offset(offset: u32) -> Self {
        Self {
            count: 20,
            mode: PaginationMode::Offset(offset),
        }
    }

    /// Creates pagination from a cursor string.
    pub fn from_cursor(cursor: &str) -> Result<Self, SearchError> {
        let page_cursor = PageCursor::decode(cursor)?;
        Ok(Self {
            count: 20,
            mode: PaginationMode::Cursor(Some(page_cursor)),
        })
    }

    /// Sets the count limit.
    pub fn with_count(mut self, count: u32) -> Self {
        self.count = count;
        self
    }

    /// Returns the offset if using offset-based pagination.
    pub fn offset_value(&self) -> Option<u32> {
        match &self.mode {
            PaginationMode::Offset(offset) => Some(*offset),
            _ => None,
        }
    }

    /// Returns the cursor if using cursor-based pagination.
    pub fn cursor_value(&self) -> Option<&PageCursor> {
        match &self.mode {
            PaginationMode::Cursor(Some(cursor)) => Some(cursor),
            _ => None,
        }
    }
}

/// An opaque cursor for keyset pagination.
///
/// The cursor encodes the position in the result set in a way that is:
/// - Stable across concurrent modifications
/// - Efficient for the database to seek to
/// - Opaque to clients (they can't construct or modify it)
///
/// # Encoding
///
/// Cursors are base64-encoded JSON containing:
/// - Sort key values for the last returned item
/// - The resource ID for tie-breaking
/// - Version information for cursor compatibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageCursor {
    /// Cursor format version.
    version: u8,

    /// The sort key values at the cursor position.
    sort_values: Vec<CursorValue>,

    /// The resource ID at the cursor position (for tie-breaking).
    resource_id: String,

    /// The direction of pagination.
    direction: CursorDirection,
}

/// A value in the cursor for sorting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CursorValue {
    /// String value.
    String(String),
    /// Numeric value.
    Number(i64),
    /// Decimal value.
    Decimal(f64),
    /// Boolean value.
    Boolean(bool),
    /// Null value.
    Null,
}

/// Direction of cursor pagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CursorDirection {
    /// Fetching the next page (forward).
    #[default]
    Next,
    /// Fetching the previous page (backward).
    Previous,
}

impl PageCursor {
    /// Creates a new cursor at the given position.
    pub fn new(sort_values: Vec<CursorValue>, resource_id: impl Into<String>) -> Self {
        Self {
            version: 1,
            sort_values,
            resource_id: resource_id.into(),
            direction: CursorDirection::Next,
        }
    }

    /// Creates a cursor for the previous page.
    pub fn previous(sort_values: Vec<CursorValue>, resource_id: impl Into<String>) -> Self {
        Self {
            version: 1,
            sort_values,
            resource_id: resource_id.into(),
            direction: CursorDirection::Previous,
        }
    }

    /// Returns the sort values.
    pub fn sort_values(&self) -> &[CursorValue] {
        &self.sort_values
    }

    /// Returns the resource ID.
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    /// Returns the direction.
    pub fn direction(&self) -> CursorDirection {
        self.direction
    }

    /// Encodes the cursor to an opaque string.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        URL_SAFE_NO_PAD.encode(&json)
    }

    /// Decodes a cursor from an opaque string.
    pub fn decode(s: &str) -> Result<Self, SearchError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| SearchError::InvalidCursor {
                cursor: s.to_string(),
            })?;

        serde_json::from_slice(&bytes).map_err(|_| SearchError::InvalidCursor {
            cursor: s.to_string(),
        })
    }
}

impl From<&str> for CursorValue {
    fn from(s: &str) -> Self {
        CursorValue::String(s.to_string())
    }
}

impl From<String> for CursorValue {
    fn from(s: String) -> Self {
        CursorValue::String(s)
    }
}

impl From<i64> for CursorValue {
    fn from(n: i64) -> Self {
        CursorValue::Number(n)
    }
}

impl From<f64> for CursorValue {
    fn from(n: f64) -> Self {
        CursorValue::Decimal(n)
    }
}

impl From<bool> for CursorValue {
    fn from(b: bool) -> Self {
        CursorValue::Boolean(b)
    }
}

impl From<()> for CursorValue {
    fn from(_: ()) -> Self {
        CursorValue::Null
    }
}

/// Information about a page of results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    /// The cursor for the next page, if there is one.
    pub next_cursor: Option<String>,

    /// The cursor for the previous page, if there is one.
    pub previous_cursor: Option<String>,

    /// Total count of matching resources (if requested and available).
    pub total: Option<u64>,

    /// Whether there are more results after this page.
    pub has_next: bool,

    /// Whether there are results before this page.
    pub has_previous: bool,
}

impl PageInfo {
    /// Creates page info indicating no more pages.
    pub fn end() -> Self {
        Self {
            next_cursor: None,
            previous_cursor: None,
            total: None,
            has_next: false,
            has_previous: false,
        }
    }

    /// Creates page info with a next cursor.
    pub fn with_next(cursor: PageCursor) -> Self {
        Self {
            next_cursor: Some(cursor.encode()),
            previous_cursor: None,
            total: None,
            has_next: true,
            has_previous: false,
        }
    }

    /// Sets the total count.
    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }

    /// Sets the previous cursor.
    pub fn with_previous(mut self, cursor: PageCursor) -> Self {
        self.previous_cursor = Some(cursor.encode());
        self.has_previous = true;
        self
    }
}

/// A page of search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    /// The items in this page.
    pub items: Vec<T>,

    /// Pagination information.
    pub page_info: PageInfo,
}

impl<T> Page<T> {
    /// Creates a new page with the given items and page info.
    pub fn new(items: Vec<T>, page_info: PageInfo) -> Self {
        Self { items, page_info }
    }

    /// Creates an empty page.
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            page_info: PageInfo::end(),
        }
    }

    /// Returns true if this page has no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the number of items in this page.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Maps the items to a different type.
    pub fn map<U, F>(self, f: F) -> Page<U>
    where
        F: FnMut(T) -> U,
    {
        Page {
            items: self.items.into_iter().map(f).collect(),
            page_info: self.page_info,
        }
    }
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self::empty()
    }
}

/// A FHIR Bundle for search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchBundle {
    /// The bundle type (always "searchset").
    #[serde(rename = "type")]
    pub bundle_type: String,

    /// Total count of matching resources.
    pub total: Option<u64>,

    /// Links for pagination.
    pub link: Vec<BundleLink>,

    /// The bundle entries.
    pub entry: Vec<BundleEntry>,
}

/// A link in a FHIR Bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleLink {
    /// The relation type (self, next, previous, first, last).
    pub relation: String,

    /// The URL.
    pub url: String,
}

/// An entry in a FHIR Bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleEntry {
    /// The full URL of the resource.
    #[serde(rename = "fullUrl", skip_serializing_if = "Option::is_none")]
    pub full_url: Option<String>,

    /// The resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Value>,

    /// Search information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<BundleEntrySearch>,
}

/// Search information for a bundle entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleEntrySearch {
    /// How this entry matched the search (match, include, outcome).
    pub mode: SearchEntryMode,

    /// Search ranking score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

/// How a bundle entry matched the search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchEntryMode {
    /// This is a match to the search parameters.
    Match,
    /// This is included because of _include/_revinclude.
    Include,
    /// This is an OperationOutcome about the search.
    Outcome,
}

impl SearchBundle {
    /// Creates a new search bundle.
    pub fn new() -> Self {
        Self {
            bundle_type: "searchset".to_string(),
            total: None,
            link: Vec::new(),
            entry: Vec::new(),
        }
    }

    /// Sets the total count.
    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }

    /// Adds a link.
    pub fn with_link(mut self, relation: impl Into<String>, url: impl Into<String>) -> Self {
        self.link.push(BundleLink {
            relation: relation.into(),
            url: url.into(),
        });
        self
    }

    /// Adds an entry.
    pub fn with_entry(mut self, entry: BundleEntry) -> Self {
        self.entry.push(entry);
        self
    }

    /// Adds a self link.
    pub fn with_self_link(self, url: impl Into<String>) -> Self {
        self.with_link("self", url)
    }

    /// Adds a next link.
    pub fn with_next_link(self, url: impl Into<String>) -> Self {
        self.with_link("next", url)
    }

    /// Adds a previous link.
    pub fn with_previous_link(self, url: impl Into<String>) -> Self {
        self.with_link("previous", url)
    }
}

impl Default for SearchBundle {
    fn default() -> Self {
        Self::new()
    }
}

impl BundleEntry {
    /// Creates a new match entry.
    pub fn match_entry(full_url: impl Into<String>, resource: Value) -> Self {
        Self {
            full_url: Some(full_url.into()),
            resource: Some(resource),
            search: Some(BundleEntrySearch {
                mode: SearchEntryMode::Match,
                score: None,
            }),
        }
    }

    /// Sets the search relevance score (`entry.search.score`). A `None` score
    /// leaves the entry unchanged. Has no effect if the entry has no `search`
    /// component (e.g. an outcome entry).
    pub fn with_score(mut self, score: Option<f64>) -> Self {
        if let (Some(s), Some(search)) = (score, self.search.as_mut()) {
            search.score = Some(s);
        }
        self
    }

    /// Creates a new include entry.
    pub fn include_entry(full_url: impl Into<String>, resource: Value) -> Self {
        Self {
            full_url: Some(full_url.into()),
            resource: Some(resource),
            search: Some(BundleEntrySearch {
                mode: SearchEntryMode::Include,
                score: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pagination_default() {
        let pagination = Pagination::default();
        assert_eq!(pagination.count, 20);
        assert!(matches!(pagination.mode, PaginationMode::Cursor(None)));
    }

    #[test]
    fn test_pagination_offset() {
        let pagination = Pagination::offset(100);
        assert_eq!(pagination.offset_value(), Some(100));
    }

    #[test]
    fn test_cursor_encode_decode() {
        let cursor = PageCursor::new(
            vec![CursorValue::String("2024-01-01".to_string())],
            "patient-123",
        );

        let encoded = cursor.encode();
        let decoded = PageCursor::decode(&encoded).unwrap();

        assert_eq!(decoded.resource_id(), "patient-123");
        assert_eq!(decoded.direction(), CursorDirection::Next);
    }

    #[test]
    fn test_cursor_decode_invalid() {
        let result = PageCursor::decode("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn test_cursor_previous() {
        let cursor = PageCursor::previous(vec![CursorValue::Number(100)], "obs-456");
        assert_eq!(cursor.direction(), CursorDirection::Previous);
    }

    #[test]
    fn test_page_info_with_next() {
        let cursor = PageCursor::new(vec![], "id");
        let info = PageInfo::with_next(cursor);
        assert!(info.has_next);
        assert!(info.next_cursor.is_some());
    }

    #[test]
    fn test_page_map() {
        let page = Page::new(vec![1, 2, 3], PageInfo::end());

        let mapped = page.map(|x| x * 2);
        assert_eq!(mapped.items, vec![2, 4, 6]);
    }

    #[test]
    fn test_search_bundle_builder() {
        let bundle = SearchBundle::new()
            .with_total(100)
            .with_self_link("https://example.com/Patient?name=Smith")
            .with_next_link("https://example.com/Patient?name=Smith&_cursor=xxx");

        assert_eq!(bundle.total, Some(100));
        assert_eq!(bundle.link.len(), 2);
        assert_eq!(bundle.link[0].relation, "self");
        assert_eq!(bundle.link[1].relation, "next");
    }

    #[test]
    fn test_bundle_entry_match() {
        let entry = BundleEntry::match_entry(
            "https://example.com/Patient/123",
            serde_json::json!({"resourceType": "Patient"}),
        );

        assert_eq!(entry.search.as_ref().unwrap().mode, SearchEntryMode::Match);
    }

    #[test]
    fn test_cursor_value_conversions() {
        let s: CursorValue = "test".into();
        assert!(matches!(s, CursorValue::String(_)));

        let n: CursorValue = 42i64.into();
        assert!(matches!(n, CursorValue::Number(_)));

        let b: CursorValue = true.into();
        assert!(matches!(b, CursorValue::Boolean(_)));
    }
}
