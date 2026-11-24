//! Scope checks for the SQL-on-FHIR operations.
//!
//! `$sql-run` and `$sql-export` are mounted at the system level, so the
//! authorization middleware cannot name the resource types a request reads:
//! they are whatever the resolved subject names — a ViewDefinition's
//! `resource`, or the `resource` of every leaf ViewDefinition in a Library's
//! dependency graph. Each of those types needs the same Search grant a plain
//! search on it would, judged by [`SmartScopePolicy`] exactly as the
//! middleware judges `GET /{type}`. A grant that rests on a `patient/` scope
//! is refused: a view cannot be confined to the launch patient's compartment.

use helios_audit::{AuditAction, AuditEventBuilder};
use helios_auth::{AccessScope, FhirOperation, Principal, SmartScopePolicy};
use helios_persistence::tenant::TenantContext;
use serde_json::Value;

use super::graph::{GraphPlan, PlanNode};
use crate::audit::AttributeToPrincipal;
use crate::error::RestError;
use crate::state::AppState;

/// The resource type a ViewDefinition reads, when it names one.
pub(super) fn view_resource_type(view: &Value) -> Option<&str> {
    view.get("resource").and_then(|v| v.as_str())
}

/// The resource type of every leaf ViewDefinition a resolved plan reads.
pub(super) fn plan_resource_types(plan: &GraphPlan) -> impl Iterator<Item = &str> {
    plan.nodes.iter().filter_map(|node| match node {
        PlanNode::Leaf { view, .. } => view_resource_type(view),
        PlanNode::SqlView { .. } | PlanNode::Terminology { .. } => None,
    })
}

/// [`check_read_scopes`] for `subject`, with the decision recorded as an
/// `AuditEvent` when an audit sink is configured: outcome `0` naming every
/// resource type the operation reads, or `8` for a refusal.
///
/// Neither middleware records these operations. The authorization middleware
/// leaves them to their handler, and the audit middleware leaves every `POST`
/// without a resource type to the batch handler. So this event is their only
/// trace in the audit trail.
pub(super) async fn authorize_reads<'a, S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    principal: Option<&Principal>,
    operation: &str,
    subject: &Value,
    resource_types: impl IntoIterator<Item = &'a str>,
) -> Result<(), RestError>
where
    S: helios_persistence::core::ResourceStorage,
{
    let mut types: Vec<&str> = resource_types.into_iter().collect();
    types.sort_unstable();
    types.dedup();
    let decision = check_read_scopes(principal, operation, types.iter().copied());
    let Some(sink) = state.audit_sink() else {
        return decision;
    };
    let reads = if types.is_empty() {
        "no resource type".to_string()
    } else {
        types.join(", ")
    };
    let (outcome, verdict) = match &decision {
        Ok(()) => ("0", "Granted"),
        Err(_) => ("8", "Forbidden"),
    };
    let mut event = AuditEventBuilder::new(state.audit_source_observer())
        .action(AuditAction::Execute)
        .outcome(outcome)
        .outcome_desc(format!("{verdict}: {operation} reading {reads}"))
        .tenant(tenant.tenant_id().as_str())
        .detail("audit-operation", operation.trim_start_matches('$'));
    for resource_type in &types {
        event = event.detail("resource-type", *resource_type);
    }
    if let Some(label) = subject_label(subject) {
        event = event.detail("subject", label);
    }
    if let Some(principal) = principal {
        event = event.principal(principal);
    }
    sink.record(event.build()).await;
    decision
}

/// How an audit event names a subject: its canonical `url`, else
/// `{resourceType}/{id}`; `None` for an inline subject with neither.
fn subject_label(subject: &Value) -> Option<String> {
    if let Some(url) = subject.get("url").and_then(Value::as_str) {
        return Some(url.to_string());
    }
    let resource_type = subject.get("resourceType")?.as_str()?;
    let id = subject.get("id")?.as_str()?;
    Some(format!("{resource_type}/{id}"))
}

