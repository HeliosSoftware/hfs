//! Pure SQL-on-FHIR fixtures and independent evaluator expectations.
//!
//! Included by persistence and REST tests. Storage adapters seed the records
//! and may replace `last_updated` with the timestamp returned by a write.
//! No SQL, pools, or backend-specific dependencies belong in this module.

#![allow(dead_code)]

use std::collections::HashSet;

use chrono::{DateTime, TimeZone, Utc};
use helios_fhir::FhirVersion;
use helios_persistence::core::sof_runner::ViewFilters;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct FixtureResource {
    pub resource_type: &'static str,
    pub id: String,
    pub data: Value,
    /// Storage metadata, deliberately independent of the JSON's meta field.
    pub last_updated: DateTime<Utc>,
    pub deleted: bool,
}

#[derive(Clone)]
pub struct OrderingCase {
    pub name: &'static str,
    pub view: Value,
    pub filters: ViewFilters,
}

pub fn base_timestamp() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
}

fn record(resource_type: &'static str, data: Value) -> FixtureResource {
    FixtureResource {
        resource_type,
        id: data["id"].as_str().unwrap().to_string(),
        data,
        last_updated: base_timestamp(),
        deleted: false,
    }
}

/// The issue's original fixture: 150 names, 300 givens, ten addresses,
/// one resource with no names and one with only a rejected name.
pub fn exact_fixture() -> Vec<FixtureResource> {
    vec![
        record(
            "Patient",
            json!({"resourceType":"Patient", "id":"p-large", "gender":"male", "active":true,
            "name":(1..=150).map(|i| json!({"family":format!("Family-{i}"),
                "use":if i <= 75 {"official"} else {"temp"},
                "given":[format!("Given-{i}-a"),format!("Given-{i}-b")]})).collect::<Vec<_>>(),
            "address":(1..=10).map(|i| json!({"city":format!("City-{i}")})).collect::<Vec<_>>() }),
        ),
        record("Patient", json!({"resourceType":"Patient","id":"p-empty"})),
        record(
            "Patient",
            json!({"resourceType":"Patient","id":"p-filtered",
            "name":[{"family":"Rejected","use":"temp"}]}),
        ),
    ]
}

/// Tied timestamps, duplicate values, absent leaves, deletion and reversed
/// insertion order. Case and punctuation exercise bytewise resource ids.
pub fn fixture() -> Vec<FixtureResource> {
    let mut records = exact_fixture();
    for i in 0..240 {
        let mut data = json!({"resourceType":"Patient","id":format!("pm-{i:04}"),
            "active":true,"gender":if i % 2 == 0 {"male"} else {"female"}});
        if i % 5 != 0 {
            data["name"] = json!(
                (0..i % 4 + 1)
                    .map(|n| {
                        if i % 7 == 0 && n == 0 {
                            json!({"given":["Solo"],"use":"official"})
                        } else {
                            json!({"family":format!("F-{}", (i + n) % 11),
                        "use":if n % 2 == 0 {"official"} else {"temp"},
                        "given":[format!("G-{n}-z"),format!("G-{n}-a")]})
                        }
                    })
                    .collect::<Vec<_>>()
            );
        }
        if i % 3 != 0 {
            data["address"] = json!([{"city":"City-z"},{"city":"City-a"}]);
        }
        let mut resource = record("Patient", data);
        resource.last_updated += chrono::Duration::seconds((i / 8) as i64);
        resource.deleted = i % 37 == 5;
        records.push(resource);
    }
    for id in ["a-2", "A-2", "a.1", "A.1", "Z-1", "z-1"] {
        records.push(record(
            "Patient",
            json!({"resourceType":"Patient","id":id,
            "gender":"male","name":[{"family":"duplicate","use":"official","given":["z","a"]}]}),
        ));
    }
    records.push(record(
        "Group",
        json!({"resourceType":"Group","id":"g-members","type":"person","actual":true,
        "member":[{"entity":{"reference":"Patient/pm-0005"}},
            {"entity":{"reference":"Patient/p-large"}},
            {"entity":{"reference":"Patient/A-2"}},
            {"entity":{"reference":"Patient/p-large"}},
            {"entity":{"reference":"Practitioner/ignored"}}]}),
    ));
    records.push(record(
        "Group",
        json!({"resourceType":"Group","id":"g-empty","type":"person","actual":true}),
    ));
    let mut deleted_group = record(
        "Group",
        json!({"resourceType":"Group","id":"g-deleted","type":"person","actual":true,
        "member":[{"entity":{"reference":"Patient/p-large"}}]}),
    );
    deleted_group.deleted = true;
    records.push(deleted_group);
    records.sort_by(|a, b| b.id.as_bytes().cmp(a.id.as_bytes()));
    records
}

