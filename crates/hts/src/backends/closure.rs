//! Building a `$closure` ConceptMap group from each input code's ancestors,
//! shared by every backend.
//!
//! Backends differ only in how they load a code's ancestors; once each
//! distinct input code's ancestors are in hand, pairing them is pure and lives
//! here, so a request costs one ancestor walk per distinct code on every
//! backend rather than one per ordered pair.

use std::collections::HashSet;

use serde_json::{Value, json};

/// The distinct codes of `codes`, in first-seen order.
pub(crate) fn distinct_codes(codes: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    codes
        .iter()
        .map(String::as_str)
        .filter(|code| seen.insert(*code))
        .collect()
}

/// The `element` entries of a `$closure` group over `codes`, which must be
/// distinct: one per code that is an ancestor of another input code, listing
/// those codes as `subsumes` targets, both in input order.
///
/// `ancestors[i]` holds the ancestors of `codes[i]`; codes outside `codes`
/// are ignored, and a code listed among its own ancestors (a hierarchy cycle)
/// is never paired with itself.
pub(crate) fn closure_elements(codes: &[&str], ancestors: &[HashSet<String>]) -> Vec<Value> {
    codes
        .iter()
        .filter_map(|ancestor| {
            let targets: Vec<Value> = codes
                .iter()
                .zip(ancestors)
                .filter(|(descendant, above)| *descendant != ancestor && above.contains(*ancestor))
                .map(|(descendant, _)| json!({"code": descendant, "equivalence": "subsumes"}))
                .collect();
            (!targets.is_empty()).then(|| json!({"code": ancestor, "target": targets}))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(codes: &[&str]) -> HashSet<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn distinct_codes_keep_their_first_position() {
        let codes = ["b", "a", "b", "c", "a"].map(String::from);
        assert_eq!(distinct_codes(&codes), ["b", "a", "c"]);
    }

    #[test]
    fn each_ancestor_lists_its_input_descendants_in_input_order() {
        let codes = ["leaf", "root", "mid"];
        let ancestors = [set(&["mid", "root"]), set(&[]), set(&["root", "outside"])];
        assert_eq!(
            closure_elements(&codes, &ancestors),
            [
                json!({"code": "root", "target": [
                    {"code": "leaf", "equivalence": "subsumes"},
                    {"code": "mid", "equivalence": "subsumes"}
                ]}),
                json!({"code": "mid", "target": [
                    {"code": "leaf", "equivalence": "subsumes"}
                ]}),
            ]
        );
    }

    #[test]
    fn a_code_in_a_cycle_is_not_its_own_target() {
        let codes = ["a", "b"];
        let ancestors = [set(&["a", "b"]), set(&["a", "b"])];
        assert_eq!(
            closure_elements(&codes, &ancestors),
            [
                json!({"code": "a", "target": [{"code": "b", "equivalence": "subsumes"}]}),
                json!({"code": "b", "target": [{"code": "a", "equivalence": "subsumes"}]}),
            ]
        );
    }
}
