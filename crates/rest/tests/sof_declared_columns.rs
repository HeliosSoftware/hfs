//! #1623: buffered and sharded output of a ViewDefinition run by an in-DB SQL
//! runner lists the view's declared columns, in SQL projection order, whatever
//! the first row (of the result, or of a shard) carries.
//!
//! PostgreSQL's row mapper omits a SQL-NULL column from the row object, and
//! the formatters used to take their columns from the first row, so a NULL in
//! that row dropped the column and every later value of it. Raw row objects
//! (streamed NDJSON, export JSON/NDJSON) stay exactly what the runner yields.
//!
//! Every scenario runs against SQLite and against a real PostgreSQL container
//! (testcontainers, Docker required), so the PostgreSQL module is the one that
//! exercises the NULL-omitting mapper.
//!
//! The complex-view scenarios (expanded, union and repeat views) also check
//! that unlimited runs, every downloaded export shard and SQLQuery dependency
//! materialization keep all rows, values and multiplicity, and that visible
//! keys are exactly the declared columns.
#![cfg(all(feature = "R4", any(feature = "sqlite", feature = "postgres")))]

#[cfg(feature = "postgres")]
#[path = "common/container_cleanup.rs"]
mod container_cleanup;

use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use futures::StreamExt;
use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");
const PREFER: HeaderName = HeaderName::from_static("prefer");
const ACCEPT: HeaderName = HeaderName::from_static("accept");

/// One seeded tenant behind a REST server whose `$sql-run` and `$sql-export`
/// both use the backend's in-DB runner.
pub struct Harness {
    server: TestServer,
    runner: Arc<dyn SofRunner>,
    tenant: TenantContext,
    tenant_id: String,
    /// Whether the runner omits a SQL-NULL column from its row objects
    /// (PostgreSQL) instead of carrying it as JSON `null` (SQLite).
    omits_null_keys: bool,
}

impl Harness {
    fn tenant_header(&self) -> HeaderValue {
        HeaderValue::from_str(&self.tenant_id).expect("tenant header")
    }

    /// The runner's own rows for `view`, unlimited.
    async fn runner_rows(&self, view: &Value) -> Vec<Value> {
        let mut stream = self
            .runner
            .run_view(&self.tenant, view.clone(), ViewFilters::default())
            .await
            .expect("runner accepts the view");
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row.expect("runner row"));
        }
        rows
    }

    async fn sql_run(&self, view: &Value, query: &str) -> axum_test::TestResponse {
        let response = self
            .server
            .post(&format!("/$sql-run?{query}"))
            .add_header(X_TENANT_ID, self.tenant_header())
            .json(view)
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "{}",
            response.text()
        );
        response
    }

    /// Runs `$sql-export` for `view` and downloads every shard, in manifest
    /// order.
    async fn export_shards(&self, view: &Value, query: &str) -> Vec<Vec<u8>> {
        let submit = self
            .server
            .post(&format!("/$sql-export?{query}"))
            .add_header(PREFER, HeaderValue::from_static("respond-async"))
            .add_header(X_TENANT_ID, self.tenant_header())
            .json(view)
            .await;
        assert_eq!(
            submit.status_code(),
            StatusCode::ACCEPTED,
            "{}",
            submit.text()
        );
        let status_url = submit
            .headers()
            .get("content-location")
            .and_then(|v| v.to_str().ok())
            .expect("content-location")
            .to_string();
        let manifest = self.poll_to_manifest(&status_url).await;
        let mut shards = Vec::new();
        for output in manifest["parameter"]
            .as_array()
            .expect("manifest parameters")
            .iter()
            .filter(|p| p["name"] == "output")
        {
            for part in output["part"].as_array().expect("output parts") {
                if part["name"] != "location" {
                    continue;
                }
                let url = part["valueUri"].as_str().expect("location valueUri");
                let path = &url[url.find("/export/").expect("export download path")..];
                let download = self
                    .server
                    .get(path)
                    .add_header(X_TENANT_ID, self.tenant_header())
                    .await;
                assert_eq!(
                    download.status_code(),
                    StatusCode::OK,
                    "{}",
                    download.text()
                );
                shards.push(download.as_bytes().to_vec());
            }
        }
        shards
    }

    async fn poll_to_manifest(&self, status_url: &str) -> Value {
        for _ in 0..200 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let poll = self
                .server
                .get(status_url)
                .add_header(X_TENANT_ID, self.tenant_header())
                .await;
            match poll.status_code() {
                StatusCode::SEE_OTHER => {
                    let result_url = poll
                        .headers()
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .expect("result location")
                        .to_string();
                    let result = self
                        .server
                        .get(&result_url)
                        .add_header(X_TENANT_ID, self.tenant_header())
                        .await;
                    assert_eq!(result.status_code(), StatusCode::OK, "{}", result.text());
                    return result.json();
                }
                StatusCode::ACCEPTED => continue,
                other => panic!("unexpected export poll status {other}: {}", poll.text()),
            }
        }
        panic!("export did not complete: {status_url}");
    }
}

// ============================================================================
// Fixtures
// ============================================================================