pub fn patient_view(select: Value) -> Value {
    json!({"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":select})
}

/// Only flat and forEach shapes carry the global ordering contract.
pub fn ordering_cases() -> Vec<OrderingCase> {
    let case = |name, select| OrderingCase {
        name,
        view: patient_view(select),
        filters: ViewFilters::default(),
    };
    let mut cases = vec![
        case(
            "flat",
            json!([{"column":[{"path":"id","name":"r"},{"path":"gender","name":"u0"}]}]),
        ),
        case(
            "foreach",
            json!([{"column":[{"path":"id","name":"id"}]},
            {"forEach":"name","column":[{"path":"family","name":"family"}]}]),
        ),
        case(
            "nested",
            json!([{"forEach":"name","select":[
            {"column":[{"path":"family","name":"family"}]},
            {"forEach":"given","column":[{"path":"$this","name":"given","type":"string"}]}]}]),
        ),
        case(
            "chained",
            json!([{"forEach":"name.given","column":[{"path":"$this","name":"given","type":"string"}]}]),
        ),
        case(
            "cartesian",
            json!([{"column":[{"path":"id","name":"id"}]},
            {"forEach":"name","column":[{"path":"family","name":"family"}]},
            {"forEach":"address","column":[{"path":"city","name":"city"}]}]),
        ),
        case(
            "nullable",
            json!([{"forEachOrNull":"name","column":[{"path":"family","name":"family"}]}]),
        ),
        case(
            "nullable-filtered",
            json!([{"forEachOrNull":"name.where(use = 'official')",
            "column":[{"path":"family","name":"family"}]}]),
        ),
        case(
            "element-filter",
            json!([{"forEach":"name.where(use = 'official')",
            "column":[{"path":"family","name":"family"}]}]),
        ),
    ];
    let mut constants = case(
        "constant-and-since",
        json!([{"column":[{"path":"id","name":"id"},{"path":"%marker","name":"marker"}]},
        {"forEachOrNull":"name","column":[{"path":"family","name":"family"}]}]),
    );
    constants.view["constant"] = json!([{"name":"unused","valueString":"not bound"},
        {"name":"gender","valueString":"male"},{"name":"marker","valueString":"resources r"}]);
    constants.view["where"] = json!([{"path":"gender = %gender"}]);
    constants.filters.since = Some(base_timestamp() + chrono::Duration::seconds(1));
    cases.push(constants);
    let mut patients = cases[1].clone();
    patients.name = "patient";
    patients.filters.patient = vec![
        "p-large".into(),
        "Patient/A-2".into(),
        "Patient/pm-0005".into(),
    ];
    cases.push(patients);
    let mut groups = cases[1].clone();
    groups.name = "group-and-explicit-patient";
    groups.filters.group = vec!["g-members".into(), "Group/unknown".into()];
    groups.filters.patient = vec!["Patient/a.1".into()];
    cases.push(groups);
    let mut empty = cases[0].clone();
    empty.name = "empty-group";
    empty.filters.group = vec!["Group/g-empty".into()];
    cases.push(empty);
    let mut deleted = cases[0].clone();
    deleted.name = "deleted-group";
    deleted.filters.group = vec!["g-deleted".into()];
    cases.push(deleted);
    cases.push(case(
        "nested-cartesian",
        json!([
        {"column":[{"path":"id","name":"id"}]},
        {"forEach":"name","select":[{"column":[{"path":"family","name":"family"}]},
            {"forEach":"given","column":[{"path":"$this","name":"given","type":"string"}]}]},
        {"forEach":"address","column":[{"path":"city","name":"city"}]}]),
    ));

    cases
}

