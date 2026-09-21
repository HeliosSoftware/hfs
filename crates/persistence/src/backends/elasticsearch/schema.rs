//! Elasticsearch index schema and mapping definitions.
//!
//! Defines the index structure for FHIR resources in Elasticsearch.
//! Uses nested objects for search parameters to ensure correct multi-value matching.

use elasticsearch::indices::{
    IndicesCreateParts, IndicesExistsParts, IndicesGetMappingParts, IndicesGetSettingsParts,
    IndicesPutMappingParts, IndicesPutSettingsParts, IndicesPutTemplateParts,
};
use serde_json::{Value, json};

use crate::error::{BackendError, StorageResult};

use super::backend::ElasticsearchBackend;

/// Version of the index mapping built by `create_index_mapping`.
///
/// Every index HFS creates carries this number in its mapping `_meta` under
/// [`SCHEMA_VERSION_META_KEY`]. At startup, and the first time a process touches
/// an index in `ensure_index`, `reconcile_index_mappings` /
/// `reconcile_index` compare the stored number with this one and `PUT
/// _mapping` the current mapping onto any index that is behind. An index with
/// no marker predates the mechanism and counts as version `0`.
///
/// # When to bump
///
/// Bump this by one in the same commit as **any** edit to the `mappings` half
/// of `create_index_mapping`. Without the bump, existing indices never see
/// the change (#1335: `ignore_malformed` from #1314 reached new indices only).
/// Edits to the `settings` half do not need a bump: settings are not part of
/// `PUT _mapping`, and the dynamic ones have their own pass
/// (`raise_nested_objects_limit`).
///
/// # What a bump can and cannot deliver
///
/// Elasticsearch only accepts *additive* mapping changes on a live index.
/// Verified by hand against 7.17.29 and 8.15.0, which behave the same:
///
/// - **Applies in place:** a new field, a new sub-field (`fields`), `_meta`,
///   and these parameters of an existing field: `ignore_malformed` (including
///   on a date inside a `nested` object), `ignore_above`, `coerce`, `meta`.
///   It affects documents indexed from then on; nothing is re-indexed.
/// - **Rejected with a `400`** (`illegal_argument_exception` "Cannot update
///   parameter [...]" / "cannot be changed from type", `mapper_exception`):
///   a field's `type`, `format`, `analyzer`, `normalizer`, `null_value`,
///   `index`, `doc_values`, `store`, an object's `enabled`, and `object` ⇄
///   `nested`. Such a change needs a new index and a `$reindex`; a bump alone
///   only produces the error below for every existing index, on every start.
/// - A mapping that references a *new* analyzer or normalizer is rejected too
///   (`mapper_parsing_exception`): analysis settings are static.
///
/// Two properties of `PUT _mapping` shape the implementation. `_meta` is
/// **replaced**, not merged, so the marker is merged into whatever `_meta` an
/// index already has. And an updatable parameter that is *omitted* is reset to
/// its default, so an older build must never re-apply its mapping over a newer
/// one: an index whose stored version is at or above this one is left alone.
///
/// # Failure policy
///
/// Reconciliation never blocks startup and never fails a write. A failure is
/// logged at `error` with the index name and the server carries on with that
/// index on its old mapping.
///
/// # History
///
/// - `1` — first versioned mapping; delivers `ignore_malformed` on
///   `search_params.date.value` and `search_params.composite.date` (#1314) to
///   indices created before it.
pub const SCHEMA_VERSION: u64 = 1;

/// The key, in an index mapping's `_meta`, that holds [`SCHEMA_VERSION`].
pub const SCHEMA_VERSION_META_KEY: &str = "hfs_schema_version";

