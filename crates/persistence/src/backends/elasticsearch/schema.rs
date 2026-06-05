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
                                }
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
                                "code": { "type": "keyword" }
                            }
                        },
                        "reference": {
                            "type": "nested",
                            "properties": {
                                "name": { "type": "keyword" },
                                "reference": { "type": "keyword" },
                                "resource_type": { "type": "keyword" },
                                "resource_id": { "type": "keyword" }
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

/// Deletes an index for the given tenant and resource type.
#[allow(dead_code)]
pub async fn delete_index(
    backend: &ElasticsearchBackend,
    tenant_id: &str,
    resource_type: &str,
) -> StorageResult<()> {
    let index = backend.index_name(tenant_id, resource_type);

    let response = backend
        .client()
        .indices()
        .delete(elasticsearch::indices::IndicesDeleteParts::Index(&[&index]))
        .send()
        .await
        .map_err(|e| {
            crate::error::StorageError::Backend(BackendError::Internal {
                backend_name: "elasticsearch".to_string(),
                message: format!("Failed to delete index {}: {}", index, e),
                source: None,
            })
        })?;

    let status = response.status_code();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        // 404 is OK (index doesn't exist)
        if !body.contains("index_not_found_exception") {
            return Err(crate::error::StorageError::Backend(
                BackendError::Internal {
                    backend_name: "elasticsearch".to_string(),
                    message: format!("Failed to delete index {}: {}", index, body),
                    source: None,
                },
            ));
        }
    }

    tracing::debug!("Deleted Elasticsearch index '{}'", index);
    Ok(())
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
