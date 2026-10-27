//! #1977: a transaction Bundle stages its creates and sends them in chunks,
//! with no existence read, instead of 3 to 7 commands per create.
//!
//! Child module of the `mongodb_tests` root; `use super::*;` reaches its
//! private harness and the helpers `deferred_search_index.rs` shares.
//!
//! What the Bundle's transaction sent is read from the server's own profiler
//! (level 2 on the test's database, see [`TxnProfile`]): the oplog shows
//! neither reads nor the deletes that match nothing, which are exactly what this
//! change removes or batches. The transaction is found by the `lsid` and
//! `txnNumber` of its `resources` inserts, so the post-commit batches of a
//! deferred Bundle, which are transactions of their own, are not counted. A
//! positive control (the documents the inserts report) keeps the zero counts
//! from being vacuous.
//!
//! The behaviour that has to survive the change is pinned next to the counts: a
//! reused id (stored, deleted, twice in one chunk, twice across a chunk
//! boundary) still fails at its own entry with the same message, an entry that
//! reads (`GET`, `ifNoneExist`, `PUT`, `PATCH`, `DELETE`) sees the creates
//! before it, and a Bundle of several chunks ends with one version and one row
//! set per resource.

use super::*;

use std::collections::{BTreeMap, HashMap};

use futures::TryStreamExt;

use crate::bulk_submit::FailPoint;

use super::deferred_search_index::{
    MRN_SYSTEM, THRESHOLD, deferred_backend, lacks_transactions, ledger_count, located,
    marker_count, medication_request_json, normalized_fingerprints, observation_json,
    patient_bundle, patient_json, patients_by_family, patients_by_mrn, post, post_if_none_exist,
    raw_db, reference_param, row_counts,
};

/// Observations in the Bundle of the command-count tests: 1 Patient + 205 + 1
/// MedicationRequest = 207 POSTs, which are chunks of 100, 100 and 7.
const OBSERVATIONS: usize = 205;
const ENTRIES: usize = OBSERVATIONS + 2;
const CHUNKS: usize = 3;

// ---------------------------------------------------------------------------
// Backends
// ---------------------------------------------------------------------------

/// How the backend handles the Bundle's search rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Rows are written in the transaction (the deferral is off).
    Inline,
    /// A Bundle of [`THRESHOLD`] entries or more writes its rows after the
    /// commit (#1975).
    Deferred,
    /// Search is offloaded: no rows are written at all.
    Offloaded,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::Inline => "inline",
            Mode::Deferred => "deferred",
            Mode::Offloaded => "offloaded",
        }
    }
}

async fn chunk_backend(test: &str, mode: Mode) -> Option<MongoBackend> {
    let min_entries = match mode {
        Mode::Inline => Some(0),
        Mode::Deferred => Some(THRESHOLD),
        Mode::Offloaded => None,
    };
    deferred_backend(test, test, min_entries, |config| {
        config.search_offloaded = mode == Mode::Offloaded;
    })
    .await
}

/// A backend for `test` in `mode`, or `None` (after saying so) when the shared
/// MongoDB is unavailable.
async fn backend_or_skip(test: &str, mode: Mode) -> Option<MongoBackend> {
    let backend = chunk_backend(test, mode).await;
    if backend.is_none() {
        eprintln!("Skipping {test} (requires Docker or HFS_TEST_MONGODB_URL)");
    }
    backend
}

// ---------------------------------------------------------------------------
// The profiler
// ---------------------------------------------------------------------------

/// Turns the profiler on for `db`, with a `system.profile` big enough to hold
/// every command of the test. `false` when the server refuses (an external
/// server without the privilege); the harness's own server must not.
async fn start_profiling(db: &mongodb::Database) -> bool {
    let started = async {
        db.run_command(doc! { "profile": 0_i32 }).await?;
        // An operation still in flight from an earlier capture recreates the
        // default collection when it finishes; drop it again and retry.
        for attempt in 1.. {
            db.collection::<Document>("system.profile").drop().await?;
            match db
                .create_collection("system.profile")
                .capped(true)
                .size(16 * 1024 * 1024)
                .await
            {
                Ok(()) => break,
                Err(e)
                    if attempt < 10
                        && matches!(
                            e.kind.as_ref(),
                            mongodb::error::ErrorKind::Command(c) if c.code == 48
                        ) => {}
                Err(e) => return Err(e),
            }
        }
        db.run_command(doc! { "profile": 2_i32 }).await
    }
    .await;
    match started {
        Ok(_) => true,
        Err(e) => {
            assert!(
                !transactions_required(),
                "the harness's own MongoDB must allow the profiler: {e}"
            );
            eprintln!("the profiler is refused here ({e}); skipping the command-count assertions");
            false
        }
    }
}

/// The numeric field `field` of a profile entry, 0 when it has none.
fn numeric(entry: &Document, field: &str) -> i64 {
    entry
        .get_i64(field)
        .or_else(|_| entry.get_i32(field).map(i64::from))
        .unwrap_or(0)
}

/// The `(lsid, txnNumber)` of the transaction a profile entry ran in, `Null`
/// for each part it has none of (a command outside a transaction has no
/// `txnNumber`). The server records them in the entry's `command`; later
/// versions also at the top level.
fn transaction_of(entry: &Document) -> (Bson, Bson) {
    let field = |name: &str| {
        entry
            .get(name)
            .or_else(|| entry.get_document("command").ok()?.get(name))
            .cloned()
            .unwrap_or(Bson::Null)
    };
    (field("lsid"), field("txnNumber"))
}

/// Every command the Bundle's transaction ran, from `system.profile`.
struct TxnProfile {
    db: String,
    ops: Vec<Document>,
}

