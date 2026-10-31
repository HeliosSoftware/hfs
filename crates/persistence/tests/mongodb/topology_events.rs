//! #2411: the driver's topology (SDAM) events forget a backend's kept
//! transactional `hello` answer.
//!
//! Since #2308 a backend keeps the transactional answer of its first successful
//! `hello` and sends none before later writes. Before #2411 it forgot the answer
//! only when a write reported a problem, so the first write after a replica set
//! came back as a standalone server was refused with `503`. Now the backend's
//! client reports each server-description change, and a server whose type
//! changes to one without transactions, or to `Unknown`, forgets the answer
//! (`backends::mongodb::topology_events`).
//!
//! No synthetic event is possible with the `mongodb` 3.7 API (the event structs
//! are `#[non_exhaustive]` and `ServerInfo` has no public constructor), so the
//! classification and its effect on the memo are unit-tested in the module
//! itself and the wiring is tested here, with real driver events:
//!
//! - `a_dropped_connection_the_driver_reports_as_unknown_forgets_the_kept_answer`
//!   runs on the shared server. One read's connection is dropped, which makes
//!   the driver mark the server `Unknown`; the next create asks `hello`, and the
//!   one after it does not. The backend's client is built by its terminology
//!   store, not by the backend, so the test also covers a collaborator that
//!   initialises the shared client cell.
//! - `a_replica_set_restarted_as_a_standalone_server_takes_its_next_write_without_a_503`
//!   runs on a mongod of its own, started as a one-node replica set and
//!   restarted in place without `--replSet`: the issue's scenario.
//!
//! Child module of the `mongodb_tests` root: `use super::*;` reaches its
//! harness. As in `single_write_topology.rs`, the backends keep the driver's
//! server monitor quiet (`serverMonitoringMode=poll`,
//! `heartbeatFrequencyMS=600000`), so the `hello` commands counted by a
//! `failCommand` that blocks for 0 ms are the writes' own. The driver still
//! checks the server at once after a failed operation; it is that check, and the
//! failed operation itself, that report the change.

use super::*;

use std::time::{Duration, Instant};

use testcontainers::ImageExt;
use testcontainers::core::{CmdWaitFor, ExecCommand};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

use crate::bulk_submit::FailPoint;

/// The quiet monitor of the module doc.
const QUIET: &str = "retryWrites=false&serverMonitoringMode=poll&heartbeatFrequencyMS=600000";

fn observation(id: &str, status: &str) -> serde_json::Value {
    json!({
        "resourceType": "Observation",
        "id": id,
        "status": status,
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
    })
}

/// Whether the server behind `backend` is a replica set or a mongos, asked
/// directly (not through the backend, so no failpoint applies).
async fn deployment_is_transactional(backend: &MongoBackend) -> bool {
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("a raw client for the topology question");
    let hello = client
        .database("admin")
        .run_command(doc! { "hello": 1_i32 })
        .await
        .expect("hello");
    hello.contains_key("setName") || hello.get_str("msg").is_ok_and(|msg| msg == "isdbgrid")
}

/// A `failCommand` that matches every `hello` of the scoped `appName` and
/// changes nothing: it blocks the command for 0 ms and fails nothing, so the
/// failpoint's `count` is the number of `hello` commands the backend sent.
fn counting_hello() -> Document {
    doc! { "failCommands": ["hello"], "blockConnection": true, "blockTimeMS": 0_i32 }
}

/// A server counter (`count` of `configureFailPoint`) as a number: the server
/// sends an `Int32` or an `Int64`.
fn counter(value: Option<&Bson>) -> i64 {
    match value {
        Some(Bson::Int32(count)) => i64::from(*count),
        Some(Bson::Int64(count)) => *count,
        other => panic!("the server returned an invalid counter: {other:?}"),
    }
}

/// Runs `work` while a [`counting_hello`] failpoint is on for `app` on the
/// shared server, and returns its result with the number of `hello` commands
/// `app` sent meanwhile. `None`, after printing why, when failpoints are not
/// available.
async fn hellos_during<T>(
    app: &str,
    work: impl std::future::Future<Output = T>,
) -> Option<(T, i64)> {
    let fail_point =
        FailPoint::enable(app, counting_hello(), doc! { "times": 1_000_000_i32 }).await?;
    let output = work.await;
    let hellos = fail_point.off_and_count().await;
    Some((output, hellos))
}

