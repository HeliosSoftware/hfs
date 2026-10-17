//! The lock plan of a PostgreSQL transaction Bundle, derived before `BEGIN`
//! (#1637).
//!
//! A transaction Bundle used to take the tenant gate `EXCLUSIVE`, which made
//! every Bundle of a tenant wait for the one before it: twenty concurrent
//! importers ran one at a time and the queue outlived `statement_timeout`.
//! What the exclusive gate protects is narrower than "no other Bundle runs":
//!
//! * a resource's `search_index` and `resource_fts` rows must not be rewritten
//!   from a body another writer has already superseded, which needs the gate
//!   shared plus that resource's key from the body read to `COMMIT`;
//! * a SearchParameter write must not interleave with a writer that extracted
//!   its index values from the old registry, which needs the gate exclusive.
//!
//! So the plan is, by entry:
//!
//! | Entry | Locks |
//! |---|---|
//! | `POST`, no `resource.id` (every REST POST) | none: the id is minted inside the transaction, so nobody else can address the resource before `COMMIT` |
//! | `POST` with `resource.id` | key for `(type, id)` |
//! | `PUT`, `PATCH`, `DELETE`, `GET` `Type/id` | key for `(type, id)` |
//! | `PUT`, `PATCH`, `DELETE` `Type?criteria` | **exclusive**: the target is found by a search inside the transaction, so its key is not known before `BEGIN` |
//! | anything naming a `SearchParameter` | **exclusive** |
//! | an entry the planner cannot classify | **exclusive**, so its error is the one it always was |
//! | more than [`MAX_BUNDLE_LOCK_KEYS`] distinct keys | **exclusive**, to stay inside the lock table |
//! | more than [`MAX_BUNDLE_LOCK_KEYS`] `POST`s with `ifNoneExist` | **exclusive**, for the same reason: each one that finds no match can hold a criteria lock until `COMMIT` |
//!
//! Everything else takes the shared gate and the sorted keys, exactly as the
//! bulk-submit planned path does. `ifNoneExist` adds no key to the plan: the
//! executor takes a criteria lock when its search finds nothing (see
//! `lock_protocol`).
//!
//! A planned Bundle therefore holds at most 1 gate + [`MAX_BUNDLE_LOCK_KEYS`]
//! resource keys + [`MAX_BUNDLE_LOCK_KEYS`] criteria locks; see the constant for
//! what that means for the server's lock table.
//!
//! The planner parses entry URLs with the executor's own parser
//! ([`parse_resource_url`]) and reads a `POST`'s id the way
//! [`Transaction::create`](crate::core::Transaction::create) does, so a planned
//! key and an executed key cannot disagree. A disagreement would still fail
//! closed: the transaction refuses any `(type, id)` outside its plan.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::core::{BundleEntry, BundleMethod};

use super::storage::parse_resource_url;

/// Most distinct resource keys, and most conditional creates, a Bundle may carry
/// before it takes the exclusive gate instead. The two caps are separate.
///
/// Advisory locks live in the server's shared lock table, which holds roughly
/// `max_locks_per_transaction x (max_connections + max_prepared_transactions)`
/// entries (64 x 100 = 6,400 by default), and never use the per-backend fast
/// path. Twenty Bundles of 1,600 explicit ids would need five times that and end
/// in `out of shared memory`.
///
/// What one planned Bundle can hold is up to **257 advisory locks**: one tenant
/// gate, 128 resource keys and 128 criteria locks. Each `POST` with an
/// `ifNoneExist` that finds no match can take a criteria lock and keep it to
/// `COMMIT` (see `lock_protocol`), so the cap on conditional creates is the cap
/// on those. That is about twice what a reindex group holds (a gate and 128
/// resource keys, 129).
/// Twenty such Bundles at once need about 5,140 of a default server's 6,400
/// slots before any relation lock, reindex group or bulk-submit batch is
/// counted, so the table can still fill.
///
/// The caps keep that bounded; they do not rule it out. A Bundle that finds the
/// lock table full (SQLSTATE `53200`), whether taking its gate and keys at
/// `BEGIN` or a criteria lock mid-way, ends as a retryable 503 and not as a
/// failure of the request (`lock_protocol::is_transient_lock_sqlstate`). An
/// operator with heavy explicit-id or conditional imports raises
/// `max_locks_per_transaction`. Above a cap the Bundle takes the one exclusive
/// gate lock, which is what every Bundle did before the plan existed.
pub(super) const MAX_BUNDLE_LOCK_KEYS: usize = 128;

