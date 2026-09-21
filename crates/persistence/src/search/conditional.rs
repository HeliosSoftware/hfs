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
//! # Encoding
//!
//! The criteria string is the query portion of a search URL, **as it appears
//! on the wire**: `application/x-www-form-urlencoded`, like any search query.
//! That is what `If-None-Exist` and `Bundle.entry.request.ifNoneExist` are
//! defined to carry, and what the REST layer hands over untouched for a
//! conditional URL. It is decoded exactly once, in
//! [`parse_conditional_criteria`], with the parser direct search uses — after
//! the split into pairs, so a decoded `&`, `=` or `+` inside a value stays
//! inside it (#1322). A caller that already holds decoded pairs uses
//! [`build_conditional_query_from_pairs`]; it must never join them into a
//! string for this module to split again.

use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{
    CompositeSearchComponent, SearchModifier, SearchParamType, SearchParameter, SearchQuery,
    SearchValue,
};

use super::registry::{SearchParameterRegistry, fallback_param_type};
use super::type_qualifier::ResourceTypeScope;
use super::value_parser::{
    param_requires_terminology, parse_typed_values, split_unescaped_commas, validate_modifier,
};

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

/// Resource-level parameters that are criteria even when the tenant's registry
/// has no definition for them (a backend built without the spec files registers
/// only a handful): the ones [`fallback_param_type`] can place in the index
/// column the extractor writes them under. Every other name has to be
/// registered for the resource type, or for `Resource`.
const ALWAYS_INDEXED_PARAMS: &[&str] = &[
    "_id",
    "_lastUpdated",
    "_tag",
    "_profile",
    "_security",
    "_source",
];

