//! Chaos by corruption of the Elasticsearch secondary's contained-document
//! bookkeeping (#2291), against a real cluster.
//!
//! Each test corrupts the state of the cluster by hand (documents PUT, DELETEd
//! or rewritten with the plain client, behind the backend's back) and then
//! checks how the backend behaves afterwards: no panic, no current child lost,
//! nothing of ANOTHER resource or tenant destroyed, and whether the next normal
//! write heals the damage. Every test prints a `RESULT[n]` line that says what
//! was observed.
//!
//! Set `HFS_TEST_ES_URL` to use a running cluster; otherwise a 7.17.29
//! container is started. Run with:
//! `cargo test -p helios-persistence --features R4,elasticsearch,sqlite --test elasticsearch_contained_chaos_corrupt_real -- --nocapture --include-ignored`

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

use std::path::PathBuf;
use std::sync::Arc;

use elasticsearch::Elasticsearch;
use elasticsearch::http::transport::Transport;
use helios_fhir::FhirVersion;
use helios_persistence::StoredResource;
use helios_persistence::backends::elasticsearch::{
    ElasticsearchBackend, ElasticsearchConfig, WriteRefreshPolicy,
};
use helios_persistence::core::{Backend, PurgableStorage, ResourceStorage, SearchProvider};
use helios_persistence::search::{SearchParameterLoader, TenantSearchRegistries};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ContainedMode, ContainedReturn, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::sync::OnceCell;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

const ES_IMAGE_TAG: &str = "7.17.29";
const ES_STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

struct SharedEs {
    addr: String,
    _container: Option<testcontainers::ContainerAsync<ElasticSearch>>,
}

static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

async fn es_addr() -> &'static str {
    &SHARED_ES
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("HFS_TEST_ES_URL") {
                let addr = url
                    .trim_start_matches("http://")
                    .trim_end_matches('/')
                    .to_string();
                return SharedEs {
                    addr,
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
                addr: format!("{host}:{port}"),
                _container: Some(container),
            }
        })
        .await
        .addr
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

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
    let base = std::env::var("HFS_TEST_ES_INDEX_PREFIX").unwrap_or_else(|_| "hfs".to_string());
    format!("{base}_{}", uuid::Uuid::new_v4().simple())
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

async fn raw_client() -> Elasticsearch {
    Elasticsearch::new(Transport::single_node(&format!("http://{}", es_addr().await)).unwrap())
}

async fn make_backend(prefix: &str) -> ElasticsearchBackend {
    let config = ElasticsearchConfig {
        nodes: vec![format!("http://{}", es_addr().await)],
        index_prefix: prefix.to_string(),
        number_of_replicas: 0,
        refresh_interval: "1ms".to_string(),
        write_refresh: WriteRefreshPolicy::WaitFor,
        ..Default::default()
    };
    let backend = ElasticsearchBackend::with_shared_registry(config, search_registry())
        .expect("create ES backend");
    backend.initialize().await.expect("initialize ES backend");
    backend
}

/// A backend on a fresh prefix, and the tenant the test works as.
struct World {
    prefix: String,
    es: ElasticsearchBackend,
    t: TenantContext,
}

async fn world(tenant_id: &str) -> World {
    let prefix = new_prefix();
    let es = make_backend(&prefix).await;
    World {
        prefix,
        es,
        t: tenant(tenant_id),
    }
}

impl World {
    /// A second tenant on the same prefix (its own indices).
    fn other(&self, tenant_id: &str) -> TenantContext {
        tenant(tenant_id)
    }

    async fn finish(self) {
        cleanup(&self.prefix).await;
    }

    /// Creates `rtype/id` with one contained Organization `org` named `name`.
    async fn make(&self, rtype: &str, id: &str, name: Option<&str>) -> StoredResource {
        self.make_as(&self.t.clone(), rtype, id, name).await
    }

    async fn make_as(
        &self,
        t: &TenantContext,
        rtype: &str,
        id: &str,
        name: Option<&str>,
    ) -> StoredResource {
        self.es
            .create(t, rtype, holder(rtype, id, name), FhirVersion::default())
            .await
            .expect("create")
    }

    async fn update(
        &self,
        stored: &StoredResource,
        name: Option<&str>,
    ) -> Result<StoredResource, helios_persistence::error::StorageError> {
        self.es
            .update(
                &self.t,
                stored,
                holder(stored.resource_type(), stored.id(), name),
            )
            .await
    }

    /// Local ids of the contained Organizations named `name`, as a search sees them.
    async fn shown(&self, name: &str) -> Vec<String> {
        self.shown_as(&self.t.clone(), name).await
    }

    async fn shown_as(&self, t: &TenantContext, name: &str) -> Vec<String> {
        contained_hits(&self.es, t, name, ContainedReturn::Contained)
            .await
            .into_iter()
            .map(|(_, id)| id)
            .collect()
    }

    /// `Type/id` of the containers of Organizations named `name`.
    async fn containers(&self, name: &str) -> Vec<String> {
        containers_of(&self.es, &self.t, name).await
    }
}

fn holder(rtype: &str, id: &str, name: Option<&str>) -> Value {
    let mut r = json!({ "resourceType": rtype, "id": id });
    if let Some(name) = name {
        r["contained"] = json!([{ "resourceType": "Organization", "id": "org", "name": name }]);
    }
    r
}

/// `_id` of a contained Organization of `ctype/cid` under `local`.
fn child_id(ctype: &str, cid: &str, local: &str) -> String {
    format!("Organization_{ctype}_{cid}#{local}")
}

