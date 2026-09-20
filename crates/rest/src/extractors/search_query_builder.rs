//! Search query builder.
//!
//! Converts REST search parameters to persistence layer SearchQuery.

use std::collections::HashMap;

use helios_persistence::search::{
    SearchParameterRegistry, parse_typed_values, split_unescaped_commas, validate_modifier,
};
use helios_persistence::types::{
    CompositeSearchComponent, ContainedMode, ContainedReturn, IncludeDirective, IncludeType,
    ReverseChainedParameter, SearchModifier, SearchParamType, SearchParameter, SearchQuery,
    SearchValue, SortDirective, SummaryMode, TotalMode,
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
        // `_summary=count` exists to return Bundle.total and nothing else, so
        // it implies an accurate total; an explicit `_total` still wins (#254).
        if query.summary == Some(SummaryMode::Count) && query.total.is_none() {
            query.total = Some(TotalMode::Accurate);
        }
    }

    // Process _elements
    if let Some(elements) = params.elements() {
        query.elements = elements.to_vec();
    }

    // Process _contained / _containedType
    if let Some(v) = params.get("_contained") {
        query.contained = match v.as_str() {
            "false" | "" => ContainedMode::Off,
            "true" => ContainedMode::On,
            "both" => ContainedMode::Both,
            other => {
                return Err(RestError::InvalidParameter {
                    param: "_contained".to_string(),
                    message: format!(
                        "invalid _contained value '{other}' (expected 'true', 'false', or 'both')"
                    ),
                });
            }
        };
    }
    if let Some(v) = params.get("_containedType") {
        query.contained_return = match v.as_str() {
            "container" | "" => ContainedReturn::Container,
            "contained" => ContainedReturn::Contained,
            other => {
                return Err(RestError::InvalidParameter {
                    param: "_containedType".to_string(),
                    message: format!(
                        "invalid _containedType value '{other}' (expected 'container' or 'contained')"
                    ),
                });
            }
        };
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

    // Expand wildcard includes (_include=Type:*) into one directive per
    // reference search parameter of the source type.
    expand_wildcard_includes(&mut query.includes, registry);

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

        // Handle _list: restrict results to members of the referenced List
        // resource(s). Resolved application-side into an `_id` filter. A value
        // may be a bare logical id (`42`) or a `List/42` reference; both are
        // accepted and normalized to the logical id by the resolver.
        if name == "_list" {
            if !value.is_empty() {
                query.list.push(value.to_string());
            }
            continue;
        }

        // Parse the parameter
        let param = parse_search_parameter(resource_type, name, value, registry)?;
        query.parameters.push(param);
    }

    // Reject a date value that is not a date here, before `_list` and chain
    // resolution run, with the one grammar every storage backend enforces
    // again at its own gate (that gate is what covers conditional operations
    // and chain terminals, which never pass through this function). Backends
    // used to disagree on such a value: PostgreSQL searched for the current
    // time instead (#1289), Elasticsearch for the year 2000 (#1293), and
    // SQLite rolled `2024-02-30` over into March (#1295).
    helios_persistence::search::validate_date_values(&query)?;

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

/// The `_`-prefixed global/result parameters this server actually honours.
///
/// Every other underscore name is unknown, exactly like a misspelled ordinary
/// parameter: a blanket `starts_with('_')` bypass would wave through `_typo`
/// and any future spec parameter this server has not implemented, so
/// `Prefer: handling=strict` could never reject one and the self link would
/// claim it had been applied (see issue #524).
///
/// Result/control parameters (`_count`, `_sort`, `_format`, …) are already
/// filtered out of [`SearchParams::search_params`], but are listed here too so
/// the set stays a complete statement of what the server honours and the check
/// is correct for any caller.
///
/// `_query` is deliberately absent — named queries are not implemented, and
/// [`crate::handlers`] rejects them outright.
pub const GLOBAL_SEARCH_PARAMS: &[&str] = &[
    // Filters over resource metadata / content.
    "_id",
    "_lastUpdated",
    "_tag",
    "_profile",
    "_security",
    "_source",
    "_text",
    "_content",
    "_filter",
    "_list",
    "_has",
    "_type",
    // Result-shaping parameters.
    "_contained",
    "_containedType",
    "_include",
    "_revinclude",
    "_sort",
    "_count",
    "_offset",
    "_cursor",
    "_total",
    "_summary",
    "_elements",
    "_score",
    "_format",
    "_pretty",
];

/// Returns the names of search parameters that are not recognized for the given
/// resource type.
///
/// A parameter is "unknown" when its base name (after stripping any modifier and
/// chain) is neither one of the [`GLOBAL_SEARCH_PARAMS`] this server honours nor
/// registered for `resource_type` or for `Resource`.
///
/// Per FHIR search, an unsupported parameter may be ignored only under lenient
/// handling and only if that is reported; under `Prefer: handling=strict` it is
/// an error. Callers turn this list into a `400` under strict handling, and
/// under lenient handling drop these parameters from both the executed query and
/// the searchset self link.
pub fn unknown_search_params(
    resource_type: &str,
    params: &SearchParams,
    registry: &SearchParameterRegistry,
) -> Vec<String> {
    let mut unknown = Vec::new();
    for (name, _) in params.search_params() {
        // Reverse chaining is validated when parsed; skip here.
        if name == "_has" || name.starts_with("_has:") {
            continue;
        }
        // Strip modifier (`name:exact`, `subject:Patient`) then chain (`a.b`).
        let base = name
            .split(':')
            .next()
            .unwrap_or(name)
            .split('.')
            .next()
            .unwrap_or(name);
        let known = GLOBAL_SEARCH_PARAMS.contains(&base)
            || registry.get_param(resource_type, base).is_some()
            || registry.get_param("Resource", base).is_some();
        if !known {
            unknown.push(name.clone());
        }
    }
    unknown
}

