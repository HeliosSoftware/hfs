//! Normalisation of empty representations in a resource before it is written
//! (#2554).
//!
//! FHIR's JSON format has no empty values: an object is never `{}`, an array
//! is never `[]`, and a property is never `null`. The one `null` it allows is
//! a placeholder in a primitive array, lining a value up with its extensions
//! in the `_name` array beside it:
//!
//! ```json
//! "given": ["Ann", null],
//! "_given": [null, {"extension": [{"url": "http://example.org/x", "valueString": "y"}]}]
//! ```
//!
//! [`drop_empty_elements`] removes those empty representations bottom up, as
//! HAPI's parser does, while keeping each primitive value with its
//! extensions: `{"type": [{"coding": []}]}` loses `coding`, then the emptied
//! item, then `type` itself. It does nothing else: empty strings, unknown
//! elements, cardinality, profiles and invariants are left to validation.
//!
//! It is applied on these write paths: the REST create, update and
//! conditional update bodies, batch and transaction `POST`/`PUT` entry
//! resources, every PATCH result (instance, conditional, batch and
//! transaction), and each `$bulk-submit` resource as written, after its
//! import mode is applied. Writes that call a storage backend directly are
//! not covered.

use serde_json::{Map, Value};

/// Removes every empty object, empty array and `null` member from `value`,
/// keeping primitive-array placeholders.
///
/// A primitive array `name` and its `_name` array are one sequence of
/// (value, extension) slots, the shorter array padded with implicit `null`s
/// (FHIR allows the two lengths to differ). Each slot is cleaned in place;
/// a slot with neither a value nor an extension is removed from both arrays
/// at the same index, so every remaining value keeps its extension; and an
/// array left holding only `null`s is omitted. Either array may be absent.
/// Applying it twice gives the same result as applying it once.
pub fn drop_empty_elements(value: &mut Value) {
    match value {
        Value::Object(object) => drop_empty_members(object),
        Value::Array(items) => drop_empty_items(items),
        _ => {}
    }
}

fn drop_empty_members(object: &mut Map<String, Value>) {
    let mut paired: Vec<String> = Vec::new();
    for (key, value) in object.iter() {
        if !value.is_array() {
            continue;
        }
        let base = match key.strip_prefix('_') {
            Some(base) if object.get(base).is_none_or(Value::is_array) => base,
            Some(_) => continue,
            None if object.get(&format!("_{key}")).is_some_and(Value::is_array) => key,
            None => continue,
        };
        if !paired.iter().any(|known| known == base) {
            paired.push(base.to_string());
        }
    }

    for (key, value) in object.iter_mut() {
        let in_pair = paired
            .iter()
            .any(|base| key == base || key.strip_prefix('_') == Some(base.as_str()));
        if !in_pair {
            drop_empty_elements(value);
        }
    }
    for base in &paired {
        drop_empty_slots(object, base);
    }

    object.retain(|_, value| !is_empty(value));
}

/// Cleans a complex array: items that are `null` or end up empty go.
fn drop_empty_items(items: &mut Vec<Value>) {
    for item in items.iter_mut() {
        drop_empty_elements(item);
    }
    items.retain(|item| !is_empty(item));
}

