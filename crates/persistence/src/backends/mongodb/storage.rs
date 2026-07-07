//! ResourceStorage implementation for MongoDB.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use mongodb::{
    ClientSession, Cursor, SessionCursor,
    bson::{self, Bson, DateTime as BsonDateTime, Document, doc},
    error::Error as MongoError,
};
use serde_json::Value;

use crate::core::{
    BundleEntry, BundleEntryResult, BundleMethod, BundleProvider, BundleResult, BundleType,
    HistoryEntry, HistoryMethod, HistoryPage, HistoryParams, InstanceHistoryProvider,
    ResourceStorage, SystemHistoryProvider, TypeHistoryProvider, VersionedStorage, normalize_etag,
};
use crate::error::{
    BackendError, ConcurrencyError, ResourceError, StorageError, StorageResult, TransactionError,
};
use crate::search::converters::IndexValue;
use crate::search::extractor::ExtractedValue;
use crate::search::{SearchParameterLoader, SearchParameterStatus};
use crate::tenant::TenantContext;
use crate::types::{CursorValue, Page, PageCursor, PageInfo, StoredResource};

use super::MongoBackend;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message,
        source: None,
    })
}

#[derive(Debug, Clone)]
enum PendingSearchParameterChange {
    Create(Value),
    Update { old: Value, new: Value },
    Delete(Value),
}

fn serialization_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::SerializationError { message })
}

fn is_duplicate_key_error(err: &MongoError) -> bool {
    err.to_string().contains("E11000")
}

fn ensure_resource_identity(resource_type: &str, id: &str, resource: &mut Value) {
    if let Some(obj) = resource.as_object_mut() {
        obj.insert(
            "resourceType".to_string(),
            Value::String(resource_type.to_string()),
        );
        obj.insert("id".to_string(), Value::String(id.to_string()));
    }
}

fn value_to_document(value: &Value) -> StorageResult<Document> {
    let bson = bson::to_bson(value)
        .map_err(|e| serialization_error(format!("Failed to serialize resource: {}", e)))?;
    match bson {
        Bson::Document(doc) => Ok(doc),
        _ => Err(serialization_error(
            "Resource payload must serialize to a BSON document".to_string(),
        )),
    }
}

fn document_to_value(doc: &Document) -> StorageResult<Value> {
    bson::from_bson::<Value>(Bson::Document(doc.clone()))
        .map_err(|e| serialization_error(format!("Failed to deserialize resource: {}", e)))
}

fn bson_to_chrono(dt: &BsonDateTime) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap_or_else(Utc::now)
}

fn chrono_to_bson(dt: DateTime<Utc>) -> BsonDateTime {
    BsonDateTime::from_millis(dt.timestamp_millis())
}