async fn cleanup(prefix: &str) {
    let client = raw_client().await;
    let pattern = format!("{prefix}_*");
    if let Ok(response) = client
        .cat()
        .indices(elasticsearch::cat::CatIndicesParts::Index(&[&pattern]))
        .h(&["index"])
        .expand_wildcards(&[elasticsearch::params::ExpandWildcards::All])
        .format("json")
        .send()
        .await
        && let Ok(rows) = response.json::<Value>().await
    {
        for name in rows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["index"].as_str())
        {
            let _ = client
                .indices()
                .delete(elasticsearch::indices::IndicesDeleteParts::Index(&[name]))
                .send()
                .await;
        }
    }
    let _ = client
        .indices()
        .delete_template(elasticsearch::indices::IndicesDeleteTemplateParts::Name(
            &format!("{prefix}_template"),
        ))
        .send()
        .await;
}

async fn contained_hits(
    es: &ElasticsearchBackend,
    tenant: &TenantContext,
    name: &str,
    ret: ContainedReturn,
) -> Vec<(String, String)> {
    es.refresh_index(tenant.tenant_id().as_str(), "Organization")
        .await
        .ok();
    let mut q = SearchQuery::new("Organization");
    q.contained = ContainedMode::On;
    q.contained_return = ret;
    q.parameters.push(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(name)],
        chain: vec![],
        components: vec![],
    });
    let mut items: Vec<(String, String)> = es
        .search(tenant, &q)
        .await
        .expect("contained search must not fail")
        .resources
        .items
        .iter()
        .map(|r| (r.resource_type().to_string(), r.id().to_string()))
        .collect();
    items.sort();
    items
}

async fn containers_of(
    es: &ElasticsearchBackend,
    tenant: &TenantContext,
    name: &str,
) -> Vec<String> {
    es.refresh_index(tenant.tenant_id().as_str(), "Organization")
        .await
        .ok();
    let mut q = SearchQuery::new("Organization");
    q.contained = ContainedMode::On;
    q.contained_return = ContainedReturn::Container;
    q.parameters.push(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq(name)],
        chain: vec![],
        components: vec![],
    });
    let mut urls: Vec<String> = es
        .search(tenant, &q)
        .await
        .expect("contained search must not fail")
        .resources
        .items
        .iter()
        .map(|r| r.url())
        .collect();
    urls.sort();
    urls
}

// ---------------------------------------------------------------------------
// Raw access
// ---------------------------------------------------------------------------

async fn refresh_all(prefix: &str) {
    let client = raw_client().await;
    let pattern = format!("{prefix}_*");
    let _ = client
        .indices()
        .refresh(elasticsearch::indices::IndicesRefreshParts::Index(&[
            &pattern,
        ]))
        .send()
        .await;
}

/// How many contained documents exist under `prefix` (any container, any tenant).
async fn contained_doc_count(prefix: &str) -> u64 {
    refresh_all(prefix).await;
    let client = raw_client().await;
    let pattern = format!("{prefix}_*");
    let body = client
        .count(elasticsearch::CountParts::Index(&[&pattern]))
        .body(json!({ "query": { "term": { "is_contained": true } } }))
        .send()
        .await
        .expect("count")
        .json::<Value>()
        .await
        .expect("count body");
    body["count"].as_u64().expect("count")
}

/// The container document of `rtype/id`: `(index, _id, _source)`.
async fn find_container(prefix: &str, rtype: &str, id: &str) -> (String, String, Value) {
    refresh_all(prefix).await;
    let client = raw_client().await;
    let pattern = format!("{prefix}_*");
    let body = client
        .search(elasticsearch::SearchParts::Index(&[&pattern]))
        .body(json!({ "query": { "bool": {
            "filter": [
                { "term": { "resource_type": rtype } },
                { "term": { "resource_id": id } },
            ],
            "must_not": [ { "term": { "is_contained": true } } ],
        } } }))
        .send()
        .await
        .expect("find container")
        .json::<Value>()
        .await
        .expect("find container body");
    let hit = body["hits"]["hits"][0].clone();
    (
        hit["_index"].as_str().expect("container index").to_string(),
        hit["_id"].as_str().expect("container id").to_string(),
        hit["_source"].clone(),
    )
}

/// Every contained document of `ctype/cid`: `(index, _id, _source)`.
async fn raw_children(prefix: &str, ctype: &str, cid: &str) -> Vec<(String, String, Value)> {
    refresh_all(prefix).await;
    let client = raw_client().await;
    let pattern = format!("{prefix}_*");
    let body = client
        .search(elasticsearch::SearchParts::Index(&[&pattern]))
        .size(1000)
        .body(json!({ "query": { "bool": { "filter": [
            { "term": { "is_contained": true } },
            { "term": { "container_type": ctype } },
            { "term": { "container_id": cid } },
        ] } } }))
        .send()
        .await
        .expect("raw children")
        .json::<Value>()
        .await
        .expect("raw children body");
    body["hits"]["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| {
            (
                h["_index"].as_str().unwrap().to_string(),
                h["_id"].as_str().unwrap().to_string(),
                h["_source"].clone(),
            )
        })
        .collect()
}

async fn child_ids(prefix: &str, ctype: &str, cid: &str) -> Vec<String> {
    let mut ids: Vec<String> = raw_children(prefix, ctype, cid)
        .await
        .into_iter()
        .map(|(_, id, _)| id)
        .collect();
    ids.sort();
    ids
}

async fn index_raw(index: &str, id: &str, source: Value) -> u16 {
    let client = raw_client().await;
    client
        .index(elasticsearch::IndexParts::IndexId(index, id))
        .refresh(elasticsearch::params::Refresh::True)
        .body(source)
        .send()
        .await
        .expect("index raw")
        .status_code()
        .as_u16()
}

async fn delete_raw(index: &str, id: &str) {
    let client = raw_client().await;
    let _ = client
        .delete(elasticsearch::DeleteParts::IndexId(index, id))
        .refresh(elasticsearch::params::Refresh::True)
        .send()
        .await
        .expect("delete raw");
}

