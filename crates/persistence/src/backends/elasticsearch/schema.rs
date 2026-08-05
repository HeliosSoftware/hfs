//! Elasticsearch index schema and mapping definitions.
//!
//! Defines the index structure for FHIR resources in Elasticsearch.
//! Uses nested objects for search parameters to ensure correct multi-value matching.

use elasticsearch::indices::{IndicesCreateParts, IndicesExistsParts, IndicesPutTemplateParts};
use serde_json::json;

use crate::error::{BackendError, StorageResult};

use super::backend::ElasticsearchBackend;

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
                                "value": {
                                    "type": "date",
                                    "format": "strict_date_optional_time||epoch_millis||yyyy||yyyy-MM||yyyy-MM-dd"
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
                                    "format": "strict_date_optional_time||epoch_millis||yyyy||yyyy-MM||yyyy-MM-dd"
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

/// Ensures an index exists for the given tenant and resource type, creating it if necessary.
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
            crate::error::StorageError::Backend(BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message: format!("Failed to check index existence: {}", e),
                source: None,
            })
        })?;

    if exists_response.status_code().is_success() {
        return Ok(());
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
        .map_err(|e| {
            crate::error::StorageError::Backend(BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message: format!("Failed to create index {}: {}", index, e),
                source: None,
            })
        })?;

    let status = response.status_code();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        // 400 with "resource_already_exists_exception" is OK (race condition)
        if body.contains("resource_already_exists_exception") {
            return Ok(());
        }
        return Err(crate::error::StorageError::Backend(
            BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message: format!(
                    "Failed to create index {} (status {}): {}",
                    index, status, body
                ),
                source: None,
            },
        ));
    }

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

        // Verify normalizer
        assert!(mapping["settings"]["analysis"]["normalizer"]["lowercase_normalizer"].is_object());
    }
}
