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
//! References that are absolute but do not end in a FHIR `Type/id` (for
//! example `http://example.org/foo`) get no field, so they stay selected: the
//! update touches nothing for them (`ctx.op = 'noop'`) and costs one scan.

use elasticsearch::UpdateByQueryParts;
use elasticsearch::params::Conflicts;
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
  if (i2 < 0) { continue; }
  String id = rest.substring(i2 + 1);
  String head = rest.substring(0, i2);
  int i1 = head.lastIndexOf('/');
  if (i1 < 0) { continue; }
  String type = head.substring(i1 + 1);
  if (type.length() == 0 || id.length() == 0 || id.length() > 64) { continue; }
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
  if (!ok) { continue; }
  r.reference_target = type + '/' + id;
  changed = true;
}
if (!changed) { ctx.op = 'noop'; }
"#;

/// Selects the documents that still have an absolute reference without
/// `reference_target`.
pub(super) fn pending_query() -> Value {
    json!({
        "nested": {
            "path": "search_params.reference",
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

/// What a call to [`migrate_reference_targets`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// No document is waiting for the field.
    NothingPending,
    /// An update was started; `task` is its id when the cluster returned one
    /// (it is absent when the update ran to completion in the request).
    Started { pending: u64, task: Option<String> },
    /// The cluster refused or could not be reached; logged, retried next start.
    Failed(String),
}

/// Starts the migration over `target` (an index name or a glob) if any document
/// is still waiting for `reference_target`. Never fails: errors are logged and
/// the next start tries again. With `wait`, returns once the update has
/// finished instead of leaving it as a background task.
pub(super) async fn migrate_reference_targets(
    backend: &ElasticsearchBackend,
    target: &str,
    wait: bool,
) -> MigrationOutcome {
    let targets = [target];
    let counted = backend
        .client()
        .count(elasticsearch::CountParts::Index(&targets))
        .allow_no_indices(true)
        .ignore_unavailable(true)
        .body(json!({ "query": pending_query() }))
        .send()
        .await;
    let pending = match counted {
        Ok(response) if response.status_code().is_success() => response
            .json::<Value>()
            .await
            .ok()
            .and_then(|body| body["count"].as_u64())
            .unwrap_or(0),
        Ok(response) => {
            let status = response.status_code().as_u16();
            tracing::warn!(
                target,
                status,
                "could not count Elasticsearch documents waiting for \
                 `reference_target` (#2394); they are retried at the next start"
            );
            return MigrationOutcome::Failed(format!("count answered status {status}"));
        }
        Err(e) => {
            tracing::warn!(
                target,
                error = %e,
                "could not count Elasticsearch documents waiting for \
                 `reference_target` (#2394); they are retried at the next start"
            );
            return MigrationOutcome::Failed(e.to_string());
        }
    };
    if pending == 0 {
        return MigrationOutcome::NothingPending;
    }

    let sent = backend
        .client()
        .update_by_query(UpdateByQueryParts::Index(&targets))
        .allow_no_indices(true)
        .ignore_unavailable(true)
        .conflicts(Conflicts::Proceed)
        .scroll_size(SCROLL_SIZE)
        .requests_per_second(REQUESTS_PER_SECOND)
        .wait_for_completion(wait)
        .refresh(true)
        .body(update_body())
        .send()
        .await;
    match sent {
        Ok(response) if response.status_code().is_success() => {
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            let task = body["task"].as_str().map(str::to_string);
            tracing::info!(
                target,
                pending,
                task = task.as_deref().unwrap_or("-"),
                "started the background migration that fills `reference_target` on \
                 Elasticsearch documents indexed before #2394; absolute references \
                 match reference searches and _revinclude as it progresses, no \
                 `$reindex` is needed"
            );
            MigrationOutcome::Started { pending, task }
        }
        Ok(response) => {
            let status = response.status_code().as_u16();
            let text = response.text().await.unwrap_or_default();
            tracing::warn!(
                target,
                status,
                response = %text.chars().take(300).collect::<String>(),
                "Elasticsearch refused the `reference_target` migration (#2394); \
                 it is retried at the next start"
            );
            MigrationOutcome::Failed(format!("update_by_query answered status {status}"))
        }
        Err(e) => {
            tracing::warn!(
                target,
                error = %e,
                "could not start the `reference_target` migration (#2394); \
                 it is retried at the next start"
            );
            MigrationOutcome::Failed(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_selects_http_rows_without_the_field() {
        let q = pending_query().to_string();
        assert!(q.contains(r#""path":"search_params.reference""#));
        assert!(q.contains(r#""http://""#) && q.contains(r#""https://""#));
        assert!(q.contains(r#""must_not""#) && q.contains("reference_target"));
    }

    #[test]
    fn the_body_carries_the_painless_script() {
        let body = update_body();
        assert_eq!(body["script"]["lang"], "painless");
        assert!(
            body["script"]["source"]
                .as_str()
                .unwrap()
                .contains("ctx.op = 'noop'")
        );
    }
}