/// Returns warnings for `_sort` directives the backends cannot honor (#958).
///
/// An unknown sort parameter, or one whose type carries no sortable value
/// column (composite, special), silently degrades to a stable id sort in the
/// storage backends — indistinguishable from the sort being ignored. Callers
/// surface these on the searchset as an OperationOutcome entry so the
/// degradation is reported instead of absorbed.
pub fn unsortable_sort_warnings(
    resource_type: &str,
    params: &SearchParams,
    registry: &SearchParameterRegistry,
) -> Vec<String> {
    let Some(sort) = params.get("_sort") else {
        return Vec::new();
    };
    let mut warnings = Vec::new();
    for directive in sort.split(',') {
        let code = directive.trim().trim_start_matches('-');
        if code.is_empty() || code == "_id" || code == "_lastUpdated" {
            continue;
        }
        let def = registry
            .get_param(resource_type, code)
            .or_else(|| registry.get_param("Resource", code));
        match def {
            None => warnings.push(format!(
                "_sort parameter '{code}' is not a search parameter of {resource_type}; \
                 results are ordered by id instead"
            )),
            Some(def) => {
                let kind = def.param_type.to_string();
                if kind == "composite" || kind == "special" {
                    warnings.push(format!(
                        "_sort parameter '{code}' has type {kind}, which cannot be \
                         sorted; results are ordered by id instead"
                    ));
                }
            }
        }
    }
    warnings
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
    // A dotted name is a chained parameter (e.g. "patient.name" or
    // "subject:Patient.name:exact"). A colon inside it is a hop's type
    // qualifier, except on the last part, where it is the modifier of the
    // chain's terminal parameter — `parse_parameter_name` cuts at the first
    // colon and would misread either.
    let (base_name, chain, modifier) = if name.contains('.') {
        parse_chain(name)?
    } else {
        let (base_name, modifier) = parse_parameter_name(name)?;
        (base_name, vec![], modifier)
    };

    // FHIR defines :missing as a single, case-sensitive boolean literal.
    // Validate it before splitting comma-separated OR values so malformed
    // inputs cannot silently become `missing=false` in a storage backend.
    check_missing_literal(name, modifier.as_ref(), value)?;

    // Parse the value(s) - multiple values separated by comma are ORed.
    // Prefix extraction (gt/lt/ge/le/sa/eb/ap/eq/ne) is only meaningful for
    // date/number/quantity types per FHIR. For tokens/strings/references/uris,
    // the raw value is the value — e.g. status code "appended" must not be
    // misread as Ap-prefix + "pended".
    // Split the OR-list on UNescaped commas and unescape `\,` / `\\`, per FHIR
    // value escaping. A literal comma in a value is written `\,` and must not
    // start a new OR-alternative.
    let raw_values: Vec<String> = split_unescaped_commas(value);

    // Resolve the canonical type from the search parameter registry and parse
    // the values for it. This is deterministic for any registered parameter
    // (which is everything in the FHIR spec); the value-shape heuristic is
    // reached only for unregistered custom params. The routine is shared with
    // the chain resolver, which parses the terminal parameter of a chained or
    // `_has` search the same way — for a chained parameter `base_name` is the
    // reference hop, so its values stay raw here and are typed there. See
    // `helios_persistence::search::parse_typed_values`.
    let (param_type, values) = parse_typed_values(registry, resource_type, base_name, &raw_values);

    // FHIR-spec modifier validation: reject a modifier that is not defined for
    // this parameter's type (e.g. `:exact` on a token, `:contains` on a date),
    // or `:missing` on a parameter with no presence index, with a 400 rather
    // than silently ignoring it — see `validate_modifier`. A chained
    // parameter's modifier belongs to its terminal parameter, whose type is
    // not known until the chain resolver has walked the hops; the resolver
    // runs the same check there.
    if let (Some(m), true) = (&modifier, chain.is_empty()) {
        validate_modifier(registry, resource_type, base_name, param_type, m).map_err(
            |message| RestError::InvalidParameter {
                param: name.to_string(),
                message,
            },
        )?;
    }

    // Reject a value that is not a number for a registry-known number or
    // quantity parameter, rather than handing it to a storage backend (#1319):
    // PostgreSQL and Elasticsearch used to skip a value whose number part does
    // not parse, so `probability=abc` was an unconstrained search; SQLite
    // matches nothing and MongoDB errors. For a quantity only the number part —
    // everything before the first `|` — is checked; system and code are free
    // text. Scoped like the modifier check above: an unregistered param only
    // has a guessed type. Skipped for `:missing` (its value is a boolean), for
    // chains (the value belongs to the chain's last link, whose type is not
    // resolved here), and for an empty value, which is left to the backend.
    //
    // Date values get the same treatment from the gate every backend shares,
    // `helios_persistence::search::validate_date_values`, in
    // `build_search_query`.
    let registered = registry.get_param(resource_type, base_name).is_some()
        || registry.get_param("Resource", base_name).is_some();
    if registered
        && matches!(
            param_type,
            SearchParamType::Number | SearchParamType::Quantity
        )
        && chain.is_empty()
        && !matches!(modifier, Some(SearchModifier::Missing))
    {
        let number_part = |v: &str| -> String {
            match param_type {
                SearchParamType::Quantity => v.split('|').next().unwrap_or_default().to_string(),
                _ => v.to_string(),
            }
        };
        if let Some(bad) = values
            .iter()
            .find(|v| !v.value.is_empty() && !is_number_search_value(&number_part(&v.value)))
        {
            return Err(RestError::InvalidParameter {
                param: name.to_string(),
                message: format!(
                    "'{}' is not a valid {param_type} value (expected [prefix]number{}, where \
                     number is a decimal with an optional exponent)",
                    bad.value,
                    if param_type == SearchParamType::Quantity {
                        "[|system|code]"
                    } else {
                        ""
                    }
                ),
            });
        }
    }

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

    Ok(param)
}

