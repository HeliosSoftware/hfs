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

use crate::core::{PurgableStorage, ResourceStorage};
use crate::error::{BackendError, ResourceError, StorageError, StorageResult};
use crate::search::converters::IndexValue;
use crate::search::extractor::ExtractedValue;
use crate::search::reindex::{ReindexSource, ReindexTarget, ResourcePage};
use crate::tenant::{Operation, TenantContext};
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

/// The cluster could not be reached (connection refused, DNS, TLS, timeout).
///
/// Distinct from [`internal_error`] because the caller must never confuse it
/// with "the resource is not there": we never got an answer at all.
fn unavailable_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Unavailable {
        backend_name: "elasticsearch".to_string(),
        message,
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

        for contained in self.tenant_extractor(tenant_id).extract_contained(resource) {
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

    async fn readiness_check(&self) -> Result<(), BackendError> {
        <Self as crate::core::Backend>::health_check(self).await
    }

    fn bulk_write_concurrency(&self) -> usize {
        // One index request per resource; ES absorbs parallel writers.
        8
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        tenant.check_permission(Operation::Create, resource_type)?;

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
            .tenant_extractor(tenant_id)
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

        // Deciding "this resource is new, start at version 1" requires knowing that
        // it does not already exist. Only a 404 establishes that. If the existence
        // check failed at the transport layer we must not guess "new" — doing so
        // would reset the version of a resource that does exist, silently clobbering
        // its history.
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
            Ok(resp) if resp.status_code().as_u16() == 404 => ("1".to_string(), true),
            Ok(resp) => {
                let status = resp.status_code().as_u16();
                let body = resp.text().await.unwrap_or_default();
                return Err(internal_error(format!(
                    "Failed to check existence of {resource_type}/{id} (status {status}): {body}"
                )));
            }
            Err(e) => {
                return Err(unavailable_error(format!(
                    "Elasticsearch unreachable while checking existence of {resource_type}/{id}: {e}"
                )));
            }
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
            .tenant_extractor(tenant_id)
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

        // `Ok(None)` means "this resource does not exist" — a factual claim about
        // the data. Only ES itself can license that claim, by answering 404. A
        // transport failure (cluster down, DNS, TLS, timeout) or a 5xx/401/403
        // means we never learned anything, and must surface as an error. Reporting
        // it as "not found" would make a down cluster indistinguishable from an
        // empty one, which is exactly the misleading result this contract forbids.
        let response = match response {
            Ok(r) => r,
            Err(e) => {
                return Err(unavailable_error(format!(
                    "Elasticsearch unreachable while reading {resource_type}/{id}: {e}"
                )));
            }
        };

        let status = response.status_code();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(internal_error(format!(
                "Failed to read {resource_type}/{id} (status {}): {body}",
                status.as_u16()
            )));
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
        let resource_type = current.resource_type();
        tenant.check_permission(Operation::Update, resource_type)?;

        let tenant_id = tenant.tenant_id().as_str();
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
            .tenant_extractor(tenant_id)
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
        tenant.check_permission(Operation::Delete, resource_type)?;

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

        // As in `read`: a count of 0 is a claim about the data. Make it only when
        // the cluster says so, or when the index genuinely does not exist (404).
        // An unreachable cluster must error, not silently report "zero resources".
        match response {
            Ok(resp) if resp.status_code().is_success() => {
                let body: Value = resp.json().await.unwrap_or_default();
                Ok(body.get("count").and_then(|c| c.as_u64()).unwrap_or(0))
            }
            // Index doesn't exist yet — legitimately zero.
            Ok(resp) if resp.status_code().as_u16() == 404 => Ok(0),
            Ok(resp) => {
                let status = resp.status_code().as_u16();
                let body = resp.text().await.unwrap_or_default();
                Err(internal_error(format!(
                    "Count failed (status {status}): {body}"
                )))
            }
            Err(e) => Err(unavailable_error(format!(
                "Elasticsearch unreachable during count: {e}"
            ))),
        }
    }

    // `supports_tenant_registry` stays `false`: ES is a search secondary, never
    // the registry of record. Only the data purge is implemented, so that
    // composite storage can clear a purged tenant's offloaded search documents
    // (in `*-elasticsearch` modes the primary's own search index is empty).
    async fn purge_tenant_data(&self, id: &str) -> StorageResult<u64> {
        crate::tenant::ensure_mutable_tenant(id)?;
        // Documents are matched by an exact `tenant_id` term, not by the index
        // pattern alone: index names lowercase the tenant id, so the pattern
        // for tenant `acme` would also sweep `acme_corp`'s indices.
        let pattern = format!("{}_{}_*", self.config().index_prefix, id.to_lowercase());
        let body = json!({
            "query": { "bool": { "filter": [
                { "term": { "tenant_id": id } }
            ]}}
        });
        // Missing indices are fine (nothing to delete).
        let response = self
            .client()
            .delete_by_query(DeleteByQueryParts::Index(&[&pattern]))
            .ignore_unavailable(true)
            .refresh(true)
            .body(body)
            .send()
            .await
            .map_err(|e| internal_error(format!("purge_tenant_data: {}", e)))?;
        let body: Value = response.json().await.unwrap_or_default();
        // Deleted-doc count (includes contained/tombstone docs) — informational;
        // the composite reports the primary's count to the admin API.
        Ok(body.get("deleted").and_then(|d| d.as_u64()).unwrap_or(0))
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

// ============================================================================
// PurgableStorage
//
// Elasticsearch keeps no history — a document is the current version and
// nothing else — so `purge` and `delete` collapse to the same operation here.
// `delete` is already a hard delete (see above), so purge reuses it.
//
// Elasticsearch is never a standalone backend; it is always a search secondary
// in front of a SQL, MongoDB, or S3 primary. Purging it is therefore always
// part of a composite fan-out, and losing a document here is recoverable by
// reindexing from the primary.
// ============================================================================

#[async_trait]
impl PurgableStorage for ElasticsearchBackend {
    async fn purge(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<()> {
        // ResourceStorage::delete already removes the document outright and
        // sweeps its contained-resource docs, which is exactly purge semantics
        // for a backend with no history.
        match ResourceStorage::delete(self, tenant, resource_type, id).await {
            // A purge whose document is already absent has nothing to undo. The
            // composite fan-out retries a failed purge, so this must be
            // idempotent or the retry would fail on the backend that succeeded.
            Err(StorageError::Resource(ResourceError::NotFound { .. })) => Ok(()),
            other => other,
        }
    }

    async fn purge_all(&self, tenant: &TenantContext, resource_type: &str) -> StorageResult<u64> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);

        let deleted = delete_by_query_scoped(
            self,
            &[&index],
            json!({ "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } }
            ]}}}),
        )
        .await?;

        // Contained-resource docs derived from this type live in *other* type
        // indices, so they need a second, tenant-scoped sweep.
        let pattern = tenant_index_pattern(self, tenant_id);
        delete_by_query_scoped(
            self,
            &[&pattern],
            json!({ "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } },
                { "term": { "is_contained": true } },
                { "term": { "container_type": resource_type } }
            ]}}}),
        )
        .await?;

        Ok(deleted)
    }
}

