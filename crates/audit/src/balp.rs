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
    /// R — Read
    Read,
    /// U — Update
    Update,
    /// D — Delete
    Delete,
    /// E — Query (BALP Query profile uses execute action code)
    Query,
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
            Self::Query => "E",
            Self::Execute => "E",
        }
    }
}

/// Detect the BALP audit action from method + FHIR interaction context.
pub fn detect_interaction(
    method: &str,
    path: &str,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
) -> AuditAction {
    let method = method.to_ascii_uppercase();

    if matches!(method.as_str(), "GET" | "HEAD") {
        // GET/HEAD on a resource type endpoint is typically search/type-history.
        if resource_type.is_some() && resource_id.is_none() {
            return AuditAction::Query;
        }

        // History interactions are query-like, including instance history.
        if resource_id.is_some() && is_history_path(path) {
            return AuditAction::Query;
        }

        if resource_id.is_some() {
            return AuditAction::Read;
        }

        // System history (`GET /_history`) is also query-like.
        if resource_type.is_none() && is_history_path(path) {
            return AuditAction::Query;
        }

        return AuditAction::Execute;
    }

    match method.as_str() {
        "POST" if path.contains("_search") => AuditAction::Query,
        "POST" if resource_type.is_some() => AuditAction::Create,
        "POST" => AuditAction::Execute,
        "PUT" | "PATCH" => AuditAction::Update,
        "DELETE" => AuditAction::Delete,
        _ => AuditAction::Execute,
    }
}

fn is_history_path(path: &str) -> bool {
    path == "/_history" || path.contains("/_history")
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
        (AuditAction::Query, true) => profiles::PATIENT_QUERY,
        (AuditAction::Query, false) => profiles::QUERY,
        (AuditAction::Execute, true) => profiles::PATIENT_READ, // Fallback
        (AuditAction::Execute, false) => profiles::AUTH_TOKEN_USE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_interaction ───────────────────────────────────────────────

    #[test]
    fn test_detect_type_search_get_as_query() {
        assert_eq!(
            detect_interaction("GET", "/Patient", Some("Patient"), None),
            AuditAction::Query
        );
    }

    #[test]
    fn test_detect_type_history_get_as_query() {
        assert_eq!(
            detect_interaction("GET", "/Patient/_history", Some("Patient"), None),
            AuditAction::Query
        );
    }

    #[test]
    fn test_detect_system_history_get_as_query() {
        assert_eq!(
            detect_interaction("GET", "/_history", None, None),
            AuditAction::Query
        );
    }

    #[test]
    fn test_detect_instance_history_get_as_query() {
        assert_eq!(
            detect_interaction("GET", "/Patient/123/_history", Some("Patient"), Some("123")),
            AuditAction::Query
        );
    }

    #[test]
    fn test_detect_read_get_as_read() {
        assert_eq!(
            detect_interaction("GET", "/Patient/123", Some("Patient"), Some("123")),
            AuditAction::Read
        );
    }

    #[test]
    fn test_detect_post_search_as_query() {
        assert_eq!(
            detect_interaction("POST", "/Patient/_search", Some("Patient"), None),
            AuditAction::Query
        );
    }

    #[test]
    fn test_detect_post_create_as_create() {
        assert_eq!(
            detect_interaction("POST", "/Patient", Some("Patient"), None),
            AuditAction::Create
        );
    }

    #[test]
    fn test_detect_post_root_as_execute_for_batch_transaction() {
        assert_eq!(
            detect_interaction("POST", "/", None, None),
            AuditAction::Execute
        );
    }

    #[test]
    fn test_detect_unknown_method_as_execute() {
        assert_eq!(
            detect_interaction("OPTIONS", "/Patient", Some("Patient"), None),
            AuditAction::Execute
        );
    }

    #[test]
    fn test_detect_interaction_case_insensitive_method() {
        assert_eq!(
            detect_interaction("get", "/Patient/123", Some("Patient"), Some("123")),
            AuditAction::Read
        );
        assert_eq!(
            detect_interaction("Post", "/Patient", Some("Patient"), None),
            AuditAction::Create
        );
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

    #[test]
    fn test_query_uses_execute_code_but_parses_as_execute() {
        assert_eq!(AuditAction::Query.to_code(), "E");
        assert_eq!(action_from_code("E"), AuditAction::Execute);
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
    fn test_query_without_patient_uses_query_profile() {
        assert_eq!(select_profile(AuditAction::Query, false), profiles::QUERY);
    }

    #[test]
    fn test_query_with_patient_uses_patient_query_profile() {
        assert_eq!(
            select_profile(AuditAction::Query, true),
            profiles::PATIENT_QUERY
        );
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
        assert_eq!(AuditAction::Query.to_code(), "E");
        assert_eq!(AuditAction::Execute.to_code(), "E");
    }
}
