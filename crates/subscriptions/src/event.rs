//! Resource event types emitted after write operations.

use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use helios_persistence::tenant::TenantId;

/// An event emitted after a resource write operation (create, update, or delete).
///
/// This is the input to the subscription engine's evaluation pipeline.
#[derive(Debug, Clone)]
pub struct ResourceEvent {
    /// The tenant that owns the resource.
    pub tenant_id: TenantId,

    /// The FHIR version of the resource.
    pub fhir_version: FhirVersion,

    /// The resource type (e.g., "Patient", "Observation").
    pub resource_type: String,

    /// The resource ID.
    pub resource_id: String,

    /// The version ID after the write operation.
    pub version_id: String,

    /// The type of interaction that produced this event.
    pub event_type: ResourceEventType,

    /// The resource content after the interaction (`None` for delete).
    pub resource: Option<serde_json::Value>,

    /// The resource content before the interaction (`None` for create).
    pub previous_resource: Option<serde_json::Value>,

    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
}

/// The type of interaction that produced a resource event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceEventType {
    /// A new resource was created.
    Create,
    /// An existing resource was updated (including patch).
    Update,
    /// A resource was deleted.
    Delete,
}

impl std::fmt::Display for ResourceEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Create => write!(f, "create"),
            Self::Update => write!(f, "update"),
            Self::Delete => write!(f, "delete"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_display() {
        assert_eq!(ResourceEventType::Create.to_string(), "create");
        assert_eq!(ResourceEventType::Update.to_string(), "update");
        assert_eq!(ResourceEventType::Delete.to_string(), "delete");
    }
}