/// Patients in creation order, which is also id order: `a1` has only an id and
/// a family name, so every other demographics column is SQL NULL in the first
/// row of an ordinary (last-updated, id ordered) select.
fn demographics_patients() -> Vec<Value> {
    vec![
        json!({"resourceType": "Patient", "id": "a1", "name": [{"family": "Bare"}]}),
        full_patient("a2", "female", "Parker"),
    ]
}

fn full_patient(id: &str, gender: &str, family: &str) -> Value {
    json!({
        "resourceType": "Patient", "id": id, "active": true, "gender": gender,
        "birthDate": "2015-12-29", "name": [{"family": family}],
        "address": [{"city": "Everett"}]
    })
}

/// `marital` is declared but no patient has a value: an all-NULL column.
fn demographics_view() -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "name": "patient_demographics",
        "status": "active",
        "resource": "Patient",
        "select": [{"column": [
            {"name": "id", "path": "getResourceKey()", "type": "id"},
            {"name": "gender", "path": "gender"},
            {"name": "birth_date", "path": "birthDate", "type": "date"},
            {"name": "active", "path": "active", "type": "boolean"},
            {"name": "family", "path": "name.first().family"},
            {"name": "city", "path": "address.first().city"},
            {"name": "marital", "path": "maritalStatus.text"}
        ]}]
    })
}

const DEMOGRAPHICS_COLUMNS: [&str; 7] = [
    "id",
    "gender",
    "birth_date",
    "active",
    "family",
    "city",
    "marital",
];

/// The declared columns of [`demographics_view`] for every row.
fn demographics_rows() -> Vec<Vec<Value>> {
    let null = Value::Null;
    vec![
        vec![
            json!("a1"),
            null.clone(),
            null.clone(),
            null.clone(),
            json!("Bare"),
            null.clone(),
            null.clone(),
        ],
        vec![
            json!("a2"),
            json!("female"),
            json!("2015-12-29"),
            json!(true),
            json!("Parker"),
            json!("Everett"),
            null,
        ],
    ]
}

/// A `unionAll` declared before a sibling column: the SQL layout projects the
/// shared `id` and `gender` before the branch columns, and declares `value` /
/// `system` once although both branches name them.
fn union_view() -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "name": "patient_telecoms",
        "status": "active",
        "resource": "Patient",
        "select": [
            {"column": [{"name": "id", "path": "getResourceKey()", "type": "id"}]},
            {"unionAll": [
                {"forEach": "telecom", "column": [
                    {"name": "value", "path": "value"},
                    {"name": "system", "path": "system"}
                ]},
                {"forEach": "contact.telecom", "column": [
                    {"name": "value", "path": "value"},
                    {"name": "system", "path": "system"}
                ]}
            ]},
            {"column": [{"name": "gender", "path": "gender"}]}
        ]
    })
}

const UNION_COLUMNS: [&str; 4] = ["id", "gender", "value", "system"];

fn union_patients() -> Vec<Value> {
    vec![
        json!({"resourceType": "Patient", "id": "a1", "telecom": [{"value": "tel-555"}]}),
        json!({
            "resourceType": "Patient", "id": "a2", "gender": "male",
            "contact": [{"telecom": [{"system": "email", "value": "x@example.org"}]}]
        }),
    ]
}

fn union_rows() -> Vec<Vec<Value>> {
    vec![
        vec![json!("a1"), Value::Null, json!("tel-555"), Value::Null],
        vec![
            json!("a2"),
            json!("male"),
            json!("x@example.org"),
            json!("email"),
        ],
    ]
}

/// No patient matches: an empty result.
fn empty_view() -> Value {
    let mut view = demographics_view();
    view["where"] = json!([{"path": "gender = 'no-such-gender'"}]);
    view
}

/// Four patients, two rows per shard: the second shard starts with the
/// NULL-bearing `a3` (an interior shard boundary).
fn sharded_patients() -> Vec<Value> {
    vec![
        full_patient("a1", "female", "One"),
        full_patient("a2", "male", "Two"),
        json!({"resourceType": "Patient", "id": "a3", "name": [{"family": "Three"}]}),
        full_patient("a4", "other", "Four"),
    ]
}

// ============================================================================
// Decoding helpers
// ============================================================================

fn names(columns: &[&str]) -> Vec<String> {
    columns.iter().map(|c| (*c).to_string()).collect()
}

/// A raw row object projected onto `columns`, a missing key as `null`.
fn full_row(row: &Value, columns: &[&str]) -> Vec<Value> {
    columns
        .iter()
        .map(|c| row.get(*c).cloned().unwrap_or(Value::Null))
        .collect()
}

fn csv_cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn csv_line(cells: &[Value]) -> String {
    cells.iter().map(csv_cell).collect::<Vec<_>>().join(",")
}

fn object_keys(row: &Value) -> Vec<String> {
    row.as_object()
        .expect("row object")
        .keys()
        .cloned()
        .collect()
}

