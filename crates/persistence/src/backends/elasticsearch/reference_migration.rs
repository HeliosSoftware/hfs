//! Fills `search_params.reference.reference_target` on documents indexed
//! before schema version 3 (#2394).
//!
//! `reference_target` (`Type/id` of an absolute `http(s)` reference) is computed
//! when a resource is indexed, so a mapping update alone leaves old documents
//! without it, and a reference search or `_revinclude` would not match their
//! absolute references. Instead of asking operators to run `$reindex`, the
//! backend asks Elasticsearch to derive the field from the reference string each
//! document already holds, with one `_update_by_query` that runs inside the
//! cluster.
//!
//! * **No state to keep.** The query selects only nested reference rows that
//!   have an `http(s)` reference and no `reference_target`, so a second start
//!   (or a second instance starting at the same time) picks up exactly what is
//!   left, and finds nothing once the work is done. An interrupted run resumes
//!   the same way.
//! * **Safe next to live writes.** `conflicts=proceed`: a document rewritten
//!   concurrently was indexed with the field already.
//! * **Background and throttled.** `wait_for_completion=false` returns a task id
//!   and the cluster does the work; `requests_per_second` bounds the load.
//! * **Same rule as the writer.** [`REFERENCE_TARGET_SCRIPT`] mirrors
//!   `types::absolute_reference_target`; a test indexes a table of references
//!   in the old shape and compares the cluster's result with the Rust function.
//!
//! An absolute reference that does not end in a FHIR `Type/id` (for example
//! `http://example.org/foo`) gets `reference_target = ""`, which no search
//! matches. That marks the row as handled — the writer does the same — so the
//! query stops selecting it and a finished migration really finds nothing.
//!
//! Only indices whose mapping already has `reference_target` as a `keyword` are
//! touched. On any other index the script would create the field by dynamic
//! mapping, as `text`, and the real mapping could then never be applied. That
//! also leaves out indices under the prefix that HFS did not create.

use elasticsearch::http::Method;
use elasticsearch::http::headers::HeaderMap;
use elasticsearch::indices::IndicesGetMappingParts;
use elasticsearch::params::Conflicts;
use elasticsearch::{CountParts, Elasticsearch, UpdateByQueryParts};
use serde_json::{Value, json};

use super::backend::ElasticsearchBackend;

/// Documents per scroll batch of the update.
const SCROLL_SIZE: i64 = 500;

/// Throttle, in documents per second, so the migration does not compete with
/// searches and writes. About 7 million documents per hour.
const REQUESTS_PER_SECOND: i64 = 2000;