impl TxnProfile {
    /// Stops the profiler and picks out the Bundle's transaction: the one that
    /// inserted into `resources`. A Bundle that was replayed has more than one,
    /// which fails the test: the counts below are for a single attempt.
    async fn of_bundle(db: &mongodb::Database) -> TxnProfile {
        db.run_command(doc! { "profile": 0_i32 }).await.unwrap();
        let profiled: Vec<Document> = db
            .collection::<Document>("system.profile")
            .find(doc! {})
            .sort(doc! { "ts": 1_i32 })
            .await
            .expect("read system.profile")
            .try_collect()
            .await
            .expect("read system.profile");
        let ns = format!("{}.resources", db.name());
        let mut transactions: Vec<(Bson, Bson)> = Vec::new();
        for entry in profiled
            .iter()
            .filter(|e| e.get_str("op").is_ok_and(|op| op == "insert"))
            .filter(|e| e.get_str("ns").is_ok_and(|v| v == ns))
        {
            let key = transaction_of(entry);
            assert!(
                key.1 != Bson::Null,
                "a Bundle's `resources` insert runs in a transaction, but the profile entry has \
                 no txnNumber: {entry:?}"
            );
            if !transactions.contains(&key) {
                transactions.push(key);
            }
        }
        assert_eq!(
            transactions.len(),
            1,
            "expected the profile to hold exactly one transaction that inserted into `resources`"
        );
        let key = transactions.remove(0);
        let (mut ops, rest): (Vec<Document>, Vec<Document>) =
            profiled.into_iter().partition(|e| transaction_of(e) == key);
        // The server records a delete as the bare statement, with nothing of the
        // transaction it ran in (5.0), so its deletes are the ones that ran
        // between the transaction's first and last command. The database is the
        // test's own, and a deferred Bundle's post-commit batches start after the
        // commit.
        let at = |e: &Document| e.get_datetime("ts").map(|ts| ts.timestamp_millis());
        let first = ops.iter().filter_map(|e| at(e).ok()).min();
        let last = ops.iter().filter_map(|e| at(e).ok()).max();
        if let (Some(first), Some(last)) = (first, last) {
            ops.extend(rest.into_iter().filter(|e| {
                e.get_str("op").is_ok_and(|op| op == "remove")
                    && at(e).is_ok_and(|ts| (first..=last).contains(&ts))
            }));
        }
        TxnProfile {
            db: db.name().to_string(),
            ops,
        }
    }

    fn on(&self, collection: &str, op: &str) -> Vec<&Document> {
        let ns = format!("{}.{collection}", self.db);
        self.ops
            .iter()
            .filter(|e| e.get_str("ns").is_ok_and(|v| v == ns))
            .filter(|e| e.get_str("op").is_ok_and(|v| v == op))
            .collect()
    }

    /// Commands of kind `op` (`insert`, `remove`) on `collection`.
    fn commands(&self, collection: &str, op: &str) -> usize {
        self.on(collection, op).len()
    }

    /// Documents those commands inserted (`ninserted`).
    fn inserted(&self, collection: &str) -> i64 {
        self.on(collection, "insert")
            .into_iter()
            .map(|e| numeric(e, "ninserted"))
            .sum()
    }

    /// `find` commands on `collection`.
    fn finds(&self, collection: &str) -> usize {
        self.ops
            .iter()
            .filter(|e| e.get_str("op").is_ok_and(|op| op == "query"))
            .filter(|e| {
                e.get_document("command")
                    .is_ok_and(|c| c.get_str("find").is_ok_and(|f| f == collection))
            })
            .count()
    }

    /// `op ns` -> count, for an assertion message.
    fn summary(&self) -> String {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &self.ops {
            let key = format!(
                "{} {}",
                entry.get_str("op").unwrap_or("?"),
                entry
                    .get_str("ns")
                    .unwrap_or("?")
                    .trim_start_matches(&format!("{}.", self.db))
            );
            *counts.entry(key).or_default() += 1;
        }
        format!("{counts:?}")
    }

    /// The commands of a POST-only Bundle that sends `ENTRIES` creates in
    /// `CHUNKS` chunks, whatever it does with search rows.
    fn assert_create_chunks(&self) {
        let context = self.summary();
        // Positive control: the inserts really carried the Bundle's documents,
        // so the zero counts below are about this transaction and not an empty one.
        assert_eq!(
            self.inserted("resources"),
            ENTRIES as i64,
            "resources documents inserted: {context}"
        );
        assert_eq!(
            self.commands("resources", "insert"),
            CHUNKS,
            "resources insert commands: {context}"
        );
        assert_eq!(
            self.inserted("resource_history"),
            ENTRIES as i64,
            "history documents inserted: {context}"
        );
        assert_eq!(
            self.commands("resource_history", "insert"),
            CHUNKS,
            "history insert commands: {context}"
        );
        assert_eq!(
            self.finds("resources"),
            0,
            "no existence read on `resources`: {context}"
        );
    }

