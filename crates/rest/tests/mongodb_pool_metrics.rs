//! #2284: a `?maxPoolSize=` in the MongoDB connection URI is honoured, and
//! `GET /metrics` exposes the pool as `mongodb_pool_*` gauges plus the
//! `mongodb_pool_checkout_wait_seconds` histogram, all labelled only by
//! `service` and `pool`.
//!
//! The backend is built from a URI carrying `maxPoolSize=3` and no
//! `HFS_MONGODB_MAX_CONNECTIONS`, registered the way `hfs` registers its
//! primary pool, and the real `/metrics` router is scraped. The max gauge
//! reading 3 shows the URI option reached the driver end to end.
//!
//! The global Prometheus recorder is process-wide, so this binary holds a
//! single test.
//!
//! Requires Docker (testcontainers spins up a real MongoDB instance), or an
//! instance named by `HFS_TEST_MONGODB_URL`.

#![cfg(feature = "mongodb")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod mongodb_pool_metrics_tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum_test::TestServer;
    use helios_fhir::FhirVersion;
    use helios_observability::pool_metrics::MongoPoolGauges;
    use helios_persistence::backends::mongodb::{MongoBackend, MongoBackendConfig};
    use helios_persistence::core::{Backend, ResourceStorage};
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
    use serde_json::json;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::mongo::Mongo;

    const MAX: &str = "mongodb_pool_max_connections";
    const CONNECTIONS: &str = "mongodb_pool_connections";
    const IN_USE: &str = "mongodb_pool_in_use_connections";
    const IDLE: &str = "mongodb_pool_idle_connections";
    const WAITING: &str = "mongodb_pool_waiting_callers";
    const WAIT_COUNT: &str = "mongodb_pool_checkout_wait_seconds_count";
    const POOL: &str = "primary";

    /// Bounded polling for the in-use gauge to settle after the last request.
    const SETTLE_DEADLINE: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(20);

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"))
    }

    fn base_with_pool(base: &str, size: u32) -> String {
        if base.contains('?') {
            format!("{base}&maxPoolSize={size}")
        } else {
            format!("{}/?maxPoolSize={size}", base.trim_end_matches('/'))
        }
    }

    async fn scrape(server: &TestServer) -> String {
        let response = server.get("/metrics").await;
        response.assert_status_ok();
        response.text()
    }

    /// One exposition sample: `name{k="v",...} value`.
    struct Sample<'a> {
        name: &'a str,
        labels: Vec<(&'a str, &'a str)>,
        value: &'a str,
    }

    /// Splits one exposition line into its name, labels and value. Comment
    /// and blank lines yield `None`.
    fn parse_line(line: &str) -> Option<Sample<'_>> {
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (series, value) = line.rsplit_once(' ')?;
        let (name, labels) = match series.split_once('{') {
            Some((name, rest)) => {
                let body = rest.strip_suffix('}')?;
                let labels = body
                    .split(',')
                    .filter(|pair| !pair.is_empty())
                    .filter_map(|pair| {
                        let (k, v) = pair.split_once('=')?;
                        Some((k, v.trim_matches('"')))
                    })
                    .collect();
                (name, labels)
            }
            None => (series, Vec::new()),
        };
        Some(Sample {
            name,
            labels,
            value,
        })
    }

    /// The value of `metric` for `pool="primary"`, if the series is present.
    fn value(body: &str, metric: &str) -> Option<f64> {
        body.lines().filter_map(parse_line).find_map(|sample| {
            (sample.name == metric && sample.labels.contains(&("pool", POOL)))
                .then(|| sample.value.parse().expect("numeric sample value"))
        })
    }

    #[tokio::test]
    async fn uri_pool_size_is_honoured_and_pool_metrics_are_exported() {
        helios_observability::metrics::init("hfs");

        // Honour an instance the caller already has, like the other MongoDB suites.
        let external = std::env::var("HFS_TEST_MONGODB_URL")
            .ok()
            .filter(|url| !url.trim().is_empty());
        let mut _container = None;
        let base = match external {
            Some(url) => url,
            None => {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                let container = super::container_cleanup::with_cleanup_label(
                    Mongo::default()
                        .with_tag("7.0")
                        .with_label("github.run_id", &run_id)
                        .with_startup_timeout(Duration::from_secs(120)),
                )
                .start()
                .await
                .expect("failed to start MongoDB container");
                let host = container.get_host().await.expect("host").to_string();
                let port = container.get_host_port_ipv4(27017).await.expect("port");
                _container = Some(container);
                format!("mongodb://{host}:{port}")
            }
        };
        let connection_string = base_with_pool(&base, 3);

        // `max_connections` stays `None`: only the URI sets the pool size.
        let backend = Arc::new(
            MongoBackend::new(MongoBackendConfig {
                connection_string,
                database_name: format!("pool_metrics_{}", uuid::Uuid::new_v4().simple()),
                data_dir: Some(data_dir()),
                ..Default::default()
            })
            .expect("MongoBackend"),
        );

        // Registered the way `hfs` registers its primary pool.
        backend.pool_stats().set_label(POOL);
        helios_observability::pool_metrics::register_mongodb_pool(POOL, {
            let backend = Arc::clone(&backend);
            move || {
                let snapshot = backend.pool_stats().snapshot();
                MongoPoolGauges {
                    max_size: snapshot.max_size,
                    connections: snapshot.connections,
                    in_use: snapshot.in_use,
                    idle: snapshot.idle,
                    waiting: snapshot.waiters,
                }
            }
        });
        backend.initialize().await.expect("schema");

        let tenant = TenantContext::new(
            TenantId::new("pool-metrics"),
            TenantPermissions::full_access(),
        );
        for i in 0..5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "id": format!("p{i}")}),
                    FhirVersion::default(),
                )
                .await
                .expect("create patient");
        }

        let server = TestServer::new(helios_observability::metrics::router()).expect("test server");

        // Every request has returned, so once the driver has processed the
        // last check-in nothing is in use and nobody waits.
        let started = Instant::now();
        let body = loop {
            let body = scrape(&server).await;
            if value(&body, IN_USE) == Some(0.0) || started.elapsed() > SETTLE_DEADLINE {
                break body;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        };

        assert_eq!(
            value(&body, MAX),
            Some(3.0),
            "?maxPoolSize=3 in the URI must reach the driver; scraped body:\n{body}"
        );
        assert_eq!(value(&body, IN_USE), Some(0.0), "in use: {body}");
        assert_eq!(value(&body, WAITING), Some(0.0), "waiting: {body}");
        let connections = value(&body, CONNECTIONS).expect("connections series");
        assert!(
            (1.0..=3.0).contains(&connections),
            "open connections must be within the pool: {body}"
        );
        assert_eq!(
            value(&body, IDLE),
            Some(connections),
            "with nothing in use every open connection is idle: {body}"
        );
        let waits = value(&body, WAIT_COUNT).expect("checkout wait histogram series");
        assert!(waits > 0.0, "checkouts were timed: {body}");
        assert!(
            body.contains("mongodb_pool_checkout_wait_seconds_bucket{"),
            "the checkout wait is a histogram, not a summary: {body}"
        );

        // Only `service` and `pool` label these series; a tenant or resource
        // label would let the metric's cardinality follow the data.
        let label_keys: BTreeSet<&str> = body
            .lines()
            .filter_map(parse_line)
            .filter(|sample| sample.name.starts_with("mongodb_pool_"))
            .flat_map(|sample| sample.labels.into_iter().map(|(k, _)| k))
            .filter(|key| *key != "le")
            .collect();
        assert_eq!(
            label_keys,
            BTreeSet::from(["pool", "service"]),
            "unexpected labels on mongodb_pool_* series: {body}"
        );

        // Callers queue when the pool is smaller than the concurrency: a
        // second backend with `maxPoolSize=1` and 12 concurrent writers
        // must show waiters while they run and none once they finish.
        let single = Arc::new(
            MongoBackend::new(MongoBackendConfig {
                connection_string: base_with_pool(&base, 1),
                database_name: format!("pool_waiters_{}", uuid::Uuid::new_v4().simple()),
                data_dir: Some(data_dir()),
                ..Default::default()
            })
            .expect("MongoBackend"),
        );
        single.initialize().await.expect("schema");
        let writers: Vec<_> = (0..12)
            .map(|i| {
                let backend = Arc::clone(&single);
                let tenant = tenant.clone();
                tokio::spawn(async move {
                    backend
                        .create(
                            &tenant,
                            "Patient",
                            json!({"resourceType": "Patient", "id": format!("w{i}")}),
                            FhirVersion::default(),
                        )
                        .await
                        .expect("create patient")
                })
            })
            .collect();
        let mut max_waiters = 0;
        while !writers.iter().all(|w| w.is_finished()) {
            max_waiters = max_waiters.max(single.pool_stats().snapshot().waiters);
            tokio::task::yield_now().await;
        }
        for writer in writers {
            writer.await.expect("writer task");
        }
        assert!(
            max_waiters > 0,
            "12 concurrent writers on a 1-connection pool must queue"
        );
        let started = Instant::now();
        while single.pool_stats().snapshot().waiters != 0 && started.elapsed() < SETTLE_DEADLINE {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        let snapshot = single.pool_stats().snapshot();
        assert_eq!(snapshot.max_size, 1);
        assert_eq!(snapshot.waiters, 0, "queue drains once the writers finish");
    }
}
