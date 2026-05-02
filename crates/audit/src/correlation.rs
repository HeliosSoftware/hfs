//! Correlated audit events for batch and transaction bundles.
//!
//! When HFS processes a FHIR batch or transaction bundle, the outer HTTP request
//! generates a single audit event via the middleware.  This module adds per-entry
//! audit events that share a correlation ID, so the full set of operations within
//! the bundle can be traced as a group.

use helios_fhir::r4::AuditEvent;

use crate::balp::AuditAction;
use crate::builder::AuditEventBuilder;

/// Correlation context linking audit events within a single bundle.
#[derive(Debug, Clone)]
pub struct AuditCorrelation {
    /// Unique identifier for this bundle (UUID v4).
    pub bundle_id: String,
    /// Bundle type: `"batch"` or `"transaction"`.
    pub bundle_type: String,
}

impl AuditCorrelation {
    /// Create a new correlation context with a generated UUID.
    pub fn new(bundle_type: &str) -> Self {
        Self {
            bundle_id: uuid::Uuid::new_v4().to_string(),
            bundle_type: bundle_type.to_string(),
        }
    }
}

/// Lightweight description of a single bundle entry for audit purposes.
#[derive(Debug, Clone)]
pub struct BundleAuditEntry {
    /// HTTP method from the bundle entry request (e.g. `"POST"`, `"PUT"`).
    pub method: String,
    /// FHIR resource type, if identifiable.
    pub resource_type: Option<String>,
    /// Resource logical ID, if identifiable.
    pub resource_id: Option<String>,
    /// HTTP status code from the entry response.
    pub status: u16,
    /// Patient reference, if resolved.
    pub patient_ref: Option<String>,
}

/// Build an audit event for a single entry within a correlated bundle.
///
/// The resulting event carries `bundle-id`, `bundle-type`, and `entry-index`
/// details so it can be linked to the parent bundle and sibling entries.
pub fn build_entry_event(
    correlation: &AuditCorrelation,
    source_observer: &str,
    entry_index: usize,
    entry: &BundleAuditEntry,
    agent: Option<&str>,
) -> AuditEvent {
    let action = match entry.method.to_ascii_uppercase().as_str() {
        "POST" => AuditAction::Create,
        "GET" | "HEAD" => AuditAction::Read,
        "PUT" | "PATCH" => AuditAction::Update,
        "DELETE" => AuditAction::Delete,
        _ => AuditAction::Execute,
    };
    let outcome = if entry.status < 400 { "0" } else { "8" };

    let mut builder = AuditEventBuilder::new(source_observer)
        .action(action)
        .outcome(outcome)
        .detail("bundle-id", &correlation.bundle_id)
        .detail("bundle-type", &correlation.bundle_type)
        .detail("entry-index", entry_index.to_string());

    if let (Some(rt), Some(rid)) = (&entry.resource_type, &entry.resource_id) {
        builder = builder.resource(rt, rid);
    }
    if let Some(ref p) = entry.patient_ref {
        builder = builder.patient(p);
    }
    if let Some(a) = agent {
        builder = builder.agent(a, None, true);
    }

    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::balp;

    #[test]
    fn test_correlation_id_generated() {
        let c = AuditCorrelation::new("transaction");
        assert_eq!(c.bundle_id.len(), 36); // UUID format
        assert_eq!(c.bundle_type, "transaction");
    }

    #[test]
    fn test_entry_event_has_bundle_details() {
        let correlation = AuditCorrelation::new("batch");
        let entry = BundleAuditEntry {
            method: "POST".to_string(),
            resource_type: Some("Patient".to_string()),
            resource_id: Some("123".to_string()),
            status: 201,
            patient_ref: None,
        };
        let event = build_entry_event(&correlation, "Device/hfs", 0, &entry, None);
        let entities = event.entity.as_ref().unwrap();
        let details = entities[0].detail.as_ref().unwrap();

        // Should have bundle-id, bundle-type, entry-index
        assert_eq!(details.len(), 3);
        assert_eq!(details[0].r#type.value.as_deref(), Some("bundle-id"));
        assert_eq!(details[1].r#type.value.as_deref(), Some("bundle-type"));
        assert_eq!(details[2].r#type.value.as_deref(), Some("entry-index"));
    }

    #[test]
    fn test_entry_event_profile_selection() {
        let correlation = AuditCorrelation::new("transaction");

        // POST → Create action
        let create_entry = BundleAuditEntry {
            method: "POST".to_string(),
            resource_type: Some("Observation".to_string()),
            resource_id: Some("obs-1".to_string()),
            status: 201,
            patient_ref: None,
        };
        let event = build_entry_event(&correlation, "Device/hfs", 0, &create_entry, None);
        let profiles = event.meta.as_ref().unwrap().profile.as_ref().unwrap();
        assert_eq!(profiles[0].value.as_deref(), Some(balp::profiles::CREATE));

        // DELETE → Delete action
        let delete_entry = BundleAuditEntry {
            method: "DELETE".to_string(),
            resource_type: Some("Observation".to_string()),
            resource_id: Some("obs-1".to_string()),
            status: 204,
            patient_ref: None,
        };
        let event = build_entry_event(&correlation, "Device/hfs", 1, &delete_entry, None);
        let profiles = event.meta.as_ref().unwrap().profile.as_ref().unwrap();
        assert_eq!(profiles[0].value.as_deref(), Some(balp::profiles::DELETE));
    }

    #[test]
    fn test_batch_vs_transaction_type() {
        let batch = AuditCorrelation::new("batch");
        let txn = AuditCorrelation::new("transaction");
        assert_eq!(batch.bundle_type, "batch");
        assert_eq!(txn.bundle_type, "transaction");
    }

    #[test]
    fn test_multiple_entries_share_bundle_id() {
        let correlation = AuditCorrelation::new("transaction");
        let entry = BundleAuditEntry {
            method: "PUT".to_string(),
            resource_type: Some("Patient".to_string()),
            resource_id: Some("p1".to_string()),
            status: 200,
            patient_ref: None,
        };

        let e1 = build_entry_event(&correlation, "Device/hfs", 0, &entry, None);
        let e2 = build_entry_event(&correlation, "Device/hfs", 1, &entry, None);

        let get_bundle_id = |event: &AuditEvent| -> String {
            let details = event.entity.as_ref().unwrap()[0].detail.as_ref().unwrap();
            match &details[0].value {
                Some(helios_fhir::r4::AuditEventEntityDetailValue::String(s)) => {
                    s.value.clone().unwrap()
                }
                _ => panic!("Expected string value"),
            }
        };

        assert_eq!(get_bundle_id(&e1), get_bundle_id(&e2));
        assert_eq!(get_bundle_id(&e1), correlation.bundle_id);
    }
}
