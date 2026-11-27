//! #2233 Stage 2: the exact total of a metadata `:missing=true` with `:not` /
//! `:missing=false` streams the stamped ids and checks them in membership
//! batches. These cells shrink the batch with the test-only
//! `with_missing_check_batch_rows`, so a few hundred resources cross many
//! batch boundaries, and compare every total with the single aggregation the
//! split replaces (the `ParamSortOverCap` origin still runs it) and with the
//! rows written.

use super::super::super::CountOrigin;
use super::super::with_missing_check_batch_rows;
use super::*;
use crate::error::{BackendError, StorageError};

fn assert_timeout<T>(result: crate::error::StorageResult<T>) {
    assert!(
        matches!(
            result,
            Err(StorageError::Backend(BackendError::Timeout { .. }))
        ),
        "expected a timeout, never a number"
    );
}

const ROWS: usize = 300;

fn missing(name: &str, value: &str) -> SearchParameter {
    let mut param = parameter(name, value);
    param.modifier = Some(SearchModifier::Missing);
    param
}

fn not(name: &str, values: &[&str]) -> SearchParameter {
    let mut param = parameter(name, values[0]);
    param.modifier = Some(SearchModifier::Not);
    param.values = values.iter().map(|value| SearchValue::eq(*value)).collect();
    param
}

/// The stamp state of resource `n`.
#[derive(Clone, Copy, PartialEq)]
enum Stamp {
    /// `missing_params: ["method"]`
    Lacks,
    /// `missing_params: []`: stamped as having `method`.
    Has,
    /// No `missing_params`: its (absent) rows decide.
    Unstamped,
    /// No `missing_params` and the pending marker: counted by nobody.
    Pending,
    /// `missing_params: null`: neither side.
    Null,
}

fn stamp(n: usize) -> Stamp {
    match n % 11 {
        3 => Stamp::Unstamped,
        5 => Stamp::Pending,
        7 => Stamp::Null,
        1 | 9 => Stamp::Has,
        _ => Stamp::Lacks,
    }
}

/// Resources whose `code` row is removed: `code:missing=false` rejects them.
fn without_code(n: usize) -> bool {
    n % 6 == 2
}

struct Shape {
    label: &'static str,
    params: Vec<SearchParameter>,
    keeps: fn(&Row, usize) -> bool,
}

fn shapes() -> Vec<Shape> {
    vec![
        Shape {
            label: "method:missing=true&status:not=final",
            params: vec![missing("method", "true"), not("status", &["final"])],
            keeps: |row, _| row.status != "final",
        },
        Shape {
            label: "status:not=final&method:missing=true",
            params: vec![not("status", &["final"]), missing("method", "true")],
            keeps: |row, _| row.status != "final",
        },
        Shape {
            label: "method:missing=true&code:missing=false",
            params: vec![missing("method", "true"), missing("code", "false")],
            keeps: |_, n| !without_code(n),
        },
        Shape {
            label: "method:missing=true&status:not=final&code:missing=false",
            params: vec![
                missing("method", "true"),
                not("status", &["final"]),
                missing("code", "false"),
            ],
            keeps: |row, n| row.status != "final" && !without_code(n),
        },
        Shape {
            label: "method:missing=true&status:not=final,preliminary",
            params: vec![
                missing("method", "true"),
                not("status", &["final", "preliminary"]),
            ],
            keeps: |_, _| false,
        },
    ]
}

/// Whether resource `n` lacks `method` for today's semantics: stamped with
/// the name, or unstamped, not pending and without a `method` row (no row
/// carries one).
fn lacks_method(n: usize) -> bool {
    matches!(stamp(n), Stamp::Lacks | Stamp::Unstamped)
}