/// One hundred and fifty deliberately non-lexical elements and JSON-looking
/// strings. HumanName objects also detect SQLite JSON subtype loss.
pub fn cell_fixture() -> Vec<FixtureResource> {
    let names: Vec<Value> = (0..150).map(|i| json!({
        "family":if i == 0 {"Skipped".into()} else if i == 1 {"Zulu".into()} else {format!("A-{i:03}")},
        "use":if i == 0 {"old"} else {"official"},
        "given":[format!("z-{i}"),"null","true","0123",format!("a-{i}")]
    })).collect();
    let telecom:Vec<_>=(0..150).map(|i| json!({"system":if i==0 {"email"} else {"phone"},
        "value":match i {0=>"Skipped".into(),1=>"Zulu".into(),2=>"null".into(),3=>"true".into(),4=>"0123".into(),_=>format!("A-{i:03}")}})).collect();
    vec![
        record(
            "Patient",
            json!({"resourceType":"Patient","id":"cells","name":names,"gender":"male",
                "contact":[{"name":{"family":"Contact-z"},"telecom":telecom.clone()},
                    {"name":{"family":"Contact-a"},"telecom":telecom}]}),
        ),
        record(
            "Patient",
            json!({"resourceType":"Patient","id":"no-match","name":[{"family":"none","use":"old"}]}),
        ),
        record(
            "Patient",
            json!({"resourceType":"Patient","id":"empty-cells"}),
        ),
        record(
            "QuestionnaireResponse",
            json!({"resourceType":"QuestionnaireResponse","id":"qr-cells","status":"completed",
            "item":[{"linkId":"root","answer":(0..150).map(|i| json!({"valueString":format!("z-{i}")})).collect::<Vec<_>>(),
                "item":[{"linkId":"child","answer":[{"valueString":"null"},{"valueString":"true"},{"valueString":"0123"}]}]}]}),
        ),
    ]
}

/// `(name, SQL view, evaluator view, ordered rows)`. Bare where-pick uses the
/// explicit first() oracle because its existing SQL contract is scalar.
pub fn cell_cases() -> Vec<(&'static str, Value, Value, bool)> {
    let columns = json!([
        {"path":"name","name":"objects","collection":true},
        {"path":"name.family","name":"families","collection":true},
        {"path":"name.given","name":"givens","collection":true},
        {"path":"name.given.join('|')","name":"joined","type":"string"},
        {"path":"name.where(use = 'official').first().family","name":"pick","type":"string"}
    ]);
    let flat = patient_view(json!([{"column":columns.clone()}]));
    // Focus-local `given.join()` is an existing identity lowering outside
    // this issue. Use the supported JoinAggregate invocation shape instead.
    let foreach = patient_view(json!([{"forEach":"contact","column":[
        {"path":"name.family","name":"family"},
        {"path":"telecom","name":"objects","collection":true},
        {"path":"telecom.value","name":"values","collection":true},
        {"path":"telecom.value.join('|')","name":"joined","type":"string"},
        {"path":"telecom.where(system = 'phone').first().value","name":"pick","type":"string"}]}]));
    let union = patient_view(json!([{"unionAll":[{"column":columns.clone()},{"column":columns}]}]));
    let repeat = json!({"resourceType":"ViewDefinition","resource":"QuestionnaireResponse","select":[
        {"column":[{"path":"id","name":"id"},{"path":"item.linkId","name":"root_links","collection":true}]},
        {"repeat":["item"],"column":[{"path":"linkId","name":"link"},
            {"path":"answer","name":"objects","collection":true},
            {"path":"answer.valueString","name":"answers","collection":true},
            {"path":"answer.valueString.join('|')","name":"joined","type":"string"},
            {"path":"answer.where(valueString != 'absent').first().valueString","name":"pick","type":"string"}]}]});
    let first = patient_view(
        json!([{"column":[{"path":"name.where(use = 'official').first().family","name":"pick","type":"string"}]}]),
    );
    let bare = patient_view(
        json!([{"column":[{"path":"name.where(use = 'official').family","name":"pick","type":"string"}]}]),
    );
    vec![
        ("flat-cells", flat.clone(), flat, true),
        ("foreach-cells", foreach.clone(), foreach, true),
        ("union-cells", union.clone(), union, false),
        ("repeat-cells", repeat.clone(), repeat, false),
        ("first-pick", first.clone(), first.clone(), true),
        ("bare-pick", bare, first, true),
    ]
}

