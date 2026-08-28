//! Folding a composite parameter's extracted components into denormalized rows.
//!
//! Postgres stores a composite instance as ONE `search_index` row carrying every
//! component's value, rather than one row per component (issue #279). That turns
//! "code = X AND value > Y within the same composite instance" into a plain
//! conjunction a single index can answer, instead of a grouped aggregate that had
//! to read ~110k rows over ~108k heap blocks to return 21.
//!
//! This module is the pure part of that change: it takes the extractor's
//! per-component [`ExtractedValue`]s and produces the rows to insert. It holds no
//! SQL and no client, so it is unit-testable without a database — which matters,
//! because the cross-product below is the easiest thing here to get wrong.
//!
//! ## Cross-product
//!
//! A component's expression may yield several values — a `CodeableConcept` with
//! two codings is ordinary, not exotic. Each *combination* of component values is
//! a distinct match, so a group with 2 codes and 1 quantity produces 2 rows. That
//! is what preserves the semantics of the grouped form: any (code, value) pair
//! that the old `MAX(CASE …)` HAVING would have accepted appears as some row here.

use crate::search::converters::IndexValue;
use crate::search::extractor::ExtractedValue;

/// One denormalized composite row, ready to insert.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct CompositeRow {
    pub param_name: String,
    pub param_url: String,
    pub composite_group: i32,
    pub value_token_system: Option<String>,
    pub value_token_code: Option<String>,
    pub value_token_system_2: Option<String>,
    pub value_token_code_2: Option<String>,
    pub value_string: Option<String>,
    pub value_date: Option<String>,
    pub value_number: Option<f64>,
    pub value_number_2: Option<f64>,
    pub value_quantity_value: Option<f64>,
    pub value_quantity_unit: Option<String>,
    pub value_quantity_system: Option<String>,
    pub value_reference: Option<String>,
    pub value_uri: Option<String>,
}

impl CompositeRow {
    /// Places one component's value into the columns for its slot.
    ///
    /// Slot 2 targets the `_2` columns; only token and number ever need it (max
    /// observed per family across the 46 R4 composites is 2).
    fn place(&mut self, slot: u8, value: &IndexValue) {
        let second = slot >= 2;
        match value {
            IndexValue::Token { system, code, .. } => {
                if second {
                    self.value_token_system_2 = system.clone();
                    self.value_token_code_2 = Some(code.clone());
                } else {
                    self.value_token_system = system.clone();
                    self.value_token_code = Some(code.clone());
                }
            }
            IndexValue::Number(n) => {
                if second {
                    self.value_number_2 = Some(*n);
                } else {
                    self.value_number = Some(*n);
                }
            }
            IndexValue::Quantity {
                value,
                unit,
                system,
                ..
            } => {
                self.value_quantity_value = Some(*value);
                self.value_quantity_unit = unit.clone();
                self.value_quantity_system = system.clone();
            }
            IndexValue::String(s) => self.value_string = Some(s.clone()),
            IndexValue::Date { value, .. } => self.value_date = Some(value.clone()),
            IndexValue::Reference { reference, .. } => {
                // Same normalization as `IndexRow::from_extracted` — a composite
                // reference component is read by the same predicates.
                self.value_reference =
                    Some(crate::types::strip_reference_version(reference).to_string())
            }
            IndexValue::Uri(u) => self.value_uri = Some(u.clone()),
        }
    }
}

