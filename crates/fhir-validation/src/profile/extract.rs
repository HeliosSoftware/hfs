//! [`StructureDefinition`] → [`ExtractedProfile`] conversion.
//!
//! # Differential vs snapshot
//!
//! FHIR stores **constraint profiles** in two parallel views:
//! - **Differential** — only elements the profile *changes*, relative to
//!   `StructureDefinition.base_definition`. Omitted fields mean “unchanged from
//!   base,” not “absent.”
//! - **Snapshot** — the full merged element tree (base + profile), suitable for
//!   operational use; each row is a complete [`ElementDefinition`] for that path.
//!
//! This module **only walks** the differential’s `element` list (`StructureDefinitionDifferential`).
//! It does **not** iterate the snapshot’s `element` list by itself, so the set of
//! paths and the scope of profile validation match the profile *delta* (what
//! appears in the differential). Base resource validation is assumed to happen
//! elsewhere.
//!
//! When a snapshot is present (typical in published Implementation Guides), each
//! differential path is **resolved** to the snapshot row with the same
//! `path`, and **that** row is used for extraction.
//! Rationale: the snapshot row is the merged truth and carries inherited
//! metadata (e.g. `type` for terminology binding target kind) that sparse
//! differential rows may omit. If the snapshot is missing or has no row for a
//! path, the differential row is used as-is.
//!
//! Resolution is **whole-row**: we pick either the snapshot or the differential
//! [`ElementDefinition`], not a field-by-field merge. Published IGs are built so
//! the snapshot matches the profile; we do **not** compute a snapshot from base
//! + differential here (that is the IG Publisher / tooling layer).
//!
//! [`ExtractedElementRule`]'s `id` is always taken from the **differential** row for
//! authoring alignment; all other extracted fields come from the resolved
//! [`ElementDefinition`].
//!
//! Snapshot-only StructureDefinitions (no differential) are **not** supported;
//! extraction requires a non-empty differential element list.
//!
//! # `ElementDefinition.condition`
//!
//! [`ElementDefinition.condition`](https://hl7.org/fhir/elementdefinition-definitions.html#ElementDefinition.condition)
//! is **not** extracted. Those ids reference separate `ElementDefinition.constraint`
//! entries whose applicability is conditional; evaluating them requires a dedicated
//! condition engine and is left for future work.
//!
//! # `StructureDefinition.kind` and `StructureDefinition.derivation`
//!
//! Extraction **requires** both fields (non-empty codes). Values are parsed
//! against the same code sets as the FHIR ValueSets
//! [`structure-definition-kind`](https://hl7.org/fhir/valueset-structure-definition-kind.html)
//! and [`type-derivation-rule`](https://hl7.org/fhir/valueset-type-derivation-rule.html),
//! aligned with [`fhir_validation_types`]. Unknown codes are
//! rejected with [`crate::ValidationError::InvalidStructureDefinition`]. The parsed
//! values are stored on [`ExtractedProfile`] for downstream use; instance
//! validation may still be limited to resource-shaped data depending on the
//! caller or validator configuration.
//!
//! # Multi-version extraction
//!
//! R4, R4B, and R5 `StructureDefinition` resources share the same FHIR JSON shape
//! for the fields used here. The implementation is centralized in
//! [`crate::profile::extract_core::extract_structure_definition_profile_from_json`];
//! version-specific entry points serialize typed resources to JSON and delegate to
//! that function.

use crate::ValidationError;
use crate::profile::structure_definition_extract::StructureDefinitionExtractMessage as SdMsg;
use crate::profile::types::ExtractedProfile;

pub use crate::profile::extract_core::extract_structure_definition_profile_from_json;
pub use crate::profile::extract_core::prune_json_nulls;

/// Extract normalized validation metadata from an R4 `StructureDefinition`.
#[cfg(feature = "R4")]
pub fn extract_r4_structure_definition_profile(
    sd: &helios_fhir::r4::StructureDefinition,
) -> Result<ExtractedProfile, ValidationError> {
    let v = serde_json::to_value(sd).map_err(|err| {
        ValidationError::from(SdMsg::SerializeFailed {
            error: err.to_string(),
        })
    })?;
    extract_structure_definition_profile_from_json(&v)
}

/// Extract normalized validation metadata from an R4B `StructureDefinition`.
#[cfg(feature = "R4B")]
pub fn extract_r4b_structure_definition_profile(
    sd: &helios_fhir::r4b::StructureDefinition,
) -> Result<ExtractedProfile, ValidationError> {
    let v = serde_json::to_value(sd).map_err(|err| {
        ValidationError::from(SdMsg::SerializeFailed {
            error: err.to_string(),
        })
    })?;
    extract_structure_definition_profile_from_json(&v)
}

/// Extract normalized validation metadata from an R5 `StructureDefinition`.
///
/// See the [module-level overview](crate::profile::extract) for differential vs
/// snapshot behavior.
///
/// The extracted result is an [`ExtractedProfile`] consumed by the runtime
/// validator.
#[cfg(feature = "R5")]
pub fn extract_r5_structure_definition_profile(
    sd: &helios_fhir::r5::StructureDefinition,
) -> Result<ExtractedProfile, ValidationError> {
    let v = serde_json::to_value(sd).map_err(|err| {
        ValidationError::from(SdMsg::SerializeFailed {
            error: err.to_string(),
        })
    })?;
    extract_structure_definition_profile_from_json(&v)
}

#[cfg(feature = "R6")]
pub fn extract_r6_structure_definition_profile(
    sd: &helios_fhir::r6::StructureDefinition,
) -> Result<ExtractedProfile, ValidationError> {
    let v = serde_json::to_value(sd).map_err(|err| {
        ValidationError::from(SdMsg::SerializeFailed {
            error: err.to_string(),
        })
    })?;
    extract_structure_definition_profile_from_json(&v)
}
