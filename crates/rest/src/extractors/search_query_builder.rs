//! Search query builder.
//!
//! Converts REST search parameters to persistence layer SearchQuery.

use std::collections::HashMap;

use helios_persistence::search::{SearchParameterRegistry, resolve_param_type};
use helios_persistence::types::{
    CompositeSearchComponent, IncludeDirective, IncludeType, ReverseChainedParameter,
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue, SortDirective,
    SummaryMode, TotalMode,
};

use super::SearchParams;
use crate::error::RestError;

/// Builds a SearchQuery from REST parameters.
///
/// This function converts HTTP query parameters into the persistence layer's
/// SearchQuery type, handling:
/// - Parameter modifiers (e.g., `name:exact`)
/// - Value prefixes (e.g., `gt2020-01-01`)
/// - Chained parameters (e.g., `patient.name`)
/// - Reverse chaining (_has parameters)
/// - _include/_revinclude directives
/// - System parameters (_count, _sort, _total, etc.)
pub fn build_search_query(
    resource_type: &str,
    params: &SearchParams,
    registry: &SearchParameterRegistry,
) -> Result<SearchQuery, RestError> {
    let mut query = SearchQuery::new(resource_type);

    // Process system parameters
    if let Some(count) = params.count() {
        query.count = Some(count as u32);
    }

    if let Some(offset) = params.offset() {
        query.offset = Some(offset as u32);
    }

    // Process cursor (_cursor)
    if let Some(cursor) = params.get("_cursor") {
        query.cursor = Some(cursor.clone());
    }

    // Process sort parameters
    if let Some(sort_params) = params.sort() {
        for sort in sort_params {
            let directive = if sort.ascending {
                SortDirective::parse(&sort.field)
            } else {
                SortDirective::parse(&format!("-{}", sort.field))
            };
            // Resolve the search-parameter type so backends can sort by the
            // indexed value column. `_id`/`_lastUpdated` are columns on the
            // resources table and need no type.
            let param_type = if directive.parameter.starts_with('_') {
                None
            } else {
                registry
                    .get_param(resource_type, &directive.parameter)
                    .map(|d| d.param_type)
            };
            query.sort.push(directive.with_param_type(param_type));
        }
    }

    // Process _total
    if let Some(total) = params.total() {
        query.total = parse_total_mode(total);
    }

    // Process _summary
    if let Some(summary) = params.get("_summary") {
        query.summary = parse_summary_mode(summary);
    }

    // Process _elements
    if let Some(elements) = params.elements() {
        query.elements = elements.to_vec();
    }

    // Process _include directives
    for include in params.include() {
        if let Some(directive) = parse_include_directive(include, IncludeType::Include) {
            query.includes.push(directive);
        }
    }

    // Process _revinclude directives
    for revinclude in params.revinclude() {
        if let Some(directive) = parse_include_directive(revinclude, IncludeType::Revinclude) {
            query.includes.push(directive);
        }
    }

    // Store raw parameters for debugging, grouping repeated keys.
    let mut raw_params: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in params.iter() {
        raw_params.entry(k.clone()).or_default().push(v.clone());
    }
    query.raw_params = raw_params;

    // Process search parameters (non-system params)
    for (name, value) in params.search_params() {
        // Handle _has (reverse chaining)
        if name == "_has" || name.starts_with("_has:") {
            if let Some(reverse_chain) = parse_has_parameter(name, value)? {
                query.reverse_chains.push(reverse_chain);
            }
            continue;
        }

        // Parse the parameter
        let param = parse_search_parameter(resource_type, name, value, registry)?;
        query.parameters.push(param);
    }

    Ok(query)
}

/// Builds a SearchQuery from a raw HashMap.
///
/// Convenience function when you don't have a SearchParams instance.
pub fn build_search_query_from_map(
    resource_type: &str,
    params: &HashMap<String, String>,
    registry: &SearchParameterRegistry,
) -> Result<SearchQuery, RestError> {
    let search_params = SearchParams::from_map(params.clone());
    build_search_query(resource_type, &search_params, registry)
}

