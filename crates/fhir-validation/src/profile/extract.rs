//! [`StructureDefinition`](https://hl7.org/fhir/structuredefinition.html) → [`ExtractedProfile`] conversion.
//!
//! # Differential vs snapshot
//!
//! FHIR stores **constraint profiles** in two parallel views:
//! - **Differential** — only elements the profile *changes*, relative to
//!   `StructureDefinition.base_definition`. Omitted fields mean “unchanged from
//!   base,” not “absent.”
//! - **Snapshot** — the full merged element tree (base + profile), suitable for
//!   operational use; each row is a complete [`ElementDefinition`](https://hl7.org/fhir/elementdefinition.html) for that path.
//!
//! Extraction is **snapshot-first**:
//! - When `snapshot.element` is present, extraction walks the snapshot rows.
//! - When snapshot is absent, extraction falls back to differential rows.
//!
//! A present snapshot must be well-formed (`snapshot.element` non-empty array);
//! malformed/empty snapshots are rejected as invalid `StructureDefinition`
//! instead of silently falling back to differential.
//!
//! Snapshot-first keeps inherited metadata and constraints from the full merged
//! profile view (base chain + local deltas), which is required for profile-on-
//! profile validation scenarios.
//!
//! [`ExtractedElementRule`](crate::profile::types::ExtractedElementRule)'s `id` prefers the **differential** row for authoring
//! alignment when a matching path exists; otherwise snapshot `id` (or `path`) is
//! used.
//!
//! Snapshot-only `StructureDefinition`s (**no** `differential.element`, or an empty
//! differential when `snapshot.element` is present) are supported: extraction walks
//! snapshot rows and uses each element’s `id` when no matching differential path exists.
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
//!
//! # Version entry points
//!
//! Use `extract_r4_structure_definition_profile`, `extract_r5_structure_definition_profile`, etc.
//! (`cfg` per crate features) when you already have a typed `helios_fhir` `StructureDefinition`.
//! For JSON from disk or unknown version, call [`extract_structure_definition_profile_from_json`]
//! directly.

use crate::ValidationError;
use crate::profile::structure_definition_extract::StructureDefinitionExtractMessage as SdMsg;
use crate::profile::types::ExtractedProfile;

pub use crate::profile::extract_core::extract_structure_definition_profile_from_json;
pub use crate::profile::extract_core::prune_json_nulls;

/// Extract normalized validation metadata from an R4 `StructureDefinition`.
///
/// See the [module-level overview](crate::profile::extract) for differential vs snapshot behavior.
/// The result is an [`ExtractedProfile`] for [`crate::profile::validate::validate_profile`].
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
///
/// See the [module-level overview](crate::profile::extract). Output is an [`ExtractedProfile`]
/// for [`crate::profile::validate::validate_profile`].
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
/// validator ([`crate::profile::validate::validate_profile`]).
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

/// Extract normalized validation metadata from an R6 `StructureDefinition`.
///
/// See the [module-level overview](crate::profile::extract). Output is an [`ExtractedProfile`]
/// for [`crate::profile::validate::validate_profile`].
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
