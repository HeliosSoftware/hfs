//! Package imports and REST mutations must preserve one coherent resource.

mod common;

use std::sync::Arc;
#[cfg(feature = "postgres")]
use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use helios_hts::{
    config::HtsConfig,
    import::{BundleImportBackend, ImportStats},
    state::AppState,
};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};
use tower::ServiceExt;

struct WriteApp {
    router: Router,
    importer: Arc<dyn BundleImportBackend>,
    #[cfg(feature = "postgres")]
    pool: Option<deadpool_postgres::Pool>,
    _directory: Option<tempfile::TempDir>,
}

impl WriteApp {
    #[cfg(feature = "sqlite")]
    fn sqlite() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("writes.db");
        let store = helios_persistence::backends::sqlite::SqliteBackend::open(&path).unwrap();
        store.init_schema().unwrap();
        let backend =
            helios_hts::backends::SqliteTerminologyBackend::new(path.to_str().unwrap()).unwrap();
        let state = AppState::new(backend.clone()).with_resource_store(store);
        Self {
            router: helios_hts::server::create_app(&HtsConfig::default(), state),
            importer: Arc::new(backend),
            #[cfg(feature = "postgres")]
            pool: None,
            _directory: Some(directory),
        }
    }

    #[cfg(feature = "postgres")]
    async fn postgres() -> Self {
        let shared = common::pg_http_url().await;
        let database = format!("hts_coordination_{}", uuid::Uuid::new_v4().simple());
        let (admin, connection) = tokio_postgres::connect(shared, tokio_postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(connection);
        admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .unwrap();
        admin
            .batch_execute(&format!(
                "ALTER DATABASE {database} SET default_transaction_isolation TO 'repeatable read'"
            ))
            .await
            .unwrap();
        let (base, _) = shared.rsplit_once('/').unwrap();
        let url = format!("{base}/{database}");
        let store =
            helios_persistence::backends::postgres::PostgresBackend::from_connection_string(&url)
                .await
                .unwrap();
        store.init_schema().await.unwrap();
        let backend = helios_hts::backends::PostgresTerminologyBackend::new(&url)
            .await
            .unwrap();
        let pool = backend.pool().clone();
        let state = AppState::new(backend.clone()).with_resource_store_pg(store);
        Self {
            router: helios_hts::server::create_app(&HtsConfig::default(), state),
            importer: Arc::new(backend),
            pool: Some(pool),
            _directory: None,
        }
    }

    async fn import(&self, resources: &[Value]) -> ImportStats {
        import(self.importer.clone(), resources.to_vec()).await
    }

    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        request(self.router.clone(), method, path, body).await
    }

    async fn search(&self, kind: &str, url: &str) -> Vec<Value> {
        let (status, result) = self
            .request("GET", &format!("/{kind}?url={url}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        result["entry"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| entry["resource"].clone())
            .collect()
    }
}

async fn request(
    router: Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/fhir+json")
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

async fn import(importer: Arc<dyn BundleImportBackend>, resources: Vec<Value>) -> ImportStats {
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": resources
            .into_iter()
            .map(|resource| json!({"resource": resource}))
            .collect::<Vec<_>>()
    });
    importer
        .import_bundle(
            &TenantContext::system(),
            &serde_json::to_vec(&bundle).unwrap(),
        )
        .await
        .unwrap()
}

#[cfg(feature = "postgres")]
fn code_system(id: &str, url: &str, version: &str, code: &str) -> Value {
    json!({"resourceType":"CodeSystem", "id":id, "url":url, "version":version,
        "status":"active", "content":"complete", "concept":[{"code":code,"display":code}]})
}

fn resource(kind: &str, id: &str, label: &str) -> Value {
    let system = "http://example.org/write-owner/CodeSystem";
    let mut value = json!({
        "resourceType": kind,
        "id": id,
        "url": format!("http://example.org/write-owner/{kind}"),
        "version": "1",
        "name": label,
        "status": "active"
    });
    match kind {
        "CodeSystem" => {
            value["content"] = json!("complete");
            value["concept"] = json!([{"code":label,"display":label}]);
        }
        "ValueSet" => {
            value["compose"] = json!({"include":[{"system":system,"concept":[{"code":label}]}]})
        }
        "ConceptMap" => {
            value["group"] = json!([{"source":system,"target":"http://example.org/target",
            "element":[{"code":"source","target":[{"code":label,"equivalence":"equivalent"}]}]}])
        }
        _ => panic!("unexpected fixture type"),
    }
    value
}

