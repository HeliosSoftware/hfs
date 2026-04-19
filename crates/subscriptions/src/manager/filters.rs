//! Subscription filter parsing and validation.

use crate::error::SubscriptionError;
use crate::topics::FilterDefinition;

/// A parsed subscription filter criterion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionFilter {
    /// The resource type (e.g., "Observation").
    pub resource_type: Option<String>,

    /// The filter parameter name (e.g., "code").
    pub filter_parameter: String,

    /// The comparator (e.g., "eq", "in").
    pub comparator: String,

    /// The filter value (e.g., "http://loinc.org|1234-5").
    pub value: String,
}

/// Parses a filter criteria string in backport IG format.
///
/// The format is: `[ResourceType?]param=value` or `[ResourceType?]param=comparator:value`.
///
/// Examples:
/// - `Observation?code=http://loinc.org|1234-5`
/// - `Encounter?patient=Patient/123`
/// - `code=http://loinc.org|1234-5` (no resource type prefix)
pub fn parse_filter_string(filter: &str) -> Result<SubscriptionFilter, SubscriptionError> {
    let (resource_type, param_str) = if let Some(idx) = filter.find('?') {
        (Some(filter[..idx].to_string()), &filter[idx + 1..])
    } else {
        (None, filter)
    };

    let (param_name, value_str) =
        param_str
            .split_once('=')
            .ok_or_else(|| SubscriptionError::InvalidFilter {
                message: format!("filter missing '=' separator: {filter}"),
            })?;

    if param_name.is_empty() {
        return Err(SubscriptionError::InvalidFilter {
            message: "filter parameter name is empty".to_string(),
        });
    }

    if value_str.is_empty() {
        return Err(SubscriptionError::InvalidFilter {
            message: format!("filter value is empty for parameter '{param_name}'"),
        });
    }

    // Check for comparator prefix (e.g., "gt:5" or "ne:finished").
    let (comparator, value) = if let Some((comp, val)) = value_str.split_once(':') {
        // Only treat as comparator if it's a known FHIR comparator.
        if is_known_comparator(comp) {
            (comp.to_string(), val.to_string())
        } else {
            // Not a comparator — the colon is part of the value (e.g., system|code).
            ("eq".to_string(), value_str.to_string())
        }
    } else {
        ("eq".to_string(), value_str.to_string())
    };

    Ok(SubscriptionFilter {
        resource_type,
        filter_parameter: param_name.to_string(),
        comparator,
        value,
    })
}

fn is_known_comparator(s: &str) -> bool {
    matches!(
        s,
        "eq" | "ne" | "gt" | "lt" | "ge" | "le" | "sa" | "eb" | "in" | "not-in"
    )
}