/// Builds a SearchQuery from ordered key/value pairs.
///
/// Unlike [`build_search_query_from_map`], this preserves repeated parameters
/// (FHIR AND semantics) and multiple `_include`/`_revinclude`/`_has` directives.
pub fn build_search_query_from_pairs(
    resource_type: &str,
    pairs: &[(String, String)],
    registry: &SearchParameterRegistry,
) -> Result<SearchQuery, RestError> {
    let search_params = SearchParams::from_pairs(pairs.to_vec());
    build_search_query(resource_type, &search_params, registry)
}

/// Parses a single search parameter with potential modifiers.
fn parse_search_parameter(
    resource_type: &str,
    name: &str,
    value: &str,
    registry: &SearchParameterRegistry,
) -> Result<SearchParameter, RestError> {
    let (param_name, modifier) = parse_parameter_name(name);

    // Check for chained parameters (e.g., "patient.name" or "subject:Patient.name")
    let (base_name, chain) = parse_chain(param_name);

    // Parse the value(s) - multiple values separated by comma are ORed.
    // Prefix extraction (gt/lt/ge/le/sa/eb/ap/eq/ne) is only meaningful for
    // date/number/quantity types per FHIR. For tokens/strings/references/uris,
    // the raw value is the value — e.g. status code "appended" must not be
    // misread as Ap-prefix + "pended".
    let raw_values: Vec<&str> = value.split(',').map(str::trim).collect();
    let tentative_values: Vec<SearchValue> =
        raw_values.iter().map(|v| SearchValue::parse(v)).collect();

    // Resolve the canonical type from the search parameter registry. This is
    // deterministic for any registered parameter (which is everything in the
    // FHIR spec); the value-shape heuristic is reached only for unregistered
    // custom params. See `helios_persistence::search::resolve_param_type`.
    let param_type = resolve_param_type(registry, resource_type, base_name, &tentative_values);

    // FHIR-spec modifier validation: reject a modifier that is not defined for
    // this parameter's type (e.g. `:exact` on a token, `:contains` on a date)
    // with a 400, rather than silently ignoring it. Scoped to registry-known,
    // non-special params:
    //   * unregistered custom params get a value-shape heuristic type, so
    //     gating them could falsely reject a legitimate custom modifier;
    //   * the `special` full-text params (`_text`, `_content`, …) carry
    //     server-specific modifier semantics outside the typed modifier table.
    // `is_valid_for` reflects what this server honors; combinations that are
    // spec-valid but not yet implemented are enabled as their phase lands.
    if let Some(m) = &modifier {
        let registered = registry.get_param(resource_type, base_name).is_some()
            || registry.get_param("Resource", base_name).is_some();
        if registered && param_type != SearchParamType::Special && !m.is_valid_for(param_type) {
            return Err(RestError::InvalidParameter {
                param: name.to_string(),
                message: format!(
                    "search modifier ':{m}' is not supported for {param_type} parameter '{base_name}'"
                ),
            });
        }
    }

    let values: Vec<SearchValue> = if matches!(
        param_type,
        SearchParamType::Date | SearchParamType::Number | SearchParamType::Quantity
    ) {
        tentative_values
    } else {
        raw_values.iter().map(|v| SearchValue::eq(*v)).collect()
    };

    let mut param = SearchParameter {
        name: base_name.to_string(),
        param_type,
        modifier,
        values,
        chain,
        components: vec![],
    };

    // For composite parameters, resolve the component sub-parameters from the
    // registry (type + code), so the backend can match each component within
    // the same composite instance.
    if matches!(param_type, SearchParamType::Composite) {
        if let Some(def) = registry.get_param(resource_type, base_name) {
            if let Some(comps) = def.component.as_ref() {
                param.components = comps
                    .iter()
                    .filter_map(|c| {
                        registry
                            .get_by_url(&c.definition)
                            .map(|sub| CompositeSearchComponent {
                                param_type: sub.param_type,
                                param_name: sub.code.clone(),
                            })
                    })
                    .collect();
            }
        }
    }

    // Handle :missing modifier specially
    if param
        .modifier
        .as_ref()
        .is_some_and(|m| *m == SearchModifier::Missing)
    {
        // The value should be "true" or "false"
        param.values = vec![SearchValue::eq(value)];
    }

    Ok(param)
}