fn normalize_date_for_mongo(value: &str) -> Option<DateTime<Utc>> {
    let normalized = if value.contains('T') {
        if value.contains('Z') || value.contains('+') || value.matches('-').count() > 2 {
            value.to_string()
        } else {
            format!("{}+00:00", value)
        }
    } else if value.len() == 10 {
        format!("{}T00:00:00+00:00", value)
    } else if value.len() == 7 {
        format!("{}-01T00:00:00+00:00", value)
    } else if value.len() == 4 {
        format!("{}-01-01T00:00:00+00:00", value)
    } else {
        value.to_string()
    };

    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn next_version(version: &str) -> StorageResult<String> {
    let parsed = version
        .parse::<u64>()
        .map_err(|e| serialization_error(format!("Invalid version value '{}': {}", version, e)))?;
    Ok((parsed + 1).to_string())
}

fn extract_deleted_at(doc: &Document) -> Option<DateTime<Utc>> {
    match doc.get("deleted_at") {
        Some(Bson::DateTime(dt)) => Some(bson_to_chrono(dt)),
        _ => None,
    }
}

fn extract_created_at(doc: &Document, fallback: DateTime<Utc>) -> DateTime<Utc> {
    doc.get_datetime("created_at")
        .map(bson_to_chrono)
        .unwrap_or(fallback)
}

fn extract_last_updated(doc: &Document, fallback: DateTime<Utc>) -> DateTime<Utc> {
    doc.get_datetime("last_updated")
        .map(bson_to_chrono)
        .unwrap_or(fallback)
}

fn extract_fhir_version(doc: &Document, fallback: FhirVersion) -> FhirVersion {
    doc.get_str("fhir_version")
        .ok()
        .and_then(FhirVersion::from_storage)
        .unwrap_or(fallback)
}

fn parse_version_id(version_id: &str) -> i64 {
    version_id.parse::<i64>().unwrap_or(0)
}

fn history_method_for(version_id: &str, is_deleted: bool) -> HistoryMethod {
    if is_deleted {
        HistoryMethod::Delete
    } else if version_id == "1" {
        HistoryMethod::Post
    } else {
        HistoryMethod::Put
    }
}

fn apply_history_params_filter(filter: &mut Document, params: &HistoryParams) {
    if !params.include_deleted {
        filter.insert("is_deleted", false);
    }

    let mut last_updated = Document::new();
    if let Some(since) = params.since {
        last_updated.insert("$gte", chrono_to_bson(since));
    }
    if let Some(before) = params.before {
        last_updated.insert("$lt", chrono_to_bson(before));
    }

    if !last_updated.is_empty() {
        filter.insert("last_updated", Bson::Document(last_updated));
    }
}

async fn collect_documents(mut cursor: Cursor<Document>) -> StorageResult<Vec<Document>> {
    let mut docs = Vec::new();
    while cursor
        .advance()
        .await
        .map_err(|e| internal_error(format!("Failed to advance MongoDB cursor: {}", e)))?
    {
        let doc = cursor.deserialize_current().map_err(|e| {
            internal_error(format!("Failed to deserialize MongoDB document: {}", e))
        })?;
        docs.push(doc);
    }
    Ok(docs)
}

async fn collect_session_documents(
    mut cursor: SessionCursor<Document>,
    session: &mut ClientSession,
) -> StorageResult<Vec<Document>> {
    let mut docs = Vec::new();
    while cursor
        .advance(session)
        .await
        .map_err(|e| internal_error(format!("Failed to advance MongoDB session cursor: {}", e)))?
    {
        let doc = cursor.deserialize_current().map_err(|e| {
            internal_error(format!(
                "Failed to deserialize MongoDB session document: {}",
                e
            ))
        })?;
        docs.push(doc);
    }
    Ok(docs)
}

fn parse_cursor_version(params: &HistoryParams) -> Option<i64> {
    let cursor = params.pagination.cursor_value()?;
    let value = cursor.sort_values().first()?;
    match value {
        CursorValue::String(version) => version.parse::<i64>().ok(),
        CursorValue::Number(version) => Some(*version),
        _ => None,
    }
}

fn parse_cursor_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_type_history_cursor(params: &HistoryParams) -> Option<(DateTime<Utc>, String)> {
    let cursor = params.pagination.cursor_value()?;
    let sort_values = cursor.sort_values();
    if sort_values.len() < 2 {
        return None;
    }

    let timestamp = match sort_values.first()? {
        CursorValue::String(value) => parse_cursor_timestamp(value)?,
        _ => return None,
    };

    let id = match sort_values.get(1)? {
        CursorValue::String(value) => value.clone(),
        _ => return None,
    };

    Some((timestamp, id))
}

fn parse_system_history_cursor(params: &HistoryParams) -> Option<(DateTime<Utc>, String, String)> {
    let cursor = params.pagination.cursor_value()?;
    let sort_values = cursor.sort_values();
    if sort_values.len() < 3 {
        return None;
    }

    let timestamp = match sort_values.first()? {
        CursorValue::String(value) => parse_cursor_timestamp(value)?,
        _ => return None,
    };

    let resource_type = match sort_values.get(1)? {
        CursorValue::String(value) => value.clone(),
        _ => return None,
    };

    let id = match sort_values.get(2)? {
        CursorValue::String(value) => value.clone(),
        _ => return None,
    };

    Some((timestamp, resource_type, id))
}

#[derive(Debug, Clone)]
struct ParsedHistoryRow {
    resource_type: String,
    id: String,
    version_id: String,
    content: Value,
    last_updated: DateTime<Utc>,
    is_deleted: bool,
    deleted_at: Option<DateTime<Utc>>,
    fhir_version: FhirVersion,
}

impl ParsedHistoryRow {
    fn into_stored_resource(self, tenant: &TenantContext) -> StoredResource {
        StoredResource::from_storage(
            &self.resource_type,
            &self.id,
            &self.version_id,
            tenant.tenant_id().clone(),
            self.content,
            self.last_updated,
            self.last_updated,
            self.deleted_at,
            self.fhir_version,
        )
    }

    fn into_history_entry(self, tenant: &TenantContext) -> HistoryEntry {
        let method = history_method_for(&self.version_id, self.is_deleted);
        let timestamp = self.last_updated;
        let resource = self.into_stored_resource(tenant);

        HistoryEntry {
            resource,
            method,
            timestamp,
        }
    }
}

fn parse_history_row(
    doc: &Document,
    fallback_resource_type: Option<&str>,
    fallback_id: Option<&str>,
) -> StorageResult<ParsedHistoryRow> {
    let resource_type = doc
        .get_str("resource_type")
        .ok()
        .map(str::to_string)
        .or_else(|| fallback_resource_type.map(str::to_string))
        .ok_or_else(|| internal_error("Missing resource_type in history document".to_string()))?;

    let id = doc
        .get_str("id")
        .ok()
        .map(str::to_string)
        .or_else(|| fallback_id.map(str::to_string))
        .ok_or_else(|| internal_error("Missing id in history document".to_string()))?;

    let version_id = doc
        .get_str("version_id")
        .map_err(|e| internal_error(format!("Missing history version_id: {}", e)))?
        .to_string();

    let payload = doc
        .get_document("data")
        .map_err(|e| internal_error(format!("Missing history payload: {}", e)))?;
    let content = document_to_value(payload)?;

    let now = Utc::now();
    let last_updated = extract_last_updated(doc, now);
    let is_deleted = doc.get_bool("is_deleted").unwrap_or(false);
    let deleted_at = extract_deleted_at(doc).or(if is_deleted { Some(last_updated) } else { None });
    let fhir_version = extract_fhir_version(doc, FhirVersion::default_enabled());

    Ok(ParsedHistoryRow {
        resource_type,
        id,
        version_id,
        content,
        last_updated,
        is_deleted,
        deleted_at,
        fhir_version,
    })
}

fn parse_simple_bundle_search_params(params: &str) -> Vec<(String, String)> {
    params
        .split('&')
        .filter_map(|pair| {
            let mut iter = pair.splitn(2, '=');
            let key = iter.next()?.trim();
            let value = iter.next()?.trim();

            if key.is_empty() || value.is_empty() {
                return None;
            }

            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn document_to_stored_resource(
    doc: &Document,
    tenant: &TenantContext,
    fallback_resource_type: &str,
) -> StorageResult<StoredResource> {
    let resource_type = doc
        .get_str("resource_type")
        .ok()
        .unwrap_or(fallback_resource_type)
        .to_string();

    let id = doc
        .get_str("id")
        .map_err(|e| internal_error(format!("Missing resource id in MongoDB document: {}", e)))?
        .to_string();

    let version_id = doc
        .get_str("version_id")
        .map_err(|e| internal_error(format!("Missing version_id in MongoDB document: {}", e)))?
        .to_string();

    let payload = doc.get_document("data").map_err(|e| {
        internal_error(format!(
            "Missing resource payload in MongoDB document: {}",
            e
        ))
    })?;
    let content = document_to_value(payload)?;

    let now = Utc::now();
    let created_at = extract_created_at(doc, now);
    let last_updated = extract_last_updated(doc, now);
    let deleted_at = extract_deleted_at(doc);
    let fhir_version = extract_fhir_version(doc, FhirVersion::default_enabled());

    Ok(StoredResource::from_storage(
        resource_type,
        id,
        version_id,
        tenant.tenant_id().clone(),
        content,
        created_at,
        last_updated,
        deleted_at,
        fhir_version,
    ))
}

async fn begin_required_bundle_transaction_session(
    db: &mongodb::Database,
) -> Result<ClientSession, TransactionError> {
    let mut session =
        db.client()
            .start_session()
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Failed to start MongoDB session: {}", e),
            })?;

    let hello = db.run_command(doc! { "hello": 1_i32 }).await.map_err(|e| {
        TransactionError::RolledBack {
            reason: format!("Failed to inspect MongoDB topology: {}", e),
        }
    })?;

    let supports_transactions = hello.contains_key("setName")
        || hello
            .get_str("msg")
            .map(|value| value == "isdbgrid")
            .unwrap_or(false);

    if !supports_transactions {
        return Err(TransactionError::UnsupportedIsolationLevel {
            level: "transaction bundles for mongodb require replica-set or sharded topology"
                .to_string(),
        });
    }

    session
        .start_transaction()
        .await
        .map_err(|e| TransactionError::RolledBack {
            reason: format!("Failed to start MongoDB transaction: {}", e),
        })?;

    Ok(session)
}

async fn begin_best_effort_multi_write_session(
    db: &mongodb::Database,
) -> (Option<ClientSession>, bool) {
    let mut session = db.client().start_session().await.ok();
    let mut transaction_active = false;

    if let Some(active_session) = session.as_mut() {
        // Transactions are only supported on replica sets and sharded deployments.
        // Fall back to non-transactional writes for standalone servers.
        let hello = db.run_command(doc! { "hello": 1_i32 }).await.ok();
        let supports_transactions = hello.as_ref().is_some_and(|doc| {
            doc.contains_key("setName")
                || doc
                    .get_str("msg")
                    .map(|value| value == "isdbgrid")
                    .unwrap_or(false)
        });

        if supports_transactions && active_session.start_transaction().await.is_ok() {
            transaction_active = true;
        } else {
            // Fallback to non-transactional writes on deployments that don't support transactions.
            session = None;
        }
    }

    (session, transaction_active)
}

async fn commit_best_effort_multi_write_session(
    session: &mut Option<ClientSession>,
    transaction_active: bool,
    operation: &str,
) -> StorageResult<()> {
    if !transaction_active {
        return Ok(());
    }

    if let Some(active_session) = session.as_mut() {
        active_session.commit_transaction().await.map_err(|e| {
            internal_error(format!(
                "Failed to commit MongoDB transaction after {}: {}",
                operation, e
            ))
        })?;
    }

    Ok(())
}

#[async_trait]
impl ResourceStorage for MongoBackend {
    fn backend_name(&self) -> &'static str {
        "mongodb"
    }

    fn is_cluster_shared(&self) -> bool {
        true
    }

    fn sof_runner(&self) -> Option<std::sync::Arc<dyn crate::core::sof_runner::SofRunner>> {
        use crate::sof::mongodb::MongoInDbRunner;
        // Native in-DB runner: compiles the ViewDefinition to an aggregation
        // pipeline executed against the `resources` collection.
        Some(std::sync::Arc::new(MongoInDbRunner::new(
            self.client_cell(),
            self.config().clone(),
        )))
    }

    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let (mut session, transaction_active) = begin_best_effort_multi_write_session(&db).await;
        let tenant_id = tenant.tenant_id().as_str();

        // Extract or generate ID
        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Check if resource already exists (including deleted resources).
        let identity_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": &id,
        };

        let existing = if let Some(active_session) = session.as_mut() {
            resources
                .find_one(identity_filter.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to check existence (session): {}", e))
                })?
        } else {
            resources
                .find_one(identity_filter)
                .await
                .map_err(|e| internal_error(format!("Failed to check existence: {}", e)))?
        };

        if existing.is_some() {
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.to_string(),
                id,
            }));
        }

        let mut resource = resource;
        ensure_resource_identity(resource_type, &id, &mut resource);

        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let version_id = "1".to_string();
        let fhir_version_str = fhir_version.as_mime_param().to_string();

        let resource_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": &id,
            "version_id": &version_id,
            "data": Bson::Document(payload.clone()),
            "created_at": now_bson,
            "last_updated": now_bson,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": &fhir_version_str,
        };

        if let Some(active_session) = session.as_mut() {
            resources
                .insert_one(resource_doc.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    if is_duplicate_key_error(&e) {
                        StorageError::Resource(ResourceError::AlreadyExists {
                            resource_type: resource_type.to_string(),
                            id: id.clone(),
                        })
                    } else {
                        internal_error(format!("Failed to insert resource (session): {}", e))
                    }
                })?;
        } else {
            resources.insert_one(resource_doc).await.map_err(|e| {
                if is_duplicate_key_error(&e) {
                    StorageError::Resource(ResourceError::AlreadyExists {
                        resource_type: resource_type.to_string(),
                        id: id.clone(),
                    })
                } else {
                    internal_error(format!("Failed to insert resource: {}", e))
                }
            })?;
        }

        let history_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": &id,
            "version_id": &version_id,
            "data": Bson::Document(payload),
            "created_at": now_bson,
            "last_updated": now_bson,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": fhir_version_str,
        };

        if let Some(active_session) = session.as_mut() {
            history
                .insert_one(history_doc)
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!(
                        "Failed to insert resource history (session): {}",
                        e
                    ))
                })?;
        } else {
            history
                .insert_one(history_doc)
                .await
                .map_err(|e| internal_error(format!("Failed to insert resource history: {}", e)))?;
        }

        self.index_resource(&db, tenant_id, resource_type, &id, &resource, &mut session)
            .await?;

        if resource_type == "SearchParameter" {
            self.handle_search_parameter_create(&resource)?;
        }

        commit_best_effort_multi_write_session(&mut session, transaction_active, "create").await?;

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
        let existing = self.read(tenant, resource_type, id).await?;

        if let Some(current) = existing {
            let updated = self.update(tenant, &current, resource).await?;
            Ok((updated, false))
        } else {
            let mut resource = resource;
            if let Some(obj) = resource.as_object_mut() {
                obj.insert("id".to_string(), Value::String(id.to_string()));
            }
            let created = self
                .create(tenant, resource_type, resource, fhir_version)
                .await?;
            Ok((created, true))
        }
    }

    async fn read(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let maybe_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to read resource: {}", e)))?;

        let Some(doc) = maybe_doc else {
            return Ok(None);
        };

        let is_deleted = doc.get_bool("is_deleted").unwrap_or(false);
        if is_deleted {
            return Err(StorageError::Resource(ResourceError::Gone {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
                deleted_at: extract_deleted_at(&doc),
            }));
        }

        let version_id = doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing version_id: {}", e)))?
            .to_string();

        let payload = doc
            .get_document("data")
            .map_err(|e| internal_error(format!("Missing resource payload: {}", e)))?;
        let content = document_to_value(payload)?;

        let now = Utc::now();
        let created_at = extract_created_at(&doc, now);
        let last_updated = extract_last_updated(&doc, now);
        let fhir_version = extract_fhir_version(&doc, FhirVersion::default_enabled());

        Ok(Some(StoredResource::from_storage(
            resource_type,
            id,
            version_id,
            tenant.tenant_id().clone(),
            content,
            created_at,
            last_updated,
            None,
            fhir_version,
        )))
    }

    async fn update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let (mut session, transaction_active) = begin_best_effort_multi_write_session(&db).await;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = current.resource_type();
        let id = current.id();

        let current_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "is_deleted": false,
        };

        let maybe_existing = if let Some(active_session) = session.as_mut() {
            resources
                .find_one(current_filter.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to load current resource (session): {}", e))
                })?
        } else {
            resources
                .find_one(current_filter)
                .await
                .map_err(|e| internal_error(format!("Failed to load current resource: {}", e)))?
        };

        let Some(existing_doc) = maybe_existing else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        let actual_version = existing_doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing current version: {}", e)))?
            .to_string();

        if actual_version != current.version_id() {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version,
                },
            ));
        }

        let new_version = next_version(current.version_id())?;

        let mut resource = resource;
        ensure_resource_identity(resource_type, id, &mut resource);
        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let fhir_version = current.fhir_version();
        let fhir_version_str = fhir_version.as_mime_param().to_string();

        let update_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": current.version_id(),
            "is_deleted": false,
        };
        let update_doc = doc! {
            "$set": {
                "version_id": &new_version,
                "data": Bson::Document(payload.clone()),
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": &fhir_version_str,
            }
        };

        let update_result = if let Some(active_session) = session.as_mut() {
            resources
                .update_one(update_filter.clone(), update_doc.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to update resource (session): {}", e))
                })?
        } else {
            resources
                .update_one(update_filter, update_doc)
                .await
                .map_err(|e| internal_error(format!("Failed to update resource: {}", e)))?
        };

        if update_result.matched_count == 0 {
            let latest = resources
                .find_one(doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "id": id,
                })
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to reload version conflict state: {}", e))
                })?;

            let actual = latest
                .as_ref()
                .and_then(|d| d.get_str("version_id").ok())
                .unwrap_or("unknown")
                .to_string();

            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version: actual,
                },
            ));
        }

        let created_at = extract_created_at(&existing_doc, now);

        let history_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": &new_version,
            "data": Bson::Document(payload),
            "created_at": chrono_to_bson(created_at),
            "last_updated": now_bson,
            "is_deleted": false,
            "deleted_at": Bson::Null,
            "fhir_version": fhir_version_str,
        };

        if let Some(active_session) = session.as_mut() {
            history
                .insert_one(history_doc)
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!(
                        "Failed to insert updated history row (session): {}",
                        e
                    ))
                })?;
        } else {
            history.insert_one(history_doc).await.map_err(|e| {
                internal_error(format!("Failed to insert updated history row: {}", e))
            })?;
        }

        self.index_resource(&db, tenant_id, resource_type, id, &resource, &mut session)
            .await?;

        if resource_type == "SearchParameter" {
            self.handle_search_parameter_update(current.content(), &resource)?;
        }

        commit_best_effort_multi_write_session(&mut session, transaction_active, "update").await?;

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version,
            tenant.tenant_id().clone(),
            resource,
            created_at,
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
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let (mut session, transaction_active) = begin_best_effort_multi_write_session(&db).await;
        let tenant_id = tenant.tenant_id().as_str();

        let delete_lookup_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "is_deleted": false,
        };

        let maybe_existing = if let Some(active_session) = session.as_mut() {
            resources
                .find_one(delete_lookup_filter.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!(
                        "Failed to check resource before delete (session): {}",
                        e
                    ))
                })?
        } else {
            resources
                .find_one(delete_lookup_filter)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to check resource before delete: {}", e))
                })?
        };

        let Some(existing_doc) = maybe_existing else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        let current_version = existing_doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing current version: {}", e)))?
            .to_string();
        let new_version = next_version(&current_version)?;

        let payload = existing_doc
            .get_document("data")
            .map_err(|e| internal_error(format!("Missing resource payload: {}", e)))?
            .clone();
        let resource_value = document_to_value(&payload)?;
        let fhir_version = existing_doc
            .get_str("fhir_version")
            .unwrap_or("4.0")
            .to_string();
        let created_at = extract_created_at(&existing_doc, Utc::now());

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);

        let delete_update_filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": &current_version,
            "is_deleted": false,
        };
        let delete_update_doc = doc! {
            "$set": {
                "version_id": &new_version,
                "is_deleted": true,
                "deleted_at": now_bson,
                "last_updated": now_bson,
            }
        };

        let update_result = if let Some(active_session) = session.as_mut() {
            resources
                .update_one(delete_update_filter.clone(), delete_update_doc.clone())
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to soft-delete resource (session): {}", e))
                })?
        } else {
            resources
                .update_one(delete_update_filter, delete_update_doc)
                .await
                .map_err(|e| internal_error(format!("Failed to soft-delete resource: {}", e)))?
        };

        if update_result.matched_count == 0 {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        let history_doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
            "version_id": &new_version,
            "data": Bson::Document(payload),
            "created_at": chrono_to_bson(created_at),
            "last_updated": now_bson,
            "is_deleted": true,
            "deleted_at": now_bson,
            "fhir_version": fhir_version,
        };

        if let Some(active_session) = session.as_mut() {
            history
                .insert_one(history_doc)
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!(
                        "Failed to insert deletion history row (session): {}",
                        e
                    ))
                })?;
        } else {
            history.insert_one(history_doc).await.map_err(|e| {
                internal_error(format!("Failed to insert deletion history row: {}", e))
            })?;
        }

        self.delete_search_index(&db, tenant_id, resource_type, id, &mut session)
            .await?;

        if resource_type == "SearchParameter" {
            self.handle_search_parameter_delete(&resource_value)?;
        }

        commit_best_effort_multi_write_session(&mut session, transaction_active, "delete").await?;

        Ok(())
    }

    async fn exists(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<bool> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let count = resources
            .count_documents(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to check resource existence: {}", e)))?;

        Ok(count > 0)
    }

    async fn read_batch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        ids: &[&str],
    ) -> StorageResult<Vec<StoredResource>> {
        let mut resources = Vec::with_capacity(ids.len());

        for id in ids {
            if let Some(resource) = self.read(tenant, resource_type, id).await? {
                resources.push(resource);
            }
        }

        Ok(resources)
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        resource_type: Option<&str>,
    ) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "is_deleted": false,
        };

        if let Some(resource_type) = resource_type {
            filter.insert("resource_type", resource_type);
        }

        resources
            .count_documents(filter)
            .await
            .map_err(|e| internal_error(format!("Failed to count resources: {}", e)))
    }

    async fn count_by_day(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<crate::core::DailyResourceCount>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let since_bson = BsonDateTime::from_millis(since.timestamp_millis());

        // Bucket on the server: `$match` narrows to this tenant/type/window
        // (covered by the `(tenant_id, last_updated)` index), then `$group`
        // collapses to one document per UTC calendar day. `$dateToString` with an
        // explicit UTC timezone keeps day boundaries consistent with the SQL
        // backends.
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "is_deleted": false,
                "last_updated": { "$gte": since_bson },
            }},
            doc! { "$group": {
                "_id": { "$dateToString": {
                    "format": "%Y-%m-%d",
                    "date": "$last_updated",
                    "timezone": "UTC",
                }},
                "n": { "$sum": 1 },
            }},
            doc! { "$sort": { "_id": 1 } },
        ];

        let mut cursor = resources
            .aggregate(pipeline)
            .await
            .map_err(|e| internal_error(format!("Failed to aggregate count_by_day: {}", e)))?;

        let mut out = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| internal_error(format!("count_by_day cursor advance: {}", e)))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| internal_error(format!("count_by_day cursor deserialize: {}", e)))?;
            let day_str = doc.get_str("_id").unwrap_or_default();
            // `$sum: 1` yields an int32 unless it overflows into int64.
            let n = doc
                .get_i32("n")
                .map(i64::from)
                .or_else(|_| doc.get_i64("n"))
                .unwrap_or(0);
            if let Ok(day) = chrono::NaiveDate::parse_from_str(day_str, "%Y-%m-%d") {
                out.push(crate::core::DailyResourceCount {
                    day,
                    count: n.max(0) as u64,
                });
            }
        }
        Ok(out)
    }

    async fn activity_histogram(
        &self,
        tenant: &TenantContext,
        since: DateTime<Utc>,
    ) -> StorageResult<Vec<crate::core::ActivityCell>> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let since_bson = BsonDateTime::from_millis(since.timestamp_millis());

        // `$dayOfWeek` returns 1=Sunday..7=Saturday and `$hour` 0..23, both in the
        // requested timezone. We subtract 1 from the weekday below to land on the
        // 0=Sunday..6=Saturday convention shared with the SQL backends.
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "last_updated": { "$gte": since_bson },
            }},
            doc! { "$group": {
                "_id": {
                    "wd": { "$dayOfWeek": { "date": "$last_updated", "timezone": "UTC" } },
                    "hr": { "$hour": { "date": "$last_updated", "timezone": "UTC" } },
                },
                "n": { "$sum": 1 },
            }},
        ];

        let mut cursor = history.aggregate(pipeline).await.map_err(|e| {
            internal_error(format!("Failed to aggregate activity histogram: {}", e))
        })?;

        let mut out = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| internal_error(format!("activity cursor advance: {}", e)))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|e| internal_error(format!("activity cursor deserialize: {}", e)))?;
            let id = match doc.get_document("_id") {
                Ok(id) => id,
                Err(_) => continue,
            };
            let wd = id.get_i32("wd").unwrap_or(1); // 1=Sunday..7=Saturday
            let hr = id.get_i32("hr").unwrap_or(0);
            let n = doc
                .get_i32("n")
                .map(i64::from)
                .or_else(|_| doc.get_i64("n"))
                .unwrap_or(0);
            out.push(crate::core::ActivityCell {
                weekday: (wd - 1).clamp(0, 6) as u8,
                hour: hr.clamp(0, 23) as u8,
                count: n.max(0) as u64,
            });
        }
        Ok(out)
    }

    async fn count_all_types(&self, tenant: &TenantContext) -> StorageResult<Vec<(String, u64)>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let pipeline = vec![
            doc! { "$match": { "tenant_id": tenant_id, "is_deleted": false } },
            doc! { "$group": { "_id": "$resource_type", "n": { "$sum": 1 } } },
        ];
        grouped_string_counts(resources, pipeline).await
    }

    async fn count_by_types(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
    ) -> StorageResult<Vec<(String, u64)>> {
        // Nothing to match; avoid an empty `$in` round-trip.
        if resource_types.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        // Same filters as `count_all_types` plus a `resource_type IN (...)` restriction.
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "is_deleted": false,
                "resource_type": { "$in": resource_types.to_vec() },
            }},
            doc! { "$group": { "_id": "$resource_type", "n": { "$sum": 1 } } },
        ];
        grouped_string_counts(resources, pipeline).await
    }

    async fn count_by_tenant(&self) -> StorageResult<Vec<(String, u64)>> {
        // Cross-tenant admin aggregate (see trait docs): no tenant filter.
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let pipeline = vec![
            doc! { "$match": { "is_deleted": false } },
            doc! { "$group": { "_id": "$tenant_id", "n": { "$sum": 1 } } },
        ];
        grouped_string_counts(resources, pipeline).await
    }
}

