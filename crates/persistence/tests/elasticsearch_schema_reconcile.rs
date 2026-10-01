//! Mapping reconciliation of existing Elasticsearch indices, and `search_count`
//! on a missing index, against a real cluster (#1335).
//!
//! HFS used to send a mapping only when it created an index, so a mapping
//! change (`ignore_malformed` on the date fields, #1314) never reached an index
//! that already existed. These tests build such an index, start a backend on
//! it, and check what the cluster then holds and accepts.
//!
//! The container is the same 7.17.29 image `elasticsearch_tests` uses. Set
//! `HFS_TEST_ES_URL` (e.g. `http://localhost:9200`) to run against a cluster
//! that is already up instead — that is how the suite was run against 8.15.0,
//! the version the Inferno and smoke workflows deploy.
//!
//! Run with:
//! `cargo test -p helios-persistence --features elasticsearch --test elasticsearch_schema_reconcile`

#![cfg(feature = "elasticsearch")]

use std::path::PathBuf;
use std::sync::Arc;

use elasticsearch::http::transport::Transport;
use elasticsearch::indices::{IndicesGetMappingParts, IndicesPutMappingParts};
use elasticsearch::{Elasticsearch, IndexParts, SearchParts};
use helios_fhir::FhirVersion;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, SCHEMA_VERSION, SCHEMA_VERSION_META_KEY,
    WriteRefreshPolicy,
};
use helios_persistence::core::{Backend, ResourceStorage, SearchProvider};
use helios_persistence::search::{SearchParameterLoader, TenantSearchRegistries};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchQuery, SearchValue, StoredResource,
};
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::sync::OnceCell;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

/// See `ES_IMAGE_TAG` in `elasticsearch_tests.rs` for why this tag.
const ES_IMAGE_TAG: &str = "7.17.29";
const ES_STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

const DATE_FORMAT: &str = "strict_date_optional_time||epoch_millis||yyyy||yyyy-MM||yyyy-MM-dd";

struct SharedEs {
    url: String,
    /// Kept alive for the duration of the test binary.
    _container: Option<testcontainers::ContainerAsync<ElasticSearch>>,
}

static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

async fn es_url() -> &'static str {
    &SHARED_ES
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("HFS_TEST_ES_URL") {
                return SharedEs {
                    url,
                    _container: None,
                };
            }
            let container = container_cleanup::with_cleanup_label(
                ElasticSearch::default()
                    .with_tag(ES_IMAGE_TAG)
                    .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                    .with_startup_timeout(ES_STARTUP_TIMEOUT),
            )
            .start()
            .await
            .expect("Failed to start Elasticsearch container");
            let port = container.get_host_port_ipv4(9200).await.expect("port");
            let host = container.get_host().await.expect("host");
            SharedEs {
                url: format!("http://{host}:{port}"),
                _container: Some(container),
            }
        })
        .await
        .url
}

/// A registry loaded from the spec files: with only the embedded parameters
/// `family` indexes nothing and every search below would pass vacuously.
fn search_registry() -> Arc<TenantSearchRegistries> {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let loader = SearchParameterLoader::new(FhirVersion::default());
    let registries = Arc::new(TenantSearchRegistries::base_only());
    {
        let mut registry = registries.base().write();
        for param in loader.load_embedded().expect("embedded params") {
            let _ = registry.register(param);
        }
        for param in loader
            .load_from_spec_file(&data_dir)
            .expect("spec search parameters")
        {
            let _ = registry.register(param);
        }
    }
    registries
}

fn new_prefix() -> String {
    format!("hfs_{}", uuid::Uuid::new_v4().simple())
}

/// A backend on `prefix`, *not yet initialized*.
async fn backend_on(prefix: &str) -> ElasticsearchBackend {
    let config = ElasticsearchConfig {
        nodes: vec![es_url().await.to_string()],
        index_prefix: prefix.to_string(),
        number_of_replicas: 0,
        refresh_interval: "1ms".to_string(),
        write_refresh: WriteRefreshPolicy::WaitFor,
        ..Default::default()
    };
    ElasticsearchBackend::with_shared_registry(config, search_registry())
        .expect("Failed to create ElasticsearchBackend")
}

async fn started_backend_on(prefix: &str) -> ElasticsearchBackend {
    let backend = backend_on(prefix).await;
    backend.initialize().await.expect("initialize");
    backend
}

async fn raw_client() -> Elasticsearch {
    Elasticsearch::new(Transport::single_node(es_url().await).expect("transport"))
}

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("reconcile"), TenantPermissions::full_access())
}

