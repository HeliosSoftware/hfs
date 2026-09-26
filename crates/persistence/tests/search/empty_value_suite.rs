//! Backend-agnostic empty search value suite: a value that is empty, or has an
//! empty alternative in its OR-list, is an error on every backend and every
//! search path — never a search that matches everything (issue #1380).
//!
//! To a backend an empty alternative is the value `""`. Before the shared gate
//! (`helios_persistence::search::validate_value_presence`) all four made the
//! same things of it, and they were wrong in the same way:
//!
//! - a string parameter (`family=Zzz,`, `family=`), which is a prefix match,
//!   returned every resource that has a value, as did `:contains` and `:text`,
//!   and a chained `subject.family=Zzz,` every resource with a subject;
//! - a token's `:not=` returned every resource, its `:text=` every resource
//!   with a display;
//! - `identifier:of-type=` returned every resource on PostgreSQL and
//!   Elasticsearch and none on SQLite, `url:below=` every resource on
//!   PostgreSQL and none elsewhere;
//! - a plain token, reference or uri matched nothing.
//!
//! This suite drives each backend's *real* [`SearchProvider::search`] path —
//! directly, through a composite, and through the terminal searches the chain
//! resolver issues for a chained parameter and for `_has`.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `numeric_validation_suite.rs`. The backend must be built with the spec
//! search parameters loaded: the positive controls refuse to go on until the
//! fixture is searchable by every parameter type used below.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::error::{SearchError, StorageError, StorageResult};
use helios_persistence::search::resolve_chains;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, CompositeSearchComponent, ReverseChainedParameter, SearchModifier,
    SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

/// What every empty value must come back as.
const REJECTED: &str = "EmptyValue";

/// The OR-lists no backend may run, as the REST layer splits them. Each stands
/// for a request: `Zzz,` / `,Zzz` / `Zzz,,Abc` / `` / `,` / `Zzz, ` (the
/// splitter trims) / a whitespace-only value built by hand.
const EMPTY_SHAPES: &[&[&str]] = &[
    &["Zzz", ""],
    &["", "Zzz"],
    &["Zzz", "", "Abc"],
    &[""],
    &["", ""],
    &["  "],
];

/// Collects one line per scenario, so a failing backend reports the whole
/// table rather than its first row.
#[derive(Default)]
struct Report {
    lines: Vec<String>,
    failures: usize,
}

impl Report {
    fn record(&mut self, scenario: &str, expected: &str, actual: String) {
        let ok = expected == actual;
        if !ok {
            self.failures += 1;
        }
        self.lines.push(format!(
            "{} | {scenario} | expected {expected} | actual {actual}",
            if ok { "ok  " } else { "FAIL" }
        ));
    }

    fn finish(self, what: &str) {
        assert!(
            self.failures == 0,
            "{what}: {} scenario(s) failed\n{}",
            self.failures,
            self.lines.join("\n")
        );
    }
}

fn ids(expected: &[&str]) -> String {
    let sorted: BTreeSet<&str> = expected.iter().copied().collect();
    format!("{sorted:?}")
}

/// A search's outcome as one comparable string: the sorted ids it found,
/// [`REJECTED`], or the unexpected error.
fn outcome(result: StorageResult<helios_persistence::core::SearchResult>) -> String {
    match result {
        Ok(found) => {
            let sorted: BTreeSet<&str> = found.resources.items.iter().map(|r| r.id()).collect();
            format!("{sorted:?}")
        }
        Err(StorageError::Search(SearchError::EmptyValue { .. })) => REJECTED.to_string(),
        Err(other) => format!("Err({other})"),
    }
}

fn direct(
    resource_type: &str,
    name: &str,
    param_type: SearchParamType,
    modifier: Option<SearchModifier>,
    values: &[&str],
) -> SearchQuery {
    SearchQuery::new(resource_type)
        .with_parameter(SearchParameter {
            name: name.to_string(),
            param_type,
            modifier,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            ..Default::default()
        })
        .with_count(100)
}

fn composite(values: &[&str]) -> SearchQuery {
    SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value-quantity".to_string(),
                },
            ],
            ..Default::default()
        })
        .with_count(100)
}

/// `Observation?subject:Patient.family=<values>`, as the REST layer builds it:
/// split into alternatives, but raw — only the resolver knows the terminal's
/// type.
fn chained(values: &[&str]) -> SearchQuery {
    SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            chain: vec![ChainedParameter {
                reference_param: "subject".to_string(),
                target_type: Some("Patient".to_string()),
                target_param: "family".to_string(),
            }],
            ..Default::default()
        })
        .with_count(100)
}

/// `Patient?_has:Observation:subject:<param>=<value>`; the value is unsplit.
fn has(param: &str, value: &str) -> SearchQuery {
    let mut query = SearchQuery::new("Patient").with_count(100);
    query.reverse_chains = vec![ReverseChainedParameter::terminal(
        "Observation",
        "subject",
        param,
        SearchValue::eq(value),
    )];
    query
}