/// Compute before any SQL. Filter using storage metadata, order eligible
/// resources once, then evaluate each resource and concatenate its rows.
/// Output rows and cells are never sorted by the oracle.
pub fn evaluator_oracle(
    view: &Value,
    resources: &[FixtureResource],
    filters: &ViewFilters,
) -> Vec<Value> {
    let mut targets: HashSet<String> = filters
        .patient
        .iter()
        .map(|r| {
            if r.starts_with("Patient/") {
                r.clone()
            } else {
                format!("Patient/{r}")
            }
        })
        .collect();
    if !filters.group.is_empty() {
        let groups: Vec<_> = resources
            .iter()
            .filter(|r| r.resource_type == "Group" && !r.deleted)
            .map(|r| r.data.clone())
            .collect();
        targets.extend(helios_sof::resolve_group_members_to_patient_refs(
            &filters.group,
            &groups,
        ));
        if targets.is_empty() {
            return Vec::new();
        }
    }
    let mut eligible: Vec<_> = resources
        .iter()
        .filter(|r| {
            !r.deleted
                && Some(r.resource_type) == view["resource"].as_str()
                && filters.since.is_none_or(|since| r.last_updated >= since)
                && (targets.is_empty()
                    || helios_sof::resource_in_patient_compartment(
                        &r.data,
                        &targets,
                        FhirVersion::R4,
                    )
                    .expect("compartment oracle"))
        })
        .collect();
    eligible.sort_by(|a, b| {
        a.last_updated
            .cmp(&b.last_updated)
            .then_with(|| a.id.as_bytes().cmp(b.id.as_bytes()))
    });
    let resources = eligible.iter().map(|r| r.data.clone()).collect::<Vec<_>>();
    let original = evaluator_rows(view, &resources);
    if legacy_shape(view) {
        return original;
    }
    let ordered = root_factor_rows(view, &resources);
    assert_multiset_strict(
        &ordered,
        &original,
        "root factor oracle preserves original complete rows/cells",
    );
    ordered
}

fn legacy_shape(node: &Value) -> bool {
    match node {
        Value::Object(object) => {
            object.contains_key("unionAll")
                || object.contains_key("repeat")
                || object.values().any(legacy_shape)
        }
        Value::Array(array) => array.iter().any(legacy_shape),
        _ => false,
    }
}