/// Parses a parameter name into the base name and optional modifier.
///
/// Examples:
/// - "name" -> ("name", None)
/// - "name:exact" -> ("name", Some(Exact))
/// - "subject:Patient" -> ("subject", Some(Type("Patient")))
fn parse_parameter_name(name: &str) -> (&str, Option<SearchModifier>) {
    if let Some(colon_pos) = name.find(':') {
        let param_name = &name[..colon_pos];
        let modifier_str = &name[colon_pos + 1..];

        // Check if there's a chain after the modifier
        // e.g., "subject:Patient.name" -> modifier is "Patient", then chain follows
        let modifier_str = if let Some(dot_pos) = modifier_str.find('.') {
            &modifier_str[..dot_pos]
        } else {
            modifier_str
        };

        let modifier = SearchModifier::parse(modifier_str);
        (param_name, modifier)
    } else {
        (name, None)
    }
}

/// Parses chain elements from a parameter name.
///
/// Examples:
/// - "name" -> ("name", [])
/// - "patient.name" -> ("patient", [ChainedParameter{...}])
/// - "subject:Patient.organization.name" -> ("subject", [ChainedParameter{target_type: Patient, ...}])
fn parse_chain(name: &str) -> (&str, Vec<helios_persistence::types::ChainedParameter>) {
    // Handle type-qualified chains like "subject:Patient.organization.name"
    let (base_with_type, rest) = if let Some(dot_pos) = name.find('.') {
        (&name[..dot_pos], Some(&name[dot_pos + 1..]))
    } else {
        (name, None)
    };

    // Extract type modifier from base if present
    let (base_name, target_type) = if let Some(colon_pos) = base_with_type.find(':') {
        let base = &base_with_type[..colon_pos];
        let type_str = &base_with_type[colon_pos + 1..];
        (base, Some(type_str.to_string()))
    } else {
        (base_with_type, None)
    };

    // No chain if no dot
    let rest = match rest {
        Some(r) => r,
        None => return (base_name, vec![]),
    };

    let mut chain = Vec::new();
    let parts: Vec<&str> = rest.split('.').collect();

    // For simple chains like "patient.name", we need one ChainedParameter
    // For complex chains like "subject.organization.name", we need multiple
    if parts.len() == 1 {
        // Simple chain: patient.name
        chain.push(helios_persistence::types::ChainedParameter {
            reference_param: base_name.to_string(),
            target_type,
            target_param: parts[0].to_string(),
        });
    } else {
        // Complex chain: build step by step
        // For subject.organization.name:
        // 1. reference_param=subject, target_param=organization
        // 2. reference_param=organization, target_param=name
        for i in 0..parts.len() {
            let ref_param = if i == 0 {
                base_name.to_string()
            } else {
                // Get base part of previous (strip any type modifier)
                let prev = parts[i - 1];
                if let Some(colon_pos) = prev.find(':') {
                    prev[..colon_pos].to_string()
                } else {
                    prev.to_string()
                }
            };

            // Check if current part has type qualifier
            let (current_param, part_type) = if let Some(colon_pos) = parts[i].find(':') {
                (
                    parts[i][..colon_pos].to_string(),
                    Some(parts[i][colon_pos + 1..].to_string()),
                )
            } else {
                (parts[i].to_string(), None)
            };

            chain.push(helios_persistence::types::ChainedParameter {
                reference_param: ref_param,
                target_type: if i == 0 {
                    target_type.clone()
                } else {
                    part_type
                },
                target_param: current_param,
            });
        }
    }

    (base_name, chain)
}