/// Resolves the query's chains — each hop is a search of its own, through the
/// backend's gate — and runs what is left.
async fn resolved<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> String
where
    S: ResourceStorage + SearchProvider,
{
    match resolve_chains(backend, tenant, query).await {
        Ok(rewritten) => outcome(backend.search(tenant, &rewritten).await),
        Err(e) => outcome(Err(e)),
    }
}

/// Four Patients — `ev-zzz` (family Zzz, female, identifier `111` typed MR, a
/// general practitioner), `ev-abc` (family "Abc, Jr", male), `ev-comma`
/// (family `Comma, J`) and `ev-bare`, which has none of it — the Practitioner,
/// two Observations about the first two, and two ValueSets. With a filter
/// dropped or matching `""`, a search returns the decoys.
async fn seed<S: ResourceStorage>(backend: &S, tenant: &TenantContext) {
    let version = FhirVersion::default();
    let resources = [
        json!({"resourceType": "Practitioner", "id": "ev-doc", "name": [{"family": "Doc"}]}),
        json!({
            "resourceType": "Patient",
            "id": "ev-zzz",
            "identifier": [{
                "system": "http://example.org/mrn",
                "value": "111",
                "type": {"coding": [{
                    "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                    "code": "MR",
                }]},
            }],
            "name": [{"family": "Zzz"}],
            "gender": "female",
            "generalPractitioner": [{"reference": "Practitioner/ev-doc"}],
        }),
        json!({
            "resourceType": "Patient",
            "id": "ev-abc",
            "name": [{"family": "Abc, Jr"}],
            "gender": "male",
        }),
        json!({"resourceType": "Patient", "id": "ev-comma", "name": [{"family": "Comma, J"}]}),
        json!({"resourceType": "Patient", "id": "ev-bare"}),
        json!({
            "resourceType": "Observation",
            "id": "ev-obs-a",
            "status": "final",
            "code": {"coding": [{"system": "http://loinc.org", "code": "8480-6"}], "text": "alpha"},
            "subject": {"reference": "Patient/ev-zzz"},
            "valueQuantity": {
                "value": 5.4,
                "unit": "mg",
                "system": "http://unitsofmeasure.org",
                "code": "mg",
            },
        }),
        json!({
            "resourceType": "Observation",
            "id": "ev-obs-b",
            "status": "final",
            "code": {"coding": [{"system": "http://loinc.org", "code": "9999-9"}]},
            "subject": {"reference": "Patient/ev-abc"},
        }),
        json!({
            "resourceType": "ValueSet",
            "id": "ev-vs-one",
            "url": "http://example.org/vs/one",
            "status": "active",
        }),
        json!({
            "resourceType": "ValueSet",
            "id": "ev-vs-two",
            "url": "http://example.org/vs/two",
            "status": "active",
        }),
    ];
    for resource in resources {
        let resource_type = resource["resourceType"]
            .as_str()
            .expect("typed")
            .to_string();
        backend
            .create(tenant, &resource_type, resource, version)
            .await
            .expect("seed resource");
    }
}

