//! REST transport glue for the shared, independent SOF ordering oracle.

#[allow(dead_code)]
#[path = "../../../persistence/tests/common/sof_prefix_matrix.rs"]
mod sof_prefix_matrix;

#[allow(unused_imports)]
pub use sof_prefix_matrix::{
    FixtureResource, assert_multiset_strict, assert_ordered, assert_prefix, cell_cases,
    cell_fixture, evaluator_oracle, exact_fixture, fixture, ordering_cases,
};

use axum::http::StatusCode;
use axum_test::TestServer;
use futures::TryStreamExt;
use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::sof_runner::{RowStream, SofError, SofRunner, ViewFilters};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// Official fixtures retain their expected-key, numeric-tolerance comparator.
/// Our contract tests check complete rows, multiplicity and array positions.
#[derive(Clone, Copy)]
pub enum Comparison {
    Official,
    Unordered,
    Ordered,
    Prefix(usize),
}

pub fn compare_rows(mode: Comparison, actual: &[Value], expected: &[Value]) -> Option<String> {
    fn official_value(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::String(x), Value::String(y)) => x == y,
            (Value::Number(x), Value::Number(y)) => x
                .as_f64()
                .zip(y.as_f64())
                .is_some_and(|(x, y)| (x - y).abs() < 1e-9),
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(x, y)| official_value(x, y))
            }
            _ => false,
        }
    }
    let expected = match mode {
        Comparison::Prefix(limit) => &expected[..limit.min(expected.len())],
        _ => expected,
    };
    if actual.len() != expected.len() {
        return Some(format!(
            "row count mismatch: got {}, expected {}",
            actual.len(),
            expected.len()
        ));
    }
    if matches!(mode, Comparison::Ordered | Comparison::Prefix(_)) {
        return (actual != expected).then(|| sof_prefix_matrix::first_difference(actual, expected));
    }
    let mut remaining: Vec<_> = actual.iter().collect();
    for expected in expected {
        let at = remaining.iter().position(|actual| match mode {
            Comparison::Official => expected.as_object().is_some_and(|object| {
                object.iter().all(|(key, value)| {
                    actual
                        .get(key)
                        .is_some_and(|actual| official_value(actual, value))
                })
            }),
            Comparison::Unordered => *actual == expected,
            _ => unreachable!(),
        });
        let Some(at) = at else {
            return Some(format!("no matching actual row for expected: {expected}"));
        };
        // Keep the official comparator's original greedy candidate order.
        remaining.remove(at);
    }
    None
}

pub fn parse_ndjson(body: &str) -> Vec<Value> {
    body.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let row: Value = serde_json::from_str(line).expect("valid NDJSON row");
            assert!(row.is_object(), "complete row object: {row}");
            row
        })
        .collect()
}

/// Read the written resource back before calculating the oracle. SQLite stores
/// timestamps at a different precision from the timestamp returned by create.
pub async fn seed_fixture<S: ResourceStorage>(
    backend: &S,
    tenant: &TenantContext,
    mut resources: Vec<FixtureResource>,
) -> Vec<FixtureResource> {
    for resource in &mut resources {
        let created = backend
            .create(
                tenant,
                resource.resource_type,
                resource.data.clone(),
                FhirVersion::R4,
            )
            .await
            .expect("seed ordering resource");
        assert_eq!(
            created.id(),
            resource.id,
            "create preserves supplied fixture id"
        );
        let stored = backend
            .read(tenant, resource.resource_type, &resource.id)
            .await
            .expect("read persisted ordering metadata")
            .expect("created resource exists");
        resource.last_updated = stored.last_modified();
        resource.data = stored.content_with_meta();
        assert_eq!(
            stored.id(),
            resource.id,
            "read preserves supplied fixture id"
        );
        assert_eq!(
            resource.data["id"], resource.id,
            "persisted content has fixture id"
        );
        if resource.deleted {
            backend
                .delete(tenant, resource.resource_type, &resource.id)
                .await
                .expect("seed deleted resource");
        }
    }
    resources
}

