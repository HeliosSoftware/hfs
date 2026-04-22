//! Normalized intermediate models used by the validation generator.
//!
//! This module defines the generator-facing data structures produced by
//! `extract.rs` and consumed by `emit.rs`.
//!
//! The goal of these models is to separate raw FHIR specification parsing from
//! Rust code emission. Instead of emitting directly from raw
//! `StructureDefinition` / `ElementDefinition` JSON, the generator first
//! normalizes the data into a stable intermediate representation.
//!
//! Key design points:
//! - `TypeValidationModel` represents one generated Rust type that may receive
//!   validation metadata
//! - `StructureKind` captures the FHIR specification category of a type
//!   (`resource`, `complex-type`, `primitive-type`, `logical`)
//! - `ParentKind` captures inheritance/runtime behavior used by emitted
//!   validation code (`DomainResource`, `BackboneElement`, `Element`, etc.)
//! - `FieldModel`, `InvariantModel`, and `BindingModel` capture the direct
//!   child metadata needed for recursive validation emission
//!
//! This split keeps extraction FHIR-aware and emission Rust-oriented.

use std::collections::BTreeMap;

use fhir_validation_types::BindingTargetKind;

/// Normalized validation metadata for one generated Rust type.
///
/// A single FHIR `StructureDefinition` may produce:
/// - one root `TypeValidationModel` for the main generated type
/// - additional nested models for generated backbone / element-derived helper
///   types such as `Patient.contact`
///
/// This model is the main contract between extraction and emission.
#[derive(Debug, Clone, Default)]
pub struct TypeValidationModel {
    /// Rust type name in the generated FHIR crate, e.g. `Patient`.
    pub rust_type: String,

    /// Canonical FHIR path / root path, e.g. `Patient` or `Patient.contact`.
    pub fhir_path: String,

    /// The source StructureDefinition canonical URL, when available.
    pub structure_definition_url: Option<String>,

    /// The base definition canonical URL, when available.
    pub base_definition: Option<String>,

    /// FHIR specification category for this generated type.
    ///
    /// This follows `StructureDefinition.kind` semantics:
    /// `resource`, `complex-type`, `primitive-type`, or `logical`.
    pub structure_kind: StructureKind,

    /// Inheritance/runtime family inferred from base definition or typed
    /// element declarations.
    ///
    /// This is used by emitted validation code to decide recursive behavior for
    /// things like `DomainResource`, `BackboneElement`, or `Element`.
    pub parent_kind: ParentKind,

    /// Invariants declared directly on this type path.
    pub invariants: Vec<InvariantModel>,

    /// Direct child bindings for this type path.
    pub bindings: Vec<BindingModel>,

    /// Direct child fields of this type from snapshot elements.
    pub fields: Vec<FieldModel>,

    /// Optional ancestry / hierarchy hints captured during extraction.
    ///
    /// Key = child type, value = direct parent type.
    pub direct_supertypes: BTreeMap<String, String>,
}

impl TypeValidationModel {
    /// Construct a new normalized validation model with the supplied Rust type,
    /// FHIR path, and inheritance/runtime family.
    ///
    /// `structure_kind` is initialized as `Unknown` and is filled in during
    /// extraction.
    pub fn new(
        rust_type: impl Into<String>,
        fhir_path: impl Into<String>,
        parent_kind: ParentKind,
    ) -> Self {
        Self {
            rust_type: rust_type.into(),
            fhir_path: fhir_path.into(),
            structure_definition_url: None,
            base_definition: None,
            structure_kind: StructureKind::Unknown,
            parent_kind,
            invariants: Vec::new(),
            bindings: Vec::new(),
            fields: Vec::new(),
            direct_supertypes: BTreeMap::new(),
        }
    }
}

/// FHIR specification category for a generated type.
///
/// This mirrors `StructureDefinition.kind`, not inheritance.
///
/// Important:
/// - `Resource` means the type is a FHIR resource in specification terms
/// - nested backbone/helper types inside resources are still usually
///   `ComplexType`
/// - inheritance-specific concepts such as `BackboneElement` and
///   `DomainResource` are represented separately by `ParentKind`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StructureKind {
    #[default]
    Unknown,
    Resource,
    ComplexType,
    PrimitiveType,
    Logical,
}

/// High-level inheritance/runtime classification of a generated FHIR Rust type.
///
/// Unlike `StructureKind`, this enum reflects how emitted validation code should
/// treat a type structurally:
/// - `DomainResource` and `Resource` affect resource-level recursion
/// - `BackboneElement` and `Element` affect nested datatype/backbone traversal
/// - `Primitive` / `ComplexType` capture non-resource helper behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParentKind {
    #[default]
    Unknown,
    Primitive,
    Element,
    BackboneElement,
    ComplexType,
    Resource,
    DomainResource,
}

/// Normalized metadata for one direct child field under a generated type.
///
/// This information is used during emission to:
/// - recurse into nested datatypes / backbone elements
/// - emit correct instance-path rebasing
/// - recognize repeated fields
/// - recognize and handle FHIR choice elements
#[derive(Debug, Clone, Default)]
pub struct FieldModel {
    /// Snapshot element id, e.g. `Patient.gender`.
    #[allow(dead_code)]
    pub element_id: String,

    /// Snapshot element path, e.g. `Patient.gender`.
    pub fhir_path: String,

    /// Rust field name expected in the generated model, e.g. `gender`,
    /// `multiple_birth`.
    pub rust_field_name: String,

    /// Original FHIR field name, e.g. `gender`, `multipleBirth`.
    #[allow(dead_code)]
    pub fhir_field_name: String,

