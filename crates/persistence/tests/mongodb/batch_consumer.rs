//! #2233: the batched candidate consumer (`consume_batches` in
//! `search_impl/search_plan.rs`) through every caller that reaches it.
//!
//! The consumer streams one predicate's grouped candidate ids from a
//! `search_index` cursor in batches of 4,096 and keeps at most two batch
//! checks in flight. Each cell here streams far more than 3 x 4,096 = 12,288
//! candidates, so the cursor needs at least three `getMore`s while batch
//! checks are pending:
//!
//! | cell            | selection                | how it is reached                                  |
//! |-----------------|--------------------------|----------------------------------------------------|
//! | `Count`         | `Count`                  | `search_count`, one positive token                 |
//! | `PageAndCount`  | `PageAndCount`           | `search`, `_total=accurate&_sort=_id`              |
//! | `BoundedPage`   | `BoundedPage`            | `search`, no total, two tokens, `_offset=257`      |
//! | `EstimatedCount`| `EstimatedCount`         | `search_count`, `_total=estimate`, two tokens      |
//! | `EstimatedPage` | `EstimatedPageAndCount`  | `search`, `_total=estimate&_sort=_id`, two tokens  |
//! | `MissingCount`  | `Count`, missing checks  | `search_count`, a token plus `specimen:missing=true` |
//!
//! Every expected value comes from the fixture's generator functions below,
//! never from MongoDB. Every cell also proves that the batched route really
//! ran: the `helios_mongodb_plan` / `helios_mongodb_page` events must name
//! the batched plan and strategy, and the profiler must show at least three
//! completed `getMore`s of a `batchSize: 4096` cursor on `search_index`.
//!
//! The `*_on_a_one_connection_pool` tests run the same cells on a backend
//! whose pool has a single connection, under a bounded timeout. Before the
//! #2233 fix, while the consumer awaited the cursor's `getMore`, one started
//! batch check was not polled but held, or was queued ahead for, the only
//! connection, so the `getMore` never got one and the search did not end: a
//! REST request is dropped by its timeout layer (`HFS_REQUEST_TIMEOUT`, 30 s by
//! default), other callers wait without bound. The consumer now polls every
//! started check while it reads; these tests fail with `POOL DEADLOCK` if that
//! regresses.

use super::*;
use futures::TryStreamExt;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

const TEST: &str = "batch_consumer";
/// Resources written; ids `b00000`..`b39999`.
const RESOURCES: usize = 40_000;
const SYSTEM: &str = "urn:hfs:test:batch-consumer";
/// The consumer's candidate batch size (`COUNT_CANDIDATE_BATCH_ROWS`).
const BATCH_ROWS: i64 = 4_096;
/// Index-only ids: rows for both tokens, no resource document.
const ORPHANS: [&str; 3] = ["b-orphan-0", "b-orphan-1", "b-orphan-2"];
/// Bounded-page offset: above the 256-row sample, so the adaptive policy runs.
const BOUNDED_OFFSET: u32 = 257;
const PAGE: u32 = 20;
/// A healthy search here finishes in seconds; this only bounds a hang.
const DEADLOCK_BOUND: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// Fixture: generator functions (the oracle) and seeding
// ---------------------------------------------------------------------------

fn id_of(n: usize) -> String {
    format!("b{n:05}")
}

/// `code=urn:hfs:test:batch-consumer|a`, else `|z`: 20,000 resources.
fn has_a(n: usize) -> bool {
    n.is_multiple_of(2)
}

/// `status=final`, else `preliminary`: 20,313 resources. The overlap with
/// `has_a` is every 128th resource only, so a 256-row sample of either
/// token's candidates holds far fewer than 16 live matches.
fn is_final(n: usize) -> bool {
    !n.is_multiple_of(2) || n.is_multiple_of(128)
}

fn has_specimen(n: usize) -> bool {
    n.is_multiple_of(5)
}

/// Soft-deleted, index rows kept: candidates that exact totals must reject.
fn is_deleted(n: usize) -> bool {
    n.is_multiple_of(7)
}