/// A Parquet file's field names and its rows as JSON objects.
fn read_parquet(bytes: &[u8]) -> (Vec<String>, Vec<Value>) {
    use parquet::file::reader::{FileReader, SerializedFileReader};
    let reader = SerializedFileReader::new(bytes::Bytes::copy_from_slice(bytes)).expect("parquet");
    let fields = reader
        .metadata()
        .file_metadata()
        .schema_descr()
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let rows = reader
        .get_row_iter(None)
        .expect("parquet rows")
        .map(|row| {
            let row = row.expect("parquet row");
            let object: serde_json::Map<String, Value> = row
                .get_column_iter()
                .map(|(name, field)| (name.clone(), parquet_field(field)))
                .collect();
            Value::Object(object)
        })
        .collect();
    (fields, rows)
}

/// The scalar Parquet values these views produce, as JSON.
fn parquet_field(field: &parquet::record::Field) -> Value {
    use parquet::record::Field;
    match field {
        Field::Null => Value::Null,
        Field::Bool(b) => json!(b),
        Field::Str(s) => json!(s),
        Field::Int(i) => json!(i),
        Field::Long(i) => json!(i),
        Field::Double(d) => json!(d),
        other => panic!("unexpected parquet value {other:?}"),
    }
}

/// An Arrow IPC stream's field names and row count.
fn read_arrow(bytes: &[u8]) -> (Vec<String>, usize) {
    let reader =
        arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes.to_vec()), None)
            .expect("arrow stream");
    let fields = reader
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    let rows = reader.map(|batch| batch.expect("batch").num_rows()).sum();
    (fields, rows)
}

// ============================================================================
// Scenarios, run once per engine by `declared_column_scenarios!`
// ============================================================================

async fn buffered_json_keeps_columns_omitted_by_the_first_row(h: Harness) {
    let rows: Vec<Value> = h.sql_run(&demographics_view(), "_format=json").await.json();
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(object_keys(row), names(&DEMOGRAPHICS_COLUMNS));
    }
    let cells: Vec<Vec<Value>> = rows
        .iter()
        .map(|r| full_row(r, &DEMOGRAPHICS_COLUMNS))
        .collect();
    assert_eq!(cells, demographics_rows());
}

async fn buffered_csv_keeps_columns_omitted_by_the_first_row(h: Harness) {
    let body = h
        .sql_run(&demographics_view(), "_format=csv&header=true")
        .await
        .text();
    let mut expected = vec![DEMOGRAPHICS_COLUMNS.join(",")];
    expected.extend(demographics_rows().iter().map(|r| csv_line(r)));
    assert_eq!(body.lines().collect::<Vec<_>>(), expected);
}

async fn buffered_parquet_and_arrow_keep_columns_omitted_by_the_first_row(h: Harness) {
    let parquet = h
        .sql_run(&demographics_view(), "_format=parquet")
        .await
        .as_bytes()
        .to_vec();
    let (fields, rows) = read_parquet(&parquet);
    assert_eq!(fields, names(&DEMOGRAPHICS_COLUMNS));
    let cells: Vec<Vec<Value>> = rows
        .iter()
        .map(|r| full_row(r, &DEMOGRAPHICS_COLUMNS))
        .collect();
    assert_eq!(cells, demographics_rows());

    let arrow = h
        .sql_run(&demographics_view(), "_format=arrow")
        .await
        .as_bytes()
        .to_vec();
    assert_eq!(read_arrow(&arrow), (names(&DEMOGRAPHICS_COLUMNS), 2));
}

async fn fhir_parameters_keep_later_values_and_omit_null_cells(h: Harness) {
    let body: Value = h.sql_run(&demographics_view(), "_format=fhir").await.json();
    let parts: Vec<Vec<String>> = body["parameter"]
        .as_array()
        .expect("row parameters")
        .iter()
        .map(|row| {
            row["part"]
                .as_array()
                .expect("row parts")
                .iter()
                .map(|p| p["name"].as_str().expect("part name").to_string())
                .collect()
        })
        .collect();
    assert_eq!(
        parts,
        vec![
            names(&["id", "family"]),
            names(&["id", "gender", "birth_date", "active", "family", "city"]),
        ]
    );
}

async fn empty_buffered_results_keep_the_declared_columns(h: Harness) {
    let view = empty_view();
    let csv = h.sql_run(&view, "_format=csv&header=true").await.text();
    assert_eq!(csv, format!("{}\n", DEMOGRAPHICS_COLUMNS.join(",")));
    let json: Value = h.sql_run(&view, "_format=json").await.json();
    assert_eq!(json, json!([]));
    let parquet = h
        .sql_run(&view, "_format=parquet")
        .await
        .as_bytes()
        .to_vec();
    assert_eq!(
        read_parquet(&parquet),
        (names(&DEMOGRAPHICS_COLUMNS), vec![])
    );
    let arrow = h.sql_run(&view, "_format=arrow").await.as_bytes().to_vec();
    assert_eq!(read_arrow(&arrow), (names(&DEMOGRAPHICS_COLUMNS), 0));
}