    /// True when the element path is a choice like `value[x]`.
    pub is_choice: bool,

    /// Base name of a FHIR choice element, e.g. `value`, `multipleBirth`.
    #[allow(dead_code)]
    pub choice_base_name: Option<String>,

    /// Generated Rust enum name for a FHIR choice element, e.g.
    /// `PatientMultipleBirth`.
    pub choice_enum_name: Option<String>,

    /// Declared FHIR type codes for the element.
    pub type_codes: Vec<String>,

    /// Declared target profiles, if any.
    #[allow(dead_code)]
    pub target_profiles: Vec<String>,

    /// Declared profiles, if any.
    #[allow(dead_code)]
    pub profiles: Vec<String>,

    /// Minimum cardinality.
    #[allow(dead_code)]
    pub min: u32,

    /// Raw max cardinality, e.g. `1`, `*`.
    #[allow(dead_code)]
    pub max: String,

    /// True when the element is repeating.
    pub is_array: bool,

    /// True when min > 0.
    pub is_required: bool,
}

/// Normalized invariant definition extracted from `StructureDefinition`.
///
/// Each invariant is attached to the generated type path on which it is
/// declared and is later emitted as a generated `InvariantDef`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InvariantModel {
    pub key: String,
    pub severity: SeverityModel,
    pub path: String,
    pub expression: String,
    pub human: String,
    pub source: Option<String>,
    pub element_id: String,
}

/// Normalized binding definition extracted from `ElementDefinition.binding`.
///
/// Each binding is attached to the generated type path that owns the bound
/// direct child field. During emission, these models become generated
/// `BindingDef`s used by version-specific binding application code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BindingModel {
    pub path: String,
    pub strength: BindingStrengthModel,
    pub value_set: String,
    pub binding_name: Option<String>,
    pub description: Option<String>,

    /// The bindable runtime shape of the target field:
    /// primitive `code`, `Coding`, `CodeableConcept`, or unsupported.
    pub target_kind: BindingTargetKindModel,
    pub element_id: String,
    pub element_path: String,
    pub type_codes: Vec<String>,

    /// The subset of declared type codes that are actually bindable by the
    /// current validator.
    pub bindable_type_codes: Vec<String>,

    /// True when this binding comes from a choice element such as `value[x]`.
    pub is_choice_binding: bool,
}

/// Internal invariant severity representation used during extraction before
/// emission into runtime validation types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeverityModel {
    Fatal,
    #[default]
    Error,
    Warning,
    Information,
}

impl SeverityModel {
    /// Render this internal severity as emitted Rust tokens for generated code.
    pub fn as_rust_tokens(self) -> &'static str {
        match self {
            Self::Fatal => "fhir_validation_types::Severity::Fatal",
            Self::Error => "fhir_validation_types::Severity::Error",
            Self::Warning => "fhir_validation_types::Severity::Warning",
            Self::Information => "fhir_validation_types::Severity::Information",
        }
    }
}

/// Internal binding strength representation used during extraction before
/// emission into runtime validation types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BindingStrengthModel {
    Required,
    Extensible,
    Preferred,
    #[default]
    Example,
}

impl BindingStrengthModel {
    /// Render this internal binding strength as emitted Rust tokens for
    /// generated code.
    pub fn as_rust_tokens(self) -> &'static str {
        match self {
            Self::Required => "fhir_validation_types::BindingStrength::Required",
            Self::Extensible => "fhir_validation_types::BindingStrength::Extensible",
            Self::Preferred => "fhir_validation_types::BindingStrength::Preferred",
            Self::Example => "fhir_validation_types::BindingStrength::Example",
        }
    }
}

/// Internal classification of the bindable runtime shape of an element.
///
/// This is extracted from the element's declared type codes and later emitted
/// as `fhir_validation_types::BindingTargetKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BindingTargetKindModel {
    Code,
    Coding,
    CodeableConcept,
    String,
    Uri,
    Choice,
    CodeableReference,
    Quantity,
    #[default]
    Unsupported,
}

impl BindingTargetKindModel {
    /// Render this internal binding target kind as emitted Rust tokens for
    /// generated code.
    pub fn as_rust_tokens(self) -> &'static str {
        match self {
            Self::Code => "fhir_validation_types::BindingTargetKind::Code",
            Self::Coding => "fhir_validation_types::BindingTargetKind::Coding",
            Self::CodeableConcept => "fhir_validation_types::BindingTargetKind::CodeableConcept",
            Self::Unsupported => "fhir_validation_types::BindingTargetKind::Unsupported",
            Self::String => "fhir_validation_types::BindingTargetKind::String",
            Self::Uri => "fhir_validation_types::BindingTargetKind::Uri",
            Self::Choice => "fhir_validation_types::BindingTargetKind::Choice",
            Self::CodeableReference => {
                "fhir_validation_types::BindingTargetKind::CodeableReference"
            }
            Self::Quantity => "fhir_validation_types::BindingTargetKind::Quantity",
        }
    }
}

impl From<BindingTargetKind> for BindingTargetKindModel {
    fn from(k: BindingTargetKind) -> Self {
        match k {
            BindingTargetKind::Code => Self::Code,
            BindingTargetKind::Coding => Self::Coding,
            BindingTargetKind::CodeableConcept => Self::CodeableConcept,
            BindingTargetKind::String => Self::String,
            BindingTargetKind::Uri => Self::Uri,
            BindingTargetKind::Choice => Self::Choice,
            BindingTargetKind::CodeableReference => Self::CodeableReference,
            BindingTargetKind::Quantity => Self::Quantity,
            BindingTargetKind::Unsupported => Self::Unsupported,
        }
    }
}
