//! Array slicing: the mark/sweep pass.
//!
//! For every slicing-bearing schema in the element's set:
//! - **mark**: match each array item against each slice's pattern
//!   (`_.isMatch`-style partial deep equality), counting matches per slice;
//! - items matched by at least one slice **with a schema** are validated
//!   once, cooperatively, against the element set extended with *all* their
//!   matched slice schemas (the reference validator validates once per
//!   matched slice against `elset ∪ that slice`, and then re-validates every
//!   item in the plain pass — duplicating base-set errors; we consume
//!   matched items instead, which is indistinguishable on the conformance
//!   suite and saner on real resources);
//! - **sweep**: `rules: closed` (with `@default` absorption) and
//!   `rules: openAtEnd` violations, `ordered` violations, then per-slice
//!   `min`/`max` cardinality in slice declaration order.
//!
//! Divergences from the reference validator (which implements only pattern
//! matching + cardinality): `closed`/`openAtEnd`/`ordered`/`@default` are
//! enforced (pinned by `tests/fixtures/extended/slicing_rules.json`), and a
//! `max: 0` prohibited slice is enforced (the reference skips falsy bounds).
//!
//! Match types other than `pattern` (`type`, `profile`, `binding`,
//! `resolve-ref`) are deliberately not evaluated yet — they arrive in a later
//! phase alongside the converter's discriminator translation. A slice whose
//! matcher we cannot evaluate matches nothing, and a slice with no `match`
//! at all (a constraining slice) also matches nothing.

use super::errors::{self, ErrorKind};
use super::walk::{add_schemas_to_set, is_partial_match, validate_node, SchemaSet, WalkCtx};
use crate::schema::{Slice, Slicing};
use indexmap::IndexMap;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// The reserved slice name that absorbs unmatched items in closed slicing.
const DEFAULT_SLICE: &str = "@default";

struct LayerMark {
    slicing: Slicing,
    /// Matched-item count per slice, in declaration order.
    counters: IndexMap<String, u64>,
    /// For each array item, the slice names it matched (declaration order).
    item_matches: Vec<Vec<String>>,
}