/// Creates the index mapping for FHIR resources.
///
/// The mapping includes:
/// - Top-level metadata fields (resource_type, resource_id, version_id, etc.)
/// - `content`: raw FHIR JSON (stored but not indexed)
/// - `narrative_text`: extracted text from resource.text.div for `_text` search
/// - `content_text`: full resource string content for `_content` search
/// - `search_params`: nested fields for each search parameter type
pub fn create_index_mapping(config: &super::backend::ElasticsearchConfig) -> serde_json::Value {
    json!({
        "settings": {
            "number_of_shards": config.number_of_shards,
            "number_of_replicas": config.number_of_replicas,
            "index.max_result_window": config.max_result_window,
            "index.mapping.nested_objects.limit": config.nested_objects_limit,
            "refresh_interval": config.refresh_interval,
            "analysis": {
                "normalizer": {
                    "lowercase_normalizer": {
                        "type": "custom",
                        "filter": ["lowercase"]
                    }
                }
            }
        },
        "mappings": {
            // Any edit below needs a `SCHEMA_VERSION` bump to reach existing
            // indices — see its docs.
            "_meta": { SCHEMA_VERSION_META_KEY: SCHEMA_VERSION },
            "properties": {
                // Metadata fields
                "resource_type": { "type": "keyword" },
                "resource_id": { "type": "keyword" },
                "tenant_id": { "type": "keyword" },
                "version_id": { "type": "keyword" },
                "last_updated": { "type": "date" },
                "fhir_version": { "type": "keyword" },
                "is_deleted": { "type": "boolean" },

                // `_contained` search: a doc extracted from a container's
                // `contained[]` entry is flagged `is_contained` and carries the
                // container's identity plus the contained resource's local id.
                // Its `resource_type`/`search_params` describe the contained
                // resource (so it lands in that type's index and matches normally).
                "is_contained": { "type": "boolean" },
                "container_type": { "type": "keyword" },
                "container_id": { "type": "keyword" },
                "contained_local_id": { "type": "keyword" },

                // Raw FHIR JSON - stored but not indexed
                "content": { "type": "object", "enabled": false },

                // Full-text search fields
                "narrative_text": {
                    "type": "text",
                    "analyzer": "standard"
                },
                "content_text": {
                    "type": "text",
                    "analyzer": "standard"
                },

                // Search parameter fields - all nested for correct multi-value matching
                "search_params": {
                    "properties": {
                        "string": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                "value": {
                                    "type": "text",
                                    "analyzer": "standard",
                                    "fields": {
                                        "keyword": {
                                            "type": "keyword"
                                        },
                                        "lowercase": {
                                            "type": "keyword",
                                            "normalizer": "lowercase_normalizer"
                                        }
                                    }
                                },
                                // Case- and accent-folded value (NFD + combining-mark
                                // stripping, computed by the writer) for accent-
                                // insensitive string search.
                                "folded": { "type": "keyword" }
                            }
                        },
                        "token": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                "system": { "type": "keyword" },
                                "code": { "type": "keyword" },
                                "display": {
                                    "type": "text",
                                    "analyzer": "standard",
                                    "fields": {
                                        "keyword": { "type": "keyword" }
                                    }
                                },
                                "identifier_type_system": { "type": "keyword" },
                                "identifier_type_code": { "type": "keyword" }
                            }
                        },
                        "date": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                // The writer only ever sends a complete UTC
                                // instant (`es_index_date`). `ignore_malformed`
                                // is the net under it: a value Elasticsearch
                                // cannot parse costs the document that one
                                // field instead of rejecting the whole
                                // document (#1314). Existing indices get it
                                // from the reconcile pass (#1335).
                                "value": {
                                    "type": "date",
                                    "format": "strict_date_optional_time||epoch_millis||yyyy||yyyy-MM||yyyy-MM-dd",
                                    "ignore_malformed": true
                                },
                                "precision": { "type": "keyword" }
                            }
                        },
                        "number": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                "value": { "type": "double" }
                            }
                        },
                        "quantity": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                "value": { "type": "double" },
                                "unit": { "type": "keyword" },
                                "system": { "type": "keyword" },
                                "code": { "type": "keyword" },
                                // UCUM-canonical value/unit (computed by the writer)
                                // for unit-equivalent quantity search (g ⇄ mg).
                                "canonical_value": { "type": "double" },
                                "canonical_unit": { "type": "keyword" }
                            }
                        },
                        "reference": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                "reference": { "type": "keyword" },
                                "resource_type": { "type": "keyword" },
                                "resource_id": { "type": "keyword" },
                                "display": { "type": "text" }
                            }
                        },
                        "uri": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                "value": {
                                    "type": "keyword",
                                    "fields": {
                                        "text": { "type": "text" }
                                    }
                                }
                            }
                        },
                        "composite": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                "group_id": { "type": "integer" },
                                // Component values stored inline (as arrays) so a
                                // single nested query matches all components of the
                                // same composite instance.
                                "token_system": { "type": "keyword" },
                                "token_code": { "type": "keyword" },
                                "string": {
                                    "type": "keyword",
                                    "fields": {
                                        "lowercase": {
                                            "type": "keyword",
                                            "normalizer": "lowercase_normalizer"
                                        }
                                    }
                                },
                                "number": { "type": "double" },
                                "quantity_value": { "type": "double" },
                                "quantity_unit": { "type": "keyword" },
                                "quantity_system": { "type": "keyword" },
                                "date": {
                                    "type": "date",
                                    "format": "strict_date_optional_time||epoch_millis||yyyy||yyyy-MM||yyyy-MM-dd",
                                    "ignore_malformed": true
                                },
                                "reference": { "type": "keyword" },
                                "uri": { "type": "keyword" }
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Creates an index template so new indices automatically get the correct mapping.
pub async fn create_index_template(backend: &ElasticsearchBackend) -> StorageResult<()> {
    let template_name = format!("{}_template", backend.config().index_prefix);
    let pattern = format!("{}_*", backend.config().index_prefix);
    let mapping = create_index_mapping(backend.config());

    let template_body = json!({
        "index_patterns": [pattern],
        "settings": mapping["settings"],
        "mappings": mapping["mappings"]
    });

    let response = backend
        .client()
        .indices()
        .put_template(IndicesPutTemplateParts::Name(&template_name))
        .body(template_body)
        .send()
        .await
        .map_err(|e| {
            crate::error::StorageError::Backend(BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message: format!("Failed to create index template: {}", e),
                source: None,
            })
        })?;

    let status = response.status_code();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(crate::error::StorageError::Backend(
            BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message: format!(
                    "Failed to create index template (status {}): {}",
                    status, body
                ),
                source: None,
            },
        ));
    }

    tracing::info!(
        "Created Elasticsearch index template '{}' for pattern '{}'",
        template_name,
        pattern
    );

    // Startup is the one moment an operator is reading these logs, so it is where
    // a pre-fix index layout gets surfaced. Best effort — never fails startup.
    warn_on_misplaced_documents(backend).await;

    Ok(())
}

