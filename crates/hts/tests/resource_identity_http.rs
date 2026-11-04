//! Public resource identity must agree across search, read, and instance operations.

mod common;

use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "postgres")]
use std::time::{Duration, Instant};

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
#[cfg(feature = "postgres")]
use helios_fhir::FhirVersion;
#[cfg(feature = "sqlite")]
use helios_hts::backends::SqliteTerminologyBackend;
#[cfg(feature = "postgres")]
use helios_hts::{
    backends::PostgresTerminologyBackend, import::BundleImportBackend, traits::TerminologyMetadata,
};
use helios_hts::{config::HtsConfig, state::AppState};
#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;
#[cfg(feature = "postgres")]
use helios_persistence::{
    backends::postgres::PostgresBackend, core::ResourceStorage, tenant::TenantContext,
};
use serde_json::{Value, json};
use tower::ServiceExt;

const ROOT: &str = "http://example.org/resource-identity";

type ResourceKey = (String, String, Option<String>);
type PublicIds = BTreeMap<ResourceKey, String>;

enum IdentityDatabase {
    #[cfg(feature = "sqlite")]
    Sqlite(r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>),
    #[cfg(feature = "postgres")]
    Postgres(deadpool_postgres::Pool),
}

struct IdentityApp {
    router: Router,
    database: IdentityDatabase,
    _directory: Option<tempfile::TempDir>,
}

impl IdentityApp {
    #[cfg(feature = "sqlite")]
    fn sqlite(with_resource_store: bool) -> Self {
        let (state, directory, pool) = if with_resource_store {
            let directory = tempfile::tempdir().expect("temporary SQLite directory");
            let path = directory.path().join("identity.db");
            let store = SqliteBackend::open(&path).expect("open resource store");
            store.init_schema().expect("initialize resource store");
            let backend = SqliteTerminologyBackend::new(path.to_str().unwrap())
                .expect("open terminology store");
            let pool = backend.pool().clone();
            (
                AppState::new(backend).with_resource_store(store),
                Some(directory),
                pool,
            )
        } else {
            let backend = SqliteTerminologyBackend::in_memory().expect("terminology store");
            let pool = backend.pool().clone();
            (AppState::new(backend), None, pool)
        };
        Self {
            router: helios_hts::server::create_app(&HtsConfig::default(), state),
            database: IdentityDatabase::Sqlite(pool),
            _directory: directory,
        }
    }

    #[cfg(feature = "postgres")]
    async fn postgres() -> Self {
        let shared = common::pg_http_url().await;
        let database = format!("hts_identity_{}", uuid::Uuid::new_v4().simple());
        let (admin, connection) = tokio_postgres::connect(shared, tokio_postgres::NoTls)
            .await
            .expect("admin connection");
        tokio::spawn(connection);
        admin
            .batch_execute(&format!("CREATE DATABASE {database}"))
            .await
            .expect("isolated identity database");
        let (base, _) = shared.rsplit_once('/').expect("database URL");
        let url = format!("{base}/{database}");
        let store = PostgresBackend::from_connection_string(&url)
            .await
            .expect("resource store");
        store.init_schema().await.expect("resource schema");
        let backend = PostgresTerminologyBackend::new(&url)
            .await
            .expect("terminology store");
        let pool = backend.pool().clone();
        let state = AppState::new(backend).with_resource_store_pg(store);
        Self {
            router: helios_hts::server::create_app(&HtsConfig::default(), state),
            database: IdentityDatabase::Postgres(pool),
            _directory: None,
        }
    }

    async fn request_with_headers(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        if_match: Option<&str>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/fhir+json");
        if let Some(etag) = if_match {
            builder = builder.header("if-match", etag);
        }
        let response = self
            .router
            .clone()
            .oneshot(
                builder
                    .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
                    .unwrap(),
            )
            .await
            .expect("HTTP response");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response bytes");
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("FHIR JSON response")
        };
        (status, headers, body)
    }

    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, _, body) = self.request_with_headers(method, path, body, None).await;
        (status, body)
    }

    async fn execute(&self, sqlite: &str, postgres: &str) {
        match &self.database {
            #[cfg(feature = "sqlite")]
            IdentityDatabase::Sqlite(pool) => pool.get().unwrap().execute_batch(sqlite).unwrap(),
            #[cfg(feature = "postgres")]
            IdentityDatabase::Postgres(pool) => pool
                .get()
                .await
                .unwrap()
                .batch_execute(postgres)
                .await
                .unwrap(),
        }
        let _ = (sqlite, postgres);
    }

    /// Snapshot actual database rows, including raw persistence/history and normalized content.
    async fn rows(&self, table: &str) -> Vec<Value> {
        let mut rows: Vec<Value> = match &self.database {
            #[cfg(feature = "sqlite")]
            IdentityDatabase::Sqlite(pool) => {
                let conn = pool.get().unwrap();
                let mut statement = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
                let names = statement
                    .column_names()
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect::<Vec<_>>();
                statement
                    .query_map([], |row| {
                        let mut value = serde_json::Map::new();
                        for (index, name) in names.iter().enumerate() {
                            let field = match row.get_ref(index)? {
                                rusqlite::types::ValueRef::Null => Value::Null,
                                rusqlite::types::ValueRef::Integer(number) => json!(number),
                                rusqlite::types::ValueRef::Real(number) => json!(number),
                                rusqlite::types::ValueRef::Text(bytes) => {
                                    json!(String::from_utf8_lossy(bytes))
                                }
                                rusqlite::types::ValueRef::Blob(bytes) => {
                                    serde_json::from_slice(bytes).unwrap_or_else(|_| json!(bytes))
                                }
                            };
                            value.insert(name.clone(), field);
                        }
                        Ok(Value::Object(value))
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            }
            #[cfg(feature = "postgres")]
            IdentityDatabase::Postgres(pool) => pool
                .get()
                .await
                .unwrap()
                .query(&format!("SELECT to_jsonb(t) FROM {table} t"), &[])
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.get(0))
                .collect(),
        };
        rows.sort_by_key(Value::to_string);
        rows
    }

    async fn snapshot(&self) -> BTreeMap<String, Vec<Value>> {
        let mut snapshot = BTreeMap::new();
        for table in [
            "resources",
            "resource_history",
            "code_systems",
            "value_sets",
            "concept_maps",
            "concepts",
            "concept_properties",
            "concept_designations",
            "concept_hierarchy",
            "concept_closure",
            "value_set_expansions",
        ] {
            snapshot.insert(table.to_owned(), self.rows(table).await);
        }
        snapshot
    }

    async fn expect(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        expected: StatusCode,
    ) -> Value {
        let (status, response) = self.request(method, path, body).await;
        assert_eq!(status, expected, "{method} {path}: {response}");
        response
    }

    async fn import(&self, resources: &[Value]) {
        let bundle = json!({
            "resourceType": "Bundle", "type": "collection",
            "entry": resources
                .iter()
                .map(|resource| json!({"resource": resource}))
                .collect::<Vec<_>>()
        });
        let (status, response) = self.request("POST", "/import", Some(bundle)).await;
        assert!(
            status == StatusCode::OK || status == StatusCode::MULTI_STATUS,
            "{response}"
        );
        if let Some(errors) = response.get("errors") {
            assert!(
                errors.as_array().expect("import error array").is_empty(),
                "{response}"
            );
        }
    }

    async fn search(&self, resource_type: &str, suffix: &str) -> Vec<Value> {
        let path = format!("/{resource_type}?_count=100{suffix}");
        let body = self.expect("GET", &path, None, StatusCode::OK).await;
        body["entry"]
            .as_array()
            .expect("search entries")
            .iter()
            .map(|entry| entry["resource"].clone())
            .collect()
    }
}

fn key(resource: &Value) -> ResourceKey {
    (
        resource["resourceType"]
            .as_str()
            .expect("resource type")
            .to_owned(),
        resource["url"].as_str().expect("canonical URL").to_owned(),
        resource
            .get("version")
            .map(|version| version.as_str().expect("business version").to_owned()),
    )
}

fn public_id(resource: &Value) -> &str {
    let id = resource["id"].as_str().expect("public resource ID");
    assert!(!id.is_empty() && id.len() <= 64, "invalid ID length: {id}");
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.')),
        "invalid FHIR ID: {id}"
    );
    id
}

fn terminology(
    resource_type: &str,
    id: &str,
    url: &str,
    version: Option<&str>,
    code: &str,
) -> Value {
    let mut resource = json!({
        "resourceType": resource_type, "id": id, "url": url,
        "name": "IdentityFixture", "title": "Identity fixture", "status": "active"
    });
    if let Some(version) = version {
        resource["version"] = json!(version);
    }
    match resource_type {
        "CodeSystem" => {
            resource["content"] = json!("complete");
            resource["concept"] = json!([{"code": code, "display": code}]);
        }
        "ValueSet" => {
            resource["compose"] = json!({"include": [{
                "system": format!("{ROOT}/support"), "concept": [{"code": code}]
            }]});
        }
        "ConceptMap" => resource["group"] = json!([]),
        _ => panic!("unsupported test resource {resource_type}"),
    }
    resource
}

fn collision_fixture() -> Vec<Value> {
    let mut support = terminology(
        "CodeSystem",
        "support",
        &format!("{ROOT}/support"),
        None,
        "one",
    );
    support["concept"] = json!([
        {"code": "one"}, {"code": "two"}, {"code": "unversioned"}, {"code": "other"}
    ]);
    let mut resources = vec![support];
    for resource_type in ["CodeSystem", "ValueSet"] {
        for (version, code) in [
            (Some("1.0"), "one"),
            (Some("2.0"), "two"),
            (None, "unversioned"),
        ] {
            resources.push(terminology(
                resource_type,
                "reused-id",
                &format!("{ROOT}/{resource_type}/family"),
                version,
                code,
            ));
        }
        resources.push(terminology(
            resource_type,
            "reused-id",
            &format!("{ROOT}/{resource_type}/other"),
            Some("1.0"),
            "other",
        ));
    }
    resources
}

async fn assert_search_and_read(app: &IdentityApp, expected: &[Value]) -> PublicIds {
    let expected_by_key: BTreeMap<_, _> = expected
        .iter()
        .map(|resource| (key(resource), resource))
        .collect();
    let mut ids = PublicIds::new();
    for resource_type in ["CodeSystem", "ValueSet"] {
        let expected_keys: BTreeSet<_> = expected_by_key
            .keys()
            .filter(|key| key.0 == resource_type)
            .cloned()
            .collect();
        let mut first_search = None;
        for suffix in ["", "&name=Identity", "&title:contains=fixture"] {
            let results = app.search(resource_type, suffix).await;
            let mut found = BTreeMap::new();
            let mut unique_ids = BTreeSet::new();
            for resource in results {
                let resource_key = key(&resource);
                let expected_resource = expected_by_key
                    .get(&resource_key)
                    .expect("unexpected search result");
                let id = public_id(&resource).to_owned();
                assert!(
                    unique_ids.insert(id.clone()),
                    "duplicate {resource_type} public ID: {id}"
                );
                assert!(
                    found.insert(resource_key.clone(), id.clone()).is_none(),
                    "duplicate canonical/version"
                );
                let read = app
                    .expect(
                        "GET",
                        &format!("/{resource_type}/{id}"),
                        None,
                        StatusCode::OK,
                    )
                    .await;
                assert_eq!(read["id"], id);
                assert_eq!(key(&read), resource_key);
                for field in ["concept", "compose", "group"] {
                    assert_eq!(
                        read.get(field),
                        expected_resource.get(field),
                        "GET {resource_type}/{id} field {field}"
                    );
                }
            }
            assert_eq!(
                found.keys().cloned().collect::<BTreeSet<_>>(),
                expected_keys
            );
            if let Some(first) = &first_search {
                assert_eq!(&found, first, "search helper changed public identity");
            } else {
                ids.extend(found.clone());
                first_search = Some(found);
            }
        }
    }
    ids
}

