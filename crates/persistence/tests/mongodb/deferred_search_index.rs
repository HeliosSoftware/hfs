//! #1975: a transaction Bundle of at least
//! `MongoBackendConfig::transaction_bundle_deferred_index_entries` entries
//! commits without its `search_index` rows and writes them after the commit.
//!
//! Child module of the `mongodb_tests` root; `use super::*;` reaches its
//! private harness, as in `transaction_retry.rs`.
//!
//! Every test runs with a threshold of 5 entries ([`THRESHOLD`]) so a 6-entry
//! Bundle already takes the deferred path. What the transaction wrote is read
//! from the server's own record of it: the `applyOps` entry of `local.oplog.rs`
//! the commit produced (see [`Oplog`]). That proves "no search writes inside the
//! transaction" and "below N is unchanged" without a production-only hook. The
//! post-commit step is interrupted with the server's `failCommand` failpoint
//! (scoped to the backend's `appName`, like `transaction_retry.rs`), with a
//! request that is dropped mid-step, and with a 1 ms `bundle_transaction_deadline`.
//! A restart is simulated by a fresh backend on the same database calling
//! `repair_deferred_search_index`.
//!
//! The last test is an ignored ceiling harness with its own small-cache
//! mongo:7.0 replica set; see [`deferred_indexing_raises_the_lone_bundle_ceiling`].

use super::*;

use std::time::{Duration, Instant};

use futures::TryStreamExt;
use mongodb::bson::Timestamp;
use mongodb::bson::oid::ObjectId;
use serde_json::Value;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mongo::Mongo;

use crate::bulk_submit::FailPoint;

/// The deferral threshold of every test (except where stated): a Bundle of 5 or
/// more entries defers its rows.
pub(super) const THRESHOLD: usize = 5;

pub(super) const MRN_SYSTEM: &str = "http://example.org/mrn";

// ---------------------------------------------------------------------------
// Backends
// ---------------------------------------------------------------------------

/// A backend on the shared replica set whose driver connections carry `app`
/// (for failpoints), with `transaction_bundle_deferred_index_entries` set to
/// `min_entries` (`Some(0)` = never defer, `None` = the gate's room).
/// `configure` adjusts the config just before the backend is built.
pub(super) async fn deferred_backend(
    test: &str,
    app: &str,
    min_entries: Option<usize>,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> Option<MongoBackend> {
    let connection_string = shared_mongo::connection_string().await?;
    let connection_string = append_query_param(&connection_string, "retryWrites=false");
    let mut config = MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test),
        app_name: app.to_string(),
        data_dir: Some(repo_data_dir()),
        transaction_bundle_deferred_index_entries: min_entries,
        ..Default::default()
    };
    configure(&mut config);
    build_backend(config).await
}

/// Has `backend` record the `missing_params` fingerprints (#1999) of every
/// type a [`patient_bundle`] writes, and waits until the catalog holds them.
/// The first write of a type records its fingerprint with a background
/// `update` upsert; an `update` failpoint enabled before that would be spent
/// (or counted) on the catalog write instead of the post-commit batch. The
/// resources go to a tenant of their own.
async fn record_missing_fingerprints(backend: &MongoBackend) {
    let tenant = create_tenant("missing-fp-record");
    let resources = [
        patient_json("Recorded", "MRN-RECORDED"),
        observation_json("Patient/recorded", 0),
        medication_request_json("Patient/recorded"),
        json!({"resourceType": "Medication"}),
    ];
    for content in resources {
        let resource_type = content["resourceType"].as_str().unwrap().to_string();
        backend
            .create(&tenant, &resource_type, content, FhirVersion::default())
            .await
            .expect("create a resource to record its fingerprint");
    }
    let waited = Instant::now();
    while count_docs(backend, "missing_params_fingerprints", doc! {}).await < 4 {
        assert!(
            waited.elapsed() < Duration::from_secs(10),
            "the missing_params fingerprints were never recorded"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A second backend on `backend`'s database under another `appName`, with the
/// default config: a restarted server, or another instance, that runs the
/// repair.
async fn same_db_backend(backend: &MongoBackend, app: &str) -> Option<MongoBackend> {
    let config = MongoBackendConfig {
        connection_string: backend.config().connection_string.clone(),
        database_name: backend.config().database_name.clone(),
        app_name: app.to_string(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    };
    build_backend(config).await
}

/// True when the topology cannot run transactions and the test must skip (only
/// possible against an external standalone `HFS_TEST_MONGODB_URL`).
pub(super) fn lacks_transactions<T>(result: &Result<T, TransactionError>) -> bool {
    if !matches!(
        result,
        Err(TransactionError::UnsupportedIsolationLevel { .. })
    ) {
        return false;
    }
    assert!(
        !transactions_required(),
        "the harness's own Mongo container is a replica set and must support transactions"
    );
    eprintln!("Skipping (MongoDB topology does not support transactions)");
    true
}

// ---------------------------------------------------------------------------
// Bundles
// ---------------------------------------------------------------------------

pub(super) fn post(url: &str, resource: Value, full_url: &str) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Post,
        url: url.to_string(),
        resource: Some(resource),
        full_url: Some(full_url.to_string()),
        ..Default::default()
    }
}

pub(super) fn post_if_none_exist(
    url: &str,
    resource: Value,
    full_url: &str,
    criteria: &str,
) -> BundleEntry {
    BundleEntry {
        if_none_exist: Some(criteria.to_string()),
        ..post(url, resource, full_url)
    }
}

pub(super) fn patient_json(family: &str, mrn: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "identifier": [{"system": MRN_SYSTEM, "value": mrn}],
        "name": [{"family": family}],
    })
}

pub(super) fn observation_json(subject: &str, n: usize) -> Value {
    json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
        "subject": {"reference": subject},
        "valueQuantity": {
            "value": 60 + n,
            "unit": "/min",
            "system": "http://unitsofmeasure.org",
            "code": "/min",
        },
    })
}

/// A MedicationRequest whose `contained` holds the Medication its
/// `medicationReference` points at, so it has `search_index_contained` rows.
pub(super) fn medication_request_json(subject: &str) -> Value {
    json!({
        "resourceType": "MedicationRequest",
        "status": "active",
        "intent": "order",
        "medicationReference": {"reference": "#med"},
        "subject": {"reference": subject},
        "contained": [{
            "resourceType": "Medication",
            "id": "med",
            "code": {
                "coding": [{
                    "system": "http://www.nlm.nih.gov/research/umls/rxnorm",
                    "code": "314076"
                }],
                "text": "Lisinopril"
            }
        }],
    })
}

/// One Patient (identifier `MRN_SYSTEM|<mrn>`, name `family`) under a `urn:uuid`,
/// `n_obs` Observations whose subject is that urn, and one MedicationRequest
/// with a contained Medication for the same patient: `n_obs + 2` POST entries,
/// in that order.
pub(super) fn patient_bundle(n_obs: usize, family: &str, mrn: &str) -> Vec<BundleEntry> {
    let urn = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    let mut entries = vec![post("Patient", patient_json(family, mrn), &urn)];
    for n in 0..n_obs {
        entries.push(post(
            "Observation",
            observation_json(&urn, n),
            &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        ));
    }
    entries.push(post(
        "MedicationRequest",
        medication_request_json(&urn),
        &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
    ));
    entries
}

/// `(resource type, id)` of every entry of a result, from its `location`.
pub(super) fn located(result: &BundleResult) -> Vec<(String, String)> {
    result
        .entries
        .iter()
        .map(|entry| {
            let location = entry
                .location
                .as_deref()
                .expect("every entry of these Bundles has a location");
            let path = location.split("/_history").next().unwrap_or(location);
            let mut parts = path.rsplit('/');
            let id = parts.next().expect("a location names an id");
            let resource_type = parts.next().expect("a location names a type");
            (resource_type.to_string(), id.to_string())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// What the database holds
// ---------------------------------------------------------------------------

pub(super) async fn raw_db(backend: &MongoBackend) -> mongodb::Database {
    raw_test_client(&backend.config().connection_string)
        .await
        .expect("raw client")
        .database(&backend.config().database_name)
}

/// Documents in the `search_index_pending` ledger.
pub(super) async fn ledger_count(backend: &MongoBackend) -> u64 {
    count_docs(backend, "search_index_pending", doc! {}).await
}

/// `resources` documents of the tenant still carrying `search_rows_pending`.
pub(super) async fn marker_count(backend: &MongoBackend, tenant_id: &str) -> u64 {
    count_docs(
        backend,
        "resources",
        doc! { "tenant_id": tenant_id, "search_rows_pending": { "$exists": true } },
    )
    .await
}

/// `(search_index, search_index_contained)` rows of one resource.
pub(super) async fn row_counts(
    backend: &MongoBackend,
    tenant_id: &str,
    resource_type: &str,
    id: &str,
) -> (u64, u64) {
    let key = doc! {
        "tenant_id": tenant_id,
        "resource_type": resource_type,
        "resource_id": id,
    };
    (
        count_docs(backend, "search_index", key.clone()).await,
        count_docs(backend, "search_index_contained", key).await,
    )
}

async fn counts_for(
    backend: &MongoBackend,
    tenant_id: &str,
    resources: &[(String, String)],
) -> Vec<(u64, u64)> {
    let mut counts = Vec::new();
    for (resource_type, id) in resources {
        counts.push(row_counts(backend, tenant_id, resource_type, id).await);
    }
    counts
}

/// `value` as JSON with every object's keys sorted.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

/// Every row of one resource in both row collections, minus `_id`, `resource_id`
/// and `tenant_id`, as canonical JSON, sorted. Two resources with equal
/// fingerprints have the same rows (a duplicate row set would differ).
async fn row_fingerprints(
    backend: &MongoBackend,
    tenant_id: &str,
    resource_type: &str,
    id: &str,
) -> Vec<String> {
    let db = raw_db(backend).await;
    let mut out = Vec::new();
    for collection in ["search_index", "search_index_contained"] {
        let rows: Vec<Document> = db
            .collection::<Document>(collection)
            .find(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": id,
            })
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        for mut row in rows {
            row.remove("_id");
            row.remove("resource_id");
            row.remove("tenant_id");
            let value = Bson::Document(row).into_relaxed_extjson();
            out.push(format!("{collection}:{}", canonical(&value)));
        }
    }
    out.sort();
    out
}

/// [`row_fingerprints`] of each resource, with the Bundle's own ids replaced by
/// `<idN>` (N = the resource's position), so Bundles run on different backends
/// compare equal when their rows do.
pub(super) async fn normalized_fingerprints(
    backend: &MongoBackend,
    tenant_id: &str,
    resources: &[(String, String)],
) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for (resource_type, id) in resources {
        let mut rows = row_fingerprints(backend, tenant_id, resource_type, id).await;
        for row in &mut rows {
            for (n, (_, known)) in resources.iter().enumerate() {
                *row = row.replace(known.as_str(), &format!("<id{n}>"));
            }
        }
        rows.sort();
        out.push(rows);
    }
    out
}

/// `(type, id)` of every `resources` document of the tenant, in no set order.
async fn stored_refs(backend: &MongoBackend, tenant_id: &str) -> Vec<(String, String)> {
    let rows: Vec<Document> = raw_db(backend)
        .await
        .collection::<Document>("resources")
        .find(doc! { "tenant_id": tenant_id })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    rows.iter()
        .map(|row| {
            (
                row.get_str("resource_type").unwrap().to_string(),
                row.get_str("id").unwrap().to_string(),
            )
        })
        .collect()
}

async fn search_ids(
    backend: &MongoBackend,
    tenant: &TenantContext,
    query: SearchQuery,
) -> Vec<String> {
    let mut ids: Vec<String> = backend
        .search(tenant, &query)
        .await
        .expect("search")
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_string())
        .collect();
    ids.sort();
    ids
}

fn param(
    name: &str,
    param_type: SearchParamType,
    value: &str,
) -> helios_persistence::types::SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    }
}

fn token_param(name: &str, value: &str) -> SearchParameter {
    param(name, SearchParamType::Token, value)
}

fn string_param(name: &str, value: &str) -> SearchParameter {
    param(name, SearchParamType::String, value)
}

pub(super) fn reference_param(name: &str, value: &str) -> SearchParameter {
    param(name, SearchParamType::Reference, value)
}

/// Ids of the Patients with identifier `MRN_SYSTEM|<mrn>`.
pub(super) async fn patients_by_mrn(
    backend: &MongoBackend,
    tenant: &TenantContext,
    mrn: &str,
) -> Vec<String> {
    search_ids(
        backend,
        tenant,
        SearchQuery::new("Patient")
            .with_parameter(token_param("identifier", &format!("{MRN_SYSTEM}|{mrn}"))),
    )
    .await
}

pub(super) async fn patients_by_family(
    backend: &MongoBackend,
    tenant: &TenantContext,
    family: &str,
) -> Vec<String> {
    search_ids(
        backend,
        tenant,
        SearchQuery::new("Patient").with_parameter(string_param("family", family)),
    )
    .await
}

/// The `missing_params` array of one `resources` document (#1984), or `None`
/// when the document has none: a failed extraction removes it.
async fn missing_params_of(
    backend: &MongoBackend,
    tenant_id: &str,
    resource_type: &str,
    id: &str,
) -> Option<Vec<String>> {
    let stored = raw_db(backend)
        .await
        .collection::<Document>("resources")
        .find_one(doc! { "tenant_id": tenant_id, "resource_type": resource_type, "id": id })
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{resource_type}/{id} is stored"));
    let names = stored.get_array("missing_params").ok()?;
    Some(
        names
            .iter()
            .map(|name| name.as_str().expect("a parameter name").to_string())
            .collect(),
    )
}

/// The `missing_params_fp` stamp of one `resources` document (#1999), or `None`
/// when the document has none. The stamp depends only on the active definitions
/// of the resource's type, so two backends with the same definitions give equal
/// stamps. A deferred create has none until its batch (or the repair) writes it.
async fn missing_params_fp_of(
    backend: &MongoBackend,
    tenant_id: &str,
    resource_type: &str,
    id: &str,
) -> Option<String> {
    let stored = raw_db(backend)
        .await
        .collection::<Document>("resources")
        .find_one(doc! { "tenant_id": tenant_id, "resource_type": resource_type, "id": id })
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{resource_type}/{id} is stored"));
    stored.get_str("missing_params_fp").ok().map(str::to_string)
}

/// `name:missing=<missing>` as a search parameter.
fn missing_param(name: &str, param_type: SearchParamType, missing: bool) -> SearchParameter {
    SearchParameter {
        modifier: Some(SearchModifier::Missing),
        ..param(name, param_type, if missing { "true" } else { "false" })
    }
}