async fn create_patient(backend: &ElasticsearchBackend, id: &str, family: &str) -> StoredResource {
    backend
        .create(
            &tenant(),
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{ "family": family }],
                "birthDate": "1980-05-17"
            }),
            FhirVersion::default(),
        )
        .await
        .expect("create")
}

fn family_query(family: &str) -> SearchQuery {
    SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "family".to_string(),
        param_type: SearchParamType::String,
        values: vec![SearchValue::parse(family)],
        ..Default::default()
    })
}

async fn found_ids(backend: &ElasticsearchBackend, query: &SearchQuery) -> Vec<String> {
    let mut ids: Vec<String> = backend
        .search(&tenant(), query)
        .await
        .expect("search")
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect();
    ids.sort();
    ids
}

/// The mapping of `index`.
async fn mapping_of(index: &str) -> Value {
    let body: Value = raw_client()
        .await
        .indices()
        .get_mapping(IndicesGetMappingParts::Index(&[index]))
        .send()
        .await
        .expect("get mapping")
        .json()
        .await
        .expect("mapping json");
    body[index]["mappings"].clone()
}

/// `(search_params.date.value, search_params.composite.date)` of a mapping.
fn date_fields(mapping: &Value) -> (&Value, &Value) {
    let sp = &mapping["properties"]["search_params"]["properties"];
    (
        &sp["date"]["properties"]["value"],
        &sp["composite"]["properties"]["date"],
    )
}

/// Puts `index` back to what HFS created before #1314/#1335: date fields with
/// no `ignore_malformed`, and no schema version. An updatable mapping parameter
/// that is left out of a `PUT _mapping` is reset to its default, and `_meta` is
/// replaced wholesale, so this yields exactly the old mapping — everything else
/// in it is untouched. `meta` is what the old index's `_meta` should hold.
async fn downgrade_to_pre_1335_mapping(index: &str, meta: Value) {
    let date = json!({ "type": "date", "format": DATE_FORMAT });
    let response = raw_client()
        .await
        .indices()
        .put_mapping(IndicesPutMappingParts::Index(&[index]))
        .body(json!({
            "_meta": meta.clone(),
            "properties": { "search_params": { "properties": {
                "date": { "type": "nested", "properties": { "value": date } },
                "composite": { "type": "nested", "properties": { "date": date } }
            } } }
        }))
        .send()
        .await
        .expect("put old mapping");
    assert!(response.status_code().is_success(), "downgrade failed");

    let mapping = mapping_of(index).await;
    let (date, composite_date) = date_fields(&mapping);
    assert_eq!(date["type"], "date");
    assert!(date.get("ignore_malformed").is_none(), "{date}");
    assert!(composite_date.get("ignore_malformed").is_none());
    assert_eq!(mapping.get("_meta").cloned().unwrap_or(json!({})), meta);
}

fn assert_reconciled(mapping: &Value) {
    let (date, composite_date) = date_fields(mapping);
    assert_eq!(date["ignore_malformed"], json!(true), "{date}");
    assert_eq!(composite_date["ignore_malformed"], json!(true));
    // Schema version 2 (#1391): the end of the range a date covers.
    let end = &mapping["properties"]["search_params"]["properties"]["date"]["properties"]["end"];
    assert_eq!(end["type"], "date", "{end}");
    assert_eq!(end["ignore_malformed"], json!(true), "{end}");
    assert_eq!(
        mapping["_meta"][SCHEMA_VERSION_META_KEY],
        json!(SCHEMA_VERSION)
    );
}

/// Indexes, straight into `index`, a copy of the seeded patient's document
/// renamed to `id` and given a date Elasticsearch cannot parse — in both date
/// fields. HFS's own writer drops such a value before it is sent (#1314), so
/// going around it is the only way to put the mapping itself to the test.
/// Returns the HTTP status and body.
async fn index_document_with_malformed_dates(index: &str, id: &str) -> (u16, String) {
    let client = raw_client().await;
    let hits: Value = client
        .search(SearchParts::Index(&[index]))
        .body(json!({ "query": { "term": { "resource_id": "seed" } } }))
        .send()
        .await
        .expect("find seed")
        .json()
        .await
        .expect("seed json");
    let mut doc = hits["hits"]["hits"][0]["_source"].clone();
    assert!(doc.is_object(), "seed document not found: {hits}");

    doc["resource_id"] = json!(id);
    doc["content"]["id"] = json!(id);
    doc["search_params"]["date"] = json!([
        { "name": "birthdate", "value": "not-a-date", "precision": "day" }
    ]);
    doc["search_params"]["composite"] = json!([
        { "name": "some-composite", "group_id": 0, "date": ["31/12/1999"] }
    ]);

    let response = client
        .index(IndexParts::IndexId(index, &format!("Patient_{id}")))
        .refresh(elasticsearch::params::Refresh::WaitFor)
        .body(doc)
        .send()
        .await
        .expect("index request");
    let status = response.status_code().as_u16();
    (status, response.text().await.unwrap_or_default())
}

