//! Physical collector of orphaned contained documents (#2291).
//!
//! A contained document (`is_contained: true`) stands only while its container
//! exists and carries the same `version_id` (the read-side filter,
//! `contained_keys` in `search_impl.rs`). A write cut short, a race, or a
//! `$reindex` without `clearExisting` over containers written before the id
//! record existed can leave documents that fail that test: hidden, but still
//! on disk and in the index statistics. [`ElasticsearchBackend::collect_orphans`]
//! removes them, by id, in bounded pages.
//!
//! # How a page goes
//!
//! 1. Under one `contained_sweeps` permit (#2203): one `_search` of at most
//!    `page_size` contained documents of the tenant, sorted by
//!    `container_type, container_id, resource_type, resource_id` (all keyword
//!    with doc values; `resource_type` + `resource_id` is unique per tenant)
//!    and continued with `search_after`; then the page's distinct containers
//!    through `container_versions` (non-real-time `_mget`, the filter's own
//!    lookup). Documents whose container is missing, deleted, or at another
//!    version are *suspects*. The permit is released.
//! 2. The suspects' containers are locked (`lock_containers`, ascending, as a
//!    sync page does), then a permit is taken (lock before permit, the order
//!    every writer uses, so neither can wait for the other), the containers
//!    are re-read with a **real-time** `_mget`, and only the documents whose
//!    container is *still* missing or at another version are deleted, by
//!    `_index`/`_id`, in one `_bulk`. A writer of this process holds the
//!    container's lock from before its `_bulk` (children first, container
//!    last) until it is done, so a child of a write in flight is never taken
//!    for an orphan: the collector waits for the write and then sees its
//!    container.
//!
//! Never a `_delete_by_query`. Request-level transient failures are retried on
//! the read schedule (`MAX_SEARCH_RETRIES`) and then returned; a `_bulk` item
//! that fails is counted in [`OrphanStats::errors`] and left for the next run.
//! Running it again is harmless: what is left is what is current.
//!
//! # Limits
//!
//! A writer in **another** HFS process does not take this process's lock. If
//! its children reached the index and its container has not yet, the
//! collector deletes those children; the container then lists documents that
//! are gone and `_contained` searches miss them until the resource is written
//! again (a false negative, never a wrong hit). The same cross-process gap is
//! noted for the writers themselves in the design doc (3.6).

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use elasticsearch::http::request::JsonBody;
use elasticsearch::{BulkParts, MgetParts};
use serde_json::{Value, json};

use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;

use super::backend::ElasticsearchBackend;
use super::search_impl::{
    EsFailureClass, MAX_SEARCH_RETRIES, RETRY_BASE_DELAY_MS, ReadOp, classify_es_failure,
    send_read_with_retry,
};

/// Contained documents read per page by [`ElasticsearchBackend::collect_orphans`].
pub const ORPHAN_PAGE_SIZE: usize = 500;

/// Containers per real-time `_mget`, as in `container_versions`.
const LIVE_MGET_DOCS_PER_REQUEST: usize = 500;

/// What one run of the collector did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OrphanStats {
    /// Pages of contained documents read.
    pub pages: u64,
    /// Contained documents examined.
    pub examined: u64,
    /// Contained documents deleted because their container was missing or at
    /// another version, also after the real-time re-read.
    pub orphans_deleted: u64,
    /// Deletes that failed (left for the next run).
    pub errors: u64,
}

/// One contained document of a page.
#[derive(Debug, Clone)]
struct ChildHit {
    index: String,
    doc_id: String,
    container: (String, String),
    version_id: Option<String>,
}

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "elasticsearch".to_string(),
        message,
        source: None,
    })
}

/// The query of the tenant's contained documents, optionally of one container
/// type.
fn children_query(tenant_id: &str, container_type: Option<&str>) -> Value {
    let mut filter = vec![
        json!({ "term": { "tenant_id": tenant_id } }),
        json!({ "term": { "is_contained": true } }),
    ];
    if let Some(container_type) = container_type {
        filter.push(json!({ "term": { "container_type": container_type } }));
    }
    json!({ "bool": { "filter": filter } })
}

/// The `_search` body of one page.
fn page_body(
    tenant_id: &str,
    container_type: Option<&str>,
    page_size: usize,
    after: Option<&Value>,
) -> Value {
    let sort_key = |field: &str| {
        let mut key = serde_json::Map::new();
        key.insert(
            field.to_string(),
            json!({ "order": "asc", "unmapped_type": "keyword" }),
        );
        Value::Object(key)
    };
    let mut body = json!({
        "size": page_size,
        "track_total_hits": false,
        "query": children_query(tenant_id, container_type),
        "_source": ["container_type", "container_id", "version_id"],
        "sort": [
            sort_key("container_type"),
            sort_key("container_id"),
            sort_key("resource_type"),
            sort_key("resource_id"),
        ],
    });
    if let Some(after) = after {
        body["search_after"] = after.clone();
    }
    body
}

