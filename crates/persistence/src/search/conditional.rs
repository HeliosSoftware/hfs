//! Parsing of conditional-interaction criteria into a [`SearchQuery`].
//!
//! `If-None-Exist`, conditional update / patch / delete, and a Bundle entry's
//! `ifNoneExist` all hand a backend a raw `name=value&name=value` string. What
//! that string matches decides whether a write creates a duplicate, or lands
//! on — or deletes — somebody else's resource, so it has to mean exactly what
//! the same string means as a direct search.
//!
//! Every backend used to build these criteria for itself, and every copy
//! parsed the value with [`SearchValue::parse`] regardless of the parameter's
//! type. That routine strips any leading two letters that spell a comparator
//! (`eq ne gt lt ge le sa eb ap`, case-insensitively), so `family=Neal`
//! searched for `al` and `identifier=ne123` for `123` (#1312): the existing
//! Neal was not found (a duplicate create), while an unrelated Allen, or the
//! patient whose identifier really is `123`, was — and a conditional `PUT` or
//! `DELETE` was applied to them instead.
//!
//! This module is the one builder all of them now share. It goes through
//! [`parse_typed_values`], the routine the REST query builder and the chain
//! resolver use, so conditional criteria cannot drift from direct search
//! again: comparator prefixes only for date / number / quantity parameters,
//! OR-lists split on unescaped commas, and `name:modifier` honoured.
//!
//! The criteria string is taken as already percent-decoded — the REST layer
//! decodes it once, and no backend decodes it again.

use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{
    CompositeSearchComponent, SearchModifier, SearchParamType, SearchParameter, SearchQuery,
    SearchValue,
};

use super::registry::{SearchParameterRegistry, fallback_param_type};
use super::value_parser::{parse_typed_values, split_unescaped_commas};

/// Upper bound on the matches a conditional interaction asks the search for.
/// One match and "more than one" are all it ever distinguishes; the bound only
/// keeps a broad criterion from materialising a whole resource type.
const CONDITIONAL_MATCH_LIMIT: u32 = 1000;

/// Parameters that shape a search *response* rather than select resources.
///
/// A client that appends `_format=json` to `PUT /Patient?identifier=…` is
/// negotiating content, not adding a criterion. Treated as a filter, such a
/// name has no index rows, never matches, and turns every conditional update
/// into a create — so they are dropped, as direct search drops them.
const RESULT_PARAMS: &[&str] = &[
    "_format",
    "_pretty",
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
    "_score",
];