/// The in-process engine expands a newly declared sibling outside previous
/// combinations. The SQL contract keeps the first declaration outside the
/// next one. Evaluate each root branch independently for the same resource,
/// then generate its Cartesian product left-outer/right-inner. This does not
/// sort rows or cells, change the view input, or derive expectations from SQL.
/// Nested/chained branches with one producing child per scope stay verbatim;
/// nested sibling products require their own model and are rejected here.
fn root_factor_rows(view: &Value, resources: &[Value]) -> Vec<Value> {
    fn producing(node: &Value) -> bool {
        node.get("forEach").is_some()
            || node.get("forEachOrNull").is_some()
            || node
                .get("select")
                .and_then(Value::as_array)
                .is_some_and(|children| children.iter().any(producing))
    }
    fn validate(node: &Value) {
        if let Some(children) = node.get("select").and_then(Value::as_array) {
            assert!(
                children.iter().filter(|child| producing(child)).count() <= 1,
                "root factor oracle does not support nested multi-producing scopes"
            );
            for child in children {
                validate(child);
            }
        }
    }
    let branches = view["select"].as_array().expect("root select branches");
    let prepared: Vec<_> = branches
        .iter()
        .map(|branch| {
            validate(branch);
            let mut branch_view = view.clone();
            branch_view["select"] = json!([branch]);
            prepare_evaluator(&branch_view)
        })
        .collect();
    let mut output = Vec::new();
    for (index, resource) in resources.iter().enumerate() {
        let mut product = vec![json!({})];
        for branch in &prepared {
            let right = evaluate_resource(branch, resource, index);
            let mut next = Vec::new();
            for left in &product {
                for row in &right {
                    let mut merged = left.as_object().unwrap().clone();
                    for (name, value) in row.as_object().unwrap() {
                        assert!(
                            merged.insert(name.clone(), value.clone()).is_none(),
                            "root factor oracle requires disjoint complete branch column sets: {name}"
                        );
                    }
                    next.push(Value::Object(merged));
                }
            }
            product = next;
        }
        output.extend(product);
    }
    output
}

fn prepare_evaluator(view: &Value) -> helios_sof::PreparedViewDefinition {
    let view = helios_sof::parse_view_definition_for_version(view.clone(), FhirVersion::R4)
        .expect("parse oracle view");
    helios_sof::PreparedViewDefinition::new(view).expect("prepare oracle view")
}

fn evaluate_resource(
    prepared: &helios_sof::PreparedViewDefinition,
    resource: &Value,
    index: usize,
) -> Vec<Value> {
    let result = prepared
        .process_chunk(helios_sof::ResourceChunk {
            resources: vec![resource.clone()],
            chunk_index: index,
            is_last: true,
        })
        .expect("evaluate oracle resource");
    result
        .rows
        .into_iter()
        .map(|row| {
            assert_eq!(
                result.columns.len(),
                row.values.len(),
                "complete oracle column set"
            );
            Value::Object(
                result
                    .columns
                    .iter()
                    .cloned()
                    .zip(row.values.into_iter().map(|v| v.unwrap_or(Value::Null)))
                    .collect(),
            )
        })
        .collect()
}

