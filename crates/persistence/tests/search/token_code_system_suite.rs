//! Backend-agnostic suite for `system|code` token searches on elements whose
//! datatype is `code` (issue #1379).
//!
//! A `code` primitive is a bare JSON string, so its index row carries no
//! system of its own: FHIR says "the system is implicit", defined by the
//! element's binding. `Patient?gender=female` matched, but
//! `gender=http://hl7.org/fhir/administrative-gender|female` never did, and
//! since terminology expansion always yields `system|code` tokens, neither
//! did `gender:in=<valueset>`.
//!
//! The index now marks such rows with
//! `helios_persistence::search::IMPLICIT_TOKEN_SYSTEM`, and a `system|code`
//! search accepts a marked row whatever system the client named. It does NOT
//! verify the named system against the binding (the index does not know it),
//! so a wrong system matches too; the table below states that outright. The
//! negative control is the other half: a `Coding` that genuinely has no
//! `system` is not marked and still never matches `system|code`.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `number_exponent_suite.rs`. The backend must be built with the spec search
//! parameters loaded; the suite's positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

const GENDER: &str = "http://hl7.org/fhir/administrative-gender";
const OBS_STATUS: &str = "http://hl7.org/fhir/observation-status";
const LOINC: &str = "http://loinc.org";

/// One row of the table: resource type, parameter, modifier, value, and the
/// seeded ids it must match.
type Case = (
    &'static str,
    &'static str,
    Option<SearchModifier>,
    String,
    &'static [&'static str],
);

/// The seeded resources:
///
/// - `pt-f` / `pt-m`: `Patient.gender` female / male; `pt-none` has no gender.
/// - `ob-loinc`: `status` final, `code` = `http://loinc.org|1234-5`.
/// - `ob-nosys`: `status` final, `code` = a Coding with ONLY a code, `1234-5`
///   — the negative control.
/// - `ob-prelim`: `status` preliminary, `code` = `http://loinc.org|9999-9`.
fn cases() -> Vec<Case> {
    let none = None;
    let not = Some(SearchModifier::Not);
    vec![
        // `code` element, Patient.gender.
        (
            "Patient",
            "gender",
            none.clone(),
            "female".into(),
            &["pt-f"],
        ),
        (
            "Patient",
            "gender",
            none.clone(),
            format!("{GENDER}|female"),
            &["pt-f"],
        ),
        // A `code` element has no system property, so `|code` keeps matching.
        (
            "Patient",
            "gender",
            none.clone(),
            "|female".into(),
            &["pt-f"],
        ),
        // The named system is NOT verified for a `code` element.
        (
            "Patient",
            "gender",
            none.clone(),
            "http://wrong.example|female".into(),
            &["pt-f"],
        ),
        (
            "Patient",
            "gender",
            none.clone(),
            format!("{GENDER}|male,{GENDER}|other"),
            &["pt-m"],
        ),
        // `:not` is the exact negation, and includes the patient with no
        // gender at all.
        (
            "Patient",
            "gender",
            not.clone(),
            format!("{GENDER}|female"),
            &["pt-m", "pt-none"],
        ),
        // `system|` names every code OF a system. The index cannot tell which
        // system a `code` element draws from, so this stays a non-match
        // rather than returning every patient that has a gender.
        ("Patient", "gender", none.clone(), format!("{GENDER}|"), &[]),
        // `code` element, Observation.status.
        (
            "Observation",
            "status",
            none.clone(),
            format!("{OBS_STATUS}|final"),
            &["ob-loinc", "ob-nosys"],
        ),
        (
            "Observation",
            "status",
            not.clone(),
            format!("{OBS_STATUS}|final"),
            &["ob-prelim"],
        ),
        // CodeableConcept element, Observation.code: real systems are still
        // compared exactly.
        (
            "Observation",
            "code",
            none.clone(),
            "1234-5".into(),
            &["ob-loinc", "ob-nosys"],
        ),
        (
            "Observation",
            "code",
            none.clone(),
            format!("{LOINC}|1234-5"),
            // NEGATIVE CONTROL: not `ob-nosys`, whose Coding has no system.
            &["ob-loinc"],
        ),
        (
            "Observation",
            "code",
            none.clone(),
            "http://wrong.example|1234-5".into(),
            &[],
        ),
        (
            "Observation",
            "code",
            not.clone(),
            format!("{LOINC}|1234-5"),
            &["ob-nosys", "ob-prelim"],
        ),
        (
            "Observation",
            "code",
            none.clone(),
            format!("{LOINC}|"),
            &["ob-loinc", "ob-prelim"],
        ),
    ]
}