/// Splits extracted values into the non-composite ones (written unchanged, one
/// row each) and the denormalized composite rows.
///
/// Composite values are keyed by `(param_name, composite_group)`; within a group
/// they are bucketed by slot and crossed, so every combination of component
/// values becomes one row.
///
/// ## Incomplete groups are dropped
///
/// A composite search is a conjunction over ALL of the parameter's components —
/// `code-value-quantity=8480-6$gt90` constrains both. A group that produced
/// fewer axes than the definition declares is therefore unreachable: no
/// well-formed composite query can match it. Such groups used to be written
/// anyway, and they dominate the table on real data — an Observation with a
/// `code` and a `valueQuantity` still got a `code-value-date` row and a
/// `code-value-string` row, one per code, purely because `code` was present.
/// On the benchmark corpus that was ~5M of 39.5M index rows, every one of them
/// paying an insert, an index maintenance cost and a referential-integrity
/// check on write.
///
/// These rows are unreachable by construction on this backend, not merely by
/// the spec's reading: `build_composite_condition` splits the supplied value on
/// `$` and emits `1 = 0` when the part count does not equal the definition's
/// component count, so a query naming fewer components than the parameter has
/// already returns nothing. A partial row can therefore never be the reason a
/// search does or does not match. (The SQLite backend agrees from the other
/// direction — its grouped `HAVING` requires every queried component present.)
pub(crate) fn fold_composites(
    values: Vec<ExtractedValue>,
) -> (Vec<ExtractedValue>, Vec<CompositeRow>) {
    let mut plain = Vec::new();
    // Preserve first-seen order so output is deterministic (tests, and stable
    // insert order for anyone reading the table).
    let mut groups: Vec<((String, u32), Vec<ExtractedValue>)> = Vec::new();

    for value in values {
        match value.composite_group {
            None => plain.push(value),
            Some(group) => {
                let key = (value.param_name.clone(), group);
                match groups.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, bucket)) => bucket.push(value),
                    None => groups.push((key, vec![value])),
                }
            }
        }
    }

    let mut rows = Vec::new();
    for ((param_name, group), members) in groups {
        // Bucket by slot, keeping component order.
        let mut slots: Vec<(u8, Vec<ExtractedValue>)> = Vec::new();
        let mut param_url = String::new();
        let mut arity: Option<u8> = None;
        for member in members {
            if param_url.is_empty() {
                param_url = member.param_url.clone();
            }
            // Every member of a group carries the same arity; take the first.
            arity = arity.or(member.composite_arity);
            if is_codeless_token(&member.value) {
                continue;
            }
            let slot = member.composite_slot.unwrap_or(1);
            // A component of a *different* family in the same slot number is a
            // separate axis of the cross-product, so key on (slot, family) via
            // the discriminant of the value rather than the slot alone.
            let axis = (slot, family_of(&member.value));
            match slots
                .iter_mut()
                .find(|(s, vs)| (*s, family_of(&vs[0].value)) == axis)
            {
                Some((_, bucket)) => bucket.push(member),
                None => slots.push((slot, vec![member])),
            }
        }

        // An instance that is missing a component can never satisfy a composite
        // search, so it contributes no rows. `arity` is the number of axes a
        // complete instance has; `slots` is how many this one actually produced.
        // Values that predate the arity field (or come from a backend that does
        // not set it) fall back to the old "any non-empty group" rule.
        let required = arity.map(usize::from).unwrap_or(1);
        if slots.len() < required {
            continue;
        }

        // Cross the axes: start with one empty row and multiply by each axis.
        let mut partial = vec![CompositeRow {
            param_name: param_name.clone(),
            param_url: param_url.clone(),
            composite_group: group as i32,
            ..Default::default()
        }];
        for (slot, members) in &slots {
            let mut next = Vec::with_capacity(partial.len() * members.len());
            for base in &partial {
                for member in members {
                    let mut row = base.clone();
                    row.place(*slot, &member.value);
                    next.push(row);
                }
            }
            partial = next;
        }
        rows.extend(partial);
    }

    (plain, rows)
}