async fn assert_exact_instance_expansions(app: &IdentityApp, ids: &PublicIds) {
    for (url_suffix, version, expected_code) in [
        ("family", Some("1.0"), "one"),
        ("family", Some("2.0"), "two"),
        ("family", None, "unversioned"),
        ("other", Some("1.0"), "other"),
    ] {
        let id = &ids[&(
            "ValueSet".to_owned(),
            format!("{ROOT}/ValueSet/{url_suffix}"),
            version.map(str::to_owned),
        )];
        for method in ["GET", "POST"] {
            let body =
                (method == "POST").then(|| json!({"resourceType": "Parameters", "parameter": []}));
            let expansion = app
                .expect(
                    method,
                    &format!("/ValueSet/{id}/$expand"),
                    body,
                    StatusCode::OK,
                )
                .await;
            assert_eq!(expansion["id"], *id);
            assert_eq!(expansion["url"], format!("{ROOT}/ValueSet/{url_suffix}"));
            assert_eq!(
                expansion.get("version").and_then(Value::as_str),
                version,
                "instance expansion must retain the selected business version"
            );
            let contains = expansion["expansion"]["contains"]
                .as_array()
                .expect("expanded concepts");
            assert_eq!(contains.len(), 1, "{method} {id}: {expansion}");
            assert_eq!(contains[0]["system"], format!("{ROOT}/support"));
            assert_eq!(
                contains[0]["code"], expected_code,
                "instance operation selected a sibling"
            );
        }
    }
}

async fn imported_identity_contract(app: IdentityApp) {
    let mut resources = collision_fixture();
    app.import(&resources).await;
    let before = assert_search_and_read(&app, &resources).await;
    assert_exact_instance_expansions(&app, &before).await;
    resources.reverse();
    app.import(&resources).await;
    let after = assert_search_and_read(&app, &resources).await;
    assert_eq!(before, after, "reimport must not reassign public IDs");
    assert_exact_instance_expansions(&app, &after).await;
}

