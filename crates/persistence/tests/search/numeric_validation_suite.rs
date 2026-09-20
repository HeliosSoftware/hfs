//! Backend-agnostic number / quantity search value suite: one grammar, one
//! answer to a value that is not a number (issues #1319, #1340).
//!
//! The numeric sibling of `date_precision_suite.rs`. Before the shared gate
//! (`helios_persistence::search::validate_numeric_values`) a backend handed
//! `probability=abc` made of it what it liked:
//!
//! - Elasticsearch dropped the constraint — the handler returned `None`, which
//!   the query builder filters out — so the search returned every resource;
//! - PostgreSQL did the same until #1332, and has matched nothing since;
//! - SQLite matched nothing (`1 = 0`);
//! - MongoDB returned an error.
//!
//! And every backend but PostgreSQL read a number with `f64::from_str`, which
//! takes `inf`, `nan` and rounds `1e999` to infinity: `probability=ltinf`
//! matched every indexed row.
//!
//! This suite drives each backend's *real* [`SearchProvider::search`] path —
//! directly, through a composite component, and through the terminal searches
//! the chain resolver issues for a chained parameter and for `_has` — and its
//! `ConditionalStorage` path where it has one, and holds them to the same
//! answers.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `date_precision_suite.rs`. The backend must be built with the spec search
//! parameters loaded: without them nothing here is indexed and every "matches
//! nothing" would pass vacuously — which is why each table opens with positive
//! controls, and the suite refuses to go on until they are searchable.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalStorage, ResourceStorage,
    SearchProvider,
};
use helios_persistence::error::{SearchError, StorageError, StorageResult};
use helios_persistence::search::resolve_chains;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, CompositeSearchComponent, ReverseChainedParameter, SearchParamType,
    SearchParameter, SearchQuery, SearchValue,
};

/// What every invalid value must come back as.
const REJECTED: &str = "InvalidNumberValue";

/// `probability=` value → the seeded RiskAssessments it must match.
const NUMBER_CASES: &[(&str, &[&str])] = &[
    ("0.2", &["ra-low"]),
    ("eq0.2", &["ra-low"]),
    ("gt0.5", &["ra-high"]),
    ("ne0.2", &["ra-high"]),
    ("lt1e3", &["ra-high", "ra-low"]),
    ("gt2.5E-1", &["ra-high"]),
    ("gt-1", &["ra-high", "ra-low"]),
    ("gt1e3", &[]),
    // The leniencies of the shared grammar.
    ("+0.2", &["ra-low"]),
    ("gt.5", &["ra-high"]),
    ("gt0.", &["ra-high", "ra-low"]),
    ("gt000.5", &["ra-high"]),
    // A form-decoded `+` in the exponent.
    ("lt1e 3", &["ra-high", "ra-low"]),
];

/// `value-quantity=` value → the seeded Observations it must match.
const QUANTITY_CASES: &[(&str, &[&str])] = &[
    ("5.4", &["uq-a", "uq-pipe"]),
    ("5.4|http://unitsofmeasure.org|mg", &["uq-a"]),
    ("5.4||mg", &["uq-a"]),
    ("5.4|mg", &["uq-a"]),
    ("gt6", &["uq-b"]),
    ("gt6|http://unitsofmeasure.org|mg", &["uq-b"]),
    ("lt1e3||mg", &["uq-a", "uq-b"]),
    ("gt5.5E0||mg", &["uq-b"]),
    ("gt-1||mg", &["uq-a", "uq-b"]),
    ("+5.4||mg", &["uq-a"]),
    ("gt1e3", &[]),
    // An escaped `|` is part of the code, not a separator.
    ("5.4|http://unitsofmeasure.org|a\\|b", &["uq-pipe"]),
    ("5.4||a\\|b", &["uq-pipe"]),
    ("5.4|a\\|b", &["uq-pipe"]),
];

/// Number parts no backend may interpret, under any prefix. Every one used to
/// be a 200 somewhere.
const INVALID_NUMBERS: &[&str] = &[
    "abc",
    "gtabc",
    "neabc",
    "ltabc",
    "apabc",
    "1e",
    "gt",
    "",
    "0.2abc",
    "0x10",
    "1_000",
    // `f64::from_str` takes these; as bounds they match every row.
    "inf",
    "-inf",
    "ltinf",
    "gt-inf",
    "ltinfinity",
    "nan",
    "NaN",
    "ltnan",
    "neNaN",
    "1e999",
    "lt1e999",
];

