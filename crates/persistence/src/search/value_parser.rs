//! Parsing of raw search-parameter values into typed [`SearchValue`]s.
//!
//! This is the single routine that turns the text after `=` in a FHIR search
//! into an OR-list of values: it splits on unescaped commas, resolves the
//! parameter's type from the registry, and strips a comparator prefix
//! (`ge`, `lt`, …) only for the types that define one. Direct parameters (the
//! REST query builder) and the terminal parameter of a chained / `_has` search
//! (the [chain resolver](super::chain_resolver)) both go through it, so a
//! chained `birthdate=ge1980-01-01` means exactly what the direct one does.

use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{
    ReverseChainedParameter, SearchModifier, SearchParamType, SearchParameter, SearchQuery,
    SearchValue,
};

use super::registry::{SearchParameterRegistry, resolve_param_type};
use super::text_fold::fold_text;

/// Splits a FHIR search value into its comma-separated OR-alternatives,
/// respecting backslash escaping.
///
/// Per the FHIR spec, the characters `, | $ \` are escaped with a leading
/// backslash inside a value. Only the comma is the OR-list separator at this
/// layer, so we split on *unescaped* commas and unescape `\,` and `\\` here.
/// Other escapes (`\|`, `\$`) are left intact for the backend's token/composite
/// parsing to interpret.
pub fn split_unescaped_commas(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some(',') => {
                    cur.push(',');
                    chars.next();
                }
                Some('\\') => {
                    cur.push('\\');
                    chars.next();
                }
                // Preserve other escapes (e.g. `\|`, `\$`) for downstream parsing.
                _ => cur.push('\\'),
            },
            ',' => {
                out.push(cur.trim().to_string());
                cur = String::new();
            }
            _ => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

/// Resolves `name`'s type on `resource_type` and parses its already
/// comma-split `raw_values` accordingly.
///
/// Prefix extraction (gt/lt/ge/le/sa/eb/ap/eq/ne) is only meaningful for
/// date/number/quantity types per FHIR. For tokens/strings/references/uris,
/// the raw value is the value — e.g. status code "appended" must not be
/// misread as Ap-prefix + "pended", nor the family name "Lee" as `le` + "e".
///
/// The type comes from the search parameter registry, which is deterministic
/// for any registered parameter (everything in the FHIR spec); the value-shape
/// heuristic inside [`resolve_param_type`] is reached only for unregistered
/// custom params, and sees the values with any prefix already stripped.
///
/// The prefix rule itself is [`SearchValue::parse_for_type`]'s.
pub fn parse_typed_values(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
    raw_values: &[String],
) -> (SearchParamType, Vec<SearchValue>) {
    let tentative_values: Vec<SearchValue> =
        raw_values.iter().map(|v| SearchValue::parse(v)).collect();

    let param_type = resolve_param_type(registry, resource_type, name, &tentative_values);

    let values = raw_values
        .iter()
        .map(|v| SearchValue::parse_for_type(v, param_type))
        .collect();

    (param_type, values)
}

/// Refuses a search value that is empty, or has an empty alternative in its
/// comma-separated OR-list: `family=Zzz,`, `family=,Zzz`, `family=a,,b`,
/// `family=,` — and `family=` itself, when it gets this far.
///
/// An empty alternative is not "no constraint" to a backend; it is the value
/// `""`, and what that matches depends on the parameter. A string search is a
/// prefix match and every string starts with `""`, so `family=Zzz,` returned
/// every Patient that has a family name; `:contains` and `:text` likewise, a
/// token's `:not=` every resource, and `:of-type=` or a uri's `:below=` every
/// resource on some backends and none on others (#1380). A trailing comma is
/// what joining a list with an empty element renders as: a client bug the
/// client should hear about, as it does for a date or a number that is not one
/// ([`validate_date_values`](super::validate_date_values),
/// [`validate_numeric_values`](super::validate_numeric_values)). As there,
/// `Prefer: handling` is not consulted — the parameter is known, its value is
/// malformed.
///
/// FHIR has a parameter with no value at all (`family=`) *ignored* rather than
/// refused, and the REST search handlers drop such a pair before a query is
/// built. That is a decision about a request, which only an entry point can
/// make: criteria that guard a write refuse it instead
/// ([`reject_empty_criterion_values`](super::conditional::reject_empty_criterion_values)).
/// A storage backend cannot ignore what it is handed, so here it is an error,
/// never a query.
///
/// Covers direct parameters, the components of a composite value, the raw
/// values of a chained parameter and the (unsplit) value of a `_has`. Under
/// `:missing` the value is the boolean literal, checked where it is parsed.
/// An escaped comma (`family=a\,`) is data, not a separator.
pub fn validate_value_presence(query: &SearchQuery) -> StorageResult<()> {
    for param in &query.parameters {
        if has_empty_value(param) {
            return Err(empty_value(display_name(param)));
        }
    }
    for reverse_chain in &query.reverse_chains {
        validate_has_value_presence(reverse_chain)?;
    }
    Ok(())
}