/// Writes the stamp states and removes the `code` rows of [`without_code`].
async fn prepare(fixture: &Fixture) {
    let resources = fixture.db.collection::<Document>("resources");
    let ids = |state: Stamp| -> Vec<String> {
        (0..fixture.rows.len())
            .filter(|n| stamp(*n) == state)
            .map(|n| fixture.rows[n].id.clone())
            .collect()
    };
    for (state, update) in [
        (
            Stamp::Lacks,
            doc! { "$set": { "missing_params": ["method"] } },
        ),
        (Stamp::Has, doc! { "$set": { "missing_params": [] } }),
        (
            Stamp::Null,
            doc! { "$set": { "missing_params": Bson::Null } },
        ),
        (
            Stamp::Pending,
            doc! { "$set": { "search_rows_pending": true } },
        ),
    ] {
        resources
            .update_many(
                doc! { "tenant_id": PLAN_TENANT, "id": { "$in": ids(state) } },
                update,
            )
            .await
            .unwrap();
    }
    let codeless: Vec<String> = (0..fixture.rows.len())
        .filter(|n| without_code(*n))
        .map(|n| fixture.rows[n].id.clone())
        .collect();
    fixture
        .db
        .collection::<Document>("search_index")
        .delete_many(doc! { "tenant_id": PLAN_TENANT, "param_name": "code",
        "resource_id": { "$in": codeless } })
        .await
        .unwrap();
    trust_missing_metadata(&fixture.backend, &fixture.db).await;
}

fn expected(fixture: &Fixture, shape: &Shape) -> u64 {
    (0..fixture.rows.len())
        .filter(|n| {
            let row = &fixture.rows[*n];
            !row.deleted && lacks_method(*n) && (shape.keeps)(row, *n)
        })
        .count() as u64
}

fn query(shape: &Shape) -> SearchQuery {
    let mut query = SearchQuery::new("Observation");
    query.parameters = shape.params.clone();
    query.total = Some(TotalMode::Accurate);
    query
}

/// The stamped stream reads of one total: `find`s on `resources` hinted onto
/// the missing-params index (the freshness probes use the fingerprint index).
async fn stream_finds(db: &mongodb::Database) -> Vec<Document> {
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    collect_documents(
        db.collection::<Document>("system.profile")
            .find(doc! { "op": "query", "command.find": "resources",
            "command.hint": "idx_resources_missing_params" })
            .await
            .unwrap(),
    )
    .await
    .unwrap()
    .into_iter()
    .filter_map(|row| row.get_document("command").ok().cloned())
    .collect()
}

#[tokio::test]
async fn missing_checks_totals_equal_the_single_aggregation_for_any_batch_size() {
    let Some(mut fixture) = Fixture::new("missing-checks-batches", ROWS).await else {
        return;
    };
    fixture.mark_deleted(20, 26).await;
    prepare(&fixture).await;
    for shape in shapes() {
        let query = query(&shape);
        let want = expected(&fixture, &shape);
        let single = fixture
            .backend
            .count_standard(&fixture.tenant, &query, CountOrigin::ParamSortOverCap)
            .await
            .unwrap();
        assert_eq!(single, want, "{}: the single aggregation", shape.label);
        for rows in [1usize, 2, 3, 7, COUNT_CANDIDATE_BATCH_ROWS] {
            restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
            let total = with_missing_check_batch_rows(
                rows,
                fixture.backend.search_count(&fixture.tenant, &query),
            )
            .await
            .unwrap_or_else(|error| panic!("{} batch {rows}: {error:?}", shape.label));
            let finds = stream_finds(&fixture.db).await;
            assert_eq!(total, want, "{} batch {rows}: total", shape.label);
            assert_eq!(
                finds.len(),
                1,
                "{} batch {rows}: one stamped stream: {finds:?}",
                shape.label
            );
            let batch_size = finds[0]
                .get_i32("batchSize")
                .map(i64::from)
                .or_else(|_| finds[0].get_i64("batchSize"));
            assert_eq!(
                batch_size,
                Ok(rows as i64),
                "{} batch {rows}: stream batch",
                shape.label
            );
        }
    }
    fixture.close().await;
}