/// The index setting capping how many nested objects one document may hold.
const NESTED_OBJECTS_LIMIT_SETTING: &str = "index.mapping.nested_objects.limit";

/// Raises the nested-object limit on every existing index under the configured
/// prefix whose current limit is below the configured one, and returns how many
/// indices were raised.
///
/// New indices take the limit from the index template, but a template only
/// applies when an index is created. A deployment that indexed data before
/// #1050 keeps Elasticsearch's default of 10000 on those indices, and keeps
/// silently dropping large resources from search on the next write or
/// `$reindex`. The setting is dynamic, so it changes on a live index without
/// closing or reindexing it.
///
/// The limit is only ever raised: an index an operator already set higher by
/// hand is left alone.
pub async fn raise_nested_objects_limit(backend: &ElasticsearchBackend) -> StorageResult<usize> {
    /// Index names per update request, keeping the request URL short.
    const INDICES_PER_REQUEST: usize = 50;

    let target = u64::from(backend.config().nested_objects_limit);
    let pattern = format!("{}_*", backend.config().index_prefix);

    let response = backend
        .client()
        .indices()
        .get_settings(IndicesGetSettingsParts::IndexName(
            &[&pattern],
            &[NESTED_OBJECTS_LIMIT_SETTING],
        ))
        .include_defaults(true)
        .flat_settings(true)
        .allow_no_indices(true)
        .ignore_unavailable(true)
        .send()
        .await
        .map_err(|e| settings_error(format!("Failed to read index settings: {e}")))?;
    let status = response.status_code();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| settings_error(format!("Failed to parse index settings: {e}")))?;
    if !status.is_success() {
        return Err(settings_error(format!(
            "Reading index settings failed (status {status}): {body}"
        )));
    }

    let below = indices_below_nested_limit(&body, target);
    for chunk in below.chunks(INDICES_PER_REQUEST) {
        let names: Vec<&str> = chunk.iter().map(String::as_str).collect();
        let mut settings = serde_json::Map::new();
        settings.insert(NESTED_OBJECTS_LIMIT_SETTING.to_string(), json!(target));
        let response = backend
            .client()
            .indices()
            .put_settings(IndicesPutSettingsParts::Index(&names))
            .body(serde_json::Value::Object(settings))
            .send()
            .await
            .map_err(|e| {
                settings_error(format!(
                    "Failed to raise {NESTED_OBJECTS_LIMIT_SETTING}: {e}"
                ))
            })?;
        let status = response.status_code();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(settings_error(format!(
                "Raising {NESTED_OBJECTS_LIMIT_SETTING} failed (status {status}): {text}"
            )));
        }
    }
    Ok(below.len())
}

/// The indices in a `GET _settings?include_defaults=true&flat_settings=true`
/// response whose nested-object limit is below `target`, sorted by name.
///
/// An explicit per-index value wins over the cluster default. An index that
/// reports the setting in neither place is left alone rather than guessed at.
fn indices_below_nested_limit(body: &serde_json::Value, target: u64) -> Vec<String> {
    let Some(indices) = body.as_object() else {
        return Vec::new();
    };
    let mut below: Vec<String> = indices
        .iter()
        .filter_map(|(name, entry)| {
            let value = entry
                .get("settings")
                .and_then(|s| s.get(NESTED_OBJECTS_LIMIT_SETTING))
                .or_else(|| {
                    entry
                        .get("defaults")
                        .and_then(|d| d.get(NESTED_OBJECTS_LIMIT_SETTING))
                })?;
            let current = value
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| value.as_u64())?;
            (current < target).then(|| name.clone())
        })
        .collect();
    below.sort();
    below
}

fn settings_error(message: String) -> crate::error::StorageError {
    crate::error::StorageError::Backend(BackendError::Internal {
        backend_name: "elasticsearch".to_string(),
        message,
        source: None,
    })
}

/// The cluster could not be reached, timed out, or asked to be retried (`429`,
/// `5xx`) while an index was checked or created.
///
/// Distinct from a rejection because `ensure_index` runs before every page a
/// rebuild writes: reported as `Internal`, a network blip here failed the whole
/// page as *permanently* rejected resources, and the rebuild never retried them
/// (#1125).
fn index_unavailable(message: String) -> crate::error::StorageError {
    crate::error::StorageError::Backend(BackendError::Unavailable {
        backend_name: "elasticsearch".to_string(),
        message,
    })
}