/// Painless mirror of `absolute_reference_target`. Operates on every nested
/// reference row that has no `reference_target` yet and leaves the document
/// untouched (`noop`) when no row changed.
pub(super) const REFERENCE_TARGET_SCRIPT: &str = r#"
def sp = ctx._source.search_params;
if (sp == null || !(sp.reference instanceof List)) { ctx.op = 'noop'; return; }
boolean changed = false;
for (def r : sp.reference) {
  if (r.reference_target != null || !(r.reference instanceof String)) { continue; }
  String s = r.reference;
  String rest;
  if (s.startsWith('https://')) { rest = s.substring(8); }
  else if (s.startsWith('http://')) { rest = s.substring(7); }
  else { continue; }
  int cut = rest.length();
  int q = rest.indexOf('?');
  if (q >= 0 && q < cut) { cut = q; }
  int h = rest.indexOf('#');
  if (h >= 0 && h < cut) { cut = h; }
  rest = rest.substring(0, cut);
  int v = rest.indexOf('/_history/');
  if (v >= 0) { rest = rest.substring(0, v); }
  int i2 = rest.lastIndexOf('/');
  if (i2 < 0) { r.reference_target = ''; changed = true; continue; }
  String id = rest.substring(i2 + 1);
  String head = rest.substring(0, i2);
  int i1 = head.lastIndexOf('/');
  if (i1 < 0) { r.reference_target = ''; changed = true; continue; }
  String type = head.substring(i1 + 1);
  if (type.length() == 0 || id.length() == 0 || id.length() > 64) { r.reference_target = ''; changed = true; continue; }
  boolean ok = true;
  for (int k = 0; k < type.length() && ok; k++) {
    int c = type.charAt(k);
    boolean upper = c >= 65 && c <= 90;
    boolean lower = c >= 97 && c <= 122;
    boolean digit = c >= 48 && c <= 57;
    if (k == 0) { ok = upper; } else { ok = upper || lower || digit; }
  }
  for (int k = 0; k < id.length() && ok; k++) {
    int c = id.charAt(k);
    ok = (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || (c >= 48 && c <= 57) || c == 45 || c == 46;
  }
  if (!ok) { r.reference_target = ''; changed = true; continue; }
  r.reference_target = type + '/' + id;
  changed = true;
}
if (!changed) { ctx.op = 'noop'; }
"#;

/// Selects the documents that still have an absolute reference without
/// `reference_target`. `ignore_unmapped` keeps an index without the nested
/// path from failing the whole request.
pub(super) fn pending_query() -> Value {
    json!({
        "nested": {
            "path": "search_params.reference",
            "ignore_unmapped": true,
            "query": { "bool": {
                "filter": [{ "bool": { "should": [
                    { "prefix": { "search_params.reference.reference": "http://" } },
                    { "prefix": { "search_params.reference.reference": "https://" } }
                ] } }],
                "must_not": [
                    { "exists": { "field": "search_params.reference.reference_target" } }
                ]
            } }
        }
    })
}

/// The `_update_by_query` body.
pub(super) fn update_body() -> Value {
    json!({
        "query": pending_query(),
        "script": { "lang": "painless", "source": REFERENCE_TARGET_SCRIPT }
    })
}

/// Indices per request, keeping the URL short.
const INDICES_PER_REQUEST: usize = 50;

/// How often the watcher asks the cluster about a running migration.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

/// The watcher gives up on a task after this long; the migration itself keeps
/// running in the cluster.
const WATCH_LIMIT: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Names, out of `body` (a filtered `GET {target}/_mapping`), of the indices
/// whose `reference_target` is mapped as a `keyword`.
pub(super) fn indices_with_reference_target(body: &Value) -> Vec<String> {
    let mut names: Vec<String> = body
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, index)| {
            index["mappings"]["properties"]["search_params"]["properties"]["reference"]
                ["properties"]["reference_target"]["type"]
                == "keyword"
        })
        .map(|(name, _)| name.clone())
        .collect();
    names.sort();
    names
}

/// How a finished `_update_by_query` task ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TaskSummary {
    pub updated: u64,
    pub version_conflicts: u64,
    pub failures: Vec<String>,
}

/// Reads a `GET _tasks/{id}` body: `None` while the task is still running.
pub(super) fn summarize_task(body: &Value) -> Option<TaskSummary> {
    if body["completed"].as_bool() != Some(true) {
        return None;
    }
    let mut failures: Vec<String> = Vec::new();
    if !body["error"].is_null() {
        failures.push(body["error"].to_string());
    }
    for key in ["failures", "search_failures"] {
        for f in body["response"][key].as_array().into_iter().flatten() {
            failures.push(f.to_string());
        }
    }
    Some(TaskSummary {
        updated: body["response"]["updated"].as_u64().unwrap_or(0),
        version_conflicts: body["response"]["version_conflicts"].as_u64().unwrap_or(0),
        failures,
    })
}

/// What a call to [`migrate_reference_targets`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// Indices with a mapped `reference_target` that were looked at.
    pub examined: usize,
    /// Task ids of the updates started.
    pub tasks: Vec<String>,
    /// Requests that failed; each was logged.
    pub failed: usize,
}

