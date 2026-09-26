//! The subscriptions engine as a post-commit [`WriteObserver`] (#1078).
//!
//! The server's write paths report every committed write once, to one
//! [`WriteObservers`](helios_persistence::core::WriteObservers) fan-out; this
//! adapter subscribes the engine to it. It turns each resource write that
//! carries a [`WriteNotice`] into the [`ResourceEvent`] the handlers used to
//! build by hand, and hands it to
//! [`SubscriptionEngine::on_resource_event`] on a spawned task.
//!
//! What gets announced is decided by the write path, not here: a write whose
//! notice is `None` (for example a conditional create) is deliberately silent,
//! and aggregate [`WriteEvent::Counts`] and purge [`WriteEvent::Erased`] events
//! never produce a notification.

use std::sync::Arc;

use helios_persistence::core::{WriteEvent, WriteKind, WriteObserver};
use tracing::debug;

use crate::engine::SubscriptionEngine;
use crate::event::{ResourceEvent, ResourceEventType};

/// Feeds committed resource writes to a [`SubscriptionEngine`].
pub struct SubscriptionWriteObserver {
    engine: Arc<SubscriptionEngine>,
}

impl SubscriptionWriteObserver {
    /// Wraps the engine that should receive resource notifications.
    pub fn new(engine: Arc<SubscriptionEngine>) -> Self {
        Self { engine }
    }
}

impl std::fmt::Debug for SubscriptionWriteObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionWriteObserver").finish()
    }
}

/// The engine event a write announces, or `None` when it announces nothing:
/// an aggregate or purge event, a write without a notice, or a notice with no
/// resource id.
fn resource_event(event: &WriteEvent) -> Option<ResourceEvent> {
    let WriteEvent::Resource(write) = event else {
        return None;
    };
    let notice = write.notice.as_ref()?;
    if notice.resource_id.is_empty() {
        return None;
    }
    Some(ResourceEvent {
        tenant_id: write.tenant.clone(),
        fhir_version: write.fhir_version,
        resource_type: write.resource_type.clone(),
        resource_id: notice.resource_id.clone(),
        version_id: notice.version_id.clone(),
        event_type: match notice.kind {
            WriteKind::Create => ResourceEventType::Create,
            WriteKind::Update => ResourceEventType::Update,
            WriteKind::Delete => ResourceEventType::Delete,
        },
        resource: notice.resource.clone(),
        previous_resource: notice.previous.clone(),
        timestamp: write.at,
    })
}