/// What reconciling one index's mapping came to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReconcileOutcome {
    /// Already at [`SCHEMA_VERSION`] or newer; nothing was sent.
    Current,
    /// The current mapping was applied and the version marker written.
    Updated,
    /// The index disappeared between being listed and being updated.
    Gone,
    /// The cluster could not answer (transport failure, `409`, `429`, `5xx`).
    /// Worth trying again: the next startup, or the next `ensure_index`, does.
    Transient(String),
    /// Elasticsearch refused the mapping (`400`: a change that cannot be made
    /// in place) or the request (`401`/`403`: no `manage` privilege). Trying
    /// again cannot help.
    Rejected(String),
}

/// Totals of one startup [`reconcile_index_mappings`] pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Indices already at [`SCHEMA_VERSION`] or newer.
    pub current: usize,
    /// Indices brought up to [`SCHEMA_VERSION`].
    pub updated: usize,
    /// Indices left on their old mapping; each was logged.
    pub failed: usize,
}

/// `filter_path` that keeps a `GET _mapping` response to a few bytes per index:
/// the `_meta` holding the version marker, plus one field every HFS index has.
/// The second entry keeps an index with no `_meta` in the response (a filter
/// that matches nothing drops the index altogether) and leaves out an index
/// under the prefix that HFS did not create.
const RECONCILE_FILTER_PATH: &[&str] = &[
    "*.mappings._meta",
    "*.mappings.properties.resource_type.type",
];

/// The [`SCHEMA_VERSION`] an index's mapping `_meta` records; `0` when it
/// records none.
fn stored_schema_version(meta: Option<&Value>) -> u64 {
    meta.and_then(|m| m.get(SCHEMA_VERSION_META_KEY))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// The HFS indices in a filtered `GET _mapping` response whose stored version
/// is below [`SCHEMA_VERSION`], each with its existing `_meta`, sorted by name;
/// and how many HFS indices are already current.
fn stale_indices(body: &Value) -> (Vec<(String, Option<Value>)>, usize) {
    let mut stale = Vec::new();
    let mut current = 0;
    for (name, entry) in body.as_object().into_iter().flatten() {
        if entry
            .pointer("/mappings/properties/resource_type")
            .is_none()
        {
            continue;
        }
        let meta = entry.pointer("/mappings/_meta");
        if stored_schema_version(meta) >= SCHEMA_VERSION {
            current += 1;
        } else {
            stale.push((name.clone(), meta.cloned()));
        }
    }
    stale.sort_by(|a, b| a.0.cmp(&b.0));
    (stale, current)
}

/// The `PUT _mapping` body for an index: the current mapping, with the version
/// marker merged into the `_meta` the index already has. `PUT _mapping`
/// replaces `_meta` wholesale, so sending only the marker would erase any
/// other key an operator put there.
fn reconcile_mapping_body(
    config: &super::backend::ElasticsearchConfig,
    existing_meta: Option<&Value>,
) -> Value {
    let mut mappings = create_index_mapping(config)["mappings"].clone();
    let mut meta = existing_meta
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    meta.insert(SCHEMA_VERSION_META_KEY.to_string(), json!(SCHEMA_VERSION));
    mappings["_meta"] = Value::Object(meta);
    mappings
}

/// Whether an existing `_meta` holds anything besides the version marker, in
/// which case the index needs a `PUT _mapping` body of its own.
fn has_foreign_meta(meta: Option<&Value>) -> bool {
    meta.and_then(Value::as_object)
        .is_some_and(|m| m.keys().any(|k| k != SCHEMA_VERSION_META_KEY))
}

/// Classifies a non-success `PUT _mapping` (or `GET _mapping`) answer.
fn classify_mapping_failure(status: u16, body: &str) -> ReconcileOutcome {
    let detail = format!("status {status}: {body}");
    match status {
        404 => ReconcileOutcome::Gone,
        // `409`: a concurrent cluster-state update won; the same request
        // succeeds once it has settled.
        409 | 429 | 500..=599 => ReconcileOutcome::Transient(detail),
        _ => ReconcileOutcome::Rejected(detail),
    }
}

/// Sends one `PUT _mapping` to `indices`.
///
/// Idempotent, which is what makes several HFS instances starting together
/// safe: they all read the same stale version and all send the same body, and
/// Elasticsearch serialises the mapping updates on the master — the second one
/// is a no-op.
async fn put_mapping(
    backend: &ElasticsearchBackend,
    indices: &[&str],
    body: Value,
) -> ReconcileOutcome {
    let response = match backend
        .client()
        .indices()
        .put_mapping(IndicesPutMappingParts::Index(indices))
        .body(body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => return ReconcileOutcome::Transient(e.to_string()),
    };
    let status = response.status_code();
    if status.is_success() {
        return ReconcileOutcome::Updated;
    }
    let text = response.text().await.unwrap_or_default();
    classify_mapping_failure(status.as_u16(), &text)
}

/// Reads the filtered mapping of `target` (an index name or a glob).
async fn read_schema_versions(
    backend: &ElasticsearchBackend,
    target: &str,
) -> Result<Value, ReconcileOutcome> {
    let response = backend
        .client()
        .indices()
        .get_mapping(IndicesGetMappingParts::Index(&[target]))
        .filter_path(RECONCILE_FILTER_PATH)
        .allow_no_indices(true)
        .ignore_unavailable(true)
        .send()
        .await
        .map_err(|e| ReconcileOutcome::Transient(e.to_string()))?;
    let status = response.status_code();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(classify_mapping_failure(status.as_u16(), &text));
    }
    response
        .json::<Value>()
        .await
        .map_err(|e| ReconcileOutcome::Transient(format!("unreadable mapping response: {e}")))
}