async fn crud_identity_contract(app: IdentityApp) {
    for resource_type in ["CodeSystem", "ValueSet", "ConceptMap"] {
        let mut resource = terminology(
            resource_type,
            "unused",
            &format!("{ROOT}/{resource_type}/created"),
            Some("1.0"),
            "one",
        );
        resource.as_object_mut().unwrap().remove("id");
        let created = app
            .expect(
                "POST",
                &format!("/{resource_type}"),
                Some(resource),
                StatusCode::CREATED,
            )
            .await;
        let id = public_id(&created).to_owned();
        let original_path = format!("/{resource_type}/{id}");
        for suffix in ["", "&name=Identity", "&title:contains=fixture"] {
            let results = app.search(resource_type, suffix).await;
            assert_eq!(results.len(), 1);
            assert_eq!(results[0]["id"], id);
            assert_eq!(key(&results[0]), key(&created));
        }
        let read = app
            .expect("GET", &original_path, None, StatusCode::OK)
            .await;
        assert_eq!(
            read, created,
            "search public ID must address the created resource"
        );
        if resource_type == "ConceptMap" {
            app.expect("DELETE", &original_path, None, StatusCode::NO_CONTENT)
                .await;
            app.expect("GET", &original_path, None, StatusCode::NOT_FOUND)
                .await;
            continue;
        }

        let imported = terminology(
            resource_type,
            &id,
            &format!("{ROOT}/{resource_type}/imported"),
            Some("1.0"),
            "two",
        );
        app.import(std::slice::from_ref(&imported)).await;
        let results = app.search(resource_type, "").await;
        let collision = results
            .iter()
            .find(|result| key(result) == key(&imported))
            .expect("colliding import preserved");
        assert_ne!(
            public_id(collision),
            id,
            "import must not alias a live CRUD resource"
        );
        assert_eq!(
            app.expect("GET", &original_path, None, StatusCode::OK)
                .await,
            created
        );

        app.expect("DELETE", &original_path, None, StatusCode::NO_CONTENT)
            .await;
        let replacement = terminology(
            resource_type,
            &id,
            &format!("{ROOT}/{resource_type}/after-delete"),
            Some("1.0"),
            "other",
        );
        app.import(std::slice::from_ref(&replacement)).await;
        let results = app.search(resource_type, "").await;
        let collision = results
            .iter()
            .find(|result| key(result) == key(&replacement))
            .expect("import after deletion preserved");
        let imported_id = public_id(collision);
        assert_ne!(
            imported_id, id,
            "import must not reuse a tombstoned CRUD ID"
        );
        let read = app
            .expect(
                "GET",
                &format!("/{resource_type}/{imported_id}"),
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(key(&read), key(&replacement));
        app.expect("GET", &original_path, None, StatusCode::NOT_FOUND)
            .await;

        let competing = terminology(
            resource_type,
            imported_id,
            &format!("{ROOT}/{resource_type}/post-collision"),
            Some("1.0"),
            "one",
        );
        let (status, response) = app
            .request(
                "POST",
                &format!("/{resource_type}"),
                Some(competing.clone()),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "POST must ignore a supplied imported ID: {response}"
        );
        assert_ne!(public_id(&response), imported_id);
        let imported_read = app
            .expect(
                "GET",
                &format!("/{resource_type}/{imported_id}"),
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(
            key(&imported_read),
            key(&replacement),
            "POST changed the resource behind an existing imported ID"
        );
    }
}

async fn changed_business_version_contract(app: IdentityApp) {
    let support = collision_fixture().remove(0);
    app.import(&[support]).await;
    for resource_type in ["CodeSystem", "ValueSet"] {
        let url = format!("{ROOT}/{resource_type}/updated-version");
        let mut original = terminology(resource_type, "unused", &url, Some("1.0"), "one");
        original.as_object_mut().unwrap().remove("id");
        let created = app
            .expect(
                "POST",
                &format!("/{resource_type}"),
                Some(original),
                StatusCode::CREATED,
            )
            .await;
        let id = public_id(&created).to_owned();
        let path = format!("/{resource_type}/{id}");
        let sibling = terminology(resource_type, &id, &url, Some("2.0"), "two");
        app.import(&[sibling]).await;
        let filter = format!("&url={url}");
        let before = app.search(resource_type, &filter).await;
        assert_eq!(before.len(), 2);
        let sibling_id = public_id(
            before
                .iter()
                .find(|resource| resource["version"] == "2.0")
                .expect("imported sibling"),
        )
        .to_owned();
        assert_ne!(sibling_id, id);

        let updated = terminology(resource_type, &id, &url, Some("3.0"), "other");
        let response = app
            .expect("PUT", &path, Some(updated), StatusCode::OK)
            .await;
        assert_eq!(response["id"], id);
        assert_eq!(response["version"], "3.0");
        let after = app.search(resource_type, &filter).await;
        assert_eq!(
            after.len(),
            2,
            "old business version must not retain the updated resource ID"
        );
        let version_ids: BTreeMap<_, _> = after
            .iter()
            .map(|resource| {
                (
                    resource["version"].as_str().unwrap().to_owned(),
                    public_id(resource).to_owned(),
                )
            })
            .collect();
        assert_eq!(
            version_ids,
            BTreeMap::from([
                ("2.0".to_owned(), sibling_id.clone()),
                ("3.0".to_owned(), id.clone())
            ])
        );
        let read = app.expect("GET", &path, None, StatusCode::OK).await;
        assert_eq!(read, response);
        let sibling_read = app
            .expect(
                "GET",
                &format!("/{resource_type}/{sibling_id}"),
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(sibling_read["version"], "2.0");
        if resource_type == "ValueSet" {
            for (selected_id, expected_code) in [(&id, "other"), (&sibling_id, "two")] {
                let expanded = app
                    .expect(
                        "GET",
                        &format!("/ValueSet/{selected_id}/$expand"),
                        None,
                        StatusCode::OK,
                    )
                    .await;
                assert_eq!(
                    expanded["expansion"]["contains"].as_array().unwrap().len(),
                    1
                );
                assert_eq!(expanded["expansion"]["contains"][0]["code"], expected_code);
            }
        }
        app.expect("DELETE", &path, None, StatusCode::NO_CONTENT)
            .await;
        app.expect("GET", &path, None, StatusCode::NOT_FOUND).await;
        let survivors = app.search(resource_type, &filter).await;
        assert_eq!(
            survivors.len(),
            1,
            "deleting one public ID must preserve the imported sibling"
        );
        assert_eq!(public_id(&survivors[0]), sibling_id);
        let sibling_read = app
            .expect(
                "GET",
                &format!("/{resource_type}/{sibling_id}"),
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(sibling_read["version"], "2.0");
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_imported_resources_have_unique_readable_ids_and_exact_expansions() {
    imported_identity_contract(IdentityApp::sqlite(false)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_imported_resources_have_unique_readable_ids_and_exact_expansions() {
    imported_identity_contract(IdentityApp::postgres().await).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_imports_preserve_live_and_deleted_crud_identities() {
    crud_identity_contract(IdentityApp::sqlite(true)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_imports_preserve_live_and_deleted_crud_identities() {
    crud_identity_contract(IdentityApp::postgres().await).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_business_version_updates_preserve_identity_and_imported_siblings() {
    changed_business_version_contract(IdentityApp::sqlite(true)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_business_version_updates_preserve_identity_and_imported_siblings() {
    changed_business_version_contract(IdentityApp::postgres().await).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_concurrent_imports_preserve_unique_and_stable_public_ids() {
    let app = IdentityApp::postgres().await;
    let left = [terminology(
        "ValueSet",
        "concurrent-source-id",
        &format!("{ROOT}/concurrent/left"),
        Some("1.0"),
        "one",
    )];
    let right = [terminology(
        "ValueSet",
        "concurrent-source-id",
        &format!("{ROOT}/concurrent/right"),
        Some("1.0"),
        "two",
    )];
    tokio::join!(app.import(&left), app.import(&right));

    let results = app.search("ValueSet", "").await;
    assert_eq!(
        results.len(),
        2,
        "concurrent imports must preserve both canonicals"
    );
    let ids: BTreeSet<_> = results
        .iter()
        .map(|resource| public_id(resource).to_owned())
        .collect();
    assert_eq!(
        ids.len(),
        2,
        "concurrent imports must not reuse a public ID"
    );
    for expected in [&left[0], &right[0]] {
        let result = results
            .iter()
            .find(|resource| key(resource) == key(expected))
            .expect("imported canonical");
        let read = app
            .expect(
                "GET",
                &format!("/ValueSet/{}", public_id(result)),
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(key(&read), key(expected));
        assert_eq!(read["compose"], expected["compose"]);
    }

    let shared_url = format!("{ROOT}/concurrent/shared");
    let first = [terminology(
        "ValueSet",
        "first-source-id",
        &shared_url,
        Some("1.0"),
        "one",
    )];
    let second = [terminology(
        "ValueSet",
        "second-source-id",
        &shared_url,
        Some("1.0"),
        "two",
    )];
    tokio::join!(app.import(&first), app.import(&second));
    let filter = format!("&url={shared_url}");
    let results = app.search("ValueSet", &filter).await;
    assert_eq!(
        results.len(),
        1,
        "one canonical/version must have one public identity"
    );
    let id = public_id(&results[0]).to_owned();
    let read = app
        .expect("GET", &format!("/ValueSet/{id}"), None, StatusCode::OK)
        .await;
    assert_eq!(key(&read), key(&first[0]));
    assert!(read["compose"] == first[0]["compose"] || read["compose"] == second[0]["compose"]);

    app.import(&first).await;
    app.import(&second).await;
    let results = app.search("ValueSet", &filter).await;
    assert_eq!(results.len(), 1);
    assert_eq!(
        public_id(&results[0]),
        id,
        "reimport must retain the concurrently allocated identity"
    );
    let read = app
        .expect("GET", &format!("/ValueSet/{id}"), None, StatusCode::OK)
        .await;
    assert_eq!(read["compose"], second[0]["compose"]);
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_legacy_identity_migration_is_stable_and_lookups_use_indexes() {
    let shared_url = common::pg_http_url().await;
    let (admin, connection) = tokio_postgres::connect(shared_url, tokio_postgres::NoTls)
        .await
        .expect("admin connection");
    tokio::spawn(connection);
    let database = format!("hts_identity_{}", uuid::Uuid::new_v4().simple());
    admin
        .batch_execute(&format!("CREATE DATABASE {database}"))
        .await
        .expect("isolated legacy database");
    let (base, _) = shared_url.rsplit_once('/').expect("database path");
    let url = format!("{base}/{database}");
    let (legacy, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("legacy database connection");
    tokio::spawn(connection);
    helios_hts::backends::postgres::schema::apply(&legacy)
        .await
        .expect("legacy normalized schema");
    let store = PostgresBackend::from_connection_string(&url)
        .await
        .expect("resource store");
    store.init_schema().await.expect("resource store schema");
    let context = TenantContext::system();
    for (id, suffix) in [
        ("owned", "live"),
        ("deleted", "dead"),
        ("drift-owner", "drifted-live"),
        ("swap-a", "swap-a"),
        ("swap-b", "swap-b"),
    ] {
        let resource = terminology(
            "ValueSet",
            id,
            &format!("{ROOT}/migration/{suffix}"),
            Some("1.0"),
            "one",
        );
        store
            .create(
                &context,
                "ValueSet",
                resource,
                FhirVersion::default_enabled(),
            )
            .await
            .expect("legacy CRUD resource");
    }
    store
        .delete(&context, "ValueSet", "deleted")
        .await
        .expect("legacy tombstone");

    let invalid_source_id = "x".repeat(65);
    for (storage_id, source_id, suffix, version) in [
        ("owned|1.0", "owned", "live", Some("1.0")),
        ("owned", "owned", "collision", None),
        ("duplicate|1.0", "duplicate", "versions", Some("1.0")),
        ("duplicate|2.0", "duplicate", "versions", Some("2.0")),
        ("duplicate", "duplicate", "published-later", None),
        ("deleted|1.0", "deleted", "dead", Some("1.0")),
        ("old-public", "new-source", "renamed-import", None),
        ("numeric-legacy", "unused", "numeric-id", None),
        ("swap-a", "old-source-a", "swap-b", Some("1.0")),
        ("swap-b", "old-source-b", "swap-a", Some("1.0")),
        (
            "drift-owner|1.0",
            "renamed-source",
            "drifted-live",
            Some("1.0"),
        ),
        (
            "invalid|1.0",
            invalid_source_id.as_str(),
            "invalid",
            Some("1.0"),
        ),
    ] {
        let canonical = format!("{ROOT}/migration/{suffix}");
        let mut resource = terminology("ValueSet", source_id, &canonical, version, "one");
        if storage_id == "numeric-legacy" {
            resource["id"] = json!(123);
        }
        let compose = resource["compose"].to_string();
        legacy
            .execute(
                "INSERT INTO value_sets(id, url, version, name, title, status, compose_json, \
                 resource_json, created_at, updated_at)
             VALUES ($1, $2, $3, 'IdentityFixture', 'Identity fixture', 'active', $4, $5, \
             '2026-01-01', '2026-01-01')",
                &[&storage_id, &canonical, &version, &compose, &resource],
            )
            .await
            .expect("legacy ValueSet row");
    }
    legacy
        .batch_execute(
            "UPDATE value_sets SET created_at = '2026-02-01' WHERE id = 'duplicate';
         INSERT INTO value_sets(id, url, status, compose_json, created_at, updated_at)
         VALUES ('synthetic', 'http://example.org/resource-identity/migration/synthetic', 'active',
                 '{\"include\":[{\"system\":\"http://example.org/support\"}]}', '2026-01-01', \
                 '2026-01-01');
         INSERT INTO value_sets(id, url, status, created_at, updated_at)
         SELECT 'bulk-' || n, 'http://example.org/resource-identity/migration/bulk/' || n,
                'active', '2026-01-01', '2026-01-01' FROM generate_series(1, 5000) AS n;",
        )
        .await
        .expect("synthetic legacy resources");
    let code_system = terminology(
        "CodeSystem",
        "legacy-cs",
        &format!("{ROOT}/migration/cs"),
        Some("1.0"),
        "one",
    );
    legacy.execute(
        "INSERT INTO code_systems(id, url, version, status, resource_json, created_at, updated_at)
         VALUES ('legacy-cs|1.0', $1, '1.0', 'active', $2, '2026-01-01', '2026-01-01')",
        &[&code_system["url"].as_str().unwrap(), &code_system],
    ).await.expect("legacy CodeSystem");

    let backend = PostgresTerminologyBackend::new(&url)
        .await
        .expect("migrate public identities");
    let client = backend.pool().get().await.expect("migrated connection");
    let read_ids = |rows: Vec<tokio_postgres::Row>| -> BTreeMap<String, String> {
        rows.into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect()
    };
    let before = read_ids(
        client
            .query(
                "SELECT id, COALESCE(resource_json->>'id', id) FROM value_sets ORDER BY id",
                &[],
            )
            .await
            .unwrap(),
    );
    assert_eq!(before.len(), 5013);
    assert_eq!(
        before["owned|1.0"], "owned",
        "existing CRUD resource owns its public ID"
    );
    assert_ne!(
        before["owned"], "owned",
        "legacy storage key must yield to CRUD ownership"
    );
    assert_ne!(
        before["deleted|1.0"], "deleted",
        "a tombstone reserves its ID"
    );
    assert_ne!(before["duplicate|1.0"], before["duplicate|2.0"]);
    assert_eq!(
        before["duplicate"], "duplicate",
        "an older invalid storage ID must not claim a later valid published ID"
    );
    assert_ne!(before["duplicate|1.0"], "duplicate");
    assert_ne!(before["duplicate|2.0"], "duplicate");
    assert_eq!(before["synthetic"], "synthetic");
    assert_eq!(before["numeric-legacy"], "numeric-legacy");
    let repaired_id: Value = client
        .query_one(
            "SELECT resource_json->'id' FROM value_sets WHERE id = 'numeric-legacy'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(repaired_id, json!("numeric-legacy"));
    assert_eq!(
        before["old-public"], "old-public",
        "a valid public ID exposed before migration must remain stable"
    );
    assert_eq!(
        before["drift-owner|1.0"], "drift-owner",
        "canonical reimport changing its source ID must not displace the CRUD owner"
    );
    assert_eq!(
        before["swap-a"], "swap-b",
        "a temporary legacy storage key must not block another row's CRUD owner"
    );
    assert_eq!(before["swap-b"], "swap-a");
    for id in ["swap-a", "swap-b"] {
        let resource = backend
            .resource_by_id("ValueSet", id)
            .unwrap()
            .expect("migrated CRUD owner");
        assert_eq!(resource["url"], format!("{ROOT}/migration/{id}"));
    }
    assert_eq!(before.values().collect::<BTreeSet<_>>().len(), before.len());
    for id in before.values() {
        public_id(&json!({"id": id}));
    }
    let synthetic = backend
        .resource_by_id("ValueSet", "synthetic")
        .unwrap()
        .expect("synthetic resource");
    assert_eq!(synthetic["id"], "synthetic");
    assert_eq!(
        synthetic["compose"]["include"][0]["system"],
        "http://example.org/support"
    );
    let migrated_cs = backend
        .resource_by_id("CodeSystem", "legacy-cs")
        .unwrap()
        .expect("migrated CodeSystem");
    assert_eq!(migrated_cs["version"], "1.0");
    assert_eq!(migrated_cs["concept"], code_system["concept"]);

    PostgresTerminologyBackend::new(&url)
        .await
        .expect("repeat startup migration");
    let after = read_ids(
        client
            .query(
                "SELECT id, COALESCE(resource_json->>'id', id) FROM value_sets ORDER BY id",
                &[],
            )
            .await
            .unwrap(),
    );
    assert_eq!(before, after, "restart must not change existing public IDs");
    client.batch_execute("ANALYZE value_sets").await.unwrap();
    let storage_query = "SELECT url FROM value_sets WHERE id = $1";
    let public_query = "SELECT url FROM value_sets WHERE COALESCE(resource_json->>'id', id) = $1";
    let sample = "bulk-2500";
    for (query, expected_index) in [
        (storage_query, "value_sets_pkey"),
        (public_query, "idx_value_sets_public_id"),
    ] {
        let explain = format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {query}");
        let plan: Value = client.query_one(&explain, &[&sample]).await.unwrap().get(0);
        let rendered = plan.to_string();
        assert!(rendered.contains(expected_index), "{plan}");
        assert!(
            !rendered.contains("Seq Scan"),
            "public-ID resolution must seek: {plan}"
        );
        assert_eq!(plan[0]["Plan"]["Actual Rows"], 1);
        println!("identity lookup {expected_index}: {plan}");
    }

    let mut storage_time = Duration::ZERO;
    let mut public_time = Duration::ZERO;
    for _ in 0..50 {
        let started = Instant::now();
        let old: String = client
            .query_one(storage_query, &[&sample])
            .await
            .unwrap()
            .get(0);
        storage_time += started.elapsed();
        let started = Instant::now();
        let new: String = client
            .query_one(public_query, &[&sample])
            .await
            .unwrap()
            .get(0);
        public_time += started.elapsed();
        assert_eq!(old, new);
    }
    println!(
        "warm identity lookup, 50 interleaved reads each: storage-key mean {:.3} ms, public-ID \
         mean {:.3} ms",
        storage_time.as_secs_f64() * 1000.0 / 50.0,
        public_time.as_secs_f64() * 1000.0 / 50.0
    );

    let canonical = format!("{ROOT}/migration/new-import");
    let mut resource = terminology("ValueSet", "duplicate", &canonical, Some("1.0"), "one");
    let mut assigned_id = None;
    for source_id in ["duplicate", "new-source-on-reimport"] {
        resource["id"] = json!(source_id);
        let bundle = json!({
            "resourceType": "Bundle", "type": "collection",
            "entry": [{"resource": resource}]
        });
        let stats = backend
            .import_bundle(&context, &serde_json::to_vec(&bundle).unwrap())
            .await
            .expect("import after legacy migration");
        assert!(stats.errors.is_empty(), "{:?}", stats.errors);
        let stored: Value = client
            .query_one(
                "SELECT resource_json FROM value_sets WHERE url = $1",
                &[&canonical],
            )
            .await
            .unwrap()
            .get(0);
        let id = public_id(&stored).to_owned();
        assert_ne!(id, "duplicate", "an existing public ID remains reserved");
        if let Some(previous_id) = &assigned_id {
            assert_eq!(&id, previous_id, "reimport must retain the stored JSON ID");
        }
        let read = backend.resource_by_id("ValueSet", &id).unwrap().unwrap();
        assert_eq!(read, stored, "stored JSON and public read must agree");
        assigned_id = Some(id);
    }
    let extra_columns: i64 = client
        .query_one(
            "SELECT count(*) FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name IN ('code_systems', 'value_sets')
               AND column_name = 'logical_id'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(extra_columns, 0, "identity must use existing resource JSON");
}

fn rich_code_system(id: &str, url: &str, version: &str) -> Value {
    let mut resource = terminology("CodeSystem", id, url, Some(version), "parent");
    resource["property"] = json!([{"code":"kind","type":"string"}]);
    resource["concept"] = json!([{
        "code":"parent", "display":"Parent",
        "designation":[{"language":"en","value":"Parent synonym"}],
        "property":[{"code":"kind","valueString":"root"}],
        "concept":[{"code":"child","display":"Child",
            "designation":[{"language":"en","value":"Child synonym"}],
            "property":[{"code":"kind","valueString":"leaf"}]}]
    }]);
    resource
}

async fn post_identity_contract(app: IdentityApp) {
    for resource_type in ["CodeSystem", "ValueSet", "ConceptMap"] {
        let supplied_ids = [
            json!(""),
            json!("x|1"),
            json!("x".repeat(65)),
            json!(123),
            Value::Null,
            json!([]),
            json!({"id":"nested"}),
            json!("chosen-id"),
        ];
        for (index, id) in supplied_ids.into_iter().enumerate() {
            let url = format!("{ROOT}/post/{resource_type}/{index}");
            let mut resource = terminology(resource_type, "placeholder", &url, Some("1"), "one");
            resource["id"] = id.clone();
            resource["meta"] = json!({
                "versionId": "9999", "lastUpdated": "2000-01-01T00:00:00Z",
                "tag": [{"system": "http://example.org/tags", "code": "retained"}],
                "profile": ["http://example.org/profiles/terminology"]
            });
            let supplied_meta = resource["meta"].clone();
            let (status, headers, created) = app
                .request_with_headers("POST", &format!("/{resource_type}"), Some(resource), None)
                .await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            assert_ne!(
                created["id"], id,
                "POST must discard supplied identity before parsing"
            );
            assert_write_identity(&app, resource_type, &created, &headers, status, "1").await;
            assert_ne!(created["meta"]["lastUpdated"], supplied_meta["lastUpdated"]);
            assert_eq!(created["meta"]["tag"], supplied_meta["tag"]);
            assert_eq!(created["meta"]["profile"], supplied_meta["profile"]);
            if index == 0 {
                let mut replacement = created.clone();
                replacement["name"] = json!("UpdatedAfterPost");
                let (status, updated_headers, updated) = app
                    .request_with_headers(
                        "PUT",
                        &format!("/{resource_type}/{}", public_id(&created)),
                        Some(replacement),
                        Some("W/\"1\""),
                    )
                    .await;
                assert_eq!(status, StatusCode::OK, "{updated}");
                assert_write_identity(&app, resource_type, &updated, &updated_headers, status, "2")
                    .await;
                assert_resource_read(
                    &app,
                    headers["location"].to_str().unwrap(),
                    &created,
                    &headers,
                )
                .await;
            }
        }

        let original = terminology(
            resource_type,
            "occupied",
            &format!("{ROOT}/occupied/{resource_type}"),
            Some("1"),
            "one",
        );
        let path = format!("/{resource_type}/occupied");
        let saved = app
            .expect("PUT", &path, Some(original.clone()), StatusCode::CREATED)
            .await;
        let imported = terminology(
            resource_type,
            "imported-post",
            &format!("{ROOT}/imported-post/{resource_type}"),
            Some("1"),
            "one",
        );
        app.import(std::slice::from_ref(&imported)).await;
        for supplied_id in ["occupied", "imported-post"] {
            let unrelated = terminology(
                resource_type,
                supplied_id,
                &format!("{ROOT}/ignored/{resource_type}/{supplied_id}"),
                None,
                "other",
            );
            let created = app
                .expect(
                    "POST",
                    &format!("/{resource_type}"),
                    Some(unrelated),
                    StatusCode::CREATED,
                )
                .await;
            assert_ne!(public_id(&created), supplied_id);
        }
        assert_eq!(app.expect("GET", &path, None, StatusCode::OK).await, saved);
        let imported_read = app
            .expect(
                "GET",
                &format!("/{resource_type}/imported-post"),
                None,
                StatusCode::OK,
            )
            .await;
        assert_eq!(key(&imported_read), key(&imported));

        let before = app.snapshot().await;
        let collision = app
            .expect(
                "POST",
                &format!("/{resource_type}"),
                Some(original.clone()),
                StatusCode::CONFLICT,
            )
            .await;
        assert_eq!(collision["resourceType"], "OperationOutcome");
        assert_eq!(collision["issue"][0]["code"], "conflict");
        assert_eq!(
            app.snapshot().await,
            before,
            "canonical conflict must roll back every write"
        );
        app.expect("DELETE", &path, None, StatusCode::NO_CONTENT)
            .await;
        let old_history: Vec<_> = app
            .rows("resource_history")
            .await
            .into_iter()
            .filter(|row| row["resource_type"] == resource_type && row["id"] == "occupied")
            .collect();
        let replacement = app
            .expect(
                "POST",
                &format!("/{resource_type}"),
                Some(original),
                StatusCode::CREATED,
            )
            .await;
        assert_ne!(public_id(&replacement), "occupied");
        app.expect("GET", &path, None, StatusCode::NOT_FOUND).await;
        let after_history: Vec<_> = app
            .rows("resource_history")
            .await
            .into_iter()
            .filter(|row| row["resource_type"] == resource_type && row["id"] == "occupied")
            .collect();
        assert_eq!(
            after_history, old_history,
            "POST must not resurrect or alter the old identity"
        );
    }
}

async fn assert_resource_read(
    app: &IdentityApp,
    path: &str,
    resource: &Value,
    write_headers: &HeaderMap,
) {
    let (status, headers, read) = app.request_with_headers("GET", path, None, None).await;
    assert_eq!(status, StatusCode::OK, "GET {path}: {read}");
    assert_eq!(
        &read, resource,
        "GET {path} must return the selected revision"
    );
    assert_eq!(headers["etag"], write_headers["etag"]);
    assert_eq!(headers["last-modified"], write_headers["last-modified"]);
    assert!(!headers.contains_key("location"));
}

async fn assert_write_identity(
    app: &IdentityApp,
    resource_type: &str,
    resource: &Value,
    headers: &HeaderMap,
    status: StatusCode,
    version: &str,
) {
    let id = public_id(resource);
    let path = format!("/{resource_type}/{id}");
    assert_eq!(resource["meta"]["versionId"], version);
    let modified =
        chrono::DateTime::parse_from_rfc2822(headers["last-modified"].to_str().unwrap()).unwrap();
    let updated =
        chrono::DateTime::parse_from_rfc3339(resource["meta"]["lastUpdated"].as_str().unwrap())
            .unwrap();
    assert_eq!(modified.timestamp(), updated.timestamp());
    assert_eq!(
        headers["etag"].to_str().unwrap(),
        format!("W/\"{version}\"")
    );
    if status == StatusCode::CREATED {
        let location = headers["location"].to_str().unwrap();
        assert_eq!(location, format!("{path}/_history/{version}"));
        // Follow the actual response header: a correctly spelled but unserved
        // version URL must fail this contract.
        assert_resource_read(app, location, resource, headers).await;
    } else {
        assert_eq!(status, StatusCode::OK);
        assert!(!headers.contains_key("location"));
    }
    assert_resource_read(app, &path, resource, headers).await;
    let rows = app
        .search(
            resource_type,
            &format!("&url={}", resource["url"].as_str().unwrap()),
        )
        .await;
    let searched = rows
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("search must publish the returned ID: {rows:?}"));
    assert_eq!(key(searched), key(resource));
    assert_eq!(
        searched["meta"], resource["meta"],
        "search must publish server-managed metadata"
    );
}

async fn put_creation_contract(app: IdentityApp) {
    for kind in ["CodeSystem", "ValueSet", "ConceptMap"] {
        let path = format!("/{kind}/chosen");
        let mut original = terminology(
            kind,
            "chosen",
            &format!("{ROOT}/put/{kind}"),
            Some("1"),
            "one",
        );
        original["meta"] = json!({
            "versionId": "9999", "lastUpdated": "2000-01-01T00:00:00Z",
            "tag": [{"system": "http://example.org/tags", "code": "retained"}],
            "profile": ["http://example.org/profiles/terminology"]
        });
        for invalid_id in [None, Some(json!("different")), Some(Value::Null)] {
            let mut invalid = original.clone();
            match invalid_id {
                Some(id) => {
                    invalid["id"] = id;
                }
                None => {
                    invalid.as_object_mut().unwrap().remove("id");
                }
            }
            let before = app.snapshot().await;
            app.expect("PUT", &path, Some(invalid), StatusCode::BAD_REQUEST)
                .await;
            assert_eq!(app.snapshot().await, before);
        }
        let before = app.snapshot().await;
        let (status, _, _) = app
            .request_with_headers("PUT", &path, Some(original.clone()), Some("W/\"1\""))
            .await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
        assert_eq!(
            app.snapshot().await,
            before,
            "If-Match cannot create an absent resource"
        );
        let (status, headers, created) = app
            .request_with_headers("PUT", &path, Some(original.clone()), None)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_write_identity(&app, kind, &created, &headers, status, "1").await;
        let created_headers = headers.clone();
        for missing in [
            format!("{path}/_history/999"),
            format!("/{kind}/unknown/_history/1"),
        ] {
            let response = app
                .expect("GET", &missing, None, StatusCode::NOT_FOUND)
                .await;
            assert_eq!(response["resourceType"], "OperationOutcome");
        }
        assert_ne!(
            created["meta"]["lastUpdated"],
            original["meta"]["lastUpdated"]
        );
        assert_eq!(created["meta"]["tag"], original["meta"]["tag"]);
        assert_eq!(created["meta"]["profile"], original["meta"]["profile"]);
        let mut updated = original.clone();
        updated["name"] = json!("Updated");
        let (status, headers, updated) = app
            .request_with_headers("PUT", &path, Some(updated), Some("W/\"1\""))
            .await;
        assert_eq!(status, StatusCode::OK, "{updated}");
        assert_write_identity(&app, kind, &updated, &headers, status, "2").await;
        assert_resource_read(
            &app,
            created_headers["location"].to_str().unwrap(),
            &created,
            &created_headers,
        )
        .await;
        assert_eq!(updated["meta"]["tag"], original["meta"]["tag"]);
        assert_eq!(updated["meta"]["profile"], original["meta"]["profile"]);
        let before = app.snapshot().await;
        let (status, _, _) = app
            .request_with_headers("PUT", &path, Some(original.clone()), Some("W/\"1\""))
            .await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
        assert_eq!(app.snapshot().await, before);
        let mut taken = original.clone();
        taken["id"] = json!("taken");
        let conflict = app
            .expect(
                "PUT",
                &format!("/{kind}/taken"),
                Some(taken),
                StatusCode::CONFLICT,
            )
            .await;
        assert_eq!(conflict["issue"][0]["code"], "conflict");
        assert_eq!(
            app.snapshot().await,
            before,
            "PUT to a new id cannot take another resource's canonical identity"
        );

        // Use a separate identity to pin create v1 → delete v2 → recreate v3.
        let mut restored = original.clone();
        restored["id"] = json!("restored");
        restored["url"] = json!(format!("{ROOT}/restore/{kind}"));
        let restored_path = format!("/{kind}/restored");
        let (status, original_headers, before_delete) = app
            .request_with_headers("PUT", &restored_path, Some(restored.clone()), None)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{before_delete}");
        assert_write_identity(&app, kind, &before_delete, &original_headers, status, "1").await;
        app.expect("DELETE", &restored_path, None, StatusCode::NO_CONTENT)
            .await;
        let deleted = app.snapshot().await;
        assert_resource_read(
            &app,
            original_headers["location"].to_str().unwrap(),
            &before_delete,
            &original_headers,
        )
        .await;
        let deletion_path = format!("{restored_path}/_history/2");
        let deletion = app
            .expect("GET", &deletion_path, None, StatusCode::GONE)
            .await;
        assert_eq!(deletion["resourceType"], "OperationOutcome");
        assert_eq!(deletion["issue"][0]["code"], "deleted");
        assert!(deletion.get("id").is_none());
        assert!(deletion.get("url").is_none());
        let (status, _, _) = app
            .request_with_headers(
                "PUT",
                &restored_path,
                Some(restored.clone()),
                Some("W/\"2\""),
            )
            .await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
        assert_eq!(
            app.snapshot().await,
            deleted,
            "If-Match cannot restore a deleted resource"
        );
        let (status, headers, result) = app
            .request_with_headers("PUT", &restored_path, Some(restored.clone()), None)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{result}");
        assert_write_identity(&app, kind, &result, &headers, status, "3").await;
        assert_resource_read(
            &app,
            original_headers["location"].to_str().unwrap(),
            &before_delete,
            &original_headers,
        )
        .await;
        app.expect("GET", &deletion_path, None, StatusCode::GONE)
            .await;
        let history: Vec<_> = app
            .rows("resource_history")
            .await
            .into_iter()
            .filter(|row| row["resource_type"] == kind && row["id"] == "restored")
            .collect();
        assert_eq!(history.len(), 3);
        assert_eq!(
            history
                .iter()
                .map(|row| row["version_id"].as_str().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["1", "2", "3"])
        );
        for old in deleted["resource_history"]
            .iter()
            .filter(|row| row["resource_type"] == kind && row["id"] == "restored")
        {
            assert!(
                history.contains(old),
                "recreation must preserve earlier history verbatim"
            );
        }
        app.expect("DELETE", &restored_path, None, StatusCode::NO_CONTENT)
            .await;
        let replacement = app
            .expect(
                "POST",
                &format!("/{kind}"),
                Some(restored.clone()),
                StatusCode::CREATED,
            )
            .await;
        let before = app.snapshot().await;
        app.expect("PUT", &restored_path, Some(restored), StatusCode::CONFLICT)
            .await;
        assert_eq!(
            app.snapshot().await,
            before,
            "restore must not take another resource's canonical identity"
        );
        app.expect("GET", &restored_path, None, StatusCode::NOT_FOUND)
            .await;
        assert_eq!(
            app.expect(
                "GET",
                &format!("/{kind}/{}", public_id(&replacement)),
                None,
                StatusCode::OK
            )
            .await,
            replacement
        );
    }
    let metadata = app.expect("GET", "/metadata", None, StatusCode::OK).await;
    for kind in ["CodeSystem", "ValueSet", "ConceptMap"] {
        let entry = metadata["rest"][0]["resource"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["type"] == kind)
            .unwrap();
        assert_eq!(
            entry["updateCreate"], true,
            "{kind} must advertise create-through-PUT"
        );
        assert!(
            entry["interaction"]
                .as_array()
                .unwrap()
                .iter()
                .any(|interaction| { interaction["code"] == "vread" })
        );
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_put_create_update_and_restore_preserve_identity_and_history() {
    put_creation_contract(IdentityApp::sqlite(true)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_put_create_update_and_restore_preserve_identity_and_history() {
    put_creation_contract(IdentityApp::postgres().await).await;
}

async fn failed_creation_contract(app: IdentityApp) {
    for (kind, table) in [
        ("CodeSystem", "code_systems"),
        ("ValueSet", "value_sets"),
        ("ConceptMap", "concept_maps"),
    ] {
        let path = format!("/{kind}/restore-failure");
        let original = terminology(
            kind,
            "restore-failure",
            &format!("{ROOT}/failed-restore/{kind}"),
            Some("1"),
            "one",
        );
        app.expect("PUT", &path, Some(original.clone()), StatusCode::CREATED)
            .await;
        app.expect("DELETE", &path, None, StatusCode::NO_CONTENT)
            .await;
        app.execute(
            &format!(
                "CREATE TRIGGER reject_create_{table} BEFORE INSERT ON {table}
                 WHEN NEW.name = 'RejectNormalization'
                 BEGIN SELECT RAISE(ABORT, 'forced creation failure'); END;"
            ),
            &format!(
                "CREATE FUNCTION reject_create_{table}() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                     IF NEW.name = 'RejectNormalization' THEN
                         RAISE EXCEPTION 'forced creation failure';
                     END IF;
                     RETURN NEW;
                 END $$;
                 CREATE TRIGGER reject_create_{table} BEFORE INSERT ON {table}
                 FOR EACH ROW EXECUTE FUNCTION reject_create_{table}();"
            ),
        )
        .await;
        let before = app.snapshot().await;
        let mut rejected = original.clone();
        rejected["name"] = json!("RejectNormalization");
        app.expect(
            "PUT",
            &path,
            Some(rejected.clone()),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
        assert_eq!(
            app.snapshot().await,
            before,
            "failed {kind} restoration must preserve tombstone and history"
        );
        app.expect("GET", &path, None, StatusCode::NOT_FOUND).await;
        rejected["id"] = json!("new-failure");
        rejected["url"] = json!(format!("{ROOT}/failed-create/{kind}"));
        app.expect(
            "PUT",
            &format!("/{kind}/new-failure"),
            Some(rejected.clone()),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
        assert_eq!(
            app.snapshot().await,
            before,
            "failed {kind} PUT creation must leave no partial rows"
        );
        app.expect(
            "POST",
            &format!("/{kind}"),
            Some(rejected),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
        assert_eq!(
            app.snapshot().await,
            before,
            "failed {kind} POST creation must leave no partial rows"
        );
        // A failed attempt must not consume a version or poison the next transaction.
        let (status, headers, restored) = app
            .request_with_headers("PUT", &path, Some(original), None)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{restored}");
        assert_write_identity(&app, kind, &restored, &headers, status, "3").await;
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_failed_create_and_restore_roll_back_all_stores() {
    failed_creation_contract(IdentityApp::sqlite(true)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_failed_create_and_restore_roll_back_all_stores() {
    failed_creation_contract(IdentityApp::postgres().await).await;
}

async fn atomic_failure_contract(app: IdentityApp) {
    let original = rich_code_system("atomic", &format!("{ROOT}/atomic"), "1");
    app.expect(
        "PUT",
        "/CodeSystem/atomic",
        Some(original.clone()),
        StatusCode::CREATED,
    )
    .await;
    let mut cached = terminology(
        "ValueSet",
        "atomic-cache",
        &format!("{ROOT}/atomic-cache"),
        None,
        "unused",
    );
    cached["compose"] = json!({"include":[{"system":format!("{ROOT}/atomic"),"version":"1"}]});
    app.import(&[cached]).await;
    let expanded = app
        .expect(
            "GET",
            "/ValueSet/atomic-cache/$expand",
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(
        expansion_codes(&expanded),
        BTreeSet::from(["parent".to_owned(), "child".to_owned()])
    );
    assert!(
        !app.rows("value_set_expansions").await.is_empty(),
        "rollback fixture must prewarm a persistent expansion"
    );
    let (status, headers, read) = app
        .request_with_headers("GET", "/CodeSystem/atomic", None, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers["etag"].to_str().unwrap().to_owned();
    let snapshot = app.snapshot().await;
    let mut malformed = original.clone();
    malformed.as_object_mut().unwrap().remove("url");
    app.expect(
        "PUT",
        "/CodeSystem/atomic",
        Some(malformed),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(
        app.snapshot().await,
        snapshot,
        "invalid replacement must not write"
    );

    app.execute(
        "CREATE TRIGGER reject_normalize BEFORE INSERT \
         ON concepts WHEN NEW.code = 'reject-normalize'
         BEGIN SELECT RAISE(ABORT, 'forced normalization failure'); END;
         CREATE TRIGGER reject_closure BEFORE INSERT \
         ON concept_closure WHEN NEW.descendant_code = 'reject-closure'
         BEGIN SELECT RAISE(ABORT, 'forced closure failure'); END;",
        "CREATE FUNCTION reject_identity_normalize() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN IF NEW.code = 'reject-normalize' \
         THEN RAISE EXCEPTION 'forced normalization failure'; END IF; RETURN NEW; END $$;
         CREATE TRIGGER reject_normalize BEFORE INSERT ON concepts FOR EACH ROW \
         EXECUTE FUNCTION reject_identity_normalize();
         CREATE FUNCTION reject_identity_closure() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN IF NEW.descendant_code = 'reject-closure' \
         THEN RAISE EXCEPTION 'forced closure failure'; END IF; RETURN NEW; END $$;
         CREATE TRIGGER reject_closure BEFORE INSERT ON concept_closure FOR EACH ROW \
         EXECUTE FUNCTION reject_identity_closure();",
    )
    .await;
    for failed_code in ["reject-normalize", "reject-closure"] {
        let mut replacement = original.clone();
        replacement["version"] = json!("2");
        replacement["concept"][0]["concept"][0]["code"] = json!(failed_code);
        app.expect(
            "PUT",
            "/CodeSystem/atomic",
            Some(replacement),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
        assert_eq!(
            app.snapshot().await,
            snapshot,
            "{failed_code}: raw/history/normalized writes must roll back together"
        );
        let (status, headers, after) = app
            .request_with_headers("GET", "/CodeSystem/atomic", None, None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["etag"], etag.as_str());
        assert_eq!(after, read);
    }
    app.execute(
        "CREATE TRIGGER reject_identity_delete BEFORE DELETE ON code_systems
         BEGIN SELECT RAISE(ABORT, 'forced deletion failure'); END;",
        "CREATE FUNCTION reject_identity_delete() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'forced deletion failure'; END $$;
         CREATE TRIGGER reject_identity_delete BEFORE DELETE ON code_systems FOR EACH ROW \
         EXECUTE FUNCTION reject_identity_delete();",
    )
    .await;
    app.expect(
        "DELETE",
        "/CodeSystem/atomic",
        None,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert_eq!(
        app.snapshot().await,
        snapshot,
        "failed normalized delete must roll back raw tombstone/history"
    );
    let (_, headers, after) = app
        .request_with_headers("GET", "/CodeSystem/atomic", None, None)
        .await;
    assert_eq!(headers["etag"], etag.as_str());
    assert_eq!(after, read);
}

async fn assert_imported_baseline(app: &IdentityApp, path: &str, original: &Value) {
    let (status, headers, mut baseline) = app
        .request_with_headers("GET", &format!("{path}/_history/1"), None, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{baseline}");
    assert_eq!(baseline["meta"]["versionId"], "1");
    assert_eq!(headers["etag"], "W/\"1\"");
    assert!(baseline["meta"]["lastUpdated"].is_string());
    let mut expected = original.clone();
    baseline.as_object_mut().unwrap().remove("meta");
    expected.as_object_mut().unwrap().remove("meta");
    assert_eq!(
        baseline, expected,
        "promotion must expose the original imported revision"
    );
}

async fn imported_write_contract(app: IdentityApp) {
    for resource_type in ["CodeSystem", "ValueSet"] {
        let url = format!("{ROOT}/imported-write/{resource_type}");
        let imported = terminology(resource_type, "imported", &url, Some("1"), "one");
        let sibling = terminology(resource_type, "imported", &url, Some("2"), "two");
        app.import(&[imported.clone(), sibling.clone()]).await;
        let resources = app.search(resource_type, &format!("&url={url}")).await;
        let selected = resources.iter().find(|r| r["version"] == "1").unwrap();
        let id = public_id(selected).to_owned();
        assert_eq!(id, "imported", "first import keeps its available source ID");
        let sibling = resources
            .iter()
            .find(|r| r["version"] == "2")
            .unwrap()
            .clone();
        let path = format!("/{resource_type}/{id}");
        let (_, headers, before_read) = app.request_with_headers("GET", &path, None, None).await;
        assert!(
            !headers.contains_key("etag"),
            "import-only resource has no persistence version yet"
        );
        app.expect(
            "GET",
            &format!("{path}/_history/1"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;
        let baseline = app.snapshot().await;
        let mut replacement = imported.clone();
        replacement["name"] = json!("Updated");
        let (status, _, _) = app
            .request_with_headers("PUT", &path, Some(replacement.clone()), Some("W/\"1\""))
            .await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
        assert_eq!(
            app.snapshot().await,
            baseline,
            "failed If-Match must not materialize a baseline"
        );

        let mut collision = replacement.clone();
        collision["version"] = json!("2");
        app.expect("PUT", &path, Some(collision), StatusCode::CONFLICT)
            .await;
        assert_eq!(
            app.snapshot().await,
            baseline,
            "canonical collision must preserve both owners"
        );
        let (status, headers, updated) = app
            .request_with_headers("PUT", &path, Some(replacement), None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["id"], id);
        assert_eq!(headers["etag"], "W/\"2\"");
        assert_imported_baseline(&app, &path, &before_read).await;
        let history = app
            .rows("resource_history")
            .await
            .into_iter()
            .filter(|row| row["resource_type"] == resource_type && row["id"] == id)
            .collect::<Vec<_>>();
        assert_eq!(
            history.len(),
            2,
            "PUT must persist a real imported baseline and update"
        );
        assert_eq!(
            history
                .iter()
                .map(|row| row["version_id"].as_str().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["1", "2"])
        );
        let original = history.iter().find(|row| row["version_id"] == "1").unwrap();
        assert_eq!(original["data"]["url"], before_read["url"]);
        assert_eq!(original["data"]["name"], before_read["name"]);
        let after_update = app.snapshot().await;
        let mut collision = updated.clone();
        collision["version"] = json!("2");
        app.expect("PUT", &path, Some(collision), StatusCode::CONFLICT)
            .await;
        assert_eq!(
            app.snapshot().await,
            after_update,
            "raw-backed canonical collision must preserve both owners"
        );
        let sibling_path = format!("/{resource_type}/{}", public_id(&sibling));
        assert_eq!(
            app.expect("GET", &sibling_path, None, StatusCode::OK).await,
            sibling
        );

        app.expect("DELETE", &sibling_path, None, StatusCode::NO_CONTENT)
            .await;
        app.expect("GET", &sibling_path, None, StatusCode::NOT_FOUND)
            .await;
        let deleted = app.snapshot().await;
        app.expect("DELETE", &sibling_path, None, StatusCode::NOT_FOUND)
            .await;
        assert_eq!(
            app.snapshot().await,
            deleted,
            "repeated DELETE must not append history or mutate normalized rows"
        );
        let sibling_history = app
            .rows("resource_history")
            .await
            .into_iter()
            .filter(|row| row["resource_type"] == resource_type && row["id"] == sibling["id"])
            .collect::<Vec<_>>();
        assert_eq!(
            sibling_history.len(),
            2,
            "import-only DELETE must persist a baseline and tombstone"
        );
        let raw = app
            .rows("resources")
            .await
            .into_iter()
            .find(|row| row["resource_type"] == resource_type && row["id"] == sibling["id"])
            .unwrap();
        assert!(raw["is_deleted"] == true || raw["is_deleted"] == 1);
        assert_eq!(
            app.expect("GET", &path, None, StatusCode::OK).await,
            updated,
            "deleting sibling must preserve selected resource"
        );
        app.import(std::slice::from_ref(&sibling)).await;
        app.expect("GET", &sibling_path, None, StatusCode::NOT_FOUND)
            .await;
        let reimported = app
            .search(resource_type, &format!("&url={url}"))
            .await
            .into_iter()
            .find(|r| r["version"] == "2")
            .unwrap();
        assert_ne!(
            reimported["id"], sibling["id"],
            "reimport must not resurrect tombstoned ID"
        );
        let before = app.snapshot().await;
        app.expect(
            "DELETE",
            &format!("/{resource_type}/never-used"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;
        assert_eq!(
            app.snapshot().await,
            before,
            "DELETE of a never-used ID must not write any rows"
        );
    }
}

async fn imported_concept_map_write_contract(app: IdentityApp) {
    let source = format!("{ROOT}/map-source");
    let target = format!("{ROOT}/map-target");
    let mut imported = terminology(
        "ConceptMap",
        "imported-map",
        &format!("{ROOT}/imported-map"),
        Some("1"),
        "one",
    );
    imported["group"] = json!([{
        "source": source, "target": target,
        "element": [{"code": "one", "target": [{
            "code": "uno", "equivalence": "equivalent"
        }]}]
    }]);
    let mut sibling = imported.clone();
    sibling["id"] = json!("other-map");
    sibling["url"] = json!(format!("{ROOT}/other-map"));
    app.import(&[imported.clone(), sibling.clone()]).await;
    let path = "/ConceptMap/imported-map";
    let (status, headers, original) = app.request_with_headers("GET", path, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(original["id"], "imported-map");
    assert!(!headers.contains_key("etag"));
    app.expect(
        "GET",
        &format!("{path}/_history/1"),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
    let translation_path = format!("{path}/$translate?system={source}&code=one");
    let before = app
        .expect("GET", &translation_path, None, StatusCode::OK)
        .await;
    assert!(
        before["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .any(|parameter| {
                parameter["name"] == "match"
                    && parameter["part"].as_array().unwrap().iter().any(|part| {
                        part["name"] == "concept" && part["valueCoding"]["code"] == "uno"
                    })
            })
    );

    let mut replacement = imported.clone();
    replacement["name"] = json!("UpdatedMap");
    replacement["group"][0]["element"][0]["target"][0]["code"] = json!("dos");
    let baseline = app.snapshot().await;
    let (status, _, _) = app
        .request_with_headers("PUT", path, Some(replacement.clone()), Some("W/\"1\""))
        .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(app.snapshot().await, baseline);
    let (status, headers, updated) = app
        .request_with_headers("PUT", path, Some(replacement), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["etag"], "W/\"2\"");
    assert_eq!(updated["id"], original["id"]);
    assert_eq!(app.expect("GET", path, None, StatusCode::OK).await, updated);
    assert_imported_baseline(&app, path, &original).await;
    let history = app
        .rows("resource_history")
        .await
        .into_iter()
        .filter(|row| row["resource_type"] == "ConceptMap" && row["id"] == "imported-map")
        .collect::<Vec<_>>();
    assert_eq!(history.len(), 2);
    assert_eq!(
        history.iter().find(|r| r["version_id"] == "1").unwrap()["data"]["group"],
        original["group"]
    );
    assert_eq!(
        history.iter().find(|r| r["version_id"] == "2").unwrap()["data"]["group"],
        updated["group"]
    );
    let after = app
        .expect("GET", &translation_path, None, StatusCode::OK)
        .await;
    let matches = after["parameter"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["name"] == "match")
        .flat_map(|p| p["part"].as_array().unwrap())
        .filter(|p| p["name"] == "concept")
        .map(|p| p["valueCoding"]["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(matches, vec!["dos"]);

    // DELETE must also establish history for a map that has never been PUT.
    let sibling_path = "/ConceptMap/other-map";
    app.expect("DELETE", sibling_path, None, StatusCode::NO_CONTENT)
        .await;
    app.expect("GET", sibling_path, None, StatusCode::NOT_FOUND)
        .await;
    let history = app
        .rows("resource_history")
        .await
        .into_iter()
        .filter(|row| row["resource_type"] == "ConceptMap" && row["id"] == "other-map")
        .collect::<Vec<_>>();
    assert_eq!(history.len(), 2);
    assert_eq!(
        history.iter().find(|r| r["version_id"] == "1").unwrap()["data"]["group"],
        sibling["group"]
    );
    let deleted = app.snapshot().await;
    app.expect("DELETE", sibling_path, None, StatusCode::NOT_FOUND)
        .await;
    assert_eq!(app.snapshot().await, deleted);
    assert_eq!(app.expect("GET", path, None, StatusCode::OK).await, updated);
    assert_eq!(app.search("ConceptMap", "").await.len(), 1);

    app.expect("DELETE", path, None, StatusCode::NO_CONTENT)
        .await;
    app.expect("GET", &translation_path, None, StatusCode::NOT_FOUND)
        .await;
    assert!(app.search("ConceptMap", "").await.is_empty());
    assert!(app.rows("concept_map_elements").await.is_empty());
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_import_only_concept_map_writes_preserve_history_and_translation() {
    imported_concept_map_write_contract(IdentityApp::sqlite(true)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_import_only_concept_map_writes_preserve_history_and_translation() {
    imported_concept_map_write_contract(IdentityApp::postgres().await).await;
}

async fn delete_cascade_contract(app: IdentityApp) {
    let url = format!("{ROOT}/cascade");
    app.import(&[
        rich_code_system("cascade", &url, "1"),
        rich_code_system("cascade", &url, "2"),
    ])
    .await;
    let selected = app
        .search("CodeSystem", &format!("&url={url}"))
        .await
        .into_iter()
        .find(|r| r["version"] == "1")
        .unwrap();
    let before = app.snapshot().await;
    let storage_id = before["code_systems"]
        .iter()
        .find(|r| r["version"] == "1")
        .unwrap()["id"]
        .clone();
    let concept_ids = before["concepts"]
        .iter()
        .filter(|r| r["system_id"] == storage_id)
        .map(|r| r["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(concept_ids.len(), 2);
    for table in ["concept_properties", "concept_designations"] {
        assert_eq!(
            before[table]
                .iter()
                .filter(|r| concept_ids.contains(&r["concept_id"]))
                .count(),
            2
        );
    }
    for table in ["concept_hierarchy", "concept_closure"] {
        assert!(
            !before[table]
                .iter()
                .filter(|r| r["system_id"] == storage_id)
                .collect::<Vec<_>>()
                .is_empty(),
            "fixture must contain {table}"
        );
    }
    app.expect(
        "DELETE",
        &format!("/CodeSystem/{}", public_id(&selected)),
        None,
        StatusCode::NO_CONTENT,
    )
    .await;
    let after = app.snapshot().await;
    for table in [
        "code_systems",
        "concepts",
        "concept_hierarchy",
        "concept_closure",
    ] {
        let remaining = before[table]
            .iter()
            .filter(|r| {
                if table == "code_systems" {
                    r["id"] != storage_id
                } else {
                    r["system_id"] != storage_id
                }
            })
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            after[table], remaining,
            "delete must remove selected {table} only"
        );
    }
    for table in ["concept_properties", "concept_designations"] {
        let remaining = before[table]
            .iter()
            .filter(|r| !concept_ids.contains(&r["concept_id"]))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            after[table], remaining,
            "delete must cascade {table}, preserving sibling data"
        );
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_post_discards_supplied_ids_and_preserves_existing_owners() {
    post_identity_contract(IdentityApp::sqlite(true)).await;
}
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_post_discards_supplied_ids_and_preserves_existing_owners() {
    post_identity_contract(IdentityApp::postgres().await).await;
}
#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_failed_normalization_rolls_back_raw_history_and_indexes() {
    atomic_failure_contract(IdentityApp::sqlite(true)).await;
}
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_failed_normalization_rolls_back_raw_history_and_indexes() {
    atomic_failure_contract(IdentityApp::postgres().await).await;
}
#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_import_only_writes_preserve_identity_and_real_history() {
    imported_write_contract(IdentityApp::sqlite(true)).await;
}
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_import_only_writes_preserve_identity_and_real_history() {
    imported_write_contract(IdentityApp::postgres().await).await;
}
#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_delete_removes_selected_children_and_preserves_sibling_rows() {
    delete_cascade_contract(IdentityApp::sqlite(true)).await;
}
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_delete_removes_selected_children_and_preserves_sibling_rows() {
    delete_cascade_contract(IdentityApp::postgres().await).await;
}

fn expansion_codes(expansion: &Value) -> BTreeSet<String> {
    fn visit(entries: &[Value], codes: &mut BTreeSet<String>) {
        for entry in entries {
            if let Some(code) = entry["code"].as_str() {
                codes.insert(code.to_owned());
            }
            if let Some(children) = entry["contains"].as_array() {
                visit(children, codes);
            }
        }
    }
    let mut codes = BTreeSet::new();
    visit(
        expansion["expansion"]["contains"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        &mut codes,
    );
    codes
}

async fn expand_request(
    app: &IdentityApp,
    method: &str,
    instance: Option<&str>,
    url: &str,
    version: Option<&str>,
    options: &[(&str, Value)],
) -> (StatusCode, Value) {
    let path = instance.map_or_else(
        || "/ValueSet/$expand".to_owned(),
        |id| format!("/ValueSet/{id}/$expand"),
    );
    let mut parameters = options
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect::<Vec<_>>();
    if instance.is_none() {
        parameters.push(("url".into(), json!(url)));
        if let Some(version) = version {
            parameters.push(("valueSetVersion".into(), json!(version)));
        }
    }
    if method == "GET" {
        let mut query = form_urlencoded::Serializer::new(String::new());
        for (name, value) in parameters {
            query.append_pair(
                &name,
                &value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
        app.request("GET", &format!("{path}?{}", query.finish()), None)
            .await
    } else {
        let parameters = parameters
            .into_iter()
            .map(|(name, value)| {
                let kind = if value.is_boolean() {
                    "valueBoolean"
                } else if value.is_number() {
                    "valueInteger"
                } else {
                    "valueString"
                };
                let mut parameter = json!({"name":name});
                parameter[kind] = value;
                parameter
            })
            .collect::<Vec<_>>();
        app.request(
            "POST",
            &path,
            Some(json!({"resourceType":"Parameters","parameter":parameters})),
        )
        .await
    }
}

async fn mixed_expansion_invalidation_contract(app: IdentityApp) {
    let system_a = format!("{ROOT}/mixed-a");
    let system_b = format!("{ROOT}/mixed-b");
    let mixed_url = format!("{ROOT}/mixed-valueset");
    let unrelated_url = format!("{ROOT}/unrelated-valueset");
    let mut mixed = terminology("ValueSet", "mixed-valueset", &mixed_url, None, "unused");
    mixed["compose"] = json!({"include":[{"system":system_a},{"system":system_b}]});
    let mut unrelated = terminology(
        "ValueSet",
        "unrelated-valueset",
        &unrelated_url,
        None,
        "unused",
    );
    unrelated["compose"] = json!({"include":[{"system":system_b}]});
    app.import(&[
        terminology("CodeSystem", "mixed-a", &system_a, Some("1"), "old-a"),
        terminology("CodeSystem", "mixed-b", &system_b, Some("1"), "retained-b"),
        mixed,
        unrelated,
    ])
    .await;
    let b_before = app
        .expect("GET", "/CodeSystem/mixed-b", None, StatusCode::OK)
        .await;
    for (url, expected) in [
        (
            &mixed_url,
            BTreeSet::from(["old-a".to_owned(), "retained-b".to_owned()]),
        ),
        (&unrelated_url, BTreeSet::from(["retained-b".to_owned()])),
    ] {
        let (status, expansion) = expand_request(&app, "GET", None, url, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{expansion}");
        assert_eq!(expansion_codes(&expansion), expected);
    }
    let rows = app.rows("value_sets").await;
    let mixed_id = &rows.iter().find(|r| r["url"] == mixed_url).unwrap()["id"];
    let unrelated_id = &rows.iter().find(|r| r["url"] == unrelated_url).unwrap()["id"];
    let cached = app.rows("value_set_expansions").await;
    assert_eq!(
        cached
            .iter()
            .filter(|r| &r["value_set_id"] == mixed_id)
            .count(),
        2
    );
    let unrelated_before = cached
        .iter()
        .filter(|r| &r["value_set_id"] == unrelated_id)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(unrelated_before.len(), 1);
    app.expect(
        "PUT",
        "/CodeSystem/mixed-a",
        Some(terminology(
            "CodeSystem",
            "mixed-a",
            &system_a,
            Some("1"),
            "new-a",
        )),
        StatusCode::OK,
    )
    .await;
    let cached = app.rows("value_set_expansions").await;
    assert!(
        cached.iter().all(|r| &r["value_set_id"] != mixed_id),
        "all rows of an affected expansion must be invalidated, including unchanged system B"
    );
    assert_eq!(
        cached
            .iter()
            .filter(|r| &r["value_set_id"] == unrelated_id)
            .cloned()
            .collect::<Vec<_>>(),
        unrelated_before
    );
    assert_eq!(
        app.expect("GET", "/CodeSystem/mixed-b", None, StatusCode::OK)
            .await,
        b_before
    );
    for method in ["GET", "POST"] {
        let (status, expansion) = expand_request(&app, method, None, &mixed_url, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{expansion}");
        assert_eq!(
            expansion_codes(&expansion),
            BTreeSet::from(["new-a".to_owned(), "retained-b".to_owned()])
        );
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_update_invalidates_whole_mixed_expansion_and_keeps_unrelated_cache() {
    mixed_expansion_invalidation_contract(IdentityApp::sqlite(true)).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_update_invalidates_whole_mixed_expansion_and_keeps_unrelated_cache() {
    mixed_expansion_invalidation_contract(IdentityApp::postgres().await).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_delete_invalidates_warm_implicit_expansion_and_preserves_sibling() {
    let app = IdentityApp::sqlite(true);
    let system = format!("{ROOT}/implicit-delete");
    let resources = [
        terminology(
            "CodeSystem",
            "implicit-delete",
            &system,
            Some("1"),
            "surviving-code",
        ),
        terminology(
            "CodeSystem",
            "implicit-delete",
            &system,
            Some("2"),
            "deleted-code",
        ),
    ];
    app.import(&resources).await;
    let stored = app.search("CodeSystem", &format!("&url={system}")).await;
    let selected = stored.iter().find(|r| r["version"] == "2").unwrap();
    let sibling = stored.iter().find(|r| r["version"] == "1").unwrap();
    let sibling_path = format!("/CodeSystem/{}", public_id(sibling));
    let sibling_before = app.expect("GET", &sibling_path, None, StatusCode::OK).await;
    let implicit_url = format!("{system}?fhir_vs");
    // An unbounded request synchronously populates the persistent cache; no
    // background cache-warming thread or timing assumptions are involved.
    for method in ["GET", "POST"] {
        let (status, expansion) =
            expand_request(&app, method, None, &implicit_url, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{expansion}");
        assert_eq!(
            expansion_codes(&expansion),
            BTreeSet::from(["deleted-code".to_owned()])
        );
    }
    let cached = app.rows("implicit_expansion_cache").await;
    assert_eq!(
        cached.len(),
        1,
        "fixture must populate the persistent cache"
    );
    assert_eq!(cached[0]["code"], "deleted-code");

    app.expect(
        "DELETE",
        &format!("/CodeSystem/{}", public_id(selected)),
        None,
        StatusCode::NO_CONTENT,
    )
    .await;
    assert!(
        app.rows("implicit_expansion_cache").await.is_empty(),
        "delete must invalidate persisted implicit concepts"
    );
    assert_eq!(
        app.expect("GET", &sibling_path, None, StatusCode::OK).await,
        sibling_before
    );
    for method in ["GET", "POST", "GET"] {
        let (status, expansion) =
            expand_request(&app, method, None, &implicit_url, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{expansion}");
        assert_eq!(
            expansion_codes(&expansion),
            BTreeSet::from(["surviving-code".to_owned()])
        );
    }
    let cached = app.rows("implicit_expansion_cache").await;
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0]["code"], "surviving-code");
}

async fn expansion_execution_contract(app: IdentityApp) {
    let system = format!("{ROOT}/expand-system");
    let mut cs1 = rich_code_system("expand-system", &system, "1");
    cs1["concept"][0]["code"] = json!("parent-v1");
    cs1["concept"][0]["concept"][0]["code"] = json!("child-v1");
    let mut cs2 = cs1.clone();
    cs2["version"] = json!("2");
    cs2["concept"][0]["code"] = json!("parent-v2");
    cs2["concept"][0]["concept"][0]["code"] = json!("child-v2");
    let mut resources = vec![cs1, cs2];
    for (id, suffix, pin) in [
        ("expand-pinned", "pinned", Some("1")),
        ("expand-reverse", "reverse", Some("1")),
        ("expand-default", "default", None),
    ] {
        let mut vs = terminology(
            "ValueSet",
            id,
            &format!("{ROOT}/expand/{suffix}"),
            Some("1"),
            "unused",
        );
        vs["compose"] = json!({"include":[{"system":system}]});
        if let Some(pin) = pin {
            vs["compose"]["include"][0]["version"] = json!(pin);
        }
        vs["contained"] = json!([{
            "resourceType": "CodeSystem",
            "id": "unused-contained",
            "url": format!("{ROOT}/contained"),
            "status": "active",
            "content": "complete",
            "concept": [{"code": "must-not-appear"}]
        }]);
        resources.push(vs);
    }
    let inner_url = format!("{ROOT}/expand/inner");
    for version in ["1", "2"] {
        let mut inner = terminology(
            "ValueSet",
            "expand-inner",
            &inner_url,
            Some(version),
            "unused",
        );
        inner["compose"] = json!({"include":[{"system":system,"version":version}]});
        resources.push(inner);
    }
    let outer_url = format!("{ROOT}/expand/outer");
    let mut outer = terminology("ValueSet", "expand-outer", &outer_url, Some("1"), "unused");
    outer["compose"] = json!({"include":[{"valueSet":[inner_url]}]});
    resources.push(outer);
    let missing_url = format!("{ROOT}/expand/missing");
    let mut missing = terminology(
        "ValueSet",
        "expand-missing",
        &missing_url,
        Some("1"),
        "unused",
    );
    missing["compose"] = json!({"include":[{"system":format!("{ROOT}/never-imported")}]});
    resources.push(missing);
    app.import(&resources).await;

    let expected1 = BTreeSet::from(["parent-v1".to_owned(), "child-v1".to_owned()]);
    let expected2 = BTreeSet::from(["parent-v2".to_owned(), "child-v2".to_owned()]);
    // Every cold/warm request is checked against fixture codes, not only the other route.
    for method in ["GET", "POST"] {
        for instance in [true, false] {
            let id = instance.then_some("expand-pinned");
            let url = format!("{ROOT}/expand/pinned");
            let calls = [
                (vec![], &expected1),
                (
                    vec![("force-system-version", json!(format!("{system}|2")))],
                    &expected2,
                ),
                (
                    vec![("force-system-version", json!(format!("{system}|2")))],
                    &expected2,
                ),
                (vec![], &expected1),
                (
                    vec![("system-version", json!(format!("{system}|2")))],
                    &expected1,
                ),
            ];
            for (options, expected) in calls {
                let (status, result) =
                    expand_request(&app, method, id, &url, Some("1"), &options).await;
                assert_eq!(
                    status,
                    StatusCode::OK,
                    "{method} {instance} {options:?}: {result}"
                );
                assert_eq!(expansion_codes(&result), *expected);
                assert_eq!(result["id"], "expand-pinned");
                assert_eq!(result["version"], "1");
                assert!(
                    result.get("contained").is_none(),
                    "stored expansion must not echo contained resources"
                );
            }
            for _ in 0..2 {
                let (status, hierarchy) = expand_request(
                    &app,
                    method,
                    id,
                    &url,
                    Some("1"),
                    &[("excludeNested", json!(false))],
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{hierarchy}");
                assert_eq!(expansion_codes(&hierarchy), expected1);
                let parent = hierarchy["expansion"]["contains"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| r["code"] == "parent-v1")
                    .expect("hierarchy root");
                assert!(
                    parent["contains"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|r| r["code"] == "child-v1"),
                    "excludeNested=false must retain child nesting"
                );
                let (status, filtered) = expand_request(
                    &app,
                    method,
                    id,
                    &url,
                    Some("1"),
                    &[("filter", json!("Child")), ("count", json!(1))],
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{filtered}");
                assert_eq!(
                    expansion_codes(&filtered),
                    BTreeSet::from(["child-v1".to_owned()])
                );
            }
        }
    }
    // Forced-first must not poison persistent caches used by later ordinary expansions.
    for (suffix, id, parameter, forced, ordinary) in [
        (
            "reverse",
            "expand-reverse",
            "force-system-version",
            format!("{system}|2"),
            &expected1,
        ),
        (
            "default",
            "expand-default",
            "system-version",
            format!("{system}|1"),
            &expected2,
        ),
        (
            "outer",
            "expand-outer",
            "default-valueset-version",
            format!("{inner_url}|1"),
            &expected2,
        ),
    ] {
        let forced_expected = if suffix == "reverse" {
            &expected2
        } else {
            &expected1
        };
        let url = format!("{ROOT}/expand/{suffix}");
        for instance in [true, false] {
            for (options, expected) in [
                (vec![(parameter, json!(forced))], forced_expected),
                (vec![], ordinary),
                (vec![(parameter, json!(forced))], forced_expected),
            ] {
                let (status, result) = expand_request(
                    &app,
                    "GET",
                    instance.then_some(id),
                    &url,
                    Some("1"),
                    &options,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{suffix}: {result}");
                assert_eq!(
                    expansion_codes(&result),
                    *expected,
                    "{suffix}: override must survive cold/warm cache transitions"
                );
            }
        }
    }
    for method in ["GET", "POST"] {
        let canonical = expand_request(&app, method, None, &missing_url, Some("1"), &[]).await;
        let instance = expand_request(
            &app,
            method,
            Some("expand-missing"),
            &missing_url,
            Some("1"),
            &[],
        )
        .await;
        assert_eq!(
            canonical.0,
            StatusCode::OK,
            "stored URL expansion skips unknown systems"
        );
        assert_eq!(
            instance.0,
            StatusCode::OK,
            "stored instance must use the same executor"
        );
        assert_eq!(canonical.1["expansion"]["total"], 0);
        assert_eq!(instance.1["expansion"]["total"], 0);
        assert_eq!(instance.1.get("issue"), canonical.1.get("issue"));
        assert_eq!(expansion_codes(&instance.1), BTreeSet::new());
        assert_eq!(
            instance.1.get("expansion").map(|e| &e["total"]),
            canonical.1.get("expansion").map(|e| &e["total"])
        );
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn sqlite_instance_expansion_uses_url_execution_semantics() {
    expansion_execution_contract(IdentityApp::sqlite(true)).await;
}
#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_instance_expansion_uses_url_execution_semantics() {
    expansion_execution_contract(IdentityApp::postgres().await).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_instance_expansion_populates_the_selected_persistent_cache() {
    let app = IdentityApp::postgres().await;
    app.import(&collision_fixture()).await;
    let values = app.search("ValueSet", "").await;
    let selected = values
        .iter()
        .find(|r| r["url"] == format!("{ROOT}/ValueSet/family") && r.get("version").is_none())
        .unwrap();
    let id = public_id(selected);
    assert!(app.rows("value_set_expansions").await.is_empty());
    for method in ["GET", "POST"] {
        let (status, result) = expand_request(
            &app,
            method,
            Some(id),
            selected["url"].as_str().unwrap(),
            None,
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["id"], id);
        assert!(result.get("version").is_none());
        assert_eq!(
            expansion_codes(&result),
            BTreeSet::from(["unversioned".to_owned()])
        );
    }
    let normalized = app.rows("value_sets").await;
    let storage_id = &normalized
        .iter()
        .find(|row| row["resource_json"]["id"] == id)
        .unwrap()["id"];
    let cache = app.rows("value_set_expansions").await;
    assert_eq!(
        cache.len(),
        1,
        "instance expansion must materialize the existing normalized ValueSet row"
    );
    assert_eq!(cache[0]["value_set_id"], *storage_id);
    assert_eq!(cache[0]["code"], "unversioned");

    // Unrelated ValueSet imports clear both handler and backend memory caches,
    // while leaving this ValueSet's persistent expansion rows untouched.
    let selected_id = id.replace('\'', "''");
    app.execute(
        "",
        &format!(
            "CREATE FUNCTION reject_selected_cache_rewrite() RETURNS trigger LANGUAGE plpgsql AS $$
         DECLARE selected_id text;
         BEGIN
           IF TG_OP = 'DELETE' THEN selected_id := OLD.value_set_id;
           ELSE selected_id := NEW.value_set_id; END IF;
           IF EXISTS (SELECT 1 FROM value_sets WHERE id = selected_id \
           AND resource_json->>'id' = '{selected_id}') THEN
             RAISE EXCEPTION 'selected persistent cache must be reused without rewriting';
           END IF;
           IF TG_OP = 'DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
         END $$;
         CREATE TRIGGER reject_selected_cache_rewrite BEFORE INSERT OR UPDATE OR DELETE
         ON value_set_expansions FOR EACH ROW EXECUTE FUNCTION reject_selected_cache_rewrite();"
        ),
    )
    .await;
    for (index, method) in ["GET", "POST"].into_iter().enumerate() {
        let unrelated = terminology(
            "ValueSet",
            &format!("cache-evict-{index}"),
            &format!("{ROOT}/cache-evict/{index}"),
            None,
            "one",
        );
        app.import(&[unrelated]).await;
        assert_eq!(
            app.rows("value_set_expansions").await,
            cache,
            "evicting memory must retain the selected persistent cache"
        );
        let (status, result) = expand_request(
            &app,
            method,
            Some(id),
            selected["url"].as_str().unwrap(),
            None,
            &[],
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "persistent cache must be read after memory eviction: {result}"
        );
        assert_eq!(result["id"], id);
        assert!(result.get("version").is_none());
        assert_eq!(
            expansion_codes(&result),
            BTreeSet::from(["unversioned".to_owned()])
        );
        assert_eq!(app.rows("value_set_expansions").await, cache);
    }
}
