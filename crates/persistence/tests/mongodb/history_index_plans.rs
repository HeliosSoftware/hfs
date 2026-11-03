//! #2353: explain plans of every `resource_history` reader with and without
//! `idx_history_resource_updated`.
//!
//! The index shares its equality prefix `{tenant_id, resource_type, id}` with
//! the unique `idx_history_identity` and adds `last_updated: -1` after it. This
//! probe measures whether any reader needs that order.
//!
//! Child module of the `mongodb_tests` root: `use super::*;` reaches its private
//! harness (`create_backend`, `create_tenant`, `raw_test_client`,
//! `command_for_explain`, `collect_index_names`, `contains_stage_named`,
//! `drop_test_index`).
//!
//! How it measures. Every reader is called through the backend's public API with
//! the database profiler at level 2. The profiler entries on `resource_history`
//! carry the commands the backend really sent (a `find`, or the aggregate behind
//! `count_documents`), so each find and aggregate is rebuilt with
//! [`command_for_explain`] and re-run at `executionStats` verbosity. The keys and
//! docs examined come from that explain, not from the profiler entry, because a
//! profiler entry for a `find` covers only its first batch. A `delete` is
//! destructive, so `purge` (its count and its delete) is measured from its
//! profiler entries (`planSummary`, `keysExamined`, `docsExamined`) and the type
//! and tenant purges from explain of the delete the backend sends.
//!
//! A `create` is profiled for `keysInserted`, the index keys of one history insert.
//!
//! The dataset has one resource with 300 versions among 5,000 two-version
//! resources, 1,000 observations and a second tenant.
//!
//! Two legs run on one database: `before` with `idx_history_resource_updated`
//! (created here if the schema no longer creates it) and `after` with it
//! dropped. The test prints one `HISTORY_PLAN` line per operation (with
//! `--nocapture`), then asserts the checks in [`check_legs`].
//!
//! Since #2375 an instance-history page is a covered read of the resource's
//! version ids from `idx_history_identity`, followed by the documents of the
//! next `count + 1` versions. Each `history_instance_*` reader therefore shows
//! `#1` (the version-id read) and `#2`.. (the fetches).
//!
//! The second test boots the backend over a store that still has the index, in
//! inline and in `off` build mode, and checks that schema init drops it.

use super::*;

use chrono::{DateTime, Utc};
use futures::stream::TryStreamExt;
use helios_persistence::types::Pagination;
use mongodb::bson::DateTime as BsonDateTime;

const OLD_INDEX: &str = "idx_history_resource_updated";
const IDENTITY_INDEX: &str = "idx_history_identity";
const TYPE_INDEX: &str = "idx_history_type_updated";
const SYSTEM_INDEX: &str = "idx_history_system_updated";

const TENANT: &str = "tenant-hist-plans";
const OTHER_TENANT: &str = "tenant-hist-plans-other";
const KEYS_TENANT: &str = "tenant-hist-plans-keys";

/// The one resource with many versions.
const HOT_ID: &str = "hot";
const HOT_VERSIONS: i64 = 300;
/// The page size of every instance-history reader the probe measures.
const PAGE: u32 = 50;
/// Resources with two versions each (`Patient`).
const FILLER_PATIENTS: i64 = 5_000;
const FILLER_OBSERVATIONS: i64 = 1_000;
const OTHER_TENANT_PATIENTS: i64 = 1_000;
/// Versions of each of the two purge targets (`Basic/purge-before`,
/// `Basic/purge-after`).
const PURGE_VERSIONS: i64 = 50;
const PURGE_TYPE: &str = "Basic";

/// Creates the index exactly as the pre-#2353 `ensure_history_indexes` did. A
/// no-op where the schema still creates it; recreates it on a store a previous
/// build initialised.
async fn create_old_index(history: &Collection<Document>) {
    history
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {
                    "tenant_id": 1_i32,
                    "resource_type": 1_i32,
                    "id": 1_i32,
                    "last_updated": -1_i32
                })
                .options(Some(
                    mongodb::options::IndexOptions::builder()
                        .name(Some(OLD_INDEX.to_string()))
                        .unique(Some(false))
                        .build(),
                ))
                .build(),
        )
        .await
        .expect("create idx_history_resource_updated");
}

async fn index_names_of(history: &Collection<Document>) -> Vec<String> {
    let mut names = history.list_index_names().await.expect("list index names");
    names.sort();
    names
}

/// The `since` and `before` bounds of the time-filtered readers.
struct Windows {
    /// Matches `hot` v250..=v300 (and the template row, written now).
    since: DateTime<Utc>,
    /// Matches `hot` v1..=v50 (and every older filler row).
    before: DateTime<Utc>,
    /// Older than every row: the window covers the whole type, the case where a
    /// type-wide scan is worst for one resource's `_since` read.
    wide_since: DateTime<Utc>,
}

fn utc_millis(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).expect("valid timestamp")
}