/// Parses _has parameter (reverse chaining).
///
/// Format: _has:[type]:[reference-param]:[search-param]=value
/// Examples:
/// - _has:Observation:patient:code=1234-5
/// - Nested: _has:Observation:patient:_has:Provenance:target:agent=practitioner-id
fn parse_has_parameter(
    name: &str,
    value: &str,
) -> Result<Option<ReverseChainedParameter>, RestError> {
    // Handle both _has:... format and _has key with value containing the chain
    let chain_str = if name == "_has" {
        // Value format: Observation:patient:code
        value
    } else if let Some(rest) = name.strip_prefix("_has:") {
        // Name format: _has:Observation:patient:code, value is the search value
        rest
    } else {
        return Ok(None);
    };

    // Split the chain
    let parts: Vec<&str> = chain_str.splitn(4, ':').collect();

    if parts.len() < 3 {
        return Err(RestError::InvalidParameter {
            param: name.to_string(),
            message:
                "Invalid _has format. Expected _has:[type]:[reference-param]:[search-param]=value"
                    .to_string(),
        });
    }

    let source_type = parts[0].to_string();
    let reference_param = parts[1].to_string();
    let search_param = parts[2].to_string();

    // Get the search value
    let search_value = if name == "_has" {
        // For _has=Observation:patient:code:value format
        if parts.len() > 3 {
            SearchValue::eq(parts[3])
        } else {
            return Err(RestError::InvalidParameter {
                param: name.to_string(),
                message: "Missing value for _has parameter".to_string(),
            });
        }
    } else {
        SearchValue::eq(value)
    };

    // Check for nested _has, e.g.
    // `_has:Observation:subject:_has:Provenance:target:agent`.
    // Everything after `source_type:reference_param:` is itself a `_has:...`
    // expression, which we parse recursively.
    if search_param == "_has" {
        let inner = &chain_str[parts[0].len() + parts[1].len() + 2..];
        let nested = parse_has_parameter(inner, value)?;
        if let Some(nested_chain) = nested {
            return Ok(Some(ReverseChainedParameter::nested(
                source_type,
                reference_param,
                nested_chain,
            )));
        }
    }

    Ok(Some(ReverseChainedParameter::terminal(
        source_type,
        reference_param,
        search_param,
        search_value,
    )))
}

/// Parses _include/_revinclude directive.
///
/// Format: [source-type]:[search-param]:[target-type]
/// Or with :iterate modifier: [source-type]:[search-param]:iterate
/// Examples:
/// - Observation:patient
/// - Observation:subject:Patient
/// - Observation:patient:iterate
fn parse_include_directive(directive: &str, include_type: IncludeType) -> Option<IncludeDirective> {
    let parts: Vec<&str> = directive.split(':').collect();

    if parts.is_empty() {
        return None;
    }

    let source_type = parts[0].to_string();
    let search_param = parts.get(1).map(|s| s.to_string()).unwrap_or_default();

    // Check for :iterate modifier or target type
    let (target_type, iterate) = if let Some(third) = parts.get(2) {
        if *third == "iterate" {
            (None, true)
        } else {
            (
                Some(third.to_string()),
                parts.get(3).is_some_and(|s| *s == "iterate"),
            )
        }
    } else {
        (None, false)
    };

    Some(IncludeDirective {
        include_type,
        source_type,
        search_param,
        target_type,
        iterate,
    })
}

/// Parses _total parameter value.
fn parse_total_mode(value: &str) -> Option<TotalMode> {
    match value.to_lowercase().as_str() {
        "none" => Some(TotalMode::None),
        "estimate" => Some(TotalMode::Estimate),
        "accurate" => Some(TotalMode::Accurate),
        _ => None,
    }
}

