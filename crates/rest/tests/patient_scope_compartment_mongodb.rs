//! A `patient/` scope is confined to the launch patient's compartment (#1618)
//! — MongoDB.
//!
//! Runs the scenarios in `common/patient_scope.rs` (see
//! `patient_scope_compartment.rs` for SQLite) against a MongoDB container.
//! Bulk export jobs live in an in-memory SQLite store, as `hfs` wires its
//! embedded job-store sidecar for MongoDB. Each scenario gets its own
//! database, and builds its indexes inline, before it writes
//! (`index_build_mode`). A failing scenario prints the server's own log (see
//! `common/patient_scope.rs`); `writes_under_load` is the `--ignored`
//! reproduction harness for #2045.
//!
//! The container is a single-node replica set, as in the persistence suite's
//! `mongodb_tests.rs`: a standalone mongod has no oplog, so `hfs` answers
//! every transaction Bundle with 501 there, and the transaction scenario would
//! never reach the compartment checks.
//!
//! Requires Docker; honours `HFS_TEST_MONGODB_URL` (which must then be a
//! replica set too), and skips (reported as skipped, not passing) when neither
//! is available — matching every other MongoDB suite in this workspace.

#![cfg(all(feature = "mongodb", feature = "sqlite"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/patient_scope.rs"]
mod patient_scope;

use std::sync::Arc;

use axum_test::TestServer;
use helios_persistence::backends::mongodb::{IndexBuildMode, MongoBackend, MongoBackendConfig};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::core::{Backend, BulkExportJobStore};
use helios_rest::ServerConfig;
use helios_rest::bulk_export_auth::BearerScopeAuth;
use std::time::{Duration, Instant};

use testcontainers::ImageExt;
use testcontainers::core::ExecCommand;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;
use tokio::sync::OnceCell;

struct SharedMongo {
    connection_string: String,
    /// `None` when `HFS_TEST_MONGODB_URL` pointed us at an instance we do
    /// not own, so there is nothing to tear down.
    _container: Option<testcontainers::ContainerAsync<Mongo>>,
}

static SHARED_MONGO: OnceCell<Option<SharedMongo>> = OnceCell::const_new();

/// Polls until the one-member replica set has elected itself primary, so no
/// test races a connection against a node still in `STARTUP2`/`SECONDARY`.
async fn wait_for_writable_primary(container: &testcontainers::ContainerAsync<Mongo>) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(mut exec) = container
            .exec(ExecCommand::new([
                "mongosh",
                "--quiet",
                "--eval",
                "db.hello().isWritablePrimary",
            ]))
            .await
            && let Ok(stdout) = exec.stdout_to_vec().await
            && String::from_utf8_lossy(&stdout).trim() == "true"
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "replica set rs0 did not report a writable primary within 60s"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// A single-node replica set, started and initiated as
/// `mongodb_tests.rs`'s `shared_mongo` does it.
async fn shared_mongo() -> Option<&'static SharedMongo> {
    SHARED_MONGO
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("HFS_TEST_MONGODB_URL") {
                if !url.trim().is_empty() {
                    return Some(SharedMongo {
                        connection_string: url,
                        _container: None,
                    });
                }
            }
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = container_cleanup::with_cleanup_label(
                Mongo::default()
                    .with_tag("7.0")
                    .with_label("github.run_id", &run_id)
                    .with_cmd([
                        "mongod",
                        "--bind_ip_all",
                        "--wiredTigerCacheSizeGB",
                        "0.25",
                        "--replSet",
                        "rs0",
                        "--oplogSize",
                        "128",
                        // mongod's 5ms transaction lock wait turned index
                        // build overlap into `LockTimeout` 500s on the seed
                        // writes. Raised, and kept although each scenario now
                        // builds its indexes inline, before it writes (see
                        // `index_build_mode`); not re-measured without it.
                        "--setParameter",
                        "maxTransactionLockRequestTimeoutMillis=5000",
                    ])
                    // Each scenario's database holds WiredTiger handles for
                    // every collection and index: the suite peaks around 930
                    // open files, and the CI Docker host allows 1024. Past
                    // that mongod panics and every request fails with 500
                    // (as in `mongodb_tests.rs` and #1944).
                    .with_ulimit("nofile", 64000, Some(64000))
                    .with_startup_timeout(Duration::from_secs(120)),
            )
            .start()
            .await
            .ok()?;
            // The member advertises its in-container address; the client
            // connects with `directConnection=true` through the mapped port.
            let mut initiate = container
                .exec(ExecCommand::new([
                    "mongosh",
                    "--quiet",
                    "--eval",
                    "rs.initiate({_id:\"rs0\",members:[{_id:0,host:\"localhost:27017\"}]})",
                ]))
                .await
                .expect("rs.initiate on the Mongo container");
            let _ = initiate.stdout_to_vec().await;
            wait_for_writable_primary(&container).await;
            let port = container.get_host_port_ipv4(27017).await.ok()?;
            let host = container.get_host().await.ok()?.to_string();
            Some(SharedMongo {
                connection_string: format!("mongodb://{host}:{port}/?directConnection=true"),
                _container: Some(container),
            })
        })
        .await
        .as_ref()
}