/// Logs an index that stays on its old mapping, and records the ones that a
/// retry cannot fix so `ensure_index` does not ask again on every write.
fn note_reconcile_outcome(backend: &ElasticsearchBackend, index: &str, outcome: &ReconcileOutcome) {
    match outcome {
        ReconcileOutcome::Current | ReconcileOutcome::Updated => {
            backend.mark_schema_checked(index);
        }
        ReconcileOutcome::Gone => {}
        ReconcileOutcome::Transient(detail) => tracing::error!(
            index,
            schema_version = SCHEMA_VERSION,
            detail,
            "could not reconcile the Elasticsearch index mapping; the index keeps its old \
             mapping until the next attempt (next write to it, or next startup)"
        ),
        ReconcileOutcome::Rejected(detail) => {
            backend.mark_schema_checked(index);
            tracing::error!(
                index,
                schema_version = SCHEMA_VERSION,
                detail,
                "Elasticsearch refused the current index mapping; the index keeps its old \
                 mapping. A `400` means the change cannot be made in place (recreate the index \
                 and `$reindex`); a `401`/`403` means the HFS user lacks the `manage` index \
                 privilege"
            );
        }
    }
}

/// Brings every existing HFS index under the configured prefix up to
/// [`SCHEMA_VERSION`]. Run at startup; see [`SCHEMA_VERSION`] for the
/// convention and the failure policy.
///
/// Indices are `{prefix}_{tenant}_{type}` — one per tenant per resource type —
/// so they are discovered with the `{prefix}_*` glob, as the template and
/// [`raise_nested_objects_limit`] do. One request when nothing is stale (the
/// normal start); otherwise one more per `INDICES_PER_PUT` stale indices.
///
/// Infallible by design: every failure is logged and counted, never returned.
pub async fn reconcile_index_mappings(backend: &ElasticsearchBackend) -> ReconcileReport {
    /// Index names per `PUT _mapping`, keeping the request URL short.
    const INDICES_PER_PUT: usize = 50;

    let pattern = format!("{}_*", backend.config().index_prefix);
    let mut report = ReconcileReport::default();

    let body = match read_schema_versions(backend, &pattern).await {
        Ok(body) => body,
        Err(outcome) => {
            // No index could be read, so none is named: log the glob.
            note_reconcile_outcome(backend, &pattern, &outcome);
            report.failed = usize::from(outcome != ReconcileOutcome::Gone);
            return report;
        }
    };
    let (stale, current) = stale_indices(&body);
    report.current = current;

    // Indices with nothing of their own in `_meta` share one body, so they go
    // out in chunks; a failed chunk is redone index by index so the error
    // names the index that caused it.
    let (own_body, shared): (Vec<_>, Vec<_>) = stale
        .into_iter()
        .partition(|(_, meta)| has_foreign_meta(meta.as_ref()));
    let mut singly: Vec<(String, Option<Value>)> = own_body;
    for chunk in shared.chunks(INDICES_PER_PUT) {
        let names: Vec<&str> = chunk.iter().map(|(name, _)| name.as_str()).collect();
        let body = reconcile_mapping_body(backend.config(), None);
        if names.len() > 1 && put_mapping(backend, &names, body).await == ReconcileOutcome::Updated
        {
            for name in names {
                note_reconcile_outcome(backend, name, &ReconcileOutcome::Updated);
            }
            report.updated += chunk.len();
        } else {
            singly.extend(chunk.iter().cloned());
        }
    }
    for (name, meta) in singly {
        let body = reconcile_mapping_body(backend.config(), meta.as_ref());
        let outcome = put_mapping(backend, &[&name], body).await;
        note_reconcile_outcome(backend, &name, &outcome);
        match outcome {
            ReconcileOutcome::Updated => report.updated += 1,
            ReconcileOutcome::Current | ReconcileOutcome::Gone => {}
            ReconcileOutcome::Transient(_) | ReconcileOutcome::Rejected(_) => report.failed += 1,
        }
    }

    if report.updated > 0 || report.failed > 0 {
        tracing::info!(
            schema_version = SCHEMA_VERSION,
            updated = report.updated,
            failed = report.failed,
            current = report.current,
            "reconciled Elasticsearch index mappings"
        );
    }
    report
}