/// Whether `param` carries a value [`validate_value_presence`] refuses: an
/// empty (or whitespace-only) one, a string that folds to one, or a composite
/// value with such a component.
///
/// The storage backends ask this again where they build a parameter's
/// condition, and answer a `true` with one that matches **nothing** — the
/// whole parameter, `:not` included, since negating "nothing" is "everything".
/// The gate reports the error on every ordinary path; this is what stands
/// behind it should a caller ever skip it.
pub fn has_empty_value(param: &SearchParameter) -> bool {
    // A `_filter` value is an expression, whose string literals may hold
    // anything, doubled commas included; its parser judges it.
    if matches!(param.modifier, Some(SearchModifier::Missing)) || param.name == "_filter" {
        return false;
    }
    let composite = param.chain.is_empty() && param.param_type == SearchParamType::Composite;
    // String matching compares accent-folded text (except under `:exact`), and
    // a value made of combining marks alone folds to nothing: as empty as `""`.
    let folded = param.chain.is_empty()
        && param.param_type == SearchParamType::String
        && !matches!(param.modifier, Some(SearchModifier::Exact));
    param.values.iter().any(|value| {
        is_blank(&value.value)
            || folded && is_blank(&fold_text(&value.value))
            || composite
                && split_composite_components(&value.value)
                    .iter()
                    .any(|c| is_blank(c))
    })
}

fn validate_has_value_presence(has: &ReverseChainedParameter) -> StorageResult<()> {
    if let Some(nested) = &has.nested {
        return validate_has_value_presence(nested);
    }
    let Some(value) = &has.value else {
        return Ok(());
    };
    if has.terminal_param().1 == Some("missing") {
        return Ok(());
    }
    // A `_has` value reaches the chain resolver unsplit.
    if split_unescaped_commas(&value.value)
        .iter()
        .any(|alternative| alternative.is_empty())
    {
        return Err(empty_value(format!(
            "_has:{}:{}:{}",
            has.source_type, has.reference_param, has.search_param
        )));
    }
    Ok(())
}

/// The parameter as the client wrote it, as nearly as the parsed form tells:
/// `family:exact`, `subject.family` (a longer chain's intermediate hops are
/// not repeated).
fn display_name(param: &SearchParameter) -> String {
    let mut out = param.name.clone();
    if let Some(hop) = param.chain.last() {
        out.push('.');
        out.push_str(&hop.target_param);
    }
    match &param.modifier {
        // `Display` writes the legacy camelCase spelling.
        Some(SearchModifier::OfType) => out.push_str(":of-type"),
        Some(modifier) => {
            out.push(':');
            out.push_str(&modifier.to_string());
        }
        None => {}
    }
    out
}

fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}

/// The `$`-separated components of a composite value; `\$` is data.
fn split_composite_components(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        let last = parts.len() - 1;
        match c {
            '\\' if chars.peek() == Some(&'$') => {
                chars.next();
                parts[last].push('$');
            }
            '$' => parts.push(String::new()),
            _ => parts[last].push(c),
        }
    }
    parts
}

/// What is wrong with a value [`validate_value_presence`] refuses, worded for
/// the client.
pub const EMPTY_VALUE_REASON: &str = "the value is empty, or has an empty alternative in its \
     comma-separated list (a leading, trailing or doubled comma); a comma that belongs to the \
     value is written '\\,'";

