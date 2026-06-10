//! UCUM integration module for FHIRPath
//!
//! This module provides integration with the octofhir-ucum crate for handling
//! UCUM (Unified Code for Units of Measure) operations in FHIRPath expressions.

use octofhir_ucum::fhir::{FhirError, FhirQuantity, convert_quantity};
use octofhir_ucum::{analyse, is_comparable, unit_divide, unit_multiply, validate};
use rust_decimal::Decimal;
use std::str::FromStr;

/// Validates whether a string is a valid UCUM unit expression
pub fn validate_unit(unit: &str) -> bool {
    // First check if it's a calendar unit (which is always valid)
    if is_time_unit(unit) {
        return true;
    }
    // Otherwise validate as UCUM
    validate(unit).is_ok()
}

/// Canonicalizes a quantity to its UCUM canonical unit.
///
/// Returns `(canonical_value, canonical_unit)`, or `None` if the unit is
/// unknown or cannot be converted. Used by the search index to store a
/// unit-normalized quantity so that, e.g., `1 g` and `1000 mg` compare equal.
pub fn canonicalize_quantity(value: f64, unit: &str) -> Option<(f64, String)> {
    if unit.is_empty() {
        return None;
    }
    // `get_canonical_units` reduces the unit to its dimension-based base form and
    // gives the conversion factor/offset, so commensurable units (g, mg, kg)
    // share the same canonical `unit` string and comparable canonical values.
    let canonical = octofhir_ucum::get_canonical_units(unit).ok()?;
    if canonical.unit.is_empty() {
        return None;
    }
    let canonical_value = value * canonical.factor + canonical.offset;
    Some((canonical_value, canonical.unit))
}

/// Converts a value from one UCUM unit to another
pub fn convert_units(value: Decimal, from_unit: &str, to_unit: &str) -> Result<Decimal, String> {
    // Normalize calendar units to UCUM format
    let ucum_from = calendar_to_ucum_unit(from_unit);
    let ucum_to = calendar_to_ucum_unit(to_unit);

    // Convert Decimal to f64 more safely
    let value_f64 = value
        .to_string()
        .parse::<f64>()
        .map_err(|e| format!("Failed to convert value to f64: {}", e))?;

    // Create a FhirQuantity with the source value and unit
    let source_quantity = FhirQuantity::with_ucum_code(value_f64, &ucum_from);

    // Convert to the target unit
    match convert_quantity(&source_quantity, &ucum_to) {
        Ok(converted) => {
            // Round to avoid floating point precision issues
            // Use 10 decimal places which should be sufficient for UCUM conversions
            let rounded_value = (converted.value * 1e10).round() / 1e10;

            // Convert to Decimal
            Decimal::try_from(rounded_value)
                .or_else(|_| Decimal::from_str(&format!("{:.10}", rounded_value)))
                .map_err(|e| format!("Failed to convert result to Decimal: {}", e))
        }
        Err(FhirError::UcumError(e)) => Err(format!("UCUM conversion error: {}", e)),
        Err(e) => Err(format!("Conversion error: {}", e)),
    }
}

/// Checks if two UCUM units are comparable (have the same dimension)
pub fn units_are_comparable(unit1: &str, unit2: &str) -> bool {
    // Normalize calendar units to UCUM format before comparison
    let ucum_unit1 = calendar_to_ucum_unit(unit1);
    let ucum_unit2 = calendar_to_ucum_unit(unit2);
    is_comparable(&ucum_unit1, &ucum_unit2).unwrap_or(false)
}

/// Checks if two quantities are equivalent (same value when converted to common units)
pub fn quantities_are_equivalent(
    value1: Decimal,
    unit1: &str,
    value2: Decimal,
    unit2: &str,
) -> Result<bool, String> {
    // Normalize calendar units to UCUM format
    let ucum_unit1 = calendar_to_ucum_unit(unit1);
    let ucum_unit2 = calendar_to_ucum_unit(unit2);

    // Convert Decimals to f64
    let value1_f64 = value1
        .to_string()
        .parse::<f64>()
        .map_err(|e| format!("Failed to convert value1 to f64: {}", e))?;
    let value2_f64 = value2
        .to_string()
        .parse::<f64>()
        .map_err(|e| format!("Failed to convert value2 to f64: {}", e))?;

    let _q1 = FhirQuantity::with_ucum_code(value1_f64, &ucum_unit1);
    let q2 = FhirQuantity::with_ucum_code(value2_f64, &ucum_unit2);

    // Check if units are comparable first
    if !is_comparable(&ucum_unit1, &ucum_unit2).unwrap_or(false) {
        return Ok(false);
    }

    // Convert both to a common unit for comparison
    // If units are the same, no conversion needed
    if ucum_unit1 == ucum_unit2 {
        // For equivalence, use 1% relative tolerance
        let tolerance = value1_f64.abs() * 0.01;
        let diff = (value1_f64 - value2_f64).abs();
        return Ok(diff <= tolerance);
    }

    // Convert q2 to q1's unit for comparison
    match convert_quantity(&q2, &ucum_unit1) {
        Ok(converted) => {
            // For equivalence, use 1% relative tolerance based on the larger value
            // This ensures symmetry in the comparison
            let max_value = value1_f64.abs().max(converted.value.abs());
            let tolerance = max_value * 0.01;
            let diff = (value1_f64 - converted.value).abs();
            Ok(diff <= tolerance)
        }
        Err(_) => Ok(false),
    }
}

