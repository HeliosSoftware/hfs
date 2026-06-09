//! ResourceStorage implementation for Elasticsearch.
//!
//! Provides the minimal CRUD operations needed for the SyncManager to propagate
//! changes from the primary backend. The ES backend is primarily a search secondary,
//! but it must implement ResourceStorage for sync support.

use async_trait::async_trait;
use chrono::Utc;
use elasticsearch::{DeleteByQueryParts, DeleteParts, GetParts, IndexParts};
use helios_fhir::FhirVersion;
use serde_json::{Value, json};

use crate::core::ResourceStorage;
use crate::error::{BackendError, ResourceError, StorageError, StorageResult};
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

/// Builds an ES document from a FHIR resource and its extracted search values.
/// Appends a value to a JSON array field on `obj`, creating the array if needed.
fn push_array_field(obj: &mut Value, key: &str, val: Value) {
    match obj.get_mut(key) {
        Some(Value::Array(arr)) => arr.push(val),
        _ => obj[key] = json!([val]),
    }
}

/// Merges one composite component's value into the composite instance object,
/// placing it in the array field matching the component's value type. All
/// components of one instance share a nested object, so a single nested query
/// can require every component to match within the same instance.
fn merge_composite_component(entry: &mut Value, value: &IndexValue) {
    match value {
        IndexValue::String(s) => push_array_field(entry, "string", json!(s)),
        IndexValue::Token { system, code, .. } => {
            push_array_field(entry, "token_code", json!(code));
            if let Some(sys) = system {
                push_array_field(entry, "token_system", json!(sys));
            }
        }
        IndexValue::Number(n) => push_array_field(entry, "number", json!(n)),
        IndexValue::Quantity {
            value,
            unit,
            system,
            ..
        } => {
            push_array_field(entry, "quantity_value", json!(value));
            if let Some(u) = unit {
                push_array_field(entry, "quantity_unit", json!(u));
            }
            if let Some(s) = system {
                push_array_field(entry, "quantity_system", json!(s));
            }
        }
        IndexValue::Date { value, .. } => push_array_field(entry, "date", json!(value)),
        IndexValue::Reference { reference, .. } => {
            push_array_field(entry, "reference", json!(reference))
        }
        IndexValue::Uri(u) => push_array_field(entry, "uri", json!(u)),
    }
}

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
    // Composite instances, keyed by (param name, group) so all components of one
    // instance land in a single nested object with inline (array) component values.
    let mut composite_groups: std::collections::BTreeMap<(String, u32), Value> =
        std::collections::BTreeMap::new();

    for ev in extracted_values {
        // Composite component values are accumulated into their instance object
        // rather than the per-type arrays.
        if let Some(group) = ev.composite_group {
            let entry = composite_groups
                .entry((ev.param_name.clone(), group))
                .or_insert_with(|| json!({ "name": ev.param_name, "group_id": group }));
            merge_composite_component(entry, &ev.value);
            continue;
        }

        match &ev.value {
            IndexValue::String(s) => {
                string_params.push(json!({
                    "name": ev.param_name,
                    "value": s,
                    "folded": crate::search::fold_text(s),
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
                // UCUM-canonical value/unit so quantity search matches equivalent
                // units (g ⇄ mg). Uses the code if present, else the unit display.
                if let Some((cv, cu)) = code
                    .as_deref()
                    .or(unit.as_deref())
                    .and_then(|u| helios_fhirpath::ucum::canonicalize_quantity(*value, u))
                {
                    qty["canonical_value"] = json!(cv);
                    qty["canonical_unit"] = json!(cu);
                }
                quantity_params.push(qty);
            }
            IndexValue::Reference {
                reference,
                resource_type: ref_type,
                resource_id: ref_id,
                display,
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
                if let Some(d) = display {
                    ref_doc["display"] = json!(d);
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
    }

    let composite_params: Vec<Value> = composite_groups.into_values().collect();

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

/// Synthetic ES `resource_id` for a contained-resource document: the
/// container's id plus the contained local id, so it is unique within the
/// contained type's index and stable across re-indexing.
pub(crate) fn contained_resource_id(container_id: &str, local_id: &str) -> String {
    format!("{}#{}", container_id, local_id)
}

/// Builds an ES document for a contained resource (`_contained` search). The
/// doc describes the contained resource (its `resource_type`, `content`, and
/// `search_params`) so it matches normally within that type's index, and is
/// tagged with the container's identity for result resolution.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_es_contained_document(
    tenant_id: &str,
    container_type: &str,
    container_id: &str,
    contained_type: &str,
    local_id: &str,
    contained_content: &Value,
    version_id: &str,
    fhir_version: FhirVersion,
    extracted_values: &[ExtractedValue],
) -> Value {
    let synthetic_id = contained_resource_id(container_id, local_id);
    let mut doc = build_es_document(
        tenant_id,
        contained_type,
        &synthetic_id,
        version_id,
        contained_content,
        fhir_version,
        extracted_values,
    );
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("is_contained".to_string(), json!(true));
        obj.insert("container_type".to_string(), json!(container_type));
        obj.insert("container_id".to_string(), json!(container_id));
        obj.insert("contained_local_id".to_string(), json!(local_id));
    }
    doc
}

impl ElasticsearchBackend {
    /// Deletes all contained-resource docs derived from the given container
    /// (across the tenant's indices), used before re-indexing or on container
    /// delete so removed contained resources don't linger.
    pub(crate) async fn delete_contained_docs(
        &self,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
    ) -> StorageResult<()> {
        let pattern = format!(
            "{}_{}_*",
            self.config().index_prefix,
            tenant_id.to_lowercase()
        );
        let body = json!({
            "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } },
                { "term": { "is_contained": true } },
                { "term": { "container_type": container_type } },
                { "term": { "container_id": container_id } }
            ]}}
        });
        // Missing indices are fine (nothing to delete).
        let _ = self
            .client()
            .delete_by_query(DeleteByQueryParts::Index(&[&pattern]))
            .ignore_unavailable(true)
            .refresh(true)
            .body(body)
            .send()
            .await
            .map_err(|e| internal_error(format!("Failed to delete contained docs: {}", e)))?;
        Ok(())
    }

    /// Extracts and indexes a container's `contained[]` resources as separate
    /// `is_contained` docs in their respective type indices. When `delete_first`
    /// is set, stale contained docs for this container are removed first
    /// (needed on update, where contained resources may have changed/been removed).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn index_contained_docs(
        &self,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
        resource: &Value,
        fhir_version: FhirVersion,
        version_id: &str,
        delete_first: bool,
    ) -> StorageResult<()> {
        if delete_first {
            self.delete_contained_docs(tenant_id, container_type, container_id)
                .await?;
        }

        for contained in self.search_extractor().extract_contained(resource) {
            let doc = build_es_contained_document(
                tenant_id,
                container_type,
                container_id,
                &contained.contained_type,
                &contained.local_id,
                &contained.content,
                version_id,
                fhir_version,
                &contained.values,
            );
            schema::ensure_index(self, tenant_id, &contained.contained_type).await?;
            let index = self.index_name(tenant_id, &contained.contained_type);
            let doc_id = Self::document_id(
                &contained.contained_type,
                &contained_resource_id(container_id, &contained.local_id),
            );
            let response = self
                .client()
                .index(IndexParts::IndexId(&index, &doc_id))
                .body(doc)
                .send()
                .await
                .map_err(|e| internal_error(format!("Failed to index contained doc: {}", e)))?;
            if !response.status_code().is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(internal_error(format!(
                    "Failed to index contained doc: {}",
                    body
                )));
            }
        }
        Ok(())
    }
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

        let version_id = "1";

        // Ensure the resource has correct type and id
        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.clone()));
        }

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
            version_id,
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

        // Index any contained resources for `_contained` search. New resource,
        // so no stale docs to delete first.
        if resource
            .get("contained")
            .and_then(|c| c.as_array())
            .is_some_and(|a| !a.is_empty())
        {
            self.index_contained_docs(
                tenant_id,
                resource_type,
                &id,
                &resource,
                fhir_version,
                version_id,
                false,
            )
            .await?;
        }

        let now = Utc::now();
        Ok(StoredResource::from_storage(
            resource_type,
            &id,
            version_id,
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

        // Check if document exists
        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, id);

        let existing = self
            .client()
            .get(GetParts::IndexId(&index, &doc_id))
            .send()
            .await;

        let (version_id, is_new) = match existing {
            Ok(resp) if resp.status_code().is_success() => {
                let body = resp.json::<Value>().await.unwrap_or_default();
                let current_version: u64 = body
                    .get("_source")
                    .and_then(|s| s.get("version_id"))
                    .and_then(|v| v.as_str())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                ((current_version + 1).to_string(), false)
            }
            _ => ("1".to_string(), true),
        };

        // Ensure resource has correct type and id
        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.to_string()));
        }

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

        // Re-sync contained docs (delete stale, then re-index) so updates that
        // add/remove/change `contained[]` entries are reflected.
        self.index_contained_docs(
            tenant_id,
            resource_type,
            id,
            &resource,
            fhir_version,
            &version_id,
            true,
        )
        .await?;

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

        let mut resource = resource;
        if let Some(obj) = resource.as_object_mut() {
            obj.insert(
                "resourceType".to_string(),
                Value::String(resource_type.to_string()),
            );
            obj.insert("id".to_string(), Value::String(id.to_string()));
        }

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
                return Err(StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                }));
            }
            let body = response.text().await.unwrap_or_default();
            return Err(internal_error(format!(
                "Failed to delete document (status {}): {}",
                status, body
            )));
        }

        // Remove any contained-resource docs derived from this container.
        self.delete_contained_docs(tenant_id, resource_type, id)
            .await?;

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
            None => format!(
                "{}_{}_*",
                self.config().index_prefix,
                tenant_id.to_lowercase()
            ),
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

    let fhir_version = FhirVersion::from_mime_param(fhir_version_str)
        .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

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
