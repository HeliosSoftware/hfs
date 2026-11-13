//! Resource metadata labels on update, and the `$meta-add` / `$meta-delete`
//! changes (#2562).
//!
//! FHIR asks servers to merge existing and new tags and security labels on
//! an update, and to replace profiles with the submitted ones (R4 2.26.3.9,
//! R5 2.1.27.5.3.10). [`merge_update_labels`] applies that rule to a PUT;
//! [`put_preserving_labels`] is the unconditional PUT that reads the current
//! version, merges and compares-and-swaps. [`MetaChange`] is the change a
//! metadata operation makes to one stored version.
//!
//! Tags and security labels are sets keyed by `system` + `code`; profiles
//! are keyed by their URL and move together with their `_profile` slot.

use helios_fhir::FhirVersion;
use serde_json::{Map, Value};

use super::preconditions::EntityTagPrecondition;
use super::storage::{
    ConditionalDeleteResult, ConditionalStorage, ConditionalUpdateResult, ResourceStorage,
};
use crate::error::{ConcurrencyError, ResourceError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

/// Attempts a derived write makes: the first one plus three retries, the
/// bound the backends' own upsert loops use.
pub const WRITE_ATTEMPTS: usize = 4;

const LABEL_LISTS: [&str; 2] = ["tag", "security"];

/// Which stored version a metadata operation changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaTarget {
    /// The current version, as last read: the write is refused when the
    /// version or its same-version revision has moved since.
    Current {
        /// The version id the caller read.
        expected_version: String,
        /// The [`StoredResource::meta_seq`] the caller read.
        expected_meta_seq: u64,
    },
    /// An explicit version, from `_history/[vid]`.
    Historical {
        /// The version id named by the request.
        version_id: String,
    },
}

/// The change a `$meta-add` or `$meta-delete` makes; each carries a
/// validated `Meta`.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaChange {
    /// Adds the profiles, tags and security labels of the `Meta`.
    Add(Value),
    /// Removes the profiles, tags and security labels of the `Meta`.
    Delete(Value),
}

/// The outcome of a metadata rewrite.
#[derive(Debug, Clone)]
pub struct MetaRewrite {
    /// The rewritten version: same version id, new `lastUpdated`.
    pub resource: StoredResource,
    /// Whether the current version changed (and with it the search index).
    pub current_changed: bool,
}

/// FHIR's update rule for metadata labels: the union of the stored and
/// submitted `meta.tag` and `meta.security`, and `meta.profile` exactly as
/// submitted.
///
/// A submitted label that matches a stored one replaces it in its stored
/// position; new labels follow in submitted order; duplicates collapse to
/// the first. Empty labels and lists are removed from the result.
pub fn merge_update_labels(current: &Value, submitted: &Value) -> Value {
    let mut merged = submitted.clone();
    let stored_meta = current.get("meta").and_then(Value::as_object);
    let submitted_meta = submitted.get("meta").and_then(Value::as_object);
    let Some(object) = merged.as_object_mut() else {
        return merged;
    };

    let mut meta = submitted_meta.cloned().unwrap_or_default();
    for list in LABEL_LISTS {
        let stored = codings(stored_meta, list);
        let incoming = codings(submitted_meta, list);
        let labels = union_codings(stored, incoming);
        if labels.is_empty() {
            meta.remove(list);
        } else {
            meta.insert(list.to_string(), Value::Array(labels));
        }
    }
    clean_meta(&mut meta);
    if meta.is_empty() {
        object.remove("meta");
    } else {
        object.insert("meta".to_string(), Value::Object(meta));
    }
    merged
}