async fn union_columns_are_deduplicated_in_sql_projection_order(h: Harness) {
    let view = union_view();
    let raw = h.runner_rows(&view).await;
    let expected: Vec<Vec<Value>> = raw.iter().map(|r| full_row(r, &UNION_COLUMNS)).collect();
    assert_eq!(expected, union_rows(), "runner rows: {raw:?}");

    let rows: Vec<Value> = h.sql_run(&view, "_format=json").await.json();
    for row in &rows {
        assert_eq!(object_keys(row), names(&UNION_COLUMNS));
    }
    let cells: Vec<Vec<Value>> = rows.iter().map(|r| full_row(r, &UNION_COLUMNS)).collect();
    assert_eq!(cells, expected);

    let csv = h.sql_run(&view, "_format=csv&header=true").await.text();
    let mut lines = vec![UNION_COLUMNS.join(",")];
    lines.extend(expected.iter().map(|r| csv_line(r)));
    assert_eq!(csv.lines().collect::<Vec<_>>(), lines);
}

async fn sharded_csv_and_parquet_exports_keep_every_declared_column(h: Harness) {
    let view = demographics_view();
    let raw = h.runner_rows(&view).await;
    assert_eq!(raw.len(), 4);
    assert_eq!(
        raw[2]["id"], "a3",
        "a3 must start the second shard: {raw:?}"
    );
    let expected: Vec<Vec<Value>> = raw
        .iter()
        .map(|r| full_row(r, &DEMOGRAPHICS_COLUMNS))
        .collect();

    let csv_shards = h.export_shards(&view, "_format=csv&header=true").await;
    assert_eq!(csv_shards.len(), 2);
    let mut csv_rows: Vec<String> = Vec::new();
    for shard in &csv_shards {
        let text = String::from_utf8(shard.clone()).expect("utf-8 csv");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(DEMOGRAPHICS_COLUMNS.join(",").as_str()));
        csv_rows.extend(lines.map(str::to_string));
    }
    assert_eq!(
        csv_rows,
        expected.iter().map(|r| csv_line(r)).collect::<Vec<_>>()
    );

    let parquet_shards = h.export_shards(&view, "_format=parquet").await;
    assert_eq!(parquet_shards.len(), 2);
    let mut parquet_rows: Vec<Vec<Value>> = Vec::new();
    for shard in &parquet_shards {
        let (fields, rows) = read_parquet(shard);
        assert_eq!(fields, names(&DEMOGRAPHICS_COLUMNS));
        parquet_rows.extend(rows.iter().map(|r| full_row(r, &DEMOGRAPHICS_COLUMNS)));
    }
    assert_eq!(parquet_rows, expected);
}

