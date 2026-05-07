//! Extracted **runtime model** for a single `StructureDefinition` and its element rows.
//!
//! These types are **not** a full faithful mirror of FHIR `ElementDefinition`—only fields needed
//! for validation in this crate are retained. Construction is always via
//! [`crate::profile::extract`] / [`crate::profile::extract_core::extract_structure_definition_profile_from_json`].
//!
//! # Rule list semantics
//!
//! [`ExtractedProfile::element_rules`] is a flat list of [`ExtractedElementRule`] records keyed by
//! [`ExtractedElementRule::path`] (and slice identity via [`ExtractedElementRule::slice_name`]).
//! Slicing **base** rows carry [`ExtractedElementRule::slicing`]; individual slices are separate
//! rules sharing the same path with distinct slice names.

use fhir_validation_types::{
    BindingDef, InvariantDef, StructureDefinitionKind, TypeDerivationRule,
};

/// Extracted validation metadata for a single constrained profile.
///
/// Built from [`StructureDefinition`](https://hl7.org/fhir/structuredefinition.html) via
/// [`crate::profile::extract`] (version-specific extractors and
/// [`extract_structure_definition_profile_from_json`](crate::profile::extract::extract_structure_definition_profile_from_json)).
/// Extraction is **snapshot-first** when `snapshot.element` is present, with
/// differential fallback when snapshot is absent. See `extract` module docs for
/// details.
///
/// [`ExtractedProfile::kind`] and [`ExtractedProfile::derivation`] mirror `StructureDefinition.kind` and
/// `StructureDefinition.derivation` (FHIR ValueSets, aligned with
/// `helios-fhir` R5 terminology code systems).
#[derive(Debug, Clone, Default)]
pub struct ExtractedProfile {
    /// Canonical `StructureDefinition.url` (globally unique profile identifier).
    pub url: String,
    /// `StructureDefinition.version` when present.
    pub version: Option<String>,
    /// Short machine name from the SD (`name`).
    pub name: Option<String>,
    /// Human title (`title`).
    pub title: Option<String>,
    /// The `StructureDefinition.type` code (root path / type name); e.g. `Patient` or `Address`.
    pub resource_type: String,
    /// `StructureDefinition.baseDefinition` canonical when the profile derives from another SD.
    pub base_definition: Option<String>,
    /// `StructureDefinition.snapshot.extension` `snapshot-base-version` `valueString`, when
    /// present (FHIR IG tooling); used to pick HL7 web package paths for `baseDefinition` fetch.
    pub snapshot_base_version: Option<String>,

    /// `kind` — `resource`, `logical`, `complex-type`, or `primitive-type`.
    pub kind: StructureDefinitionKind,
    /// `derivation` — `specialization` vs `constraint` for this SD.
    pub derivation: TypeDerivationRule,

    /// Resource-level (`Patient`-path) `ElementDefinition.constraint` invariants from extraction.
    pub invariants: Vec<InvariantDef>,
    /// All extracted element rows (root and nested), including slice rows.
    pub element_rules: Vec<ExtractedElementRule>,
}

// impl Default for ExtractedProfile {
//     fn default() -> Self {
//         Self {
//             url: String::new(),
//             version: None,
//             name: None,
//             title: None,
//             resource_type: String::new(),
//             base_definition: None,
//             kind: StructureDefinitionKind::default(),
//             derivation: TypeDerivationRule::default(),
//             invariants: Vec::new(),
//             element_rules: Vec::new(),
//         }
//     }
// }

/// Extracted fixed or pattern constraint captured as normalized JSON.
///
/// These constraints are used both for direct value validation and for slicing
/// discriminators of kind `value`.
#[derive(Debug, Clone)]
pub enum ExtractedValueConstraint {
    Fixed(serde_json::Value),
    Pattern(serde_json::Value),
}

