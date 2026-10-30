//! The implied system of a `code` element, checked on the query side (#1395).
//!
//! A `code` primitive carries no system of its own, so its index row carries
//! the [`IMPLICIT_TOKEN_SYSTEM`](super::IMPLICIT_TOKEN_SYSTEM) marker, and
//! every backend lets a `system|code` search match a marked row whatever system
//! it names (#1379). FHIR says the system is not open, though: it is implied by
//! the element's binding, and a different system is a different code.
//! `Patient?gender=http://wrong.example|female` must not match a female
//! patient.
//!
//! The index cannot know the binding, but a query that names a spec search
//! parameter can: `helios_fhir::implied_code_systems` holds, for each token
//! parameter whose every path is a `code` element with a required binding to a
//! single-CodeSystem ValueSet, that one system. [`narrow_implied_code_systems`]
//! rewrites a `system|code` value naming any other system into
//! `{NO_TOKEN_SYSTEM}|`, which every backend answers as an exact match on a
//! system no row ever has: nothing. As one alternative of an OR-list it drops
//! out; under `:not` it negates into everything. No backend changes, and no
//! reindex.
//!
//! Left alone, and still matching any system:
//! - parameters outside the table (a `Coding` / `CodeableConcept` path, an
//!   extensible binding, a ValueSet spanning several systems, a custom
//!   SearchParameter whose expression differs from the spec's);
//! - `|code` (a `code` element has no system property, so it matches) and a
//!   bare `code`;
//! - modifiers other than none and `:not` — `:in` / `:above` / `:below` have
//!   already been expanded into plain `system|code` values by the time this
//!   runs, and `:text` / `:of-type` do not read the system this way.

use crate::types::{SearchModifier, SearchParamType, SearchParameter, SearchValue};

use super::SearchParameterRegistry;

/// A token system no index row ever carries. Like the
/// [`IMPLICIT_TOKEN_SYSTEM`](super::IMPLICIT_TOKEN_SYSTEM) marker it contains
/// spaces, which a FHIR `uri` cannot, and nothing writes it to the index; a
/// `system|` search for it is therefore a search that matches nothing.
pub const NO_TOKEN_SYSTEM: &str = "urn:x-helios:no code system";

/// The system implied for every `code` element that search parameter `code`
/// reaches on `resource_type`, when the registry's definition of it is the
/// spec's and the spec gives it a single one.
pub fn implied_code_system(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    code: &str,
) -> Option<&'static str> {
    let (base, definition) = match registry.get_param(resource_type, code) {
        Some(definition) => (resource_type, definition),
        None => ("Resource", registry.get_param("Resource", code)?),
    };
    if definition.param_type != SearchParamType::Token {
        return None;
    }
    helios_fhir::implied_code_systems::implied_code_system(base, code, &definition.expression)
}

/// Rewrites each `system|code` value of a token search on `name` whose system
/// is not the one implied for the parameter into a value that matches nothing.
/// Returns whether any value was rewritten.
///
/// Call it on the final values of a search — after terminology expansion — and
/// once per resource type, since the implied system is the resource type's.
pub fn narrow_implied_code_systems(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
    param_type: SearchParamType,
    modifier: Option<&SearchModifier>,
    values: &mut [SearchValue],
) -> bool {
    if param_type != SearchParamType::Token || !matches!(modifier, None | Some(SearchModifier::Not))
    {
        return false;
    }
    let Some(implied) = implied_code_system(registry, resource_type, name) else {
        return false;
    };
    let mut narrowed = false;
    for value in values {
        let Some((system, code)) = value.value.split_once('|') else {
            continue;
        };
        if !system.is_empty() && !code.is_empty() && system != implied {
            value.value = format!("{NO_TOKEN_SYSTEM}|");
            narrowed = true;
        }
    }
    narrowed
}

/// [`narrow_implied_code_systems`] on an unchained [`SearchParameter`].
pub fn narrow_parameter_implied_code_systems(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    param: &mut SearchParameter,
) -> bool {
    if !param.chain.is_empty() {
        return false;
    }
    narrow_implied_code_systems(
        registry,
        resource_type,
        &param.name,
        param.param_type,
        param.modifier.as_ref(),
        &mut param.values,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{SearchParameterDefinition, SearchParameterSource};

    const GENDER: &str = "http://hl7.org/fhir/administrative-gender";
    const GENDER_EXPRESSION: &str =
        "Patient.gender | Person.gender | Practitioner.gender | RelatedPerson.gender";

    fn registry(expression: &str) -> SearchParameterRegistry {
        let mut registry = SearchParameterRegistry::new();
        registry
            .register(
                SearchParameterDefinition::new(
                    "http://hl7.org/fhir/SearchParameter/individual-gender",
                    "gender",
                    SearchParamType::Token,
                    expression,
                )
                .with_base(["Patient"])
                .with_source(SearchParameterSource::Embedded),
            )
            .unwrap();
        registry
    }

    fn narrow(
        registry: &SearchParameterRegistry,
        modifier: Option<SearchModifier>,
        values: &[&str],
    ) -> Vec<String> {
        let mut values: Vec<SearchValue> = values.iter().map(|v| SearchValue::eq(*v)).collect();
        narrow_implied_code_systems(
            registry,
            "Patient",
            "gender",
            SearchParamType::Token,
            modifier.as_ref(),
            &mut values,
        );
        values.into_iter().map(|v| v.value).collect()
    }

    #[test]
    fn the_spec_gender_parameter_implies_administrative_gender() {
        assert_eq!(
            implied_code_system(&registry(GENDER_EXPRESSION), "Patient", "gender"),
            Some(GENDER)
        );
    }

    #[test]
    fn a_wrong_system_matches_nothing_and_the_rest_is_untouched() {
        let nothing = format!("{NO_TOKEN_SYSTEM}|");
        let right = format!("{GENDER}|female");
        let registry = registry(GENDER_EXPRESSION);
        assert_eq!(
            narrow(
                &registry,
                None,
                &[
                    "http://wrong.example|female",
                    &right,
                    "|female",
                    "female",
                    "http://wrong.example|"
                ],
            ),
            [
                nothing.as_str(),
                &right,
                "|female",
                "female",
                "http://wrong.example|"
            ]
        );
        assert_eq!(
            narrow(
                &registry,
                Some(SearchModifier::Not),
                &["http://wrong.example|female"]
            ),
            [nothing.as_str()]
        );
    }

    #[test]
    fn other_modifiers_and_custom_expressions_are_untouched() {
        let wrong = "http://wrong.example|female";
        assert_eq!(
            narrow(
                &registry(GENDER_EXPRESSION),
                Some(SearchModifier::Text),
                &[wrong]
            ),
            [wrong]
        );
        assert_eq!(
            narrow(&registry("Patient.extension.value"), None, &[wrong]),
            [wrong]
        );
    }
}
