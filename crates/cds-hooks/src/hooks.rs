//! Hook context types for all hooks defined in the
//! [CDS Hooks Library](https://cds-hooks.hl7.org/hooks/).
//!
//! Each hook represents a specific point in a clinical workflow where decision
//! support can be invoked. This module provides strongly-typed context structs
//! for every hook in the library, enabling compile-time validation of hook requests.
//!
//! # Defined Hooks
//!
//! | Hook | Maturity | Description |
//! |------|----------|-------------|
//! | [`PatientViewContext`] | 5 - Mature | User opens a patient's record |
//! | [`OrderSelectContext`] | 4 - Documented | Clinician selects an order |
//! | [`OrderSignContext`] | 5 - Mature | Clinician signs one or more orders |
//! | [`EncounterStartContext`] | 1 - Submitted | A new encounter is initiated |
//! | [`EncounterDischargeContext`] | 1 - Submitted | Patient is being discharged |
//! | [`AppointmentBookContext`] | 1 - Submitted | One or more appointments are being booked |
//! | [`OrderDispatchContext`] | 0 - Draft | An order is being dispatched to a performer |
//! | [`AllergyIntoleranceCreateContext`] | 1 - Submitted | A new allergy is being added |
//! | [`MedicationRefillContext`] | 1 - Submitted | A medication refill is requested |
//! | [`ProblemListItemCreateContext`] | 1 - Submitted | A new problem is added to the list |

use serde::{Deserialize, Serialize};

/// Trait implemented by all hook context types.
///
/// Provides metadata about the hook, including its name and specification version.
/// Implementors should use the corresponding context struct for their hook type.
///
/// # Example
///
/// ```
/// use helios_cds_hooks::hooks::{HookContext, PatientViewContext};
///
/// let ctx = PatientViewContext {
///     user_id: "Practitioner/123".to_string(),
///     patient_id: "456".to_string(),
///     encounter_id: None,
/// };
///
/// assert_eq!(PatientViewContext::HOOK_NAME, "patient-view");
/// assert_eq!(PatientViewContext::HOOK_VERSION, "1.0");
/// ```
pub trait HookContext: Serialize + for<'de> Deserialize<'de> + Send + Sync {
    /// The hook name as defined in the CDS Hooks Library (e.g. `"patient-view"`).
    const HOOK_NAME: &'static str;

    /// The version of this hook definition.
    const HOOK_VERSION: &'static str;

    /// The CDS Hooks specification version this hook is defined against.
    const SPECIFICATION_VERSION: &'static str;

    /// The maturity level of this hook (0-6).
    const HOOK_MATURITY: u8;
}

// ---------------------------------------------------------------------------
// patient-view (Maturity 5 - Mature)
// ---------------------------------------------------------------------------

/// Context for the **patient-view** hook.
///
/// Fires when a user opens a patient's record. Typically called only once at
/// the beginning of a user's interaction with a specific patient's record.
///
/// The user may be a clinician (Practitioner/PractitionerRole) or a patient
/// or proxy (Patient/RelatedPerson) viewing their own record.
///
/// # Specification
///
/// - **Hook name:** `patient-view`
/// - **Hook version:** 1.0
/// - **Maturity:** 5 - Mature
///
/// # Example
///
/// ```json
/// {
///   "userId": "PractitionerRole/123",
///   "patientId": "1288992",
///   "encounterId": "456"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatientViewContext {
    /// The id of the current user.
    ///
    /// Format: `[ResourceType]/[id]` where ResourceType is one of
    /// `Practitioner`, `PractitionerRole`, `Patient`, or `RelatedPerson`.
    #[serde(rename = "userId")]
    pub user_id: String,

    /// The FHIR `Patient.id` of the current patient in context.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the current encounter in context, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "encounterId")]
    pub encounter_id: Option<String>,
}