/// Runs a `$group`-by-string aggregation and collects `(_id, n)` pairs, where
/// `_id` is a string key and `n` a `$sum` count. Shared by `count_all_types`
/// and `count_by_tenant`.
async fn grouped_string_counts(
    collection: mongodb::Collection<Document>,
    pipeline: Vec<Document>,
) -> StorageResult<Vec<(String, u64)>> {
    let mut cursor = collection
        .aggregate(pipeline)
        .await
        .map_err(|e| internal_error(format!("Failed to aggregate grouped counts: {}", e)))?;
    let mut out = Vec::new();
    while cursor
        .advance()
        .await
        .map_err(|e| internal_error(format!("grouped counts cursor advance: {}", e)))?
    {
        let doc = cursor
            .deserialize_current()
            .map_err(|e| internal_error(format!("grouped counts cursor deserialize: {}", e)))?;
        let key = doc.get_str("_id").unwrap_or_default().to_string();
        if key.is_empty() {
            continue;
        }
        let n = doc
            .get_i32("n")
            .map(i64::from)
            .or_else(|_| doc.get_i64("n"))
            .unwrap_or(0);
        out.push((key, n.max(0) as u64));
    }
    Ok(out)
}

impl MongoBackend {
    pub(crate) async fn index_resource(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
        session: &mut Option<ClientSession>,
    ) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }

        self.delete_search_index(db, tenant_id, resource_type, resource_id, session)
            .await?;

        let mut index_docs = match self.search_extractor().extract(resource, resource_type) {
            Ok(values) => values
                .iter()
                .filter_map(|value| {
                    self.build_search_index_document(tenant_id, resource_type, resource_id, value)
                })
                .collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!(
                    "Search extraction failed for {}/{}: {}. Using minimal fallback index values.",
                    resource_type,
                    resource_id,
                    e
                );
                self.index_minimal_fallback_documents(
                    tenant_id,
                    resource_type,
                    resource_id,
                    resource,
                )
            }
        };

        // Also index any contained resources for `_contained` search. These rows
        // share the container's (resource_type, resource_id) — so the earlier
        // delete-by-(type,id) cleans them too — but are flagged `is_contained`
        // and carry the contained resource's type and local id.
        for contained in self.search_extractor().extract_contained(resource) {
            for value in &contained.values {
                if let Some(d) = self.build_contained_index_document(
                    tenant_id,
                    resource_type,
                    resource_id,
                    &contained.contained_type,
                    &contained.local_id,
                    value,
                ) {
                    index_docs.push(d);
                }
            }
        }

        if index_docs.is_empty() {
            return Ok(());
        }

        let collection = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);

        if let Some(active_session) = session.as_mut() {
            collection
                .insert_many(index_docs)
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to insert search index entries: {}", e))
                })?;
        } else {
            collection.insert_many(index_docs).await.map_err(|e| {
                internal_error(format!("Failed to insert search index entries: {}", e))
            })?;
        }

        Ok(())
    }

    pub(crate) async fn delete_search_index(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        session: &mut Option<ClientSession>,
    ) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }

        let collection = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "resource_id": resource_id,
        };

        if let Some(active_session) = session.as_mut() {
            collection
                .delete_many(filter)
                .session(active_session)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to delete search index entries: {}", e))
                })?;
        } else {
            collection.delete_many(filter).await.map_err(|e| {
                internal_error(format!("Failed to delete search index entries: {}", e))
            })?;
        }

        Ok(())
    }

    fn build_search_index_document(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        value: &ExtractedValue,
    ) -> Option<Document> {
        let mut doc = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "resource_id": resource_id,
            "param_name": &value.param_name,
            "param_url": &value.param_url,
        };

        match &value.value {
            IndexValue::String(v) => {
                doc.insert("value_string", v.to_lowercase());
            }
            IndexValue::Token {
                system,
                code,
                display,
                identifier_type_system,
                identifier_type_code,
            } => {
                if let Some(system) = system {
                    doc.insert("value_token_system", system.clone());
                }
                doc.insert("value_token_code", code.clone());
                if let Some(display) = display {
                    doc.insert("value_token_display", display.clone());
                }
                if let Some(type_system) = identifier_type_system {
                    doc.insert("value_identifier_type_system", type_system.clone());
                }
                if let Some(type_code) = identifier_type_code {
                    doc.insert("value_identifier_type_code", type_code.clone());
                }
            }
            IndexValue::Date {
                value: date,
                precision,
            } => {
                let normalized = match normalize_date_for_mongo(date) {
                    Some(v) => v,
                    None => {
                        tracing::warn!(
                            "Skipping invalid date index value '{}' for parameter '{}'",
                            date,
                            value.param_name
                        );
                        return None;
                    }
                };
                doc.insert("value_date", chrono_to_bson(normalized));
                doc.insert("value_date_precision", precision.to_string());
            }
            IndexValue::Number(v) => {
                doc.insert("value_number", *v);
            }
            IndexValue::Quantity {
                value,
                unit,
                system,
                ..
            } => {
                doc.insert("value_quantity_value", *value);
                if let Some(unit) = unit {
                    doc.insert("value_quantity_unit", unit.clone());
                }
                if let Some(system) = system {
                    doc.insert("value_quantity_system", system.clone());
                }
            }
            IndexValue::Reference {
                reference, display, ..
            } => {
                doc.insert("value_reference", reference.clone());
                if let Some(d) = display {
                    doc.insert("value_reference_display", d.clone());
                }
            }
            IndexValue::Uri(uri) => {
                doc.insert("value_uri", uri.clone());
            }
        }

        if let Some(group) = value.composite_group {
            doc.insert("composite_group", group as i32);
        }

        Some(doc)
    }

    /// Builds a contained-resource search-index document (`_contained` search):
    /// the same value columns as [`Self::build_search_index_document`], with the
    /// container's `(resource_type, resource_id)`, flagged `is_contained` and
    /// carrying the contained resource's type and local id.
    fn build_contained_index_document(
        &self,
        tenant_id: &str,
        container_type: &str,
        container_id: &str,
        contained_type: &str,
        contained_local_id: &str,
        value: &ExtractedValue,
    ) -> Option<Document> {
        let mut doc =
            self.build_search_index_document(tenant_id, container_type, container_id, value)?;
        doc.insert("is_contained", true);
        doc.insert("contained_type", contained_type);
        doc.insert("contained_local_id", contained_local_id);
        Some(doc)
    }

    fn index_minimal_fallback_documents(
        &self,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> Vec<Document> {
        let mut docs = Vec::new();

        let resource_id_value = resource
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(resource_id);

        docs.push(doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "resource_id": resource_id,
            "param_name": "_id",
            "param_url": "http://hl7.org/fhir/SearchParameter/Resource-id",
            "value_token_code": resource_id_value,
        });

        if let Some(last_updated) = resource
            .get("meta")
            .and_then(|meta| meta.get("lastUpdated"))
            .and_then(|v| v.as_str())
            .and_then(normalize_date_for_mongo)
        {
            docs.push(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": resource_id,
                "param_name": "_lastUpdated",
                "param_url": "http://hl7.org/fhir/SearchParameter/Resource-lastUpdated",
                "value_date": chrono_to_bson(last_updated),
            });
        }

        docs
    }

    fn handle_search_parameter_create(&self, resource: &Value) -> StorageResult<()> {
        let loader = SearchParameterLoader::new(self.config().fhir_version);

        match loader.parse_resource(resource) {
            Ok(def) => {
                if def.status == SearchParameterStatus::Active {
                    let mut registry = self.search_registry().write();
                    if let Err(e) = registry.register(def) {
                        tracing::debug!("SearchParameter registration skipped: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to parse SearchParameter for registry update: {}", e);
            }
        }

        Ok(())
    }

    fn handle_search_parameter_update(
        &self,
        old_resource: &Value,
        new_resource: &Value,
    ) -> StorageResult<()> {
        let loader = SearchParameterLoader::new(self.config().fhir_version);

        let old_def = loader.parse_resource(old_resource).ok();
        let new_def = loader.parse_resource(new_resource).ok();

        match (old_def, new_def) {
            (Some(old), Some(new)) => {
                let mut registry = self.search_registry().write();

                if old.url != new.url {
                    let _ = registry.unregister(&old.url);
                    if new.status == SearchParameterStatus::Active {
                        let _ = registry.register(new);
                    }
                } else if old.status != new.status {
                    if let Err(e) = registry.update_status(&new.url, new.status) {
                        tracing::debug!("SearchParameter status update skipped: {}", e);
                    }
                } else {
                    let _ = registry.unregister(&old.url);
                    if new.status == SearchParameterStatus::Active {
                        let _ = registry.register(new);
                    }
                }
            }
            (None, Some(new)) => {
                if new.status == SearchParameterStatus::Active {
                    let mut registry = self.search_registry().write();
                    let _ = registry.register(new);
                }
            }
            (Some(old), None) => {
                let mut registry = self.search_registry().write();
                let _ = registry.unregister(&old.url);
            }
            (None, None) => {}
        }

        Ok(())
    }

    fn handle_search_parameter_delete(&self, resource: &Value) -> StorageResult<()> {
        if let Some(url) = resource.get("url").and_then(|v| v.as_str()) {
            let mut registry = self.search_registry().write();
            if let Err(e) = registry.unregister(url) {
                tracing::debug!("SearchParameter unregistration skipped: {}", e);
            }
        }

        Ok(())
    }
}

#[async_trait]
impl VersionedStorage for MongoBackend {
    async fn vread(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        version_id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let maybe_doc = history
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "version_id": version_id,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to read historical version: {}", e)))?;

        let Some(doc) = maybe_doc else {
            return Ok(None);
        };

        let row = parse_history_row(&doc, Some(resource_type), Some(id))?;
        Ok(Some(row.into_stored_resource(tenant)))
    }

    async fn update_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        resource: Value,
    ) -> StorageResult<StoredResource> {
        let current = self.read(tenant, resource_type, id).await?.ok_or_else(|| {
            StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            })
        })?;

        let expected = normalize_etag(expected_version);
        let actual = normalize_etag(current.version_id());

        if expected != actual {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
                    actual_version: actual.to_string(),
                },
            ));
        }

        self.update(tenant, &current, resource).await
    }

    async fn delete_with_match(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
    ) -> StorageResult<()> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let maybe_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to load resource for delete_with_match: {}",
                    e
                ))
            })?;

        let Some(doc) = maybe_doc else {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        };

        let actual = doc.get_str("version_id").map_err(|e| {
            internal_error(format!(
                "Missing current version for delete_with_match: {}",
                e
            ))
        })?;

        let expected = normalize_etag(expected_version);
        let actual = normalize_etag(actual);

        if expected != actual {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
                    actual_version: actual.to_string(),
                },
            ));
        }

        self.delete(tenant, resource_type, id).await
    }

    async fn list_versions(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Vec<String>> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let cursor = history
            .find(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to query version history: {}", e)))?;

        let docs = collect_documents(cursor).await?;
        let mut versions = docs
            .iter()
            .filter_map(|doc| doc.get_str("version_id").ok().map(str::to_string))
            .collect::<Vec<_>>();

        versions.sort_by(|a, b| {
            parse_version_id(a)
                .cmp(&parse_version_id(b))
                .then_with(|| a.cmp(b))
        });

        Ok(versions)
    }
}

