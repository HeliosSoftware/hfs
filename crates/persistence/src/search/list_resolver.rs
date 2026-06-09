//! Backend-agnostic resolution of `_list` search.
//!
//! FHIR `_list` search (`Patient?_list=42`) restricts results to the resources
//! referenced by a `List` resource's `entry.item`. No per-backend
//! `SearchProvider::search` understands this directly, so — like chained search
//! (see [`super::chain_resolver`]) — we resolve it application-side: read the
//! referenced `List`(s), collect the member references that point at the searched
//! resource type, and rewrite the query into an `_id` filter that any backend can
//! execute.
//!
//! Multiple `_list` values are AND-ed: a result must be a member of *every*
//! referenced list. Functional list values (the `$current-*` pseudo-lists) are
//! not resolved here; the REST layer rejects them before this point.

use std::collections::HashSet;

use crate::core::ResourceStorage;
use crate::error::StorageResult;
use crate::tenant::TenantContext;
use crate::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, strip_reference_version,
};

/// Sentinel `_id` value that cannot match any real resource, used to force an
/// empty result when a `_list` resolves to no members.
const NO_MATCH_SENTINEL: &str = "__list_search_no_match__";

/// Returns true if the query carries any `_list` filter.
pub fn query_has_list(query: &SearchQuery) -> bool {
    !query.list.is_empty()
}