/// [`hellos_during`] on a server of the test's own, whose `failCommand` is not
/// shared with the rest of the suite, so there is no lock to hold.
async fn hellos_on<T>(
    admin: &mongodb::Database,
    app: &str,
    work: impl std::future::Future<Output = T>,
) -> (T, i64) {
    let mut data = counting_hello();
    data.insert("appName", app);
    let on = admin
        .run_command(doc! {
            "configureFailPoint": "failCommand",
            "mode": { "times": 1_000_000_i32 },
            "data": data,
        })
        .await
        .expect("configureFailPoint failCommand on");
    let output = work.await;
    let off = admin
        .run_command(doc! { "configureFailPoint": "failCommand", "mode": "off" })
        .await
        .expect("configureFailPoint failCommand off");
    (output, counter(off.get("count")) - counter(on.get("count")))
}

/// Waits, up to `seconds`, until a read through the backend's own client
/// succeeds (it reads the seeded `Observation/obs-seed`), so that the write
/// under test does not depend on how soon the driver rediscovers the server.
/// Panics when the server never comes back.
async fn wait_until_usable(backend: &MongoBackend, tenant: &TenantContext, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if backend
            .read(tenant, "Observation", "obs-seed")
            .await
            .is_ok()
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the backend's client could not read again within {seconds} s"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn resource_rows(backend: &MongoBackend, id: &str) -> u64 {
    count_docs(backend, "resources", doc! { "id": id }).await
}

async fn history_rows(backend: &MongoBackend, id: &str) -> u64 {
    count_docs(backend, "resource_history", doc! { "id": id }).await
}

/// A backend on the shared server whose client is built by its terminology
/// store: the store reads before anything else touches the backend, so the
/// handler the backend registers is the one the store passes to the driver
/// (#2411). `None` when no MongoDB is available.
async fn backend_whose_terminology_store_builds_the_client(
    test: &str,
    app: &str,
) -> Option<MongoBackend> {
    const MAX_ATTEMPTS: u32 = 3;
    let connection_string = shared_mongo::connection_string().await?;
    let config = MongoBackendConfig {
        connection_string: append_query_param(&connection_string, QUIET),
        database_name: build_test_database_name(test),
        app_name: app.to_string(),
        data_dir: Some(repo_data_dir()),
        max_connections: TEST_BACKEND_MAX_POOL,
        index_build: IndexBuildMode::Inline,
        ..Default::default()
    };
    let tenant = create_tenant(test);
    let mut attempt = 1;
    loop {
        let backend = MongoBackend::new(config.clone()).expect("a MongoBackend");
        let store = backend
            .sof_terminology_store()
            .expect("mongodb has a terminology store");
        let failure = match store
            .load(&tenant, "http://example.org/fhir/ValueSet/sdam-2411")
            .await
        {
            Ok(None) => match backend.initialize().await {
                Ok(()) => return Some(backend),
                Err(err) => format!("{err:?}"),
            },
            Ok(Some(_)) => panic!("a database that is new holds no terminology relation"),
            Err(err) => format!("{err:?}"),
        };
        if !msg_is_mongo_unavailable(&failure) {
            panic!("the backend whose terminology store builds the client failed: {failure}");
        }
        if attempt >= MAX_ATTEMPTS {
            skip_or_fail_unavailable(&format!(
                "shared mongo unreachable after {MAX_ATTEMPTS} attempts ({failure})"
            ));
            return None;
        }
        eprintln!("#2411 setup attempt {attempt}/{MAX_ATTEMPTS} failed ({failure}); retrying");
        tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt))).await;
        attempt += 1;
    }
}

