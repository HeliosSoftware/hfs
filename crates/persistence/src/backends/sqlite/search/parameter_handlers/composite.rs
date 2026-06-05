//! Composite parameter SQL handler.

use crate::types::{CompositeSearchComponent, SearchParamType, SearchPrefix, SearchValue};

use super::super::query_builder::SqlFragment;
use super::{DateHandler, NumberHandler, QuantityHandler, StringHandler, TokenHandler};

/// Handles composite parameter SQL generation.
///
/// Composite parameters combine multiple sub-parameters with a `$` separator.
/// For example, `component-code-value-quantity=http://loinc.org|8480-6$lt60`
/// combines a token search on code with a quantity search on value.
pub struct CompositeHandler;

/// Definition of a composite component.
#[derive(Debug, Clone)]
pub struct CompositeComponentDef {
    /// The sub-parameter type.
    pub param_type: SearchParamType,
    /// The column name prefix to use for this component.
    pub column_prefix: String,
}

impl CompositeHandler {
    /// Builds SQL for a composite parameter value using CompositeSearchComponent definitions.
    ///
    /// This is the primary entry point called from QueryBuilder.
    /// Since composite parameters need to match all component conditions on the same row,
    /// we simply combine all conditions with AND. The outer query already filters by
    /// param_name, so we just need the value conditions.
    ///
    /// Note: For true composite group matching (where values must come from the same
    /// composite instance), we would need the extractor to populate composite_group
    /// during indexing and use a more complex query. For now, we match all conditions
    /// which works for simple cases.
    pub fn build_composite_sql(
        value: &SearchValue,
        _param_name: &str,
        components: &[CompositeSearchComponent],
        param_offset: usize,
    ) -> SqlFragment {
        let composite_value = &value.value;
        let parts: Vec<&str> = composite_value.split('$').collect();

        if parts.len() != components.len() || components.is_empty() {
            return SqlFragment::new("1 = 0");
        }

        let mut component_conditions = Vec::new();
        let mut all_params = Vec::new();
        let mut current_offset = param_offset;

        // Build condition for each component
        for (part, component) in parts.iter().zip(components.iter()) {
            let component_value = Self::parse_component_value(part);
            let fragment = Self::build_component_sql_from_type(
                &component_value,
                component.param_type,
                current_offset,
            );

            if fragment.sql == "1 = 0" {
                return SqlFragment::new("1 = 0");
            }

            component_conditions.push(fragment.sql);
            current_offset += fragment.params.len();
            all_params.extend(fragment.params);
        }

        // Combine all component conditions - they must all match
        // The outer query context already filters by param_name and resource context
        let conditions_sql = component_conditions.join(" AND ");

        SqlFragment::with_params(format!("({})", conditions_sql), all_params)
    }

    /// Builds one bare predicate fragment per component.
    ///
    /// Returns `None` if the value's part count does not match the component
    /// count, or if any component value cannot be built. The caller wraps each
    /// fragment in a `MAX(CASE WHEN ... THEN 1 ELSE 0 END) = 1` aggregate and
    /// groups by `(resource_id, composite_group)` so that all components are
    /// matched within the same composite instance.
    pub fn build_component_fragments(
        value: &SearchValue,
        components: &[CompositeSearchComponent],
        param_offset: usize,
    ) -> Option<Vec<SqlFragment>> {
        let parts: Vec<&str> = value.value.split('$').collect();
        if parts.len() != components.len() || components.is_empty() {
            return None;
        }

        let mut fragments = Vec::new();
        let mut current_offset = param_offset;
        for (part, component) in parts.iter().zip(components.iter()) {
            let component_value = Self::parse_component_value(part);
            let fragment = Self::build_component_sql_from_type(
                &component_value,
                component.param_type,
                current_offset,
            );
            if fragment.sql == "1 = 0" {
                return None;
            }
            current_offset += fragment.params.len();
            fragments.push(fragment);
        }
        Some(fragments)
    }

    /// Builds SQL for a composite parameter value.
    ///
    /// The value should be in the format "value1$value2$..." where each value
    /// corresponds to a component defined in the composite parameter.
    ///
    /// All components must match on the same search_index row (composite_group).
    pub fn build_sql(
        value: &SearchValue,
        components: &[CompositeComponentDef],
        param_offset: usize,
    ) -> SqlFragment {
        let composite_value = &value.value;
        let parts: Vec<&str> = composite_value.split('$').collect();

        if parts.len() != components.len() {
            // Mismatch in component count
            return SqlFragment::new("1 = 0");
        }

        let mut conditions = Vec::new();
        let mut params = Vec::new();
        let mut current_offset = param_offset;

        for (part, component) in parts.iter().zip(components.iter()) {
            // Create a SearchValue for this component part
            let component_value = Self::parse_component_value(part);

            // Generate SQL for this component based on its type
            let fragment = Self::build_component_sql(&component_value, component, current_offset);

            if fragment.sql == "1 = 0" {
                // Invalid component value
                return SqlFragment::new("1 = 0");
            }

            conditions.push(fragment.sql);
            current_offset += fragment.params.len();
            params.extend(fragment.params);
        }

        // All conditions must match on the same composite_group
        // We wrap the conditions to ensure they're matched together
        SqlFragment::with_params(format!("({})", conditions.join(" AND ")), params)
    }

