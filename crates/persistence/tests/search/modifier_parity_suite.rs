//! Backend-agnostic modifier parity suite (issue #1408).
//!
//! MongoDB answered `identifier:of-type=…`, `subject:identifier=…` and
//! `subject:Patient=…` with "unsupported modifier" while SQLite, PostgreSQL
//! and Elasticsearch served them, and nothing noticed: every backend had its
//! own modifier tests, none shared. This suite is the shared one. It walks
//! every modifier [`SearchModifier::is_valid_for`] allows on each parameter
//! type over one seeded data set and states, per cell, the ids that must
//! match.
//!
//! A backend that really does differ passes its differences in as
//! [`Divergence`]s, each keyed by the cell's label (`Type?param:modifier=value`)
//! with what that backend returns instead. A divergence that stops being one
//! fails the suite too, so the list cannot rot, and an unknown label is an
//! error. Anything not listed must agree with the table.
//!
//! Semantics worth knowing before reading the table:
//!
//! - `:identifier` on a reference is implemented by SQLite, PostgreSQL and
//!   MongoDB as "the reference's *target* has this identifier" (a join on the
//!   target's `identifier` rows). `Reference.identifier` itself — a logical
//!   reference — is not indexed by the shared extractor, so `ob-logical` is
//!   matched by no backend.
//! - `:[type]` with a bare id is `Type/id`, version-agnostic, and never the
//!   same id under another type (`Group/p1`).
//! - `:in` / `:not-in` need a terminology server and `:above` / `:below` on a
//!   token need subsumption; neither exists at this layer. The cells are here
//!   so that what each backend does with them is on record.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `token_code_system_suite.rs`. The backend must be built with the spec
//! search parameters loaded; the positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::error::{SearchError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

const V2_0203: &str = "http://terminology.hl7.org/CodeSystem/v2-0203";
const MRN: &str = "http://example.org/mrn";
const SSN: &str = "http://example.org/ssn";
const LOINC: &str = "http://loinc.org";
const VS: &str = "http://example.org/fhir/ValueSet/a";
const ABS_PATIENT: &str = "http://example.org/fhir/Patient/p1";

/// What a cell must produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// The search succeeds and returns exactly these ids.
    Ids(&'static [&'static str]),
    /// The search is refused as an unsupported modifier.
    Rejected,
}

/// A cell on which one backend knowingly differs from the table.
pub struct Divergence {
    /// The cell's label, `Type?param:modifier=value`.
    pub label: &'static str,
    /// What this backend produces instead.
    pub expect: Expect,
}

struct Case {
    resource_type: &'static str,
    param: &'static str,
    param_type: SearchParamType,
    modifier: SearchModifier,
    value: String,
    /// A second, unmodified token parameter ANDed with the first, so the
    /// modified one is also exercised where it does not drive the search.
    and: Option<(&'static str, &'static str)>,
    expect: Expect,
}

impl Case {
    fn label(&self) -> String {
        let and = self
            .and
            .map(|(param, value)| format!("&{param}={value}"))
            .unwrap_or_default();
        format!(
            "{}?{}:{}={}{and}",
            self.resource_type, self.param, self.modifier, self.value
        )
    }

    fn and(mut self, param: &'static str, value: &'static str) -> Self {
        self.and = Some((param, value));
        self
    }
}

fn case(
    resource_type: &'static str,
    param: &'static str,
    param_type: SearchParamType,
    modifier: SearchModifier,
    value: impl Into<String>,
    expect: Expect,
) -> Case {
    Case {
        resource_type,
        param,
        param_type,
        modifier,
        value: value.into(),
        and: None,
        expect,
    }
}