/// Gets a second, identical `code` row: one id must still count once.
fn duplicated(n: usize) -> bool {
    n.is_multiple_of(3)
}

fn live(filter: impl Fn(usize) -> bool) -> Vec<String> {
    (0..RESOURCES)
        .filter(|n| !is_deleted(*n) && filter(*n))
        .map(id_of)
        .collect()
}

/// Ids with rows for both tokens, deleted or not, plus the orphans: what an
/// index estimate counts (it validates no resource).
fn indexed_a_and_final() -> u64 {
    ((0..RESOURCES).filter(|n| has_a(*n) && is_final(*n)).count() + ORPHANS.len()) as u64
}

fn observation(id: &str, a: bool, final_: bool, specimen: bool) -> serde_json::Value {
    let mut body = json!({
        "resourceType": "Observation", "id": id,
        "status": if final_ { "final" } else { "preliminary" },
        "code": {"coding": [{"system": SYSTEM, "code": if a { "a" } else { "z" }}]},
    });
    if specimen {
        body["specimen"] = json!({"reference": "Specimen/s1"});
    }
    body
}

/// One resource written through the real path, used as a clone template.
struct Template {
    resource: Document,
    rows: Vec<Document>,
}

async fn template(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &TenantContext,
    shape: (bool, bool, bool),
) -> Template {
    let (a, final_, specimen) = shape;
    let id = format!(
        "tpl-{}{}{}",
        u8::from(a),
        u8::from(final_),
        u8::from(specimen)
    );
    backend
        .seed_create(
            tenant,
            "Observation",
            observation(&id, a, final_, specimen),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let tenant_id = tenant.tenant_id().as_str();
    let resources = db.collection::<Document>("resources");
    let index = db.collection::<Document>("search_index");
    let mut resource = resources
        .find_one(doc! {"tenant_id": tenant_id, "id": &id})
        .await
        .unwrap()
        .expect("template resource");
    resource.remove("_id");
    let rows: Vec<Document> = index
        .find(doc! {"tenant_id": tenant_id, "resource_id": &id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let rows: Vec<Document> = rows
        .into_iter()
        .map(|mut row| {
            row.remove("_id");
            row
        })
        .collect();
    for (name, present) in [("code", true), ("status", true), ("specimen", specimen)] {
        assert_eq!(
            rows.iter().any(|row| row.get_str("param_name") == Ok(name)),
            present,
            "template {id}: {name} row"
        );
    }
    resources
        .delete_one(doc! {"tenant_id": tenant_id, "id": &id})
        .await
        .unwrap();
    index
        .delete_many(doc! {"tenant_id": tenant_id, "resource_id": &id})
        .await
        .unwrap();
    Template { resource, rows }
}

/// Writes the fixture: [`RESOURCES`] clones of real-path templates (their
/// rows and `missing_params` stamps are the writer's own), plus the orphans.
async fn seed(backend: &MongoBackend, tenant: &TenantContext) {
    let db = backend.get_database().await.unwrap();
    let mut templates = HashMap::new();
    for a in [false, true] {
        for final_ in [false, true] {
            for specimen in [false, true] {
                let shape = (a, final_, specimen);
                templates.insert(shape, template(backend, &db, tenant, shape).await);
            }
        }
    }
    let resources = db.collection::<Document>("resources");
    let index = db.collection::<Document>("search_index");
    let epoch = mongodb::bson::DateTime::parse_rfc3339_str("2026-01-01T00:00:00Z")
        .unwrap()
        .timestamp_millis();
    for start in (0..RESOURCES).step_by(5_000) {
        let mut documents = Vec::new();
        let mut rows = Vec::new();
        for n in start..(start + 5_000).min(RESOURCES) {
            let id = id_of(n);
            let template = &templates[&(has_a(n), is_final(n), has_specimen(n))];
            let mut resource = template.resource.clone();
            resource.insert("id", &id);
            resource.get_document_mut("data").unwrap().insert("id", &id);
            resource.insert(
                "last_updated",
                mongodb::bson::DateTime::from_millis(epoch + n as i64 * 1_000),
            );
            resource.insert("is_deleted", is_deleted(n));
            documents.push(resource);
            for row in &template.rows {
                let mut row = row.clone();
                row.insert("resource_id", &id);
                let repeat = duplicated(n) && row.get_str("param_name") == Ok("code");
                if repeat {
                    rows.push(row.clone());
                }
                rows.push(row);
            }
        }
        resources.insert_many(documents).await.unwrap();
        index.insert_many(rows).await.unwrap();
    }
    let shape = &templates[&(true, true, false)];
    let mut rows = Vec::new();
    for id in ORPHANS {
        for row in &shape.rows {
            let mut row = row.clone();
            row.insert("resource_id", id);
            rows.push(row);
        }
    }
    index.insert_many(rows).await.unwrap();
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

fn token(name: &str, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.into(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::parse(value)],
        chain: vec![],
        components: vec![],
    }
}

fn code_a() -> SearchQuery {
    SearchQuery::new("Observation").with_parameter(token("code", &format!("{SYSTEM}|a")))
}

fn code_a_final() -> SearchQuery {
    code_a().with_parameter(token("status", "final"))
}

fn by_id(query: SearchQuery, count: u32) -> SearchQuery {
    query
        .with_sort(SortDirective::parse("_id"))
        .with_count(count)
}

// ---------------------------------------------------------------------------
// Route evidence: plan events and profiler
// ---------------------------------------------------------------------------

/// The `helios_mongodb_plan` / `helios_mongodb_page` events of one search, as
/// `(field, value)` pairs: `filter_strategy` / `total_filter_strategy` (the
/// plan a page or a total selected), `total_strategy`, `page_strategy`.
#[derive(Clone, Default)]
struct RouteLog(Arc<Mutex<Vec<(String, String)>>>);

impl RouteLog {
    fn events(&self) -> Vec<(String, String)> {
        self.0.lock().unwrap().clone()
    }

    fn has(&self, field: &str, value: &str) -> bool {
        self.events()
            .iter()
            .any(|(name, recorded)| name == field && recorded == value)
    }
}

#[derive(Default)]
struct RouteVisitor {
    span_field: Option<String>,
    fields: Vec<(String, String)>,
}

impl tracing::field::Visit for RouteVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "span_field" => self.span_field = Some(value.to_owned()),
            name @ ("filter_strategy" | "total_strategy" | "page_strategy") => {
                self.fields.push((name.to_owned(), value.to_owned()));
            }
            _ => {}
        }
    }
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl tracing::Subscriber for RouteLog {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        matches!(
            metadata.target(),
            "helios_mongodb_plan" | "helios_mongodb_page"
        )
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::DEBUG)
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = RouteVisitor::default();
        event.record(&mut visitor);
        let mut events = self.0.lock().unwrap();
        for (name, value) in visitor.fields {
            // A filter plan is reported under the span field it fills.
            let name = match (name.as_str(), visitor.span_field.as_deref()) {
                ("filter_strategy", Some(span_field)) => span_field.to_owned(),
                _ => name,
            };
            events.push((name, value));
        }
    }
}