async fn doc_exists(index: &str, id: &str) -> bool {
    let client = raw_client().await;
    client
        .get(elasticsearch::GetParts::IndexId(index, id))
        .send()
        .await
        .expect("get raw")
        .status_code()
        .as_u16()
        == 200
}

/// Rewrites the container document of `rtype/id` after `edit`; the status of the
/// rewrite (`200`/`201`, or `400` if the mapping refuses the edit).
async fn edit_container(prefix: &str, rtype: &str, id: &str, edit: impl FnOnce(&mut Value)) -> u16 {
    let (index, doc_id, mut source) = find_container(prefix, rtype, id).await;
    edit(&mut source);
    index_raw(&index, &doc_id, source).await
}

/// A contained document in the mould of `template`, for `ctype/cid` under
/// `local` at `version`: `(_id, _source)`.
fn child_like(
    template: &Value,
    ctype: &str,
    cid: &str,
    local: &str,
    version: &str,
) -> (String, Value) {
    let mut source = template.clone();
    source["container_type"] = json!(ctype);
    source["container_id"] = json!(cid);
    source["contained_local_id"] = json!(local);
    source["resource_id"] = json!(format!("{ctype}_{cid}#{local}"));
    source["version_id"] = json!(version);
    (child_id(ctype, cid, local), source)
}

/// The first contained document of `ctype/cid`: `(index, source)`.
async fn live_child(prefix: &str, ctype: &str, cid: &str) -> (String, Value) {
    let children = raw_children(prefix, ctype, cid).await;
    let (index, _, source) = children.into_iter().next().expect("a live child exists");
    (index, source)
}

async fn content_names(
    es: &ElasticsearchBackend,
    t: &TenantContext,
    q: &SearchQuery,
) -> Vec<String> {
    es.search(t, q)
        .await
        .expect("search")
        .resources
        .items
        .iter()
        .map(|r| r.content()["name"].as_str().unwrap_or("?").to_string())
        .collect()
}

fn report(case: u8, line: &str) {
    eprintln!("RESULT[{case}] {line}");
}

// ---------------------------------------------------------------------------
// Case 1: malformed `contained_doc_ids`
// ---------------------------------------------------------------------------

type ListMaker = Box<dyn Fn(&str) -> Value + Send + Sync>;

/// `(tag, the list as a function of the container id, whether the backend
/// should read it as unreadable and so sweep)`.
fn malformed_lists() -> Vec<(&'static str, ListMaker, bool)> {
    vec![
        ("null", Box::new(|_| Value::Null), true),
        (
            "string",
            Box::new(|cid| json!(child_id("Observation", cid, "org"))),
            true,
        ),
        ("object", Box::new(|_| json!({ "a": 1 })), true),
        ("numbers", Box::new(|_| json!([1, 2, 3])), true),
        ("noundersc", Box::new(|_| json!(["abc#def"])), true),
        (
            "nohash",
            Box::new(|_| json!(["Organization_Observation_x"])),
            true,
        ),
        ("empty", Box::new(|_| json!([])), false),
        (
            "dups",
            Box::new(|cid| {
                let own = child_id("Observation", cid, "org");
                let gone = child_id("Observation", cid, "gone");
                json!([own, own, gone, gone])
            }),
            false,
        ),
        (
            "big",
            Box::new(|cid| {
                let mut ids = vec![json!(child_id("Observation", cid, "org"))];
                ids.extend(
                    (0..10_000).map(|i| json!(child_id("Observation", cid, &format!("n{i}")))),
                );
                Value::Array(ids)
            }),
            false,
        ),
    ]
}