impl HookContext for PatientViewContext {
    const HOOK_NAME: &'static str = "patient-view";
    const HOOK_VERSION: &'static str = "1.0";
    const SPECIFICATION_VERSION: &'static str = "1.0";
    const HOOK_MATURITY: u8 = 5;
}

// ---------------------------------------------------------------------------
// order-select (Maturity 4 - Documented)
// ---------------------------------------------------------------------------

/// Context for the **order-select** hook.
///
/// Fires after the clinician selects one or more new orders from a list of
/// potential orders for a specific patient. The newly selected order may or may
/// not have all details specified. This hook is among the first workflow events
/// for an order entering a draft status.
///
/// Decision support should focus on the *selected* orders (those newly selected
/// or currently being authored). The non-selected orders are included to provide
/// context for pending actions.
///
/// This hook replaces (deprecates) the `medication-prescribe` hook.
///
/// # Specification
///
/// - **Hook name:** `order-select`
/// - **Hook version:** 1.0
/// - **Maturity:** 4 - Documented
///
/// # Example
///
/// ```json
/// {
///   "userId": "PractitionerRole/123",
///   "patientId": "1288992",
///   "encounterId": "89284",
///   "selections": ["MedicationRequest/smart-MedicationRequest-103"],
///   "draftOrders": {
///     "resourceType": "Bundle",
///     "type": "collection",
///     "entry": []
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderSelectContext {
    /// The id of the current user.
    ///
    /// Expected to be of type `Practitioner` or `PractitionerRole`.
    #[serde(rename = "userId")]
    pub user_id: String,

    /// The FHIR `Patient.id` of the current patient in context.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the current encounter in context, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "encounterId")]
    pub encounter_id: Option<String>,

    /// The FHIR ids of the newly selected order(s).
    ///
    /// References resources in the `draft_orders` Bundle.
    /// For example, `["MedicationRequest/103"]`.
    pub selections: Vec<String>,

    /// A Bundle of FHIR request resources with draft status, representing all
    /// unsigned orders from the current ordering session.
    ///
    /// Includes both the newly selected orders and any previously selected
    /// orders that haven't been signed yet.
    #[serde(rename = "draftOrders")]
    pub draft_orders: serde_json::Value,
}

impl HookContext for OrderSelectContext {
    const HOOK_NAME: &'static str = "order-select";
    const HOOK_VERSION: &'static str = "1.0";
    const SPECIFICATION_VERSION: &'static str = "1.0";
    const HOOK_MATURITY: u8 = 4;
}

// ---------------------------------------------------------------------------
// order-sign (Maturity 5 - Mature)
// ---------------------------------------------------------------------------

/// Context for the **order-sign** hook.
///
/// Fires when a clinician is ready to sign one or more orders for a patient.
/// This hook is among the last workflow events before an order is promoted out
/// of a draft status. The context contains all order details (dose, quantity,
/// route, etc.), making it suitable for CDS Services that require complete
/// order information.
///
/// This hook can also fire when orders are being re-signed after revision
/// (e.g. status changes, date extensions). In this case, orders may have a
/// status other than `draft`.
///
/// This hook replaces (deprecates) the `medication-prescribe` and `order-review` hooks.
///
/// # Specification
///
/// - **Hook name:** `order-sign`
/// - **Hook version:** 1.0
/// - **Maturity:** 5 - Mature
///
/// # Example
///
/// ```json
/// {
///   "userId": "PractitionerRole/123",
///   "patientId": "1288992",
///   "encounterId": "89284",
///   "draftOrders": {
///     "resourceType": "Bundle",
///     "type": "collection",
///     "entry": []
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderSignContext {
    /// The id of the current user.
    ///
    /// Expected to be of type `Practitioner` or `PractitionerRole`.
    #[serde(rename = "userId")]
    pub user_id: String,

    /// The FHIR `Patient.id` of the current patient in context.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the current encounter in context, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "encounterId")]
    pub encounter_id: Option<String>,

    /// A Bundle of FHIR request resources with draft status, representing orders
    /// that aren't yet signed from the current ordering session.
    #[serde(rename = "draftOrders")]
    pub draft_orders: serde_json::Value,
}