/// `|code` on an element that does carry systems. SQLite and Elasticsearch
/// implement "has no system"; PostgreSQL and MongoDB treat `|code` as a bare
/// code (pre-existing, out of scope for #1379), so the expectation is the
/// caller's.
fn no_system_case(strict: bool) -> Case {
    let expected: &'static [&'static str] = if strict {
        &["ob-nosys"]
    } else {
        &["ob-loinc", "ob-nosys"]
    };
    ("Observation", "code", None, "|1234-5".into(), expected)
}

fn query(
    resource_type: &str,
    param: &str,
    modifier: Option<SearchModifier>,
    value: &str,
) -> SearchQuery {
    SearchQuery::new(resource_type)
        .with_parameter(SearchParameter {
            name: param.to_string(),
            param_type: SearchParamType::Token,
            modifier,
            // Comma-separated values are an OR-list, as the REST layer parses them.
            values: value.split(',').map(SearchValue::eq).collect(),
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

/// Seeds the resources under a caller-unique tenant and asserts the table.
/// `strict_no_system` says whether the backend implements `|code` as "has no
/// system" (see [`no_system_case`]).
pub async fn system_qualified_tokens_match_code_elements<S>(
    backend: &S,
    tenant_base: &str,
    strict_no_system: bool,
) where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    let resources = [
        ("Patient", json!({"id": "pt-f", "gender": "female"})),
        ("Patient", json!({"id": "pt-m", "gender": "male"})),
        ("Patient", json!({"id": "pt-none", "active": true})),
        (
            "Observation",
            json!({
                "id": "ob-loinc",
                "status": "final",
                "code": {"coding": [{"system": LOINC, "code": "1234-5"}]},
            }),
        ),
        (
            "Observation",
            json!({
                "id": "ob-nosys",
                "status": "final",
                "code": {"coding": [{"code": "1234-5"}]},
            }),
        ),
        (
            "Observation",
            json!({
                "id": "ob-prelim",
                "status": "preliminary",
                "code": {"coding": [{"system": LOINC, "code": "9999-9"}]},
            }),
        ),
    ];
    for (resource_type, resource) in resources {
        let id = resource["id"].as_str().unwrap_or_default().to_string();
        backend
            .create(&tenant, resource_type, resource, FhirVersion::default())
            .await
            .unwrap_or_else(|e| panic!("create {id} failed: {e}"));
    }

    // Positive controls, polled because Elasticsearch is near-real-time. A
    // failure here means the parameter did not index — a backend built without
    // the spec search parameters — not that the fix is wrong.
    for (resource_type, param, value, expected) in [
        ("Patient", "gender", "female", &["pt-f"][..]),
        ("Observation", "status", "preliminary", &["ob-prelim"][..]),
        ("Observation", "code", "9999-9", &["ob-prelim"][..]),
    ] {
        let control = query(resource_type, param, None, value);
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

    let mut table = cases();
    table.push(no_system_case(strict_no_system));

    let mut failures = Vec::new();
    for (resource_type, param, modifier, value, expected) in table {
        let got = matched(
            backend,
            &tenant,
            &query(resource_type, param, modifier.clone(), &value),
        )
        .await;
        let shown = match &modifier {
            Some(m) => format!("{resource_type}?{param}:{m}={value}"),
            None => format!("{resource_type}?{param}={value}"),
        };
        println!("{shown} -> {got:?}");
        if got != ids(expected) {
            failures.push(format!("{shown}: got {got:?}, expected {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