/// The issue's case. An index with the old mapping rejects a whole document
/// for one malformed date; after a backend starts on it the mapping has
/// `ignore_malformed`, and the same document indexes and is found.
#[tokio::test]
async fn startup_reconciles_an_old_index_and_a_malformed_date_then_indexes() {
    let prefix = new_prefix();
    let first = started_backend_on(&prefix).await;
    create_patient(&first, "seed", "Reconcileseed").await;
    let index = first.index_name("reconcile", "Patient");
    // Positive control: the registry indexes `family`.
    assert_eq!(
        found_ids(&first, &family_query("Reconcileseed")).await,
        ["seed"]
    );

    downgrade_to_pre_1335_mapping(&index, json!({ "owner": "ops" })).await;
    let (status, body) = index_document_with_malformed_dates(&index, "bad-before").await;
    assert_eq!(status, 400, "the old mapping must reject it: {body}");
    assert!(body.contains("search_params.date.value"), "{body}");

    // A new server process starts on the existing index.
    let second = started_backend_on(&prefix).await;
    let mapping = mapping_of(&index).await;
    assert_reconciled(&mapping);
    assert_eq!(
        mapping["_meta"]["owner"], "ops",
        "the rest of `_meta` must survive"
    );

    let (status, body) = index_document_with_malformed_dates(&index, "bad-after").await;
    assert!(
        (200..300).contains(&status),
        "the reconciled mapping must accept it ({status}): {body}"
    );
    assert_eq!(
        found_ids(&second, &family_query("Reconcileseed")).await,
        ["bad-after", "seed"],
        "the document is searchable by its other parameters"
    );
}