impl HookContext for OrderSignContext {
    const HOOK_NAME: &'static str = "order-sign";
    const HOOK_VERSION: &'static str = "1.0";
    const SPECIFICATION_VERSION: &'static str = "1.0";
    const HOOK_MATURITY: u8 = 5;
}

// ---------------------------------------------------------------------------
// encounter-start (Maturity 1 - Submitted)
// ---------------------------------------------------------------------------

/// Context for the **encounter-start** hook.
///
/// Fires when the user is initiating a new encounter. In an inpatient setting,
/// this is the point of admission; in an outpatient setting, this is when the
/// patient checks in for an in-person or virtual visit.
///
/// The encounter may be in `planned`, `arrived`, `triaged`, or `in-progress`
/// status when this hook fires.
///
/// # Specification
///
/// - **Hook name:** `encounter-start`
/// - **Hook version:** 1.0
/// - **Maturity:** 1 - Submitted
///
/// # Example
///
/// ```json
/// {
///   "userId": "PractitionerRole/A2340113",
///   "patientId": "1288992",
///   "encounterId": "456"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncounterStartContext {
    /// The id of the current user.
    ///
    /// Expected to be of type `Practitioner` or `PractitionerRole`.
    #[serde(rename = "userId")]
    pub user_id: String,

    /// The FHIR `Patient.id` of the current patient in context.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the encounter being started.
    #[serde(rename = "encounterId")]
    pub encounter_id: String,
}

impl HookContext for EncounterStartContext {
    const HOOK_NAME: &'static str = "encounter-start";
    const HOOK_VERSION: &'static str = "1.0";
    const SPECIFICATION_VERSION: &'static str = "1.0";
    const HOOK_MATURITY: u8 = 1;
}

// ---------------------------------------------------------------------------
// encounter-discharge (Maturity 1 - Submitted)
// ---------------------------------------------------------------------------

/// Context for the **encounter-discharge** hook.
///
/// Fires when the user is performing the discharge process for an encounter
/// where the notion of "discharge" is relevant — typically an inpatient encounter.
/// It may be invoked at the start, end, or any time during the discharge process.
///
/// CDS Services may use this hook to:
/// - Verify whether discharge is appropriate
/// - Check discharge medications
/// - Ensure continuity of care planning
/// - Verify necessary documentation for discharge processing
///
/// # Specification
///
/// - **Hook name:** `encounter-discharge`
/// - **Hook version:** 1.0
/// - **Maturity:** 1 - Submitted
///
/// # Example
///
/// ```json
/// {
///   "userId": "PractitionerRole/A2340113",
///   "patientId": "1288992",
///   "encounterId": "456"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncounterDischargeContext {
    /// The id of the current user.
    ///
    /// Expected to be of type `Practitioner` or `PractitionerRole`.
    #[serde(rename = "userId")]
    pub user_id: String,

    /// The FHIR `Patient.id` of the patient being discharged.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the encounter being ended.
    #[serde(rename = "encounterId")]
    pub encounter_id: String,
}

impl HookContext for EncounterDischargeContext {
    const HOOK_NAME: &'static str = "encounter-discharge";
    const HOOK_VERSION: &'static str = "1.0";
    const SPECIFICATION_VERSION: &'static str = "1.0";
    const HOOK_MATURITY: u8 = 1;
}

// ---------------------------------------------------------------------------
// appointment-book (Maturity 1 - Submitted)
// ---------------------------------------------------------------------------

