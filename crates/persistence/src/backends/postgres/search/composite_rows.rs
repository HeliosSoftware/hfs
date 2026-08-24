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
                self.value_reference = Some(reference.clone())
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
        for member in members {
            if param_url.is_empty() {
                param_url = member.param_url.clone();
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

        if slots.is_empty() {
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
        ExtractedValue::new(param, "http://example.org/sp", ty, value)
            .with_composite_group(group)
            .with_composite_slot(slot)
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
}