impl MetaChange {
    /// Applies this change to a resource's `meta`.
    ///
    /// Matching follows the OperationDefinitions: tags and security labels
    /// by `system` + `code` (display and version are ignored), profiles by
    /// their full URL. Adding never duplicates and leaves a match unchanged;
    /// deleting a label that is not there is not an error.
    pub fn apply(&self, content: &mut Value) {
        let Some(object) = content.as_object_mut() else {
            return;
        };
        let mut meta = object
            .get("meta")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        match self {
            MetaChange::Add(change) => {
                let change = change.as_object();
                for list in LABEL_LISTS {
                    let mut labels = codings(Some(&meta), list);
                    for coding in codings(change, list) {
                        if !labels.iter().any(|l| same_label(l, &coding)) {
                            labels.push(coding);
                        }
                    }
                    set_list(&mut meta, list, labels);
                }
                let mut slots = profile_slots(Some(&meta));
                for slot in profile_slots(change) {
                    let known = slot
                        .url()
                        .is_some_and(|url| slots.iter().any(|s| s.url() == Some(url)));
                    if !known {
                        slots.push(slot);
                    }
                }
                set_profile_slots(&mut meta, slots);
            }
            MetaChange::Delete(change) => {
                let change = change.as_object();
                for list in LABEL_LISTS {
                    let removed = codings(change, list);
                    let mut labels = codings(Some(&meta), list);
                    labels.retain(|l| !removed.iter().any(|r| same_label(l, r)));
                    set_list(&mut meta, list, labels);
                }
                let removed: Vec<String> = profile_slots(change)
                    .iter()
                    .filter_map(|s| s.url().map(str::to_string))
                    .collect();
                let mut slots = profile_slots(Some(&meta));
                slots.retain(|s| !s.url().is_some_and(|url| removed.iter().any(|r| r == url)));
                set_profile_slots(&mut meta, slots);
            }
        }

        clean_meta(&mut meta);
        if meta.is_empty() {
            object.remove("meta");
        } else {
            object.insert("meta".to_string(), Value::Object(meta));
        }
    }
}

/// The unconditional FHIR update (`PUT [type]/[id]` with no precondition):
/// creates the resource, restores a deleted one, or updates the current
/// version with its tags and security labels merged in.
///
/// Every attempt starts from `submitted`, so a retry merges with the version
/// it read rather than with an earlier one. A lost race of any kind starts
/// the next attempt, up to [`WRITE_ATTEMPTS`]; the last error is returned.
/// An uncertain commit outcome is returned at once.
pub async fn put_preserving_labels<S>(
    storage: &S,
    tenant: &TenantContext,
    resource_type: &str,
    id: &str,
    submitted: Value,
    fhir_version: FhirVersion,
) -> StorageResult<(StoredResource, bool)>
where
    S: ResourceStorage + ?Sized,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        let result = match storage.read(tenant, resource_type, id).await {
            Ok(Some(current)) => {
                let merged = merge_update_labels(current.content(), &submitted);
                storage
                    .update(tenant, &current, merged)
                    .await
                    .map(|stored| (stored, false))
            }
            Ok(None) => {
                let mut body = submitted.clone();
                if let Some(object) = body.as_object_mut() {
                    object.insert("id".to_string(), Value::String(id.to_string()));
                }
                storage
                    .create(tenant, resource_type, body, fhir_version)
                    .await
                    .map(|stored| (stored, true))
            }
            Err(StorageError::Resource(ResourceError::Gone { .. })) => storage
                .restore_tombstone(tenant, resource_type, id, submitted.clone())
                .await
                .map(|stored| (stored, true)),
            Err(e) => Err(e),
        };
        match result {
            Err(
                StorageError::Concurrency(ConcurrencyError::VersionConflict { .. })
                | StorageError::Resource(
                    ResourceError::NotFound { .. } | ResourceError::AlreadyExists { .. },
                ),
            ) if attempt < WRITE_ATTEMPTS => continue,
            other => return other,
        }
    }
}

/// The conditional update (`PUT [type]?criteria`) with upsert as `upsert`
/// asks, for a store that resolves criteria itself.
///
/// The single match's labels are merged into `resource`, and the update
/// compares-and-swaps on the match's version and revision — the ones
/// `If-Match` and the criteria were evaluated against. A metadata rewrite
/// that lands in between keeps the version but may change whether the
/// criteria match, so the criteria are resolved again, up to
/// [`WRITE_ATTEMPTS`] times.
#[allow(clippy::too_many_arguments)]
pub async fn conditional_update_resolving<S>(
    storage: &S,
    tenant: &TenantContext,
    resource_type: &str,
    resource: Value,
    search_params: &str,
    upsert: bool,
    fhir_version: FhirVersion,
    if_match: &EntityTagPrecondition,
) -> StorageResult<ConditionalUpdateResult>
where
    S: ConditionalStorage + ?Sized,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut matches = storage
            .resolve_conditional_matches(tenant, resource_type, search_params)
            .await?;
        return match matches.len() {
            0 => {
                // `If-Match` names a version; nothing matched, so nothing can
                // carry it and the create must not run (#1381).
                super::conditional_if_match_gate(if_match, resource_type, None)?;
                if upsert {
                    let created = storage
                        .create(tenant, resource_type, resource, fhir_version)
                        .await?;
                    Ok(ConditionalUpdateResult::Created(created))
                } else {
                    Ok(ConditionalUpdateResult::NoMatch)
                }
            }
            1 => {
                let matched = matches.remove(0);
                super::conditional_if_match_gate(if_match, resource_type, Some(&matched))?;
                let merged = merge_update_labels(matched.content(), &resource);
                match storage.update(tenant, &matched, merged).await {
                    Err(e) if is_same_version_conflict(&e) && attempt < WRITE_ATTEMPTS => continue,
                    result => result.map(ConditionalUpdateResult::Updated),
                }
            }
            n => Ok(ConditionalUpdateResult::MultipleMatches(n)),
        };
    }
}