/// Starts the migration for `target` (an index name or a glob): for every index
/// with a mapped `reference_target` that still has documents waiting for it, one
/// `_update_by_query` runs in the cluster and a watcher logs how it ends. Once
/// per index per process. Never fails: errors are logged and the next start
/// tries again.
pub(super) async fn migrate_reference_targets(
    backend: &ElasticsearchBackend,
    target: &str,
) -> MigrationReport {
    let mut report = MigrationReport::default();
    let mapped = match mapped_indices(backend, target).await {
        Ok(names) => names,
        Err(detail) => {
            tracing::warn!(
                target,
                detail,
                "could not read the Elasticsearch mappings to find documents waiting \
                 for `reference_target` (#2394); retried at the next start"
            );
            report.failed += 1;
            return report;
        }
    };
    report.examined = mapped.len();
    let todo: Vec<String> = mapped
        .into_iter()
        .filter(|name| backend.claim_reference_migration(name))
        .collect();
    for chunk in todo.chunks(INDICES_PER_REQUEST) {
        let names: Vec<&str> = chunk.iter().map(String::as_str).collect();
        match start_for(backend, &names).await {
            Ok(Some(task)) => report.tasks.push(task),
            Ok(None) => {}
            Err(detail) => {
                report.failed += 1;
                for name in chunk {
                    backend.release_reference_migration(name);
                }
                tracing::warn!(
                    indices = chunk.len(),
                    sample = chunk[0].as_str(),
                    detail,
                    "could not start the `reference_target` migration (#2394); \
                     retried at the next start"
                );
            }
        }
    }
    report
}

/// The indices matching `target` whose mapping has `reference_target`.
async fn mapped_indices(
    backend: &ElasticsearchBackend,
    target: &str,
) -> Result<Vec<String>, String> {
    let response = backend
        .client()
        .indices()
        .get_mapping(IndicesGetMappingParts::Index(&[target]))
        .filter_path(&[
            "*.mappings.properties.search_params.properties.reference.properties.reference_target.type",
        ])
        .allow_no_indices(true)
        .ignore_unavailable(true)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status_code();
    if status.as_u16() == 404 {
        return Ok(Vec::new());
    }
    if !status.is_success() {
        return Err(format!("get mapping answered status {status}"));
    }
    let body = response.json::<Value>().await.map_err(|e| e.to_string())?;
    Ok(indices_with_reference_target(&body))
}

