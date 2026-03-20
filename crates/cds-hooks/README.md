# helios-cds-hooks

Rust types and traits for building [CDS Hooks](https://cds-hooks.hl7.org/) services conforming to the HL7 CDS Hooks specification (v3.0.0-ballot). This crate provides a complete, strongly-typed foundation for implementing clinical decision support services that integrate with Electronic Health Record (EHR) systems.

## Overview

[CDS Hooks](https://cds-hooks.hl7.org/) is an HL7 standard that defines a "hook"-based pattern for invoking clinical decision support from within a clinician's workflow. The specification supports:

- **Synchronous, workflow-triggered CDS calls** returning information and suggestions via *cards*
- **Launching user-facing apps** (e.g. SMART on FHIR) when additional interaction is required
- **Feedback collection** to enable services to learn from clinician decisions

This crate provides the Rust building blocks for the *CDS Service* side of the protocol: data types for all request/response structures, strongly-typed context structs for every hook in the [CDS Hooks Library](https://cds-hooks.hl7.org/hooks/), and an async service trait that integrates with any Rust web framework.

### Architecture

```text
┌─────────────┐         ┌──────────────┐         ┌──────────────────┐
│  CDS Client │─hook──▶ │  CDS Server  │─calls──▶│ CdsHooksService  │
│   (EHR)     │◀─cards─ │  (your app)  │◀─resp── │ (your impl)      │
└─────────────┘         └──────────────┘         └──────────────────┘
      │                        │
      │                        ▼
      │                 GET /cds-services ──▶ DiscoveryResponse
      │                 POST /cds-services/{id} ──▶ CdsResponse
      └─feedback──────▶ POST /cds-services/{id}/feedback
```

The CDS Client (an EHR or other clinical system) calls CDS Services at specific points in the clinician's workflow called *hooks*. Each hook provides contextual data (patient, encounter, orders, etc.) and the CDS Service responds with *cards* containing decision support guidance.

## Quick Start

### Implementing a CDS Service

```rust
use async_trait::async_trait;
use helios_cds_hooks::{
    CdsHooksService, CdsRequest, CdsResponse, CdsService, Card, CdsHooksError,
    hooks::{HookContext, PatientViewContext},
};
use std::collections::HashMap;

struct PatientGreeter;

#[async_trait]
impl CdsHooksService for PatientGreeter {
    type Context = PatientViewContext;

    fn definition(&self) -> CdsService {
        CdsService {
            hook: PatientViewContext::HOOK_NAME.to_string(),
            title: Some("Patient Greeter".to_string()),
            description: "Displays a greeting when a patient chart is opened".to_string(),
            id: "patient-greeter".to_string(),
            prefetch: Some(HashMap::from([(
                "patientToGreet".to_string(),
                "Patient/{{context.patientId}}".to_string(),
            )])),
            usage_requirements: None,
            version: Some("STU3".to_string()),
            extension: None,
        }
    }

    async fn call(
        &self,
        request: &CdsRequest,
        context: &PatientViewContext,
    ) -> Result<CdsResponse, CdsHooksError> {
        // Access prefetched patient data
        let greeting = if let Some(prefetch) = &request.prefetch {
            if let Some(Some(patient)) = prefetch.get("patientToGreet") {
                let name = patient["name"][0]["given"][0]
                    .as_str()
                    .unwrap_or("Patient");
                format!("Hello, {}!", name)
            } else {
                format!("Hello, patient {}!", context.patient_id)
            }
        } else {
            format!("Hello, patient {}!", context.patient_id)
        };

        Ok(CdsResponse::with_cards(vec![
            Card::info(greeting, "Patient Greeter"),
        ]))
    }
}
```

### Creating Cards

Cards are the primary mechanism for delivering decision support to clinicians. The crate provides convenience constructors for common card types:

```rust
use helios_cds_hooks::{Card, Indicator, Source, Suggestion, Action, ActionType, Link, LinkType};

// Simple informational card
let info_card = Card::info("Patient is up to date on vaccinations", "Immunization Checker");

// Warning with details
let mut warning_card = Card::warning(
    "Potential drug-drug interaction detected",
    "Drug Interaction Checker",
);
warning_card.detail = Some("Amoxicillin may interact with Warfarin. \
    Consider monitoring INR more frequently.".to_string());

// Critical alert with suggestions
let mut critical_card = Card::critical(
    "Severe allergy to prescribed medication",
    "Allergy Safety Alert",
);
critical_card.suggestions = Some(vec![
    Suggestion {
        label: "Remove Penicillin from order".to_string(),
        uuid: Some("remove-penicillin".to_string()),
        is_recommended: Some(true),
        actions: Some(vec![Action {
            action_type: ActionType::Delete,
            description: Some("Remove the Penicillin order".to_string()),
            resource: None,
            resource_id: Some("MedicationRequest/penicillin-rx-1".to_string()),
        }]),
        action_selection_behavior: None,
    },
]);
critical_card.selection_behavior = Some(helios_cds_hooks::SelectionBehavior::AtMostOne);
```

### Building a Discovery Response

```rust
use helios_cds_hooks::{DiscoveryResponse, CdsService};
use std::collections::HashMap;

let discovery = DiscoveryResponse {
    services: vec![
        CdsService {
            hook: "patient-view".to_string(),
            title: Some("Patient Greeter".to_string()),
            description: "Greets patients when their chart is opened".to_string(),
            id: "patient-greeter".to_string(),
            prefetch: Some(HashMap::from([
                ("patientToGreet".to_string(), "Patient/{{context.patientId}}".to_string()),
            ])),
            usage_requirements: None,
            version: None,
            extension: None,
        },
        CdsService {
            hook: "order-sign".to_string(),
            title: Some("Drug Interaction Checker".to_string()),
            description: "Checks for drug-drug interactions before signing orders".to_string(),
            id: "drug-interaction-checker".to_string(),
            prefetch: Some(HashMap::from([
                ("patient".to_string(), "Patient/{{context.patientId}}".to_string()),
                ("medications".to_string(), "MedicationRequest?patient={{context.patientId}}".to_string()),
            ])),
            usage_requirements: None,
            version: None,
            extension: None,
        },
    ],
};

let json = serde_json::to_string_pretty(&discovery).unwrap();
```

## Supported Hooks

All hooks from the [CDS Hooks Library](https://cds-hooks.hl7.org/hooks/) are supported with strongly-typed context structs:

| Hook | Context Type | Maturity | Description |
|------|-------------|----------|-------------|
| `patient-view` | `PatientViewContext` | 5 - Mature | User opens a patient's record |
| `order-sign` | `OrderSignContext` | 5 - Mature | Clinician is ready to sign orders |
| `order-select` | `OrderSelectContext` | 4 - Documented | Clinician selects a new order |
| `encounter-start` | `EncounterStartContext` | 1 - Submitted | A new encounter begins (admission/check-in) |
| `encounter-discharge` | `EncounterDischargeContext` | 1 - Submitted | Patient is being discharged |
| `appointment-book` | `AppointmentBookContext` | 1 - Submitted | Future appointments are being scheduled |
| `allergyintolerance-create` | `AllergyIntoleranceCreateContext` | 1 - Submitted | A new allergy is being added |
| `medication-refill` | `MedicationRefillContext` | 1 - Submitted | A medication refill is requested |
| `problem-list-item-create` | `ProblemListItemCreateContext` | 1 - Submitted | A new problem is added to the list |
| `order-dispatch` | `OrderDispatchContext` | 0 - Draft | An order is dispatched to a performer |

### Hook Context Fields

Each hook context provides the fields defined in the specification. Fields marked as REQUIRED are non-optional in the struct; OPTIONAL fields use `Option<T>`.

#### patient-view

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | `String` | Yes | Practitioner, PractitionerRole, Patient, or RelatedPerson reference |
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `encounter_id` | `Option<String>` | No | FHIR Encounter.id |

#### order-select

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | `String` | Yes | Practitioner or PractitionerRole reference |
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `encounter_id` | `Option<String>` | No | FHIR Encounter.id |
| `selections` | `Vec<String>` | Yes | FHIR ids of newly selected orders |
| `draft_orders` | `serde_json::Value` | Yes | Bundle of all unsigned orders from the session |

#### order-sign

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | `String` | Yes | Practitioner or PractitionerRole reference |
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `encounter_id` | `Option<String>` | No | FHIR Encounter.id |
| `draft_orders` | `serde_json::Value` | Yes | Bundle of draft orders being signed |

#### encounter-start / encounter-discharge

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | `String` | Yes | Practitioner or PractitionerRole reference |
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `encounter_id` | `String` | Yes | FHIR Encounter.id |

#### appointment-book

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | `String` | Yes | Practitioner, PractitionerRole, Patient, or RelatedPerson reference |
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `encounter_id` | `Option<String>` | No | FHIR Encounter.id |
| `appointments` | `serde_json::Value` | Yes | Bundle of proposed Appointments |

#### order-dispatch

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `dispatched_orders` | `Vec<String>` | Yes | References to Request resources being dispatched |
| `performer` | `String` | Yes | Reference to the performer being assigned |
| `fulfillment_tasks` | `Option<Vec<Value>>` | No | Task resources describing fulfillment requests |

#### allergyintolerance-create

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | `String` | Yes | Practitioner reference |
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `encounter_id` | `Option<String>` | No | FHIR Encounter.id |
| `allergy_intolerance` | `serde_json::Value` | Yes | The AllergyIntolerance resource being created |

#### medication-refill

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | `Option<String>` | No | Practitioner or PractitionerRole reference (may not be in context) |
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `encounter_id` | `Option<String>` | No | FHIR Encounter.id |
| `medications` | `serde_json::Value` | Yes | Bundle of draft MedicationRequest resources |

#### problem-list-item-create

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | `String` | Yes | Practitioner or PractitionerRole reference |
| `patient_id` | `String` | Yes | FHIR Patient.id |
| `encounter_id` | `Option<String>` | No | FHIR Encounter.id |
| `conditions` | `serde_json::Value` | Yes | Bundle of Condition resources with category `problem-list-item` |

## Core Types

### Protocol Types

| Type | Description |
|------|-------------|
| `CdsRequest` | Hook request sent from CDS Client to CDS Service |
| `CdsResponse` | Response containing cards and optional system actions |
| `DiscoveryResponse` | Response to the discovery endpoint listing available services |
| `CdsService` | Service metadata for the discovery endpoint |
| `FhirAuthorization` | OAuth 2.0 token and metadata for FHIR server access |

### Card Types

| Type | Description |
|------|-------------|
| `Card` | Decision support card with summary, detail, indicator, source |
| `Source` | Information source for a card (label, URL, icon, topic) |
| `Suggestion` | A suggested set of changes (may contain multiple actions) |
| `Action` | A create, update, or delete operation on a FHIR resource |
| `Link` | A link to an app or external information |
| `Coding` | A code from a code system (CDS Hooks-specific, not FHIR) |

### Feedback Types

| Type | Description |
|------|-------------|
| `FeedbackRequest` | Feedback sent from CDS Client about card outcomes |
| `Feedback` | Outcome for a single card (accepted or overridden) |
| `AcceptedSuggestion` | Identifies a suggestion that was accepted |
| `OverrideReason` | Coded reason and optional comment for overriding a card |

### Enums

| Enum | Values | Description |
|------|--------|-------------|
| `Indicator` | `Info`, `Warning`, `Critical` | Card urgency level |
| `ActionType` | `Create`, `Update`, `Delete` | FHIR resource modification type |
| `LinkType` | `Absolute`, `Smart` | URL type (plain URL or SMART launch) |
| `SelectionBehavior` | `AtMostOne`, `Any` | Card suggestion selection behavior |
| `ActionSelectionBehavior` | `All`, `Any`, `AtMostOne` | Action selection within a suggestion |
| `FeedbackOutcome` | `Accepted`, `Overridden` | Outcome of user interaction with a card |

## The CdsHooksService Trait

The `CdsHooksService` trait is the main extension point for implementing CDS services:

```rust
#[async_trait]
pub trait CdsHooksService: Send + Sync {
    type Context: HookContext;

    /// Service metadata for the discovery endpoint.
    fn definition(&self) -> CdsService;

    /// Process a hook request and return decision support cards.
    async fn call(
        &self,
        request: &CdsRequest,
        context: &Self::Context,
    ) -> Result<CdsResponse, CdsHooksError>;

    /// Handle feedback about card outcomes (optional).
    async fn on_feedback(&self, feedback: &FeedbackRequest) -> Result<(), CdsHooksError>;

    /// Extract and deserialize the hook context from a raw request.
    fn extract_context(&self, request: &CdsRequest) -> Result<Self::Context, CdsHooksError>;
}
```

### Error Handling

`CdsHooksError` maps to appropriate HTTP status codes:

| Variant | HTTP Status | When to Use |
|---------|------------|-------------|
| `PreconditionFailed` | 412 | Required prefetch data is missing and cannot be fetched |
| `InvalidContext` | 400 | The request context doesn't match the expected hook |
| `InternalError` | 500 | An unexpected error occurred during processing |

## Serialization

All types implement `Serialize` and `Deserialize` with JSON field names matching the CDS Hooks specification exactly:

```rust
use helios_cds_hooks::CdsResponse;

// Deserialize from a JSON string
let json = r#"{"cards": []}"#;
let response: CdsResponse = serde_json::from_str(json).unwrap();

// Serialize to JSON
let json_out = serde_json::to_string_pretty(&response).unwrap();
```

Rust field names use `snake_case` while JSON serialization uses the `camelCase` names required by the specification (e.g. `hook_instance` serializes to `hookInstance`, `draft_orders` to `draftOrders`).

Optional fields are omitted from serialized JSON when `None`, following the CDS Hooks specification requirement that optional attributes with no value must be omitted.

## FHIR Version Agnosticism

CDS Hooks is designed to be agnostic of FHIR version. Context fields that contain FHIR resources (such as `draftOrders`, `appointments`, `conditions`, etc.) are typed as `serde_json::Value` to accommodate any FHIR version. This mirrors the specification's design where the same hook may carry R4, R5, or other FHIR version resources.

## Testing

```bash
# Run all tests
cargo test -p helios-cds-hooks

# Run with output
cargo test -p helios-cds-hooks -- --nocapture
```

## References

- [CDS Hooks Specification (v3.0.0-ballot)](https://cds-hooks.hl7.org/)
- [CDS Hooks Library](https://cds-hooks.hl7.org/hooks/)
- [CDS Hooks GitHub](https://github.com/cds-hooks)
- [SMART on FHIR](https://smarthealthit.org/)

## License

MIT