/// #1391: an index laid out at schema version 1 has no `search_params.date.end`.
/// A backend starting on it adds the field, and a `Period` indexed afterwards
/// is found by the range comparisons that read it.
#[tokio::test]
async fn startup_adds_the_date_range_end_to_a_version_1_index() {
    // The current mapping without `end`, marked version 1: what a version-1
    // build created. A field cannot be removed from a live index, so it is
    // copied into a fresh one.
    let template = started_backend_on(&new_prefix()).await;
    create_patient(&template, "seed", "Templateseed").await;
    let mut mapping = mapping_of(&template.index_name("reconcile", "Patient")).await;
    let date = &mut mapping["properties"]["search_params"]["properties"]["date"]["properties"];
    assert!(date.as_object_mut().unwrap().remove("end").is_some());
    mapping["_meta"] = json!({ SCHEMA_VERSION_META_KEY: 1 });

    let prefix = new_prefix();
    let index = backend_on(&prefix)
        .await
        .index_name("reconcile", "Encounter");
    let response = raw_client()
        .await
        .indices()
        .create(elasticsearch::indices::IndicesCreateParts::Index(&index))
        .body(json!({
            "settings": {
                "number_of_replicas": 0,
                "analysis": { "normalizer": { "lowercase_normalizer": {
                    "type": "custom", "filter": ["lowercase"]
                } } }
            },
            "mappings": mapping
        }))
        .send()
        .await
        .expect("create a version-1 index");
    assert!(response.status_code().is_success(), "create failed");

    let backend = started_backend_on(&prefix).await;
    assert_reconciled(&mapping_of(&index).await);

    backend
        .create(
            &tenant(),
            "Encounter",
            json!({
                "resourceType": "Encounter",
                "id": "e1",
                "status": "finished",
                "class": { "code": "AMB" },
                "period": { "start": "2020-03-01", "end": "2020-09-30" }
            }),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    let date_query = |value: &str| {
        SearchQuery::new("Encounter").with_parameter(SearchParameter {
            name: "date".to_string(),
            param_type: SearchParamType::Date,
            values: vec![SearchValue::parse(value)],
            ..Default::default()
        })
    };
    assert_eq!(found_ids(&backend, &date_query("2020")).await, ["e1"]);
    assert!(found_ids(&backend, &date_query("2020-03")).await.is_empty());
}

/// #1391: `hfs` does not run `initialize` at startup, so an index at schema
/// version 1 keeps its mapping until its first write. A descending date sort
/// reads `search_params.date.end`, which such an index lacks: the search must
/// still work (it used to be a 400, "No mapping found ... in order to sort").
#[tokio::test]
async fn a_descending_date_sort_works_on_an_index_not_yet_reconciled() {
    let template = started_backend_on(&new_prefix()).await;
    create_patient(&template, "seed", "Templateseed").await;
    let mut mapping = mapping_of(&template.index_name("reconcile", "Patient")).await;
    let date = &mut mapping["properties"]["search_params"]["properties"]["date"]["properties"];
    assert!(date.as_object_mut().unwrap().remove("end").is_some());
    mapping["_meta"] = json!({ SCHEMA_VERSION_META_KEY: 1 });

    let prefix = new_prefix();
    // Not initialized, as `hfs` builds it.
    let backend = backend_on(&prefix).await;
    let index = backend.index_name("reconcile", "Encounter");
    let response = raw_client()
        .await
        .indices()
        .create(elasticsearch::indices::IndicesCreateParts::Index(&index))
        .body(json!({
            "settings": {
                "number_of_replicas": 0,
                "analysis": { "normalizer": { "lowercase_normalizer": {
                    "type": "custom", "filter": ["lowercase"]
                } } }
            },
            "mappings": mapping
        }))
        .send()
        .await
        .expect("create a version-1 index");
    assert!(response.status_code().is_success(), "create failed");
    assert_eq!(mapping_version_meta(&index).await, Some(1));

    for direction in [
        helios_persistence::types::SortDirection::Descending,
        helios_persistence::types::SortDirection::Ascending,
    ] {
        let query =
            SearchQuery::new("Encounter").with_sort(helios_persistence::types::SortDirective {
                parameter: "date".to_string(),
                direction,
                param_type: Some(SearchParamType::Date),
            });
        let result = backend.search(&tenant(), &query).await;
        assert!(
            result.is_ok(),
            "a {direction:?} date sort must not fail on an unreconciled index: {:?}",
            result.err()
        );
    }
    // Searching does not reconcile the index.
    assert_eq!(mapping_version_meta(&index).await, Some(1));
}

/// The `hfs_schema_version` in an index's mapping `_meta`, if any.
async fn mapping_version_meta(index: &str) -> Option<u64> {
    mapping_of(index).await["_meta"][SCHEMA_VERSION_META_KEY].as_u64()
}

/// The per-index mapping version Elasticsearch keeps in the cluster state; it
/// goes up by one for every mapping update that changed something.
async fn mapping_version(index: &str) -> u64 {
    let body: Value = raw_client()
        .await
        .cluster()
        .state(elasticsearch::cluster::ClusterStateParts::MetricIndex(
            &["metadata"],
            &[index],
        ))
        .send()
        .await
        .expect("cluster state")
        .json()
        .await
        .expect("cluster state json");
    body["metadata"]["indices"][index]["mapping_version"]
        .as_u64()
        .unwrap_or_else(|| panic!("no mapping_version for {index}: {body}"))
}

/// A second start changes nothing: the marker is already current. (That no
/// `PUT _mapping` is even sent is asserted in `elasticsearch_search_wiremock`.)
#[tokio::test]
async fn reconcile_is_a_no_op_on_the_second_start() {
    let prefix = new_prefix();
    let first = started_backend_on(&prefix).await;
    create_patient(&first, "seed", "Noopseed").await;
    let index = first.index_name("reconcile", "Patient");
    downgrade_to_pre_1335_mapping(&index, json!({})).await;

    let before = mapping_version(&index).await;
    started_backend_on(&prefix).await;
    let reconciled = mapping_version(&index).await;
    assert!(reconciled > before, "the first start updates the mapping");
    let mapping = mapping_of(&index).await;
    assert_reconciled(&mapping);

    started_backend_on(&prefix).await;
    assert_eq!(mapping_version(&index).await, reconciled);
    assert_eq!(mapping_of(&index).await, mapping);
}

/// A brand-new index is born at the current version and is never touched.
#[tokio::test]
async fn a_new_index_carries_the_current_schema_version() {
    let prefix = new_prefix();
    let backend = started_backend_on(&prefix).await;
    create_patient(&backend, "seed", "Newseed").await;
    let index = backend.index_name("reconcile", "Patient");

    assert_reconciled(&mapping_of(&index).await);
    let version = mapping_version(&index).await;
    started_backend_on(&prefix).await;
    assert_eq!(mapping_version(&index).await, version);
}

/// An index that turns stale *after* startup — an older HFS instance creating
/// it during a rolling upgrade — is reconciled by the first write that goes
/// through `ensure_index`, and the write itself succeeds.
#[tokio::test]
async fn ensure_index_reconciles_an_index_the_startup_pass_did_not_see() {
    let prefix = new_prefix();
    let older = started_backend_on(&prefix).await;
    let newer = started_backend_on(&prefix).await;

    // Created after `newer` started, with the old mapping.
    create_patient(&older, "seed", "Rollingseed").await;
    let index = older.index_name("reconcile", "Patient");
    downgrade_to_pre_1335_mapping(&index, json!({})).await;

    create_patient(&newer, "second", "Rollingseed").await;
    assert_reconciled(&mapping_of(&index).await);
    assert_eq!(
        found_ids(&newer, &family_query("Rollingseed")).await,
        ["second", "seed"]
    );
}

/// Records the `WARN`-and-above events this crate logs. `#[traced_test]` is no
/// use in an integration test: it keeps only the test crate's own events. It is
/// installed as the *global* subscriber, once: a per-thread one races with the
/// other tests in this binary over tracing's per-callsite interest cache, and
/// a test would sometimes not see events its own thread logged. Every test
/// therefore reads it by the name of an index of its own.
#[derive(Clone, Default)]
struct WarningLog(Arc<std::sync::Mutex<Vec<String>>>);

struct FieldText(String);

impl tracing::field::Visit for FieldText {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={value:?} ", field.name()));
    }
}

