//! Stored resource types.
//!
//! This module defines the [`StoredResource`] type, which wraps FHIR resources
//! with persistence metadata such as tenant, version, and timestamps.

use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tenant::TenantId;

/// A FHIR resource with persistence metadata.
///
/// `StoredResource` wraps a FHIR resource (stored as JSON) along with
/// metadata required for persistence operations:
///
/// - **Identity**: Resource type and ID
/// - **Versioning**: Version ID for optimistic locking
/// - **Tenancy**: Tenant that owns the resource
/// - **FHIR Version**: The FHIR specification version (R4, R4B, R5, R6)
/// - **Timestamps**: Creation, modification, and deletion times
/// - **ETag**: For HTTP caching and conditional updates
///
/// # Examples
///
/// ```
/// use helios_persistence::types::StoredResource;
/// use helios_persistence::tenant::TenantId;
/// use helios_fhir::FhirVersion;
/// use serde_json::json;
///
/// let resource = StoredResource::new(
///     "Patient",
///     "123",
///     TenantId::new("acme"),
///     json!({
///         "resourceType": "Patient",
///         "id": "123",
///         "name": [{"family": "Smith"}]
///     }),
///     FhirVersion::default(),
/// );
///
/// assert_eq!(resource.resource_type(), "Patient");
/// assert_eq!(resource.id(), "123");
/// assert_eq!(resource.version_id(), "1");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredResource {
    /// The FHIR resource type (e.g., "Patient", "Observation").
    resource_type: String,

    /// The resource's logical ID.
    id: String,

    /// The version ID (monotonically increasing).
    version_id: String,

    /// The tenant that owns this resource.
    tenant_id: TenantId,

    /// The FHIR specification version this resource conforms to.
    fhir_version: FhirVersion,

    /// The resource content as JSON.
    content: Value,

    /// When the resource was first created.
    created_at: DateTime<Utc>,

    /// When the resource was last modified.
    last_modified: DateTime<Utc>,

    /// If the resource has been deleted, when it was deleted.
    deleted_at: Option<DateTime<Utc>>,

    /// ETag for HTTP caching (typically derived from version_id).
    etag: String,

    /// HTTP method that created this version.
    method: Option<ResourceMethod>,
}

/// HTTP method that created a resource version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ResourceMethod {
    /// Resource was created via POST.
    Post,
    /// Resource was created/updated via PUT.
    Put,
    /// Resource was updated via PATCH.
    Patch,
    /// Resource was deleted via DELETE.
    Delete,
}

impl StoredResource {
    /// Creates a new stored resource with the given properties.
    ///
    /// This creates a new resource with:
    /// - Version ID set to "1"
    /// - Created and last_modified set to current time
    /// - ETag derived from version ID
    pub fn new(
        resource_type: impl Into<String>,
        id: impl Into<String>,
        tenant_id: TenantId,
        content: Value,
        fhir_version: FhirVersion,
    ) -> Self {
        let now = Utc::now();
        let version_id = "1".to_string();
        let etag = format!("W/\"{}\"", version_id);

        Self {
            resource_type: resource_type.into(),
            id: id.into(),
            version_id,
            tenant_id,
            fhir_version,
            content,
            created_at: now,
            last_modified: now,
            deleted_at: None,
            etag,
            method: Some(ResourceMethod::Post),
        }
    }

    /// Creates a stored resource from existing data (e.g., loaded from database).
    #[allow(clippy::too_many_arguments)]
    pub fn from_storage(
        resource_type: impl Into<String>,
        id: impl Into<String>,
        version_id: impl Into<String>,
        tenant_id: TenantId,
        content: Value,
        created_at: DateTime<Utc>,
        last_modified: DateTime<Utc>,
        deleted_at: Option<DateTime<Utc>>,
        fhir_version: FhirVersion,
    ) -> Self {
        let version_id = version_id.into();
        let etag = format!("W/\"{}\"", version_id);

        Self {
            resource_type: resource_type.into(),
            id: id.into(),
            version_id,
            tenant_id,
            fhir_version,
            content,
            created_at,
            last_modified,
            deleted_at,
            etag,
            method: None,
        }
    }

    /// Returns the FHIR resource type.
    pub fn resource_type(&self) -> &str {
        &self.resource_type
    }

    /// Returns the resource's logical ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the version ID.
    pub fn version_id(&self) -> &str {
        &self.version_id
    }

    /// Returns the tenant that owns this resource.
    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    /// Returns the FHIR specification version of this resource.
    pub fn fhir_version(&self) -> FhirVersion {
        self.fhir_version
    }

    /// Returns the resource content as JSON.
    pub fn content(&self) -> &Value {
        &self.content
    }

    /// Returns a mutable reference to the resource content.
    pub fn content_mut(&mut self) -> &mut Value {
        &mut self.content
    }

