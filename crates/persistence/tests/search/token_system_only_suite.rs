//! Backend-agnostic suite for `[system]|` token searches on an Identifier
//! with a system and no value, and a Coding with a system and no code
//! (issue #2965).
//!
//! FHIR defines `[parameter]=[system]|` as "any element where the value of
//! [system] matches the system property of the Identifier or Coding"
//! (search.html#token, the same sentence in R4, R4B and R5), and both
//! `Identifier.value` and `Coding.code` are optional. The token converter
//! used to index an Identifier only when it had a value and a Coding only
//! when it had a code, so such an element left no row: `system|` missed it,
//! and so did `:missing=false`. It now writes a row with the system and an
//! empty code. No `code` or `|code` search can match an empty code, so the
//! rest of the table — the controls — reads exactly as before.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `token_code_system_suite.rs`. The backend must be built with the spec
//! search parameters loaded; the suite's positive controls fail loudly if it
//! was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

use crate::seeding::SeedingStorage;

const MRN: &str = "http://hospital.example/mrn";
const OTHER: &str = "http://other.example/id";
const LOINC: &str = "http://loinc.org";
const TAGS: &str = "http://example.org/tags";

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
/// - `pt-sysonly`: an Identifier with ONLY a system, `MRN`.
/// - `pt-sysvalue`: an Identifier of `MRN` with value `123` — the control.
/// - `pt-other`: an Identifier of another system, value `123`.
/// - `pt-none`: no identifier at all.
/// - `ob-sysonly`: `code` is a CodeableConcept whose only Coding has ONLY a
///   system, LOINC.
/// - `ob-coded`: `code` is LOINC `1234-5` — the control.
/// - `ob-tag-sysonly` / `ob-tag-empty` / `ob-tag-coded`: a bare `meta.tag`
///   Coding of `TAGS` with no code, with `"code": ""` (what `$meta-add`
///   stores), and with code `t1`. None of them has a `code`.
fn cases() -> Vec<Case> {
    let none = None;
    let not = Some(SearchModifier::Not);
    let missing = Some(SearchModifier::Missing);
    vec![
        // Identifier.
        (
            "Patient",
            "identifier",
            none.clone(),
            format!("{MRN}|"),
            &["pt-sysonly", "pt-sysvalue"],
        ),
        (
            "Patient",
            "identifier",
            none.clone(),
            format!("{MRN}|123"),
            &["pt-sysvalue"],
        ),
        (
            "Patient",
            "identifier",
            none.clone(),
            "123".into(),
            &["pt-other", "pt-sysvalue"],
        ),
        (
            "Patient",
            "identifier",
            none.clone(),
            format!("{OTHER}|"),
            &["pt-other"],
        ),
        // `|code` means "no system": the system-only row has a system.
        ("Patient", "identifier", none.clone(), "|123".into(), &[]),
        (
            "Patient",
            "identifier",
            not.clone(),
            format!("{MRN}|123"),
            &["pt-none", "pt-other", "pt-sysonly"],
        ),
        (
            "Patient",
            "identifier",
            not.clone(),
            format!("{MRN}|"),
            &["pt-none", "pt-other"],
        ),
        // `pt-sysonly` has an identifier.
        (
            "Patient",
            "identifier",
            missing.clone(),
            "false".into(),
            &["pt-other", "pt-sysonly", "pt-sysvalue"],
        ),
        (
            "Patient",
            "identifier",
            missing.clone(),
            "true".into(),
            &["pt-none"],
        ),
        // Coding inside a CodeableConcept.
        (
            "Observation",
            "code",
            none.clone(),
            format!("{LOINC}|"),
            &["ob-coded", "ob-sysonly"],
        ),
        (
            "Observation",
            "code",
            none.clone(),
            format!("{LOINC}|1234-5"),
            &["ob-coded"],
        ),
        (
            "Observation",
            "code",
            missing.clone(),
            "false".into(),
            &["ob-coded", "ob-sysonly"],
        ),
        // A bare Coding, `meta.tag`.
        (
            "Observation",
            "_tag",
            none.clone(),
            format!("{TAGS}|"),
            &["ob-tag-coded", "ob-tag-empty", "ob-tag-sysonly"],
        ),
        (
            "Observation",
            "_tag",
            none.clone(),
            format!("{TAGS}|t1"),
            &["ob-tag-coded"],
        ),
    ]
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
            values: vec![SearchValue::eq(value)],
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
pub async fn system_only_tokens_match_system_pipe<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    let tagged = |id: &str, tag: serde_json::Value| json!({"id": id, "status": "final", "meta": {"tag": [tag]}});
    let resources = [
        (
            "Patient",
            json!({"id": "pt-sysonly", "identifier": [{"system": MRN}]}),
        ),
        (
            "Patient",
            json!({"id": "pt-sysvalue", "identifier": [{"system": MRN, "value": "123"}]}),
        ),
        (
            "Patient",
            json!({"id": "pt-other", "identifier": [{"system": OTHER, "value": "123"}]}),
        ),
        ("Patient", json!({"id": "pt-none", "active": true})),
        (
            "Observation",
            json!({
                "id": "ob-sysonly",
                "status": "final",
                "code": {"coding": [{"system": LOINC}]},
            }),
        ),
        (
            "Observation",
            json!({
                "id": "ob-coded",
                "status": "final",
                "code": {"coding": [{"system": LOINC, "code": "1234-5"}]},
            }),
        ),
        (
            "Observation",
            tagged("ob-tag-sysonly", json!({"system": TAGS})),
        ),
        (
            "Observation",
            tagged("ob-tag-empty", json!({"system": TAGS, "code": ""})),
        ),
        (
            "Observation",
            tagged("ob-tag-coded", json!({"system": TAGS, "code": "t1"})),
        ),
    ];
    for (resource_type, resource) in resources {
        let id = resource["id"].as_str().unwrap_or_default().to_string();
        backend
            .seed_create(&tenant, resource_type, resource, FhirVersion::default())
            .await
            .unwrap_or_else(|e| panic!("create {id} failed: {e}"));
    }

    // Positive controls, polled because Elasticsearch is near-real-time. A
    // failure here means the parameter did not index — a backend built without
    // the spec search parameters — not that the fix is wrong.
    let tag_control = format!("{TAGS}|t1");
    for (resource_type, param, value, expected) in [
        (
            "Patient",
            "identifier",
            "123",
            &["pt-other", "pt-sysvalue"][..],
        ),
        ("Observation", "code", "1234-5", &["ob-coded"][..]),
        (
            "Observation",
            "_tag",
            tag_control.as_str(),
            &["ob-tag-coded"][..],
        ),
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

    let mut failures = Vec::new();
    for (resource_type, param, modifier, value, expected) in cases() {
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
