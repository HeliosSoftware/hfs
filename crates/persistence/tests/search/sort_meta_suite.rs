//! Backend-agnostic suite for `_sort` on the `meta`-derived parameters
//! `_tag`, `_security`, `_profile` and `_source` (#1711).
//!
//! They are `Resource`-level token (`_tag`, `_security`) and uri (`_profile`,
//! `_source`) parameters, and the extractor indexes them like any other. The
//! REST layer used to hand them to the backends untyped, so every backend
//! sorted by id. This pins the backend half: given the type the REST layer
//! now resolves, each backend orders by the indexed value, and a resource
//! with no value sorts last in either direction (#1606).
//!
//! Included by `#[path]` into each backend's test binary, like
//! `sort_missing_suite.rs`.

#![allow(dead_code)]

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, SortDirection, SortDirective,
};

/// (id, meta value). The values run opposite to the ids, so an id fallback
/// is told apart from a real sort in both directions; `sm-d` has no `meta`.
const PATIENTS: &[(&str, Option<&str>)] = &[
    ("sm-a", Some("zz")),
    ("sm-b", Some("mm")),
    ("sm-c", Some("aa")),
    ("sm-d", None),
];

const PARAMS: &[(&str, SearchParamType)] = &[
    ("_tag", SearchParamType::Token),
    ("_security", SearchParamType::Token),
    ("_profile", SearchParamType::Uri),
    ("_source", SearchParamType::Uri),
];

async fn seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    for (id, value) in PATIENTS {
        let mut patient = json!({"resourceType": "Patient", "id": id, "active": true});
        if let Some(value) = value {
            patient["meta"] = json!({
                "tag": [{"system": "http://example.org/tag", "code": value}],
                "security": [{"system": "http://example.org/sec", "code": value}],
                "profile": [format!("http://example.org/profile/{value}")],
                "source": format!("http://example.org/source/{value}")
            });
        }
        backend
            .create(tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap_or_else(|e| panic!("create Patient/{id} failed: {e}"));
    }
}

async fn ids<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> Vec<String>
where
    S: ResourceStorage + SearchProvider,
{
    backend
        .search(tenant, query)
        .await
        .unwrap_or_else(|e| panic!("search {query:?} failed: {e}"))
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

/// Waits until every seeded Patient is searchable, and every tagged one by
/// `_tag`: the positive control that the meta values are indexed, and the
/// wait for an eventually-consistent index.
async fn wait_for_seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    let active = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "active".to_string(),
            param_type: SearchParamType::Token,
            values: vec![SearchValue::eq("true")],
            ..Default::default()
        })
        .with_count(100);
    let tagged = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "_tag".to_string(),
            param_type: SearchParamType::Token,
            values: PATIENTS
                .iter()
                .filter_map(|(_, v)| *v)
                .map(|v| SearchValue::eq(format!("http://example.org/tag|{v}")))
                .collect(),
            ..Default::default()
        })
        .with_count(100);
    let want_tagged = PATIENTS.iter().filter(|(_, v)| v.is_some()).count();
    for attempt in 0..60 {
        let all = ids(backend, tenant, &active).await.len();
        let with_tag = ids(backend, tenant, &tagged).await.len();
        if all == PATIENTS.len() && with_tag == want_tagged {
            return;
        }
        assert!(
            attempt < 59,
            "the seeded Patients never became searchable (active {all}/{}, _tag {with_tag}/{want_tagged})",
            PATIENTS.len(),
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// `_sort` on each meta parameter orders by its indexed value, ascending and
/// descending, with the resource that has no value last both ways.
pub async fn meta_params_sort_by_value<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;
    wait_for_seed(backend, &tenant).await;

    for (param, param_type) in PARAMS {
        for (direction, expected) in [
            (SortDirection::Ascending, ["sm-c", "sm-b", "sm-a", "sm-d"]),
            (SortDirection::Descending, ["sm-a", "sm-b", "sm-c", "sm-d"]),
        ] {
            let query = SearchQuery::new("Patient")
                .with_count(100)
                .with_sort(SortDirective {
                    parameter: param.to_string(),
                    direction,
                    param_type: Some(*param_type),
                });
            assert_eq!(
                ids(backend, &tenant, &query).await,
                expected,
                "_sort={param} {direction:?}"
            );
        }
    }
}