/// Seeds the fixture under a caller-unique tenant and asserts that every empty
/// shape is rejected on every search path, next to controls that match.
pub async fn empty_values_are_rejected_on_every_path<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    use SearchModifier as M;
    use SearchParamType::{Reference, String as Str, Token, Uri};

    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;

    // Positive controls, and the wait for eventually-consistent indexes: the
    // fixture must be findable by string, token, reference, uri, composite,
    // chain and `_has` before any rejection below means the *value* was the
    // reason.
    let controls: Vec<(&str, SearchQuery, &[&str])> = vec![
        (
            "family=Zzz",
            direct("Patient", "family", Str, None, &["Zzz"]),
            &["ev-zzz"],
        ),
        (
            "family=Zzz,Abc",
            direct("Patient", "family", Str, None, &["Zzz", "Abc"]),
            &["ev-abc", "ev-zzz"],
        ),
        // `family=Comma\, J`: an escaped comma is data, not a separator.
        (
            "family=Comma\\, J",
            direct("Patient", "family", Str, None, &["Comma, J"]),
            &["ev-comma"],
        ),
        (
            "family:exact=Zzz",
            direct("Patient", "family", Str, Some(M::Exact), &["Zzz"]),
            &["ev-zzz"],
        ),
        (
            "family:contains=zz",
            direct("Patient", "family", Str, Some(M::Contains), &["zz"]),
            &["ev-zzz"],
        ),
        (
            "gender=female",
            direct("Patient", "gender", Token, None, &["female"]),
            &["ev-zzz"],
        ),
        (
            "gender:not=female",
            direct("Patient", "gender", Token, Some(M::Not), &["female"]),
            &["ev-abc", "ev-bare", "ev-comma"],
        ),
        (
            "identifier=111",
            direct("Patient", "identifier", Token, None, &["111"]),
            &["ev-zzz"],
        ),
        (
            "general-practitioner=Practitioner/ev-doc",
            direct(
                "Patient",
                "general-practitioner",
                Reference,
                None,
                &["Practitioner/ev-doc"],
            ),
            &["ev-zzz"],
        ),
        (
            "url=http://example.org/vs/one",
            direct("ValueSet", "url", Uri, None, &["http://example.org/vs/one"]),
            &["ev-vs-one"],
        ),
        (
            "url:below=http://example.org/vs",
            direct(
                "ValueSet",
                "url",
                Uri,
                Some(M::Below),
                &["http://example.org/vs"],
            ),
            &["ev-vs-one", "ev-vs-two"],
        ),
        // `:missing` takes a boolean, and is none of this suite's business.
        (
            "family:missing=true",
            direct("Patient", "family", Str, Some(M::Missing), &["true"]),
            &["ev-bare"],
        ),
        (
            "code-value-quantity=8480-6$5.4",
            composite(&["8480-6$5.4"]),
            &["ev-obs-a"],
        ),
        (
            "subject:Patient.family=Zzz",
            chained(&["Zzz"]),
            &["ev-obs-a"],
        ),
        (
            "_has:Observation:subject:code=8480-6",
            has("code", "8480-6"),
            &["ev-zzz"],
        ),
        (
            "_has:Observation:subject:code=8480-6,9999-9",
            has("code", "8480-6,9999-9"),
            &["ev-abc", "ev-zzz"],
        ),
    ];
    for attempt in 0..60 {
        let mut pending = Vec::new();
        for (scenario, query, expected) in &controls {
            let actual = resolved(backend, &tenant, query).await;
            if actual != ids(expected) {
                pending.push(format!(
                    "{scenario}: expected {}, got {actual}",
                    ids(expected)
                ));
            }
        }
        if pending.is_empty() {
            break;
        }
        assert!(
            attempt < 59,
            "the positive controls never held — is the backend built with the spec search \
             parameters?\n{}",
            pending.join("\n")
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let mut report = Report::default();

    // ---- direct: every type, every modifier that takes a value ---------------
    let direct_params: &[(&str, &str, SearchParamType, Option<SearchModifier>)] = &[
        ("Patient", "family", Str, None),
        ("Patient", "family", Str, Some(M::Exact)),
        ("Patient", "family", Str, Some(M::Contains)),
        ("Patient", "family", Str, Some(M::Text)),
        ("Patient", "gender", Token, None),
        ("Patient", "gender", Token, Some(M::Not)),
        ("Patient", "gender", Token, Some(M::Text)),
        ("Patient", "identifier", Token, Some(M::OfType)),
        ("Patient", "_id", Token, None),
        ("Patient", "_tag", Token, None),
        ("Patient", "general-practitioner", Reference, None),
        (
            "Patient",
            "general-practitioner",
            Reference,
            Some(M::Identifier),
        ),
        ("ValueSet", "url", Uri, None),
        ("ValueSet", "url", Uri, Some(M::Below)),
        ("ValueSet", "url", Uri, Some(M::Above)),
        ("ValueSet", "url", Uri, Some(M::Contains)),
    ];
    for (resource_type, name, param_type, modifier) in direct_params {
        for shape in EMPTY_SHAPES {
            let suffix = modifier
                .as_ref()
                .map(|m| format!(":{m}"))
                .unwrap_or_default();
            report.record(
                &format!("{resource_type}?{name}{suffix}={shape:?}"),
                REJECTED,
                outcome(
                    backend
                        .search(
                            &tenant,
                            &direct(resource_type, name, *param_type, modifier.clone(), shape),
                        )
                        .await,
                ),
            );
        }
    }
    // A string that folds to nothing — a lone combining acute accent — is as
    // empty as `""` to the folded prefix match; `:exact` compares it raw.
    report.record(
        "Patient?family=U+0301",
        REJECTED,
        outcome(
            backend
                .search(
                    &tenant,
                    &direct("Patient", "family", Str, None, &["\u{301}"]),
                )
                .await,
        ),
    );
    report.record(
        "Patient?family:exact=U+0301",
        &ids(&[]),
        outcome(
            backend
                .search(
                    &tenant,
                    &direct("Patient", "family", Str, Some(M::Exact), &["\u{301}"]),
                )
                .await,
        ),
    );

    // ---- composite: an empty alternative, an empty component -----------------
    for shape in [
        &["8480-6$5.4", ""][..],
        &[""][..],
        &["$5.4"][..],
        &[" $5.4"][..],
    ] {
        report.record(
            &format!("Observation?code-value-quantity={shape:?}"),
            REJECTED,
            outcome(backend.search(&tenant, &composite(shape)).await),
        );
    }

    // ---- chained and `_has` terminals ------------------------------------------
    for shape in EMPTY_SHAPES {
        report.record(
            &format!("Observation?subject:Patient.family={shape:?}"),
            REJECTED,
            resolved(backend, &tenant, &chained(shape)).await,
        );
    }
    for value in ["8480-6,", ",8480-6", "8480-6,,9999-9", "", ",", " "] {
        report.record(
            &format!("Patient?_has:Observation:subject:code={value:?}"),
            REJECTED,
            resolved(backend, &tenant, &has("code", value)).await,
        );
        report.record(
            &format!("Patient?_has:Observation:subject:code:text={value:?}"),
            REJECTED,
            resolved(backend, &tenant, &has("code:text", value)).await,
        );
    }

    report.finish("empty search values");
}