/// Ids of the resources of `resource_type` that `name:missing=<missing>` finds.
async fn ids_missing(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
    name: &str,
    param_type: SearchParamType,
    missing: bool,
) -> Vec<String> {
    search_ids(
        backend,
        tenant,
        SearchQuery::new(resource_type).with_parameter(missing_param(name, param_type, missing)),
    )
    .await
}

async fn observations_of(
    backend: &MongoBackend,
    tenant: &TenantContext,
    patient_id: &str,
) -> Vec<String> {
    search_ids(
        backend,
        tenant,
        SearchQuery::new("Observation")
            .with_parameter(reference_param("subject", &format!("Patient/{patient_id}"))),
    )
    .await
}

/// Everything a [`patient_bundle`] is searched by finds it, and every resource
/// of it has rows. `resources` is the Bundle's `located` result.
async fn assert_bundle_searchable(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resources: &[(String, String)],
    family: &str,
    mrn: &str,
    n_obs: usize,
) {
    let patient_id = &resources[0].1;
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
    assert_eq!(
        observations_of(backend, tenant, patient_id).await.len(),
        n_obs,
        "Observation?subject"
    );
    let tenant_id = tenant.tenant_id().as_str();
    for (resource_type, id) in resources {
        let (own, _) = row_counts(backend, tenant_id, resource_type, id).await;
        assert!(own > 0, "{resource_type}/{id} has search rows");
    }
}

async fn assert_no_rows(backend: &MongoBackend, tenant_id: &str, resources: &[(String, String)]) {
    for (resource_type, id) in resources {
        assert_eq!(
            row_counts(backend, tenant_id, resource_type, id).await,
            (0, 0),
            "{resource_type}/{id} has no search rows yet"
        );
    }
}

/// The first repair pass (no minimum age) after which the ledger is empty. A
/// pass can lose a write conflict to a transaction the test left behind and
/// leave its chunk for the next one, so this retries for a few seconds.
async fn repair_until_clear(
    backend: &MongoBackend,
) -> helios_persistence::backends::mongodb::DeferredIndexRepairReport {
    for _ in 0..20 {
        let report = backend
            .repair_deferred_search_index(Duration::ZERO, 10)
            .await
            .expect("repair pass");
        if report.remaining == 0 {
            return report;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the repair did not clear the ledger in 10 s");
}

// ---------------------------------------------------------------------------
// The oplog
// ---------------------------------------------------------------------------

/// The newest oplog timestamp, or `None` when the oplog cannot be read (an
/// external standalone server).
async fn oplog_head(client: &Client) -> Option<Timestamp> {
    let head = client
        .database("local")
        .collection::<Document>("oplog.rs")
        .find_one(doc! {})
        .sort(doc! { "$natural": -1_i32 })
        .await
        .ok()??;
    head.get_timestamp("ts").ok()
}

/// The `applyOps` operations of every transaction committed after `since` that
/// touched `db_name`, oldest first, one entry per committed transaction.
async fn committed_txns(client: &Client, db_name: &str, since: Timestamp) -> Vec<Vec<Document>> {
    let entries: Vec<Document> = client
        .database("local")
        .collection::<Document>("oplog.rs")
        .find(doc! {
            "ts": { "$gt": since },
            "op": "c",
            "o.applyOps.ns": { "$regex": format!("^{db_name}\\.") },
        })
        .sort(doc! { "ts": 1_i32 })
        .await
        .expect("read the oplog")
        .try_collect()
        .await
        .expect("read the oplog");
    entries
        .iter()
        .filter_map(|entry| {
            let ops = entry.get_document("o").ok()?.get_array("applyOps").ok()?;
            Some(
                ops.iter()
                    .filter_map(|op| op.as_document().cloned())
                    .collect(),
            )
        })
        .collect()
}

/// A window on the oplog that opens when it is taken.
struct Oplog {
    client: Client,
    since: Timestamp,
}

impl Oplog {
    /// Marks the current end of the oplog. On the harness's own replica set the
    /// oplog must be readable, and a failure to read it fails the test: the
    /// oplog assertions are the proof of what the transaction wrote, and they
    /// must not vanish quietly. Against an external `HFS_TEST_MONGODB_URL`,
    /// which may be a standalone server, it returns `None` (after saying so)
    /// and the caller skips its oplog assertions.
    async fn begin(backend: &MongoBackend) -> Option<Oplog> {
        let client = raw_test_client(&backend.config().connection_string)
            .await
            .ok();
        let since = match &client {
            Some(client) => oplog_head(client).await,
            None => None,
        };
        match (client, since) {
            (Some(client), Some(since)) => Some(Oplog { client, since }),
            _ => {
                assert!(
                    !transactions_required(),
                    "the harness's own Mongo replica set must expose local.oplog.rs to the \
                     test client; the oplog assertions would otherwise be skipped silently"
                );
                eprintln!("the oplog cannot be read here; skipping the oplog assertions");
                None
            }
        }
    }

    /// The transactions committed on `backend`'s database since [`Oplog::begin`].
    async fn committed(&self, backend: &MongoBackend) -> Vec<Vec<Document>> {
        committed_txns(&self.client, &backend.config().database_name, self.since).await
    }
}

fn count_ops(ops: &[Document], db: &str, collection: &str, op: &str) -> usize {
    let ns = format!("{db}.{collection}");
    ops.iter()
        .filter(|entry| {
            entry.get_str("ns").is_ok_and(|v| v == ns) && entry.get_str("op").is_ok_and(|v| v == op)
        })
        .count()
}

/// The inserted documents of a transaction's `resources` inserts.
fn resource_inserts<'a>(ops: &'a [Document], db: &str) -> Vec<&'a Document> {
    let ns = format!("{db}.resources");
    ops.iter()
        .filter(|entry| {
            entry.get_str("ns").is_ok_and(|v| v == ns)
                && entry.get_str("op").is_ok_and(|v| v == "i")
        })
        .filter_map(|entry| entry.get_document("o").ok())
        .collect()
}

/// The transaction that inserted into `resources`: the Bundle's own. The
/// post-commit batches only update `resources`.
fn bundle_txn<'a>(txns: &'a [Vec<Document>], db: &str) -> &'a [Document] {
    txns.iter()
        .find(|ops| count_ops(ops, db, "resources", "i") > 0)
        .expect("the oplog holds the Bundle's transaction")
}

// ---------------------------------------------------------------------------
// Below the threshold, and with the deferral off: unchanged
// ---------------------------------------------------------------------------

/// A Bundle that does not defer: its transaction holds exactly the search rows
/// that exist afterwards, no ledger and no marker, and nothing else commits.
async fn assert_inline_bundle(test: &str, min_entries: Option<usize>, n_obs: usize) {
    let Some(backend) = deferred_backend(test, test, min_entries, |_| {}).await else {
        eprintln!("Skipping {test} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-inline");
    let tenant_id = tenant.tenant_id().as_str();
    let oplog = Oplog::begin(&backend).await;

    let entries = patient_bundle(n_obs, "Inline", "MRN-INLINE");
    let count = entries.len();
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, test).await else {
        return;
    };
    assert_eq!(result.entries.len(), count);
    let resources = located(&result);

    assert_eq!(ledger_count(&backend).await, 0, "no ledger");
    assert_eq!(marker_count(&backend, tenant_id).await, 0, "no marker");
    assert_bundle_searchable(&backend, &tenant, &resources, "Inline", "MRN-INLINE", n_obs).await;

    let own = count_docs(&backend, "search_index", doc! { "tenant_id": tenant_id }).await;
    let contained = count_docs(
        &backend,
        "search_index_contained",
        doc! { "tenant_id": tenant_id },
    )
    .await;
    assert!(own > 0 && contained > 0);

    if let Some(oplog) = &oplog {
        let db = backend.config().database_name.clone();
        let txns = oplog.committed(&backend).await;
        assert_eq!(txns.len(), 1, "one transaction: the Bundle's, no batches");
        let ops = bundle_txn(&txns, &db);
        assert_eq!(count_ops(ops, &db, "search_index", "i") as u64, own);
        assert_eq!(
            count_ops(ops, &db, "search_index_contained", "i") as u64,
            contained
        );
        assert_eq!(count_ops(ops, &db, "search_index_pending", "i"), 0);
        assert!(
            resource_inserts(ops, &db)
                .iter()
                .all(|inserted| !inserted.contains_key("search_rows_pending")),
            "no resource carries a marker"
        );
    }
}

/// Four entries are below the threshold of 5: the rows are written inside the
/// transaction, as before #1975.
#[tokio::test]
async fn a_bundle_below_the_threshold_writes_its_search_rows_inside_the_transaction() {
    assert_inline_bundle("def_below", Some(THRESHOLD), 2).await;
}

/// `Some(0)` turns the deferral off: even a 6-entry Bundle keeps today's
/// operations.
#[tokio::test]
async fn a_bundle_with_the_deferral_off_writes_its_search_rows_inside_the_transaction() {
    assert_inline_bundle("def_off", Some(0), 4).await;
}

// ---------------------------------------------------------------------------
// Above the threshold
// ---------------------------------------------------------------------------

/// The deferred path: the Bundle's transaction writes `resources`, history and
/// one ledger document and no search row, and no `missing_params` or
/// `missing_params_fp` on the resources it creates (#1975); each of the three
/// batches writes one type's rows and then, in one update statement per
/// resource, clears its marker and writes its `missing_params`; afterwards
/// everything is searchable.
#[tokio::test]
async fn a_deferred_bundle_commits_without_search_writes_and_is_searchable_after_the_response() {
    const NAME: &str = "a_deferred_bundle_commits_without_search_writes";
    let Some(backend) = deferred_backend("def_commit", "def-commit", Some(THRESHOLD), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-commit");
    let tenant_id = tenant.tenant_id().as_str();
    let oplog = Oplog::begin(&backend).await;

    let entries = patient_bundle(4, "Deferred", "MRN-T2");
    assert_eq!(entries.len(), 6);
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, NAME).await else {
        return;
    };
    let resources = located(&result);

    // The response is not sent before the rows are there.
    assert_eq!(ledger_count(&backend).await, 0, "the ledger is cleared");
    assert_eq!(marker_count(&backend, tenant_id).await, 0, "no marker left");
    assert_bundle_searchable(&backend, &tenant, &resources, "Deferred", "MRN-T2", 4).await;

    let patient_id = &resources[0].1;
    // The Patient has a family and no gender: `:missing=true` is answered by
    // its `missing_params`, `:missing=false` by its rows.
    assert_eq!(
        ids_missing(
            &backend,
            &tenant,
            "Patient",
            "gender",
            SearchParamType::Token,
            true
        )
        .await,
        vec![patient_id.clone()],
        "Patient?gender:missing=true"
    );
    assert_eq!(
        ids_missing(
            &backend,
            &tenant,
            "Patient",
            "family",
            SearchParamType::String,
            false
        )
        .await,
        vec![patient_id.clone()],
        "Patient?family:missing=false"
    );
    let medication_request = &resources[5];
    assert!(
        row_counts(
            &backend,
            tenant_id,
            &medication_request.0,
            &medication_request.1
        )
        .await
        .1 > 0,
        "the contained Medication has rows in search_index_contained"
    );

    // _include and _revinclude find the Bundle's references.
    let observations = backend
        .search(
            &tenant,
            &SearchQuery::new("Observation")
                .with_parameter(reference_param("subject", &format!("Patient/{patient_id}"))),
        )
        .await
        .unwrap()
        .resources
        .items;
    let forward = IncludeDirective {
        include_type: IncludeType::Include,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: Some("Patient".to_string()),
        iterate: false,
    };
    let included = backend
        .resolve_includes(&tenant, &observations[..1], &[forward])
        .await
        .expect("_include");
    assert_eq!(included.len(), 1);
    assert_eq!(included[0].id(), patient_id);

    let reverse = IncludeDirective {
        include_type: IncludeType::Revinclude,
        source_type: "Observation".to_string(),
        search_param: "subject".to_string(),
        target_type: None,
        iterate: false,
    };
    let with_revinclude = backend
        .search(
            &tenant,
            &SearchQuery::new("Patient")
                .with_parameter(token_param("identifier", &format!("{MRN_SYSTEM}|MRN-T2")))
                .with_include(reverse),
        )
        .await
        .expect("_revinclude");
    assert_eq!(
        with_revinclude
            .included
            .iter()
            .filter(|r| r.resource_type() == "Observation")
            .count(),
        4,
        "_revinclude brings the four Observations"
    );

    // The transaction wrote no `missing_params`; the batches have by now.
    for (resource_type, id) in &resources {
        assert!(
            missing_params_of(&backend, tenant_id, resource_type, id)
                .await
                .is_some(),
            "{resource_type}/{id} has missing_params once its batch has run"
        );
    }

    let Some(oplog) = oplog else { return };
    let db = backend.config().database_name.clone();
    let txns = oplog.committed(&backend).await;
    let ops = bundle_txn(&txns, &db);
    assert_eq!(count_ops(ops, &db, "search_index", "i"), 0);
    assert_eq!(count_ops(ops, &db, "search_index_contained", "i"), 0);
    // No assertion on deletes here: a `delete_many` that matches no document
    // writes no oplog entry, with or without the skip, and these resources are
    // new. `deferred_updates_and_deletes_cut_before_indexing_...` proves the
    // skipped deletes: with the post-commit step cut, the rows of the updated
    // and the deleted Patient are still there until the repair.
    assert_eq!(
        count_ops(ops, &db, "search_index_pending", "i"),
        1,
        "one ledger document for six resources"
    );
    assert_eq!(count_ops(ops, &db, "resource_history", "i"), 6);
    let inserted = resource_inserts(ops, &db);
    assert_eq!(inserted.len(), 6);
    assert!(
        inserted
            .iter()
            .all(|resource| resource.get_str("search_rows_pending").is_ok()),
        "every resource of the Bundle is marked"
    );
    assert!(
        inserted.iter().all(|resource| {
            !resource.contains_key("missing_params") && !resource.contains_key("missing_params_fp")
        }),
        "a deferred create's metadata is written by its batch, not by the transaction"
    );

    // One batch transaction per type (Patient, Observation, MedicationRequest);
    // each writes its type's rows and then, in one update statement per
    // resource, clears the marker and rewrites `missing_params`: still one `u`
    // op per resource.
    assert_eq!(txns.len(), 4, "the Bundle plus three batches");
    let mut rows = 0;
    let mut cleared = 0;
    for batch in txns
        .iter()
        .filter(|candidate| count_ops(candidate, &db, "resources", "i") == 0)
    {
        assert_eq!(count_ops(batch, &db, "resource_history", "i"), 0);
        assert!(count_ops(batch, &db, "search_index", "i") > 0);
        rows += count_ops(batch, &db, "search_index", "i");
        cleared += count_ops(batch, &db, "resources", "u");
    }
    assert_eq!(cleared, 6, "the batches clear six markers");
    assert_eq!(
        rows as u64,
        count_docs(&backend, "search_index", doc! { "tenant_id": tenant_id }).await
    );
}