pub fn run_parameters(view: &Value, filters: &ViewFilters, limit: Option<usize>) -> Value {
    let mut parameters = vec![
        json!({"name":"_format","valueCode":"ndjson"}),
        json!({"name":"subjectResource","resource":view}),
    ];
    if let Some(limit) = limit {
        parameters.push(json!({"name":"_limit","valueInteger":limit}));
    }
    if let Some(since) = filters.since {
        parameters.push(json!({"name":"_since","valueInstant":since.to_rfc3339()}));
    }
    for patient in &filters.patient {
        parameters.push(json!({"name":"patient","valueReference":{"reference":patient}}));
    }
    for group in &filters.group {
        parameters.push(json!({"name":"group","valueReference":{"reference":group}}));
    }
    json!({"resourceType":"Parameters","parameter":parameters})
}

pub async fn http_rows(
    server: &TestServer,
    tenant: &str,
    view: &Value,
    filters: &ViewFilters,
    limit: usize,
) -> Vec<Value> {
    let response = server
        .post("/$sql-run")
        .add_header("x-tenant-id", tenant)
        .json(&run_parameters(view, filters, Some(limit)))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    parse_ndjson(&response.text())
}

pub async fn unlimited(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    filters: &ViewFilters,
) -> Vec<Value> {
    let mut filters = filters.clone();
    filters.limit = None;
    runner
        .run_view(tenant, view.clone(), filters)
        .await
        .expect("unlimited runner")
        .try_collect()
        .await
        .expect("consume complete unlimited stream")
}

pub async fn preview_acceptance(
    server: &TestServer,
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    tenant_id: &str,
    resources: &[FixtureResource],
) {
    let cases = ordering_cases();
    assert_eq!(cases.len(), 14, "all shared REST ordering/filter cases");
    for mut case in cases {
        if case.filters.since.is_some() {
            let mut timestamps: Vec<_> = resources
                .iter()
                .filter(|r| {
                    r.resource_type == "Patient" && !r.deleted && r.data["gender"] == "male"
                })
                .map(|r| r.last_updated)
                .collect();
            timestamps.sort();
            assert!(!timestamps.is_empty(), "positive male since fixture");
            case.filters.since = Some(timestamps[timestamps.len() / 2]);
        }
        // Compute independently before the first SQL or HTTP execution.
        let expected = evaluator_oracle(&case.view, resources, &case.filters);
        match case.name {
            "patient" => assert_eq!(
                expected.len(),
                if resources.iter().any(|r| r.id == "A-2") {
                    151
                } else {
                    150
                },
                "positive patient selection excludes deleted member"
            ),
            "group-and-explicit-patient" if resources.iter().any(|r| r.id == "g-members") => {
                assert_eq!(
                    expected.len(),
                    152,
                    "positive Group union, duplicate membership and deleted member"
                )
            }
            "empty-group" | "deleted-group" => assert!(
                expected.is_empty(),
                "{name}: no matching members",
                name = case.name
            ),
            "constant-and-since" => assert!(!expected.is_empty(), "positive timestamp filter"),
            _ => {}
        }
        if case.name == "patient"
            || (case.name == "group-and-explicit-patient"
                && resources.iter().any(|r| r.id == "g-members"))
        {
            let ids = expected
                .iter()
                .map(|row| row["id"].as_str().expect("selected resource id"))
                .collect::<std::collections::BTreeSet<_>>();
            let expected_ids = if case.name == "group-and-explicit-patient" {
                vec!["A-2", "a.1", "p-large"]
            } else if resources.iter().any(|r| r.id == "A-2") {
                vec!["A-2", "p-large"]
            } else {
                vec!["p-large"]
            };
            assert_eq!(
                ids,
                expected_ids.into_iter().collect(),
                "positive patient/Group identities"
            );
        }
        if resources.len() == 3 {
            let count = match case.name {
                "flat" => Some(3),
                "foreach" => Some(151),
                "nested" | "chained" => Some(300),
                "cartesian" => Some(1500),
                "nullable" => Some(152),
                "nullable-filtered" => Some(77),
                "element-filter" => Some(75),
                "nested-cartesian" => Some(3000),
                _ => None,
            };
            if let Some(count) = count {
                assert_eq!(expected.len(), count, "exact fixture {}", case.name);
            }
        }
        let all = unlimited(runner, tenant, &case.view, &case.filters).await;
        assert_ordered(&all, &expected, case.name);
        eprintln!(
            "REST prefix case={} resources={} unlimited_rows={} limits=1,50,10000",
            case.name,
            resources.len(),
            expected.len()
        );
        for limit in [1, 50, 10_000] {
            let actual = http_rows(server, tenant_id, &case.view, &case.filters, limit).await;
            let difference = compare_rows(Comparison::Prefix(limit), &actual, &expected);
            assert!(
                difference.is_none(),
                "{} limit {limit}: {difference:?}",
                case.name
            );
            assert_prefix(&actual, &all, limit, case.name);
        }
    }
}

