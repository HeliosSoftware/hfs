//! Pure helpers for MongoDB composite search parameters (#1206).
//!
//! Composite rows are stored one `search_index` document per component
//! value, all sharing `param_name` = the composite's own code and a
//! `composite_group` (the base-instance index) — the same layout SQLite
//! queries. A value `A$B` matches a resource when some `composite_group`
//! under the composite parameter has a row satisfying A and a row
//! satisfying B. Nothing here talks to MongoDB: the Mongo-facing wiring
//! (probing, per-batch pair checks) lives in `search_impl.rs`.
//! The value splitter shared with every backend is
//! `crate::search::composite_value::split_composite_value`.

use std::collections::{HashMap, HashSet};

use mongodb::bson::{Bson, Document, doc};

use crate::types::{CompositeSearchComponent, SearchParamType, SearchParameter, SearchValue};

/// The field the quantity row of a token+quantity composite group carries: the
/// bare code and the `system|code` of every token row of the same
/// `composite_group`, so `idx_search_composite_tuple_qty` pairs the code with
/// the value without a second row.
pub(super) const TUPLE_CODE_FIELD: &str = "tuple_code";

/// Copies the codes of each composite group's token rows into the group's
/// quantity rows as `tuple_code`. A group without both a token row and a
/// quantity row is left alone, as is every row without a `composite_group`.
pub(super) fn stamp_tuple_codes(rows: &mut [Document]) {
    let mut codes: HashMap<(String, i32), Vec<String>> = HashMap::new();
    for row in rows.iter() {
        let (Ok(param), Ok(group), Ok(code)) = (
            row.get_str("param_name"),
            row.get_i32("composite_group"),
            row.get_str("value_token_code"),
        ) else {
            continue;
        };
        let forms = codes.entry((param.to_string(), group)).or_default();
        let mut push = |form: String| {
            if !forms.contains(&form) {
                forms.push(form);
            }
        };
        push(code.to_string());
        if let Ok(system) = row.get_str("value_token_system") {
            push(format!("{system}|{code}"));
        }
    }
    if codes.is_empty() {
        return;
    }
    for row in rows.iter_mut() {
        if !row.contains_key("value_quantity_value") {
            continue;
        }
        let (Ok(param), Ok(group)) = (row.get_str("param_name"), row.get_i32("composite_group"))
        else {
            continue;
        };
        if let Some(forms) = codes.get(&(param.to_string(), group)) {
            let forms: Vec<Bson> = forms.iter().cloned().map(Bson::String).collect();
            row.insert(TUPLE_CODE_FIELD, forms);
        }
    }
}

/// The `tuple_code` condition equivalent to the token component `value` of a
/// token+quantity composite (`code` or `system|code`), or `None` when the form
/// is not an equality on a code (`|code`, `system|`, empty): those keep the
/// pair-validation path.
pub(super) fn tuple_code_condition(value: &str) -> Option<Bson> {
    match value.split_once('|') {
        None => (!value.is_empty()).then(|| Bson::String(value.to_string())),
        Some((system, code)) if !system.is_empty() && !code.is_empty() => {
            // Same systems the token filter accepts: the named one and the
            // implicit-system marker of a `code` element.
            let forms: Vec<Bson> = crate::search::implicit_system_candidates(system)
                .iter()
                .map(|system| Bson::String(format!("{system}|{code}")))
                .collect();
            Some(Bson::Document(doc! { "$in": forms }))
        }
        Some(_) => None,
    }
}

/// Whether `param` is a composite of exactly a token and a quantity, in that
/// order: the shape `tuple_code` is written for.
pub(super) fn is_token_quantity(param: &SearchParameter) -> bool {
    matches!(
        param.components.as_slice(),
        [token, quantity]
            if token.param_type == SearchParamType::Token
                && quantity.param_type == SearchParamType::Quantity
    )
}

/// Builds a synthetic single-value [`SearchParameter`] for one composite
/// component, suitable for passing to `MongoBackend::build_index_value_filter`.
///
/// `name` is set to the composite's own name because that is what every
/// `search_index` row for this composite carries in `param_name` — the
/// component has no row of its own to scope by.
pub(super) fn component_param(
    composite: &SearchParameter,
    component: &CompositeSearchComponent,
    value: SearchValue,
) -> SearchParameter {
    SearchParameter {
        name: composite.name.clone(),
        param_type: component.param_type,
        modifier: None,
        values: vec![value],
        chain: vec![],
        components: vec![],
    }
}

