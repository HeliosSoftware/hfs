#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminologyValidationError {
    /// Missing required terminology context or value.
    InvalidInput(String),
    /// A primitive code was supplied without a system and local inference was not possible.
    MissingSystem(String),
    /// A code was supplied for a known CodeSystem, but the code itself is unknown.
    UnknownCode { system: String, code: String },
    /// The code is known, but not a member of the bound ValueSet.
    NotInValueSet {
        valueset_url: String,
        system: Option<String>,
        code: String,
    },
    /// The provided display does not match the canonical display for the code.
    WrongDisplay {
        system: String,
        code: String,
        expected: String,
        provided: String,
    },
    /// Local rules insufficient; remote terminology validation is required.
    RemoteValidationRequired(String),
}

impl std::fmt::Display for TerminologyValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "{}", msg),
            Self::MissingSystem(msg) => write!(f, "{}", msg),
            Self::UnknownCode { system, code } => {
                write!(f, "Unknown code '{}' in CodeSystem '{}'", code, system)
            }
            Self::NotInValueSet {
                valueset_url,
                system,
                code,
            } => {
                if let Some(system) = system {
                    write!(
                        f,
                        "Code '{}#{}' is not in ValueSet '{}'",
                        system, code, valueset_url
                    )
                } else {
                    write!(f, "Code '{}' is not in ValueSet '{}'", code, valueset_url)
                }
            }
            Self::WrongDisplay {
                system,
                code,
                expected,
                provided,
            } => {
                write!(
                    f,
                    "Wrong display '{}' for {}#{}. Expected '{}'",
                    provided, system, code, expected
                )
            }
            Self::RemoteValidationRequired(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for TerminologyValidationError {}