pub async fn cell_acceptance(
    server: &TestServer,
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    tenant_id: &str,
    resources: &[FixtureResource],
) {
    let cases = cell_cases();
    assert_eq!(cases.len(), 6, "all shared collection-cell shapes");
    for (name, view, oracle_view, ordered) in cases {
        let filters = ViewFilters::default();
        let expected = evaluator_oracle(&oracle_view, resources, &filters);
        let all = unlimited(runner, tenant, &view, &filters).await;
        eprintln!(
            "REST cell case={name} ordered={ordered} unlimited_rows={}",
            expected.len()
        );
        let mode = if ordered {
            Comparison::Ordered
        } else {
            Comparison::Unordered
        };
        assert!(
            compare_rows(mode, &all, &expected).is_none(),
            "{name}: {all:?} != {expected:?}"
        );
        let actual = http_rows(server, tenant_id, &view, &filters, 10_000).await;
        assert!(
            compare_rows(mode, &actual, &expected).is_none(),
            "HTTP {name}"
        );
        if ordered {
            let actual = http_rows(server, tenant_id, &view, &filters, 1).await;
            assert_prefix(&actual, &expected, 1, name);
        }
    }
}

/// Follow the public asynchronous protocol and concatenate shard locations in
/// manifest order. No sorting of locations, output rows or collection cells.
pub async fn export_rows(
    server: &TestServer,
    tenant_id: &str,
    subject: &Value,
) -> (Vec<Value>, Vec<usize>) {
    let body = json!({"resourceType":"Parameters","parameter":[
        {"name":"_format","valueCode":"ndjson"},
        {"name":"subject","part":[{"name":"name","valueString":"ordered"},
            {"name":"subjectResource","resource":subject}]}]});
    let response = server
        .post("/$sql-export")
        .add_header("x-tenant-id", tenant_id)
        .add_header("prefer", "respond-async")
        .json(&body)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::ACCEPTED,
        "{}",
        response.text()
    );
    let status_url = response
        .headers()
        .get("content-location")
        .expect("Content-Location")
        .to_str()
        .unwrap()
        .to_owned();
    let manifest = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let response = server
                .get(&status_url)
                .add_header("x-tenant-id", tenant_id)
                .await;
            match response.status_code() {
                StatusCode::SEE_OTHER => {
                    assert!(
                        response.text().is_empty(),
                        "303 completion has an empty body"
                    );
                    let url = response
                        .headers()
                        .get("location")
                        .expect("303 Location")
                        .to_str()
                        .unwrap();
                    let manifest = server.get(url).add_header("x-tenant-id", tenant_id).await;
                    assert_eq!(
                        manifest.status_code(),
                        StatusCode::OK,
                        "{}",
                        manifest.text()
                    );
                    break manifest.json::<Value>();
                }
                StatusCode::ACCEPTED => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                status => panic!("export status {status}: {}", response.text()),
            }
        }
    })
    .await
    .expect("export completes within 30 seconds");
    assert_eq!(manifest["resourceType"], "Parameters");
    let mut rows = Vec::new();
    let mut shard_counts = Vec::new();
    for output in manifest["parameter"]
        .as_array()
        .expect("manifest parameters")
        .iter()
        .filter(|p| p["name"] == "output")
    {
        for location in output["part"]
            .as_array()
            .expect("output parts")
            .iter()
            .filter(|p| p["name"] == "location")
        {
            let url = location["valueUri"].as_str().expect("manifest location");
            let response = server.get(url).add_header("x-tenant-id", tenant_id).await;
            assert_eq!(
                response.status_code(),
                StatusCode::OK,
                "shard {url}: {}",
                response.text()
            );
            let shard = parse_ndjson(&response.text());
            shard_counts.push(shard.len());
            rows.extend(shard);
        }
    }
    (rows, shard_counts)
}

