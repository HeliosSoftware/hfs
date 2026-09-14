//! Reporting committed writes to the post-commit write observer (#1078).
//!
//! Every handler that commits a write calls [`report`] exactly once, after the
//! storage write succeeded, with two independent facts:
//!
//! - `live_delta` — how the tenant's live count of the type changed (`+1` for
//!   a create, including an update that created or restored a resource, `-1`
//!   for a delete of a live resource, `0` otherwise). The dashboard counters
//!   read this.
//! - `notice` — what resource notifications should say, decided exactly as the
//!   handlers decided subscription events before this funnel existed. `None` is
//!   a deliberate "do not notify": conditional create/update/delete, conditional
//!   patch and conditional batch entries never announced, and still do not.
//!
//! Consumers (the dashboard counters, the subscriptions engine) subscribe to
//! [`AppState::write_observer`]; nothing here depends on which are wired.

use chrono::Utc;
use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceWrite, WriteEvent, WriteKind, WriteNotice, WriteObserver};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use serde_json::Value;

use crate::state::AppState;

/// Reports one committed write of one resource.
pub(crate) fn report<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    fhir_version: FhirVersion,
    resource_type: &str,
    live_delta: i64,
    notice: Option<WriteNotice>,
) where
    S: helios_persistence::core::ResourceStorage,
{
    state
        .write_observer()
        .on_write(&WriteEvent::Resource(ResourceWrite {
            tenant: tenant.tenant_id().clone(),
            fhir_version,
            resource_type: resource_type.to_string(),
            live_delta,
            notice,
            at: Utc::now(),
        }));
}

/// The create/update notice for a stored resource: its id, version and
/// content with meta, with no previous content.
pub(crate) fn stored_notice(kind: WriteKind, stored: &StoredResource) -> WriteNotice {
    WriteNotice {
        kind,
        resource_id: stored.id().to_string(),
        version_id: stored.version_id().to_string(),
        resource: Some(stored.content_with_meta()),
        previous: None,
    }
}

/// The create/update notice for a resource already serialized with meta (the
/// transaction path's results), as `(resource type, notice)`, or `None` when
/// the JSON names no `resourceType` or `id`.
pub(crate) fn json_notice(kind: WriteKind, resource: &Value) -> Option<(String, WriteNotice)> {
    let resource_type = resource.get("resourceType").and_then(Value::as_str)?;
    let resource_id = resource.get("id").and_then(Value::as_str)?;
    let version_id = resource
        .get("meta")
        .and_then(|meta| meta.get("versionId"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some((
        resource_type.to_string(),
        WriteNotice {
            kind,
            resource_id: resource_id.to_string(),
            version_id: version_id.to_string(),
            resource: Some(resource.clone()),
            previous: None,
        },
    ))
}

/// The delete notice: no version, no content, and the pre-delete content when
/// the write path had it.
pub(crate) fn delete_notice(resource_id: &str, previous: Option<Value>) -> WriteNotice {
    WriteNotice {
        kind: WriteKind::Delete,
        resource_id: resource_id.to_string(),
        version_id: String::new(),
        resource: None,
        previous,
    }
}

/// The notice kind of an update that may have created the resource.
pub(crate) fn upsert_kind(created: bool) -> WriteKind {
    if created {
        WriteKind::Create
    } else {
        WriteKind::Update
    }
}