impl WriteObserver for SubscriptionWriteObserver {
    fn on_write(&self, event: &WriteEvent) {
        let Some(event) = resource_event(event) else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            debug!(
                resource_type = %event.resource_type,
                resource_id = %event.resource_id,
                "No Tokio runtime; subscription event not dispatched"
            );
            return;
        };
        debug!(
            resource_type = %event.resource_type,
            resource_id = %event.resource_id,
            event_type = %event.event_type,
            "Emitting subscription event"
        );
        let engine = Arc::clone(&self.engine);
        runtime.spawn(async move {
            engine.on_resource_event(event).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use helios_fhir::FhirVersion;
    use helios_persistence::core::{ErasedScope, ResourceWrite, WriteNotice, WriteOrigin};
    use helios_persistence::tenant::TenantId;
    use serde_json::json;

    use crate::config::SubscriptionConfig;

    fn write(notice: Option<WriteNotice>) -> WriteEvent {
        WriteEvent::Resource(ResourceWrite {
            tenant: TenantId::new("acme".to_string()),
            fhir_version: FhirVersion::default(),
            resource_type: "Patient".to_string(),
            live_delta: 1,
            notice,
            at: Utc::now(),
        })
    }

    fn notice(kind: WriteKind, id: &str) -> WriteNotice {
        WriteNotice {
            kind,
            resource_id: id.to_string(),
            version_id: "2".to_string(),
            resource: Some(json!({"resourceType": "Patient", "id": id})),
            previous: Some(json!({"resourceType": "Patient", "id": id, "active": true})),
        }
    }

    #[test]
    fn a_notice_becomes_the_same_engine_event_the_handlers_built() {
        let event = write(Some(notice(WriteKind::Update, "p1")));
        let WriteEvent::Resource(source) = &event else {
            unreachable!()
        };
        let built = resource_event(&event).expect("announced");
        assert_eq!(built.tenant_id.as_str(), "acme");
        assert_eq!(built.fhir_version, FhirVersion::default());
        assert_eq!(built.resource_type, "Patient");
        assert_eq!(built.resource_id, "p1");
        assert_eq!(built.version_id, "2");
        assert_eq!(built.event_type, ResourceEventType::Update);
        assert_eq!(
            built.resource,
            Some(json!({"resourceType": "Patient", "id": "p1"}))
        );
        assert_eq!(
            built.previous_resource,
            Some(json!({"resourceType": "Patient", "id": "p1", "active": true}))
        );
        assert_eq!(built.timestamp, source.at);

        for (kind, expected) in [
            (WriteKind::Create, ResourceEventType::Create),
            (WriteKind::Delete, ResourceEventType::Delete),
        ] {
            let built = resource_event(&write(Some(notice(kind, "p1")))).expect("announced");
            assert_eq!(built.event_type, expected);
        }
    }

    #[test]
    fn silent_writes_counts_and_purges_announce_nothing() {
        assert!(resource_event(&write(None)).is_none(), "no notice");
        assert!(
            resource_event(&write(Some(notice(WriteKind::Create, "")))).is_none(),
            "empty resource id"
        );
        assert!(
            resource_event(&WriteEvent::Counts {
                tenant: TenantId::new("acme".to_string()),
                resource_type: "Patient".to_string(),
                created: 3,
                updated: 1,
                deleted: 1,
                origin: WriteOrigin::BulkSubmit,
                at: Utc::now(),
            })
            .is_none()
        );
        assert!(
            resource_event(&WriteEvent::Erased {
                tenant: TenantId::new("acme".to_string()),
                scope: ErasedScope::Tenant,
            })
            .is_none()
        );
    }

    fn engine() -> Arc<SubscriptionEngine> {
        Arc::new(SubscriptionEngine::new(
            SubscriptionConfig::default(),
            "http://localhost:8080".to_string(),
        ))
    }

    #[test]
    fn without_a_runtime_the_event_is_dropped_not_panicked() {
        let observer = SubscriptionWriteObserver::new(engine());
        observer.on_write(&write(Some(notice(WriteKind::Create, "p1"))));
    }

    /// End to end through the engine: a Subscription create notice registers
    /// it, a delete notice deregisters it, and a notice-less write does
    /// neither.
    #[tokio::test]
    async fn announced_writes_reach_the_engine_and_silent_ones_do_not() {
        use crate::topics::{ResourceTrigger, TopicDefinition};
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let hook = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&hook)
            .await;

        let engine = engine();
        let topic_url = "http://example.org/topic/patient-create";
        engine.topic_registry().add_topic(TopicDefinition {
            canonical_url: topic_url.to_string(),
            title: None,
            resource_triggers: vec![ResourceTrigger {
                resource_type: "Patient".to_string(),
                interactions: vec![ResourceEventType::Create],
                fhirpath_criteria: None,
            }],
            can_filter_by: vec![],
            notification_shape: vec![],
        });
        let subscription = crate::manager::tests::build_subscription_json(
            topic_url,
            "rest-hook",
            Some(&format!("{}/hook", hook.uri())),
        );
        let observer = SubscriptionWriteObserver::new(Arc::clone(&engine));
        let subscription_write = |kind: WriteKind, announced: bool| {
            WriteEvent::Resource(ResourceWrite {
                tenant: TenantId::new("acme".to_string()),
                fhir_version: FhirVersion::default(),
                resource_type: "Subscription".to_string(),
                live_delta: 0,
                notice: announced.then(|| WriteNotice {
                    kind,
                    resource_id: "sub-1".to_string(),
                    version_id: "1".to_string(),
                    resource: (kind != WriteKind::Delete).then(|| subscription.clone()),
                    previous: None,
                }),
                at: Utc::now(),
            })
        };
        let registered = || engine.manager().get_subscription("acme", "sub-1").is_some();
        async fn settle(check: impl Fn() -> bool) -> bool {
            for _ in 0..200 {
                if check() {
                    return true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            false
        }

        observer.on_write(&subscription_write(WriteKind::Create, false));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!registered(), "a write without a notice is silent");

        observer.on_write(&subscription_write(WriteKind::Create, true));
        assert!(settle(registered).await, "the create notice registered it");

        observer.on_write(&subscription_write(WriteKind::Delete, true));
        assert!(
            settle(|| !registered()).await,
            "the delete notice deregistered it"
        );
    }
}