const SEARCH_PARAMETER: &str = "SearchParameter";

/// What a planned (shared-gate) transaction locks, and what it may then touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LockPlan {
    /// Every `(resource type, id)` the transaction may address. It takes a key
    /// for each, sorted, after the shared gate; it poisons on any other.
    pub(super) resources: Vec<(String, String)>,
    /// Whether a `create` of an id the transaction minted itself needs no key
    /// (a Bundle: see the module docs). Bulk submit keeps its rule of a
    /// complete key set and sets this false.
    pub(super) minted_creates: bool,
    /// Whether a conditional create that found no match takes a criteria lock
    /// before it searches again and creates (a Bundle). Bulk submit has no
    /// `ifNoneExist` and sets this false.
    pub(super) criteria_locks: bool,
}

impl LockPlan {
    /// The plan of a bulk-submit batch: exactly these keys, nothing minted.
    pub(super) fn resources_only(resources: Vec<(String, String)>) -> Self {
        Self {
            resources,
            minted_creates: false,
            criteria_locks: false,
        }
    }
}

/// Why a Bundle takes the exclusive gate rather than a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ExclusiveReason {
    /// An entry writes (or reads) a SearchParameter.
    SearchParameter,
    /// A `PUT`/`PATCH`/`DELETE` addressed by search criteria.
    ConditionalEntry,
    /// An entry the planner cannot read an identity from.
    Unclassifiable,
    /// More than [`MAX_BUNDLE_LOCK_KEYS`] distinct keys; the count is carried.
    TooManyKeys(usize),
    /// More than [`MAX_BUNDLE_LOCK_KEYS`] conditional creates; the count is
    /// carried.
    TooManyConditionalCreates(usize),
}

/// How a Bundle begins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BundleLockMode {
    /// Shared gate plus the plan's keys.
    Shared(LockPlan),
    /// The exclusive gate, for the stated reason.
    Exclusive(ExclusiveReason),
}

/// The type segment of an entry URL's path, ignoring the query string. Only
/// used to recognise a `SearchParameter`; instance identities go through
/// [`parse_resource_url`].
fn url_type(url: &str) -> Option<&str> {
    let path = url.split_once('?').map_or(url, |(path, _)| path);
    path.rsplit('/').find(|segment| !segment.is_empty())
}

