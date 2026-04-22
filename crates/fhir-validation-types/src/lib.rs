/// Severity of a validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Fatal,
    Error,
    Warning,
    Information,
}
/// Binding strength defined by FHIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingStrength {
    Required,
    Extensible,
    Preferred,
    Example,
}

/// [`StructureDefinition.kind`](https://hl7.org/fhir/valueset-structure-definition-kind.html).
///
/// Codes match `http://hl7.org/fhir/structure-definition-kind` (see generated
/// `StructureDefinitionKind` in `helios-fhir` R5 terminology).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StructureDefinitionKind {
    #[default]
    Resource,
    PrimitiveType,
    ComplexType,
    Logical,
}

impl StructureDefinitionKind {
    pub fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "primitive-type" => Self::PrimitiveType,
            "complex-type" => Self::ComplexType,
            "resource" => Self::Resource,
            "logical" => Self::Logical,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PrimitiveType => "primitive-type",
            Self::ComplexType => "complex-type",
            Self::Resource => "resource",
            Self::Logical => "logical",
        }
    }
}

/// [`StructureDefinition.derivation`](https://hl7.org/fhir/valueset-type-derivation-rule.html).
///
/// Codes match `http://hl7.org/fhir/type-derivation-rule` (see generated
/// `TypeDerivationRule` in `helios-fhir` R5 terminology).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TypeDerivationRule {
    #[default]
    Constraint,
    Specialization,
}

impl TypeDerivationRule {
    pub fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "specialization" => Self::Specialization,
            "constraint" => Self::Constraint,
            _ => return None,
        })
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Specialization => "specialization",
            Self::Constraint => "constraint",
        }
    }
}

/// Which concrete type a ValueSet [`ElementDefinition.binding`](https://hl7.org/fhir/elementdefinition-definitions.html#ElementDefinition.binding)
/// applies to at runtime.
///
/// FHIR allows bindings only on types that carry terminology: primitive `code`,
/// `string`, `uri`, and datatypes `Coding`, `CodeableConcept`, `CodeableReference`,
/// and `Quantity` (see the binding rules on `ElementDefinition`).
///
/// [`Choice`](BindingTargetKind::Choice) is used when the element declares multiple
/// bindable types (for example a choice `[x]`); the validator picks the handler
/// from the instance JSON shape. [`Unsupported`](BindingTargetKind::Unsupported)
/// means the declared types are not bindable (or cannot be classified).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingTargetKind {
    Code,
    Coding,
    CodeableConcept,
    String,
    Uri,
    Choice,
    CodeableReference,
    Quantity,
    Unsupported,
}

/// Normalize a `code` from [`ElementDefinition.type`](https://hl7.org/fhir/elementdefinition-definitions.html#ElementDefinition.type).
///
/// FHIR may use `http://hl7.org/fhirpath/System.*` URLs for logical types; this
/// maps them to the short names used elsewhere in validation (same rules as
/// `fhir-validation-gen` extraction).
pub fn normalize_fhir_element_type_code(code: &str) -> String {
    match code {
        "http://hl7.org/fhirpath/System.Boolean" => "boolean".to_string(),
        "http://hl7.org/fhirpath/System.String" => "string".to_string(),
        "http://hl7.org/fhirpath/System.Integer" => "integer".to_string(),
        "http://hl7.org/fhirpath/System.Long" => "integer64".to_string(),
        "http://hl7.org/fhirpath/System.Decimal" => "decimal".to_string(),
        "http://hl7.org/fhirpath/System.Date" => "date".to_string(),
        "http://hl7.org/fhirpath/System.DateTime" => "dateTime".to_string(),
        "http://hl7.org/fhirpath/System.Time" => "time".to_string(),
        other => other.to_string(),
    }
}