/// A total whose membership check is blocked past the search deadline
/// (batches of 7 ids) answers the timeout, never a number, only after that
/// check finished, with the admission permit back, and the same total is
/// exact once the failpoint is off. The failpoint would block two checks; a
/// mixed total has one command in flight at a time, so the next check sees
/// the deadline before it sends anything and only one reaches MongoDB.
#[tokio::test]
async fn missing_checks_deadline_answers_after_the_one_blocked_check() {
    let Some(fixture) =
        Fixture::with_search_timeout("missing-checks-deadline", ROWS, Some(750)).await
    else {
        return;
    };
    prepare(&fixture).await;
    let Some(session) = fixture.failpoints("missing-checks-deadline").await else {
        fixture.close().await;
        return;
    };
    let shape = &shapes()[0];
    let armed = session
        .admin
        .run_command(doc! {
            "configureFailPoint": "failCommand", "mode": { "times": 2 },
            "data": { "failCommands": ["aggregate"], "appName": fixture.backend.config().app_name_or_default(),
                "namespace": format!("{}.search_index", fixture.db.name()),
                "blockConnection": true, "blockTimeMS": 1500 },
        })
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let result = with_missing_check_batch_rows(
        7,
        fixture.backend.search_count(&fixture.tenant, &query(shape)),
    )
    .await;
    let elapsed = started.elapsed();
    assert_timeout(result);
    assert!(
        elapsed >= Duration::from_millis(1400),
        "returned after {elapsed:?}, before the blocked checks finished"
    );
    let gate = fixture.backend.broad_search_gate().unwrap();
    assert_eq!(gate.available_permits(), 1, "the permit is back");
    let disabled = disable_failpoint(&session).await;
    assert_eq!(
        failpoint_hits(&disabled) - failpoint_hits(&armed),
        1,
        "only the blocked membership check reached MongoDB: no second one ran beside it"
    );
    let followup = search_deadline::run(
        30_000,
        fixture.backend.search_count(&fixture.tenant, &query(shape)),
    )
    .await
    .unwrap();
    assert_eq!(followup, expected(&fixture, shape));
    fixture.close().await;
}

/// A membership check that fails (the first one) ends the total with that
/// error: the stamped count is never cut short and summed with the
/// unstamped one.
#[tokio::test]
async fn missing_checks_failed_batch_is_an_error_never_a_partial_total() {
    let Some(fixture) = Fixture::new("missing-checks-failed-batch", ROWS).await else {
        return;
    };
    prepare(&fixture).await;
    let Some(session) = fixture.failpoints("missing-checks-failed-batch").await else {
        fixture.close().await;
        return;
    };
    let shape = &shapes()[0];
    let armed = session
        .admin
        .run_command(doc! {
            "configureFailPoint": "failCommand", "mode": { "times": 1 },
            "data": { "failCommands": ["aggregate"], "appName": fixture.backend.config().app_name_or_default(),
                "namespace": format!("{}.search_index", fixture.db.name()),
                "errorCode": 96_i32 },
        })
        .await
        .unwrap();
    let result = with_missing_check_batch_rows(
        7,
        fixture.backend.search_count(&fixture.tenant, &query(shape)),
    )
    .await;
    let disabled = disable_failpoint(&session).await;
    assert_eq!(
        failpoint_hits(&disabled) - failpoint_hits(&armed),
        1,
        "the first membership check failed"
    );
    assert!(
        result.is_err(),
        "a failed batch answered the number {result:?}"
    );
    fixture.close().await;
}

/// The deadline cell over a long stream: 2,000 resources in batches of one
/// id, so the deadline passes with well over a thousand batches still
/// unread. The stream starts none of them beside the blocked check (the
/// failpoint would block a second check and counts one hit), the next batch
/// answers the timeout before it sends anything, and the total returns the
/// timeout only after the blocked check finished.
#[tokio::test]
async fn missing_checks_deadline_on_a_long_stream_sends_no_other_check() {
    const BLOCK_MS: i64 = 3_000;
    let Some(fixture) =
        Fixture::with_search_timeout("missing-checks-in-flight", 2_000, Some(750)).await
    else {
        return;
    };
    prepare(&fixture).await;
    let Some(session) = fixture.failpoints("missing-checks-in-flight").await else {
        fixture.close().await;
        return;
    };
    let shape = &shapes()[0];
    let armed = session
        .admin
        .run_command(doc! {
            "configureFailPoint": "failCommand", "mode": { "times": 2 },
            "data": { "failCommands": ["aggregate"], "appName": fixture.backend.config().app_name_or_default(),
                "namespace": format!("{}.search_index", fixture.db.name()),
                "blockConnection": true, "blockTimeMS": BLOCK_MS },
        })
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let result = with_missing_check_batch_rows(
        1,
        fixture.backend.search_count(&fixture.tenant, &query(shape)),
    )
    .await;
    let elapsed = started.elapsed();
    assert_timeout(result);
    let disabled = disable_failpoint(&session).await;
    assert_eq!(
        failpoint_hits(&disabled) - failpoint_hits(&armed),
        1,
        "one membership check was blocked and no other batch reached MongoDB"
    );
    assert!(
        elapsed >= Duration::from_millis(BLOCK_MS as u64 - 300),
        "returned after {elapsed:?}, while the blocked check was still in flight"
    );
    fixture.close().await;
}