/// The conditional delete (`DELETE [type]?criteria`) for a store that
/// resolves criteria itself: the single match is deleted only while it is
/// still the revision the criteria selected (see
/// [`delete_conditional_match`](super::delete_conditional_match)); a match
/// changed in between is resolved again, up to [`WRITE_ATTEMPTS`] times.
pub async fn conditional_delete_resolving<S>(
    storage: &S,
    tenant: &TenantContext,
    resource_type: &str,
    search_params: &str,
    if_match: &EntityTagPrecondition,
) -> StorageResult<ConditionalDeleteResult>
where
    S: ConditionalStorage + ?Sized,
{
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut matches = storage
            .resolve_conditional_matches(tenant, resource_type, search_params)
            .await?;
        return match matches.len() {
            0 => {
                // A supplied `If-Match` fails against no match, as it does on
                // `DELETE [type]/[id]` for a missing resource.
                super::conditional_if_match_gate(if_match, resource_type, None)?;
                Ok(ConditionalDeleteResult::NoMatch)
            }
            1 => {
                let matched = matches.remove(0);
                super::conditional_if_match_gate(if_match, resource_type, Some(&matched))?;
                match super::delete_conditional_match(storage, tenant, if_match, &matched).await {
                    Err(e) if is_same_version_conflict(&e) && attempt < WRITE_ATTEMPTS => continue,
                    result => result.map(|()| ConditionalDeleteResult::Deleted(matched)),
                }
            }
            n => Ok(ConditionalDeleteResult::MultipleMatches(n)),
        };
    }
}

/// Whether `error` is a compare-and-swap that lost to a write which kept the
/// public version: a metadata rewrite. A caller that read that version may
/// read again and redo its work, its `If-Match` still satisfied.
pub fn is_same_version_conflict(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Concurrency(ConcurrencyError::VersionConflict {
            expected_version,
            actual_version,
            ..
        }) if expected_version == actual_version
    )
}

/// The `VersionConflict` a same-version compare-and-swap miss reports.
pub fn same_version_conflict(resource_type: &str, id: &str, version: &str) -> StorageError {
    StorageError::Concurrency(ConcurrencyError::VersionConflict {
        resource_type: resource_type.to_string(),
        id: id.to_string(),
        expected_version: version.to_string(),
        actual_version: version.to_string(),
    })
}

/// The labels of one list, cleaned before anything is matched: empty members
/// are dropped from each Coding, and a Coding left empty is dropped, so a
/// legacy `{}` can never stand in for a label that has an identity of its own.
fn codings(meta: Option<&Map<String, Value>>, list: &str) -> Vec<Value> {
    let mut labels = meta
        .and_then(|m| m.get(list))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for label in labels.iter_mut() {
        if let Value::Object(coding) = label {
            coding.retain(|_, v| !is_empty_value(v));
        }
    }
    labels.retain(|l| !is_empty_value(l));
    labels
}

fn union_codings(stored: Vec<Value>, incoming: Vec<Value>) -> Vec<Value> {
    let mut labels: Vec<Value> = Vec::with_capacity(stored.len() + incoming.len());
    for coding in stored {
        if !labels.iter().any(|l| same_label(l, &coding)) {
            labels.push(coding);
        }
    }
    let mut asserted = vec![false; labels.len()];
    for coding in incoming {
        match labels.iter().position(|l| same_label(l, &coding)) {
            Some(index) if !asserted[index] => {
                labels[index] = coding;
                asserted[index] = true;
            }
            Some(_) => {}
            None => {
                labels.push(coding);
                asserted.push(true);
            }
        }
    }
    labels
}

/// The identity of a tag or security label: its `system` and `code`, an
/// absent member compared as absent. A non-object item has no identity.
fn label_identity(coding: &Value) -> Option<(Option<&str>, Option<&str>)> {
    let object = coding.as_object()?;
    Some((
        object.get("system").and_then(Value::as_str),
        object.get("code").and_then(Value::as_str),
    ))
}