#[async_trait]
impl InstanceHistoryProvider for MongoBackend {
    async fn history_instance(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "id": id,
        };
        apply_history_params_filter(&mut filter, params);

        let cursor = history
            .find(filter)
            .await
            .map_err(|e| internal_error(format!("Failed to query instance history: {}", e)))?;

        let docs = collect_documents(cursor).await?;
        let mut rows = docs
            .iter()
            .map(|doc| parse_history_row(doc, Some(resource_type), Some(id)))
            .collect::<StorageResult<Vec<_>>>()?;

        rows.sort_by(|a, b| {
            parse_version_id(&b.version_id)
                .cmp(&parse_version_id(&a.version_id))
                .then_with(|| b.last_updated.cmp(&a.last_updated))
        });

        if let Some(cursor_version) = parse_cursor_version(params) {
            rows.retain(|row| parse_version_id(&row.version_id) < cursor_version);
        }

        let page_len = params.pagination.count as usize;
        let has_more = rows.len() > page_len;
        if has_more {
            rows.truncate(page_len);
        }

        let page_info = if has_more {
            if let Some(last) = rows.last() {
                PageInfo::with_next(PageCursor::new(
                    vec![CursorValue::String(last.version_id.clone())],
                    id.to_string(),
                ))
            } else {
                PageInfo::end()
            }
        } else {
            PageInfo::end()
        };