pub async fn export_acceptance(
    server: &TestServer,
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    tenant_id: &str,
    resources: &[FixtureResource],
) {
    let cases = ordering_cases();
    let mut views: Vec<_> = [1, 2, 3, 4, 5]
        .into_iter()
        .map(|i| (cases[i].name, cases[i].view.clone()))
        .collect();
    views.extend(
        cell_cases()
            .into_iter()
            .filter(|(name, _, _, _)| *name == "flat-cells" || *name == "foreach-cells")
            .map(|(name, view, _, _)| (name, view)),
    );
    assert_eq!(
        views.len(),
        7,
        "all selected ordered row and cell export shapes"
    );
    for (name, view) in views {
        let expected = evaluator_oracle(&view, resources, &ViewFilters::default());
        let all = unlimited(runner, tenant, &view, &ViewFilters::default()).await;
        assert_ordered(&all, &expected, name);
        let (exported, shards) = export_rows(server, tenant_id, &view).await;
        eprintln!(
            "REST export case={name} rows={} shards={} shard_rows=7",
            exported.len(),
            shards.len()
        );
        assert_ordered(&exported, &all, name);
        assert_ordered(&exported, &expected, name);
        assert_eq!(
            shards.iter().sum::<usize>(),
            expected.len(),
            "{name}: all shard rows"
        );
        assert_eq!(
            shards.len(),
            expected.len().div_ceil(7),
            "{name}: explicit seven-row shards"
        );
        assert!(
            shards.iter().all(|rows| (1..=7).contains(rows)),
            "{name}: {shards:?}"
        );
        if expected.len() > 7 {
            assert!(shards.len() > 1, "{name}: multiple shards exercised");
        }
        let preview = http_rows(server, tenant_id, &view, &ViewFilters::default(), 50).await;
        assert_prefix(&preview, &exported, 50, name);
    }
}

/// Observe dependency filters while retaining the real backend runner.
pub struct RecordingRunner {
    inner: Arc<dyn SofRunner>,
    pub limits: Mutex<Vec<Option<usize>>>,
}