/// [`stream_id_batches`] on its own with the batch consumer's limit of two
/// checks, over 8 ids read 2 per cursor batch, in batches of 2: the first
/// check sleeps and then sets a flag, the second fails at once. The error is
/// returned only after the first check finished.
#[tokio::test]
async fn stream_id_batches_drains_started_checks_on_error() {
    use super::super::{COUNT_BATCH_CONCURRENCY, IdStreamText, stream_id_batches};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let Some(fixture) = Fixture::new("stream-batches-drain", 8).await else {
        return;
    };
    let mut cursor = fixture
        .db
        .collection::<Document>("resources")
        .find(doc! { "tenant_id": PLAN_TENANT })
        .projection(doc! { "_id": 0, "id": 1 })
        .batch_size(2)
        .await
        .unwrap();
    let finished = Arc::new(AtomicBool::new(false));
    let mut started = 0usize;
    let result = stream_id_batches(
        &mut cursor,
        "id",
        &IdStreamText {
            read: "read",
            invalid: "invalid",
            missing_id: "missing id",
        },
        2,
        COUNT_BATCH_CONCURRENCY,
        &mut (),
        |_, _batch: Vec<String>| {
            started += 1;
            let first = started == 1;
            let finished = finished.clone();
            Ok(Box::pin(async move {
                if first {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    finished.store(true, Ordering::SeqCst);
                    Ok(())
                } else {
                    Err(internal_error("injected check failure".to_owned()))
                }
            })
                as std::pin::Pin<
                    Box<dyn Future<Output = StorageResult<()>> + Send>,
                >)
        },
        |_, ()| {},
    )
    .await;
    assert!(result.is_err(), "the failed check is the result");
    assert_eq!(started, 2, "two checks started before the failure");
    assert!(
        finished.load(Ordering::SeqCst),
        "the error was returned while the first check was still running"
    );
    fixture.close().await;
}

/// The database commands one client has in flight, from the driver's command
/// monitoring events: started and not yet succeeded or failed.
#[derive(Default)]
struct CommandsInFlight {
    running: HashMap<i32, String>,
    started: Vec<String>,
    /// `name:collection` of every started command, in order.
    sequence: Vec<String>,
    max: usize,
    /// Every set of command names seen in flight together, sorted.
    overlaps: BTreeSet<String>,
}

impl CommandsInFlight {
    fn handler(
        state: &std::sync::Arc<std::sync::Mutex<Self>>,
    ) -> mongodb::event::EventHandler<mongodb::event::command::CommandEvent> {
        use mongodb::event::command::CommandEvent;
        let state = state.clone();
        mongodb::event::EventHandler::callback(move |event: CommandEvent| {
            let mut state = state.lock().unwrap();
            match event {
                CommandEvent::Started(event) => {
                    let field = match event.command_name.as_str() {
                        "getMore" => "collection",
                        name => name,
                    };
                    let collection = event.command.get_str(field).unwrap_or("").to_owned();
                    state
                        .sequence
                        .push(format!("{}:{collection}", event.command_name));
                    state.started.push(event.command_name.clone());
                    state.running.insert(event.request_id, event.command_name);
                    state.max = state.max.max(state.running.len());
                    if state.running.len() > 1 {
                        let mut names: Vec<_> = state.running.values().cloned().collect();
                        names.sort();
                        let names = names.join("+");
                        state.overlaps.insert(names);
                    }
                }
                CommandEvent::Succeeded(event) => {
                    state.running.remove(&event.request_id);
                }
                CommandEvent::Failed(event) => {
                    state.running.remove(&event.request_id);
                }
                _ => {}
            }
        })
    }
}