/// Brings one existing index up to [`SCHEMA_VERSION`], once per process.
///
/// The startup pass covers the indices that exist at startup. This covers the
/// rest: an index an older HFS instance creates afterwards during a rolling
/// upgrade, and one the startup pass could not reach. On the write path, so it
/// never returns an error — see [`SCHEMA_VERSION`].
async fn reconcile_index(backend: &ElasticsearchBackend, index: &str) {
    if backend.is_schema_checked(index) {
        return;
    }
    let outcome = match read_schema_versions(backend, index).await {
        Err(outcome) => outcome,
        Ok(body) => match stale_indices(&body) {
            (stale, _) if !stale.is_empty() => {
                let meta = stale[0].1.as_ref();
                let body = reconcile_mapping_body(backend.config(), meta);
                put_mapping(backend, &[index], body).await
            }
            (_, 1) => ReconcileOutcome::Current,
            // Not an index HFS laid out (no `resource_type` field), or gone.
            _ => ReconcileOutcome::Gone,
        },
    };
    if outcome == ReconcileOutcome::Updated {
        tracing::info!(
            index,
            schema_version = SCHEMA_VERSION,
            "reconciled Elasticsearch index mapping"
        );
    }
    note_reconcile_outcome(backend, index, &outcome);
}

/// Ensures an index exists for the given tenant and resource type, creating it
/// if necessary, and that an existing one carries the current mapping.
pub async fn ensure_index(
    backend: &ElasticsearchBackend,
    tenant_id: &str,
    resource_type: &str,
) -> StorageResult<()> {
    let index = backend.index_name(tenant_id, resource_type);

    // Check if index exists
    let exists_response = backend
        .client()
        .indices()
        .exists(IndicesExistsParts::Index(&[&index]))
        .send()
        .await
        .map_err(|e| {
            index_unavailable(format!("Failed to check index existence for {index}: {e}"))
        })?;

    let exists_status = exists_response.status_code();
    if exists_status.is_success() {
        reconcile_index(backend, &index).await;
        return Ok(());
    }
    // A throttled or failing cluster says nothing about whether the index
    // exists; creating it now would only fail the same way, or worse, race.
    if super::storage::is_transient_bulk_status(u64::from(exists_status.as_u16())) {
        return Err(index_unavailable(format!(
            "Failed to check index existence for {index} (status {exists_status})"
        )));
    }

    // Create the index with mappings
    let mapping = create_index_mapping(backend.config());

    let response = backend
        .client()
        .indices()
        .create(IndicesCreateParts::Index(&index))
        .body(mapping)
        .send()
        .await
        .map_err(|e| index_unavailable(format!("Failed to create index {index}: {e}")))?;

    let status = response.status_code();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        // 400 with "resource_already_exists_exception" is OK (race condition)
        if body.contains("resource_already_exists_exception") {
            return Ok(());
        }
        let message = format!(
            "Failed to create index {} (status {}): {}",
            index, status, body
        );
        if super::storage::is_transient_bulk_status(u64::from(status.as_u16())) {
            return Err(index_unavailable(message));
        }
        return Err(crate::error::StorageError::Backend(
            BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message,
                source: None,
            },
        ));
    }

    // Created from the current mapping, marker included.
    backend.mark_schema_checked(&index);
    tracing::debug!("Created Elasticsearch index '{}'", index);
    Ok(())
}

// `delete_index` — a whole-index `DELETE` addressed by index name alone — was
// removed here (issue #384). It was `#[allow(dead_code)]` with no callers, and it
// was safe only because `index_name` is injective. Keeping an unreachable,
// untested whole-index drop around is a latent footgun: the first caller to wire
// it up would not re-derive that argument. Its plausible use — tenant offboarding
// — is already served, document-level and tenant-term-filtered, by
// `ResourceStorage::purge_tenant_data` and `PurgableStorage::purge_all`.