/// Seeds `resource_history` with raw documents cloned from one real history row
/// written by `backend.create`, with explicit timestamps.
async fn seed(backend: &MongoBackend, history: &Collection<Document>) -> Windows {
    let tenant = create_tenant(TENANT);
    let created = backend
        .seed_create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Template"}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create the template resource");
    let mut template = history
        .find_one(doc! { "tenant_id": TENANT, "id": created.id() })
        .await
        .expect("read the template history row")
        .expect("the template history row exists");
    template.remove("_id");

    let row = |tenant: &str, resource_type: &str, id: &str, version: i64, at_ms: i64| {
        let mut row = template.clone();
        row.insert("tenant_id", tenant);
        row.insert("resource_type", resource_type);
        row.insert("id", id);
        row.insert("version_id", version.to_string());
        let at = BsonDateTime::from_millis(at_ms);
        row.insert("last_updated", at);
        row.insert("created_at", at);
        if let Ok(data) = row.get_document_mut("data") {
            data.insert("resourceType", resource_type);
            data.insert("id", id);
        }
        row
    };

    let now_ms = Utc::now().timestamp_millis();
    let hot_base = now_ms - 2 * 3_600 * 1_000;
    let old_base = now_ms - 30 * 86_400_000;

    let mut rows: Vec<Document> = Vec::new();
    for version in 1..=HOT_VERSIONS {
        rows.push(row(
            TENANT,
            "Patient",
            HOT_ID,
            version,
            hot_base + version * 1_000,
        ));
    }
    for i in 0..FILLER_PATIENTS {
        let id = format!("f-{i:05}");
        rows.push(row(TENANT, "Patient", &id, 1, old_base + i * 1_000));
        rows.push(row(TENANT, "Patient", &id, 2, old_base + i * 1_000 + 500));
    }
    for i in 0..FILLER_OBSERVATIONS {
        let id = format!("o-{i:05}");
        rows.push(row(TENANT, "Observation", &id, 1, old_base + i * 1_000));
    }
    for i in 0..OTHER_TENANT_PATIENTS {
        let id = format!("p-{i:05}");
        rows.push(row(OTHER_TENANT, "Patient", &id, 1, old_base + i * 1_000));
    }
    for id in ["purge-before", "purge-after"] {
        for version in 1..=PURGE_VERSIONS {
            rows.push(row(
                TENANT,
                PURGE_TYPE,
                id,
                version,
                old_base + version * 1_000,
            ));
        }
    }
    for chunk in rows.chunks(1_000) {
        history.insert_many(chunk).await.expect("seed history rows");
    }

    Windows {
        since: utc_millis(hot_base + 250_000),
        before: utc_millis(hot_base + 50_500),
        wide_since: utc_millis(old_base - 86_400_000),
    }
}

/// Runs `call` with the profiler at level 2 and returns its value with the
/// profiler entries on `resource_history`, oldest first. `getmore` entries are
/// left out on purpose: the explain re-run measures a whole query.
///
/// `None` when the server refuses the profiler (the caller skips).
async fn profiled<T>(
    db: &mongodb::Database,
    call: impl std::future::Future<Output = T>,
) -> Option<(T, Vec<Document>)> {
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;
    let profile = db.collection::<Document>("system.profile");
    let _ = profile.drop().await;
    if let Err(e) = db.run_command(doc! { "profile": 2_i32 }).await {
        eprintln!("Skipping history_index_plans: {{profile: 2}} was refused ({e})");
        return None;
    }
    let value = call.await;
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;
    let entries = profile
        .find(doc! {
            "ns": format!("{}.resource_history", db.name()),
            "op": { "$in": ["query", "command", "remove", "insert"] },
        })
        // system.profile is a capped collection, so natural order is the order the
        // entries were written. `ts` has millisecond resolution and can tie for
        // back-to-back ops (latest_write_marker's find and count), which
        // check_legs pairs by position.
        .sort(doc! { "$natural": 1_i32 })
        .await
        .expect("read system.profile")
        .try_collect::<Vec<Document>>()
        .await
        .expect("collect system.profile");
    Some((value, entries))
}

/// One measured operation on `resource_history`.
#[derive(Debug, Clone)]
struct Measured {
    reader: String,
    op: String,
    /// Index names in the winning plan (deduplicated, plan order).
    indexes: Vec<String>,
    chain: String,
    plan_summary: String,
    keys: i64,
    docs: i64,
    returned: i64,
    sort: bool,
    collscan: bool,
    /// `ndeleted` of a `remove` entry.
    deleted: Option<i64>,
    /// `keysInserted` of an `insert` entry.
    inserted: Option<i64>,
}

impl Measured {
    fn uses(&self, index: &str) -> bool {
        self.indexes.iter().any(|name| name == index)
    }

    fn line(&self, leg: &str) -> String {
        let indexes = if self.indexes.is_empty() {
            "none".to_string()
        } else {
            self.indexes.join("|")
        };
        let mut line = format!(
            "HISTORY_PLAN leg={leg} reader={} op={} indexes={indexes} chain={} \
             plan_summary=\"{}\" keys={} docs={} returned={} sort={} collscan={}",
            self.reader,
            self.op,
            self.chain,
            self.plan_summary,
            self.keys,
            self.docs,
            self.returned,
            self.sort,
            self.collscan,
        );
        if let Some(deleted) = self.deleted {
            line.push_str(&format!(" ndeleted={deleted}"));
        }
        if let Some(inserted) = self.inserted {
            line.push_str(&format!(" keysInserted={inserted}"));
        }
        line
    }
}