/// A `contained_doc_ids` of the wrong type, an unreadable entry, empty, with
/// duplicates or with 10 000 entries, then an update (and, on another container,
/// a delete): no panic, the current child stays visible, a list read as
/// unreadable is swept.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case1_malformed_contained_doc_ids_do_not_break_updates_and_deletes() {
    let w = world("chaos1").await;
    let mut failures: Vec<String> = Vec::new();

    for (tag, make, unreadable) in malformed_lists() {
        // ---- update flow ----
        let cid = format!("u{tag}");
        let name = format!("U{tag}");
        let v1 = w.make("Observation", &cid, Some(&name)).await;
        let (index, template) = live_child(&w.prefix, "Observation", &cid).await;
        let (extra_id, extra) =
            child_like(&template, "Observation", &cid, "extra", v1.version_id());
        assert!(index_raw(&index, &extra_id, extra).await < 300);

        let list = make(&cid);
        let status = edit_container(&w.prefix, "Observation", &cid, |s| {
            s["contained_doc_ids"] = list;
        })
        .await;
        if tag == "object" {
            // The mapping (`keyword`) refuses an object: the state cannot exist.
            if status != 400 {
                failures.push(format!(
                    "object list: expected the mapping to refuse it (400), got {status}"
                ));
            }
            report(
                1,
                "object: Elasticsearch's own mapping refuses an object in contained_doc_ids (400); state unreachable",
            );
            continue;
        }
        assert!(
            status < 300,
            "{tag}: the corrupt list was stored (status {status})"
        );

        match w.update(&v1, Some(&name)).await {
            Err(e) => failures.push(format!("{tag}: update failed: {e}")),
            Ok(_) => {
                let shown = w.shown(&name).await;
                if shown != vec!["org".to_string()] {
                    failures.push(format!(
                        "{tag}: after update the children shown are {shown:?}, want [org]"
                    ));
                }
                let physical = child_ids(&w.prefix, "Observation", &cid).await;
                let live = vec![child_id("Observation", &cid, "org")];
                if unreadable && physical != live {
                    failures.push(format!(
                        "{tag}: an unreadable list was not swept: physical children {physical:?}"
                    ));
                }
                report(
                    1,
                    &format!(
                        "{tag}: update ok, shown={shown:?}, physical after update={} ({})",
                        physical.len(),
                        if unreadable {
                            "swept"
                        } else {
                            "extra child kept, hidden by version"
                        }
                    ),
                );
            }
        }

        // ---- delete flow ----
        let cid = format!("d{tag}");
        let name = format!("D{tag}");
        let v1 = w.make("Observation", &cid, Some(&name)).await;
        let (index, template) = live_child(&w.prefix, "Observation", &cid).await;
        let (extra_id, extra) =
            child_like(&template, "Observation", &cid, "extra", v1.version_id());
        assert!(index_raw(&index, &extra_id, extra).await < 300);
        let list = make(&cid);
        assert!(
            edit_container(&w.prefix, "Observation", &cid, |s| s["contained_doc_ids"] =
                list)
            .await
                < 300
        );
        match w.es.delete(&w.t, "Observation", &cid).await {
            Err(e) => failures.push(format!("{tag}: delete failed: {e}")),
            Ok(()) => {
                let physical = child_ids(&w.prefix, "Observation", &cid).await;
                let shown = w.shown(&name).await;
                if !shown.is_empty() {
                    failures.push(format!(
                        "{tag}: after delete children still shown: {shown:?}"
                    ));
                }
                if unreadable && !physical.is_empty() {
                    failures.push(format!(
                        "{tag}: delete of an unreadable list left {physical:?}"
                    ));
                }
                report(
                    1,
                    &format!(
                        "{tag}: delete ok, shown={shown:?}, physical left={} {physical:?}",
                        physical.len()
                    ),
                );
            }
        }
    }

    w.finish().await;
    assert!(
        failures.is_empty(),
        "case 1 failures:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Case 2: the list names the children of ANOTHER container
// ---------------------------------------------------------------------------

/// Observation/o1's list names the child of Observation/o2; updating o1 must not
/// delete o2's child.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case2a_update_does_not_delete_the_children_of_another_container() {
    let w = world("chaos2a").await;
    let o1 = w.make("Observation", "o1", Some("AcmeOne")).await;
    w.make("Observation", "o2", Some("AcmeTwo")).await;
    assert_eq!(w.containers("AcmeTwo").await, vec!["Observation/o2"]);

    let foreign = child_id("Observation", "o2", "org");
    let list = json!([foreign]);
    assert!(
        edit_container(&w.prefix, "Observation", "o1", |s| s["contained_doc_ids"] =
            list)
        .await
            < 300
    );
    w.update(&o1, Some("AcmeOne")).await.expect("update o1");

    let survives = w.containers("AcmeTwo").await == vec!["Observation/o2"];
    report(
        2,
        &format!("update of o1 whose list names o2's child: o2's child survives={survives}"),
    );
    // o1 heals: its own child is visible and its list is its own.
    assert_eq!(w.containers("AcmeOne").await, vec!["Observation/o1"]);
    w.finish().await;
    assert!(
        survives,
        "BUG: updating Observation/o1 deleted the child of Observation/o2"
    );
}

/// The list of Observation/o1 names the child of Patient/o1 (another container
/// type, same id and same local id); updating o1 must not delete it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case2b_update_does_not_delete_the_children_of_a_container_of_another_type() {
    let w = world("chaos2b").await;
    let o1 = w.make("Observation", "o1", Some("AcmeObs")).await;
    w.make("Patient", "o1", Some("AcmePat")).await;
    assert_eq!(w.containers("AcmePat").await, vec!["Patient/o1"]);

    let foreign = child_id("Patient", "o1", "org");
    let list = json!([foreign]);
    assert!(
        edit_container(&w.prefix, "Observation", "o1", |s| s["contained_doc_ids"] =
            list)
        .await
            < 300
    );
    w.update(&o1, Some("AcmeObs")).await.expect("update");

    let survives = w.containers("AcmePat").await == vec!["Patient/o1"];
    report(
        2,
        &format!(
            "update of Observation/o1 whose list names Patient/o1's child: survives={survives}"
        ),
    );
    w.finish().await;
    assert!(
        survives,
        "BUG: updating Observation/o1 deleted the child of Patient/o1"
    );
}

/// Deleting Observation/o1 whose list names the child of Observation/o2 must not
/// delete o2's child.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case2c_delete_does_not_delete_the_children_of_another_container() {
    let w = world("chaos2c").await;
    w.make("Observation", "o1", Some("AcmeOne")).await;
    w.make("Observation", "o2", Some("AcmeTwo")).await;
    let list = json!([child_id("Observation", "o2", "org")]);
    assert!(
        edit_container(&w.prefix, "Observation", "o1", |s| s["contained_doc_ids"] =
            list)
        .await
            < 300
    );
    w.es.delete(&w.t, "Observation", "o1")
        .await
        .expect("delete o1");

    let survives = w.containers("AcmeTwo").await == vec!["Observation/o2"];
    report(
        2,
        &format!("delete of o1 whose list names o2's child: o2's child survives={survives}"),
    );
    w.finish().await;
    assert!(
        survives,
        "BUG: deleting Observation/o1 deleted the child of Observation/o2"
    );
}

// ---------------------------------------------------------------------------
// Case 3: the list names ids of another TENANT
// ---------------------------------------------------------------------------

