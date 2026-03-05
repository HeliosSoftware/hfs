//! ResourceStorage implementation for Elasticsearch.
//!
//! Provides the minimal CRUD operations needed for the SyncManager to propagate
//! changes from the primary backend. The ES backend is primarily a search secondary,
//! but it must implement ResourceStorage for sync support.

use async_trait::async_trait;
use chrono::Utc;
use elasticsearch::{DeleteParts, GetParts, IndexParts};
use helios_fhir::FhirVersion;
use serde_json::{Value, json};

use crate::core::ResourceStorage;
use crate::error::{BackendError, StorageError, StorageResult};
use crate::search::converters::IndexValue;
use crate::search::extractor::ExtractedValue;
use crate::tenant::TenantContext;
use crate::types::StoredResource;

use super::backend::ElasticsearchBackend;
use super::schema;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "elasticsearch".to_string(),
        message,
        source: None,
    })
}

/// Content extracted from a resource for full-text search.
struct SearchableContent {
    narrative: String,
    full_content: String,
}

/// Extracts searchable text content from a FHIR resource.
fn extract_searchable_content(resource: &Value) -> SearchableContent {
    SearchableContent {
        narrative: extract_narrative(resource),
        full_content: extract_all_strings(resource),
    }
}

/// Extracts narrative text from resource.text.div, stripping HTML tags.
fn extract_narrative(resource: &Value) -> String {
    resource
        .get("text")
        .and_then(|t| t.get("div"))
        .and_then(|d| d.as_str())
        .map(strip_html_tags)
        .unwrap_or_default()
}

/// Strips HTML tags from a string, returning plain text.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                result.push(' ');
            }
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Normalize whitespace
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extracts all string values from a JSON value recursively.
fn extract_all_strings(value: &Value) -> String {
    let mut parts = Vec::new();
    collect_strings(value, &mut parts);
    parts.join(" ")
}

fn collect_strings(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            if !s.is_empty() {
                parts.push(s.clone());
            }
        }
        Value::Object(map) => {
            for (key, val) in map {
                // Skip metadata fields and large binary data
                if key == "div" || key == "data" {
                    continue;
                }
                collect_strings(val, parts);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                collect_strings(val, parts);
            }
        }
        _ => {}
    }
}

/// Ensures the JSON resource payload has the expected `resourceType` and `id`.
fn normalize_resource_shape(resource_type: &str, id: &str, mut resource: Value) -> Value {
    if let Some(obj) = resource.as_object_mut() {
        obj.insert(
            "resourceType".to_string(),
            Value::String(resource_type.to_string()),
        );
        obj.insert("id".to_string(), Value::String(id.to_string()));
    }
    resource
}

/// Returns the requested version encoded in `resource.meta.versionId`, if any.
fn requested_version(resource: &Value) -> Option<u64> {
    resource
        .get("meta")
        .and_then(|m| m.get("versionId"))
        .and_then(|v| v.as_str())
        .and_then(|v| v.parse::<u64>().ok())
}

/// Parses `version_id` from an ES `_source` object.
fn source_version(source: &Value) -> Option<u64> {
    source
        .get("version_id")
        .and_then(|v| v.as_str())
        .and_then(|v| v.parse::<u64>().ok())
}