/// Quantity values whose number part is missing or not a number.
const INVALID_QUANTITIES: &[&str] = &[
    "abc|http://unitsofmeasure.org|mg",
    "neabc|http://unitsofmeasure.org|mg",
    "gt|http://unitsofmeasure.org|mg",
    "|http://unitsofmeasure.org|mg",
    "||mg",
    "1e||mg",
    "ltinf||mg",
    "nenan||mg",
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
        Err(StorageError::Search(SearchError::InvalidNumberValue { .. })) => REJECTED.to_string(),
        Err(other) => format!("Err({other})"),
    }
}

fn direct(
    resource_type: &str,
    name: &str,
    param_type: SearchParamType,
    value: &str,
) -> SearchQuery {
    SearchQuery::new(resource_type)
        .with_parameter(SearchParameter {
            name: name.to_string(),
            param_type,
            values: vec![SearchValue::parse(value)],
            ..Default::default()
        })
        .with_count(100)
}

fn composite(value: &str) -> SearchQuery {
    SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq(value)],
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

/// `DiagnosticReport?result.value-quantity=<value>`, as the REST layer builds
/// it: the value is raw, because only the resolver knows the terminal's type.
fn chained(value: &str) -> SearchQuery {
    SearchQuery::new("DiagnosticReport")
        .with_parameter(SearchParameter {
            name: "result".to_string(),
            param_type: SearchParamType::Reference,
            values: vec![SearchValue::eq(value)],
            chain: vec![ChainedParameter {
                reference_param: "result".to_string(),
                target_type: Some("Observation".to_string()),
                target_param: "value-quantity".to_string(),
            }],
            ..Default::default()
        })
        .with_count(100)
}

/// `Patient?_has:<source_type>:subject:<param>=<value>`.
fn has(source_type: &str, param: &str, value: &str) -> SearchQuery {
    let mut query = SearchQuery::new("Patient").with_count(100);
    query.reverse_chains = vec![ReverseChainedParameter::terminal(
        source_type,
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

fn observation(id: Option<&str>, quantity: Option<(f64, &str)>) -> Value {
    let mut observation = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [{"system": "http://loinc.org", "code": "8480-6"}]},
        "subject": {"reference": "Patient/nv-patient"},
    });
    if let Some(id) = id {
        observation["id"] = json!(id);
    }
    if let Some((value, unit)) = quantity {
        observation["valueQuantity"] = json!({
            "value": value,
            "unit": unit,
            "system": "http://unitsofmeasure.org",
            "code": unit,
        });
    }
    observation
}

/// Three RiskAssessments (`ra-low` 0.2, `ra-high` 0.8, `ra-none` without a
/// probability), four Observations coded 8480-6 (`uq-a` 5.4 mg, `uq-b` 6.5 mg,
/// `uq-pipe` 5.4 in the unit `a|b`, `uq-none` without a value), the Patient
/// they are all about, and a DiagnosticReport whose result is `uq-a`. The
/// valueless resources are the decoys a dropped constraint returns.
async fn seed<S: ResourceStorage>(backend: &S, tenant: &TenantContext) {
    let version = FhirVersion::default();
    backend
        .create(
            tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "nv-patient", "name": [{"family": "Numeric"}]}),
            version,
        )
        .await
        .expect("seed patient");
    for (id, probability) in [
        ("ra-low", Some(0.2)),
        ("ra-high", Some(0.8)),
        ("ra-none", None),
    ] {
        let mut prediction = json!({"outcome": {"text": "x"}});
        if let Some(p) = probability {
            prediction["probabilityDecimal"] = json!(p);
        }
        backend
            .create(
                tenant,
                "RiskAssessment",
                json!({
                    "resourceType": "RiskAssessment",
                    "id": id,
                    "status": "final",
                    "subject": {"reference": "Patient/nv-patient"},
                    "prediction": [prediction],
                }),
                version,
            )
            .await
            .expect("seed risk assessment");
    }
    for (id, quantity) in [
        ("uq-a", Some((5.4, "mg"))),
        ("uq-b", Some((6.5, "mg"))),
        ("uq-pipe", Some((5.4, "a|b"))),
        ("uq-none", None),
    ] {
        backend
            .create(
                tenant,
                "Observation",
                observation(Some(id), quantity),
                version,
            )
            .await
            .expect("seed observation");
    }
    backend
        .create(
            tenant,
            "DiagnosticReport",
            json!({
                "resourceType": "DiagnosticReport",
                "id": "nv-report",
                "status": "final",
                "code": {"text": "panel"},
                "subject": {"reference": "Patient/nv-patient"},
                "result": [{"reference": "Observation/uq-a"}],
            }),
            version,
        )
        .await
        .expect("seed diagnostic report");
}