/// Tenant A's container lists ids that exist only in tenant B; A's update and
/// delete never touch B's documents.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case3_a_list_naming_ids_of_another_tenant_never_touches_it() {
    let w = world("chaos3a").await;
    let tb = w.other("chaos3b");
    let a1 = w.make("Observation", "o1", Some("AlphaA")).await;
    w.make_as(&tb, "Observation", "oB", Some("BravoB")).await;
    // The same id pattern in tenant B as tenant A's own child.
    w.make_as(&tb, "Observation", "o1", Some("CharlieB")).await;

    let list = json!([
        child_id("Observation", "oB", "org"),
        child_id("Observation", "o1", "org"),
    ]);
    let a_index = w.es.index_name("chaos3a", "Observation");
    let id = "Observation_o1".to_string();
    let client = raw_client().await;
    let got = client
        .get(elasticsearch::GetParts::IndexId(&a_index, &id))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let mut src = got["_source"].clone();
    src["contained_doc_ids"] = list.clone();
    assert!(index_raw(&a_index, &id, src).await < 300);

    let a2 = w.update(&a1, Some("AlphaA")).await.expect("update A/o1");
    let b_ok = contained_hits(&w.es, &tb, "BravoB", ContainedReturn::Container).await
        == vec![("Observation".to_string(), "oB".to_string())]
        && contained_hits(&w.es, &tb, "CharlieB", ContainedReturn::Container).await
            == vec![("Observation".to_string(), "o1".to_string())];
    report(
        3,
        &format!("after A's update, tenant B's children intact={b_ok}"),
    );
    assert!(b_ok, "BUG: tenant A's update deleted tenant B's children");

    let src = {
        let got = client
            .get(elasticsearch::GetParts::IndexId(&a_index, &id))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        let mut s = got["_source"].clone();
        s["contained_doc_ids"] = json!([
            child_id("Observation", "oB", "org"),
            child_id("Observation", "o1", "org")
        ]);
        s
    };
    assert!(index_raw(&a_index, &id, src).await < 300);
    w.es.delete(&w.t, "Observation", a2.id())
        .await
        .expect("delete A/o1");
    let b_ok = contained_hits(&w.es, &tb, "BravoB", ContainedReturn::Container)
        .await
        .len()
        == 1
        && contained_hits(&w.es, &tb, "CharlieB", ContainedReturn::Container)
            .await
            .len()
            == 1;
    report(
        3,
        &format!("after A's delete, tenant B's children intact={b_ok}"),
    );
    w.finish().await;
    assert!(b_ok, "BUG: tenant A's delete deleted tenant B's children");
}

// ---------------------------------------------------------------------------
// Case 4: container deleted by hand, children left
// ---------------------------------------------------------------------------

/// A container removed with a raw DELETE leaves its children hidden.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case4a_children_of_a_hand_deleted_container_are_hidden() {
    let w = world("chaos4a").await;
    w.make("Observation", "o1", Some("Acme")).await;
    let (index, id, _) = find_container(&w.prefix, "Observation", "o1").await;
    delete_raw(&index, &id).await;

    let shown = w.shown("Acme").await;
    let containers = w.containers("Acme").await;
    let physical = contained_doc_count(&w.prefix).await;
    report(
        4,
        &format!(
            "container deleted by hand: shown={shown:?} containers={containers:?} physical children={physical}"
        ),
    );
    w.finish().await;
    assert!(shown.is_empty() && containers.is_empty());
    assert_eq!(physical, 1);
}

/// After the hand deletion, the resource is recreated in the SAME version
/// without children: the orphan must not reappear.
#[ignore = "known limit: needs a primary that REUSES a version of a resource it has not purged; purge sweeps, a primary keeps versions rising after a delete"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case4b_recreating_the_container_in_the_same_version_does_not_resurrect_the_orphans() {
    let w = world("chaos4b").await;
    let v1 = w.make("Observation", "o1", Some("Acme")).await;
    let (index, id, _) = find_container(&w.prefix, "Observation", "o1").await;
    delete_raw(&index, &id).await;

    let again = w.make("Observation", "o1", None).await;
    assert_eq!(again.version_id(), v1.version_id(), "same version again");
    let shown = w.shown("Acme").await;
    let containers = w.containers("Acme").await;
    report(
        4,
        &format!(
            "recreate same version without children: orphans reappear={} (shown={shown:?} containers={containers:?}); a create over an absent container never sweeps",
            !shown.is_empty()
        ),
    );
    w.finish().await;
    assert!(
        shown.is_empty() && containers.is_empty(),
        "BUG(latent): orphan children resurface when the container is recreated in the same version \
         (create over Prior::Absent does not sweep; src storage.rs index_new / commit_container)"
    );
}

/// A purge after the recreation removes the orphans physically.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case4c_purge_removes_the_orphans_physically() {
    let w = world("chaos4c").await;
    w.make("Observation", "o1", Some("Acme")).await;
    let (index, id, _) = find_container(&w.prefix, "Observation", "o1").await;
    delete_raw(&index, &id).await;
    w.make("Observation", "o1", None).await;

    w.es.purge(&w.t, "Observation", "o1").await.expect("purge");
    let physical = contained_doc_count(&w.prefix).await;
    let shown = w.shown("Acme").await;
    report(
        4,
        &format!("purge: physical children left={physical}, shown={shown:?}"),
    );
    w.finish().await;
    assert_eq!(physical, 0);
    assert!(shown.is_empty());
}

// ---------------------------------------------------------------------------
// Case 5: container `version_id` changed by hand
// ---------------------------------------------------------------------------

