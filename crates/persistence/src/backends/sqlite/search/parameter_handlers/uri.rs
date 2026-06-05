//! URI parameter SQL handler.

use crate::types::{SearchModifier, SearchValue};

use super::super::query_builder::{SqlFragment, SqlParam};

/// Handles URI parameter SQL generation.
pub struct UriHandler;

impl UriHandler {
    /// Builds SQL for a URI parameter value.
    ///
    /// Default behavior is exact match.
    ///
    /// Modifiers:
    /// - `:below` - matches URIs that start with the given value
    /// - `:above` - matches URIs that the given value starts with
    pub fn build_sql(
        value: &SearchValue,
        modifier: Option<&SearchModifier>,
        param_offset: usize,
    ) -> SqlFragment {
        let param_num = param_offset + 1;
        let uri_value = &value.value;

        match modifier {
            Some(SearchModifier::Contains) => {
                // :contains - case-insensitive substring match on the URI.
                SqlFragment::with_params(
                    format!("value_uri LIKE '%' || ?{} || '%'", param_num),
                    vec![SqlParam::string(uri_value)],
                )
            }
            Some(SearchModifier::Below) => {
                // Below: match the URI or any URI that starts with it
                // For "http://example.org", matches "http://example.org" and "http://example.org/foo"
                SqlFragment::with_params(
                    format!(
                        "(value_uri = ?{} OR value_uri LIKE ?{} || '/%')",
                        param_num,
                        param_num + 1
                    ),
                    vec![SqlParam::string(uri_value), SqlParam::string(uri_value)],
                )
            }
            Some(SearchModifier::Above) => {
                // Above: match URIs that the given value starts with
                // For "http://example.org/foo/bar", matches "http://example.org", "http://example.org/foo", etc.
                SqlFragment::with_params(
                    format!(
                        "(?{} = value_uri OR ?{} LIKE value_uri || '/%')",
                        param_num,
                        param_num + 1
                    ),
                    vec![SqlParam::string(uri_value), SqlParam::string(uri_value)],
                )
            }
            _ => {
                // Default: exact match
                SqlFragment::with_params(
                    format!("value_uri = ?{}", param_num),
                    vec![SqlParam::string(uri_value)],
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchPrefix;

    #[test]
    fn test_uri_exact() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://example.org/fhir/ValueSet/123");
        let frag = UriHandler::build_sql(&value, None, 0);

        assert!(frag.sql.contains("value_uri = ?1"));
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn test_uri_contains() {
        let value = SearchValue::new(SearchPrefix::Eq, "example.org");
        let frag = UriHandler::build_sql(&value, Some(&SearchModifier::Contains), 0);

        assert!(frag.sql.contains("LIKE '%' ||"));
        assert!(frag.sql.contains("|| '%'"));
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn test_uri_below() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://example.org");
        let frag = UriHandler::build_sql(&value, Some(&SearchModifier::Below), 0);

        assert!(frag.sql.contains("OR"));
        assert!(frag.sql.contains("LIKE"));
        assert!(frag.sql.contains("|| '/%'"));
    }

    #[test]
    fn test_uri_above() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://example.org/fhir/ValueSet/123");
        let frag = UriHandler::build_sql(&value, Some(&SearchModifier::Above), 0);

        assert!(frag.sql.contains("OR"));
        assert!(frag.sql.contains("LIKE"));
    }
}