/// The hits of a page, how many hits the answer carried (judged or not), and
/// the `sort` values of its last hit.
fn parse_page(response: &Value) -> StorageResult<(Vec<ChildHit>, usize, Option<Value>)> {
    let hits = response
        .pointer("/hits/hits")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            internal_error("Failed to read contained documents: no hits in the answer".into())
        })?;
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        let (Some(index), Some(doc_id)) = (
            hit.get("_index").and_then(Value::as_str),
            hit.get("_id").and_then(Value::as_str),
        ) else {
            continue;
        };
        let source = hit.get("_source");
        let field = |name: &str| {
            source
                .and_then(|s| s.get(name))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        // A document without its container's identity cannot be judged; the
        // filter hides it, and this collector leaves it alone.
        let (Some(container_type), Some(container_id)) =
            (field("container_type"), field("container_id"))
        else {
            continue;
        };
        out.push(ChildHit {
            index: index.to_string(),
            doc_id: doc_id.to_string(),
            container: (container_type, container_id),
            version_id: field("version_id"),
        });
    }
    let last_sort = hits.last().and_then(|h| h.get("sort")).cloned();
    Ok((out, hits.len(), last_sort))
}

/// The children whose container is not at their version in `versions`.
fn not_current<'a>(
    children: impl IntoIterator<Item = &'a ChildHit>,
    versions: &HashMap<(String, String), String>,
) -> Vec<ChildHit> {
    children
        .into_iter()
        .filter(|c| versions.get(&c.container) != c.version_id.as_ref())
        .cloned()
        .collect()
}

