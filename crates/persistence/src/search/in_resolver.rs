//! Backend-agnostic resolution of the `_in` search parameter (R5/R6).
//!
//! `_in` (`Patient?_in=Group/101`) restricts results to the active members of a
//! referenced `List` or `Group`. Like `_list` (see [`super::list_resolver`]) it
//! is resolved application-side: read the referenced container(s), collect the
//! member references that point at the searched resource type, and rewrite the
//! query into an `_id` filter that any backend can execute.
//!
//! `_in` stays an ordinary [`SearchParameter`] in `query.parameters` until it is
//! resolved; the spec expression for it is the placeholder `Resource.id`, so it
//! is never indexed (see `NON_INDEXABLE_PARAM_CODES` in the extractor).
//!
//! Semantics:
//! - a typed value (`List/1`, `Group/1`, an absolute URL, or a bare id with a
//!   `:List` / `:Group` modifier) is resolved against that type; a bare id
//!   without a modifier matches either a `List` or a `Group` with that id;
//! - comma-separated values within one parameter are OR-ed (union), repeated
//!   `_in` parameters are AND-ed (intersection);
//! - a missing or deleted container, or one without members, matches nothing;
//! - a `Group` whose membership is not enumerated (`definitional`,
//!   `conceptual`) cannot be answered by reading member references and is
//!   refused rather than answered as empty;
//! - any other referenced type is refused. `CareTeam` is a valid `_in` target
//!   in the spec but is not implemented.

use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;

use super::list_resolver::{id_for_type, intersect};
use crate::core::ResourceStorage;
use crate::error::{ResourceError, SearchError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
    strip_reference_version,
};

/// Sentinel `_id` value that cannot match any real resource, used to force an
/// empty result when an `_in` resolves to no members.
const NO_MATCH_SENTINEL: &str = "__in_search_no_match__";

const PARAM_NAME: &str = "_in";

/// Returns true if the query carries any direct (non-chained) `_in` filter.
pub fn query_has_in(query: &SearchQuery) -> bool {
    query.parameters.iter().any(is_in_param)
}

fn is_in_param(param: &SearchParameter) -> bool {
    param.name == PARAM_NAME && param.chain.is_empty()
}

fn refuse(message: String) -> StorageError {
    StorageError::Search(SearchError::QueryParseError { message })
}