        let entries = rows
            .into_iter()
            .map(|row| row.into_history_entry(tenant))
            .collect::<Vec<_>>();

        Ok(Page::new(entries, page_info))
    }

    async fn history_instance_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        history
            .count_documents(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to count instance history: {}", e)))
    }
}

#[async_trait]
impl TypeHistoryProvider for MongoBackend {
    async fn history_type(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
        };
        apply_history_params_filter(&mut filter, params);

        let cursor = history
            .find(filter)
            .await
            .map_err(|e| internal_error(format!("Failed to query type history: {}", e)))?;

        let docs = collect_documents(cursor).await?;
        let mut rows = docs
            .iter()
            .map(|doc| parse_history_row(doc, Some(resource_type), None))
            .collect::<StorageResult<Vec<_>>>()?;

        rows.sort_by(|a, b| {
            b.last_updated
                .cmp(&a.last_updated)
                .then_with(|| b.id.cmp(&a.id))
                .then_with(|| parse_version_id(&b.version_id).cmp(&parse_version_id(&a.version_id)))
        });

        if let Some((cursor_timestamp, cursor_id)) = parse_type_history_cursor(params) {
            rows.retain(|row| {
                row.last_updated < cursor_timestamp
                    || (row.last_updated == cursor_timestamp && row.id < cursor_id)
            });
        }

