//! #1623: shared fixture and view shapes for the SQL-on-FHIR planner /
//! statistics prefix matrices (`test_pg_complex_prefix_planner_matrix`,
//! `test_sqlite_complex_prefix_statistics_matrix`).
//!
//! The fixture is large enough for plans to differ between statistics and
//! planner conditions: a few thousand Patients with several names and
//! addresses (some absent, some without the projected value) and a few
//! hundred QuestionnaireResponses with nested `item` / `answer.item` trees.
//! Many resources share a `last_updated` (groups of consecutive ids plus one
//! large cluster spread across distant ids), first visible values repeat
//! across resources, some first values are missing, and some resources are
//! deleted. [`fixture`] lists resources in reverse id order, so insertion
//! order never coincides with the documented order.
//!
//! `#[path]`-include this file from a test binary; it is not part of
//! `common/mod.rs`.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};

/// One fixture row, written straight into `resources` by the test.
pub struct FixtureResource {
    pub resource_type: &'static str,
    pub id: String,
    /// The stored resource, with `resourceType` and `id` like the backends
    /// store it.
    pub data: Value,
    pub last_updated: DateTime<Utc>,
    pub deleted: bool,
}

/// Small Patients besides the large one.
pub const PATIENTS: usize = 2400;
/// Small QuestionnaireResponses besides the large one.
pub const RESPONSES: usize = 300;

fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

/// Consecutive ids share a timestamp in groups of `group`; `cluster` ids
/// (spread across the whole range) all share the base instant.
fn timestamp(index: usize, group: usize, cluster: bool) -> DateTime<Utc> {
    if cluster {
        base()
    } else {
        base() + chrono::Duration::seconds(1 + (index / group) as i64)
    }
}

fn patient(index: usize) -> Value {
    let mut patient = json!({"resourceType":"Patient","id":format!("pm-{index:05}")});
    match index % 4 {
        0 => patient["gender"] = json!("male"),
        1 => patient["gender"] = json!("female"),
        2 => patient["gender"] = json!("other"),
        _ => {}
    }
    match index % 5 {
        0 => {}
        // A name without `family`: a NULL family on a present name.
        1 => patient["name"] = json!([{"given":["Solo"]}]),
        names => {
            patient["name"] = json!(
                (0..names - 1)
                    .map(|n| json!({
                        "family": format!("F-{:02}", (index * 7 + n) % 40),
                        "given": (0..=n).map(|g| format!("G-{n}-{g}")).collect::<Vec<_>>()
                    }))
                    .collect::<Vec<_>>()
            );
        }
    }
    let addresses = index % 3;
    if addresses > 0 {
        patient["address"] = json!(
            (0..addresses)
                .map(|m| if m == 1 && index.is_multiple_of(9) {
                    json!({"line":["no city"]})
                } else {
                    json!({"city": format!("C-{}", (index + m) % 11)})
                })
                .collect::<Vec<_>>()
        );
    }
    patient
}

fn large_patient() -> Value {
    json!({
        "resourceType":"Patient", "id":"p-large", "gender":"male",
        "name": (1..=150).map(|index| json!({
            "family":format!("Family-{index}"),
            "given":[format!("Given-{index}-a"),format!("Given-{index}-b")]
        })).collect::<Vec<_>>(),
        "address":(1..=10).map(|index| json!({"city":format!("City-{index}")})).collect::<Vec<_>>()
    })
}

fn response(index: usize) -> Value {
    let id = format!("qm-{index:04}");
    let items: Vec<Value> = (0..index % 4 + 1)
        .map(|n| {
            let mut item = json!({});
            // One item per tenth response has no linkId: a NULL first value.
            if !(index.is_multiple_of(10) && n == 0) {
                item["linkId"] = json!(format!("L-{:02}", (index + n) % 15));
            }
            if n % 2 == 0 {
                item["answer"] = json!([{
                    "valueString": format!("A-{index}-{n}"),
                    "item": [{"linkId": format!("L-{n}-child"),
                        "item": [{"linkId": "L-grand"}]}]
                }]);
            } else {
                item["item"] = json!([{"linkId": format!("L-{n}-sub")}]);
            }
            item
        })
        .collect();
    json!({"resourceType":"QuestionnaireResponse","id":id,
        "status": if index % 3 == 2 { "in-progress" } else { "completed" },
        "item": items})
}

fn large_response() -> Value {
    json!({"resourceType":"QuestionnaireResponse","id":"qr-large","status":"completed",
        "item":(1..=150).map(|index| json!({
            "linkId":format!("Item-{index}"),
            "answer":[{"valueString":format!("Answer-{index}"),
                "item":[{"linkId":format!("Child-{index}")}]}]
        })).collect::<Vec<_>>()})
}