/// An integer BSON value of any width.
fn number(document: &Document, key: &str) -> Option<i64> {
    match document.get(key)? {
        Bson::Int32(n) => Some(i64::from(*n)),
        Bson::Int64(n) => Some(*n),
        Bson::Double(n) => Some(*n as i64),
        _ => None,
    }
}

/// `{ a: 1, b: -1 }`, the way a profiler `planSummary` prints a key pattern.
fn format_key_pattern(key: &Document) -> String {
    let inner = key
        .iter()
        .map(|(name, direction)| match direction {
            Bson::Int32(n) => format!("{name}: {n}"),
            Bson::Int64(n) => format!("{name}: {n}"),
            Bson::Double(n) => format!("{name}: {n}"),
            other => format!("{name}: {other}"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{ {inner} }}")
}

/// `(index name, formatted key pattern)` of every `resource_history` index.
async fn index_patterns(db: &mongodb::Database) -> Vec<(String, String)> {
    let listing = db
        .run_command(doc! { "listIndexes": "resource_history" })
        .await
        .expect("listIndexes resource_history");
    listing
        .get_document("cursor")
        .and_then(|cursor| cursor.get_array("firstBatch"))
        .expect("listIndexes firstBatch")
        .iter()
        .filter_map(Bson::as_document)
        .map(|index| {
            (
                index.get_str("name").unwrap_or_default().to_string(),
                format_key_pattern(index.get_document("key").expect("index key")),
            )
        })
        .collect()
}

/// A stage chain such as `FETCH<-IXSCAN(idx_history_identity)`.
fn plan_chain(plan: &Document) -> String {
    let mut node = plan;
    if let Ok(inner) = node.get_document("queryPlan") {
        node = inner;
    }
    let mut parts = Vec::new();
    loop {
        let stage = node.get_str("stage").unwrap_or("?");
        match node.get_str("indexName") {
            Ok(name) => parts.push(format!("{stage}({name})")),
            Err(_) => parts.push(stage.to_string()),
        }
        if let Ok(next) = node.get_document("inputStage") {
            node = next;
            continue;
        }
        if let Ok(branches) = node.get_array("inputStages") {
            let inner: Vec<String> = branches
                .iter()
                .filter_map(Bson::as_document)
                .map(plan_chain)
                .collect();
            parts.push(format!("[{}]", inner.join("|")));
        }
        break;
    }
    parts.join("<-")
}

/// Explains `inner` (a `find`, `aggregate` or `delete` command body) at
/// `executionStats` verbosity and reads the winning plan and the totals.
///
/// An aggregate explains as `stages[].$cursor` when the cursor stage is a
/// separate stage, and at the top level when the whole pipeline was pushed down
/// into the plan; both are read.
async fn explain_measured(
    db: &mongodb::Database,
    reader: &str,
    op: &str,
    plan_summary: &str,
    inner: Document,
) -> Measured {
    let explain = db
        .run_command(doc! { "explain": inner, "verbosity": "executionStats" })
        .await
        .unwrap_or_else(|e| panic!("explain for {reader} failed: {e}"));
    let source = explain
        .get_array("stages")
        .ok()
        .and_then(|stages| {
            stages
                .iter()
                .find_map(|stage| stage.as_document()?.get_document("$cursor").ok())
        })
        .unwrap_or(&explain);
    let winning = source
        .get_document("queryPlanner")
        .and_then(|planner| planner.get_document("winningPlan"))
        .unwrap_or_else(|e| panic!("explain for {reader} has no winningPlan: {e}: {explain:?}"));
    let stats = source
        .get_document("executionStats")
        .unwrap_or_else(|e| panic!("explain for {reader} has no executionStats: {e}"));
    let mut indexes = Vec::new();
    collect_index_names(winning, &mut indexes);
    let mut seen = Vec::new();
    indexes.retain(|name| {
        let new = !seen.contains(name);
        seen.push(name.clone());
        new
    });
    Measured {
        reader: reader.to_string(),
        op: op.to_string(),
        indexes,
        chain: plan_chain(winning),
        plan_summary: plan_summary.to_string(),
        keys: number(stats, "totalKeysExamined").expect("totalKeysExamined"),
        docs: number(stats, "totalDocsExamined").expect("totalDocsExamined"),
        returned: number(stats, "nReturned").unwrap_or(0),
        sort: contains_stage_named(winning, "SORT"),
        collscan: contains_stage_named(winning, "COLLSCAN") || plan_summary.contains("COLLSCAN"),
        deleted: None,
        inserted: None,
    }
}

/// Turns one profiler entry into a [`Measured`], `None` for a command the probe
/// does not measure.
///
/// A `find` or aggregate is replayed under explain when `replay` is set. A
/// reader that changes the data it reads (`purge`) is not replayed: the replay
/// would see the rows already gone, so such an entry is measured from its own
/// `keysExamined`, `docsExamined` and `planSummary`, as a `delete` always is.
/// Those counts are complete for a single-batch command like `count_documents`.
async fn measure_entry(
    db: &mongodb::Database,
    patterns: &[(String, String)],
    reader: &str,
    entry: &Document,
    replay: bool,
) -> Option<Measured> {
    let op = entry.get_str("op").unwrap_or("?");
    let plan_summary = entry.get_str("planSummary").unwrap_or_default();
    match op {
        "query" | "command" if replay => {
            let command = entry.get_document("command").ok()?;
            if !command.contains_key("find") && !command.contains_key("aggregate") {
                println!(
                    "HISTORY_PLAN_SKIPPED reader={reader} op={op} command={:?}",
                    command.keys().next()
                );
                return None;
            }
            Some(explain_measured(db, reader, op, plan_summary, command_for_explain(command)).await)
        }
        "query" | "command" | "remove" => {
            let indexes = patterns
                .iter()
                .filter(|(_, pattern)| plan_summary.contains(pattern.as_str()))
                .map(|(name, _)| name.clone())
                .collect();
            let deleted = number(entry, "ndeleted");
            Some(Measured {
                reader: reader.to_string(),
                op: op.to_string(),
                indexes,
                chain: "profile-entry".to_string(),
                plan_summary: plan_summary.to_string(),
                keys: number(entry, "keysExamined").unwrap_or(0),
                docs: number(entry, "docsExamined").unwrap_or(0),
                returned: deleted.or(number(entry, "nreturned")).unwrap_or(0),
                sort: entry.get_bool("hasSortStage").unwrap_or(false),
                collscan: plan_summary.contains("COLLSCAN"),
                deleted,
                inserted: None,
            })
        }
        "insert" => Some(Measured {
            reader: reader.to_string(),
            op: op.to_string(),
            indexes: Vec::new(),
            chain: "profile-entry".to_string(),
            plan_summary: plan_summary.to_string(),
            keys: 0,
            docs: 0,
            returned: number(entry, "ninserted").unwrap_or(0),
            sort: false,
            collscan: false,
            deleted: None,
            inserted: number(entry, "keysInserted"),
        }),
        _ => None,
    }
}

/// One leg's measurements, appended reader by reader.
struct Leg<'a> {
    name: &'static str,
    db: &'a mongodb::Database,
    patterns: Vec<(String, String)>,
    measured: Vec<Measured>,
}

impl Leg<'_> {
    fn record(&mut self, mut entries: Vec<Measured>) {
        if entries.len() > 1 {
            for (i, entry) in entries.iter_mut().enumerate() {
                entry.reader = format!("{}#{}", entry.reader, i + 1);
            }
        }
        for entry in entries {
            println!("{}", entry.line(self.name));
            self.measured.push(entry);
        }
    }

    /// Calls a reader under the profiler and measures what it sent to
    /// `resource_history`, replaying its finds and aggregates under explain.
    /// `None` when the profiler was refused.
    async fn run<T>(
        &mut self,
        reader: &str,
        call: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        self.run_with(reader, true, call).await
    }

    /// [`Self::run`] for a reader that changes what it reads: its entries are
    /// measured as the profiler recorded them (see [`measure_entry`]).
    async fn run_destructive<T>(
        &mut self,
        reader: &str,
        call: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        self.run_with(reader, false, call).await
    }

    async fn run_with<T>(
        &mut self,
        reader: &str,
        replay: bool,
        call: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        let (value, entries) = profiled(self.db, call).await?;
        let mut measured = Vec::new();
        for entry in &entries {
            if let Some(m) = measure_entry(self.db, &self.patterns, reader, entry, replay).await {
                measured.push(m);
            }
        }
        assert!(
            !measured.is_empty(),
            "reader {reader} sent no measurable command to resource_history \
             ({} profiler entries)",
            entries.len()
        );
        self.record(measured);
        Some(value)
    }

    /// Explains a `delete_many` filter without running it.
    async fn explain_delete(&mut self, reader: &str, filter: Document) {
        let measured = explain_measured(
            self.db,
            reader,
            "explain-delete",
            "",
            doc! {
                "delete": "resource_history",
                "deletes": [{ "q": filter, "limit": 0_i32 }],
            },
        )
        .await;
        self.record(vec![measured]);
    }
}