// ============================================================================
// ReindexSource / ReindexTarget
//
// Elasticsearch has no separate search-index structure: the indexed document
// *is* the search entry. That makes it a legitimate ReindexTarget but means the
// two write methods behave differently from a SQL backend's — see each below.
// ============================================================================

#[async_trait]
impl ReindexTarget for ElasticsearchBackend {
    /// A no-op, deliberately.
    ///
    /// For a SQL backend, search entries are rows that must be cleared before
    /// being rewritten or stale ones survive. For Elasticsearch the entries are
    /// *fields of the resource document*, and `write_search_entries` re-indexes
    /// that whole document under the same `_id`, which replaces it wholesale —
    /// no stale field can survive. Actually deleting here would remove the
    /// resource itself between the delete and the write, so the reindex would
    /// briefly (and, if it then failed, permanently) drop it from search.
    async fn delete_search_entries(
        &self,
        _tenant: &TenantContext,
        _resource_type: &str,
        _resource_id: &str,
    ) -> StorageResult<u64> {
        Ok(0)
    }

    async fn write_search_entries(
        &self,
        tenant: &TenantContext,
        resource: &StoredResource,
    ) -> StorageResult<usize> {
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = resource.resource_type();
        let resource_id = resource.id();
        let content = resource.content();
        let fhir_version = resource.fhir_version();

        let extracted_values = self
            .tenant_extractor(tenant_id)
            .extract(content, resource_type)
            .map_err(|e| internal_error(format!("Search parameter extraction failed: {e}")))?;

        // The document carries version_id and fhir_version, neither of which is
        // recoverable from the resource JSON — which is why ReindexTarget hands
        // over the whole StoredResource rather than just its content.
        let doc = build_es_document(
            tenant_id,
            resource_type,
            resource_id,
            resource.version_id(),
            content,
            fhir_version,
            &extracted_values,
        );

        schema::ensure_index(self, tenant_id, resource_type).await?;

        let index = self.index_name(tenant_id, resource_type);
        let doc_id = Self::document_id(resource_type, resource_id);

        let response = self
            .client()
            .index(IndexParts::IndexId(&index, &doc_id))
            .body(doc)
            .send()
            .await
            .map_err(|e| internal_error(format!("Failed to index document: {e}")))?;

        let status = response.status_code();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(internal_error(format!(
                "Failed to index document (status {status}): {body}"
            )));
        }

