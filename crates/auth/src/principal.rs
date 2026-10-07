use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::scope::ScopeSet;

/// Represents an authenticated identity extracted from a validated JWT.
///
/// Injected into Axum request extensions by the auth middleware after
/// successful token validation.
///
/// `Default` exists so literals (tests, embedders) can use `..Default::default()`
/// and keep compiling when fields are added. The default principal has an empty
/// subject, no scopes and an expiry at the Unix epoch, so it authorizes nothing.
#[derive(Debug, Clone, Default)]
pub struct Principal {
    /// The `sub` (subject) claim from the JWT.
    pub subject: String,
    /// The `iss` (issuer) claim from the JWT.
    pub issuer: String,
    /// The tenant ID extracted from the configured JWT claim.
    pub tenant_id: Option<String>,
    /// Parsed SMART v2 scopes granted to this principal.
    pub scopes: ScopeSet,
    /// The `jti` (JWT ID) claim, if the token carried one. Informational —
    /// bearer access tokens are reusable, so this is not a single-use marker.
    pub jti: Option<String>,
    /// Token expiration time.
    pub expires_at: DateTime<Utc>,
    /// Additional claims from the JWT not captured in other fields.
    pub custom_claims: Map<String, Value>,
    /// SMART App Launch context bound to the token (`patient`, `encounter`,
    /// `fhirUser`), or `None` when the token carries none of those claims. The
    /// claims also stay in [`custom_claims`](Self::custom_claims).
    pub launch_context: Option<LaunchContext>,
}

impl Principal {
    /// Returns the client/subject identifier.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Returns the token issuer.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Returns the tenant ID if present in the token.
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }
}

/// The SMART App Launch context the authorization server bound to an access
/// token, read from the claims named by `HFS_AUTH_PATIENT_CLAIM`,
/// `HFS_AUTH_ENCOUNTER_CLAIM` and `HFS_AUTH_FHIR_USER_CLAIM`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchContext {
    /// The patient claim as issued, normally a bare Patient id.
    pub patient: Option<String>,
    /// The encounter claim as issued.
    pub encounter: Option<String>,
    /// The fhirUser claim as issued, e.g. `Practitioner/123`.
    pub fhir_user: Option<String>,
}

impl LaunchContext {
    /// Reads the launch context from the named claims.
    ///
    /// Only non-empty string claims count. Returns `None` when none of the
    /// three is present.
    pub fn from_claims(
        claims: &Map<String, Value>,
        patient_claim: &str,
        encounter_claim: &str,
        fhir_user_claim: &str,
    ) -> Option<Self> {
        let read = |name: &str| {
            claims
                .get(name)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(String::from)
        };
        let ctx = Self {
            patient: read(patient_claim),
            encounter: read(encounter_claim),
            fhir_user: read(fhir_user_claim),
        };
        if ctx.patient.is_none() && ctx.encounter.is_none() && ctx.fhir_user.is_none() {
            None
        } else {
            Some(ctx)
        }
    }

    /// The launch patient's logical id, if the patient claim names a usable one.
    ///
    /// A bare `<id>` or a `Patient/<id>` relative reference is accepted; any
    /// other form (another resource type, an absolute URL, `_history`, empty)
    /// yields `None`.
    pub fn patient_id(&self) -> Option<&str> {
        let raw = self.patient.as_deref()?;
        let id = raw.strip_prefix("Patient/").unwrap_or(raw);
        is_fhir_id(id).then_some(id)
    }
}

/// FHIR `id` syntax: 1 to 64 characters of `[A-Za-z0-9\-.]`.
fn is_fhir_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claims(value: Value) -> Map<String, Value> {
        value.as_object().expect("object").clone()
    }

    fn read_default(map: &Map<String, Value>) -> Option<LaunchContext> {
        LaunchContext::from_claims(map, "patient", "encounter", "fhirUser")
    }

    fn with_patient(patient: Option<&str>) -> LaunchContext {
        LaunchContext {
            patient: patient.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn from_claims_reads_all_three_default_claims() {
        let map = claims(json!({
            "patient": "p1",
            "encounter": "e1",
            "fhirUser": "Practitioner/123",
        }));
        assert_eq!(
            read_default(&map),
            Some(LaunchContext {
                patient: Some("p1".into()),
                encounter: Some("e1".into()),
                fhir_user: Some("Practitioner/123".into()),
            })
        );
    }

    #[test]
    fn from_claims_is_none_when_nothing_is_present() {
        assert_eq!(read_default(&claims(json!({"sub": "u"}))), None);
    }

    #[test]
    fn from_claims_ignores_empty_and_non_string_values() {
        let map = claims(json!({
            "patient": "",
            "encounter": 42,
            "fhirUser": {"reference": "Practitioner/1"},
        }));
        assert_eq!(read_default(&map), None);

        let map = claims(json!({"patient": "", "encounter": "e1"}));
        let ctx = read_default(&map).expect("encounter present");
        assert_eq!(ctx.patient, None);
        assert_eq!(ctx.encounter.as_deref(), Some("e1"));
    }

    #[test]
    fn from_claims_honours_custom_names_only() {
        let map = claims(json!({"patient": "p1", "launch_patient": "p9"}));
        let ctx = LaunchContext::from_claims(&map, "launch_patient", "encounter", "fhirUser")
            .expect("custom claim present");
        assert_eq!(ctx.patient.as_deref(), Some("p9"));

        // The default-named claim is not read when a different name is configured.
        let map = claims(json!({"patient": "p1"}));
        assert_eq!(
            LaunchContext::from_claims(&map, "launch_patient", "encounter", "fhirUser"),
            None
        );
    }

    #[test]
    fn patient_id_accepts_bare_and_relative_reference_forms() {
        assert_eq!(with_patient(Some("p1")).patient_id(), Some("p1"));
        assert_eq!(with_patient(Some("Patient/p1")).patient_id(), Some("p1"));
        assert_eq!(
            with_patient(Some("Patient/a.b-C9")).patient_id(),
            Some("a.b-C9")
        );
    }

    #[test]
    fn patient_id_rejects_unusable_values() {
        let too_long = "a".repeat(65);
        for bad in [
            None,
            Some(""),
            Some("Patient/"),
            Some("Group/g1"),
            Some("https://x.example/fhir/Patient/p1"),
            Some("Patient/p1/_history/1"),
            Some("p 1"),
            Some(too_long.as_str()),
        ] {
            assert_eq!(with_patient(bad).patient_id(), None, "{bad:?}");
        }
        // The 64-character boundary is accepted.
        let max = "a".repeat(64);
        assert_eq!(with_patient(Some(&max)).patient_id(), Some(max.as_str()));
    }
}