/// Splits conditional criteria into `(name, value)` pairs.
///
/// Pairs without an `=`, with an empty name, or with an empty value are
/// dropped. Repeated names are kept, in order: FHIR ANDs them.
pub fn parse_conditional_criteria(criteria: &str) -> Vec<(String, String)> {
    criteria
        .split('&')
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            let (name, value) = (name.trim(), value.trim());
            if name.is_empty() || value.is_empty() {
                return None;
            }
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

/// Builds the typed search parameters a list of criteria pairs describes.
///
/// Result-shaping parameters (`_format`, `_count`, …) are skipped. Criteria
/// this layer cannot evaluate — chained parameters, `_has`, `_list` — are
/// refused rather than searched for under their literal name, which would
/// match nothing and quietly turn the interaction into an unconditional one.
pub fn build_conditional_parameters(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    pairs: &[(String, String)],
) -> StorageResult<Vec<SearchParameter>> {
    let mut parameters = Vec::with_capacity(pairs.len());

    for (raw_name, raw_value) in pairs {
        if RESULT_PARAMS.contains(&raw_name.as_str()) {
            continue;
        }
        if raw_name == "_has" || raw_name.starts_with("_has:") {
            return Err(StorageError::Search(SearchError::ReverseChainNotSupported));
        }
        if raw_name.contains('.') {
            return Err(StorageError::Search(
                SearchError::ChainedSearchNotSupported {
                    chain: format!("{raw_name} (in conditional criteria)"),
                },
            ));
        }
        if raw_name == "_list" {
            return Err(query_error(
                "'_list' is not supported in conditional criteria".to_string(),
            ));
        }

        let (name, modifier) = match raw_name.split_once(':') {
            Some((name, modifier_str)) => {
                let modifier = SearchModifier::parse(modifier_str).ok_or_else(|| {
                    query_error(format!(
                        "unknown search modifier ':{modifier_str}' on conditional criterion \
                         '{raw_name}'"
                    ))
                })?;
                (name, Some(modifier))
            }
            None => (raw_name.as_str(), None),
        };

        // `:missing` takes one case-sensitive boolean; anything else must not
        // quietly become `missing=false` in a backend.
        if matches!(modifier, Some(SearchModifier::Missing))
            && !matches!(raw_value.as_str(), "true" | "false")
        {
            return Err(query_error(format!(
                "the :missing modifier on '{raw_name}' requires exactly 'true' or 'false'"
            )));
        }

        let definition = registry
            .get_param(resource_type, name)
            .or_else(|| registry.get_param("Resource", name));
        let raw_values = split_unescaped_commas(raw_value);

        let (param_type, values) = match (&definition, fallback_param_type(name)) {
            // A registry miss on a name the fallback table knows keeps the
            // table's answer: it records which index column the extractor
            // writes those rows under (see `fallback_param_type`).
            (None, fallback) if fallback != SearchParamType::String => {
                (fallback, values_for_type(fallback, &raw_values))
            }
            // Registered parameters — everything in the spec — resolve
            // deterministically; anything else gets the same value-shape
            // heuristic direct search applies.
            _ => parse_typed_values(registry, resource_type, name, &raw_values),
        };

        if let Some(m) = &modifier {
            if definition.is_some()
                && param_type != SearchParamType::Special
                && !m.is_valid_for(param_type)
            {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: m.to_string(),
                    param_type: param_type.to_string(),
                }));
            }
        }

        // Composite parameters carry their component types so a backend can
        // match every component within one composite instance.
        let components = match (&definition, param_type) {
            (Some(def), SearchParamType::Composite) => def
                .component
                .iter()
                .flatten()
                .filter_map(|c| {
                    registry
                        .get_by_url(&c.definition)
                        .map(|sub| CompositeSearchComponent {
                            param_type: sub.param_type,
                            param_name: sub.code.clone(),
                        })
                })
                .collect(),
            _ => Vec::new(),
        };

        parameters.push(SearchParameter {
            name: name.to_string(),
            param_type,
            modifier,
            values,
            chain: vec![],
            components,
        });
    }

    Ok(parameters)
}

/// Builds the search a conditional interaction's criteria describe, or `None`
/// when they select nothing — matching everything would be the literal
/// reading, but no conditional interaction means that.
pub fn build_conditional_query(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    criteria: &str,
) -> StorageResult<Option<SearchQuery>> {
    let pairs = parse_conditional_criteria(criteria);
    let parameters = build_conditional_parameters(registry, resource_type, &pairs)?;
    if parameters.is_empty() {
        return Ok(None);
    }

    Ok(Some(SearchQuery {
        resource_type: resource_type.to_string(),
        parameters,
        count: Some(CONDITIONAL_MATCH_LIMIT),
        ..Default::default()
    }))
}

/// Parses already comma-split values for a known type: a comparator prefix is
/// recognised only where FHIR defines one.
fn values_for_type(param_type: SearchParamType, raw_values: &[String]) -> Vec<SearchValue> {
    let prefixed = matches!(
        param_type,
        SearchParamType::Date | SearchParamType::Number | SearchParamType::Quantity
    );
    raw_values
        .iter()
        .map(|v| {
            if prefixed {
                SearchValue::parse(v)
            } else {
                SearchValue::eq(v.clone())
            }
        })
        .collect()
}