/// The matrix. See [`seed`] for the resources the ids name.
fn cases() -> Vec<Case> {
    use Expect::{Ids, Rejected};
    use SearchModifier as M;
    use SearchParamType as T;

    let patient = || M::Type("Patient".to_string());
    let group = || M::Type("Group".to_string());

    vec![
        // ---- token ---------------------------------------------------
        // `:of-type` is `type-system|type-code|identifier-value`.
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            format!("{V2_0203}|MR|12345"),
            Ids(&["p1"]),
        ),
        // Same value, other type: the type really is compared.
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            format!("{V2_0203}|SS|12345"),
            Ids(&["p2"]),
        ),
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            format!("{V2_0203}|MR|99999"),
            Ids(&[]),
        ),
        // An empty part is not compared.
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            "|MR|12345",
            Ids(&["p1"]),
        ),
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            format!("{V2_0203}||12345"),
            Ids(&["p1", "p2"]),
        ),
        // The grammar has three parts; fewer name nothing.
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            "MR|12345",
            Ids(&[]),
        ),
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            "12345",
            Ids(&[]),
        ),
        // An OR-list of `:of-type` values.
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            format!("{V2_0203}|MR|12345,{V2_0203}|SS|12345"),
            Ids(&["p1", "p2"]),
        ),
        case(
            "Observation",
            "code",
            T::Token,
            M::Text,
            "heart",
            Ids(&["ob-pat"]),
        ),
        case(
            "Observation",
            "code",
            T::Token,
            M::CodeText,
            "heart",
            Ids(&["ob-pat"]),
        ),
        // `:code-text` is starts-with, `:text` is not.
        case(
            "Observation",
            "code",
            T::Token,
            M::CodeText,
            "rate",
            Ids(&[]),
        ),
        case(
            "Observation",
            "code",
            T::Token,
            M::Not,
            format!("{LOINC}|1234-5"),
            Ids(&["ob-abs", "ob-grp", "ob-logical", "ob-nosubj", "ob-ver"]),
        ),
        case(
            "Observation",
            "code",
            T::Token,
            M::Missing,
            "true",
            Ids(&["ob-nosubj"]),
        ),
        case(
            "Observation",
            "code",
            T::Token,
            M::Missing,
            "false",
            Ids(&["ob-abs", "ob-grp", "ob-logical", "ob-pat", "ob-ver"]),
        ),
        // Terminology-backed: not available at this layer.
        case(
            "Observation",
            "code",
            T::Token,
            M::In,
            "http://example.org/fhir/ValueSet/a",
            Rejected,
        ),
        case(
            "Observation",
            "code",
            T::Token,
            M::NotIn,
            "http://example.org/fhir/ValueSet/a",
            Rejected,
        ),
        case(
            "Observation",
            "code",
            T::Token,
            M::Above,
            format!("{LOINC}|1234-5"),
            Rejected,
        ),
        case(
            "Observation",
            "code",
            T::Token,
            M::Below,
            format!("{LOINC}|1234-5"),
            Rejected,
        ),
        // ---- string --------------------------------------------------
        case(
            "Patient",
            "family",
            T::String,
            M::Exact,
            "Smith",
            Ids(&["p1"]),
        ),
        case(
            "Patient",
            "family",
            T::String,
            M::Exact,
            "smith",
            Ids(&["p3"]),
        ),
        case(
            "Patient",
            "family",
            T::String,
            M::Contains,
            "MIT",
            Ids(&["p1", "p2", "p3"]),
        ),
        case(
            "Patient",
            "family",
            T::String,
            M::Contains,
            "thso",
            Ids(&["p2"]),
        ),
        case(
            "Patient",
            "family",
            T::String,
            M::Text,
            "thso",
            Ids(&["p2"]),
        ),
        case(
            "Patient",
            "family",
            T::String,
            M::Missing,
            "true",
            Ids(&["p4"]),
        ),
        case(
            "Patient",
            "family",
            T::String,
            M::Missing,
            "false",
            Ids(&["p1", "p2", "p3"]),
        ),
        // ---- reference -----------------------------------------------
        // `subject:Patient=p1` is `subject=Patient/p1`: not `Group/p1`.
        case(
            "Observation",
            "subject",
            T::Reference,
            patient(),
            "p1",
            Ids(&["ob-pat", "ob-ver"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            group(),
            "p1",
            Ids(&["ob-grp"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            patient(),
            "Patient/p1",
            Ids(&["ob-pat", "ob-ver"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            patient(),
            "Patient/p1/_history/2",
            Ids(&["ob-pat", "ob-ver"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            patient(),
            "nobody",
            Ids(&[]),
        ),
        // The modifier and the value disagree on the type: nothing can be
        // both.
        case(
            "Observation",
            "subject",
            T::Reference,
            patient(),
            "Group/p1",
            Ids(&[]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            patient(),
            ABS_PATIENT,
            Ids(&["ob-abs"]),
        ),
        // Two ids under one type modifier.
        case(
            "Observation",
            "subject",
            T::Reference,
            patient(),
            "p1,nobody",
            Ids(&["ob-pat", "ob-ver"]),
        ),
        // `:identifier`: the reference's target has the identifier.
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Identifier,
            format!("{MRN}|12345"),
            Ids(&["ob-pat", "ob-ver"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Identifier,
            format!("{MRN}|"),
            Ids(&["ob-pat", "ob-ver"]),
        ),
        // Value alone: `p1` (a Patient) and the Group `p1` both carry 12345.
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Identifier,
            "12345",
            Ids(&["ob-grp", "ob-pat", "ob-ver"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Identifier,
            format!("{MRN}|99999"),
            Ids(&[]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Missing,
            "true",
            Ids(&["ob-logical", "ob-nosubj"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Missing,
            "false",
            Ids(&["ob-abs", "ob-grp", "ob-pat", "ob-ver"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Contains,
            "example.org",
            Ids(&["ob-abs"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Text,
            "smith",
            Ids(&["ob-pat"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::CodeText,
            "john",
            Ids(&["ob-pat"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Below,
            "http://example.org/fhir/Patient",
            Ids(&["ob-abs"]),
        ),
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Above,
            format!("{ABS_PATIENT}/_history/1"),
            Ids(&["ob-abs"]),
        ),
        // Another tenant's Patient `p1` carries MRN 77777; this tenant's does
        // not, and its references must not match on the strength of it.
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Identifier,
            format!("{MRN}|77777"),
            Ids(&[]),
        ),
        // ANDed with a plain parameter, in both directions of selectivity.
        case(
            "Observation",
            "subject",
            T::Reference,
            M::Identifier,
            format!("{MRN}|12345"),
            Ids(&["ob-pat"]),
        )
        .and("code", "1234-5"),
        case(
            "Observation",
            "subject",
            T::Reference,
            patient(),
            "p1",
            Ids(&["ob-ver"]),
        )
        .and("code", "9999-9"),
        case(
            "Patient",
            "identifier",
            T::Token,
            M::OfType,
            format!("{V2_0203}||12345"),
            Ids(&["p2"]),
        )
        .and("family", "smithson"),
        // ---- uri -----------------------------------------------------
        case(
            "ValueSet",
            "url",
            T::Uri,
            M::Below,
            VS,
            Ids(&["vs-a", "vs-sub"]),
        ),
        case(
            "ValueSet",
            "url",
            T::Uri,
            M::Above,
            format!("{VS}/sub"),
            Ids(&["vs-a", "vs-sub"]),
        ),
        case(
            "ValueSet",
            "url",
            T::Uri,
            M::Contains,
            "ValueSet/a/s",
            Ids(&["vs-sub"]),
        ),
        case(
            "ValueSet",
            "url",
            T::Uri,
            M::Missing,
            "true",
            Ids(&["vs-none"]),
        ),
        case(
            "ValueSet",
            "url",
            T::Uri,
            M::Missing,
            "false",
            Ids(&["vs-a", "vs-sub"]),
        ),
        // ---- date / number / quantity ----------------------------------
        case(
            "Patient",
            "birthdate",
            T::Date,
            M::Missing,
            "true",
            Ids(&["p2", "p3", "p4"]),
        ),
        case(
            "Patient",
            "birthdate",
            T::Date,
            M::Missing,
            "false",
            Ids(&["p1"]),
        ),
        case(
            "RiskAssessment",
            "probability",
            T::Number,
            M::Missing,
            "true",
            Ids(&["ra-none"]),
        ),
        case(
            "RiskAssessment",
            "probability",
            T::Number,
            M::Missing,
            "false",
            Ids(&["ra-half"]),
        ),
        case(
            "Observation",
            "value-quantity",
            T::Quantity,
            M::Missing,
            "false",
            Ids(&["ob-pat"]),
        ),
        case(
            "Observation",
            "value-quantity",
            T::Quantity,
            M::Missing,
            "true",
            Ids(&["ob-abs", "ob-grp", "ob-logical", "ob-nosubj", "ob-ver"]),
        ),
    ]
}

fn query(
    resource_type: &str,
    param: &str,
    param_type: SearchParamType,
    modifier: Option<SearchModifier>,
    value: &str,
) -> SearchQuery {
    SearchQuery::new(resource_type)
        .with_parameter(SearchParameter {
            name: param.to_string(),
            param_type,
            modifier,
            // Comma-separated values are an OR-list, as the REST layer parses them.
            values: value.split(',').map(SearchValue::eq).collect(),
            ..Default::default()
        })
        .with_count(100)
}

/// The query of one cell: the modified parameter, plus its companion if any.
fn case_query(case: &Case) -> SearchQuery {
    let query = query(
        case.resource_type,
        case.param,
        case.param_type,
        Some(case.modifier.clone()),
        &case.value,
    );
    match case.and {
        Some((param, value)) => query.with_parameter(SearchParameter {
            name: param.to_string(),
            // `family` is the one string companion; the rest are tokens.
            param_type: if param == "family" {
                SearchParamType::String
            } else {
                SearchParamType::Token
            },
            modifier: None,
            values: vec![SearchValue::eq(value)],
            ..Default::default()
        }),
        None => query,
    }
}

/// Runs one query and reduces the result to what the table compares.
async fn outcome<S>(
    backend: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> Result<BTreeSet<String>, String>
where
    S: ResourceStorage + SearchProvider,
{
    match backend.search(tenant, query).await {
        Ok(result) => Ok(result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect()),
        Err(StorageError::Search(SearchError::UnsupportedModifier { .. })) => {
            Err("REJECTED".to_string())
        }
        Err(other) => Err(format!("ERROR {other}")),
    }
}

fn render(outcome: &Result<BTreeSet<String>, String>) -> String {
    match outcome {
        Ok(ids) => format!("{:?}", ids.iter().collect::<Vec<_>>()),
        Err(e) => e.clone(),
    }
}

fn render_expect(expect: &Expect) -> String {
    match expect {
        Expect::Ids(ids) => {
            let sorted: BTreeSet<&str> = ids.iter().copied().collect();
            format!("{:?}", sorted.iter().collect::<Vec<_>>())
        }
        Expect::Rejected => "REJECTED".to_string(),
    }
}

/// The seeded resources:
///
/// - Patients: `p1` Smith, born 1980, MRN 12345 typed `MR`; `p2` Smithson,
///   SSN 12345 typed `SS`; `p3` smith (lower case), no identifier; `p4` has
///   neither name nor identifier.
/// - Group `p1`: the same id as the Patient, with an untyped identifier
///   `http://example.org/grp|12345`.
/// - Observations, by `subject`: `ob-pat` `Patient/p1` (display "John Smith",
///   coded LOINC 1234-5 "Heart rate", 5 mg); `ob-ver` `Patient/p1/_history/2`;
///   `ob-abs` the absolute URL; `ob-grp` `Group/p1`; `ob-logical` only a
///   `Reference.identifier`; `ob-nosubj` neither subject nor code.
/// - ValueSets `vs-a`, `vs-sub` (a URL below `vs-a`'s), `vs-none` (no URL).
/// - RiskAssessments `ra-half` (probability 0.5) and `ra-none`.
async fn seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    let other_code = json!({"coding": [{"system": LOINC, "code": "9999-9"}]});
    let resources = [
        (
            "Patient",
            json!({
                "id": "p1",
                "identifier": [{
                    "type": {"coding": [{"system": V2_0203, "code": "MR"}]},
                    "system": MRN,
                    "value": "12345",
                }],
                "name": [{"family": "Smith", "given": ["John"]}],
                "birthDate": "1980-01-01",
            }),
        ),
        (
            "Patient",
            json!({
                "id": "p2",
                "identifier": [{
                    "type": {"coding": [{"system": V2_0203, "code": "SS"}]},
                    "system": SSN,
                    "value": "12345",
                }],
                "name": [{"family": "Smithson"}],
            }),
        ),
        (
            "Patient",
            json!({"id": "p3", "name": [{"family": "smith"}]}),
        ),
        ("Patient", json!({"id": "p4", "active": true})),
        (
            "Group",
            json!({
                "id": "p1",
                "type": "person",
                "actual": true,
                "identifier": [{"system": "http://example.org/grp", "value": "12345"}],
            }),
        ),
        (
            "Observation",
            json!({
                "id": "ob-pat",
                "status": "final",
                "code": {"coding": [{"system": LOINC, "code": "1234-5", "display": "Heart rate"}]},
                "subject": {"reference": "Patient/p1", "display": "John Smith"},
                "valueQuantity": {
                    "value": 5, "unit": "mg",
                    "system": "http://unitsofmeasure.org", "code": "mg",
                },
            }),
        ),
        (
            "Observation",
            json!({
                "id": "ob-ver",
                "status": "final",
                "code": other_code.clone(),
                "subject": {"reference": "Patient/p1/_history/2"},
            }),
        ),
        (
            "Observation",
            json!({
                "id": "ob-abs",
                "status": "final",
                "code": other_code.clone(),
                "subject": {"reference": ABS_PATIENT},
            }),
        ),
        (
            "Observation",
            json!({
                "id": "ob-grp",
                "status": "final",
                "code": other_code.clone(),
                "subject": {"reference": "Group/p1"},
            }),
        ),
        (
            "Observation",
            json!({
                "id": "ob-logical",
                "status": "final",
                "code": other_code.clone(),
                "subject": {"identifier": {"system": MRN, "value": "12345"}},
            }),
        ),
        ("Observation", json!({"id": "ob-nosubj", "status": "final"})),
        (
            "ValueSet",
            json!({"id": "vs-a", "status": "active", "url": VS}),
        ),
        (
            "ValueSet",
            json!({"id": "vs-sub", "status": "active", "url": format!("{VS}/sub")}),
        ),
        ("ValueSet", json!({"id": "vs-none", "status": "active"})),
        (
            "RiskAssessment",
            json!({
                "id": "ra-half",
                "status": "final",
                "subject": {"reference": "Patient/p1"},
                "prediction": [{"probabilityDecimal": 0.5}],
            }),
        ),
        (
            "RiskAssessment",
            json!({
                "id": "ra-none",
                "status": "final",
                "subject": {"reference": "Patient/p1"},
            }),
        ),
    ];
    for (resource_type, resource) in resources {
        let id = resource["id"].as_str().unwrap_or_default().to_string();
        backend
            .create(tenant, resource_type, resource, FhirVersion::default())
            .await
            .unwrap_or_else(|e| panic!("create {resource_type}/{id} failed: {e}"));
    }
}

/// Seeds the data under a caller-unique tenant and asserts the matrix, with
/// the caller's [`Divergence`]s applied.
pub async fn every_valid_modifier_agrees_across_backends<S>(
    backend: &S,
    tenant_base: &str,
    divergences: &[Divergence],
) where
    S: ResourceStorage + SearchProvider,
{
    use SearchParamType as T;

    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;

    // The same Patient id under another tenant, with an identifier this
    // tenant's `p1` does not have.
    let other = TenantContext::new(
        TenantId::new(format!("{tenant_base}-other")),
        TenantPermissions::full_access(),
    );
    backend
        .create(
            &other,
            "Patient",
            json!({"id": "p1", "identifier": [{"system": MRN, "value": "77777"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap_or_else(|e| panic!("create the other tenant's Patient/p1 failed: {e}"));

    // Positive controls: the unmodified search on every parameter the matrix
    // uses. Polled because Elasticsearch is near-real-time. A failure here
    // means the parameter did not index — a backend built without the spec
    // search parameters — not that a modifier is wrong.
    let controls: [(&str, &str, SearchParamType, &str, &[&str]); 9] = [
        ("Patient", "identifier", T::Token, "12345", &["p1", "p2"]),
        ("Patient", "family", T::String, "smiths", &["p2"]),
        ("Patient", "birthdate", T::Date, "1980-01-01", &["p1"]),
        ("Observation", "code", T::Token, "1234-5", &["ob-pat"]),
        (
            "Observation",
            "subject",
            T::Reference,
            "Patient/p1",
            &["ob-pat", "ob-ver"],
        ),
        (
            "Observation",
            "subject",
            T::Reference,
            "Group/p1",
            &["ob-grp"],
        ),
        (
            "Observation",
            "value-quantity",
            T::Quantity,
            "5",
            &["ob-pat"],
        ),
        ("ValueSet", "url", T::Uri, VS, &["vs-a"]),
        (
            "RiskAssessment",
            "probability",
            T::Number,
            "0.5",
            &["ra-half"],
        ),
    ];
    for (resource_type, param, param_type, value, expected) in controls {
        let control = query(resource_type, param, param_type, None, value);
        let want: BTreeSet<String> = expected.iter().map(|id| id.to_string()).collect();
        let mut got = Ok(BTreeSet::new());
        for _ in 0..60 {
            got = outcome(backend, &tenant, &control).await;
            if got.as_ref() == Ok(&want) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert_eq!(
            got,
            Ok(want),
            "positive control {resource_type}?{param}={value}"
        );
    }

    let table = cases();
    let labels: BTreeSet<String> = table.iter().map(Case::label).collect();
    for divergence in divergences {
        assert!(
            labels.contains(divergence.label),
            "divergence names no cell of the matrix: {}",
            divergence.label
        );
    }

    let mut failures = Vec::new();
    for case in &table {
        let label = case.label();
        let query = case_query(case);
        let got = outcome(backend, &tenant, &query).await;
        let shown = render(&got);
        println!("{label} -> {shown}");

        // `search_count` takes its own route through some backends; it must
        // count what `search` returns.
        if let Ok(ids) = &got {
            match backend.search_count(&tenant, &query).await {
                Ok(count) if count == ids.len() as u64 => {}
                Ok(count) => failures.push(format!(
                    "{label}: search_count is {count}, search returned {}",
                    ids.len()
                )),
                Err(e) => failures.push(format!("{label}: search_count failed: {e}")),
            }
        }

        let agreed = render_expect(&case.expect);
        match divergences.iter().find(|d| d.label == label) {
            Some(divergence) => {
                let diverged = render_expect(&divergence.expect);
                if shown == agreed {
                    failures.push(format!(
                        "{label}: now agrees with the matrix ({agreed}); remove the divergence"
                    ));
                } else if shown != diverged {
                    failures.push(format!(
                        "{label}: got {shown}, expected the known divergence {diverged} \
                         (matrix: {agreed})"
                    ));
                }
            }
            None => {
                if shown != agreed {
                    failures.push(format!("{label}: got {shown}, expected {agreed}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
