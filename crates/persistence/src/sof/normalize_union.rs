//! `unionAll` normalization for the in-DB compiler.
//!
//! A ViewDefinition's `select` tree may carry `unionAll` at any depth. The
//! plan IR only represents a union at the root, so this pass rewrites the
//! tree into a list of *alternatives*, each a `select` array with no
//! `unionAll` anywhere; the planner then emits one `Project` per alternative
//! and a root `Union` over them.
//!
//! The rewrite distributes the union over its enclosing clause:
//!
//! ```text
//! forEach X { cols, unionAll [A, B] }
//!   =>  forEach X { cols, select [A] }  ∪  forEach X { cols, select [B] }
//! ```
//!
//! A branch becomes one more nested `select` of its alternative, which is how
//! the in-process engine combines a branch with the row built so far (a cross
//! product). The result is exact under multiset semantics; row order is not
//! preserved, and every consumer of the in-DB runners compares unordered.
//! `%rowIndex` is unaffected because it is fixed by the iteration directive,
//! which each alternative keeps.
//!
//! Views without any `unionAll` are returned unchanged as one alternative.

use serde_json::{Map, Value};

use crate::core::sof_runner::SofError;

/// Upper bound on the number of alternatives one view may expand to.
pub const MAX_ALTERNATIVES: usize = 64;

/// Rewrites `selects` into `unionAll`-free alternatives.
///
/// Returns one alternative when the tree has no `unionAll`. Fails with
/// [`SofError::Uncompilable`] when the expansion would exceed
/// [`MAX_ALTERNATIVES`], and with [`SofError::InvalidViewDefinition`] on an
/// empty `unionAll` list.
pub fn distribute_union_all(selects: &[Value]) -> Result<Vec<Vec<Value>>, SofError> {
    product_of_clauses(selects)
}

fn too_many() -> SofError {
    SofError::Uncompilable {
        reason: format!(
            "unionAll expands to more than {MAX_ALTERNATIVES} alternatives after normalization"
        ),
    }
}

fn has_union(clause: &Value) -> bool {
    let Some(obj) = clause.as_object() else {
        return false;
    };
    obj.contains_key("unionAll")
        || obj
            .get("select")
            .and_then(Value::as_array)
            .is_some_and(|sub| sub.iter().any(has_union))
}

/// Cross product of the alternatives of each sibling clause.
fn product_of_clauses(clauses: &[Value]) -> Result<Vec<Vec<Value>>, SofError> {
    let mut acc: Vec<Vec<Value>> = vec![Vec::new()];
    for clause in clauses {
        let alts = clause_alternatives(clause)?;
        if acc.len().saturating_mul(alts.len()) > MAX_ALTERNATIVES {
            return Err(too_many());
        }
        let mut next = Vec::with_capacity(acc.len() * alts.len());
        for prefix in &acc {
            for alt in &alts {
                let mut combo = prefix.clone();
                combo.push(alt.clone());
                next.push(combo);
            }
        }
        acc = next;
    }
    Ok(acc)
}

