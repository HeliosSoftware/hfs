//! Token parameter SQL handler.

use crate::types::{SearchModifier, SearchValue};

use super::super::query_builder::{SqlFragment, SqlParam};

/// Handles token parameter SQL generation.
pub struct TokenHandler;

impl TokenHandler {
    /// Builds SQL for a token parameter value.
    ///
    /// Token values can be:
    /// - `code` - matches any system
    /// - `system|code` - matches specific system and code
    /// - `|code` - matches code with no system (empty or null)
    /// - `system|` - matches any code in system
    ///
    /// With `:of-type` modifier (for identifiers):
    /// - `type-system|type-code|identifier-value` - matches identifier by type and value
    pub fn build_sql(
        value: &SearchValue,
        modifier: Option<&SearchModifier>,
        param_offset: usize,
    ) -> SqlFragment {
        let param_num = param_offset + 1;

        // Handle :not modifier
        if matches!(modifier, Some(SearchModifier::Not)) {
            let inner = Self::build_sql(value, None, param_offset);
            return SqlFragment::with_params(format!("NOT ({})", inner.sql), inner.params);
        }

        // Handle :text modifier - search on display text (Coding.display, CodeableConcept.text)
        if matches!(modifier, Some(SearchModifier::Text)) {
            // Search on the display text column for human-readable text matching
            return SqlFragment::with_params(
                format!(
                    "value_token_display COLLATE NOCASE LIKE '%' || ?{} || '%'",
                    param_num
                ),
                vec![SqlParam::string(value.value.to_lowercase())],
            );
        }

        // Handle :code-text modifier - case-insensitive starts-with match on the
        // display text (Coding.display, CodeableConcept.text). Distinct from
        // :text, which is a substring/contains match.
        if matches!(modifier, Some(SearchModifier::CodeText)) {
            return SqlFragment::with_params(
                format!(
                    "value_token_display COLLATE NOCASE LIKE ?{} || '%'",
                    param_num
                ),
                vec![SqlParam::string(value.value.to_lowercase())],
            );
        }

        // Handle :text-advanced modifier - FTS5-based advanced text search
        // Supports boolean operators (AND, OR, NOT), phrase matching, prefix search, and NEAR
        if matches!(modifier, Some(SearchModifier::TextAdvanced)) {
            return Self::build_text_advanced_sql(&value.value, param_offset);
        }

        // Handle :code-only modifier
        if matches!(modifier, Some(SearchModifier::CodeOnly)) {
            return SqlFragment::with_params(
                format!("value_token_code = ?{}", param_num),
                vec![SqlParam::string(&value.value)],
            );
        }

        // Handle :of-type modifier (for identifier searches)
        if matches!(modifier, Some(SearchModifier::OfType)) {
            return Self::build_of_type_sql(&value.value, param_offset);
        }

        // Parse the token value
        let token_value = &value.value;

        if let Some(pipe_pos) = token_value.find('|') {
            let system = &token_value[..pipe_pos];
            let code = &token_value[pipe_pos + 1..];

            if system.is_empty() {
                // |code - match code with no system
                SqlFragment::with_params(
                    format!(
                        "(value_token_system IS NULL OR value_token_system = '') AND value_token_code = ?{}",
                        param_num
                    ),
                    vec![SqlParam::string(code)],
                )
            } else if code.is_empty() {
                // system| - match any code in system
                SqlFragment::with_params(
                    format!("value_token_system = ?{}", param_num),
                    vec![SqlParam::string(system)],
                )
            } else {
                // system|code - exact match
                SqlFragment::with_params(
                    format!(
                        "value_token_system = ?{} AND value_token_code = ?{}",
                        param_num,
                        param_num + 1
                    ),
                    vec![SqlParam::string(system), SqlParam::string(code)],
                )
            }
        } else {
            // code only - match any system
            SqlFragment::with_params(
                format!("value_token_code = ?{}", param_num),
                vec![SqlParam::string(token_value)],
            )
        }
    }

    /// Builds SQL for the `:text-advanced` modifier using FTS5.
    ///
    /// The `:text-advanced` modifier (FHIR v6.0.0) provides advanced full-text
    /// search capabilities including:
    /// - Porter stemming (e.g., "running" matches "run")
    /// - Boolean operators (AND, OR, NOT)
    /// - Phrase matching ("heart attack")
    /// - Prefix matching (cardio*)
    /// - Proximity search (NEAR operator)
    ///
    /// This searches on the token display text (Coding.display, CodeableConcept.text)
    /// that has been indexed in the FTS5 virtual table.
    fn build_text_advanced_sql(query: &str, param_offset: usize) -> SqlFragment {
        use super::super::fts::Fts5Search;

        // Use FTS5 for advanced matching on token display text
        // The search_index_fts now includes value_token_display
        Fts5Search::build_advanced_query(query, param_offset + 1)
    }