impl tracing::Subscriber for WarningLog {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if event.metadata().target().starts_with("helios_persistence") {
            let mut text = FieldText(String::new());
            event.record(&mut text);
            self.0.lock().unwrap().push(text.0);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

static WARNINGS: std::sync::OnceLock<WarningLog> = std::sync::OnceLock::new();

impl WarningLog {
    fn install() -> &'static WarningLog {
        WARNINGS.get_or_init(|| {
            let log = WarningLog::default();
            tracing::subscriber::set_global_default(log.clone())
                .expect("no other global subscriber in this test binary");
            log
        })
    }

    /// The warnings about documents indexed before #1391 that name `index`.
    fn reindex_warnings_for(&self, index: &str) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.contains("before #1391") && line.contains(index))
            .cloned()
            .collect()
    }
}

/// #1391: upgrading an index below schema version 2 that holds documents warns,
/// once, that they have no `search_params.date.end` until `$reindex`; an empty
/// index and one already at version 2 do not. Run on the real cluster so the
/// document count the decision rests on is the one Elasticsearch reports.
#[tokio::test]
async fn upgrading_an_old_index_with_documents_warns_that_reindex_is_needed() {
    let log = WarningLog::install();

    // Version 1 with a document: warned, once, naming the index.
    let prefix = new_prefix();
    let first = started_backend_on(&prefix).await;
    create_patient(&first, "seed", "Warnseed").await;
    let index = first.index_name("reconcile", "Patient");
    downgrade_to_pre_1335_mapping(&index, json!({ SCHEMA_VERSION_META_KEY: 1 })).await;
    assert!(
        log.reindex_warnings_for(&index).is_empty(),
        "nothing to warn about yet"
    );
    started_backend_on(&prefix).await;
    let warnings = log.reindex_warnings_for(&index);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    for needle in [
        "search_params.date.end",
        "$reindex",
        "eq, ne, gt, ge, le, eb or ap",
    ] {
        assert!(
            warnings[0].contains(needle),
            "{needle} missing: {}",
            warnings[0]
        );
    }

    // The second start finds it current: no second warning.
    started_backend_on(&prefix).await;
    assert_eq!(log.reindex_warnings_for(&index).len(), 1);

    // An old index with no document left in it: nothing to reindex.
    let empty_prefix = new_prefix();
    let empty = started_backend_on(&empty_prefix).await;
    create_patient(&empty, "gone", "Emptyseed").await;
    let empty_index = empty.index_name("reconcile", "Patient");
    let deleted = raw_client()
        .await
        .delete(elasticsearch::DeleteParts::IndexId(
            &empty_index,
            "Patient_gone",
        ))
        .refresh(elasticsearch::params::Refresh::True)
        .send()
        .await
        .expect("delete");
    assert!(deleted.status_code().is_success(), "delete failed");
    downgrade_to_pre_1335_mapping(&empty_index, json!({ SCHEMA_VERSION_META_KEY: 1 })).await;
    started_backend_on(&empty_prefix).await;
    assert_reconciled(&mapping_of(&empty_index).await);
    assert!(log.reindex_warnings_for(&empty_index).is_empty());

    // Already at the current version, documents and all: not touched, no warning.
    let current_prefix = new_prefix();
    let current = started_backend_on(&current_prefix).await;
    create_patient(&current, "seed", "Currentseed").await;
    let current_index = current.index_name("reconcile", "Patient");
    started_backend_on(&current_prefix).await;
    assert!(log.reindex_warnings_for(&current_index).is_empty());

    // The write path reconciles an index the startup pass did not see, and
    // gives the same warning for it.
    let rolling_prefix = new_prefix();
    let older = started_backend_on(&rolling_prefix).await;
    let newer = started_backend_on(&rolling_prefix).await;
    create_patient(&older, "seed", "Rollingwarn").await;
    let rolling_index = older.index_name("reconcile", "Patient");
    downgrade_to_pre_1335_mapping(&rolling_index, json!({ SCHEMA_VERSION_META_KEY: 1 })).await;
    create_patient(&newer, "second", "Rollingwarn").await;
    assert_eq!(log.reindex_warnings_for(&rolling_index).len(), 1);
}

