//! `_since` boundary table for the SoF runners (#1864, acceptance A18).
//!
//! `_since` is inclusive: a resource whose stored instant equals the cutoff is
//! kept, one a tick earlier than the cutoff is dropped. A cutoff finer than the
//! backend stores is rounded **up** to the backend's tick, so a cutoff a
//! fraction of a tick after a resource's stored instant drops that resource
//! (it was stored before the cutoff). One table per runner:
//!
//! | module             | stored instant          | tick  |
//! |--------------------|-------------------------|-------|
//! | `sqlite`           | RFC 3339 text, ns       | 1 ns  |
//! | `postgres`         | `timestamptz`, µs       | 1 µs  |
//! | `mongodb`          | BSON date, ms           | 1 ms  |
//! | `mongodb_fallback` | JSON `meta.lastUpdated` | 1 ms  |
//!
//! `mongodb_fallback` is the in-process runner MongoDB hands `patient`/`group`
//! runs to; it compares the JSON stamp, which has the same ms precision as the
//! stored date. The S3 runner's table is in `src/backends/s3/tests.rs`
//! (`since_boundary_table_compares_the_stored_instant`, MockS3Client).
//!
//! Run with:
//!   cargo test -p helios-persistence --features postgres,mongodb --test sof_since_precision
//!
//! The PostgreSQL and MongoDB modules need Docker (testcontainers). Set
//! `HFS_TEST_REQUIRE_MONGODB=1` to make a missing MongoDB container a failure.

#[cfg(any(feature = "postgres", feature = "mongodb"))]
#[path = "common/container_cleanup.rs"]
mod container_cleanup;

/// The table shared by every runner module.
mod table {
    #![allow(dead_code)]

    use chrono::{DateTime, Duration, Utc};

    /// One row: a cutoff relative to the stored instant `s`, and whether the
    /// resource must be in the result.
    pub struct Row {
        pub name: &'static str,
        pub cutoff: DateTime<Utc>,
        pub kept: bool,
    }

    /// The rows for a backend whose stored instants have precision `tick`.
    /// `sub_tick` is `None` when no finer cutoff exists (a 1 ns tick).
    pub fn rows(s: DateTime<Utc>, tick: Duration, sub_tick: Option<Duration>) -> Vec<Row> {
        let mut rows = vec![
            Row {
                name: "cutoff equal to the stored instant: kept",
                cutoff: s,
                kept: true,
            },
            Row {
                name: "cutoff one tick after: dropped",
                cutoff: s + tick,
                kept: false,
            },
            Row {
                name: "cutoff one tick before: kept",
                cutoff: s - tick,
                kept: true,
            },
            Row {
                name: "cutoff in the future: empty",
                cutoff: s + Duration::days(1),
                kept: false,
            },
        ];
        if let Some(sub) = sub_tick {
            rows.push(Row {
                name: "cutoff a fraction of a tick after (rounds up): dropped",
                cutoff: s + sub,
                kept: false,
            });
        }
        rows
    }

    /// Checks every row with `kept_at(cutoff)` and fails once, listing every
    /// row that disagrees.
    pub async fn check<F, Fut>(backend: &str, s: DateTime<Utc>, rows: Vec<Row>, mut kept_at: F)
    where
        F: FnMut(DateTime<Utc>) -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let mut wrong = Vec::new();
        for row in &rows {
            let kept = kept_at(row.cutoff).await;
            eprintln!(
                "SOF-SINCE {backend}: stored {} cutoff {} -> kept={kept} (expected {}) [{}]",
                s.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
                row.cutoff
                    .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
                row.kept,
                row.name
            );
            if kept != row.kept {
                wrong.push(format!(
                    "{}: cutoff {} kept={kept}, expected {}",
                    row.name,
                    row.cutoff
                        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
                    row.kept
                ));
            }
        }
        assert!(
            wrong.is_empty(),
            "{backend} `_since` boundary table, stored instant {}:\n  {}",
            s.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            wrong.join("\n  ")
        );
    }
}

