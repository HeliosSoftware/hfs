//! Messages for [`crate::ValidationError::InvalidStructureDefinition`] when parsing
//! `StructureDefinition` JSON for profile validation.

use std::fmt;

/// Structured reason for a failed StructureDefinition extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureDefinitionExtractMessage {
    JsonMustBeObject,
    MissingResourceType,
    ExpectedResourceType { got: String },
    UrlRequired,
    KindRequired,
    UnknownKind { value: String },
    DerivationRequired,
    UnknownDerivation { value: String },
    TypeRequired,
    DifferentialElementMustBeArray,
    DifferentialElementNonEmpty,
    DifferentialElementEntryMustBeObject,
    DifferentialElementMissingPath,
    SnapshotElementMustBeArray,
    SnapshotElementNonEmpty,
    SnapshotElementEntryMustBeObject,
    SnapshotElementMissingPath,
    ElementMustBeObject,
    BindingMissingValueSet,
    BindingMustBeObject,
    BindingStrengthRequired,
    UnknownBindingStrength { value: String },
    SlicingMustBeObject,
    SerializeFailed { error: String },
}

impl fmt::Display for StructureDefinitionExtractMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::JsonMustBeObject => f.write_str("StructureDefinition JSON must be an object"),
            Self::MissingResourceType => f.write_str("Missing resourceType on StructureDefinition"),
            Self::ExpectedResourceType { got } => {
                write!(f, "Expected resourceType StructureDefinition, got {got}")
            }
            Self::UrlRequired => f.write_str("StructureDefinition.url is required"),
            Self::KindRequired => f.write_str("StructureDefinition.kind is required"),
            Self::UnknownKind { value } => {
                write!(f, "Unknown StructureDefinition.kind '{value}'")
            }
            Self::DerivationRequired => f.write_str("StructureDefinition.derivation is required"),
            Self::UnknownDerivation { value } => {
                write!(f, "Unknown StructureDefinition.derivation '{value}'")
            }
            Self::TypeRequired => f.write_str("StructureDefinition.type is required"),
            Self::DifferentialElementMustBeArray => write!(
                f,
                "StructureDefinition.differential.element must be a non-empty array"
            ),
            Self::DifferentialElementNonEmpty => write!(
                f,
                "StructureDefinition.differential.element must be non-empty"
            ),
            Self::DifferentialElementEntryMustBeObject => write!(
                f,
                "StructureDefinition.differential.element entry must be an object"
            ),
            Self::DifferentialElementMissingPath => {
                f.write_str("StructureDefinition differential element missing path")
            }
            Self::SnapshotElementMustBeArray => {
                write!(
                    f,
                    "StructureDefinition.snapshot.element must be a non-empty array"
                )
            }
            Self::SnapshotElementNonEmpty => {
                write!(f, "StructureDefinition.snapshot.element must be non-empty")
            }
            Self::SnapshotElementEntryMustBeObject => {
                write!(
                    f,
                    "StructureDefinition.snapshot.element entry must be an object"
                )
            }
            Self::SnapshotElementMissingPath => {
                f.write_str("StructureDefinition snapshot element missing path")
            }
            Self::ElementMustBeObject => {
                f.write_str("StructureDefinition element must be an object")
            }
            Self::BindingMissingValueSet => write!(
                f,
                "ElementDefinition.binding is missing a value set reference"
            ),
            Self::BindingMustBeObject => f.write_str("ElementDefinition.binding must be an object"),
            Self::BindingStrengthRequired => write!(
                f,
                "ElementDefinition.binding.strength is required when binding is present"
            ),
            Self::UnknownBindingStrength { value } => {
                write!(f, "Unknown ElementDefinition.binding.strength '{value}'")
            }
            Self::SlicingMustBeObject => f.write_str("ElementDefinition.slicing must be an object"),
            Self::SerializeFailed { error } => {
                write!(
                    f,
                    "Failed to serialize StructureDefinition to JSON: {error}"
                )
            }
        }
    }
}
