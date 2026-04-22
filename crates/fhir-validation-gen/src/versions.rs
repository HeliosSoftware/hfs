/// Supported FHIR releases for validation generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FhirVersion {
    #[default]
    R4,
    R4B,
    R5,
    R6,
}

impl FhirVersion {
    /// Short module / directory name used by generated code layouts.
    #[allow(dead_code)]
    pub fn module_name(self) -> &'static str {
        match self {
            Self::R4 => "r4",
            Self::R4B => "r4b",
            Self::R5 => "r5",
            Self::R6 => "r6",
        }
    }

    /// Human-readable label for diagnostics and generated comments.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::R4 => "FHIR R4",
            Self::R4B => "FHIR R4B",
            Self::R5 => "FHIR R5",
            Self::R6 => "FHIR R6",
        }
    }

    /// Trait name expected from the runtime validation crate.
    pub fn validatable_trait_name(self) -> &'static str {
        match self {
            Self::R4 => "R4Validatable",
            Self::R4B => "R4BValidatable",
            Self::R5 => "R5Validatable",
            Self::R6 => "R6Validatable",
        }
    }
    pub fn validatable_trait_name_async(self) -> &'static str {
        match self {
            Self::R4 => "R4ValidatableAsync",
            Self::R4B => "R4BValidatableAsync",
            Self::R5 => "R5ValidatableAsync",
            Self::R6 => "R6ValidatableAsync",
        }
    }

    /// `cfg(feature = ...)` feature gate used when emitting impl blocks.
    pub fn validation_feature(self) -> &'static str {
        match self {
            Self::R4 => "R4",
            Self::R4B => "R4B",
            Self::R5 => "R5",
            Self::R6 => "R6",
        }
    }

    /// Module path segment for `helios-fhir`, e.g. `helios_fhir::r4`.
    #[allow(dead_code)]
    pub fn helios_module_name(self) -> &'static str {
        self.module_name()
    }

    /// Returns all supported versions in a stable order.
    #[allow(dead_code)]
    pub const fn all() -> [FhirVersion; 4] {
        [
            FhirVersion::R4,
            FhirVersion::R4B,
            FhirVersion::R5,
            FhirVersion::R6,
        ]
    }
}

impl std::fmt::Display for FhirVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

impl std::str::FromStr for FhirVersion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "r4" | "4" | "fhir-r4" | "fhir_r4" => Ok(Self::R4),
            "r4b" | "4b" | "fhir-r4b" | "fhir_r4b" => Ok(Self::R4B),
            "r5" | "5" | "fhir-r5" | "fhir_r5" => Ok(Self::R5),
            "r6" | "6" | "fhir-r6" | "fhir_r6" => Ok(Self::R6),
            other => Err(format!("Unsupported FHIR version: {other}")),
        }
    }
}