/// A container whose `version_id` is higher, lower, absent or not a string hides
/// its children (no failure), and the next update rewrites them: all healed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case5_a_container_with_a_hand_edited_version_hides_children_and_heals_on_update() {
    let w = world("chaos5").await;
    let mut failures: Vec<String> = Vec::new();
    let variants: Vec<(&str, Option<Value>)> = vec![
        ("higher", Some(json!("99"))),
        ("lower", Some(json!("0"))),
        ("absent", None),
        ("number", Some(json!(7))),
        ("object", Some(json!({ "x": 1 }))),
    ];
    for (tag, value) in variants {
        let cid = format!("c{tag}");
        let name = format!("V{tag}");
        let v1 = w.make("Observation", &cid, Some(&name)).await;
        let status = edit_container(&w.prefix, "Observation", &cid, |s| match &value {
            Some(v) => s["version_id"] = v.clone(),
            None => {
                s.as_object_mut().unwrap().remove("version_id");
            }
        })
        .await;
        if status >= 300 {
            report(
                5,
                &format!("{tag}: mapping refuses the edit ({status}); state unreachable"),
            );
            continue;
        }
        let before = w.containers(&name).await;
        let before_children = w.shown(&name).await;
        if !before.is_empty() || !before_children.is_empty() {
            failures.push(format!(
                "{tag}: mismatching version still shows {before:?} {before_children:?}"
            ));
        }
        match w.update(&v1, Some(&name)).await {
            Err(e) => failures.push(format!("{tag}: update failed: {e}")),
            Ok(_) => {
                let after = w.containers(&name).await;
                if after != vec![format!("Observation/{cid}")] {
                    failures.push(format!("{tag}: not healed after update: {after:?}"));
                }
                report(
                    5,
                    &format!(
                        "{tag}: hidden before update (no error), visible after update={after:?}"
                    ),
                );
            }
        }
    }
    w.finish().await;
    assert!(
        failures.is_empty(),
        "case 5 failures:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Case 6: child with the container's version but wrong content / wrong container
// ---------------------------------------------------------------------------

/// A child carrying the container's version whose content was tampered with is
/// shown as it is (known limit: the version is the only check); an update
/// overwrites it with the real content.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case6a_a_tampered_child_with_the_current_version_is_shown_until_the_next_update() {
    let w = world("chaos6a").await;
    let v1 = w.make("Observation", "o1", Some("Acme")).await;
    let children = raw_children(&w.prefix, "Observation", "o1").await;
    let (index, id, mut src) = children[0].clone();
    // content is `enabled: false`: tamper through whichever form it was stored in.
    if src["content"].is_object() {
        src["content"]["name"] = json!("Tampered");
    } else if let Some(text) = src["content"].as_str() {
        let mut c: Value = serde_json::from_str(text).unwrap();
        c["name"] = json!("Tampered");
        src["content"] = json!(c.to_string());
    }
    assert!(index_raw(&index, &id, src).await < 300);

    let mut q = SearchQuery::new("Organization");
    q.contained = ContainedMode::On;
    q.contained_return = ContainedReturn::Contained;
    q.parameters.push(SearchParameter {
        name: "name".to_string(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Acme")],
        chain: vec![],
        components: vec![],
    });
    let before = content_names(&w.es, &w.t, &q).await;
    report(
        6,
        &format!(
            "tampered child (same version) is returned with content name={before:?} (known limit)"
        ),
    );
    w.update(&v1, Some("Acme")).await.expect("update");
    let after = content_names(&w.es, &w.t, &q).await;
    report(
        6,
        &format!("after update the child's content name={after:?}"),
    );
    w.finish().await;
    assert_eq!(
        after,
        vec!["Acme".to_string()],
        "the update restores the real content"
    );
}

/// A child whose `container_id` points at another container, at that container's
/// version, is shown as its child (known limit); the next update of the real
/// owner hides it and nothing of the real children is lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case6b_a_child_claiming_another_container_is_shown_until_that_container_updates() {
    let w = world("chaos6b").await;
    let o1 = w.make("Observation", "o1", Some("AcmeOne")).await;
    let o2 = w.make("Observation", "o2", Some("AcmeTwo")).await;
    let (index, template) = live_child(&w.prefix, "Observation", "o2").await;
    // The _id is in o1's namespace, the fields claim o2 and its version.
    let (_, mut src) = child_like(&template, "Observation", "o2", "spoof", o2.version_id());
    src["container_id"] = json!("o2");
    let spoof_id = child_id("Observation", "o1", "spoof");
    assert!(index_raw(&index, &spoof_id, src).await < 300);

    let shown = w.shown("AcmeTwo").await;
    report(
        6,
        &format!(
            "child claiming o2 from o1's _id namespace is shown for o2: {shown:?} (known limit)"
        ),
    );

    let o1b = w.update(&o1, Some("AcmeOne")).await.expect("update o1");
    let still = w.shown("AcmeTwo").await;
    report(
        6,
        &format!("after o1's update (it does not list the spoof): shown for o2={still:?}"),
    );
    let o2b = w.update(&o2, Some("AcmeTwo")).await.expect("update o2");
    let healed = w.shown("AcmeTwo").await;
    report(6, &format!("after o2's update: shown for o2={healed:?}"));
    let one = w.shown("AcmeOne").await;
    w.finish().await;
    let _ = (o1b, o2b);
    assert_eq!(
        healed,
        vec!["org".to_string()],
        "o2's update hides the spoof"
    );
    assert_eq!(one, vec!["org".to_string()], "o1's own child is intact");
}

// ---------------------------------------------------------------------------
// Case 7: physical orphans with the current version after delete + recreate
// ---------------------------------------------------------------------------

/// An orphan the list never named, with the container's version: a `delete` must
/// remove it physically.
#[ignore = "known limit: an unlisted orphan stays physically after delete (hidden by the version filter; removed by purge or the orphan collector)"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case7a_delete_removes_an_orphan_the_list_does_not_name() {
    let w = world("chaos7a").await;
    let v1 = w.make("Observation", "o1", Some("Acme")).await;
    let (index, template) = live_child(&w.prefix, "Observation", "o1").await;
    let (orphan, src) = child_like(&template, "Observation", "o1", "orphan", v1.version_id());
    assert!(index_raw(&index, &orphan, src).await < 300);

    w.es.delete(&w.t, "Observation", "o1")
        .await
        .expect("delete");
    let left = child_ids(&w.prefix, "Observation", "o1").await;
    report(
        7,
        &format!("delete with an unlisted orphan: physical children left={left:?}"),
    );
    w.finish().await;
    assert!(
        left.is_empty(),
        "BUG(latent): delete of a Known container leaves unlisted orphans: {left:?}"
    );
}