impl RecordingRunner {
    pub fn new(inner: Arc<dyn SofRunner>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            limits: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl SofRunner for RecordingRunner {
    async fn run_view(
        &self,
        tenant: &TenantContext,
        view: Value,
        filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        self.limits.lock().unwrap().push(filters.limit);
        self.inner.run_view(tenant, view, filters).await
    }
    fn runner_name(&self) -> &'static str {
        self.inner.runner_name()
    }
}

/// Exercise a real foreach leaf through SQLView -> SQLQuery and export. SQL
/// is supplied to the public Library operation; this helper opens no database.
pub async fn dependency_acceptance<S: ResourceStorage>(
    server: &TestServer,
    backend: &S,
    tenant: &TenantContext,
    tenant_id: &str,
    recorder: &RecordingRunner,
) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let mut fixture = exact_fixture();
    fixture[0].data["name"][1] = fixture[0].data["name"][0].clone();
    let fixture = seed_fixture(backend, tenant, fixture).await;
    let view = json!({"resourceType":"ViewDefinition","id":"expanded-order","status":"active","resource":"Patient",
        "select":[{"forEach":"name","column":[{"path":"family","name":"family","type":"string"}]}]});
    let expected = evaluator_oracle(&view, &fixture, &ViewFilters::default());
    assert_eq!(
        expected.len(),
        151,
        "150 names plus the filtered-resource name"
    );
    assert_eq!(
        expected
            .iter()
            .filter(|row| row["family"] == "Family-1")
            .count(),
        2,
        "duplicate leaf rows"
    );
    backend
        .create(tenant, "ViewDefinition", view, FhirVersion::R4)
        .await
        .expect("expanded leaf");
    let library = |id: &str, kind: &str, sql: &str, reference: &str, label: &str| {
        json!({
        "resourceType":"Library","id":id,"status":"active",
        "type":{"coding":[{"system":"https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes","code":kind}]},
        "content":[{"contentType":"application/sql","data":STANDARD.encode(sql)}],
            "relatedArtifact":[{"type":"depends-on","label":label,"resource":reference}]})
    };
    let middle = library(
        "expanded-middle",
        "sql-view",
        "SELECT family FROM leaf",
        "ViewDefinition/expanded-order",
        "leaf",
    );
    backend
        .create(tenant, "Library", middle, FhirVersion::R4)
        .await
        .expect("SQLView intermediate");
    let count = library(
        "expanded-count",
        "sql-query",
        "SELECT COUNT(*) AS total FROM source",
        "Library/expanded-middle",
        "source",
    );
    let rows = http_rows(server, tenant_id, &count, &ViewFilters::default(), 1).await;
    assert_ordered(
        &rows,
        &[json!({"total":151})],
        "final cap leaves complete expanded dependency",
    );
    let project = library(
        "expanded-project",
        "sql-query",
        "SELECT family FROM source",
        "Library/expanded-middle",
        "source",
    );
    let rows = http_rows(server, tenant_id, &project, &ViewFilters::default(), 10_000).await;
    assert_multiset_strict(
        &rows,
        &expected,
        "complete SQLQuery dependency rows and duplicates",
    );
    let capped = http_rows(server, tenant_id, &project, &ViewFilters::default(), 1).await;
    assert_eq!(capped.len(), 1, "cap applies only to final SQLQuery rows");
    let (exported, shards) = export_rows(server, tenant_id, &project).await;
    assert_multiset_strict(
        &exported,
        &expected,
        "expanded SQLQuery export dependency chain",
    );
    assert_eq!(
        shards.len(),
        expected.len().div_ceil(7),
        "SQLQuery seven-row shards"
    );
    assert_eq!(shards.iter().sum::<usize>(), expected.len());
    assert!(shards.iter().all(|rows| (1..=7).contains(rows)));
    let limits = recorder.limits.lock().unwrap();
    assert_eq!(
        limits.len(),
        4,
        "one real leaf materialization per HTTP run/export"
    );
    assert!(
        limits.iter().all(Option::is_none),
        "all dependency runners must receive limit=None: {limits:?}"
    );
}

#[cfg(test)]
mod comparator_tests {
    use super::*;

    #[test]
    fn official_mode_preserves_expected_keys_and_numeric_tolerance() {
        let actual = [json!({"value":1.0,"extra":true}), json!({"value":2})];
        assert!(
            compare_rows(
                Comparison::Official,
                &actual,
                &[json!({"value":2}), json!({"value":1})]
            )
            .is_none()
        );
        assert!(
            compare_rows(Comparison::Official, &[json!({})], &[json!({"value":null})]).is_some()
        );
        assert!(
            compare_rows(
                Comparison::Official,
                &[json!({"value":"1"})],
                &[json!({"value":1})]
            )
            .is_some()
        );
    }

    #[test]
    fn strict_modes_detect_extra_keys_duplicate_loss_and_array_permutation() {
        let rows = [
            json!({"array":["z","a"],"value":null}),
            json!({"array":["z","a"],"value":null}),
        ];
        for mode in [
            Comparison::Unordered,
            Comparison::Ordered,
            Comparison::Prefix(2),
        ] {
            assert!(compare_rows(mode, &rows, &rows).is_none());
            assert!(compare_rows(mode, &[rows[0].clone()], &rows).is_some());
            assert!(
                compare_rows(
                    mode,
                    &[json!({"array":["a","z"],"value":null}), rows[1].clone()],
                    &rows
                )
                .is_some()
            );
            assert!(
                compare_rows(
                    mode,
                    &[
                        json!({"array":["z","a"],"value":null,"extra":1}),
                        rows[1].clone()
                    ],
                    &rows
                )
                .is_some()
            );
        }
        let different = [json!({"n":1}), json!({"n":2})];
        assert!(
            compare_rows(
                Comparison::Ordered,
                &[different[1].clone(), different[0].clone()],
                &different
            )
            .is_some()
        );
        assert!(
            compare_rows(
                Comparison::Unordered,
                &[different[1].clone(), different[0].clone()],
                &different
            )
            .is_none()
        );
    }
}