/// Runs every reader once. `purge_target` is the id of the `Basic` resource this
/// leg purges. `None` when the profiler is refused.
async fn measure_leg(
    backend: &MongoBackend,
    db: &mongodb::Database,
    name: &'static str,
    windows: &Windows,
    purge_target: &str,
) -> Option<Vec<Measured>> {
    let tenant = create_tenant(TENANT);
    let mut leg = Leg {
        name,
        db,
        patterns: index_patterns(db).await,
        measured: Vec::new(),
    };
    let rest = HistoryParams::new().count(PAGE).include_deleted(true);

    let version = leg
        .run("vread", backend.vread(&tenant, "Patient", HOT_ID, "150"))
        .await?;
    assert!(version.expect("vread").is_some(), "hot v150 must exist");

    let versions = leg
        .run(
            "list_versions",
            backend.list_versions(&tenant, "Patient", HOT_ID),
        )
        .await?;
    assert_eq!(versions.expect("list_versions").len() as i64, HOT_VERSIONS);

    let page1 = leg
        .run(
            "history_instance_all",
            backend.history_instance(&tenant, "Patient", HOT_ID, &rest),
        )
        .await?
        .expect("history_instance");
    assert_eq!(page1.items.len(), 50);
    assert!(page1.page_info.has_next, "300 versions page at 50");

    let mut page2_params = rest.clone();
    page2_params.pagination = Pagination::with_cursor(
        PAGE,
        page1.page_info.next_cursor.clone().expect("next cursor"),
    );
    let page2 = leg
        .run(
            "history_instance_page2",
            backend.history_instance(&tenant, "Patient", HOT_ID, &page2_params),
        )
        .await?
        .expect("history_instance page 2");
    assert_eq!(page2.items.len(), 50);

    let live = HistoryParams::new().count(PAGE);
    leg.run(
        "history_instance_live",
        backend.history_instance(&tenant, "Patient", HOT_ID, &live),
    )
    .await?
    .expect("history_instance live");

    let since = rest.clone().since(windows.since);
    let since_page = leg
        .run(
            "history_instance_since",
            backend.history_instance(&tenant, "Patient", HOT_ID, &since),
        )
        .await?
        .expect("history_instance since");
    assert!(!since_page.items.is_empty(), "the since window has rows");

    let wide = rest.clone().since(windows.wide_since);
    let wide_page = leg
        .run(
            "history_instance_since_wide",
            backend.history_instance(&tenant, "Patient", HOT_ID, &wide),
        )
        .await?
        .expect("history_instance since the whole type");
    assert_eq!(wide_page.items.len(), 50, "a page of the 300 versions");

    let before = rest.clone().before(windows.before);
    let before_page = leg
        .run(
            "history_instance_before",
            backend.history_instance(&tenant, "Patient", HOT_ID, &before),
        )
        .await?
        .expect("history_instance before");
    assert_eq!(before_page.items.len(), 50, "hot v1..=v50");

    let counted = leg
        .run(
            "history_instance_count",
            backend.history_instance_count(&tenant, "Patient", HOT_ID),
        )
        .await?
        .expect("history_instance_count");
    assert_eq!(counted as i64, HOT_VERSIONS);

    let type_params = HistoryParams::new().count(10).include_deleted(true);
    leg.run(
        "history_type",
        backend.history_type(&tenant, "Patient", &type_params),
    )
    .await?
    .expect("history_type");
    leg.run(
        "history_type_since",
        backend.history_type(
            &tenant,
            "Patient",
            &type_params.clone().since(windows.since),
        ),
    )
    .await?
    .expect("history_type since");
    leg.run(
        "history_type_count",
        backend.history_type_count(&tenant, "Patient"),
    )
    .await?
    .expect("history_type_count");
    leg.run(
        "history_system",
        backend.history_system(&tenant, &type_params),
    )
    .await?
    .expect("history_system");
    leg.run(
        "history_system_since",
        backend.history_system(&tenant, &type_params.clone().since(windows.since)),
    )
    .await?
    .expect("history_system since");
    leg.run(
        "history_system_count",
        backend.history_system_count(&tenant),
    )
    .await?
    .expect("history_system_count");

    leg.run(
        "count_deltas_by_bucket",
        backend.count_deltas_by_bucket(&tenant, "Patient", windows.since, 60),
    )
    .await?
    .expect("count_deltas_by_bucket");
    leg.run(
        "count_deltas_by_type_and_bucket",
        backend.count_deltas_by_type_and_bucket(
            &tenant,
            &["Patient", "Observation"],
            windows.since,
            60,
        ),
    )
    .await?
    .expect("count_deltas_by_type_and_bucket");
    leg.run(
        "activity_histogram",
        backend.activity_histogram(&tenant, windows.since),
    )
    .await?
    .expect("activity_histogram");
    leg.run(
        "latest_write_marker",
        backend.latest_write_marker(&tenant, Some(windows.since)),
    )
    .await?
    .expect("latest_write_marker");

    // The type and tenant purges delete; explain does not run the delete.
    let other = OTHER_TENANT;
    leg.explain_delete(
        "purge_all_delete",
        doc! { "tenant_id": other, "resource_type": "Patient" },
    )
    .await;
    leg.explain_delete("purge_tenant_delete", doc! { "tenant_id": other })
        .await;

    leg.run_destructive("purge", backend.purge(&tenant, PURGE_TYPE, purge_target))
        .await?
        .expect("purge");

    // A write, for the number of index keys one history insert costs.
    leg.run(
        "keys_inserted",
        backend.create(
            &create_tenant(KEYS_TENANT),
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Keys"}]}),
            FhirVersion::default(),
        ),
    )
    .await?
    .expect("create for keysInserted");

    Some(leg.measured)
}