    /// Consumes self and returns the content.
    pub fn into_content(self) -> Value {
        self.content
    }

    /// Returns when the resource was created.
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// Returns when the resource was last modified.
    pub fn last_modified(&self) -> DateTime<Utc> {
        self.last_modified
    }

    /// Returns when the resource was deleted, if applicable.
    pub fn deleted_at(&self) -> Option<DateTime<Utc>> {
        self.deleted_at
    }

    /// Returns `true` if the resource has been deleted.
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Returns the ETag for HTTP caching.
    pub fn etag(&self) -> &str {
        &self.etag
    }

    /// Returns the HTTP method that created this version.
    pub fn method(&self) -> Option<ResourceMethod> {
        self.method
    }

    /// Returns the full URL path for this resource (e.g., "Patient/123").
    pub fn url(&self) -> String {
        format!("{}/{}", self.resource_type, self.id)
    }

    /// Returns the versioned URL path (e.g., "Patient/123/_history/1").
    pub fn versioned_url(&self) -> String {
        format!(
            "{}/{}/_history/{}",
            self.resource_type, self.id, self.version_id
        )
    }

    /// Creates a new version of this resource with updated content.
    ///
    /// The new version will have:
    /// - Incremented version ID
    /// - Updated last_modified timestamp
    /// - New ETag
    /// - Same FHIR version as the original
    pub fn new_version(self, content: Value, method: ResourceMethod) -> Self {
        let version: u64 = self.version_id.parse().unwrap_or(0);
        let new_version_id = (version + 1).to_string();
        let etag = format!("W/\"{}\"", new_version_id);

        Self {
            resource_type: self.resource_type,
            id: self.id,
            version_id: new_version_id,
            tenant_id: self.tenant_id,
            fhir_version: self.fhir_version,
            content,
            created_at: self.created_at,
            last_modified: Utc::now(),
            deleted_at: None,
            etag,
            method: Some(method),
        }
    }

    /// Marks this resource as deleted.
    ///
    /// Creates a new version with the deleted_at timestamp set.
    pub fn mark_deleted(self) -> Self {
        let version: u64 = self.version_id.parse().unwrap_or(0);
        let new_version_id = (version + 1).to_string();
        let etag = format!("W/\"{}\"", new_version_id);
        let now = Utc::now();

        Self {
            resource_type: self.resource_type,
            id: self.id,
            version_id: new_version_id,
            tenant_id: self.tenant_id,
            fhir_version: self.fhir_version,
            content: self.content,
            created_at: self.created_at,
            last_modified: now,
            deleted_at: Some(now),
            etag,
            method: Some(ResourceMethod::Delete),
        }
    }

    /// Checks if the given ETag matches this resource's ETag.
    ///
    /// Used for If-Match conditional updates.
    pub fn matches_etag(&self, etag: &str) -> bool {
        // Strip W/ prefix and quotes for comparison
        let normalized_self = self.etag.trim_start_matches("W/").trim_matches('"');
        let normalized_other = etag.trim_start_matches("W/").trim_matches('"');
        normalized_self == normalized_other
    }

    /// Returns the FHIR Meta element for this resource.
    pub fn meta(&self) -> ResourceMeta {
        ResourceMeta {
            version_id: self.version_id.clone(),
            last_updated: self.last_modified,
        }
    }
}

/// FHIR Meta element extracted from a stored resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMeta {
    /// The version ID.
    #[serde(rename = "versionId")]
    pub version_id: String,

    /// The last update timestamp.
    #[serde(rename = "lastUpdated")]
    pub last_updated: DateTime<Utc>,
}

/// Builder for creating stored resources with custom metadata.
#[derive(Debug, Default)]
pub struct StoredResourceBuilder {
    resource_type: Option<String>,
    id: Option<String>,
    version_id: Option<String>,
    tenant_id: Option<TenantId>,
    fhir_version: Option<FhirVersion>,
    content: Option<Value>,
    created_at: Option<DateTime<Utc>>,
    last_modified: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
    method: Option<ResourceMethod>,
}