/// Validates a list of subscription filters against a topic's `canFilterBy` definitions.
///
/// Returns `Ok(())` if all filters are valid, or an error describing the first invalid filter.
pub fn validate_filters(
    filters: &[SubscriptionFilter],
    can_filter_by: &[FilterDefinition],
) -> Result<(), SubscriptionError> {
    for filter in filters {
        let matching_def = can_filter_by.iter().find(|def| {
            // Match filter parameter name.
            def.filter_parameter == filter.filter_parameter
                // If the topic filter has a resource type constraint, it must match.
                && def
                    .resource_type
                    .as_ref()
                    .is_none_or(|rt| filter.resource_type.as_ref().is_none_or(|frt| frt == rt))
        });

        match matching_def {
            None => {
                return Err(SubscriptionError::InvalidFilter {
                    message: format!(
                        "filter parameter '{}' is not supported by the topic",
                        filter.filter_parameter
                    ),
                });
            }
            Some(def) => {
                // If the topic defines allowed comparators, check the filter uses one.
                if !def.comparators.is_empty() && !def.comparators.contains(&filter.comparator) {
                    return Err(SubscriptionError::InvalidFilter {
                        message: format!(
                            "comparator '{}' not supported for filter parameter '{}' (supported: {:?})",
                            filter.comparator, filter.filter_parameter, def.comparators
                        ),
                    });
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_filter_with_resource_type() {
        let filter = parse_filter_string("Observation?code=http://loinc.org|1234-5").unwrap();
        assert_eq!(filter.resource_type.as_deref(), Some("Observation"));
        assert_eq!(filter.filter_parameter, "code");
        assert_eq!(filter.comparator, "eq");
        assert_eq!(filter.value, "http://loinc.org|1234-5");
    }

    #[test]
    fn test_parse_filter_without_resource_type() {
        let filter = parse_filter_string("code=http://loinc.org|1234-5").unwrap();
        assert!(filter.resource_type.is_none());
        assert_eq!(filter.filter_parameter, "code");
        assert_eq!(filter.value, "http://loinc.org|1234-5");
    }

    #[test]
    fn test_parse_filter_with_comparator() {
        let filter = parse_filter_string("Observation?value-quantity=gt:5").unwrap();
        assert_eq!(filter.comparator, "gt");
        assert_eq!(filter.value, "5");
    }

    #[test]
    fn test_parse_filter_colon_in_value_not_comparator() {
        // "http://loinc.org|1234-5" contains a colon but "http" is not a comparator.
        let filter = parse_filter_string("code=http://loinc.org|1234-5").unwrap();
        assert_eq!(filter.comparator, "eq");
        assert_eq!(filter.value, "http://loinc.org|1234-5");
    }

    #[test]
    fn test_parse_filter_reference() {
        let filter = parse_filter_string("Encounter?patient=Patient/123").unwrap();
        assert_eq!(filter.filter_parameter, "patient");
        assert_eq!(filter.value, "Patient/123");
    }

    #[test]
    fn test_parse_filter_missing_equals() {
        let result = parse_filter_string("Observation?code");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_filter_empty_param() {
        let result = parse_filter_string("=value");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_filter_empty_value() {
        let result = parse_filter_string("code=");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_filters_success() {
        let filters = vec![SubscriptionFilter {
            resource_type: Some("Observation".to_string()),
            filter_parameter: "code".to_string(),
            comparator: "eq".to_string(),
            value: "http://loinc.org|1234-5".to_string(),
        }];

        let can_filter_by = vec![FilterDefinition {
            resource_type: Some("Observation".to_string()),
            filter_parameter: "code".to_string(),
            comparators: vec!["eq".to_string(), "in".to_string()],
            modifiers: vec![],
        }];

        assert!(validate_filters(&filters, &can_filter_by).is_ok());
    }

    #[test]
    fn test_validate_filters_unknown_parameter() {
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "unknown-param".to_string(),
            comparator: "eq".to_string(),
            value: "value".to_string(),
        }];

        let can_filter_by = vec![FilterDefinition {
            resource_type: Some("Observation".to_string()),
            filter_parameter: "code".to_string(),
            comparators: vec!["eq".to_string()],
            modifiers: vec![],
        }];

        let result = validate_filters(&filters, &can_filter_by);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_filters_unsupported_comparator() {
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "code".to_string(),
            comparator: "gt".to_string(),
            value: "value".to_string(),
        }];

        let can_filter_by = vec![FilterDefinition {
            resource_type: Some("Observation".to_string()),
            filter_parameter: "code".to_string(),
            comparators: vec!["eq".to_string()],
            modifiers: vec![],
        }];

        let result = validate_filters(&filters, &can_filter_by);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_filters_empty_comparators_allows_any() {
        let filters = vec![SubscriptionFilter {
            resource_type: None,
            filter_parameter: "code".to_string(),
            comparator: "gt".to_string(),
            value: "value".to_string(),
        }];

        let can_filter_by = vec![FilterDefinition {
            resource_type: None,
            filter_parameter: "code".to_string(),
            comparators: vec![], // No restriction.
            modifiers: vec![],
        }];

        assert!(validate_filters(&filters, &can_filter_by).is_ok());
    }

    #[test]
    fn test_validate_empty_filters_always_valid() {
        assert!(validate_filters(&[], &[]).is_ok());
    }
}