async fn rest_ownership_contract(app: WriteApp) {
    let originals: Vec<Value> = ["CodeSystem", "ValueSet", "ConceptMap"]
        .into_iter()
        .map(|kind| resource(kind, &kind.to_lowercase(), "package"))
        .collect();
    assert!(app.import(&originals).await.errors.is_empty());
    let mut persisted = Vec::new();
    for original in &originals {
        let kind = original["resourceType"].as_str().unwrap();
        let id = original["id"].as_str().unwrap();
        let (status, updated) = app
            .request(
                "PUT",
                &format!("/{kind}/{id}"),
                Some(resource(kind, id, "rest")),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{updated}");
        persisted.push(updated);
    }
    let mut mixed = originals.clone();
    for original in &originals {
        let mut unrelated = original.clone();
        unrelated["id"] = json!(format!("new-{}", original["id"].as_str().unwrap()));
        unrelated["url"] = json!(format!("{}/new", original["url"].as_str().unwrap()));
        mixed.push(unrelated);
    }
    let stats = app.import(&mixed).await;
    assert_eq!(stats.errors.len(), 3, "{stats:?}");
    assert_eq!(
        (stats.code_systems, stats.value_sets, stats.concept_maps),
        (1, 1, 1)
    );
    for saved in &persisted {
        let kind = saved["resourceType"].as_str().unwrap();
        let id = saved["id"].as_str().unwrap();
        let (status, read) = app.request("GET", &format!("/{kind}/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{read}");
        assert_eq!(
            &read, saved,
            "import must not change REST content or version"
        );
        let rows = app.search(kind, saved["url"].as_str().unwrap()).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "rest");
        assert_eq!(rows[0]["id"], saved["id"]);
    }
    let (status, expanded) = app.request("GET", "/ValueSet/valueset/$expand", None).await;
    assert_eq!(status, StatusCode::OK, "{expanded}");
    assert_eq!(expanded["expansion"]["contains"][0]["code"], "rest");
    let (status, lookup) = app
        .request("GET", "/CodeSystem/codesystem/$lookup?code=rest", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{lookup}");

    // A changed ConceptMap business version does not create a second URL owner.
    let mut conflicting = resource("ConceptMap", "different-map", "replacement");
    conflicting["version"] = json!("2");
    assert_eq!(
        app.request("POST", "/ConceptMap", Some(conflicting))
            .await
            .0,
        StatusCode::CONFLICT
    );
    // Source-ID collision must not bypass canonical ownership by changing URL.
    let mut moved = resource("ConceptMap", "conceptmap", "replacement");
    moved["url"] = json!("http://example.org/changed-map-url");
    assert_eq!(app.import(&[moved]).await.errors.len(), 1);
    assert_eq!(
        app.request("GET", "/ConceptMap/conceptmap", None).await.1,
        persisted[2]
    );

    assert_eq!(
        app.request("DELETE", "/ConceptMap/conceptmap", None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.import(&[originals[2].clone()]).await.errors.len(), 1);
    let mut recreated = originals[2].clone();
    recreated["id"] = json!("new-map-id");
    assert!(app.import(&[recreated]).await.errors.is_empty());
    assert_eq!(
        app.request("GET", "/ConceptMap/conceptmap", None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.request("GET", "/ConceptMap/new-map-id", None).await.0,
        StatusCode::OK
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_reimport_preserves_rest_edits_and_imports_unrelated_resources() {
    rest_ownership_contract(WriteApp::sqlite()).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_reimport_preserves_rest_edits_and_imports_unrelated_resources() {
    rest_ownership_contract(WriteApp::postgres().await).await;
}

#[cfg(feature = "postgres")]
const GATE_KEY: i64 = 2_136_019;

/// Observe PostgreSQL's actual wait graph rather than assuming scheduling from
/// a sleep. The caller releases the gate only after both operations are waiting.
#[cfg(feature = "postgres")]
async fn blocked_by(client: &deadpool_postgres::Client, blocker: i32) -> i32 {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let row = client
                .query_opt(
                    "SELECT pid FROM pg_stat_activity WHERE datname = current_database() \
                     AND $1 = ANY(pg_blocking_pids(pid)) ORDER BY pid LIMIT 1",
                    &[&blocker],
                )
                .await
                .unwrap();
            if let Some(row) = row {
                return row.get(0);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("request reached the database gate")
}

#[cfg(feature = "postgres")]
async fn install_gate(client: &deadpool_postgres::Client, table: &str, event: &str) {
    // The table and event are fixture-owned constants, never request input.
    client
        .batch_execute(&format!(
            "CREATE FUNCTION pause_write() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
             PERFORM pg_advisory_xact_lock({GATE_KEY}); \
             IF TG_OP = 'DELETE' THEN RETURN OLD; END IF; RETURN NEW; END $$; \
             CREATE TRIGGER pause_write BEFORE {event} ON {table} FOR EACH ROW \
             EXECUTE FUNCTION pause_write()"
        ))
        .await
        .unwrap();
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_import_waiting_for_delete_cannot_resurrect_the_deleted_id() {
    let app = WriteApp::postgres().await;
    let original = code_system("deleted", "http://example.org/delete-race", "1", "original");
    assert!(
        app.import(std::slice::from_ref(&original))
            .await
            .errors
            .is_empty()
    );
    let pool = app.pool.as_ref().unwrap();
    let mut gate_client = pool.get().await.unwrap();
    let observer = pool.get().await.unwrap();
    install_gate(&gate_client, "code_systems", "DELETE").await;
    let gate = gate_client.transaction().await.unwrap();
    gate.execute("SELECT pg_advisory_xact_lock($1)", &[&GATE_KEY])
        .await
        .unwrap();
    let gate_pid: i32 = gate
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let deleting = tokio::spawn(request(
        app.router.clone(),
        "DELETE",
        "/CodeSystem/deleted",
        None,
    ));
    let delete_pid = blocked_by(&observer, gate_pid).await;
    let importing = tokio::spawn(import(app.importer.clone(), vec![original]));
    blocked_by(&observer, delete_pid).await;
    gate.rollback().await.unwrap();
    assert_eq!(deleting.await.unwrap().0, StatusCode::NO_CONTENT);
    assert!(importing.await.unwrap().errors.is_empty());
    assert_eq!(
        app.request("GET", "/CodeSystem/deleted", None).await.0,
        StatusCode::NOT_FOUND
    );
    let rows = app
        .search("CodeSystem", "http://example.org/delete-race")
        .await;
    assert_eq!(rows.len(), 1);
    let new_id = rows[0]["id"].as_str().unwrap();
    assert_ne!(new_id, "deleted");
    assert_eq!(
        app.request("GET", &format!("/CodeSystem/{new_id}"), None)
            .await
            .0,
        StatusCode::OK
    );
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_concurrent_import_only_puts_preserve_both_history_updates() {
    let app = WriteApp::postgres().await;
    let url = "http://example.org/put-race";
    assert!(
        app.import(&[code_system("edited", url, "1", "original")])
            .await
            .errors
            .is_empty()
    );
    let pool = app.pool.as_ref().unwrap();
    let mut gate_client = pool.get().await.unwrap();
    let observer = pool.get().await.unwrap();
    install_gate(&gate_client, "resources", "INSERT").await;
    let gate = gate_client.transaction().await.unwrap();
    gate.execute("SELECT pg_advisory_xact_lock($1)", &[&GATE_KEY])
        .await
        .unwrap();
    let gate_pid: i32 = gate
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let first = tokio::spawn(request(
        app.router.clone(),
        "PUT",
        "/CodeSystem/edited",
        Some(code_system("edited", url, "1", "first")),
    ));
    let first_pid = blocked_by(&observer, gate_pid).await;
    let second = tokio::spawn(request(
        app.router.clone(),
        "PUT",
        "/CodeSystem/edited",
        Some(code_system("edited", url, "1", "second")),
    ));
    blocked_by(&observer, first_pid).await;
    gate.rollback().await.unwrap();
    let (first_status, first_body) = first.await.unwrap();
    let (second_status, second_body) = second.await.unwrap();
    assert_eq!(first_status, StatusCode::OK, "{first_body}");
    assert_eq!(second_status, StatusCode::OK, "{second_body}");
    let tenant = TenantContext::system();
    let history = observer
        .query(
            "SELECT version_id, data->'concept'->0->>'code'
             FROM resource_history
             WHERE tenant_id = $1 AND resource_type = 'CodeSystem' AND id = 'edited'
             ORDER BY version_id",
            &[&tenant.tenant_id().as_str()],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect::<Vec<_>>();
    assert_eq!(
        history,
        vec![
            ("1".to_owned(), "original".to_owned()),
            ("2".to_owned(), "first".to_owned()),
            ("3".to_owned(), "second".to_owned()),
        ],
        "both PUTs must retain the imported baseline and their own history versions"
    );
    assert_eq!(second_body["concept"][0]["code"], "second");
    assert_eq!(
        app.request("GET", "/CodeSystem/edited", None).await.1,
        second_body
    );
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_replace_locks_old_canonical_even_when_source_and_public_ids_differ() {
    let app = WriteApp::postgres().await;
    let old_url = "http://example.org/old-canonical";
    let new_url = "http://example.org/new-canonical";
    let original = code_system("shared", old_url, "2", "old");
    assert!(
        app.import(&[
            code_system("shared", old_url, "1", "sibling"),
            original.clone()
        ])
        .await
        .errors
        .is_empty()
    );
    let rows = app.search("CodeSystem", old_url).await;
    let old_id = rows.iter().find(|row| row["version"] == "2").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(old_id, "shared");
    let pool = app.pool.as_ref().unwrap();
    let mut gate_client = pool.get().await.unwrap();
    let observer = pool.get().await.unwrap();
    install_gate(&gate_client, "code_systems", "DELETE").await;
    let gate = gate_client.transaction().await.unwrap();
    gate.execute("SELECT pg_advisory_xact_lock($1)", &[&GATE_KEY])
        .await
        .unwrap();
    let gate_pid: i32 = gate
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let path = format!("/CodeSystem/{old_id}");
    let router = app.router.clone();
    let replacement = code_system(&old_id, new_url, "3", "edited");
    let replacing =
        tokio::spawn(async move { request(router, "PUT", &path, Some(replacement)).await });
    let replace_pid = blocked_by(&observer, gate_pid).await;
    let importing = tokio::spawn(import(app.importer.clone(), vec![original]));
    blocked_by(&observer, replace_pid).await;
    gate.rollback().await.unwrap();
    let (status, updated) = replacing.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert!(importing.await.unwrap().errors.is_empty());
    assert_eq!(updated["url"], new_url);
    let old_rows = app.search("CodeSystem", old_url).await;
    assert_eq!(old_rows.len(), 2);
    let imported = old_rows.iter().find(|row| row["version"] == "2").unwrap();
    assert_ne!(imported["id"], old_id);
    assert_eq!(
        app.request("GET", &format!("/CodeSystem/{old_id}"), None)
            .await
            .1,
        updated
    );
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_waiting_reimport_sees_the_rest_owner_that_just_committed() {
    let app = WriteApp::postgres().await;
    let url = "http://example.org/owner-race";
    let original = code_system("owner", url, "1", "package");
    assert!(
        app.import(std::slice::from_ref(&original))
            .await
            .errors
            .is_empty()
    );
    let pool = app.pool.as_ref().unwrap();
    let mut gate_client = pool.get().await.unwrap();
    let observer = pool.get().await.unwrap();
    install_gate(&gate_client, "code_systems", "DELETE").await;
    let gate = gate_client.transaction().await.unwrap();
    gate.execute("SELECT pg_advisory_xact_lock($1)", &[&GATE_KEY])
        .await
        .unwrap();
    let gate_pid: i32 = gate
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let updating = tokio::spawn(request(
        app.router.clone(),
        "PUT",
        "/CodeSystem/owner",
        Some(code_system("owner", url, "1", "rest")),
    ));
    let update_pid = blocked_by(&observer, gate_pid).await;
    let importing = tokio::spawn(import(app.importer.clone(), vec![original]));
    blocked_by(&observer, update_pid).await;
    gate.rollback().await.unwrap();
    let (status, updated) = updating.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{updated}");
    let stats = importing.await.unwrap();
    assert_eq!(stats.errors.len(), 1, "{stats:?}");
    assert_eq!(stats.code_systems, 0);
    assert_eq!(
        app.request("GET", "/CodeSystem/owner", None).await.1,
        updated
    );
    let rows = app.search("CodeSystem", url).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["concept"][0]["code"], "rest");
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_opposite_canonical_replacements_conflict_without_deadlocking() {
    let app = WriteApp::postgres().await;
    let first_url = "http://example.org/reversed-locks/a";
    let second_url = "http://example.org/reversed-locks/b";
    let first = code_system("first", first_url, "1", "first");
    let second = code_system("second", second_url, "1", "second");
    assert!(
        app.import(&[first.clone(), second.clone()])
            .await
            .errors
            .is_empty()
    );
    let (one, two) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            app.request(
                "PUT",
                "/CodeSystem/first",
                Some(code_system("first", second_url, "1", "swap-a"))
            ),
            app.request(
                "PUT",
                "/CodeSystem/second",
                Some(code_system("second", first_url, "1", "swap-b"))
            ),
        )
    })
    .await
    .expect("opposite lock sets are acquired in one consistent order");
    assert_eq!(one.0, StatusCode::CONFLICT, "{}", one.1);
    assert_eq!(two.0, StatusCode::CONFLICT, "{}", two.1);
    assert_eq!(app.request("GET", "/CodeSystem/first", None).await.1, first);
    assert_eq!(
        app.request("GET", "/CodeSystem/second", None).await.1,
        second
    );
}

#[cfg(feature = "postgres")]
async fn concurrent_put_creation_contract(recreate: bool) {
    let app = WriteApp::postgres().await;
    let url = "http://example.org/create-put-race";
    let path = "/CodeSystem/chosen";
    if recreate {
        let (status, body) = app
            .request(
                "PUT",
                path,
                Some(code_system("chosen", url, "1", "original")),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(
            app.request("DELETE", path, None).await.0,
            StatusCode::NO_CONTENT
        );
    }
    let pool = app.pool.as_ref().unwrap();
    let mut gate_client = pool.get().await.unwrap();
    let observer = pool.get().await.unwrap();
    install_gate(
        &gate_client,
        "resources",
        if recreate { "UPDATE" } else { "INSERT" },
    )
    .await;
    let gate = gate_client.transaction().await.unwrap();
    gate.execute("SELECT pg_advisory_xact_lock($1)", &[&GATE_KEY])
        .await
        .unwrap();
    let gate_pid: i32 = gate
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let first = tokio::spawn(request(
        app.router.clone(),
        "PUT",
        path,
        Some(code_system("chosen", url, "1", "first")),
    ));
    let first_pid = blocked_by(&observer, gate_pid).await;
    let second = tokio::spawn(request(
        app.router.clone(),
        "PUT",
        path,
        Some(code_system("chosen", url, "1", "second")),
    ));
    blocked_by(&observer, first_pid).await;
    gate.rollback().await.unwrap();
    let (first_status, first_body) = first.await.unwrap();
    let (second_status, second_body) = second.await.unwrap();
    assert_eq!(first_status, StatusCode::CREATED, "{first_body}");
    assert_eq!(second_status, StatusCode::OK, "{second_body}");
    let first_version = if recreate { "3" } else { "1" };
    let second_version = if recreate { "4" } else { "2" };
    assert_eq!(first_body["meta"]["versionId"], first_version);
    assert_eq!(second_body["meta"]["versionId"], second_version);
    assert_eq!(app.request("GET", path, None).await.1, second_body);
    let rows = app.search("CodeSystem", url).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "chosen");
    assert_eq!(rows[0]["concept"][0]["code"], "second");
    let tenant = TenantContext::system();
    let history = observer
        .query(
            "SELECT version_id, data->'concept'->0->>'code'
             FROM resource_history
             WHERE tenant_id = $1 AND resource_type = 'CodeSystem' AND id = 'chosen'
             ORDER BY version_id",
            &[&tenant.tenant_id().as_str()],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect::<Vec<_>>();
    let mut expected = if recreate {
        vec![
            ("1".to_owned(), "original".to_owned()),
            ("2".to_owned(), "original".to_owned()),
        ]
    } else {
        vec![]
    };
    expected.extend([
        (first_version.to_owned(), "first".to_owned()),
        (second_version.to_owned(), "second".to_owned()),
    ]);
    assert_eq!(
        history, expected,
        "waiting PUT must observe the committed creation/restoration"
    );
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_concurrent_put_creation_has_one_create_and_one_update() {
    concurrent_put_creation_contract(false).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_concurrent_put_recreation_preserves_deleted_history() {
    concurrent_put_creation_contract(true).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_concurrent_posts_for_one_canonical_leave_one_owner() {
    let app = WriteApp::postgres().await;
    let pool = app.pool.as_ref().unwrap();
    let mut gate_client = pool.get().await.unwrap();
    let observer = pool.get().await.unwrap();
    install_gate(&gate_client, "code_systems", "INSERT").await;
    let gate = gate_client.transaction().await.unwrap();
    gate.execute("SELECT pg_advisory_xact_lock($1)", &[&GATE_KEY])
        .await
        .unwrap();
    let gate_pid: i32 = gate
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let url = "http://example.org/create-post-race";
    let first = tokio::spawn(request(
        app.router.clone(),
        "POST",
        "/CodeSystem",
        Some(code_system("ignored", url, "1", "first")),
    ));
    let first_pid = blocked_by(&observer, gate_pid).await;
    let second = tokio::spawn(request(
        app.router.clone(),
        "POST",
        "/CodeSystem",
        Some(code_system("also-ignored", url, "1", "second")),
    ));
    blocked_by(&observer, first_pid).await;
    gate.rollback().await.unwrap();
    let (first_status, first_body) = first.await.unwrap();
    let (second_status, second_body) = second.await.unwrap();
    assert_eq!(first_status, StatusCode::CREATED, "{first_body}");
    assert_eq!(second_status, StatusCode::CONFLICT, "{second_body}");
    assert_eq!(second_body["resourceType"], "OperationOutcome");
    let rows = app.search("CodeSystem", url).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], first_body["id"]);
    assert_eq!(rows[0]["concept"][0]["code"], "first");
    for table in ["resources", "resource_history", "code_systems"] {
        let count: i64 = observer
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 1, "losing POST must not leave rows in {table}");
    }
}

/// Each advisory lock takes a slot in PostgreSQL's shared lock table (by
/// default 64 × `max_connections` entries). Locking every resource's identity
/// separately made a 5,000-resource bootstrap batch fail with "out of shared
/// memory", so an import must hold a bounded number of identity locks.
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_import_identity_locks_are_bounded_by_bundle_size() {
    let app = WriteApp::postgres().await;
    let pool = app.pool.as_ref().unwrap();
    let mut gate_client = pool.get().await.unwrap();
    let observer = pool.get().await.unwrap();
    install_gate(&gate_client, "code_systems", "INSERT").await;
    let gate = gate_client.transaction().await.unwrap();
    gate.execute("SELECT pg_advisory_xact_lock($1)", &[&GATE_KEY])
        .await
        .unwrap();
    let gate_pid: i32 = gate
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    let resources = (0..2_000)
        .map(|index| {
            code_system(
                &format!("bounded-{index}"),
                &format!("http://example.org/bounded-locks/{index}"),
                "1",
                "code",
            )
        })
        .collect::<Vec<_>>();
    let importing = tokio::spawn(import(app.importer.clone(), resources));
    // The import has taken its identity locks and waits at its first insert.
    let import_pid = blocked_by(&observer, gate_pid).await;
    let held: i64 = observer
        .query_one(
            "SELECT COUNT(*) FROM pg_locks WHERE pid = $1 AND locktype = 'advisory' AND granted",
            &[&import_pid],
        )
        .await
        .unwrap()
        .get(0);
    gate.rollback().await.unwrap();
    assert!(
        held <= 256,
        "a 2,000-resource import held {held} advisory locks"
    );
    let stats = importing.await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(stats.code_systems, 2_000);
}
