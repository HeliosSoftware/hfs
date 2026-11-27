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

/// A total whose two in-flight membership checks are blocked past the search
/// deadline answers the timeout, never a number, and only after both
/// checks finished, with the admission permit back.
#[tokio::test]
async fn missing_checks_deadline_drains_in_flight_checks() {
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
        2,
        "both in-flight membership checks reached MongoDB"
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

/// The deadline expires while the first membership check is still blocked
/// on the server and the stream keeps starting batches: a later batch answers
/// the timeout at once, and the total returns it only after the blocked check
/// finished (no driver call outlives the search). Timing-dependent: the
/// deterministic drain cell is `stream_id_batches_drains_started_checks_on_error`.
#[tokio::test]
async fn missing_checks_error_waits_for_the_check_still_in_flight() {
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
            "configureFailPoint": "failCommand", "mode": { "times": 1 },
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
        "one membership check was blocked"
    );
    assert!(
        elapsed >= Duration::from_millis(BLOCK_MS as u64 - 300),
        "returned after {elapsed:?}, while the blocked check was still in flight"
    );
    fixture.close().await;
}

/// [`stream_id_batches`] on its own, over 8 ids read 2 per cursor batch, in
/// batches of 2: the first check sleeps and then sets a flag, the second fails
/// at once. The error is returned only after the first check finished.
#[tokio::test]
async fn stream_id_batches_drains_started_checks_on_error() {
    use super::super::{IdStreamText, stream_id_batches};
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