/// How each scenario's backend builds its post-boot indexes: inline, so the
/// builds are done before the scenario writes.
///
/// In the product's default `background` mode the scenario's own builds overlap
/// its writes, and mongod aborts some write transactions with a labelled
/// `TransientTransactionError`: `SnapshotUnavailable` (246) on mongo 5.0.6,
/// `LockTimeout` (24) or a catalog-change `WriteConflict` (112) on 7.0. The
/// product replays all of them: #1997 added the replay for labelled aborts
/// other than 112, and #2231 (commit 42eb4a616) added it for the
/// catalog-change 112.
///
/// That overlap is what caused the 500s of #2045. Run on the parent of #1997's
/// merge (d0af69a4c) with mongo 5.0.6, as CI was then, the background-mode
/// suite failed `writes_stay_inside_the_compartment` in 1 of 10 runs, and
/// `writes_under_load` (14 x 5) in 18 of 70 scenarios. Every one was a 246 at
/// "Failed to delete search index entries" of an update or delete, which that
/// code answered with a 500. In `inline` mode the same code failed 0 of 10
/// runs and 0 of 70 scenarios: `inline` hides that bug, it does not fix it.
/// Current code on 5.0.6 in background mode replays the 246 and passed 210 of
/// 210 scenarios.
///
/// Writes during background builds stay covered by
/// `crates/persistence/tests/mongodb/write_abort_retry.rs`, whose failpoints
/// abort the search-index delete of an update and of a delete with a labelled
/// 246, and by `writes_under_load` with `HFS_MONGODB_INDEX_BUILD=background`.
/// `HFS_MONGODB_INDEX_BUILD` (`background`, `inline`, `off`; the product's own
/// variable) overrides this default.
fn index_build_mode() -> IndexBuildMode {
    match std::env::var("HFS_MONGODB_INDEX_BUILD") {
        Ok(_) => IndexBuildMode::from_env().unwrap_or_else(|message| panic!("{message}")),
        Err(_) => IndexBuildMode::Inline,
    }
}

async fn create_test_server(_name: &str) -> Option<TestServer> {
    let mongo = shared_mongo().await?;
    let backend = MongoBackend::new(MongoBackendConfig {
        connection_string: mongo.connection_string.clone(),
        // MongoDB caps database names at 64 bytes.
        database_name: format!("pscope_{}", uuid::Uuid::new_v4().simple()),
        data_dir: Some(patient_scope::data_dir()),
        index_build: index_build_mode(),
        ..Default::default()
    })
    .expect("failed to create MongoBackend");
    backend
        .initialize()
        .await
        .expect("failed to initialize MongoDB schema");
    let backend = Arc::new(backend);

    // `data_dir` so the job store loads the spec SearchParameters instead of
    // warning about a missing `./data` at the head of every captured log.
    let jobs = Arc::new(
        SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(patient_scope::data_dir()),
                ..Default::default()
            },
        )
        .expect("in-memory sqlite job store"),
    );
    jobs.init_schema().expect("init job store schema");
    let state = helios_rest::AppState::new(backend, ServerConfig::for_testing()).with_bulk_export(
        jobs as Arc<dyn BulkExportJobStore>,
        patient_scope::export_output(),
        Arc::new(BearerScopeAuth),
    );
    Some(patient_scope::serve(
        helios_rest::routing::fhir_routes::create_routes(state),
    ))
}

patient_scope_suite!(create_test_server);

// --- Reproduction harness for #2045 -------------------------------------------

/// What one `writes_under_load` scenario did.
struct StressRun {
    thread: usize,
    round: usize,
    /// The panic message of a failed assertion, `None` when the scenario passed.
    failure: Option<String>,
    /// Whether the scenario ran at all (`false` without Docker).
    ran: bool,
    /// Everything the server logged during the scenario.
    log: String,
}

