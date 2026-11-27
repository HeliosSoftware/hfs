//! Scope checks for the SQL-on-FHIR operations.
//!
//! `$sql-run` and `$sql-export` are mounted at the system level, so the
//! authorization middleware cannot name the resource types a request reads:
//! they are whatever the resolved subject names — a ViewDefinition's
//! `resource`, the `resource` of every leaf ViewDefinition in a Library's
//! dependency graph, and `ValueSet` or `ConceptMap` for each of its
//! terminology dependencies. Each of those types needs the same Search grant
//! a plain search on it would, judged by [`SmartScopePolicy`] exactly as the
//! middleware judges `GET /{type}`. A grant that rests on a `patient/` scope
//! is refused: a view cannot be confined to the launch patient's compartment.
//!
//! Which of the two a terminology dependency is, is only known once it is
//! resolved, from the persisted table or from the terminology server. So a
//! token that may search neither is refused before either is consulted
//! ([`check_terminology_reachable`]), and the resolved kind is checked with
//! the rest of the plan ([`plan_resource_types`]).

use helios_auth::{AccessScope, FhirOperation, Principal, SmartScopePolicy};
use serde_json::Value;

use super::graph::{GraphPlan, PlanNode};
use crate::error::RestError;

/// The resource type a ViewDefinition reads, when it names one.
pub(super) fn view_resource_type(view: &Value) -> Option<&str> {
    view.get("resource").and_then(|v| v.as_str())
}

/// The resource type of every node a resolved plan reads: the `resource` of
/// a leaf ViewDefinition, and `ValueSet` or `ConceptMap` for a terminology
/// dependency.
pub(super) fn plan_resource_types(plan: &GraphPlan) -> impl Iterator<Item = &str> {
    plan.nodes.iter().filter_map(|node| match node {
        PlanNode::Leaf { view, .. } => view_resource_type(view),
        PlanNode::Terminology { relation, .. } => Some(relation.kind.as_str()),
        PlanNode::SqlView { .. } => None,
    })
}

/// The resource types a terminology dependency may turn out to be.
const TERMINOLOGY_TYPES: [&str; 2] = ["ValueSet", "ConceptMap"];

/// Refuses, before terminology dependency `url` is resolved, a token that
/// could not read it whichever kind it turns out to be: one with no
/// unrestricted Search grant on `ValueSet` nor on `ConceptMap`. Neither the
/// persisted table nor the terminology server is consulted for such a token.
/// With authentication disabled there is no principal and nothing to check.
pub(super) fn check_terminology_reachable(
    principal: Option<&Principal>,
    operation: &str,
    url: &str,
) -> Result<(), RestError> {
    let Some(principal) = principal else {
        return Ok(());
    };
    let granted = TERMINOLOGY_TYPES.iter().any(|resource_type| {
        matches!(
            SmartScopePolicy::check(principal, resource_type, FhirOperation::Search),
            Ok(AccessScope::Unrestricted)
        )
    });
    if granted {
        Ok(())
    } else {
        Err(RestError::Forbidden {
            message: format!(
                "{operation} depends on '{url}', which is not a stored ViewDefinition or Library. \
                 Resolving it as a ValueSet or ConceptMap needs an unrestricted search grant on \
                 one of the two, and this token has neither"
            ),
        })
    }
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
    use helios_sof::sqlquery::{TerminologyKind, TerminologyRelation};
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

    fn plan(nodes: Vec<PlanNode>) -> GraphPlan {
        GraphPlan {
            nodes,
            subject_edges: Vec::new(),
        }
    }

    fn terminology_node(kind: TerminologyKind) -> PlanNode {
        PlanNode::Terminology {
            internal_name: "t0".to_string(),
            relation: TerminologyRelation {
                kind,
                rows: Vec::new(),
            },
            name: "http://example.org/terminology".to_string(),
        }
    }

    #[test]
    fn a_terminology_dependency_reads_its_kind() {
        let p = plan(vec![
            PlanNode::Leaf {
                internal_name: "v0".to_string(),
                view: json!({"resourceType": "ViewDefinition", "resource": "Patient"}),
            },
            terminology_node(TerminologyKind::ValueSet),
            terminology_node(TerminologyKind::ConceptMap),
        ]);
        assert_eq!(
            plan_resource_types(&p).collect::<Vec<_>>(),
            ["Patient", "ValueSet", "ConceptMap"]
        );
    }

    #[test]
    fn a_plan_of_terminology_alone_still_needs_a_grant() {
        let p = plan(vec![terminology_node(TerminologyKind::ValueSet)]);
        let openid = principal("openid");
        let err =
            check_read_scopes(Some(&openid), "$sql-run", plan_resource_types(&p)).unwrap_err();
        assert!(
            matches!(err, RestError::Forbidden { ref message } if message.contains("ValueSet"))
        );
        let concept_maps = principal("user/ConceptMap.rs");
        assert!(
            check_read_scopes(Some(&concept_maps), "$sql-run", plan_resource_types(&p)).is_err(),
            "a ConceptMap grant does not read a ValueSet"
        );
        let value_sets = principal("user/ValueSet.rs");
        assert!(check_read_scopes(Some(&value_sets), "$sql-run", plan_resource_types(&p)).is_ok());
    }

    #[test]
    fn a_terminology_dependency_is_reached_only_with_a_grant_on_either_kind() {
        let url = "http://example.org/vs";
        assert!(check_terminology_reachable(None, "$sql-run", url).is_ok());
        for scopes in ["user/ValueSet.rs", "user/ConceptMap.rs", "system/*.rs"] {
            assert!(
                check_terminology_reachable(Some(&principal(scopes)), "$sql-run", url).is_ok(),
                "{scopes}"
            );
        }
        for scopes in [
            "openid",
            "user/Observation.rs",
            "patient/*.rs",
            "user/ValueSet.c",
        ] {
            let err = check_terminology_reachable(Some(&principal(scopes)), "$sql-export", url)
                .unwrap_err();
            assert!(
                matches!(err, RestError::Forbidden { ref message }
                    if message.contains("$sql-export") && message.contains(url)),
                "{scopes}"
            );
        }
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