/// Splits form-urlencoded conditional criteria into decoded `(name, value)`
/// pairs.
///
/// The decoding is `form_urlencoded`'s, the parser the REST layer reads a
/// search query with: `%XX` escapes are resolved and `+` is a space, in names
/// and values alike, *after* the split on `&` and `=` — so
/// `identifier=http%3A%2F%2Fexample.org%7C123` names `http://example.org|123`,
/// and an encoded `&` or `=` is part of its value. `%2C` decodes to a comma,
/// which then separates OR alternatives exactly as a literal one does; a comma
/// that belongs to the value is escaped the FHIR way, `\,`.
///
/// Pairs with an empty name are dropped. A pair with an empty value — `name=`,
/// or a bare `name` with no `=` at all, which is the same thing to a form
/// parser — is **kept**, so that [`build_conditional_parameters`] can refuse
/// it: dropping it here silently widened the precondition of a write (#1360).
/// Repeated names are kept, in order: FHIR ANDs them.
pub fn parse_conditional_criteria(criteria: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(criteria.as_bytes())
        .filter_map(|(name, value)| {
            let (name, value) = (name.trim(), value.trim());
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

/// Refuses a criterion that carries no value: `identifier=`, a bare
/// `identifier`, or an OR-list with an empty alternative (`identifier=,`,
/// `family=Jones,`).
///
/// This is what an unset template variable renders as (`identifier={{mrn}}`).
/// Dropping the pair leaves the *other* criteria to decide the match — `PUT
/// Patient?identifier=&family=Jones` overwrote whichever Jones there was — and,
/// when it was the only criterion, leaves none, so a conditional create went
/// ahead unguarded. An empty alternative is no better: a string search for the
/// prefix `""` matches every value. Direct search evaluates the empty value
/// (an empty token finds nothing); a read can afford that. As for an unknown
/// parameter (#1323), `Prefer: handling` is not consulted.
///
/// Result parameters ([`RESULT_PARAMS`]) are no criteria and are exempt.
/// [`build_conditional_parameters`] applies this to every pair; a resolver that
/// reads the parsed pairs without going through it has to call this itself.
pub fn reject_empty_criterion_values(pairs: &[(String, String)]) -> StorageResult<()> {
    for (raw_name, raw_value) in pairs {
        if RESULT_PARAMS.contains(&raw_name.as_str()) {
            continue;
        }
        if split_unescaped_commas(raw_value)
            .iter()
            .any(|alternative| alternative.is_empty())
        {
            return Err(empty_value(raw_name));
        }
    }
    Ok(())
}

/// Builds the typed search parameters a list of criteria pairs describes.
///
/// Result-shaping parameters ([`RESULT_PARAMS`]: `_format`, `_count`, …) are
/// skipped. Criteria this layer cannot evaluate — chained parameters, `_has`,
/// `_list` — are refused rather than searched for under their literal name,
/// which would match nothing and quietly turn the interaction into an
/// unconditional one.
///
/// # Modifiers
///
/// A `:modifier` is held to direct search's rules — a known modifier or a
/// resource type in `types` ([`ResourceTypeScope`], the rule and the error text
/// direct search uses), and [`validate_modifier`] for the parameter's type — and a
/// modifier only a terminology server can answer
/// ([`param_requires_terminology`]) is refused as not supported, since nothing
/// expands conditional criteria.
///
/// # Unknown parameters
///
/// A criterion whose parameter the tenant's registry does not define for
/// `resource_type` (or for `Resource`) is an error, which REST answers with a
/// `400` naming it (#1323). A search may ignore a parameter it does not know —
/// that is what `Prefer: handling=lenient` asks for — but these criteria are the
/// precondition of a *write*, and neither way of carrying on is safe: searching
/// for the unknown name matches nothing, so `If-None-Exist: identifer=123`
/// creates the duplicate it exists to prevent and a conditional update creates
/// instead of updating; ignoring it widens the match, so the update or delete
/// lands on a resource the client did not name. FHIR defines `Prefer: handling`
/// for searches and is silent on conditional interactions, so the header is
/// deliberately not consulted.
pub fn build_conditional_parameters(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    pairs: &[(String, String)],
    types: ResourceTypeScope,
) -> StorageResult<Vec<SearchParameter>> {
    // First, so that `_has=`, `a.b=` and `x:missing=` are all reported as what
    // they are: a criterion without a value (#1360).
    reject_empty_criterion_values(pairs)?;

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
                let modifier = types.parse_modifier(modifier_str).ok_or_else(|| {
                    query_error(types.unknown_modifier_message(modifier_str, name))
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
        if definition.is_none() && !ALWAYS_INDEXED_PARAMS.contains(&name) {
            return Err(unknown_parameter(resource_type, name));
        }
        let raw_values = split_unescaped_commas(raw_value);

        let (param_type, values) = match &definition {
            // A registry miss on a parameter every resource is indexed under
            // takes the fallback table's type: it records which index column
            // the extractor writes those rows under (see
            // `fallback_param_type`).
            None => {
                let fallback = fallback_param_type(name);
                (fallback, values_for_type(fallback, &raw_values))
            }
            // Registered parameters resolve deterministically.
            Some(_) => parse_typed_values(registry, resource_type, name, &raw_values),
        };

        if let Some(m) = &modifier {
            // The rules direct search and the chain resolver apply.
            validate_modifier(registry, resource_type, name, param_type, m).map_err(query_error)?;
            // Direct search hands `:in` and token `:above` / `:below` to a
            // terminology server before it builds the query. Nothing expands
            // conditional criteria, and a backend given the bare modifier
            // matches the value-set URL or the code literally — so the
            // precondition would silently mean something else.
            if param_requires_terminology(registry, resource_type, name, m) {
                return Err(StorageError::Search(SearchError::IncludeNotSupported {
                    operation: format!(
                        "search modifier ':{m}' on conditional criterion '{raw_name}' needs \
                         terminology expansion, which conditional criteria do not get; it is"
                    ),
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
///
/// `criteria` is form-urlencoded (see the [module notes](self#encoding)).
/// `types` is the FHIR version the criteria are searched in — the one the
/// backend's index and registry were built for — so a `:[type]` qualifier
/// cannot name a type only another enabled version has (#1366).
pub fn build_conditional_query(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    criteria: &str,
    types: ResourceTypeScope,
) -> StorageResult<Option<SearchQuery>> {
    let pairs = parse_conditional_criteria(criteria);
    build_conditional_query_from_pairs(registry, resource_type, &pairs, types)
}

/// [`build_conditional_query`] for criteria that are already decoded
/// `(name, value)` pairs.
pub fn build_conditional_query_from_pairs(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    pairs: &[(String, String)],
    types: ResourceTypeScope,
) -> StorageResult<Option<SearchQuery>> {
    let parameters = build_conditional_parameters(registry, resource_type, pairs, types)?;
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

fn unknown_parameter(resource_type: &str, name: &str) -> StorageError {
    query_error(format!(
        "conditional criteria name the search parameter '{name}', which is not known for \
         {resource_type}. Criteria guard a write, so an unknown parameter is an error rather \
         than ignored (Prefer: handling does not apply); nothing was written"
    ))
}

fn empty_value(raw_name: &str) -> StorageError {
    query_error(format!(
        "conditional criterion '{raw_name}' has no value, or an empty alternative in its \
         comma-separated list. Criteria guard a write, so it is an error rather than dropped, \
         which would widen the match (Prefer: handling does not apply); nothing was written"
    ))
}

fn query_error(message: String) -> StorageError {
    StorageError::Search(SearchError::QueryParseError { message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::registry::SearchParameterDefinition;
    use crate::types::SearchPrefix;

    /// The builders under test, judged against the build's default FHIR
    /// version as a backend judges them against its configured one.
    fn scope() -> ResourceTypeScope {
        ResourceTypeScope::version(helios_fhir::FhirVersion::default_enabled())
    }

    fn build_conditional_query(
        registry: &SearchParameterRegistry,
        resource_type: &str,
        criteria: &str,
    ) -> StorageResult<Option<SearchQuery>> {
        super::build_conditional_query(registry, resource_type, criteria, scope())
    }

    fn build_conditional_query_from_pairs(
        registry: &SearchParameterRegistry,
        resource_type: &str,
        pairs: &[(String, String)],
    ) -> StorageResult<Option<SearchQuery>> {
        super::build_conditional_query_from_pairs(registry, resource_type, pairs, scope())
    }

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

        let param = one("general-practitioner:Practitioner=p1");
        assert_eq!(
            param.modifier,
            Some(SearchModifier::Type("Practitioner".to_string()))
        );

        let registry = registry();
        for criteria in [
            "identifier:exact=ne123",
            "family:bogus=Neal",
            "family:missing=maybe",
            // A capitalised suffix is a type qualifier only if it is a type.
            "general-practitioner:Bogus=p1",
            "general-practitioner:practitioner=p1",
            // No presence row to test `:missing` against.
            "_text:missing=true",
        ] {
            assert!(
                matches!(
                    build_conditional_query(&registry, "Patient", criteria),
                    Err(StorageError::Search(SearchError::QueryParseError { .. }))
                ),
                "{criteria} must be refused"
            );
        }
    }

    #[test]
    fn terminology_backed_modifiers_are_not_searched_literally() {
        let registry = registry();
        for criteria in [
            "identifier:in=http://example.org/ValueSet/mrns",
            "identifier:not-in=http://example.org/ValueSet/mrns",
            "identifier:below=123",
            "identifier:above=123",
        ] {
            assert!(
                matches!(
                    build_conditional_query(&registry, "Patient", criteria),
                    Err(StorageError::Search(
                        SearchError::IncludeNotSupported { .. }
                    ))
                ),
                "{criteria} must be refused as not supported"
            );
        }
        // On a reference, `:below` is structural and resolved natively.
        assert_eq!(
            one("general-practitioner:below=Practitioner/p1").modifier,
            Some(SearchModifier::Below)
        );
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

        // The whole list, pinned: a name dropped from it becomes an unknown
        // parameter (a 400); a name added to it stops being a criterion.
        let expected = [
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
        assert_eq!(RESULT_PARAMS, expected);
        for name in expected {
            assert!(
                build_conditional_query(&registry, "Patient", &format!("{name}=x"))
                    .expect(name)
                    .is_none(),
                "{name}"
            );
        }

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
    fn unknown_parameters_are_refused_by_name() {
        let registry = registry();
        for (criteria, name) in [
            ("identifer=123", "identifer"),
            ("identifer:exact=123", "identifer"),
            ("identifier=123&nickname=Lee", "nickname"),
            // Case matters, as it does in a search.
            ("Identifier=123", "Identifier"),
            // The fallback table's bare-name heuristics (`patient`, `subject`,
            // …) do not make a parameter known for this type.
            ("subject=Patient/1", "subject"),
            // Not a result parameter, and nothing this layer evaluates.
            ("_type=Patient", "_type"),
            ("_query=mine", "_query"),
        ] {
            let error = build_conditional_query(&registry, "Patient", criteria)
                .expect_err(criteria)
                .to_string();
            assert!(error.contains(&format!("'{name}'")), "{criteria}: {error}");
        }
    }

    #[test]
    fn resource_level_parameters_need_no_registration() {
        // `registry()` defines none of them.
        for (criteria, param_type) in [
            ("_id=p1", SearchParamType::Token),
            ("_tag=gold", SearchParamType::Token),
            ("_security=R", SearchParamType::Token),
            ("_profile=http://example.org/p", SearchParamType::Uri),
            ("_source=http://example.org/feed", SearchParamType::Uri),
            ("_lastUpdated=gt2020-01-01", SearchParamType::Date),
        ] {
            assert_eq!(one(criteria).param_type, param_type, "{criteria}");
        }
    }

    #[test]
    fn criteria_are_form_urlencoded() {
        fn pairs(criteria: &str) -> Vec<(String, String)> {
            parse_conditional_criteria(criteria)
        }
        fn pair(name: &str, value: &str) -> (String, String) {
            (name.to_string(), value.to_string())
        }

        assert_eq!(
            pairs("identifier=http%3A%2F%2Fexample.org%7C123"),
            vec![pair("identifier", "http://example.org|123")]
        );
        // Unencoded criteria read as before.
        assert_eq!(
            pairs("identifier=http://example.org|123&family=Neal"),
            vec![
                pair("identifier", "http://example.org|123"),
                pair("family", "Neal")
            ]
        );
        // Decoding follows the split, so an encoded `&` or `=` is data; a
        // literal `=` after the first is data too.
        assert_eq!(
            pairs("identifier=http://x?a%3D1%26b=2|v&family%3Aexact=Neal"),
            vec![
                pair("identifier", "http://x?a=1&b=2|v"),
                pair("family:exact", "Neal")
            ]
        );
        // `+` is a space, `%2B` a plus — as in a search URL.
        assert_eq!(
            pairs("family=Mary+Ann&given=a%2Bb"),
            vec![pair("family", "Mary Ann"), pair("given", "a+b")]
        );
        // Nothing at all.
        for criteria in ["", "&", "=Neal", "=", "&&"] {
            assert!(pairs(criteria).is_empty(), "{criteria:?}");
        }
        // A name without a value is kept, for the builder to refuse (#1360).
        for criteria in ["family", "family=", "family=+", "family=%20"] {
            assert_eq!(pairs(criteria), vec![pair("family", "")], "{criteria:?}");
        }
    }

    #[test]
    fn a_criterion_without_a_value_is_refused_not_dropped() {
        for (criteria, named) in [
            ("identifier=", "'identifier'"),
            ("identifier", "'identifier'"),
            ("identifier=&family=Neal", "'identifier'"),
            ("family=Neal&identifier", "'identifier'"),
            ("identifier=+", "'identifier'"),
            ("identifier=,", "'identifier'"),
            ("family=Neal,", "'family'"),
            ("family=,Neal", "'family'"),
            ("identifier:missing=", "'identifier:missing'"),
            ("_id=", "'_id'"),
            // Reported as empty, not as a chain / `_has` / unknown parameter.
            ("general-practitioner.name=", "'general-practitioner.name'"),
            (
                "_has:Observation:patient:code=",
                "'_has:Observation:patient:code'",
            ),
            ("nonsense=", "'nonsense'"),
        ] {
            let message = build_conditional_query(&registry(), "Patient", criteria)
                .expect_err(criteria)
                .to_string();
            assert!(message.contains(named), "{criteria}: {message}");
            assert!(message.contains("no value"), "{criteria}: {message}");
        }

        // An escaped comma is data, not an empty alternative.
        assert_eq!(one("family=Neal\\,").values[0].value, "Neal,");
        // `:missing` carries a value.
        assert_eq!(
            one("identifier:missing=true").modifier,
            Some(SearchModifier::Missing)
        );
        // Result parameters are no criteria, with or without a value.
        assert_eq!(one("_format=&family=Neal&_pretty").name, "family");
        for criteria in ["_format=", "_format", "_count=&_summary"] {
            assert!(
                build_conditional_query(&registry(), "Patient", criteria)
                    .expect("valid")
                    .is_none(),
                "{criteria}"
            );
        }
    }

    #[test]
    fn an_encoded_comma_separates_alternatives_like_a_literal_one() {
        for criteria in ["family=Neal%2CLevine", "family=Neal,Levine"] {
            let values: Vec<String> = one(criteria).values.into_iter().map(|v| v.value).collect();
            assert_eq!(values, vec!["Neal", "Levine"], "{criteria}");
        }
        for criteria in ["family=Neal%5C%2CLevine", "family=Neal\\,Levine"] {
            let param = one(criteria);
            assert_eq!(param.values.len(), 1, "{criteria}");
            assert_eq!(param.values[0].value, "Neal,Levine", "{criteria}");
        }
    }

    #[test]
    fn decoded_pairs_are_never_split_again() {
        let pairs = vec![("identifier".to_string(), "a&family=Wilson".to_string())];
        let query = build_conditional_query_from_pairs(&registry(), "Patient", &pairs)
            .expect("valid")
            .expect("some");
        assert_eq!(query.parameters.len(), 1);
        assert_eq!(query.parameters[0].values[0].value, "a&family=Wilson");
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

    /// The rule and the text are direct search's (`ResourceTypeScope`), so a
    /// conditional criterion is refused in the words a search would be (#1366).
    #[test]
    fn type_qualifiers_share_direct_searchs_rule_and_message() {
        let version = helios_fhir::FhirVersion::default_enabled();
        for (criteria, expected) in [
            (
                "general-practitioner:Bogus=p1",
                format!(
                    "unknown search modifier ':Bogus' on parameter 'general-practitioner'; it is \
                     neither a search modifier nor a resource type of FHIR {version}"
                ),
            ),
            (
                "general-practitioner:practitioner=p1",
                "(modifiers and resource type names are case-sensitive: ':Practitioner'?)"
                    .to_string(),
            ),
            (
                "family:EXACT=Neal",
                "(modifiers and resource type names are case-sensitive: ':exact'?)".to_string(),
            ),
        ] {
            match build_conditional_query(&registry(), "Patient", criteria) {
                Err(StorageError::Search(SearchError::QueryParseError { message })) => {
                    assert!(message.contains(&expected), "{criteria}: {message}")
                }
                other => panic!("{criteria} must be refused, got {other:?}"),
            }
        }
    }

    /// Only a multi-version build can tell the versions apart.
    #[cfg(all(feature = "R4", feature = "R5"))]
    #[test]
    fn a_type_qualifier_of_another_enabled_version_is_refused() {
        use helios_fhir::FhirVersion;
        // ActorDefinition is new in R5; DocumentManifest did not survive R4B.
        for (criteria, ok, other) in [
            (
                "general-practitioner:ActorDefinition=a1",
                FhirVersion::R5,
                FhirVersion::R4,
            ),
            (
                "general-practitioner:DocumentManifest=d1",
                FhirVersion::R4,
                FhirVersion::R5,
            ),
        ] {
            let build = |v| {
                super::build_conditional_query(
                    &registry(),
                    "Patient",
                    criteria,
                    ResourceTypeScope::version(v),
                )
            };
            assert!(build(ok).is_ok(), "{criteria} is valid in {ok}");
            match build(other) {
                Err(StorageError::Search(SearchError::QueryParseError { message })) => assert!(
                    message.contains(&format!("nor a resource type of FHIR {other}")),
                    "{message}"
                ),
                result => panic!("{criteria} must be refused in {other}, got {result:?}"),
            }
        }
    }
}
