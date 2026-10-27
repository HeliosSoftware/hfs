//! Backend-agnostic suite for number and quantity search values written with
//! an exponent (issue #1337).
//!
//! FHIR derives the implicit range of an `eq`/`ne` search value from its
//! significant figures, and an exponent moves the last significant digit:
//! `1e-1` is `[0.05, 0.15)` like `0.1`, `1e2` is `[50, 150)`, and `1.00e2` is
//! `[99.5, 100.5)` like `100`. `helios_persistence::search::implicit_precision`
//! used to count every character after the `.` as a fraction digit and to
//! ignore the exponent, so `1e2` searched `[99.5, 100.5)` and `1.00e2`
//! searched `[99.99995, 100.00005)`.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `date_precision_suite.rs`. The backend must be built with the spec search
//! parameters loaded; the suite's positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// (id, `ChargeItem.factorOverride`), either side of the bounds of `1e-1`.
const FACTORS: &[(&str, f64)] = &[
    ("ci-004", 0.04),
    ("ci-006", 0.06),
    ("ci-010", 0.1),
    ("ci-014", 0.14),
    ("ci-016", 0.16),
];

/// (id, `Observation.valueQuantity.value` in mg), either side of the bounds
/// of `1e2` and of `100`.
const DOSES: &[(&str, f64)] = &[
    ("ob-040", 40.0),
    ("ob-060", 60.0),
    ("ob-099-7", 99.7),
    ("ob-100", 100.0),
    ("ob-140", 140.0),
    ("ob-160", 160.0),
];

/// `factor-override=` value → the seeded ChargeItems it must match. The first
/// row is the positive control.
const NUMBER_CASES: &[(&str, &[&str])] = &[
    ("ge0", &["ci-004", "ci-006", "ci-010", "ci-014", "ci-016"]),
    ("0.1", &["ci-006", "ci-010", "ci-014"]),
    ("1e-1", &["ci-006", "ci-010", "ci-014"]),
    ("1E-1", &["ci-006", "ci-010", "ci-014"]),
    ("+1e-1", &["ci-006", "ci-010", "ci-014"]),
    ("eq1e-1", &["ci-006", "ci-010", "ci-014"]),
    // Two significant figures: [0.095, 0.105).
    ("1.0e-1", &["ci-010"]),
    ("10e-2", &["ci-010"]),
    // [0.145, 0.155) holds nothing.
    ("1.5e-1", &[]),
    ("ne1e-1", &["ci-004", "ci-016"]),
    ("ne1.0e-1", &["ci-004", "ci-006", "ci-014", "ci-016"]),
    // Comparators ignore the precision, with or without an exponent.
    ("gt1e-1", &["ci-014", "ci-016"]),
    ("ge1e-1", &["ci-010", "ci-014", "ci-016"]),
    ("lt1e-1", &["ci-004", "ci-006"]),
    ("le1.0e-1", &["ci-004", "ci-006", "ci-010"]),
];

/// `value-quantity=` value → the seeded Observations it must match. The first
/// row is the positive control.
const QUANTITY_CASES: &[(&str, &[&str])] = &[
    (
        "ge0||mg",
        &["ob-040", "ob-060", "ob-099-7", "ob-100", "ob-140", "ob-160"],
    ),
    ("100||mg", &["ob-099-7", "ob-100"]),
    ("1.00e2||mg", &["ob-099-7", "ob-100"]),
    ("1.0e2||mg", &["ob-099-7", "ob-100"]),
    ("1e2||mg", &["ob-060", "ob-099-7", "ob-100", "ob-140"]),
    ("1E+2||mg", &["ob-060", "ob-099-7", "ob-100", "ob-140"]),
    ("1e2", &["ob-060", "ob-099-7", "ob-100", "ob-140"]),
    (
        "1e2|http://unitsofmeasure.org|mg",
        &["ob-060", "ob-099-7", "ob-100", "ob-140"],
    ),
    ("ne1e2||mg", &["ob-040", "ob-160"]),
    ("ge1e2||mg", &["ob-100", "ob-140", "ob-160"]),
    ("lt1e2||mg", &["ob-040", "ob-060", "ob-099-7"]),
];

/// For backends that also match UCUM-equivalent units: the converted bounds
/// come from the same range, and `1e-1 g` is `[50 mg, 150 mg)`.
const CONVERTED_QUANTITY_CASES: &[(&str, &[&str])] = &[
    ("1e-1||g", &["ob-060", "ob-099-7", "ob-100", "ob-140"]),
    ("1.00e-1||g", &["ob-099-7", "ob-100"]),
    ("ne1e-1||g", &["ob-040", "ob-160"]),
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

/// Seeds the resources under a caller-unique tenant and asserts the tables.
/// `unit_conversion` adds the cases that need UCUM-canonical quantity columns.
pub async fn exponent_values_use_significant_figures<S>(
    backend: &S,
    tenant_base: &str,
    unit_conversion: bool,
) where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    for (id, factor) in FACTORS {
        backend
            .create(
                &tenant,
                "ChargeItem",
                json!({
                    "id": id,
                    "status": "billable",
                    "code": {"text": "x"},
                    "subject": {"reference": "Patient/ne-subject"},
                    "factorOverride": factor,
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create {id} failed: {e}"));
    }
    for (id, dose) in DOSES {
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": id,
                    "status": "final",
                    "code": {"text": "dose"},
                    "valueQuantity": {
                        "value": dose,
                        "unit": "mg",
                        "system": "http://unitsofmeasure.org",
                        "code": "mg",
                    },
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("create {id} failed: {e}"));
    }

    let number = ("ChargeItem", "factor-override", SearchParamType::Number);
    let quantity = ("Observation", "value-quantity", SearchParamType::Quantity);

    // Positive controls, polled because Elasticsearch is near-real-time. A
    // failure here means the parameter did not index — a backend built without
    // the spec search parameters — not that the fix is wrong.
    for ((resource_type, param, param_type), (value, expected)) in
        [(number, NUMBER_CASES[0]), (quantity, QUANTITY_CASES[0])]
    {
        let control = query(resource_type, param, param_type, value);
        let mut got = BTreeSet::new();
        for _ in 0..60 {
            got = matched(backend, &tenant, &control).await;
            if got == ids(expected) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert_eq!(
            got,
            ids(expected),
            "positive control {resource_type}?{param}={value}"
        );
    }

    let converted: &[(&str, &[&str])] = if unit_conversion {
        CONVERTED_QUANTITY_CASES
    } else {
        &[]
    };
    let tables = [
        (number, NUMBER_CASES),
        (quantity, QUANTITY_CASES),
        (quantity, converted),
    ];
    let mut failures = Vec::new();
    for ((resource_type, param, param_type), cases) in tables {
        for (value, expected) in cases {
            let got = matched(
                backend,
                &tenant,
                &query(resource_type, param, param_type, value),
            )
            .await;
            if got != ids(expected) {
                failures.push(format!(
                    "{resource_type}?{param}={value}: got {got:?}, expected {expected:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
