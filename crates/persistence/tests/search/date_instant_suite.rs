//! Backend-agnostic suite for date parameters on `instant` targets (#2962).
//!
//! Every indexed date is a range `[ts, te)` (#1391), and a `date` or
//! `dateTime` covers one unit of its own precision: `2020-01-01T10:00:00Z` is
//! the whole second. An `instant` is not like that. FHIR search says it is
//! "considered a fixed point in time with an interval smaller than the
//! precision of the system, i.e. an interval with an effective width of 0"
//! (<https://hl7.org/fhir/R4/search.html#date>, the same in R4B and R5). An
//! `instant` written to the second used to be indexed as the whole second, so
//! `issued=eq2020-01-01T10:00:00.000Z` missed it and
//! `issued=gt2020-01-01T10:00:00.500Z` found it.
//!
//! It is indexed as the point `[t, t + one unit of the backend's
//! resolution)` now, whatever precision it is written to. A `dateTime` with
//! the same text, in the same parameter, keeps its one-second range: the
//! controls below.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `date_period_suite.rs`. The expectations are the R4/R4B/R5 ones:
//! `DiagnosticReport.issued` is an `instant` there (R6 makes it a
//! `dateTime`). The backend must be built with the spec search parameters
//! loaded; the positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

use crate::seeding::SeedingStorage;

/// `DiagnosticReport.issued`, an `instant`, written to the second.
const DR_INSTANT_S: &str = "dr-instant-s";
/// `DiagnosticReport.issued` with the same instant written to the millisecond.
const DR_INSTANT_MS: &str = "dr-instant-ms";
/// `DiagnosticReport.effectiveDateTime`, a `dateTime` with the same text: the
/// `date` parameter's control.
const DR_DATETIME_S: &str = "dr-datetime-s";
/// `Observation.effectiveInstant`, reached through the `effective[x]` choice
/// of the `date` parameter.
const OBS_INSTANT_S: &str = "obs-instant-s";
/// `Observation.effectiveDateTime` with the same text, in the same parameter:
/// the choice's control.
const OBS_DATETIME_S: &str = "obs-datetime-s";

const AT: &str = "2020-01-01T10:00:00Z";
const AT_MS: &str = "2020-01-01T10:00:00.000Z";