/// Intersects `(resource_id, composite_group)` pairs across components: a
/// resource id survives only if some single `composite_group` satisfies
/// every component (present in every set at that same pair).
///
/// Empty input (no components) yields an empty set.
pub(super) fn intersect_component_pairs(
    per_component: Vec<HashSet<(String, i32)>>,
) -> HashSet<String> {
    let mut iter = per_component.into_iter();
    let Some(mut surviving_pairs) = iter.next() else {
        return HashSet::new();
    };
    for next in iter {
        surviving_pairs.retain(|pair| next.contains(pair));
    }
    surviving_pairs.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_pairs_drops_group_where_only_one_component_matches() {
        // Component A matches in group 0 only; component B matches nothing.
        let a: HashSet<(String, i32)> = [("r1".to_string(), 0)].into_iter().collect();
        let b: HashSet<(String, i32)> = HashSet::new();
        let result = intersect_component_pairs(vec![a, b]);
        assert!(result.is_empty());
    }

    #[test]
    fn intersect_pairs_rejects_cross_group_match() {
        // r1: component A matches in group 0, component B matches in group 1.
        // Not the same group, so r1 must not match.
        let a: HashSet<(String, i32)> = [("r1".to_string(), 0)].into_iter().collect();
        let b: HashSet<(String, i32)> = [("r1".to_string(), 1)].into_iter().collect();
        let result = intersect_component_pairs(vec![a, b]);
        assert!(result.is_empty());
    }

    #[test]
    fn intersect_pairs_accepts_same_group_match() {
        // r1 matches both components in group 1.
        let a: HashSet<(String, i32)> = [("r1".to_string(), 0), ("r1".to_string(), 1)]
            .into_iter()
            .collect();
        let b: HashSet<(String, i32)> = [("r1".to_string(), 1)].into_iter().collect();
        let result = intersect_component_pairs(vec![a, b]);
        assert_eq!(result, ["r1".to_string()].into_iter().collect());
    }

    #[test]
    fn intersect_pairs_empty_input_yields_empty_set() {
        let result = intersect_component_pairs(vec![]);
        assert!(result.is_empty());
    }

    fn row(param: &str, group: i32, extra: Document) -> Document {
        let mut d = doc! { "param_name": param, "composite_group": group };
        d.extend(extra);
        d
    }

    #[test]
    fn stamp_copies_every_code_form_of_the_group_into_its_quantity_row() {
        let mut rows = vec![
            row(
                "code-value-quantity",
                0,
                doc! { "value_token_code": "8867-4", "value_token_system": "http://loinc.org" },
            ),
            row(
                "code-value-quantity",
                0,
                doc! { "value_token_code": "x" },
            ),
            row("code-value-quantity", 0, doc! { "value_quantity_value": 61.0 }),
            row(
                "code-value-quantity",
                1,
                doc! { "value_token_code": "other", "value_token_system": "s" },
            ),
            row("code-value-quantity", 1, doc! { "value_quantity_value": 1.0 }),
        ];
        stamp_tuple_codes(&mut rows);
        let tuple = |i: usize| rows[i].get_array(TUPLE_CODE_FIELD).ok().cloned();
        assert_eq!(
            tuple(2),
            Some(vec![
                Bson::from("8867-4"),
                Bson::from("http://loinc.org|8867-4"),
                Bson::from("x"),
            ])
        );
        assert_eq!(
            tuple(4),
            Some(vec![Bson::from("other"), Bson::from("s|other")])
        );
        assert!(tuple(0).is_none() && tuple(1).is_none() && tuple(3).is_none());
    }

    #[test]
    fn stamp_leaves_groups_without_both_halves_and_other_params_alone() {
        let mut rows = vec![
            row("code-value-quantity", 0, doc! { "value_quantity_value": 1.0 }),
            row("combo-code-value-quantity", 0, doc! { "value_token_code": "c" }),
            doc! { "param_name": "value-quantity", "value_quantity_value": 2.0 },
        ];
        stamp_tuple_codes(&mut rows);
        assert!(rows.iter().all(|r| !r.contains_key(TUPLE_CODE_FIELD)));
    }

    #[test]
    fn tuple_condition_accepts_only_code_equalities() {
        assert_eq!(tuple_code_condition("8867-4"), Some(Bson::from("8867-4")));
        let with_system = tuple_code_condition("http://loinc.org|8867-4").unwrap();
        assert_eq!(
            with_system,
            Bson::Document(doc! { "$in": [
                "http://loinc.org|8867-4",
                format!("{}|8867-4", crate::search::IMPLICIT_TOKEN_SYSTEM),
            ] })
        );
        for rejected in ["", "|c", "s|", "|"] {
            assert_eq!(tuple_code_condition(rejected), None, "{rejected}");
        }
    }
}