    fn assert_no_search_writes(&self) {
        let context = self.summary();
        for collection in ["search_index", "search_index_contained"] {
            assert_eq!(
                self.commands(collection, "insert"),
                0,
                "{collection} inserts: {context}"
            );
            assert_eq!(
                self.commands(collection, "remove"),
                0,
                "{collection} removes: {context}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// What the database holds
// ---------------------------------------------------------------------------

/// Rows per `resource_id` of the tenant in one search collection.
async fn rows_by_resource(
    backend: &MongoBackend,
    tenant_id: &str,
    collection: &str,
) -> HashMap<String, i64> {
    let rows: Vec<Document> = raw_db(backend)
        .await
        .collection::<Document>(collection)
        .aggregate(vec![
            doc! { "$match": { "tenant_id": tenant_id } },
            doc! { "$group": { "_id": "$resource_id", "n": { "$sum": 1_i32 } } },
        ])
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    rows.iter()
        .map(|row| (row.get_str("_id").unwrap().to_string(), numeric(row, "n")))
        .collect()
}

/// `(resources, resource_history, search_index, search_index_contained)`
/// documents of the tenant.
async fn store_counts(backend: &MongoBackend, tenant_id: &str) -> [u64; 4] {
    let mut counts = [0; 4];
    for (n, collection) in [
        "resources",
        "resource_history",
        "search_index",
        "search_index_contained",
    ]
    .into_iter()
    .enumerate()
    {
        counts[n] = count_docs(backend, collection, doc! { "tenant_id": tenant_id }).await;
    }
    counts
}

/// The rows `(search_index, search_index_contained)` a lone `create` writes for
/// each shape of a [`patient_bundle`] -- Patient, Observation,
/// MedicationRequest -- on a twin tenant of `backend`.
async fn lone_row_counts(backend: &MongoBackend, family: &str, mrn: &str) -> [(u64, u64); 3] {
    let twin = create_tenant("chunks-twin");
    let twin_id = twin.tenant_id().as_str();
    let patient = backend
        .create(
            &twin,
            "Patient",
            patient_json(family, mrn),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let subject = format!("Patient/{}", patient.id());
    let observation = backend
        .create(
            &twin,
            "Observation",
            observation_json(&subject, 0),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let medication_request = backend
        .create(
            &twin,
            "MedicationRequest",
            medication_request_json(&subject),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let counts = [
        row_counts(backend, twin_id, "Patient", patient.id()).await,
        row_counts(backend, twin_id, "Observation", observation.id()).await,
        row_counts(
            backend,
            twin_id,
            "MedicationRequest",
            medication_request.id(),
        )
        .await,
    ];
    assert!(
        counts.iter().all(|(own, _)| *own > 0) && counts[2].1 > 0,
        "a lone create writes rows for each shape (and contained rows for the MedicationRequest): \
         {counts:?}"
    );
    // 100 Observations must stay under the 5,000-row cap of a chunk
    // (`SEARCH_INDEX_INSERT_CHUNK`, bundle_creates.rs): a registry change that
    // makes the row cap cut the chunks fails here, not as a chunking regression.
    let per_observation = counts[1].0 + counts[1].1;
    assert!(
        per_observation * 100 < 5_000,
        "{per_observation} rows per Observation: 100 of them reach the 5,000-row cap of a chunk, \
         so the chunks of these tests are cut by rows and not by entries"
    );
    counts
}

/// A Bundle of several chunks ends as one lone create per resource would: one
/// `resources` document and one history document at version 1 each, and exactly
/// the rows a lone create writes, so no row set is written twice or left over.
/// `expected` is [`lone_row_counts`]; `rows` is false when search is offloaded.
async fn assert_end_state(
    backend: &MongoBackend,
    tenant_id: &str,
    stored: &[(String, String)],
    expected: &[(u64, u64); 3],
    rows: bool,
) {
    let total = stored.len() as u64;
    let live = doc! { "tenant_id": tenant_id, "version_id": "1", "is_deleted": false };
    assert_eq!(
        count_docs(backend, "resources", doc! { "tenant_id": tenant_id }).await,
        total
    );
    assert_eq!(count_docs(backend, "resources", live.clone()).await, total);
    assert_eq!(
        count_docs(backend, "resource_history", doc! { "tenant_id": tenant_id }).await,
        total,
        "one history document per resource"
    );
    assert_eq!(
        count_docs(backend, "resource_history", live).await,
        total,
        "every history document is version 1"
    );

    let own = rows_by_resource(backend, tenant_id, "search_index").await;
    let contained = rows_by_resource(backend, tenant_id, "search_index_contained").await;
    if !rows {
        assert!(own.is_empty() && contained.is_empty(), "no rows: {own:?}");
        return;
    }
    let (mut own_total, mut contained_total) = (0, 0);
    for (resource_type, id) in stored {
        let shape = match resource_type.as_str() {
            "Patient" => expected[0],
            "Observation" => expected[1],
            "MedicationRequest" => expected[2],
            other => panic!("unexpected resource type {other}"),
        };
        assert_eq!(
            own.get(id).copied().unwrap_or(0),
            shape.0 as i64,
            "search_index rows of {resource_type}/{id}"
        );
        assert_eq!(
            contained.get(id).copied().unwrap_or(0),
            shape.1 as i64,
            "search_index_contained rows of {resource_type}/{id}"
        );
        own_total += shape.0 as i64;
        contained_total += shape.1 as i64;
    }
    assert_eq!(
        own.values().sum::<i64>(),
        own_total,
        "no search_index row beyond the Bundle's resources"
    );
    assert_eq!(
        contained.values().sum::<i64>(),
        contained_total,
        "no search_index_contained row beyond the Bundle's resources"
    );
}

/// Everything a [`patient_bundle`] is searched by finds it. `n_obs` can exceed
/// one page, so the Observation search asks for them all.
async fn assert_searchable(
    backend: &MongoBackend,
    tenant: &TenantContext,
    stored: &[(String, String)],
    family: &str,
    mrn: &str,
    n_obs: usize,
) {
    let patient_id = &stored[0].1;
    assert_eq!(
        patients_by_mrn(backend, tenant, mrn).await,
        vec![patient_id.clone()],
        "Patient?identifier"
    );
    assert_eq!(
        patients_by_family(backend, tenant, family).await,
        vec![patient_id.clone()],
        "Patient?family"
    );
    let observations = backend
        .search(
            tenant,
            &SearchQuery::new("Observation")
                .with_parameter(reference_param("subject", &format!("Patient/{patient_id}")))
                .with_count(500),
        )
        .await
        .expect("search")
        .resources
        .items;
    assert_eq!(observations.len(), n_obs, "Observation?subject");
}

// ---------------------------------------------------------------------------
// Command counts
// ---------------------------------------------------------------------------

/// 207 POSTs go out as three chunks. Inline, each chunk is one insert per
/// collection and one delete of the old rows per search collection, and there
/// is no `find` on `resources`: 3 + 3 + 3 + 3 + 3 + 1 commands where a create
/// used to cost 6 or 7 on its own.
#[tokio::test]
async fn a_post_only_bundle_sends_its_creates_in_chunks_without_existence_reads() {
    const NAME: &str = "a_post_only_bundle_sends_its_creates_in_chunks_without_existence_reads";
    let Some(backend) = backend_or_skip("chunks_inline", Mode::Inline).await else {
        return;
    };
    let tenant = create_tenant("chunks-inline");
    let tenant_id = tenant.tenant_id().as_str();
    let expected = lone_row_counts(&backend, "Chunked", "MRN-CHUNK").await;

    let db = raw_db(&backend).await;
    let profiled = start_profiling(&db).await;
    let entries = patient_bundle(OBSERVATIONS, "Chunked", "MRN-CHUNK");
    assert_eq!(entries.len(), ENTRIES);
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, NAME).await else {
        return;
    };
    assert!(result.entries.iter().all(|entry| entry.status == 201));
    let stored = located(&result);

    if profiled {
        let profile = TxnProfile::of_bundle(&db).await;
        profile.assert_create_chunks();
        let context = profile.summary();
        for collection in ["search_index", "search_index_contained"] {
            assert_eq!(
                profile.commands(collection, "remove"),
                CHUNKS,
                "{collection} removes, one per chunk: {context}"
            );
        }
        assert_eq!(
            profile.commands("search_index", "insert"),
            CHUNKS,
            "search_index inserts: {context}"
        );
        // Only the last chunk holds the MedicationRequest with a contained resource.
        assert_eq!(
            profile.commands("search_index_contained", "insert"),
            1,
            "search_index_contained inserts: {context}"
        );
        assert_eq!(profile.commands("search_index_pending", "insert"), 0);
    }

    assert_eq!(ledger_count(&backend).await, 0, "no ledger");
    assert_eq!(marker_count(&backend, tenant_id).await, 0, "no marker");
    assert_end_state(&backend, tenant_id, &stored, &expected, true).await;
    assert_searchable(
        &backend,
        &tenant,
        &stored,
        "Chunked",
        "MRN-CHUNK",
        OBSERVATIONS,
    )
    .await;
    let medication_request = &stored[ENTRIES - 1];
    assert!(
        row_counts(
            &backend,
            tenant_id,
            &medication_request.0,
            &medication_request.1
        )
        .await
        .1 > 0,
        "the MedicationRequest has contained rows"
    );
}

/// The same Bundle on a backend that defers its search rows: the chunks write
/// no row and delete none, and the ledger is one insert after the last chunk.
#[tokio::test]
async fn a_deferred_post_only_bundle_sends_its_creates_in_chunks_without_search_writes() {
    const NAME: &str =
        "a_deferred_post_only_bundle_sends_its_creates_in_chunks_without_search_writes";
    let Some(backend) = backend_or_skip("chunks_deferred", Mode::Deferred).await else {
        return;
    };
    let tenant = create_tenant("chunks-deferred");
    let tenant_id = tenant.tenant_id().as_str();
    let expected = lone_row_counts(&backend, "Chunked", "MRN-CHUNK").await;

    let db = raw_db(&backend).await;
    let profiled = start_profiling(&db).await;
    let entries = patient_bundle(OBSERVATIONS, "Chunked", "MRN-CHUNK");
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, NAME).await else {
        return;
    };
    assert!(result.entries.iter().all(|entry| entry.status == 201));
    let stored = located(&result);

    if profiled {
        let profile = TxnProfile::of_bundle(&db).await;
        profile.assert_create_chunks();
        profile.assert_no_search_writes();
        assert_eq!(
            profile.commands("search_index_pending", "insert"),
            1,
            "one ledger insert, after the last chunk: {}",
            profile.summary()
        );
        assert_eq!(profile.inserted("search_index_pending"), 1);
    }

    // The response is not sent before the rows are there.
    assert_eq!(ledger_count(&backend).await, 0, "the ledger is cleared");
    assert_eq!(marker_count(&backend, tenant_id).await, 0, "no marker left");
    assert_end_state(&backend, tenant_id, &stored, &expected, true).await;
    assert_searchable(
        &backend,
        &tenant,
        &stored,
        "Chunked",
        "MRN-CHUNK",
        OBSERVATIONS,
    )
    .await;
}

/// With search offloaded the Bundle writes no row and deletes none, and still
/// sends its creates in chunks.
#[tokio::test]
async fn an_offloaded_post_only_bundle_sends_its_creates_in_chunks() {
    const NAME: &str = "an_offloaded_post_only_bundle_sends_its_creates_in_chunks";
    let Some(backend) = backend_or_skip("chunks_offloaded", Mode::Offloaded).await else {
        return;
    };
    let tenant = create_tenant("chunks-offloaded");
    let tenant_id = tenant.tenant_id().as_str();

    let db = raw_db(&backend).await;
    let profiled = start_profiling(&db).await;
    let entries = patient_bundle(OBSERVATIONS, "Chunked", "MRN-CHUNK");
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, NAME).await else {
        return;
    };
    assert!(result.entries.iter().all(|entry| entry.status == 201));
    let stored = located(&result);

    if profiled {
        let profile = TxnProfile::of_bundle(&db).await;
        profile.assert_create_chunks();
        profile.assert_no_search_writes();
        assert_eq!(profile.commands("search_index_pending", "insert"), 0);
    }

    assert_end_state(&backend, tenant_id, &stored, &[(0, 0); 3], false).await;
}

// ---------------------------------------------------------------------------
// A reused id still fails at its own entry
// ---------------------------------------------------------------------------

const MODES: [Mode; 2] = [Mode::Inline, Mode::Deferred];

/// `count` POST Patients, each with a family of its own and no id.
fn patient_posts(count: usize) -> Vec<BundleEntry> {
    (0..count)
        .map(|n| {
            post(
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": format!("Reuse{n}")}]}),
                &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            )
        })
        .collect()
}

/// Gives the POST at `at` the body id `id`.
fn with_body_id(mut entries: Vec<BundleEntry>, at: usize, id: &str) -> Vec<BundleEntry> {
    entries[at].resource.as_mut().expect("a POST has a body")["id"] = json!(id);
    entries
}

/// Seeds `Patient/id` for the tenant; with `deleted` it is then soft-deleted.
async fn seed_patient(backend: &MongoBackend, tenant: &TenantContext, id: &str, deleted: bool) {
    backend
        .create(
            tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": id, "name": [{"family": "Seed"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    if deleted {
        backend.delete(tenant, "Patient", id).await.unwrap();
    }
}

fn already_exists(id: &str) -> String {
    format!("Entry processing failed: resource already exists: Patient/{id}")
}

/// Runs the Bundle and returns the `BundleError` it must end in, or `None` when
/// the topology cannot run transactions. Nothing of the Bundle may be stored:
/// the tenant's documents and rows are what `before` counted.
async fn assert_bundle_error(
    backend: &MongoBackend,
    tenant: &TenantContext,
    entries: Vec<BundleEntry>,
    before: [u64; 4],
    name: &str,
) -> Option<(usize, String)> {
    let result = backend
        .process_transaction(tenant, entries, FhirVersion::default())
        .await;
    if lacks_transactions(&result) {
        return None;
    }
    let failure = match result {
        Err(TransactionError::BundleError { index, message }) => (index, message),
        Err(other) => panic!("{name}: expected a BundleError, got {other:?}"),
        Ok(_) => panic!("{name}: the Bundle committed"),
    };
    let tenant_id = tenant.tenant_id().as_str();
    assert_eq!(
        store_counts(backend, tenant_id).await,
        before,
        "{name}: nothing of the Bundle is stored"
    );
    assert_eq!(ledger_count(backend).await, 0, "{name}: no ledger");
    assert_eq!(
        marker_count(backend, tenant_id).await,
        0,
        "{name}: no marker"
    );
    Some(failure)
}

/// A body id that is stored: entry 3 of 6 fails.
#[tokio::test]
async fn a_body_id_that_is_stored_fails_at_its_entry() {
    for mode in MODES {
        let name = format!("stored_id_{}", mode.label());
        let Some(backend) = backend_or_skip(&name, mode).await else {
            return;
        };
        let tenant = create_tenant("chunks-reuse");
        seed_patient(&backend, &tenant, "dup-stored", false).await;
        let before = store_counts(&backend, tenant.tenant_id().as_str()).await;

        let entries = with_body_id(patient_posts(6), 3, "dup-stored");
        let Some(failure) = assert_bundle_error(&backend, &tenant, entries, before, &name).await
        else {
            return;
        };
        assert_eq!(failure, (3, already_exists("dup-stored")), "{name}");
    }
}

/// A body id of a deleted resource: a soft-deleted resource keeps its document,
/// so the id is still taken.
#[tokio::test]
async fn a_body_id_of_a_deleted_resource_fails_at_its_entry() {
    for mode in MODES {
        let name = format!("deleted_id_{}", mode.label());
        let Some(backend) = backend_or_skip(&name, mode).await else {
            return;
        };
        let tenant = create_tenant("chunks-reuse");
        seed_patient(&backend, &tenant, "dup-deleted", true).await;
        let before = store_counts(&backend, tenant.tenant_id().as_str()).await;

        let entries = with_body_id(patient_posts(6), 3, "dup-deleted");
        let Some(failure) = assert_bundle_error(&backend, &tenant, entries, before, &name).await
        else {
            return;
        };
        assert_eq!(failure, (3, already_exists("dup-deleted")), "{name}");
    }
}

/// The same id twice in one Bundle fails at the second entry with the
/// duplicate, not with the Bundle's own overlap message, whether both creates
/// are in one chunk (entries 1 and 4) or the first was sent with an earlier one
/// (entries 2 and 150 of 160, so the first is in chunk one and the second in chunk two).
#[tokio::test]
async fn the_same_id_twice_fails_at_the_second_entry() {
    for mode in MODES {
        let Some(backend) = backend_or_skip(&format!("twice_{}", mode.label()), mode).await else {
            return;
        };
        for (first, second, count) in [(1, 4, 6), (2, 150, 160)] {
            let name = format!("twice_{first}_{second}_{}", mode.label());
            let tenant = create_tenant(&format!("chunks-twice-{first}-{second}"));
            let before = store_counts(&backend, tenant.tenant_id().as_str()).await;

            let entries = with_body_id(patient_posts(count), first, "dup-twice");
            let entries = with_body_id(entries, second, "dup-twice");
            let Some(failure) =
                assert_bundle_error(&backend, &tenant, entries, before, &name).await
            else {
                return;
            };
            assert_eq!(failure, (second, already_exists("dup-twice")), "{name}");
        }
    }
}

/// An earlier create that the server refuses is reported before a later entry
/// that is invalid: the Bundle ran its entries in order, and entry 0 is where it
/// would have stopped.
#[tokio::test]
async fn an_earlier_duplicate_is_reported_before_a_later_invalid_entry() {
    for mode in MODES {
        let name = format!("duplicate_then_invalid_{}", mode.label());
        let Some(backend) = backend_or_skip(&name, mode).await else {
            return;
        };
        let tenant = create_tenant("chunks-reuse");
        seed_patient(&backend, &tenant, "dup-first", false).await;
        let before = store_counts(&backend, tenant.tenant_id().as_str()).await;

        let mut entries = with_body_id(patient_posts(6), 0, "dup-first");
        // No `resourceType`: processing the entry fails before it stages anything.
        entries[1].resource = Some(json!({"name": [{"family": "Invalid"}]}));
        let Some(failure) = assert_bundle_error(&backend, &tenant, entries, before, &name).await
        else {
            return;
        };
        assert_eq!(failure, (0, already_exists("dup-first")), "{name}");
    }
}

/// A PUT to the id of a deleted resource restores it, as the same PUT outside a
/// Bundle does, instead of failing as the id is taken (#2145): the entry answers
/// 201, the version chain continues (create 1, delete 2, restore 3) on the
/// tombstone's document, and the restored body is searchable.
#[tokio::test]
async fn a_put_over_a_deleted_id_restores_it() {
    for mode in MODES {
        let name = format!("put_over_deleted_{}", mode.label());
        let Some(backend) = backend_or_skip(&name, mode).await else {
            return;
        };
        let tenant = create_tenant("chunks-reuse");
        seed_patient(&backend, &tenant, "dup-put", true).await;
        let [resources, history, ..] = store_counts(&backend, tenant.tenant_id().as_str()).await;

        let mut entries = patient_posts(6);
        entries[2] = BundleEntry {
            method: BundleMethod::Put,
            url: "Patient/dup-put".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": "dup-put",
                "name": [{"family": "RestoredByPut"}],
            })),
            ..Default::default()
        };
        let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, &name).await
        else {
            return;
        };
        assert_eq!(result.entries[2].status, 201, "{name}: the PUT restores");

        let restored = backend
            .read(&tenant, "Patient", "dup-put")
            .await
            .expect("the restored resource reads")
            .expect("the restored resource is live");
        assert_eq!(restored.version_id(), "3", "{name}: the chain continues");
        assert_eq!(
            restored.content()["name"][0]["family"],
            "RestoredByPut",
            "{name}"
        );

        // Five POSTs add five documents; the restore reuses the tombstone's.
        // Every entry adds one history version.
        let [resources_after, history_after, ..] =
            store_counts(&backend, tenant.tenant_id().as_str()).await;
        assert_eq!(resources_after, resources + 5, "{name}: resources");
        assert_eq!(history_after, history + 6, "{name}: history");

        assert_eq!(
            patients_by_family(&backend, &tenant, "RestoredByPut").await,
            vec!["dup-put".to_string()],
            "{name}: the restored body is searchable"
        );
    }
}