        self.index_contained_docs(
            tenant_id,
            resource_type,
            resource_id,
            content,
            fhir_version,
            resource.version_id(),
            true,
        )
        .await?;

        Ok(extracted_values.len())
    }

    async fn clear_search_index(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let tenant_id = tenant.tenant_id().as_str();

        // MUST be a delete-by-query with a `tenant_id` term filter, never a
        // delete of the indices matching the tenant's index pattern. The
        // pattern `{prefix}_{tenant}_*` is a prefix glob, so tenant "a" matches
        // tenant "ab"'s indices — deleting by pattern would destroy a
        // prefix-sharing tenant's data. The term filter is what actually bounds
        // this to one tenant; the pattern only narrows which indices to scan.
        let pattern = tenant_index_pattern(self, tenant_id);
        delete_by_query_scoped(
            self,
            &[&pattern],
            json!({ "query": { "bool": { "filter": [
                { "term": { "tenant_id": tenant_id } }
            ]}}}),
        )
        .await
    }
}

#[async_trait]
impl ReindexSource for ElasticsearchBackend {
    async fn list_resource_types(&self, tenant: &TenantContext) -> StorageResult<Vec<String>> {
        let tenant_id = tenant.tenant_id().as_str();
        let pattern = tenant_index_pattern(self, tenant_id);

        let response = self
            .client()
            .search(elasticsearch::SearchParts::Index(&[&pattern]))
            .body(json!({
                "size": 0,
                "query": { "bool": { "filter": [
                    { "term": { "tenant_id": tenant_id } },
                    { "term": { "is_deleted": false } }
                ]}},
                "aggs": { "types": { "terms": { "field": "resource_type", "size": 1000 } } }
            }))
            .allow_no_indices(true)
            .ignore_unavailable(true)
            .send()
            .await
            .map_err(|e| internal_error(format!("Failed to list resource types: {e}")))?;

        if !response.status_code().is_success() {
            return Ok(Vec::new());
        }

        let body: Value = response.json().await.unwrap_or_default();
        Ok(body
            .pointer("/aggregations/types/buckets")
            .and_then(|b| b.as_array())
            .map(|buckets| {
                buckets
                    .iter()
                    .filter_map(|b| b.get("key").and_then(|k| k.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn count_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        ResourceStorage::count(self, tenant, Some(resource_type)).await
    }

    async fn fetch_resources_page(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> StorageResult<ResourcePage> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);

        // `search_after` keyset pagination over a total sort order. Excludes
        // contained docs, which are synthetic and get rebuilt from their
        // container rather than being reindexed in their own right.
        let mut body = json!({
            "size": limit,
            "query": { "bool": {
                "filter": [
                    { "term": { "tenant_id": tenant_id } },
                    { "term": { "is_deleted": false } }
                ],
                "must_not": [ { "term": { "is_contained": true } } ]
            }},
            "sort": [
                { "last_updated": "asc" },
                { "resource_id": "asc" }
            ]
        });

        if let Some(raw) = cursor {
            let after: Value = serde_json::from_str(raw)
                .map_err(|e| internal_error(format!("Malformed reindex cursor: {e}")))?;
            body["search_after"] = after;
        }

        let response = self
            .client()
            .search(elasticsearch::SearchParts::Index(&[&index]))
            .body(body)
            .allow_no_indices(true)
            .ignore_unavailable(true)
            .send()
            .await
            .map_err(|e| internal_error(format!("Failed to fetch resources: {e}")))?;

        if !response.status_code().is_success() {
            return Ok(ResourcePage {
                resources: Vec::new(),
                next_cursor: None,
            });
        }

        let payload: Value = response.json().await.unwrap_or_default();
        let hits = payload
            .pointer("/hits/hits")
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default();

        let mut resources = Vec::new();
        for hit in &hits {
            if let Some(source) = hit.get("_source")
                && let Some(stored) = parse_stored_resource(source, tenant)?
            {
                resources.push(stored);
            }
        }

        // Only a full page can have a successor; the sort values of the last hit
        // become the next `search_after`.
        let next_cursor = match (hits.len() as u32 == limit, hits.last()) {
            (true, Some(last)) => last
                .get("sort")
                .map(|s| s.to_string())
                .filter(|_| !resources.is_empty()),
            _ => None,
        };

        Ok(ResourcePage {
            resources,
            next_cursor,
        })
    }
}

/// The index glob covering every one of a tenant's type indices.
///
/// This is a *prefix* glob: tenant "a" also matches tenant "ab"'s indices.
/// Every query built on it MUST also carry a `tenant_id` term filter — the
/// pattern narrows which indices are scanned, the filter is what enforces
/// tenant isolation.
fn tenant_index_pattern(backend: &ElasticsearchBackend, tenant_id: &str) -> String {
    format!(
        "{}_{}_*",
        backend.config().index_prefix,
        tenant_id.to_lowercase()
    )
}

/// Runs a delete-by-query and returns how many documents it removed.
///
/// Missing indices are not an error — a tenant that has never been written to
/// simply has nothing to delete.
async fn delete_by_query_scoped(
    backend: &ElasticsearchBackend,
    indices: &[&str],
    body: Value,
) -> StorageResult<u64> {
    let response = backend
        .client()
        .delete_by_query(DeleteByQueryParts::Index(indices))
        .ignore_unavailable(true)
        .refresh(true)
        .conflicts(elasticsearch::params::Conflicts::Proceed)
        .body(body)
        .send()
        .await
        .map_err(|e| internal_error(format!("Failed to delete by query: {e}")))?;

    if !response.status_code().is_success() {
        let status = response.status_code();
        let text = response.text().await.unwrap_or_default();
        return Err(internal_error(format!(
            "Failed to delete by query (status {status}): {text}"
        )));
    }

    let payload: Value = response.json().await.unwrap_or_default();
    Ok(payload.get("deleted").and_then(|d| d.as_u64()).unwrap_or(0))
}
