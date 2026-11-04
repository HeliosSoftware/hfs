//! Confinement of a request to the SMART launch patient's compartment (#1618).
//!
//! A `patient/` scope is access to **one** patient's data: SMART App Launch
//! defines patient-specific scopes as access limited to the launch patient's
//! compartment. [`helios_auth::SmartScopePolicy::check`] therefore answers a
//! request that is granted only by a `patient/` scope with
//! [`AccessScope::PatientCompartment`], and the authorization middleware
//! attaches that grant to the request. Handlers read it back through the
//! [`PatientCompartment`] extractor and confine what they do:
//!
//! - a type-level search gets the Patient compartment filter the compartment
//!   search (`GET /Patient/{id}/{type}`) already uses, and a search *for*
//!   Patient is pinned to the launch patient's id ([`confine_query`]);
//! - an instance read, update, patch or delete checks the stored payload's
//!   membership, and a create or update checks the submitted payload
//!   ([`assert_member`]);
//! - resources pulled in by `_include` / `_revinclude` outside the compartment
//!   are dropped ([`retain_members`]).
//!
//! Membership is decided the way bulk export decides it: from the payload,
//! with the parameter set of the bundled `CompartmentDefinition`
//! ([`PatientCompartmentMatcher`]), so a search, an export and an instance
//! read agree on what belongs to a patient. The Patient resource itself is a
//! member of its own compartment by id (the compartment table links other
//! patients to it through `link` only).
//!
//! What a confined request may not do at all (system or type history, other
//! patients' compartments, conditional writes, operations the handlers cannot
//! confine) is refused by the middleware before any handler runs; see
//! `middleware::auth::patient_compartment_path_check`.
//!
//! [`confine_query`]: PatientCompartment::confine_query
//! [`assert_member`]: PatientCompartment::assert_member
//! [`retain_members`]: PatientCompartment::retain_members

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use helios_auth::AccessScope;
use helios_fhir::FhirVersion;
use helios_persistence::core::patient_compartment::PatientCompartmentMatcher;
use helios_persistence::search::SearchParameterRegistry;
use helios_persistence::types::{
    CompartmentMembership, SearchParamType, SearchParameter, SearchPrefix, SearchQuery,
    SearchValue, StoredResource,
};
use parking_lot::RwLock;
use serde_json::Value;

use crate::error::{RestError, RestResult};

/// The Patient compartment a request is confined to, if any.
///
/// `None` inside means the request is not confined: its grant rests on a
/// `user/` or `system/` scope, or authentication is disabled. As an extractor
/// it never fails; it reads the [`AccessScope`] the authorization middleware
/// stored in the request extensions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatientCompartment(Option<String>);

impl PatientCompartment {
    /// A request that is not confined to a compartment.
    pub fn unconfined() -> Self {
        Self(None)
    }

    /// A request confined to the compartment of the patient with this id.
    pub fn confined_to(patient_id: impl Into<String>) -> Self {
        Self(Some(patient_id.into()))
    }

    /// The confinement an access grant implies.
    pub fn from_access_scope(scope: Option<&AccessScope>) -> Self {
        match scope {
            Some(AccessScope::PatientCompartment { patient_id }) => Self::confined_to(patient_id),
            Some(AccessScope::Unrestricted) | None => Self::unconfined(),
        }
    }