/// Derives how `entries` lock, from the entries alone. No database access.
pub(super) fn plan_bundle_locks(entries: &[BundleEntry], max_keys: usize) -> BundleLockMode {
    use BundleLockMode::Exclusive;

    let mut keys: BTreeSet<(String, String)> = BTreeSet::new();
    let mut conditional_creates = 0usize;
    for entry in entries {
        if entry.criteria.is_some() {
            return Exclusive(ExclusiveReason::ConditionalEntry);
        }
        match entry.method {
            BundleMethod::Post => {
                let Some(resource) = entry.resource.as_ref() else {
                    return Exclusive(ExclusiveReason::Unclassifiable);
                };
                let Some(resource_type) = resource.get("resourceType").and_then(Value::as_str)
                else {
                    return Exclusive(ExclusiveReason::Unclassifiable);
                };
                if resource_type == SEARCH_PARAMETER
                    || url_type(&entry.url) == Some(SEARCH_PARAMETER)
                {
                    return Exclusive(ExclusiveReason::SearchParameter);
                }
                // The id `Transaction::create` uses when the payload has one;
                // without one it mints, and a minted id needs no key.
                if let Some(id) = resource.get("id").and_then(Value::as_str) {
                    keys.insert((resource_type.to_string(), id.to_string()));
                }
                if entry.if_none_exist.is_some() {
                    conditional_creates += 1;
                }
            }
            BundleMethod::Get | BundleMethod::Put | BundleMethod::Patch | BundleMethod::Delete => {
                let Ok((resource_type, id)) = parse_resource_url(&entry.url) else {
                    return Exclusive(ExclusiveReason::Unclassifiable);
                };
                let body_type = entry
                    .resource
                    .as_ref()
                    .and_then(|resource| resource.get("resourceType"))
                    .and_then(Value::as_str);
                if resource_type == SEARCH_PARAMETER
                    || (entry.method == BundleMethod::Put && body_type == Some(SEARCH_PARAMETER))
                {
                    return Exclusive(ExclusiveReason::SearchParameter);
                }
                keys.insert((resource_type, id));
            }
        }
    }
    if keys.len() > max_keys {
        return Exclusive(ExclusiveReason::TooManyKeys(keys.len()));
    }
    if conditional_creates > max_keys {
        return Exclusive(ExclusiveReason::TooManyConditionalCreates(
            conditional_creates,
        ));
    }
    BundleLockMode::Shared(LockPlan {
        resources: keys.into_iter().collect(),
        minted_creates: true,
        criteria_locks: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchParameter;
    use serde_json::json;

    fn entry(method: BundleMethod, url: &str, resource: Option<Value>) -> BundleEntry {
        BundleEntry {
            method,
            url: url.to_string(),
            resource,
            ..Default::default()
        }
    }

    fn post(resource: Value) -> BundleEntry {
        let url = resource["resourceType"].as_str().unwrap_or("").to_string();
        entry(BundleMethod::Post, &url, Some(resource))
    }

    fn conditional(method: BundleMethod, url: &str) -> BundleEntry {
        BundleEntry {
            criteria: Some(Vec::<SearchParameter>::new()),
            ..entry(method, url, Some(json!({"resourceType": "Patient"})))
        }
    }

    fn shared(mode: BundleLockMode) -> LockPlan {
        match mode {
            BundleLockMode::Shared(plan) => plan,
            other => panic!("expected a shared plan, got {other:?}"),
        }
    }

    fn keys(plan: &LockPlan) -> Vec<(&str, &str)> {
        plan.resources
            .iter()
            .map(|(t, i)| (t.as_str(), i.as_str()))
            .collect()
    }

    /// Every REST POST reaches storage without an id: the shared gate and no
    /// key, however many there are.
    #[test]
    fn posts_with_server_assigned_ids_need_no_key() {
        let entries: Vec<_> = (0..2000)
            .map(|_| post(json!({"resourceType": "Observation"})))
            .collect();
        let plan = shared(plan_bundle_locks(&entries, MAX_BUNDLE_LOCK_KEYS));
        assert!(plan.resources.is_empty());
        assert!(plan.minted_creates && plan.criteria_locks);
    }

    /// `ifNoneExist` adds nothing to the plan: the executor locks on a miss.
    #[test]
    fn a_conditional_create_adds_no_key() {
        let mut create = post(json!({"resourceType": "Organization"}));
        create.if_none_exist = Some("identifier=x|1".to_string());
        let plan = shared(plan_bundle_locks(&[create], MAX_BUNDLE_LOCK_KEYS));
        assert!(plan.resources.is_empty());
    }

    #[test]
    fn a_post_carrying_an_id_is_keyed_on_it() {
        let entries = [
            post(json!({"resourceType": "Patient", "id": "given"})),
            // Not a string: `create` would mint, so there is nothing to key.
            post(json!({"resourceType": "Patient", "id": 7})),
        ];
        let plan = shared(plan_bundle_locks(&entries, MAX_BUNDLE_LOCK_KEYS));
        assert_eq!(keys(&plan), vec![("Patient", "given")]);
    }

    #[test]
    fn instance_entries_are_keyed_on_their_type_and_id() {
        let entries = [
            entry(
                BundleMethod::Put,
                "Patient/a",
                Some(json!({"resourceType": "Patient"})),
            ),
            entry(
                BundleMethod::Patch,
                "Observation/b",
                Some(json!({"resourceType": "Parameters"})),
            ),
            entry(BundleMethod::Delete, "Encounter/c", None),
            entry(BundleMethod::Get, "Condition/d", None),
        ];
        let plan = shared(plan_bundle_locks(&entries, MAX_BUNDLE_LOCK_KEYS));
        assert_eq!(
            keys(&plan),
            vec![
                ("Condition", "d"),
                ("Encounter", "c"),
                ("Observation", "b"),
                ("Patient", "a"),
            ]
        );
    }

    /// The executor's own parser: a server prefix is stripped, and the key is
    /// the last two segments.
    #[test]
    fn absolute_and_prefixed_urls_are_read_as_the_executor_reads_them() {
        let entries = [
            entry(
                BundleMethod::Put,
                "https://example.org/fhir/Patient/abs",
                Some(json!({"resourceType": "Patient"})),
            ),
            entry(BundleMethod::Delete, "/fhir/Observation/rel", None),
        ];
        let plan = shared(plan_bundle_locks(&entries, MAX_BUNDLE_LOCK_KEYS));
        assert_eq!(
            keys(&plan),
            vec![("Observation", "rel"), ("Patient", "abs")]
        );
    }

    #[test]
    fn repeated_identities_collapse_to_one_key() {
        let put = || {
            entry(
                BundleMethod::Put,
                "Patient/same",
                Some(json!({"resourceType": "Patient"})),
            )
        };
        let entries = [
            put(),
            entry(BundleMethod::Delete, "Patient/same", None),
            post(json!({"resourceType": "Patient", "id": "same"})),
            put(),
        ];
        let plan = shared(plan_bundle_locks(&entries, 1));
        assert_eq!(keys(&plan), vec![("Patient", "same")]);
    }

    #[test]
    fn an_empty_bundle_plans_the_shared_gate_alone() {
        let plan = shared(plan_bundle_locks(&[], MAX_BUNDLE_LOCK_KEYS));
        assert!(plan.resources.is_empty());
    }

    /// A SearchParameter in any position and spelling stays exclusive: by the
    /// URL's type, by the body's `resourceType`, by method.
    #[test]
    fn any_search_parameter_entry_is_exclusive() {
        let search_parameter = json!({"resourceType": "SearchParameter"});
        let cases = [
            post(search_parameter.clone()),
            entry(
                BundleMethod::Post,
                "SearchParameter",
                Some(json!({"resourceType": "Patient"})),
            ),
            entry(
                BundleMethod::Put,
                "SearchParameter/x",
                Some(search_parameter.clone()),
            ),
            entry(
                BundleMethod::Put,
                "Patient/x",
                Some(search_parameter.clone()),
            ),
            entry(BundleMethod::Patch, "SearchParameter/x", Some(json!({}))),
            entry(BundleMethod::Delete, "SearchParameter/x", None),
            entry(
                BundleMethod::Delete,
                "https://h/fhir/SearchParameter/x",
                None,
            ),
            entry(BundleMethod::Get, "SearchParameter/x", None),
        ];
        for case in cases {
            assert_eq!(
                plan_bundle_locks(
                    &[post(json!({"resourceType": "Patient"})), case.clone()],
                    MAX_BUNDLE_LOCK_KEYS
                ),
                BundleLockMode::Exclusive(ExclusiveReason::SearchParameter),
                "{} {}",
                case.method,
                case.url
            );
        }
    }

    /// A URL-borne conditional entry is resolved by a search inside the
    /// transaction, so its key is unknown before BEGIN.
    #[test]
    fn a_url_conditional_entry_is_exclusive() {
        for method in [BundleMethod::Put, BundleMethod::Patch, BundleMethod::Delete] {
            assert_eq!(
                plan_bundle_locks(
                    &[conditional(method, "Patient?identifier=x|1")],
                    MAX_BUNDLE_LOCK_KEYS
                ),
                BundleLockMode::Exclusive(ExclusiveReason::ConditionalEntry),
                "{method}"
            );
        }
    }

    /// What the planner cannot read an identity from keeps the exclusive path,
    /// and with it the error the entry always produced.
    #[test]
    fn what_cannot_be_classified_is_exclusive() {
        let cases = [
            // No resource, no resourceType.
            entry(BundleMethod::Post, "Patient", None),
            post(json!({"name": []})),
            // A type with no id.
            entry(
                BundleMethod::Put,
                "Patient",
                Some(json!({"resourceType": "Patient"})),
            ),
            entry(BundleMethod::Delete, "", None),
            entry(BundleMethod::Get, "Patient?name=x", None),
        ];
        for case in cases {
            assert_eq!(
                plan_bundle_locks(std::slice::from_ref(&case), MAX_BUNDLE_LOCK_KEYS),
                BundleLockMode::Exclusive(ExclusiveReason::Unclassifiable),
                "{} {}",
                case.method,
                case.url
            );
        }
    }

    #[test]
    fn the_key_cap_is_inclusive() {
        let puts = |n: usize| -> Vec<BundleEntry> {
            (0..n)
                .map(|i| {
                    entry(
                        BundleMethod::Put,
                        &format!("Patient/{i}"),
                        Some(json!({"resourceType": "Patient"})),
                    )
                })
                .collect()
        };
        let at_cap = shared(plan_bundle_locks(
            &puts(MAX_BUNDLE_LOCK_KEYS),
            MAX_BUNDLE_LOCK_KEYS,
        ));
        assert_eq!(at_cap.resources.len(), MAX_BUNDLE_LOCK_KEYS);
        assert_eq!(
            plan_bundle_locks(&puts(MAX_BUNDLE_LOCK_KEYS + 1), MAX_BUNDLE_LOCK_KEYS),
            BundleLockMode::Exclusive(ExclusiveReason::TooManyKeys(MAX_BUNDLE_LOCK_KEYS + 1))
        );
        // Entries beyond the cap that repeat an identity are not more keys.
        let mut repeated = puts(MAX_BUNDLE_LOCK_KEYS);
        repeated.extend(puts(MAX_BUNDLE_LOCK_KEYS));
        assert_eq!(
            shared(plan_bundle_locks(&repeated, MAX_BUNDLE_LOCK_KEYS))
                .resources
                .len(),
            MAX_BUNDLE_LOCK_KEYS
        );
    }

    /// Each conditional create that finds no match can hold a criteria lock to
    /// `COMMIT`, so their count is bounded like the keys are.
    #[test]
    fn the_conditional_create_cap_is_inclusive() {
        let creates = |n: usize| -> Vec<BundleEntry> {
            (0..n)
                .map(|i| {
                    let mut create = post(json!({"resourceType": "Organization"}));
                    create.if_none_exist = Some(format!("identifier=x|{i}"));
                    create
                })
                .collect()
        };
        let at_cap = shared(plan_bundle_locks(
            &creates(MAX_BUNDLE_LOCK_KEYS),
            MAX_BUNDLE_LOCK_KEYS,
        ));
        assert!(at_cap.resources.is_empty());
        assert_eq!(
            plan_bundle_locks(&creates(MAX_BUNDLE_LOCK_KEYS + 1), MAX_BUNDLE_LOCK_KEYS),
            BundleLockMode::Exclusive(ExclusiveReason::TooManyConditionalCreates(
                MAX_BUNDLE_LOCK_KEYS + 1
            ))
        );
        // Plain POSTs are free however many there are.
        let mut mixed = creates(MAX_BUNDLE_LOCK_KEYS);
        mixed.extend((0..500).map(|_| post(json!({"resourceType": "Observation"}))));
        assert!(matches!(
            plan_bundle_locks(&mixed, MAX_BUNDLE_LOCK_KEYS),
            BundleLockMode::Shared(_)
        ));
    }

    /// Bulk submit keeps its complete-key-set rule.
    #[test]
    fn a_bulk_plan_mints_nothing_and_takes_no_criteria_lock() {
        let plan = LockPlan::resources_only(vec![("Patient".into(), "a".into())]);
        assert!(!plan.minted_creates && !plan.criteria_locks);
        assert_eq!(keys(&plan), vec![("Patient", "a")]);
    }
}
