//! #2593: every `searchParam` that `GET /metadata` lists is one the storage's
//! `search` serves.
//!
//! `/metadata` builds each type's `searchParam` list from the SearchParameter
//! registry, filtered by `SearchProvider::serves_search_param`. Before that
//! filter the list was the whole registry, so it named parameters the backend
//! refuses: `Location.near` (`400` on every backend) and, on MongoDB, `_text`
//! and `_content` (`501`). FHIR's `searchParam` lists the "Search parameters for
//! implementations to support and/or make use of"
//! (https://hl7.org/fhir/R4/capabilitystatement-definitions.html#CapabilityStatement.rest.resource.searchParam).
//!
//! Each test reads the statement a real backend's server answers, for each
//! enabled FHIR version, and searches every listed parameter of every type once,
//! with a value its type parses. None may be refused: every search answers
//! `200`. A composite's value has one part per component, and `/metadata`
//! lists a composite only when each component is in the registry and served,
//! so that holds for composites too. Where `_text` and `_content` are listed,
//! each must also find a stored Patient by its narrative and miss it by a word
//! it does not hold, so a backend that stops evaluating them (and answers an
//! empty `200`) fails. Each test also pins what that backend leaves out.
//!
//! SQLite runs everywhere. MongoDB, PostgreSQL and Elasticsearch (behind a
//! SQLite primary, as `hfs` wires `sqlite-elasticsearch`) start a container
//! through testcontainers, or use `HFS_TEST_MONGODB_URL`, `HFS_PG_HOST` (with
//! `HFS_PG_PORT` and `HFS_PG_PASSWORD`) or `HFS_TEST_ES_URL` when set.

#![cfg(any(feature = "sqlite", feature = "mongodb", feature = "postgres"))]

#[cfg(any(feature = "mongodb", feature = "postgres", feature = "elasticsearch"))]
#[path = "common/container_cleanup.rs"]
mod container_cleanup;

/// The check every backend's test runs.
mod contract {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use axum::http::{HeaderName, HeaderValue, StatusCode, header};
    use axum_test::TestServer;
    use futures::StreamExt;
    use helios_fhir::FhirVersion;
    use helios_persistence::composite::DynSearchProvider;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use helios_rest::ServerConfig;
    use helios_rest::config::{MultitenancyConfig, TenantRoutingMode};
    use serde_json::Value;

    const X_TENANT_ID: HeaderName = HeaderName::from_static("x-tenant-id");

