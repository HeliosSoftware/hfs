//! Parsing of raw search-parameter values into typed [`SearchValue`]s.
//!
//! This is the single routine that turns the text after `=` in a FHIR search
//! into an OR-list of values: it splits on unescaped commas, resolves the
//! parameter's type from the registry, and strips a comparator prefix
//! (`ge`, `lt`, …) only for the types that define one. Direct parameters (the
//! REST query builder) and the terminal parameter of a chained / `_has` search
//! (the [chain resolver](super::chain_resolver)) both go through it, so a
//! chained `birthdate=ge1980-01-01` means exactly what the direct one does.

use crate::types::{SearchModifier, SearchParamType, SearchValue};

use super::registry::{SearchParameterRegistry, resolve_param_type};

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
pub fn parse_typed_values(
    registry: &SearchParameterRegistry,
    resource_type: &str,
    name: &str,
    raw_values: &[String],
) -> (SearchParamType, Vec<SearchValue>) {
    let tentative_values: Vec<SearchValue> =
        raw_values.iter().map(|v| SearchValue::parse(v)).collect();

    let param_type = resolve_param_type(registry, resource_type, name, &tentative_values);

    let values = if matches!(
        param_type,
        SearchParamType::Date | SearchParamType::Number | SearchParamType::Quantity
    ) {
        tentative_values
    } else {
        raw_values
            .iter()
            .map(|v| SearchValue::eq(v.clone()))
            .collect()
    };

    (param_type, values)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchPrefix;

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