        let page_len = params.pagination.count as usize;
        let has_more = rows.len() > page_len;
        if has_more {
            rows.truncate(page_len);
        }

        let page_info = if has_more {
            if let Some(last) = rows.last() {
                PageInfo::with_next(PageCursor::new(
                    vec![
                        CursorValue::String(last.last_updated.to_rfc3339()),
                        CursorValue::String(last.id.clone()),
                    ],
                    resource_type.to_string(),
                ))
            } else {
                PageInfo::end()
            }
        } else {
            PageInfo::end()
        };

        let entries = rows
            .into_iter()
            .map(|row| row.into_history_entry(tenant))
            .collect::<Vec<_>>();

        Ok(Page::new(entries, page_info))
    }

    async fn history_type_count(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
    ) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        history
            .count_documents(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to count type history: {}", e)))
    }
}

#[async_trait]
impl SystemHistoryProvider for MongoBackend {
    async fn history_system(
        &self,
        tenant: &TenantContext,
        params: &HistoryParams,
    ) -> StorageResult<HistoryPage> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let mut filter = doc! {
            "tenant_id": tenant_id,
        };
        apply_history_params_filter(&mut filter, params);

        let cursor = history
            .find(filter)
            .await
            .map_err(|e| internal_error(format!("Failed to query system history: {}", e)))?;

        let docs = collect_documents(cursor).await?;
        let mut rows = docs
            .iter()
            .map(|doc| parse_history_row(doc, None, None))
            .collect::<StorageResult<Vec<_>>>()?;

        rows.sort_by(|a, b| {
            b.last_updated
                .cmp(&a.last_updated)
                .then_with(|| b.resource_type.cmp(&a.resource_type))
                .then_with(|| b.id.cmp(&a.id))
                .then_with(|| parse_version_id(&b.version_id).cmp(&parse_version_id(&a.version_id)))
        });

        if let Some((cursor_timestamp, cursor_type, cursor_id)) =
            parse_system_history_cursor(params)
        {
            rows.retain(|row| {
                row.last_updated < cursor_timestamp
                    || (row.last_updated == cursor_timestamp
                        && (row.resource_type < cursor_type
                            || (row.resource_type == cursor_type && row.id < cursor_id)))
            });
        }

        let page_len = params.pagination.count as usize;
        let has_more = rows.len() > page_len;
        if has_more {
            rows.truncate(page_len);
        }

        let page_info = if has_more {
            if let Some(last) = rows.last() {
                PageInfo::with_next(PageCursor::new(
                    vec![
                        CursorValue::String(last.last_updated.to_rfc3339()),
                        CursorValue::String(last.resource_type.clone()),
                        CursorValue::String(last.id.clone()),
                    ],
                    "system".to_string(),
                ))
            } else {
                PageInfo::end()
            }
        } else {
            PageInfo::end()
        };

        let entries = rows
            .into_iter()
            .map(|row| row.into_history_entry(tenant))
            .collect::<Vec<_>>();

        Ok(Page::new(entries, page_info))
    }

    async fn history_system_count(&self, tenant: &TenantContext) -> StorageResult<u64> {
        let db = self.get_database().await?;
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        history
            .count_documents(doc! {
                "tenant_id": tenant_id,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to count system history: {}", e)))
    }
}

#[async_trait]
impl BundleProvider for MongoBackend {
    async fn process_transaction(
        &self,
        tenant: &TenantContext,
        entries: Vec<BundleEntry>,
    ) -> Result<BundleResult, TransactionError> {
        let db = self
            .get_database()
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Failed to acquire MongoDB database: {}", e),
            })?;

        let mut session = begin_required_bundle_transaction_session(&db).await?;

        let mut results = Vec::with_capacity(entries.len());
        let mut error_info: Option<(usize, String)> = None;
        let mut reference_map: HashMap<String, String> = HashMap::new();
        let mut pending_search_parameter_changes: Vec<PendingSearchParameterChange> = Vec::new();
        let mut entries = entries;

        for (idx, entry) in entries.iter_mut().enumerate() {
            if let Some(resource) = entry.resource.as_mut() {
                resolve_bundle_references(resource, &reference_map);
            }

            let result = self
                .process_bundle_entry_transaction(
                    &db,
                    &mut session,
                    tenant,
                    entry,
                    &mut pending_search_parameter_changes,
                )
                .await;

            match result {
                Ok(entry_result) => {
                    if entry_result.status >= 400 {
                        error_info = Some((
                            idx,
                            format!("Entry failed with status {}", entry_result.status),
                        ));
                        break;
                    }

                    if entry.method == BundleMethod::Post {
                        if let Some(full_url) = entry.full_url.as_ref() {
                            if let Some(location) = entry_result.location.as_ref() {
                                let reference = location
                                    .split("/_history")
                                    .next()
                                    .unwrap_or(location)
                                    .to_string();
                                reference_map.insert(full_url.clone(), reference);
                            }
                        }
                    }

                    results.push(entry_result);
                }
                Err(e) => {
                    error_info = Some((idx, format!("Entry processing failed: {}", e)));
                    break;
                }
            }
        }

        if let Some((index, message)) = error_info {
            let _ = session.abort_transaction().await;
            return Err(TransactionError::BundleError { index, message });
        }

        session
            .commit_transaction()
            .await
            .map_err(|e| TransactionError::RolledBack {
                reason: format!("Commit failed: {}", e),
            })?;

        for change in pending_search_parameter_changes {
            let result = match change {
                PendingSearchParameterChange::Create(resource) => {
                    self.handle_search_parameter_create(&resource)
                }
                PendingSearchParameterChange::Update { old, new } => {
                    self.handle_search_parameter_update(&old, &new)
                }
                PendingSearchParameterChange::Delete(resource) => {
                    self.handle_search_parameter_delete(&resource)
                }
            };

            if let Err(e) = result {
                tracing::warn!(
                    "Transaction committed but failed to apply SearchParameter registry update: {}",
                    e
                );
            }
        }

        Ok(BundleResult {
            bundle_type: BundleType::Transaction,
            entries: results,
        })
    }

    async fn process_batch(
        &self,
        _tenant: &TenantContext,
        _entries: Vec<BundleEntry>,
    ) -> StorageResult<BundleResult> {
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "mongodb".to_string(),
            capability: "BundleProvider".to_string(),
        }))
    }
}

impl MongoBackend {
    async fn process_bundle_entry_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        entry: &BundleEntry,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<BundleEntryResult> {
        match entry.method {
            BundleMethod::Get => {
                let (resource_type, id) = self.parse_url(&entry.url)?;
                match self
                    .read_resource_in_bundle_transaction(db, session, tenant, &resource_type, &id)
                    .await?
                {
                    Some(resource) => Ok(BundleEntryResult::ok(resource)),
                    None => Ok(BundleEntryResult::error(
                        404,
                        serde_json::json!({
                            "resourceType": "OperationOutcome",
                            "issue": [{"severity": "error", "code": "not-found"}]
                        }),
                    )),
                }
            }
            BundleMethod::Post => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let resource_type = resource
                    .get("resourceType")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        StorageError::Validation(
                            crate::error::ValidationError::MissingRequiredField {
                                field: "resourceType".to_string(),
                            },
                        )
                    })?;

