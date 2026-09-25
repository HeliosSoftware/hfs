//! Backend-agnostic `_contained` search suite (issues #1336, #1362, #1363).
//!
//! `_contained=true|both` matches the resources *inside* a container, and each
//! backend answers it from its own index shape: SQLite and PostgreSQL group
//! `is_contained` rows of `search_index` per contained entity, MongoDB
//! aggregates a separate `search_index_contained` collection, Elasticsearch
//! indexes every contained resource as a document of its own. Two classes of
//! bug hid in those differences, each on some backends only:
//!
//! - a repeated parameter (`date=ge2020-01-01&date=le2020-12-31`) was a
//!   disjunction, because "every criterion matched" was proven by counting
//!   distinct parameter *names* (#1336 on PostgreSQL, #1362 on SQLite and
//!   MongoDB);
//! - whole classes of criteria — `_tag`/`_profile`/`_security`/`_id`/
//!   `_lastUpdated`, composites, and modifiers — were dropped without a word,
//!   so the search answered a wider question than the one asked (#1363).
//!
//! The rule the second scenario holds every backend to is the one the issue
//! states: a criterion is either applied or the search is refused with an
//! error naming the parameter. Which of the two a backend does is its own
//! business; returning the unfiltered answer is never acceptable.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `date_precision_suite.rs`. The backend must be built with the spec search
//! parameters loaded: without them nothing inside a container is indexed and
//! every "does not match" here would pass vacuously — so both scenarios start
//! with positive controls and refuse to go on without them.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider, SearchResult};
use helios_persistence::error::{SearchError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, CompartmentMembership, CompositeSearchComponent, ContainedMode,
    ContainedReturn, ReverseChainedParameter, SearchModifier, SearchParamType, SearchParameter,
    SearchPrefix, SearchQuery, SearchValue, TotalMode,
};

/// What a case must produce.
#[derive(Clone, Copy)]
enum Expect {
    /// Exactly these ids. An error is a failure.
    Ids(&'static [&'static str]),
    /// Exactly these ids, or a search error naming the parameter: the backend
    /// may decline a criterion its contained index cannot answer, but it may
    /// not answer without it.
    IdsOrRejected(&'static [&'static str], &'static str),
}

struct Case {
    label: &'static str,
    mode: ContainedMode,
    returns: ContainedReturn,
    parameters: Vec<SearchParameter>,
    compartment: Option<CompartmentMembership>,
    expect: Expect,
}

impl Case {
    fn new(label: &'static str, parameters: Vec<SearchParameter>, expect: Expect) -> Self {
        Self {
            label,
            mode: ContainedMode::On,
            returns: ContainedReturn::Container,
            parameters,
            compartment: None,
            expect,
        }
    }

    fn in_patient_compartment(mut self, patient: &str) -> Self {
        self.compartment = Some(CompartmentMembership {
            params: vec!["subject".to_string(), "performer".to_string()],
            reference: format!("Patient/{patient}"),
        });
        self
    }

    fn returning_contained(mut self) -> Self {
        self.returns = ContainedReturn::Contained;
        self
    }

    fn both(mut self) -> Self {
        self.mode = ContainedMode::Both;
        self
    }

    fn query(&self) -> SearchQuery {
        let mut query = SearchQuery::new("Observation");
        query.contained = self.mode;
        query.contained_return = self.returns;
        query.parameters = self.parameters.clone();
        query.compartment = self.compartment.clone();
        query
    }
}

fn param(name: &str, ty: SearchParamType, values: Vec<SearchValue>) -> SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type: ty,
        values,
        ..Default::default()
    }
}

/// A date parameter; each value carries its own prefix (`ge2020-01-01`).
fn date(values: &[&str]) -> SearchParameter {
    param(
        "date",
        SearchParamType::Date,
        values.iter().map(|v| SearchValue::parse(v)).collect(),
    )
}

/// A parameter whose values are taken literally, never read for a prefix.
fn literal(name: &str, ty: SearchParamType, value: &str) -> SearchParameter {
    param(name, ty, vec![SearchValue::new(SearchPrefix::Eq, value)])
}

fn token(name: &str, value: &str) -> SearchParameter {
    literal(name, SearchParamType::Token, value)
}

fn with_modifier(mut parameter: SearchParameter, modifier: SearchModifier) -> SearchParameter {
    parameter.modifier = Some(modifier);
    parameter
}

/// `code-value-quantity`, with the components the REST layer would resolve.
fn code_value_quantity(value: &str) -> SearchParameter {
    let mut parameter = literal("code-value-quantity", SearchParamType::Composite, value);
    parameter.components = vec![
        CompositeSearchComponent {
            param_type: SearchParamType::Token,
            param_name: "code".to_string(),
        },
        CompositeSearchComponent {
            param_type: SearchParamType::Quantity,
            param_name: "value-quantity".to_string(),
        },
    ];
    parameter
}

/// `component-code-value-quantity`, whose components repeat within a resource.
fn component_code_value_quantity(value: &str) -> SearchParameter {
    let mut parameter = literal(
        "component-code-value-quantity",
        SearchParamType::Composite,
        value,
    );
    parameter.components = vec![
        CompositeSearchComponent {
            param_type: SearchParamType::Token,
            param_name: "component-code".to_string(),
        },
        CompositeSearchComponent {
            param_type: SearchParamType::Quantity,
            param_name: "component-value-quantity".to_string(),
        },
    ];
    parameter
}

fn observation(id: &str, code: &str, date: &str, categories: &[&str]) -> Value {
    let categories: Vec<Value> = categories
        .iter()
        .map(|c| json!({"coding": [{"system": "http://example.org/cat", "code": c}]}))
        .collect();
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": "final",
        "category": categories,
        "code": {"coding": [{"system": "http://loinc.org", "code": code}]},
        "effectiveDateTime": date,
    })
}