/// A Stage 2 mixed total never has two database commands in flight: not two
/// membership checks, not a check while the stamped stream's next `getMore`
/// is pending, not the unstamped count beside the stream. Every command is
/// recorded by the driver's command monitoring on a client of its own, and
/// every `find`, `getMore` and `aggregate` of that client is held 100 ms on
/// the server, so any second command the total starts before the first one
/// answered is in flight beside it. Batches of 32 ids over the stamped
/// stream give several batches, `getMore`s and two membership commands per
/// batch. The order is pinned too: from the stamped stream's `find` on,
/// no two `getMore`s follow each other (each batch is checked before the
/// next one is read), and the unstamped count is the only `aggregate` on
/// `resources` and the last command. The same total then runs on a pool of
/// one connection, bounded so that a pool deadlock fails instead of
/// hanging: that pass proves no hang and the exact total, not the absence
/// of overlap, which a pool of one forbids anyway.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_checks_total_has_one_command_in_flight() {
    let label = "missing-checks-one-command";
    let Some(fixture) = Fixture::new(label, ROWS).await else {
        return;
    };
    prepare(&fixture).await;
    let Some(session) = fixture.failpoints(label).await else {
        fixture.close().await;
        return;
    };
    let uri = shared_mongo::connection_string().await.unwrap();
    let shape = &shapes()[3];
    let query = query(shape);
    let want = expected(&fixture, shape);
    let tenant_id = fixture.tenant.tenant_id().as_str();
    for pool in [None, Some(1u32)] {
        let app_name = format!("hfs-plan-{label}-{}", uuid::Uuid::new_v4().simple());
        let state = std::sync::Arc::new(std::sync::Mutex::new(CommandsInFlight::default()));
        let mut options = mongodb::options::ClientOptions::parse(&uri).await.unwrap();
        options.app_name = Some(app_name.clone());
        options.max_pool_size = pool;
        options.command_event_handler = Some(CommandsInFlight::handler(&state));
        let client = mongodb::Client::with_options(options).unwrap();
        let db = client.database(fixture.db.name());
        let missing_support =
            fixture
                .backend
                .classify_missing(tenant_id, &query.resource_type, &query);
        let armed = session
            .admin
            .run_command(doc! {
                "configureFailPoint": "failCommand", "mode": "alwaysOn",
                "data": { "failCommands": ["find", "getMore", "aggregate"], "appName": &app_name,
                    "blockConnection": true, "blockTimeMS": 100 },
            })
            .await
            .unwrap();
        let mut facts = ProbeFacts::default();
        let total = tokio::time::timeout(
            Duration::from_secs(120),
            with_missing_check_batch_rows(
                32,
                fixture.backend.count_standard_with_facts(
                    &fixture.tenant,
                    &db,
                    &query,
                    &missing_support,
                    BatchSelection::Count,
                    CountOrigin::Query,
                    &mut facts,
                ),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("pool {pool:?}: the mixed total did not finish"))
        .unwrap();
        let disabled = disable_failpoint(&session).await;
        client.shutdown().await;
        let state = state.lock().unwrap();
        assert_eq!(total, want, "pool {pool:?}: total");
        assert!(
            failpoint_hits(&disabled) - failpoint_hits(&armed) > 0,
            "pool {pool:?}: no command was held"
        );
        let count = |name: &str| state.started.iter().filter(|n| *n == name).count();
        assert!(
            count("getMore") >= 2 && count("aggregate") >= 4,
            "pool {pool:?}: the stamped stream crossed batches: {:?}",
            state.started
        );
        assert!(
            state.running.is_empty(),
            "pool {pool:?}: commands still in flight after the total: {:?}",
            state.running
        );
        assert_eq!(
            state.max, 1,
            "pool {pool:?}: commands in flight together: {:?}; sequence {:?}",
            state.overlaps, state.started
        );
        let sequence: Vec<&str> = state
            .sequence
            .iter()
            .map(String::as_str)
            .filter(|command| !command.starts_with("endSessions:"))
            .collect();
        let stream = sequence
            .iter()
            .position(|command| *command == "find:resources")
            .unwrap_or_else(|| panic!("pool {pool:?}: no stamped stream: {sequence:?}"));
        let tail = &sequence[stream..];
        assert!(
            tail.windows(2)
                .all(|pair| pair != ["getMore:resources", "getMore:resources"]),
            "pool {pool:?}: two stream reads without a check between: {sequence:?}"
        );
        let unstamped: Vec<usize> = (0..tail.len())
            .filter(|at| tail[*at] == "aggregate:resources")
            .collect();
        assert_eq!(
            unstamped,
            vec![tail.len() - 1],
            "pool {pool:?}: the unstamped count is the last command and the only \
             aggregate on resources after the stream: {sequence:?}"
        );
    }
    fixture.close().await;
}