    /// Builds SQL for the `:of-type` modifier used with identifier parameters.
    ///
    /// The `:of-type` modifier allows searching by both the type and value of an identifier.
    /// Format: `type-system|type-code|identifier-value`
    ///
    /// For example:
    /// - `Patient?identifier:of-type=http://terminology.hl7.org/CodeSystem/v2-0203|MR|12345`
    ///   matches patients with a Medical Record Number identifier with value "12345".
    ///
    /// This implementation uses the dedicated type columns:
    /// - `value_identifier_type_system` - stores identifier.type.coding[0].system
    /// - `value_identifier_type_code` - stores identifier.type.coding[0].code
    fn build_of_type_sql(value: &str, param_offset: usize) -> SqlFragment {
        let mut param_num = param_offset + 1;

        // Parse the three-part format: type-system|type-code|identifier-value
        let parts: Vec<&str> = value.splitn(3, '|').collect();

        match parts.len() {
            3 => {
                let type_system = parts[0];
                let type_code = parts[1];
                let identifier_value = parts[2];

                let mut conditions = Vec::new();
                let mut params = Vec::new();

                // Always match on identifier value (required)
                if !identifier_value.is_empty() {
                    conditions.push(format!("value_token_code = ?{}", param_num));
                    params.push(SqlParam::string(identifier_value));
                    param_num += 1;
                }

                // Match on type system if provided
                if !type_system.is_empty() {
                    conditions.push(format!("value_identifier_type_system = ?{}", param_num));
                    params.push(SqlParam::string(type_system));
                    param_num += 1;
                }

                // Match on type code if provided
                if !type_code.is_empty() {
                    conditions.push(format!("value_identifier_type_code = ?{}", param_num));
                    params.push(SqlParam::string(type_code));
                }

                if conditions.is_empty() {
                    // No valid conditions
                    SqlFragment::new("1 = 0")
                } else {
                    SqlFragment::with_params(conditions.join(" AND "), params)
                }
            }
            2 => {
                // type-code|value format (no type-system)
                let type_code = parts[0];
                let identifier_value = parts[1];

                let mut conditions = Vec::new();
                let mut params = Vec::new();

                if !identifier_value.is_empty() {
                    conditions.push(format!("value_token_code = ?{}", param_num));
                    params.push(SqlParam::string(identifier_value));
                    param_num += 1;
                }

                if !type_code.is_empty() {
                    conditions.push(format!("value_identifier_type_code = ?{}", param_num));
                    params.push(SqlParam::string(type_code));
                }

                if conditions.is_empty() {
                    SqlFragment::new("1 = 0")
                } else {
                    SqlFragment::with_params(conditions.join(" AND "), params)
                }
            }
            1 => {
                // Just value (no type info)
                SqlFragment::with_params(
                    format!("value_token_code = ?{}", param_num),
                    vec![SqlParam::string(value)],
                )
            }
            _ => {
                // Empty or invalid format - return empty match
                SqlFragment::new("1 = 0")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchPrefix;

    #[test]
    fn test_token_code_only() {
        let value = SearchValue::new(SearchPrefix::Eq, "12345");
        let frag = TokenHandler::build_sql(&value, None, 0);

        assert!(frag.sql.contains("value_token_code = ?1"));
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn test_token_system_and_code() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://loinc.org|12345-6");
        let frag = TokenHandler::build_sql(&value, None, 0);

        assert!(frag.sql.contains("value_token_system = ?1"));
        assert!(frag.sql.contains("value_token_code = ?2"));
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn test_token_no_system() {
        let value = SearchValue::new(SearchPrefix::Eq, "|12345");
        let frag = TokenHandler::build_sql(&value, None, 0);

        assert!(frag.sql.contains("IS NULL OR"));
        assert!(frag.sql.contains("value_token_code = ?1"));
    }

    #[test]
    fn test_token_system_only() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://loinc.org|");
        let frag = TokenHandler::build_sql(&value, None, 0);

        assert!(frag.sql.contains("value_token_system = ?1"));
        assert!(!frag.sql.contains("value_token_code"));
    }

    #[test]
    fn test_token_not_modifier() {
        let value = SearchValue::new(SearchPrefix::Eq, "12345");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::Not), 0);

        assert!(frag.sql.starts_with("NOT ("));
    }

    #[test]
    fn test_code_text_starts_with_display() {
        // :code-text is a case-insensitive starts-with match on the display.
        let value = SearchValue::new(SearchPrefix::Eq, "Heart");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::CodeText), 0);