async fn raw_row_objects_stay_what_the_runner_yields(h: Harness) {
    let view = demographics_view();
    let raw = h.runner_rows(&view).await;
    // `a3` has only an id and a family name.
    let bare_keys = if h.omits_null_keys {
        names(&["id", "family"])
    } else {
        names(&DEMOGRAPHICS_COLUMNS)
    };
    assert_eq!(raw[2]["id"], "a3");
    assert_eq!(object_keys(&raw[2]), bare_keys);

    let ndjson = h.sql_run(&view, "_format=ndjson").await.text();
    let streamed: Vec<Value> = ndjson
        .lines()
        .map(|l| serde_json::from_str(l).expect("ndjson line"))
        .collect();
    assert_eq!(streamed, raw);
    for (line, row) in streamed.iter().zip(&raw) {
        assert_eq!(object_keys(line), object_keys(row));
    }

    let shards = h.export_shards(&view, "_format=ndjson").await;
    let exported: Vec<Value> = shards
        .iter()
        .flat_map(|s| {
            String::from_utf8(s.clone())
                .expect("utf-8")
                .lines()
                .map(|l| serde_json::from_str::<Value>(l).expect("ndjson line"))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(exported, raw);
    for (line, row) in exported.iter().zip(&raw) {
        assert_eq!(object_keys(line), object_keys(row));
    }

    let shards = h.export_shards(&view, "_format=json").await;
    let exported: Vec<Value> = shards
        .iter()
        .flat_map(|s| serde_json::from_slice::<Vec<Value>>(s).expect("json shard"))
        .collect();
    assert_eq!(exported, raw);
    for (row, raw_row) in exported.iter().zip(&raw) {
        assert_eq!(object_keys(row), object_keys(raw_row));
    }
}

/// The FHIR-envelope NDJSON representation keeps its first-row columns.
async fn enveloped_ndjson_keeps_first_row_columns(h: Harness) {
    let view = demographics_view();
    let raw = h.runner_rows(&view).await;
    let first_row_columns = object_keys(&raw[0]);
    let response = h
        .server
        .post("/$sql-run?_format=ndjson")
        .add_header(X_TENANT_ID, h.tenant_header())
        .add_header(ACCEPT, HeaderValue::from_static("application/fhir+json"))
        .json(&view)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    let binary: Value = response.json();
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD
        .decode(binary["data"].as_str().expect("Binary.data"))
        .expect("base64");
    for line in String::from_utf8(data).expect("utf-8").lines() {
        let row: Value = serde_json::from_str(line).expect("ndjson line");
        assert_eq!(object_keys(&row), first_row_columns);
    }
}

// ============================================================================
// #1623: complex (expanded / union / repeat) views keep every row and every
// value, with multiplicity, through unlimited runs, sharded export downloads
// and SQLQuery dependency materialization; visible keys are exactly the
// declared columns (no internal ordering helpers).
// ============================================================================

/// Expanded, union (with duplicate visible rows, a NULL first value) and
/// repeat (duplicate nodes, a node without `linkId`) inputs, in creation
/// order.
fn complex_resources() -> Vec<Value> {
    vec![
        json!({"resourceType": "Patient", "id": "c1",
            "name": [{"family": "Ash", "given": ["A", "B"]}, {"family": "Ash", "given": ["A"]}],
            "address": [{"city": "X"}]}),
        json!({"resourceType": "Patient", "id": "c2", "gender": "female"}),
        json!({"resourceType": "Patient", "id": "c3", "name": [{"given": ["Solo"]}]}),
        json!({"resourceType": "Patient", "id": "c4", "name": [{"family": "Birch"}],
            "address": [{"city": "Y"}, {"city": "Y"}]}),
        json!({"resourceType": "Patient", "id": "c5", "name": [{"family": "Ash"}]}),
        json!({"resourceType": "QuestionnaireResponse", "id": "q1", "status": "completed",
        "item": [
            {"linkId": "a", "answer": [{"valueString": "x", "item": [{"linkId": "a.1"}]}]},
            {"linkId": "b", "item": [{"linkId": "b.1"}]}
        ]}),
        json!({"resourceType": "QuestionnaireResponse", "id": "q2", "status": "completed",
            "item": [{"linkId": "a"}, {"linkId": "a"}]}),
        json!({"resourceType": "QuestionnaireResponse", "id": "q3", "status": "completed",
            "item": [{"answer": [{"valueString": "y"}]}]}),
    ]
}

/// `(case, view, declared columns in SQL projection order, row count)`.
fn complex_views() -> Vec<(&'static str, Value, Vec<&'static str>, usize)> {
    let view = |name: &str, resource: &str, select: Value| {
        json!({"resourceType": "ViewDefinition", "name": name, "status": "active",
            "url": format!("http://example.org/sof/ViewDefinition/{name}"),
            "resource": resource, "select": select})
    };
    vec![
        (
            "expanded",
            view(
                "complex_expanded",
                "Patient",
                json!([
                    {"column": [{"name": "id", "path": "getResourceKey()", "type": "id"}]},
                    {"forEach": "name", "column": [{"name": "family", "path": "family"}]},
                    {"forEachOrNull": "address", "column": [{"name": "city", "path": "city"}]}
                ]),
            ),
            vec!["id", "family", "city"],
            // c1: 2 names x 1 address; c3, c5: 1 name, no address; c4: 1 x 2.
            6,
        ),
        (
            "union",
            view(
                "complex_union",
                "Patient",
                json!([{"unionAll": [
                    {"forEach": "name", "column": [{"name": "value", "path": "family"},
                        {"name": "kind", "path": "'name'"}]},
                    {"forEach": "address", "column": [{"name": "value", "path": "city"},
                        {"name": "kind", "path": "'address'"}]}
                ]}]),
            ),
            vec!["value", "kind"],
            // Names: Ash, Ash, NULL, Birch, Ash; addresses: X, Y, Y.
            8,
        ),
        (
            "repeat",
            view(
                "complex_repeat",
                "QuestionnaireResponse",
                json!([
                    {"column": [{"name": "qr", "path": "getResourceKey()", "type": "id"}]},
                    {"repeat": ["item", "answer.item"],
                        "column": [{"name": "link", "path": "linkId"}]}
                ]),
            ),
            vec!["qr", "link"],
            // q1: a, a.1, b, b.1; q2: a, a; q3: one node without linkId.
            7,
        ),
    ]
}

fn ndjson_rows(bytes: &[u8]) -> Vec<Value> {
    String::from_utf8(bytes.to_vec())
        .expect("utf-8 ndjson")
        .lines()
        .map(|l| serde_json::from_str(l).expect("ndjson line"))
        .collect()
}

/// Rows as canonical strings, sorted: a multiset.
fn multiset(rows: &[Vec<Value>]) -> Vec<String> {
    let mut keys: Vec<String> = rows
        .iter()
        .map(|row| serde_json::to_string(row).expect("row"))
        .collect();
    keys.sort();
    keys
}

/// A raw row's keys: exactly the declared columns (SQLite) or the declared
/// columns whose value is not NULL (the PostgreSQL row mapper omits NULLs),
/// in declared order either way.
fn assert_raw_keys(h: &Harness, row: &Value, columns: &[&str], case: &str) {
    let expected: Vec<String> = columns
        .iter()
        .filter(|c| !h.omits_null_keys || row.get(**c).is_some_and(|v| !v.is_null()))
        .map(|c| (*c).to_string())
        .collect();
    assert_eq!(object_keys(row), expected, "{case}: raw keys of {row}");
    if h.omits_null_keys {
        assert!(
            row.as_object().unwrap().values().all(|v| !v.is_null()),
            "{case}: {row}"
        );
    }
}

async fn complex_export_full_rows_and_shard_schema(h: Harness, shard_rows: usize) {
    for (case, view, columns, total) in complex_views() {
        let raw = h.runner_rows(&view).await;
        assert_eq!(raw.len(), total, "{case}: runner rows {raw:?}");
        let cells: Vec<Vec<Value>> = raw.iter().map(|r| full_row(r, &columns)).collect();
        let unlimited = ndjson_rows(h.sql_run(&view, "_format=ndjson").await.as_bytes());
        assert_eq!(unlimited, raw, "{case}: unlimited $sql-run");
        for row in &unlimited {
            assert_raw_keys(&h, row, &columns, case);
        }
        if case != "expanded" {
            assert!(
                multiset(&cells).windows(2).any(|w| w[0] == w[1]),
                "{case}: the fixture must produce duplicate visible rows: {cells:?}"
            );
        }

        // NDJSON shards concatenated in manifest order are the unlimited
        // rows: same order, multiplicity and keys.
        let shards = h.export_shards(&view, "_format=ndjson").await;
        assert_eq!(shards.len(), total.div_ceil(shard_rows), "{case}: shards");
        let mut exported = Vec::new();
        for shard in &shards {
            let rows = ndjson_rows(shard);
            assert!(
                !rows.is_empty() && rows.len() <= shard_rows,
                "{case}: shard of {} rows",
                rows.len()
            );
            exported.extend(rows);
        }
        assert_eq!(exported, unlimited, "{case}: ndjson export rows");
        for (exported, run) in exported.iter().zip(&unlimited) {
            assert_eq!(object_keys(exported), object_keys(run), "{case}");
        }

        // CSV and Parquet shards each carry the declared schema, and their
        // rows concatenate to the same full rows.
        let csv_shards = h.export_shards(&view, "_format=csv&header=true").await;
        assert_eq!(csv_shards.len(), shards.len(), "{case}: csv shards");
        let mut csv_rows = Vec::new();
        for shard in &csv_shards {
            let text = String::from_utf8(shard.clone()).expect("utf-8 csv");
            let mut lines = text.lines();
            assert_eq!(lines.next(), Some(columns.join(",").as_str()), "{case}");
            csv_rows.extend(lines.map(str::to_string));
        }
        assert_eq!(
            csv_rows,
            cells.iter().map(|r| csv_line(r)).collect::<Vec<_>>(),
            "{case}: csv export rows"
        );
        let parquet_shards = h.export_shards(&view, "_format=parquet").await;
        assert_eq!(parquet_shards.len(), shards.len(), "{case}: parquet shards");
        let mut parquet_rows = Vec::new();
        for shard in &parquet_shards {
            let (fields, rows) = read_parquet(shard);
            assert_eq!(fields, names(&columns), "{case}: parquet shard schema");
            parquet_rows.extend(rows.iter().map(|r| full_row(r, &columns)));
        }
        assert_eq!(parquet_rows, cells, "{case}: parquet export rows");
    }
}

async fn complex_run_keys_are_exactly_the_declared_columns(h: Harness) {
    for (case, view, columns, total) in complex_views() {
        let raw = h.runner_rows(&view).await;
        let buffered: Vec<Value> = h.sql_run(&view, "_format=json").await.json();
        assert_eq!(buffered.len(), total, "{case}");
        for row in &buffered {
            assert_eq!(object_keys(row), names(&columns), "{case}: json keys");
        }
        let cells: Vec<Vec<Value>> = buffered.iter().map(|r| full_row(r, &columns)).collect();
        assert_eq!(
            cells,
            raw.iter()
                .map(|r| full_row(r, &columns))
                .collect::<Vec<_>>(),
            "{case}: json rows"
        );
        let streamed = ndjson_rows(h.sql_run(&view, "_format=ndjson").await.as_bytes());
        assert_eq!(streamed.len(), total, "{case}");
        for row in &streamed {
            assert_raw_keys(&h, row, &columns, case);
        }
        // Previews carry the same keys.
        let limited: Vec<Value> = h.sql_run(&view, "_format=json&_limit=2").await.json();
        assert_eq!(limited, buffered[..2], "{case}: preview prefix");
    }
}

/// Runs a SQLQuery over the dependency `t` = `view` (supplied inline) with
/// `_limit` on the final query.
async fn sql_query_json(h: &Harness, view: &Value, sql: &str, limit: Option<i64>) -> Vec<Value> {
    use base64::Engine as _;
    let library = json!({
        "resourceType": "Library", "status": "active",
        "type": {"coding": [{"system": "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes",
            "code": "sql-query"}]},
        "content": [{"contentType": "application/sql",
            "data": base64::engine::general_purpose::STANDARD.encode(sql)}],
        "relatedArtifact": [{"type": "depends-on", "label": "t", "resource": view["url"]}]
    });
    let mut parameters = vec![
        json!({"name": "_format", "valueCode": "json"}),
        json!({"name": "subjectResource", "resource": library}),
        json!({"name": "context", "resource": view}),
    ];
    if let Some(limit) = limit {
        parameters.push(json!({"name": "_limit", "valueInteger": limit}));
    }
    let response = h
        .server
        .post("/$sql-run")
        .add_header(X_TENANT_ID, h.tenant_header())
        .add_header(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/fhir+json"),
        )
        .json(&json!({"resourceType": "Parameters", "parameter": parameters}))
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "{}",
        response.text()
    );
    response.json()
}

async fn complex_dependency_full_contents(h: Harness, cases: &[&str]) {
    for (case, view, columns, total) in complex_views()
        .into_iter()
        .filter(|(case, ..)| cases.contains(case))
    {
        let raw = h.runner_rows(&view).await;
        let expected = multiset(
            &raw.iter()
                .map(|r| full_row(r, &columns))
                .collect::<Vec<_>>(),
        );
        let list = columns
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");

        // `_limit=1` caps the final query only: one aggregate row that holds
        // every dependency row.
        let aggregate = sql_query_json(
            &h,
            &view,
            &format!(
                "SELECT COUNT(*) AS n, json_group_array(json_array({list})) AS all_rows FROM t"
            ),
            Some(1),
        )
        .await;
        assert_eq!(aggregate.len(), 1, "{case}: {aggregate:?}");
        assert_eq!(aggregate[0]["n"], json!(total), "{case}");
        let materialized: Vec<Vec<Value>> = serde_json::from_str(
            aggregate[0]["all_rows"]
                .as_str()
                .expect("json_group_array text"),
        )
        .expect("aggregated rows");
        assert_eq!(
            multiset(&materialized),
            expected,
            "{case}: dependency contents"
        );

        // Every row and value, through `SELECT *` ordered by all columns.
        let rows =
            sql_query_json(&h, &view, &format!("SELECT * FROM t ORDER BY {list}"), None).await;
        assert_eq!(rows.len(), total, "{case}");
        let selected: Vec<Vec<Value>> = rows.iter().map(|r| full_row(r, &columns)).collect();
        assert_eq!(multiset(&selected), expected, "{case}: SELECT * contents");
        let limited = sql_query_json(
            &h,
            &view,
            &format!("SELECT * FROM t ORDER BY {list}"),
            Some(1),
        )
        .await;
        assert_eq!(limited, rows[..1], "{case}: _limit caps the final query");
    }
}

macro_rules! declared_column_scenarios {
    ($harness:path) => {
        #[tokio::test]
        async fn buffered_json_keeps_columns_omitted_by_the_first_row() {
            let h = $harness(super::demographics_patients(), 2).await;
            super::buffered_json_keeps_columns_omitted_by_the_first_row(h).await;
        }

        #[tokio::test]
        async fn buffered_csv_keeps_columns_omitted_by_the_first_row() {
            let h = $harness(super::demographics_patients(), 2).await;
            super::buffered_csv_keeps_columns_omitted_by_the_first_row(h).await;
        }

        #[tokio::test]
        async fn buffered_parquet_and_arrow_keep_columns_omitted_by_the_first_row() {
            let h = $harness(super::demographics_patients(), 2).await;
            super::buffered_parquet_and_arrow_keep_columns_omitted_by_the_first_row(h).await;
        }

        #[tokio::test]
        async fn fhir_parameters_keep_later_values_and_omit_null_cells() {
            let h = $harness(super::demographics_patients(), 2).await;
            super::fhir_parameters_keep_later_values_and_omit_null_cells(h).await;
        }

        #[tokio::test]
        async fn empty_buffered_results_keep_the_declared_columns() {
            let h = $harness(super::demographics_patients(), 2).await;
            super::empty_buffered_results_keep_the_declared_columns(h).await;
        }

        #[tokio::test]
        async fn union_columns_are_deduplicated_in_sql_projection_order() {
            let h = $harness(super::union_patients(), 2).await;
            super::union_columns_are_deduplicated_in_sql_projection_order(h).await;
        }

        #[tokio::test]
        async fn sharded_csv_and_parquet_exports_keep_every_declared_column() {
            let h = $harness(super::sharded_patients(), 2).await;
            super::sharded_csv_and_parquet_exports_keep_every_declared_column(h).await;
        }

        #[tokio::test]
        async fn raw_row_objects_stay_what_the_runner_yields() {
            let h = $harness(super::sharded_patients(), 2).await;
            super::raw_row_objects_stay_what_the_runner_yields(h).await;
        }

        #[tokio::test]
        async fn enveloped_ndjson_keeps_first_row_columns() {
            let h = $harness(super::demographics_patients(), 2).await;
            super::enveloped_ndjson_keeps_first_row_columns(h).await;
        }
    };
}

/// Seeds `resources` in order, one create each, so creation order is the
/// ordinary-select row order.
async fn seed<B>(backend: &B, tenant: &TenantContext, resources: Vec<Value>)
where
    B: helios_persistence::core::ResourceStorage,
{
    for resource in resources {
        let resource_type = resource["resourceType"]
            .as_str()
            .expect("resourceType")
            .to_string();
        backend
            .create(
                tenant,
                &resource_type,
                resource,
                helios_fhir::FhirVersion::R4,
            )
            .await
            .expect("seed resource");
        // Distinct `last_updated` values, so creation order is row order.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use super::*;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::export::{InMemoryController, InMemorySink};

    async fn harness(resources: Vec<Value>, shard_rows: usize) -> Harness {
        let backend = SqliteBackend::with_config(":memory:", Default::default()).expect("sqlite");
        backend.init_schema().expect("schema");
        let backend = Arc::new(backend);
        let tenant_id = "declared-columns".to_string();
        let tenant =
            TenantContext::new(TenantId::new(&tenant_id), TenantPermissions::full_access());
        seed(backend.as_ref(), &tenant, resources).await;
        let runner = backend.sof_runner().expect("in-DB runner");
        assert_eq!(runner.runner_name(), "sqlite-indb");
        let controller = InMemoryController::with_shard_rows(
            Arc::clone(&runner),
            InMemorySink::new("http://localhost"),
            None,
            Some(shard_rows),
        );
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(Arc::clone(&runner))
            .with_export_controller(Arc::new(controller));
        let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
            .expect("server");
        Harness {
            server,
            runner,
            tenant,
            tenant_id,
            omits_null_keys: false,
        }
    }

    declared_column_scenarios!(harness);

    #[tokio::test]
    async fn test_sqlite_complex_export_full_rows_and_shard_schema() {
        let h = harness(super::complex_resources(), 3).await;
        super::complex_export_full_rows_and_shard_schema(h, 3).await;
    }

    #[tokio::test]
    async fn test_sqlite_complex_dependency_full_contents() {
        let h = harness(super::complex_resources(), 3).await;
        super::complex_dependency_full_contents(h, &["expanded", "repeat"]).await;
    }

    /// A `unionAll` dependency (branches declare the same columns) must
    /// materialize too. Before #1623 the dependency table was created from
    /// `TableSchema::from_view_definition`, which repeats every branch's
    /// columns, so `CREATE TABLE` reported a duplicate column.
    #[tokio::test]
    async fn test_sqlite_union_dependency_full_contents() {
        let h = harness(super::complex_resources(), 3).await;
        super::complex_dependency_full_contents(h, &["union"]).await;
    }

    #[tokio::test]
    async fn test_sqlite_complex_run_keys_are_exactly_the_declared_columns() {
        let h = harness(super::complex_resources(), 3).await;
        super::complex_run_keys_are_exactly_the_declared_columns(h).await;
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::tenant::{TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::export::{InMemoryController, InMemorySink};
    use std::path::PathBuf;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    struct SharedPg {
        host: String,
        port: u16,
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    fn config(host: &str, port: u16) -> PostgresConfig {
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));
        PostgresConfig {
            host: host.to_string(),
            port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir: Some(data_dir),
            ..Default::default()
        }
    }

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                // PG 16: the backend sends `plan_cache_mode` as a startup
                // option, which the module's default PG 11 rejects.
                let container = crate::container_cleanup::with_cleanup_label(
                    Postgres::default()
                        .with_tag("16-alpine")
                        .with_label("github.run_id", &run_id),
                )
                .start()
                .await
                .expect("failed to start PostgreSQL container");
                let port = container.get_host_port_ipv4(5432).await.expect("host port");
                let host = container.get_host().await.expect("host").to_string();
                let backend = PostgresBackend::new(config(&host, port))
                    .await
                    .expect("PostgresBackend");
                backend.init_schema().await.expect("schema");
                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    async fn harness(resources: Vec<Value>, shard_rows: usize) -> Harness {
        let pg = shared_pg().await;
        let backend = Arc::new(
            PostgresBackend::new(config(&pg.host, pg.port))
                .await
                .expect("PostgresBackend"),
        );
        let tenant_id = format!("declared_columns_{}", uuid::Uuid::new_v4().simple());
        let tenant =
            TenantContext::new(TenantId::new(&tenant_id), TenantPermissions::full_access());
        seed(backend.as_ref(), &tenant, resources).await;
        let runner = backend.sof_runner().expect("in-DB runner");
        assert_eq!(runner.runner_name(), "postgres-indb");
        let controller = InMemoryController::with_shard_rows(
            Arc::clone(&runner),
            InMemorySink::new("http://localhost"),
            None,
            Some(shard_rows),
        );
        let state = helios_rest::AppState::new(Arc::clone(&backend), ServerConfig::for_testing())
            .with_sof_runner(Arc::clone(&runner))
            .with_export_controller(Arc::new(controller));
        let server = TestServer::new(helios_rest::routing::fhir_routes::create_routes(state))
            .expect("server");
        Harness {
            server,
            runner,
            tenant,
            tenant_id,
            omits_null_keys: true,
        }
    }

    declared_column_scenarios!(harness);

    #[tokio::test]
    async fn test_pg_complex_export_full_rows_and_shard_schema() {
        let h = harness(super::complex_resources(), 3).await;
        super::complex_export_full_rows_and_shard_schema(h, 3).await;
    }

    #[tokio::test]
    async fn test_pg_complex_dependency_full_contents() {
        let h = harness(super::complex_resources(), 3).await;
        super::complex_dependency_full_contents(h, &["expanded", "repeat"]).await;
    }

    /// A `unionAll` dependency (branches declare the same columns) must
    /// materialize too. Before #1623 the dependency table was created from
    /// `TableSchema::from_view_definition`, which repeats every branch's
    /// columns, so `CREATE TABLE` reported a duplicate column.
    #[tokio::test]
    async fn test_pg_union_dependency_full_contents() {
        let h = harness(super::complex_resources(), 3).await;
        super::complex_dependency_full_contents(h, &["union"]).await;
    }

    #[tokio::test]
    async fn test_pg_complex_run_keys_are_exactly_the_declared_columns() {
        let h = harness(super::complex_resources(), 3).await;
        super::complex_run_keys_are_exactly_the_declared_columns(h).await;
    }
}