/// The rule is `entries >= N`: a Bundle of exactly 5 entries defers.
#[tokio::test]
async fn a_bundle_of_exactly_the_threshold_defers() {
    const NAME: &str = "a_bundle_of_exactly_the_threshold_defers";
    let Some(backend) = deferred_backend("def_exact", "def-exact", Some(THRESHOLD), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-exact");
    let oplog = Oplog::begin(&backend).await;

    let entries = patient_bundle(3, "Exact", "MRN-EXACT");
    assert_eq!(entries.len(), THRESHOLD);
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, NAME).await else {
        return;
    };
    assert_bundle_searchable(
        &backend,
        &tenant,
        &located(&result),
        "Exact",
        "MRN-EXACT",
        3,
    )
    .await;

    if let Some(oplog) = oplog {
        let db = backend.config().database_name.clone();
        let txns = oplog.committed(&backend).await;
        let ops = bundle_txn(&txns, &db);
        assert_eq!(count_ops(ops, &db, "search_index", "i"), 0);
        assert_eq!(count_ops(ops, &db, "search_index_pending", "i"), 1);
    }
}

/// The deferred rows are the rows the inline path writes: the same Bundle on an
/// inline backend and on a deferred one leaves equal rows for every entry, own
/// and contained, with ids and references normalized.
#[tokio::test]
async fn deferred_rows_match_inline_rows() {
    const NAME: &str = "deferred_rows_match_inline_rows";
    let Some(inline) =
        deferred_backend("def_twin_inline", "def-twin-inline", Some(0), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some(deferred) = deferred_backend(
        "def_twin_deferred",
        "def-twin-deferred",
        Some(THRESHOLD),
        |_| {},
    )
    .await
    else {
        return;
    };
    let tenant_a = create_tenant("def-twin-a");
    let tenant_b = create_tenant("def-twin-b");

    let Some(inline_result) = process_transaction_or_skip(
        &inline,
        &tenant_a,
        patient_bundle(4, "Twin", "MRN-TWIN"),
        NAME,
    )
    .await
    else {
        return;
    };
    let Some(deferred_result) = process_transaction_or_skip(
        &deferred,
        &tenant_b,
        patient_bundle(4, "Twin", "MRN-TWIN"),
        NAME,
    )
    .await
    else {
        return;
    };
    assert_eq!(ledger_count(&deferred).await, 0);

    let inline_resources = located(&inline_result);
    let deferred_resources = located(&deferred_result);
    let inline_rows = normalized_fingerprints(&inline, "def-twin-a", &inline_resources).await;
    let deferred_rows = normalized_fingerprints(&deferred, "def-twin-b", &deferred_resources).await;
    assert_eq!(inline_rows.len(), 6);
    for (n, (a, b)) in inline_rows.iter().zip(&deferred_rows).enumerate() {
        assert!(!a.is_empty(), "entry {n} has rows");
        assert_eq!(a, b, "entry {n}: deferred rows equal inline rows");
    }
    assert!(
        inline_rows
            .last()
            .unwrap()
            .iter()
            .any(|row| row.starts_with("search_index_contained:")),
        "the MedicationRequest's contained rows are compared too"
    );

    // The same extraction gives both paths the same `missing_params` (#1984).
    for (n, ((inline_type, inline_id), (deferred_type, deferred_id))) in
        inline_resources.iter().zip(&deferred_resources).enumerate()
    {
        let inline_missing = missing_params_of(&inline, "def-twin-a", inline_type, inline_id).await;
        let deferred_missing =
            missing_params_of(&deferred, "def-twin-b", deferred_type, deferred_id).await;
        assert!(inline_missing.is_some(), "entry {n} has missing_params");
        assert_eq!(
            inline_missing, deferred_missing,
            "entry {n}: deferred missing_params equal inline missing_params"
        );
        // The transaction writes no stamp for a deferred create, so the batch
        // is the stamp's only writer (#1975).
        let inline_fp = missing_params_fp_of(&inline, "def-twin-a", inline_type, inline_id).await;
        let deferred_fp =
            missing_params_fp_of(&deferred, "def-twin-b", deferred_type, deferred_id).await;
        assert!(
            deferred_fp.is_some(),
            "entry {n}: the batch stamps missing_params_fp on a deferred create (#1975)"
        );
        assert_eq!(
            inline_fp, deferred_fp,
            "entry {n}: deferred missing_params_fp equals inline missing_params_fp"
        );
        if n == 0 {
            // The Patient has a family and no gender.
            let names = deferred_missing.unwrap();
            assert!(names.iter().any(|name| name == "gender"), "{names:?}");
            assert!(!names.iter().any(|name| name == "family"), "{names:?}");
        }
    }
}

/// The Bundle's transaction writes neither the rows nor the `missing_params` of
/// the resources it creates; its batch writes both (#1975, #1984). With the step
/// cut by a 1 ms deadline both are absent until the repair, so a created
/// resource has no `missing_params` and `:missing` is answered from the rows: a
/// resource without rows lists as missing every parameter, `:missing=true`
/// finds the Patient for `family` although it has a family, and `:missing=false`
/// finds nothing yet. That is the documented lag (README, Consistency). The
/// repair writes the rows and the `missing_params`, and all four answers are
/// right.
#[tokio::test]
async fn a_deferred_bundle_answers_missing_from_its_rows_until_its_batch_writes_metadata() {
    const NAME: &str = "a_deferred_bundle_answers_missing_from_its_rows";
    let Some(backend) = deferred_backend(
        "def_missing_lag",
        "def-missing-lag",
        Some(THRESHOLD),
        |config| config.bundle_transaction_deadline = Some(Duration::from_millis(1)),
    )
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    // `:missing=true` takes the `missing_params` route only once its index is
    // built; before that it reads rows, and this test would prove nothing.
    assert!(
        matches!(
            backend.wait_for_missing_index_build().await,
            Some(BuildOutcome::UpToDate | BuildOutcome::Built { .. })
        ),
        "idx_resources_missing_params is built"
    );
    let tenant = create_tenant("def-missing-lag");
    let tenant_id = tenant.tenant_id().as_str();

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        patient_bundle(4, "MissingLag", "MRN-MISSING-LAG"),
        NAME,
    )
    .await
    else {
        return;
    };
    let resources = located(&result);
    let patient_id = resources[0].1.clone();
    assert_eq!(ledger_count(&backend).await, 1, "the step was cut");
    assert_eq!(marker_count(&backend, tenant_id).await, 6);
    assert_no_rows(&backend, tenant_id, &resources).await;

    for (resource_type, id) in &resources {
        assert_eq!(
            missing_params_of(&backend, tenant_id, resource_type, id).await,
            None,
            "{resource_type}/{id} has no missing_params from the transaction"
        );
    }

    // Before the rows: with no metadata and no rows, the Patient lists as
    // missing every parameter. `gender` is right by that; `family` is the lag.
    let found = |name: &'static str, param_type: SearchParamType, missing: bool| {
        ids_missing(&backend, &tenant, "Patient", name, param_type, missing)
    };
    assert_eq!(
        found("gender", SearchParamType::Token, true).await,
        vec![patient_id.clone()],
        "Patient?gender:missing=true before the rows"
    );
    assert_eq!(
        found("family", SearchParamType::String, true).await,
        vec![patient_id.clone()],
        "Patient?family:missing=true before the rows lists the Patient although it has a \
         family: the documented lag, a metadata-less resource without rows"
    );
    assert!(
        found("family", SearchParamType::String, false)
            .await
            .is_empty(),
        "Patient?family:missing=false reads rows, which the repair has yet to write"
    );

    let repair = same_db_backend(&backend, "def-missing-lag-repair")
        .await
        .unwrap();
    repair_until_clear(&repair).await;

    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_eq!(
        found("family", SearchParamType::String, false).await,
        vec![patient_id.clone()],
        "Patient?family:missing=false after the rows"
    );
    assert_eq!(
        found("gender", SearchParamType::Token, true).await,
        vec![patient_id.clone()],
        "Patient?gender:missing=true after the rows"
    );
    assert!(
        found("gender", SearchParamType::Token, false)
            .await
            .is_empty(),
        "Patient?gender:missing=false after the rows"
    );
    assert!(
        found("family", SearchParamType::String, true)
            .await
            .is_empty(),
        "Patient?family:missing=true after the rows"
    );
    for (resource_type, id) in &resources {
        assert!(
            missing_params_of(&backend, tenant_id, resource_type, id)
                .await
                .is_some(),
            "{resource_type}/{id}: the repair writes missing_params"
        );
    }
    let names = missing_params_of(&backend, tenant_id, "Patient", &patient_id)
        .await
        .unwrap();
    assert!(names.iter().any(|name| name == "gender"), "{names:?}");
    assert!(!names.iter().any(|name| name == "family"), "{names:?}");
}

/// A SearchParameter created by a deferred Bundle applies to the rows and to
/// the `missing_params` of the Patients created beside it: the cache is
/// reloaded after the commit and before the post-commit step, and the step
/// writes both from the committed registry. The transaction itself writes no
/// `missing_params` for a create (#1975); an extraction there would have used
/// the registry as it was, which knew nothing of the new parameter, and the
/// Patients that lack the value would not list it as missing and would match
/// neither `:missing=true` nor `:missing=false`.
///
/// There is no inline twin: an inline Bundle extracts its entries before the
/// reload too. A `$reindex` of the same Patients is the reference instead: it
/// extracts with the committed registry, so it must change nothing.
#[tokio::test]
async fn a_search_parameter_created_in_a_deferred_bundle_applies_to_its_missing_params() {
    use helios_persistence::search::ReindexTarget;

    const NAME: &str =
        "a_search_parameter_created_in_a_deferred_bundle_applies_to_its_missing_params";
    let Some(backend) =
        deferred_backend("def_sp_missing", "def-sp-missing", Some(THRESHOLD), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-sp-missing");
    let tenant_id = tenant.tenant_id().as_str();

    let nickname_patient = json!({
        "resourceType": "Patient",
        "name": [{"family": "SpWith"}, {"use": "nickname", "given": ["Nick"]}],
    });
    let plain_patient = || {
        json!({
            "resourceType": "Patient",
            "name": [{"family": "SpWithout"}],
        })
    };
    let entries = vec![
        post(
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "url": "http://example.org/fhir/SearchParameter/def-nickname",
                "name": "DefNickname",
                "status": "active",
                "code": "def-nickname",
                "base": ["Patient"],
                "type": "string",
                "expression": "Patient.name.where(use='nickname').given",
            }),
            &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        ),
        post(
            "Patient",
            nickname_patient,
            &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        ),
        post(
            "Patient",
            plain_patient(),
            &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        ),
        post(
            "Patient",
            plain_patient(),
            &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        ),
        post(
            "Patient",
            plain_patient(),
            &format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        ),
    ];
    assert_eq!(entries.len(), THRESHOLD);
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, NAME).await else {
        return;
    };
    let resources = located(&result);
    assert_eq!(ledger_count(&backend).await, 0);
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    let patients: Vec<String> = resources[1..].iter().map(|(_, id)| id.clone()).collect();
    let (with_nickname, without_nickname) = (&patients[0], &patients[1..]);

    // The Patients without a nickname list the new parameter as missing, the
    // one with a nickname does not.
    let listed = |names: &Option<Vec<String>>| {
        names
            .as_ref()
            .expect("the Patient has missing_params")
            .iter()
            .any(|name| name == "def-nickname")
    };
    assert!(
        !listed(&missing_params_of(&backend, tenant_id, "Patient", with_nickname).await),
        "the Patient with a nickname"
    );
    for id in without_nickname {
        assert!(
            listed(&missing_params_of(&backend, tenant_id, "Patient", id).await),
            "Patient/{id} lacks a nickname and lists def-nickname as missing"
        );
    }

    let mut expected_missing = without_nickname.to_vec();
    expected_missing.sort();
    assert_eq!(
        ids_missing(
            &backend,
            &tenant,
            "Patient",
            "def-nickname",
            SearchParamType::String,
            true
        )
        .await,
        expected_missing,
        "Patient?def-nickname:missing=true"
    );
    assert_eq!(
        ids_missing(
            &backend,
            &tenant,
            "Patient",
            "def-nickname",
            SearchParamType::String,
            false
        )
        .await,
        vec![with_nickname.clone()],
        "Patient?def-nickname:missing=false"
    );
    assert_eq!(
        search_ids(
            &backend,
            &tenant,
            SearchQuery::new("Patient").with_parameter(string_param("def-nickname", "Nick"))
        )
        .await,
        vec![with_nickname.clone()],
        "Patient?def-nickname=Nick"
    );

    // A $reindex of the same Patients extracts with the committed registry and
    // so rewrites nothing: rows and missing_params are already what it makes.
    let mut before = Vec::new();
    let mut stored = Vec::new();
    for id in &patients {
        before.push((
            row_fingerprints(&backend, tenant_id, "Patient", id).await,
            missing_params_of(&backend, tenant_id, "Patient", id).await,
        ));
        stored.push(
            backend
                .read(&tenant, "Patient", id)
                .await
                .unwrap()
                .expect("the Patient"),
        );
    }
    let outcomes = backend.write_search_entries_page(&tenant, &stored).await;
    assert!(outcomes.iter().all(Result::is_ok), "{outcomes:?}");
    for (id, (rows, missing)) in patients.iter().zip(&before) {
        assert_eq!(
            &row_fingerprints(&backend, tenant_id, "Patient", id).await,
            rows,
            "Patient/{id}: a reindex leaves the deferred rows as they are"
        );
        assert_eq!(
            &missing_params_of(&backend, tenant_id, "Patient", id).await,
            missing,
            "Patient/{id}: a reindex leaves missing_params as the step wrote them"
        );
    }
}