/// The fixture in reverse id order (per type, Patients first).
pub fn fixture() -> Vec<FixtureResource> {
    let mut resources = Vec::new();
    let mut patients: Vec<FixtureResource> = (0..PATIENTS)
        .map(|index| FixtureResource {
            resource_type: "Patient",
            id: format!("pm-{index:05}"),
            data: patient(index),
            last_updated: timestamp(index, 8, index % 100 < 3),
            deleted: index % 37 == 5,
        })
        .collect();
    patients.push(FixtureResource {
        resource_type: "Patient",
        id: "p-large".into(),
        data: large_patient(),
        last_updated: base(),
        deleted: false,
    });
    let mut responses: Vec<FixtureResource> = (0..RESPONSES)
        .map(|index| FixtureResource {
            resource_type: "QuestionnaireResponse",
            id: format!("qm-{index:04}"),
            data: response(index),
            last_updated: timestamp(index, 6, index % 50 < 2),
            deleted: index % 41 == 7,
        })
        .collect();
    responses.push(FixtureResource {
        resource_type: "QuestionnaireResponse",
        id: "qr-large".into(),
        data: large_response(),
        last_updated: base(),
        deleted: false,
    });
    for group in [&mut patients, &mut responses] {
        group.sort_by(|a, b| b.id.cmp(&a.id));
    }
    resources.extend(patients);
    resources.extend(responses);
    resources
}

fn tie(value: &str) -> Value {
    json!([{"path":"'tie'","name":"tie"},{"path":value,"name":"value"}])
}

fn view(resource: &str, select: Value) -> Value {
    json!({"resourceType":"ViewDefinition","resource":resource,"select":select})
}

/// `(case, view, minimum unlimited rows)`: every complex SQL shape whose
/// order the matrices pin.
pub fn shapes() -> Vec<(&'static str, Value, usize)> {
    vec![
        (
            // The issue's exact nullable case: no id in the projection.
            "nullable-forEachOrNull",
            view(
                "Patient",
                json!([{"forEachOrNull":"name","column":[{"path":"family","name":"family"}]}]),
            ),
            PATIENTS,
        ),
        (
            "cartesian",
            view(
                "Patient",
                json!([{"column":[{"path":"id","name":"id"}]},
                    {"forEach":"name","column":[{"path":"family","name":"family"}]},
                    {"forEach":"address","column":[{"path":"city","name":"city"}]}]),
            ),
            1500,
        ),
        (
            "union-equal-first-column",
            view(
                "Patient",
                json!([{"unionAll":[
                    {"column":tie("id")},
                    {"forEach":"name","column":tie("family")}
                ]}]),
            ),
            PATIENTS,
        ),
        (
            "union-nullable-first-column",
            view(
                "Patient",
                json!([{"unionAll":[
                    {"column":[{"path":"gender","name":"v"},{"path":"'resource'","name":"kind"}]},
                    {"forEachOrNull":"name","column":[{"path":"family","name":"v"},
                        {"path":"'name'","name":"kind"}]}
                ]}]),
            ),
            PATIENTS,
        ),
        (
            "outer-foreach-union",
            view(
                "Patient",
                json!([{"forEach":"name","unionAll":[
                    {"column":tie("family")},
                    {"forEach":"given","column":tie("$this")}
                ]}]),
            ),
            PATIENTS,
        ),
        (
            "union-with-repeat-branch",
            view(
                "QuestionnaireResponse",
                json!([{"unionAll":[
                    {"repeat":["item"],"column":tie("linkId")},
                    {"column":tie("id")}
                ]}]),
            ),
            RESPONSES,
        ),
        (
            "multi-path-repeat-post-foreach",
            view(
                "QuestionnaireResponse",
                json!([{"repeat":["item","answer.item"],"select":[
                    {"column":[{"path":"linkId","name":"link"},
                        {"path":"%rowIndex","name":"i","type":"integer"}]},
                    {"forEachOrNull":"answer","column":[{"path":"valueString","name":"ans"},
                        {"path":"%rowIndex","name":"ans_i","type":"integer"}]}
                ]}]),
            ),
            RESPONSES,
        ),
        (
            "indexed",
            view(
                "Patient",
                json!([{"column":[{"path":"id","name":"id"}]},
                    {"forEachOrNull":"name[1]","column":[{"path":"family","name":"family"},
                        {"path":"%rowIndex","name":"index","type":"integer"}]}]),
            ),
            PATIENTS / 2,
        ),
    ]
}

/// The shapes whose plans the matrices log per condition.
pub const EXPLAINED_SHAPES: [&str; 3] = [
    "cartesian",
    "union-with-repeat-branch",
    "multi-path-repeat-post-foreach",
];