                if let Some(search_params) = entry.if_none_exist.as_ref() {
                    let matches = self
                        .find_matching_resources_in_bundle_transaction(
                            db,
                            session,
                            tenant,
                            &resource_type,
                            search_params,
                        )
                        .await?;

                    match matches.len() {
                        0 => {}
                        1 => {
                            return Ok(BundleEntryResult::ok(
                                matches.into_iter().next().expect("single match must exist"),
                            ));
                        }
                        n => {
                            return Ok(BundleEntryResult::error(
                                412,
                                serde_json::json!({
                                    "resourceType": "OperationOutcome",
                                    "issue": [{
                                        "severity": "error",
                                        "code": "multiple-matches",
                                        "diagnostics": format!(
                                            "Conditional create matched {} resources",
                                            n
                                        )
                                    }]
                                }),
                            ));
                        }
                    }
                }

                let created = self
                    .create_resource_in_bundle_transaction(
                        db,
                        session,
                        tenant,
                        &resource_type,
                        resource,
                        pending_search_parameter_changes,
                    )
                    .await?;
                Ok(BundleEntryResult::created(created))
            }
            BundleMethod::Put => {
                let resource = entry.resource.clone().ok_or_else(|| {
                    StorageError::Validation(crate::error::ValidationError::MissingRequiredField {
                        field: "resource".to_string(),
                    })
                })?;

                let (resource_type, id) = self.parse_url(&entry.url)?;

                match self
                    .read_resource_in_bundle_transaction(db, session, tenant, &resource_type, &id)
                    .await?
                {
                    Some(existing) => {
                        if let Some(if_match) = entry.if_match.as_ref() {
                            let expected = normalize_etag(if_match);
                            let actual = normalize_etag(existing.version_id());
                            if expected != actual {
                                return Ok(BundleEntryResult::error(
                                    412,
                                    serde_json::json!({
                                        "resourceType": "OperationOutcome",
                                        "issue": [{"severity": "error", "code": "conflict", "diagnostics": "ETag mismatch"}]
                                    }),
                                ));
                            }
                        }

                        let updated = self
                            .update_resource_in_bundle_transaction(
                                db,
                                session,
                                tenant,
                                &existing,
                                resource,
                                pending_search_parameter_changes,
                            )
                            .await?;
                        Ok(BundleEntryResult::ok(updated))
                    }
                    None => {
                        let mut resource_with_id = resource;
                        resource_with_id["id"] = serde_json::json!(id);

                        let created = self
                            .create_resource_in_bundle_transaction(
                                db,
                                session,
                                tenant,
                                &resource_type,
                                resource_with_id,
                                pending_search_parameter_changes,
                            )
                            .await?;
                        Ok(BundleEntryResult::created(created))
                    }
                }
            }
            BundleMethod::Delete => {
                let (resource_type, id) = self.parse_url(&entry.url)?;

                if let Some(if_match) = entry.if_match.as_ref() {
                    match self
                        .delete_with_match_resource_in_bundle_transaction(
                            db,
                            session,
                            tenant,
                            &resource_type,
                            &id,
                            if_match,
                            pending_search_parameter_changes,
                        )
                        .await
                    {
                        Ok(()) => Ok(BundleEntryResult::deleted()),
                        Err(StorageError::Resource(ResourceError::NotFound { .. })) => {
                            Ok(BundleEntryResult::error(
                                404,
                                serde_json::json!({
                                    "resourceType": "OperationOutcome",
                                    "issue": [{"severity": "error", "code": "not-found"}]
                                }),
                            ))
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    match self
                        .delete_resource_in_bundle_transaction(
                            db,
                            session,
                            tenant,
                            &resource_type,
                            &id,
                            pending_search_parameter_changes,
                        )
                        .await
                    {
                        Ok(()) => Ok(BundleEntryResult::deleted()),
                        Err(StorageError::Resource(ResourceError::NotFound { .. })) => {
                            Ok(BundleEntryResult::deleted())
                        }
                        Err(e) => Err(e),
                    }
                }
            }
            BundleMethod::Patch => Ok(BundleEntryResult::error(
                501,
                serde_json::json!({
                    "resourceType": "OperationOutcome",
                    "issue": [{"severity": "error", "code": "not-supported", "diagnostics": "PATCH not implemented in transaction bundles"}]
                }),
            )),
        }
    }

    async fn create_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<StoredResource> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let id = resource
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let existing = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": &id,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to check resource existence in transaction: {}",
                    e
                ))
            })?;

        if existing.is_some() {
            return Err(StorageError::Resource(ResourceError::AlreadyExists {
                resource_type: resource_type.to_string(),
                id,
            }));
        }

        let mut resource = resource;
        ensure_resource_identity(resource_type, &id, &mut resource);
        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let version_id = "1".to_string();
        let fhir_version = FhirVersion::default_enabled();
        let fhir_version_str = fhir_version.as_mime_param().to_string();

        resources
            .insert_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": &id,
                "version_id": &version_id,
                "data": Bson::Document(payload.clone()),
                "created_at": now_bson,
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": &fhir_version_str,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                if is_duplicate_key_error(&e) {
                    StorageError::Resource(ResourceError::AlreadyExists {
                        resource_type: resource_type.to_string(),
                        id: id.clone(),
                    })
                } else {
                    internal_error(format!("Failed to insert resource in transaction: {}", e))
                }
            })?;

        history
            .insert_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": &id,
                "version_id": &version_id,
                "data": Bson::Document(payload),
                "created_at": now_bson,
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": &fhir_version_str,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!("Failed to insert history in transaction: {}", e))
            })?;

        self.index_resource_in_bundle_transaction(
            db,
            session,
            tenant_id,
            resource_type,
            &id,
            &resource,
        )
        .await?;

        if resource_type == "SearchParameter" {
            pending_search_parameter_changes
                .push(PendingSearchParameterChange::Create(resource.clone()));
        }

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

    async fn update_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<StoredResource> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = current.resource_type();
        let id = current.id();

        let existing_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to load current resource in transaction: {}",
                    e
                ))
            })?
            .ok_or_else(|| {
                StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                })
            })?;

        let actual_version = existing_doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing current version in transaction: {}", e)))?
            .to_string();

        if actual_version != current.version_id() {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version,
                },
            ));
        }

        let new_version = next_version(current.version_id())?;
        let mut resource = resource;
        ensure_resource_identity(resource_type, id, &mut resource);
        let payload = value_to_document(&resource)?;

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);
        let fhir_version = current.fhir_version();
        let fhir_version_str = fhir_version.as_mime_param().to_string();

        let update_result = resources
            .update_one(
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "id": id,
                    "version_id": current.version_id(),
                    "is_deleted": false,
                },
                doc! {
                    "$set": {
                        "version_id": &new_version,
                        "data": Bson::Document(payload.clone()),
                        "last_updated": now_bson,
                        "is_deleted": false,
                        "deleted_at": Bson::Null,
                        "fhir_version": &fhir_version_str,
                    }
                },
            )
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!("Failed to update resource in transaction: {}", e))
            })?;

        if update_result.matched_count == 0 {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: current.version_id().to_string(),
                    actual_version: "unknown".to_string(),
                },
            ));
        }

        let created_at = extract_created_at(&existing_doc, now);

        history
            .insert_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "version_id": &new_version,
                "data": Bson::Document(payload),
                "created_at": chrono_to_bson(created_at),
                "last_updated": now_bson,
                "is_deleted": false,
                "deleted_at": Bson::Null,
                "fhir_version": &fhir_version_str,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!("Failed to insert history in transaction: {}", e))
            })?;

        self.index_resource_in_bundle_transaction(
            db,
            session,
            tenant_id,
            resource_type,
            id,
            &resource,
        )
        .await?;

        if resource_type == "SearchParameter" {
            pending_search_parameter_changes.push(PendingSearchParameterChange::Update {
                old: current.content().clone(),
                new: resource.clone(),
            });
        }

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            new_version,
            tenant.tenant_id().clone(),
            resource,
            created_at,
            now,
            None,
            fhir_version,
        ))
    }

    async fn delete_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<()> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let history = db.collection::<Document>(MongoBackend::RESOURCE_HISTORY_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let existing_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to load resource for delete in transaction: {}",
                    e
                ))
            })?
            .ok_or_else(|| {
                StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                })
            })?;

        let current_version = existing_doc
            .get_str("version_id")
            .map_err(|e| {
                internal_error(format!(
                    "Missing current version in transaction delete: {}",
                    e
                ))
            })?
            .to_string();
        let new_version = next_version(&current_version)?;

        let payload = existing_doc
            .get_document("data")
            .map_err(|e| {
                internal_error(format!(
                    "Missing resource payload in transaction delete: {}",
                    e
                ))
            })?
            .clone();
        let resource_value = document_to_value(&payload)?;
        let fhir_version = existing_doc
            .get_str("fhir_version")
            .unwrap_or("4.0")
            .to_string();
        let created_at = extract_created_at(&existing_doc, Utc::now());

        let now = Utc::now();
        let now_bson = chrono_to_bson(now);

        let update_result = resources
            .update_one(
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "id": id,
                    "version_id": &current_version,
                    "is_deleted": false,
                },
                doc! {
                    "$set": {
                        "version_id": &new_version,
                        "is_deleted": true,
                        "deleted_at": now_bson,
                        "last_updated": now_bson,
                    }
                },
            )
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to soft-delete resource in transaction: {}",
                    e
                ))
            })?;

        if update_result.matched_count == 0 {
            return Err(StorageError::Resource(ResourceError::NotFound {
                resource_type: resource_type.to_string(),
                id: id.to_string(),
            }));
        }

        history
            .insert_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "version_id": &new_version,
                "data": Bson::Document(payload),
                "created_at": chrono_to_bson(created_at),
                "last_updated": now_bson,
                "is_deleted": true,
                "deleted_at": now_bson,
                "fhir_version": fhir_version,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to insert delete history in transaction: {}",
                    e
                ))
            })?;

        self.delete_search_index_in_bundle_transaction(db, session, tenant_id, resource_type, id)
            .await?;

        if resource_type == "SearchParameter" {
            pending_search_parameter_changes
                .push(PendingSearchParameterChange::Delete(resource_value));
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn delete_with_match_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        expected_version: &str,
        pending_search_parameter_changes: &mut Vec<PendingSearchParameterChange>,
    ) -> StorageResult<()> {
        let existing = self
            .read_resource_in_bundle_transaction(db, session, tenant, resource_type, id)
            .await?
            .ok_or_else(|| {
                StorageError::Resource(ResourceError::NotFound {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                })
            })?;

        let expected = normalize_etag(expected_version);
        let actual = normalize_etag(existing.version_id());
        if expected != actual {
            return Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict {
                    resource_type: resource_type.to_string(),
                    id: id.to_string(),
                    expected_version: expected.to_string(),
                    actual_version: actual.to_string(),
                },
            ));
        }

        self.delete_resource_in_bundle_transaction(
            db,
            session,
            tenant,
            resource_type,
            id,
            pending_search_parameter_changes,
        )
        .await
    }

    async fn read_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let maybe_doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!("Failed to read resource in transaction: {}", e))
            })?;

        maybe_doc
            .as_ref()
            .map(|doc| document_to_stored_resource(doc, tenant, resource_type))
            .transpose()
    }

    async fn find_matching_resources_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        let parsed_params = parse_simple_bundle_search_params(search_params);
        if parsed_params.is_empty() {
            return Ok(Vec::new());
        }

        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let cursor = resources
            .find(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "is_deleted": false,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to query conditional matches in transaction: {}",
                    e
                ))
            })?;

        let docs = collect_session_documents(cursor, session).await?;
        let mut matches = Vec::new();

        for doc in docs {
            let payload = doc.get_document("data").map_err(|e| {
                internal_error(format!(
                    "Missing payload while matching conditionals: {}",
                    e
                ))
            })?;
            let resource = document_to_value(payload)?;

            if resource_matches_bundle_search_params(&resource, &parsed_params)
                && doc
                    .get_str("resource_type")
                    .map(|rt| rt == resource_type)
                    .unwrap_or(true)
            {
                matches.push(document_to_stored_resource(&doc, tenant, resource_type)?);
            }
        }

        Ok(matches)
    }

    async fn index_resource_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource: &Value,
    ) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }

        self.delete_search_index_in_bundle_transaction(
            db,
            session,
            tenant_id,
            resource_type,
            resource_id,
        )
        .await?;

        let index_docs = match self.search_extractor().extract(resource, resource_type) {
            Ok(values) => values
                .iter()
                .filter_map(|value| {
                    self.build_search_index_document(tenant_id, resource_type, resource_id, value)
                })
                .collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!(
                    "Search extraction failed for {}/{} in transaction: {}. Using minimal fallback index values.",
                    resource_type,
                    resource_id,
                    e
                );
                self.index_minimal_fallback_documents(
                    tenant_id,
                    resource_type,
                    resource_id,
                    resource,
                )
            }
        };

        if index_docs.is_empty() {
            return Ok(());
        }

        db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION)
            .insert_many(index_docs)
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to insert search_index entries in transaction: {}",
                    e
                ))
            })?;

        Ok(())
    }

    async fn delete_search_index_in_bundle_transaction(
        &self,
        db: &mongodb::Database,
        session: &mut ClientSession,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
    ) -> StorageResult<()> {
        if self.is_search_offloaded() {
            return Ok(());
        }

        db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION)
            .delete_many(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "resource_id": resource_id,
            })
            .session(&mut *session)
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to delete search_index entries in transaction: {}",
                    e
                ))
            })?;

        Ok(())
    }

    fn parse_url(&self, url: &str) -> StorageResult<(String, String)> {
        let path = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .map(|s| s.find('/').map(|i| &s[i..]).unwrap_or(s))
            .unwrap_or(url);

        let path = path.trim_start_matches('/');
        let parts: Vec<&str> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();

        if parts.len() >= 2 {
            let len = parts.len();
            Ok((parts[len - 2].to_string(), parts[len - 1].to_string()))
        } else {
            Err(StorageError::Validation(
                crate::error::ValidationError::InvalidReference {
                    reference: url.to_string(),
                    message: "URL must be in format ResourceType/id".to_string(),
                },
            ))
        }
    }
}