/// Completed `getMore`s of a `batchSize: 4096` aggregate on `search_index`:
/// the consumer's candidate stream (and the adaptive sample that it continues).
fn stream_get_mores(operations: &[Document]) -> usize {
    operations
        .iter()
        .filter(|operation| operation.get_str("op") == Ok("getmore"))
        .map(composite_plan::command_of)
        .filter(|command| {
            command.get_str("aggregate") == Ok("search_index")
                && command.get_document("cursor").is_ok_and(|cursor| {
                    cursor
                        .get_i32("batchSize")
                        .map(i64::from)
                        .or_else(|_| cursor.get_i64("batchSize"))
                        == Ok(BATCH_ROWS)
                })
        })
        .count()
}

// ---------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Cell {
    Count,
    PageAndCount,
    BoundedPage,
    EstimatedCount,
    EstimatedPage,
    MissingCount,
}

/// What a cell returned: a total, page ids, or both.
#[derive(Debug, PartialEq)]
struct Answer {
    total: Option<u64>,
    ids: Option<Vec<String>>,
}

impl Cell {
    /// The value the fixture's generator functions dictate.
    fn expected(self) -> Answer {
        let a_final = live(|n| has_a(n) && is_final(n));
        match self {
            Cell::Count => Answer {
                total: Some(live(has_a).len() as u64),
                ids: None,
            },
            Cell::PageAndCount => {
                let ids = live(has_a);
                Answer {
                    total: Some(ids.len() as u64),
                    ids: Some(ids[..PAGE as usize].to_vec()),
                }
            }
            Cell::BoundedPage => {
                let start = BOUNDED_OFFSET as usize;
                let end = (start + PAGE as usize).min(a_final.len());
                assert!(start < end, "the bounded page must not be empty");
                Answer {
                    total: None,
                    ids: Some(a_final[start..end].to_vec()),
                }
            }
            Cell::EstimatedCount => Answer {
                total: Some(indexed_a_and_final()),
                ids: None,
            },
            Cell::EstimatedPage => Answer {
                total: Some(indexed_a_and_final()),
                ids: Some(a_final[..PAGE as usize].to_vec()),
            },
            Cell::MissingCount => Answer {
                total: Some(live(|n| has_a(n) && !has_specimen(n)).len() as u64),
                ids: None,
            },
        }
    }