/// Extracted rule metadata for a single constrained element path.
///
/// Depending on the source ElementDefinition, a rule may carry cardinality,
/// binding, invariant, fixed/pattern, type, or slicing metadata.
///
/// `ElementDefinition.condition` is **not** extracted: enforcing conditional
/// element applicability requires evaluating the referenced condition expressions
/// in context, which is deferred to future work.
#[derive(Debug, Clone, Default)]
pub struct ExtractedElementRule {
    /// Element `id` from the SD (differential `id` preferred when merged with snapshot paths).
    pub id: String,
    /// `ElementDefinition.path` — dotted logical path, with `[x]` for true polymorphic elements.
    pub path: String,
    /// `min` cardinality when constrained in this row.
    pub min: Option<u32>,
    /// `max` as in FHIR (`"*"`, `"1"`, `"0"`, …).
    pub max: Option<String>,
    /// Terminology binding when declared (`ValueSet` / `CodeSystem`).
    pub binding: Option<BindingDef>,
    /// `ElementDefinition.constraint` rows (FHIRPath invariants) attached to this path.
    pub constraints: Vec<InvariantDef>,
    /// `fixed[x]` / `pattern[x]` normalized to JSON for this element, when present.
    pub value_constraint: Option<ExtractedValueConstraint>,
    /// `ElementDefinition.type` entries (code + optional `profile` / `targetProfile` URLs).
    pub type_constraints: Vec<ExtractedTypeConstraint>,
    /// Present on the **introducer** row for a sliced repeating element (`ElementDefinition.slicing`).
    pub slicing: Option<ExtractedSlicing>,
    /// `sliceName` for slice rows; distinguishes slices sharing the same `path`.
    pub slice_name: Option<String>,
    /// `ElementDefinition.maxLength` (string-like primitives).
    pub max_length: Option<u32>,
    /// `ElementDefinition.minValue[x]` as JSON (single `minValue*` key), for runtime checks.
    ///
    /// Values are produced from the typed `ElementDefinition` min-value union (R5) via
    /// `serde_json::to_value`, so validation sees the same shape as FHIR JSON without
    /// tying the extracted model to a specific generated enum type.
    pub min_value: Option<serde_json::Value>,
    /// `ElementDefinition.maxValue[x]` as JSON (single `maxValue*` key).
    ///
    /// Same encoding approach as [`ExtractedElementRule::min_value`].
    pub max_value: Option<serde_json::Value>,
    /// `ElementDefinition.mustSupport`.
    pub must_support: Option<bool>,
    /// `ElementDefinition.isModifier`.
    pub is_modifier: Option<bool>,
    /// `ElementDefinition.isModifierReason`.
    pub is_modifier_reason: Option<String>,
}

// impl Default for ExtractedElementRule {
//     fn default() -> Self {
//         Self {
//             id: String::new(),
//             path: String::new(),
//             min: None,
//             max: None,
//             binding: None,
//             constraints: Vec::new(),
//             value_constraint: None,
//             type_constraints: Vec::new(),
//             slicing: None,
//             slice_name: None,
//             max_length: None,
//             min_value: None,
//             max_value: None,
//             must_support: None,
//             is_modifier: None,
//             is_modifier_reason: None,
//         }
//     }
// }
/// Extracted type constraint for an element.
///
/// `code` is the primary allowed type code, while `profiles` and
/// `target_profiles` capture profile-qualified type restrictions when present.
///
/// `aggregation` and `versioning` mirror `ElementDefinition.type.aggregation` and
/// `ElementDefinition.type.versioning` (reference semantics); they are empty or
/// absent when the profile does not declare them.
#[derive(Debug, Clone, Default)]
pub struct ExtractedTypeConstraint {
    /// Primary `ElementDefinition.type.code` (e.g. `string`, `CodeableConcept`, `Reference`).
    pub code: String,
    /// `type.profile` canonical URLs constraining the datatype / profile.
    pub profiles: Vec<String>,
    /// `type.targetProfile` URLs for reference targets, when declared.
    pub target_profiles: Vec<String>,
    /// Resource aggregation mode codes (`contained`, `referenced`, `bundled`), when present.
    pub aggregation: Vec<String>,
    /// Reference version rule (`either`, `independent`, `specific`), when present.
    pub versioning: Option<String>,
}

/// Extracted slicing metadata for a repeating element.
///
/// This corresponds to `ElementDefinition.slicing` and records the declared
/// discriminators, whether slice order matters, and the slicing openness rules.
#[derive(Debug, Clone)]
pub struct ExtractedSlicing {
    /// Ordered discriminator specifications (`type`, `value`, …) evaluated for slice matching.
    pub discriminators: Vec<ExtractedSliceDiscriminator>,
    /// When true, instance array order must follow slice declaration order for `position` rules.
    pub ordered: bool,
    /// Whether slices are closed, open, or open-at-end relative to undiscriminated repeats.
    pub rules: ExtractedSlicingRules,
}

/// One extracted slicing discriminator.
///
/// `discriminator_type` identifies how slice membership is differentiated, and
/// `path` identifies the nominated element relative to the sliced element.
#[derive(Debug, Clone)]
pub struct ExtractedSliceDiscriminator {
    /// Discriminator kind from the profile (FHIR `discriminator.type`).
    pub discriminator_type: ExtractedDiscriminatorType,
    /// Element path relative to the sliced node (FHIR `discriminator.path`).
    pub path: String,
}

/// Supported discriminator kinds extracted from
/// `ElementDefinition.slicing.discriminator.type`.
///
/// Note that runtime support may intentionally lag behind the full set of
/// extracted enum values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractedDiscriminatorType {
    Value,
    Exists,
    Pattern,
    Type,
    Profile,
    Position,
}

/// Extracted slicing openness rules.
///
/// These correspond to the FHIR slicing rule codes:
/// - `Closed`
/// - `Open`
/// - `OpenAtEnd`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractedSlicingRules {
    Closed,
    Open,
    OpenAtEnd,
}