/// Context for the **appointment-book** hook.
///
/// Fires when the user is scheduling one or more future encounters/visits for a
/// patient. The hook may be triggered for appointments with the creator, a
/// clinician within the same organization, or even for appointments outside the
/// creator's organization. It may fire at any point during the booking process.
///
/// CDS Services may use this hook to:
/// - Intervene in scheduling decisions (when, where, what services)
/// - Identify pre-appointment actions
/// - Enforce scheduling policies
///
/// # Specification
///
/// - **Hook name:** `appointment-book`
/// - **Hook version:** 1.0
/// - **Maturity:** 1 - Submitted
///
/// # Example
///
/// ```json
/// {
///   "userId": "PractitionerRole/A2340113",
///   "patientId": "1288992",
///   "appointments": {
///     "resourceType": "Bundle",
///     "entry": [
///       {
///         "resource": {
///           "resourceType": "Appointment",
///           "id": "apt1",
///           "status": "proposed"
///         }
///       }
///     ]
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppointmentBookContext {
    /// The id of the current user.
    ///
    /// May be of type `Practitioner`, `PractitionerRole`, `Patient`, or
    /// `RelatedPerson`. Patient or RelatedPerson are appropriate when a patient
    /// or their proxy is booking the appointment.
    #[serde(rename = "userId")]
    pub user_id: String,

    /// The FHIR `Patient.id` of the patient the appointment(s) are for.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the encounter where booking was initiated, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "encounterId")]
    pub encounter_id: Option<String>,

    /// FHIR Bundle of Appointment resources in `proposed` state.
    pub appointments: serde_json::Value,
}

impl HookContext for AppointmentBookContext {
    const HOOK_NAME: &'static str = "appointment-book";
    const HOOK_VERSION: &'static str = "1.0";
    const SPECIFICATION_VERSION: &'static str = "1.0";
    const HOOK_MATURITY: u8 = 1;
}

// ---------------------------------------------------------------------------
// order-dispatch (Maturity 0 - Draft)
// ---------------------------------------------------------------------------

/// Context for the **order-dispatch** hook.
///
/// Fires when a practitioner is selecting a candidate performer for a pre-existing
/// order (or set of orders) that was not assigned to a specific performer. For
/// example, selecting an imaging center for a radiology order or a cardiologist
/// for a referral.
///
/// This hook only occurs when the order is agnostic as to the performer and a
/// separate process is used to select and seek action by a specific performer.
/// The same order may be dispatched multiple times (e.g. initial targets refuse,
/// or partial fulfillment is requested).
///
/// The fulfillment process is typically represented in FHIR using `Task` resources.
///
/// # Specification
///
/// - **Hook name:** `order-dispatch`
/// - **Hook version:** 1.1
/// - **Maturity:** 0 - Draft
///
/// # Example
///
/// ```json
/// {
///   "patientId": "1288992",
///   "dispatchedOrders": ["ServiceRequest/proc002"],
///   "performer": "Organization/some-performer"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderDispatchContext {
    /// The FHIR `Patient.id` of the current patient in context.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// Collection of FHIR local references for the Request resource(s) for which
    /// fulfillment is sought.
    ///
    /// For example, `["ServiceRequest/123"]`.
    #[serde(rename = "dispatchedOrders")]
    pub dispatched_orders: Vec<String>,

    /// The FHIR local reference for the performer being asked to execute the order.
    ///
    /// May reference a `Practitioner`, `PractitionerRole`, `Organization`,
    /// `CareTeam`, etc. For example, `"Practitioner/456"`.
    pub performer: String,

    /// Collection of `Task` instances describing the fulfillment request, including
    /// timing and constraints.
    ///
    /// If provided, each Task is for a separate order and must reference one of the
    /// `dispatched_orders`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "fulfillmentTasks")]
    pub fulfillment_tasks: Option<Vec<serde_json::Value>>,
}

impl HookContext for OrderDispatchContext {
    const HOOK_NAME: &'static str = "order-dispatch";
    const HOOK_VERSION: &'static str = "1.1";
    const SPECIFICATION_VERSION: &'static str = "2.0";
    const HOOK_MATURITY: u8 = 0;
}

// ---------------------------------------------------------------------------
// allergyintolerance-create (Maturity 1 - Submitted)
// ---------------------------------------------------------------------------