    /// The `(field, value)` events that prove the batched consumer ran.
    fn route(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Cell::Count | Cell::MissingCount => &[
                ("total_filter_strategy", "batched_index"),
                ("total_strategy", "exact_batches"),
            ],
            Cell::EstimatedCount => &[
                ("total_filter_strategy", "batched_index"),
                ("total_strategy", "index_estimate"),
            ],
            Cell::PageAndCount => &[
                ("filter_strategy", "batched_index"),
                ("total_strategy", "combined_exact"),
            ],
            Cell::EstimatedPage => &[
                ("filter_strategy", "batched_index"),
                ("total_strategy", "combined_index_estimate"),
            ],
            Cell::BoundedPage => &[
                ("filter_strategy", "batched_index"),
                ("page_strategy", "bounded"),
                ("total_strategy", "omitted"),
            ],
        }
    }

    async fn run(self, backend: &MongoBackend, tenant: &TenantContext) -> StorageResult<Answer> {
        let count = |query: SearchQuery| async move {
            backend
                .search_count(tenant, &query)
                .await
                .map(|total| Answer {
                    total: Some(total),
                    ids: None,
                })
        };
        let page = |query: SearchQuery| async move {
            backend.search(tenant, &query).await.map(|result| Answer {
                total: result.total,
                ids: Some(
                    result
                        .resources
                        .items
                        .iter()
                        .map(|resource| resource.id().to_owned())
                        .collect(),
                ),
            })
        };
        match self {
            Cell::Count => count(code_a()).await,
            Cell::PageAndCount => {
                let mut query = by_id(code_a(), PAGE);
                query.total = Some(TotalMode::Accurate);
                page(query).await
            }
            Cell::BoundedPage => {
                let mut query = by_id(code_a_final(), PAGE);
                query.offset = Some(BOUNDED_OFFSET);
                query.total = Some(TotalMode::None);
                page(query).await
            }
            Cell::EstimatedCount => {
                let mut query = code_a_final();
                query.total = Some(TotalMode::Estimate);
                count(query).await
            }
            Cell::EstimatedPage => {
                let mut query = by_id(code_a_final(), PAGE);
                query.total = Some(TotalMode::Estimate);
                page(query).await
            }
            Cell::MissingCount => {
                count(code_a().with_parameter(SearchParameter {
                    name: "specimen".into(),
                    param_type: SearchParamType::Reference,
                    modifier: Some(SearchModifier::Missing),
                    values: vec![SearchValue::eq("true")],
                    chain: vec![],
                    components: vec![],
                }))
                .await
            }
        }
    }
}

type StorageResult<T> = Result<T, StorageError>;

