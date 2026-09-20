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

/// Seeds one Patient for [`unmarked_rows_keep_their_old_behaviour`] and waits
/// for it to be searchable. The caller then strips the marker from its
/// `gender` row through the backend's own store, which is what a row indexed
/// before #1379 looks like: no system at all.
pub async fn seed_for_unmarked_rows<S>(backend: &S, tenant_base: &str) -> TenantContext
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    backend
        .create(
            &tenant,
            "Patient",
            json!({"id": "pt-old", "gender": "female"}),
            FhirVersion::default(),
        )
        .await
        .unwrap_or_else(|e| panic!("create pt-old failed: {e}"));

    // Indexed as today: the qualified form matches. Polled for Elasticsearch.
    let qualified = query("Patient", "gender", None, &format!("{GENDER}|female"));
    let mut got = BTreeSet::new();
    for _ in 0..60 {
        got = matched(backend, &tenant, &qualified).await;
        if got == ids(&["pt-old"]) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(got, ids(&["pt-old"]), "marked row matches system|code");
    tenant
}

/// A row without the marker cannot be told from a system-less Coding, so it
/// behaves exactly as it did before #1379 — it never over-matches — until the
/// resource is reindexed.
pub async fn unmarked_rows_keep_their_old_behaviour<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    let not = Some(SearchModifier::Not);
    let qualified = format!("{GENDER}|female");
    let mut failures = Vec::new();
    for (modifier, value, expected) in [
        // Positive control: the row is still there.
        (None, "female", &["pt-old"][..]),
        (None, "|female", &["pt-old"][..]),
        (None, qualified.as_str(), &[][..]),
        (not, qualified.as_str(), &["pt-old"][..]),
    ] {
        let got = matched(
            backend,
            tenant,
            &query("Patient", "gender", modifier.clone(), value),
        )
        .await;
        if got != ids(expected) {
            failures.push(format!(
                "gender {modifier:?} = {value}: got {got:?}, expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The same predicate reached as the terminal of a forward chain and of
/// `_has`. (No composite case: R4 defines no usable composite with a `code`
/// component — `DocumentReference?relationship` has its component expressions
/// swapped in the spec. The composite builders are pinned by unit tests.)
///
/// Chains go through `resolve_chains`, as the REST layer sends them; the SQL
/// chain builders' copies of the predicate are pinned by their unit tests.
pub async fn system_qualified_tokens_in_chains<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    use helios_persistence::search::resolve_chains;
    use helios_persistence::types::{ChainedParameter, ReverseChainedParameter};

    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    let observation = |id: &str, status: &str, patient: &str| {
        json!({
            "id": id,
            "status": status,
            "code": {"coding": [{"system": LOINC, "code": "1234-5"}]},
            "subject": {"reference": format!("Patient/{patient}")},
        })
    };
    let resources = [
        ("Patient", json!({"id": "pt-f", "gender": "female"})),
        ("Patient", json!({"id": "pt-m", "gender": "male"})),
        ("Observation", observation("ob-f", "final", "pt-f")),
        ("Observation", observation("ob-m", "preliminary", "pt-m")),
    ];
    for (resource_type, resource) in resources {
        let id = resource["id"].as_str().unwrap_or_default().to_string();
        backend
            .create(&tenant, resource_type, resource, FhirVersion::default())
            .await
            .unwrap_or_else(|e| panic!("create {id} failed: {e}"));
    }

    let forward = |value: &str| {
        SearchQuery::new("Observation")
            .with_parameter(SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                values: vec![SearchValue::eq(value)],
                chain: vec![ChainedParameter {
                    reference_param: "subject".to_string(),
                    target_type: Some("Patient".to_string()),
                    target_param: "gender".to_string(),
                }],
                ..Default::default()
            })
            .with_count(100)
    };
    let reverse = |value: &str| {
        let mut query = SearchQuery::new("Patient").with_count(100);
        query.reverse_chains.push(ReverseChainedParameter::terminal(
            "Observation",
            "subject",
            "status",
            SearchValue::eq(value),
        ));
        query
    };
    // (label, query, expected). The unqualified rows are the positive
    // controls, and the first of them is polled for Elasticsearch.
    let cases: Vec<(String, SearchQuery, &[&str])> = vec![
        (
            "subject:Patient.gender=female".into(),
            forward("female"),
            &["ob-f"],
        ),
        (
            "subject:Patient.gender=<system>|female".into(),
            forward(&format!("{GENDER}|female")),
            &["ob-f"],
        ),
        (
            "_has:Observation:subject:status=final".into(),
            reverse("final"),
            &["pt-f"],
        ),
        (
            "_has:Observation:subject:status=<system>|final".into(),
            reverse(&format!("{OBS_STATUS}|final")),
            &["pt-f"],
        ),
    ];

    let mut failures = Vec::new();
    for (index, (label, query, expected)) in cases.iter().enumerate() {
        let resolved = resolve_chains(backend, &tenant, query)
            .await
            .unwrap_or_else(|e| panic!("resolve {label} failed: {e}"));
        let mut got = matched(backend, &tenant, &resolved).await;
        if index == 0 {
            for _ in 0..60 {
                if got == ids(expected) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                got = matched(backend, &tenant, &resolved).await;
            }
            assert_eq!(got, ids(expected), "positive control {label}");
        }
        if got != ids(expected) {
            failures.push(format!("{label}: got {got:?}, expected {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