/// Whether `value` (comparator prefix already removed) is the number of a FHIR
/// number or quantity search value: an optional sign, digits with an optional
/// fraction, and an optional exponent.
///
/// Deliberately the widest reading, because a false rejection turns a working
/// search into a 400: a leading `+`, leading zeros and a bare leading or
/// trailing point (`.5`, `5.`) are tolerated. What it stops is what is not a
/// number at all — `abc`, `1e` — and the words a float parser accepts for
/// non-finite values (`inf`, `nan`), which as a comparison bound match every
/// row. Mirrors the PostgreSQL builder's `parse_search_number`, which stays as
/// defence in depth.
fn is_number_search_value(value: &str) -> bool {
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (unsigned, None),
    };
    if let Some(exponent) = exponent {
        let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if exponent.is_empty() || !digits(exponent) {
            return false;
        }
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return false;
    }
    // Rules out a literal that overflows to infinity (`1e999`).
    value.parse::<f64>().is_ok_and(f64::is_finite)
}

/// Parses a direct (unchained) parameter name into the base name and optional
/// modifier.
///
/// A `:suffix` that is neither a search modifier nor a resource-type qualifier
/// is an error: dropping it would run a different search than the one asked
/// for (`name:exat=Smith` would widen an exact match into a prefix match).
///
/// Examples:
/// - "name" -> ("name", None)
/// - "name:exact" -> ("name", Some(Exact))
/// - "subject:Patient" -> ("subject", Some(Type("Patient")))
/// - "name:bogus" -> error
fn parse_parameter_name(name: &str) -> Result<(&str, Option<SearchModifier>), RestError> {
    let (param_name, suffix) = split_qualifier(name);
    let modifier = suffix
        .map(|s| {
            parse_modifier(s).ok_or_else(|| RestError::InvalidParameter {
                param: name.to_string(),
                message: format!(
                    "unknown search modifier ':{s}' on parameter '{param_name}'; it is neither a \
                     search modifier nor a resource type"
                ),
            })
        })
        .transpose()?;
    Ok((param_name, modifier))
}

/// Parses a `:suffix` as a search modifier, or as the `:[type]` qualifier of a
/// reference parameter.
///
/// `SearchModifier::parse` reads any capitalised suffix as a type qualifier, so
/// the name is checked against the resource types of the enabled FHIR versions
/// here — `subject:Bogus` is no more a modifier than `subject:bogus`. Whether
/// the modifier suits the parameter's *type* (a `:[type]` qualifier is only
/// defined for references) is `validate_modifier`'s job.
fn parse_modifier(suffix: &str) -> Option<SearchModifier> {
    match SearchModifier::parse(suffix)? {
        SearchModifier::Type(t) if !crate::fhir_types::is_valid_resource_type(&t) => None,
        modifier => Some(modifier),
    }
}

/// A parsed chain: the base parameter, one hop per reference parameter, and the
/// modifier written on the terminal parameter.
type ParsedChain<'a> = (
    &'a str,
    Vec<helios_persistence::types::ChainedParameter>,
    Option<SearchModifier>,
);

/// Parses chain elements from a parameter name.
///
/// A chain `a:T0.b:T1.c` is one hop per reference parameter: hop 0 follows `a`
/// (to a `T0`) and targets `b`; hop 1 follows `b` (to a `T1`) and targets `c`.
/// A `:Type` qualifier constrains the reference parameter it is written on, so
/// it belongs to the hop whose `reference_param` that is.
///
/// The last part, `c`, is the terminal search parameter, not a reference: a
/// `:suffix` on it is a search *modifier* (`name:exact`), returned separately —
/// it becomes the chained `SearchParameter`'s modifier, which the chain
/// resolver applies to the terminal search. A suffix that is not a modifier is
/// an error rather than something to drop silently.
///
/// Examples:
/// - "name" -> ("name", [], None)
/// - "patient.name" -> ("patient", [{patient, None, name}], None)
/// - "subject:Patient.organization:Organization.name:exact" ->
///   ("subject", [{subject, Patient, organization}, {organization, Organization, name}], Exact)
fn parse_chain(name: &str) -> Result<ParsedChain<'_>, RestError> {
    let mut parts = name.split('.');
    // `split` always yields at least one item.
    let (base_name, mut qualifier) = split_qualifier(parts.next().unwrap_or(name));
    let mut reference_param = base_name;

    let mut chain = Vec::new();
    let mut modifier = None;
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        // Every part but the last is itself a reference parameter, whose
        // qualifier belongs to the *next* hop.
        let (target_param, suffix) = split_qualifier(part);
        let next_qualifier = if parts.peek().is_some() {
            suffix
        } else {
            modifier = suffix
                .map(|s| parse_terminal_modifier(name, target_param, s))
                .transpose()?;
            None
        };
        chain.push(helios_persistence::types::ChainedParameter {
            reference_param: reference_param.to_string(),
            target_type: qualifier.map(str::to_string),
            target_param: target_param.to_string(),
        });
        reference_param = target_param;
        qualifier = next_qualifier;
    }

    Ok((base_name, chain, modifier))
}

/// Parses the `:suffix` written on the terminal parameter of a chain or `_has`.
///
/// Whether the modifier suits the terminal parameter's *type* is checked by the
/// chain resolver, which is where that type becomes known.
fn parse_terminal_modifier(
    name: &str,
    terminal_param: &str,
    suffix: &str,
) -> Result<SearchModifier, RestError> {
    parse_modifier(suffix).ok_or_else(|| RestError::InvalidParameter {
        param: name.to_string(),
        message: format!(
            "unknown search modifier ':{suffix}' on '{terminal_param}', the last parameter of \
             the chain; only a search modifier may follow it (a ':Type' qualifier belongs on a \
             reference parameter)"
        ),
    })
}

/// FHIR defines `:missing` as a single, case-sensitive boolean literal.
fn check_missing_literal(
    name: &str,
    modifier: Option<&SearchModifier>,
    value: &str,
) -> Result<(), RestError> {
    if matches!(modifier, Some(SearchModifier::Missing)) && !matches!(value, "true" | "false") {
        return Err(RestError::InvalidParameter {
            param: name.to_string(),
            message: "the :missing modifier requires exactly 'true' or 'false'".to_string(),
        });
    }
    Ok(())
}