/// A backend on its own database, with the default test pool or `pool`.
///
/// `None` when no MongoDB is available: the suite's skip, which
/// `HFS_TEST_REQUIRE_MONGODB` turns into a failure (`skip_or_fail_unavailable`,
/// called by `shared_mongo` and `build_backend`).
async fn backend_for(cell: Cell, pool: Option<u32>) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let name = format!(
        "{TEST}_{cell:?}_{}",
        pool.map_or("pool".into(), |p| p.to_string())
    );
    build_backend(MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(&name),
        data_dir: Some(repo_data_dir()),
        // Stable probes keep the plan from changing on a loaded host.
        probe_timeout_ms: 10_000,
        max_connections: pool,
        ..Default::default()
    })
    .await
}

/// Seeds a fresh database, runs `cell` under the profiler (on a separate
/// client, so the backend's pool serves only the search) with a bounded wait,
/// and checks the value and the route.
async fn check(cell: Cell, pool: Option<u32>) {
    let Some(backend) = backend_for(cell, pool).await else {
        eprintln!("Skipping {TEST} {cell:?} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("batch-consumer");
    seed(&backend, &tenant).await;
    let observer = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap();
    let db = observer.database(&backend.config().database_name);

    pin_tracing_interest();
    let log = RouteLog::default();
    let dispatch = tracing::Dispatch::new(log.clone());
    let _guard = tracing::dispatcher::set_default(&dispatch);
    let started = std::time::Instant::now();
    let (outcome, operations) = composite_plan::profiled(
        &db,
        tokio::time::timeout(DEADLOCK_BOUND, cell.run(&backend, &tenant)),
    )
    .await;
    let elapsed = started.elapsed();
    let get_mores = stream_get_mores(&operations);
    let events = log.events();
    eprintln!(
        "{TEST}: cell={cell:?} pool={pool:?} elapsed={elapsed:?} \
         stream_get_mores={get_mores} events={events:?}"
    );
    let answer = match outcome {
        Err(_) => panic!(
            "POOL DEADLOCK: {cell:?} on pool {pool:?} did not finish within \
             {DEADLOCK_BOUND:?}; completed stream getMores: {get_mores}; route events: {events:?}"
        ),
        Ok(Err(error)) => panic!("SEARCH ERROR: {cell:?} on pool {pool:?}: {error:?}"),
        Ok(Ok(answer)) => answer,
    };
    assert_eq!(
        answer,
        cell.expected(),
        "{cell:?}: value differs from the oracle"
    );
    for (field, value) in cell.route() {
        assert!(
            log.has(field, value),
            "{cell:?}: no `{field} = {value}` event, so the batched consumer was not \
             shown to run; events: {events:?}"
        );
    }
    assert!(
        get_mores >= 3,
        "{cell:?}: the candidate stream completed {get_mores} getMores of a \
         batchSize-{BATCH_ROWS} search_index cursor; more than three batches need at least 3"
    );
}

// ---------------------------------------------------------------------------
// Default pool: must pass before and after the fix
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mongodb_batch_consumer_count() {
    check(Cell::Count, None).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_page_and_count() {
    check(Cell::PageAndCount, None).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_bounded_page() {
    check(Cell::BoundedPage, None).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_estimated_count() {
    check(Cell::EstimatedCount, None).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_estimated_page_and_count() {
    check(Cell::EstimatedPage, None).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_missing_true_with_positive_count() {
    check(Cell::MissingCount, None).await;
}

// ---------------------------------------------------------------------------
// One-connection pool: hung before the fix; a hang fails as `POOL DEADLOCK`
// ---------------------------------------------------------------------------

const ONE: Option<u32> = Some(1);

#[tokio::test]
async fn mongodb_batch_consumer_count_on_a_one_connection_pool() {
    check(Cell::Count, ONE).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_page_and_count_on_a_one_connection_pool() {
    check(Cell::PageAndCount, ONE).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_bounded_page_on_a_one_connection_pool() {
    check(Cell::BoundedPage, ONE).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_estimated_count_on_a_one_connection_pool() {
    check(Cell::EstimatedCount, ONE).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_estimated_page_and_count_on_a_one_connection_pool() {
    check(Cell::EstimatedPage, ONE).await;
}

#[tokio::test]
async fn mongodb_batch_consumer_missing_true_with_positive_count_on_a_one_connection_pool() {
    check(Cell::MissingCount, ONE).await;
}
