//! #1907: `GET /metrics` exposes the PostgreSQL connection pool's state as
//! four gauges labelled only by `service` and `pool`:
//! `postgres_pool_max_connections`, `postgres_pool_connections`,
//! `postgres_pool_available_connections` and `postgres_pool_waiting_callers`.
//!
//! A one-connection pool is built against a PostgreSQL container and the
//! real `/metrics` router is scraped while the test checks the connection
//! out and while a second caller waits for it, so the gauges must move with
//! the pool rather than report a value fixed at startup.
//!
//! The global Prometheus recorder is process-wide, so this binary holds a
//! single test.
//!
//! Requires Docker (testcontainers spins up a real PostgreSQL instance).

#![cfg(feature = "postgres")]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

mod postgres_pool_metrics_tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum_test::TestServer;
    use helios_observability::pool_metrics::PoolGauges;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    const MAX: &str = "postgres_pool_max_connections";
    const SIZE: &str = "postgres_pool_connections";
    const AVAILABLE: &str = "postgres_pool_available_connections";
    const WAITING: &str = "postgres_pool_waiting_callers";
    const POOL: &str = "primary";

    /// Bounded polling for the waiting caller, kept well below the default
    /// 10 s `pool_wait_timeout_secs` so the waiter is still parked when seen.
    const WAIT_DEADLINE: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(10);

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"))
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
    fn gauge(body: &str, metric: &str) -> Option<f64> {
        body.lines().filter_map(parse_line).find_map(|sample| {
            (sample.name == metric && sample.labels.contains(&("pool", POOL)))
                .then(|| sample.value.parse().expect("numeric gauge value"))
        })
    }

    fn assert_gauge(body: &str, metric: &str, expected: f64, step: &str) {
        assert_eq!(
            gauge(body, metric),
            Some(expected),
            "{step}: expected {metric}{{pool=\"{POOL}\"}} = {expected}; scraped body:\n{body}"
        );
    }

    #[tokio::test]
    async fn pool_gauges_move_on_checkout_and_wait() {
        helios_observability::metrics::init("hfs");

        let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
        // Pinned to 16: the backend sends `plan_cache_mode` as a startup
        // option, which the module's default postgres:11 rejects.
        let container = super::container_cleanup::with_cleanup_label(
            Postgres::default()
                .with_tag("16-alpine")
                .with_label("github.run_id", &run_id),
        )
        .start()
        .await
        .expect("failed to start PostgreSQL container");
        let config = PostgresConfig {
            host: container.get_host().await.expect("host").to_string(),
            port: container.get_host_port_ipv4(5432).await.expect("port"),
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 1,
            data_dir: Some(data_dir()),
            ..Default::default()
        };
        let backend = Arc::new(
            PostgresBackend::new(config)
                .await
                .expect("connect to postgres"),
        );

        // Registered the way `hfs` registers its primary pool.
        helios_observability::pool_metrics::register_postgres_pool(POOL, {
            let backend = Arc::clone(&backend);
            move || {
                let status = backend.pool_status();
                PoolGauges {
                    max_size: status.max_size,
                    size: status.size,
                    available: status.available,
                    waiting: status.waiting,
                }
            }
        });

        let server = TestServer::new(helios_observability::metrics::router()).expect("test server");

        // Idle: the connectivity check in `PostgresBackend::new` leaves one
        // open, idle connection.
        let body = scrape(&server).await;
        assert!(
            body.contains(&format!("{AVAILABLE}{{")) && gauge(&body, AVAILABLE).is_some(),
            "idle: /metrics has no {AVAILABLE} series with pool=\"{POOL}\"; scraped body:\n{body}"
        );
        assert_gauge(&body, AVAILABLE, 1.0, "idle");
        assert_gauge(&body, SIZE, 1.0, "idle");
        assert_gauge(&body, WAITING, 0.0, "idle");
        assert_gauge(&body, MAX, 1.0, "idle");

        // The pool family carries only the `service` and `pool` labels: no
        // tenant, resource-type or database-name label.
        for Sample { name, labels, .. } in body.lines().filter_map(parse_line) {
            if name.starts_with("postgres_pool_") {
                let keys: BTreeSet<&str> = labels.iter().map(|(k, _)| *k).collect();
                assert_eq!(
                    keys,
                    BTreeSet::from(["pool", "service"]),
                    "unexpected labels on {name}; scraped body:\n{body}"
                );
            }
        }

        // Checkout: the only connection is in use.
        let holder = backend.get_client().await.expect("checkout");
        let body = scrape(&server).await;
        assert_gauge(&body, AVAILABLE, 0.0, "checkout");
        assert_gauge(&body, SIZE, 1.0, "checkout");
        assert_gauge(&body, WAITING, 0.0, "checkout");

        // Wait: a second caller parks on the exhausted pool.
        let waiter = tokio::spawn({
            let backend = Arc::clone(&backend);
            async move { backend.get_client().await.map(drop) }
        });
        let deadline = Instant::now() + WAIT_DEADLINE;
        let body = loop {
            let body = scrape(&server).await;
            if gauge(&body, WAITING) == Some(1.0) || Instant::now() >= deadline {
                break body;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        };
        assert_gauge(&body, WAITING, 1.0, "wait");

        // Release: the waiter gets the connection, then returns it.
        drop(holder);
        waiter
            .await
            .expect("waiter task")
            .expect("waiter checks out the released connection");
        let body = scrape(&server).await;
        assert_gauge(&body, AVAILABLE, 1.0, "release");
        assert_gauge(&body, WAITING, 0.0, "release");
    }
}
