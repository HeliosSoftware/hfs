//! T2 cluster test for the HTS bootstrap advisory lock (D1).
//!
//! Proves `schema::with_bootstrap_lock` — the fix for N cold-starting HTS
//! instances racing the same heavy terminology import — actually serializes
//! concurrent callers against a shared PostgreSQL database, using two
//! independently constructed `PostgresTerminologyBackend` handles (never a
//! cloned `Arc`), per `docs/cluster-testing-strategy.md`'s T2 contract.
//!
//! Run with:
//!   `cargo test -p helios-hts --features postgres --test postgres_bootstrap_lock`

#![cfg(feature = "postgres")]

use helios_hts::backends::PostgresTerminologyBackend;
use helios_hts::backends::postgres::schema::with_bootstrap_lock;
use std::sync::Arc;
use std::sync::OnceLock;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::{Barrier, OnceCell};

const LABEL_KEY: &str = "io.helios.hts.test-pool";
const LABEL_VALUE: &str = "hts-bootstrap-lock-pg";

static CONTAINER: OnceLock<ContainerAsync<Postgres>> = OnceLock::new();
static DB_URL: OnceCell<String> = OnceCell::const_new();

/// Force-remove the testcontainer at process exit (see
/// `postgres_integration_tests.rs` for why: `static` values are never
/// dropped, so this synchronous `docker rm -f` by label is the backstop).
#[ctor::dtor]
fn cleanup_container() {
    let filter = format!("label={LABEL_KEY}={LABEL_VALUE}");
    let Ok(listing) = std::process::Command::new("docker")
        .args(["ps", "-aq", "--filter", &filter])
        .output()
    else {
        return;
    };
    let ids = String::from_utf8_lossy(&listing.stdout);
    for id in ids.split_whitespace() {
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", id])
            .output();
    }
}

async fn db_url() -> &'static str {
    DB_URL
        .get_or_init(|| async {
            use testcontainers::{ImageExt, runners::AsyncRunner};
            let container = Postgres::default()
                .with_label(LABEL_KEY, LABEL_VALUE)
                .start()
                .await
                .expect("Failed to start Postgres container");
            let host = container.get_host().await.expect("get host");
            let port = container.get_host_port_ipv4(5432).await.expect("get port");
            let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
            let _ = CONTAINER.set(container);
            url
        })
        .await
}

async fn fresh_backend() -> PostgresTerminologyBackend {
    PostgresTerminologyBackend::new(db_url().await)
        .await
        .expect("Backend should initialize")
}

/// Exclusivity: two independently constructed handles race
/// `with_bootstrap_lock` around a classic read-sleep-write critical section.
/// Without serialization both tasks would read the same starting value and
/// the second write would clobber the first (a lost update, final value 1
/// instead of 2) — the artificial delay between the read and the write
/// widens the race window enough that an unserialized run reliably loses an
/// update, while a serialized run never does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_integration_cluster_bootstrap_lock_serializes_racing_callers() {
    let a = fresh_backend().await;
    let b = fresh_backend().await;

    // Test-local counter row, independent of any HTS schema table.
    {
        let client = a.pool().get().await.expect("get client");
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS bootstrap_lock_race (id INT PRIMARY KEY, value INT); \
                 DELETE FROM bootstrap_lock_race; \
                 INSERT INTO bootstrap_lock_race (id, value) VALUES (1, 0);",
            )
            .await
            .expect("seed race table");
    }

    let barrier = Arc::new(Barrier::new(2));

    let race = |backend: PostgresTerminologyBackend, barrier: Arc<Barrier>| {
        tokio::spawn(async move {
            barrier.wait().await;
            with_bootstrap_lock(backend.pool(), || async {
                let client = backend.pool().get().await?;
                let row = client
                    .query_one("SELECT value FROM bootstrap_lock_race WHERE id = 1", &[])
                    .await?;
                let value: i32 = row.get(0);

                // Widen the race window: without the advisory lock, both
                // tasks would be sitting on this read at the same time.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                client
                    .execute(
                        "UPDATE bootstrap_lock_race SET value = $1 WHERE id = 1",
                        &[&(value + 1)],
                    )
                    .await?;
                Ok(())
            })
            .await
        })
    };

    let ta = race(a, barrier.clone());
    let tb = race(b, barrier);

    ta.await.expect("task a join").expect("task a work");
    tb.await.expect("task b join").expect("task b work");

    let verify = fresh_backend().await;
    let client = verify.pool().get().await.expect("get client");
    let row = client
        .query_one("SELECT value FROM bootstrap_lock_race WHERE id = 1", &[])
        .await
        .expect("read final value");
    let value: i32 = row.get(0);
    assert_eq!(
        value, 2,
        "both racing critical sections should have run to completion serialized \
         (no lost update); a value of 1 means the advisory lock did not serialize them"
    );
}