impl StoredResourceBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the resource type.
    pub fn resource_type(mut self, resource_type: impl Into<String>) -> Self {
        self.resource_type = Some(resource_type.into());
        self
    }

    /// Sets the resource ID.
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets the version ID.
    pub fn version_id(mut self, version_id: impl Into<String>) -> Self {
        self.version_id = Some(version_id.into());
        self
    }

    /// Sets the tenant ID.
    pub fn tenant_id(mut self, tenant_id: TenantId) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// Sets the FHIR version.
    pub fn fhir_version(mut self, fhir_version: FhirVersion) -> Self {
        self.fhir_version = Some(fhir_version);
        self
    }

    /// Sets the content.
    pub fn content(mut self, content: Value) -> Self {
        self.content = Some(content);
        self
    }

    /// Sets the created timestamp.
    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    /// Sets the last modified timestamp.
    pub fn last_modified(mut self, last_modified: DateTime<Utc>) -> Self {
        self.last_modified = Some(last_modified);
        self
    }

    /// Sets the deleted timestamp.
    pub fn deleted_at(mut self, deleted_at: DateTime<Utc>) -> Self {
        self.deleted_at = Some(deleted_at);
        self
    }

    /// Sets the HTTP method.
    pub fn method(mut self, method: ResourceMethod) -> Self {
        self.method = Some(method);
        self
    }

    /// Builds the stored resource.
    ///
    /// # Panics
    ///
    /// Panics if required fields (resource_type, id, tenant_id, content) are not set.
    /// Uses the default FHIR version if not specified.
    pub fn build(self) -> StoredResource {
        let now = Utc::now();
        let version_id = self.version_id.unwrap_or_else(|| "1".to_string());
        let etag = format!("W/\"{}\"", version_id);

        StoredResource {
            resource_type: self.resource_type.expect("resource_type is required"),
            id: self.id.expect("id is required"),
            version_id,
            tenant_id: self.tenant_id.expect("tenant_id is required"),
            fhir_version: self.fhir_version.unwrap_or_default(),
            content: self.content.expect("content is required"),
            created_at: self.created_at.unwrap_or(now),
            last_modified: self.last_modified.unwrap_or(now),
            deleted_at: self.deleted_at,
            etag,
            method: self.method,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_new_stored_resource() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            TenantId::new("tenant-1"),
            json!({"resourceType": "Patient", "id": "123"}),
            FhirVersion::default(),
        );

        assert_eq!(resource.resource_type(), "Patient");
        assert_eq!(resource.id(), "123");
        assert_eq!(resource.version_id(), "1");
        assert_eq!(resource.tenant_id().as_str(), "tenant-1");
        assert_eq!(resource.fhir_version(), FhirVersion::default());
        assert!(!resource.is_deleted());
    }

    #[test]
    fn test_url_generation() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            TenantId::new("t1"),
            json!({}),
            FhirVersion::default(),
        );

        assert_eq!(resource.url(), "Patient/123");
        assert_eq!(resource.versioned_url(), "Patient/123/_history/1");
    }

    #[test]
    fn test_new_version() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            TenantId::new("t1"),
            json!({"name": "v1"}),
            FhirVersion::default(),
        );

        let updated = resource.new_version(json!({"name": "v2"}), ResourceMethod::Put);

        assert_eq!(updated.version_id(), "2");
        assert_eq!(updated.content()["name"], "v2");
        assert_eq!(updated.method(), Some(ResourceMethod::Put));
        assert_eq!(updated.fhir_version(), FhirVersion::default());
    }

    #[test]
    fn test_mark_deleted() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            TenantId::new("t1"),
            json!({}),
            FhirVersion::default(),
        );

        let deleted = resource.mark_deleted();

        assert!(deleted.is_deleted());
        assert!(deleted.deleted_at().is_some());
        assert_eq!(deleted.version_id(), "2");
        assert_eq!(deleted.method(), Some(ResourceMethod::Delete));
        assert_eq!(deleted.fhir_version(), FhirVersion::default());
    }

    #[test]
    fn test_etag_matching() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            TenantId::new("t1"),
            json!({}),
            FhirVersion::default(),
        );

        assert!(resource.matches_etag("W/\"1\""));
        assert!(resource.matches_etag("\"1\""));
        assert!(resource.matches_etag("1"));
        assert!(!resource.matches_etag("2"));
    }

    #[test]
    fn test_builder() {
        let resource = StoredResourceBuilder::new()
            .resource_type("Observation")
            .id("obs-1")
            .tenant_id(TenantId::new("t1"))
            .content(json!({}))
            .version_id("5")
            .method(ResourceMethod::Put)
            .build();

        assert_eq!(resource.resource_type(), "Observation");
        assert_eq!(resource.version_id(), "5");
        assert_eq!(resource.method(), Some(ResourceMethod::Put));
        assert_eq!(resource.fhir_version(), FhirVersion::default());
    }

    #[test]
    fn test_serde_roundtrip() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            TenantId::new("t1"),
            json!({"resourceType": "Patient"}),
            FhirVersion::default(),
        );

        let json = serde_json::to_string(&resource).unwrap();
        let parsed: StoredResource = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.resource_type(), resource.resource_type());
        assert_eq!(parsed.id(), resource.id());
        assert_eq!(parsed.version_id(), resource.version_id());
        assert_eq!(parsed.fhir_version(), resource.fhir_version());
    }
}