/// Splits `param:Qualifier` into the parameter and its optional qualifier.
fn split_qualifier(part: &str) -> (&str, Option<&str>) {
    match part.split_once(':') {
        Some((param, qualifier)) => (param, Some(qualifier)),
        None => (part, None),
    }
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

    // `_has:Observation:subject:code:not=…` — what follows the terminal
    // parameter is its search modifier. `ReverseChainedParameter` carries it
    // inside `search_param`, as written (see `terminal_param`); the chain
    // resolver applies it to the terminal search and checks it against the
    // parameter's type. Here it only has to *be* a modifier. (In the
    // `_has=Type:ref:param:value` form the fourth part is the value.)
    let search_param = match parts.get(3) {
        Some(suffix) if name != "_has" => {
            let modifier = parse_terminal_modifier(name, &search_param, suffix)?;
            check_missing_literal(name, Some(&modifier), value)?;
            format!("{search_param}:{suffix}")
        }
        _ => search_param,
    };

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

/// Expands wildcard include directives (`_include=Type:*`) into a concrete
/// directive for each reference-typed search parameter of the source type,
/// preserving the original `iterate` / `target_type` flags. Non-wildcard
/// directives pass through unchanged.
fn expand_wildcard_includes(
    includes: &mut Vec<IncludeDirective>,
    registry: &SearchParameterRegistry,
) {
    if !includes.iter().any(|d| d.search_param == "*") {
        return;
    }
    let mut expanded = Vec::with_capacity(includes.len());
    for directive in includes.drain(..) {
        if directive.search_param == "*" {
            for def in registry.get_active_params(&directive.source_type) {
                if def.param_type == SearchParamType::Reference {
                    expanded.push(IncludeDirective {
                        search_param: def.code.clone(),
                        ..directive.clone()
                    });
                }
            }
        } else {
            expanded.push(directive);
        }
    }
    *includes = expanded;
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
    use helios_persistence::types::{SearchParamType, SearchPrefix};

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
            (
                "Observation-value-quantity",
                "value-quantity",
                SearchParamType::Quantity,
                "Observation",
            ),
            (
                "RiskAssessment-probability",
                "probability",
                SearchParamType::Number,
                "RiskAssessment",
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
        let (name, modifier) = parse_parameter_name("name").unwrap();
        assert_eq!(name, "name");
        assert!(modifier.is_none());
    }

    #[test]
    fn test_split_unescaped_commas() {
        // Plain OR-list splits on commas.
        assert_eq!(split_unescaped_commas("a,b,c"), vec!["a", "b", "c"]);
        // An escaped comma stays part of one value and is unescaped.
        assert_eq!(
            split_unescaped_commas("Smith\\, John,Doe"),
            vec!["Smith, John", "Doe"]
        );
        // Escaped backslash is unescaped; other escapes (\|) are preserved.
        assert_eq!(split_unescaped_commas("a\\\\b"), vec!["a\\b"]);
        assert_eq!(split_unescaped_commas("sys\\|code"), vec!["sys\\|code"]);
        assert_eq!(split_unescaped_commas("left\\$right"), vec!["left\\$right"]);
        assert_eq!(split_unescaped_commas("a\\,b,c"), vec!["a,b", "c"]);
        assert_eq!(split_unescaped_commas("a\\\\,b"), vec!["a\\", "b"]);
        assert_eq!(
            split_unescaped_commas("Muñoz\\,García"),
            vec!["Muñoz,García"]
        );
        // Keep empty alternatives visible to validation instead of silently
        // dropping them at this syntax boundary.
        assert_eq!(split_unescaped_commas("a,,b"), vec!["a", "", "b"]);
        assert_eq!(split_unescaped_commas(",a"), vec!["", "a"]);
        assert_eq!(split_unescaped_commas("a,"), vec!["a", ""]);
        // Surrounding whitespace is trimmed.
        assert_eq!(split_unescaped_commas(" a , b "), vec!["a", "b"]);
    }

    #[test]
    fn test_parse_parameter_name_with_modifier() {
        let (name, modifier) = parse_parameter_name("name:exact").unwrap();
        assert_eq!(name, "name");
        assert_eq!(modifier, Some(SearchModifier::Exact));
    }

    #[test]
    fn test_parse_parameter_name_with_type_modifier() {
        let (name, modifier) = parse_parameter_name("subject:Patient").unwrap();
        assert_eq!(name, "subject");
        assert_eq!(modifier, Some(SearchModifier::Type("Patient".to_string())));
    }

    /// #1318: a suffix that is neither a modifier nor a resource type is a 400
    /// naming the parameter and the suffix — not a search without the modifier.
    #[test]
    fn test_unknown_modifier_on_direct_parameter_is_rejected() {
        let reg = test_registry();
        for (resource_type, name, suffix) in [
            ("Patient", "name:bogus", ":bogus"),
            // A typo must not widen an exact match into a prefix match.
            ("Patient", "name:exat", ":exat"),
            ("Patient", "name:", ":"),
            ("Patient", "name:exact:extra", ":exact:extra"),
            // Capitalised, but not a resource type.
            ("Observation", "subject:Bogus", ":Bogus"),
            ("Observation", "subject:bogus", ":bogus"),
            // Global parameters are direct parameters too.
            ("Patient", "_id:bogus", ":bogus"),
            // An unregistered parameter reaching the builder gets no pass: the
            // suffix is not a modifier whatever the parameter turns out to be.
            ("Patient", "custom:bogus", ":bogus"),
        ] {
            match parse_search_parameter(resource_type, name, "x", &reg) {
                Err(RestError::InvalidParameter { param, message }) => {
                    assert_eq!(param, name);
                    assert!(message.contains(&format!("'{suffix}'")), "{message}");
                    let base = name.split(':').next().unwrap();
                    assert!(message.contains(&format!("'{base}'")), "{message}");
                }
                other => panic!("{name}: expected InvalidParameter, got {other:?}"),
            }
        }
    }

    /// #1318: the `:[type]` qualifier stays valid on a reference parameter, is
    /// a wrong-type modifier anywhere else, and needs a real resource type.
    #[test]
    fn test_type_qualifier_needs_a_reference_parameter_and_a_resource_type() {
        let reg = test_registry();
        let p = parse_search_parameter("Observation", "subject:Patient", "123", &reg).unwrap();
        assert_eq!(p.name, "subject");
        assert_eq!(
            p.modifier,
            Some(SearchModifier::Type("Patient".to_string()))
        );

        assert!(matches!(
            parse_search_parameter("Patient", "birthdate:Patient", "1980", &reg),
            Err(RestError::InvalidParameter { .. })
        ));
        // Case-sensitive, like every resource type name.
        assert!(parse_search_parameter("Observation", "subject:PATIENT", "123", &reg).is_err());
        // The same rule holds on the terminal of a chain.
        assert!(parse_chain("subject.general-practitioner:Bogus").is_err());
        assert!(parse_chain("subject.general-practitioner:Practitioner").is_ok());
    }

    /// #1318: every modifier in `SearchModifier` is still accepted on a
    /// parameter type it is defined for.
    #[test]
    fn test_every_known_modifier_is_still_accepted() {
        let reg = test_registry();
        for (resource_type, name, value, expected) in [
            ("Patient", "name:exact", "x", SearchModifier::Exact),
            ("Patient", "name:contains", "x", SearchModifier::Contains),
            ("Patient", "name:text", "x", SearchModifier::Text),
            ("Patient", "name:missing", "true", SearchModifier::Missing),
            ("Observation", "code:not", "x", SearchModifier::Not),
            ("Observation", "code:text", "x", SearchModifier::Text),
            ("Observation", "code:above", "x", SearchModifier::Above),
            ("Observation", "code:below", "x", SearchModifier::Below),
            ("Observation", "code:in", "x", SearchModifier::In),
            ("Observation", "code:not-in", "x", SearchModifier::NotIn),
            (
                "Observation",
                "code:of-type",
                "a|b|c",
                SearchModifier::OfType,
            ),
            (
                "Observation",
                "code:ofType",
                "a|b|c",
                SearchModifier::OfType,
            ),
            (
                "Observation",
                "code:code-text",
                "x",
                SearchModifier::CodeText,
            ),
            (
                "Observation",
                "code:text-advanced",
                "x",
                SearchModifier::TextAdvanced,
            ),
            (
                "Observation",
                "subject:identifier",
                "s|v",
                SearchModifier::Identifier,
            ),
            (
                "Observation",
                "subject:Patient",
                "1",
                SearchModifier::Type("Patient".to_string()),
            ),
            (
                "Observation",
                "date:missing",
                "false",
                SearchModifier::Missing,
            ),
        ] {
            let p = parse_search_parameter(resource_type, name, value, &reg)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(p.modifier, Some(expected), "{name}");
        }
    }

    #[test]
    fn test_parse_chain_simple() {
        let (name, chain, _) = parse_chain("patient.name").unwrap();
        assert_eq!(name, "patient");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].reference_param, "patient");
        assert_eq!(chain[0].target_param, "name");
    }

    #[test]
    fn test_parse_chain_with_type() {
        let (name, chain, _) = parse_chain("subject:Patient.name").unwrap();
        assert_eq!(name, "subject");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].reference_param, "subject");
        assert_eq!(chain[0].target_type, Some("Patient".to_string()));
        assert_eq!(chain[0].target_param, "name");
    }

    #[test]
    fn test_parse_chain_multi_level() {
        let (name, chain, _) = parse_chain("subject.organization.name").unwrap();
        assert_eq!(name, "subject");
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].reference_param, "subject");
        assert_eq!(chain[0].target_param, "organization");
        assert_eq!(chain[1].reference_param, "organization");
        assert_eq!(chain[1].target_param, "name");
    }

    /// One hop of an expected chain: (reference_param, target_type, target_param).
    type Hop = (&'static str, Option<&'static str>, &'static str);

    type OwnedHop = (String, Option<String>, String);

    fn hops(chain: &[helios_persistence::types::ChainedParameter]) -> Vec<OwnedHop> {
        chain
            .iter()
            .map(|c| {
                (
                    c.reference_param.clone(),
                    c.target_type.clone(),
                    c.target_param.clone(),
                )
            })
            .collect()
    }
    fn owned(expected: &[Hop]) -> Vec<OwnedHop> {
        expected
            .iter()
            .map(|(r, t, p)| (r.to_string(), t.map(str::to_string), p.to_string()))
            .collect()
    }

    /// #1303: a `:Type` qualifier constrains the reference parameter it is
    /// written on, at every position of the chain.
    #[test]
    fn test_parse_chain_type_qualifier_positions() {
        let cases: &[(&str, &[Hop])] = &[
            ("patient.name", &[("patient", None, "name")]),
            (
                "subject:Patient.name",
                &[("subject", Some("Patient"), "name")],
            ),
            (
                "subject.organization.name",
                &[
                    ("subject", None, "organization"),
                    ("organization", None, "name"),
                ],
            ),
            (
                "subject:Patient.organization.name",
                &[
                    ("subject", Some("Patient"), "organization"),
                    ("organization", None, "name"),
                ],
            ),
            (
                "subject.organization:Organization.name",
                &[
                    ("subject", None, "organization"),
                    ("organization", Some("Organization"), "name"),
                ],
            ),
            (
                "subject:Patient.organization:Organization.name",
                &[
                    ("subject", Some("Patient"), "organization"),
                    ("organization", Some("Organization"), "name"),
                ],
            ),
            (
                "encounter:Encounter.subject:Patient.general-practitioner:Practitioner.name",
                &[
                    ("encounter", Some("Encounter"), "subject"),
                    ("subject", Some("Patient"), "general-practitioner"),
                    ("general-practitioner", Some("Practitioner"), "name"),
                ],
            ),
            (
                "encounter.subject.general-practitioner:Organization.name",
                &[
                    ("encounter", None, "subject"),
                    ("subject", None, "general-practitioner"),
                    ("general-practitioner", Some("Organization"), "name"),
                ],
            ),
        ];
        let mut failures = Vec::new();
        for (name, expected) in cases {
            let (base, chain, modifier) = parse_chain(name).unwrap();
            if base != expected[0].0 || hops(&chain) != owned(expected) || modifier.is_some() {
                failures.push(format!(
                    "{name}\n   got      {:?} {modifier:?}\n   expected {:?} None",
                    hops(&chain),
                    owned(expected)
                ));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    /// #1302: the `:suffix` of the last part is the terminal parameter's search
    /// modifier — never a type qualifier, and never part of the param name.
    #[test]
    fn test_parse_chain_terminal_modifier() {
        let cases: &[(&str, &[Hop], SearchModifier)] = &[
            (
                "subject:Patient.name:exact",
                &[("subject", Some("Patient"), "name")],
                SearchModifier::Exact,
            ),
            (
                "subject.name:contains",
                &[("subject", None, "name")],
                SearchModifier::Contains,
            ),
            (
                "subject:Patient.birthdate:missing",
                &[("subject", Some("Patient"), "birthdate")],
                SearchModifier::Missing,
            ),
            (
                "subject:Patient.organization:Organization.name:exact",
                &[
                    ("subject", Some("Patient"), "organization"),
                    ("organization", Some("Organization"), "name"),
                ],
                SearchModifier::Exact,
            ),
            (
                "encounter.subject.general-practitioner:Organization.name:contains",
                &[
                    ("encounter", None, "subject"),
                    ("subject", None, "general-practitioner"),
                    ("general-practitioner", Some("Organization"), "name"),
                ],
                SearchModifier::Contains,
            ),
            // On a reference terminal, `:Type` *is* a modifier, as it is on a
            // direct `subject:Patient=…`.
            (
                "encounter.subject:Patient",
                &[("encounter", None, "subject")],
                SearchModifier::Type("Patient".to_string()),
            ),
        ];
        let mut failures = Vec::new();
        for (name, expected, expected_modifier) in cases {
            let (base, chain, modifier) = parse_chain(name).unwrap();
            if base != expected[0].0
                || hops(&chain) != owned(expected)
                || modifier.as_ref() != Some(expected_modifier)
            {
                failures.push(format!(
                    "{name}\n   got      {:?} {modifier:?}\n   expected {:?} {expected_modifier:?}",
                    hops(&chain),
                    owned(expected)
                ));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));

        // The modifier ends up on the chained parameter itself, which is where
        // the chain resolver reads it from.
        let param = parse_search_parameter(
            "Observation",
            "subject:Patient.name:exact",
            "Smith",
            &test_registry(),
        )
        .unwrap();
        assert_eq!(param.name, "subject");
        assert_eq!(param.modifier, Some(SearchModifier::Exact));
        assert_eq!(param.chain[0].target_param, "name");
    }

    /// #1302: a last-part suffix that is no modifier is a 400, not dropped.
    #[test]
    fn test_parse_chain_rejects_unknown_terminal_suffix() {
        let reg = test_registry();
        for name in [
            "subject:Patient.name:bogus",
            "subject.organization.name:exact:extra",
        ] {
            assert!(parse_chain(name).is_err(), "{name}");
            assert!(
                matches!(
                    parse_search_parameter("Observation", name, "x", &reg),
                    Err(RestError::InvalidParameter { .. })
                ),
                "{name}"
            );
        }
        // :missing keeps its literal check on a chained terminal.
        assert!(
            parse_search_parameter("Observation", "subject.birthdate:missing", "yes", &reg)
                .is_err()
        );
        assert!(
            parse_search_parameter("Observation", "subject.birthdate:missing", "true", &reg)
                .is_ok()
        );
    }

    /// #1302: `_has:Type:ref:param:modifier` keeps the modifier, in
    /// `search_param`, for the chain resolver to apply.
    #[test]
    fn test_parse_has_parameter_terminal_modifier() {
        let chain = parse_has_parameter("_has:Observation:subject:code:not", "1234-5")
            .unwrap()
            .unwrap();
        assert_eq!(chain.search_param, "code:not");
        assert_eq!(chain.terminal_param(), ("code", Some("not")));

        let plain = parse_has_parameter("_has:Observation:subject:code", "1234-5")
            .unwrap()
            .unwrap();
        assert_eq!(plain.terminal_param(), ("code", None));

        let nested = parse_has_parameter(
            "_has:Encounter:subject:_has:Observation:encounter:code:not",
            "1234-5",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            nested.nested.as_ref().unwrap().terminal_param(),
            ("code", Some("not"))
        );

        assert!(parse_has_parameter("_has:Observation:subject:code:bogus", "1").is_err());
        assert!(parse_has_parameter("_has:Observation:subject:code:missing", "yes").is_err());
        assert!(parse_has_parameter("_has:Observation:subject:code:missing", "true").is_ok());
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
    fn test_missing_modifier_accepts_exact_boolean_literals() {
        let registry = test_registry();

        for value in ["true", "false"] {
            let param =
                parse_search_parameter("Patient", "birthdate:missing", value, &registry).unwrap();
            assert_eq!(param.modifier, Some(SearchModifier::Missing));
            assert_eq!(param.values.len(), 1);
            assert_eq!(param.values[0].value, value);
        }
    }

    #[test]
    fn test_missing_modifier_rejects_non_boolean_literals() {
        let registry = test_registry();

        for value in ["", "TRUE", "False", "invalid", "true,false"] {
            let error = parse_search_parameter("Patient", "birthdate:missing", value, &registry)
                .unwrap_err();
            assert!(
                matches!(error, RestError::InvalidParameter { .. }),
                "unexpected error for {value:?}: {error:?}"
            );
        }
    }

    /// Builds the whole query for one `name=value` pair, which is where date
    /// values are validated.
    fn build_one(
        resource_type: &str,
        name: &str,
        value: &str,
        registry: &SearchParameterRegistry,
    ) -> Result<SearchQuery, RestError> {
        build_search_query_from_pairs(
            resource_type,
            &[(name.to_string(), value.to_string())],
            registry,
        )
    }

    #[test]
    fn test_date_search_value_grammar() {
        // The grammar itself is `helios_persistence::search::FhirDateValue`'s,
        // and is tested exhaustively there; this pins that the REST layer
        // applies that one and no laxer reading of its own.
        let registry = test_registry();

        for value in [
            "2024",
            "2024-02",
            "2024-02-29",
            // Minute precision is valid in search (not in the datatype).
            "2013-04-05T09:20",
            "2013-04-05T09:20:00",
            "2013-04-05T09:20:00Z",
            "2013-04-05T09:20:00-04:00",
            "2013-04-05T09:20:00+05:30",
            // A `+` that form decoding turned into a space (#1296).
            "2013-04-05T09:20:00 05:30",
            "2021-11-10T16:48:57.246958-08:00",
            "2016-12-31T23:59:60Z",
        ] {
            assert!(
                build_one("Patient", "birthdate", value, &registry).is_ok(),
                "{value} is a date"
            );
        }
        for value in [
            "",
            "not-a-date",
            "2024-13-45",
            "2023-02-29",
            "2024-1-5",
            "20240115",
            "2024-01-15T",
            "2024-01-15T10",
            "2024-01T10:00",
            "2024-01-15T25:00:00Z",
            "2024-01-15T24:00:00Z",
            "2024-01-15T10:00:00.Z",
            "2024-01-15T10:00.5",
            "2024-01-15T10:00:00-99",
            "2024-01-15T10:00:00+05:3",
            "2024-01-15T10:00:00+15:00",
            "2024-01-15 10:00:00",
            // Lower case is not in the grammar. The validator this replaced
            // let `z` through.
            "2024-01-15T10:00:00z",
            "0000",
            "now",
        ] {
            assert!(
                matches!(
                    build_one("Patient", "birthdate", value, &registry),
                    Err(RestError::InvalidParameter { .. })
                ),
                "{value:?} is not a date"
            );
        }
    }

    #[test]
    fn test_invalid_date_value_is_rejected() {
        // #1289: these reached the storage backend, where PostgreSQL bound the
        // current time in their place.
        let registry = test_registry();

        for value in [
            "not-a-date",
            "gtnot-a-date",
            "lt2024-13-45",
            "lt2024-02-30",
            // One bad alternative spoils the OR-list.
            "2024-01-15,nope",
        ] {
            let error = build_one("Patient", "birthdate", value, &registry).unwrap_err();
            assert!(
                matches!(&error, RestError::InvalidParameter { param, .. } if param == "birthdate"),
                "unexpected error for {value:?}: {error:?}"
            );
        }

        // `_lastUpdated` is a date whether or not the registry lists it.
        let error = build_one("Patient", "_lastUpdated", "gtnot-a-date", &registry).unwrap_err();
        assert!(
            matches!(&error, RestError::InvalidParameter { param, .. } if param == "_lastUpdated"),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn test_valid_date_values_still_parse() {
        let registry = test_registry();

        for (value, prefix, bare) in [
            ("1990", SearchPrefix::Eq, "1990"),
            ("ge1990-05", SearchPrefix::Ge, "1990-05"),
            (
                "lt2013-04-05T09:20:00-04:00",
                SearchPrefix::Lt,
                "2013-04-05T09:20:00-04:00",
            ),
        ] {
            let query = build_one("Patient", "birthdate", value, &registry).unwrap();
            assert_eq!(query.parameters[0].values[0].prefix, prefix);
            assert_eq!(query.parameters[0].values[0].value, bare);
        }
    }

    #[test]
    fn test_date_validation_leaves_other_parameters_alone() {
        let registry = test_registry();

        // `:missing` carries a boolean, not a date.
        build_one("Patient", "birthdate:missing", "true", &registry).unwrap();
        // A chain's value belongs to its last link, whose type is not known
        // here; the storage gate validates the terminal query instead.
        build_one("Observation", "patient.birthdate", "whatever", &registry).unwrap();
        // An unregistered parameter is never typed as a date.
        build_one("Patient", "custom-date", "not-a-date", &registry).unwrap();
        // Not a date parameter at all.
        build_one("Patient", "name", "2024-13-45", &registry).unwrap();
    }

    #[test]
    fn test_number_search_value_grammar() {
        for value in [
            "0", "5", "5.4", "-5.4", "100.00", "1e3", "1E3", "1e+3", "1.5E-2", "-1.5e-2",
            // Tolerated beyond the FHIR decimal grammar.
            "+5.4", "007", ".5", "5.", "-.5",
        ] {
            assert!(is_number_search_value(value), "should accept {value:?}");
        }
        for value in [
            "abc",
            "",
            " ",
            " 5",
            "5 ",
            "1e",
            "1e+",
            "e5",
            ".",
            "-",
            "--5",
            "0.2abc",
            "1.2.3",
            "1e5.5",
            "1_000",
            "0x10",
            "inf",
            "+inf",
            "-Infinity",
            "NaN",
            "nan",
            "1e999",
        ] {
            assert!(!is_number_search_value(value), "should reject {value:?}");
        }
    }

    #[test]
    fn test_invalid_number_value_is_rejected() {
        // #1319: these reached the storage backend, where PostgreSQL and
        // Elasticsearch dropped the constraint.
        let registry = test_registry();

        for value in [
            "abc", "gtabc", "neabc", "1e", "ltinf", "nenan",
            // One bad alternative spoils the OR-list.
            "0.5,nope",
        ] {
            let error = parse_search_parameter("RiskAssessment", "probability", value, &registry)
                .unwrap_err();
            assert!(
                matches!(&error, RestError::InvalidParameter { param, .. } if param == "probability"),
                "unexpected error for {value:?}: {error:?}"
            );
        }

        for value in [
            "abc",
            "abc|http://unitsofmeasure.org|mg",
            "gt|http://unitsofmeasure.org|mg",
            "|http://unitsofmeasure.org|mg",
            "||mg",
            "ltinf||mg",
            "5.4||mg,abc||mg",
        ] {
            let error = parse_search_parameter("Observation", "value-quantity", value, &registry)
                .unwrap_err();
            assert!(
                matches!(&error, RestError::InvalidParameter { param, .. } if param == "value-quantity"),
                "unexpected error for {value:?}: {error:?}"
            );
        }
    }

    #[test]
    fn test_valid_number_and_quantity_values_still_parse() {
        let registry = test_registry();

        for (value, prefix, bare) in [
            ("0.5", SearchPrefix::Eq, "0.5"),
            ("ge0.5", SearchPrefix::Ge, "0.5"),
            ("lt1e3", SearchPrefix::Lt, "1e3"),
            ("ne-1.5E-2", SearchPrefix::Ne, "-1.5E-2"),
        ] {
            let param =
                parse_search_parameter("RiskAssessment", "probability", value, &registry).unwrap();
            assert_eq!(param.values[0].prefix, prefix);
            assert_eq!(param.values[0].value, bare);
        }

        for (value, prefix, bare) in [
            ("5.4", SearchPrefix::Eq, "5.4"),
            ("5.4|mg", SearchPrefix::Eq, "5.4|mg"),
            ("5.4||mg", SearchPrefix::Eq, "5.4||mg"),
            (
                "gt5.4|http://unitsofmeasure.org|mg",
                SearchPrefix::Gt,
                "5.4|http://unitsofmeasure.org|mg",
            ),
            // System and code are free text, escaped pipe included.
            (
                "le1e3|http://x|a\\|b",
                SearchPrefix::Le,
                "1e3|http://x|a\\|b",
            ),
            (
                "ap-5.4||not a number",
                SearchPrefix::Ap,
                "-5.4||not a number",
            ),
        ] {
            let param =
                parse_search_parameter("Observation", "value-quantity", value, &registry).unwrap();
            assert_eq!(param.values[0].prefix, prefix);
            assert_eq!(param.values[0].value, bare);
        }
    }

    #[test]
    fn test_number_validation_leaves_other_parameters_alone() {
        let registry = test_registry();

        // `:missing` carries a boolean, not a number.
        parse_search_parameter("RiskAssessment", "probability:missing", "true", &registry).unwrap();
        parse_search_parameter("Observation", "value-quantity:missing", "false", &registry)
            .unwrap();
        // An unregistered parameter only has a guessed type.
        parse_search_parameter("RiskAssessment", "custom-number", "gtabc", &registry).unwrap();
        // Not a number parameter at all.
        parse_search_parameter("Patient", "name", "abc", &registry).unwrap();
        // An empty value is left to the backend, as for dates.
        parse_search_parameter("RiskAssessment", "probability", "", &registry).unwrap();
    }

    #[test]
    fn test_missing_modifier_rejects_composite_parameters() {
        let mut registry = test_registry();
        registry
            .register(
                SearchParameterDefinition::new(
                    "http://hl7.org/fhir/SearchParameter/Observation-code-value-quantity",
                    "code-value-quantity",
                    SearchParamType::Composite,
                    "ignored",
                )
                .with_base(vec!["Observation"]),
            )
            .unwrap();

        let error = parse_search_parameter(
            "Observation",
            "code-value-quantity:missing",
            "true",
            &registry,
        )
        .unwrap_err();

        assert!(matches!(error, RestError::InvalidParameter { .. }));
    }

    #[test]
    fn test_missing_modifier_rejects_non_indexed_special_parameters() {
        let registry = test_registry();

        for name in ["_text:missing", "_content:missing"] {
            let error = parse_search_parameter("Patient", name, "true", &registry).unwrap_err();
            assert!(
                matches!(error, RestError::InvalidParameter { .. }),
                "unexpected error for {name}: {error:?}"
            );
        }
    }

    #[test]
    fn test_language_missing_requires_a_registered_search_parameter() {
        let mut registry = test_registry();
        assert!(parse_search_parameter("Patient", "_language:missing", "true", &registry).is_err());

        registry
            .register(
                SearchParameterDefinition::new(
                    "http://hl7.org/fhir/SearchParameter/Resource-language",
                    "_language",
                    SearchParamType::Token,
                    "Resource.language",
                )
                .with_base(vec!["Resource"]),
            )
            .unwrap();

        let param =
            parse_search_parameter("Patient", "_language:missing", "true", &registry).unwrap();
        assert_eq!(param.modifier, Some(SearchModifier::Missing));
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
    fn test_unknown_search_params_flags_unhonoured_underscore_names() {
        // Regression for #524: a blanket `starts_with('_')` bypass let every
        // underscore name through, so strict handling could never reject one.
        let registry = test_registry();
        let unknown = |q: Vec<(&str, &str)>| {
            let sp = SearchParams::from_pairs(
                q.into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            );
            unknown_search_params("Patient", &sp, &registry)
        };

        // Underscore names the server does not implement are unknown.
        assert_eq!(unknown(vec![("_typo", "x")]), vec!["_typo".to_string()]);
        assert_eq!(
            unknown(vec![("_whatever", "x")]),
            vec!["_whatever".to_string()]
        );
        // …including with a modifier, which is stripped before the check.
        assert_eq!(
            unknown(vec![("_typo:exact", "x")]),
            vec!["_typo:exact".to_string()]
        );

        // Every global parameter the server honours stays allowed, even though
        // the test registry holds no `Resource`-level definitions.
        for name in GLOBAL_SEARCH_PARAMS {
            assert!(
                unknown(vec![(name, "x")]).is_empty(),
                "{name} must be treated as a known global parameter"
            );
        }

        // Reverse chaining and ordinary registered parameters are unaffected.
        assert!(unknown(vec![("_has:Observation:patient:code", "1")]).is_empty());
        assert!(unknown(vec![("name", "Smith")]).is_empty());
        assert_eq!(unknown(vec![("nope", "x")]), vec!["nope".to_string()]);
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
