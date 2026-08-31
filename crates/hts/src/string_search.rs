//! Backend-neutral matching for the FHIR `string` search parameter type.
//!
//! SQLite and PostgreSQL do not provide identical Unicode case and accent
//! handling. HTS therefore normalizes metadata in Rust and applies pagination
//! after matching. This module intentionally does not normalize punctuation or
//! non-significant whitespace, which FHIR describes as recommended behavior.

use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use crate::types::ResourceSearchQuery;

/// FHIR `string` matching modes supported by HTS.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FhirStringSearchMode {
    /// Case- and accent-insensitive prefix matching.
    #[default]
    Prefix,
    /// Case- and accent-insensitive substring matching.
    Contains,
    /// Equality of the decoded Unicode scalar-value sequence.
    Exact,
}

/// A compiled search value. The normalized query is computed only once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FhirStringSearch {
    value: String,
    normalized: String,
    mode: FhirStringSearchMode,
}

impl FhirStringSearch {
    /// Compile a non-empty effective search value.
    ///
    /// Values that are empty before or after normalization are ignored. In
    /// particular, a value containing only combining marks must not turn a
    /// `contains` search into a predicate that matches every resource.
    pub(crate) fn new(value: &str, mode: FhirStringSearchMode) -> Option<Self> {
        if value.is_empty() {
            return None;
        }
        let normalized = normalize(value);
        if normalized.is_empty() {
            return None;
        }
        Some(Self {
            value: value.to_owned(),
            normalized,
            mode,
        })
    }

    /// Whether a present candidate satisfies this search value.
    pub(crate) fn matches(&self, candidate: &str) -> bool {
        match self.mode {
            FhirStringSearchMode::Prefix => normalize(candidate).starts_with(&self.normalized),
            FhirStringSearchMode::Contains => normalize(candidate).contains(&self.normalized),
            FhirStringSearchMode::Exact => candidate == self.value,
        }
    }
}

/// The compiled name/title predicates for one resource search.
pub(crate) struct ResourceStringSearch {
    name: Option<FhirStringSearch>,
    title: Option<FhirStringSearch>,
}

impl ResourceStringSearch {
    /// Compile each query value exactly once.
    pub(crate) fn new(query: &ResourceSearchQuery) -> Self {
        Self {
            name: query
                .name
                .as_deref()
                .and_then(|value| FhirStringSearch::new(value, query.name_mode)),
            title: query
                .title
                .as_deref()
                .and_then(|value| FhirStringSearch::new(value, query.title_mode)),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.name.is_none() && self.title.is_none()
    }
}

/// Metadata needed to apply name/title filters before pagination.
#[derive(Clone, Debug)]
pub(crate) struct ResourceSearchRow {
    pub id: String,
    pub url: String,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: String,
}

/// Apply FHIR string filters and then `_offset`/`_count`.
pub(crate) fn filter_rows(
    rows: impl IntoIterator<Item = ResourceSearchRow>,
    query: &ResourceSearchQuery,
    search: &ResourceStringSearch,
) -> Vec<ResourceSearchRow> {
    let offset = query.offset.unwrap_or(0) as usize;
    let count = query.count.unwrap_or(20) as usize;

    rows.into_iter()
        .filter(|row| {
            search.name.as_ref().is_none_or(|search| {
                row.name
                    .as_deref()
                    .is_some_and(|candidate| search.matches(candidate))
            }) && search.title.as_ref().is_none_or(|search| {
                row.title
                    .as_deref()
                    .is_some_and(|candidate| search.matches(candidate))
            })
        })
        .skip(offset)
        .take(count)
        .collect()
}

/// Normalize case and combining marks consistently across storage backends.
fn normalize(value: &str) -> String {
    value
        .nfd()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .nfd()
        .filter(|character| !is_combining_mark(*character))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_and_contains_are_case_and_accent_insensitive() {
        let prefix = FhirStringSearch::new("cafe", FhirStringSearchMode::Prefix).unwrap();
        assert!(prefix.matches("Café terminology"));
        assert!(prefix.matches("Cafe\u{301} terminology"));
        assert!(!prefix.matches("Terminology café"));

        let contains = FhirStringSearch::new("CAFE", FhirStringSearchMode::Contains).unwrap();
        assert!(contains.matches("Terminology café"));
    }

    #[test]
    fn normalization_handles_non_ascii_case_expansion() {
        let search = FhirStringSearch::new("STRASSE", FhirStringSearchMode::Prefix).unwrap();
        assert!(search.matches("Straße terminology"));
    }

    #[test]
    fn exact_preserves_case_accents_and_unicode_sequence() {
        let search = FhirStringSearch::new("Café", FhirStringSearchMode::Exact).unwrap();
        assert!(search.matches("Café"));
        assert!(!search.matches("café"));
        assert!(!search.matches("Cafe\u{301}"));
        assert!(!search.matches("Cafe"));
    }

    #[test]
    fn empty_and_normalization_empty_values_are_ignored() {
        assert!(FhirStringSearch::new("", FhirStringSearchMode::Prefix).is_none());
        assert!(FhirStringSearch::new("\u{301}", FhirStringSearchMode::Contains).is_none());
    }

    #[test]
    fn missing_fields_do_not_match_and_pagination_follows_filtering() {
        let query = ResourceSearchQuery {
            name: Some("match".to_owned()),
            count: Some(1),
            offset: Some(1),
            ..ResourceSearchQuery::default()
        };
        let rows = [None, Some("MatchOne"), Some("other"), Some("matchTwo")]
            .into_iter()
            .enumerate()
            .map(|(index, name)| ResourceSearchRow {
                id: index.to_string(),
                url: format!("http://example.org/{index}"),
                version: None,
                name: name.map(str::to_owned),
                title: None,
                status: "active".to_owned(),
            });

        let search = ResourceStringSearch::new(&query);
        let selected = filter_rows(rows, &query, &search);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name.as_deref(), Some("matchTwo"));
    }
}
