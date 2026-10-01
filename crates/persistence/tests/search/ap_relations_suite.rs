//! Backend-agnostic suite for the `ap` (approximately) prefix on the
//! quantity component of a composite parameter (issue #1390):
//! `code-value-quantity` and `component-code-value-quantity`, on every
//! backend. A component gets the window a plain quantity parameter gets,
//! `[v - m, v + m]` with `m = max(10% of |v|, half the implicit precision)`,
//! which `ap_prefix_suite.rs` pins for plain parameters.
//!
//! Each table starts with a positive control (a range no window is involved
//! in), which fails loudly when the backend was built without the spec search
//! parameters. Included by `#[path]` into each backend's test binary, like
//! `ap_prefix_suite.rs`.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

const LOINC: &str = "http://loinc.org";
const UCUM: &str = "http://unitsofmeasure.org";

// Two codes, so that a composite whose token component picks the wrong
// observation is visible.
const CODE_A: &str = "ap-code-a";
const CODE_B: &str = "ap-code-b";

fn tenant(base: &str) -> TenantContext {
    TenantContext::new(TenantId::new(base), TenantPermissions::full_access())
}

fn ids(expected: &[&str]) -> BTreeSet<String> {
    expected.iter().map(|id| id.to_string()).collect()
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

/// Polls `control` until it returns `expected`: Elasticsearch is
/// near-real-time. A failure means the parameter did not index — a backend
/// built without the spec search parameters — not that `ap` is wrong.
async fn positive_control<S>(
    backend: &S,
    tenant: &TenantContext,
    control: &SearchQuery,
    expected: &[&str],
    label: &str,
) where
    S: ResourceStorage + SearchProvider,
{
    let mut got = BTreeSet::new();
    for _ in 0..60 {
        got = matched(backend, tenant, control).await;
        if got == ids(expected) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(got, ids(expected), "positive control {label}");
}

async fn create<S>(backend: &S, tenant: &TenantContext, resource_type: &str, resource: Value)
where
    S: ResourceStorage + SearchProvider,
{
    let id = resource["id"].as_str().unwrap_or_default().to_string();
    backend
        .create(tenant, resource_type, resource, FhirVersion::default())
        .await
        .unwrap_or_else(|e| panic!("create {resource_type}/{id} failed: {e}"));
}

fn code_component() -> CompositeSearchComponent {
    CompositeSearchComponent {
        param_type: SearchParamType::Token,
        param_name: "code".to_string(),
    }
}

/// A composite query with the components the registry would supply. The value
/// is taken literally: the backend splits it on `$` and reads each part's
/// prefix itself.
fn composite(param: &str, components: Vec<CompositeSearchComponent>, value: &str) -> SearchQuery {
    SearchQuery::new("Observation")
        .with_parameter(SearchParameter {
            name: param.to_string(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq(value)],
            components,
            ..Default::default()
        })
        .with_count(100)
}

fn code_value_quantity(value: &str) -> SearchQuery {
    composite(
        "code-value-quantity",
        vec![
            code_component(),
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "value-quantity".to_string(),
            },
        ],
        value,
    )
}

fn component_code_value_quantity(value: &str) -> SearchQuery {
    composite(
        "component-code-value-quantity",
        vec![
            CompositeSearchComponent {
                param_type: SearchParamType::Token,
                param_name: "component-code".to_string(),
            },
            CompositeSearchComponent {
                param_type: SearchParamType::Quantity,
                param_name: "component-value-quantity".to_string(),
            },
        ],
        value,
    )
}

const COMPOSITE_QUANTITIES: &[(&str, &str, f64)] = &[
    ("ap-cq-a-089", CODE_A, 89.0),
    ("ap-cq-a-091", CODE_A, 91.0),
    ("ap-cq-a-100", CODE_A, 100.0),
    ("ap-cq-a-109", CODE_A, 109.0),
    ("ap-cq-a-111", CODE_A, 111.0),
    // Inside the window, under the other code.
    ("ap-cq-b-091", CODE_B, 91.0),
    ("ap-cq-b-100", CODE_B, 100.0),
    ("ap-cq-b-160", CODE_B, 160.0),
];

/// `code-value-quantity=` value → the Observations it must match. The first
/// row is the positive control.
const CODE_VALUE_QUANTITY_CASES: &[(&str, &[&str])] = &[
    (
        "ap-code-a$ge0",
        &[
            "ap-cq-a-089",
            "ap-cq-a-091",
            "ap-cq-a-100",
            "ap-cq-a-109",
            "ap-cq-a-111",
        ],
    ),
    // 10% of the value: [90, 110]. The other code's 91 and 100 stay out.
    (
        "ap-code-a$ap100",
        &["ap-cq-a-091", "ap-cq-a-100", "ap-cq-a-109"],
    ),
    ("ap-code-b$ap100", &["ap-cq-b-091", "ap-cq-b-100"]),
    // One significant figure: [50, 150].
    (
        "ap-code-a$ap1e2",
        &[
            "ap-cq-a-089",
            "ap-cq-a-091",
            "ap-cq-a-100",
            "ap-cq-a-109",
            "ap-cq-a-111",
        ],
    ),
    ("ap-code-b$ap1e2", &["ap-cq-b-091", "ap-cq-b-100"]),
    // Nothing of code A is near 160, though code B has one.
    ("ap-code-a$ap160", &[]),
    ("ap-code-b$ap160", &["ap-cq-b-160"]),
];

/// (id, [(component code, value in mg)]). Both components of an Observation
/// are in the window of one of the queries below, but only when they belong
/// to the same component do they match together.
const COMPONENT_OBSERVATIONS: &[(&str, [(&str, f64); 2])] = &[
    ("ap-cc-1", [(CODE_A, 100.0), (CODE_B, 200.0)]),
    ("ap-cc-2", [(CODE_A, 200.0), (CODE_B, 100.0)]),
    ("ap-cc-3", [(CODE_A, 109.0), (CODE_B, 91.0)]),
    ("ap-cc-4", [(CODE_A, 160.0), (CODE_B, 160.0)]),
];

/// `component-code-value-quantity=` value → the Observations it must match.
/// The first row is the positive control.
const COMPONENT_CASES: &[(&str, &[&str])] = &[
    (
        "ap-code-a$ge0",
        &["ap-cc-1", "ap-cc-2", "ap-cc-3", "ap-cc-4"],
    ),
    // `ap-cc-2` has a 100 too, but under the other code.
    ("ap-code-a$ap100", &["ap-cc-1", "ap-cc-3"]),
    ("ap-code-b$ap100", &["ap-cc-2", "ap-cc-3"]),
    ("ap-code-a$ap200", &["ap-cc-2"]),
    ("ap-code-b$ap160", &["ap-cc-4"]),
];

/// Seeds the Observations under a caller-unique tenant and asserts the tables.
pub async fn ap_composite<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = tenant(tenant_base);

    for (id, code, dose) in COMPOSITE_QUANTITIES {
        create(
            backend,
            &tenant,
            "Observation",
            json!({
                "id": id,
                "status": "final",
                "code": {"coding": [{"system": LOINC, "code": code}]},
                "valueQuantity": {
                    "value": dose, "unit": "mg", "system": UCUM, "code": "mg",
                },
            }),
        )
        .await;
    }
    for (id, components) in COMPONENT_OBSERVATIONS {
        let components: Vec<Value> = components
            .iter()
            .map(|(code, dose)| {
                json!({
                    "code": {"coding": [{"system": LOINC, "code": code}]},
                    "valueQuantity": {
                        "value": dose, "unit": "mg", "system": UCUM, "code": "mg",
                    },
                })
            })
            .collect();
        create(
            backend,
            &tenant,
            "Observation",
            json!({
                "id": id,
                "status": "final",
                "code": {"coding": [{"system": LOINC, "code": "ap-panel"}]},
                "component": components,
            }),
        )
        .await;
    }
    let (value, expected) = CODE_VALUE_QUANTITY_CASES[0];
    positive_control(
        backend,
        &tenant,
        &code_value_quantity(value),
        expected,
        &format!("Observation?code-value-quantity={value}"),
    )
    .await;
    let (value, expected) = COMPONENT_CASES[0];
    positive_control(
        backend,
        &tenant,
        &component_code_value_quantity(value),
        expected,
        &format!("Observation?component-code-value-quantity={value}"),
    )
    .await;
    let mut failures = Vec::new();
    for (value, expected) in CODE_VALUE_QUANTITY_CASES {
        let got = matched(backend, &tenant, &code_value_quantity(value)).await;
        if got != ids(expected) {
            failures.push(format!(
                "Observation?code-value-quantity={value}: got {got:?}, expected {expected:?}"
            ));
        }
    }
    for (value, expected) in COMPONENT_CASES {
        let got = matched(backend, &tenant, &component_code_value_quantity(value)).await;
        if got != ids(expected) {
            failures.push(format!(
                "Observation?component-code-value-quantity={value}: got {got:?}, \
                 expected {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