/// Cleans the `name` / `_name` pair as one sequence of slots (see
/// [`drop_empty_elements`]).
fn drop_empty_slots(object: &mut Map<String, Value>, base: &str) {
    let extension_key = format!("_{base}");
    for key in [base, extension_key.as_str()] {
        if let Some(Value::Array(items)) = object.get_mut(key) {
            for item in items.iter_mut() {
                drop_empty_elements(item);
                if is_empty(item) {
                    *item = Value::Null;
                }
            }
        }
    }

    let slot_is_null = |key: &str, index: usize| match object.get(key) {
        Some(Value::Array(items)) => items.get(index).is_none_or(Value::is_null),
        _ => true,
    };
    let length = [base, extension_key.as_str()]
        .iter()
        .filter_map(|key| object.get(*key).and_then(Value::as_array).map(Vec::len))
        .max()
        .unwrap_or(0);
    let keep: Vec<bool> = (0..length)
        .map(|index| !(slot_is_null(base, index) && slot_is_null(&extension_key, index)))
        .collect();

    for key in [base, extension_key.as_str()] {
        if let Some(Value::Array(items)) = object.get_mut(key) {
            let mut flags = keep.iter();
            items.retain(|_| *flags.next().unwrap_or(&true));
            if items.iter().all(Value::is_null) {
                items.clear();
            }
        }
    }
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Object(object) => object.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Cleans `value`, and checks that a second cleanup changes nothing.
    fn cleaned(mut value: Value) -> Value {
        drop_empty_elements(&mut value);
        let mut again = value.clone();
        drop_empty_elements(&mut again);
        assert_eq!(again, value, "a second cleanup must be a no-op");
        value
    }

    #[test]
    fn empty_object_array_and_null_members_are_removed() {
        let resource = json!({
            "resourceType": "InsurancePlan",
            "id": "empty-type",
            "status": "active",
            "type": [{}],
            "alias": [],
            "period": {},
            "name": null
        });
        assert_eq!(
            cleaned(resource),
            json!({"resourceType": "InsurancePlan", "id": "empty-type", "status": "active"})
        );
    }

    #[test]
    fn removal_is_bottom_up() {
        let resource = json!({
            "resourceType": "InsurancePlan",
            "type": [{"coding": [{"system": null, "extension": []}]}],
            "contact": [{"name": {"given": []}, "telecom": [null]}]
        });
        assert_eq!(cleaned(resource), json!({"resourceType": "InsurancePlan"}));
    }

    #[test]
    fn non_empty_siblings_survive() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{}, {"family": "Doe", "given": []}, null],
            "active": false,
            "multipleBirthInteger": 0,
            "gender": ""
        });
        assert_eq!(
            cleaned(resource),
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Doe"}],
                "active": false,
                "multipleBirthInteger": 0,
                "gender": ""
            })
        );
    }

    #[test]
    fn primitive_array_placeholders_are_kept() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{
                "given": ["Ann", null],
                "_given": [null, {"extension": [{"url": "http://example.org/x", "valueString": "y"}]}]
            }]
        });
        assert_eq!(cleaned(resource.clone()), resource);
    }

    #[test]
    fn an_all_null_value_array_is_omitted() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{
                "given": [null],
                "_given": [{"id": "g1"}]
            }]
        });
        assert_eq!(
            cleaned(resource),
            json!({"resourceType": "Patient", "name": [{"_given": [{"id": "g1"}]}]})
        );
    }

    #[test]
    fn an_emptied_pair_item_is_removed_from_both_arrays() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{
                "given": [null, "Ann", null, "Bo"],
                "_given": [{}, null, {"extension": [{"url": "http://example.org/x", "valueString": "y"}]}, {"extension": []}]
            }]
        });
        assert_eq!(
            cleaned(resource),
            json!({
                "resourceType": "Patient",
                "name": [{
                    "given": ["Ann", null, "Bo"],
                    "_given": [null, {"extension": [{"url": "http://example.org/x", "valueString": "y"}]}, null]
                }]
            })
        );
    }

    #[test]
    fn an_extension_array_of_placeholders_only_is_removed() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{"given": ["Ann", null], "_given": [null, {}]}]
        });
        assert_eq!(
            cleaned(resource),
            json!({"resourceType": "Patient", "name": [{"given": ["Ann"]}]})
        );
    }

    #[test]
    fn a_null_value_without_an_extension_array_is_removed() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{"given": ["Ann", null]}]
        });
        assert_eq!(
            cleaned(resource),
            json!({"resourceType": "Patient", "name": [{"given": ["Ann"]}]})
        );
    }

    #[test]
    fn a_pair_that_empties_entirely_is_removed() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{"given": [null], "_given": [{}], "family": "Doe"}]
        });
        assert_eq!(
            cleaned(resource),
            json!({"resourceType": "Patient", "name": [{"family": "Doe"}]})
        );
    }

    #[test]
    fn unequal_lengths_keep_every_slot_that_carries_something() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{
                "given": ["Ann", null, "Bo"],
                "_given": [null, {"id": "g1"}]
            }]
        });
        assert_eq!(cleaned(resource.clone()), resource);
    }

    /// The shorter array is padded with implicit nulls: the empty slots at
    /// both ends go, and `Ann` keeps its extension.
    #[test]
    fn a_shorter_value_array_is_aligned_with_implicit_nulls() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{"given": [null, "Ann"], "_given": [null, {"id": "g"}, null]}]
        });
        assert_eq!(
            cleaned(resource),
            json!({
                "resourceType": "Patient",
                "name": [{"given": ["Ann"], "_given": [{"id": "g"}]}]
            })
        );
    }

    #[test]
    fn a_shorter_extension_array_is_aligned_with_implicit_nulls() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{"given": ["a", null, "b", null], "_given": [{"id": "x"}]}]
        });
        assert_eq!(
            cleaned(resource),
            json!({
                "resourceType": "Patient",
                "name": [{"given": ["a", "b"], "_given": [{"id": "x"}]}]
            })
        );
    }

    #[test]
    fn an_extension_array_without_values_drops_its_empty_slots() {
        let extension = json!({"extension": [{"url": "http://example.org/x", "valueString": "y"}]});
        let resource = json!({
            "resourceType": "Patient",
            "name": [{"family": "Doe", "_given": [null, extension, {}]}]
        });
        assert_eq!(
            cleaned(resource),
            json!({
                "resourceType": "Patient",
                "name": [{"family": "Doe", "_given": [extension]}]
            })
        );
    }

    #[test]
    fn an_all_null_extension_array_is_omitted() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{"given": ["a", "b"], "_given": [null, null]}]
        });
        assert_eq!(
            cleaned(resource),
            json!({"resourceType": "Patient", "name": [{"given": ["a", "b"]}]})
        );
    }

    /// Empty content inside a slot empties the slot without moving the
    /// others: each value keeps the extension at its own index.
    #[test]
    fn empty_slot_content_is_cleaned_in_place() {
        let resource = json!({
            "resourceType": "Patient",
            "name": [{
                "given": ["a", [], "b"],
                "_given": [{"extension": [{}]}, {"id": "y", "extension": []}, null]
            }]
        });
        assert_eq!(
            cleaned(resource),
            json!({
                "resourceType": "Patient",
                "name": [{"given": ["a", null, "b"], "_given": [null, {"id": "y"}, null]}]
            })
        );
    }

    #[test]
    fn empty_singular_primitive_extensions_are_removed() {
        let resource = json!({
            "resourceType": "Patient",
            "birthDate": "2000-01-01",
            "_birthDate": {"extension": []},
            "_gender": null,
            "_active": {"id": "a1"}
        });
        assert_eq!(
            cleaned(resource),
            json!({
                "resourceType": "Patient",
                "birthDate": "2000-01-01",
                "_active": {"id": "a1"}
            })
        );
    }

    #[test]
    fn contained_resources_are_cleaned() {
        let resource = json!({
            "resourceType": "Observation",
            "contained": [{"resourceType": "Patient", "id": "p", "name": [{}]}, {}],
            "status": "final"
        });
        assert_eq!(
            cleaned(resource),
            json!({
                "resourceType": "Observation",
                "contained": [{"resourceType": "Patient", "id": "p"}],
                "status": "final"
            })
        );
    }

    #[test]
    fn a_resource_without_empty_elements_is_unchanged() {
        let resource = json!({
            "resourceType": "Patient",
            "id": "p1",
            "meta": {"profile": ["http://example.org/p"]},
            "name": [{"family": "Doe", "given": ["Ann", "Bo"]}],
            "extension": [{"url": "http://example.org/x", "valueBoolean": true}]
        });
        assert_eq!(cleaned(resource.clone()), resource);
    }
}