fn env_usize(name: &str, default: usize) -> usize {
    match std::env::var(name) {
        Ok(raw) => raw
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{name} must be a whole number, got {raw:?}")),
        Err(_) => default,
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "(panic payload is not a string)".to_string()
    }
}

/// Whether a failed scenario's panic message is one of the two 503s the
/// MongoDB backend answers with "retry the request" (`replay_aborted_writes` in
/// crates/persistence/src/backends/mongodb/storage.rs): the write was aborted
/// on every attempt until its replays ran out, or rolled back for eviction
/// (#1895). Neither is the 500 of #2045.
fn is_documented_retry_503(failure: &str) -> bool {
    failure.contains("503 Service Unavailable")
        && failure.contains(r#""code":"transient""#)
        && ((failure.contains("was aborted by the database")
            && failure.contains("times; retry the request"))
            || failure.contains("under cache pressure; retry the request"))
}

/// One stress thread: `rounds` fresh scenarios, each on its own
/// `current_thread` runtime as `#[tokio::test]` gives it, so the thread-local
/// server-log capture sees the handlers, the driver and the index builders.
fn stress_thread(thread: usize, rounds: usize) -> Vec<StressRun> {
    (0..rounds)
        .map(|round| {
            let mut log = patient_scope::ServerLog::capture("writes_under_load");
            log.disarm();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("scenario runtime");
            // Setup is inside `catch_unwind` too: `create_test_server` panics
            // when the backend cannot be created or initialised (inline builds
            // turn a failed index build into an error there), and a panic
            // that escaped `block_on` would throw away every thread's results.
            let outcome = runtime.block_on(futures::FutureExt::catch_unwind(
                std::panic::AssertUnwindSafe(async {
                    let Some(server) = create_test_server("writes_under_load").await else {
                        return false;
                    };
                    patient_scope::seed(&server).await;
                    patient_scope::writes_stay_inside_the_compartment(&server).await;
                    true
                }),
            ));
            // Drops the scenario's background tasks (the index builders) too,
            // so whatever they log is part of the capture.
            drop(runtime);
            StressRun {
                thread,
                round,
                ran: !matches!(outcome, Ok(false)),
                failure: match &outcome {
                    Err(payload) => Some(panic_message(payload.as_ref())),
                    Ok(_) => None,
                },
                log: log.text(),
            }
        })
        .collect()
}

/// Counts the server-log events that matter to #2045, keyed by what happened
/// (and to which write), the step that failed, and the MongoDB error code with
/// the collection, wording and labels of the driver error. Returns the table,
/// one line per key.
fn tally_server_events(runs: &[StressRun]) -> String {
    use std::collections::BTreeMap;

    // (log line contains, event). Every replay is a transient abort that the
    // pre-#1997 code answered with a 500 when it came from the search-index
    // step of an update or delete. The needles must track the log wording in
    // `crates/persistence/src/backends/mongodb/storage.rs`
    // (`replay_aborted_writes`, `soft_delete`) and `crates/rest/src/error.rs`
    // (`client_response`): a reworded message makes its row silently disappear.
    // `the_tally_tells_a_replay_from_a_give_up_and_an_eviction` pins the first
    // three against copies of those lines: the eviction line's "not running it
    // again" must not match the replay needle.
    // The index builders are not in this table: their lines are told apart by
    // their target (`search_index_builder`, `missing_index_builder`) and level
    // below, so a build that failed is not counted as one that ran.
    const EVENTS: [(&str, &str); 6] = [
        ("transaction; running it again", "replayed"),
        ("times; giving up", "gave up (503)"),
        ("back for eviction", "eviction (503)"),
        ("internal error while processing request", "500"),
        ("a concurrent writer won", "lost race"),
        ("retrying once", "delete retried"),
    ];
    let code = regex::Regex::new(r"Error code (\d+) \((\w+)\)").expect("code regex");
    let step = regex::Regex::new(r"internal error in mongodb: ([^:]+):").expect("step regex");
    // The replay WARN names the write ("create", "update", "delete", ...).
    let write = regex::Regex::new(r"the (\w+) transaction").expect("write regex");

    // What else a driver error line says: the collection a write conflict
    // names, the catalog-change wording of a conflict the server raised
    // because an index build changed the collection under the transaction,
    // and the error labels that decide whether the write is run again.
    let collection = regex::Regex::new(r"collection '[^']*\.(\w+)'").expect("collection regex");
    let labels = regex::Regex::new(r"labels: \{([^}]*)\}").expect("labels regex");

    let mut table: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for line in runs.iter().flat_map(|run| run.log.lines()) {
        let event = if line.contains("index_builder") {
            // The tracing fmt level column: a builder logs INFO as it runs
            // and WARN or ERROR for a failed build or index check. Before
            // #2268 it also logged the #1391 upgrade notice on these fresh
            // databases in background mode; that notice now needs rows that
            // were there at boot.
            if line.contains(" WARN ") || line.contains("ERROR ") {
                "index build warning/error"
            } else {
                "index build"
            }
        } else if let Some((_, event)) = EVENTS.iter().find(|(needle, _)| line.contains(needle)) {
            *event
        } else {
            continue;
        };
        let write = write.captures(line).map(|c| c[1].to_string());
        let event = match &write {
            Some(write) => format!("{event} [{write}]"),
            None => event.to_string(),
        };
        let step = step
            .captures(line)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| "-".to_string());
        let mut code_column = code
            .captures(line)
            .map(|c| format!("{} ({})", &c[1], &c[2]))
            .unwrap_or_else(|| "-".to_string());
        if let Some(c) = collection.captures(line) {
            code_column.push_str(&format!(", on {}", &c[1]));
        }
        if line.contains("due to catalog changes") {
            code_column.push_str(", catalog change");
        }
        if let Some(c) = labels.captures(line) {
            code_column.push_str(&format!(", labels {}", &c[1]));
        }
        *table.entry((event, step, code_column)).or_default() += 1;
    }
    if table.is_empty() {
        return "  (none)".to_string();
    }
    table
        .iter()
        .map(|((event, step, code), n)| format!("  {n:>5}  {event:<24}  {step}  |  {code}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Reproduction harness for #2045: `writes_stay_inside_the_compartment` got a
/// 500 on a plain PUT or DELETE in about one CI run in five. The 500s happened
/// in CI on 2026-10-13/14, on mongo 5.0.6 before #1997. This harness
/// reproduces them on that code (see [`index_build_mode`]): a labelled
/// `SnapshotUnavailable` (246) at "Failed to delete search index entries",
/// which #1997 replays. On 7.0 most measured replays are the catalog-change
/// 112, a 7.0 behaviour replayed since #2231.
///
/// Runs the scenario (seed, then the writes) on `HFS_PSCOPE_STRESS_THREADS`
/// threads (default 14, one per scenario of the suite), `HFS_PSCOPE_STRESS_ROUNDS`
/// times each (default 5), every run on its own database of the one shared
/// `mongod` so the load is the suite's own, only denser. It reports how many
/// scenarios failed and a table of the replays, lost races and 500s the server
/// logged, keyed by step and MongoDB error code, then fails when a scenario
/// failed with anything other than a documented retry 503 (a 500 above all).
/// Those 503s are the backend's answer when its replays run out or on an
/// eviction rollback; they are counted in the summary and printed in full, but
/// do not fail the run unless `HFS_PSCOPE_STRESS_STRICT=1`. A failing
/// scenario's panic message and server log are printed in full;
/// `HFS_TEST_SERVER_LOG_ALWAYS=1` prints every scenario's log.
///
/// Compare `HFS_MONGODB_INDEX_BUILD=background` (the product's default, and
/// what the suite ran with until #2045) with `inline` (the suite's default):
/// aborts that only `background` shows come from the scenario's own index
/// builds overlapping its writes. Background mode under heavy load can still
/// end in a 503 when the replays run out; at 32 threads this has hit seed
/// creates.
///
/// Run it alone, under the machine's MongoDB-suite lock:
/// `cargo test -p helios-rest --features mongodb --test
/// patient_scope_compartment_mongodb writes_under_load -- --ignored --nocapture`
#[test]
#[ignore = "reproduction harness for #2045; run with --ignored, alone"]
fn writes_under_load() {
    let threads = env_usize("HFS_PSCOPE_STRESS_THREADS", 14);
    let rounds = env_usize("HFS_PSCOPE_STRESS_ROUNDS", 5);

    // Starts the shared container; kept alive to the end of the run.
    let container_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    if container_runtime.block_on(shared_mongo()).is_none() {
        eprintln!("Skipping writes_under_load (requires Docker)");
        return;
    }

    let started = Instant::now();
    let runs: Vec<StressRun> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|thread| scope.spawn(move || stress_thread(thread, rounds)))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .expect("stress thread panicked outside a scenario")
            })
            .collect()
    });
    let elapsed = started.elapsed();

    let ran = runs.iter().filter(|run| run.ran).count();
    let failed = runs.iter().filter(|run| run.failure.is_some()).count();
    let retry_503 = runs
        .iter()
        .filter(|run| run.failure.as_deref().is_some_and(is_documented_retry_503))
        .count();
    let other = failed - retry_503;
    let strict = std::env::var("HFS_PSCOPE_STRESS_STRICT").as_deref() == Ok("1");
    let rate = if ran == 0 {
        0.0
    } else {
        100.0 * failed as f64 / ran as f64
    };
    let summary = format!(
        "writes_under_load: {ran} of {} scenarios ran on {threads} threads x {rounds} rounds in {:.1}s; \
         {failed} failed ({rate:.1}%): {other} with anything other than a documented retry 503, \
         {retry_503} with a documented retry 503; HFS_MONGODB_INDEX_BUILD in effect: {:?}\n\
         server events (count, event, step, error code):\n{}",
        runs.len(),
        elapsed.as_secs_f64(),
        index_build_mode(),
        tally_server_events(&runs),
    );
    eprintln!("{summary}");

    let always = std::env::var("HFS_TEST_SERVER_LOG_ALWAYS").as_deref() == Ok("1");
    for run in &runs {
        if let Some(failure) = &run.failure {
            let note = if is_documented_retry_503(failure) {
                " (documented retry 503)"
            } else {
                ""
            };
            eprintln!(
                "==== FAILED thread {} round {}{note}: {failure}\n==== server log ====\n{}\n==== end of server log ====",
                run.thread,
                run.round,
                run.log.trim_end()
            );
        } else if always {
            eprintln!(
                "==== thread {} round {} passed; server log ====\n{}\n==== end of server log ====",
                run.thread,
                run.round,
                run.log.trim_end()
            );
        }
    }
    drop(container_runtime);
    if strict {
        assert_eq!(failed, 0, "{summary}");
    } else {
        assert_eq!(other, 0, "{summary}");
    }
}