    /// The repository's `data/` directory: the spec SearchParameters.
    pub fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../data")
            .canonicalize()
            .expect("repo data dir")
    }

    /// Header-routed, under `tenant`.
    pub fn config(tenant: &str) -> ServerConfig {
        ServerConfig {
            multitenancy: MultitenancyConfig {
                routing_mode: TenantRoutingMode::HeaderOnly,
                ..Default::default()
            },
            base_url: "http://localhost:8080".to_string(),
            default_tenant: tenant.to_string(),
            ..ServerConfig::for_testing()
        }
    }

    fn accept(version: FhirVersion) -> HeaderValue {
        HeaderValue::from_str(&format!(
            "application/fhir+json; fhirVersion={}",
            version.as_mime_param()
        ))
        .expect("accept")
    }

    /// Each type's `searchParam` entries, `(name, type)`, by type.
    pub type Listed = BTreeMap<String, BTreeSet<(String, String)>>;

    /// The `searchParam` lists of the statement `server` answers for `version`.
    pub async fn listed(server: &TestServer, tenant: &str, version: FhirVersion) -> Listed {
        let response = server
            .get("/metadata")
            .add_header(X_TENANT_ID, HeaderValue::from_str(tenant).expect("tenant"))
            .add_header(header::ACCEPT, accept(version))
            .await;
        response.assert_status(StatusCode::OK);
        let statement: Value = serde_json::from_str(&response.text()).expect("JSON");
        statement["rest"][0]["resource"]
            .as_array()
            .expect("resources")
            .iter()
            .map(|resource| {
                let params = resource["searchParam"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|p| {
                                (
                                    p["name"].as_str().expect("name").to_string(),
                                    p["type"].as_str().expect("type").to_string(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (resource["type"].as_str().expect("type").to_string(), params)
            })
            .collect()
    }

    /// The names `listed` has for `resource_type`.
    pub fn names(listed: &Listed, resource_type: &str) -> BTreeSet<String> {
        listed
            .get(resource_type)
            .unwrap_or_else(|| panic!("{resource_type} not in the statement"))
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// A server over a backend, the tenant it answers, and the storage behind
    /// it, for its registry.
    pub struct Served {
        pub server: TestServer,
        pub tenant: String,
        pub storage: DynSearchProvider,
    }

    /// One search: its type, parameter and value.
    struct Search {
        resource_type: String,
        name: String,
        value: String,
    }

    /// The search for one listed parameter. `2020` parses as every type: a
    /// year, a number, a quantity, a code, a string, an id and a relative URI.
    /// A composite's value has one `2020` per component; each must resolve, or
    /// the composite should not have been listed.
    fn search(served: &Served, resource_type: &str, name: &str, param_type: &str) -> Search {
        let mut value = match name {
            "_contained" => "false".to_string(),
            "_containedType" => "container".to_string(),
            _ => "2020".to_string(),
        };
        if param_type == "composite" {
            let tenant = TenantContext::new(
                TenantId::new(&served.tenant),
                TenantPermissions::full_access(),
            );
            let registry = served.storage.search_param_registry(&tenant);
            let registry = registry.read();
            let definition = registry
                .get_param(resource_type, name)
                .unwrap_or_else(|| panic!("{resource_type}.{name} is not registered"));
            let components = definition.component.clone().unwrap_or_default();
            assert!(
                !components.is_empty()
                    && components
                        .iter()
                        .all(|c| registry.get_by_url(&c.definition).is_some()),
                "{resource_type}.{name} is listed, but a component is not in the registry"
            );
            value = vec!["2020"; components.len()].join("$");
        }
        Search {
            resource_type: resource_type.to_string(),
            name: name.to_string(),
            value,
        }
    }

    /// Searches every parameter `listed` names once, and returns the searches
    /// that did not answer `200`, with status and body.
    pub async fn refused(served: &Served, version: FhirVersion, listed: &Listed) -> Vec<String> {
        let searches: Vec<Search> = listed
            .iter()
            .flat_map(|(resource_type, params)| {
                params
                    .iter()
                    .map(move |(name, param_type)| search(served, resource_type, name, param_type))
            })
            .collect();
        assert!(searches.len() > 1000, "{} searches", searches.len());
        let tenant = HeaderValue::from_str(&served.tenant).expect("tenant");
        futures::stream::iter(searches)
            .map(|search| {
                let tenant = tenant.clone();
                async move {
                    let response = served
                        .server
                        .get(&format!("/{}", search.resource_type))
                        .add_query_param(&search.name, &search.value)
                        .add_query_param("_count", "1")
                        .add_header(X_TENANT_ID, tenant)
                        .add_header(header::ACCEPT, accept(version))
                        .await;
                    let status = response.status_code();
                    (status != StatusCode::OK).then(|| {
                        format!(
                            "{}?{}={}: {status} {}",
                            search.resource_type,
                            search.name,
                            search.value,
                            response.text().chars().take(400).collect::<String>()
                        )
                    })
                }
            })
            .buffer_unordered(8)
            .filter_map(|refused| async move { refused })
            .collect()
            .await
    }

    /// The ids of the Patients `GET /Patient?{name}={value}` finds.
    async fn patients_found(
        served: &Served,
        version: FhirVersion,
        name: &str,
        value: &str,
    ) -> Vec<String> {
        let response = served
            .server
            .get("/Patient")
            .add_query_param(name, value)
            .add_header(
                X_TENANT_ID,
                HeaderValue::from_str(&served.tenant).expect("tenant"),
            )
            .add_header(header::ACCEPT, accept(version))
            .await;
        response.assert_status(StatusCode::OK);
        let bundle: Value = serde_json::from_str(&response.text()).expect("JSON");
        bundle["entry"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e["resource"]["resourceType"] == "Patient")
                    .filter_map(|e| e["resource"]["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Where `_text` or `_content` is listed on Patient, it is evaluated: a
    /// Patient whose narrative and name hold "Quillfeather" is found by that
    /// word, and not by one it does not hold.
    async fn full_text_is_evaluated(served: &Served, version: FhirVersion, listed: &Listed) {
        let full_text: Vec<&str> = ["_text", "_content"]
            .into_iter()
            .filter(|name| names(listed, "Patient").contains(*name))
            .collect();
        if full_text.is_empty() {
            return;
        }
        let response = served
            .server
            .post("/Patient")
            .add_header(
                X_TENANT_ID,
                HeaderValue::from_str(&served.tenant).expect("tenant"),
            )
            .add_header(header::ACCEPT, accept(version))
            .text(
                serde_json::json!({
                    "resourceType": "Patient",
                    "text": {
                        "status": "generated",
                        "div": "<div xmlns=\"http://www.w3.org/1999/xhtml\">Quillfeather</div>"
                    },
                    "name": [{ "family": "Quillfeather" }]
                })
                .to_string(),
            )
            .content_type(&format!(
                "application/fhir+json; fhirVersion={}",
                version.as_mime_param()
            ))
            .await;
        response.assert_status(StatusCode::CREATED);
        let created: Value = serde_json::from_str(&response.text()).expect("JSON");
        let id = created["id"].as_str().expect("id").to_string();
        for name in full_text {
            assert_eq!(
                patients_found(served, version, name, "Quillfeather").await,
                std::slice::from_ref(&id),
                "{version:?}: Patient?{name}=Quillfeather"
            );
            assert!(
                patients_found(served, version, name, "Zanzibarite")
                    .await
                    .is_empty(),
                "{version:?}: Patient?{name}=Zanzibarite"
            );
        }
    }

    /// The check: every listed parameter is served, and `absent` is not
    /// listed, `(type, name)`, for each enabled version. `present` must be
    /// listed. `server_for(version)` is a fresh server over the backend,
    /// configured for that version.
    pub async fn every_listed_param_is_served<F, Fut>(
        server_for: F,
        absent: &[(&str, &str)],
        present: &[(&str, &str)],
    ) where
        F: Fn(FhirVersion) -> Fut,
        Fut: std::future::Future<Output = Option<Served>>,
    {
        for version in FhirVersion::enabled_versions() {
            let Some(served) = server_for(*version).await else {
                eprintln!("skipped: backend unavailable");
                return;
            };
            let listed = listed(&served.server, &served.tenant, *version).await;
            let refused = refused(&served, *version, &listed).await;
            assert!(
                refused.is_empty(),
                "{version:?}: {} listed parameter(s) refused:\n{}",
                refused.len(),
                refused.join("\n")
            );
            for (resource_type, name) in absent {
                if listed.contains_key(*resource_type) {
                    assert!(
                        !names(&listed, resource_type).contains(*name),
                        "{version:?}: {resource_type}.{name} is listed"
                    );
                }
            }
            for (resource_type, name) in present {
                assert!(
                    names(&listed, resource_type).contains(*name),
                    "{version:?}: {resource_type}.{name} is not listed"
                );
            }
            full_text_is_evaluated(&served, *version, &listed).await;
        }
    }
}

/// What SQLite, PostgreSQL and Elasticsearch leave out: the `special`
/// parameters they build no condition for (`Location.contains` and
/// `Composition.section-text` exist from R5 only, and are checked where the
/// version lists the type), `Composition.section-code-text`, whose
/// `section-text` component is one of them, and the two composites whose
/// component definitions neither the R5 nor the R6 bundle publishes (#2774
/// lists the gaps of each version; the check above covers them all, and
/// these are the ones that hold in every version that has them).
#[cfg(any(feature = "sqlite", feature = "postgres"))]
const SQL_ABSENT: &[(&str, &str)] = &[
    ("Location", "near"),
    ("Location", "contains"),
    ("Composition", "section-text"),
    ("Composition", "section-code-text"),
    ("Ingredient", "strength-concentration-ratio"),
    ("Ingredient", "strength-presentation-ratio"),
];

/// What they keep: the full-text parameters they evaluate, `_list`, which
/// REST resolves into an `_id` filter, and a composite whose components are
/// all served.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
const SQL_PRESENT: &[(&str, &str)] = &[
    ("Patient", "_text"),
    ("Patient", "_content"),
    ("Patient", "_list"),
    ("Location", "_text"),
    ("Location", "address"),
    ("Observation", "code-value-quantity"),
];

#[cfg(feature = "sqlite")]
mod sqlite {
    use std::sync::Arc;

    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};

    use crate::contract;

    fn server(version: FhirVersion) -> contract::Served {
        let backend = SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(contract::data_dir()),
                fhir_version: version,
                ..Default::default()
            },
        )
        .expect("in-memory SQLite");
        backend.init_schema().expect("init schema");
        let backend = Arc::new(backend);
        let tenant = "test-tenant".to_string();
        let state = helios_rest::AppState::new(backend.clone(), contract::config(&tenant));
        let routes = helios_rest::routing::fhir_routes::create_routes(state);
        contract::Served {
            server: TestServer::new(routes).expect("test server"),
            tenant,
            storage: backend,
        }
    }

    #[tokio::test]
    async fn every_listed_param_is_served_on_sqlite() {
        contract::every_listed_param_is_served(
            |version| async move { Some(server(version)) },
            crate::SQL_ABSENT,
            crate::SQL_PRESENT,
        )
        .await;
    }
}

#[cfg(feature = "mongodb")]
mod mongodb {
    use std::sync::Arc;

    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::Backend;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    use crate::contract;

    struct SharedMongo {
        url: String,
        _container: Option<testcontainers::ContainerAsync<Mongo>>,
    }

    static SHARED_MONGO: OnceCell<Option<SharedMongo>> = OnceCell::const_new();

    /// `HFS_TEST_MONGODB_URL` when set, else a container, as the other MongoDB
    /// REST suites do; `None` when neither is available.
    async fn shared_mongo() -> Option<&'static SharedMongo> {
        SHARED_MONGO
            .get_or_init(|| async {
                if let Ok(url) = std::env::var("HFS_TEST_MONGODB_URL") {
                    if !url.trim().is_empty() {
                        return Some(SharedMongo {
                            url,
                            _container: None,
                        });
                    }
                }
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = crate::container_cleanup::with_cleanup_label(
                    Mongo::default()
                        .with_tag("9.0.8")
                        .with_env_var("MONGO_TCMALLOC_PER_CPU_CACHE_SIZE_BYTES", "10485760")
                        .with_label("github.run_id", &run_id)
                        .with_startup_timeout(std::time::Duration::from_secs(120)),
                )
                .start()
                .await
                .ok()?;
                let port = container.get_host_port_ipv4(27017).await.ok()?;
                let host = container.get_host().await.ok()?.to_string();
                Some(SharedMongo {
                    url: format!("mongodb://{host}:{port}"),
                    _container: Some(container),
                })
            })
            .await
            .as_ref()
    }

    async fn server(version: FhirVersion) -> Option<contract::Served> {
        let mongo = shared_mongo().await?;
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: mongo.url.clone(),
            database_name: format!("rest_served_params_{}", uuid::Uuid::new_v4().simple()),
            data_dir: Some(contract::data_dir()),
            fhir_version: version,
            ..Default::default()
        })
        .expect("MongoBackend");
        backend.initialize().await.expect("initialize MongoDB");
        let backend = Arc::new(backend);
        let tenant = "test-tenant".to_string();
        let state = helios_rest::AppState::new(backend.clone(), contract::config(&tenant));
        let routes = helios_rest::routing::fhir_routes::create_routes(state);
        Some(contract::Served {
            server: TestServer::new(routes).expect("test server"),
            tenant,
            storage: backend,
        })
    }

    /// MongoDB refuses every `special` parameter it is given, `_text` and
    /// `_content` with `501`, so the statement lists none of them, nor
    /// `Composition.section-code-text` (R5 and later), which it refuses for
    /// its `special` component. `_list`, which REST resolves into an `_id`
    /// filter, stays.
    #[tokio::test]
    async fn every_listed_param_is_served_on_mongodb() {
        contract::every_listed_param_is_served(
            server,
            &[
                ("Patient", "_text"),
                ("Patient", "_content"),
                ("Location", "_text"),
                ("Location", "_content"),
                ("Location", "near"),
                ("Location", "contains"),
                ("Composition", "section-text"),
                ("Composition", "section-code-text"),
                ("Ingredient", "strength-concentration-ratio"),
                ("Ingredient", "strength-presentation-ratio"),
            ],
            &[
                ("Patient", "_list"),
                ("Patient", "name"),
                ("Location", "address"),
                ("Observation", "code-value-quantity"),
            ],
        )
        .await;
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use std::sync::Arc;

    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::OnceCell;

    use crate::contract;

    struct SharedPg {
        host: String,
        port: u16,
        password: String,
        _container: Option<testcontainers::ContainerAsync<Postgres>>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

    /// `HFS_PG_HOST` (with `HFS_PG_PORT`, `HFS_PG_PASSWORD`) when set, else a
    /// PostgreSQL 16 container, as the other PostgreSQL REST suites use.
    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                if let Ok(host) = std::env::var("HFS_PG_HOST") {
                    return SharedPg {
                        host,
                        port: std::env::var("HFS_PG_PORT")
                            .ok()
                            .and_then(|p| p.parse().ok())
                            .unwrap_or(5432),
                        password: std::env::var("HFS_PG_PASSWORD")
                            .unwrap_or_else(|_| "postgres".to_string()),
                        _container: None,
                    };
                }
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
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
                SharedPg {
                    host,
                    port,
                    password: "postgres".to_string(),
                    _container: Some(container),
                }
            })
            .await
    }

    async fn server(version: FhirVersion) -> Option<contract::Served> {
        let pg = shared_pg().await;
        let backend = PostgresBackend::new(PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some(pg.password.clone()),
            max_connections: 8,
            data_dir: Some(contract::data_dir()),
            fhir_version: version,
            ..Default::default()
        })
        .await
        .expect("PostgresBackend");
        backend.init_schema().await.expect("init schema");
        let backend = Arc::new(backend);
        let tenant = format!("served_params_{}", uuid::Uuid::new_v4().simple());
        let state = helios_rest::AppState::new(backend.clone(), contract::config(&tenant));
        let routes = helios_rest::routing::fhir_routes::create_routes(state);
        Some(contract::Served {
            server: TestServer::new(routes).expect("test server"),
            tenant,
            storage: backend,
        })
    }

    #[tokio::test]
    async fn every_listed_param_is_served_on_postgres() {
        contract::every_listed_param_is_served(server, crate::SQL_ABSENT, crate::SQL_PRESENT).await;
    }
}