/// Original evaluator output, verbatim per resource; legacy union/repeat
/// cell tests use this and only assert the strict multiset of complete rows.
pub fn evaluator_rows(view: &Value, resources: &[Value]) -> Vec<Value> {
    let prepared = prepare_evaluator(view);
    resources
        .iter()
        .enumerate()
        .flat_map(|(index, resource)| evaluate_resource(&prepared, resource, index))
        .collect()
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    #[test]
    fn total_order_oracle_root_factor_preserves_cartesian_cells_and_multiplicity() {
        let fixture = exact_fixture();
        let view = ordering_cases()[4].view.clone();
        let actual = evaluator_oracle(&view, &fixture, &ViewFilters::default());
        assert_eq!(
            actual[0],
            json!({"id":"p-large","family":"Family-1","city":"City-1"})
        );
        assert_eq!(
            actual[1],
            json!({"id":"p-large","family":"Family-1","city":"City-2"})
        );
        let original = evaluator_rows(&view, &[fixture[0].data.clone()]);
        assert_eq!(
            original[1]["family"], "Family-2",
            "pin original evaluator order rather than sorting it"
        );
        assert_multiset_strict(&actual, &original, "root Cartesian factor");
    }

    #[test]
    fn total_order_oracle_single_nested_chained_and_legacy_keep_evaluator_output() {
        let fixture = exact_fixture();
        let resources: Vec<_> = fixture.iter().map(|r| r.data.clone()).collect();
        let cases = ordering_cases();
        for index in [2, 3] {
            assert_ordered(
                &root_factor_rows(&cases[index].view, &resources),
                &evaluator_rows(&cases[index].view, &resources),
                "single nested/chained",
            );
        }
        let fixture = cell_fixture();
        for (_, view, _, ordered) in cell_cases() {
            if !ordered {
                assert!(legacy_shape(&view));
                let mut resources: Vec<_> = fixture
                    .iter()
                    .filter(|r| Some(r.resource_type) == view["resource"].as_str())
                    .collect();
                resources.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));
                assert_ordered(
                    &evaluator_oracle(&view, &fixture, &ViewFilters::default()),
                    &evaluator_rows(
                        &view,
                        &resources.iter().map(|r| r.data.clone()).collect::<Vec<_>>(),
                    ),
                    "legacy evaluator remains verbatim",
                );
            }
        }
    }

    #[test]
    fn total_order_oracle_root_nested_cartesian_preserves_branch_occurrences() {
        let fixture = exact_fixture();
        let view = ordering_cases().last().unwrap().view.clone();
        let actual = evaluator_oracle(&view, &fixture, &ViewFilters::default());
        assert_eq!(actual.len(), 3000);
        assert_eq!(
            actual[0],
            json!({"id":"p-large","family":"Family-1","given":"Given-1-a","city":"City-1"})
        );
        assert_eq!(
            actual[1],
            json!({"id":"p-large","family":"Family-1","given":"Given-1-a","city":"City-2"})
        );
        assert_eq!(
            actual[10],
            json!({"id":"p-large","family":"Family-1","given":"Given-1-b","city":"City-1"})
        );
    }

    #[test]
    fn total_order_oracle_rejects_nested_multi_producing_scopes() {
        let unsupported = patient_view(json!([{"forEach":"name","select":[
            {"forEach":"given","column":[{"path":"$this","name":"given","type":"string"}]},
            {"forEach":"prefix","column":[{"path":"$this","name":"prefix","type":"string"}]}]}]));
        assert!(std::panic::catch_unwind(|| root_factor_rows(&unsupported, &[])).is_err());
    }
}

pub fn first_difference(actual: &[Value], expected: &[Value]) -> String {
    match actual.iter().zip(expected).position(|(a, e)| a != e) {
        Some(i) => format!(
            "row {i}: got {} expected {} (lengths {} / {})",
            actual[i],
            expected[i],
            actual.len(),
            expected.len()
        ),
        None => format!("lengths {} / {}", actual.len(), expected.len()),
    }
}

pub fn assert_ordered(actual: &[Value], expected: &[Value], context: &str) {
    assert!(
        actual == expected,
        "{context}: {}",
        first_difference(actual, expected)
    );
}

pub fn assert_prefix(actual: &[Value], expected: &[Value], limit: usize, context: &str) {
    assert_ordered(actual, &expected[..limit.min(expected.len())], context);
}

/// Legacy union/repeat: compare full cells and duplicate row counts, while
/// leaving their global row order outside this issue's contract.
pub fn assert_multiset_strict(actual: &[Value], expected: &[Value], context: &str) {
    assert_eq!(actual.len(), expected.len(), "{context}: row multiplicity");
    let mut unmatched: Vec<_> = expected.iter().collect();
    for (index, row) in actual.iter().enumerate() {
        let at = unmatched
            .iter()
            .position(|candidate| *candidate == row)
            .unwrap_or_else(|| panic!("{context}: unexpected complete row {index}: {row}"));
        unmatched.swap_remove(at);
    }
    assert!(unmatched.is_empty(), "{context}: missing rows");
}

#[derive(Clone, Copy)]
pub struct PlannerCondition {
    pub name: &'static str,
    pub analyze_first: bool,
    pub settings: &'static [(&'static str, &'static str)],
    pub plan_cache_mode: &'static str,
}

