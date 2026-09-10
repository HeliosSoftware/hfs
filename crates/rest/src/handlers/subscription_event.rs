//! Subscription event emission helper.
//!
//! Constructs a `ResourceEvent` from handler context and dispatches it to the
//! subscription engine.

use std::sync::Arc;

use helios_fhir::FhirVersion;
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use helios_subscriptions::{ResourceEvent, ResourceEventType, SubscriptionEngine};
use tracing::debug;

/// Emits a subscription event for a successful resource write.
///
/// This function constructs a `ResourceEvent` from the handler context and
/// spawns the subscription engine's `on_resource_event` asynchronously.
/// It is a no-op if the subscription engine is not configured.
pub fn emit_subscription_event(
    engine: &Arc<SubscriptionEngine>,
    tenant: &TenantContext,
    stored: &StoredResource,
    fhir_version: FhirVersion,
    event_type: ResourceEventType,
) {
    let event = ResourceEvent {
        tenant_id: tenant.tenant_id().clone(),
        fhir_version,
        resource_type: stored.resource_type().to_string(),
        resource_id: stored.id().to_string(),
        version_id: stored.version_id().to_string(),
        event_type,
        resource: Some(stored.content_with_meta()),
        previous_resource: None,
        timestamp: chrono::Utc::now(),
    };

    let engine = Arc::clone(engine);

    debug!(
        resource_type = %event.resource_type,
        resource_id = %event.resource_id,
        event_type = %event.event_type,
        "Emitting subscription event"
    );

    tokio::spawn(async move {
        engine.on_resource_event(event).await;
    });
}

/// Emits a create/update subscription event from an already-serialized
/// resource.
///
/// The atomic transaction path returns each committed entry as JSON
/// (`content_with_meta`) rather than a [`StoredResource`], so this builds the
/// event from the resource's own `resourceType` / `id` / `meta.versionId`.
/// It is a no-op if any of those are absent.
pub fn emit_subscription_event_from_json(
    engine: &Arc<SubscriptionEngine>,
    tenant: &TenantContext,
    resource: &serde_json::Value,
    fhir_version: FhirVersion,
    event_type: ResourceEventType,
) {
    let (Some(resource_type), Some(resource_id)) = (
        resource
            .get("resourceType")
            .and_then(serde_json::Value::as_str),
        resource.get("id").and_then(serde_json::Value::as_str),
    ) else {
        return;
    };
    let version_id = resource
        .get("meta")
        .and_then(|m| m.get("versionId"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();

    let event = ResourceEvent {
        tenant_id: tenant.tenant_id().clone(),
        fhir_version,
        resource_type: resource_type.to_string(),
        resource_id: resource_id.to_string(),
        version_id,
        event_type,
        resource: Some(resource.clone()),
        previous_resource: None,
        timestamp: chrono::Utc::now(),
    };

    let engine = Arc::clone(engine);

    debug!(
        resource_type = %event.resource_type,
        resource_id = %event.resource_id,
        event_type = %event.event_type,
        "Emitting subscription event (transaction)"
    );

    tokio::spawn(async move {
        engine.on_resource_event(event).await;
    });
}

/// Emits a subscription event for a resource delete.
///
/// Delete events carry the resource type and ID but no resource content.
pub fn emit_delete_event(
    engine: &Arc<SubscriptionEngine>,
    tenant: &TenantContext,
    resource_type: &str,
    resource_id: &str,
    fhir_version: FhirVersion,
    previous_resource: Option<serde_json::Value>,
) {
    let event = ResourceEvent {
        tenant_id: tenant.tenant_id().clone(),
        fhir_version,
        resource_type: resource_type.to_string(),
        resource_id: resource_id.to_string(),
        version_id: String::new(),
        event_type: ResourceEventType::Delete,
        resource: None,
        previous_resource,
        timestamp: chrono::Utc::now(),
    };

    let engine = Arc::clone(engine);

    debug!(
        resource_type = %event.resource_type,
        resource_id = %event.resource_id,
        "Emitting subscription delete event"
    );

    tokio::spawn(async move {
        engine.on_resource_event(event).await;
    });
}