#[cfg(all(feature = "elasticsearch", feature = "sqlite"))]
mod elasticsearch {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::elasticsearch::{
        ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
    };
    use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
    use helios_persistence::composite::{
        CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
    };
    use helios_persistence::core::{Backend, BackendKind};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::elastic_search::ElasticSearch;
    use tokio::sync::OnceCell;

    use crate::contract;

    struct SharedEs {
        url: String,
        _container: Option<testcontainers::ContainerAsync<ElasticSearch>>,
    }

    static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

    /// `HFS_TEST_ES_URL` when set, else the 7.17 container the other
    /// Elasticsearch suites use.
    async fn shared_es() -> &'static SharedEs {
        SHARED_ES
            .get_or_init(|| async {
                if let Ok(url) = std::env::var("HFS_TEST_ES_URL") {
                    if !url.trim().is_empty() {
                        return SharedEs {
                            url,
                            _container: None,
                        };
                    }
                }
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = crate::container_cleanup::with_cleanup_label(
                    ElasticSearch::default()
                        .with_tag("7.17.29")
                        .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                        .with_label("github.run_id", &run_id)
                        .with_startup_timeout(std::time::Duration::from_secs(300)),
                )
                .start()
                .await
                .expect("failed to start Elasticsearch container");
                let port = container.get_host_port_ipv4(9200).await.expect("port");
                let host = container.get_host().await.expect("host");
                SharedEs {
                    url: format!("http://{host}:{port}"),
                    _container: Some(container),
                }
            })
            .await
    }

    /// SQLite primary with its search offloaded to Elasticsearch, as
    /// `start_sqlite_elasticsearch` wires it: every search, and so the
    /// statement's `serves_search_param`, is Elasticsearch's.
    async fn server(version: FhirVersion) -> Option<contract::Served> {
        let es = shared_es().await;
        let mut sqlite = SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(contract::data_dir()),
                fhir_version: version,
                ..Default::default()
            },
        )
        .expect("in-memory SQLite");
        sqlite.init_schema().expect("init schema");
        sqlite.set_search_offloaded(true);
        let sqlite = Arc::new(sqlite);
        let elastic = ElasticsearchBackend::with_shared_registry(
            ElasticsearchConfig {
                nodes: vec![es.url.clone()],
                index_prefix: format!("served_{}", uuid::Uuid::new_v4().simple()),
                number_of_replicas: 0,
                refresh_interval: "1ms".to_string(),
                write_refresh: WriteRefreshPolicy::WaitFor,
                fhir_version: version,
                ..Default::default()
            },
            sqlite.tenant_registries().clone(),
        )
        .expect("ElasticsearchBackend");
        elastic
            .initialize()
            .await
            .expect("initialize Elasticsearch");
        let elastic = Arc::new(elastic);

        let config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Synchronous)
            .build()
            .expect("composite config");
        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
        backends.insert("es".to_string(), elastic.clone() as DynStorage);
        let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
        providers.insert("sqlite".to_string(), sqlite.clone() as DynSearchProvider);
        providers.insert("es".to_string(), elastic as DynSearchProvider);
        let composite = CompositeStorage::new(config, backends)
            .expect("composite storage")
            .with_search_providers(providers)
            .with_full_primary(sqlite);

        let composite = Arc::new(composite);
        let tenant = "test-tenant".to_string();
        let state = helios_rest::AppState::new(composite.clone(), contract::config(&tenant));
        let routes = helios_rest::routing::fhir_routes::create_routes(state);
        Some(contract::Served {
            server: TestServer::new(routes).expect("test server"),
            tenant,
            storage: composite,
        })
    }

    #[tokio::test]
    async fn every_listed_param_is_served_on_elasticsearch() {
        contract::every_listed_param_is_served(server, crate::SQL_ABSENT, crate::SQL_PRESENT).await;
    }
}