/// Builds an ES document from a FHIR resource and its extracted search values.
pub(crate) fn build_es_document(
    tenant_id: &str,
    resource_type: &str,
    resource_id: &str,
    version_id: &str,
    content: &Value,
    fhir_version: FhirVersion,
    extracted_values: &[ExtractedValue],
) -> Value {
    let searchable = extract_searchable_content(content);

    let mut string_params: Vec<Value> = Vec::new();
    let mut token_params: Vec<Value> = Vec::new();
    let mut date_params: Vec<Value> = Vec::new();
    let mut number_params: Vec<Value> = Vec::new();
    let mut quantity_params: Vec<Value> = Vec::new();
    let mut reference_params: Vec<Value> = Vec::new();
    let mut uri_params: Vec<Value> = Vec::new();
    let mut composite_params: Vec<Value> = Vec::new();

    for ev in extracted_values {
        match &ev.value {
            IndexValue::String(s) => {
                string_params.push(json!({
                    "name": ev.param_name,
                    "value": s,
                }));
            }
            IndexValue::Token {
                system,
                code,
                display,
                identifier_type_system,
                identifier_type_code,
            } => {
                let mut token = json!({
                    "name": ev.param_name,
                    "code": code,
                });
                if let Some(sys) = system {
                    token["system"] = json!(sys);
                }
                if let Some(disp) = display {
                    token["display"] = json!(disp);
                }
                if let Some(its) = identifier_type_system {
                    token["identifier_type_system"] = json!(its);
                }
                if let Some(itc) = identifier_type_code {
                    token["identifier_type_code"] = json!(itc);
                }
                token_params.push(token);
            }
            IndexValue::Date { value, precision } => {
                date_params.push(json!({
                    "name": ev.param_name,
                    "value": value,
                    "precision": format!("{:?}", precision).to_lowercase(),
                }));
            }
            IndexValue::Number(n) => {
                number_params.push(json!({
                    "name": ev.param_name,
                    "value": n,
                }));
            }
            IndexValue::Quantity {
                value,
                unit,
                system,
                code,
            } => {
                let mut qty = json!({
                    "name": ev.param_name,
                    "value": value,
                });
                if let Some(u) = unit {
                    qty["unit"] = json!(u);
                }
                if let Some(s) = system {
                    qty["system"] = json!(s);
                }
                if let Some(c) = code {
                    qty["code"] = json!(c);
                }
                quantity_params.push(qty);
            }
            IndexValue::Reference {
                reference,
                resource_type: ref_type,
                resource_id: ref_id,
            } => {
                let mut ref_doc = json!({
                    "name": ev.param_name,
                    "reference": reference,
                });
                if let Some(rt) = ref_type {
                    ref_doc["resource_type"] = json!(rt);
                }
                if let Some(ri) = ref_id {
                    ref_doc["resource_id"] = json!(ri);
                }
                reference_params.push(ref_doc);
            }
            IndexValue::Uri(u) => {
                uri_params.push(json!({
                    "name": ev.param_name,
                    "value": u,
                }));
            }
        }

        if let Some(group) = ev.composite_group {
            composite_params.push(json!({
                "name": ev.param_name,
                "group_id": group,
            }));
        }
    }

    json!({
        "resource_type": resource_type,
        "resource_id": resource_id,
        "tenant_id": tenant_id,
        "version_id": version_id,
        "last_updated": Utc::now().to_rfc3339(),
        "fhir_version": fhir_version.as_mime_param(),
        "is_deleted": false,
        "content": content,
        "narrative_text": searchable.narrative,
        "content_text": searchable.full_content,
        "search_params": {
            "string": string_params,
            "token": token_params,
            "date": date_params,
            "number": number_params,
            "quantity": quantity_params,
            "reference": reference_params,
            "uri": uri_params,
            "composite": composite_params,
        }
    })
}

#[async_trait]
impl ResourceStorage for ElasticsearchBackend {
    fn backend_name(&self) -> &'static str {
        "elasticsearch"
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        let tenant_id = tenant.tenant_id().as_str();

        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let resource = normalize_resource_shape(resource_type, &id, resource);
        let version_id = requested_version(&resource).unwrap_or(1).to_string();

        // Extract search parameters
        let extracted_values = self
            .search_extractor()
            .extract(&resource, resource_type)
            .unwrap_or_default();

        // Build ES document
        let doc = build_es_document(
            tenant_id,
            resource_type,
            &id,
            &version_id,
            &resource,
            fhir_version,
            &extracted_values,
        );

        // Ensure index exists
        schema::ensure_index(self, tenant_id, resource_type).await?;