/// Requires an unrestricted Search grant on every resource type `operation`
/// reads. With authentication disabled there is no principal and nothing to
/// check.
pub(super) fn check_read_scopes<'a>(
    principal: Option<&Principal>,
    operation: &str,
    resource_types: impl IntoIterator<Item = &'a str>,
) -> Result<(), RestError> {
    let Some(principal) = principal else {
        return Ok(());
    };
    for resource_type in resource_types {
        match SmartScopePolicy::check(principal, resource_type, FhirOperation::Search) {
            Ok(AccessScope::Unrestricted) => {}
            Ok(AccessScope::PatientCompartment { .. }) => {
                return Err(RestError::Forbidden {
                    message: format!(
                        "{operation} reads {resource_type}, which this token is granted only by a \
                         patient/ scope; a view cannot be confined to the launch patient's \
                         compartment"
                    ),
                });
            }
            Err(error) => {
                return Err(RestError::Forbidden {
                    message: format!("{operation} reads {resource_type}: {error}"),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_auth::ScopeSet;
    use serde_json::json;

    fn principal(scopes: &str) -> Principal {
        Principal {
            subject: "sub".to_string(),
            issuer: "https://idp.example.com".to_string(),
            scopes: ScopeSet::parse(scopes),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            ..Default::default()
        }
    }

    #[test]
    fn no_principal_is_allowed() {
        assert!(check_read_scopes(None, "$sql-run", ["Observation"]).is_ok());
    }

    #[test]
    fn a_matching_user_scope_is_allowed() {
        let p = principal("user/Observation.rs");
        assert!(check_read_scopes(Some(&p), "$sql-run", ["Observation"]).is_ok());
    }

    #[test]
    fn a_wildcard_scope_covers_every_type() {
        let p = principal("system/*.rs");
        assert!(check_read_scopes(Some(&p), "$sql-run", ["Observation", "Patient"]).is_ok());
    }

    #[test]
    fn an_unrelated_scope_is_refused() {
        let p = principal("user/Appointment.rs");
        let err = check_read_scopes(Some(&p), "$sql-run", ["Observation"]).unwrap_err();
        assert!(
            matches!(err, RestError::Forbidden { ref message } if message.contains("Observation"))
        );
    }

    #[test]
    fn no_resource_scope_is_refused() {
        let p = principal("openid");
        assert!(check_read_scopes(Some(&p), "$sql-run", ["Observation"]).is_err());
    }

    #[test]
    fn a_patient_scope_alone_is_refused() {
        let p = principal("patient/*.rs");
        let err = check_read_scopes(Some(&p), "$sql-run", ["Observation"]).unwrap_err();
        assert!(matches!(err, RestError::Forbidden { .. }));
    }

    #[test]
    fn one_ungranted_type_refuses_the_whole_set() {
        let p = principal("user/Observation.rs");
        assert!(check_read_scopes(Some(&p), "$sql-run", ["Observation", "Patient"]).is_err());
    }

    #[test]
    fn a_subject_is_named_by_its_canonical_then_its_id() {
        assert_eq!(
            subject_label(&json!({"resourceType": "Library", "id": "q", "url": "http://x/q"}))
                .as_deref(),
            Some("http://x/q")
        );
        assert_eq!(
            subject_label(&json!({"resourceType": "ViewDefinition", "id": "v"})).as_deref(),
            Some("ViewDefinition/v")
        );
        assert_eq!(
            subject_label(&json!({"resourceType": "ViewDefinition"})),
            None
        );
    }

    #[test]
    fn view_resource_type_reads_the_resource_element() {
        assert_eq!(
            view_resource_type(&json!({"resourceType": "ViewDefinition", "resource": "Patient"})),
            Some("Patient")
        );
        assert_eq!(
            view_resource_type(&json!({"resourceType": "ViewDefinition"})),
            None
        );
    }
}