/// The measurements of `reader` (`reader` itself or `reader#n`).
fn select<'a>(rows: &'a [Measured], reader: &str) -> Vec<&'a Measured> {
    let numbered = format!("{reader}#");
    rows.iter()
        .filter(|m| m.reader == reader || m.reader.starts_with(&numbered))
        .collect()
}

/// `after` does not examine more than `before`, with a little slack for the
/// rows the profiled call itself or the planner's choice can move by.
fn not_worse(before: i64, after: i64) -> bool {
    after <= before + before / 100 + 2
}

/// One failure string: which leg and reader, why, and the measured line.
fn failure(leg: &str, m: &Measured, why: String) -> String {
    format!("{leg}/{}: {why}\n    {}", m.reader, m.line(leg))
}

/// The failure strings of the acceptance checks; empty when the index is not
/// needed.
fn check_legs(before: &[Measured], after: &[Measured]) -> Vec<String> {
    let mut failures = Vec::new();

    // Readers of one resource's rows: the unique identity index serves them,
    // and none reads more than that resource's versions. For instance history
    // the identity index serves the covered version-id read and the fetches,
    // and the page-size bound on the documents is checked below.
    for (reader, max_keys, max_docs) in [
        ("vread", 2, 1),
        ("list_versions", HOT_VERSIONS + 1, HOT_VERSIONS),
        ("history_instance_all", HOT_VERSIONS + 1, HOT_VERSIONS),
        ("history_instance_page2", HOT_VERSIONS + 1, HOT_VERSIONS),
        ("history_instance_live", HOT_VERSIONS + 1, HOT_VERSIONS),
        ("history_instance_count", HOT_VERSIONS + 1, HOT_VERSIONS),
        ("purge", PURGE_VERSIONS + 1, PURGE_VERSIONS),
    ] {
        let rows = select(after, reader);
        if rows.is_empty() {
            failures.push(format!("after/{reader}: not measured"));
        }
        for m in rows {
            if !m.uses(IDENTITY_INDEX) {
                failures.push(failure(
                    "after",
                    m,
                    format!("does not use {IDENTITY_INDEX}"),
                ));
            }
            if m.uses(OLD_INDEX) {
                failures.push(failure("after", m, format!("still names {OLD_INDEX}")));
            }
            if m.collscan {
                failures.push(failure("after", m, "collection scan".to_string()));
            }
            if m.sort {
                failures.push(failure("after", m, "blocking SORT".to_string()));
            }
            if m.keys > max_keys || m.docs > max_docs {
                failures.push(failure(
                    "after",
                    m,
                    format!("examines more than {max_keys} keys / {max_docs} docs"),
                ));
            }
        }
    }

    // Time-filtered instance history: any index but the dropped one, bounded by
    // one resource's versions.
    for reader in [
        "history_instance_since",
        "history_instance_since_wide",
        "history_instance_before",
    ] {
        let rows = select(after, reader);
        if rows.is_empty() {
            failures.push(format!("after/{reader}: not measured"));
        }
        for m in rows {
            if m.uses(OLD_INDEX) {
                failures.push(failure("after", m, format!("still names {OLD_INDEX}")));
            }
            if m.collscan {
                failures.push(failure("after", m, "collection scan".to_string()));
            }
            if m.sort {
                failures.push(failure("after", m, "blocking SORT".to_string()));
            }
            if m.keys > HOT_VERSIONS + 1 || m.docs > HOT_VERSIONS {
                failures.push(failure(
                    "after",
                    m,
                    format!(
                        "examines more than one resource's versions ({} keys / {} docs)",
                        HOT_VERSIONS + 1,
                        HOT_VERSIONS
                    ),
                ));
            }
        }
    }

    // #2375: an instance-history page reads the resource's version ids from
    // idx_history_identity without touching a document, then fetches about one
    // page of documents. `before` still has to look at every version, because
    // its window holds the oldest ones.
    const PAGE_DOCS: i64 = PAGE as i64 + 1;
    for (reader, max_docs) in [
        ("history_instance_all", PAGE_DOCS),
        ("history_instance_page2", PAGE_DOCS),
        ("history_instance_live", PAGE_DOCS),
        ("history_instance_since", PAGE_DOCS),
        ("history_instance_since_wide", PAGE_DOCS),
        ("history_instance_before", HOT_VERSIONS),
    ] {
        let rows = select(after, reader);
        // The documents of every command of the reader, also when it has fewer
        // commands than expected: a reader that went back to one command which
        // examines the whole resource must fail this bound too.
        let total: i64 = rows.iter().map(|m| m.docs).sum();
        if total > max_docs {
            let why = format!(
                "examines {total} documents over {} commands, more than {max_docs}",
                rows.len()
            );
            match rows.last() {
                Some(last) => failures.push(failure("after", last, why)),
                None => failures.push(format!("after/{reader}: {why}")),
            }
        }
        if rows.len() < 2 {
            failures.push(format!(
                "after/{reader}: expected a version-id read and at least one document fetch, \
                 got {} command(s)",
                rows.len()
            ));
            continue;
        }
        let key_read = rows[0];
        if !key_read.uses(IDENTITY_INDEX) || key_read.docs != 0 || key_read.chain.contains("FETCH")
        {
            failures.push(failure(
                "after",
                key_read,
                format!("the version-id read is not a covered read of {IDENTITY_INDEX}"),
            ));
        }
        for fetch in &rows[1..] {
            if fetch.keys > 2 * fetch.docs + 2 {
                failures.push(failure(
                    "after",
                    fetch,
                    format!(
                        "a document fetch examines {} keys for {} documents",
                        fetch.keys, fetch.docs
                    ),
                ));
            }
        }
    }

    // Type, system, dashboard and whole-type or whole-tenant purge readers: no
    // collection scan, and no more keys or docs than with the index present.
    // The find readers also keep their index-order sort.
    for (reader, sorted) in [
        ("history_type", true),
        ("history_type_since", true),
        ("history_type_count", false),
        ("history_system", true),
        ("history_system_since", true),
        ("history_system_count", false),
        ("count_deltas_by_bucket", false),
        ("count_deltas_by_type_and_bucket", false),
        ("activity_histogram", false),
        ("latest_write_marker", false),
        ("purge_all_delete", false),
        ("purge_tenant_delete", false),
    ] {
        let after_rows = select(after, reader);
        let before_rows = select(before, reader);
        if after_rows.is_empty() || after_rows.len() != before_rows.len() {
            failures.push(format!("{reader}: not measured in both legs"));
            continue;
        }
        for (b, a) in before_rows.iter().zip(&after_rows) {
            if a.collscan {
                failures.push(failure("after", a, "collection scan".to_string()));
            }
            if sorted && a.sort {
                failures.push(failure("after", a, "blocking SORT".to_string()));
            }
            if !not_worse(b.keys, a.keys) || !not_worse(b.docs, a.docs) {
                failures.push(failure(
                    "after",
                    a,
                    format!(
                        "examines more than before ({} keys / {} docs)",
                        b.keys, b.docs
                    ),
                ));
            }
        }
    }

    // The readers the issue asks to confirm never use the dropped index, so
    // dropping it cannot move them.
    for reader in [
        "history_type",
        "history_type_since",
        "history_system",
        "history_system_since",
        "count_deltas_by_bucket",
        "count_deltas_by_type_and_bucket",
        "activity_histogram",
        "latest_write_marker",
    ] {
        for m in select(before, reader) {
            if m.uses(OLD_INDEX) {
                failures.push(failure("before", m, format!("uses {OLD_INDEX}")));
            }
        }
    }

    // Purge counts and deletes every version of its target in both legs.
    for (leg, rows) in [("before", before), ("after", after)] {
        for m in select(rows, "purge") {
            if m.keys < PURGE_VERSIONS {
                failures.push(failure(
                    leg,
                    m,
                    format!("examined fewer than the {PURGE_VERSIONS} versions of its target"),
                ));
            }
            if m.op == "remove" && m.deleted != Some(PURGE_VERSIONS) {
                failures.push(failure(
                    leg,
                    m,
                    format!("deleted {:?}, expected {PURGE_VERSIONS}", m.deleted),
                ));
            }
        }
    }

    // One history insert writes one key fewer.
    let inserted_before = select(before, "keys_inserted")
        .iter()
        .find_map(|m| m.inserted);
    let inserted_after = select(after, "keys_inserted")
        .iter()
        .find_map(|m| m.inserted);
    match (inserted_before, inserted_after) {
        (Some(b), Some(a)) => {
            if a != b - 1 {
                failures.push(format!(
                    "keysInserted of one history insert: before {b}, after {a}, expected {}",
                    b - 1
                ));
            }
        }
        _ => failures.push(format!(
            "keysInserted missing on the history insert profile entry \
             (before {inserted_before:?}, after {inserted_after:?})"
        )),
    }

    failures
}

