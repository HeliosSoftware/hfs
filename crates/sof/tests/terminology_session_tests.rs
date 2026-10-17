//! Every row, where clause, forEach/repeat item and streamed chunk of one ViewDefinition run
//! shares one FHIRPath `TerminologySession` (#1802): identical lookups are sent once and the
//! `FHIRPATH_TERMINOLOGY_MAX_CALLS` budget spans the run. helios-sof takes the terminology
//! server from `FHIRPATH_TERMINOLOGY_SERVER`, so these tests set process env and `ENV_LOCK`
//! serializes them.

use std::io::Cursor;

use helios_sof::{
    ChunkConfig, ContentType, PreparedViewDefinition, ResourceChunk, SofBundle, SofError,
    SofViewDefinition, process_ndjson_chunked, run_view_definition,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const VS: &str = "http://example.org/fhir/ValueSet/test";

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn set_env(key: &str, value: Option<&str>) {
    // SAFETY: ENV_LOCK serializes every test in this binary, and only the holding test's view
    // run reads these variables.
    unsafe {
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}

/// Points helios-sof at the stub server and restores the previous env on drop.
struct TerminologyEnv {
    saved: Vec<(&'static str, Option<String>)>,
    // Declared after `saved` so `Drop` restores the env before the lock is released.
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl TerminologyEnv {
    async fn new(server: &MockServer, max_calls: Option<&str>) -> Self {
        Self::with_server_url(Some(&server.uri()), max_calls).await
    }

    /// Like [`Self::new`], with an explicit server URL; `None` leaves no server configured.
    async fn with_server_url(url: Option<&str>, max_calls: Option<&str>) -> Self {
        let guard = ENV_LOCK.lock().await;
        let keys = [
            "FHIRPATH_TERMINOLOGY_SERVER",
            "FHIRPATH_TERMINOLOGY_MAX_CALLS",
        ];
        let saved = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        set_env("FHIRPATH_TERMINOLOGY_SERVER", url);
        set_env("FHIRPATH_TERMINOLOGY_MAX_CALLS", max_calls);
        Self {
            saved,
            _guard: guard,
        }
    }
}

impl Drop for TerminologyEnv {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            set_env(key, value.as_deref());
        }
    }
}

/// A terminology server whose `$validate-code` always answers `result=true`.
async fn terminology_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "result", "valueBoolean": true}]
        })))
        .mount(&server)
        .await;
    server
}

async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

fn patient(id: &str, gender: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "gender": gender,
        "name": [{"use": "official", "family": "Smith"}]
    })
}

fn patient_with_use(id: &str, name_use: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "gender": "female",
        "name": [{"use": name_use, "family": "Smith"}]
    })
}

fn bundle(resources: &[Value]) -> SofBundle {
    let mut bundle_json = json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": []
    });
    let entries = bundle_json["entry"].as_array_mut().unwrap();
    for resource in resources {
        entries.push(json!({ "resource": resource }));
    }
    SofBundle::R4(serde_json::from_value(bundle_json).expect("valid R4 bundle"))
}

fn view(view_json: Value) -> SofViewDefinition {
    let mut v = view_json;
    let obj = v.as_object_mut().unwrap();
    obj.insert("resourceType".into(), "ViewDefinition".into());
    obj.insert("status".into(), "active".into());
    SofViewDefinition::R4(serde_json::from_value(v).expect("valid R4 ViewDefinition"))
}

/// Looks up the same codes from the where clause, a row column, a forEach item column and a
/// nested select column, so every kind of evaluation context takes part.
fn shared_lookup_view() -> SofViewDefinition {
    view(json!({
        "resource": "Patient",
        "where": [{"path": format!("gender.memberOf('{VS}')")}],
        "select": [
            {"column": [
                {"name": "id", "path": "id"},
                {"name": "gender_in_vs", "path": format!("gender.memberOf('{VS}')")}
            ]},
            {
                "forEach": "name",
                "column": [{"name": "use_in_vs", "path": format!("use.memberOf('{VS}')")}],
                "select": [{"column": [
                    {"name": "nested_use_in_vs", "path": format!("use.memberOf('{VS}')")}
                ]}]
            }
        ]
    }))
}

fn gender_view() -> SofViewDefinition {
    view(json!({
        "resource": "Patient",
        "select": [{"column": [
            {"name": "id", "path": "id"},
            {"name": "gender_in_vs", "path": format!("gender.memberOf('{VS}')")}
        ]}]
    }))
}