/// `DiagnosticReport?issued=` value → the reports it must match. Both are
/// indexed as `[10:00:00.000, 10:00:00.001)`.
const ISSUED_CASES: &[(&str, &[&str])] = &[
    // The two queries #2962 reported.
    ("eq2020-01-01T10:00:00.000Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("gt2020-01-01T10:00:00.500Z", &[]),
    // Every other prefix at the millisecond, either side of the point.
    ("2020-01-01T10:00:00.000Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("ne2020-01-01T10:00:00.000Z", &[]),
    ("ge2020-01-01T10:00:00.000Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("le2020-01-01T10:00:00.000Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("lt2020-01-01T10:00:00.500Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("ge2020-01-01T10:00:00.500Z", &[]),
    ("le2020-01-01T10:00:00.500Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("sa2020-01-01T10:00:00.000Z", &[]),
    ("eb2020-01-01T10:00:00.001Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("eb2020-01-01T10:00:00.000Z", &[]),
    ("2020-01-01T10:00:00.001Z", &[]),
    // Coarser search values still contain the point, as before.
    ("eq2020-01-01T10:00:00Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("2020-01-01T10:00Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("2020-01-01", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("gt2020-01-01T09:59:59Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
    ("gt2020-01-01T10:00:00Z", &[]),
    ("lt2020-01-01T10:00:01Z", &[DR_INSTANT_MS, DR_INSTANT_S]),
];

/// `DiagnosticReport?date=` value → the reports it must match. Only
/// `dr-datetime-s` has an `effective[x]`: a `dateTime`, so the whole second.
const DR_DATE_CASES: &[(&str, &[&str])] = &[
    ("eq2020-01-01T10:00:00.000Z", &[]),
    ("gt2020-01-01T10:00:00.500Z", &[DR_DATETIME_S]),
    ("eq2020-01-01T10:00:00Z", &[DR_DATETIME_S]),
];

/// `Observation?date=` value → the observations it must match: the instant
/// and the dateTime arm of one choice, with the same text.
const OBS_DATE_CASES: &[(&str, &[&str])] = &[
    ("eq2020-01-01T10:00:00.000Z", &[OBS_INSTANT_S]),
    ("gt2020-01-01T10:00:00.500Z", &[OBS_DATETIME_S]),
    (
        "lt2020-01-01T10:00:00.500Z",
        &[OBS_DATETIME_S, OBS_INSTANT_S],
    ),
    ("ne2020-01-01T10:00:00.000Z", &[OBS_DATETIME_S]),
    ("eq2020-01-01T10:00:00Z", &[OBS_DATETIME_S, OBS_INSTANT_S]),
    ("gt2020-01-01T10:00:00Z", &[]),
];

fn query(
    resource_type: &str,
    param: &str,
    param_type: SearchParamType,
    value: &str,
) -> SearchQuery {
    SearchQuery::new(resource_type)
        .with_parameter(SearchParameter {
            name: param.to_string(),
            param_type,
            values: vec![SearchValue::parse(value)],
            ..Default::default()
        })
        .with_count(100)
}

async fn matched<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> BTreeSet<String>
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

fn ids(expected: &[&str]) -> BTreeSet<String> {
    expected.iter().map(|id| id.to_string()).collect()
}

async fn create<S>(backend: &S, tenant: &TenantContext, resource_type: &str, resource: Value)
where
    S: ResourceStorage + SearchProvider,
{
    create_as(
        backend,
        tenant,
        resource_type,
        resource,
        FhirVersion::default(),
    )
    .await;
}

async fn create_as<S>(
    backend: &S,
    tenant: &TenantContext,
    resource_type: &str,
    resource: Value,
    version: FhirVersion,
) where
    S: ResourceStorage + SearchProvider,
{
    let id = resource["id"].as_str().unwrap_or_default().to_string();
    backend
        .seed_create(tenant, resource_type, resource, version)
        .await
        .unwrap_or_else(|e| panic!("create {resource_type}/{id} failed: {e}"));
}

/// Waits until `expected` resources of the type are searchable by
/// `status=final`: the positive control that they are stored and indexed, and
/// the wait for eventually-consistent indexes.
async fn wait_for<S>(backend: &S, tenant: &TenantContext, resource_type: &str, expected: usize)
where
    S: ResourceStorage + SearchProvider,
{
    let control = query(resource_type, "status", SearchParamType::Token, "final");
    for attempt in 0..60 {
        let visible = matched(backend, tenant, &control).await;
        if visible.len() == expected {
            return;
        }
        assert!(
            attempt < 59,
            "the seeded {resource_type}s never became searchable by status ({}/{expected})",
            visible.len(),
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    let report = |id: &str, field: &str, value: &str| {
        json!({
            "id": id,
            "status": "final",
            "code": {"text": "instant suite"},
            field: value,
        })
    };
    for resource in [
        report(DR_INSTANT_S, "issued", AT),
        report(DR_INSTANT_MS, "issued", AT_MS),
        report(DR_DATETIME_S, "effectiveDateTime", AT),
    ] {
        create(backend, tenant, "DiagnosticReport", resource).await;
    }
    for (id, field) in [
        (OBS_INSTANT_S, "effectiveInstant"),
        (OBS_DATETIME_S, "effectiveDateTime"),
    ] {
        create(
            backend,
            tenant,
            "Observation",
            json!({
                "id": id,
                "status": "final",
                "code": {"text": "instant suite"},
                field: AT,
            }),
        )
        .await;
    }
    wait_for(backend, tenant, "DiagnosticReport", 3).await;
    wait_for(backend, tenant, "Observation", 2).await;
}

/// Seeds the resources under a caller-unique tenant and asserts every table:
/// `DiagnosticReport.issued`, an `instant` element; `DiagnosticReport.date`
/// over a `dateTime`, the control; and `Observation.date`, whose
/// `effective[x]` is an `instant` on one resource and a `dateTime` on the
/// other.
pub async fn instant_targets_are_points<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;

    for (value, expected) in ISSUED_CASES {
        assert_eq!(
            matched(
                backend,
                &tenant,
                &query("DiagnosticReport", "issued", SearchParamType::Date, value)
            )
            .await,
            ids(expected),
            "DiagnosticReport?issued={value}"
        );
    }
    for (value, expected) in DR_DATE_CASES {
        assert_eq!(
            matched(
                backend,
                &tenant,
                &query("DiagnosticReport", "date", SearchParamType::Date, value)
            )
            .await,
            ids(expected),
            "DiagnosticReport?date={value} (effectiveDateTime)"
        );
    }
    for (value, expected) in OBS_DATE_CASES {
        assert_eq!(
            matched(
                backend,
                &tenant,
                &query("Observation", "date", SearchParamType::Date, value)
            )
            .await,
            ids(expected),
            "Observation?date={value} (effectiveInstant vs effectiveDateTime)"
        );
    }
}

/// One resource of `resource_type` with `field` set to `AT` (written to the
/// second), stored as `version`.
async fn seed_one<S>(
    backend: &S,
    tenant: &TenantContext,
    resource_type: &str,
    id: &str,
    field: &str,
    version: FhirVersion,
) where
    S: ResourceStorage + SearchProvider,
{
    let mut resource = json!({"id": id, "status": "final", "code": {"text": "instant suite"}});
    if resource_type == "ImagingSelection" {
        resource["status"] = json!("available");
    }
    resource[field] = json!(AT);
    create_as(backend, tenant, resource_type, resource, version).await;
}

/// Asserts the two searches #2962 reported against one resource whose
/// `param` target is written to the second: as a point, `eq…:00.000Z` finds
/// it and `gt…:00.500Z` does not; as a one-second range, the reverse.
async fn assert_point_or_range<S>(
    backend: &S,
    tenant: &TenantContext,
    resource_type: &str,
    param: &str,
    id: &str,
    point: bool,
) where
    S: ResourceStorage + SearchProvider,
{
    let found = ids(&[id]);
    let (eq, gt) = if point {
        (found.clone(), BTreeSet::new())
    } else {
        (BTreeSet::new(), found.clone())
    };
    let what = if point {
        "a point"
    } else {
        "a one-second range"
    };
    for (value, expected) in [
        ("eq2020-01-01T10:00:00.000Z", eq),
        ("gt2020-01-01T10:00:00.500Z", gt),
        // Either way the whole second contains it: the positive control.
        ("eq2020-01-01T10:00:00Z", found.clone()),
    ] {
        assert_eq!(
            matched(
                backend,
                tenant,
                &query(resource_type, param, SearchParamType::Date, value)
            )
            .await,
            expected,
            "{resource_type}?{param}={value}: the target must be {what} for the backend's FHIR version"
        );
    }
}

/// The backend's configured FHIR version decides which elements are
/// `instant`s (#2962), not the first version the build enables. The backend
/// must be configured for R6, with R6's search parameters loaded.
///
/// R6 makes `DiagnosticReport.issued` a `dateTime`, so it keeps its
/// one-second range: `issued=eq…:00.000Z` does not find a report written
/// `…:00Z`, and `gt…:00.500Z` does. `Observation.effectiveInstant` is still an
/// `instant` in R6, the control that the R6 backend applies the point rule.
/// A backend that built its extractor for another version (R4, the first
/// enabled in an all-versions build) would index `issued` as a point.
#[cfg(feature = "R6")]
pub async fn r6_backend_types_by_r6<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let version = FhirVersion::R6;
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed_one(
        backend,
        &tenant,
        "DiagnosticReport",
        DR_INSTANT_S,
        "issued",
        version,
    )
    .await;
    seed_one(
        backend,
        &tenant,
        "Observation",
        OBS_INSTANT_S,
        "effectiveInstant",
        version,
    )
    .await;
    wait_for(backend, &tenant, "DiagnosticReport", 1).await;
    wait_for(backend, &tenant, "Observation", 1).await;
    assert_point_or_range(
        backend,
        &tenant,
        "DiagnosticReport",
        "issued",
        DR_INSTANT_S,
        false,
    )
    .await;
    assert_point_or_range(backend, &tenant, "Observation", "date", OBS_INSTANT_S, true).await;
}

/// The R4 control for [`r6_backend_types_by_r6`], in the same style: on a
/// backend configured for R4, `DiagnosticReport.issued` is an `instant` and a
/// point.
#[cfg(feature = "R4")]
pub async fn r4_backend_types_by_r4<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let version = FhirVersion::R4;
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed_one(
        backend,
        &tenant,
        "DiagnosticReport",
        DR_INSTANT_S,
        "issued",
        version,
    )
    .await;
    wait_for(backend, &tenant, "DiagnosticReport", 1).await;
    assert_point_or_range(
        backend,
        &tenant,
        "DiagnosticReport",
        "issued",
        DR_INSTANT_S,
        true,
    )
    .await;
}

/// R5 adds `ImagingSelection.issued`, an `instant` (R4 has no
/// `ImagingSelection`). On a backend configured for R5 it is a point; an
/// extractor built for R4 would not find the element and keep the range.
#[cfg(feature = "R5")]
pub async fn r5_backend_types_by_r5<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let version = FhirVersion::R5;
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed_one(
        backend,
        &tenant,
        "ImagingSelection",
        "is-instant-s",
        "issued",
        version,
    )
    .await;
    let control = query(
        "ImagingSelection",
        "status",
        SearchParamType::Token,
        "available",
    );
    for attempt in 0..60 {
        if matched(backend, &tenant, &control).await.len() == 1 {
            break;
        }
        assert!(
            attempt < 59,
            "the seeded ImagingSelection never became searchable by status"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_point_or_range(
        backend,
        &tenant,
        "ImagingSelection",
        "issued",
        "is-instant-s",
        true,
    )
    .await;
}