/// A type with more resources than one batch holds is written in several
/// batches: 300 Observations are two (256 and 44), the Patient one. The oplog
/// shows the Bundle's transaction and exactly those three batch transactions.
#[tokio::test]
async fn a_deferred_bundle_indexes_a_type_in_several_batches() {
    const NAME: &str = "a_deferred_bundle_indexes_a_type_in_several_batches";
    let Some(backend) =
        deferred_backend("def_batches", "def-batches", Some(THRESHOLD), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-batches");
    let tenant_id = tenant.tenant_id().as_str();
    let oplog = Oplog::begin(&backend).await;

    let urn = "urn:uuid:def-batches-patient";
    let mut entries = vec![post("Patient", patient_json("Batches", "MRN-BATCH"), urn)];
    for n in 0..300 {
        entries.push(post(
            "Observation",
            observation_json(urn, n),
            &format!("urn:uuid:def-batches-{n}"),
        ));
    }
    let Some(result) = process_transaction_or_skip(&backend, &tenant, entries, NAME).await else {
        return;
    };
    let resources = located(&result);
    let patient_id = &resources[0].1;

    let found = search_ids(
        &backend,
        &tenant,
        SearchQuery::new("Observation")
            .with_parameter(reference_param("subject", &format!("Patient/{patient_id}")))
            .with_count(500),
    )
    .await;
    assert_eq!(found.len(), 300, "every Observation is found by subject");
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_eq!(ledger_count(&backend).await, 0);

    let Some(oplog) = oplog else { return };
    let db = backend.config().database_name.clone();
    let txns = oplog.committed(&backend).await;
    assert_eq!(
        txns.len(),
        4,
        "the Bundle, one Patient batch, two Observation batches"
    );
    let mut cleared: Vec<usize> = txns
        .iter()
        .filter(|ops| count_ops(ops, &db, "resources", "i") == 0)
        .map(|batch| count_ops(batch, &db, "resources", "u"))
        .collect();
    cleared.sort_unstable();
    assert_eq!(cleared, vec![1, 44, 256], "markers cleared per batch");
}

// ---------------------------------------------------------------------------
// Interruption, repair, idempotence
// ---------------------------------------------------------------------------

/// Every post-commit batch fails (an `update` failpoint; a POST-only Bundle
/// sends no `update` of its own), so the Bundle commits with its ledger and its
/// markers and no rows. A restarted server's repair, which honours its minimum
/// age, then writes the rows and clears the ledger.
#[tokio::test]
async fn interrupted_post_commit_indexing_is_repaired_at_startup() {
    const NAME: &str = "interrupted_post_commit_indexing_is_repaired_at_startup";
    let app = "fp-def-startup";
    let Some(backend) = deferred_backend("def_startup", app, Some(THRESHOLD), |_| {}).await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-startup");
    let tenant_id = tenant.tenant_id().as_str();
    record_missing_fingerprints(&backend).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["update"], "errorCode": 2 },
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };

    let result = backend
        .process_transaction(
            &tenant,
            patient_bundle(4, "Startup", "MRN-START"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;
    if lacks_transactions(&result) {
        return;
    }
    let result = result.expect("a committed Bundle is not failed by its post-commit step");
    let resources = located(&result);
    assert_eq!(entered, 5, "the first batch is tried five times, then left");

    assert_eq!(ledger_count(&backend).await, 1);
    assert_eq!(marker_count(&backend, tenant_id).await, 6);
    assert_no_rows(&backend, tenant_id, &resources).await;
    assert!(
        patients_by_mrn(&backend, &tenant, "MRN-START")
            .await
            .is_empty()
    );

    // The ledger document as the transaction wrote it.
    let ledger: Document = raw_db(&backend)
        .await
        .collection::<Document>("search_index_pending")
        .find_one(doc! {})
        .await
        .unwrap()
        .expect("the ledger document");
    assert_eq!(ledger.get_str("tenant_id").unwrap(), tenant_id);
    assert!(ledger.get_str("bundle_id").is_ok());
    assert_eq!(ledger.get_i32("chunk").unwrap(), 0);
    assert_eq!(ledger.get_i32("chunks").unwrap(), 1);
    assert_eq!(ledger.get_i64("entries").unwrap(), 6);
    assert_eq!(ledger.get_i32("attempts").unwrap(), 0);
    assert_eq!(ledger.get_array("refs").unwrap().len(), 6);
    assert!(ledger.get_datetime("not_before").is_ok());

    let Some(restarted) = same_db_backend(&backend, "def-startup-restarted").await else {
        return;
    };
    let young = restarted
        .repair_deferred_search_index(Duration::from_secs(3600), 10)
        .await
        .unwrap();
    assert_eq!(
        young.examined, 0,
        "the chunk is younger than the minimum age"
    );
    assert_eq!(young.remaining, 1);
    assert_eq!(
        restarted
            .count_deferred_search_index_chunks()
            .await
            .unwrap(),
        1
    );

    let report = restarted
        .repair_deferred_search_index(Duration::ZERO, 10)
        .await
        .unwrap();
    assert_eq!(report.examined, 1);
    assert_eq!(report.resources, 6);
    assert_eq!(report.reindexed, 6);
    assert_eq!(report.cleared, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.remaining, 0);
    assert!(report.rows > 0);

    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_bundle_searchable(&backend, &tenant, &resources, "Startup", "MRN-START", 4).await;
}

/// The request is dropped while a batch is blocked inside the server (the client
/// went away): the half-done batch is not visible, and the repair finishes the
/// Bundle.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_the_request_during_post_commit_indexing_is_repaired() {
    const NAME: &str = "dropping_the_request_during_post_commit_indexing_is_repaired";
    let app = "fp-def-drop";
    let Some(backend) = deferred_backend("def_drop", app, Some(THRESHOLD), |_| {}).await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("def-drop");
    let tenant_id = tenant.tenant_id().as_str();
    record_missing_fingerprints(&backend).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["update"], "blockConnection": true, "blockTimeMS": 3000_i64 },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let task = {
        let backend = Arc::clone(&backend);
        tokio::spawn(async move {
            backend
                .process_transaction(
                    &create_tenant("def-drop"),
                    patient_bundle(4, "Dropped", "MRN-DROP"),
                    FhirVersion::default(),
                )
                .await
        })
    };

    // The Bundle has committed (its ledger is there) and its first batch is
    // blocked in the server.
    let waited = Instant::now();
    while ledger_count(&backend).await == 0 {
        assert!(
            waited.elapsed() < Duration::from_secs(10),
            "the Bundle never committed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    fail_point.wait_until_entered(1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !task.is_finished(),
        "the request is inside its blocked batch"
    );
    task.abort();
    let _ = task.await;

    let resources = stored_refs(&backend, tenant_id).await;
    assert_eq!(resources.len(), 6);
    assert_no_rows(&backend, tenant_id, &resources).await;
    assert_eq!(ledger_count(&backend).await, 1);

    // Past the block (and the abort of the dropped transaction behind it).
    tokio::time::sleep(Duration::from_millis(3500)).await;
    fail_point.off().await;
    let repair = same_db_backend(&backend, "def-drop-repair").await.unwrap();
    let report = repair_until_clear(&repair).await;
    assert_eq!(report.cleared, 1);

    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    for (resource_type, id) in &resources {
        assert!(
            row_counts(&backend, tenant_id, resource_type, id).await.0 > 0,
            "{resource_type}/{id} is indexed"
        );
    }
    assert_eq!(
        patients_by_mrn(&backend, &tenant, "MRN-DROP").await.len(),
        1
    );
}

/// With a 1 ms `bundle_transaction_deadline` the post-commit step starts no
/// batch: the Bundle is answered and the ledger is left to the repair.
#[tokio::test]
async fn post_commit_indexing_cut_by_the_deadline_is_left_to_the_repair() {
    const NAME: &str = "post_commit_indexing_cut_by_the_deadline_is_left_to_the_repair";
    let Some(backend) = deferred_backend("def_deadline", "def-deadline", Some(THRESHOLD), |c| {
        c.bundle_transaction_deadline = Some(Duration::from_millis(1));
    })
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-deadline");
    let tenant_id = tenant.tenant_id().as_str();

    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        patient_bundle(4, "Deadline", "MRN-DEADLINE"),
        NAME,
    )
    .await
    else {
        return;
    };
    let resources = located(&result);
    assert_eq!(ledger_count(&backend).await, 1);
    assert_eq!(marker_count(&backend, tenant_id).await, 6);
    assert_no_rows(&backend, tenant_id, &resources).await;

    let repair = same_db_backend(&backend, "def-deadline-repair")
        .await
        .unwrap();
    let report = repair
        .repair_deferred_search_index(Duration::ZERO, 10)
        .await
        .unwrap();
    assert_eq!(
        (
            report.examined,
            report.reindexed,
            report.cleared,
            report.failed,
            report.remaining
        ),
        (1, 6, 1, 0, 0)
    );
    assert_bundle_searchable(&backend, &tenant, &resources, "Deadline", "MRN-DEADLINE", 4).await;
}

/// Plants one ledger document naming `refs`, due now.
async fn plant_chunk(
    db: &mongodb::Database,
    tenant_id: &str,
    bundle_id: &str,
    chunk: i32,
    chunks: i32,
    refs: &[(String, String)],
) {
    let now = mongodb::bson::DateTime::now();
    db.collection::<Document>("search_index_pending")
        .insert_one(doc! {
            "_id": ObjectId::new(),
            "tenant_id": tenant_id,
            "bundle_id": bundle_id,
            "chunk": chunk,
            "chunks": chunks,
            "entries": refs.len() as i64,
            "created_at": now,
            "not_before": now,
            "attempts": 0_i32,
            "refs": refs
                .iter()
                .map(|(resource_type, id)| format!("{resource_type}/{id}"))
                .collect::<Vec<_>>(),
        })
        .await
        .unwrap();
}

/// Plants the state a deferred Bundle leaves: `search_rows_pending` on each
/// resource and one ledger document naming them all.
async fn replant_ledger(
    backend: &MongoBackend,
    tenant_id: &str,
    resources: &[(String, String)],
    stamp_markers: bool,
) {
    let db = raw_db(backend).await;
    if stamp_markers {
        db.collection::<Document>("resources")
            .update_many(
                doc! { "tenant_id": tenant_id },
                doc! { "$set": { "search_rows_pending": "replay" } },
            )
            .await
            .unwrap();
    }
    plant_chunk(&db, tenant_id, "replay", 0, 1, resources).await;
}

/// Running the rewrite again is a no-op or a faithful redo: with the markers and
/// a ledger document planted for an already indexed Bundle the repair leaves
/// exactly one row set per resource, and a ledger document whose resources carry
/// no marker is cleared without touching a row.
#[tokio::test]
async fn repair_and_post_commit_indexing_are_idempotent() {
    const NAME: &str = "repair_and_post_commit_indexing_are_idempotent";
    let Some(backend) = deferred_backend("def_idem", "def-idem", Some(THRESHOLD), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-idem");
    let tenant_id = tenant.tenant_id().as_str();
    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        patient_bundle(4, "Idem", "MRN-IDEM"),
        NAME,
    )
    .await
    else {
        return;
    };
    let resources = located(&result);
    let snapshot = normalized_fingerprints(&backend, tenant_id, &resources).await;
    assert!(snapshot.iter().all(|rows| !rows.is_empty()));

    // 1. Markers and a ledger document again: the rows are rewritten, once.
    replant_ledger(&backend, tenant_id, &resources, true).await;
    let report = backend
        .repair_deferred_search_index(Duration::ZERO, 10)
        .await
        .unwrap();
    assert_eq!(
        (report.examined, report.reindexed, report.cleared),
        (1, 6, 1)
    );
    assert_eq!(
        normalized_fingerprints(&backend, tenant_id, &resources).await,
        snapshot,
        "exactly one row set per resource"
    );
    assert_eq!(marker_count(&backend, tenant_id).await, 0);

    // 2. Nothing left to do.
    let again = backend
        .repair_deferred_search_index(Duration::ZERO, 10)
        .await
        .unwrap();
    assert_eq!((again.examined, again.remaining), (0, 0));

    // 3. A ledger document with no marker behind it is cleared and rewrites nothing.
    replant_ledger(&backend, tenant_id, &resources, false).await;
    let report = backend
        .repair_deferred_search_index(Duration::ZERO, 10)
        .await
        .unwrap();
    assert_eq!(
        (
            report.examined,
            report.reindexed,
            report.rows,
            report.cleared
        ),
        (1, 0, 0, 1)
    );
    assert_eq!(
        normalized_fingerprints(&backend, tenant_id, &resources).await,
        snapshot
    );
}

/// A repair that cannot write a chunk keeps it: the failure is recorded on the
/// document, which waits another minimum age, and a later pass finishes it.
#[tokio::test]
async fn a_failed_repair_keeps_the_chunk_and_records_the_failure() {
    const NAME: &str = "a_failed_repair_keeps_the_chunk_and_records_the_failure";
    let Some(backend) =
        deferred_backend("def_repair_fail", "def-repair-fail", Some(THRESHOLD), |c| {
            c.bundle_transaction_deadline = Some(Duration::from_millis(1));
        })
        .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-repair-fail");
    let tenant_id = tenant.tenant_id().as_str();
    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        patient_bundle(4, "Failing", "MRN-FAIL"),
        NAME,
    )
    .await
    else {
        return;
    };
    let resources = located(&result);
    assert_eq!(ledger_count(&backend).await, 1);

    let app = "fp-def-repair-fail";
    let Some(repair) = same_db_backend(&backend, app).await else {
        return;
    };
    // The batch's row inserts fail. (An `update` failpoint would also fail the
    // `update` that records the failure on the ledger document.)
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["insert"], "errorCode": 2 },
        doc! { "times": 100 },
    )
    .await
    else {
        return;
    };
    let failed = repair
        .repair_deferred_search_index(Duration::ZERO, 10)
        .await
        .unwrap();
    fail_point.off().await;
    assert_eq!(
        (
            failed.examined,
            failed.cleared,
            failed.failed,
            failed.remaining
        ),
        (1, 0, 1, 1)
    );
    assert_eq!(marker_count(&backend, tenant_id).await, 6);

    let ledger: Document = raw_db(&backend)
        .await
        .collection::<Document>("search_index_pending")
        .find_one(doc! {})
        .await
        .unwrap()
        .expect("the chunk is kept");
    assert_eq!(ledger.get_i32("attempts").unwrap(), 1);
    assert!(!ledger.get_str("last_error").unwrap().is_empty());
    assert!(
        ledger.get_datetime("not_before").unwrap() > ledger.get_datetime("created_at").unwrap(),
        "the chunk waits another minimum age"
    );
    // Not yet due again for a long minimum age.
    let waiting = repair
        .repair_deferred_search_index(Duration::from_secs(3600), 10)
        .await
        .unwrap();
    assert_eq!((waiting.examined, waiting.remaining), (0, 1));

    let report = repair_until_clear(&repair).await;
    assert_eq!((report.cleared, report.failed), (1, 0));
    assert_bundle_searchable(&backend, &tenant, &resources, "Failing", "MRN-FAIL", 4).await;
}