/// Run the slicing pass over an array element. Returns the indices of items
/// that were already fully validated (consumed) here.
pub(super) fn validate_slices(
    ctx: &mut WalkCtx<'_>,
    elset: &SchemaSet,
    items: &[Value],
) -> HashSet<usize> {
    let slicings: Vec<Slicing> = elset
        .schemas()
        .filter_map(|s| s.slicing.clone())
        .collect();
    if slicings.is_empty() {
        return HashSet::new();
    }

    // ---- mark ----
    let mut marks: Vec<LayerMark> = Vec::with_capacity(slicings.len());
    for slicing in slicings {
        let mut counters: IndexMap<String, u64> =
            slicing.slices.keys().map(|k| (k.clone(), 0)).collect();
        let mut item_matches: Vec<Vec<String>> = vec![Vec::new(); items.len()];
        for (index, item) in items.iter().enumerate() {
            for (name, slice) in &slicing.slices {
                if name == DEFAULT_SLICE {
                    continue;
                }
                if slice_matches(slice, item) {
                    *counters.get_mut(name).expect("counter exists") += 1;
                    item_matches[index].push(name.clone());
                }
            }
        }
        // `@default` (closed slicing only) absorbs items no other slice
        // matched; count them so its own min/max can apply.
        if slicing.slices.contains_key(DEFAULT_SLICE)
            && slicing.rules.as_deref() == Some("closed")
        {
            let absorbed = item_matches.iter().filter(|m| m.is_empty()).count() as u64;
            *counters.get_mut(DEFAULT_SLICE).expect("counter exists") = absorbed;
        }
        marks.push(LayerMark { slicing, counters, item_matches });
    }

    // ---- validate matched items (item order), consuming them ----
    let mut consumed: HashSet<usize> = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let mut slice_schemas: Vec<(String, Arc<crate::schema::FhirSchema>)> = Vec::new();
        for mark in &marks {
            for name in &mark.item_matches[index] {
                if let Some(schema) = &mark.slicing.slices[name].schema {
                    slice_schemas.push((name.clone(), Arc::clone(schema)));
                }
            }
        }
        if slice_schemas.is_empty() {
            continue;
        }
        let mut extended = elset.clone();
        for (name, schema) in &slice_schemas {
            add_schemas_to_set(ctx, &mut extended, Arc::clone(schema), name);
        }
        ctx.path.push_index(index);
        validate_node(ctx, &extended, item);
        ctx.path.pop();
        consumed.insert(index);
    }

    // ---- sweep: rules, order, cardinality (per layer, elset order) ----
    for mark in &marks {
        let rules = mark.slicing.rules.as_deref().unwrap_or("open");

        if rules == "closed" {
            let default_slice = mark.slicing.slices.get(DEFAULT_SLICE);
            for (index, item) in items.iter().enumerate() {
                if !mark.item_matches[index].is_empty() {
                    continue;
                }
                match default_slice {
                    Some(Slice { schema: Some(schema), .. }) => {
                        let mut extended = elset.clone();
                        add_schemas_to_set(ctx, &mut extended, Arc::clone(schema), DEFAULT_SLICE);
                        ctx.path.push_index(index);
                        validate_node(ctx, &extended, item);
                        ctx.path.pop();
                        consumed.insert(index);
                    }
                    Some(_) => {
                        // @default without a schema: absorbed, nothing extra
                        // to check; the plain pass still validates the item.
                    }
                    None => {
                        ctx.path.push_index(index);
                        ctx.error(
                            ErrorKind::SliceUnmatched,
                            errors::msg_slice_closed_unmatched(),
                        );
                        ctx.path.pop();
                    }
                }
            }
        } else if rules == "openAtEnd"
            && let Some(last_matched) =
                mark.item_matches.iter().rposition(|m| !m.is_empty())
            {
                for index in 0..last_matched {
                    if mark.item_matches[index].is_empty() {
                        ctx.path.push_index(index);
                        ctx.error(ErrorKind::SliceUnmatched, errors::msg_slice_open_at_end());
                        ctx.path.pop();
                    }
                }
            }

        if mark.slicing.ordered == Some(true) {
            let mut max_order_seen: Option<u64> = None;
            for (index, matches) in mark.item_matches.iter().enumerate() {
                let Some(first) = matches.first() else {
                    continue;
                };
                let Some(order) = mark.slicing.slices[first].order else {
                    continue;
                };
                if let Some(seen) = max_order_seen
                    && order < seen
                {
                    ctx.path.push_index(index);
                    ctx.error(ErrorKind::SliceOrder, errors::msg_slice_order(first));
                    ctx.path.pop();
                }
                max_order_seen = Some(max_order_seen.unwrap_or(0).max(order));
            }
        }

        for (name, slice) in &mark.slicing.slices {
            let found = mark.counters[name];
            if let Some(min) = slice.min
                && min > 0
                && found < min
            {
                ctx.error(ErrorKind::SliceCardinality, errors::msg_slice_min(min, found));
            }
            // Unlike the reference validator (which skips falsy bounds),
            // `max: 0` — a prohibited slice, a real FHIR pattern — is
            // enforced.
            if let Some(max) = slice.max
                && found > max
            {
                ctx.error(ErrorKind::SliceCardinality, errors::msg_slice_max(max, found));
            }
        }
    }

    consumed
}

/// Does an item belong to a slice? Only `pattern` matching is evaluated
/// today; a missing `match` (constraining slice) matches nothing, and a
/// `match` with no `value` matches everything (lodash `_.isMatch` semantics
/// for an empty source).
fn slice_matches(slice: &Slice, item: &Value) -> bool {
    let Some(match_) = &slice.match_ else {
        return false;
    };
    match match_.value.as_ref() {
        Some(pattern) => is_partial_match(item, pattern),
        None => true,
    }
}