/// #2353: with `idx_history_resource_updated` dropped, every `resource_history`
/// reader keeps a bounded, non-blocking plan: the readers of one resource's rows
/// use `idx_history_identity` and read at most that resource's versions; the
/// type, system and dashboard readers never used the dropped index and examine
/// no more than before.
#[tokio::test]
async fn history_reads_keep_bounded_plans_without_idx_history_resource_updated() {
    let Some(backend) = create_backend("hist_idx_plans").await else {
        eprintln!(
            "Skipping history_reads_keep_bounded_plans_without_idx_history_resource_updated \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("raw MongoDB client");
    let db = client.database(&backend.config().database_name);
    let history: Collection<Document> = db.collection("resource_history");

    let windows = seed(&backend, &history).await;
    create_old_index(&history).await;
    let indexes = index_names_of(&history).await;
    assert!(
        indexes.iter().any(|name| name == OLD_INDEX),
        "the before leg needs {OLD_INDEX}: {indexes:?}"
    );
    println!(
        "HISTORY_PLAN_SETUP tenant_rows={} other_tenant_rows={} hot_versions={HOT_VERSIONS} \
         filler_patients={FILLER_PATIENTS} filler_observations={FILLER_OBSERVATIONS} \
         purge_versions={PURGE_VERSIONS} indexes_before={indexes:?}",
        history
            .count_documents(doc! { "tenant_id": TENANT })
            .await
            .unwrap(),
        history
            .count_documents(doc! { "tenant_id": OTHER_TENANT })
            .await
            .unwrap(),
    );

    let Some(before) = measure_leg(&backend, &db, "before", &windows, "purge-before").await else {
        return;
    };

    drop_test_index(&backend, "resource_history", OLD_INDEX).await;
    let indexes = index_names_of(&history).await;
    assert!(
        !indexes.iter().any(|name| name == OLD_INDEX),
        "{OLD_INDEX} must be gone for the after leg: {indexes:?}"
    );
    println!("HISTORY_PLAN_SETUP indexes_after={indexes:?}");

    let Some(after) = measure_leg(&backend, &db, "after", &windows, "purge-after").await else {
        return;
    };

    // One comparison line per reader, for the record.
    for m in &after {
        if let Some(b) = select(&before, &m.reader).first() {
            println!(
                "HISTORY_PLAN_DIFF reader={} indexes={}->{} keys={}->{} docs={}->{}",
                m.reader,
                b.indexes.join("|"),
                m.indexes.join("|"),
                b.keys,
                m.keys,
                b.docs,
                m.docs,
            );
        }
    }
    println!(
        "HISTORY_PLAN_SETUP type_index={TYPE_INDEX} system_index={SYSTEM_INDEX} \
         readers_using_old_index_before={:?}",
        before
            .iter()
            .filter(|m| m.uses(OLD_INDEX))
            .map(|m| m.reader.as_str())
            .collect::<Vec<_>>()
    );

    let failures = check_legs(&before, &after);
    assert!(
        failures.is_empty(),
        "history readers regress without {OLD_INDEX}:\n{}",
        failures.join("\n")
    );
}

/// The `resource_history` indexes a boot leaves, sorted.
fn expected_history_indexes() -> Vec<String> {
    let mut names: Vec<String> = ["_id_", IDENTITY_INDEX, TYPE_INDEX, SYSTEM_INDEX]
        .into_iter()
        .map(str::to_string)
        .collect();
    names.sort();
    names
}

/// #2353: schema init no longer creates `idx_history_resource_updated` and drops
/// it where an earlier build created it: on a fresh database, on a store that
/// has it (with a key for a stored version), and in `HFS_MONGODB_INDEX_BUILD=off`
/// mode, which defers only the `search_index` builds. The unique identity index
/// is unchanged, and boot records this build's `SCHEMA_VERSION` (the drop needs
/// no version change).
#[tokio::test]
async fn schema_init_drops_idx_history_resource_updated() {
    let Some(backend) = create_backend("hist_idx_drop").await else {
        eprintln!(
            "Skipping schema_init_drops_idx_history_resource_updated \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let config = backend.config().clone();
    let client = raw_test_client(&config.connection_string)
        .await
        .expect("raw MongoDB client");
    let db = client.database(&config.database_name);
    let history: Collection<Document> = db.collection("resource_history");

    // A fresh database: the three remaining indexes, and the unique identity
    // index exactly as before.
    assert_eq!(index_names_of(&history).await, expected_history_indexes());
    let models: Vec<mongodb::IndexModel> = history
        .list_indexes()
        .await
        .expect("list indexes")
        .try_collect()
        .await
        .expect("collect indexes");
    let identity = models
        .iter()
        .find(|model| {
            model.options.as_ref().and_then(|o| o.name.as_deref()) == Some(IDENTITY_INDEX)
        })
        .expect("the identity index exists");
    assert_eq!(
        identity.keys,
        doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "id": 1_i32, "version_id": 1_i32 }
    );
    assert_eq!(
        identity.options.as_ref().and_then(|o| o.unique),
        Some(true),
        "idx_history_identity stays unique"
    );

    // A store an earlier build initialised: the index exists and holds a key
    // for a stored version.
    let tenant = create_tenant("tenant-hist-idx-drop");
    let created = backend
        .seed_create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Dropped"}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    create_old_index(&history).await;
    assert!(
        index_names_of(&history)
            .await
            .contains(&OLD_INDEX.to_string())
    );

    // Boot again (inline build mode, as the test harness forces): dropped.
    let rebooted = build_backend(config.clone())
        .await
        .expect("mongo stays available");
    assert_eq!(index_names_of(&history).await, expected_history_indexes());

    // Boot in `off` mode: the drop is a catalog change and is not deferred.
    create_old_index(&history).await;
    assert!(
        index_names_of(&history)
            .await
            .contains(&OLD_INDEX.to_string())
    );
    let off = MongoBackend::new(MongoBackendConfig {
        index_build: IndexBuildMode::Off,
        ..config.clone()
    })
    .expect("backend for off mode");
    off.initialize().await.expect("boot in off mode");
    assert_eq!(
        index_names_of(&history).await,
        expected_history_indexes(),
        "off mode must drop the index too"
    );

    // History reads still work, and boot records this build's schema version
    // (the drop needs no version change).
    assert_eq!(
        rebooted
            .list_versions(&tenant, "Patient", created.id())
            .await
            .expect("list_versions"),
        vec!["1".to_string()]
    );
    assert!(
        off.vread(&tenant, "Patient", created.id(), "1")
            .await
            .expect("vread")
            .is_some()
    );
    let version = db
        .collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await
        .expect("read schema_version")
        .expect("schema_version document");
    assert_eq!(
        version.get_i32("version").expect("version"),
        helios_persistence::backends::mongodb::SCHEMA_VERSION
    );
}