/// Resolves a query's `_list` filters into an `_id` filter, returning a
/// rewritten query that a plain `search()` can execute.
///
/// Each `_list` value is resolved to the set of `resource_type` member ids of
/// the referenced `List`; the sets are intersected (`AND`). If any list is
/// missing or contributes no members, the rewritten query is forced to match
/// nothing. Queries without `_list` are returned unchanged.
pub async fn resolve_list<S>(
    storage: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> StorageResult<SearchQuery>
where
    S: ResourceStorage + ?Sized,
{
    if !query_has_list(query) {
        return Ok(query.clone());
    }

    let base_type = query.resource_type.as_str();
    let mut id_sets: Vec<HashSet<String>> = Vec::with_capacity(query.list.len());
    for list_ref in &query.list {
        // Accept both a bare logical id (`42`) and a `List/42` reference.
        let list_id = list_ref.strip_prefix("List/").unwrap_or(list_ref);
        id_sets.push(member_ids(storage, tenant, base_type, list_id).await?);
    }

    let matched_ids = intersect(id_sets);

    // Rewrite: drop the `_list` filters and add an `_id` filter for the resolved
    // member ids. Repeated `_id` parameters AND together, so this composes with
    // any user-supplied `_id` and with chain resolution's own `_id` filter.
    let mut rewritten = query.clone();
    rewritten.list.clear();

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

/// Reads `List/<list_id>` and collects the logical ids of its `entry.item`
/// references that point at `base_type`. A missing list yields an empty set
/// (which forces an empty result). Entries flagged `deleted` are skipped.
async fn member_ids<S>(
    storage: &S,
    tenant: &TenantContext,
    base_type: &str,
    list_id: &str,
) -> StorageResult<HashSet<String>>
where
    S: ResourceStorage + ?Sized,
{
    let Some(list) = storage.read(tenant, "List", list_id).await? else {
        return Ok(HashSet::new());
    };

    let mut ids = HashSet::new();
    if let Some(entries) = list.content().get("entry").and_then(|e| e.as_array()) {
        for entry in entries {
            // `entry.deleted = true` marks an item removed from the list.
            if entry
                .get("deleted")
                .and_then(|d| d.as_bool())
                .unwrap_or(false)
            {
                continue;
            }
            let reference = entry
                .get("item")
                .and_then(|item| item.get("reference"))
                .and_then(|r| r.as_str());
            if let Some(reference) = reference {
                if let Some(id) = id_for_type(reference, base_type) {
                    ids.insert(id);
                }
            }
        }
    }
    Ok(ids)
}

/// Extracts the logical id from a reference if it targets `base_type`. Handles
/// both relative (`Patient/123`) and absolute (`http://host/fhir/Patient/123`)
/// references, ignoring any `/_history/<vid>` version suffix.
fn id_for_type(reference: &str, base_type: &str) -> Option<String> {
    let reference = strip_reference_version(reference);
    let (type_part, id) = reference.rsplit_once('/')?;
    let type_name = type_part.rsplit('/').next().unwrap_or(type_part);
    (type_name == base_type).then(|| id.to_string())
}

/// Intersects a list of id sets (`AND` across `_list` filters). An empty input
/// yields an empty set.
fn intersect(mut sets: Vec<HashSet<String>>) -> HashSet<String> {
    let mut iter = sets.drain(..);
    let mut acc = match iter.next() {
        Some(s) => s,
        None => return HashSet::new(),
    };
    for s in iter {
        acc.retain(|id| s.contains(id));
    }
    acc
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

    async fn create_list(
        b: &SqliteBackend,
        t: &TenantContext,
        id: &str,
        entries: serde_json::Value,
    ) {
        b.create(
            t,
            "List",
            json!({ "resourceType": "List", "id": id, "status": "current", "mode": "working", "entry": entries }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    }

    async fn matched_ids(b: &SqliteBackend, t: &TenantContext, query: &SearchQuery) -> Vec<String> {
        let rewritten = resolve_list(b, t, query).await.unwrap();
        let result = b.search(t, &rewritten).await.unwrap();
        let mut ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn list_filters_to_members() {
        let b = backend();
        let t = tenant();
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([
                { "item": { "reference": "Patient/smith" } },
                { "item": { "reference": "Patient/jones" } },
            ]),
        )
        .await;

        let mut query = SearchQuery::new("Patient");
        query.list.push("l1".to_string());
        assert_eq!(matched_ids(&b, &t, &query).await, vec!["jones", "smith"]);
    }

    #[tokio::test]
    async fn list_reference_prefix_accepted() {
        let b = backend();
        let t = tenant();
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([{ "item": { "reference": "Patient/doe" } }]),
        )
        .await;

        let mut query = SearchQuery::new("Patient");
        query.list.push("List/l1".to_string());
        assert_eq!(matched_ids(&b, &t, &query).await, vec!["doe"]);
    }

    #[tokio::test]
    async fn list_skips_deleted_entries_and_other_types() {
        let b = backend();
        let t = tenant();
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([
                { "item": { "reference": "Patient/smith" } },
                { "item": { "reference": "Patient/jones" }, "deleted": true },
                { "item": { "reference": "Observation/o1" } },
            ]),
        )
        .await;

        let mut query = SearchQuery::new("Patient");
        query.list.push("l1".to_string());
        assert_eq!(matched_ids(&b, &t, &query).await, vec!["smith"]);
    }

    #[tokio::test]
    async fn multiple_lists_intersect() {
        let b = backend();
        let t = tenant();
        seed_patients(&b, &t).await;
        create_list(
            &b,
            &t,
            "l1",
            json!([
                { "item": { "reference": "Patient/smith" } },
                { "item": { "reference": "Patient/jones" } },
            ]),
        )
        .await;
        create_list(
            &b,
            &t,
            "l2",
            json!([
                { "item": { "reference": "Patient/jones" } },
                { "item": { "reference": "Patient/doe" } },
            ]),
        )
        .await;

        let mut query = SearchQuery::new("Patient");
        query.list.push("l1".to_string());
        query.list.push("l2".to_string());
        assert_eq!(matched_ids(&b, &t, &query).await, vec!["jones"]);
    }

    #[tokio::test]
    async fn missing_list_yields_empty() {
        let b = backend();
        let t = tenant();
        seed_patients(&b, &t).await;

        let mut query = SearchQuery::new("Patient");
        query.list.push("does-not-exist".to_string());
        assert!(matched_ids(&b, &t, &query).await.is_empty());
    }
}