/// An index a newer build has already taken past this version is left alone:
/// re-applying this build's mapping could reset a parameter the newer one set.
#[tokio::test]
async fn an_index_at_a_newer_schema_version_is_not_downgraded() {
    let prefix = new_prefix();
    let first = started_backend_on(&prefix).await;
    create_patient(&first, "seed", "Newerseed").await;
    let index = first.index_name("reconcile", "Patient");
    downgrade_to_pre_1335_mapping(
        &index,
        json!({ SCHEMA_VERSION_META_KEY: SCHEMA_VERSION + 1 }),
    )
    .await;

    let version = mapping_version(&index).await;
    let second = started_backend_on(&prefix).await;
    create_patient(&second, "second", "Newerseed").await;
    assert_eq!(mapping_version(&index).await, version);
    assert_eq!(
        mapping_of(&index).await["_meta"][SCHEMA_VERSION_META_KEY],
        json!(SCHEMA_VERSION + 1)
    );
}

/// Several HFS instances starting at once on the same stale indices all start,
/// and every index ends up reconciled.
#[tokio::test]
async fn concurrent_startups_reconcile_the_same_indices_safely() {
    let prefix = new_prefix();
    let first = started_backend_on(&prefix).await;
    let mut indices = Vec::new();
    for n in 0..6 {
        let tenant_id = format!("concurrent-{n}");
        let ctx = TenantContext::new(TenantId::new(&tenant_id), TenantPermissions::full_access());
        first
            .create(
                &ctx,
                "Patient",
                json!({ "resourceType": "Patient", "id": "seed", "name": [{ "family": "C" }] }),
                FhirVersion::default(),
            )
            .await
            .expect("create");
        let index = first.index_name(&tenant_id, "Patient");
        downgrade_to_pre_1335_mapping(&index, json!({})).await;
        indices.push(index);
    }

    let mut backends = Vec::new();
    for _ in 0..5 {
        backends.push(backend_on(&prefix).await);
    }
    let results = futures::future::join_all(backends.iter().map(|b| b.initialize())).await;
    for result in results {
        result.expect("every concurrent startup must succeed");
    }
    for index in &indices {
        assert_reconciled(&mapping_of(index).await);
    }
}

/// A count on a type that was never written — no index yet — is 0, and a count
/// on one that was is not (positive control).
#[tokio::test]
async fn count_on_a_missing_index_is_zero() {
    let backend = started_backend_on(&new_prefix()).await;
    create_patient(&backend, "seed", "Countseed").await;

    assert_eq!(
        backend
            .search_count(&tenant(), &SearchQuery::new("Patient"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        backend
            .search_count(&tenant(), &family_query("Countseed"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        backend
            .search_count(&tenant(), &SearchQuery::new("Observation"))
            .await
            .unwrap(),
        0
    );
    let empty = backend
        .search(&tenant(), &SearchQuery::new("Observation"))
        .await
        .unwrap();
    assert!(empty.resources.items.is_empty());
    assert_eq!(empty.total, Some(0));
}
