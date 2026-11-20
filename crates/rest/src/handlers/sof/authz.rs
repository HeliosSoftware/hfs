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

use helios_auth::{AccessScope, FhirOperation, Principal, SmartScopePolicy};
use serde_json::Value;

use super::graph::{GraphPlan, PlanNode};
use crate::error::RestError;

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
