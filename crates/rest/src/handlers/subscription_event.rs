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
/// dispatches it to the subscription engine. Topic lifecycle resources are
/// handled inline so their in-memory state is visible before the HTTP response
/// returns; subscriptions and ordinary data-resource events remain
/// fire-and-forget.
/// It is a no-op if the subscription engine is not configured.
pub async fn emit_subscription_event(
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
        resource: Some(stored.content().clone()),
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

    if should_handle_inline(&event.resource_type, event.fhir_version) {
        engine.on_resource_event(event).await;
        return;
    }

    tokio::spawn(async move {
        engine.on_resource_event(event).await;
    });
}

/// Emits a subscription event for a resource delete.
///
/// Delete events carry the resource type and ID but no resource content.
pub async fn emit_delete_event(
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

    if should_handle_inline(&event.resource_type, event.fhir_version) {
        engine.on_resource_event(event).await;
        return;
    }

    tokio::spawn(async move {
        engine.on_resource_event(event).await;
    });
}

fn should_handle_inline(resource_type: &str, fhir_version: FhirVersion) -> bool {
    match resource_type {
        "SubscriptionTopic" => true,
        #[cfg(feature = "R4")]
        "Basic" if fhir_version == FhirVersion::R4 => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_lifecycle_resources_are_inline() {
        let version = FhirVersion::default();

        assert!(should_handle_inline("SubscriptionTopic", version));
    }

    #[test]
    fn subscriptions_are_not_inline() {
        assert!(!should_handle_inline(
            "Subscription",
            FhirVersion::default()
        ));
    }

    #[test]
    fn ordinary_resource_events_are_not_inline() {
        assert!(!should_handle_inline("Encounter", FhirVersion::default()));
    }

    #[cfg(feature = "R4")]
    #[test]
    fn r4_basic_events_are_inline_for_backport_topic_detection() {
        assert!(should_handle_inline("Basic", FhirVersion::R4));
    }
}
