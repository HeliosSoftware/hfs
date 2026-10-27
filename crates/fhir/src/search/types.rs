//! FHIR search parameter type enum.
//!
//! Lifted from `helios-persistence::types::search_params` so the registry
//! can live in `helios-fhir` (foundational). Persistence's runtime query
//! types (`SearchValue`, `SearchPrefix`, `SearchModifier`, `SearchQuery`,
//! …) stay in persistence — they are query-execution concerns.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// FHIR search parameter types.
///
/// See: https://build.fhir.org/search.html#ptypes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchParamType {
    #[default]
    /// A simple string, like a name or description.
    String,
    /// A search against a URI.
    Uri,
    /// A search for a number.
    Number,
    /// A search for a date, dateTime, or period.
    Date,
    /// A quantity, with a number and units.
    Quantity,
    /// A code from a code system or value set.
    Token,
    /// A reference to another resource.
    Reference,
    /// A composite search parameter that combines others.
    Composite,
    /// Special search parameters (_id, _lastUpdated, etc.).
    Special,
}

impl fmt::Display for SearchParamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchParamType::String => write!(f, "string"),
            SearchParamType::Uri => write!(f, "uri"),
            SearchParamType::Number => write!(f, "number"),
            SearchParamType::Date => write!(f, "date"),
            SearchParamType::Quantity => write!(f, "quantity"),
            SearchParamType::Token => write!(f, "token"),
            SearchParamType::Reference => write!(f, "reference"),
            SearchParamType::Composite => write!(f, "composite"),
            SearchParamType::Special => write!(f, "special"),
        }
    }
}

impl FromStr for SearchParamType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "string" => Ok(SearchParamType::String),
            "uri" => Ok(SearchParamType::Uri),
            "number" => Ok(SearchParamType::Number),
            "date" => Ok(SearchParamType::Date),
            "quantity" => Ok(SearchParamType::Quantity),
            "token" => Ok(SearchParamType::Token),
            "reference" => Ok(SearchParamType::Reference),
            "composite" => Ok(SearchParamType::Composite),
            "special" => Ok(SearchParamType::Special),
            _ => Err(format!("unknown search parameter type: {}", s)),
        }
    }
}