/// Parses _summary parameter value.
fn parse_summary_mode(value: &str) -> Option<SummaryMode> {
    match value.to_lowercase().as_str() {
        "true" => Some(SummaryMode::True),
        "false" => Some(SummaryMode::False),
        "text" => Some(SummaryMode::Text),
        "data" => Some(SummaryMode::Data),
        "count" => Some(SummaryMode::Count),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_persistence::search::SearchParameterDefinition;
    use helios_persistence::types::SearchParamType;

    /// Builds a small registry covering just the params the tests exercise.
    /// Keeps tests hermetic without depending on the embedded data set.
    fn test_registry() -> SearchParameterRegistry {
        let mut r = SearchParameterRegistry::new();
        let entries = [
            ("Patient-name", "name", SearchParamType::String, "Patient"),
            (
                "Patient-birthdate",
                "birthdate",
                SearchParamType::Date,
                "Patient",
            ),
            (
                "Observation-code",
                "code",
                SearchParamType::Token,
                "Observation",
            ),
            (
                "Observation-patient",
                "patient",
                SearchParamType::Reference,
                "Observation",
            ),
            (
                "Observation-subject",
                "subject",
                SearchParamType::Reference,
                "Observation",
            ),
            (
                "Observation-date",
                "date",
                SearchParamType::Date,
                "Observation",
            ),
        ];
        for (id, code, ty, base) in entries {
            r.register(
                SearchParameterDefinition::new(
                    format!("http://hl7.org/fhir/SearchParameter/{}", id),
                    code,
                    ty,
                    "ignored",
                )
                .with_base(vec![base]),
            )
            .unwrap();
        }
        r
    }

    #[test]
    fn test_parse_parameter_name_simple() {
        let (name, modifier) = parse_parameter_name("name");
        assert_eq!(name, "name");
        assert!(modifier.is_none());
    }

    #[test]
    fn test_parse_parameter_name_with_modifier() {
        let (name, modifier) = parse_parameter_name("name:exact");
        assert_eq!(name, "name");
        assert_eq!(modifier, Some(SearchModifier::Exact));
    }

    #[test]
    fn test_parse_parameter_name_with_type_modifier() {
        let (name, modifier) = parse_parameter_name("subject:Patient");
        assert_eq!(name, "subject");
        assert_eq!(modifier, Some(SearchModifier::Type("Patient".to_string())));
    }

    #[test]
    fn test_parse_chain_simple() {
        let (name, chain) = parse_chain("patient.name");
        assert_eq!(name, "patient");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].reference_param, "patient");
        assert_eq!(chain[0].target_param, "name");
    }

    #[test]
    fn test_parse_chain_with_type() {
        let (name, chain) = parse_chain("subject:Patient.name");
        assert_eq!(name, "subject");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].reference_param, "subject");
        assert_eq!(chain[0].target_type, Some("Patient".to_string()));
        assert_eq!(chain[0].target_param, "name");
    }

    #[test]
    fn test_parse_chain_multi_level() {
        let (name, chain) = parse_chain("subject.organization.name");
        assert_eq!(name, "subject");
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].reference_param, "subject");
        assert_eq!(chain[0].target_param, "organization");
        assert_eq!(chain[1].reference_param, "organization");
        assert_eq!(chain[1].target_param, "name");
    }

    #[test]
    fn test_parse_include_directive() {
        let directive = parse_include_directive("Observation:patient", IncludeType::Include);
        assert!(directive.is_some());
        let dir = directive.unwrap();
        assert_eq!(dir.source_type, "Observation");
        assert_eq!(dir.search_param, "patient");
        assert!(dir.target_type.is_none());
        assert!(!dir.iterate);
    }

    #[test]
    fn test_parse_include_directive_with_target() {
        let directive =
            parse_include_directive("Observation:subject:Patient", IncludeType::Include);
        assert!(directive.is_some());
        let dir = directive.unwrap();
        assert_eq!(dir.source_type, "Observation");
        assert_eq!(dir.search_param, "subject");
        assert_eq!(dir.target_type, Some("Patient".to_string()));
    }

    #[test]
    fn test_parse_include_directive_with_iterate() {
        let directive =
            parse_include_directive("Observation:patient:iterate", IncludeType::Include);
        assert!(directive.is_some());
        let dir = directive.unwrap();
        assert!(dir.iterate);
    }

    #[test]
    fn test_parse_has_parameter() {
        let result = parse_has_parameter("_has:Observation:patient:code", "8867-4").unwrap();
        assert!(result.is_some());
        let chain = result.unwrap();
        assert_eq!(chain.source_type, "Observation");
        assert_eq!(chain.reference_param, "patient");
        assert_eq!(chain.search_param, "code");
    }

    #[test]
    fn test_parse_nested_has_parameter() {
        // _has:Observation:subject:_has:Provenance:target:agent=prac-1
        let result = parse_has_parameter(
            "_has:Observation:subject:_has:Provenance:target:agent",
            "prac-1",
        )
        .unwrap()
        .expect("nested chain parsed");

        // Outer level.
        assert_eq!(result.source_type, "Observation");
        assert_eq!(result.reference_param, "subject");
        assert!(result.value.is_none(), "outer level carries no value");
        assert_eq!(result.depth(), 2);

        // Inner (terminal) level.
        let inner = result.nested.expect("inner chain present");
        assert_eq!(inner.source_type, "Provenance");
        assert_eq!(inner.reference_param, "target");
        assert_eq!(inner.search_param, "agent");
        assert_eq!(
            inner.value.as_ref().map(|v| v.value.as_str()),
            Some("prac-1")
        );
        assert!(inner.is_terminal());
    }

    #[test]
    fn test_build_search_query_basic() {
        let mut params = HashMap::new();
        params.insert("name".to_string(), "Smith".to_string());
        params.insert("_count".to_string(), "10".to_string());

        let search_params = SearchParams::from_map(params);
        let query = build_search_query("Patient", &search_params, &test_registry()).unwrap();

        assert_eq!(query.resource_type, "Patient");
        assert_eq!(query.count, Some(10));
        assert_eq!(query.parameters.len(), 1);
        assert_eq!(query.parameters[0].name, "name");
    }

    #[test]
    fn test_build_search_query_with_modifier() {
        let mut params = HashMap::new();
        params.insert("name:exact".to_string(), "Smith".to_string());

        let search_params = SearchParams::from_map(params);
        let query = build_search_query("Patient", &search_params, &test_registry()).unwrap();

        assert_eq!(query.parameters.len(), 1);
        assert_eq!(query.parameters[0].name, "name");
        assert_eq!(query.parameters[0].modifier, Some(SearchModifier::Exact));
    }

    #[test]
    fn test_build_search_query_with_prefix() {
        let mut params = HashMap::new();
        params.insert("birthdate".to_string(), "gt2000-01-01".to_string());

        let search_params = SearchParams::from_map(params);
        let query = build_search_query("Patient", &search_params, &test_registry()).unwrap();

        assert_eq!(query.parameters.len(), 1);
        assert_eq!(
            query.parameters[0].values[0].prefix,
            helios_persistence::types::SearchPrefix::Gt
        );
        assert_eq!(query.parameters[0].values[0].value, "2000-01-01");
    }

    #[test]
    fn test_token_value_starting_with_prefix_keyword_keeps_full_value() {
        // Regression: status code "appended" used to be parsed as Ap-prefix +
        // "pended", which made strict backends (MongoDB) reject the query
        // with 400. Token values must not extract a prefix.
        let mut registry = SearchParameterRegistry::new();
        registry
            .register(
                helios_persistence::search::SearchParameterDefinition::new(
                    "http://hl7.org/fhir/SearchParameter/DiagnosticReport-status",
                    "status",
                    SearchParamType::Token,
                    "ignored",
                )
                .with_base(vec!["DiagnosticReport"]),
            )
            .unwrap();

        let param = parse_search_parameter(
            "DiagnosticReport",
            "status",
            "registered,partial,appended,entered-in-error",
            &registry,
        )
        .unwrap();

        assert_eq!(param.values.len(), 4);
        for v in &param.values {
            assert_eq!(v.prefix, helios_persistence::types::SearchPrefix::Eq);
        }
        assert_eq!(param.values[0].value, "registered");
        assert_eq!(param.values[1].value, "partial");
        assert_eq!(param.values[2].value, "appended");
        assert_eq!(param.values[3].value, "entered-in-error");
    }

    #[test]
    fn test_build_search_query_with_sort() {
        let mut params = HashMap::new();
        params.insert("_sort".to_string(), "-date,name".to_string());

        let search_params = SearchParams::from_map(params);
        let query = build_search_query("Observation", &search_params, &test_registry()).unwrap();

        assert_eq!(query.sort.len(), 2);
        assert_eq!(query.sort[0].parameter, "date");
        assert_eq!(
            query.sort[0].direction,
            helios_persistence::types::SortDirection::Descending
        );
        assert_eq!(query.sort[1].parameter, "name");
        assert_eq!(
            query.sort[1].direction,
            helios_persistence::types::SortDirection::Ascending
        );
    }

    #[test]
    fn test_build_search_query_with_include() {
        let mut params = HashMap::new();
        params.insert("_include".to_string(), "Observation:patient".to_string());

        let search_params = SearchParams::from_map(params);
        let query = build_search_query("Observation", &search_params, &test_registry()).unwrap();

        assert_eq!(query.includes.len(), 1);
        assert_eq!(query.includes[0].source_type, "Observation");
        assert_eq!(query.includes[0].search_param, "patient");
    }

    #[test]
    fn test_repeated_search_params_are_anded() {
        // `?name=Smith&name=Jones` -> two ANDed parameters, both preserved.
        let pairs = vec![
            ("name".to_string(), "Smith".to_string()),
            ("name".to_string(), "Jones".to_string()),
        ];
        let sp = SearchParams::from_pairs(pairs);
        let query = build_search_query("Patient", &sp, &test_registry()).unwrap();

        let name_values: Vec<&str> = query
            .parameters
            .iter()
            .filter(|p| p.name == "name")
            .flat_map(|p| p.values.iter().map(|v| v.value.as_str()))
            .collect();
        assert_eq!(query.parameters.len(), 2, "both name params preserved");
        assert!(name_values.contains(&"Smith"));
        assert!(name_values.contains(&"Jones"));
    }

    #[test]
    fn test_repeated_include_directives_preserved() {
        // `?_include=A&_include=B` -> two include directives (was last-wins before).
        let pairs = vec![
            ("_include".to_string(), "Observation:subject".to_string()),
            ("_include".to_string(), "Observation:encounter".to_string()),
        ];
        let sp = SearchParams::from_pairs(pairs);
        let query = build_search_query("Observation", &sp, &test_registry()).unwrap();

        assert_eq!(query.includes.len(), 2);
        let params: Vec<&str> = query
            .includes
            .iter()
            .map(|d| d.search_param.as_str())
            .collect();
        assert!(params.contains(&"subject"));
        assert!(params.contains(&"encounter"));
    }

    #[test]
    fn test_repeated_has_chains_preserved() {
        // Repeated `_has` -> multiple reverse chains, all preserved.
        let pairs = vec![
            (
                "_has:Observation:patient:code".to_string(),
                "1234-5".to_string(),
            ),
            (
                "_has:Observation:patient:status".to_string(),
                "final".to_string(),
            ),
        ];
        let sp = SearchParams::from_pairs(pairs);
        let query = build_search_query("Patient", &sp, &test_registry()).unwrap();

        assert_eq!(query.reverse_chains.len(), 2);
    }

    #[test]
    fn test_mixed_comma_and_repeat_include() {
        // Comma and repeat combine: `?_include=A,B&_include=C` -> three directives.
        let pairs = vec![
            (
                "_include".to_string(),
                "Observation:subject,Observation:encounter".to_string(),
            ),
            ("_include".to_string(), "Observation:patient".to_string()),
        ];
        let sp = SearchParams::from_pairs(pairs);
        let query = build_search_query("Observation", &sp, &test_registry()).unwrap();
        assert_eq!(query.includes.len(), 3);
    }

    #[test]
    fn test_param_type_resolved_from_registry() {
        let registry = test_registry();
        let name = parse_search_parameter("Patient", "name", "Smith", &registry).unwrap();
        assert_eq!(name.param_type, SearchParamType::String);

        let code = parse_search_parameter("Observation", "code", "1234-5", &registry).unwrap();
        assert_eq!(code.param_type, SearchParamType::Token);

        let patient =
            parse_search_parameter("Observation", "patient", "Patient/1", &registry).unwrap();
        assert_eq!(patient.param_type, SearchParamType::Reference);
    }

    #[test]
    fn test_param_type_unregistered_falls_back_to_value_shape() {
        // No entry for "made-up" anywhere — value heuristic kicks in.
        let registry = SearchParameterRegistry::new();
        let date_param =
            parse_search_parameter("Custom", "made-up", "2020-01-01", &registry).unwrap();
        assert_eq!(date_param.param_type, SearchParamType::Date);

        let plain = parse_search_parameter("Custom", "made-up", "hello", &registry).unwrap();
        assert_eq!(plain.param_type, SearchParamType::String);
    }
}
