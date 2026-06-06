//! Reference parameter SQL handler.

use crate::types::{SearchModifier, SearchValue};

use super::super::query_builder::{SqlFragment, SqlParam};

/// Handles reference parameter SQL generation.
pub struct ReferenceHandler;

impl ReferenceHandler {
    /// Builds SQL for a reference parameter value.
    ///
    /// Reference values can be:
    /// - `id` - local reference (just the id)
    /// - `Type/id` - relative reference
    /// - `url` - absolute URL reference
    ///
    /// Modifiers:
    /// - `:Type` - restrict to specific resource type (e.g., subject:Patient)
    /// - `:identifier` - search by identifier instead of reference
    pub fn build_sql(
        value: &SearchValue,
        modifier: Option<&SearchModifier>,
        param_offset: usize,
    ) -> SqlFragment {
        let param_num = param_offset + 1;
        let ref_value = &value.value;

        // Handle :identifier modifier
        if matches!(modifier, Some(SearchModifier::Identifier)) {
            return Self::build_identifier_condition(ref_value, param_num);
        }

        // Handle :contains modifier - case-insensitive substring match on the
        // stored reference string (e.g. "Patient/123" contains "23").
        if matches!(modifier, Some(SearchModifier::Contains)) {
            return SqlFragment::with_params(
                format!("value_reference LIKE '%' || ?{} || '%'", param_num),
                vec![SqlParam::string(ref_value)],
            );
        }

        // Handle :text (contains) and :code-text (starts-with) on the indexed
        // Reference.display text.
        if matches!(modifier, Some(SearchModifier::Text)) {
            return SqlFragment::with_params(
                format!(
                    "value_reference_display COLLATE NOCASE LIKE '%' || ?{} || '%'",
                    param_num
                ),
                vec![SqlParam::string(value.value.to_lowercase())],
            );
        }
        if matches!(modifier, Some(SearchModifier::CodeText)) {
            return SqlFragment::with_params(
                format!(
                    "value_reference_display COLLATE NOCASE LIKE ?{} || '%'",
                    param_num
                ),
                vec![SqlParam::string(value.value.to_lowercase())],
            );
        }

        // :below / :above - URL/path-prefix hierarchy on the reference value
        // (e.g. an absolute/canonical reference URL). Mirrors uri :below/:above
        // (two placeholders, two bound params). Note: canonical `|version`
        // comparison is not implemented.
        if matches!(modifier, Some(SearchModifier::Below)) {
            return SqlFragment::with_params(
                format!(
                    "(value_reference = ?{} OR value_reference LIKE ?{} || '/%')",
                    param_num,
                    param_num + 1
                ),
                vec![SqlParam::string(ref_value), SqlParam::string(ref_value)],
            );
        }
        if matches!(modifier, Some(SearchModifier::Above)) {
            return SqlFragment::with_params(
                format!(
                    "(?{} = value_reference OR ?{} LIKE value_reference || '/%')",
                    param_num,
                    param_num + 1
                ),
                vec![SqlParam::string(ref_value), SqlParam::string(ref_value)],
            );
        }

        // Handle :Type modifier (restrict to specific resource type)
        if let Some(SearchModifier::Type(type_name)) = modifier {
            // The reference must be to the specified type
            let expected_prefix = format!("{}/", type_name);

            if ref_value.contains('/') {
                // Value already has type - just match it
                SqlFragment::with_params(
                    format!("value_reference = ?{}", param_num),
                    vec![SqlParam::string(ref_value)],
                )
            } else {
                // Value is just an ID - prepend the type
                SqlFragment::with_params(
                    format!("value_reference = ?{}", param_num),
                    vec![SqlParam::string(format!(
                        "{}{}",
                        expected_prefix, ref_value
                    ))],
                )
            }
        } else {
            // No modifier - match the reference as given
            Self::build_reference_condition(ref_value, param_num)
        }
    }

    /// Builds a condition for a standard reference value.
    fn build_reference_condition(ref_value: &str, param_num: usize) -> SqlFragment {
        if ref_value.contains('/') {
            // Type/id or full URL - exact match
            SqlFragment::with_params(
                format!("value_reference = ?{}", param_num),
                vec![SqlParam::string(ref_value)],
            )
        } else {
            // Just an ID - match any reference ending with this ID
            // This handles cases where the stored reference might be "Patient/123" but
            // the search is just "123"
            SqlFragment::with_params(
                format!(
                    "(value_reference = ?{} OR value_reference LIKE '%/' || ?{})",
                    param_num,
                    param_num + 1
                ),
                vec![SqlParam::string(ref_value), SqlParam::string(ref_value)],
            )
        }
    }

