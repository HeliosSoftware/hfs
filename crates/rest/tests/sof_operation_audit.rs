//! `$sql-run` and `$sql-export` record their scope decision as an AuditEvent
//! (#2661): outcome `0` naming the caller, the operation and every resource
//! type the subject reads, and outcome `8` for a refusal. Neither middleware
//! audits these `POST`s, so without it a caller could read a whole resource
//! type through a view and leave no trace.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum_test::TestServer;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use helios_audit::{AuditConfig, AuditMiddlewareState, AuditSink, ExclusionFilter};
use helios_auth::{AuthConfig, AuthError, AuthProvider, Principal, ScopeSet};
use helios_fhir::r4::AuditEvent;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_rest::{AuthMiddlewareState, ServerConfig};
use serde_json::{Value, json};

const AUTHORIZATION: HeaderName = HeaderName::from_static("authorization");
const PREFER: HeaderName = HeaderName::from_static("prefer");
const LIB_TYPE_SYSTEM: &str = "https://sql-on-fhir.org/ig/CodeSystem/LibraryTypesCodes";
const VIEW_URL: &str = "http://example.org/ViewDefinition/observations";
const LIBRARY_URL: &str = "http://example.org/Library/observation-ids";

#[derive(Clone, Default)]
struct CollectorSink {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

#[async_trait]
impl AuditSink for CollectorSink {
    async fn record(&self, event: AuditEvent) {
        self.events.lock().expect("collector poisoned").push(event);
    }
    async fn flush(&self) {}
    fn name(&self) -> &str {
        "collector"
    }
}

impl CollectorSink {
    /// The events naming `operation` in their `audit-operation` detail, as
    /// JSON, oldest first.
    fn for_operation(&self, operation: &str) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| serde_json::to_value(e).unwrap())
            .filter(|e| details(e, "audit-operation") == [operation])
            .collect()
    }

    fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

/// Every `entity.detail` value of `event` named `name`.
fn details(event: &Value, name: &str) -> Vec<String> {
    event["entity"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entity| entity["detail"].as_array().into_iter().flatten())
        .filter(|d| d["type"] == name)
        .filter_map(|d| d["valueString"].as_str().map(str::to_string))
        .collect()
}

struct StubProvider;

#[async_trait]
impl AuthProvider for StubProvider {
    /// The bearer token is the scope string itself; the subject names it.
    async fn authenticate(&self, authorization_header: &str) -> Result<Principal, AuthError> {
        let scopes = authorization_header
            .strip_prefix("Bearer ")
            .ok_or(AuthError::InvalidSignature)?;
        Ok(Principal {
            subject: format!("sub {scopes}"),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(scopes),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            ..Default::default()
        })
    }

    fn name(&self) -> &str {
        "stub"
    }
}

/// The server, its collecting sink, and the directory its `$sql-export` jobs
/// write to, removed when dropped.
fn server() -> (TestServer, CollectorSink, tempfile::TempDir) {
    let exports = tempfile::tempdir().expect("export dir");
    let backend = SqliteBackend::in_memory().expect("in-memory sqlite");
    backend.init_schema().expect("init schema");
    let sink = CollectorSink::default();
    let auth = Arc::new(AuthMiddlewareState {
        provider: Arc::new(StubProvider),
        config: Arc::new(AuthConfig::default()),
        audit_sink: Arc::new(sink.clone()),
        audit_source_observer: "Device/test".to_string(),
        audit_exclusion_filter: ExclusionFilter::new(vec![]),
        tenant_url_routing: false,
        sessions: None,
    });
    let audit = Arc::new(AuditMiddlewareState {
        sink: Arc::new(sink.clone()),
        config: AuditConfig::default(),
        exclusion_filter: ExclusionFilter::new(Vec::new()),
    });
    let app = helios_rest::create_app_with_auth(
        backend,
        ServerConfig {
            export_dir: exports.path().to_string_lossy().into_owned(),
            ..ServerConfig::for_testing()
        },
        AuthConfig::default(),
        Some(auth),
        Some(audit),
    );
    (TestServer::new(app).expect("test server"), sink, exports)
}

fn bearer(scopes: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("Bearer {scopes}")).unwrap()
}

fn observation_view() -> Value {
    json!({
        "resourceType": "ViewDefinition",
        "url": VIEW_URL,
        "status": "active",
        "resource": "Observation",
        "select": [{"column": [{"name": "id", "path": "id"}]}]
    })
}

async fn sql_run(server: &TestServer, scopes: &str, subject: Value) -> StatusCode {
    server
        .post("/$sql-run")
        .add_header(AUTHORIZATION, bearer(scopes))
        .json(&json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_format", "valueCode": "json"},
                {"name": "subjectResource", "resource": subject}
            ]
        }))
        .await
        .status_code()
}

async fn sql_export(server: &TestServer, scopes: &str) -> StatusCode {
    server
        .post("/$sql-export")
        .add_header(AUTHORIZATION, bearer(scopes))
        .add_header(PREFER, HeaderValue::from_static("respond-async"))
        .json(&observation_view())
        .await
        .status_code()
}