// ---------------------------------------------------------------------------
// An entry that reads sees the creates before it
// ---------------------------------------------------------------------------

/// `count` POST Observations that no later entry depends on, to bring a short
/// Bundle up to the deferral threshold. They come after the entries under test.
fn padding(count: usize) -> Vec<BundleEntry> {
    (0..count)
        .map(|n| {
            post(
                "Observation",
                observation_json("Patient/chunks-padding", n),
                &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            )
        })
        .collect()
}

/// A `GET` and an `ifNoneExist` POST read what the POSTs before them created:
/// the staged creates are sent first.
#[tokio::test]
async fn creates_are_sent_before_an_entry_that_reads_them() {
    for mode in MODES {
        let name = format!("read_after_create_{}", mode.label());
        let Some(backend) = backend_or_skip(&name, mode).await else {
            return;
        };

        // (a) POST then GET of the id the body supplied.
        let tenant = create_tenant("chunks-read-get");
        let mut entries = vec![
            post(
                "Patient",
                json!({"resourceType": "Patient", "id": "p-a", "name": [{"family": "ReadBack"}]}),
                "urn:uuid:chunks-p-a",
            ),
            BundleEntry {
                method: BundleMethod::Get,
                url: "Patient/p-a".to_string(),
                ..Default::default()
            },
        ];
        entries.extend(padding(4));
        let db = raw_db(&backend).await;
        let profiled = start_profiling(&db).await;
        let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, &name).await
        else {
            return;
        };
        assert_eq!(result.entries[0].status, 201, "{name}");
        assert_eq!(
            result.entries[1].status, 200,
            "{name}: the GET finds the POST"
        );
        assert_eq!(
            result.entries[1]
                .resource
                .as_ref()
                .expect("the GET returns the resource")["name"][0]["family"],
            "ReadBack",
            "{name}"
        );
        if profiled {
            // The POST went out as a chunk of its own before the GET, which is
            // the one read of `resources` (the profiler does see finds); the four
            // that follow it are the last chunk.
            let profile = TxnProfile::of_bundle(&db).await;
            let context = profile.summary();
            assert_eq!(
                profile.commands("resources", "insert"),
                2,
                "{name}: {context}"
            );
            assert_eq!(profile.inserted("resources"), 5, "{name}: {context}");
            assert_eq!(profile.finds("resources"), 1, "{name}: {context}");
        }

        // (b) POST then an ifNoneExist POST for the same identifier.
        let tenant = create_tenant("chunks-read-ifne");
        let tenant_id = tenant.tenant_id().as_str();
        let mrn = "MRN-IFNE";
        let mut entries = vec![
            post(
                "Patient",
                patient_json("Twin", mrn),
                "urn:uuid:chunks-ifne-a",
            ),
            post_if_none_exist(
                "Patient",
                patient_json("Twin", mrn),
                "urn:uuid:chunks-ifne-b",
                &format!("identifier={MRN_SYSTEM}|{mrn}"),
            ),
        ];
        entries.extend(padding(4));
        let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, &name).await
        else {
            return;
        };
        assert_eq!(result.entries[0].status, 201, "{name}");
        assert_eq!(
            result.entries[1].status, 200,
            "{name}: the criteria match entry 0"
        );
        assert_eq!(result.entries[1].effect, BundleEntryEffect::NoOp, "{name}");
        let stored = located(&result);
        assert_eq!(stored[1], stored[0], "{name}: entry 1 resolved to entry 0");
        let patient_id = &stored[0].1;
        assert_eq!(
            patients_by_mrn(&backend, &tenant, mrn).await,
            vec![patient_id.clone()],
            "{name}: exactly one Patient has the identifier"
        );
        assert_eq!(
            count_docs(
                &backend,
                "resources",
                doc! { "tenant_id": tenant_id, "resource_type": "Patient" }
            )
            .await,
            1,
            "{name}"
        );

        // Its rows are the single set a lone create writes.
        let twin = create_tenant("chunks-ifne-twin");
        let lone = backend
            .create(
                &twin,
                "Patient",
                patient_json("Twin", mrn),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let lone_id = twin.tenant_id().as_str();
        assert_eq!(
            row_counts(&backend, tenant_id, "Patient", patient_id).await,
            row_counts(&backend, lone_id, "Patient", lone.id()).await,
            "{name}: one row set"
        );
        assert_eq!(
            normalized_fingerprints(
                &backend,
                tenant_id,
                &[("Patient".to_string(), patient_id.clone())]
            )
            .await,
            normalized_fingerprints(
                &backend,
                lone_id,
                &[("Patient".to_string(), lone.id().to_string())]
            )
            .await,
            "{name}: the rows are those of a lone create"
        );
        assert_eq!(ledger_count(&backend).await, 0, "{name}");
    }
}