/// Warns, once at startup, about documents sitting in an index that the current
/// tenant → index derivation would not put them in.
///
/// # Why this exists
///
/// The #384 fix makes the derivation injective. For a tenant id that was already
/// lowercase and Elasticsearch-safe the encoding is the identity, so nothing
/// moves and this reports nothing. A deployment that actually had a
/// non-conforming tenant id, however, now addresses a *different* index, and its
/// pre-upgrade documents stay where the old derivation put them. The symptom is
/// silent: that tenant's search results go empty (or, worse, stay partial) while
/// reads, writes, and history — all served by the primary — look perfectly
/// healthy. Nobody reindexes an index they do not know is wrong.
///
/// # Why this compares documents, not index names
///
/// The obvious check — "is this index name something the encoder could have
/// produced?" — would miss the very case the issue is about. The old derivation
/// *lowercased*, so tenant `ACME` wrote to `{prefix}_acme_patient`, which is a
/// perfectly well-formed name for tenant `acme`. There is no malformed name to
/// spot. What is actually wrong is the *contents*: that index holds documents
/// whose `tenant_id` is `ACME`, which this build would place in
/// `{prefix}_+41+43+4d+45_patient`.
///
/// So this aggregates the distinct `tenant_id` values present in each index and
/// flags any whose encoded form does not match the index's own tenant segment.
/// That detects both the collision case and any stranded-index case, and it needs
/// no heuristic about name shape.
///
/// # Deliberate limits
///
/// - **Best effort; never fails startup.** Misplaced documents are inert — no
///   query path reaches them across a tenant boundary (`read` re-checks
///   `tenant_id`, every glob-scoped query carries a `term` filter), so refusing to
///   boot would turn a search-completeness problem into a total outage. An
///   unreachable cluster is silently ignored here; `health_check` reports that.
/// - **One aggregation, at startup only.** Not on the write path.
/// - It reports the condition; it does not repair it. Remediation is `$reindex`
///   for the affected tenant, then a delete-by-query filtered on that tenant's
///   exact `tenant_id` to remove the strays.
async fn warn_on_misplaced_documents(backend: &ElasticsearchBackend) {
    let prefix = &backend.config().index_prefix;
    let pattern = format!("{prefix}_*");

    let response = match backend
        .client()
        .search(elasticsearch::SearchParts::Index(&[&pattern]))
        .body(json!({
            "size": 0,
            "aggs": {
                "per_index": {
                    "terms": { "field": "_index", "size": 1000 },
                    "aggs": {
                        "tenants": { "terms": { "field": "tenant_id", "size": 100 } }
                    }
                }
            }
        }))
        .allow_no_indices(true)
        .ignore_unavailable(true)
        .send()
        .await
    {
        Ok(r) if r.status_code().is_success() => r,
        // Unreachable cluster, or a cluster with no indices yet. Not this
        // function's job to report — stay silent rather than mislead.
        _ => return,
    };

    let Ok(body) = response.json::<serde_json::Value>().await else {
        return;
    };
    let Some(index_buckets) = body
        .pointer("/aggregations/per_index/buckets")
        .and_then(|b| b.as_array())
    else {
        return;
    };

    let index_prefix = format!("{prefix}_");
    for index_bucket in index_buckets {
        let Some(index) = index_bucket.get("key").and_then(|k| k.as_str()) else {
            continue;
        };
        // `{prefix}_{tenant}_{type}`: the tenant segment is everything between
        // the prefix and the final `_`.
        let Some((tenant_segment, type_segment)) = index
            .strip_prefix(&index_prefix)
            .and_then(|rest| rest.rsplit_once('_'))
        else {
            continue;
        };

        let tenant_buckets = index_bucket
            .pointer("/tenants/buckets")
            .and_then(|b| b.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default();

        for tenant_bucket in tenant_buckets {
            let Some(tenant_id) = tenant_bucket.get("key").and_then(|k| k.as_str()) else {
                continue;
            };
            if super::naming::encode_tenant_segment(tenant_id) == tenant_segment {
                continue;
            }
            tracing::warn!(
                index = %index,
                tenant_id = %tenant_id,
                expected_index = %super::naming::index_name(prefix, tenant_id, type_segment),
                doc_count = tenant_bucket.get("doc_count").and_then(|c| c.as_u64()).unwrap_or(0),
                "Elasticsearch documents predate the injective tenant-index naming fix \
                 (issue #384): they sit in an index this build would not write them to, so \
                 they are invisible to that tenant's searches. Run `$reindex` for this \
                 tenant, then remove the strays with a delete-by-query filtered on this \
                 exact `tenant_id`."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::elasticsearch::ElasticsearchConfig;

    #[test]
    fn test_create_index_mapping_structure() {
        let config = ElasticsearchConfig::default();
        let mapping = create_index_mapping(&config);

        // Verify settings
        assert_eq!(mapping["settings"]["number_of_shards"], 1);
        assert_eq!(mapping["settings"]["number_of_replicas"], 1);
        // #1050: raised above Elasticsearch's 10000 default, which silently
        // drops resources with many indexed values from search.
        assert_eq!(
            mapping["settings"]["index.mapping.nested_objects.limit"],
            50_000
        );

        // Verify mappings exist
        let props = &mapping["mappings"]["properties"];
        assert!(props["resource_type"]["type"].as_str() == Some("keyword"));
        assert!(props["resource_id"]["type"].as_str() == Some("keyword"));
        assert!(props["content"]["enabled"].as_bool() == Some(false));
        assert!(props["narrative_text"]["type"].as_str() == Some("text"));

        // Verify nested search params
        let sp = &props["search_params"]["properties"];
        assert_eq!(sp["string"]["type"], "nested");
        assert_eq!(sp["token"]["type"], "nested");
        assert_eq!(sp["date"]["type"], "nested");
        assert_eq!(sp["number"]["type"], "nested");
        assert_eq!(sp["quantity"]["type"], "nested");
        assert_eq!(sp["reference"]["type"], "nested");
        assert_eq!(sp["uri"]["type"], "nested");

        // #1314: one malformed date must not reject a whole document.
        let date = &sp["date"]["properties"]["value"];
        assert_eq!(date["type"], "date");
        assert_eq!(date["ignore_malformed"], true);
        let composite_date = &sp["composite"]["properties"]["date"];
        assert_eq!(composite_date["type"], "date");
        assert_eq!(composite_date["ignore_malformed"], true);

        // Verify normalizer
        assert!(mapping["settings"]["analysis"]["normalizer"]["lowercase_normalizer"].is_object());
    }

    /// #1335: a new index is born at the current version, through both the
    /// create body and the template (which reuse this mapping).
    #[test]
    fn test_index_mapping_carries_schema_version() {
        let mapping = create_index_mapping(&ElasticsearchConfig::default());
        assert_eq!(
            mapping["mappings"]["_meta"][SCHEMA_VERSION_META_KEY],
            SCHEMA_VERSION
        );
    }

    /// #1335: which indices the reconcile pass updates. No marker is version
    /// 0; a newer marker is never downgraded; an index under the prefix that
    /// HFS did not lay out (no `resource_type` field) is not touched.
    #[test]
    fn test_stale_indices() {
        let hfs = json!({ "resource_type": { "type": "keyword" } });
        let body = json!({
            "hfs_t_patient": { "mappings": { "properties": hfs } },
            "hfs_t_observation": { "mappings": {
                "_meta": { "owner": "ops" }, "properties": hfs
            } },
            "hfs_t_encounter": { "mappings": {
                "_meta": { SCHEMA_VERSION_META_KEY: SCHEMA_VERSION }, "properties": hfs
            } },
            "hfs_t_condition": { "mappings": {
                "_meta": { SCHEMA_VERSION_META_KEY: SCHEMA_VERSION + 1 }, "properties": hfs
            } },
            "hfs_not_ours": { "mappings": { "_meta": { "x": 1 } } }
        });

        let (stale, current) = stale_indices(&body);
        assert_eq!(
            stale,
            vec![
                (
                    "hfs_t_observation".to_string(),
                    Some(json!({ "owner": "ops" }))
                ),
                ("hfs_t_patient".to_string(), None),
            ]
        );
        assert_eq!(current, 2);
        assert_eq!(stale_indices(&json!({})), (Vec::new(), 0));
        assert_eq!(stale_indices(&json!("not an object")), (Vec::new(), 0));
    }

    /// #1335: `PUT _mapping` replaces `_meta`, so the marker is merged into
    /// what is there; and the body is the mapping half only.
    #[test]
    fn test_reconcile_mapping_body() {
        let config = ElasticsearchConfig::default();

        let plain = reconcile_mapping_body(&config, None);
        assert_eq!(plain, create_index_mapping(&config)["mappings"]);
        assert!(plain.get("settings").is_none());

        let existing = json!({ "owner": "ops", SCHEMA_VERSION_META_KEY: 0 });
        let merged = reconcile_mapping_body(&config, Some(&existing));
        assert_eq!(
            merged["_meta"],
            json!({ "owner": "ops", SCHEMA_VERSION_META_KEY: SCHEMA_VERSION })
        );
        assert_eq!(merged["properties"], plain["properties"]);

        assert!(has_foreign_meta(Some(&existing)));
        assert!(!has_foreign_meta(Some(
            &json!({ SCHEMA_VERSION_META_KEY: 0 })
        )));
        assert!(!has_foreign_meta(None));
    }

    /// #1335: which `PUT _mapping` failures are worth another attempt.
    #[test]
    fn test_classify_mapping_failure() {
        assert_eq!(classify_mapping_failure(404, ""), ReconcileOutcome::Gone);
        for status in [409, 429, 500, 503] {
            assert!(
                matches!(
                    classify_mapping_failure(status, ""),
                    ReconcileOutcome::Transient(_)
                ),
                "{status}"
            );
        }
        for status in [400, 401, 403] {
            assert!(
                matches!(
                    classify_mapping_failure(status, ""),
                    ReconcileOutcome::Rejected(_)
                ),
                "{status}"
            );
        }
    }

    /// #1050: which existing indices the startup pass raises. An explicit
    /// value beats the cluster default, an index already above the target is
    /// never lowered, and an index that reports no value is not guessed at.
    #[test]
    fn test_indices_below_nested_limit() {
        let body = json!({
            "hfs_t_provenance": {
                "settings": {},
                "defaults": { "index.mapping.nested_objects.limit": "10000" }
            },
            "hfs_t_observation": {
                "settings": { "index.mapping.nested_objects.limit": "10000" },
                "defaults": {}
            },
            "hfs_t_patient": {
                "settings": { "index.mapping.nested_objects.limit": "80000" },
                "defaults": { "index.mapping.nested_objects.limit": "10000" }
            },
            "hfs_t_encounter": {
                "settings": { "index.mapping.nested_objects.limit": "50000" }
            },
            "hfs_t_unknown": { "settings": {}, "defaults": {} }
        });

        assert_eq!(
            indices_below_nested_limit(&body, 50_000),
            vec![
                "hfs_t_observation".to_string(),
                "hfs_t_provenance".to_string()
            ]
        );
        assert!(indices_below_nested_limit(&json!({}), 50_000).is_empty());
        assert!(indices_below_nested_limit(&json!("not an object"), 50_000).is_empty());
    }
}