/// Context for the **allergyintolerance-create** hook.
///
/// Fires when a clinician adds a new allergy or intolerance to a patient's list.
/// This hook fires during the act of finalizing the entry, allowing the CDS Service
/// to guide the clinician to cancel the addition if appropriate.
///
/// # Specification
///
/// - **Hook name:** `allergyintolerance-create`
/// - **Hook version:** 0.1.0
/// - **Maturity:** 1 - Submitted
///
/// # Example
///
/// ```json
/// {
///   "userId": "Practitioner/123",
///   "patientId": "1288992",
///   "encounterId": "89284",
///   "allergyIntolerance": {
///     "resourceType": "AllergyIntolerance",
///     "id": "RES163672",
///     "clinicalStatus": "active"
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AllergyIntoleranceCreateContext {
    /// The id of the current user.
    ///
    /// Expected to be of type `Practitioner`.
    #[serde(rename = "userId")]
    pub user_id: String,

    /// The FHIR `Patient.id` of the current patient in context.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the current encounter in context, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "encounterId")]
    pub encounter_id: Option<String>,

    /// The FHIR `AllergyIntolerance` resource that is about to be added to the
    /// patient's list of allergies.
    #[serde(rename = "allergyIntolerance")]
    pub allergy_intolerance: serde_json::Value,
}

impl HookContext for AllergyIntoleranceCreateContext {
    const HOOK_NAME: &'static str = "allergyintolerance-create";
    const HOOK_VERSION: &'static str = "0.1.0";
    const SPECIFICATION_VERSION: &'static str = "1.0";
    const HOOK_MATURITY: u8 = 1;
}

// ---------------------------------------------------------------------------
// medication-refill (Maturity 1 - Submitted)
// ---------------------------------------------------------------------------

/// Context for the **medication-refill** hook.
///
/// Fires when a medication refill request is received. This may occur outside of
/// typical prescriber workflows, with or without a user in context. The hook does
/// **not** fire for initial prescriptions or re-prescribing existing medications
/// with a new prescription.
///
/// # Specification
///
/// - **Hook name:** `medication-refill`
/// - **Hook version:** 0.1.0
/// - **Maturity:** 1 - Submitted
///
/// # Example
///
/// ```json
/// {
///   "patientId": "1288992",
///   "medications": {
///     "resourceType": "Bundle",
///     "type": "collection",
///     "entry": [
///       {
///         "resource": {
///           "resourceType": "MedicationRequest",
///           "status": "draft"
///         }
///       }
///     ]
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MedicationRefillContext {
    /// The id of the current user, if one is in context.
    ///
    /// Expected to be of type `Practitioner` or `PractitionerRole`.
    /// This field is optional because refill requests may occur outside of
    /// typical prescriber workflows.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "userId")]
    pub user_id: Option<String>,

    /// The FHIR `Patient.id` of the current patient in context.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the current encounter in context, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "encounterId")]
    pub encounter_id: Option<String>,

    /// FHIR Bundle of `MedicationRequest` resources with draft status,
    /// representing the medication(s) being refilled.
    pub medications: serde_json::Value,
}

impl HookContext for MedicationRefillContext {
    const HOOK_NAME: &'static str = "medication-refill";
    const HOOK_VERSION: &'static str = "0.1.0";
    const SPECIFICATION_VERSION: &'static str = "2.0";
    const HOOK_MATURITY: u8 = 1;
}

// ---------------------------------------------------------------------------
// problem-list-item-create (Maturity 1 - Submitted)
// ---------------------------------------------------------------------------