pub fn planner_conditions() -> Vec<PlannerCondition> {
    let condition = |name, analyze_first, settings, plan_cache_mode| PlannerCondition {
        name,
        analyze_first,
        settings,
        plan_cache_mode,
    };
    vec![
        condition("before-analyze", false, &[], "force_custom_plan"),
        condition("after-analyze", true, &[], "force_custom_plan"),
        condition(
            "hashjoin-off",
            false,
            &[("enable_hashjoin", "off")],
            "force_custom_plan",
        ),
        condition(
            "mergejoin-off",
            false,
            &[("enable_mergejoin", "off")],
            "force_custom_plan",
        ),
        condition(
            "nestloop-off",
            false,
            &[("enable_nestloop", "off")],
            "force_custom_plan",
        ),
        condition(
            "sort-off",
            false,
            &[("enable_sort", "off")],
            "force_custom_plan",
        ),
        condition(
            "incremental-sort-off",
            false,
            &[("enable_incremental_sort", "off")],
            "force_custom_plan",
        ),
        condition(
            "work-mem-64kB",
            false,
            &[("work_mem", "64kB")],
            "force_custom_plan",
        ),
        condition(
            "work-mem-64MB",
            false,
            &[("work_mem", "64MB")],
            "force_custom_plan",
        ),
        condition(
            "parallel-0",
            false,
            &[("max_parallel_workers_per_gather", "0")],
            "force_custom_plan",
        ),
        condition(
            "parallel-4",
            false,
            &[
                ("max_parallel_workers_per_gather", "4"),
                ("parallel_setup_cost", "0"),
                ("parallel_tuple_cost", "0"),
                ("min_parallel_table_scan_size", "0"),
                ("min_parallel_index_scan_size", "0"),
            ],
            "force_custom_plan",
        ),
        condition("plan-cache-custom", false, &[], "force_custom_plan"),
        condition("plan-cache-generic", false, &[], "force_generic_plan"),
        // Each run prepares a fresh statement; auto does not reuse a warm one.
        condition("plan-cache-auto-fresh-statement", false, &[], "auto"),
    ]
}

pub const WATCHED_SETTINGS: [&str; 12] = [
    "enable_hashjoin",
    "enable_mergejoin",
    "enable_nestloop",
    "enable_sort",
    "enable_incremental_sort",
    "work_mem",
    "max_parallel_workers_per_gather",
    "parallel_setup_cost",
    "parallel_tuple_cost",
    "min_parallel_table_scan_size",
    "min_parallel_index_scan_size",
    "plan_cache_mode",
];

#[derive(Clone, Copy)]
pub struct StatisticsCondition {
    pub name: &'static str,
    pub drop_indexes: bool,
    pub analyze_first: bool,
    pub pragmas: &'static str,
    pub expected: &'static [(&'static str, i64)],
}

pub fn statistics_conditions() -> Vec<StatisticsCondition> {
    let condition = |name, drop_indexes, analyze_first, pragmas, expected| StatisticsCondition {
        name,
        drop_indexes,
        analyze_first,
        pragmas,
        expected,
    };
    vec![
        condition("no-statistics", false, false, "", &[]),
        condition(
            "no-statistics-reverse",
            false,
            false,
            "PRAGMA reverse_unordered_selects=ON;",
            &[("reverse_unordered_selects", 1)],
        ),
        condition("after-analyze", false, true, "", &[]),
        condition(
            "after-analyze-reverse",
            false,
            false,
            "PRAGMA reverse_unordered_selects=ON;",
            &[("reverse_unordered_selects", 1)],
        ),
        condition(
            "automatic-index-off",
            false,
            false,
            "PRAGMA automatic_index=OFF;",
            &[("automatic_index", 0)],
        ),
        condition(
            "small-cache-file-temp",
            false,
            false,
            "PRAGMA cache_size=-64; PRAGMA temp_store=FILE;",
            &[("cache_size", -64), ("temp_store", 1)],
        ),
        condition(
            "large-cache-memory-temp",
            false,
            false,
            "PRAGMA cache_size=-65536; PRAGMA temp_store=MEMORY;",
            &[("cache_size", -65536), ("temp_store", 2)],
        ),
        condition("secondary-indexes-dropped", true, true, "", &[]),
        condition(
            "secondary-indexes-dropped-reverse",
            false,
            false,
            "PRAGMA reverse_unordered_selects=ON;",
            &[("reverse_unordered_selects", 1)],
        ),
    ]
}
