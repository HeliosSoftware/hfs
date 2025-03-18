pub mod r4;
pub mod r4b;
pub mod r5;
pub mod r6;

/// Represents a FHIR specification version
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FhirVersion {
    /// FHIR Release 4
    R4,
    /// FHIR Release 4B
    R4B,
    /// FHIR Release 5
    R5,
    /// FHIR Release 6
    R6,
}

impl FhirVersion {
    /// Returns the string representation of the FHIR version
    pub fn as_str(&self) -> &'static str {
        match self {
            FhirVersion::R4 => "r4",
            FhirVersion::R4B => "r4b",
            FhirVersion::R5 => "r5",
            FhirVersion::R6 => "r6",
        }
    }
}

impl std::fmt::Display for FhirVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Default for FhirVersion {
    fn default() -> Self {
        FhirVersion::R4
    }
}

// Implement ValueEnum for FhirVersion to support clap
impl clap::ValueEnum for FhirVersion {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            FhirVersion::R4,
            FhirVersion::R4B,
            FhirVersion::R5,
            FhirVersion::R6,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.as_str()))
    }
}
