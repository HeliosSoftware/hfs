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
/// [`kind`] and [`derivation`] mirror `StructureDefinition.kind` and
/// `StructureDefinition.derivation` (FHIR ValueSets, aligned with
/// `helios-fhir` R5 terminology code systems).
#[derive(Debug, Clone, Default)]
pub struct ExtractedProfile {
    pub url: String,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    /// The `StructureDefinition.type` code (root path / type name); e.g. `Patient` or `Address`.
    pub resource_type: String,
    pub base_definition: Option<String>,
    /// `StructureDefinition.snapshot.extension` `snapshot-base-version` `valueString`, when
    /// present (FHIR IG tooling); used to pick HL7 web package paths for `baseDefinition` fetch.
    pub snapshot_base_version: Option<String>,

    pub kind: StructureDefinitionKind,
    pub derivation: TypeDerivationRule,

    pub invariants: Vec<InvariantDef>,
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
    pub id: String,
    pub path: String,
    pub min: Option<u32>,
    pub max: Option<String>,
    pub binding: Option<BindingDef>,
    pub constraints: Vec<InvariantDef>,
    pub value_constraint: Option<ExtractedValueConstraint>,
    pub type_constraints: Vec<ExtractedTypeConstraint>,
    pub slicing: Option<ExtractedSlicing>,
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
    pub code: String,
    pub profiles: Vec<String>,
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
    pub discriminators: Vec<ExtractedSliceDiscriminator>,
    pub ordered: bool,
    pub rules: ExtractedSlicingRules,
}

/// One extracted slicing discriminator.
///
/// `discriminator_type` identifies how slice membership is differentiated, and
/// `path` identifies the nominated element relative to the sliced element.
#[derive(Debug, Clone)]
pub struct ExtractedSliceDiscriminator {
    pub discriminator_type: ExtractedDiscriminatorType,
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