/// The one event `sink` holds for `operation`, checked to name `scopes`'s
/// caller and the subject.
fn only_event(sink: &CollectorSink, operation: &str, scopes: &str, subject: &str) -> Value {
    let events = sink.for_operation(operation);
    assert_eq!(events.len(), 1, "one {operation} event: {events:#?}");
    let event = events.into_iter().next().unwrap();
    assert_eq!(event["action"], "E", "{event}");
    assert_eq!(
        event["agent"][0]["who"]["identifier"]["value"],
        format!("sub {scopes}"),
        "{event}"
    );
    assert_eq!(details(&event, "subject"), [subject], "{event}");
    event
}

#[tokio::test]
async fn sql_run_records_a_grant_and_a_refusal() {
    let (server, sink, _exports) = server();

    assert_eq!(
        sql_run(&server, "user/Observation.rs", observation_view()).await,
        StatusCode::OK
    );
    let granted = only_event(&sink, "sql-run", "user/Observation.rs", VIEW_URL);
    assert_eq!(granted["outcome"], "0", "{granted}");
    assert_eq!(
        granted["outcomeDesc"],
        "Granted: $sql-run reading Observation"
    );
    assert_eq!(details(&granted, "resource-type"), ["Observation"]);

    sink.clear();
    assert_eq!(
        sql_run(&server, "user/Appointment.rs", observation_view()).await,
        StatusCode::FORBIDDEN
    );
    let refused = only_event(&sink, "sql-run", "user/Appointment.rs", VIEW_URL);
    assert_eq!(refused["outcome"], "8", "{refused}");
    assert_eq!(
        refused["outcomeDesc"],
        "Forbidden: $sql-run reading Observation"
    );
}

#[tokio::test]
async fn sql_export_records_a_grant_and_a_refusal() {
    let (server, sink, _exports) = server();

    assert_ne!(
        sql_export(&server, "user/Observation.rs").await,
        StatusCode::FORBIDDEN
    );
    let granted = only_event(&sink, "sql-export", "user/Observation.rs", VIEW_URL);
    assert_eq!(granted["outcome"], "0", "{granted}");
    assert_eq!(details(&granted, "resource-type"), ["Observation"]);

    sink.clear();
    assert_eq!(sql_export(&server, "openid").await, StatusCode::FORBIDDEN);
    let refused = only_event(&sink, "sql-export", "openid", VIEW_URL);
    assert_eq!(refused["outcome"], "8", "{refused}");
}

/// A refused caller cannot write an event the size of its request: the
/// subject's name and the type it reads come from the body, and are capped.
#[tokio::test]
async fn caller_supplied_names_are_capped_in_the_event() {
    let (server, sink, _exports) = server();
    let long_url = format!("http://example.org/{}", "a".repeat(200_000));
    let long_type = format!("Zz{}", "b".repeat(200_000));
    let mut view = observation_view();
    view["url"] = json!(long_url);
    view["resource"] = json!(long_type);

    let status = server
        .post("/$sql-export")
        .add_header(AUTHORIZATION, bearer("openid"))
        .add_header(PREFER, HeaderValue::from_static("respond-async"))
        .json(&view)
        .await
        .status_code();
    assert_eq!(status, StatusCode::FORBIDDEN);

    let events = sink.for_operation("sql-export");
    assert_eq!(events.len(), 1, "{events:#?}");
    let event = &events[0];
    assert_eq!(event["outcome"], "8");
    let subject = details(event, "subject").remove(0);
    assert!(long_url.starts_with(subject.trim_end_matches('…')));
    assert_eq!(subject.chars().count(), 257);
    assert_eq!(details(event, "resource-type")[0].chars().count(), 257);
    let size = serde_json::to_string(event).unwrap().len();
    assert!(size < 4096, "event is {size} bytes");
}

/// A Library's event names every type its dependency graph reads, once.
#[tokio::test]
async fn a_library_run_names_the_types_its_views_read() {
    let (server, sink, _exports) = server();
    let stored = server
        .put("/ViewDefinition/observations")
        .add_header(AUTHORIZATION, bearer("system/*.cruds"))
        .json(&{
            let mut view = observation_view();
            view["id"] = json!("observations");
            view
        })
        .await;
    assert!(stored.status_code().is_success(), "{}", stored.text());
    let library = json!({
        "resourceType": "Library",
        "url": LIBRARY_URL,
        "status": "active",
        "type": {"coding": [{"system": LIB_TYPE_SYSTEM, "code": "sql-query"}]},
        "content": [{
            "contentType": "application/sql",
            "data": B64.encode("SELECT a.id FROM a JOIN b ON a.id = b.id")
        }],
        "relatedArtifact": [
            {"type": "depends-on", "label": "a", "resource": VIEW_URL},
            {"type": "depends-on", "label": "b", "resource": VIEW_URL}
        ]
    });
    sink.clear();

    assert_eq!(
        sql_run(&server, "user/Observation.rs", library.clone()).await,
        StatusCode::OK
    );
    let granted = only_event(&sink, "sql-run", "user/Observation.rs", LIBRARY_URL);
    assert_eq!(details(&granted, "resource-type"), ["Observation"]);

    sink.clear();
    assert_eq!(
        sql_run(&server, "user/Patient.rs", library).await,
        StatusCode::FORBIDDEN
    );
    let refused = only_event(&sink, "sql-run", "user/Patient.rs", LIBRARY_URL);
    assert_eq!(refused["outcome"], "8", "{refused}");
}