        // Index the document
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, &id);

        let response = self
            .client()
            .index(IndexParts::IndexId(&index, &doc_id))
            .body(doc)
            .send()
            .await
            .map_err(|e| internal_error(format!("Failed to index document: {}", e)))?;

        let status = response.status_code();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(internal_error(format!(
                "Failed to index document (status {}): {}",
                status, body
            )));
        }

        let now = Utc::now();
        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            &version_id,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        let tenant_id = tenant.tenant_id().as_str();
        let resource = normalize_resource_shape(resource_type, id, resource);
        let requested_version = requested_version(&resource);

        // Check if document exists
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        let existing = self
            .client()
            .get(GetParts::IndexId(&index, &doc_id))
            .send()
            .await;

        let mut existing_source = None;
        let existing_version = match existing {
            Ok(resp) if resp.status_code().is_success() => {
                let body = resp.json::<Value>().await.unwrap_or_default();
                existing_source = body.get("_source").cloned();
                existing_source.as_ref().and_then(source_version)
            }
            _ => None,
        };

        // Idempotency guard: if this is a replay or stale event, keep current state.
        if let (Some(existing), Some(requested)) = (existing_version, requested_version) {
            if requested <= existing {
                if let Some(ref source) = existing_source {
                    if let Some(stored) = parse_stored_resource(source, tenant)? {
                        return Ok((stored, false));
                    }
                }

                let now = Utc::now();
                return Ok((
                    StoredResource::from_storage(
                        resource_type,
                        id,
                        existing.to_string(),
                        tenant.tenant_id().clone(),
                        resource,
                        now,
                        now,
                        None,
                        fhir_version,
                    ),
                    false,
                ));
            }
        }

        let version_id = requested_version
            .or(existing_version.map(|v| v.saturating_add(1)))
            .unwrap_or(1)
            .to_string();
        let is_new = existing_version.is_none();

        // Extract search parameters
        let extracted_values = self
            .search_extractor()
            .extract(&resource, resource_type)
            .unwrap_or_default();

        let doc = build_es_document(
            tenant_id,
            resource_type,
            id,
            &version_id,
            &resource,
            fhir_version,
            &extracted_values,
        );

        // Ensure index exists
        schema::ensure_index(self, tenant_id, resource_type).await?;

        let response = self
            .client()
            .index(IndexParts::IndexId(&index, &doc_id))
            .body(doc)
            .send()
            .await
            .map_err(|e| internal_error(format!("Failed to index document: {}", e)))?;

        let status = response.status_code();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(internal_error(format!(
                "Failed to index document (status {}): {}",
                status, body
            )));
        }

        let now = Utc::now();
        Ok((
            StoredResource::from_storage(
                resource_type,
                id,
                &version_id,
                tenant.tenant_id().clone(),
                resource,
                now,
                now,
                None,
                fhir_version,
            ),
            is_new,
        ))
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        let response = self
            .client()
            .get(GetParts::IndexId(&index, &doc_id))
            .send()
            .await;

        let response = match response {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };

        if !response.status_code().is_success() {
            return Ok(None);
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| internal_error(format!("Failed to parse ES response: {}", e)))?;

        let source = match body.get("_source") {
            Some(s) => s,
            None => return Ok(None),
        };

        // Check if deleted
        if source
            .get("is_deleted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(None);
        }

        // Verify tenant
        let doc_tenant = source
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if doc_tenant != tenant_id {
            return Ok(None);
        }

        parse_stored_resource(source, tenant)
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = current.resource_type();
        let id = current.id();
        let new_version: u64 = current.version_id().parse::<u64>().unwrap_or(0) + 1;
        let version_id = new_version.to_string();
        let fhir_version = current.fhir_version();

        let resource = normalize_resource_shape(resource_type, id, resource);

        let extracted_values = self
            .search_extractor()
            .extract(&resource, resource_type)
            .unwrap_or_default();

        let doc = build_es_document(
            tenant_id,
            resource_type,
            id,
            &version_id,
            &resource,
            fhir_version,
            &extracted_values,
        );

        schema::ensure_index(self, tenant_id, resource_type).await?;

        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        let response = self
            .client()
            .index(IndexParts::IndexId(&index, &doc_id))
            .body(doc)
            .send()
            .await
            .map_err(|e| internal_error(format!("Failed to update document: {}", e)))?;

        let status = response.status_code();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(internal_error(format!(
                "Failed to update document (status {}): {}",
                status, body
            )));
        }

        let now = Utc::now();
        Ok(StoredResource::from_storage(
            resource_type,
            id,
            &version_id,
            tenant.tenant_id().clone(),
            resource,
            now,
            now,
            None,
            fhir_version,
        ))
    }

    async fn delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        let response = self
            .client()
            .delete(DeleteParts::IndexId(&index, &doc_id))
            .send()
            .await
            .map_err(|e| internal_error(format!("Failed to delete document: {}", e)))?;

        let status = response.status_code();
        if !status.is_success() {
            if status.as_u16() == 404 {
                // Idempotent delete semantics for sync replay safety.
                return Ok(());
            }
            let body = response.text().await.unwrap_or_default();
            return Err(internal_error(format!(
                "Failed to delete document (status {}): {}",
                status, body
            )));
        }

        Ok(())
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        let tenant_id = tenant.tenant_id().as_str();

        let index_pattern = match resource_type {
            Some(rt) => self.index_name(tenant_id, rt),
            None => format!("{}_*", self.tenant_index_prefix(tenant_id)),
        };

        let query = json!({
            "query": {
                "bool": {
                    "filter": [
                        { "term": { "tenant_id": tenant_id } },
                        { "term": { "is_deleted": false } }
                    ]
                }
            }
        });

        let response = self
            .client()
            .count(elasticsearch::CountParts::Index(&[&index_pattern]))
            .body(query)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status_code().is_success() => {
                let body: Value = resp.json().await.unwrap_or_default();
                Ok(body.get("count").and_then(|c| c.as_u64()).unwrap_or(0))
            }
            // If index doesn't exist, count is 0
            _ => Ok(0),
        }
    }
}

/// Parses a StoredResource from an ES `_source` document.
fn parse_stored_resource(
    source: &Value,
    tenant: &TenantContext,
) -> StorageResult<Option<StoredResource>> {
    let resource_type = source
        .get("resource_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| internal_error("Missing resource_type in ES document".to_string()))?;

    let resource_id = source
        .get("resource_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| internal_error("Missing resource_id in ES document".to_string()))?;

    let version_id = source
        .get("version_id")
        .and_then(|v| v.as_str())
        .unwrap_or("1");

    let content = source.get("content").cloned().unwrap_or_else(|| json!({}));

    let fhir_version_str = source
        .get("fhir_version")
        .and_then(|v| v.as_str())
        .unwrap_or("4.0");

    let fhir_version = FhirVersion::from_mime_param(fhir_version_str).unwrap_or_default();

    let last_updated = source
        .get("last_updated")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    Ok(Some(StoredResource::from_storage(
        resource_type,
        resource_id,
        version_id,
        tenant.tenant_id().clone(),
        content,
        last_updated,
        last_updated,
        None,
        fhir_version,
    )))
}