/// The batch consumer of `main` keeps its queue of two: a batched exact
/// count over more than two batches of 4,096 candidates has exactly two
/// commands in flight at its peak (two checks, or a check and the next
/// `getMore`), never one and never three. Same evidence as
/// [`missing_checks_total_has_one_command_in_flight`]: command monitoring on a
/// client of its own, with every `getMore` and `aggregate` held 100 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn batch_consumer_count_keeps_two_commands_in_flight() {
    let label = "batch-consumer-two-commands";
    let Some(fixture) = Fixture::new(label, 9_000).await else {
        return;
    };
    let Some(session) = fixture.failpoints(label).await else {
        fixture.close().await;
        return;
    };
    let uri = shared_mongo::connection_string().await.unwrap();
    let app_name = format!("hfs-plan-{label}-{}", uuid::Uuid::new_v4().simple());
    let state = std::sync::Arc::new(std::sync::Mutex::new(CommandsInFlight::default()));
    let mut options = mongodb::options::ClientOptions::parse(&uri).await.unwrap();
    options.app_name = Some(app_name.clone());
    options.command_event_handler = Some(CommandsInFlight::handler(&state));
    let client = mongodb::Client::with_options(options).unwrap();
    let db = client.database(fixture.db.name());
    let armed = session
        .admin
        .run_command(doc! {
            "configureFailPoint": "failCommand", "mode": "alwaysOn",
            "data": { "failCommands": ["getMore", "aggregate"], "appName": &app_name,
                "blockConnection": true, "blockTimeMS": 100 },
        })
        .await
        .unwrap();
    let plan = batched_plan(vec![predicate("status", "final")]);
    let result = tokio::time::timeout(
        Duration::from_secs(120),
        plan.execute(&db, resource_filter(), None, BatchSelection::Count),
    )
    .await
    .expect("the batched count did not finish")
    .unwrap();
    let disabled = disable_failpoint(&session).await;
    client.shutdown().await;
    let state = state.lock().unwrap();
    let want = fixture.expected(true, 1, 0, usize::MAX).len() as u64;
    assert_eq!(result.count, want, "batched count");
    assert!(
        failpoint_hits(&disabled) - failpoint_hits(&armed) > 0,
        "no command was held"
    );
    assert!(
        state.started.iter().filter(|n| *n == "getMore").count() >= 2,
        "the candidate stream crossed batches: {:?}",
        state.started
    );
    assert_eq!(
        state.max, 2,
        "commands in flight together: {:?}; sequence {:?}",
        state.overlaps, state.started
    );
    fixture.close().await;
}