/// Counts, then if needed starts the update over `names`. `Ok(None)`: nothing
/// is waiting.
async fn start_for(
    backend: &ElasticsearchBackend,
    names: &[&str],
) -> Result<Option<String>, String> {
    let response = backend
        .client()
        .count(CountParts::Index(names))
        .allow_no_indices(true)
        .ignore_unavailable(true)
        .body(json!({ "query": pending_query() }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status_code();
    if !status.is_success() {
        return Err(format!("count answered status {status}"));
    }
    let body = response.json::<Value>().await.map_err(|e| e.to_string())?;
    if body["_shards"]["failed"].as_u64().unwrap_or(0) > 0 {
        return Err(format!("count failed on some shards: {}", body["_shards"]));
    }
    let pending = body["count"]
        .as_u64()
        .ok_or_else(|| format!("count answered without a number: {body}"))?;
    if pending == 0 {
        return Ok(None);
    }

    let response = backend
        .client()
        .update_by_query(UpdateByQueryParts::Index(names))
        .allow_no_indices(true)
        .ignore_unavailable(true)
        .conflicts(Conflicts::Proceed)
        .scroll_size(SCROLL_SIZE)
        .requests_per_second(REQUESTS_PER_SECOND)
        .wait_for_completion(false)
        .refresh(true)
        .body(update_body())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status_code();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!(
            "update_by_query answered status {status}: {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    let body = response.json::<Value>().await.map_err(|e| e.to_string())?;
    let task = body["task"]
        .as_str()
        .ok_or_else(|| format!("update_by_query returned no task id: {body}"))?
        .to_string();
    tracing::info!(
        indices = names.len(),
        sample = names[0],
        pending,
        task = task.as_str(),
        "started the background migration that fills `reference_target` on \
         Elasticsearch documents indexed before #2394; absolute references match \
         reference searches and _revinclude as it progresses, no `$reindex` is needed"
    );
    watch_task(backend.client().clone(), task.clone());
    Ok(Some(task))
}

/// Logs how `task` ends: `info` on success, `warn` with the failures otherwise.
fn watch_task(client: Elasticsearch, task: String) {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let mut errors = 0u32;
        while started.elapsed() < WATCH_LIMIT {
            tokio::time::sleep(POLL_INTERVAL).await;
            // The typed Tasks API sits behind the crate's `experimental-apis`
            // feature; the endpoint itself is stable, so call it directly.
            let path = format!("/_tasks/{task}");
            let body = match client
                .send(
                    Method::Get,
                    &path,
                    HeaderMap::new(),
                    None::<&()>,
                    None::<&()>,
                    None,
                )
                .await
            {
                Ok(r) if r.status_code().is_success() => r.json::<Value>().await.ok(),
                _ => None,
            };
            let Some(body) = body else {
                errors += 1;
                if errors >= 10 {
                    tracing::warn!(
                        task = task.as_str(),
                        "lost track of the `reference_target` migration task (#2394); \
                         it keeps running in the cluster"
                    );
                    return;
                }
                continue;
            };
            errors = 0;
            if let Some(summary) = summarize_task(&body) {
                if summary.failures.is_empty() {
                    tracing::info!(
                        task = task.as_str(),
                        updated = summary.updated,
                        version_conflicts = summary.version_conflicts,
                        "finished the `reference_target` migration (#2394)"
                    );
                } else {
                    tracing::warn!(
                        task = task.as_str(),
                        updated = summary.updated,
                        failures = summary.failures.len(),
                        first = summary.failures[0].as_str(),
                        "the `reference_target` migration (#2394) ended with failures; \
                         the documents still waiting are retried at the next start"
                    );
                }
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_selects_http_rows_without_the_field() {
        let q = pending_query().to_string();
        assert!(q.contains(r#""path":"search_params.reference""#));
        assert!(q.contains(r#""ignore_unmapped":true"#));
        assert!(q.contains(r#""http://""#) && q.contains(r#""https://""#));
        assert!(q.contains(r#""must_not""#) && q.contains("reference_target"));
    }

    #[test]
    fn the_body_carries_the_painless_script() {
        let body = update_body();
        assert_eq!(body["script"]["lang"], "painless");
        let source = body["script"]["source"].as_str().unwrap();
        assert!(source.contains("ctx.op = 'noop'"));
        assert!(source.contains("r.reference_target = ''"));
    }

    #[test]
    fn only_indices_that_map_reference_target_as_keyword_are_listed() {
        let mapped = |t: &str| {
            json!({ "mappings": { "properties": { "search_params": { "properties": {
                "reference": { "properties": { "reference_target": { "type": t } } }
            } } } } })
        };
        let body = json!({
            "hfs_t_observation": mapped("keyword"),
            "hfs_t_old": { "mappings": { "properties": {} } },
            "hfs_t_text": mapped("text"),
            "hfs_logs": {},
            "hfs_a_patient": mapped("keyword"),
        });
        assert_eq!(
            indices_with_reference_target(&body),
            ["hfs_a_patient", "hfs_t_observation"]
        );
        assert!(indices_with_reference_target(&json!({})).is_empty());
    }

    #[test]
    fn a_task_summary_reports_completion_and_failures() {
        assert_eq!(summarize_task(&json!({ "completed": false })), None);
        let ok = summarize_task(&json!({
            "completed": true,
            "response": { "updated": 7, "version_conflicts": 2, "failures": [] }
        }))
        .unwrap();
        assert_eq!(
            ok,
            TaskSummary {
                updated: 7,
                version_conflicts: 2,
                failures: vec![]
            }
        );
        let bad = summarize_task(&json!({
            "completed": true,
            "error": { "type": "search_phase_execution_exception" },
            "response": { "updated": 0, "failures": [{ "cause": "x" }] }
        }))
        .unwrap();
        assert_eq!(bad.failures.len(), 2);
    }
}