/// The distinct containers of `children`, sorted.
fn distinct_containers<'a>(
    children: impl IntoIterator<Item = &'a ChildHit>,
) -> Vec<(String, String)> {
    children
        .into_iter()
        .map(|c| c.container.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// What one item of a real-time container `_mget` says: the live version, or
/// `None` for a container that does not count (absent, missing or closed
/// index, another tenant, deleted). The same reading as the filter's lookup.
fn live_version(hit: &Value, tenant_id: &str) -> Result<Option<String>, String> {
    if let Some(error) = hit.get("error") {
        return match error.get("type").and_then(Value::as_str) {
            Some("index_not_found_exception" | "index_closed_exception") => Ok(None),
            _ => Err(error.to_string()),
        };
    }
    if hit.get("found").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let Some(source) = hit.get("_source") else {
        return Ok(None);
    };
    if source.get("tenant_id").and_then(Value::as_str) != Some(tenant_id)
        || source.get("is_deleted").and_then(Value::as_bool) == Some(true)
    {
        return Ok(None);
    }
    Ok(source
        .get("version_id")
        .and_then(Value::as_str)
        .map(str::to_string))
}

impl ElasticsearchBackend {
    /// Physically deletes the tenant's contained documents whose container is
    /// missing, deleted, or at another `version_id` (#2291), the documents the
    /// read-side filter already hides. With `container_type`, only the
    /// documents of containers of that type. See the [module docs](self) for
    /// how a page is read and why a child of a write in flight is not taken.
    ///
    /// Pages of [`ORPHAN_PAGE_SIZE`]. Idempotent; never a `_delete_by_query`.
    pub async fn collect_orphans(
        &self,
        tenant: &TenantContext,
        container_type: Option<&str>,
    ) -> StorageResult<OrphanStats> {
        self.collect_orphans_paged(tenant, container_type, ORPHAN_PAGE_SIZE)
            .await
    }

    /// [`Self::collect_orphans`] with pages of `page_size` documents (at least
    /// 1).
    pub async fn collect_orphans_paged(
        &self,
        tenant: &TenantContext,
        container_type: Option<&str>,
        page_size: usize,
    ) -> StorageResult<OrphanStats> {
        let tenant_id = tenant.tenant_id().as_str();
        let page_size = page_size.max(1);
        let pattern = self.tenant_index_pattern(tenant_id);
        let mut stats = OrphanStats::default();
        let mut after: Option<Value> = None;

        loop {
            // Read the page and the containers' searchable state.
            let (examined, hit_count, last_sort, suspects) = {
                let _permit = self.contained_sweeps.acquire().await.map_err(|_| {
                    internal_error("the contained sweep gate was closed".to_string())
                })?;
                let body = page_body(tenant_id, container_type, page_size, after.as_ref());
                let Some(response) =
                    send_read_with_retry(self, ReadOp::Search, &pattern, body).await?
                else {
                    break;
                };
                let (children, hit_count, last_sort) = parse_page(&response)?;
                let containers = distinct_containers(&children);
                let versions = self.container_versions(tenant_id, &containers).await?;
                let suspects = not_current(&children, &versions);
                (children.len(), hit_count, last_sort, suspects)
            };
            if hit_count == 0 {
                break;
            }
            stats.pages += 1;
            stats.examined += examined as u64;

            if !suspects.is_empty() {
                let containers = distinct_containers(&suspects);
                let _guards = self
                    .lock_containers(
                        tenant_id,
                        containers.iter().map(|(t, i)| (t.as_str(), i.as_str())),
                    )
                    .await;
                let _permit = self.contained_sweeps.acquire().await.map_err(|_| {
                    internal_error("the contained sweep gate was closed".to_string())
                })?;
                let live = self.live_container_versions(tenant_id, &containers).await?;
                let orphans = not_current(&suspects, &live);
                if !orphans.is_empty() {
                    let (deleted, errors) = self.delete_children_by_id(&orphans).await?;
                    stats.orphans_deleted += deleted;
                    stats.errors += errors;
                }
            }

            match last_sort {
                Some(sort) if hit_count == page_size => after = Some(sort),
                _ => break,
            }
        }

        tracing::info!(
            tenant = tenant_id,
            container_type = container_type.unwrap_or("*"),
            pages = stats.pages,
            examined = stats.examined,
            orphans_deleted = stats.orphans_deleted,
            errors = stats.errors,
            "contained orphan collection finished"
        );
        Ok(stats)
    }

    /// The `version_id` of each of `containers` that exists, belongs to
    /// `tenant_id` and is not deleted, read with a **real-time** `_mget`
    /// (what a GET sees, refreshed or not).
    async fn live_container_versions(
        &self,
        tenant_id: &str,
        containers: &[(String, String)],
    ) -> StorageResult<HashMap<(String, String), String>> {
        let what = "re-read the containers of orphaned contained documents";
        let mut versions = HashMap::with_capacity(containers.len());
        for chunk in containers.chunks(LIVE_MGET_DOCS_PER_REQUEST) {
            let docs: Vec<Value> = chunk
                .iter()
                .map(|(container_type, container_id)| {
                    json!({
                        "_index": self.index_name(tenant_id, container_type),
                        "_id": Self::document_id(container_type, container_id),
                        "_source": ["version_id", "tenant_id", "is_deleted"],
                    })
                })
                .collect();
            let body = json!({ "docs": docs });
            let response = self
                .send_maintenance_with_retry(what, || {
                    self.client()
                        .mget(MgetParts::None)
                        .realtime(true)
                        .body(body.clone())
                        .send()
                })
                .await?;
            let hits = response
                .get("docs")
                .and_then(Value::as_array)
                .filter(|hits| hits.len() == chunk.len())
                .ok_or_else(|| {
                    internal_error(format!("Failed to {what}: unexpected answer: {response}"))
                })?;
            for (key, hit) in chunk.iter().zip(hits) {
                match live_version(hit, tenant_id) {
                    Ok(Some(version)) => {
                        versions.insert(key.clone(), version);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return Err(internal_error(format!("Failed to {what}: {error}")));
                    }
                }
            }
        }
        Ok(versions)
    }

    /// Deletes `children` by `_index`/`_id` in one `_bulk` (a page is at most
    /// [`ORPHAN_PAGE_SIZE`] documents) and returns `(deleted, failed)`. A
    /// document already gone counts as neither.
    async fn delete_children_by_id(&self, children: &[ChildHit]) -> StorageResult<(u64, u64)> {
        let what = "delete orphaned contained documents";
        let refresh = self.write_refresh_param();
        let lines: Vec<Value> = children
            .iter()
            .map(|c| json!({ "delete": { "_index": c.index, "_id": c.doc_id } }))
            .collect();
        let response = self
            .send_maintenance_with_retry(what, || {
                let body: Vec<JsonBody<Value>> =
                    lines.iter().cloned().map(JsonBody::from).collect();
                let mut request = self.client().bulk(BulkParts::None).body(body);
                if let Some(refresh) = refresh {
                    request = request.refresh(refresh);
                }
                request.send()
            })
            .await?;
        let items = response
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| internal_error(format!("Failed to {what}: no items in the answer")))?;
        let (mut deleted, mut failed) = (0u64, 0u64);
        for item in items {
            let item = item.get("delete").unwrap_or(item);
            let status = item.get("status").and_then(Value::as_u64).unwrap_or(0);
            match status {
                200..=299 => deleted += 1,
                404 => {}
                _ => {
                    failed += 1;
                    let id = item.get("_id").and_then(|v| v.as_str()).unwrap_or("");
                    let error = item.get("error").cloned().unwrap_or_default();
                    tracing::warn!(
                        status,
                        id,
                        error = %error,
                        "could not delete an orphaned contained document; the next run retries"
                    );
                }
            }
        }
        Ok((deleted, failed))
    }

    /// Sends a maintenance request, retrying transient answers on the read
    /// schedule ([`MAX_SEARCH_RETRIES`] retries), and returns the parsed body
    /// of a 2xx.
    async fn send_maintenance_with_retry<F, Fut>(
        &self,
        what: &str,
        mut send: F,
    ) -> StorageResult<Value>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<
                Output = Result<elasticsearch::http::response::Response, elasticsearch::Error>,
            >,
    {
        let mut last_failure = String::new();
        for attempt in 0..=MAX_SEARCH_RETRIES {
            match send().await {
                Ok(response) => {
                    let status = response.status_code().as_u16();
                    let text = response.text().await.unwrap_or_default();
                    if (200..300).contains(&status) {
                        return serde_json::from_str(&text).map_err(|e| {
                            internal_error(format!("Failed to {what}: unreadable answer: {e}"))
                        });
                    }
                    match classify_es_failure(status, &text) {
                        EsFailureClass::Retryable => {
                            last_failure = format!("status {status}: {text}");
                        }
                        EsFailureClass::BadQuery | EsFailureClass::Permanent => {
                            return Err(internal_error(format!(
                                "Failed to {what} (status {status}): {text}"
                            )));
                        }
                    }
                }
                Err(e) if e.is_timeout() => {
                    return Err(StorageError::Backend(
                        crate::error::elasticsearch_unavailable(
                            &format!(
                                "Failed to {what}: no answer within {} ms",
                                self.request_timeout_ms()
                            ),
                            &e,
                        ),
                    ));
                }
                Err(e) => last_failure = format!("Elasticsearch unreachable: {e}"),
            }
            if attempt < MAX_SEARCH_RETRIES {
                tokio::time::sleep(Duration::from_millis(RETRY_BASE_DELAY_MS << attempt)).await;
            }
        }
        Err(StorageError::Backend(
            crate::error::elasticsearch_unavailable(
                &format!("Failed to {what} after {} attempts", MAX_SEARCH_RETRIES + 1),
                &last_failure,
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(ct: &str, cid: &str, version: Option<&str>) -> ChildHit {
        ChildHit {
            index: "i".into(),
            doc_id: format!("Organization_{ct}_{cid}#o"),
            container: (ct.into(), cid.into()),
            version_id: version.map(str::to_string),
        }
    }

    #[test]
    fn a_child_is_current_only_at_its_containers_version() {
        let versions: HashMap<(String, String), String> =
            [(("Patient".into(), "a".into()), "2".into())].into();
        let children = [
            child("Patient", "a", Some("2")),
            child("Patient", "a", Some("1")),
            child("Patient", "a", None),
            child("Patient", "gone", Some("1")),
        ];
        let stale = not_current(&children, &versions);
        let ids: Vec<_> = stale
            .iter()
            .map(|c| (c.container.1.as_str(), c.version_id.as_deref()))
            .collect();
        assert_eq!(ids, [("a", Some("1")), ("a", None), ("gone", Some("1"))]);
    }

    #[test]
    fn the_live_lookup_reads_containers_like_the_filter() {
        let t = "t1";
        let live = json!({ "found": true, "_source": { "tenant_id": t, "version_id": "3" } });
        let deleted = json!({ "found": true,
            "_source": { "tenant_id": t, "version_id": "3", "is_deleted": true } });
        let other = json!({ "found": true, "_source": { "tenant_id": "x", "version_id": "3" } });
        let missing = json!({ "found": false });
        let no_index = json!({ "error": { "type": "index_not_found_exception" } });
        let broken = json!({ "error": { "type": "shard_failure" } });
        assert_eq!(live_version(&live, t), Ok(Some("3".into())));
        assert_eq!(live_version(&deleted, t), Ok(None));
        assert_eq!(live_version(&other, t), Ok(None));
        assert_eq!(live_version(&missing, t), Ok(None));
        assert_eq!(live_version(&no_index, t), Ok(None));
        assert!(live_version(&broken, t).is_err());
    }

    #[test]
    fn a_page_continues_after_its_last_sort_values() {
        let body = page_body(
            "t",
            Some("Patient"),
            2,
            Some(&json!(["Patient", "a", "O", "x"])),
        );
        assert_eq!(body["size"], 2);
        assert_eq!(body["search_after"], json!(["Patient", "a", "O", "x"]));
        assert_eq!(
            body["query"]["bool"]["filter"][2],
            json!({ "term": { "container_type": "Patient" } })
        );
        assert!(page_body("t", None, 2, None).get("search_after").is_none());
    }
}
