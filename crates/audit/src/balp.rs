//! IHE BALP v1.1.4 profile constants and selection logic.
//!
//! BALP (Basic Audit Log Patterns) defines a set of AuditEvent profiles for
//! common FHIR REST interactions. Each action has two variants: one with a
//! patient entity and one without.

// ── Profile URLs ─────────────────────────────────────────────────────────────

/// BALP profile URLs for FHIR REST interactions.
pub mod profiles {
    pub const READ: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Read";
    pub const PATIENT_READ: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientRead";
    pub const CREATE: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Create";
    pub const PATIENT_CREATE: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientCreate";
    pub const UPDATE: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Update";
    pub const PATIENT_UPDATE: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientUpdate";
    pub const DELETE: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Delete";
    pub const PATIENT_DELETE: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientDelete";
    pub const QUERY: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.Query";
    pub const PATIENT_QUERY: &str =
        "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.PatientQuery";
    pub const AUTH_TOKEN_USE: &str = "https://profiles.ihe.net/ITI/BALP/StructureDefinition/IHE.BasicAudit.OAUTHaccessTokenUse.Comprehensive";
}

// ── Well-known code systems ──────────────────────────────────────────────────

/// Code systems used in AuditEvent construction.
pub mod code_systems {
    pub const AUDIT_EVENT_TYPE: &str = "http://terminology.hl7.org/CodeSystem/audit-event-type";
    pub const RESTFUL_INTERACTION: &str = "http://hl7.org/fhir/restful-interaction";
    pub const AUDIT_ENTITY_TYPE: &str = "http://terminology.hl7.org/CodeSystem/audit-entity-type";
    pub const OBJECT_ROLE: &str = "http://terminology.hl7.org/CodeSystem/object-role";
}

// ── Action mapping ───────────────────────────────────────────────────────────

/// FHIR audit action codes (`http://hl7.org/fhir/audit-event-action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    /// C — Create
    Create,
    /// R — Read/Query
    Read,
    /// U — Update
    Update,
    /// D — Delete
    Delete,
    /// E — Execute (auth events, operations)
    Execute,
}

impl AuditAction {
    /// The single-character FHIR action code.
    pub fn to_code(self) -> &'static str {
        match self {
            Self::Create => "C",
            Self::Read => "R",
            Self::Update => "U",
            Self::Delete => "D",
            Self::Execute => "E",
        }
    }
}

/// Map an HTTP method to a BALP audit action.
pub fn action_for_method(method: &str) -> AuditAction {
    match method.to_uppercase().as_str() {
        "POST" => AuditAction::Create,
        "GET" | "HEAD" => AuditAction::Read,
        "PUT" | "PATCH" => AuditAction::Update,
        "DELETE" => AuditAction::Delete,
        _ => AuditAction::Execute,
    }
}

/// Parse a single-character FHIR action code back to an [`AuditAction`].
pub fn action_from_code(code: &str) -> AuditAction {
    match code {
        "C" => AuditAction::Create,
        "R" => AuditAction::Read,
        "U" => AuditAction::Update,
        "D" => AuditAction::Delete,
        _ => AuditAction::Execute,
    }
}

/// Select the BALP profile URL for the given action and patient presence.
pub fn select_profile(action: AuditAction, has_patient: bool) -> &'static str {
    match (action, has_patient) {
        (AuditAction::Create, true) => profiles::PATIENT_CREATE,
        (AuditAction::Create, false) => profiles::CREATE,
        (AuditAction::Read, true) => profiles::PATIENT_READ,
        (AuditAction::Read, false) => profiles::READ,
        (AuditAction::Update, true) => profiles::PATIENT_UPDATE,
        (AuditAction::Update, false) => profiles::UPDATE,
        (AuditAction::Delete, true) => profiles::PATIENT_DELETE,
        (AuditAction::Delete, false) => profiles::DELETE,
        (AuditAction::Execute, true) => profiles::PATIENT_READ, // Fallback
        (AuditAction::Execute, false) => profiles::AUTH_TOKEN_USE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── action_for_method ────────────────────────────────────────────────

    #[test]
    fn test_get_maps_to_read() {
        assert_eq!(action_for_method("GET"), AuditAction::Read);
    }

    #[test]
    fn test_head_maps_to_read() {
        assert_eq!(action_for_method("HEAD"), AuditAction::Read);
    }

    #[test]
    fn test_post_maps_to_create() {
        assert_eq!(action_for_method("POST"), AuditAction::Create);
    }

    #[test]
    fn test_put_maps_to_update() {
        assert_eq!(action_for_method("PUT"), AuditAction::Update);
    }

    #[test]
    fn test_patch_maps_to_update() {
        assert_eq!(action_for_method("PATCH"), AuditAction::Update);
    }

    #[test]
    fn test_delete_maps_to_delete() {
        assert_eq!(action_for_method("DELETE"), AuditAction::Delete);
    }

    #[test]
    fn test_unknown_method_maps_to_execute() {
        assert_eq!(action_for_method("OPTIONS"), AuditAction::Execute);
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(action_for_method("get"), AuditAction::Read);
        assert_eq!(action_for_method("Post"), AuditAction::Create);
    }

    // ── action_from_code ─────────────────────────────────────────────────

    #[test]
    fn test_action_from_code_roundtrip() {
        for action in [
            AuditAction::Create,
            AuditAction::Read,
            AuditAction::Update,
            AuditAction::Delete,
            AuditAction::Execute,
        ] {
            assert_eq!(action_from_code(action.to_code()), action);
        }
    }

    // ── select_profile ───────────────────────────────────────────────────

    #[test]
    fn test_read_without_patient() {
        assert_eq!(select_profile(AuditAction::Read, false), profiles::READ);
    }

    #[test]
    fn test_read_with_patient() {
        assert_eq!(
            select_profile(AuditAction::Read, true),
            profiles::PATIENT_READ
        );
    }

    #[test]
    fn test_create_with_patient() {
        assert_eq!(
            select_profile(AuditAction::Create, true),
            profiles::PATIENT_CREATE
        );
    }

    #[test]
    fn test_create_without_patient() {
        assert_eq!(select_profile(AuditAction::Create, false), profiles::CREATE);
    }

    #[test]
    fn test_update_with_patient() {
        assert_eq!(
            select_profile(AuditAction::Update, true),
            profiles::PATIENT_UPDATE
        );
    }

    #[test]
    fn test_delete_without_patient() {
        assert_eq!(select_profile(AuditAction::Delete, false), profiles::DELETE);
    }

    #[test]
    fn test_execute_without_patient_uses_auth_profile() {
        assert_eq!(
            select_profile(AuditAction::Execute, false),
            profiles::AUTH_TOKEN_USE
        );
    }

    // ── to_code ──────────────────────────────────────────────────────────

    #[test]
    fn test_to_code() {
        assert_eq!(AuditAction::Create.to_code(), "C");
        assert_eq!(AuditAction::Read.to_code(), "R");
        assert_eq!(AuditAction::Update.to_code(), "U");
        assert_eq!(AuditAction::Delete.to_code(), "D");
        assert_eq!(AuditAction::Execute.to_code(), "E");
    }
}