/// Multiplies two UCUM units and returns the resulting unit expression
pub fn multiply_units(unit1: &str, unit2: &str) -> Result<String, String> {
    // Normalize calendar units to UCUM format
    let ucum_unit1 = calendar_to_ucum_unit(unit1);
    let ucum_unit2 = calendar_to_ucum_unit(unit2);

    match unit_multiply(&ucum_unit1, &ucum_unit2) {
        Ok(result) => Ok(result.expression),
        Err(e) => Err(format!("Unit multiplication error: {}", e)),
    }
}

/// Divides two UCUM units and returns the resulting unit expression
pub fn divide_units(numerator: &str, denominator: &str) -> Result<String, String> {
    // Normalize calendar units to UCUM format
    let ucum_numerator = calendar_to_ucum_unit(numerator);
    let ucum_denominator = calendar_to_ucum_unit(denominator);

    match unit_divide(&ucum_numerator, &ucum_denominator) {
        Ok(result) => Ok(result.expression),
        Err(e) => Err(format!("Unit division error: {}", e)),
    }
}

/// Gets the canonical form of a UCUM unit
#[allow(dead_code)]
pub fn get_canonical_unit(unit: &str) -> Result<String, String> {
    match analyse(unit) {
        Ok(analysis) => Ok(analysis.expression),
        Err(e) => Err(format!("Unit analysis error: {}", e)),
    }
}

/// Normalizes a unit string for display and comparison
/// This handles special cases like calendar units that may have different representations
#[allow(dead_code)]
pub fn normalize_unit_string(unit: &str) -> String {
    // Remove unnecessary braces that may be added during processing
    let cleaned = unit.trim_start_matches('{').trim_end_matches('}');

    // Map common calendar units to their canonical UCUM forms if needed
    match cleaned {
        "days" => "d".to_string(),
        "day" => "d".to_string(),
        "weeks" => "wk".to_string(),
        "week" => "wk".to_string(),
        "months" => "mo".to_string(),
        "month" => "mo".to_string(),
        "years" => "a".to_string(),
        "year" => "a".to_string(),
        "hours" => "h".to_string(),
        "hour" => "h".to_string(),
        "minutes" => "min".to_string(),
        "minute" => "min".to_string(),
        "seconds" => "s".to_string(),
        "second" => "s".to_string(),
        "milliseconds" => "ms".to_string(),
        "millisecond" => "ms".to_string(),
        _ => cleaned.to_string(),
    }
}

/// Maps calendar duration units to their UCUM equivalents
pub fn calendar_to_ucum_unit(unit: &str) -> String {
    match unit.to_lowercase().as_str() {
        "year" | "years" => "a".to_string(),
        "month" | "months" => "mo".to_string(),
        "week" | "weeks" => "wk".to_string(),
        "day" | "days" => "d".to_string(),
        "hour" | "hours" => "h".to_string(),
        "minute" | "minutes" => "min".to_string(),
        "second" | "seconds" => "s".to_string(),
        "millisecond" | "milliseconds" => "ms".to_string(),
        _ => unit.to_string(),
    }
}

/// Maps UCUM time units to calendar duration units for display
#[allow(dead_code)]
pub fn ucum_to_calendar_unit(unit: &str) -> String {
    match unit {
        "a" => "year".to_string(),
        "mo" => "month".to_string(),
        "wk" => "week".to_string(),
        "d" => "day".to_string(),
        "h" => "hour".to_string(),
        "min" => "minute".to_string(),
        "s" => "second".to_string(),
        "ms" => "millisecond".to_string(),
        _ => unit.to_string(),
    }
}

/// Checks if a unit is a time duration unit
pub fn is_time_unit(unit: &str) -> bool {
    matches!(
        unit,
        "a" | "mo"
            | "wk"
            | "d"
            | "h"
            | "min"
            | "s"
            | "ms"
            | "year"
            | "years"
            | "month"
            | "months"
            | "week"
            | "weeks"
            | "day"
            | "days"
            | "hour"
            | "hours"
            | "minute"
            | "minutes"
            | "second"
            | "seconds"
            | "millisecond"
            | "milliseconds"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_unit() {
        assert!(validate_unit("mg"));
        assert!(validate_unit("g"));
        assert!(validate_unit("kg"));
        assert!(validate_unit("m/s"));
        assert!(validate_unit("cm2"));
        assert!(!validate_unit("invalid_unit"));
    }

    #[test]
    fn test_units_are_comparable() {
        assert!(units_are_comparable("g", "mg"));
        assert!(units_are_comparable("m", "cm"));
        assert!(units_are_comparable("d", "wk"));
        assert!(!units_are_comparable("g", "m"));
        assert!(!units_are_comparable("s", "kg"));
    }

    #[test]
    fn test_multiply_units() {
        // octofhir-ucum returns "m.m" for m*m, not "m2"
        assert_eq!(multiply_units("m", "m").unwrap(), "m.m");
        // Test other multiplication
        let result = multiply_units("kg", "m/s2").unwrap();
        assert!(result == "kg.m/s2" || result == "kg.m.s-2");
    }

    #[test]
    fn test_divide_units() {
        assert_eq!(divide_units("m", "s").unwrap(), "m/s");
        // octofhir-ucum returns "m/m" for m/m, not "1"
        assert_eq!(divide_units("m", "m").unwrap(), "m/m");
    }

    #[test]
    fn test_normalize_unit_string() {
        assert_eq!(normalize_unit_string("{week}"), "wk");
        assert_eq!(normalize_unit_string("days"), "d");
        assert_eq!(normalize_unit_string("mg"), "mg");
    }
}