fn empty_value(param: String) -> StorageError {
    StorageError::Search(SearchError::EmptyValue { param })
}

/// Checks that `modifier` can be applied to the parameter `name` of
/// `resource_type`, whose resolved type is `param_type`. The error is the
/// reason, worded for the client.
///
/// Like [`parse_typed_values`], this is shared by direct parameters (the REST
/// query builder) and the terminal parameter of a chained / `_has` search (the
/// [chain resolver](super::chain_resolver)), so `subject:Patient.birthdate:exact`
/// is rejected for the same reason `Patient?birthdate:exact` is.
///
/// Two rules:
///
/// * `:missing` needs an ordinary presence row in the search index. Full-text
///   and other computed `_` parameters have none, and treating them as
///   index-backed would make `:missing=true` match every resource.
/// * A modifier must be defined for the parameter's type (`:exact` is not, on
///   a token; `:contains` is not, on a date) — see
///   [`SearchModifier::is_valid_for`], which reflects what this server honors.
///   Scoped to registry-known, non-special params: an unregistered custom
///   param gets a value-shape heuristic type, so gating it could falsely reject
///   a legitimate custom modifier; and the `special` full-text params (`_text`,
///   `_content`, …) carry server-specific modifier semantics outside the typed
///   modifier table.
pub fn validate_modifier(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
    param_type: SearchParamType,
    modifier: &SearchModifier,
) -> Result<(), String> {
    let registered = registry.get_param(resource_type, name).is_some()
        || registry.get_param("Resource", name).is_some();

    if matches!(modifier, SearchModifier::Missing) && name.starts_with('_') {
        let has_presence_index = matches!(name, "_id" | "_lastUpdated")
            || registered
                && matches!(
                    name,
                    "_tag" | "_profile" | "_security" | "_source" | "_language"
                );
        if !has_presence_index {
            return Err(format!(":missing is not supported for parameter '{name}'"));
        }
    }

    if registered && param_type != SearchParamType::Special && !modifier.is_valid_for(param_type) {
        return Err(format!(
            "search modifier ':{modifier}' is not supported for {param_type} parameter '{name}'"
        ));
    }
    Ok(())
}

/// Whether `modifier` on a parameter of type `param_type` can only be answered
/// with the help of a terminology server. `None` is a parameter the registry
/// does not know.
///
/// `:in` / `:not-in` test value-set membership, so they always do. `:above` /
/// `:below` do on a **token** (code-system subsumption); on a uri or reference
/// they are structural and the backends resolve them natively.
pub fn modifier_requires_terminology(
    modifier: &SearchModifier,
    param_type: Option<SearchParamType>,
) -> bool {
    match modifier {
        SearchModifier::In | SearchModifier::NotIn => true,
        SearchModifier::Above | SearchModifier::Below => param_type == Some(SearchParamType::Token),
        _ => false,
    }
}