    /// Builds a condition for the :identifier modifier.
    ///
    /// This searches for references where the target resource has a matching identifier.
    /// Requires a join or subquery on the identifier search index.
    fn build_identifier_condition(identifier_value: &str, param_num: usize) -> SqlFragment {
        // Parse the identifier value (system|value format)
        if let Some(pipe_pos) = identifier_value.find('|') {
            let system = &identifier_value[..pipe_pos];
            let value = &identifier_value[pipe_pos + 1..];

            if system.is_empty() {
                // |value - match value with no system
                SqlFragment::with_params(
                    format!(
                        "EXISTS (SELECT 1 FROM search_index si2 WHERE si2.resource_id = SUBSTR(value_reference, INSTR(value_reference, '/') + 1) AND si2.param_name = 'identifier' AND (si2.value_token_system IS NULL OR si2.value_token_system = '') AND si2.value_token_code = ?{})",
                        param_num
                    ),
                    vec![SqlParam::string(value)],
                )
            } else if value.is_empty() {
                // system| - match any value in system
                SqlFragment::with_params(
                    format!(
                        "EXISTS (SELECT 1 FROM search_index si2 WHERE si2.resource_id = SUBSTR(value_reference, INSTR(value_reference, '/') + 1) AND si2.param_name = 'identifier' AND si2.value_token_system = ?{})",
                        param_num
                    ),
                    vec![SqlParam::string(system)],
                )
            } else {
                // system|value - exact match
                SqlFragment::with_params(
                    format!(
                        "EXISTS (SELECT 1 FROM search_index si2 WHERE si2.resource_id = SUBSTR(value_reference, INSTR(value_reference, '/') + 1) AND si2.param_name = 'identifier' AND si2.value_token_system = ?{} AND si2.value_token_code = ?{})",
                        param_num,
                        param_num + 1
                    ),
                    vec![SqlParam::string(system), SqlParam::string(value)],
                )
            }
        } else {
            // Just a value - match any system
            SqlFragment::with_params(
                format!(
                    "EXISTS (SELECT 1 FROM search_index si2 WHERE si2.resource_id = SUBSTR(value_reference, INSTR(value_reference, '/') + 1) AND si2.param_name = 'identifier' AND si2.value_token_code = ?{})",
                    param_num
                ),
                vec![SqlParam::string(identifier_value)],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchPrefix;

    #[test]
    fn test_reference_with_type() {
        let value = SearchValue::new(SearchPrefix::Eq, "Patient/123");
        let frag = ReferenceHandler::build_sql(&value, None, 0);

        assert!(frag.sql.contains("value_reference = ?1"));
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn test_reference_id_only() {
        let value = SearchValue::new(SearchPrefix::Eq, "123");
        let frag = ReferenceHandler::build_sql(&value, None, 0);

        // Should match both exact and with any type prefix
        assert!(frag.sql.contains("OR"));
        assert!(frag.sql.contains("LIKE"));
    }

    #[test]
    fn test_reference_type_modifier() {
        let value = SearchValue::new(SearchPrefix::Eq, "123");
        let frag = ReferenceHandler::build_sql(
            &value,
            Some(&SearchModifier::Type("Patient".to_string())),
            0,
        );

        assert!(frag.sql.contains("value_reference = ?1"));
        // The param should be "Patient/123"
    }

    #[test]
    fn test_reference_contains_modifier() {
        let value = SearchValue::new(SearchPrefix::Eq, "123");
        let frag = ReferenceHandler::build_sql(&value, Some(&SearchModifier::Contains), 0);

        assert!(frag.sql.contains("value_reference LIKE '%' ||"));
        assert!(frag.sql.contains("|| '%'"));
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn test_reference_text_modifier() {
        let value = SearchValue::new(SearchPrefix::Eq, "John");
        let frag = ReferenceHandler::build_sql(&value, Some(&SearchModifier::Text), 0);
        assert!(frag.sql.contains("value_reference_display"));
        assert!(frag.sql.contains("'%' || ?1 || '%'"));
    }

    #[test]
    fn test_reference_code_text_modifier() {
        let value = SearchValue::new(SearchPrefix::Eq, "John");
        let frag = ReferenceHandler::build_sql(&value, Some(&SearchModifier::CodeText), 0);
        assert!(frag.sql.contains("value_reference_display"));
        // starts-with: no leading '%'
        assert!(frag.sql.contains("LIKE ?1 || '%'"));
    }

    #[test]
    fn test_reference_below_modifier() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://x.org/Questionnaire/q");
        let frag = ReferenceHandler::build_sql(&value, Some(&SearchModifier::Below), 0);
        assert!(frag.sql.contains("value_reference = ?1"));
        assert!(frag.sql.contains("LIKE ?2 || '/%'"));
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn test_reference_above_modifier() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://x.org/Questionnaire/q");
        let frag = ReferenceHandler::build_sql(&value, Some(&SearchModifier::Above), 0);
        assert!(frag.sql.contains("?1 = value_reference"));
        assert!(frag.sql.contains("?2 LIKE value_reference || '/%'"));
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn test_reference_identifier_modifier() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://example.org|12345");
        let frag = ReferenceHandler::build_sql(&value, Some(&SearchModifier::Identifier), 0);

        assert!(frag.sql.contains("EXISTS"));
        assert!(frag.sql.contains("identifier"));
    }
}