/// Classify which bindable runtime shape a terminology binding applies to, from
/// declared element type codes.
///
/// This matches the logic in `fhir-validation-gen` (`binding_target_kind`):
/// single concrete types map to the corresponding [`BindingTargetKind`];
/// multiple types (e.g. choice `[x]`) yield [`BindingTargetKind::Choice`] when
/// any variant is bindable, otherwise [`BindingTargetKind::Unsupported`].
/// An empty slice yields [`BindingTargetKind::Unsupported`].
pub fn binding_target_kind_from_element_type_codes(type_codes: &[String]) -> BindingTargetKind {
    if type_codes.len() != 1 {
        let has_bindable_choice_variant = type_codes.iter().any(|code| {
            matches!(
                code.as_str(),
                "code"
                    | "string"
                    | "uri"
                    | "Coding"
                    | "CodeableConcept"
                    | "CodeableReference"
                    | "Quantity"
            )
        });

        return if has_bindable_choice_variant {
            BindingTargetKind::Choice
        } else {
            BindingTargetKind::Unsupported
        };
    }

    match type_codes[0].as_str() {
        "code" => BindingTargetKind::Code,
        "string" => BindingTargetKind::String,
        "uri" => BindingTargetKind::Uri,
        "Coding" => BindingTargetKind::Coding,
        "CodeableConcept" => BindingTargetKind::CodeableConcept,
        "CodeableReference" => BindingTargetKind::CodeableReference,
        "Quantity" => BindingTargetKind::Quantity,
        _ => BindingTargetKind::Unsupported,
    }
}

/// Map a single FHIR element `type.code` to [`BindingTargetKind`] when that type participates in
/// terminology bindings (`code`, `string`, `uri`, `Coding`, `CodeableConcept`, `CodeableReference`,
/// `Quantity`).
pub fn binding_target_kind_for_element_type_code(code: &str) -> Option<BindingTargetKind> {
    match code {
        "code" => Some(BindingTargetKind::Code),
        "string" => Some(BindingTargetKind::String),
        "uri" => Some(BindingTargetKind::Uri),
        "Coding" => Some(BindingTargetKind::Coding),
        "CodeableConcept" => Some(BindingTargetKind::CodeableConcept),
        "CodeableReference" => Some(BindingTargetKind::CodeableReference),
        "Quantity" => Some(BindingTargetKind::Quantity),
        _ => None,
    }
}

/// Subset of element `type` codes that are bindable for ValueSet validation.
pub fn bindable_element_type_codes(type_codes: &[String]) -> Vec<String> {
    type_codes
        .iter()
        .filter(|c| binding_target_kind_for_element_type_code(c.as_str()).is_some())
        .cloned()
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingDef {
    pub path: String,
    pub strength: crate::BindingStrength,
    pub value_set: String,
    pub binding_name: Option<String>,
    pub target_kind: BindingTargetKind,
    /// When [`BindingTargetKind::Choice`], declared bindable `ElementDefinition.type` codes for
    /// this element (same filter as [`bindable_element_type_codes`]). `None` when not a choice
    /// binding.
    pub choice_type_codes: Option<Vec<String>>,
}
/// One generated FHIR invariant attached to a resource or element.
///
/// Examples:
/// - `ele-1`
/// - `ext-1`
/// - `pat-1`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvariantDef {
    /// Invariant key, e.g. `pat-1`
    pub key: String,

    /// Severity declared by the specification/profile.
    pub severity: Severity,

    /// Declared logical path, e.g. `Patient.contact`
    pub path: String,

    /// FHIRPath expression to evaluate.
    pub expression: String,

    /// Human-readable message.
    pub human: String,
}

#[cfg(test)]
mod binding_target_kind_tests {
    use super::*;

    #[test]
    fn empty_type_codes_are_unsupported() {
        let empty: Vec<String> = vec![];
        assert_eq!(
            binding_target_kind_from_element_type_codes(&empty),
            BindingTargetKind::Unsupported
        );
    }

    #[test]
    fn single_codeable_concept_maps() {
        assert_eq!(
            binding_target_kind_from_element_type_codes(&["CodeableConcept".to_string()]),
            BindingTargetKind::CodeableConcept
        );
    }

    #[test]
    fn choice_with_bindable_variants_is_choice() {
        assert_eq!(
            binding_target_kind_from_element_type_codes(&[
                "string".to_string(),
                "CodeableConcept".to_string()
            ]),
            BindingTargetKind::Choice
        );
    }

    #[test]
    fn bindable_element_type_codes_filters_non_terminology_types() {
        let codes = vec![
            "boolean".to_string(),
            "string".to_string(),
            "Quantity".to_string(),
        ];
        assert_eq!(
            bindable_element_type_codes(&codes),
            vec!["string".to_string(), "Quantity".to_string()]
        );
    }

    #[test]
    fn binding_target_kind_for_element_type_code_maps_bindable_only() {
        assert_eq!(
            binding_target_kind_for_element_type_code("CodeableConcept"),
            Some(BindingTargetKind::CodeableConcept)
        );
        assert_eq!(binding_target_kind_for_element_type_code("boolean"), None);
    }
}