/// delete then recreate in the same version: the orphan must not resurface.
#[ignore = "known limit: needs a primary that REUSES a version after a delete; purge sweeps, a primary keeps versions rising after a delete"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case7b_recreating_after_delete_in_the_same_version_does_not_resurrect_the_orphan() {
    let w = world("chaos7b").await;
    let v1 = w.make("Observation", "o1", Some("Acme")).await;
    let (index, template) = live_child(&w.prefix, "Observation", "o1").await;
    let (orphan, src) = child_like(&template, "Observation", "o1", "orphan", v1.version_id());
    assert!(index_raw(&index, &orphan, src).await < 300);

    w.es.delete(&w.t, "Observation", "o1")
        .await
        .expect("delete");
    let again = w.make("Observation", "o1", None).await;
    assert_eq!(again.version_id(), v1.version_id());
    let shown = w.shown("Acme").await;
    report(
        7,
        &format!("delete + recreate same version: orphan shown={shown:?}"),
    );
    w.finish().await;
    assert!(
        shown.is_empty(),
        "BUG(latent): an unlisted orphan resurfaces after delete + recreate in the same version: {shown:?}"
    );
}

/// purge removes the orphan (always sweeps), also after the recreation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case7c_purge_removes_an_orphan_the_list_does_not_name() {
    let w = world("chaos7c").await;
    let v1 = w.make("Observation", "o1", Some("Acme")).await;
    let (index, template) = live_child(&w.prefix, "Observation", "o1").await;
    let (orphan, src) = child_like(&template, "Observation", "o1", "orphan", v1.version_id());
    assert!(index_raw(&index, &orphan, src).await < 300);

    w.es.purge(&w.t, "Observation", "o1").await.expect("purge");
    let again = w.make("Observation", "o1", None).await;
    assert_eq!(again.version_id(), v1.version_id());
    let shown = w.shown("Acme").await;
    let physical = contained_doc_count(&w.prefix).await;
    report(
        7,
        &format!("purge + recreate same version: shown={shown:?} physical={physical}"),
    );
    w.finish().await;
    assert!(shown.is_empty());
    assert_eq!(physical, 0);
}

// ---------------------------------------------------------------------------
// Case 8: container document of another tenant in the same index
// ---------------------------------------------------------------------------

/// A container whose `tenant_id` is another tenant's (in tenant A's index) is
/// Absent for A; children of the foreign tenant are never shown to A nor swept by A.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case8_a_container_of_another_tenant_in_the_index_mixes_no_children() {
    let w = world("chaos8").await;
    let v1 = w.make("Observation", "o1", Some("Acme")).await;
    let status = edit_container(&w.prefix, "Observation", "o1", |s| {
        s["tenant_id"] = json!("other")
    })
    .await;
    assert!(status < 300);
    let (index, template) = live_child(&w.prefix, "Observation", "o1").await;
    let (foreign, mut src) = child_like(&template, "Observation", "o1", "foreign", v1.version_id());
    src["tenant_id"] = json!("other");
    assert!(index_raw(&index, &foreign, src).await < 300);

    let shown = w.shown("Acme").await;
    report(
        8,
        &format!("foreign-tenant container: children shown to A={shown:?}"),
    );
    assert!(
        shown.is_empty(),
        "A must not show children under a container of another tenant"
    );

    // Absent for A: the update writes A's container and A's child, sweeps nothing.
    let v2 = w
        .update(&v1, Some("Acme"))
        .await
        .expect("update over a foreign container");
    let shown = w.shown("Acme").await;
    let foreign_alive = doc_exists(&index, &foreign).await;
    report(
        8,
        &format!("after A's update: shown={shown:?}, foreign child still there={foreign_alive}"),
    );
    assert_eq!(
        shown,
        vec!["org".to_string()],
        "no foreign child is shown to A"
    );
    assert!(
        foreign_alive,
        "A's update must not touch the other tenant's child"
    );

    w.es.delete(&w.t, "Observation", v2.id())
        .await
        .expect("delete");
    let foreign_alive = doc_exists(&index, &foreign).await;
    w.es.purge(&w.t, "Observation", "o1").await.expect("purge");
    let foreign_after_purge = doc_exists(&index, &foreign).await;
    report(
        8,
        &format!(
            "after A's delete: foreign child there={foreign_alive}; after A's purge: {foreign_after_purge}"
        ),
    );
    w.finish().await;
    assert!(
        foreign_alive && foreign_after_purge,
        "BUG: tenant A's delete/purge removed another tenant's child"
    );
}

// ---------------------------------------------------------------------------
// Case 9: children index closed or missing
// ---------------------------------------------------------------------------

async fn close_index(index: &str) {
    let client = raw_client().await;
    client
        .indices()
        .close(elasticsearch::indices::IndicesCloseParts::Index(&[index]))
        .send()
        .await
        .expect("close")
        .error_for_status_code()
        .expect("close ok");
}

async fn open_index(index: &str) {
    let client = raw_client().await;
    client
        .indices()
        .open(elasticsearch::indices::IndicesOpenParts::Index(&[index]))
        .send()
        .await
        .expect("open")
        .error_for_status_code()
        .expect("open ok");
}

/// Update while the children's index is closed: an error, no panic; reopened,
/// the repeat heals it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case9a_update_with_the_children_index_closed_fails_cleanly_and_heals_when_reopened() {
    let w = world("chaos9a").await;
    let v1 = w.make("Observation", "o1", Some("Alpha")).await;
    let org_index = w.es.index_name("chaos9a", "Organization");
    close_index(&org_index).await;

    let outcome = w.update(&v1, Some("Beta")).await;
    report(
        9,
        &format!(
            "update with Organization index closed: {}",
            match &outcome {
                Ok(_) => "Ok (unexpected)".to_string(),
                Err(e) => format!("Err: {e}"),
            }
        ),
    );
    open_index(&org_index).await;
    let repeat = w.update(&v1, Some("Beta")).await;
    let ok = repeat.is_ok();
    let shown = if ok {
        w.containers("Beta").await
    } else {
        vec![]
    };
    report(
        9,
        &format!("after reopening, the repeat is ok={ok}, containers for Beta={shown:?}"),
    );
    let stale = w.containers("Alpha").await;
    w.finish().await;
    assert!(
        outcome.is_err(),
        "a closed children index must fail the write loudly"
    );
    assert!(ok);
    assert_eq!(shown, vec!["Observation/o1"]);
    assert!(stale.is_empty());
}