/// Whether a token value carries no code, and so can be no composite axis.
///
/// `ValueConverter` indexes a `CodeableConcept.text` that no coding's `display`
/// already carries as a token with an empty code — a row that exists purely so
/// `:text` has something to match. `build_composite_condition` compares only
/// `value_token_system` / `value_token_code`, never a display column, so such a
/// value is an axis entry no composite query can name: the only search that
/// could select it is one supplying an empty token component, and
/// `build_composite_condition` already answers a value with the wrong number of
/// `$`-separated parts with `1 = 0`.
///
/// Keeping it in the cross-product is therefore pure multiplication. On the row
/// census for run 33029355759 a Synthea Observation carries one real coding plus
/// one text row on both `code` and `valueCodeableConcept`, so
/// `code-value-concept` wrote 2 x 2 = 4 rows per complete instance where 1 does
/// the same work, and `combo-code-value-quantity` wrote 2 x 1 where 1 x 1 does.
///
/// A group whose *only* value on some axis was code-less loses that axis and is
/// then dropped by the arity check below — correctly, because that instance has
/// no code for a composite search to name.
fn is_codeless_token(value: &IndexValue) -> bool {
    matches!(value, IndexValue::Token { code, .. } if code.is_empty())
}

/// The column family a value lands in — the axis identity for the cross-product.
fn family_of(value: &IndexValue) -> u8 {
    match value {
        IndexValue::Token { .. } => 0,
        IndexValue::String(_) => 1,
        IndexValue::Date { .. } => 2,
        IndexValue::Number(_) => 3,
        IndexValue::Quantity { .. } => 4,
        IndexValue::Reference { .. } => 5,
        IndexValue::Uri(_) => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchParamType;

    fn token(code: &str) -> IndexValue {
        IndexValue::Token {
            system: Some("http://loinc.org".to_string()),
            code: code.to_string(),
            display: None,
            identifier_type_system: None,
            identifier_type_code: None,
        }
    }

    fn quantity(v: f64) -> IndexValue {
        IndexValue::Quantity {
            value: v,
            unit: Some("mm[Hg]".to_string()),
            system: None,
            code: None,
        }
    }

    fn component(
        param: &str,
        ty: SearchParamType,
        value: IndexValue,
        group: u32,
        slot: u8,
    ) -> ExtractedValue {
        // Every composite exercised here is two-component, which is also the
        // arity the extractor would record for them.
        ExtractedValue::new(param, "http://example.org/sp", ty, value)
            .with_composite_group(group)
            .with_composite_slot(slot)
            .with_composite_arity(2)
    }

    #[test]
    fn token_plus_quantity_folds_to_one_row() {
        let (plain, rows) = fold_composites(vec![
            component(
                "code-value-quantity",
                SearchParamType::Token,
                token("8480-6"),
                0,
                1,
            ),
            component(
                "code-value-quantity",
                SearchParamType::Quantity,
                quantity(140.0),
                0,
                1,
            ),
        ]);
        assert!(plain.is_empty());
        assert_eq!(rows.len(), 1, "one composite instance is one row");
        assert_eq!(rows[0].value_token_code.as_deref(), Some("8480-6"));
        assert_eq!(rows[0].value_quantity_value, Some(140.0));
        assert_eq!(rows[0].composite_group, 0);
    }

    #[test]
    fn two_token_components_use_distinct_slots() {
        // Observation.code-value-concept: token + token. Without slotting these
        // would overwrite one another and the parameter would be unsearchable.
        let (_, rows) = fold_composites(vec![
            component(
                "code-value-concept",
                SearchParamType::Token,
                token("8480-6"),
                0,
                1,
            ),
            component(
                "code-value-concept",
                SearchParamType::Token,
                token("LA6699-8"),
                0,
                2,
            ),
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value_token_code.as_deref(), Some("8480-6"));
        assert_eq!(rows[0].value_token_code_2.as_deref(), Some("LA6699-8"));
    }

    #[test]
    fn separate_groups_never_share_a_row() {
        // The blood-pressure panel: systolic in group 0, diastolic in group 1.
        // A row that mixed them would let ?code-value-quantity=systolic$gt90
        // match on the diastolic value — the exact bug the GROUP BY prevented.
        let (_, rows) = fold_composites(vec![
            component(
                "component-code-value-quantity",
                SearchParamType::Token,
                token("8480-6"),
                0,
                1,
            ),
            component(
                "component-code-value-quantity",
                SearchParamType::Quantity,
                quantity(140.0),
                0,
                1,
            ),
            component(
                "component-code-value-quantity",
                SearchParamType::Token,
                token("8462-4"),
                1,
                1,
            ),
            component(
                "component-code-value-quantity",
                SearchParamType::Quantity,
                quantity(90.0),
                1,
                1,
            ),
        ]);
        assert_eq!(rows.len(), 2);
        let systolic = rows.iter().find(|r| r.composite_group == 0).unwrap();
        let diastolic = rows.iter().find(|r| r.composite_group == 1).unwrap();
        assert_eq!(systolic.value_token_code.as_deref(), Some("8480-6"));
        assert_eq!(systolic.value_quantity_value, Some(140.0));
        assert_eq!(diastolic.value_token_code.as_deref(), Some("8462-4"));
        assert_eq!(diastolic.value_quantity_value, Some(90.0));
    }

    #[test]
    fn multivalued_component_produces_the_cross_product() {
        // A CodeableConcept with two codings is ordinary. Both codes must be
        // searchable against the same quantity, so the group yields two rows —
        // exactly the pairs the old MAX(CASE …) HAVING would have accepted.
        let (_, rows) = fold_composites(vec![
            component(
                "code-value-quantity",
                SearchParamType::Token,
                token("8480-6"),
                0,
                1,
            ),
            component(
                "code-value-quantity",
                SearchParamType::Token,
                token("271649006"),
                0,
                1,
            ),
            component(
                "code-value-quantity",
                SearchParamType::Quantity,
                quantity(140.0),
                0,
                1,
            ),
        ]);
        assert_eq!(rows.len(), 2, "2 codes x 1 quantity = 2 rows");
        assert!(rows.iter().all(|r| r.value_quantity_value == Some(140.0)));
        let mut codes: Vec<_> = rows
            .iter()
            .map(|r| r.value_token_code.clone().unwrap())
            .collect();
        codes.sort();
        assert_eq!(codes, vec!["271649006", "8480-6"]);
    }

    #[test]
    fn a_group_missing_a_component_produces_no_row() {
        // The dominant shape on real data: an Observation carries a `code` but
        // no `valueDateTime`, so `code-value-date` has only its token axis. No
        // well-formed `code-value-date=<token>$<date>` query can match it, and
        // on the benchmark corpus these were ~5M unreachable rows.
        let (_, rows) = fold_composites(vec![component(
            "code-value-date",
            SearchParamType::Token,
            token("8480-6"),
            0,
            1,
        )]);
        assert!(rows.is_empty(), "an incomplete instance is unsearchable");
    }

    #[test]
    fn one_incomplete_group_does_not_suppress_a_complete_one() {
        // Two component groups on one Observation: the first has both axes, the
        // second only its code. Dropping is per-group, not per-parameter.
        let (_, rows) = fold_composites(vec![
            component(
                "component-code-value-quantity",
                SearchParamType::Token,
                token("8480-6"),
                0,
                1,
            ),
            component(
                "component-code-value-quantity",
                SearchParamType::Quantity,
                quantity(140.0),
                0,
                1,
            ),
            component(
                "component-code-value-quantity",
                SearchParamType::Token,
                token("8462-4"),
                1,
                1,
            ),
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].composite_group, 0);
        assert_eq!(rows[0].value_quantity_value, Some(140.0));
    }

    #[test]
    fn values_without_a_recorded_arity_keep_the_old_rule() {
        // Backends and call sites that never set `composite_arity` must not
        // silently lose rows; the fallback is the previous "any non-empty
        // group" behaviour.
        let legacy = ExtractedValue::new(
            "code-value-quantity",
            "http://example.org/sp",
            SearchParamType::Token,
            token("8480-6"),
        )
        .with_composite_group(0)
        .with_composite_slot(1);
        let (_, rows) = fold_composites(vec![legacy]);
        assert_eq!(rows.len(), 1);
    }

    fn text_token(text: &str) -> IndexValue {
        IndexValue::Token {
            system: None,
            code: String::new(),
            display: Some(text.to_string()),
            identifier_type_system: None,
            identifier_type_code: None,
        }
    }

    #[test]
    fn a_code_less_text_token_is_not_a_composite_axis() {
        // The dominant Synthea shape: `code` is one LOINC coding plus a
        // `CodeableConcept.text` whose wording differs from the coding display,
        // so the text keeps a row of its own. Crossing it with the quantity
        // doubled the composite for a row `build_composite_condition` can never
        // name.
        let (_, rows) = fold_composites(vec![
            component(
                "code-value-quantity",
                SearchParamType::Token,
                token("8480-6"),
                0,
                1,
            ),
            component(
                "code-value-quantity",
                SearchParamType::Token,
                text_token("Systolic blood pressure"),
                0,
                1,
            ),
            component(
                "code-value-quantity",
                SearchParamType::Quantity,
                quantity(140.0),
                0,
                1,
            ),
        ]);
        assert_eq!(rows.len(), 1, "1 real code x 1 quantity = 1 row");
        assert_eq!(rows[0].value_token_code.as_deref(), Some("8480-6"));
        assert_eq!(rows[0].value_quantity_value, Some(140.0));
    }

    #[test]
    fn both_axes_shed_their_text_tokens() {
        // token x token: `code-value-concept` wrote 2 x 2 = 4 rows per instance
        // on the benchmark corpus, three of them unreachable.
        let (_, rows) = fold_composites(vec![
            component(
                "code-value-concept",
                SearchParamType::Token,
                token("8480-6"),
                0,
                1,
            ),
            component(
                "code-value-concept",
                SearchParamType::Token,
                text_token("Systolic blood pressure"),
                0,
                1,
            ),
            component(
                "code-value-concept",
                SearchParamType::Token,
                token("LA6699-8"),
                0,
                2,
            ),
            component(
                "code-value-concept",
                SearchParamType::Token,
                text_token("Elevated"),
                0,
                2,
            ),
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value_token_code.as_deref(), Some("8480-6"));
        assert_eq!(rows[0].value_token_code_2.as_deref(), Some("LA6699-8"));
    }

    #[test]
    fn a_group_whose_only_code_is_text_produces_no_row() {
        // A CodeableConcept with `text` and no coding has nothing a composite
        // search can name, so the instance is unsearchable and contributes
        // nothing — the same conclusion the arity check reaches for a group
        // that is missing a component outright.
        let (_, rows) = fold_composites(vec![
            component(
                "code-value-quantity",
                SearchParamType::Token,
                text_token("Systolic blood pressure"),
                0,
                1,
            ),
            component(
                "code-value-quantity",
                SearchParamType::Quantity,
                quantity(140.0),
                0,
                1,
            ),
        ]);
        assert!(rows.is_empty());
    }

    #[test]
    fn non_composite_values_pass_through_untouched() {
        let plain_in = ExtractedValue::new(
            "code",
            "http://example.org/sp",
            SearchParamType::Token,
            token("8480-6"),
        );
        let (plain, rows) = fold_composites(vec![plain_in.clone()]);
        assert!(rows.is_empty());
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].param_name, plain_in.param_name);
    }

    /// A composite component carrying a reference is read by exactly the
    /// predicates a plain reference row is, so it is normalized on the same
    /// terms as `IndexRow::from_extracted` — see schema v32.
    #[test]
    fn a_composite_reference_component_is_version_stripped() {
        let mut row = CompositeRow::default();
        row.place(
            1,
            &IndexValue::Reference {
                reference: "Patient/1/_history/9".to_string(),
                resource_type: None,
                resource_id: None,
                display: None,
            },
        );
        assert_eq!(row.value_reference.as_deref(), Some("Patient/1"));
    }
}