    /// The launch patient's logical id when the request is confined.
    pub fn patient_id(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// True when the request is confined to a patient compartment.
    pub fn is_confined(&self) -> bool {
        self.0.is_some()
    }

    /// The compartment reference, `Patient/{id}`, when confined.
    pub fn reference(&self) -> Option<String> {
        self.0.as_ref().map(|id| format!("Patient/{id}"))
    }

    /// Whether `resource` belongs to the compartment. Always true when the
    /// request is not confined.
    ///
    /// A Patient is a member of its own compartment; any other resource is a
    /// member when one of its compartment parameters (the spec
    /// `CompartmentDefinition`, resolved against the tenant's search parameter
    /// registry) references the launch patient. A resource type with no
    /// Patient compartment parameters is never a member: `Binary`,
    /// `Practitioner`, `Organization`, `Medication` and the conformance types
    /// cannot be confined to a patient and are therefore not reachable through
    /// a `patient/` scope.
    ///
    /// `registry` is only called when the request is confined, so an
    /// unconfined request never pays for the registry lookup.
    pub fn is_member(
        &self,
        registry: impl FnOnce() -> Arc<RwLock<SearchParameterRegistry>>,
        version: FhirVersion,
        resource: &Value,
    ) -> bool {
        let Some(patient_id) = self.patient_id() else {
            return true;
        };
        let Some(resource_type) = resource.get("resourceType").and_then(Value::as_str) else {
            return false;
        };
        if resource_type == "Patient"
            && resource.get("id").and_then(Value::as_str) == Some(patient_id)
        {
            return true;
        }
        let matcher = PatientCompartmentMatcher::new(registry(), version, resource_type);
        if matcher.is_empty() {
            return false;
        }
        let refs: HashSet<String> = HashSet::from([format!("Patient/{patient_id}")]);
        matcher.is_member(resource, &refs)
    }

    /// Refuses, with `403`, a resource that is not a member of the compartment.
    ///
    /// `what` names the interaction for the client (`read`, `update`, …).
    pub fn assert_member(
        &self,
        registry: impl FnOnce() -> Arc<RwLock<SearchParameterRegistry>>,
        version: FhirVersion,
        resource: &Value,
        what: &str,
    ) -> RestResult<()> {
        if self.is_member(registry, version, resource) {
            return Ok(());
        }
        let resource_type = resource
            .get("resourceType")
            .and_then(Value::as_str)
            .unwrap_or("resource");
        Err(self.outside_compartment(format!(
            "{what} of this {resource_type} is not permitted: the token's patient/ scope is confined to the launch patient's compartment and the resource is not in it"
        )))
    }

    /// Confines a type-level [`SearchQuery`] to the compartment.
    ///
    /// A search for `Patient` is pinned to the launch patient's id (the
    /// compartment table only links *other* patients through `link`, and a
    /// patient app must not see those). Any other member type gets the same
    /// [`CompartmentMembership`] filter a compartment search applies, ANDed
    /// with the client's own parameters. A type that is not in the Patient
    /// compartment is refused with `403`.
    ///
    /// Does nothing when the request is not confined.
    pub fn confine_query(&self, version: FhirVersion, query: &mut SearchQuery) -> RestResult<()> {
        let Some(patient_id) = self.patient_id() else {
            return Ok(());
        };
        if query.resource_type == "Patient" {
            // A second `_id` parameter is ANDed with any the client sent, so
            // `GET /Patient?_id=<other>` matches nothing rather than leaking.
            query.parameters.push(SearchParameter {
                name: "_id".to_string(),
                param_type: SearchParamType::Token,
                values: vec![SearchValue::new(SearchPrefix::Eq, patient_id)],
                ..Default::default()
            });
            return Ok(());
        }
        let params = helios_fhir::get_compartment_params(version, "Patient", &query.resource_type);
        if params.is_empty() {
            return Err(self.outside_compartment(format!(
                "search on {} is not permitted: the token's patient/ scope is confined to the launch patient's compartment and {} is not a member of the Patient compartment",
                query.resource_type, query.resource_type
            )));
        }
        if query.compartment.is_some() {
            // A compartment search reaching here with another compartment set
            // has slipped past the middleware's path check. Refuse rather than
            // overwrite one filter with the other.
            return Err(self.outside_compartment(
                "search in another compartment is not permitted under a patient/ scope".to_string(),
            ));
        }
        query.compartment = Some(CompartmentMembership {
            params: params.iter().map(|s| s.to_string()).collect(),
            reference: format!("Patient/{patient_id}"),
        });
        Ok(())
    }

    /// Drops the resources that are not members of the compartment.
    ///
    /// For the `_include` / `_revinclude` entries of a confined search: the
    /// matches are confined by the query, but an include follows references
    /// wherever they lead (another patient's Encounter, a Practitioner), and
    /// those are not the launch patient's data.
    pub fn retain_members(
        &self,
        registry: impl FnOnce() -> Arc<RwLock<SearchParameterRegistry>>,
        resources: &mut Vec<StoredResource>,
    ) {
        if !self.is_confined() || resources.is_empty() {
            return;
        }
        let registry = registry();
        resources.retain(|stored| {
            self.is_member(|| registry.clone(), stored.fhir_version(), stored.content())
        });
    }

    /// The `403` for an interaction that would reach outside the compartment.
    pub fn outside_compartment(&self, message: String) -> RestError {
        RestError::Forbidden { message }
    }
}

impl<S> FromRequestParts<S> for PatientCompartment
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_access_scope(
            parts.extensions.get::<AccessScope>(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registry() -> Arc<RwLock<SearchParameterRegistry>> {
        Arc::new(RwLock::new(crate::test_support::spec_registry_r4()))
    }

    #[test]
    fn unconfined_admits_everything() {
        let scope = PatientCompartment::unconfined();
        assert!(!scope.is_confined());
        assert!(scope.is_member(
            registry,
            FhirVersion::R4,
            &json!({"resourceType": "Binary"})
        ));
        let mut query = SearchQuery::new("Binary");
        scope.confine_query(FhirVersion::R4, &mut query).unwrap();
        assert!(query.compartment.is_none());
        assert!(query.parameters.is_empty());
    }

    #[test]
    fn from_access_scope() {
        assert_eq!(
            PatientCompartment::from_access_scope(Some(&AccessScope::PatientCompartment {
                patient_id: "p1".into()
            })),
            PatientCompartment::confined_to("p1")
        );
        assert_eq!(
            PatientCompartment::from_access_scope(Some(&AccessScope::Unrestricted)),
            PatientCompartment::unconfined()
        );
        assert_eq!(
            PatientCompartment::from_access_scope(None),
            PatientCompartment::unconfined()
        );
        assert_eq!(
            PatientCompartment::confined_to("p1").reference().as_deref(),
            Some("Patient/p1")
        );
    }

    #[test]
    fn the_patient_is_a_member_of_its_own_compartment_only() {
        let scope = PatientCompartment::confined_to("p1");
        let reg = registry();
        assert!(scope.is_member(
            || reg.clone(),
            FhirVersion::R4,
            &json!({"resourceType": "Patient", "id": "p1"})
        ));
        assert!(!scope.is_member(
            || reg.clone(),
            FhirVersion::R4,
            &json!({"resourceType": "Patient", "id": "p2"})
        ));
        // Another patient linked to the launch patient is in the compartment
        // through `link`, as the compartment search would find it.
        assert!(scope.is_member(
            || reg.clone(),
            FhirVersion::R4,
            &json!({"resourceType": "Patient", "id": "p2", "link": [{"other": {"reference": "Patient/p1"}, "type": "seealso"}]})
        ));
    }

    #[test]
    fn membership_follows_any_compartment_parameter() {
        let scope = PatientCompartment::confined_to("p1");
        let reg = registry();
        let mine = json!({"resourceType": "Observation", "id": "o1", "status": "final",
            "code": {"text": "x"}, "subject": {"reference": "Patient/p1"}});
        let theirs = json!({"resourceType": "Observation", "id": "o2", "status": "final",
            "code": {"text": "x"}, "subject": {"reference": "Patient/p2"}});
        let unattributed = json!({"resourceType": "Observation", "id": "o3", "status": "final",
            "code": {"text": "x"}});
        assert!(scope.is_member(|| reg.clone(), FhirVersion::R4, &mine));
        assert!(!scope.is_member(|| reg.clone(), FhirVersion::R4, &theirs));
        assert!(!scope.is_member(|| reg.clone(), FhirVersion::R4, &unattributed));
        // A versioned reference counts as its base.
        let versioned = json!({"resourceType": "Observation", "id": "o4", "status": "final",
            "code": {"text": "x"}, "subject": {"reference": "Patient/p1/_history/3"}});
        assert!(scope.is_member(|| reg.clone(), FhirVersion::R4, &versioned));
        // AllergyIntolerance joins through `recorder` as well as `patient`.
        let recorded = json!({"resourceType": "AllergyIntolerance", "id": "a1",
            "patient": {"reference": "Patient/p2"}, "recorder": {"reference": "Patient/p1"}});
        assert!(scope.is_member(|| reg.clone(), FhirVersion::R4, &recorded));
    }

    #[test]
    fn types_outside_the_compartment_are_never_members() {
        let scope = PatientCompartment::confined_to("p1");
        let reg = registry();
        for resource in [
            json!({"resourceType": "Binary", "id": "b1", "contentType": "text/plain"}),
            json!({"resourceType": "Practitioner", "id": "pr1"}),
            json!({"resourceType": "Organization", "id": "org1"}),
            json!({"id": "no-type"}),
        ] {
            assert!(
                !scope.is_member(|| reg.clone(), FhirVersion::R4, &resource),
                "{resource}"
            );
        }
        let err = scope
            .assert_member(
                || reg.clone(),
                FhirVersion::R4,
                &json!({"resourceType": "Binary"}),
                "read",
            )
            .unwrap_err();
        assert!(matches!(err, RestError::Forbidden { .. }), "{err:?}");
    }

    #[test]
    fn confine_query_pins_patient_and_filters_members() {
        let scope = PatientCompartment::confined_to("p1");

        let mut patients = SearchQuery::new("Patient");
        scope.confine_query(FhirVersion::R4, &mut patients).unwrap();
        assert!(patients.compartment.is_none());
        assert_eq!(patients.parameters.len(), 1);
        assert_eq!(patients.parameters[0].name, "_id");
        assert_eq!(patients.parameters[0].values[0].value, "p1");

        let mut observations = SearchQuery::new("Observation");
        scope
            .confine_query(FhirVersion::R4, &mut observations)
            .unwrap();
        let membership = observations.compartment.expect("compartment filter");
        assert_eq!(membership.reference, "Patient/p1");
        assert!(membership.params.iter().any(|p| p == "subject"));
        assert!(membership.params.iter().any(|p| p == "performer"));

        let mut binaries = SearchQuery::new("Binary");
        let err = scope
            .confine_query(FhirVersion::R4, &mut binaries)
            .unwrap_err();
        assert!(matches!(err, RestError::Forbidden { .. }), "{err:?}");

        let mut other = SearchQuery::new("Observation");
        other.compartment = Some(CompartmentMembership {
            params: vec!["encounter".into()],
            reference: "Encounter/e1".into(),
        });
        assert!(scope.confine_query(FhirVersion::R4, &mut other).is_err());
    }
}