fn query_error(message: String) -> StorageError {
    StorageError::Search(SearchError::QueryParseError { message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::registry::SearchParameterDefinition;
    use crate::types::SearchPrefix;

    fn registry() -> SearchParameterRegistry {
        let mut registry = SearchParameterRegistry::new();
        for (code, param_type, expression) in [
            ("family", SearchParamType::String, "Patient.name.family"),
            ("identifier", SearchParamType::Token, "Patient.identifier"),
            ("birthdate", SearchParamType::Date, "Patient.birthDate"),
            (
                "general-practitioner",
                SearchParamType::Reference,
                "Patient.generalPractitioner",
            ),
        ] {
            registry
                .register(
                    SearchParameterDefinition::new(
                        format!("http://hl7.org/fhir/SearchParameter/Patient-{code}"),
                        code,
                        param_type,
                        expression,
                    )
                    .with_base(["Patient"]),
                )
                .expect("register");
        }
        registry
    }

    fn one(criteria: &str) -> SearchParameter {
        let query = build_conditional_query(&registry(), "Patient", criteria)
            .expect("valid criteria")
            .expect("some criteria");
        assert_eq!(query.parameters.len(), 1, "{criteria}");
        query.parameters.into_iter().next().expect("one parameter")
    }

    #[test]
    fn comparator_letters_are_kept_for_non_ordered_types() {
        for (criteria, expected) in [
            ("family=Neal", "Neal"),
            ("family=Levine", "Levine"),
            ("family=Gtari", "Gtari"),
            ("identifier=ne123", "ne123"),
            ("identifier=eq77", "eq77"),
            ("identifier=sa-1", "sa-1"),
            ("general-practitioner=le-1", "le-1"),
            // Unregistered, but the fallback table types it as a uri.
            ("_source=sandbox.example/feed", "sandbox.example/feed"),
            // Unregistered and unknown to the fallback table: value-shape
            // heuristic, which reads this as a string.
            ("nickname=Lee", "Lee"),
        ] {
            let param = one(criteria);
            assert_eq!(param.values.len(), 1, "{criteria}");
            assert_eq!(param.values[0].prefix, SearchPrefix::Eq, "{criteria}");
            assert_eq!(param.values[0].value, expected, "{criteria}");
        }
    }

    #[test]
    fn comparators_still_apply_to_dates() {
        let param = one("birthdate=ge1980-01-01");
        assert_eq!(param.param_type, SearchParamType::Date);
        assert_eq!(param.values[0].prefix, SearchPrefix::Ge);
        assert_eq!(param.values[0].value, "1980-01-01");

        let param = one("_lastUpdated=lt2024-01-01");
        assert_eq!(param.param_type, SearchParamType::Date);
        assert_eq!(param.values[0].prefix, SearchPrefix::Lt);
    }

    #[test]
    fn or_lists_split_on_unescaped_commas() {
        let param = one("family=Neal,Levine\\, Jr");
        let values: Vec<&str> = param.values.iter().map(|v| v.value.as_str()).collect();
        assert_eq!(values, vec!["Neal", "Levine, Jr"]);
    }

    #[test]
    fn modifiers_are_split_off_the_name_and_validated() {
        let param = one("family:exact=Neal");
        assert_eq!(param.name, "family");
        assert_eq!(param.modifier, Some(SearchModifier::Exact));
        assert_eq!(param.values[0].value, "Neal");

        let registry = registry();
        for criteria in [
            "identifier:exact=ne123",
            "family:bogus=Neal",
            "family:missing=maybe",
        ] {
            assert!(
                build_conditional_query(&registry, "Patient", criteria).is_err(),
                "{criteria} must be refused"
            );
        }
    }

    #[test]
    fn result_parameters_are_not_criteria() {
        let registry = registry();
        let query = build_conditional_query(
            &registry,
            "Patient",
            "identifier=ne123&_format=json&_count=5",
        )
        .expect("valid")
        .expect("some");
        assert_eq!(query.parameters.len(), 1);
        assert_eq!(query.parameters[0].name, "identifier");

        assert!(
            build_conditional_query(&registry, "Patient", "_format=json")
                .expect("valid")
                .is_none()
        );
        assert!(
            build_conditional_query(&registry, "Patient", "")
                .expect("valid")
                .is_none()
        );
    }

    #[test]
    fn criteria_this_layer_cannot_evaluate_are_refused() {
        let registry = registry();
        for criteria in [
            "general-practitioner.name=Neal",
            "_has:Observation:patient:code=1234-5",
            "_list=42",
        ] {
            assert!(
                build_conditional_query(&registry, "Patient", criteria).is_err(),
                "{criteria} must be refused"
            );
        }
    }

    #[test]
    fn repeated_names_are_kept_as_separate_parameters() {
        let query = build_conditional_query(
            &registry(),
            "Patient",
            "birthdate=ge1980-01-01&birthdate=le1980-12-31",
        )
        .expect("valid")
        .expect("some");
        assert_eq!(query.parameters.len(), 2);
    }
}