/// Context for the **problem-list-item-create** hook.
///
/// Fires once a clinician has added one or more new problems to a patient's
/// problem list. This hook fires after the problem is finalized, enabling the
/// CDS Service to recommend actions related to the problem (rather than suggesting
/// modifications to the newly created problem itself).
///
/// # Specification
///
/// - **Hook name:** `problem-list-item-create`
/// - **Hook version:** 0.1.0
/// - **Maturity:** 1 - Submitted
///
/// # Example
///
/// ```json
/// {
///   "userId": "Practitioner/123",
///   "patientId": "1288992",
///   "encounterId": "89284",
///   "conditions": {
///     "resourceType": "Bundle",
///     "entry": [
///       {
///         "resource": {
///           "resourceType": "Condition",
///           "category": [
///             {
///               "coding": [
///                 {
///                   "system": "http://terminology.hl7.org/CodeSystem/condition-category",
///                   "code": "problem-list-item"
///                 }
///               ]
///             }
///           ]
///         }
///       }
///     ]
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProblemListItemCreateContext {
    /// The id of the current user.
    ///
    /// Expected to be of type `Practitioner` or `PractitionerRole`.
    #[serde(rename = "userId")]
    pub user_id: String,

    /// The FHIR `Patient.id` of the current patient in context.
    #[serde(rename = "patientId")]
    pub patient_id: String,

    /// The FHIR `Encounter.id` of the current encounter in context, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "encounterId")]
    pub encounter_id: Option<String>,

    /// FHIR Bundle of `Condition` resources with category `problem-list-item`
    /// that have been added to the patient's problem list.
    pub conditions: serde_json::Value,
}