/// `writes_under_load` tolerates the backend's two documented "retry the
/// request" 503s and nothing else: a 500, or a 503 of another kind, still
/// fails it.
#[test]
fn only_the_documented_retry_503s_are_tolerated() {
    let exhausted = r#"seed /Patient/p1: 503 Service Unavailable {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"transient","details":{"text":"The create was aborted by the database 4 times; retry the request"}}]}"#;
    assert!(is_documented_retry_503(exhausted));

    let eviction = concat!(
        "assertion `left == right` failed: ",
        r#"{"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"transient","details":{"text":"The update was rolled back by the database under cache pressure; retry the request"}}]}"#,
        "\n  left: 503 Service Unavailable\n right: 200 OK",
    );
    assert!(is_documented_retry_503(eviction));

    let issue_2045 = concat!(
        "assertion `left == right` failed: ",
        r#"{"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"exception","details":{"text":"An internal error occurred while processing the request."}}]}"#,
        "\n  left: 500 Internal Server Error\n right: 200 OK",
    );
    assert!(!is_documented_retry_503(issue_2045));

    let wrong_code = r#"seed /Patient/p1: 503 Service Unavailable {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"exception","details":{"text":"The create was aborted by the database 4 times; retry the request"}}]}"#;
    assert!(!is_documented_retry_503(wrong_code));

    let no_server = r#"seed /Patient/p1: 503 Service Unavailable {"resourceType":"OperationOutcome","issue":[{"severity":"error","code":"transient","details":{"text":"The create found no usable database server; the database is temporarily unavailable; retry the request"}}]}"#;
    assert!(!is_documented_retry_503(no_server));
}

/// The tally's needles match the WARN lines `replay_aborted_writes` logs: a
/// replay and a write that gave up are counted as such, and an eviction
/// rollback's "not running it again" is not counted as a replay.
#[test]
fn the_tally_tells_a_replay_from_a_give_up_and_an_eviction() {
    let log = [
        "WARN mongodb aborted the create transaction; running it again: Error code 112 (WriteConflict)",
        "WARN mongodb aborted the create transaction 4 times; giving up: Error code 112 (WriteConflict)",
        "WARN mongodb rolled the update transaction back for eviction under cache pressure; not running it again: Error code 112 (WriteConflict)",
    ]
    .join("\n");
    let table = tally_server_events(&[StressRun {
        thread: 0,
        round: 0,
        failure: None,
        ran: true,
        log,
    }]);
    assert_eq!(table.lines().count(), 3, "{table}");
    for event in [
        "replayed [create]",
        "gave up (503) [create]",
        "eviction (503) [update]",
    ] {
        assert!(table.contains(event), "{event}: {table}");
    }
}