/// [`stream_id_batches`] on its own over 8 ids, read 2 per cursor batch, in
/// batches of 2, with checks that sleep 50 ms: with the mixed total's limit of
/// one, `start` is never called while another check is started and not
/// finished; with the batch consumer's limit of two it is (the cell sees an
/// overlap when there is one).
#[tokio::test]
async fn stream_id_batches_with_a_limit_of_one_starts_no_check_beside_another() {
    use super::super::{
        COUNT_BATCH_CONCURRENCY, IdStreamText, MISSING_CHECK_CONCURRENCY, stream_id_batches,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let Some(fixture) = Fixture::new("stream-batches-one", 8).await else {
        return;
    };
    for (limit, overlap_expected) in [
        (MISSING_CHECK_CONCURRENCY, false),
        (COUNT_BATCH_CONCURRENCY, true),
    ] {
        let mut cursor = fixture
            .db
            .collection::<Document>("resources")
            .find(doc! { "tenant_id": PLAN_TENANT })
            .projection(doc! { "_id": 0, "id": 1 })
            .batch_size(2)
            .await
            .unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let mut starts = 0usize;
        let mut starts_beside_a_check = 0usize;
        let mut folded = 0usize;
        stream_id_batches(
            &mut cursor,
            "id",
            &IdStreamText {
                read: "read",
                invalid: "invalid",
                missing_id: "missing id",
            },
            2,
            limit,
            &mut folded,
            |_, _batch: Vec<String>| {
                starts += 1;
                if active.fetch_add(1, Ordering::SeqCst) > 0 {
                    starts_beside_a_check += 1;
                }
                let active = active.clone();
                Ok(Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                })
                    as std::pin::Pin<
                        Box<dyn Future<Output = StorageResult<()>> + Send>,
                    >)
            },
            |folded, ()| *folded += 1,
        )
        .await
        .unwrap();
        assert_eq!((starts, folded), (4, 4), "limit {limit}: every batch");
        assert_eq!(
            starts_beside_a_check > 0,
            overlap_expected,
            "limit {limit}: {starts_beside_a_check} checks started beside another"
        );
    }
    fixture.close().await;
}

/// A backend on the fixture's database with a pool of `pool` connections and
/// an `appName` of its own, its missing metadata trusted.
async fn backend_with_pool(fixture: &Fixture, label: &str, pool: u32) -> MongoBackend {
    let mut config = fixture.backend.config().clone();
    config.max_connections = Some(pool);
    config.app_name = Some(format!(
        "hfs-plan-{label}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let backend = MongoBackend::new(config).unwrap();
    backend.init_schema().await.unwrap();
    trust_missing_metadata(&backend, &fixture.db).await;
    backend
}

fn observation(status: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Observation",
        "status": status,
        "code": { "coding": [{ "code": "base" }] },
    })
}