        assert!(
            frag.sql
                .contains("value_token_display COLLATE NOCASE LIKE ?1 || '%'")
        );
        // No leading '%': starts-with, not contains.
        assert!(!frag.sql.contains("'%' || ?1"));
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn test_of_type_full_format() {
        // Full format: type-system|type-code|identifier-value
        let value = SearchValue::new(
            SearchPrefix::Eq,
            "http://terminology.hl7.org/CodeSystem/v2-0203|MR|12345",
        );
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::OfType), 0);

        // Should match identifier value, type system, and type code
        assert!(frag.sql.contains("value_token_code = ?1"));
        assert!(frag.sql.contains("value_identifier_type_system = ?2"));
        assert!(frag.sql.contains("value_identifier_type_code = ?3"));
        assert_eq!(frag.params.len(), 3);
    }

    #[test]
    fn test_of_type_no_system() {
        // Format without type-system: |type-code|value
        let value = SearchValue::new(SearchPrefix::Eq, "|MR|12345");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::OfType), 0);

        // Should match identifier value and type code (no type system)
        assert!(frag.sql.contains("value_token_code = ?1"));
        assert!(frag.sql.contains("value_identifier_type_code = ?2"));
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn test_of_type_value_only() {
        // Format with just type-code|value
        let value = SearchValue::new(SearchPrefix::Eq, "MR|12345");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::OfType), 0);

        // Should match identifier value and type code
        assert!(frag.sql.contains("value_token_code = ?1"));
        assert!(frag.sql.contains("value_identifier_type_code = ?2"));
        assert_eq!(frag.params.len(), 2);
        // Verify parameters
        if let SqlParam::String(s) = &frag.params[0] {
            assert_eq!(s, "12345");
        } else {
            panic!("Expected string parameter for identifier value");
        }
        if let SqlParam::String(s) = &frag.params[1] {
            assert_eq!(s, "MR");
        } else {
            panic!("Expected string parameter for type code");
        }
    }

    // ============================================================================
    // :text-advanced Modifier Tests
    // ============================================================================

    #[test]
    fn test_text_advanced_simple() {
        let value = SearchValue::new(SearchPrefix::Eq, "headache");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::TextAdvanced), 0);

        // Should use FTS5 MATCH
        assert!(frag.sql.contains("search_index_fts"));
        assert!(frag.sql.contains("MATCH"));
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn test_text_advanced_boolean_or() {
        let value = SearchValue::new(SearchPrefix::Eq, "headache OR migraine");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::TextAdvanced), 0);

        assert!(frag.sql.contains("MATCH"));
        // The query param should contain OR
        if let SqlParam::String(s) = &frag.params[0] {
            assert!(s.contains("OR"), "Query should contain OR: {}", s);
        }
    }

    #[test]
    fn test_text_advanced_phrase() {
        let value = SearchValue::new(SearchPrefix::Eq, "\"heart failure\"");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::TextAdvanced), 0);

        assert!(frag.sql.contains("MATCH"));
        // The query param should be a quoted phrase
        if let SqlParam::String(s) = &frag.params[0] {
            assert!(
                s.contains("\"heart failure\""),
                "Query should contain phrase: {}",
                s
            );
        }
    }

    #[test]
    fn test_text_advanced_prefix() {
        let value = SearchValue::new(SearchPrefix::Eq, "cardio*");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::TextAdvanced), 0);

        assert!(frag.sql.contains("MATCH"));
        // The query param should contain prefix wildcard
        if let SqlParam::String(s) = &frag.params[0] {
            assert!(s.contains("cardio*"), "Query should contain prefix: {}", s);
        }
    }

    #[test]
    fn test_text_advanced_not() {
        let value = SearchValue::new(SearchPrefix::Eq, "-surgery");
        let frag = TokenHandler::build_sql(&value, Some(&SearchModifier::TextAdvanced), 0);

        assert!(frag.sql.contains("MATCH"));
        // The query param should contain NOT
        if let SqlParam::String(s) = &frag.params[0] {
            assert!(s.contains("NOT"), "Query should contain NOT: {}", s);
        }
    }
}
