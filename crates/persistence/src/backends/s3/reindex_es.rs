//! S3 -> Elasticsearch reindex support.
//!
//! This module rebuilds Elasticsearch search indices from S3 canonical objects
//! for a single tenant. It is intended for operator repair workflows.

use std::collections::HashSet;

use elasticsearch::indices::IndicesDeleteParts;
use serde_json::Value;

use crate::backends::elasticsearch::ElasticsearchBackend;
use crate::core::ResourceStorage;
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::S3Backend;

/// Options for rebuilding Elasticsearch indexes from S3 current objects.
#[derive(Debug, Clone)]
pub struct S3ToElasticsearchReindexOptions {
    /// Number of S3 objects to process per batch.
    pub batch_size: usize,
    /// Whether existing tenant indexes should be deleted before replay.
    pub clear_existing: bool,
    /// Optional list of resource types to include; when `None`, all types are processed.
    pub resource_types: Option<Vec<String>>,
}

impl Default for S3ToElasticsearchReindexOptions {
    fn default() -> Self {
        Self {
            batch_size: 500,
            clear_existing: false,
            resource_types: None,
        }
    }
}

/// Summary of a tenant reindex run.
#[derive(Debug, Clone, Default)]
pub struct S3ToElasticsearchReindexReport {
    /// Number of current pointers scanned from S3.
    pub scanned: u64,
    /// Number of live resources indexed/upserted into ES.
    pub indexed: u64,
    /// Number of delete operations applied to ES for tombstoned resources.
    pub deleted: u64,
    /// Number of scanned resources that were tombstoned in S3.
    pub skipped_deleted: u64,
}

fn parse_current_key(key: &str) -> Option<(&str, &str)> {
    let parts = key.split('/').collect::<Vec<_>>();
    let resources_pos = parts.iter().position(|part| *part == "resources")?;
    if parts.len() <= resources_pos + 3 {
        return None;
    }

    let resource_type = parts[resources_pos + 1];
    let resource_id = parts[resources_pos + 2];
    let terminal = parts[resources_pos + 3];

    if terminal != "current.json" || resource_type.is_empty() || resource_id.is_empty() {
        return None;
    }

    Some((resource_type, resource_id))
}

fn with_version_meta(mut content: Value, version: &str) -> Value {
    let Some(obj) = content.as_object_mut() else {
        return content;
    };

    let meta = obj
        .entry("meta")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    if !meta.is_object() {
        *meta = Value::Object(serde_json::Map::new());
    }

    if let Some(meta_obj) = meta.as_object_mut() {
        meta_obj.insert("versionId".to_string(), Value::String(version.to_string()));
    }

    content
}

fn normalize_type_filter(resource_types: Option<Vec<String>>) -> Option<HashSet<String>> {
    resource_types.map(|types| {
        types
            .into_iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect::<HashSet<_>>()
    })
}

impl S3Backend {
    /// Rebuilds Elasticsearch indexes from S3 current objects for a tenant.
    pub async fn reindex_to_elasticsearch(
        &self,
        tenant: &TenantContext,
        es: &ElasticsearchBackend,
        options: S3ToElasticsearchReindexOptions,
    ) -> StorageResult<S3ToElasticsearchReindexReport> {
        let batch_size = options.batch_size.max(1);
        let type_filter = normalize_type_filter(options.resource_types);
        let location = self.tenant_location(tenant)?;

        let keys = if let Some(filter) = &type_filter {
            let mut out = Vec::new();
            for resource_type in filter {
                out.extend(
                    self.list_current_keys(&location, Some(resource_type.as_str()))
                        .await?,
                );
            }
            out
        } else {
            self.list_current_keys(&location, None).await?
        };

        if options.clear_existing {
            let mut resource_types = HashSet::new();
            if let Some(filter) = &type_filter {
                resource_types.extend(filter.iter().cloned());
            } else {
                for key in &keys {
                    if let Some((resource_type, _)) = parse_current_key(key) {
                        resource_types.insert(resource_type.to_ascii_lowercase());
                    }
                }
            }

            for resource_type in resource_types {
                let index = es.index_name(tenant.tenant_id().as_str(), &resource_type);
                let response = es
                    .client()
                    .indices()
                    .delete(IndicesDeleteParts::Index(&[&index]))
                    .send()
                    .await;

                if let Ok(resp) = response {
                    let status = resp.status_code();
                    if !status.is_success() && status.as_u16() != 404 {
                        let body = resp.text().await.unwrap_or_default();
                        return Err(StorageError::Backend(BackendError::Internal {
                            backend_name: "elasticsearch".to_string(),
                            message: format!(
                                "Failed to clear index {} during reindex (status {}): {}",
                                index, status, body
                            ),
                            source: None,
                        }));
                    }
                }
            }
        }

        let mut touched_types = HashSet::new();
        let mut report = S3ToElasticsearchReindexReport::default();

        for chunk in keys.chunks(batch_size) {
            for key in chunk {
                let Some((resource_type, resource_id)) = parse_current_key(key) else {
                    continue;
                };

                if let Some(filter) = &type_filter {
                    if !filter.contains(&resource_type.to_ascii_lowercase()) {
                        continue;
                    }
                }

                report.scanned += 1;
                let loaded = self
                    .get_json_object::<StoredResource>(&location.bucket, key)
                    .await?
                    .map(|(resource, _)| resource);

                let Some(resource) = loaded else {
                    continue;
                };

                if resource.is_deleted() {
                    es.delete(tenant, resource_type, resource_id).await?;
                    report.deleted += 1;
                    report.skipped_deleted += 1;
                    touched_types.insert(resource_type.to_string());
                    continue;
                }

                let content = with_version_meta(resource.content().clone(), resource.version_id());

                es.create_or_update(
                    tenant,
                    resource_type,
                    resource_id,
                    content,
                    resource.fhir_version(),
                )
                .await?;
                report.indexed += 1;
                touched_types.insert(resource_type.to_string());
            }
        }

        for resource_type in touched_types {
            // Refresh failures should not fail the repair run; they only affect visibility latency.
            let _ = es
                .refresh_index(tenant.tenant_id().as_str(), &resource_type)
                .await;
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_current_key;

    #[test]
    fn parse_current_key_valid_paths() {
        assert_eq!(
            parse_current_key("resources/Patient/123/current.json"),
            Some(("Patient", "123"))
        );
        assert_eq!(
            parse_current_key("tenant-a/resources/Observation/o1/current.json"),
            Some(("Observation", "o1"))
        );
    }

    #[test]
    fn parse_current_key_rejects_non_current_paths() {
        assert_eq!(
            parse_current_key("resources/Patient/123/_history/1.json"),
            None
        );
        assert_eq!(parse_current_key("resources/Patient/current.json"), None);
        assert_eq!(parse_current_key("invalid"), None);
    }
}