/// A write to a resource an earlier entry of the Bundle created is still the
/// Bundle's overlap error, at the second entry, and nothing is stored: the
/// `PUT`, `DELETE` and `PATCH` find the create because it was sent first.
#[tokio::test]
async fn a_create_then_a_write_of_the_same_resource_overlaps_as_before() {
    let patch = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "operation", "part": [
            {"name": "type", "valueCode": "replace"},
            {"name": "path", "valueString": "Patient.name[0].family"},
            {"name": "value", "valueString": "After"}
        ]}]
    });
    let writes = [
        ("p-b", BundleMethod::Put, None),
        ("p-c", BundleMethod::Delete, None),
        ("p-d", BundleMethod::Patch, Some(patch)),
    ];
    for mode in MODES {
        let Some(backend) = backend_or_skip(&format!("overlap_{}", mode.label()), mode).await
        else {
            return;
        };
        for (id, method, body) in &writes {
            let name = format!("overlap_{id}_{}", mode.label());
            let tenant = create_tenant(&format!("chunks-overlap-{id}"));
            let before = store_counts(&backend, tenant.tenant_id().as_str()).await;

            let resource = match method {
                BundleMethod::Put => Some(json!({
                    "resourceType": "Patient",
                    "id": id,
                    "name": [{"family": "After"}],
                })),
                _ => body.clone(),
            };
            let mut entries = vec![
                post(
                    "Patient",
                    json!({"resourceType": "Patient", "id": id, "name": [{"family": "Before"}]}),
                    &format!("urn:uuid:chunks-{id}"),
                ),
                BundleEntry {
                    method: *method,
                    url: format!("Patient/{id}"),
                    resource,
                    ..Default::default()
                },
            ];
            entries.extend(padding(4));
            let Some((index, message)) =
                assert_bundle_error(&backend, &tenant, entries, before, &name).await
            else {
                return;
            };
            assert_eq!(index, 1, "{name}");
            assert!(
                message.contains(&format!("overlap on resource Patient/{id}")),
                "{name}: {message}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The old rows of a created id, and the scope of a reused id
// ---------------------------------------------------------------------------

const ORPHAN_ID: &str = "orphan-x";
const OLD_FAMILY: &str = "OrphanOldFamily";
const NEW_FAMILY: &str = "OrphanNewFamily";

fn patient_with_id(id: &str, family: &str) -> serde_json::Value {
    json!({"resourceType": "Patient", "id": id, "name": [{"family": family}]})
}

/// Creates `resource` and then removes its `resources` and `resource_history`
/// documents, as a purge that failed half way leaves it: only its search rows
/// remain. Returns those rows, `(search_index, search_index_contained)`.
async fn leave_orphan_rows(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
    id: &str,
    resource: serde_json::Value,
) -> (u64, u64) {
    let tenant_id = tenant.tenant_id().as_str();
    backend
        .create(tenant, resource_type, resource, FhirVersion::default())
        .await
        .unwrap();
    let db = raw_db(backend).await;
    for collection in ["resources", "resource_history"] {
        let removed = db
            .collection::<Document>(collection)
            .delete_many(doc! { "tenant_id": tenant_id, "resource_type": resource_type, "id": id })
            .await
            .unwrap();
        assert!(
            removed.deleted_count > 0,
            "{resource_type}/{id} had a {collection} document to remove"
        );
    }
    let rows = row_counts(backend, tenant_id, resource_type, id).await;
    assert!(
        rows.0 > 0,
        "{resource_type}/{id} keeps its search rows: {rows:?}"
    );
    rows
}

/// The old rows a create's id has are cleared by the chunk's delete, and nothing
/// else is: this is why the delete stays although a created id has no rows of
/// this Bundle. A purge that fails half way leaves rows for a free id, which a
/// body-supplied id can create again. The decoys differ from the target in one
/// part of the key each (tenant, type, id), so a filter that matches too much
/// deletes one of them. The Patient shares its chunk with an Observation, so the
/// delete is the `$or` of two types.
#[tokio::test]
async fn a_create_clears_orphan_rows_of_its_id_and_keeps_every_other_row() {
    for mode in MODES {
        let name = format!("orphan_rows_{}", mode.label());
        let Some(backend) = backend_or_skip(&name, mode).await else {
            return;
        };
        let tenant = create_tenant("chunks-orphan");
        let other_tenant = create_tenant("chunks-orphan-other");
        let tenant_id = tenant.tenant_id().as_str();

        leave_orphan_rows(
            &backend,
            &tenant,
            "Patient",
            ORPHAN_ID,
            patient_with_id(ORPHAN_ID, OLD_FAMILY),
        )
        .await;
        let mut observation = observation_json("Patient/orphan-x", 0);
        observation["id"] = json!(ORPHAN_ID);
        let decoys = [
            (
                &tenant,
                "Observation",
                ORPHAN_ID,
                leave_orphan_rows(&backend, &tenant, "Observation", ORPHAN_ID, observation).await,
            ),
            (
                &tenant,
                "Patient",
                "other-orphan",
                leave_orphan_rows(
                    &backend,
                    &tenant,
                    "Patient",
                    "other-orphan",
                    patient_with_id("other-orphan", OLD_FAMILY),
                )
                .await,
            ),
            (
                &other_tenant,
                "Patient",
                ORPHAN_ID,
                leave_orphan_rows(
                    &backend,
                    &other_tenant,
                    "Patient",
                    ORPHAN_ID,
                    patient_with_id(ORPHAN_ID, OLD_FAMILY),
                )
                .await,
            ),
        ];
        assert_eq!(
            patients_by_family(&backend, &tenant, OLD_FAMILY).await,
            Vec::<String>::new(),
            "{name}: the orphan rows have no resource behind them"
        );

        // Entry 0 is an Observation, entry 1 the Patient with the orphan's id: both in
        // the first chunk of 100. 112 entries are two chunks and defer their rows on
        // the deferred backend.
        let mut entries = padding(1);
        entries.push(post(
            "Patient",
            patient_with_id(ORPHAN_ID, NEW_FAMILY),
            "urn:uuid:chunks-orphan-new",
        ));
        entries.extend(padding(110));
        assert_eq!(entries.len(), 112);
        let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, &name).await
        else {
            return;
        };
        assert!(result.entries.iter().all(|entry| entry.status == 201));
        assert_eq!(
            located(&result)[1],
            ("Patient".to_string(), ORPHAN_ID.to_string())
        );

        // No row of the created id still carries the old body.
        let db = raw_db(&backend).await;
        for collection in ["search_index", "search_index_contained"] {
            let rows: Vec<Document> = db
                .collection::<Document>(collection)
                .find(doc! {
                    "tenant_id": tenant_id,
                    "resource_type": "Patient",
                    "resource_id": ORPHAN_ID,
                })
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert!(
                rows.iter()
                    .all(|row| !format!("{row:?}").to_lowercase().contains("orphanold")),
                "{name}: a {collection} row of Patient/{ORPHAN_ID} still has the old family"
            );
        }
        assert_eq!(
            patients_by_family(&backend, &tenant, OLD_FAMILY).await,
            Vec::<String>::new(),
            "{name}: the old family finds nothing"
        );
        assert_eq!(
            patients_by_family(&backend, &tenant, NEW_FAMILY).await,
            vec![ORPHAN_ID.to_string()],
            "{name}: the new family finds the created Patient"
        );

        // Its rows are those a lone create of the same body writes.
        let twin = create_tenant("chunks-orphan-twin");
        backend
            .create(
                &twin,
                "Patient",
                patient_with_id(ORPHAN_ID, NEW_FAMILY),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let (twin_id, patient) = (
            twin.tenant_id().as_str(),
            [("Patient".to_string(), ORPHAN_ID.to_string())],
        );
        let rows = row_counts(&backend, tenant_id, "Patient", ORPHAN_ID).await;
        assert_eq!(
            rows,
            row_counts(&backend, twin_id, "Patient", ORPHAN_ID).await,
            "{name}: row counts of the created id"
        );
        assert!(rows.0 > 0, "{name}: the created Patient has rows");
        assert_eq!(
            normalized_fingerprints(&backend, tenant_id, &patient).await,
            normalized_fingerprints(&backend, twin_id, &patient).await,
            "{name}: the rows are those of a lone create"
        );

        // Every other orphan keeps all of its rows.
        for (decoy_tenant, resource_type, id, before) in decoys {
            assert_eq!(
                row_counts(
                    &backend,
                    decoy_tenant.tenant_id().as_str(),
                    resource_type,
                    id
                )
                .await,
                before,
                "{name}: the rows of {resource_type}/{id} in tenant {} are untouched",
                decoy_tenant.tenant_id().as_str()
            );
        }
    }
}

/// With the existence read gone, a reused id is refused by the unique index on
/// `(tenant_id, resource_type, id)`: another tenant or another type may still
/// create the same id.
#[tokio::test]
async fn a_reused_id_is_scoped_to_its_tenant_and_type() {
    for mode in MODES {
        let name = format!("reuse_scope_{}", mode.label());
        let Some(backend) = backend_or_skip(&name, mode).await else {
            return;
        };
        let seeded = create_tenant("chunks-scope-seeded");
        let other = create_tenant("chunks-scope-other");
        seed_patient(&backend, &seeded, "dup-iso", false).await;

        // Another tenant creates the same type and id.
        let entries = with_body_id(patient_posts(6), 3, "dup-iso");
        let Some(result) = process_transaction_or_skip(&backend, &other, entries, &name).await
        else {
            return;
        };
        assert!(
            result.entries.iter().all(|entry| entry.status == 201),
            "{name}"
        );
        assert_eq!(
            located(&result)[3],
            ("Patient".to_string(), "dup-iso".to_string()),
            "{name}: another tenant"
        );

        // The seeded tenant creates another type under the same id.
        let mut observation = observation_json("Patient/dup-iso", 0);
        observation["id"] = json!("dup-iso");
        let mut entries = patient_posts(6);
        entries[3] = post(
            "Observation",
            observation,
            "urn:uuid:chunks-scope-observation",
        );
        let Some(result) = process_transaction_or_skip(&backend, &seeded, entries, &name).await
        else {
            return;
        };
        assert!(
            result.entries.iter().all(|entry| entry.status == 201),
            "{name}"
        );
        assert_eq!(
            located(&result)[3],
            ("Observation".to_string(), "dup-iso".to_string()),
            "{name}: another type"
        );

        // The same tenant and type is still refused, at its own entry.
        let before = store_counts(&backend, seeded.tenant_id().as_str()).await;
        let entries = with_body_id(patient_posts(6), 3, "dup-iso");
        let Some(failure) = assert_bundle_error(&backend, &seeded, entries, before, &name).await
        else {
            return;
        };
        assert_eq!(failure, (3, already_exists("dup-iso")), "{name}");
    }
}

// ---------------------------------------------------------------------------
// A failure on a later chunk, and a create sent on its own
// ---------------------------------------------------------------------------

/// A command the server fails on the second chunk of 150 creates is charged to
/// the first entry of that chunk (entry 100), not to entry 0 and not to the last
/// entry: a failed command names no document. Search is offloaded, so a chunk is
/// exactly two inserts, `resources` then `resource_history`, and `skip` counts
/// them: chunk one is hits 1 and 2.
#[tokio::test]
async fn a_failed_command_on_a_later_chunk_is_charged_to_its_first_entry() {
    let app = "fp-chunks-later-chunk";
    let Some(backend) = deferred_backend("chunks_later_chunk", app, None, |config| {
        config.search_offloaded = true;
    })
    .await
    else {
        eprintln!(
            "Skipping a_failed_command_on_a_later_chunk_is_charged_to_its_first_entry \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    for (skip, context) in [
        (2_i32, "Failed to insert resource in transaction"),
        (3, "Failed to insert history in transaction"),
    ] {
        let name = format!("later_chunk_skip_{skip}");
        let tenant = create_tenant(&format!("chunks-later-{skip}"));
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! { "failCommands": ["insert"], "errorCode": 2 },
            doc! { "skip": skip },
        )
        .await
        else {
            return;
        };
        let result = backend
            .process_transaction(&tenant, patient_posts(150), FhirVersion::default())
            .await;
        fail_point.off().await;
        if lacks_transactions(&result) {
            return;
        }
        match result {
            Err(TransactionError::BundleError { index, message }) => {
                assert_eq!(index, 100, "{name}: {message}");
                assert!(message.contains(context), "{name}: {message}");
            }
            other => panic!("{name}: expected a BundleError, got {other:?}"),
        }
        assert_eq!(
            store_counts(&backend, tenant.tenant_id().as_str()).await,
            [0; 4],
            "{name}: nothing of the Bundle is stored"
        );
    }
}

/// A resource of 1 MiB or more is sent in a group of its own, so the server's
/// refusal of a document for its size would be charged to its own entry: three
/// creates, the middle one large, are three inserts into each of `resources` and
/// `resource_history`, one document each.
#[tokio::test]
async fn a_large_create_is_sent_in_a_group_of_its_own() {
    const NAME: &str = "a_large_create_is_sent_in_a_group_of_its_own";
    let Some(backend) = backend_or_skip("chunks_large", Mode::Inline).await else {
        return;
    };
    let tenant = create_tenant("chunks-large");
    let small = |family: &str| {
        post(
            "Patient",
            patient_with_id(&format!("large-{family}"), family),
            &format!("urn:uuid:chunks-large-{family}"),
        )
    };
    // About 1.2 MB of narrative: over the 1 MiB (BUNDLE_CREATE_ALONE_BYTES) that
    // makes a create a group of its own.
    let div = format!(
        "<div xmlns=\"http://www.w3.org/1999/xhtml\">{}</div>",
        "x".repeat(1_200_000)
    );
    let large = post(
        "Patient",
        json!({
            "resourceType": "Patient",
            "id": "large-big",
            "name": [{"family": "Big"}],
            "text": {"status": "generated", "div": div},
        }),
        "urn:uuid:chunks-large-big",
    );
    let entries = vec![small("First"), large, small("Last")];

    let db = raw_db(&backend).await;
    let profiled = start_profiling(&db).await;
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, NAME).await else {
        return;
    };
    assert!(result.entries.iter().all(|entry| entry.status == 201));

    if profiled {
        let profile = TxnProfile::of_bundle(&db).await;
        let context = profile.summary();
        for collection in ["resources", "resource_history"] {
            let inserts = profile.on(collection, "insert");
            assert_eq!(
                inserts.len(),
                3,
                "{collection} insert commands, one per group: {context}"
            );
            assert!(
                inserts
                    .iter()
                    .all(|insert| numeric(insert, "ninserted") == 1),
                "{collection} inserts of one document each: {context}"
            );
        }
    }

    let read = backend
        .read(&tenant, "Patient", "large-big")
        .await
        .unwrap()
        .expect("the large Patient is stored");
    assert_eq!(
        read.content()["text"]["div"].as_str().map(str::len),
        Some(1_200_000 + "<div xmlns=\"http://www.w3.org/1999/xhtml\"></div>".len()),
        "the large Patient reads back whole"
    );
    assert_eq!(
        count_docs(
            &backend,
            "resources",
            doc! { "tenant_id": tenant.tenant_id().as_str() }
        )
        .await,
        3
    );
}