/// A connection the server drops makes the driver mark the server `Unknown`
/// (the first step of any outage or restart), and that change forgets the kept
/// answer: the next write asks `hello` again, and the one after it, with an
/// answer kept again, does not.
///
/// The backend's client is built by its terminology store
/// ([`backend_whose_terminology_store_builds_the_client`]), the one path where
/// the handler is not registered by the backend's own `get_client`. A client
/// that registered no handler would send no `hello` before the second create.
#[tokio::test]
async fn a_dropped_connection_the_driver_reports_as_unknown_forgets_the_kept_answer() {
    let app = "sdam-2411-unknown";
    let Some(backend) =
        backend_whose_terminology_store_builds_the_client("sdam_2411_unknown", app).await
    else {
        return;
    };
    if !deployment_is_transactional(&backend).await {
        assert!(
            !transactions_required(),
            "the harness's own MongoDB is a replica set"
        );
        eprintln!("Skipping: this MongoDB is a standalone server");
        return;
    }
    let tenant = create_tenant("sdam_2411_unknown");
    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-seed", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");

    // The seed's answer is kept: a second create sends no `hello`.
    let Some((kept, hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-kept", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    kept.expect("the create with a kept answer");
    eprintln!("#2411 hello before the drop: {hellos}");
    assert_eq!(hellos, 0, "the seed's answer is kept");

    // One read's connection is dropped. The read itself may or may not be
    // retried by the driver; what matters is that the driver marks the server
    // `Unknown` and reports it.
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["find"], "closeConnection": true },
        doc! { "times": 1_i32 },
    )
    .await
    else {
        return;
    };
    let _ = backend.read(&tenant, "Observation", "obs-seed").await;
    assert_eq!(
        fail_point.off_and_count().await,
        1,
        "the read's connection was dropped"
    );
    wait_until_usable(&backend, &tenant, 5).await;

    let Some((created, hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    created.expect("the create after the drop");
    eprintln!("#2411 hello for the create after the drop: {hellos}");
    assert_eq!(
        hellos, 1,
        "the driver reported the server Unknown, so the kept answer was forgotten and the create asks"
    );
    assert_eq!(resource_rows(&backend, "obs-1").await, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 1);

    let Some((created, hellos)) = hellos_during(
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-2", "final"),
            FhirVersion::default(),
        ),
    )
    .await
    else {
        return;
    };
    created.expect("the create after the answer was kept again");
    eprintln!("#2411 hello for the create after that: {hellos}");
    assert_eq!(hellos, 0, "the answer is kept again");
}