/// A repair works through every batch of a ledger document it claimed: one
/// resource the repair cannot read (its `data` is a string) fails its own batch
/// only. The other batches, including the other type in the same document, are
/// written; the document with the poisoned resource is kept, with the failure
/// recorded, while the two healthy documents are cleared.
#[tokio::test]
async fn a_repair_finishes_every_healthy_batch_and_keeps_a_chunk_with_a_poisoned_one() {
    const NAME: &str =
        "a_repair_finishes_every_healthy_batch_and_keeps_a_chunk_with_a_poisoned_one";
    let Some(backend) = deferred_backend("def_poison", "def-poison", Some(THRESHOLD), |c| {
        c.bundle_transaction_deadline = Some(Duration::from_millis(1));
    })
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-poison");
    let tenant_id = tenant.tenant_id().as_str();
    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        patient_bundle(4, "Poison", "MRN-POISON"),
        NAME,
    )
    .await
    else {
        return;
    };
    // Patient, four Observations, one MedicationRequest.
    let resources = located(&result);
    assert_eq!(resources.len(), 6);
    assert_eq!(
        ledger_count(&backend).await,
        1,
        "the 1 ms deadline cut the step"
    );
    assert_eq!(marker_count(&backend, tenant_id).await, 6);

    // The one ledger document is replaced by three: the Patient; three
    // Observations; and the poisoned Observation followed by the
    // MedicationRequest, so a repair that stopped at its first failure would
    // never reach the MedicationRequest.
    let poisoned = resources[3].clone();
    let db = raw_db(&backend).await;
    db.collection::<Document>("search_index_pending")
        .delete_many(doc! {})
        .await
        .unwrap();
    plant_chunk(&db, tenant_id, "poison-0", 0, 3, &[resources[0].clone()]).await;
    plant_chunk(
        &db,
        tenant_id,
        "poison-1",
        1,
        3,
        &[
            resources[1].clone(),
            resources[2].clone(),
            resources[4].clone(),
        ],
    )
    .await;
    plant_chunk(
        &db,
        tenant_id,
        "poison-2",
        2,
        3,
        &[poisoned.clone(), resources[5].clone()],
    )
    .await;
    db.collection::<Document>("resources")
        .update_one(
            doc! { "tenant_id": tenant_id, "resource_type": "Observation", "id": &poisoned.1 },
            doc! { "$set": { "data": "not a document" } },
        )
        .await
        .unwrap();

    let repair = same_db_backend(&backend, "def-poison-repair")
        .await
        .unwrap();
    let report = repair
        .repair_deferred_search_index(Duration::ZERO, 10)
        .await
        .unwrap();
    assert_eq!(
        (
            report.examined,
            report.skipped,
            report.cleared,
            report.failed,
            report.remaining
        ),
        (3, 0, 2, 1, 1),
        "{report:?}"
    );

    for resource in resources.iter().filter(|resource| **resource != poisoned) {
        let (own, _) = row_counts(&backend, tenant_id, &resource.0, &resource.1).await;
        assert!(own > 0, "{}/{} has rows", resource.0, resource.1);
    }
    assert_eq!(
        row_counts(&backend, tenant_id, &poisoned.0, &poisoned.1).await,
        (0, 0),
        "the poisoned resource has no rows"
    );
    assert_eq!(
        marker_count(&backend, tenant_id).await,
        1,
        "only the poisoned resource keeps its marker"
    );
    assert_eq!(
        count_docs(
            &backend,
            "resources",
            doc! {
                "tenant_id": tenant_id,
                "id": &poisoned.1,
                "search_rows_pending": { "$exists": true },
            }
        )
        .await,
        1
    );

    let kept: Document = db
        .collection::<Document>("search_index_pending")
        .find_one(doc! {})
        .await
        .unwrap()
        .expect("the poisoned chunk is kept");
    assert_eq!(kept.get_str("bundle_id").unwrap(), "poison-2");
    assert_eq!(kept.get_i32("attempts").unwrap(), 1);
    assert!(
        kept.get_str("last_error").unwrap().contains(&poisoned.1),
        "the failure names the resource: {kept:?}"
    );
}

/// Two repairs (two instances, or a call that overlaps another) that read the
/// same ledger document do not both work on it: the claim lets one through and
/// the other skips it. Either way the Bundle ends with one row set per resource.
///
/// The claim moves `not_before` to now, so it only keeps the other repair out
/// for `min_age`: with a zero minimum age a repair that lists the ledger after
/// the claim finds the document due again and runs it a second time. The
/// ledger document is backdated and both repairs run with a real lease.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_repairs_write_one_row_set() {
    const NAME: &str = "two_concurrent_repairs_write_one_row_set";
    let Some(backend) = deferred_backend(
        "def_two_repairs",
        "def-two-repairs-a",
        Some(THRESHOLD),
        |_| {},
    )
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some(other) = same_db_backend(&backend, "def-two-repairs-b").await else {
        return;
    };
    let tenant = create_tenant("def-two-repairs");
    let tenant_id = tenant.tenant_id().as_str();
    let Some(result) = process_transaction_or_skip(
        &backend,
        &tenant,
        patient_bundle(4, "Twice", "MRN-TWICE"),
        NAME,
    )
    .await
    else {
        return;
    };
    let resources = located(&result);
    let snapshot = normalized_fingerprints(&backend, tenant_id, &resources).await;
    assert!(snapshot.iter().all(|rows| !rows.is_empty()));

    replant_ledger(&backend, tenant_id, &resources, true).await;
    const LEASE: Duration = Duration::from_secs(600);
    let due = mongodb::bson::DateTime::from_millis(
        mongodb::bson::DateTime::now().timestamp_millis() - 2 * LEASE.as_millis() as i64,
    );
    raw_db(&backend)
        .await
        .collection::<Document>("search_index_pending")
        .update_many(doc! {}, doc! { "$set": { "not_before": due } })
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        backend.repair_deferred_search_index(LEASE, 10),
        other.repair_deferred_search_index(LEASE, 10),
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(
        first.cleared + second.cleared,
        1,
        "{first:?} {second:?}: the document is finished once"
    );
    assert_eq!(first.examined + second.examined, 1, "{first:?} {second:?}");
    assert_eq!(first.failed + second.failed, 0, "{first:?} {second:?}");
    assert_eq!(ledger_count(&backend).await, 0);
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_eq!(
        normalized_fingerprints(&backend, tenant_id, &resources).await,
        snapshot,
        "exactly one row set per resource"
    );
}

/// The live step ends at the Bundle deadline even when a batch is stuck inside
/// the server: the attempt in flight is cut, the committed Bundle is answered
/// (not left to time out and be resubmitted), and the repair finishes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_live_step_does_not_outlive_the_bundle_deadline() {
    const NAME: &str = "the_live_step_does_not_outlive_the_bundle_deadline";
    let app = "fp-def-live-deadline";
    let Some(backend) = deferred_backend("def_live_deadline", app, Some(THRESHOLD), |c| {
        c.bundle_transaction_deadline = Some(Duration::from_secs(3));
    })
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-live-deadline");
    let tenant_id = tenant.tenant_id().as_str();
    // A POST-only Bundle sends no `update` of its own: the one the failpoint
    // blocks for ten seconds is the first batch's, clearing its markers.
    record_missing_fingerprints(&backend).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["update"], "blockConnection": true, "blockTimeMS": 10_000_i64 },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let started = Instant::now();
    let result = backend
        .process_transaction(
            &tenant,
            patient_bundle(4, "Deadline3", "MRN-DL3"),
            FhirVersion::default(),
        )
        .await;
    let elapsed = started.elapsed();
    if lacks_transactions(&result) {
        fail_point.off().await;
        return;
    }
    let result = result.expect("a committed Bundle is not failed by its post-commit step");
    let resources = located(&result);
    assert!(
        elapsed < Duration::from_secs(7),
        "answered at the 3 s deadline, not after the 10 s block: {elapsed:?}"
    );
    fail_point.wait_until_entered(1).await;

    assert_eq!(ledger_count(&backend).await, 1);
    assert_eq!(marker_count(&backend, tenant_id).await, 6);
    assert_no_rows(&backend, tenant_id, &resources).await;

    // Past the block, and the abort of the cut batch's transaction behind it.
    tokio::time::sleep(Duration::from_secs(11).saturating_sub(started.elapsed())).await;
    fail_point.off().await;
    let repair = same_db_backend(&backend, "def-live-deadline-repair")
        .await
        .unwrap();
    let report = repair_until_clear(&repair).await;
    assert_eq!(report.cleared, 1);
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_bundle_searchable(&backend, &tenant, &resources, "Deadline3", "MRN-DL3", 4).await;
}

// ---------------------------------------------------------------------------
// Aborts and replays
// ---------------------------------------------------------------------------