fn same_label(a: &Value, b: &Value) -> bool {
    match (label_identity(a), label_identity(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

fn set_list(meta: &mut Map<String, Value>, list: &str, labels: Vec<Value>) {
    if labels.is_empty() {
        meta.remove(list);
    } else {
        meta.insert(list.to_string(), Value::Array(labels));
    }
}

/// One position of the repeating `profile` element: its URL and its
/// `_profile` companion (id and extensions), either of which may be absent.
#[derive(Debug, Clone, PartialEq)]
struct ProfileSlot {
    value: Value,
    companion: Value,
}

impl ProfileSlot {
    fn url(&self) -> Option<&str> {
        self.value.as_str()
    }

    fn is_empty(&self) -> bool {
        self.value.is_null() && companion_is_empty(&self.companion)
    }
}

fn companion_is_empty(companion: &Value) -> bool {
    match companion {
        Value::Null => true,
        Value::Object(object) => object.values().all(|v| match v {
            Value::Null => true,
            Value::Array(items) => items.is_empty(),
            Value::String(s) => s.is_empty(),
            _ => false,
        }),
        _ => false,
    }
}

/// The `profile` / `_profile` pair as slots, the shorter array padded with
/// the implied trailing `null`s.
fn profile_slots(meta: Option<&Map<String, Value>>) -> Vec<ProfileSlot> {
    let values = meta
        .and_then(|m| m.get("profile"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let companions = meta
        .and_then(|m| m.get("_profile"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let len = values.len().max(companions.len());
    (0..len)
        .map(|i| ProfileSlot {
            value: values.get(i).cloned().unwrap_or(Value::Null),
            companion: companions.get(i).cloned().unwrap_or(Value::Null),
        })
        .collect()
}

fn set_profile_slots(meta: &mut Map<String, Value>, slots: Vec<ProfileSlot>) {
    let slots: Vec<ProfileSlot> = slots.into_iter().filter(|s| !s.is_empty()).collect();
    meta.remove("profile");
    meta.remove("_profile");
    if slots.is_empty() {
        return;
    }
    let mut values: Vec<Value> = slots.iter().map(|s| s.value.clone()).collect();
    let mut companions: Vec<Value> = slots
        .iter()
        .map(|s| {
            if companion_is_empty(&s.companion) {
                Value::Null
            } else {
                s.companion.clone()
            }
        })
        .collect();
    while companions.last().is_some_and(Value::is_null) {
        companions.pop();
    }
    if values.iter().any(|v| !v.is_null()) {
        while values.last().is_some_and(Value::is_null) && values.len() > companions.len() {
            values.pop();
        }
        meta.insert("profile".to_string(), Value::Array(values));
    }
    if !companions.is_empty() {
        meta.insert("_profile".to_string(), Value::Array(companions));
    }
}

/// Removes empty labels, lists and members from a `meta`, so a merge never
/// carries forward an empty element a legacy stored row may hold.
fn clean_meta(meta: &mut Map<String, Value>) {
    for list in LABEL_LISTS {
        if let Some(Value::Array(labels)) = meta.get_mut(list) {
            for label in labels.iter_mut() {
                if let Value::Object(coding) = label {
                    coding.retain(|_, v| !is_empty_value(v));
                }
            }
            labels.retain(|l| !is_empty_value(l));
        }
    }
    if meta.contains_key("profile") || meta.contains_key("_profile") {
        let slots = profile_slots(Some(meta));
        set_profile_slots(meta, slots);
    }
    meta.retain(|_, v| !is_empty_value(v));
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.iter().all(is_empty_value),
        Value::Object(object) => object.values().all(is_empty_value),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tag(system: &str, code: &str) -> Value {
        json!({"system": system, "code": code})
    }

    fn task(meta: Value) -> Value {
        json!({"resourceType": "Task", "id": "t1", "status": "requested", "meta": meta})
    }

    #[test]
    fn put_merges_stored_and_submitted_tags() {
        let stored = task(json!({"tag": [tag("s", "a")]}));
        let submitted = task(json!({"tag": [tag("s", "b")]}));
        let merged = merge_update_labels(&stored, &submitted);
        assert_eq!(merged["meta"]["tag"], json!([tag("s", "a"), tag("s", "b")]));
    }

    #[test]
    fn put_without_meta_keeps_labels_and_drops_profiles() {
        let stored = task(json!({
            "tag": [tag("s", "a")],
            "security": [tag("sec", "R")],
            "profile": ["http://example.org/p"]
        }));
        let submitted = json!({"resourceType": "Task", "id": "t1", "status": "requested"});
        let merged = merge_update_labels(&stored, &submitted);
        assert_eq!(
            merged["meta"],
            json!({"tag": [tag("s", "a")], "security": [tag("sec", "R")]})
        );
    }

    #[test]
    fn put_replaces_profiles_with_the_submitted_pair() {
        let stored = task(json!({"profile": ["http://example.org/old"]}));
        let submitted = task(json!({
            "profile": ["http://example.org/new"],
            "_profile": [{"extension": [{"url": "http://example.org/x", "valueString": "y"}]}]
        }));
        let merged = merge_update_labels(&stored, &submitted);
        assert_eq!(merged["meta"]["profile"], json!(["http://example.org/new"]));
        assert_eq!(merged["meta"]["_profile"], submitted["meta"]["_profile"]);
    }

    #[test]
    fn a_resent_label_replaces_the_stored_one_in_place() {
        let stored = task(json!({"tag": [
            {"system": "s", "code": "a", "display": "old"},
            tag("s", "z")
        ]}));
        let submitted = task(json!({"tag": [
            tag("s", "n"),
            {"system": "s", "code": "a", "display": "new", "userSelected": true}
        ]}));
        let merged = merge_update_labels(&stored, &submitted);
        assert_eq!(
            merged["meta"]["tag"],
            json!([
                {"system": "s", "code": "a", "display": "new", "userSelected": true},
                tag("s", "z"),
                tag("s", "n")
            ])
        );
    }

    #[test]
    fn duplicates_collapse_to_the_first() {
        let stored = task(json!({"tag": [tag("s", "a"), tag("s", "a")]}));
        let submitted = task(json!({"tag": [
            {"system": "s", "code": "b", "display": "first"},
            {"system": "s", "code": "b", "display": "second"}
        ]}));
        let merged = merge_update_labels(&stored, &submitted);
        assert_eq!(
            merged["meta"]["tag"],
            json!([tag("s", "a"), {"system": "s", "code": "b", "display": "first"}])
        );
    }

    #[test]
    fn security_labels_merge_like_tags() {
        let stored = task(json!({"security": [tag("sec", "R")]}));
        let submitted = task(json!({"security": [tag("sec", "N")]}));
        let merged = merge_update_labels(&stored, &submitted);
        assert_eq!(
            merged["meta"]["security"],
            json!([tag("sec", "R"), tag("sec", "N")])
        );
    }

    #[test]
    fn nothing_stored_takes_the_body_as_sent() {
        let stored = json!({"resourceType": "Task", "id": "t1"});
        let submitted = task(json!({"tag": [tag("s", "a")], "source": "x"}));
        assert_eq!(merge_update_labels(&stored, &submitted), submitted);
    }

    #[test]
    fn codings_without_system_or_code_have_their_own_identity() {
        let stored = task(json!({"tag": [
            {"code": "a"}, {"system": "s"}, {"display": "only"}
        ]}));
        let submitted = task(json!({"tag": [
            {"code": "a", "display": "x"}, {"system": "s", "code": "a"}, {"display": "other"}
        ]}));
        let merged = merge_update_labels(&stored, &submitted);
        assert_eq!(
            merged["meta"]["tag"],
            json!([
                {"code": "a", "display": "x"},
                {"system": "s"},
                {"display": "other"},
                {"system": "s", "code": "a"}
            ])
        );
    }

    #[test]
    fn legacy_empty_labels_are_not_carried_forward() {
        let stored = task(json!({"tag": [{}, {"system": null}, tag("s", "a")], "security": []}));
        let submitted = task(json!({}));
        let merged = merge_update_labels(&stored, &submitted);
        assert_eq!(merged["meta"], json!({"tag": [tag("s", "a")]}));
        let merged = merge_update_labels(&task(json!({"tag": [{}]})), &task(json!({})));
        assert!(merged.get("meta").is_none());
    }

    #[test]
    fn a_legacy_empty_coding_never_stands_in_for_a_label() {
        for list in LABEL_LISTS {
            let stored = task(json!({(list): [{}, {"display": "keep"}]}));
            let merged = merge_update_labels(&stored, &task(json!({})));
            assert_eq!(merged["meta"][list], json!([{"display": "keep"}]), "{list}");

            let mut resource = task(json!({(list): [{}]}));
            MetaChange::Add(json!({(list): [{"display": "added"}]})).apply(&mut resource);
            assert_eq!(
                resource["meta"][list],
                json!([{"display": "added"}]),
                "{list}"
            );

            let mut resource = task(json!({(list): [{"code": null}, {"display": "keep"}]}));
            MetaChange::Delete(json!({(list): [{"system": "s", "code": "x"}]}))
                .apply(&mut resource);
            assert_eq!(
                resource["meta"][list],
                json!([{"display": "keep"}]),
                "{list}"
            );
        }
    }

    #[test]
    fn add_dedupes_ignoring_display_and_version() {
        let mut resource = task(json!({"tag": [{"system": "s", "code": "a", "display": "kept"}]}));
        MetaChange::Add(json!({"tag": [
            {"system": "s", "code": "a", "display": "ignored", "version": "2"},
            tag("s", "b")
        ]}))
        .apply(&mut resource);
        assert_eq!(
            resource["meta"]["tag"],
            json!([{"system": "s", "code": "a", "display": "kept"}, tag("s", "b")])
        );
    }

    #[test]
    fn add_to_a_resource_without_meta() {
        let mut resource = json!({"resourceType": "Task", "id": "t1"});
        MetaChange::Add(json!({"security": [tag("sec", "R")], "profile": ["http://p"]}))
            .apply(&mut resource);
        assert_eq!(
            resource["meta"],
            json!({"security": [tag("sec", "R")], "profile": ["http://p"]})
        );
    }

    #[test]
    fn delete_of_an_absent_label_is_not_an_error() {
        let mut resource = task(json!({"tag": [tag("s", "a")]}));
        MetaChange::Delete(json!({"tag": [tag("s", "missing")], "profile": ["http://none"]}))
            .apply(&mut resource);
        assert_eq!(resource["meta"]["tag"], json!([tag("s", "a")]));
    }

    #[test]
    fn deleting_the_last_label_removes_the_list_then_meta() {
        let mut resource = task(json!({"tag": [tag("s", "a")]}));
        MetaChange::Delete(json!({"tag": [{"system": "s", "code": "a", "display": "x"}]}))
            .apply(&mut resource);
        assert!(resource.get("meta").is_none());
    }

    #[test]
    fn profile_companions_stay_with_their_url() {
        let ext = |v: &str| json!({"extension": [{"url": "http://e", "valueString": v}]});
        let mut resource = task(json!({
            "profile": ["http://p", "http://q"],
            "_profile": [ext("p"), ext("q")]
        }));
        MetaChange::Delete(json!({"profile": ["http://p"]})).apply(&mut resource);
        assert_eq!(resource["meta"]["profile"], json!(["http://q"]));
        assert_eq!(resource["meta"]["_profile"], json!([ext("q")]));

        MetaChange::Add(
            json!({"profile": ["http://q", "http://r"], "_profile": [ext("new-q"), ext("r")]}),
        )
        .apply(&mut resource);
        assert_eq!(resource["meta"]["profile"], json!(["http://q", "http://r"]));
        assert_eq!(resource["meta"]["_profile"], json!([ext("q"), ext("r")]));
    }

    #[test]
    fn profile_pairs_pad_and_keep_extension_only_slots() {
        let ext = json!({"extension": [{"url": "http://e", "valueString": "x"}]});
        let mut resource = task(json!({
            "profile": ["http://p", null, "http://q"],
            "_profile": [null, ext.clone()]
        }));
        MetaChange::Delete(json!({"profile": ["http://p"]})).apply(&mut resource);
        assert_eq!(resource["meta"]["profile"], json!([null, "http://q"]));
        assert_eq!(resource["meta"]["_profile"], json!([ext.clone()]));

        MetaChange::Delete(json!({"profile": ["http://q"]})).apply(&mut resource);
        assert_eq!(resource["meta"]["_profile"], json!([ext]));
        assert!(resource["meta"].get("profile").is_none());
    }

    mod put {
        use super::*;
        use crate::error::TransactionError;
        use crate::tenant::{TenantContext, TenantId, TenantPermissions};
        use async_trait::async_trait;
        use std::collections::VecDeque;
        use std::sync::Mutex;

        /// A storage whose reads and write outcomes are scripted, recording
        /// every write it is asked to make.
        #[derive(Default)]
        struct Scripted {
            reads: Mutex<VecDeque<StorageResult<Option<StoredResource>>>>,
            failures: Mutex<VecDeque<StorageError>>,
            writes: Mutex<Vec<(&'static str, Value)>>,
            restore_unsupported: bool,
        }

        impl Scripted {
            fn read_then(self, read: StorageResult<Option<StoredResource>>) -> Self {
                self.reads.lock().unwrap().push_back(read);
                self
            }

            fn fail_next_write(self, error: StorageError) -> Self {
                self.failures.lock().unwrap().push_back(error);
                self
            }

            fn write(&self, kind: &'static str, content: Value) -> StorageResult<()> {
                self.writes.lock().unwrap().push((kind, content));
                match self.failures.lock().unwrap().pop_front() {
                    Some(error) => Err(error),
                    None => Ok(()),
                }
            }

            fn writes(&self) -> Vec<(&'static str, Value)> {
                self.writes.lock().unwrap().clone()
            }
        }

        fn stored(version: &str, tags: &[&str]) -> StoredResource {
            let tags: Vec<Value> = tags.iter().map(|c| tag("s", c)).collect();
            StoredResource::from_storage(
                "Task",
                "t1",
                version,
                TenantId::new("t"),
                json!({"resourceType": "Task", "id": "t1", "meta": {"tag": tags}}),
                chrono::Utc::now(),
                chrono::Utc::now(),
                None,
                FhirVersion::default(),
            )
        }

        fn conflict() -> StorageError {
            StorageError::Concurrency(ConcurrencyError::VersionConflict {
                resource_type: "Task".into(),
                id: "t1".into(),
                expected_version: "1".into(),
                actual_version: "2".into(),
            })
        }

        fn codes(content: &Value) -> Vec<String> {
            content["meta"]["tag"]
                .as_array()
                .map(|tags| {
                    tags.iter()
                        .map(|t| t["code"].as_str().unwrap().to_string())
                        .collect()
                })
                .unwrap_or_default()
        }

        #[async_trait]
        impl ResourceStorage for Scripted {
            fn backend_name(&self) -> &'static str {
                "scripted"
            }

            async fn create(
                &self,
                _tenant: &TenantContext,
                resource_type: &str,
                resource: Value,
                fhir_version: FhirVersion,
            ) -> StorageResult<StoredResource> {
                self.write("create", resource.clone())?;
                Ok(StoredResource::new(
                    resource_type,
                    "t1",
                    TenantId::new("t"),
                    resource,
                    fhir_version,
                ))
            }

            async fn create_or_update(
                &self,
                _tenant: &TenantContext,
                _resource_type: &str,
                _id: &str,
                _resource: Value,
                _fhir_version: FhirVersion,
            ) -> StorageResult<(StoredResource, bool)> {
                panic!("put_preserving_labels must not call create_or_update");
            }

            async fn read(
                &self,
                _tenant: &TenantContext,
                _resource_type: &str,
                _id: &str,
            ) -> StorageResult<Option<StoredResource>> {
                self.reads
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("an unscripted read")
            }

            async fn update(
                &self,
                _tenant: &TenantContext,
                current: &StoredResource,
                resource: Value,
            ) -> StorageResult<StoredResource> {
                self.write("update", resource.clone())?;
                Ok(current
                    .clone()
                    .new_version(resource, crate::types::ResourceMethod::Put))
            }

            async fn delete(
                &self,
                _tenant: &TenantContext,
                _resource_type: &str,
                _id: &str,
            ) -> StorageResult<()> {
                unreachable!()
            }

            async fn count(
                &self,
                _tenant: &TenantContext,
                _resource_type: Option<&str>,
            ) -> StorageResult<u64> {
                unreachable!()
            }

            async fn restore_tombstone(
                &self,
                _tenant: &TenantContext,
                resource_type: &str,
                _id: &str,
                resource: Value,
            ) -> StorageResult<StoredResource> {
                if self.restore_unsupported {
                    return Err(StorageError::Backend(
                        crate::error::BackendError::UnsupportedCapability {
                            backend_name: "scripted".into(),
                            capability: "restore_tombstone".into(),
                        },
                    ));
                }
                self.write("restore", resource.clone())?;
                Ok(StoredResource::new(
                    resource_type,
                    "t1",
                    TenantId::new("t"),
                    resource,
                    FhirVersion::default(),
                ))
            }
        }

        fn tenant() -> TenantContext {
            TenantContext::new(TenantId::new("t"), TenantPermissions::full_access())
        }

        fn body(tags: &[&str]) -> Value {
            stored("0", tags).content().clone()
        }

        async fn put(storage: &Scripted, tags: &[&str]) -> StorageResult<(StoredResource, bool)> {
            put_preserving_labels(
                storage,
                &tenant(),
                "Task",
                "t1",
                body(tags),
                FhirVersion::default(),
            )
            .await
        }

        fn gone() -> StorageError {
            StorageError::Resource(ResourceError::Gone {
                resource_type: "Task".into(),
                id: "t1".into(),
                deleted_at: None,
            })
        }

        fn not_found() -> StorageError {
            StorageError::Resource(ResourceError::NotFound {
                resource_type: "Task".into(),
                id: "t1".into(),
            })
        }

        #[tokio::test]
        async fn a_conflict_merges_again_from_the_original_body() {
            let storage = Scripted::default()
                .read_then(Ok(Some(stored("1", &["a"]))))
                .fail_next_write(conflict())
                .read_then(Ok(Some(stored("2", &["b"]))));
            let (written, created) = put(&storage, &["c"]).await.unwrap();
            assert!(!created);
            // The winner removed `a`: the retry merges with the winner only,
            // not with the body the first attempt had already merged.
            assert_eq!(codes(written.content()), ["b", "c"]);
            let writes = storage.writes();
            assert_eq!(codes(&writes[0].1), ["a", "c"]);
            assert_eq!(codes(&writes[1].1), ["b", "c"]);
        }

        #[tokio::test]
        async fn a_lost_create_merges_with_the_winner() {
            let storage = Scripted::default()
                .read_then(Ok(None))
                .fail_next_write(StorageError::Resource(ResourceError::AlreadyExists {
                    resource_type: "Task".into(),
                    id: "t1".into(),
                }))
                .read_then(Ok(Some(stored("1", &["w"]))));
            let (written, created) = put(&storage, &["c"]).await.unwrap();
            assert!(!created);
            assert_eq!(codes(written.content()), ["w", "c"]);
            assert_eq!(storage.writes()[0].0, "create");
            assert_eq!(storage.writes()[0].1["id"], "t1");
        }

        #[tokio::test]
        async fn a_lost_restore_merges_with_the_winner() {
            for loss in [not_found(), conflict()] {
                let storage = Scripted::default()
                    .read_then(Err(gone()))
                    .fail_next_write(loss)
                    .read_then(Ok(Some(stored("3", &["w"]))));
                let (written, _) = put(&storage, &["c"]).await.unwrap();
                assert_eq!(codes(written.content()), ["w", "c"]);
                let kinds: Vec<_> = storage.writes().iter().map(|w| w.0).collect();
                assert_eq!(kinds, ["restore", "update"]);
            }
        }

        #[tokio::test]
        async fn a_won_restore_takes_only_the_submitted_labels() {
            let storage = Scripted::default().read_then(Err(gone()));
            let (written, created) = put(&storage, &["c"]).await.unwrap();
            assert!(created);
            assert_eq!(codes(written.content()), ["c"]);
        }

        #[tokio::test]
        async fn an_unknown_commit_outcome_is_returned_at_once() {
            let unknown = || {
                StorageError::Transaction(TransactionError::CommitOutcomeUnknown {
                    reason: "lost".into(),
                })
            };
            let storage = Scripted::default()
                .read_then(Ok(Some(stored("1", &["a"]))))
                .fail_next_write(unknown());
            assert!(matches!(
                put(&storage, &["c"]).await,
                Err(StorageError::Transaction(
                    TransactionError::CommitOutcomeUnknown { .. }
                ))
            ));
            assert_eq!(storage.writes().len(), 1);

            let storage = Scripted::default()
                .read_then(Err(gone()))
                .fail_next_write(unknown());
            assert!(put(&storage, &["c"]).await.is_err());
            assert_eq!(storage.writes().len(), 1);
        }

        #[tokio::test]
        async fn four_lost_races_return_the_last_error() {
            let mut storage = Scripted::default();
            for _ in 0..WRITE_ATTEMPTS {
                storage = storage
                    .read_then(Ok(Some(stored("1", &["a"]))))
                    .fail_next_write(conflict());
            }
            assert!(matches!(
                put(&storage, &["c"]).await,
                Err(StorageError::Concurrency(
                    ConcurrencyError::VersionConflict { .. }
                ))
            ));
            assert_eq!(storage.writes().len(), WRITE_ATTEMPTS);
        }

        #[tokio::test]
        async fn an_unsupported_restore_is_returned_without_a_replacement_write() {
            let storage = Scripted {
                restore_unsupported: true,
                ..Default::default()
            }
            .read_then(Err(gone()));
            assert!(matches!(
                put(&storage, &["c"]).await,
                Err(StorageError::Backend(
                    crate::error::BackendError::UnsupportedCapability { .. }
                ))
            ));
            assert!(storage.writes().is_empty());
        }
    }

    #[test]
    fn is_same_version_conflict_tells_a_metadata_rewrite_from_a_new_version() {
        assert!(is_same_version_conflict(&same_version_conflict(
            "Task", "t1", "3"
        )));
        let advanced = StorageError::Concurrency(ConcurrencyError::VersionConflict {
            resource_type: "Task".into(),
            id: "t1".into(),
            expected_version: "3".into(),
            actual_version: "4".into(),
        });
        assert!(!is_same_version_conflict(&advanced));
    }
}
