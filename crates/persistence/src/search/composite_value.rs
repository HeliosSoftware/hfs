//! Splitting composite search values, and the arity gate for them (#1236).
//!
//! `split_composite_value` splits a composite search value
//! (`http://loinc.org|8302-2$gt150`) into one typed value per declared
//! component. [`validate_composite_values`] is the search gate that refuses a
//! value whose `$`-separated part count is not the component count.
//!
//! SQLite, PostgreSQL and Elasticsearch used to answer such a value with an
//! empty Bundle, and MongoDB with a 400. The gate makes it a 400
//! (`InvalidComposite`) on every backend.
//!
//! The split is on every `$`, with no `\$` unescaping, because no backend's
//! composite builder unescapes it. The gate therefore counts exactly the parts
//! the builders see.
//!
//! This differs on purpose from `value_parser::split_composite_components` and
//! `numeric_value::split_unescaped`, which treat `\$` as data. So `a\$b$5`
//! counts as three parts here, and is refused on a two-component parameter,
//! because every composite builder would see three.

use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{
    CompositeSearchComponent, SearchModifier, SearchParamType, SearchPrefix, SearchQuery,
    SearchValue,
};

/// Comparison prefixes recognized on a composite component part, tried in
/// this order.
///
/// Prefixes are recognized only on Date/Number/Quantity components. SQLite and
/// PostgreSQL (`parse_component_value`) and Elasticsearch (`split_prefix`) keep
/// their own equivalent type-aware copies since #1256; merging them into this
/// one is a follow-up. A token code like `left`
/// (or `ge123`, a plausible identifier) must keep its letters, since
/// `ge`/`eq`/etc. are not comparison prefixes for token/string/reference/uri
/// values and stripping them would corrupt the value being searched for.
const PREFIXES: [(&str, SearchPrefix); 9] = [
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

/// Splits a raw composite value (`"http://loinc.org|8302-2$gt150"`) into one
/// [`SearchValue`] per declared component, in component order.
///
/// A leading comparison prefix is stripped only for Date/Number/Quantity
/// components — a token code like `ge123` must keep its letters, since
/// `ge`/`eq`/etc. are not meaningful prefixes for token/string/reference/uri
/// values.
///
/// Returns [`SearchError::InvalidComposite`] when there are no declared
/// components, or the value's `$`-separated part count does not match the
/// component count.
pub(crate) fn split_composite_value(
    raw: &str,
    components: &[CompositeSearchComponent],
) -> StorageResult<Vec<SearchValue>> {
    if components.is_empty() {
        return Err(StorageError::Search(SearchError::InvalidComposite {
            message: "composite search parameter has no declared components".to_string(),
        }));
    }

    let parts: Vec<&str> = raw.split('$').collect();
    if parts.len() != components.len() {
        return Err(StorageError::Search(SearchError::InvalidComposite {
            message: format!(
                "composite value '{raw}' has {} component(s) separated by '$', but this \
                 parameter expects {}",
                parts.len(),
                components.len()
            ),
        }));
    }

    Ok(parts
        .into_iter()
        .zip(components.iter())
        .map(|(part, component)| parse_component_part(part, component.param_type))
        .collect())
}

/// Parses one `$`-separated part of a composite value into a [`SearchValue`],
/// stripping a leading comparison prefix only for ordered types (Date,
/// Number, Quantity). Token/String/Reference/Uri parts are kept verbatim
/// with [`SearchPrefix::Eq`].
fn parse_component_part(part: &str, param_type: SearchParamType) -> SearchValue {
    if !matches!(
        param_type,
        SearchParamType::Date | SearchParamType::Number | SearchParamType::Quantity
    ) {
        return SearchValue::new(SearchPrefix::Eq, part);
    }

    for (prefix_str, prefix) in PREFIXES {
        if let Some(stripped) = part.strip_prefix(prefix_str) {
            return SearchValue::new(prefix, stripped);
        }
    }

    SearchValue::new(SearchPrefix::Eq, part)
}

/// Refuses a composite value whose `$`-separated part count is not the
/// parameter's component count, as [`SearchError::InvalidComposite`].
///
/// Call it beside the other search gates from every entry point that builds a
/// backend query, AFTER [`super::validate_value_presence`], so that an empty
/// value or alternative (`code-value-quantity=8480-6$5.4,`) is still reported
/// as empty.
///
/// It skips `:missing` (whose value is a boolean), chained parameters, and
/// composites whose components were not resolved (there is nothing to count
/// against; each backend keeps its own handling of that). The chain resolver
/// builds composite terminal searches (`subject:Patient.<composite>`,
/// `_has:...:<composite>`) with no components, so this gate skips them like any
/// unresolved composite. They keep their earlier behaviour whatever the arity:
/// SQLite, PostgreSQL and Elasticsearch match nothing, and MongoDB answers 400
/// "no declared components". Resolving components for chain terminals is out of
/// scope for #1236.
pub fn validate_composite_values(query: &SearchQuery) -> StorageResult<()> {
    for param in &query.parameters {
        if param.param_type != SearchParamType::Composite
            || param.components.is_empty()
            || !param.chain.is_empty()
            || matches!(param.modifier, Some(SearchModifier::Missing))
        {
            continue;
        }
        for value in &param.values {
            split_composite_value(&value.value, &param.components)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChainedParameter, SearchParameter};

    fn token(name: &str) -> CompositeSearchComponent {
        CompositeSearchComponent {
            param_type: SearchParamType::Token,
            param_name: name.to_string(),
        }
    }

    fn quantity(name: &str) -> CompositeSearchComponent {
        CompositeSearchComponent {
            param_type: SearchParamType::Quantity,
            param_name: name.to_string(),
        }
    }

    fn component(param_type: SearchParamType, name: &str) -> CompositeSearchComponent {
        CompositeSearchComponent {
            param_type,
            param_name: name.to_string(),
        }
    }

    #[test]
    fn arity_mismatch_too_few_parts_errors() {
        let components = vec![token("code"), quantity("value-quantity")];
        let err = split_composite_value("8302-2", &components).unwrap_err();
        match err {
            StorageError::Search(SearchError::InvalidComposite { message }) => {
                assert!(
                    message.contains("has 1 component(s)"),
                    "message should name 1 part: {message}"
                );
                assert!(
                    message.contains("expects 2"),
                    "message should name 2 expected: {message}"
                );
            }
            other => panic!("expected InvalidComposite, got {other:?}"),
        }
    }

    #[test]
    fn arity_mismatch_too_many_parts_errors() {
        let components = vec![token("code"), quantity("value-quantity")];
        let err = split_composite_value("a$b$c", &components).unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::InvalidComposite { .. })
        ));
    }

    #[test]
    fn no_declared_components_errors() {
        let err = split_composite_value("a$b", &[]).unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::InvalidComposite { .. })
        ));
    }

    #[test]
    fn quantity_component_strips_known_prefix() {
        let components = vec![token("code"), quantity("value-quantity")];
        let values = split_composite_value("8302-2$gt150", &components).unwrap();
        assert_eq!(values[0].prefix, SearchPrefix::Eq);
        assert_eq!(values[0].value, "8302-2");
        assert_eq!(values[1].prefix, SearchPrefix::Gt);
        assert_eq!(values[1].value, "150");
    }

    #[test]
    fn token_component_keeps_prefix_like_letters_verbatim() {
        let components = vec![token("code"), quantity("value-quantity")];
        // "ge123" looks like a `ge` prefix, but this component is a Token, so
        // the whole string must survive untouched.
        let values = split_composite_value("ge123$50", &components).unwrap();
        assert_eq!(values[0].prefix, SearchPrefix::Eq);
        assert_eq!(values[0].value, "ge123");
    }

    #[test]
    fn token_component_with_system_is_untouched() {
        let components = vec![token("code"), quantity("value-quantity")];
        let values = split_composite_value("http://loinc.org|8302-2$150", &components).unwrap();
        assert_eq!(values[0].prefix, SearchPrefix::Eq);
        assert_eq!(values[0].value, "http://loinc.org|8302-2");
    }

    #[test]
    fn date_and_number_components_strip_their_prefix() {
        let date = vec![token("code"), component(SearchParamType::Date, "date")];
        let values = split_composite_value("x$ge2024-01-01", &date).unwrap();
        assert_eq!(values[1].prefix, SearchPrefix::Ge);
        assert_eq!(values[1].value, "2024-01-01");

        let number = vec![token("code"), component(SearchParamType::Number, "count")];
        let values = split_composite_value("x$sa5", &number).unwrap();
        assert_eq!(values[1].prefix, SearchPrefix::Sa);
        assert_eq!(values[1].value, "5");
    }

    fn composite_query(values: &[&str]) -> SearchQuery {
        SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code-value-quantity".into(),
            param_type: SearchParamType::Composite,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            components: vec![token("code"), quantity("value-quantity")],
            ..Default::default()
        })
    }

    #[test]
    fn gate_refuses_a_value_with_the_wrong_number_of_parts() {
        match validate_composite_values(&composite_query(&["8302-2"])) {
            Err(StorageError::Search(SearchError::InvalidComposite { message })) => {
                assert!(message.contains("has 1 component(s)"), "{message}");
                assert!(message.contains("expects 2"), "{message}");
            }
            other => panic!("expected InvalidComposite, got {other:?}"),
        }
        assert!(matches!(
            validate_composite_values(&composite_query(&["a$b$c"])),
            Err(StorageError::Search(SearchError::InvalidComposite { .. }))
        ));
        // Any OR alternative with the wrong arity is refused.
        assert!(matches!(
            validate_composite_values(&composite_query(&["8302-2$gt150", "8302-2"])),
            Err(StorageError::Search(SearchError::InvalidComposite { .. }))
        ));
    }

    #[test]
    fn gate_accepts_one_part_per_component() {
        assert!(validate_composite_values(&composite_query(&["8302-2$gt150"])).is_ok());
        assert!(
            validate_composite_values(&composite_query(&["http://loinc.org|8302-2$150"])).is_ok()
        );
    }

    #[test]
    fn gate_counts_a_backslash_dollar_as_a_separator() {
        // Pins the raw-`$` split: aligning it with value_parser's escape
        // handling has to be a deliberate change.
        match validate_composite_values(&composite_query(&[r"a\$b$5"])) {
            Err(StorageError::Search(SearchError::InvalidComposite { message })) => {
                assert!(message.contains("has 3 component(s)"), "{message}");
            }
            other => panic!("expected InvalidComposite, got {other:?}"),
        }
    }

    #[test]
    fn gate_skips_what_it_cannot_count() {
        // `:missing` carries a boolean, not a composite value.
        let missing = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code-value-quantity".into(),
            param_type: SearchParamType::Composite,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("true")],
            components: vec![token("code"), quantity("value-quantity")],
            ..Default::default()
        });
        assert!(validate_composite_values(&missing).is_ok());

        // A chained parameter is skipped; see the `validate_composite_values` doc.
        let chained = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code-value-quantity".into(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq("8302-2")],
            chain: vec![ChainedParameter {
                reference_param: "subject".into(),
                target_type: None,
                target_param: "x".into(),
            }],
            components: vec![token("code"), quantity("value-quantity")],
            ..Default::default()
        });
        assert!(validate_composite_values(&chained).is_ok());

        // Unresolved components leave nothing to count against.
        let unresolved = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code-value-quantity".into(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq("8302-2")],
            ..Default::default()
        });
        assert!(validate_composite_values(&unresolved).is_ok());

        // A non-composite parameter may contain `$` freely.
        let plain = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".into(),
            param_type: SearchParamType::Token,
            values: vec![SearchValue::eq("a$b")],
            ..Default::default()
        });
        assert!(validate_composite_values(&plain).is_ok());
    }
}