/// A Bundle that fails (the last entry's `If-Match` is wrong) rolls back with
/// everything it wrote: no resources, no ledger document, no marker, and the
/// rows of a resource it would have updated are untouched.
#[tokio::test]
async fn an_aborted_deferred_bundle_leaves_no_ledger_and_no_rows() {
    const NAME: &str = "an_aborted_deferred_bundle_leaves_no_ledger_and_no_rows";
    let Some(backend) = deferred_backend("def_abort", "def-abort", Some(THRESHOLD), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-abort");
    let tenant_id = tenant.tenant_id().as_str();
    let seed = backend
        .create(
            &tenant,
            "Patient",
            patient_json("Seed", "MRN-SEED"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let rows_before = row_fingerprints(&backend, tenant_id, "Patient", seed.id()).await;
    assert!(!rows_before.is_empty());

    let urn = "urn:uuid:def-abort-patient";
    let mut entries: Vec<BundleEntry> = (0..5)
        .map(|n| {
            post(
                "Observation",
                observation_json(urn, n),
                &format!("urn:uuid:def-abort-{n}"),
            )
        })
        .collect();
    entries.push(BundleEntry {
        method: BundleMethod::Put,
        url: format!("Patient/{}", seed.id()),
        resource: Some(json!({
            "resourceType": "Patient",
            "id": seed.id(),
            "name": [{"family": "Changed"}],
        })),
        if_match: Some("W/\"99\"".to_string()),
        ..Default::default()
    });
    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await;
    if lacks_transactions(&result) {
        return;
    }
    assert!(result.is_err(), "the wrong If-Match fails the Bundle");

    assert_eq!(ledger_count(&backend).await, 0);
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_eq!(
        backend.count(&tenant, Some("Observation")).await.unwrap(),
        0
    );
    assert_eq!(
        row_fingerprints(&backend, tenant_id, "Patient", seed.id()).await,
        rows_before
    );
}

/// The server aborts the first attempt with a transient error; the replay
/// commits. Each attempt builds its own ledger, so the aborted one leaves
/// nothing: one committed Bundle transaction, no leftover ledger document, and
/// the rows an inline twin has.
#[tokio::test]
async fn a_replayed_deferred_bundle_commits_once_with_the_rows_of_an_inline_twin() {
    const NAME: &str = "a_replayed_deferred_bundle_commits_once_with_the_rows_of_an_inline_twin";
    let app = "fp-def-replay";
    let Some(backend) = deferred_backend("def_replay", app, Some(THRESHOLD), |_| {}).await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some(twin) = deferred_backend("def_replay_twin", "def-replay-twin", Some(0), |_| {}).await
    else {
        return;
    };
    let tenant = create_tenant("def-replay");
    let tenant_id = tenant.tenant_id().as_str();

    let Some(twin_result) = process_transaction_or_skip(
        &twin,
        &tenant,
        patient_bundle(4, "Replay", "MRN-REPLAY"),
        NAME,
    )
    .await
    else {
        return;
    };
    let twin_counts = counts_for(&twin, tenant_id, &located(&twin_result)).await;

    let oplog = Oplog::begin(&backend).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["insert"],
            "errorCode": 112,
            "errorLabels": ["TransientTransactionError"],
        },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };
    let result = backend
        .process_transaction(
            &tenant,
            patient_bundle(4, "Replay", "MRN-REPLAY"),
            FhirVersion::default(),
        )
        .await;
    let entered = fail_point.off_and_count().await;
    if lacks_transactions(&result) {
        return;
    }
    let result = result.expect("a transient abort is replayed");
    assert_eq!(entered, 1);
    let resources = located(&result);

    assert_eq!(ledger_count(&backend).await, 0);
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_eq!(
        counts_for(&backend, tenant_id, &resources).await,
        twin_counts
    );
    assert_bundle_searchable(&backend, &tenant, &resources, "Replay", "MRN-REPLAY", 4).await;

    if let Some(oplog) = oplog {
        let db = backend.config().database_name.clone();
        let txns = oplog.committed(&backend).await;
        let bundles = txns
            .iter()
            .filter(|ops| count_ops(ops, &db, "resources", "i") > 0)
            .count();
        assert_eq!(bundles, 1, "the aborted attempt committed nothing");
    }
}

// ---------------------------------------------------------------------------
// ifNoneExist
// ---------------------------------------------------------------------------

/// The ledger is written inside the transaction, so a failure to write it rolls
/// the whole Bundle back, and as a server-side error: nothing the client sent
/// is at fault. A `collMod` validator no document satisfies makes every ledger
/// insert fail with a non-transient `DocumentValidationFailure` (121), whatever
/// the insert count.
#[tokio::test]
async fn a_deferred_bundle_whose_ledger_insert_fails_rolls_back_as_a_server_error() {
    const NAME: &str = "a_deferred_bundle_whose_ledger_insert_fails_rolls_back_as_a_server_error";
    let Some(backend) = deferred_backend(
        "def_ledger_fail",
        "def-ledger-fail",
        Some(THRESHOLD),
        |_| {},
    )
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("def-ledger-fail");
    let tenant_id = tenant.tenant_id().as_str();
    raw_db(&backend)
        .await
        .run_command(doc! {
            "collMod": "search_index_pending",
            "validator": { "$jsonSchema": { "required": ["never_present"] } },
            "validationLevel": "strict",
            "validationAction": "error",
        })
        .await
        .expect("collMod");

    let result = backend
        .process_transaction(
            &tenant,
            patient_bundle(4, "LedgerFail", "MRN-LEDGER"),
            FhirVersion::default(),
        )
        .await;
    if lacks_transactions(&result) {
        return;
    }
    match result {
        Err(TransactionError::RolledBack { reason }) => {
            assert!(
                reason.contains("ledger"),
                "the reason names the ledger: {reason}"
            );
        }
        other => panic!("expected TransactionError::RolledBack, got {other:?}"),
    }
    assert_eq!(ledger_count(&backend).await, 0);
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_eq!(
        stored_refs(&backend, tenant_id).await.len(),
        0,
        "no resource of the Bundle was applied"
    );
    assert_eq!(
        count_docs(
            &backend,
            "resource_history",
            doc! { "tenant_id": tenant_id }
        )
        .await,
        0
    );
}

/// Patient A, four Observations, then two Patient POSTs whose `ifNoneExist`
/// finds A by identifier and by family.
fn if_none_exist_bundle() -> Vec<BundleEntry> {
    let urn = "urn:uuid:def-ine-patient-a";
    let mut entries = vec![post("Patient", patient_json("Dup", "X"), urn)];
    for n in 0..4 {
        entries.push(post(
            "Observation",
            observation_json(urn, n),
            &format!("urn:uuid:def-ine-obs-{n}"),
        ));
    }
    entries.push(post_if_none_exist(
        "Patient",
        patient_json("Dup", "X"),
        "urn:uuid:def-ine-patient-b",
        &format!("identifier={MRN_SYSTEM}|X"),
    ));
    entries.push(post_if_none_exist(
        "Patient",
        patient_json("Dup", "X"),
        "urn:uuid:def-ine-patient-c",
        "family=Dup",
    ));
    entries
}

/// The statuses of an `if_none_exist_bundle` result and whether the two
/// conditional entries resolved to the first entry's resource.
async fn run_if_none_exist(
    backend: &MongoBackend,
    tenant: &TenantContext,
    name: &str,
) -> Option<(Vec<u16>, Vec<(String, String)>)> {
    let result = process_transaction_or_skip(backend, tenant, if_none_exist_bundle(), name).await?;
    let statuses: Vec<u16> = result.entries.iter().map(|entry| entry.status).collect();
    let resources = located(&result);
    assert_eq!(
        resources[5], resources[0],
        "the identifier criterion matches Patient A, created earlier in the same Bundle"
    );
    assert_eq!(
        resources[6], resources[0],
        "the family criterion matches Patient A, created earlier in the same Bundle"
    );
    assert_eq!(backend.count(tenant, Some("Patient")).await.unwrap(), 1);
    Some((statuses, resources))
}

/// A deferred Bundle still matches `ifNoneExist` against its own earlier
/// creates, like the inline path: the target type (Patient) keeps its rows in
/// the transaction. A deadline-cut run shows which resources were deferred:
/// the ledger names the Observations only.
#[tokio::test]
async fn deferred_if_none_exist_matches_an_earlier_same_bundle_create_like_inline() {
    const NAME: &str = "deferred_if_none_exist_matches_an_earlier_same_bundle_create_like_inline";
    let Some(inline) = deferred_backend("def_ine_inline", "def-ine-inline", Some(0), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some(deferred) = deferred_backend(
        "def_ine_deferred",
        "def-ine-deferred",
        Some(THRESHOLD),
        |_| {},
    )
    .await
    else {
        return;
    };
    let tenant = create_tenant("def-ine");

    let Some((inline_statuses, _)) = run_if_none_exist(&inline, &tenant, NAME).await else {
        return;
    };
    let Some((deferred_statuses, _)) = run_if_none_exist(&deferred, &tenant, NAME).await else {
        return;
    };
    assert_eq!(inline_statuses, vec![201, 201, 201, 201, 201, 200, 200]);
    assert_eq!(deferred_statuses, inline_statuses);
    assert_eq!(ledger_count(&deferred).await, 0);

    // The same Bundle with the post-commit step cut by the deadline.
    let Some(cut) = deferred_backend("def_ine_cut", "def-ine-cut", Some(THRESHOLD), |c| {
        c.bundle_transaction_deadline = Some(Duration::from_millis(1));
    })
    .await
    else {
        return;
    };
    let tenant_id = tenant.tenant_id().as_str();
    let Some((cut_statuses, resources)) = run_if_none_exist(&cut, &tenant, NAME).await else {
        return;
    };
    assert_eq!(cut_statuses, inline_statuses);
    assert_eq!(ledger_count(&cut).await, 1);
    let ledger: Document = raw_db(&cut)
        .await
        .collection::<Document>("search_index_pending")
        .find_one(doc! {})
        .await
        .unwrap()
        .expect("the ledger document");
    let refs: Vec<&str> = ledger
        .get_array("refs")
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(
        refs.len(),
        4,
        "only the Observations are deferred: {refs:?}"
    );
    assert!(
        refs.iter().all(|r| r.starts_with("Observation/")),
        "{refs:?}"
    );
    assert_eq!(marker_count(&cut, tenant_id).await, 4);
    assert!(
        row_counts(&cut, tenant_id, "Patient", &resources[0].1)
            .await
            .0
            > 0,
        "Patient A's rows were written inside the transaction"
    );
    for (resource_type, id) in &resources[1..5] {
        assert_eq!(row_counts(&cut, tenant_id, resource_type, id).await, (0, 0));
    }
}

// ---------------------------------------------------------------------------
// Updates and deletes
// ---------------------------------------------------------------------------

/// Seeds two Patients, then runs a Bundle that updates the first, deletes the
/// second and creates four Observations. With `cut` the post-commit step is
/// stopped by the deadline and the repair finishes it; either way no stale row
/// is left.
async fn updates_and_deletes_leave_no_stale_rows(test: &str, cut: bool) {
    let Some(backend) = deferred_backend(test, test, Some(THRESHOLD), |c| {
        if cut {
            c.bundle_transaction_deadline = Some(Duration::from_millis(1));
        }
    })
    .await
    else {
        eprintln!("Skipping {test} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    // `:missing=true` takes the `missing_params` route only once its index is
    // built; before that it reads rows, and the cut variant would prove nothing.
    assert!(
        matches!(
            backend.wait_for_missing_index_build().await,
            Some(BuildOutcome::UpToDate | BuildOutcome::Built { .. })
        ),
        "idx_resources_missing_params is built"
    );
    let tenant = create_tenant("def-upd-del");
    let tenant_id = tenant.tenant_id().as_str();
    let p1 = backend
        .create(
            &tenant,
            "Patient",
            patient_json("Old", "MRN-P1"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let p2 = backend
        .create(
            &tenant,
            "Patient",
            patient_json("Gone", "Y"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        patients_by_family(&backend, &tenant, "Old").await,
        vec![p1.id().to_string()]
    );
    assert_eq!(
        patients_by_mrn(&backend, &tenant, "Y").await,
        vec![p2.id().to_string()]
    );
    // Neither Patient has a gender, so both are listed as missing it.
    for patient in [&p1, &p2] {
        let missing = missing_params_of(&backend, tenant_id, "Patient", patient.id()).await;
        assert!(
            missing.is_some_and(|names| names.iter().any(|name| name == "gender")),
            "Patient/{} lacks a gender",
            patient.id()
        );
    }
    let mut both = vec![p1.id().to_string(), p2.id().to_string()];
    both.sort();
    assert_eq!(
        ids_missing(
            &backend,
            &tenant,
            "Patient",
            "gender",
            SearchParamType::Token,
            true
        )
        .await,
        both
    );

    // The update gives the first Patient a gender.
    let mut entries = vec![
        BundleEntry {
            method: BundleMethod::Put,
            url: format!("Patient/{}", p1.id()),
            resource: Some(json!({
                "resourceType": "Patient",
                "id": p1.id(),
                "identifier": [{"system": MRN_SYSTEM, "value": "MRN-P1"}],
                "name": [{"family": "New"}],
                "gender": "female",
            })),
            ..Default::default()
        },
        BundleEntry {
            method: BundleMethod::Delete,
            url: format!("Patient/{}", p2.id()),
            ..Default::default()
        },
    ];
    for n in 0..4 {
        entries.push(post(
            "Observation",
            observation_json(&format!("Patient/{}", p1.id()), n),
            &format!("urn:uuid:def-upd-del-{n}"),
        ));
    }
    let Some(_) = process_transaction_or_skip(&backend, &tenant, entries, test).await else {
        return;
    };

    if cut {
        // The post-commit step did not run: the old rows are still there.
        assert_eq!(ledger_count(&backend).await, 1);
        assert_eq!(marker_count(&backend, tenant_id).await, 6);
        assert_eq!(
            patients_by_family(&backend, &tenant, "Old").await,
            vec![p1.id().to_string()],
            "stale until the repair"
        );
        assert!(row_counts(&backend, tenant_id, "Patient", p2.id()).await.0 > 0);
        // The Bundle's transaction rewrote `missing_params` of both Patients
        // (the update through `resource_update`, the delete with `[]`). The
        // first Patient's rows are still the old ones, with no gender row, so
        // a rows route would list it as missing a gender; only its
        // `missing_params` excludes it, and the second Patient is deleted.
        let p1_missing = missing_params_of(&backend, tenant_id, "Patient", p1.id()).await;
        assert!(
            p1_missing
                .as_ref()
                .is_some_and(|names| !names.iter().any(|name| name == "gender")),
            "the update's transaction removed gender from missing_params: {p1_missing:?}"
        );
        assert_eq!(
            missing_params_of(&backend, tenant_id, "Patient", p2.id()).await,
            Some(Vec::<String>::new()),
            "the delete's transaction emptied missing_params"
        );
        assert!(
            ids_missing(
                &backend,
                &tenant,
                "Patient",
                "gender",
                SearchParamType::Token,
                true
            )
            .await
            .is_empty(),
            "gender:missing=true is already right before the repair"
        );
        let repair = same_db_backend(&backend, "def-upd-del-repair")
            .await
            .unwrap();
        let report = repair_until_clear(&repair).await;
        assert_eq!((report.cleared, report.failed), (1, 0));
    }

    assert!(
        patients_by_family(&backend, &tenant, "Old")
            .await
            .is_empty()
    );
    assert_eq!(
        patients_by_family(&backend, &tenant, "New").await,
        vec![p1.id().to_string()]
    );
    assert!(patients_by_mrn(&backend, &tenant, "Y").await.is_empty());
    assert_eq!(
        row_counts(&backend, tenant_id, "Patient", p2.id()).await,
        (0, 0)
    );
    assert_eq!(observations_of(&backend, &tenant, p1.id()).await.len(), 4);
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    assert_eq!(ledger_count(&backend).await, 0);

    // `missing_params` follows the update and the delete in both variants.
    let p1_missing = missing_params_of(&backend, tenant_id, "Patient", p1.id()).await;
    assert!(
        p1_missing
            .as_ref()
            .is_some_and(|names| !names.iter().any(|name| name == "gender")),
        "the updated Patient has a gender: {p1_missing:?}"
    );
    assert_eq!(
        missing_params_of(&backend, tenant_id, "Patient", p2.id()).await,
        Some(Vec::<String>::new()),
        "the deleted Patient lists nothing as missing"
    );
    assert!(
        ids_missing(
            &backend,
            &tenant,
            "Patient",
            "gender",
            SearchParamType::Token,
            true
        )
        .await
        .is_empty()
    );
    assert_eq!(
        ids_missing(
            &backend,
            &tenant,
            "Patient",
            "gender",
            SearchParamType::Token,
            false
        )
        .await,
        vec![p1.id().to_string()]
    );
}

#[tokio::test]
async fn deferred_updates_and_deletes_leave_no_stale_rows() {
    updates_and_deletes_leave_no_stale_rows("def_upd_del", false).await;
}

#[tokio::test]
async fn deferred_updates_and_deletes_cut_before_indexing_leave_no_stale_rows_after_the_repair() {
    updates_and_deletes_leave_no_stale_rows("def_upd_del_cut", true).await;
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

/// Another client updates the Patient while the Bundle's first batch is blocked
/// in the server. The batch loses its write conflict on the Patient document,
/// starts again from the current state, and the Patient ends with the rows of
/// its current content, one set of them: the same counts as a Patient created
/// inline with that content.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_concurrent_update_during_post_commit_indexing_leaves_only_current_rows() {
    const NAME: &str = "a_concurrent_update_during_post_commit_indexing_leaves_only_current_rows";
    let app = "fp-def-concurrent";
    let Some(backend_a) = deferred_backend("def_concurrent", app, Some(THRESHOLD), |_| {}).await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let Some(backend_b) = same_db_backend(&backend_a, "def-concurrent-b").await else {
        return;
    };
    let backend_a = Arc::new(backend_a);
    let tenant = create_tenant("def-concurrent");
    let tenant_id = tenant.tenant_id().as_str();
    record_missing_fingerprints(&backend_a).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! { "failCommands": ["update"], "blockConnection": true, "blockTimeMS": 2500_i64 },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let bundle = {
        let backend = Arc::clone(&backend_a);
        tokio::spawn(async move {
            backend
                .process_transaction(
                    &create_tenant("def-concurrent"),
                    patient_bundle(4, "Older", "MRN-CONC"),
                    FhirVersion::default(),
                )
                .await
        })
    };

    // The Patient exists once the Bundle has committed.
    let waited = Instant::now();
    let patient_id = loop {
        let patients: Vec<_> = stored_refs(&backend_b, tenant_id)
            .await
            .into_iter()
            .filter(|(resource_type, _)| resource_type == "Patient")
            .collect();
        if let Some((_, id)) = patients.first() {
            break id.clone();
        }
        assert!(
            waited.elapsed() < Duration::from_secs(20),
            "the Bundle never committed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    let mut updated = false;
    for _ in 0..10 {
        let current = backend_b
            .read(&tenant, "Patient", &patient_id)
            .await
            .unwrap()
            .expect("the Patient");
        let mut content = current.content().clone();
        content["name"] = json!([{"family": "Newer"}]);
        match backend_b.update(&tenant, &current, content).await {
            Ok(_) => {
                updated = true;
                break;
            }
            Err(StorageError::Concurrency(ConcurrencyError::VersionConflict { .. })) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(other) => panic!("update failed: {other}"),
        }
    }
    assert!(updated, "the concurrent update never went through");
    assert!(
        !bundle.is_finished(),
        "the update landed while the Bundle's batch was still blocked"
    );

    let result = bundle
        .await
        .expect("Bundle task")
        .expect("the Bundle commits");
    let entered = fail_point.off_and_count().await;
    assert_eq!(entered, 1, "the first batch was blocked once");
    let resources = located(&result);
    assert_eq!(resources[0].1, patient_id);

    assert_eq!(
        patients_by_family(&backend_a, &tenant, "Newer").await,
        vec![patient_id.clone()]
    );
    assert!(
        patients_by_family(&backend_a, &tenant, "Older")
            .await
            .is_empty()
    );
    assert_eq!(
        patients_by_mrn(&backend_a, &tenant, "MRN-CONC").await,
        vec![patient_id.clone()]
    );
    assert_eq!(marker_count(&backend_a, tenant_id).await, 0);
    assert_eq!(ledger_count(&backend_a).await, 0);

    let twin = backend_b
        .create(
            &tenant,
            "Patient",
            patient_json("Newer", "MRN-CONC"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        row_counts(&backend_a, tenant_id, "Patient", &patient_id).await,
        row_counts(&backend_a, tenant_id, "Patient", twin.id()).await,
        "one row set, as for a Patient created inline with the same content"
    );
    let twin_missing = missing_params_of(&backend_a, tenant_id, "Patient", twin.id()).await;
    assert!(twin_missing.is_some(), "the twin has missing_params");
    assert_eq!(
        missing_params_of(&backend_a, tenant_id, "Patient", &patient_id).await,
        twin_missing,
        "missing_params of the current content, as for a Patient created inline with it"
    );
}

// ---------------------------------------------------------------------------
// Other configurations
// ---------------------------------------------------------------------------

/// With search offloaded to a secondary the deferral does not exist: no ledger,
/// no marker, and the backend reports no threshold.
#[tokio::test]
async fn an_offloaded_backend_never_defers() {
    const NAME: &str = "an_offloaded_backend_never_defers";
    let Some(backend) = deferred_backend("def_offloaded", "def-offloaded", Some(THRESHOLD), |c| {
        c.search_offloaded = true;
    })
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    assert_eq!(backend.deferred_search_index_min_entries(), None);
    let tenant = create_tenant("def-offloaded");
    let tenant_id = tenant.tenant_id().as_str();
    let Some(_) = process_transaction_or_skip(
        &backend,
        &tenant,
        patient_bundle(4, "Offloaded", "MRN-OFF"),
        NAME,
    )
    .await
    else {
        return;
    };
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
    assert_eq!(ledger_count(&backend).await, 0);
    assert_eq!(marker_count(&backend, tenant_id).await, 0);
    let report = backend
        .repair_deferred_search_index(Duration::ZERO, 10)
        .await
        .unwrap();
    assert_eq!((report.examined, report.remaining), (0, 0));
}

/// `purge_tenant_data` removes the tenant's ledger documents with its data.
#[tokio::test]
async fn purging_a_tenant_removes_its_ledger_documents() {
    const NAME: &str = "purging_a_tenant_removes_its_ledger_documents";
    let Some(backend) = deferred_backend("def_purge", "def-purge", Some(THRESHOLD), |c| {
        c.bundle_transaction_deadline = Some(Duration::from_millis(1));
    })
    .await
    else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    for name in ["def-purge-a", "def-purge-b"] {
        let tenant = create_tenant(name);
        let Some(_) = process_transaction_or_skip(
            &backend,
            &tenant,
            patient_bundle(4, "Purge", "MRN-PURGE"),
            NAME,
        )
        .await
        else {
            return;
        };
    }
    assert_eq!(ledger_count(&backend).await, 2);

    backend.purge_tenant_data("def-purge-a").await.unwrap();
    assert_eq!(ledger_count(&backend).await, 1);
    assert_eq!(
        count_docs(
            &backend,
            "search_index_pending",
            doc! { "tenant_id": "def-purge-a" }
        )
        .await,
        0
    );
    assert_eq!(
        count_docs(
            &backend,
            "search_index_pending",
            doc! { "tenant_id": "def-purge-b" }
        )
        .await,
        1
    );
}

/// Schema init creates the ledger collection with the index the repair reads
/// through, so a Bundle's transaction never has to create the collection.
#[tokio::test]
async fn schema_init_creates_the_ledger_collection_and_its_due_index() {
    let Some(backend) = create_backend("def_schema").await else {
        eprintln!("Skipping schema_init_creates_the_ledger_collection_and_its_due_index");
        return;
    };
    let db = raw_db(&backend).await;
    let collections = db.list_collection_names().await.unwrap();
    assert!(
        collections
            .iter()
            .any(|name| name == "search_index_pending"),
        "{collections:?}"
    );
    let indexes = db
        .collection::<Document>("search_index_pending")
        .list_index_names()
        .await
        .unwrap();
    assert!(
        indexes
            .iter()
            .any(|name| name == "idx_search_index_pending_due"),
        "{indexes:?}"
    );
}

/// A database at schema version 12, which has no ledger collection, gets the
/// collection and its index from `migrate()` and moves to version 13.
#[tokio::test]
async fn migrating_from_v12_creates_the_ledger_collection_and_index() {
    const NAME: &str = "migrating_from_v12_creates_the_ledger_collection_and_index";
    let Some(backend) = create_backend("def_migrate").await else {
        eprintln!("Skipping {NAME} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db = raw_db(&backend).await;
    db.collection::<Document>("search_index_pending")
        .drop()
        .await
        .unwrap();
    db.collection::<Document>("schema_version")
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": { "version": 12_i32 } },
        )
        .await
        .unwrap();
    assert!(
        !db.list_collection_names()
            .await
            .unwrap()
            .iter()
            .any(|name| name == "search_index_pending"),
        "the v12 database has no ledger collection"
    );

    backend.migrate().await.unwrap();

    assert!(
        db.list_collection_names()
            .await
            .unwrap()
            .iter()
            .any(|name| name == "search_index_pending"),
        "migrate creates the ledger collection"
    );
    let indexes = db
        .collection::<Document>("search_index_pending")
        .list_index_names()
        .await
        .unwrap();
    assert!(
        indexes
            .iter()
            .any(|name| name == "idx_search_index_pending_due"),
        "{indexes:?}"
    );
    let version = db
        .collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await
        .unwrap()
        .expect("schema version document");
    assert_eq!(
        version.get_i32("version").unwrap(),
        helios_persistence::backends::mongodb::SCHEMA_VERSION
    );
    const {
        assert!(
            helios_persistence::backends::mongodb::SCHEMA_VERSION >= 13,
            "the deferred-index ledger arrived in schema version 13"
        );
    }
}

// ---------------------------------------------------------------------------
// The ceiling
// ---------------------------------------------------------------------------

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_usize(name: &str, default: usize) -> usize {
    env_or(name, &default.to_string())
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be an unsigned integer"))
}

/// A boolean knob: anything but `0` and `false` is on.
fn env_flag(name: &str, default: bool) -> bool {
    !matches!(
        env_or(name, if default { "true" } else { "false" }).as_str(),
        "0" | "false"
    )
}

/// Where a `TooLargeForCache` reason says the Bundle ended: the digits after
/// `Entry ` (`Entry 19766 processing failed: ...`, `Entry 21100 rolled back
/// for eviction ...`), `commit` for a reason that starts with `Commit`, and `na`
/// for anything else. The prefixes are written by `entry_attempt_error`,
/// `ledger_attempt_error` and `lone_eviction_failure` in `storage.rs`.
fn abort_entry(reason: &str) -> String {
    if reason.starts_with("Commit") {
        return "commit".to_string();
    }
    if let Some(at) = reason.find("Entry ") {
        let digits: String = reason[at + "Entry ".len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            return digits;
        }
    }
    "na".to_string()
}

#[test]
fn abort_entry_is_read_from_the_reason() {
    assert_eq!(abort_entry("Entry 19766 processing failed: x"), "19766");
    assert_eq!(
        abort_entry("Entry 21100 rolled back for eviction while the Bundle ran alone; x"),
        "21100"
    );
    assert_eq!(
        abort_entry("Commit rolled back for eviction while the Bundle ran alone; x"),
        "commit"
    );
    assert_eq!(abort_entry("x"), "na");
}

/// One way of running the ceiling Bundle. The first two are the pair #2030
/// measured; the other two isolate where the native deferred transaction
/// differs from an offloaded one (#1975).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CeilingVariant {
    /// Never defers: the rows are written inside the transaction.
    Inline,
    /// The default threshold: resources, history, markers and a ledger in the
    /// transaction; the rows after the commit.
    Deferred,
    /// `Deferred` with `missing_metadata=false`: no `missing_params` or
    /// fingerprint on any resource and no `idx_resources_missing_params`. It
    /// stands in for a deferred transaction that writes no missing metadata.
    DeferredNoMetadata,
    /// Search offloaded: the transaction holds the resource and its history.
    Offloaded,
}

impl CeilingVariant {
    const NAMES: &'static str = "inline, deferred, deferred_no_metadata, offloaded";

    fn parse(name: &str) -> Self {
        match name {
            "inline" => Self::Inline,
            "deferred" => Self::Deferred,
            "deferred_no_metadata" => Self::DeferredNoMetadata,
            "offloaded" => Self::Offloaded,
            other => panic!(
                "HFS_TEST_DEFERRED_CEILING_VARIANTS names an unknown variant {other:?}; \
                 use a comma-separated list of: {}",
                Self::NAMES
            ),
        }
    }

    fn parse_list(list: &str) -> Vec<Self> {
        let variants: Vec<Self> = list
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(Self::parse)
            .collect();
        assert!(
            !variants.is_empty(),
            "HFS_TEST_DEFERRED_CEILING_VARIANTS is empty; use a comma-separated list of: {}",
            Self::NAMES
        );
        variants
    }

    fn label(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Deferred => "deferred",
            Self::DeferredNoMetadata => "deferred_no_metadata",
            Self::Offloaded => "offloaded",
        }
    }

    /// True for the variants whose rows are written after the commit.
    fn defers(self) -> bool {
        matches!(self, Self::Deferred | Self::DeferredNoMetadata)
    }

    fn configure(self, config: &mut MongoBackendConfig) {
        match self {
            Self::Inline => config.transaction_bundle_deferred_index_entries = Some(0),
            Self::Deferred => config.transaction_bundle_deferred_index_entries = None,
            Self::DeferredNoMetadata => {
                config.transaction_bundle_deferred_index_entries = None;
                config.missing_metadata = false;
            }
            Self::Offloaded => {
                config.transaction_bundle_deferred_index_entries = Some(0);
                config.search_offloaded = true;
            }
        }
    }
}

/// A backend built directly (`build_backend` caps the pool at 4) on its own
/// database of `url`'s server, with the default gate and no deadline unless
/// `configure` sets one, so the only differences between variants are the ones
/// they set.
fn ceiling_config(
    url: &str,
    label: &str,
    configure: impl FnOnce(&mut MongoBackendConfig),
) -> MongoBackendConfig {
    let mut config = MongoBackendConfig {
        connection_string: url.to_string(),
        database_name: build_test_database_name(&format!("ceiling_{label}")),
        max_connections: 32,
        data_dir: Some(repo_data_dir()),
        index_build: IndexBuildMode::Inline,
        bundle_transaction_deadline: None,
        ..Default::default()
    };
    configure(&mut config);
    config
}

fn bson_f64(value: Option<&Bson>) -> f64 {
    match value {
        Some(Bson::Int32(v)) => f64::from(*v),
        Some(Bson::Int64(v)) => *v as f64,
        Some(Bson::Double(v)) => *v,
        _ => f64::NAN,
    }
}

/// Keys per index, from `validate` (`keysPerIndex`). `full` is cheap at the
/// probe's few hundred documents. Empty, with a note on stderr, if the server
/// does not report it.
async fn index_keys(db: &mongodb::Database, collection: &str) -> Vec<(String, i64)> {
    let reply = match db
        .run_command(doc! { "validate": collection, "full": true })
        .await
    {
        Ok(reply) => reply,
        Err(err) => {
            eprintln!("DEFERRED_CEILING_PROBE validate {collection} failed: {err}");
            return Vec::new();
        }
    };
    match reply.get_document("keysPerIndex") {
        Ok(keys) => keys
            .iter()
            .map(|(name, count)| (name.clone(), bson_f64(Some(count)) as i64))
            .collect(),
        Err(_) => {
            eprintln!("DEFERRED_CEILING_PROBE validate {collection} reported no keysPerIndex");
            Vec::new()
        }
    }
}

/// `(count, avg bytes)` of the tenant's documents in `collection`, and for
/// `resources` also how many carry a `missing_params` array and its average
/// length.
async fn probe_stats(db: &mongodb::Database, collection: &str, tenant: &str) -> [f64; 4] {
    let mut cursor = db
        .collection::<Document>(collection)
        .aggregate([
            doc! { "$match": { "tenant_id": tenant } },
            doc! { "$group": {
                "_id": Bson::Null,
                "count": { "$sum": 1 },
                "with_missing_params": {
                    "$sum": { "$cond": [{ "$isArray": "$missing_params" }, 1, 0] }
                },
                "avg_missing_params": {
                    "$avg": { "$size": { "$ifNull": ["$missing_params", Bson::Array(Vec::new())] } }
                },
                "avg_bytes": { "$avg": { "$bsonSize": "$$ROOT" } },
            } },
        ])
        .await
        .expect("aggregate");
    let group = cursor
        .try_next()
        .await
        .expect("aggregate cursor")
        .unwrap_or_default();
    [
        bson_f64(group.get("count")),
        bson_f64(group.get("with_missing_params")),
        bson_f64(group.get("avg_missing_params")),
        bson_f64(group.get("avg_bytes")),
    ]
}

/// The write-set probe: one copy of the fixture (267 entries) through each of
/// four modes, then what the transaction left behind, counted as index keys per
/// resource. Cheap and deterministic, unlike the ceiling, so it shows whether a
/// change moved the write set before a long run shows what that did to the
/// ceiling. `deferred_txn` cuts the post-commit step with a 1 ms deadline, so
/// its documents are the ones the transaction wrote; `deferred_no_metadata` is
/// the same with `missing_metadata=false`, for reference.
async fn run_ceiling_probe(url: &str, fixture: &Value) {
    let tenant = create_tenant("ceiling");
    for mode in [
        "inline",
        "deferred_txn",
        "deferred_no_metadata",
        "offloaded",
    ] {
        let config = ceiling_config(url, &format!("probe_{mode}"), |config| match mode {
            "inline" => config.transaction_bundle_deferred_index_entries = Some(0),
            "deferred_txn" | "deferred_no_metadata" => {
                config.transaction_bundle_deferred_index_entries = Some(1);
                config.bundle_transaction_deadline = Some(Duration::from_millis(1));
                config.missing_metadata = mode != "deferred_no_metadata";
            }
            _ => {
                config.transaction_bundle_deferred_index_entries = Some(0);
                config.search_offloaded = true;
            }
        });
        let backend = MongoBackend::new(config).unwrap();
        backend.initialize().await.unwrap();
        let entries = super::transaction_bundle_load::bundle_entries(fixture, 1);
        let count = entries.len();
        backend
            .process_transaction(&tenant, entries, FhirVersion::default())
            .await
            .unwrap_or_else(|err| panic!("the probe Bundle ({mode}) commits: {err}"));

        let db = raw_db(&backend).await;
        let [
            resources,
            with_missing_params,
            avg_missing_params,
            avg_resource_bytes,
        ] = probe_stats(&db, "resources", "ceiling").await;
        let [history, _, _, avg_history_bytes] =
            probe_stats(&db, "resource_history", "ceiling").await;
        let resource_keys = index_keys(&db, "resources").await;
        let history_keys = index_keys(&db, "resource_history").await;
        let keys_of = |keys: &[(String, i64)], name: &str| {
            keys.iter()
                .find(|(index, _)| index == name)
                .map_or(0.0, |(_, count)| *count as f64)
        };
        let per_resource = |keys: f64| {
            if resources > 0.0 {
                keys / resources
            } else {
                f64::NAN
            }
        };
        let missing_keys = per_resource(keys_of(&resource_keys, "idx_resources_missing_params"));
        let fp_keys = per_resource(keys_of(&resource_keys, "idx_resources_missing_params_fp"));
        let all_resource_keys =
            per_resource(resource_keys.iter().map(|(_, count)| *count as f64).sum());
        let all_history_keys = if history > 0.0 {
            history_keys
                .iter()
                .map(|(_, count)| *count as f64)
                .sum::<f64>()
                / history
        } else {
            f64::NAN
        };
        let rows = count_docs(&backend, "search_index", doc! { "tenant_id": "ceiling" }).await;
        let markers = marker_count(&backend, "ceiling").await;
        let ledger = ledger_count(&backend).await;

        println!(
            "DEFERRED_CEILING_PROBE mode={mode} entries={count} resources={resources:.0} \
             with_missing_params={with_missing_params:.0} \
             avg_missing_params={avg_missing_params:.1} \
             missing_keys_per_resource={missing_keys:.2} fp_keys_per_resource={fp_keys:.2} \
             resource_keys_per_resource={all_resource_keys:.2} \
             avg_resource_bytes={avg_resource_bytes:.0} avg_history_bytes={avg_history_bytes:.0} \
             rows_per_entry={:.1} markers={markers} ledger={ledger}",
            rows as f64 / count as f64
        );
        let list = |keys: &[(String, i64)]| {
            keys.iter()
                .map(|(name, count)| format!("{name}:{count}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        println!(
            "DEFERRED_CEILING_PROBE_INDEXES mode={mode} history_keys_per_resource={all_history_keys:.2} \
             resources=[{}] resource_history=[{}]",
            list(&resource_keys),
            list(&history_keys)
        );
        if matches!(mode, "deferred_txn" | "deferred_no_metadata") {
            assert!(
                markers > 0,
                "{mode}: the post-commit step was meant to be cut by the 1 ms deadline, \
                 so the documents would be as the transaction wrote them"
            );
        }

        db.drop().await.expect("drop the probe database");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// #1975: the size of Bundle a lone transaction can hold, per way of running it,
/// against a small WiredTiger cache. Ignored by default.
///
/// One synthetic Bundle of `COPIES` copies of the US Core fixture
/// `hfs/tests/inferno/uscore_bundle_patient_85.json` (267 entries each, so 15
/// copies are 4,005 entries) goes through one backend per variant, each on its
/// own database (dropped afterwards, with a 5 s pause) of a mongo:7.0 replica
/// set started with `--wiredTigerCacheSizeGB` (0.5 by default). Variants:
/// - `inline`: never defers; expected `TooLargeForCache` (as in #1895).
/// - `deferred`: the default threshold, the gate's room of 4,000 entries;
///   expected to commit, its rows written afterwards.
/// - `deferred_no_metadata`: `deferred` with `missing_metadata=false`, so no
///   resource carries `missing_params` or its fingerprint.
/// - `offloaded`: search offloaded, so the transaction holds only the
///   resources and their history (the `mongodb-elasticsearch` leg's write set).
///
/// Before the variants, a write-set probe (`PROBE`) sends one copy through
/// `inline`, `deferred_txn` (the deferral with its post-commit step cut, so the
/// documents are as the transaction wrote them), `deferred_no_metadata` and
/// `offloaded`, and counts index keys per resource with `validate`. It takes
/// under two minutes.
///
/// Output, one line each:
/// - `DEFERRED_CEILING_PROBE mode=.. entries=.. resources=.. with_missing_params=..
///   avg_missing_params=.. missing_keys_per_resource=.. fp_keys_per_resource=..
///   resource_keys_per_resource=.. avg_resource_bytes=.. avg_history_bytes=..
///   rows_per_entry=.. markers=.. ledger=..`, and a
///   `DEFERRED_CEILING_PROBE_INDEXES` line with the raw keys per index.
/// - `DEFERRED_CEILING variant=.. wt_cache_gb=.. entries=.. outcome=.. cause=..
///   abort_entry=.. secs=.. entries_per_s=.. resources=.. rows=.. ledger=..
///   markers=.. wt_rollbacks=..`. `outcome` is `ok`, `too_large`, `transient`
///   or `other:..`; `abort_entry` is the entry the transaction ended at (or
///   `commit`, or `na` for a committed Bundle); `entries_per_s` is the entries
///   written, up to the abort, per second.
///
/// Run with:
/// `cargo test -p helios-persistence --features mongodb --test mongodb_tests
/// deferred_indexing_raises_the_lone_bundle_ceiling -- --ignored --nocapture`
///
/// Speed depends on the load of the host. Measured on a shared 12-core host
/// with a debug build: a committing 4,005-entry Bundle ran at 48-54 entries per
/// second (74.1 s, 79.9 s and 83.7 s), and the variants that aborted ran at
/// 51-401 entries per second up to the abort, so an aborting variant took about
/// 90-245 s at 2 GB. A 28,035-entry Bundle (105 copies) that committed at 2 GB,
/// rows included, took 351.1 s and 347.5 s in two runs (79.9 and 80.7 entries
/// per second); the first whole test took 553 s. Raise `TXN_LIFETIME_SECS` (7200 for a run that may last longer than the
/// default 900 s) or the variant ends at the server's transaction lifetime
/// instead of at the cache. Run those in the background.
///
/// `abort_entry` is accurate to about one create chunk
/// (`BUNDLE_CREATE_CHUNK_ENTRIES`, 100 entries): a failed chunk is charged to the
/// entry the server names, or else to its first entry, so it can read up to 99
/// entries low (`bundle_creates.rs`, Attribution). A refusal while the ledger is
/// written reads as `abort_entry=<entries>`, because `ledger_attempt_error`
/// reuses `entry_attempt_error(entries, entries, ..)` in `storage.rs`.
///
/// Env knobs (defaults in parentheses):
/// - `HFS_TEST_DEFERRED_CEILING_WT_CACHE_GB` ("0.5"): `--wiredTigerCacheSizeGB`.
/// - `HFS_TEST_DEFERRED_CEILING_COPIES` (15): fixture copies in the Bundle.
/// - `HFS_TEST_DEFERRED_CEILING_VARIANTS` ("inline,deferred"): the variants to
///   run, in order.
/// - `HFS_TEST_DEFERRED_CEILING_PROBE` ("true"): run the write-set probe first.
/// - `HFS_TEST_DEFERRED_CEILING_TXN_LIFETIME_SECS` (900): mongod's
///   `transactionLifetimeLimitSeconds`.
/// - `HFS_TEST_DEFERRED_CEILING_MONGODB_URL`: an external replica set; no
///   container is started and the cache size is whatever that server has.
/// - `HFS_TEST_DEFERRED_CEILING_EXPECT_INLINE_ABORT` ("true"): assert the inline
///   run ends `TooLargeForCache`; with "false" any inline outcome is accepted
///   (for a cache large enough that the inline Bundle fits).
/// - `HFS_TEST_DEFERRED_CEILING_EXPECT_COMMIT` ("true"): assert that every
///   other variant commits (a deferred one with its rows written, its ledger
///   cleared and no marker left; the offloaded one with no rows); with "false"
///   any outcome is accepted, for finding where a variant aborts. Whatever the
///   outcome, an aborted Bundle must leave no resource, ledger or marker.
///
/// Every assertion is checked after all variants have run, so a long run still
/// prints every line.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "#1975 ceiling harness; run with --ignored"]
async fn deferred_indexing_raises_the_lone_bundle_ceiling() {
    let cache_gb = env_or("HFS_TEST_DEFERRED_CEILING_WT_CACHE_GB", "0.5");
    let copies = env_usize("HFS_TEST_DEFERRED_CEILING_COPIES", 15);
    let expect_inline_abort = env_flag("HFS_TEST_DEFERRED_CEILING_EXPECT_INLINE_ABORT", true);
    let expect_commit = env_flag("HFS_TEST_DEFERRED_CEILING_EXPECT_COMMIT", true);
    let probe = env_flag("HFS_TEST_DEFERRED_CEILING_PROBE", true);
    let txn_lifetime = env_usize("HFS_TEST_DEFERRED_CEILING_TXN_LIFETIME_SECS", 900);
    let variants = CeilingVariant::parse_list(&env_or(
        "HFS_TEST_DEFERRED_CEILING_VARIANTS",
        "inline,deferred",
    ));

    // Keep the container alive (and removed on drop) for the whole test.
    let _container;
    let (url, cache_label) = match std::env::var("HFS_TEST_DEFERRED_CEILING_MONGODB_URL") {
        Ok(url) => (url, "external".to_string()),
        Err(_) => {
            // Mirrors transaction_bundle_load.rs: mongo:7.0, single-member rs0,
            // a transaction lifetime of `txn_lifetime` seconds.
            let lifetime = format!("transactionLifetimeLimitSeconds={txn_lifetime}");
            let started = super::container_cleanup::with_cleanup_label(
                Mongo::default()
                    .with_tag("7.0")
                    .with_cmd([
                        "mongod",
                        "--bind_ip_all",
                        "--wiredTigerCacheSizeGB",
                        cache_gb.as_str(),
                        "--replSet",
                        "rs0",
                        "--oplogSize",
                        "1024",
                        "--setParameter",
                        lifetime.as_str(),
                    ])
                    .with_ulimit("nofile", 64000, Some(64000))
                    .with_startup_timeout(Duration::from_secs(120)),
            )
            .start()
            .await;
            let container = match started {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("skip: could not start a Mongo container: {err}");
                    return;
                }
            };
            shared_mongo::initiate_replica_set(&container).await;
            shared_mongo::wait_for_writable_primary(&container).await;
            let host = container.get_host().await.expect("container host");
            let port = container
                .get_host_port_ipv4(27017)
                .await
                .expect("container port");
            _container = container;
            (
                format!("mongodb://{host}:{port}/?directConnection=true"),
                cache_gb.clone(),
            )
        }
    };

    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../hfs/tests/inferno/uscore_bundle_patient_85.json");
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_path).expect("read fixture"))
            .expect("parse fixture");

    if probe {
        run_ceiling_probe(&url, &fixture).await;
    }

    let tenant = create_tenant("ceiling");
    let mut failures: Vec<String> = Vec::new();
    for variant in variants {
        let label = variant.label();
        let backend = MongoBackend::new(ceiling_config(&url, label, |config| {
            variant.configure(config)
        }))
        .unwrap();
        backend.initialize().await.unwrap();
        assert_eq!(
            backend.deferred_search_index_min_entries(),
            variant.defers().then_some(4000),
            "{label}: the resolved deferral threshold"
        );

        let entries = super::transaction_bundle_load::bundle_entries(&fixture, copies);
        let entry_count = entries.len();
        assert!(
            !variant.defers() || entry_count >= 4000,
            "{copies} copies are {entry_count} entries, below the 4,000 of the default threshold"
        );

        let before = super::transaction_bundle_load::eviction_rollbacks(&url).await;
        let started = Instant::now();
        let result = backend
            .process_transaction(&tenant, entries, FhirVersion::default())
            .await;
        let secs = started.elapsed().as_secs_f64();

        // Words only, so the line stays `key=value` separated by spaces.
        let words = |text: String| text.split_whitespace().collect::<Vec<_>>().join("_");
        let (outcome, cause, abort) = match &result {
            Ok(_) => ("ok".to_string(), "na".to_string(), "na".to_string()),
            Err(TransactionError::TooLargeForCache { cause, reason, .. }) => (
                "too_large".to_string(),
                words(cause.to_string()),
                abort_entry(reason),
            ),
            Err(TransactionError::Transient { .. }) => {
                ("transient".to_string(), "na".to_string(), "na".to_string())
            }
            Err(other) => (
                words(format!("other:{other}")),
                "na".to_string(),
                "na".to_string(),
            ),
        };
        // The entries written up to the end, for the rate: all of them for a
        // committed Bundle, the abort entry for one that aborted at an entry,
        // and none (`na`) for a commit or an outcome with no abort entry.
        let written = abort
            .parse::<usize>()
            .ok()
            .or((outcome == "ok").then_some(entry_count));
        let entries_per_s = written.map_or_else(
            || "na".to_string(),
            |written| format!("{:.1}", written as f64 / secs),
        );
        let tenant_filter = doc! { "tenant_id": "ceiling" };
        let resources = count_docs(&backend, "resources", tenant_filter.clone()).await;
        let rows = count_docs(&backend, "search_index", tenant_filter).await;
        let ledger = ledger_count(&backend).await;
        let markers = marker_count(&backend, "ceiling").await;
        let rollbacks = super::transaction_bundle_load::eviction_rollbacks(&url).await - before;

        println!(
            "DEFERRED_CEILING variant={label} wt_cache_gb={cache_label} entries={entry_count} \
             outcome={outcome} cause={cause} abort_entry={abort} secs={secs:.1} \
             entries_per_s={entries_per_s} resources={resources} rows={rows} ledger={ledger} \
             markers={markers} wt_rollbacks={rollbacks}"
        );

        if outcome != "ok" && (resources != 0 || ledger != 0 || markers != 0) {
            failures.push(format!(
                "{label}: an aborted Bundle left resources={resources} ledger={ledger} \
                 markers={markers}"
            ));
        }
        if variant == CeilingVariant::Inline {
            if expect_inline_abort && outcome != "too_large" {
                failures.push(format!(
                    "inline: the Bundle should not fit this cache: {outcome}"
                ));
            }
        } else if expect_commit {
            if outcome != "ok" {
                failures.push(format!("{label}: the Bundle should commit: {outcome}"));
            } else if variant.defers() {
                if rows == 0 {
                    failures.push(format!("{label}: the rows were not written"));
                }
                if ledger != 0 || markers != 0 {
                    failures.push(format!(
                        "{label}: left ledger={ledger} markers={markers} after the step"
                    ));
                }
            } else if rows != 0 {
                failures.push(format!("{label}: an offloaded Bundle wrote {rows} rows"));
            }
        }

        raw_db(&backend)
            .await
            .drop()
            .await
            .expect("drop the variant's database");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