/// `unionAll`-free alternatives of a single clause.
fn clause_alternatives(clause: &Value) -> Result<Vec<Value>, SofError> {
    if !has_union(clause) {
        return Ok(vec![clause.clone()]);
    }
    let Some(obj) = clause.as_object() else {
        return Ok(vec![clause.clone()]);
    };

    let mut base: Map<String, Value> = obj.clone();
    base.remove("select");
    base.remove("unionAll");

    // Nested selects: every combination of their alternatives.
    let nested: &[Value] = obj
        .get("select")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let nested_combos = product_of_clauses(nested)?;

    // Branches: flattened alternatives of every branch, in declaration order.
    let branch_alts: Option<Vec<Value>> = match obj.get("unionAll").and_then(Value::as_array) {
        None => None,
        Some(branches) => {
            if branches.is_empty() {
                return Err(SofError::InvalidViewDefinition(
                    "unionAll branches list is empty".to_string(),
                ));
            }
            let mut all = Vec::new();
            for branch in branches {
                all.extend(clause_alternatives(branch)?);
                if all.len() > MAX_ALTERNATIVES {
                    return Err(too_many());
                }
            }
            Some(all)
        }
    };

    let mut out = Vec::new();
    for combo in &nested_combos {
        let extras: Vec<Option<&Value>> = match &branch_alts {
            None => vec![None],
            Some(alts) => alts.iter().map(Some).collect(),
        };
        for extra in extras {
            let mut sel = combo.clone();
            if let Some(branch) = extra {
                sel.push(branch.clone());
            }
            let mut alt = base.clone();
            if !sel.is_empty() {
                alt.insert("select".to_string(), Value::Array(sel));
            }
            out.push(Value::Object(alt));
            if out.len() > MAX_ALTERNATIVES {
                return Err(too_many());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn has_no_union(v: &Value) -> bool {
        match v {
            Value::Object(o) => !o.contains_key("unionAll") && o.values().all(has_no_union),
            Value::Array(a) => a.iter().all(has_no_union),
            _ => true,
        }
    }

    #[test]
    fn view_without_union_is_returned_unchanged() {
        let sel = vec![
            json!({"column": [{"name": "id", "path": "id"}]}),
            json!({"forEach": "name", "column": [{"name": "f", "path": "family"}]}),
        ];
        let alts = distribute_union_all(&sel).unwrap();
        assert_eq!(alts, vec![sel]);
    }

    #[test]
    fn top_level_union_clause_splits_into_branches() {
        let sel = vec![json!({"unionAll": [
            {"column": [{"name": "a", "path": "id"}]},
            {"column": [{"name": "a", "path": "id"}]}
        ]})];
        let alts = distribute_union_all(&sel).unwrap();
        assert_eq!(alts.len(), 2);
        assert!(alts.iter().all(|a| has_no_union(&json!(a))));
        // Each alternative carries its branch as a nested select.
        assert_eq!(
            alts[0][0],
            json!({"select": [{"column": [{"name": "a", "path": "id"}]}]})
        );
    }

    #[test]
    fn union_inside_for_each_keeps_the_iteration_in_every_alternative() {
        let sel = vec![json!({
            "forEach": "contact",
            "column": [{"name": "i", "path": "%rowIndex"}],
            "select": [{"unionAll": [
                {"forEach": "telecom", "column": [{"name": "v", "path": "value"}]},
                {"column": [{"name": "v", "path": "name.family"}]}
            ]}]
        })];
        let alts = distribute_union_all(&sel).unwrap();
        assert_eq!(alts.len(), 2);
        for alt in &alts {
            assert_eq!(alt[0]["forEach"], json!("contact"));
            assert_eq!(alt[0]["column"][0]["name"], json!("i"));
            assert!(has_no_union(&json!(alt)));
        }
        let inner0 = &alts[0][0]["select"];
        assert_eq!(inner0[0]["select"][0]["forEach"], json!("telecom"));
    }

    #[test]
    fn union_inside_repeat_is_distributed_not_dropped() {
        let sel = vec![json!({
            "repeat": ["item"],
            "unionAll": [
                {"column": [{"name": "k", "path": "linkId"}]},
                {"column": [{"name": "k", "path": "text"}]}
            ]
        })];
        let alts = distribute_union_all(&sel).unwrap();
        assert_eq!(alts.len(), 2);
        for alt in &alts {
            assert_eq!(alt[0]["repeat"], json!(["item"]));
        }
    }

    #[test]
    fn nested_unions_flatten_and_siblings_multiply() {
        let sel = vec![
            json!({"unionAll": [
                {"unionAll": [{"column": [{"name": "a", "path": "x"}]},
                              {"column": [{"name": "a", "path": "y"}]}]},
                {"column": [{"name": "a", "path": "z"}]}
            ]}),
            json!({"unionAll": [
                {"column": [{"name": "b", "path": "p"}]},
                {"column": [{"name": "b", "path": "q"}]}
            ]}),
        ];
        let alts = distribute_union_all(&sel).unwrap();
        assert_eq!(alts.len(), 6);
        assert!(alts.iter().all(|a| a.len() == 2 && has_no_union(&json!(a))));
    }

    #[test]
    fn expansion_beyond_the_cap_is_uncompilable() {
        let branch = json!({"unionAll": [
            {"column": [{"name": "a", "path": "x"}]},
            {"column": [{"name": "a", "path": "y"}]}
        ]});
        // 2^7 = 128 alternatives.
        let sel: Vec<Value> = (0..7).map(|_| branch.clone()).collect();
        assert!(matches!(
            distribute_union_all(&sel),
            Err(SofError::Uncompilable { .. })
        ));
        // 2^6 = 64 is still allowed.
        assert_eq!(distribute_union_all(&sel[..6]).unwrap().len(), 64);
    }

    #[test]
    fn empty_union_all_is_invalid() {
        let sel = vec![json!({"unionAll": []})];
        assert!(matches!(
            distribute_union_all(&sel),
            Err(SofError::InvalidViewDefinition(_))
        ));
    }
}