impl HookContext for ProblemListItemCreateContext {
    const HOOK_NAME: &'static str = "problem-list-item-create";
    const HOOK_VERSION: &'static str = "0.1.0";
    const SPECIFICATION_VERSION: &'static str = "1.0";
    const HOOK_MATURITY: u8 = 1;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_patient_view_context() {
        let ctx = PatientViewContext {
            user_id: "PractitionerRole/123".to_string(),
            patient_id: "1288992".to_string(),
            encounter_id: Some("456".to_string()),
        };

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["userId"], "PractitionerRole/123");
        assert_eq!(json["patientId"], "1288992");
        assert_eq!(json["encounterId"], "456");
        assert_eq!(PatientViewContext::HOOK_NAME, "patient-view");
    }

    #[test]
    fn test_patient_view_optional_encounter() {
        let ctx = PatientViewContext {
            user_id: "Practitioner/abc".to_string(),
            patient_id: "1288992".to_string(),
            encounter_id: None,
        };

        let json = serde_json::to_string(&ctx).unwrap();
        assert!(!json.contains("encounterId"));
    }

    #[test]
    fn test_order_select_context() {
        let ctx = OrderSelectContext {
            user_id: "PractitionerRole/123".to_string(),
            patient_id: "1288992".to_string(),
            encounter_id: Some("89284".to_string()),
            selections: vec![
                "NutritionOrder/pureeddiet-simple".to_string(),
                "MedicationRequest/smart-MedicationRequest-103".to_string(),
            ],
            draft_orders: serde_json::json!({
                "resourceType": "Bundle",
                "type": "collection",
                "entry": []
            }),
        };

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["selections"].as_array().unwrap().len(), 2);
        assert_eq!(json["draftOrders"]["resourceType"], "Bundle");
        assert_eq!(OrderSelectContext::HOOK_NAME, "order-select");
    }

    #[test]
    fn test_order_sign_context() {
        let ctx = OrderSignContext {
            user_id: "PractitionerRole/123".to_string(),
            patient_id: "1288992".to_string(),
            encounter_id: None,
            draft_orders: serde_json::json!({
                "resourceType": "Bundle",
                "type": "collection",
                "entry": []
            }),
        };

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["draftOrders"]["type"], "collection");
        assert_eq!(OrderSignContext::HOOK_NAME, "order-sign");
        assert_eq!(OrderSignContext::HOOK_MATURITY, 5);
    }

    #[test]
    fn test_encounter_start_context() {
        let ctx = EncounterStartContext {
            user_id: "PractitionerRole/A2340113".to_string(),
            patient_id: "1288992".to_string(),
            encounter_id: "456".to_string(),
        };

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["encounterId"], "456");
        assert_eq!(EncounterStartContext::HOOK_NAME, "encounter-start");
    }

    #[test]
    fn test_encounter_discharge_context() {
        let json_str = r#"{
            "userId": "PractitionerRole/A2340113",
            "patientId": "1288992",
            "encounterId": "456"
        }"#;

        let ctx: EncounterDischargeContext = serde_json::from_str(json_str).unwrap();
        assert_eq!(ctx.encounter_id, "456");
        assert_eq!(EncounterDischargeContext::HOOK_NAME, "encounter-discharge");
    }

    #[test]
    fn test_appointment_book_context() {
        let ctx = AppointmentBookContext {
            user_id: "PractitionerRole/A2340113".to_string(),
            patient_id: "1288992".to_string(),
            encounter_id: None,
            appointments: serde_json::json!({
                "resourceType": "Bundle",
                "entry": [{
                    "resource": {
                        "resourceType": "Appointment",
                        "id": "apt1",
                        "status": "proposed"
                    }
                }]
            }),
        };

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(
            json["appointments"]["entry"][0]["resource"]["status"],
            "proposed"
        );
        assert_eq!(AppointmentBookContext::HOOK_NAME, "appointment-book");
    }

    #[test]
    fn test_order_dispatch_context() {
        let ctx = OrderDispatchContext {
            patient_id: "1288992".to_string(),
            dispatched_orders: vec!["ServiceRequest/proc002".to_string()],
            performer: "Organization/some-performer".to_string(),
            fulfillment_tasks: None,
        };

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["dispatchedOrders"][0], "ServiceRequest/proc002");
        assert_eq!(json["performer"], "Organization/some-performer");
        assert_eq!(OrderDispatchContext::HOOK_NAME, "order-dispatch");
        assert_eq!(OrderDispatchContext::HOOK_MATURITY, 0);
    }

    #[test]
    fn test_allergyintolerance_create_context() {
        let ctx = AllergyIntoleranceCreateContext {
            user_id: "Practitioner/123".to_string(),
            patient_id: "1288992".to_string(),
            encounter_id: Some("89284".to_string()),
            allergy_intolerance: serde_json::json!({
                "resourceType": "AllergyIntolerance",
                "id": "RES163672",
                "clinicalStatus": "active",
                "type": "allergy",
                "category": ["food"]
            }),
        };

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["allergyIntolerance"]["type"], "allergy");
        assert_eq!(
            AllergyIntoleranceCreateContext::HOOK_NAME,
            "allergyintolerance-create"
        );
    }

    #[test]
    fn test_medication_refill_context() {
        let ctx = MedicationRefillContext {
            user_id: None,
            patient_id: "1288992".to_string(),
            encounter_id: None,
            medications: serde_json::json!({
                "resourceType": "Bundle",
                "type": "collection",
                "entry": [{
                    "resource": {
                        "resourceType": "MedicationRequest",
                        "status": "draft"
                    }
                }]
            }),
        };

        let json = serde_json::to_string(&ctx).unwrap();
        assert!(!json.contains("userId"));
        assert_eq!(MedicationRefillContext::HOOK_NAME, "medication-refill");
    }

    #[test]
    fn test_problem_list_item_create_context() {
        let ctx = ProblemListItemCreateContext {
            user_id: "Practitioner/123".to_string(),
            patient_id: "1288992".to_string(),
            encounter_id: Some("89284".to_string()),
            conditions: serde_json::json!({
                "resourceType": "Bundle",
                "entry": [{
                    "resource": {
                        "resourceType": "Condition",
                        "category": [{
                            "coding": [{
                                "system": "http://terminology.hl7.org/CodeSystem/condition-category",
                                "code": "problem-list-item"
                            }]
                        }]
                    }
                }]
            }),
        };

        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(
            json["conditions"]["entry"][0]["resource"]["resourceType"],
            "Condition"
        );
        assert_eq!(
            ProblemListItemCreateContext::HOOK_NAME,
            "problem-list-item-create"
        );
    }
}