fn assert_call_limit_error(err: SofError, expected_fragment: &str) {
    match err {
        SofError::FhirPathError(msg) => {
            assert!(msg.contains(expected_fragment), "unexpected message: {msg}");
            assert!(
                msg.contains("FHIRPATH_TERMINOLOGY_MAX_CALLS"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected SofError::FhirPathError, got {other:?}"),
    }
}

fn twenty_patients() -> Vec<Value> {
    (0..20)
        .map(|i| patient(&format!("p{i}"), "female"))
        .collect()
}

fn ndjson(resources: &[Value]) -> Cursor<Vec<u8>> {
    let mut bytes = Vec::new();
    for resource in resources {
        bytes.extend_from_slice(resource.to_string().as_bytes());
        bytes.push(b'\n');
    }
    Cursor::new(bytes)
}

fn assert_all_lookups_true(row: &Value) {
    for column in ["gender_in_vs", "use_in_vs", "nested_use_in_vs"] {
        assert_eq!(row[column], json!(true), "column {column} in row {row}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn identical_lookups_are_sent_once_per_view_run() {
    let server = terminology_stub().await;
    let _env = TerminologyEnv::new(&server, None).await;
    let patients = twenty_patients();

    let output = run_view_definition(shared_lookup_view(), bundle(&patients), ContentType::Json)
        .expect("view run succeeds");
    let rows: Vec<Value> = serde_json::from_slice(&output).unwrap();

    assert_eq!(rows.len(), 20);
    // Asserting `true` matters: non-terminology errors in forEach item columns still become null.
    rows.iter().for_each(assert_all_lookups_true);
    // One lookup for 'female', one for 'official'.
    assert_eq!(request_count(&server).await, 2);

    // A second run starts a new session: no global cache.
    run_view_definition(shared_lookup_view(), bundle(&patients), ContentType::Json)
        .expect("second view run succeeds");
    assert_eq!(request_count(&server).await, 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn streamed_chunks_share_one_session() {
    let server = terminology_stub().await;
    let _env = TerminologyEnv::new(&server, None).await;

    let mut out: Vec<u8> = Vec::new();
    let stats = process_ndjson_chunked(
        shared_lookup_view(),
        ndjson(&twenty_patients()),
        &mut out,
        ContentType::NdJson,
        ChunkConfig {
            chunk_size: 5,
            skip_invalid_lines: false,
        },
    )
    .expect("streamed run succeeds");

    assert_eq!(stats.chunks_processed, 4);
    assert_eq!(stats.output_rows, 20);
    let text = String::from_utf8(out).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 20);
    rows.iter().for_each(assert_all_lookups_true);
    assert_eq!(request_count(&server).await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn call_limit_spans_all_rows_of_a_run() {
    let server = terminology_stub().await;
    let _env = TerminologyEnv::new(&server, Some("1")).await;
    let patients = [patient("p1", "male"), patient("p2", "female")];

    let err = run_view_definition(gender_view(), bundle(&patients), ContentType::Json).unwrap_err();

    assert_call_limit_error(err, "Error evaluating column 'gender_in_vs'");
    assert_eq!(request_count(&server).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn call_limit_spans_all_chunks_of_a_streamed_run() {
    let server = terminology_stub().await;
    let _env = TerminologyEnv::new(&server, Some("1")).await;
    let patients = [patient("p1", "male"), patient("p2", "female")];

    let mut out: Vec<u8> = Vec::new();
    let err = process_ndjson_chunked(
        gender_view(),
        ndjson(&patients),
        &mut out,
        ContentType::NdJson,
        ChunkConfig {
            chunk_size: 1,
            skip_invalid_lines: false,
        },
    )
    .unwrap_err();

    assert_call_limit_error(err, "Error evaluating column 'gender_in_vs'");
    assert_eq!(request_count(&server).await, 1);
    // Chunk 1 was flushed before chunk 2 failed.
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "output: {text}");
    let row: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(row["id"], json!("p1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn repeat_items_share_one_session() {
    let server = terminology_stub().await;
    let _env = TerminologyEnv::new(&server, None).await;
    let questionnaires: Vec<Value> = (0..20)
        .map(|i| {
            json!({
                "resourceType": "Questionnaire",
                "id": format!("q{i}"),
                "status": "active",
                "item": [{
                    "linkId": "1",
                    "type": "group",
                    "item": [{"linkId": "1.1", "type": "group"}]
                }]
            })
        })
        .collect();
    let repeat_view = view(json!({
        "resource": "Questionnaire",
        "select": [{
            "repeat": ["item"],
            "column": [
                {"name": "link_id", "path": "linkId"},
                {"name": "type_in_vs", "path": format!("type.memberOf('{VS}')")}
            ]
        }]
    }));

    let output = run_view_definition(repeat_view, bundle(&questionnaires), ContentType::Json)
        .expect("view run succeeds");
    let rows: Vec<Value> = serde_json::from_slice(&output).unwrap();

    // Two repeat items per questionnaire.
    assert_eq!(rows.len(), 40);
    // Asserting `true` matters: non-terminology errors in item columns still become null.
    for row in &rows {
        assert_eq!(row["type_in_vs"], json!(true), "row {row}");
    }
    assert_eq!(request_count(&server).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn foreach_or_null_and_union_all_items_share_one_session() {
    let server = terminology_stub().await;
    let _env = TerminologyEnv::new(&server, None).await;
    let or_null_and_union_view = view(json!({
        "resource": "Patient",
        "select": [
            {
                "forEachOrNull": "name",
                "column": [{"name": "or_null_use_in_vs", "path": format!("use.memberOf('{VS}')")}]
            },
            {
                "forEach": "name",
                "unionAll": [
                    {"column": [{"name": "union_use_in_vs", "path": format!("use.memberOf('{VS}')")}]},
                    {"column": [{"name": "union_use_in_vs", "path": format!("use.memberOf('{VS}')")}]}
                ]
            }
        ]
    }));

    let output = run_view_definition(
        or_null_and_union_view,
        bundle(&twenty_patients()),
        ContentType::Json,
    )
    .expect("view run succeeds");
    let rows: Vec<Value> = serde_json::from_slice(&output).unwrap();

    // 1 forEachOrNull row x 2 unionAll rows per patient.
    assert_eq!(rows.len(), 40);
    for row in &rows {
        assert_eq!(row["or_null_use_in_vs"], json!(true), "row {row}");
        assert_eq!(row["union_use_in_vs"], json!(true), "row {row}");
    }
    assert_eq!(request_count(&server).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn call_limit_in_foreach_item_column_fails_the_run() {
    let server = terminology_stub().await;
    let _env = TerminologyEnv::new(&server, Some("1")).await;
    let patients = [
        patient_with_use("p1", "official"),
        patient_with_use("p2", "usual"),
    ];
    let item_view = view(json!({
        "resource": "Patient",
        "select": [{
            "forEach": "name",
            "column": [{"name": "use_in_vs", "path": format!("use.memberOf('{VS}')")}]
        }]
    }));

    let err = run_view_definition(item_view, bundle(&patients), ContentType::Json).unwrap_err();

    assert_call_limit_error(err, "on a forEach/repeat item");
    assert_eq!(request_count(&server).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn call_limit_in_where_clause_fails_the_run() {
    let server = terminology_stub().await;
    let _env = TerminologyEnv::new(&server, Some("1")).await;
    let patients = [patient("p1", "male"), patient("p2", "female")];
    let where_view = view(json!({
        "resource": "Patient",
        "where": [{"path": format!("gender.memberOf('{VS}')")}],
        "select": [{"column": [{"name": "id", "path": "id"}]}]
    }));

    let err = run_view_definition(where_view, bundle(&patients), ContentType::Json).unwrap_err();

    assert_call_limit_error(err, "Error evaluating where clause");
    assert_eq!(request_count(&server).await, 1);
}

/// A terminology server whose `$validate-code` always answers `status`.
async fn status_stub(status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .respond_with(ResponseTemplate::new(status).set_body_string("stub failure"))
        .mount(&server)
        .await;
    server
}

/// A terminology server whose first `$validate-code` answer is a 500 and every later one
/// `result=true`.
async fn flaky_stub() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .respond_with(ResponseTemplate::new(500).set_body_string("try later"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/ValueSet/$validate-code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "result", "valueBoolean": true}]
        })))
        .mount(&server)
        .await;
    server
}

fn foreach_use_view() -> SofViewDefinition {
    view(json!({
        "resource": "Patient",
        "select": [{
            "forEach": "name",
            "column": [{"name": "use_in_vs", "path": format!("use.memberOf('{VS}')")}]
        }]
    }))
}

fn assert_item_error(err: SofError, fragment: &str) {
    match err {
        SofError::FhirPathError(msg) => {
            assert!(
                msg.contains("on a forEach/repeat item"),
                "unexpected message: {msg}"
            );
            assert!(msg.contains(fragment), "unexpected message: {msg}");
        }
        other => panic!("expected SofError::FhirPathError, got {other:?}"),
    }
}

fn chunk_of(index: usize) -> ResourceChunk {
    ResourceChunk {
        resources: vec![patient_with_use("p1", "official")],
        chunk_index: index,
        is_last: false,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn server_error_in_foreach_item_column_fails_the_run() {
    let server = status_stub(500).await;
    let _env = TerminologyEnv::new(&server, None).await;
    let patients = [patient_with_use("p1", "official")];

    let err =
        run_view_definition(foreach_use_view(), bundle(&patients), ContentType::Json).unwrap_err();

    assert_item_error(err, "500");
    assert_eq!(request_count(&server).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_value_set_in_foreach_item_column_fails_the_run_once() {
    let server = status_stub(404).await;
    let _env = TerminologyEnv::new(&server, None).await;
    let patients = twenty_patients();

    let err =
        run_view_definition(foreach_use_view(), bundle(&patients), ContentType::Json).unwrap_err();

    assert_item_error(err, "404");
    // a deterministic failure is cached, and rows running in parallel wait on the one request
    assert_eq!(request_count(&server).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_terminology_server_fails_foreach_item_column() {
    let _env = TerminologyEnv::with_server_url(None, None).await;
    let patients = [patient_with_use("p1", "official")];

    let err =
        run_view_definition(foreach_use_view(), bundle(&patients), ContentType::Json).unwrap_err();

    assert_item_error(err, "No terminology server is configured");
}

#[tokio::test(flavor = "multi_thread")]
async fn server_errors_still_fail_top_level_columns_and_where_clauses() {
    let server = status_stub(500).await;
    let _env = TerminologyEnv::new(&server, None).await;
    let patients = [patient("p1", "male")];

    let err = run_view_definition(gender_view(), bundle(&patients), ContentType::Json).unwrap_err();
    match err {
        SofError::FhirPathError(msg) => {
            assert!(
                msg.contains("Error evaluating column 'gender_in_vs'"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected SofError::FhirPathError, got {other:?}"),
    }

    let where_view = view(json!({
        "resource": "Patient",
        "where": [{"path": format!("gender.memberOf('{VS}')")}],
        "select": [{"column": [{"name": "id", "path": "id"}]}]
    }));
    let err = run_view_definition(where_view, bundle(&patients), ContentType::Json).unwrap_err();
    match err {
        SofError::FhirPathError(msg) => {
            assert!(
                msg.contains("Error evaluating where clause"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected SofError::FhirPathError, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn transient_failure_is_retried_by_a_later_chunk() {
    let server = flaky_stub().await;
    let _env = TerminologyEnv::new(&server, None).await;
    let prepared = PreparedViewDefinition::new(foreach_use_view()).unwrap();

    let err = prepared.process_chunk(chunk_of(0)).unwrap_err();
    assert_item_error(err, "500");
    assert_eq!(request_count(&server).await, 1);

    let result = prepared.process_chunk(chunk_of(1)).expect("retry succeeds");
    assert_eq!(result.rows[0].values[0], Some(json!(true)));
    assert_eq!(request_count(&server).await, 2);

    let result = prepared.process_chunk(chunk_of(2)).expect("cached success");
    assert_eq!(result.rows[0].values[0], Some(json!(true)));
    assert_eq!(request_count(&server).await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn deterministic_failure_is_not_resent_by_a_later_chunk() {
    let server = status_stub(404).await;
    let _env = TerminologyEnv::new(&server, None).await;
    let prepared = PreparedViewDefinition::new(foreach_use_view()).unwrap();

    assert_item_error(prepared.process_chunk(chunk_of(0)).unwrap_err(), "404");
    assert_item_error(prepared.process_chunk(chunk_of(1)).unwrap_err(), "404");
    assert_eq!(request_count(&server).await, 1);
}