/// Runs `view` with `filters` and reports whether a row with `id == want` came
/// back.
#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
async fn returns_id(
    runner: &dyn helios_persistence::core::sof_runner::SofRunner,
    tenant: &helios_persistence::tenant::TenantContext,
    filters: helios_persistence::core::sof_runner::ViewFilters,
    want: &str,
) -> bool {
    use futures::StreamExt;
    use helios_persistence::core::sof_runner::{RunContext, RunKind, RunRequest};
    let view = serde_json::json!({
        "resourceType": "ViewDefinition",
        "resource": "Observation",
        "status": "active",
        "select": [{"column": [{"name": "id", "path": "id"}]}]
    });
    let mut stream = runner
        .run_view(
            tenant,
            RunRequest::new(view, filters),
            RunContext::detached(RunKind::Preview),
        )
        .await
        .map(|output| output.rows)
        .expect("run_view");
    let mut found = false;
    while let Some(row) = stream.next().await {
        let row = row.expect("row");
        found |= row["id"].as_str() == Some(want);
    }
    found
}

#[cfg(any(feature = "sqlite", feature = "postgres", feature = "mongodb"))]
fn observation(id: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Observation",
        "id": id,
        "status": "final",
        "code": {"text": "since"},
        "subject": {"reference": "Patient/p1"}
    })
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use chrono::Duration;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::ViewFilters;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use super::table;

    #[tokio::test]
    async fn since_boundary_table() {
        let backend =
            SqliteBackend::with_config(":memory:", Default::default()).expect("SQLite backend");
        backend.init_schema().expect("init schema");
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        backend
            .create(
                &tenant,
                "Observation",
                super::observation("o1"),
                FhirVersion::R4,
            )
            .await
            .expect("seed");
        let s = backend
            .read(&tenant, "Observation", "o1")
            .await
            .expect("read")
            .expect("stored")
            .last_modified();
        let runner = backend.sof_runner().expect("runner");
        let rows = table::rows(s, Duration::nanoseconds(1), None);
        table::check("sqlite", s, rows, |cutoff| {
            let runner = runner.clone();
            let tenant = tenant.clone();
            async move {
                super::returns_id(
                    runner.as_ref(),
                    &tenant,
                    ViewFilters {
                        since: Some(cutoff),
                        ..Default::default()
                    },
                    "o1",
                )
                .await
            }
        })
        .await;
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use chrono::Duration;
    use helios_fhir::FhirVersion;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::ViewFilters;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::table;

    #[tokio::test]
    async fn since_boundary_table() {
        let container = super::container_cleanup::with_cleanup_label(
            Postgres::default().with_tag("16-alpine").with_label(
                "github.run_id",
                std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
            ),
        )
        .start()
        .await
        .expect("start PostgreSQL");
        let backend = PostgresBackend::new(PostgresConfig {
            host: container.get_host().await.unwrap().to_string(),
            port: container.get_host_port_ipv4(5432).await.unwrap(),
            dbname: "postgres".into(),
            user: "postgres".into(),
            password: Some("postgres".into()),
            max_connections: 3,
            data_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        })
        .await
        .expect("create backend");
        backend.init_schema().await.expect("initialize schema");
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        backend
            .create(
                &tenant,
                "Observation",
                super::observation("o1"),
                FhirVersion::R4,
            )
            .await
            .expect("seed");
        let s = backend
            .read(&tenant, "Observation", "o1")
            .await
            .expect("read")
            .expect("stored")
            .last_modified();
        assert_eq!(
            s.timestamp_subsec_nanos() % 1_000,
            0,
            "PostgreSQL stores microseconds"
        );
        let runner = backend.sof_runner().expect("runner");
        let rows = table::rows(
            s,
            Duration::microseconds(1),
            Some(Duration::nanoseconds(500)),
        );
        table::check("postgres", s, rows, |cutoff| {
            let runner = runner.clone();
            let tenant = tenant.clone();
            async move {
                super::returns_id(
                    runner.as_ref(),
                    &tenant,
                    ViewFilters {
                        since: Some(cutoff),
                        ..Default::default()
                    },
                    "o1",
                )
                .await
            }
        })
        .await;
    }
}

/// A MongoDB backend over a fresh database of a shared container, or `None`
/// when no container could start and `HFS_TEST_REQUIRE_MONGODB` is unset.
#[cfg(feature = "mongodb")]
mod mongo_harness {
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::Backend;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;
    use tokio::sync::OnceCell;

    struct SharedMongo {
        uri: String,
        _container: testcontainers::ContainerAsync<Mongo>,
    }

    static SHARED: OnceCell<Option<SharedMongo>> = OnceCell::const_new();

