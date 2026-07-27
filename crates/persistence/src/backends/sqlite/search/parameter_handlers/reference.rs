//! Reference parameter SQL handler.

use crate::types::{SearchModifier, SearchValue, strip_reference_version};

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
            // Normalize to a `Type/id` reference, then match version-agnostically.
            let full = if ref_value.contains('/') {
                ref_value.to_string()
            } else {
                format!("{}/{}", type_name, ref_value)
            };
            Self::build_reference_condition(&full, param_num)
        } else {
            // No modifier - match the reference as given
            Self::build_reference_condition(ref_value, param_num)
        }
    }

    /// Builds a condition for a standard reference value.
    ///
    /// Reference matching is **version-agnostic** per the FHIR spec: a versioned
    /// reference (`Patient/123/_history/2`) is normalized by stripping the
    /// version, and a stored versioned reference still matches an unversioned
    /// search (and vice versa).
    fn build_reference_condition(ref_value: &str, param_num: usize) -> SqlFragment {
        let base = strip_reference_version(ref_value);

        if base.contains('/') {
            // Type/id (or absolute URL): match the base reference, or the same
            // reference carrying any `_history` version.
            SqlFragment::with_params(
                format!(
                    "(value_reference = ?{} OR value_reference LIKE ?{} || '/_history/%')",
                    param_num,
                    param_num + 1
                ),
                vec![SqlParam::string(base), SqlParam::string(base)],
            )
        } else {
            // Just an ID - match any reference ending with this ID, with or
            // without a trailing `_history` version.
            SqlFragment::with_params(
                format!(
                    "(value_reference = ?{} \
                      OR value_reference LIKE '%/' || ?{} \
                      OR value_reference LIKE '%/' || ?{} || '/_history/%')",
                    param_num,
                    param_num + 1,
                    param_num + 2
                ),
                vec![
                    SqlParam::string(base),
                    SqlParam::string(base),
                    SqlParam::string(base),
                ],
            )
        }
    }

    /// Builds a condition for the :identifier modifier.
    ///
    /// This searches for references where the target resource has a matching identifier.
    /// Requires a join or subquery on the identifier search index.
    ///
    /// # Shape: uncorrelated `IN`, not a correlated `EXISTS`
    ///
    /// The `si2` sub-select yields the target's `Type/id`, and the *enclosing*
    /// row's `value_reference` is compared against that set from the outer
    /// scope. It deliberately does **not** correlate into the sub-select: this
    /// condition is embedded in a `SELECT … FROM search_index` and `si2` is
    /// itself `search_index`, so an unqualified `value_reference` written
    /// *inside* the sub-select binds to `si2`'s own column (innermost scope
    /// wins), not to the referencing row's. `si2` rows are `identifier` rows,
    /// whose `value_reference` is NULL, so the correlated form was always
    /// false and `:identifier` matched nothing at all.
    ///
    /// Matching is version-agnostic, like [`Self::build_reference_condition`]:
    /// a stored `Patient/p1/_history/2` is normalized to `Patient/p1` before
    /// comparison. Absolute-URL references are not resolved (unchanged).
    ///
    /// # Tenant scoping
    ///
    /// Every `si2` sub-select carries `si2.tenant_id = ?1`. It is load-bearing,
    /// not defensive: without it the sub-select yields *any* tenant's target,
    /// so `subject:identifier=…` returns this tenant's rows on the strength of
    /// another tenant's identifiers — a cross-tenant match oracle, and a
    /// violation of the `tenant_id` discriminator that
    /// [`BackendCapability::SharedSchema`](crate::core::BackendCapability::SharedSchema)
    /// promises every query carries. The Postgres equivalent has always scoped
    /// both levels (`postgres/search/query_builder.rs`).
    ///
    /// `?1` is the tenant in every param layout `QueryBuilder` produces
    /// (see its `with_param_offset`), so reusing it consumes no binding and
    /// leaves `param_num` arithmetic untouched. `si2.resource_type` is
    /// deliberately *not* constrained to the searched type: `si2` describes the
    /// reference *target*, whose type differs — it is instead concatenated into
    /// the compared `Type/id`, which is what pins the match to the right target.
    fn build_identifier_condition(identifier_value: &str, param_num: usize) -> SqlFragment {
        // The enclosing row's reference, stripped of any `/_history/<v>` suffix.
        const REF_BASE: &str = "CASE WHEN INSTR(value_reference, '/_history/') > 0 \
             THEN SUBSTR(value_reference, 1, INSTR(value_reference, '/_history/') - 1) \
             ELSE value_reference END";

        // Builds `<ref-base> IN (SELECT Type/id FROM search_index si2 WHERE ... AND <pred>)`.
        fn target_in(pred: String) -> String {
            format!(
                "{REF_BASE} IN (SELECT si2.resource_type || '/' || si2.resource_id \
                 FROM search_index si2 \
                 WHERE si2.tenant_id = ?1 AND si2.param_name = 'identifier' AND {pred})"
            )
        }

        // Parse the identifier value (system|value format)
        if let Some(pipe_pos) = identifier_value.find('|') {
            let system = &identifier_value[..pipe_pos];
            let value = &identifier_value[pipe_pos + 1..];

            if system.is_empty() {
                // |value - match value with no system
                SqlFragment::with_params(
                    target_in(format!(
                        "(si2.value_token_system IS NULL OR si2.value_token_system = '') \
                         AND si2.value_token_code = ?{param_num}"
                    )),
                    vec![SqlParam::string(value)],
                )
            } else if value.is_empty() {
                // system| - match any value in system
                SqlFragment::with_params(
                    target_in(format!("si2.value_token_system = ?{param_num}")),
                    vec![SqlParam::string(system)],
                )
            } else {
                // system|value - exact match
                SqlFragment::with_params(
                    target_in(format!(
                        "si2.value_token_system = ?{} AND si2.value_token_code = ?{}",
                        param_num,
                        param_num + 1
                    )),
                    vec![SqlParam::string(system), SqlParam::string(value)],
                )
            }
        } else {
            // Just a value - match any system
            SqlFragment::with_params(
                target_in(format!("si2.value_token_code = ?{param_num}")),
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

        // Version-agnostic: exact match or any `_history` version.
        assert!(frag.sql.contains("value_reference = ?1"));
        assert!(frag.sql.contains("'/_history/%'"));
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn test_reference_versioned_search_strips_version() {
        // Searching a versioned reference matches the version-stripped base.
        let value = SearchValue::new(SearchPrefix::Eq, "Patient/123/_history/2");
        let frag = ReferenceHandler::build_sql(&value, None, 0);
        assert_eq!(frag.params.len(), 2);
        // Both bound params are the stripped base "Patient/123".
        // (SqlParam compares via its Debug/string form.)
        let dbg = format!("{:?}", frag.params);
        assert!(dbg.contains("Patient/123"));
        assert!(!dbg.contains("_history"));
    }

    #[test]
    fn test_strip_reference_version() {
        assert_eq!(
            strip_reference_version("Patient/123/_history/2"),
            "Patient/123"
        );
        assert_eq!(strip_reference_version("Patient/123"), "Patient/123");
        assert_eq!(strip_reference_version("123"), "123");
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
        // Offset 2 is what `QueryBuilder` passes: ?1 = tenant, ?2 = resource type.
        let frag = ReferenceHandler::build_sql(&value, Some(&SearchModifier::Identifier), 2);

        assert!(frag.sql.contains("si2.param_name = 'identifier'"));
        assert!(frag.sql.contains("si2.value_token_system = ?3"));
        assert!(frag.sql.contains("si2.value_token_code = ?4"));
        assert_eq!(frag.params.len(), 2);

        // The sub-select is uncorrelated and yields the target's `Type/id`; the
        // referencing row's `value_reference` is compared from the outer scope.
        // Written inside the sub-select it would bind to `si2`'s own (NULL)
        // column instead, which is what made `:identifier` match nothing.
        assert!(
            frag.sql
                .contains("IN (SELECT si2.resource_type || '/' || si2.resource_id"),
            "identifier match must compare against the target's Type/id: {}",
            frag.sql
        );
        let subselect_start = frag.sql.find("(SELECT si2").expect("sub-select");
        assert!(
            !frag.sql[subselect_start..].contains("value_reference"),
            "`value_reference` inside the sub-select would resolve to si2's own column: {}",
            frag.sql
        );

        // Tenant scoping is load-bearing (see `build_identifier_condition`).
        assert!(frag.sql.contains("si2.tenant_id = ?1"));
    }
}
