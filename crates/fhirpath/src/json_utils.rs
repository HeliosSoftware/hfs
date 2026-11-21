//! JSON conversion utilities for FHIRPath results
//!
//! This module provides common functions for converting FHIRPath evaluation results
//! to JSON representations, ensuring consistent handling of special types like Quantity.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde_json::{Value, json};

/// Converts a FHIRPath Quantity to a JSON object with proper FHIR formatting.
///
/// For valid UCUM units, includes the UCUM system and code.
/// For non-UCUM units, only includes value and unit.
///
/// # Arguments
/// * `value` - The numeric value as a Decimal
/// * `unit` - The unit string
///
/// # Returns
/// A JSON Value representing the Quantity in FHIR format with:
/// - `value`: Numeric value (as f64)
/// - `unit`: Unit string
/// - `system`: "http://unitsofmeasure.org" (only for UCUM units)
/// - `code`: Unit code (only for UCUM units)
pub fn quantity_to_json(value: &Decimal, unit: &str) -> Value {
    // Convert Decimal to f64 for proper numeric JSON representation
    let numeric_value = value
        .to_f64()
        .unwrap_or_else(|| value.to_string().parse::<f64>().unwrap_or(0.0));

    // Check if this is a valid UCUM unit
    if crate::ucum::validate_unit(unit) {
        json!({
            "value": numeric_value,
            "unit": unit,
            "system": "http://unitsofmeasure.org",
            "code": unit
        })
    } else {
        json!({
            "value": numeric_value,
            "unit": unit
        })
    }
}

/// Converts an out-of-range decimal (`1e999999999`) to a JSON number written in
/// exponent form, never expanded. Falls back to a JSON string when `serde_json`
/// cannot hold the number (without its `arbitrary_precision` feature).
pub fn out_of_range_decimal_to_json(value: &helios_fhirpath_support::OutOfRangeDecimal) -> Value {
    let text = value.to_scientific_string();
    match serde_json::from_str::<Value>(&text) {
        Ok(number @ Value::Number(_)) => number,
        _ => Value::String(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_quantity_to_json_ucum() {
        let value = Decimal::from_str("1.5865").unwrap();
        let json = quantity_to_json(&value, "cm");

        assert_eq!(json["value"], 1.5865);
        assert_eq!(json["unit"], "cm");
        assert_eq!(json["system"], "http://unitsofmeasure.org");
        assert_eq!(json["code"], "cm");

        // Verify value is numeric
        assert!(json["value"].is_f64());
    }

    #[test]
    fn test_quantity_to_json_non_ucum() {
        let value = Decimal::from_str("42.0").unwrap();
        let json = quantity_to_json(&value, "widgets");

        assert_eq!(json["value"], 42.0);
        assert_eq!(json["unit"], "widgets");

        // Should NOT have system/code for non-UCUM units
        assert!(json.get("system").is_none());
        assert!(json.get("code").is_none());
    }

    #[test]
    fn test_quantity_to_json_precision() {
        let value = Decimal::from_str("1.58650000").unwrap();
        let json = quantity_to_json(&value, "mg");

        // Should preserve significant precision
        assert_eq!(json["value"], 1.5865);
        assert_eq!(json["unit"], "mg");
        assert_eq!(json["system"], "http://unitsofmeasure.org");
    }
}