/// Delete while the children's index is closed: an error; reopened, the repeat
/// leaves nothing behind.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case9b_delete_with_the_children_index_closed_fails_cleanly_and_heals_when_reopened() {
    let w = world("chaos9b").await;
    w.make("Observation", "o1", Some("Acme")).await;
    let org_index = w.es.index_name("chaos9b", "Organization");
    close_index(&org_index).await;

    let outcome = w.es.delete(&w.t, "Observation", "o1").await;
    report(
        9,
        &format!(
            "delete with Organization index closed: {}",
            match &outcome {
                Ok(()) => "Ok (unexpected)".to_string(),
                Err(e) => format!("Err: {e}"),
            }
        ),
    );
    let (oidx, oid, _) = (
        w.es.index_name("chaos9b", "Observation"),
        "Observation_o1".to_string(),
        (),
    );
    let container_left = doc_exists(&oidx, &oid).await;
    report(
        9,
        &format!("container document left after the failed delete={container_left}"),
    );
    open_index(&org_index).await;
    let repeat = w.es.delete(&w.t, "Observation", "o1").await;
    report(9, &format!("repeat after reopening: {repeat:?}"));
    let physical = contained_doc_count(&w.prefix).await;
    let shown = w.shown("Acme").await;
    report(
        9,
        &format!("physical children after the repeat={physical}, shown={shown:?}"),
    );
    w.finish().await;
    assert!(
        outcome.is_err(),
        "a closed children index must fail the delete loudly"
    );
    assert_eq!(physical, 0, "the repeat sweeps what the failed delete left");
    assert!(shown.is_empty());
}

/// The children's index does not exist while the container names a child in it:
/// update and delete do not panic; the state converges.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case9c_children_index_missing_while_the_container_names_it() {
    let w = world("chaos9c").await;
    let v1 = w.make("Observation", "o1", Some("Acme")).await;
    let org_index = w.es.index_name("chaos9c", "Organization");
    let client = raw_client().await;
    client
        .indices()
        .delete(elasticsearch::indices::IndicesDeleteParts::Index(&[
            &org_index,
        ]))
        .send()
        .await
        .expect("drop index");

    // The container is still recorded; a search must not fail.
    let shown = w.shown("Acme").await;
    report(
        9,
        &format!("children index dropped: search still works, shown={shown:?}"),
    );

    // Update that drops the child: the stale delete hits a missing index.
    let v2 = w.update(&v1, None).await;
    report(
        9,
        &format!(
            "update dropping the child (index missing): {:?}",
            v2.as_ref().map(|_| "Ok").map_err(|e| e.to_string())
        ),
    );
    let v2 = match v2 {
        Ok(v) => v,
        Err(_) => w.update(&v1, None).await.expect("repeat update"),
    };
    // Recreate a child so the delete has something to name, then drop the index again.
    let v3 = w
        .update(&v2, Some("Acme3"))
        .await
        .expect("update with a child again");
    client
        .indices()
        .delete(elasticsearch::indices::IndicesDeleteParts::Index(&[
            &org_index,
        ]))
        .send()
        .await
        .expect("drop index again");
    let del = w.es.delete(&w.t, "Observation", v3.id()).await;
    report(9, &format!("delete (children index missing): {del:?}"));
    let repeat = w.es.delete(&w.t, "Observation", v3.id()).await;
    report(9, &format!("repeat delete: {repeat:?}"));
    let physical = contained_doc_count(&w.prefix).await;
    w.finish().await;
    assert_eq!(physical, 0);
}

// ---------------------------------------------------------------------------
// Case 10: legacy container (no list) with old-format children
// ---------------------------------------------------------------------------

/// A container without `contained_doc_ids` whose children carry the OLD `_id`
/// (no container type): an update sweeps and leaves only the new children.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case10_a_legacy_container_with_old_format_children_is_swept_by_the_next_update() {
    let w = world("chaos10").await;
    let v1 = w.make("Observation", "o1", Some("Acme")).await;
    let (index, template) = live_child(&w.prefix, "Observation", "o1").await;
    // Turn the state into what a pre-#2291 build left: no list, old-format ids.
    let new_style = child_id("Observation", "o1", "org");
    delete_raw(&index, &new_style).await;
    for local in ["org", "old"] {
        let (_, mut src) = child_like(&template, "Observation", "o1", local, v1.version_id());
        src["resource_id"] = json!(format!("o1#{local}"));
        let old_id = format!("Organization_o1#{local}");
        assert!(index_raw(&index, &old_id, src).await < 300);
    }
    assert!(
        edit_container(&w.prefix, "Observation", "o1", |s| {
            s.as_object_mut().unwrap().remove("contained_doc_ids");
        })
        .await
            < 300
    );
    let before = child_ids(&w.prefix, "Observation", "o1").await;
    report(10, &format!("legacy state: physical children={before:?}"));

    w.update(&v1, Some("Acme"))
        .await
        .expect("update of a legacy container");
    let after = child_ids(&w.prefix, "Observation", "o1").await;
    let shown = w.shown("Acme").await;
    report(
        10,
        &format!("after update: physical children={after:?}, shown={shown:?}"),
    );
    let (_, _, container) = find_container(&w.prefix, "Observation", "o1").await;
    report(
        10,
        &format!("container list now={}", container["contained_doc_ids"]),
    );
    w.finish().await;
    assert_eq!(after, vec![new_style], "only the new child remains");
    assert_eq!(shown, vec!["org".to_string()]);
}
