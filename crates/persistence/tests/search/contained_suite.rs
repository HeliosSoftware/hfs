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
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::error::StorageError;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    CompositeSearchComponent, ContainedMode, ContainedReturn, SearchModifier, SearchParamType,
    SearchParameter, SearchPrefix, SearchQuery, SearchValue,
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
    expect: Expect,
}

impl Case {
    fn new(label: &'static str, parameters: Vec<SearchParameter>, expect: Expect) -> Self {
        Self {
            label,
            mode: ContainedMode::On,
            returns: ContainedReturn::Container,
            parameters,
            expect,
        }
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
            (Expect::IdsOrRejected(_, name), Err(message)) => message.contains(name),
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
///   R, `valueQuantity` 7 mg
/// - `m-plain`: `p1` — no `meta`, `valueQuantity` 3 mg
/// - `m-str`: `s1` — `valueString` "hello"
/// - `m-other`: `o1` — code Y with text "Glucose level", `valueString`
///   "Hello World"
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
    let mut plain = observation("p1", "X", "2020-06-15", &["cat1"]);
    plain["valueQuantity"] = quantity(3.0);
    let mut string = observation("s1", "X", "2020-06-15", &["cat1"]);
    string["valueString"] = json!("hello");
    let mut other = observation("o1", "Y", "2020-06-15", &["cat1"]);
    other["code"]["text"] = json!("Glucose level");
    other["valueString"] = json!("Hello World");

    seed_containers(
        backend,
        &tenant,
        vec![
            ("m-tagged", vec![tagged]),
            ("m-plain", vec![plain]),
            ("m-str", vec![string]),
            ("m-other", vec![other]),
        ],
    )
    .await;

    let value_string = |value: &str| literal("value-string", SearchParamType::String, value);
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
        // 3. Modifiers.
        Case::new(
            "code:not=X",
            vec![with_modifier(code_x(), SearchModifier::Not)],
            Expect::IdsOrRejected(&["m-other"], "code"),
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
            Expect::IdsOrRejected(&["m-plain", "m-tagged"], "value-string"),
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
    ];

    assert_cases(backend, &tenant, &controls, &cases).await;
}