/// A mixed total dropped while its membership check is blocked on the
/// server (a request timeout) frees its pool slot: the driver closes the
/// connection of the unanswered command, so on a pool of one the in-use
/// count returns to 0 and a write succeeds at once on a new connection, long
/// before the blocked command would have answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_checks_dropped_total_gives_its_connection_back() {
    use crate::core::ResourceStorage;
    const BLOCK_MS: i64 = 4_000;
    let label = "missing-checks-dropped";
    let Some(fixture) = Fixture::new(label, ROWS).await else {
        return;
    };
    prepare(&fixture).await;
    let Some(session) = fixture.failpoints(label).await else {
        fixture.close().await;
        return;
    };
    let backend = backend_with_pool(&fixture, label, 1).await;
    let armed = session
        .admin
        .run_command(doc! {
            "configureFailPoint": "failCommand", "mode": { "times": 1 },
            "data": { "failCommands": ["aggregate"], "appName": backend.config().app_name_or_default(),
                "namespace": format!("{}.search_index", fixture.db.name()),
                "blockConnection": true, "blockTimeMS": BLOCK_MS },
        })
        .await
        .unwrap();
    let dropped = tokio::time::timeout(
        Duration::from_millis(1_000),
        with_missing_check_batch_rows(
            7,
            backend.search_count(&fixture.tenant, &query(&shapes()[0])),
        ),
    )
    .await;
    let dropped_at = std::time::Instant::now();
    assert!(dropped.is_err(), "the total ended before it was dropped");
    while backend.pool_stats().snapshot().in_use > 0 {
        assert!(
            dropped_at.elapsed() < Duration::from_millis(1_000),
            "the dropped total still holds a connection: {:?}",
            backend.pool_stats().snapshot()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let write = tokio::time::timeout(
        Duration::from_millis(1_500),
        backend.create(
            &fixture.tenant,
            "Observation",
            observation("final"),
            helios_fhir::FhirVersion::default(),
        ),
    )
    .await
    .expect("a write waited for the dropped total's connection");
    write.unwrap();
    let written = dropped_at.elapsed();
    assert!(
        written < Duration::from_millis(BLOCK_MS as u64 - 1_000),
        "the write ended {written:?} after the drop, near the blocked command's end"
    );
    // Turning the failpoint off waits for the blocked command to leave it, so
    // it comes after the write.
    let disabled = disable_failpoint(&session).await;
    assert_eq!(
        failpoint_hits(&disabled) - failpoint_hits(&armed),
        1,
        "the membership check was blocked on the server when the total was dropped"
    );
    fixture.close().await;
}

/// Three looping mixed totals and two writers (create, update, delete) on a
/// pool of two connections: every write and every total succeeds within its
/// bound, and afterwards each total, still on the split route, equals the
/// single aggregation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_checks_totals_beside_writers_on_a_pool_of_two() {
    use crate::core::ResourceStorage;
    let label = "missing-checks-writers";
    let Some(fixture) = Fixture::new(label, ROWS).await else {
        return;
    };
    prepare(&fixture).await;
    let backend = backend_with_pool(&fixture, label, 2).await;
    let mut tasks = Vec::new();
    for index in [0usize, 2, 3] {
        let backend = backend.clone();
        let tenant = fixture.tenant.clone();
        let query = query(&shapes()[index]);
        tasks.push(tokio::spawn(async move {
            for round in 0..4 {
                tokio::time::timeout(
                    Duration::from_secs(60),
                    with_missing_check_batch_rows(7, backend.search_count(&tenant, &query)),
                )
                .await
                .unwrap_or_else(|_| panic!("total {index} round {round} timed out"))
                .unwrap_or_else(|error| panic!("total {index} round {round}: {error:?}"));
            }
        }));
    }
    for writer in 0..2 {
        let backend = backend.clone();
        let tenant = fixture.tenant.clone();
        tasks.push(tokio::spawn(async move {
            let bound = Duration::from_secs(30);
            for n in 0..20 {
                let created = tokio::time::timeout(
                    bound,
                    backend.create(
                        &tenant,
                        "Observation",
                        observation("preliminary"),
                        helios_fhir::FhirVersion::default(),
                    ),
                )
                .await
                .unwrap_or_else(|_| panic!("writer {writer} create {n} timed out"))
                .unwrap_or_else(|error| panic!("writer {writer} create {n}: {error:?}"));
                let mut changed = created.content().clone();
                changed["status"] = serde_json::json!("final");
                let updated =
                    tokio::time::timeout(bound, backend.update(&tenant, &created, changed))
                        .await
                        .unwrap_or_else(|_| panic!("writer {writer} update {n} timed out"))
                        .unwrap_or_else(|error| panic!("writer {writer} update {n}: {error:?}"));
                if n % 3 == 0 {
                    tokio::time::timeout(
                        bound,
                        backend.delete(&tenant, "Observation", updated.id()),
                    )
                    .await
                    .unwrap_or_else(|_| panic!("writer {writer} delete {n} timed out"))
                    .unwrap_or_else(|error| panic!("writer {writer} delete {n}: {error:?}"));
                }
            }
        }));
    }
    for task in futures::future::join_all(tasks).await {
        task.unwrap();
    }
    for index in [0usize, 2, 3] {
        let shape = &shapes()[index];
        let query = query(shape);
        let single = backend
            .count_standard(&fixture.tenant, &query, CountOrigin::ParamSortOverCap)
            .await
            .unwrap();
        restart_profiling(&fixture.db, 32 * 1024 * 1024).await;
        let total = with_missing_check_batch_rows(7, backend.search_count(&fixture.tenant, &query))
            .await
            .unwrap();
        let finds = stream_finds(&fixture.db).await;
        assert_eq!(finds.len(), 1, "{}: the split route ran", shape.label);
        assert_eq!(total, single, "{}: after the writes", shape.label);
    }
    fixture.close().await;
}
