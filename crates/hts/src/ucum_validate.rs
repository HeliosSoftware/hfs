//! UCUM CodeSystem validation helpers for `http://unitsofmeasure.org`.
//!
//! The UCUM `ucum-essence.xml` import stores atomic prefixes, base units, and
//! named units — not every composed expression (`mg` = milli + gram). tx.fhir.org
//! validates UCUM structurally; HTS mirrors that when a direct concept lookup
//! misses but the code is a well-formed UCUM unit expression.

use octofhir_ucum::validate as validate_ucum_expression;

use crate::types::ValidateCodeResponse;

pub const UCUM_URL: &str = "http://unitsofmeasure.org";

/// True when `url` is the UCUM CodeSystem canonical (optional `|version` suffix).
pub fn is_ucum_url(url: &str) -> bool {
    url.split_once('|').map(|(u, _)| u).unwrap_or(url) == UCUM_URL
}

/// Structural UCUM syntax check (prefix + unit composition, etc.).
pub fn is_valid_ucum_code(code: &str) -> bool {
    !code.is_empty() && validate_ucum_expression(code).is_ok()
}

/// Success response for a structurally valid composed UCUM code.
pub fn composed_code_success(code: &str, cs_version: Option<String>) -> ValidateCodeResponse {
    ValidateCodeResponse {
        result: true,
        message: None,
        display: Some(code.to_string()),
        system: Some(UCUM_URL.into()),
        cs_version,
        inactive: None,
        issues: vec![],
        caused_by_unknown_system: None,
        concept_status: None,
        normalized_code: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_composed_mg() {
        assert!(is_valid_ucum_code("mg"));
    }

    #[test]
    fn accepts_atomic_g_from_essence() {
        assert!(is_valid_ucum_code("g"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(!is_valid_ucum_code("not-a-unit"));
    }
}