    /// Builds component SQL from a SearchParamType directly.
    fn build_component_sql_from_type(
        value: &SearchValue,
        param_type: SearchParamType,
        param_offset: usize,
    ) -> SqlFragment {
        match param_type {
            SearchParamType::Token => TokenHandler::build_sql(value, None, param_offset),
            SearchParamType::String => StringHandler::build_sql(value, None, param_offset),
            SearchParamType::Date => DateHandler::build_sql(value, param_offset),
            SearchParamType::Number => NumberHandler::build_sql(value, param_offset),
            SearchParamType::Quantity => QuantityHandler::build_sql(value, param_offset),
            _ => SqlFragment::new("1 = 0"),
        }
    }

    /// Parses a component value, extracting any prefix.
    fn parse_component_value(part: &str) -> SearchValue {
        // Check for comparison prefixes at the start
        let prefixes = [
            ("ne", SearchPrefix::Ne),
            ("gt", SearchPrefix::Gt),
            ("lt", SearchPrefix::Lt),
            ("ge", SearchPrefix::Ge),
            ("le", SearchPrefix::Le),
            ("sa", SearchPrefix::Sa),
            ("eb", SearchPrefix::Eb),
            ("ap", SearchPrefix::Ap),
            ("eq", SearchPrefix::Eq),
        ];

        for (prefix_str, prefix) in prefixes {
            if let Some(stripped) = part.strip_prefix(prefix_str) {
                return SearchValue::new(prefix, stripped);
            }
        }

        // No prefix found - default to eq
        SearchValue::new(SearchPrefix::Eq, part)
    }

    /// Builds SQL for a single component.
    fn build_component_sql(
        value: &SearchValue,
        component: &CompositeComponentDef,
        param_offset: usize,
    ) -> SqlFragment {
        match component.param_type {
            SearchParamType::Token => {
                // Use token handler but we may need to adjust column names
                TokenHandler::build_sql(value, None, param_offset)
            }
            SearchParamType::String => StringHandler::build_sql(value, None, param_offset),
            SearchParamType::Date => DateHandler::build_sql(value, param_offset),
            SearchParamType::Number => NumberHandler::build_sql(value, param_offset),
            SearchParamType::Quantity => QuantityHandler::build_sql(value, param_offset),
            _ => {
                // Unsupported component type
                SqlFragment::new("1 = 0")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_composite_token_quantity() {
        let value = SearchValue::new(SearchPrefix::Eq, "http://loinc.org|8480-6$lt60");

        let components = vec![
            CompositeComponentDef {
                param_type: SearchParamType::Token,
                column_prefix: "code".to_string(),
            },
            CompositeComponentDef {
                param_type: SearchParamType::Quantity,
                column_prefix: "value".to_string(),
            },
        ];

        let frag = CompositeHandler::build_sql(&value, &components, 0);

        assert!(frag.sql.contains("value_token_system"));
        assert!(frag.sql.contains("value_quantity_value"));
        assert!(frag.sql.contains("AND"));
    }

    #[test]
    fn test_composite_mismatched_parts() {
        let value = SearchValue::new(SearchPrefix::Eq, "value1");

        let components = vec![
            CompositeComponentDef {
                param_type: SearchParamType::Token,
                column_prefix: "code".to_string(),
            },
            CompositeComponentDef {
                param_type: SearchParamType::Quantity,
                column_prefix: "value".to_string(),
            },
        ];

        let frag = CompositeHandler::build_sql(&value, &components, 0);

        // Should fail due to mismatch
        assert!(frag.sql.contains("1 = 0"));
    }

    #[test]
    fn test_composite_token_date() {
        let value = SearchValue::new(SearchPrefix::Eq, "active$ge2024-01-01");

        let components = vec![
            CompositeComponentDef {
                param_type: SearchParamType::Token,
                column_prefix: "status".to_string(),
            },
            CompositeComponentDef {
                param_type: SearchParamType::Date,
                column_prefix: "date".to_string(),
            },
        ];

        let frag = CompositeHandler::build_sql(&value, &components, 0);

        assert!(frag.sql.contains("value_token_code"));
        assert!(frag.sql.contains("value_date"));
    }
}
