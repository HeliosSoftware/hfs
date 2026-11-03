//! Attributing `AuditEvent`s to the authenticated caller (#1515).
//!
//! `helios-audit` does not depend on `helios-auth`, so the mapping from a
//! [`Principal`] to the recorded agent lives here.

use helios_audit::{AuditAgent, AuditEventBuilder};
use helios_auth::Principal;

/// Claims tried, in order, for `AuditEvent.agent.who.display`. Email is left
/// out on purpose: the display only has to let a reader recognise the user,
/// and the audit trail should not collect more personal data than that.
const DISPLAY_CLAIMS: [&str; 2] = ["preferred_username", "name"];

/// The audit agent for an authenticated principal: `sub` scoped by `iss` as
/// the identifier, the `fhirUser` claim as the reference, and the first of
/// [`DISPLAY_CLAIMS`] the token carries as the display.
pub fn audit_agent(principal: &Principal) -> AuditAgent {
    let display = DISPLAY_CLAIMS.iter().find_map(|claim| {
        principal
            .custom_claims
            .get(*claim)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    });
    AuditAgent {
        subject: principal.subject().to_string(),
        issuer: Some(principal.issuer().to_string()).filter(|i| !i.is_empty()),
        reference: principal
            .launch_context
            .as_ref()
            .and_then(|lc| lc.fhir_user.clone()),
        display,
    }
}

/// Attributes an event under construction to a principal.
pub(crate) trait AttributeToPrincipal {
    /// Records `principal` as the requesting agent and, when its token is
    /// bound to a tenant, tags the event with that tenant.
    fn principal(self, principal: &Principal) -> Self;
}

impl AttributeToPrincipal for AuditEventBuilder {
    fn principal(self, principal: &Principal) -> Self {
        let builder = self.agent_identity(audit_agent(principal));
        match principal.tenant_id() {
            Some(tenant) => builder.tenant(tenant),
            None => builder,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_auth::principal::LaunchContext;
    use serde_json::json;

    fn principal(claims: serde_json::Value) -> Principal {
        Principal {
            subject: "937f5c4f".into(),
            issuer: "https://idp.example.org/realms/fhir".into(),
            custom_claims: claims.as_object().unwrap().clone(),
            ..Default::default()
        }
    }

    #[test]
    fn display_prefers_preferred_username_over_name() {
        let p =
            principal(json!({"preferred_username": "demo", "name": "Demo User", "email": "d@x"}));
        let agent = audit_agent(&p);
        assert_eq!(agent.display.as_deref(), Some("demo"));
        assert_eq!(agent.subject, "937f5c4f");
        assert_eq!(
            agent.issuer.as_deref(),
            Some("https://idp.example.org/realms/fhir")
        );
        assert!(agent.reference.is_none());
    }

    #[test]
    fn display_falls_back_to_name_and_never_email() {
        assert_eq!(
            audit_agent(&principal(json!({"name": "Demo User"})))
                .display
                .as_deref(),
            Some("Demo User")
        );
        assert!(
            audit_agent(&principal(json!({"email": "d@x"})))
                .display
                .is_none()
        );
    }

    #[test]
    fn fhir_user_becomes_the_reference() {
        let mut p = principal(json!({}));
        p.launch_context = Some(LaunchContext {
            fhir_user: Some("Practitioner/123".into()),
            ..Default::default()
        });
        assert_eq!(
            audit_agent(&p).reference.as_deref(),
            Some("Practitioner/123")
        );
    }

    #[test]
    fn principal_tags_the_token_tenant() {
        let mut p = principal(json!({}));
        p.tenant_id = Some("acme".into());
        let event = AuditEventBuilder::new("Device/hfs").principal(&p).build();
        let tags = event.meta.unwrap().tag.unwrap();
        assert_eq!(
            tags[0].code.as_ref().unwrap().value.as_deref(),
            Some("acme")
        );
    }
}