fn resource_matches_bundle_search_params(resource: &Value, params: &[(String, String)]) -> bool {
    params.iter().all(|(name, expected)| match name.as_str() {
        "_id" => resource
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == expected),
        "identifier" => resource_identifier_matches(resource, expected),
        _ => resource_field_matches(resource.get(name), expected),
    })
}

fn resource_identifier_matches(resource: &Value, expected: &str) -> bool {
    let Some(identifier_value) = resource.get("identifier") else {
        return false;
    };

    let (system, value, has_separator) = if let Some((system, value)) = expected.split_once('|') {
        (system, value, true)
    } else {
        ("", expected, false)
    };

    match identifier_value {
        Value::Array(items) => items
            .iter()
            .any(|item| match_identifier_item(item, system, value, has_separator)),
        Value::Object(_) => match_identifier_item(identifier_value, system, value, has_separator),
        _ => false,
    }
}

fn match_identifier_item(item: &Value, system: &str, value: &str, has_separator: bool) -> bool {
    let item_system = item.get("system").and_then(Value::as_str);
    let item_value = item.get("value").and_then(Value::as_str);

    if has_separator {
        let system_matches = if system.is_empty() {
            true
        } else {
            item_system == Some(system)
        };
        let value_matches = if value.is_empty() {
            true
        } else {
            item_value == Some(value)
        };

        system_matches && value_matches
    } else {
        item_value == Some(value)
    }
}

fn resource_field_matches(value: Option<&Value>, expected: &str) -> bool {
    let Some(value) = value else {
        return false;
    };

    match value {
        Value::String(s) => s == expected,
        Value::Array(items) => items
            .iter()
            .any(|item| resource_field_matches(Some(item), expected)),
        Value::Object(map) => {
            if map
                .get("reference")
                .and_then(Value::as_str)
                .is_some_and(|reference| reference == expected)
            {
                return true;
            }

            if map
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(|value| value == expected)
            {
                return true;
            }

            map.values()
                .any(|nested| resource_field_matches(Some(nested), expected))
        }
        _ => false,
    }
}

fn resolve_bundle_references(value: &mut Value, reference_map: &HashMap<String, String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("reference") {
                if reference.starts_with("urn:uuid:") {
                    if let Some(resolved) = reference_map.get(reference) {
                        map.insert("reference".to_string(), Value::String(resolved.clone()));
                    }
                }
            }

            for nested in map.values_mut() {
                resolve_bundle_references(nested, reference_map);
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_bundle_references(item, reference_map);
            }
        }
        _ => {}
    }
}