/// Seeds the fixture under a caller-unique tenant and asserts the tables over
/// every search path.
pub async fn invalid_numbers_are_rejected_on_every_path<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    use SearchParamType::{Number, Quantity};

    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;

    // Positive controls, and the wait for eventually-consistent indexes: the
    // rows must be findable *by number, by quantity and by reference* before
    // any "matches nothing" below means anything.
    for attempt in 0..60 {
        let visible = [
            outcome(
                backend
                    .search(
                        &tenant,
                        &direct("RiskAssessment", "probability", Number, "ge0"),
                    )
                    .await,
            ) == ids(&["ra-low", "ra-high"]),
            outcome(
                backend
                    .search(
                        &tenant,
                        &direct("Observation", "value-quantity", Quantity, "ge0"),
                    )
                    .await,
            ) == ids(&["uq-a", "uq-b", "uq-pipe"]),
            outcome(backend.search(&tenant, &composite("8480-6$5.4")).await)
                == ids(&["uq-a", "uq-pipe"]),
            resolved(backend, &tenant, &chained("5.4||mg")).await == ids(&["nv-report"]),
        ];
        if visible.iter().all(|seen| *seen) {
            break;
        }
        assert!(
            attempt < 59,
            "the seeded resources never became searchable by number, quantity, composite and \
             chain ({visible:?}): is the backend built with the spec search parameters?"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let mut report = Report::default();

    // ---- direct --------------------------------------------------------------
    for (value, expected) in NUMBER_CASES {
        report.record(
            &format!("probability={value}"),
            &ids(expected),
            outcome(
                backend
                    .search(
                        &tenant,
                        &direct("RiskAssessment", "probability", Number, value),
                    )
                    .await,
            ),
        );
    }
    for (value, expected) in QUANTITY_CASES {
        report.record(
            &format!("value-quantity={value}"),
            &ids(expected),
            outcome(
                backend
                    .search(
                        &tenant,
                        &direct("Observation", "value-quantity", Quantity, value),
                    )
                    .await,
            ),
        );
    }
    for value in INVALID_NUMBERS {
        report.record(
            &format!("probability={value}"),
            REJECTED,
            outcome(
                backend
                    .search(
                        &tenant,
                        &direct("RiskAssessment", "probability", Number, value),
                    )
                    .await,
            ),
        );
    }
    for value in INVALID_NUMBERS.iter().chain(INVALID_QUANTITIES) {
        report.record(
            &format!("value-quantity={value}"),
            REJECTED,
            outcome(
                backend
                    .search(
                        &tenant,
                        &direct("Observation", "value-quantity", Quantity, value),
                    )
                    .await,
            ),
        );
    }

    // ---- composite component -------------------------------------------------
    for (value, expected) in [
        ("8480-6$5.4", &["uq-a", "uq-pipe"][..]),
        ("8480-6$gt6", &["uq-b"][..]),
        (
            "http://loinc.org|8480-6$5.4|http://unitsofmeasure.org|mg",
            &["uq-a"][..],
        ),
        ("9999-9$5.4", &[][..]),
    ] {
        report.record(
            &format!("code-value-quantity={value}"),
            &ids(expected),
            outcome(backend.search(&tenant, &composite(value)).await),
        );
    }
    for value in [
        "8480-6$abc",
        "8480-6$neabc",
        "8480-6$gt",
        "8480-6$",
        "8480-6$ltinf",
        "8480-6$nenan",
        "8480-6$1e999",
        "8480-6$abc|http://unitsofmeasure.org|mg",
        "8480-6$||mg",
    ] {
        report.record(
            &format!("code-value-quantity={value}"),
            REJECTED,
            outcome(backend.search(&tenant, &composite(value)).await),
        );
    }

    // ---- chained terminal and `_has` ----------------------------------------
    // The resolver types and prefix-parses the terminal value, then searches
    // for it through the backend's own gate.
    for (value, expected) in [
        ("5.4||mg", &["nv-report"][..]),
        ("lt6", &["nv-report"][..]),
        ("gt6", &[][..]),
    ] {
        report.record(
            &format!("DiagnosticReport?result.value-quantity={value}"),
            &ids(expected),
            resolved(backend, &tenant, &chained(value)).await,
        );
    }
    for (source, param, value, expected) in [
        (
            "Observation",
            "value-quantity",
            "gt6||mg",
            &["nv-patient"][..],
        ),
        ("Observation", "value-quantity", "gt1e3", &[][..]),
        (
            "RiskAssessment",
            "probability",
            "lt0.5",
            &["nv-patient"][..],
        ),
        ("RiskAssessment", "probability", "gt1", &[][..]),
    ] {
        report.record(
            &format!("Patient?_has:{source}:subject:{param}={value}"),
            &ids(expected),
            resolved(backend, &tenant, &has(source, param, value)).await,
        );
    }
    for value in ["abc", "gtabc", "ltinf", "nenan", "1e999", "", "||mg"] {
        report.record(
            &format!("DiagnosticReport?result.value-quantity={value}"),
            REJECTED,
            resolved(backend, &tenant, &chained(value)).await,
        );
        report.record(
            &format!("Patient?_has:Observation:subject:value-quantity={value}"),
            REJECTED,
            resolved(
                backend,
                &tenant,
                &has("Observation", "value-quantity", value),
            )
            .await,
        );
    }
    for value in ["abc", "ltinf", "neNaN"] {
        report.record(
            &format!("Patient?_has:RiskAssessment:subject:probability={value}"),
            REJECTED,
            resolved(
                backend,
                &tenant,
                &has("RiskAssessment", "probability", value),
            )
            .await,
        );
    }

    // ---- `search_count` is an entry point of its own --------------------------
    report.record(
        "search_count probability=ltinf",
        REJECTED,
        match backend
            .search_count(
                &tenant,
                &direct("RiskAssessment", "probability", Number, "ltinf"),
            )
            .await
        {
            Ok(count) => format!("{count}"),
            Err(StorageError::Search(SearchError::InvalidNumberValue { .. })) => {
                REJECTED.to_string()
            }
            Err(other) => format!("Err({other})"),
        },
    );

    report.finish("numeric search values");
}

/// Conditional criteria are a raw `name=value` string each backend turns into
/// a search of its own: `value-quantity=ltinf` on a conditional delete must
/// not name every Observation, and `value-quantity=abc` on a conditional
/// create must not quietly find nothing and create.
pub async fn invalid_numbers_are_rejected_in_conditional_criteria<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider + ConditionalStorage,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;
    let version = FhirVersion::default();
    let mut report = Report::default();

    let rejected = |e: StorageError| match e {
        StorageError::Search(SearchError::InvalidNumberValue { .. }) => REJECTED.to_string(),
        other => format!("Err({other})"),
    };

    for (criteria, expected) in [
        // Positive controls: the criteria are typed by the registry.
        ("value-quantity=gt6||mg", "Exists(uq-b)"),
        ("value-quantity=5.4", "MultipleMatches(2)"),
        ("value-quantity=abc", REJECTED),
        ("value-quantity=gtabc||mg", REJECTED),
        ("value-quantity=ltinf", REJECTED),
        ("value-quantity=nenan", REJECTED),
        ("value-quantity=||mg", REJECTED),
        ("code-value-quantity=8480-6$abc", REJECTED),
    ] {
        let actual = match backend
            .conditional_create(
                &tenant,
                "Observation",
                observation(None, Some((5.4, "mg"))),
                criteria,
                version,
            )
            .await
        {
            Ok(ConditionalCreateResult::Created(_)) => "Created".to_string(),
            Ok(ConditionalCreateResult::Exists(found)) => format!("Exists({})", found.id()),
            Ok(ConditionalCreateResult::MultipleMatches(n)) => format!("MultipleMatches({n})"),
            Err(e) => rejected(e),
        };
        report.record(&format!("conditional_create {criteria}"), expected, actual);
    }

    for (criteria, expected) in [
        ("probability=ltinf", REJECTED),
        ("probability=neabc", REJECTED),
        ("probability=lt1e999", REJECTED),
        // Positive control last: it removes a row.
        ("probability=gt0.5", "Deleted(ra-high)"),
    ] {
        let actual = match backend
            .conditional_delete(&tenant, "RiskAssessment", criteria)
            .await
        {
            Ok(ConditionalDeleteResult::Deleted(stored)) => format!("Deleted({})", stored.id()),
            Ok(ConditionalDeleteResult::NoMatch) => "NoMatch".to_string(),
            Ok(ConditionalDeleteResult::MultipleMatches(n)) => format!("MultipleMatches({n})"),
            Err(e) => rejected(e),
        };
        report.record(&format!("conditional_delete {criteria}"), expected, actual);
    }
    // Whatever the calls answered, nothing but the control may be gone.
    report.record(
        "RiskAssessments left",
        &ids(&["ra-low", "ra-none"]),
        outcome(
            backend
                .search(&tenant, &SearchQuery::new("RiskAssessment").with_count(100))
                .await,
        ),
    );

    report.finish("numeric conditional criteria");
}
