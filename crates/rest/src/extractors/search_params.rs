//! Search parameters extractor.
//!
//! Extracts and parses FHIR search parameters from query strings.

use axum::{extract::FromRequestParts, http::request::Parts};
use std::collections::HashMap;
use std::convert::Infallible;

use super::query_pairs::{collect_multi, parse_query_pairs};

/// Axum extractor for FHIR search parameters.
///
/// Parses query string parameters into a structured format for search
/// operations.
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::extractors::SearchParams;
///
/// async fn search_handler(params: SearchParams) {
///     for (name, value) in params.iter() {
///         println!("{} = {}", name, value);
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct SearchParams {
    /// Raw query parameters in request order, with repeated keys preserved.
    pairs: Vec<(String, String)>,

    /// Page size (_count).
    count: Option<usize>,

    /// Page offset (_offset).
    offset: Option<usize>,

    /// Sort parameters (_sort).
    sort: Option<Vec<SortParam>>,

    /// Include parameters (_include).
    include: Vec<String>,

    /// Reverse include parameters (_revinclude).
    revinclude: Vec<String>,

    /// Summary mode (_summary).
    summary: Option<String>,

    /// Elements to include (_elements).
    elements: Option<Vec<String>>,

    /// Total mode (_total).
    total: Option<String>,
}

/// A parsed sort parameter.
#[derive(Debug, Clone)]
pub struct SortParam {
    /// The field to sort by.
    pub field: String,
    /// Sort direction (ascending if true).
    pub ascending: bool,
}

impl SearchParams {
    /// Creates empty search params.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates search params from a HashMap.
    ///
    /// Convenience for callers that already hold a single-valued map (e.g.
    /// conditional-operation `If-None-Exist` queries). Prefer [`from_pairs`] for
    /// HTTP search requests so repeated parameters are preserved.
    ///
    /// [`from_pairs`]: SearchParams::from_pairs
    pub fn from_map(params: HashMap<String, String>) -> Self {
        Self::from_pairs(params.into_iter().collect())
    }

    /// Creates search params from ordered key/value pairs, preserving every
    /// occurrence of a repeated key (FHIR AND semantics).
    pub fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        let last = |key: &str| {
            pairs
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };

        let mut result = Self::default();

        // Single-valued control parameters: last occurrence wins.
        result.count = last("_count").and_then(|v| v.parse().ok());
        result.offset = last("_offset").and_then(|v| v.parse().ok());
        result.summary = last("_summary");
        result.total = last("_total");

        // Multi-valued: collected across all occurrences and comma-splits.
        let sort_specs = collect_multi(&pairs, "_sort");
        if !sort_specs.is_empty() {
            result.sort = Some(sort_specs.iter().map(|s| parse_one_sort(s)).collect());
        }
        result.include = collect_multi(&pairs, "_include");
        result.revinclude = collect_multi(&pairs, "_revinclude");
        let elements = collect_multi(&pairs, "_elements");
        if !elements.is_empty() {
            result.elements = Some(elements);
        }

        result.pairs = pairs;
        result
    }

    /// Returns an iterator over all parameters (repeated keys included).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.pairs.iter().map(|(k, v)| (k, v))
    }

    /// Returns an iterator over search parameters (excluding result format/pagination params).
    ///
    /// Excludes: _count, _offset, _cursor, _sort, _total, _summary, _elements,
    ///           _include, _revinclude, _contained, _containedType, _format
    ///
    /// Includes: _id, _lastUpdated, _tag, _profile, _security, _source, _has,
    ///           _list, _text, _content, _filter, _query, _type
    pub fn search_params(&self) -> impl Iterator<Item = (&String, &String)> {
        const EXCLUDED_PARAMS: &[&str] = &[
            "_count",
            "_offset",
            "_cursor",
            "_sort",
            "_total",
            "_summary",
            "_elements",
            "_include",
            "_revinclude",
            "_contained",
            "_containedType",
            "_format",
            "_pretty",
        ];
        self.pairs
            .iter()
            .map(|(k, v)| (k, v))
            .filter(|(k, _)| !EXCLUDED_PARAMS.contains(&k.as_str()))
    }

    /// Returns the page size (_count).
    pub fn count(&self) -> Option<usize> {
        self.count
    }

    /// Returns the page offset (_offset).
    pub fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// Returns the sort parameters.
    pub fn sort(&self) -> Option<&[SortParam]> {
        self.sort.as_deref()
    }

    /// Returns the include parameters.
    pub fn include(&self) -> &[String] {
        &self.include
    }

    /// Returns the reverse include parameters.
    pub fn revinclude(&self) -> &[String] {
        &self.revinclude
    }

    /// Returns the summary mode.
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    /// Returns the elements to include.
    pub fn elements(&self) -> Option<&[String]> {
        self.elements.as_deref()
    }

    /// Returns the total mode.
    pub fn total(&self) -> Option<&str> {
        self.total.as_deref()
    }

    /// Returns the last value for `name`, if present.
    pub fn get(&self, name: &str) -> Option<&String> {
        self.pairs
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }

    /// Checks if a parameter is present.
    pub fn contains(&self, name: &str) -> bool {
        self.pairs.iter().any(|(k, _)| k == name)
    }
}

/// Parses a single sort token into a structured param (`-field` = descending).
fn parse_one_sort(s: &str) -> SortParam {
    let s = s.trim();
    if let Some(field) = s.strip_prefix('-') {
        SortParam {
            field: field.to_string(),
            ascending: false,
        }
    } else {
        SortParam {
            field: s.to_string(),
            ascending: true,
        }
    }
}

impl<S> FromRequestParts<S> for SearchParams
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(SearchParams::from_pairs(parse_query_pairs(
            parts.uri.query(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_map() {
        let mut params = HashMap::new();
        params.insert("_count".to_string(), "10".to_string());
        params.insert("_offset".to_string(), "20".to_string());
        params.insert("name".to_string(), "Smith".to_string());

        let search = SearchParams::from_map(params);

        assert_eq!(search.count(), Some(10));
        assert_eq!(search.offset(), Some(20));
        assert_eq!(search.get("name"), Some(&"Smith".to_string()));
    }

    #[test]
    fn test_sort_params() {
        let mut params = HashMap::new();
        params.insert("_sort".to_string(), "name,-date".to_string());

        let search = SearchParams::from_map(params);
        let sort = search.sort().unwrap();

        assert_eq!(sort.len(), 2);
        assert_eq!(sort[0].field, "name");
        assert!(sort[0].ascending);
        assert_eq!(sort[1].field, "date");
        assert!(!sort[1].ascending);
    }

    #[test]
    fn test_search_params_filter() {
        let mut params = HashMap::new();
        params.insert("_count".to_string(), "10".to_string());
        params.insert("name".to_string(), "Smith".to_string());
        params.insert("active".to_string(), "true".to_string());

        let search = SearchParams::from_map(params);
        let search_only: Vec<_> = search.search_params().collect();

        assert_eq!(search_only.len(), 2);
    }
}
