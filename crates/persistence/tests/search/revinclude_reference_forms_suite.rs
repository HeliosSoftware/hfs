//! Backend-agnostic `_revinclude` reference-form suite (issue #2296).
//!
//! `Patient?_id=p1&_revinclude=Provenance:target` must include every
//! Provenance that references `Patient/p1`, however the reference is written:
//! relative (`Patient/p1`), versioned (`Patient/p1/_history/3`), absolute
//! (`http://example.org/fhir/Patient/p1`) or absolute and versioned. MongoDB
//! matched the reference value by exact equality and missed the last three;
//! PostgreSQL strips the version at index time and missed the absolute ones.
//!
//! The suite also holds two negative controls: an absolute reference to a
//! *different type* sharing the id (`Practitioner/p1`), and an absolute
//! reference to `pX1` when the target id is `p.1` (a regex metacharacter that
//! must not behave as a wildcard).
//!
//! A backend that knowingly differs passes [`Divergence`]s, each naming a
//! Provenance id and whether that backend includes it. A divergence that
//! stops being one fails the suite, so the list cannot rot.
//!
//! Included by `#[path]` into each backend's test binary. The backend must be
//! built with the spec search parameters loaded. Truncation under
//! `max_included_resources` is backend configuration and stays covered by
//! each backend's own test (for MongoDB,
//! `mongodb_revinclude_caps_included_and_signals_truncation`).

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider, resolve_includes_iterative};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    IncludeDirective, IncludeType, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

const ABS: &str = "http://example.org/fhir";

/// A Provenance on which one backend knowingly differs from the table.
pub struct Divergence {
    /// The Provenance's id.
    pub id: &'static str,
    /// Whether this backend includes it for the search that should match it.
    pub included: bool,
}

/// (Provenance id, target reference, the Patient id whose `_revinclude` must
/// contain it, or `None` for a negative control that no search may include).
const PROVENANCES: &[(&str, &str, Option<&str>)] = &[
    ("prov-rel", "Patient/p1", Some("p1")),
    ("prov-ver", "Patient/p1/_history/3", Some("p1")),
    ("prov-abs", "http://example.org/fhir/Patient/p1", Some("p1")),
    (
        "prov-abs-ver",
        "http://example.org/fhir/Patient/p1/_history/2",
        Some("p1"),
    ),
    ("prov-dot", "Patient/p.1", Some("p.1")),
    (
        "prov-dot-abs",
        "http://example.org/fhir/Patient/p.1",
        Some("p.1"),
    ),
    // Negative controls.
    (
        "prov-other-type",
        "http://example.org/fhir/Practitioner/p1",
        None,
    ),
    ("prov-other-id", "http://example.org/fhir/Patient/pX1", None),
];

fn revinclude_query(patient_id: &str) -> SearchQuery {
    SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            values: vec![SearchValue::eq(patient_id)],
            ..Default::default()
        })
        .with_include(IncludeDirective {
            include_type: IncludeType::Revinclude,
            source_type: "Provenance".to_string(),
            search_param: "target".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        })
        .with_count(100)
}

/// The ids of the Provenance resources `_revinclude` returns for the Patient.
/// Backends that resolve includes outside `search()` (SQLite) get the same
/// second step the REST layer runs.
async fn included_ids<S>(
    backend: &S,
    tenant: &TenantContext,
    patient_id: &str,
) -> Result<BTreeSet<String>, String>
where
    S: ResourceStorage + SearchProvider,
{
    let query = revinclude_query(patient_id);
    let mut result = backend
        .search(tenant, &query)
        .await
        .map_err(|e| format!("search failed: {e}"))?;
    if result.included.is_empty() {
        result.included =
            resolve_includes_iterative(backend, tenant, &result.resources.items, &query.includes)
                .await
                .map_err(|e| format!("include resolution failed: {e}"))?;
    }
    Ok(result
        .included
        .iter()
        .filter(|r| r.resource_type() == "Provenance")
        .map(|r| r.id().to_string())
        .collect())
}

pub async fn revinclude_matches_every_reference_form<S>(
    backend: &S,
    tenant_base: &str,
    divergences: &[Divergence],
) where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    let known: BTreeSet<&str> = PROVENANCES.iter().map(|(id, _, _)| *id).collect();
    for divergence in divergences {
        assert!(
            known.contains(divergence.id),
            "divergence names no seeded Provenance: {}",
            divergence.id
        );
    }

    let mut resources = vec![
        ("Patient", json!({"id": "p1"})),
        ("Patient", json!({"id": "p.1"})),
        ("Patient", json!({"id": "pX1"})),
        ("Practitioner", json!({"id": "p1"})),
    ];
    for (id, target, _) in PROVENANCES {
        resources.push((
            "Provenance",
            json!({
                "id": id,
                "target": [{"reference": target}],
                "recorded": "2024-01-01T00:00:00Z",
                "agent": [{"who": {"reference": format!("{ABS}/Practitioner/author")}}],
            }),
        ));
    }
    for (resource_type, resource) in resources {
        let id = resource["id"].as_str().unwrap_or_default().to_string();
        backend
            .create(&tenant, resource_type, resource, FhirVersion::default())
            .await
            .unwrap_or_else(|e| panic!("create {resource_type}/{id} failed: {e}"));
    }

    let mut failures = Vec::new();
    for patient_id in ["p1", "p.1"] {
        let want: BTreeSet<String> = PROVENANCES
            .iter()
            .filter(|(_, _, owner)| *owner == Some(patient_id))
            .filter(|(id, _, _)| {
                divergences
                    .iter()
                    .find(|d| d.id == *id)
                    .map(|d| d.included)
                    .unwrap_or(true)
            })
            .map(|(id, _, _)| id.to_string())
            .collect();

        // Polled because Elasticsearch is near-real-time.
        let mut got = Ok(BTreeSet::new());
        for _ in 0..60 {
            got = included_ids(backend, &tenant, patient_id).await;
            if got.as_ref() == Ok(&want) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        if got.as_ref() != Ok(&want) {
            failures.push(format!(
                "Patient?_id={patient_id}&_revinclude=Provenance:target\n    want {want:?}\n    got  {got:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "_revinclude disagrees with the table:\n  {}",
        failures.join("\n  ")
    );
}