async fn seed_containers<S>(
    backend: &S,
    tenant: &TenantContext,
    containers: Vec<(&str, Vec<Value>)>,
) where
    S: ResourceStorage + SearchProvider,
{
    for (id, contained) in containers {
        backend
            .create(
                tenant,
                "DiagnosticReport",
                json!({
                    "resourceType": "DiagnosticReport",
                    "id": id,
                    "status": "final",
                    "code": {"text": "panel"},
                    "contained": contained,
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed container");
    }
}

/// One case's outcome: the ids, or the error text.
async fn run<S>(
    backend: &S,
    tenant: &TenantContext,
    case: &Case,
) -> Result<BTreeSet<String>, String>
where
    S: ResourceStorage + SearchProvider,
{
    match backend.search(tenant, &case.query()).await {
        Ok(found) => Ok(found
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect()),
        Err(StorageError::Search(e)) => Err(e.to_string()),
        Err(other) => panic!("{}: not a search error: {other}", case.label),
    }
}

fn ids(expected: &[&str]) -> BTreeSet<String> {
    expected.iter().map(|id| id.to_string()).collect()
}

/// Waits for the positive controls (eventually-consistent indexes), then runs
/// every case and reports all the failures at once — the full matrix is the
/// useful output when a backend disagrees.
async fn assert_cases<S>(backend: &S, tenant: &TenantContext, controls: &[Case], cases: &[Case])
where
    S: ResourceStorage + SearchProvider,
{
    for control in controls {
        let Expect::Ids(expected) = control.expect else {
            panic!("{}: a control must not be rejectable", control.label);
        };
        for attempt in 0..60 {
            let got = run(backend, tenant, control).await;
            if got == Ok(ids(expected)) {
                break;
            }
            assert!(
                attempt < 59,
                "positive control {} never held: got {got:?}, expected {expected:?} — \
                 is the backend built with the spec search parameters?",
                control.label
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    let mut failures = Vec::new();
    for case in cases {
        let got = run(backend, tenant, case).await;
        let ok = match (case.expect, &got) {
            (Expect::Ids(expected), Ok(found)) => *found == ids(expected),
            (Expect::Ids(_), Err(_)) => false,
            (Expect::IdsOrRejected(expected, _), Ok(found)) => *found == ids(expected),
            // A refusal names the parameter, or — from a backend's general
            // modifier gate — the modifier it does not support.
            (Expect::IdsOrRejected(_, name), Err(message)) => {
                message.contains(name) || message.contains("unsupported modifier")
            }
        };
        let expected = match case.expect {
            Expect::Ids(expected) => format!("{expected:?}"),
            Expect::IdsOrRejected(expected, name) => {
                format!("{expected:?} or an error naming '{name}'")
            }
        };
        eprintln!(
            "[contained_suite] {} {} -> {got:?}",
            if ok { "ok  " } else { "FAIL" },
            case.label
        );
        if !ok {
            failures.push(format!("{}: got {got:?}, expected {expected}", case.label));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// Repeating a parameter under `_contained` is a conjunction: every occurrence
/// must hold, and on the same contained resource (#1336, #1362). A comma list
/// within one occurrence stays a disjunction.
///
/// Containers (DiagnosticReport → contained Observations):
/// - `dr-ab`: `a` (code X, 2020-06, cat1+cat2) and `b` (Y, 2021-06, cat1)
/// - `dr-b`: only `b` (Y, 2021-06, cat1) — satisfies `ge2020` alone
/// - `dr-early`: only `e` (Y, 2019-06, cat2) — satisfies `le2020` alone
/// - `dr-bc`: `b2` (Y, 2020-07, cat1) and `c` (X, 2022-01, cat2) — no one
///   contained resource has code X and a 2020 date, or both categories
/// - `dr-late`: `l` (X, 2020-11, cat1)
///
/// plus two top-level Observations for `_contained=both`: `top-in` (2020-05)
/// and `top-out` (2021-05).
pub async fn repeated_parameters_are_anded<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed_containers(
        backend,
        &tenant,
        vec![
            (
                "dr-ab",
                vec![
                    observation("a", "X", "2020-06-15", &["cat1", "cat2"]),
                    observation("b", "Y", "2021-06-15", &["cat1"]),
                ],
            ),
            ("dr-b", vec![observation("b", "Y", "2021-06-15", &["cat1"])]),
            (
                "dr-early",
                vec![observation("e", "Y", "2019-06-15", &["cat2"])],
            ),
            (
                "dr-bc",
                vec![
                    observation("b2", "Y", "2020-07-01", &["cat1"]),
                    observation("c", "X", "2022-01-01", &["cat2"]),
                ],
            ),
            (
                "dr-late",
                vec![observation("l", "X", "2020-11-20", &["cat1"])],
            ),
        ],
    )
    .await;
    for (id, when) in [("top-in", "2020-05-01"), ("top-out", "2021-05-01")] {
        backend
            .create(
                &tenant,
                "Observation",
                observation(id, "X", when, &["cat1"]),
                FhirVersion::default(),
            )
            .await
            .expect("seed top-level observation");
    }

    // Single occurrences, proving every row the cases below rely on is indexed.
    let controls = [
        Case::new(
            "date=ge2020-01-01",
            vec![date(&["ge2020-01-01"])],
            Expect::Ids(&["dr-ab", "dr-b", "dr-bc", "dr-late"]),
        ),
        Case::new(
            "date=le2020-12-31",
            vec![date(&["le2020-12-31"])],
            Expect::Ids(&["dr-ab", "dr-bc", "dr-early", "dr-late"]),
        ),
        Case::new(
            "code=X",
            vec![token("code", "X")],
            Expect::Ids(&["dr-ab", "dr-bc", "dr-late"]),
        ),
        Case::new(
            "category=cat2",
            vec![token("category", "cat2")],
            Expect::Ids(&["dr-ab", "dr-bc", "dr-early"]),
        ),
        Case::new(
            "_contained=both&date=ge2019-01-01",
            vec![date(&["ge2019-01-01"])],
            Expect::Ids(&[
                "dr-ab", "dr-b", "dr-bc", "dr-early", "dr-late", "top-in", "top-out",
            ]),
        )
        .both(),
    ];

    let range = || vec![date(&["ge2020-01-01"]), date(&["le2020-12-31"])];
    let cases = [
        // The range from the issue: dr-b and dr-early satisfy one bound only.
        Case::new(
            "date=ge2020-01-01&date=le2020-12-31",
            range(),
            Expect::Ids(&["dr-ab", "dr-bc", "dr-late"]),
        ),
        // Different parameters must hold on the SAME contained resource: dr-bc
        // has code X on `c` and a 2020 date on `b2`. Correct before #1336 too.
        Case::new(
            "code=X&date=le2020-12-31",
            vec![token("code", "X"), date(&["le2020-12-31"])],
            Expect::Ids(&["dr-ab", "dr-late"]),
        ),
        // A repeated name must not weaken the other parameters of the query.
        Case::new(
            "code=X&date=ge2020-01-01&date=le2020-12-31",
            vec![
                token("code", "X"),
                date(&["ge2020-01-01"]),
                date(&["le2020-12-31"]),
            ],
            Expect::Ids(&["dr-ab", "dr-late"]),
        ),
        // OR within an occurrence, AND across occurrences.
        Case::new(
            "date=le2020-12-31&date=lt2020-03-01,gt2020-10-01",
            vec![
                date(&["le2020-12-31"]),
                date(&["lt2020-03-01", "gt2020-10-01"]),
            ],
            Expect::Ids(&["dr-early", "dr-late"]),
        ),
        // Repeated token: both categories on one contained resource.
        Case::new(
            "category=cat1&category=cat2",
            vec![token("category", "cat1"), token("category", "cat2")],
            Expect::Ids(&["dr-ab"]),
        ),
        // `_containedType=contained` returns only the contained resources that
        // satisfy every occurrence — `a`, not its sibling `b`.
        Case::new(
            "_containedType=contained&date=ge2020-01-01&date=le2020-12-31",
            range(),
            Expect::Ids(&["a", "b2", "l"]),
        )
        .returning_contained(),
        // `_contained=both` merges top-level matches with the containers; the
        // range applies to both halves.
        Case::new(
            "_contained=both&date=ge2020-01-01&date=le2020-12-31",
            range(),
            Expect::Ids(&["dr-ab", "dr-bc", "dr-late", "top-in"]),
        )
        .both(),
    ];

    assert_cases(backend, &tenant, &controls, &cases).await;
}

/// No criterion is silently dropped under `_contained` (#1363): `_`-prefixed
/// parameters, composites and modifiers narrow the contained match, or the
/// search is refused with an error naming the parameter.
///
/// Containers (DiagnosticReport → one contained Observation each, code X
/// unless noted):
/// - `m-tagged`: `t1` — `meta.tag` foo, `meta.profile` …/p1, `meta.security`
///   R, `valueQuantity` 7 mg, `subject` Patient/pt1, an `identifier` of type
///   MR with value MR7
/// - `m-plain`: `p1` — no `meta`, `valueQuantity` 3 mg
/// - `m-str`: `s1` — `valueString` "hello"
/// - `m-other`: `o1` — code Y with text "Glucose level", `valueString`
///   "Hello World"
/// - `m-comp`: `c1` — code Z, components A = 1 mg and B = 9 mg: a composite
///   must pair a code with the quantity of the *same* component
pub async fn criteria_are_applied_or_rejected<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());

    let mut tagged = observation("t1", "X", "2020-06-15", &["cat1"]);
    tagged["meta"] = json!({
        "tag": [{"system": "http://example.org/tags", "code": "foo"}],
        "profile": ["http://example.org/StructureDefinition/p1"],
        "security": [{
            "system": "http://terminology.hl7.org/CodeSystem/v3-Confidentiality",
            "code": "R",
        }],
    });
    let quantity = |value: f64| {
        json!({
            "value": value,
            "unit": "mg",
            "system": "http://unitsofmeasure.org",
            "code": "mg",
        })
    };
    tagged["valueQuantity"] = quantity(7.0);
    tagged["subject"] = json!({"reference": "Patient/pt1"});
    tagged["identifier"] = json!([{
        "type": {"coding": [{
            "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
            "code": "MR",
        }]},
        "system": "http://example.org/obs",
        "value": "MR7",
    }]);
    let mut plain = observation("p1", "X", "2020-06-15", &["cat1"]);
    plain["valueQuantity"] = quantity(3.0);
    let mut string = observation("s1", "X", "2020-06-15", &["cat1"]);
    string["valueString"] = json!("hello");
    let mut other = observation("o1", "Y", "2020-06-15", &["cat1"]);
    other["code"]["text"] = json!("Glucose level");
    other["valueString"] = json!("Hello World");
    let mut components = observation("c1", "Z", "2020-06-15", &["cat1"]);
    components["component"] = json!([
        {
            "code": {"coding": [{"system": "http://loinc.org", "code": "A"}]},
            "valueQuantity": quantity(1.0),
        },
        {
            "code": {"coding": [{"system": "http://loinc.org", "code": "B"}]},
            "valueQuantity": quantity(9.0),
        },
    ]);

    seed_containers(
        backend,
        &tenant,
        vec![
            ("m-tagged", vec![tagged]),
            ("m-plain", vec![plain]),
            ("m-str", vec![string]),
            ("m-other", vec![other]),
            ("m-comp", vec![components]),
        ],
    )
    .await;

    let value_string = |value: &str| literal("value-string", SearchParamType::String, value);
    let subject = |value: &str| literal("subject", SearchParamType::Reference, value);
    let controls = [
        Case::new(
            "code=X",
            vec![token("code", "X")],
            Expect::Ids(&["m-plain", "m-str", "m-tagged"]),
        ),
        Case::new(
            "code=Y",
            vec![token("code", "Y")],
            Expect::Ids(&["m-other"]),
        ),
        // Default string matching: case-insensitive starts-with.
        Case::new(
            "value-string=hello",
            vec![value_string("hello")],
            Expect::Ids(&["m-other", "m-str"]),
        ),
        Case::new(
            "value-quantity=gt1",
            vec![param(
                "value-quantity",
                SearchParamType::Quantity,
                vec![SearchValue::parse("gt1")],
            )],
            Expect::Ids(&["m-plain", "m-tagged"]),
        ),
    ];

    let code_x = || token("code", "X");
    let cases = [
        // 1. `_`-prefixed parameters that describe the contained resource.
        Case::new(
            "code=X&_tag=foo",
            vec![code_x(), token("_tag", "foo")],
            Expect::IdsOrRejected(&["m-tagged"], "_tag"),
        ),
        Case::new(
            "code=X&_tag=absent",
            vec![code_x(), token("_tag", "absent")],
            Expect::IdsOrRejected(&[], "_tag"),
        ),
        Case::new(
            "_tag=foo (alone)",
            vec![token("_tag", "foo")],
            Expect::IdsOrRejected(&["m-tagged"], "_tag"),
        ),
        Case::new(
            "code=X&_profile=http://example.org/StructureDefinition/p1",
            vec![
                code_x(),
                literal(
                    "_profile",
                    SearchParamType::Uri,
                    "http://example.org/StructureDefinition/p1",
                ),
            ],
            Expect::IdsOrRejected(&["m-tagged"], "_profile"),
        ),
        Case::new(
            "code=X&_security=R",
            vec![code_x(), token("_security", "R")],
            Expect::IdsOrRejected(&["m-tagged"], "_security"),
        ),
        // `_id` is the contained resource's local id.
        Case::new(
            "code=X&_id=t1",
            vec![code_x(), token("_id", "t1")],
            Expect::IdsOrRejected(&["m-tagged"], "_id"),
        ),
        Case::new(
            "_id=t1 (alone)",
            vec![token("_id", "t1")],
            Expect::IdsOrRejected(&["m-tagged"], "_id"),
        ),
        // A contained resource has no `meta.lastUpdated` of its own; a backend
        // that answers takes the container's.
        Case::new(
            "code=X&_lastUpdated=lt1990-01-01",
            vec![
                code_x(),
                param(
                    "_lastUpdated",
                    SearchParamType::Date,
                    vec![SearchValue::parse("lt1990-01-01")],
                ),
            ],
            Expect::IdsOrRejected(&[], "_lastUpdated"),
        ),
        Case::new(
            "code=X&_lastUpdated=gt1990-01-01",
            vec![
                code_x(),
                param(
                    "_lastUpdated",
                    SearchParamType::Date,
                    vec![SearchValue::parse("gt1990-01-01")],
                ),
            ],
            Expect::IdsOrRejected(&["m-plain", "m-str", "m-tagged"], "_lastUpdated"),
        ),
        // 2. Composites.
        Case::new(
            "code-value-quantity=X$gt5",
            vec![code_value_quantity("X$gt5")],
            Expect::IdsOrRejected(&["m-tagged"], "code-value-quantity"),
        ),
        Case::new(
            "code=X&code-value-quantity=X$gt5",
            vec![code_x(), code_value_quantity("X$gt5")],
            Expect::IdsOrRejected(&["m-tagged"], "code-value-quantity"),
        ),
        // Pairing: code A goes with 1 mg and code B with 9 mg. `A$gt5` holds
        // for no single component, though A and a quantity > 5 both occur.
        Case::new(
            "component-code-value-quantity=A$lt5",
            vec![component_code_value_quantity("A$lt5")],
            Expect::IdsOrRejected(&["m-comp"], "component-code-value-quantity"),
        ),
        Case::new(
            "component-code-value-quantity=A$gt5",
            vec![component_code_value_quantity("A$gt5")],
            Expect::IdsOrRejected(&[], "component-code-value-quantity"),
        ),
        Case::new(
            "component-code-value-quantity=A$gt5,B$gt5",
            vec![SearchParameter {
                values: vec![
                    SearchValue::new(SearchPrefix::Eq, "A$gt5"),
                    SearchValue::new(SearchPrefix::Eq, "B$gt5"),
                ],
                ..component_code_value_quantity("")
            }],
            Expect::IdsOrRejected(&["m-comp"], "component-code-value-quantity"),
        ),
        Case::new(
            "code=X&code-value-quantity=X$lt5",
            vec![code_x(), code_value_quantity("X$lt5")],
            Expect::IdsOrRejected(&["m-plain"], "code-value-quantity"),
        ),
        // 3. Modifiers.
        Case::new(
            "code:not=X",
            vec![with_modifier(code_x(), SearchModifier::Not)],
            Expect::IdsOrRejected(&["m-comp", "m-other"], "code"),
        ),
        Case::new(
            "code:text=glucose",
            vec![with_modifier(
                token("code", "glucose"),
                SearchModifier::Text,
            )],
            Expect::IdsOrRejected(&["m-other"], "code"),
        ),
        Case::new(
            "value-string:exact=hello",
            vec![with_modifier(value_string("hello"), SearchModifier::Exact)],
            Expect::IdsOrRejected(&["m-str"], "value-string"),
        ),
        Case::new(
            "value-string:exact=Hello",
            vec![with_modifier(value_string("Hello"), SearchModifier::Exact)],
            Expect::IdsOrRejected(&[], "value-string"),
        ),
        Case::new(
            "value-string:contains=world",
            vec![with_modifier(
                value_string("world"),
                SearchModifier::Contains,
            )],
            Expect::IdsOrRejected(&["m-other"], "value-string"),
        ),
        Case::new(
            "value-string:missing=true",
            vec![with_modifier(value_string("true"), SearchModifier::Missing)],
            Expect::IdsOrRejected(&["m-comp", "m-plain", "m-tagged"], "value-string"),
        ),
        Case::new(
            "value-string:missing=false",
            vec![with_modifier(
                value_string("false"),
                SearchModifier::Missing,
            )],
            Expect::IdsOrRejected(&["m-other", "m-str"], "value-string"),
        ),
        Case::new(
            "code=X&value-string:missing=true",
            vec![
                code_x(),
                with_modifier(value_string("true"), SearchModifier::Missing),
            ],
            Expect::IdsOrRejected(&["m-plain", "m-tagged"], "value-string"),
        ),
        Case::new(
            "code:code-text=gluc",
            vec![with_modifier(
                token("code", "gluc"),
                SearchModifier::CodeText,
            )],
            Expect::IdsOrRejected(&["m-other"], "code"),
        ),
        Case::new(
            "identifier:of-type=…v2-0203|MR|MR7",
            vec![with_modifier(
                token(
                    "identifier",
                    "http://terminology.hl7.org/CodeSystem/v2-0203|MR|MR7",
                ),
                SearchModifier::OfType,
            )],
            Expect::IdsOrRejected(&["m-tagged"], "identifier"),
        ),
        Case::new(
            "identifier:of-type=…v2-0203|MR|other",
            vec![with_modifier(
                token(
                    "identifier",
                    "http://terminology.hl7.org/CodeSystem/v2-0203|MR|other",
                ),
                SearchModifier::OfType,
            )],
            Expect::IdsOrRejected(&[], "identifier"),
        ),
        // 4. The forms of a reference (#1407): `Type/id`, the bare id and
        // `:Type`. `:identifier` has a scenario of its own, below.
        Case::new(
            "subject=Patient/pt1",
            vec![subject("Patient/pt1")],
            Expect::Ids(&["m-tagged"]),
        ),
        Case::new(
            "subject=pt1",
            vec![subject("pt1")],
            Expect::Ids(&["m-tagged"]),
        ),
        Case::new("subject=t1", vec![subject("t1")], Expect::Ids(&[])),
        Case::new(
            "subject:Patient=pt1",
            vec![with_modifier(
                subject("pt1"),
                SearchModifier::Type("Patient".to_string()),
            )],
            Expect::IdsOrRejected(&["m-tagged"], "subject"),
        ),
        Case::new(
            "subject:Group=pt1",
            vec![with_modifier(
                subject("pt1"),
                SearchModifier::Type("Group".to_string()),
            )],
            Expect::IdsOrRejected(&[], "subject"),
        ),
        // 5. uri forms, on the one uri parameter an Observation has.
        Case::new(
            "_profile:below=http://example.org/StructureDefinition",
            vec![with_modifier(
                literal(
                    "_profile",
                    SearchParamType::Uri,
                    "http://example.org/StructureDefinition",
                ),
                SearchModifier::Below,
            )],
            Expect::IdsOrRejected(&["m-tagged"], "_profile"),
        ),
        Case::new(
            "_profile:below=http://example.org/Other",
            vec![with_modifier(
                literal("_profile", SearchParamType::Uri, "http://example.org/Other"),
                SearchModifier::Below,
            )],
            Expect::IdsOrRejected(&[], "_profile"),
        ),
        Case::new(
            "_profile:above=http://example.org/StructureDefinition/p1/extra",
            vec![with_modifier(
                literal(
                    "_profile",
                    SearchParamType::Uri,
                    "http://example.org/StructureDefinition/p1/extra",
                ),
                SearchModifier::Above,
            )],
            Expect::IdsOrRejected(&["m-tagged"], "_profile"),
        ),
    ];

    assert_cases(backend, &tenant, &controls, &cases).await;
}

/// One probe of the third scenario: a whole query (not only its parameters)
/// and what it must produce.
struct Probe {
    label: &'static str,
    query: SearchQuery,
    /// `Ok`: the ids of every match, sorted, duplicates kept (two containers
    /// may each hold a contained resource with the same local id).
    /// `Err`: text the refusal must contain.
    expect: Result<&'static [&'static str], &'static str>,
}

fn probe(
    label: &'static str,
    mode: ContainedMode,
    returns: ContainedReturn,
    expect: Result<&'static [&'static str], &'static str>,
    customize: impl FnOnce(&mut SearchQuery),
) -> Probe {
    let mut query = SearchQuery::new("Observation");
    query.contained = mode;
    query.contained_return = returns;
    customize(&mut query);
    Probe {
        label,
        query,
        expect,
    }
}

fn sorted_ids(result: &SearchResult) -> Vec<String> {
    let mut found: Vec<String> = result
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect();
    found.sort();
    found
}

/// What a probe produced: the ids of the search, the `_total` it reported,
/// `search_count`, and the ids gathered by walking it two at a time — which
/// must all describe the same set.
async fn run_probe<S>(backend: &S, tenant: &TenantContext, probe: &Probe) -> Result<String, String>
where
    S: ResourceStorage + SearchProvider,
{
    let search_error = |e: StorageError| match e {
        StorageError::Search(e) => e.to_string(),
        other => panic!("{}: not a search error: {other}", probe.label),
    };

    let mut query = probe.query.clone();
    query.total = Some(TotalMode::Accurate);
    let found = backend.search(tenant, &query).await.map_err(search_error)?;
    let ids = sorted_ids(&found);

    let counted = backend
        .search_count(tenant, &probe.query)
        .await
        .map_err(search_error)?;

    let mut paged = Vec::new();
    let mut page_totals = BTreeSet::new();
    for page in 0..20u32 {
        let mut query = query.clone();
        query.count = Some(2);
        query.offset = Some(page * 2);
        let found = backend.search(tenant, &query).await.map_err(search_error)?;
        if found.resources.items.is_empty() {
            break;
        }
        page_totals.insert(found.total);
        assert!(
            found.resources.items.len() <= 2,
            "{}: _count=2 returned {} items",
            probe.label,
            found.resources.items.len()
        );
        paged.extend(sorted_ids(&found));
    }
    paged.sort();

    Ok(format!(
        "ids={ids:?} total={:?} search_count={counted} paged={paged:?} page_totals={page_totals:?}",
        found.total
    ))
}

fn expected_probe_outcome(ids: &[&str]) -> String {
    let n = ids.len() as u64;
    let page_totals = if ids.is_empty() {
        BTreeSet::new()
    } else {
        BTreeSet::from([Some(n)])
    };
    format!(
        "ids={ids:?} total={:?} search_count={n} paged={ids:?} page_totals={page_totals:?}",
        Some(n)
    )
}

/// `_contained` with nothing else to go on, and with the constraints that live
/// outside `SearchQuery::parameters` (#1383).
///
/// - `_contained=true` alone is every contained resource of the type, in the
///   form `_containedType` asks for; `_total`, `search_count` and an
///   `_offset`/`_count` walk all describe that same set.
/// - Compartment membership is decided on the contained resource's own
///   references, like any other criterion.
/// - `_has`, `_list` and chained parameters select *top-level* resources. The
///   REST layer resolves them into an `_id` filter, and under `_contained`
///   `_id` is a contained resource's local id — so they are refused by name,
///   never resolved, dropped or misread.
///
/// Containers (DiagnosticReport → contained Observations):
/// - `u-one`: `o1` (code X, subject Patient/p1) and `o2` (Y, Patient/p2)
/// - `u-two`: `o1` (X, Patient/p1) — the same local id as in `u-one`
/// - `u-none`: a contained Specimen, no Observation
///
/// plus top-level Observations `top-1` (X, Patient/p1) and `o1` (Y,
/// Patient/p2) — the latter sharing its id with two contained resources.
pub async fn unconstrained_and_out_of_band_constraints<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    use ContainedMode::{Both, Off, On};
    use ContainedReturn::{Contained, Container};

    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    let about = |id: &str, code: &str, patient: &str| {
        let mut resource = observation(id, code, "2020-06-15", &["cat1"]);
        resource["subject"] = json!({"reference": format!("Patient/{patient}")});
        resource
    };
    seed_containers(
        backend,
        &tenant,
        vec![
            (
                "u-one",
                vec![about("o1", "X", "p1"), about("o2", "Y", "p2")],
            ),
            ("u-two", vec![about("o1", "X", "p1")]),
            (
                "u-none",
                vec![json!({"resourceType": "Specimen", "id": "s1", "status": "available"})],
            ),
        ],
    )
    .await;
    for resource in [about("top-1", "X", "p1"), about("o1", "Y", "p2")] {
        backend
            .create(&tenant, "Observation", resource, FhirVersion::default())
            .await
            .expect("seed top-level observation");
    }

    fn in_compartment(patient: &'static str) -> impl Fn(&mut SearchQuery) {
        move |q: &mut SearchQuery| {
            q.compartment = Some(CompartmentMembership {
                params: vec!["subject".to_string(), "performer".to_string()],
                reference: format!("Patient/{patient}"),
            });
        }
    }
    fn code_x(q: &mut SearchQuery) {
        q.parameters.push(token("code", "X"));
    }
    fn has_provenance(q: &mut SearchQuery) {
        q.reverse_chains.push(ReverseChainedParameter::terminal(
            "Provenance",
            "target",
            "agent",
            SearchValue::new(SearchPrefix::Eq, "Practitioner/x"),
        ));
    }
    fn in_list(q: &mut SearchQuery) {
        q.list.push("some-list".to_string());
    }

    // Positive controls: the contained rows, the top-level rows and the
    // reference rows compartment membership reads are all indexed.
    let controls = [
        probe(
            "code=X [true]",
            On,
            Container,
            Ok(&["u-one", "u-two"]),
            code_x,
        ),
        probe("code=X [false]", Off, Container, Ok(&["top-1"]), code_x),
        probe(
            "subject=Patient/p2 [true]",
            On,
            Container,
            Ok(&["u-one"]),
            |q| {
                q.parameters
                    .push(literal("subject", SearchParamType::Reference, "Patient/p2"))
            },
        ),
        probe(
            "Patient/p1/Observation [false]",
            Off,
            Container,
            Ok(&["top-1"]),
            in_compartment("p1"),
        ),
    ];
    for control in &controls {
        // Only what `search` finds is waited for; the controls' `_total`,
        // `search_count` and paging are checked with the probes below.
        let expected: Vec<String> = control
            .expect
            .expect("a control is not refused")
            .iter()
            .map(|id| id.to_string())
            .collect();
        for attempt in 0..60 {
            let got = backend
                .search(&tenant, &control.query)
                .await
                .map(|found| sorted_ids(&found));
            if got.as_ref().ok() == Some(&expected) {
                break;
            }
            assert!(
                attempt < 59,
                "positive control {} never held:\n       got {got:?}\n  expected {expected:?}\n\
                 is the backend built with the spec search parameters?",
                control.label
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    let probes = [
        // No criterion at all: every contained Observation.
        probe(
            "(none) [true]",
            On,
            Container,
            Ok(&["u-one", "u-two"]),
            |_| {},
        ),
        probe(
            "(none) [true, contained]",
            On,
            Contained,
            Ok(&["o1", "o1", "o2"]),
            |_| {},
        ),
        probe(
            "(none) [both]",
            Both,
            Container,
            Ok(&["o1", "top-1", "u-one", "u-two"]),
            |_| {},
        ),
        // The top-level `o1` and the two contained `o1` are three resources.
        probe(
            "(none) [both, contained]",
            Both,
            Contained,
            Ok(&["o1", "o1", "o1", "o2", "top-1"]),
            |_| {},
        ),
        // With a criterion: `_total` / `search_count` / paging agree too.
        probe(
            "code=X [true, contained]",
            On,
            Contained,
            Ok(&["o1", "o1"]),
            code_x,
        ),
        probe(
            "code=X [both]",
            Both,
            Container,
            Ok(&["top-1", "u-one", "u-two"]),
            code_x,
        ),
        probe(
            "code=X [both, contained]",
            Both,
            Contained,
            Ok(&["o1", "o1", "top-1"]),
            code_x,
        ),
        // Compartment membership, on the contained resource's own references.
        probe(
            "Patient/p1/Observation [true]",
            On,
            Container,
            Ok(&["u-one", "u-two"]),
            in_compartment("p1"),
        ),
        probe(
            "Patient/p2/Observation [true]",
            On,
            Container,
            Ok(&["u-one"]),
            in_compartment("p2"),
        ),
        probe(
            "Patient/p2/Observation [true, contained]",
            On,
            Contained,
            Ok(&["o2"]),
            in_compartment("p2"),
        ),
        probe(
            "Patient/p2/Observation?code=X [true]",
            On,
            Container,
            Ok(&[]),
            |q| {
                in_compartment("p2")(q);
                code_x(q);
            },
        ),
        probe(
            "Patient/p2/Observation [both]",
            Both,
            Container,
            Ok(&["o1", "u-one"]),
            in_compartment("p2"),
        ),
        probe(
            "Patient/nobody/Observation [true]",
            On,
            Container,
            Ok(&[]),
            in_compartment("nobody"),
        ),
        // Constraints on top-level resources: refused by name.
        probe("_has [true]", On, Container, Err("_has"), has_provenance),
        probe("_has [both]", Both, Container, Err("_has"), has_provenance),
        probe("_list [true]", On, Container, Err("_list"), in_list),
        probe("_list [both]", Both, Contained, Err("_list"), in_list),
        probe(
            "subject.name=x [true]",
            On,
            Container,
            Err("subject"),
            |q| {
                let mut chained = literal("subject", SearchParamType::Reference, "x");
                chained.chain = vec![ChainedParameter {
                    reference_param: "subject".to_string(),
                    target_type: Some("Patient".to_string()),
                    target_param: "name".to_string(),
                }];
                q.parameters.push(chained);
            },
        ),
    ];

    let mut failures = Vec::new();
    for probe in controls.iter().chain(&probes) {
        let got = run_probe(backend, &tenant, probe).await;
        let (ok, expected) = match (&probe.expect, &got) {
            (Ok(ids), Ok(outcome)) => {
                let expected = expected_probe_outcome(ids);
                (*outcome == expected, expected)
            }
            (Ok(ids), Err(_)) => (false, expected_probe_outcome(ids)),
            (Err(name), Ok(_)) => (false, format!("an error naming '{name}'")),
            (Err(name), Err(message)) => (
                message.contains(name) && message.contains("_contained"),
                format!("an error naming '{name}' and _contained"),
            ),
        };
        eprintln!(
            "[contained_suite] {} {} -> {got:?}",
            if ok { "ok  " } else { "FAIL" },
            probe.label
        );
        if !ok {
            failures.push(format!(
                "{}:\n       got {got:?}\n  expected {expected}",
                probe.label
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The ids of a search in the order returned, or the refusal's text.
async fn ordered_ids<S>(
    backend: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> Result<Vec<String>, String>
where
    S: ResourceStorage + SearchProvider,
{
    match backend.search(tenant, query).await {
        Ok(found) => Ok(found
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect()),
        Err(StorageError::Search(e)) => Err(e.to_string()),
        Err(other) => panic!("not a search error: {other}"),
    }
}

/// `_sort` under `_contained`, and a contained resource nothing but its id is
/// known about (#1407).
///
/// - `_sort` is applied — to the *contained* resource's values, a container
///   standing where its first matching contained resource does — or the search
///   is refused with an error naming `_sort`. Returning the matches in some
///   other order with a 200 is the one thing not allowed.
/// - A contained resource with no indexed value other than its id is still a
///   contained resource of its type: `_contained=true` alone and `_id` find it.
///
/// Containers (DiagnosticReport, created in this order → contained):
/// - `s-a`: Observation `oa` (code X, 2020-03-01)
/// - `s-b`: Observation `ob` (code X, 2020-01-01)
/// - `s-c`: Observation `oc` (code X, 2020-02-01)
/// - `s-bare`: `{"resourceType": "Location", "id": "bare"}` — a valid Location
///   that yields no search value except `_id`
/// - `s-named`: Location `named` with a `name`, the positive control for it
///
/// plus the top-level Observation `s-top` (code X, 2020-02-15). The three
/// orders that could be mistaken for one another all differ: by container id
/// `s-a, s-b, s-c`; most recently updated first `s-c, s-b, s-a`; by date
/// `s-b, s-c, s-a`.
pub async fn sort_and_id_only_contained<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    use ContainedMode::{Both, Off, On};
    use ContainedReturn::{Contained, Container};
    use helios_persistence::types::SortDirective;

    type Sort<'a> = &'a [(&'a str, Option<SearchParamType>)];

    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    for (container, local, when) in [
        ("s-a", "oa", "2020-03-01"),
        ("s-b", "ob", "2020-01-01"),
        ("s-c", "oc", "2020-02-01"),
    ] {
        seed_containers(
            backend,
            &tenant,
            vec![(container, vec![observation(local, "X", when, &["cat1"])])],
        )
        .await;
        // `_lastUpdated` orders the containers; keep their timestamps apart.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    seed_containers(
        backend,
        &tenant,
        vec![
            (
                "s-bare",
                vec![json!({"resourceType": "Location", "id": "bare"})],
            ),
            (
                "s-named",
                vec![json!({"resourceType": "Location", "id": "named", "name": "Ward 7"})],
            ),
        ],
    )
    .await;
    backend
        .create(
            &tenant,
            "Observation",
            observation("s-top", "X", "2020-02-15", &["cat1"]),
            FhirVersion::default(),
        )
        .await
        .expect("seed top-level observation");

    let build = |resource_type: &str,
                 mode: ContainedMode,
                 returns: ContainedReturn,
                 parameters: Vec<SearchParameter>,
                 sort: Sort| {
        let mut query = SearchQuery::new(resource_type);
        query.contained = mode;
        query.contained_return = returns;
        query.parameters = parameters;
        query.sort = sort
            .iter()
            .map(|(by, ty)| SortDirective::parse(by).with_param_type(*ty))
            .collect();
        query
    };
    let code_x = || vec![token("code", "X")];
    let by_date: Sort = &[("date", Some(SearchParamType::Date))];
    let by_date_desc: Sort = &[("-date", Some(SearchParamType::Date))];

    // Positive controls: every row the cases rely on is indexed, and `_sort`
    // itself works on this backend for a top-level search.
    let controls: Vec<(&str, SearchQuery, &[&str])> = vec![
        (
            "code=X [true]",
            build("Observation", On, Container, code_x(), &[]),
            &["s-a", "s-b", "s-c"],
        ),
        (
            "Location?name=Ward [true]",
            build(
                "Location",
                On,
                Container,
                vec![literal("name", SearchParamType::String, "Ward")],
                &[],
            ),
            &["s-named"],
        ),
        (
            "code=X&_sort=date [false]",
            build("Observation", Off, Container, code_x(), by_date),
            &["s-top"],
        ),
    ];
    for (label, query, expected) in &controls {
        let expected: Vec<String> = expected.iter().map(|id| id.to_string()).collect();
        for attempt in 0..60 {
            let got = ordered_ids(backend, &tenant, query).await.map(|mut found| {
                found.sort();
                found
            });
            if got.as_ref().ok() == Some(&expected) {
                break;
            }
            assert!(
                attempt < 59,
                "positive control {label} never held: got {got:?}, expected {expected:?} — \
                 is the backend built with the spec search parameters?"
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    // (label, query, the ids expected — in this order when the query sorts —
    // and the parameter a refusal may name instead, if one is acceptable)
    let sort = Some("_sort");
    let cases: Vec<(&str, SearchQuery, &[&str], Option<&str>)> = vec![
        (
            "code=X&_sort=date [true]",
            build("Observation", On, Container, code_x(), by_date),
            &["s-b", "s-c", "s-a"],
            sort,
        ),
        (
            "code=X&_sort=-date [true]",
            build("Observation", On, Container, code_x(), by_date_desc),
            &["s-a", "s-c", "s-b"],
            sort,
        ),
        (
            "code=X&_sort=-date [true, contained]",
            build("Observation", On, Contained, code_x(), by_date_desc),
            &["oa", "oc", "ob"],
            sort,
        ),
        (
            "_sort=date, no criterion [true, contained]",
            build("Observation", On, Contained, vec![], by_date),
            &["ob", "oc", "oa"],
            sort,
        ),
        // `both` is one list: the top-level match sorts among the containers.
        (
            "code=X&_sort=date [both]",
            build("Observation", Both, Container, code_x(), by_date),
            &["s-b", "s-c", "s-top", "s-a"],
            sort,
        ),
        // A contained resource has no `meta.lastUpdated`; its container's is
        // the only one there is.
        (
            "code=X&_sort=_lastUpdated [true]",
            build(
                "Observation",
                On,
                Container,
                code_x(),
                &[("_lastUpdated", None)],
            ),
            &["s-a", "s-b", "s-c"],
            sort,
        ),
        (
            "code=X&_sort=-_lastUpdated [true]",
            build(
                "Observation",
                On,
                Container,
                code_x(),
                &[("-_lastUpdated", None)],
            ),
            &["s-c", "s-b", "s-a"],
            sort,
        ),
        // The id-only contained resource.
        (
            "Location [true]",
            build("Location", On, Container, vec![], &[]),
            &["s-bare", "s-named"],
            None,
        ),
        (
            "Location [true, contained]",
            build("Location", On, Contained, vec![], &[]),
            &["bare", "named"],
            None,
        ),
        (
            "Location?_id=bare [true]",
            build("Location", On, Container, vec![token("_id", "bare")], &[]),
            &["s-bare"],
            None,
        ),
        (
            "Location?name:missing=true [true]",
            build(
                "Location",
                On,
                Container,
                vec![with_modifier(
                    literal("name", SearchParamType::String, "true"),
                    SearchModifier::Missing,
                )],
                &[],
            ),
            &["s-bare"],
            Some("name"),
        ),
    ];

    let mut failures = Vec::new();
    for (label, query, expected, may_refuse) in &cases {
        let got = ordered_ids(backend, &tenant, query).await.map(|mut found| {
            if query.sort.is_empty() {
                found.sort();
            }
            found
        });
        let ok = match (&got, may_refuse) {
            (Ok(found), _) => found
                .iter()
                .map(String::as_str)
                .eq(expected.iter().copied()),
            (Err(message), Some(name)) => message.contains(name) && message.contains("_contained"),
            (Err(_), None) => false,
        };
        eprintln!(
            "[contained_suite] {} {label} -> {got:?}",
            if ok { "ok  " } else { "FAIL" }
        );
        if !ok {
            let or_refused = may_refuse
                .map(|name| format!(" or an error naming '{name}' and _contained"))
                .unwrap_or_default();
            failures.push(format!(
                "{label}: got {got:?}, expected {expected:?}{or_refused}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// `reference:identifier` under `_contained`, for the backends that resolve it
/// through the reference's *target* (SQLite, PostgreSQL): the contained
/// resource's `subject` names a top-level Patient, and the search names that
/// Patient by one of its identifiers (#1407).
///
/// Not part of the all-backend scenarios because the backends do not agree on
/// what `:identifier` reads even for a top-level search: Elasticsearch matches
/// `Reference.identifier` token values (which the shared extractor does not
/// index under the reference parameter), and MongoDB refuses the modifier.
///
/// Containers (DiagnosticReport → contained Observation, code X): `i-mrn`
/// (`subject` Patient/ip1) and `i-other` (`subject` Patient/ip2); top-level
/// Patients `ip1` (identifier `http://example.org/mrn|42`) and `ip2` (`|43`).
/// The container `i-decoy` holds a contained *Patient* `ip9` with identifier
/// `|44` and an Observation about `DiagnosticReport/i-decoy`: a contained
/// resource's identifier rows are stored under its container, and must not
/// make the container a target.
pub async fn reference_identifier_resolves_the_target<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    let mrn = |value: &str| json!([{"system": "http://example.org/mrn", "value": value}]);
    let about = |id: &str, reference: &str| {
        let mut resource = observation(id, "X", "2020-06-15", &["cat1"]);
        resource["subject"] = json!({"reference": reference});
        resource
    };
    for (id, value) in [("ip1", "42"), ("ip2", "43")] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": id, "identifier": mrn(value)}),
                FhirVersion::default(),
            )
            .await
            .expect("seed patient");
    }
    seed_containers(
        backend,
        &tenant,
        vec![
            ("i-mrn", vec![about("o1", "Patient/ip1")]),
            ("i-other", vec![about("o1", "Patient/ip2")]),
            (
                "i-decoy",
                vec![
                    json!({"resourceType": "Patient", "id": "ip9", "identifier": mrn("44")}),
                    about("o1", "DiagnosticReport/i-decoy"),
                ],
            ),
        ],
    )
    .await;

    let subject = |value: &str| literal("subject", SearchParamType::Reference, value);
    let by_identifier =
        |value: &str| vec![with_modifier(subject(value), SearchModifier::Identifier)];
    let controls = [
        Case::new(
            "subject=Patient/ip1",
            vec![subject("Patient/ip1")],
            Expect::Ids(&["i-mrn"]),
        ),
        Case::new(
            "code=X",
            vec![token("code", "X")],
            Expect::Ids(&["i-decoy", "i-mrn", "i-other"]),
        ),
    ];
    let cases = [
        Case::new(
            "subject:identifier=http://example.org/mrn|42",
            by_identifier("http://example.org/mrn|42"),
            Expect::Ids(&["i-mrn"]),
        ),
        Case::new(
            "subject:identifier=42,43",
            vec![SearchParameter {
                values: vec![SearchValue::eq("42"), SearchValue::eq("43")],
                ..by_identifier("42").remove(0)
            }],
            Expect::Ids(&["i-mrn", "i-other"]),
        ),
        Case::new(
            "subject:identifier=http://example.org/mrn|nope",
            by_identifier("http://example.org/mrn|nope"),
            Expect::Ids(&[]),
        ),
        Case::new(
            "subject:identifier=http://example.org/mrn|44 (a contained Patient's)",
            by_identifier("http://example.org/mrn|44"),
            Expect::Ids(&[]),
        ),
        Case::new(
            "code=X&subject:identifier=…|43 [contained]",
            vec![
                token("code", "X"),
                by_identifier("http://example.org/mrn|43").remove(0),
            ],
            Expect::Ids(&["o1"]),
        )
        .returning_contained(),
    ];

    assert_cases(backend, &tenant, &controls, &cases).await;
}

/// Strict `_contained` composite pairing for MongoDB (#1407).
///
/// Unlike `criteria_are_applied_or_rejected`, which lets a backend refuse a
/// composite by naming it, these cases demand the answer from
/// `search_index_contained`.
///
/// Containers (DiagnosticReport → contained Observations):
/// - `c-single`: `c1` with components A = 1 mg and B = 9 mg — a composite
///   must pair a code with the quantity of the *same* component, so `A$gt5`
///   matches nothing even though A and a quantity > 5 both occur. Its code is
///   Z and subject is Patient/p1.
/// - `x-sib`: siblings `s1` (component A = 1 mg, code Z, Patient/p1) and
///   `s2` (component B = 9 mg, code Y, Patient/p2). Both components carry
///   `composite_group` 0 in the index, so an
///   implementation that pairs by group alone — without scoping to one
///   contained entity — would cross `s1`'s A with `s2`'s 9 mg and wrongly
///   match `A$gt5`. The differing codes, subjects, and local ids also catch
///   intersections that accidentally combine matches across siblings.
pub async fn contained_composites_pair_within_one_resource<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    let quantity = |value: f64| {
        json!({
            "value": value,
            "unit": "mg",
            "system": "http://unitsofmeasure.org",
            "code": "mg",
        })
    };
    let mut single = observation("c1", "Z", "2020-06-15", &["cat1"]);
    single["subject"] = json!({"reference": "Patient/p1"});
    single["component"] = json!([
        {
            "code": {"coding": [{"system": "http://loinc.org", "code": "A"}]},
            "valueQuantity": quantity(1.0),
        },
        {
            "code": {"coding": [{"system": "http://loinc.org", "code": "B"}]},
            "valueQuantity": quantity(9.0),
        },
    ]);
    let mut sib_a = observation("s1", "Z", "2020-06-15", &["cat1"]);
    sib_a["subject"] = json!({"reference": "Patient/p1"});
    sib_a["component"] = json!([
        {
            "code": {"coding": [{"system": "http://loinc.org", "code": "A"}]},
            "valueQuantity": quantity(1.0),
        },
    ]);
    let mut sib_b = observation("s2", "Y", "2020-06-15", &["cat1"]);
    sib_b["subject"] = json!({"reference": "Patient/p2"});
    sib_b["component"] = json!([
        {
            "code": {"coding": [{"system": "http://loinc.org", "code": "B"}]},
            "valueQuantity": quantity(9.0),
        },
    ]);

    seed_containers(
        backend,
        &tenant,
        vec![("c-single", vec![single]), ("x-sib", vec![sib_a, sib_b])],
    )
    .await;

    let controls = [
        Case::new(
            "code=Z",
            vec![token("code", "Z")],
            Expect::Ids(&["c-single", "x-sib"]),
        ),
        Case::new("code=Y", vec![token("code", "Y")], Expect::Ids(&["x-sib"])),
        Case::new(
            "subject=Patient/p2",
            vec![literal("subject", SearchParamType::Reference, "Patient/p2")],
            Expect::Ids(&["x-sib"]),
        ),
    ];
    let cases = [
        Case::new(
            "component-code-value-quantity=A$lt5",
            vec![component_code_value_quantity("A$lt5")],
            Expect::Ids(&["c-single", "x-sib"]),
        ),
        // Pairing: neither `c1`'s A (1 mg) nor `s1`'s A (1 mg) exceeds 5 mg,
        // and `s2`'s 9 mg belongs to B — on another contained resource.
        Case::new(
            "component-code-value-quantity=A$gt5",
            vec![component_code_value_quantity("A$gt5")],
            Expect::Ids(&[]),
        ),
        Case::new(
            "component-code-value-quantity=B$gt5",
            vec![component_code_value_quantity("B$gt5")],
            Expect::Ids(&["c-single", "x-sib"]),
        ),
        Case::new(
            "component-code-value-quantity=B$lt5",
            vec![component_code_value_quantity("B$lt5")],
            Expect::Ids(&[]),
        ),
        // The ordinary code criterion must hold on the same contained entity
        // as the composite, not merely elsewhere in its container.
        Case::new(
            "component-code-value-quantity=B$gt5&code=Z",
            vec![component_code_value_quantity("B$gt5"), token("code", "Z")],
            Expect::Ids(&["c-single"]),
        ),
        Case::new(
            "component-code-value-quantity=A$lt5&code=Y",
            vec![component_code_value_quantity("A$lt5"), token("code", "Y")],
            Expect::Ids(&[]),
        ),
        // Repeated composites are ANDed per contained entity. The sibling
        // container has both pairs, but in different contained Observations.
        Case::new(
            "component-code-value-quantity=A$lt5&component-code-value-quantity=B$gt5",
            vec![
                component_code_value_quantity("A$lt5"),
                component_code_value_quantity("B$gt5"),
            ],
            Expect::Ids(&["c-single"]),
        ),
        Case::new(
            "component-code-value-quantity=B$gt5&component-code-value-quantity=A$gt5",
            vec![
                component_code_value_quantity("B$gt5"),
                component_code_value_quantity("A$gt5"),
            ],
            Expect::Ids(&[]),
        ),
        Case::new(
            "Patient/p1/Observation?component-code-value-quantity=B$gt5",
            vec![component_code_value_quantity("B$gt5")],
            Expect::Ids(&["c-single"]),
        )
        .in_patient_compartment("p1"),
        Case::new(
            "Patient/p2/Observation?component-code-value-quantity=B$gt5",
            vec![component_code_value_quantity("B$gt5")],
            Expect::Ids(&["x-sib"]),
        )
        .in_patient_compartment("p2"),
        Case::new(
            "Patient/p2/Observation?component-code-value-quantity=A$lt5",
            vec![component_code_value_quantity("A$lt5")],
            Expect::Ids(&[]),
        )
        .in_patient_compartment("p2"),
        Case::new(
            "component-code-value-quantity=B$gt5&_id=c1",
            vec![component_code_value_quantity("B$gt5"), token("_id", "c1")],
            Expect::Ids(&["c-single"]),
        ),
        Case::new(
            "component-code-value-quantity=B$gt5&_id=s2",
            vec![component_code_value_quantity("B$gt5"), token("_id", "s2")],
            Expect::Ids(&["x-sib"]),
        ),
        Case::new(
            "component-code-value-quantity=B$gt5&_id=s1",
            vec![component_code_value_quantity("B$gt5"), token("_id", "s1")],
            Expect::Ids(&[]),
        ),
        // A comma list within one occurrence stays a disjunction: `B$gt5`
        // holds for one component in each container.
        Case::new(
            "component-code-value-quantity=A$gt5,B$gt5",
            vec![SearchParameter {
                values: vec![
                    SearchValue::new(SearchPrefix::Eq, "A$gt5"),
                    SearchValue::new(SearchPrefix::Eq, "B$gt5"),
                ],
                ..component_code_value_quantity("")
            }],
            Expect::Ids(&["c-single", "x-sib"]),
        ),
    ];

    assert_cases(backend, &tenant, &controls, &cases).await;
}

/// A repeated-type composite keeps its declared component order on both
/// MongoDB index collections. Unsupported composite modifiers still fail.
pub async fn repeated_type_composite_and_modifier<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    let mut observed = observation("top", "A", "2020-06-15", &["cat1"]);
    observed["code"]["coding"] = json!([
        {"system": "http://loinc.org", "code": "A"},
        {"system": "http://loinc.org", "code": "X"}
    ]);
    observed["valueCodeableConcept"] = json!({"coding": [
        {"system": "http://example.org/value", "code": "B"},
        {"system": "http://example.org/value", "code": "Y"}
    ]});
    observed["component"] = json!([
        {
            "code": {"coding": [{"system": "http://loinc.org", "code": "A"}]},
            "valueCodeableConcept": {"coding": [{"system": "http://example.org/value", "code": "B"}]}
        },
        {
            "code": {"coding": [{"system": "http://loinc.org", "code": "C"}]},
            "valueCodeableConcept": {"coding": [{"system": "http://example.org/value", "code": "D"}]}
        }
    ]);
    backend
        .create(
            &tenant,
            "Observation",
            observed.clone(),
            FhirVersion::default(),
        )
        .await
        .expect("seed top-level composite control");
    observed["id"] = json!("inside");
    seed_containers(backend, &tenant, vec![("container", vec![observed])]).await;

    // Confirm that the contained resource was indexed before checking errors.
    let mut control = SearchQuery::new("Observation");
    control.contained = ContainedMode::On;
    control.parameters.push(token("code", "A"));
    assert_eq!(
        sorted_ids(
            &backend
                .search(&tenant, &control)
                .await
                .expect("control search")
        ),
        vec!["container"],
    );

    let mut indexed_pair = literal("code-value-concept", SearchParamType::Composite, "A$B");
    indexed_pair.components = vec![
        CompositeSearchComponent {
            param_type: SearchParamType::Token,
            param_name: "code".to_string(),
        },
        CompositeSearchComponent {
            param_type: SearchParamType::Token,
            param_name: "value-concept".to_string(),
        },
    ];
    let mut pair_control = SearchQuery::new("Observation");
    pair_control.parameters.push(indexed_pair.clone());
    assert_eq!(
        sorted_ids(
            &backend
                .search(&tenant, &pair_control)
                .await
                .expect("top-level code-value-concept=A$B control")
        ),
        vec!["top"],
        "the composite pair must be indexed",
    );

    for (mode, returns, expected) in [
        (ContainedMode::Off, ContainedReturn::Container, vec!["top"]),
        (
            ContainedMode::On,
            ContainedReturn::Container,
            vec!["container"],
        ),
        (
            ContainedMode::On,
            ContainedReturn::Contained,
            vec!["inside"],
        ),
        (
            ContainedMode::Both,
            ContainedReturn::Container,
            vec!["container", "top"],
        ),
        (
            ContainedMode::Both,
            ContainedReturn::Contained,
            vec!["inside", "top"],
        ),
    ] {
        for (value, want_match) in [
            ("A$B", true),
            ("X$Y", true),
            ("X$B", true),
            ("B$A", false),
            ("Y$X", false),
        ] {
            let mut query = SearchQuery::new("Observation");
            query.contained = mode;
            query.contained_return = returns;
            query.count = Some(1);
            query.total = Some(TotalMode::Accurate);
            let mut pair = indexed_pair.clone();
            pair.values = vec![SearchValue::new(SearchPrefix::Eq, value)];
            query.parameters.push(pair);
            let result = backend
                .search(&tenant, &query)
                .await
                .expect("repeated-type composite search");
            let mut expected_ids = if want_match { expected.clone() } else { vec![] };
            expected_ids.sort();
            // A one-item page still has to report the full count.
            assert_eq!(
                result.total,
                Some(expected_ids.len() as u64),
                "{mode:?} {returns:?} {value}"
            );
            assert_eq!(
                backend.search_count(&tenant, &query).await.unwrap(),
                expected_ids.len() as u64,
                "{mode:?} {returns:?} {value}"
            );
            let mut full = query.clone();
            full.count = Some(10);
            assert_eq!(
                sorted_ids(&backend.search(&tenant, &full).await.unwrap()),
                expected_ids,
                "{mode:?} {returns:?} {value}"
            );
        }
    }

    let component_pair = SearchParameter {
        name: "component-code-value-concept".to_string(),
        components: indexed_pair.components.clone(),
        ..indexed_pair.clone()
    };
    for mode in [ContainedMode::Off, ContainedMode::On, ContainedMode::Both] {
        for (value, matched) in [
            ("A$B", true),
            ("C$D", true),
            ("B$A", false),
            ("D$C", false),
            ("A$D", false),
        ] {
            let mut query = SearchQuery::new("Observation");
            query.contained = mode;
            let mut pair = component_pair.clone();
            pair.values = vec![SearchValue::eq(value)];
            query.parameters.push(pair);
            let expected = match (mode, matched) {
                (_, false) => vec![],
                (ContainedMode::Off, true) => vec!["top"],
                (ContainedMode::On, true) => vec!["container"],
                (ContainedMode::Both, true) => vec!["container", "top"],
            };
            assert_eq!(
                sorted_ids(&backend.search(&tenant, &query).await.unwrap()),
                expected,
                "{mode:?} {value}"
            );
        }
    }

    for mode in [ContainedMode::On, ContainedMode::Both] {
        let mut modified = SearchQuery::new("Observation");
        modified.contained = mode;
        modified.count = Some(1);
        modified.parameters.push(with_modifier(
            code_value_quantity("A$gt5"),
            SearchModifier::Exact,
        ));
        for result in [
            backend.search(&tenant, &modified).await.map(|_| ()),
            backend.search_count(&tenant, &modified).await.map(|_| ()),
        ] {
            let err = result.expect_err("composite :exact must not be ignored");
            assert!(
                matches!(
                    &err,
                    StorageError::Search(SearchError::InvalidComposite { .. })
                ),
                "expected InvalidComposite for {mode:?}, got {err:?}"
            );
            assert!(err.to_string().contains(":exact"));
        }
    }
}

/// `_contained=both` lists a container that also matches top-level once (#1407).
///
/// Strict: the overlap must be removed before pagination, so one query proves
/// each resource appears exactly once, `_total` and `search_count` agree, and
/// `_count`/`_offset` pages crossing the top-level/contained boundary come
/// back full with no duplicates. MongoDB currently dedups against the
/// top-level *page* only and double-counts the overlap in `_total`, so the
/// `[both]` probes below fail until that changes.
///
/// Seeds (searched type: Observation, criterion: `code=X`):
/// - top-level Observation `shared` (code X);
/// - Observation `overlap` (own code X, so a top-level match itself) whose
///   contained Observation is also `shared` — the same local id as the
///   top-level resource, which is still a different resource;
/// - DiagnosticReport `plain` with contained Observation `solo` (code X).
///
/// `code=X [both]` is then top-level `{overlap, shared}` plus contained
/// `{overlap, plain}`, merged to `{overlap, plain, shared}` with total 3.
/// The `[both, contained]` control proves the dedup never crosses the
/// container/contained line: top-level `shared` and contained `shared` are
/// both listed.
pub async fn both_dedups_container_also_matching_top_level<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    use ContainedMode::{Both, Off, On};
    use ContainedReturn::{Contained, Container};

    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    fn code_x(q: &mut SearchQuery) {
        q.parameters.push(token("code", "X"));
    }

    backend
        .create(
            &tenant,
            "Observation",
            observation("shared", "X", "2020-06-15", &["cat1"]),
            FhirVersion::default(),
        )
        .await
        .expect("seed top-level observation");
    let mut overlap = observation("overlap", "X", "2020-06-15", &["cat1"]);
    overlap["contained"] = json!([observation("shared", "X", "2020-06-15", &["cat1"])]);
    backend
        .create(&tenant, "Observation", overlap, FhirVersion::default())
        .await
        .expect("seed overlapping observation container");
    seed_containers(
        backend,
        &tenant,
        vec![(
            "plain",
            vec![observation("solo", "X", "2020-06-15", &["cat1"])],
        )],
    )
    .await;

    // Positive controls: the top-level rows and the contained rows are indexed.
    let controls = [
        probe(
            "code=X [false]",
            Off,
            Container,
            Ok(&["overlap", "shared"]),
            code_x,
        ),
        probe(
            "code=X [true]",
            On,
            Container,
            Ok(&["overlap", "plain"]),
            code_x,
        ),
    ];
    for control in &controls {
        let expected: Vec<String> = control
            .expect
            .expect("a control is not refused")
            .iter()
            .map(|id| id.to_string())
            .collect();
        for attempt in 0..60 {
            let got = backend
                .search(&tenant, &control.query)
                .await
                .map(|found| sorted_ids(&found));
            if got.as_ref().ok() == Some(&expected) {
                break;
            }
            assert!(
                attempt < 59,
                "positive control {} never held:\n       got {got:?}\n  expected {expected:?}\n                 is the backend built with the spec search parameters?",
                control.label
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    let probes = [
        // The overlap is one result, counted once.
        probe(
            "code=X [both]",
            Both,
            Container,
            Ok(&["overlap", "plain", "shared"]),
            code_x,
        ),
        // A contained resource keeps its own identity even when its local id
        // equals a top-level id: no dedup across that line.
        probe(
            "code=X [both, contained]",
            Both,
            Contained,
            Ok(&["overlap", "shared", "shared", "solo"]),
            code_x,
        ),
        probe(
            "code=X [true, contained]",
            On,
            Contained,
            Ok(&["shared", "solo"]),
            code_x,
        ),
    ];

    let mut failures = Vec::new();
    for probe in controls.iter().chain(&probes) {
        let got = run_probe(backend, &tenant, probe).await;
        let (ok, expected) = match (&probe.expect, &got) {
            (Ok(ids), Ok(outcome)) => {
                let expected = expected_probe_outcome(ids);
                (*outcome == expected, expected)
            }
            (Ok(ids), Err(_)) => (false, expected_probe_outcome(ids)),
            (Err(name), Ok(_)) => (false, format!("an error naming '{name}'")),
            (Err(name), Err(message)) => (
                message.contains(name) && message.contains("_contained"),
                format!("an error naming '{name}' and _contained"),
            ),
        };
        eprintln!(
            "[contained_suite] {} {} -> {got:?}",
            if ok { "ok  " } else { "FAIL" },
            probe.label
        );
        if !ok {
            failures.push(format!(
                "{}:\n       got {got:?}\n  expected {expected}",
                probe.label
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));

    // One more walk with `_count=1`, so every page boundary — including each
    // side of the top-level/contained frontier at offset 2 — is crossed with
    // no short page and no duplicate.
    let mut query = SearchQuery::new("Observation");
    query.contained = Both;
    query.contained_return = Container;
    query.total = Some(TotalMode::Accurate);
    code_x(&mut query);
    let mut walked = Vec::new();
    for offset in 0..4u32 {
        let mut page = query.clone();
        page.count = Some(1);
        page.offset = Some(offset);
        let found = backend
            .search(&tenant, &page)
            .await
            .expect("both page with _count=1");
        assert_eq!(
            found.total,
            Some(3),
            "offset {offset}: _total must count the overlap once"
        );
        walked.extend(sorted_ids(&found));
    }
    walked.sort();
    assert_eq!(
        walked,
        vec![
            "overlap".to_string(),
            "plain".to_string(),
            "shared".to_string()
        ],
        "walking _count=1 across the boundary must visit each resource once"
    );
}

/// `_contained` lists every hit even past `max_result_window` (#1407).
///
/// Strict: with a window of 10 and 16 contained documents the list, `_total`,
/// `search_count` and the `_count`/`_offset` pages must all describe the same
/// 13 containers. `c-multi` holds four matching contained resources, so its
/// hits land on both sides of the page boundary: deduping each round on its
/// own drops it or lists it twice. Fails while the backend stops at one
/// request — the list holds the first 10 hits only, and `_total` with it.
///
/// Seeds (searched type: Observation, criterion: `code=X`):
/// - DiagnosticReports `c00`–`c11`, each with one contained Observation
///   (local ids `k00`–`k11`, code X);
/// - DiagnosticReport `c-multi` with four contained Observations (local ids
///   `m-a`–`m-d`, code X): 16 contained documents, more than one window;
/// - top-level Observations `top-a` and `top-b` (code X) for the `both` probes.
///
/// `code=X` is then 13 containers, 16 contained resources, and — with `both`
/// — 15 entries with the two top-level resources mixed in.
pub async fn past_window_lists_every_contained_hit<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    use ContainedMode::{Both, On};
    use ContainedReturn::{Contained, Container};

    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    for n in 0..12 {
        let local = format!("k{n:02}");
        backend
            .create(
                &tenant,
                "DiagnosticReport",
                json!({
                    "resourceType": "DiagnosticReport",
                    "id": format!("c{n:02}"),
                    "status": "final",
                    "code": {"text": "panel"},
                    "contained": [observation(&local, "X", "2020-06-15", &["cat1"])],
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed single container");
    }
    seed_containers(
        backend,
        &tenant,
        vec![(
            "c-multi",
            ["m-a", "m-b", "m-c", "m-d"]
                .into_iter()
                .map(|local| observation(local, "X", "2020-06-15", &["cat1"]))
                .collect(),
        )],
    )
    .await;
    for id in ["top-a", "top-b"] {
        backend
            .create(
                &tenant,
                "Observation",
                observation(id, "X", "2020-06-15", &["cat1"]),
                FhirVersion::default(),
            )
            .await
            .expect("seed top-level observation");
    }

    let mut expected_containers: Vec<String> = (0..12).map(|n| format!("c{n:02}")).collect();
    expected_containers.push("c-multi".to_string());
    expected_containers.sort();
    let mut expected_locals: Vec<String> = (0..12).map(|n| format!("k{n:02}")).collect();
    expected_locals.extend(
        ["m-a", "m-b", "m-c", "m-d"]
            .iter()
            .map(|id| (*id).to_string()),
    );
    expected_locals.sort();
    let mut expected_both = expected_containers.clone();
    expected_both.extend(["top-a".to_string(), "top-b".to_string()]);
    expected_both.sort();
    let mut expected_both_contained = expected_locals.clone();
    expected_both_contained.extend(["top-a".to_string(), "top-b".to_string()]);
    expected_both_contained.sort();

    fn query(mode: ContainedMode, returns: ContainedReturn) -> SearchQuery {
        let mut query = SearchQuery::new("Observation");
        query.contained = mode;
        query.contained_return = returns;
        query.parameters.push(token("code", "X"));
        query.total = Some(TotalMode::Accurate);
        query
    }

    // Positive control: every container is indexed. Polls for the
    // eventually-consistent backends; the strict probes below run once.
    let control = query(On, Container);
    for attempt in 0..60 {
        let got = backend
            .search(&tenant, &control)
            .await
            .map(|found| sorted_ids(&found));
        if got.as_ref().ok() == Some(&expected_containers) {
            break;
        }
        assert!(
            attempt < 59,
            "positive control code=X never held:\n       got {got:?}\n  expected {expected_containers:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // The whole list, its `_total` and `search_count` agree on 13 containers.
    let found = backend
        .search(&tenant, &control)
        .await
        .expect("over-window contained search");
    assert_eq!(
        found.total,
        Some(13),
        "_total must count every container past the window"
    );
    assert_eq!(
        sorted_ids(&found),
        expected_containers,
        "every container is listed once, from both rounds"
    );
    assert_eq!(
        backend
            .search_count(&tenant, &control)
            .await
            .expect("over-window contained search_count"),
        13,
        "search_count agrees with the list"
    );

    // `_count=5` pages cross the round boundary full and without duplicates.
    let mut walked = Vec::new();
    for page_no in 0..10u32 {
        let mut page = control.clone();
        page.count = Some(5);
        page.offset = Some(page_no * 5);
        let found = backend
            .search(&tenant, &page)
            .await
            .expect("over-window contained page");
        if found.resources.items.is_empty() {
            break;
        }
        assert_eq!(
            found.total,
            Some(13),
            "page {page_no}: _total stays 13 across the boundary"
        );
        walked.extend(sorted_ids(&found));
    }
    assert_eq!(
        walked.len(),
        expected_containers.len(),
        "pages are full with no duplicates and stop at the end"
    );
    walked.sort();
    assert_eq!(
        walked, expected_containers,
        "walking _count=5 visits each container exactly once"
    );
    assert_eq!(
        walked.iter().filter(|id| id.as_str() == "c-multi").count(),
        1,
        "c-multi's four hits, split across rounds, collapse to one container"
    );

    // The contained form lists all 16 resources, one per document.
    let contained = query(On, Contained);
    let found = backend
        .search(&tenant, &contained)
        .await
        .expect("over-window contained-resources search");
    assert_eq!(
        found.total,
        Some(16),
        "_total counts every contained resource"
    );
    assert_eq!(
        sorted_ids(&found),
        expected_locals,
        "no contained resource is lost past the window"
    );
    assert_eq!(
        backend
            .search_count(&tenant, &contained)
            .await
            .expect("over-window contained-resources search_count"),
        16,
        "search_count agrees with the contained list"
    );

    // `both` mixes the two top-level matches into the same walked list.
    let both = query(Both, Container);
    let found = backend
        .search(&tenant, &both)
        .await
        .expect("over-window both search");
    assert_eq!(
        found.total,
        Some(15),
        "_total mixes top-level and contained matches once each"
    );
    assert_eq!(
        sorted_ids(&found),
        expected_both,
        "both lists the top-level matches beside the containers"
    );
    assert_eq!(
        backend
            .search_count(&tenant, &both)
            .await
            .expect("over-window both search_count"),
        15,
        "search_count agrees with the both list"
    );

    let both_contained = query(Both, Contained);
    let found = backend
        .search(&tenant, &both_contained)
        .await
        .expect("over-window both-contained search");
    assert_eq!(
        found.total,
        Some(18),
        "_total counts contained and top-level resources together"
    );
    assert_eq!(
        sorted_ids(&found),
        expected_both_contained,
        "both in contained form lists every resource once"
    );
    assert_eq!(
        backend
            .search_count(&tenant, &both_contained)
            .await
            .expect("over-window both-contained search_count"),
        18,
        "search_count agrees with the both-contained list"
    );
}