/// [`modifier_requires_terminology`] for the parameter `name` of
/// `resource_type`, typed by the registry (resource-level parameters such as
/// `_tag` are declared on `Resource`).
///
/// Like [`validate_modifier`], this is shared by direct parameters (the REST
/// search handler's guard) and the terminal parameter of a chained / `_has`
/// search (the [chain resolver](super::chain_resolver) — the only place that
/// terminal's type is known), so a server without terminology answers
/// `subject:Patient.gender:below` exactly as it answers `Patient?gender:below`.
pub fn param_requires_terminology(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
    modifier: &SearchModifier,
) -> bool {
    let declared = registry
        .get_param(resource_type, name)
        .or_else(|| registry.get_param("Resource", name))
        .map(|p| p.param_type);
    modifier_requires_terminology(modifier, declared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchPrefix;

    #[test]
    fn terminology_is_needed_by_in_and_by_token_hierarchy_only() {
        use SearchModifier as M;
        use SearchParamType as T;
        for m in [M::In, M::NotIn, M::Below, M::Above] {
            assert!(modifier_requires_terminology(&m, Some(T::Token)), "{m}");
        }
        // Structural on uri / reference: the backends resolve these natively.
        for t in [T::Uri, T::Reference] {
            assert!(!modifier_requires_terminology(&M::Below, Some(t)), "{t}");
            assert!(!modifier_requires_terminology(&M::Above, Some(t)), "{t}");
        }
        // An unregistered parameter: value-set membership still needs
        // terminology, a hierarchy modifier is not known to.
        assert!(modifier_requires_terminology(&M::In, None));
        assert!(!modifier_requires_terminology(&M::Below, None));
        assert!(!modifier_requires_terminology(&M::Not, Some(T::Token)));
        assert!(!modifier_requires_terminology(&M::Exact, Some(T::String)));
    }

    #[test]
    fn splits_on_unescaped_commas_only() {
        assert_eq!(split_unescaped_commas("a,b,c"), vec!["a", "b", "c"]);
        assert_eq!(
            split_unescaped_commas("Smith\\, John,Doe"),
            vec!["Smith, John", "Doe"]
        );
        assert_eq!(split_unescaped_commas("a\\\\,b"), vec!["a\\", "b"]);
        assert_eq!(split_unescaped_commas("sys\\|code"), vec!["sys\\|code"]);
    }

    fn param(
        name: &str,
        param_type: SearchParamType,
        modifier: Option<SearchModifier>,
        values: &[&str],
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            ..Default::default()
        }
    }

    /// The parameter the gate names, or `None` when it lets the query through.
    fn refused(query: &SearchQuery) -> Option<String> {
        match validate_value_presence(query) {
            Ok(()) => None,
            Err(StorageError::Search(SearchError::EmptyValue { param })) => Some(param),
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    fn refused_value(param_type: SearchParamType, raw: &str) -> bool {
        let values = split_unescaped_commas(raw);
        let values: Vec<&str> = values.iter().map(String::as_str).collect();
        refused(&SearchQuery::new("Patient").with_parameter(param("p", param_type, None, &values)))
            .is_some()
    }

    #[test]
    fn an_empty_value_or_alternative_is_refused_for_every_type() {
        use SearchParamType as T;
        for t in [
            T::String,
            T::Token,
            T::Reference,
            T::Uri,
            T::Date,
            T::Number,
            T::Quantity,
            T::Composite,
            T::Special,
        ] {
            for raw in ["a,", ",a", "a,,b", "", ",", ",,", " ", "a, ", " ,a"] {
                assert!(refused_value(t, raw), "{t} {raw:?}");
            }
        }
    }

    #[test]
    fn values_that_are_not_empty_pass() {
        use SearchParamType as T;
        for raw in [
            "a",
            "a,b",
            // An escaped comma is data: the value `a,`, then the values `,` and `a`.
            "a\\,",
            "\\,,a",
            // Token forms with an empty part are not empty values.
            "|",
            "http://loinc.org|",
            "|8480-6",
        ] {
            for t in [T::String, T::Token, T::Reference, T::Uri] {
                assert!(!refused_value(t, raw), "{t} {raw:?}");
            }
        }
    }

    #[test]
    fn the_error_names_the_parameter_as_written() {
        use SearchParamType as T;
        let query = |p| SearchQuery::new("Patient").with_parameter(p);
        assert_eq!(
            refused(&query(param("family", T::String, None, &["Zzz", ""]))).as_deref(),
            Some("family")
        );
        assert_eq!(
            refused(&query(param(
                "gender",
                T::Token,
                Some(SearchModifier::Not),
                &[""]
            )))
            .as_deref(),
            Some("gender:not")
        );
        assert_eq!(
            refused(&query(param(
                "identifier",
                T::Token,
                Some(SearchModifier::OfType),
                &[""]
            )))
            .as_deref(),
            Some("identifier:of-type")
        );
        // A `_filter` expression is its parser's business.
        assert_eq!(
            refused(&query(param(
                "_filter",
                T::Special,
                None,
                &["given eq \"a", "", "b\""]
            ))),
            None
        );
        let mut chained = param("subject", T::Reference, None, &["Zzz", ""]);
        chained.chain = vec![crate::types::ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "family".to_string(),
        }];
        assert_eq!(
            refused(&SearchQuery::new("Observation").with_parameter(chained)).as_deref(),
            Some("subject.family")
        );
        let error = validate_value_presence(&query(param("family", T::String, None, &[""])))
            .expect_err("refused")
            .to_string();
        assert!(
            error.contains("'family'") && error.contains("empty"),
            "{error}"
        );
    }

    #[test]
    fn missing_takes_a_boolean_and_is_left_alone() {
        let query = SearchQuery::new("Patient").with_parameter(param(
            "family",
            SearchParamType::String,
            Some(SearchModifier::Missing),
            &[""],
        ));
        assert_eq!(refused(&query), None);
    }

    #[test]
    fn a_string_that_folds_to_nothing_is_empty_except_under_exact() {
        let accent = "\u{301}";
        let query = |modifier| {
            SearchQuery::new("Patient").with_parameter(param(
                "family",
                SearchParamType::String,
                modifier,
                &[accent],
            ))
        };
        assert!(refused(&query(None)).is_some());
        assert!(refused(&query(Some(SearchModifier::Contains))).is_some());
        assert_eq!(refused(&query(Some(SearchModifier::Exact))), None);
        // Only strings are folded.
        let token = SearchQuery::new("Patient").with_parameter(param(
            "gender",
            SearchParamType::Token,
            None,
            &[accent],
        ));
        assert_eq!(refused(&token), None);
    }

    #[test]
    fn a_composite_with_an_empty_component_is_refused() {
        let composite = |value| {
            SearchQuery::new("Observation").with_parameter(param(
                "code-value-quantity",
                SearchParamType::Composite,
                None,
                &[value],
            ))
        };
        for value in ["$5.4", "8480-6$", "$", "8480-6$ ", "a$$b"] {
            assert!(refused(&composite(value)).is_some(), "{value:?}");
        }
        // `\$` is data.
        for value in ["8480-6$5.4", "a\\$$5.4"] {
            assert_eq!(refused(&composite(value)), None, "{value:?}");
        }
    }

    #[test]
    fn a_has_value_is_split_before_it_is_judged() {
        let has = |search_param: &str, value: &str| {
            let mut query = SearchQuery::new("Patient");
            query.reverse_chains = vec![ReverseChainedParameter::terminal(
                "Observation",
                "subject",
                search_param,
                SearchValue::eq(value),
            )];
            query
        };
        for value in ["a,", ",a", "a,,b", "", ",", " "] {
            assert_eq!(
                refused(&has("code", value)).as_deref(),
                Some("_has:Observation:subject:code"),
                "{value:?}"
            );
            assert_eq!(
                refused(&has("code:text", value)).as_deref(),
                Some("_has:Observation:subject:code:text"),
                "{value:?}"
            );
        }
        for value in ["a", "a,b", "a\\,"] {
            assert_eq!(refused(&has("code", value)), None, "{value:?}");
        }
        assert_eq!(refused(&has("code:missing", "")), None);

        // The terminal of a nested `_has` carries the value.
        let mut nested = SearchQuery::new("Patient");
        nested.reverse_chains = vec![ReverseChainedParameter::nested(
            "Observation",
            "subject",
            ReverseChainedParameter::terminal(
                "Provenance",
                "target",
                "agent",
                SearchValue::eq("x,"),
            ),
        )];
        assert_eq!(
            refused(&nested).as_deref(),
            Some("_has:Provenance:target:agent")
        );
    }

    #[test]
    fn unregistered_date_shaped_value_gets_its_prefix_parsed() {
        let registry = SearchParameterRegistry::new();
        let (ty, values) = parse_typed_values(
            &registry,
            "Patient",
            "custom",
            &["ge1980-01-01".to_string()],
        );
        assert_eq!(ty, SearchParamType::Date);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].prefix, SearchPrefix::Ge);
        assert_eq!(values[0].value, "1980-01-01");
    }

    #[test]
    fn unregistered_string_value_keeps_prefix_like_letters() {
        let registry = SearchParameterRegistry::new();
        let (ty, values) = parse_typed_values(&registry, "Patient", "custom", &["Lee".to_string()]);
        assert_eq!(ty, SearchParamType::String);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].prefix, SearchPrefix::Eq);
        assert_eq!(values[0].value, "Lee");
    }
}