    pub async fn backend() -> Option<MongoBackend> {
        let shared = SHARED
            .get_or_init(|| async {
                let started =
                    super::container_cleanup::with_cleanup_label(Mongo::default().with_label(
                        "github.run_id",
                        std::env::var("GITHUB_RUN_ID").unwrap_or_default(),
                    ))
                    .start()
                    .await;
                match started {
                    Ok(container) => {
                        let host = container.get_host().await.unwrap().to_string();
                        let port = container.get_host_port_ipv4(27017).await.unwrap();
                        Some(SharedMongo {
                            uri: format!("mongodb://{host}:{port}"),
                            _container: container,
                        })
                    }
                    Err(e)
                        if std::env::var("HFS_TEST_REQUIRE_MONGODB")
                            .is_ok_and(|v| !v.is_empty()) =>
                    {
                        panic!("MongoDB container did not start ({e}); HFS_TEST_REQUIRE_MONGODB")
                    }
                    Err(e) => {
                        eprintln!("skipping MongoDB `_since` tables: no container ({e})");
                        None
                    }
                }
            })
            .await
            .as_ref()?;
        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: shared.uri.clone(),
            database_name: format!("sofsince_{}", uuid::Uuid::new_v4().simple()),
            data_dir: Some(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
            ..Default::default()
        })
        .expect("create MongoBackend");
        backend
            .initialize()
            .await
            .expect("initialize MongoDB schema");
        Some(backend)
    }
}

/// Seeds Patient `p1` and Observation `o1` (subject `p1`) and returns the
/// Observation's stored instant.
#[cfg(feature = "mongodb")]
async fn seed_mongo(
    backend: &helios_persistence::backends::mongodb::MongoBackend,
    tenant: &helios_persistence::tenant::TenantContext,
) -> chrono::DateTime<chrono::Utc> {
    use helios_persistence::core::ResourceStorage;
    backend
        .create(
            tenant,
            "Patient",
            serde_json::json!({"resourceType": "Patient", "id": "p1"}),
            helios_fhir::FhirVersion::R4,
        )
        .await
        .expect("seed patient");
    backend
        .create(
            tenant,
            "Observation",
            observation("o1"),
            helios_fhir::FhirVersion::R4,
        )
        .await
        .expect("seed observation");
    let s = backend
        .read(tenant, "Observation", "o1")
        .await
        .expect("read")
        .expect("stored")
        .last_modified();
    assert_eq!(
        s.timestamp_subsec_nanos() % 1_000_000,
        0,
        "MongoDB stores milliseconds"
    );
    s
}

#[cfg(feature = "mongodb")]
mod mongodb {
    use chrono::Duration;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::ViewFilters;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use super::table;

    /// The native aggregation pipeline: `last_updated` (a BSON date) against
    /// the cutoff.
    #[tokio::test]
    async fn since_boundary_table() {
        let Some(backend) = super::mongo_harness::backend().await else {
            return;
        };
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let s = super::seed_mongo(&backend, &tenant).await;
        let runner = backend.sof_runner().expect("runner");
        let rows = table::rows(
            s,
            Duration::milliseconds(1),
            Some(Duration::microseconds(500)),
        );
        table::check("mongodb", s, rows, |cutoff| {
            let runner = runner.clone();
            let tenant = tenant.clone();
            async move {
                super::returns_id(
                    runner.as_ref(),
                    &tenant,
                    ViewFilters {
                        since: Some(cutoff),
                        ..Default::default()
                    },
                    "o1",
                )
                .await
            }
        })
        .await;
    }
}

#[cfg(feature = "mongodb")]
mod mongodb_fallback {
    use chrono::Duration;
    use helios_persistence::core::ResourceStorage;
    use helios_persistence::core::sof_runner::ViewFilters;
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use super::table;

    /// A `patient` filter sends the run to the in-process fallback, which
    /// compares the JSON `meta.lastUpdated` (ms, like the stored date).
    #[tokio::test]
    async fn since_boundary_table() {
        let Some(backend) = super::mongo_harness::backend().await else {
            return;
        };
        let tenant = TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access());
        let s = super::seed_mongo(&backend, &tenant).await;
        let runner = backend.sof_runner().expect("runner");
        let rows = table::rows(
            s,
            Duration::milliseconds(1),
            Some(Duration::microseconds(500)),
        );
        table::check("mongodb_fallback", s, rows, |cutoff| {
            let runner = runner.clone();
            let tenant = tenant.clone();
            async move {
                super::returns_id(
                    runner.as_ref(),
                    &tenant,
                    ViewFilters {
                        patient: vec!["Patient/p1".to_string()],
                        since: Some(cutoff),
                        ..Default::default()
                    },
                    "o1",
                )
                .await
            }
        })
        .await;
    }
}