/// Resolves a query's `_in` filters into an `_id` filter, returning a rewritten
/// query that a plain `search()` can execute. Queries without `_in` are
/// returned unchanged.
pub async fn resolve_in<S>(
    storage: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> StorageResult<SearchQuery>
where
    S: ResourceStorage + ?Sized,
{
    resolve_in_at(storage, tenant, query, Utc::now()).await
}

async fn resolve_in_at<S>(
    storage: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
    now: DateTime<Utc>,
) -> StorageResult<SearchQuery>
where
    S: ResourceStorage + ?Sized,
{
    if !query_has_in(query) {
        return Ok(query.clone());
    }

    let base_type = query.resource_type.as_str();
    let mut id_sets: Vec<HashSet<String>> = Vec::new();
    for param in query.parameters.iter().filter(|p| is_in_param(p)) {
        let type_hint = match &param.modifier {
            None => None,
            Some(SearchModifier::Type(t)) => Some(t.as_str()),
            Some(other) => {
                return Err(refuse(format!(
                    "search modifier ':{other}' is not supported for '_in'"
                )));
            }
        };

        // Values of one parameter are alternatives: union.
        let mut union = HashSet::new();
        for value in &param.values {
            for (container_type, container_id) in containers_for(&value.value, type_hint)? {
                union.extend(
                    member_ids(
                        storage,
                        tenant,
                        base_type,
                        container_type,
                        &container_id,
                        now,
                    )
                    .await?,
                );
            }
        }
        id_sets.push(union);
    }

    let matched_ids = intersect(id_sets);

    // Drop the resolved `_in` parameters and add an `_id` filter. Repeated `_id`
    // parameters AND together, so this composes with any user-supplied `_id`.
    let mut rewritten = query.clone();
    rewritten.parameters.retain(|p| !is_in_param(p));

    let id_values: Vec<SearchValue> = if matched_ids.is_empty() {
        vec![SearchValue::eq(NO_MATCH_SENTINEL)]
    } else {
        matched_ids.iter().map(SearchValue::eq).collect()
    };
    rewritten.parameters.push(SearchParameter {
        name: "_id".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: id_values,
        chain: vec![],
        components: vec![],
    });

    Ok(rewritten)
}

/// Turns one `_in` value into the `(container type, id)` pairs it denotes.
fn containers_for(
    value: &str,
    type_hint: Option<&str>,
) -> StorageResult<Vec<(&'static str, String)>> {
    let reference = strip_reference_version(value);
    let (ref_type, id) = match reference.rsplit_once('/') {
        Some((type_part, id)) => (Some(type_part.rsplit('/').next().unwrap_or(type_part)), id),
        None => (None, reference),
    };
    if id.is_empty() {
        return Err(refuse("'_in' requires a non-empty reference".to_string()));
    }

    let container_type = match (ref_type, type_hint) {
        (Some(t), Some(h)) if t != h => {
            return Err(refuse(format!(
                "'_in:{h}' cannot be combined with a reference to {t}"
            )));
        }
        (Some(t), _) | (None, Some(t)) => Some(t),
        (None, None) => None,
    };

    match container_type {
        None => Ok(vec![("List", id.to_string()), ("Group", id.to_string())]),
        Some("List") => Ok(vec![("List", id.to_string())]),
        Some("Group") => Ok(vec![("Group", id.to_string())]),
        Some("CareTeam") => Err(refuse(
            "'_in' membership of a CareTeam is not supported; use a List or Group".to_string(),
        )),
        Some(other) => Err(refuse(format!(
            "'_in' must reference a CareTeam, Group or List, not {other}"
        ))),
    }
}

/// Reads the container and collects the ids of its active members that point
/// at `base_type`. A missing or deleted container yields an empty set.
async fn member_ids<S>(
    storage: &S,
    tenant: &TenantContext,
    base_type: &str,
    container_type: &str,
    container_id: &str,
    now: DateTime<Utc>,
) -> StorageResult<HashSet<String>>
where
    S: ResourceStorage + ?Sized,
{
    let stored = match storage.read(tenant, container_type, container_id).await {
        Ok(Some(stored)) => stored,
        Ok(None) | Err(StorageError::Resource(ResourceError::Gone { .. })) => {
            return Ok(HashSet::new());
        }
        Err(e) => return Err(e),
    };
    let content = stored.content();

    let mut ids = HashSet::new();
    if container_type == "List" {
        for entry in array(content, "entry") {
            // `entry.deleted = true` marks an item removed from the list.
            if flag(entry, "deleted") {
                continue;
            }
            let reference = entry
                .get("item")
                .and_then(|item| item.get("reference"))
                .and_then(|r| r.as_str());
            if let Some(id) = reference.and_then(|r| id_for_type(r, base_type)) {
                ids.insert(id);
            }
        }
    } else {
        // `membership` is absent on R4 and optional on R6; absent means enumerated.
        if let Some(membership) = content.get("membership").and_then(|m| m.as_str()) {
            if membership != "enumerated" {
                return Err(refuse(format!(
                    "'_in' cannot be resolved for Group/{container_id}: its membership is \
                     '{membership}', not enumerated"
                )));
            }
        }
        for member in array(content, "member") {
            if flag(member, "inactive") || period_ended(member, now) {
                continue;
            }
            let reference = member
                .get("entity")
                .and_then(|e| e.get("reference"))
                .and_then(|r| r.as_str());
            if let Some(id) = reference.and_then(|r| id_for_type(r, base_type)) {
                ids.insert(id);
            }
        }
    }
    Ok(ids)
}

fn array<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

/// True when the member's `period.end` is before `now`. An absent or
/// unparseable end counts as still active.
fn period_ended(member: &Value, now: DateTime<Utc>) -> bool {
    let Some(end) = member
        .get("period")
        .and_then(|p| p.get("end"))
        .and_then(|e| e.as_str())
    else {
        return false;
    };
    let parsed = DateTime::parse_from_rfc3339(end)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            // Date-only (or `YYYY-MM` / `YYYY`) ends run to the end of that day.
            let date = NaiveDate::parse_from_str(end.get(..10).unwrap_or(end), "%Y-%m-%d").ok()?;
            Some(date.and_hms_opt(23, 59, 59)?.and_utc())
        });
    parsed.is_some_and(|end| end < now)
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use crate::core::{ResourceStorage, SearchProvider};
    use crate::tenant::{TenantId, TenantPermissions};
    use helios_fhir::FhirVersion;
    use serde_json::json;
    use std::path::PathBuf;

    fn backend() -> SqliteBackend {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap();
        let config = SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let b = SqliteBackend::with_config(":memory:", config).unwrap();
        b.init_schema().unwrap();
        b
    }

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t"), TenantPermissions::full_access())
    }

    async fn seed_patients(b: &SqliteBackend, t: &TenantContext) {
        for id in ["smith", "jones", "doe"] {
            b.create(
                t,
                "Patient",
                json!({ "resourceType": "Patient", "id": id }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        }
    }

    async fn create_list(b: &SqliteBackend, t: &TenantContext, id: &str, entries: Value) {
        b.create(
            t,
            "List",
            json!({
                "resourceType": "List", "id": id, "status": "current",
                "mode": "working", "entry": entries
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    }

    async fn create_group(b: &SqliteBackend, t: &TenantContext, id: &str, extra: Value) {
        let mut group = json!({
            "resourceType": "Group", "id": id, "type": "person", "actual": true
        });
        for (k, v) in extra.as_object().unwrap() {
            group[k] = v.clone();
        }
        b.create(t, "Group", group, FhirVersion::default())
            .await
            .unwrap();
    }

    fn in_query(modifier: Option<SearchModifier>, values: &[&str]) -> SearchQuery {
        let mut query = SearchQuery::new("Patient");
        query.parameters.push(in_param(modifier, values));
        query
    }

    fn in_param(modifier: Option<SearchModifier>, values: &[&str]) -> SearchParameter {
        SearchParameter {
            name: "_in".to_string(),
            param_type: SearchParamType::Reference,
            modifier,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            chain: vec![],
            components: vec![],
        }
    }

    async fn try_matched_ids(
        b: &SqliteBackend,
        t: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<Vec<String>> {
        let rewritten = resolve_in(b, t, query).await?;
        assert!(!query_has_in(&rewritten), "_in must be consumed");
        let result = b.search(t, &rewritten).await.unwrap();
        let mut ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        Ok(ids)
    }

    async fn matched_ids(b: &SqliteBackend, t: &TenantContext, query: &SearchQuery) -> Vec<String> {
        try_matched_ids(b, t, query).await.unwrap()
    }

    fn member(reference: &str) -> Value {
        json!({ "entity": { "reference": reference } })
    }

    #[tokio::test]
    async fn list_filters_to_members() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([
                { "item": { "reference": "Patient/smith" } },
                { "item": { "reference": "Patient/doe" } }
            ]),
        )
        .await;

        let ids = matched_ids(&b, &t, &in_query(None, &["List/l1"])).await;
        assert_eq!(ids, vec!["doe", "smith"]);
    }

    #[tokio::test]
    async fn group_filters_to_members() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_group(
            &b,
            &t,
            "g1",
            json!({ "member": [member("Patient/jones"), member("Patient/doe")] }),
        )
        .await;

        let ids = matched_ids(&b, &t, &in_query(None, &["Group/g1"])).await;
        assert_eq!(ids, vec!["doe", "jones"]);
    }

    #[tokio::test]
    async fn bare_id_unions_list_and_group() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "x",
            json!([{ "item": { "reference": "Patient/smith" } }]),
        )
        .await;
        create_group(&b, &t, "x", json!({ "member": [member("Patient/jones")] })).await;

        let ids = matched_ids(&b, &t, &in_query(None, &["x"])).await;
        assert_eq!(ids, vec!["jones", "smith"]);
    }

    #[tokio::test]
    async fn typed_reference_and_absolute_url_accepted() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_group(&b, &t, "g1", json!({ "member": [member("Patient/jones")] })).await;

        for value in [
            "Group/g1",
            "http://example.org/fhir/Group/g1",
            "Group/g1/_history/2",
        ] {
            let ids = matched_ids(&b, &t, &in_query(None, &[value])).await;
            assert_eq!(ids, vec!["jones"], "value {value}");
        }
    }

    #[tokio::test]
    async fn type_modifier_accepted() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "x",
            json!([{ "item": { "reference": "Patient/smith" } }]),
        )
        .await;
        create_group(&b, &t, "x", json!({ "member": [member("Patient/jones")] })).await;

        let group = in_query(Some(SearchModifier::Type("Group".into())), &["x"]);
        assert_eq!(matched_ids(&b, &t, &group).await, vec!["jones"]);
        let list = in_query(Some(SearchModifier::Type("List".into())), &["x"]);
        assert_eq!(matched_ids(&b, &t, &list).await, vec!["smith"]);

        let clash = in_query(Some(SearchModifier::Type("List".into())), &["Group/x"]);
        assert!(try_matched_ids(&b, &t, &clash).await.is_err());
    }

    #[tokio::test]
    async fn group_skips_inactive_ended_and_other_types() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_group(
            &b,
            &t,
            "g1",
            json!({ "member": [
                member("Patient/smith"),
                { "entity": { "reference": "Patient/jones" }, "inactive": true },
                { "entity": { "reference": "Patient/doe" }, "period": { "end": "2000-01-01" } },
                member("Organization/smith")
            ] }),
        )
        .await;

        let ids = matched_ids(&b, &t, &in_query(None, &["Group/g1"])).await;
        assert_eq!(ids, vec!["smith"]);
    }

    #[tokio::test]
    async fn group_keeps_member_whose_period_ends_in_the_future() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_group(
            &b,
            &t,
            "g1",
            json!({ "member": [
                { "entity": { "reference": "Patient/smith" }, "period": { "end": "2999-01-01T00:00:00Z" } }
            ] }),
        )
        .await;

        let ids = matched_ids(&b, &t, &in_query(None, &["Group/g1"])).await;
        assert_eq!(ids, vec!["smith"]);
    }

    #[tokio::test]
    async fn list_skips_deleted_entries_and_other_types() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([
                { "item": { "reference": "Patient/smith" } },
                { "item": { "reference": "Patient/jones" }, "deleted": true },
                { "item": { "reference": "Observation/doe" } }
            ]),
        )
        .await;

        let ids = matched_ids(&b, &t, &in_query(None, &["List/l1"])).await;
        assert_eq!(ids, vec!["smith"]);
    }

    #[tokio::test]
    async fn multiple_values_intersect() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([
                { "item": { "reference": "Patient/smith" } },
                { "item": { "reference": "Patient/jones" } }
            ]),
        )
        .await;
        create_group(
            &b,
            &t,
            "g1",
            json!({ "member": [member("Patient/jones"), member("Patient/doe")] }),
        )
        .await;

        let mut query = in_query(None, &["List/l1"]);
        query.parameters.push(in_param(None, &["Group/g1"]));
        assert_eq!(matched_ids(&b, &t, &query).await, vec!["jones"]);
    }

    #[tokio::test]
    async fn comma_values_union() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([{ "item": { "reference": "Patient/smith" } }]),
        )
        .await;
        create_group(&b, &t, "g1", json!({ "member": [member("Patient/doe")] })).await;

        let ids = matched_ids(&b, &t, &in_query(None, &["List/l1", "Group/g1"])).await;
        assert_eq!(ids, vec!["doe", "smith"]);
    }

    #[tokio::test]
    async fn missing_or_deleted_container_yields_empty() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_group(&b, &t, "g1", json!({ "member": [member("Patient/doe")] })).await;
        create_list(&b, &t, "empty", json!([])).await;
        b.delete(&t, "Group", "g1").await.unwrap();

        for value in ["List/nope", "Group/g1", "List/empty"] {
            let ids = matched_ids(&b, &t, &in_query(None, &[value])).await;
            assert!(ids.is_empty(), "value {value} must match nothing");
        }
    }

    #[tokio::test]
    async fn definitional_group_is_refused() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_group(
            &b,
            &t,
            "g1",
            json!({ "membership": "definitional", "member": [member("Patient/doe")] }),
        )
        .await;

        let err = try_matched_ids(&b, &t, &in_query(None, &["Group/g1"]))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::QueryParseError { .. })
        ));
    }

    #[tokio::test]
    async fn non_container_reference_is_refused() {
        let (b, t) = (backend(), tenant());
        for value in ["Patient/smith", "CareTeam/c1", "Group/"] {
            let err = try_matched_ids(&b, &t, &in_query(None, &[value])).await;
            assert!(err.is_err(), "value {value} must be refused");
        }
        let not = in_query(Some(SearchModifier::Not), &["List/l1"]);
        assert!(try_matched_ids(&b, &t, &not).await.is_err());
    }

    #[tokio::test]
    async fn composes_with_existing_id_filter() {
        let (b, t) = (backend(), tenant());
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([
                { "item": { "reference": "Patient/smith" } },
                { "item": { "reference": "Patient/jones" } }
            ]),
        )
        .await;

        let mut query = in_query(None, &["List/l1"]);
        query.parameters.push(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("jones"), SearchValue::eq("doe")],
            chain: vec![],
            components: vec![],
        });
        assert_eq!(matched_ids(&b, &t, &query).await, vec!["jones"]);
    }

    #[tokio::test]
    async fn query_without_in_is_unchanged() {
        let (b, t) = (backend(), tenant());
        let query = SearchQuery::new("Patient");
        let rewritten = resolve_in(&b, &t, &query).await.unwrap();
        assert!(rewritten.parameters.is_empty());
    }
}