/// The issue's scenario: a replica set that comes back as a standalone server.
/// The first write after the restart runs without a transaction instead of
/// being refused (`503`, from the `IllegalOperation` a standalone server answers
/// to a transaction), and it asks `hello` once.
///
/// The test's own mongod runs under `bash`, which keeps the container up while
/// `mongod` is shut down; it is started as a one-node replica set, so the
/// backend keeps the transactional answer, and restarted in the same container
/// (same address, same data) without `--replSet`. The backend's driver monitor
/// is quiet, so the change reaches the driver through the first operations that
/// fail on the connections of the stopped server, as it does through the
/// streaming monitor with the default settings.
#[tokio::test]
async fn a_replica_set_restarted_as_a_standalone_server_takes_its_next_write_without_a_503() {
    let started = super::container_cleanup::with_cleanup_label(
        Mongo::default()
            .with_tag("7.0")
            .with_cmd([
                "bash",
                "-c",
                "mongod --bind_ip_all --replSet rs0 --oplogSize 128 --wiredTigerCacheSizeGB 0.25 \
                 --setParameter enableTestCommands=1 & exec sleep infinity",
            ])
            .with_startup_timeout(Duration::from_secs(120)),
    )
    .start()
    .await;
    let container = match started {
        Ok(container) => container,
        Err(error) => {
            skip_or_fail_unavailable(&format!(
                "the Mongo container for the restart test did not start ({error})"
            ));
            return;
        }
    };
    let (host, port) = match (
        container.get_host().await,
        container.get_host_port_ipv4(27017).await,
    ) {
        (Ok(host), Ok(port)) => (host, port),
        (Err(error), _) | (_, Err(error)) => {
            skip_or_fail_unavailable(&format!(
                "the Mongo container for the restart test has no reachable address ({error})"
            ));
            return;
        }
    };
    super::shared_mongo::initiate_replica_set(&container).await;
    super::shared_mongo::wait_for_writable_primary(&container).await;
    let url = format!("mongodb://{host}:{port}/?directConnection=true");

    let app = "sdam-2411-restart";
    let backend = build_backend(MongoBackendConfig {
        connection_string: append_query_param(&url, QUIET),
        database_name: build_test_database_name("sdam_2411_restart"),
        app_name: app.into(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    })
    .await
    .expect("a backend on the restart test's replica set");
    let tenant = create_tenant("sdam_2411_restart");
    let admin_client = raw_test_client(&url)
        .await
        .expect("a raw client for the replica set");
    let admin = admin_client.database("admin");

    backend
        .create(
            &tenant,
            "Observation",
            observation("obs-seed", "final"),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    let (kept, hellos) = hellos_on(
        &admin,
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-kept", "final"),
            FhirVersion::default(),
        ),
    )
    .await;
    kept.expect("the create with a kept answer");
    eprintln!("#2411 hello before the restart: {hellos}");
    assert_eq!(hellos, 0, "the seed's answer is kept");

    // Stop the replica set. `shutdown` is refused from outside the container
    // ("must run from localhost when running db without auth"), so `mongosh`
    // sends it from inside. The server closes the connection on shutdown, so
    // `mongosh` reports an error, which is ignored.
    let stopped = container
        .exec(
            ExecCommand::new([
                "mongosh",
                "--quiet",
                "--eval",
                "db.getSiblingDB('admin').shutdownServer({force: true})",
            ])
            .with_cmd_ready_condition(CmdWaitFor::exit()),
        )
        .await
        .expect("exec of the shutdown in the Mongo container");
    eprintln!(
        "#2411 shutdown exec exit code: {:?}",
        stopped.exit_code().await
    );
    drop(admin_client);

    // Start mongod again in the same container without `--replSet`. The stopped
    // mongod may still hold the data directory's lock for a moment, so retry
    // until `--fork` reports a running daemon.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let code = match container
            .exec(
                ExecCommand::new([
                    "mongod",
                    "--bind_ip_all",
                    "--wiredTigerCacheSizeGB",
                    "0.25",
                    "--setParameter",
                    "enableTestCommands=1",
                    "--fork",
                    "--logpath",
                    "/tmp/mongod-standalone.log",
                ])
                .with_cmd_ready_condition(CmdWaitFor::exit()),
            )
            .await
        {
            Ok(result) => result.exit_code().await.ok().flatten(),
            Err(error) => {
                eprintln!("#2411 exec of the restart failed: {error}");
                None
            }
        };
        if code == Some(0) {
            break;
        }
        // A failed start is reported with its log.
        if let Ok(mut log) = container
            .exec(
                ExecCommand::new(["tail", "-n", "5", "/tmp/mongod-standalone.log"])
                    .with_cmd_ready_condition(CmdWaitFor::exit()),
            )
            .await
            && let Ok(stdout) = log.stdout_to_vec().await
        {
            eprintln!(
                "#2411 mongod --fork exited with {code:?}; log tail: {}",
                String::from_utf8_lossy(&stdout)
            );
        }
        assert!(
            Instant::now() < deadline,
            "mongod did not restart without --replSet within 30 s (last exit code {code:?})"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // The restarted server answers as a standalone server.
    let probe = raw_test_client(&append_query_param(&url, "serverSelectionTimeoutMS=2000"))
        .await
        .expect("a raw client for the restarted server");
    let admin = probe.database("admin");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match admin.run_command(doc! { "hello": 1_i32 }).await {
            Ok(hello) if !hello.contains_key("setName") => break,
            Ok(hello) => eprintln!("#2411 the restarted server still names a set: {hello}"),
            Err(error) => eprintln!("#2411 hello on the restarted server: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "the restarted server did not answer as a standalone server within 30 s"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // The backend's client reads again: the first operations on the stopped
    // server's connections fail, and the driver reports the change.
    wait_until_usable(&backend, &tenant, 10).await;

    let (created, hellos) = hellos_on(
        &admin,
        app,
        backend.create(
            &tenant,
            "Observation",
            observation("obs-1", "final"),
            FhirVersion::default(),
        ),
    )
    .await;
    eprintln!("#2411 hello for the first create after the restart: {hellos}");
    let created = created.expect(
        "the first write after the restart runs without a transaction \
         (before #2411: Unavailable from the refused transaction)",
    );
    assert_eq!(created.version_id(), "1");
    assert_eq!(
        hellos, 1,
        "the driver reported the change, so the create asks"
    );
    assert_eq!(resource_rows(&backend, "obs-1").await, 1);
    assert_eq!(history_rows(&backend, "obs-1").await, 1);

    drop(probe);
    drop(container);
}
