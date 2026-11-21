//! Evaluation rules for decimals outside `rust_decimal`'s range (#2716).
//!
//! A FHIR decimal such as `1e999999999` arrives as
//! [`EvaluationResult::OutOfRangeDecimal`], alone or as the `value` of a FHIR
//! Quantity. Written out in full it is about a billion digits, so it is never
//! expanded. These rules follow the upstream `testLargeExponents` conformance group:
//!
//! - `=`, `!=`, `~`, `!~`, `<`, `<=`, `>`, `>=` against numbers, and against
//!   quantities in the same unit, compare sign, order of magnitude, then digits.
//! - `~` rounds both sides to the precision of the less precise one, so
//!   `1e-999999999 ~ 0` is true.
//! - A quantity with such a value is not converted to canonical units: against a
//!   different unit, `=` and ordering are empty and `~` is false.
//! - Arithmetic, and numeric functions such as `round()` or `lowBoundary()`, give
//!   empty.

use helios_fhirpath_support::{EvaluationResult, OutOfRangeDecimal, TypeInfoResult};
use std::cmp::Ordering;
use std::collections::HashMap;

/// FHIR types that are, or profile, Quantity and so convert to `System.Quantity`.
pub(crate) const FHIR_QUANTITY_TYPES: &[&str] = &[
    "Quantity",
    "Age",
    "Count",
    "Distance",
    "Duration",
    "SimpleQuantity",
    "MoneyQuantity",
];

/// True for an object typed as a FHIR Quantity (or a profile of it).
pub(crate) fn is_fhir_quantity_type(type_info: Option<&TypeInfoResult>) -> bool {
    type_info.is_some_and(|t| FHIR_QUANTITY_TYPES.contains(&t.name.as_str()))
}

/// Unwraps a single-item collection, the way operators treat singleton operands.
fn singleton(value: &EvaluationResult) -> &EvaluationResult {
    match value {
        EvaluationResult::Collection { items, .. } if items.len() == 1 => &items[0],
        other => other,
    }
}

/// The out-of-range decimal a value carries: the value itself, or the `value` of
/// a FHIR Quantity object.
fn carried(value: &EvaluationResult) -> Option<&OutOfRangeDecimal> {
    match singleton(value) {
        EvaluationResult::OutOfRangeDecimal(d, _, _) => Some(d),
        EvaluationResult::Object { map, type_info }
            if is_fhir_quantity_type(type_info.as_ref()) =>
        {
            match map.get("value") {
                Some(EvaluationResult::OutOfRangeDecimal(d, _, _)) => Some(d),
                _ => None,
            }
        }
        _ => None,
    }
}

/// True when the value is, or is a FHIR Quantity holding, an out-of-range decimal.
pub(crate) fn is_out_of_range(value: &EvaluationResult) -> bool {
    carried(value).is_some()
}

/// The unit of a FHIR Quantity object: the UCUM `code`, else the display `unit`,
/// else the default unit `'1'`.
fn object_unit(map: &HashMap<String, EvaluationResult>) -> String {
    match map.get("code").or_else(|| map.get("unit")) {
        Some(EvaluationResult::String(u, _, _)) => u.clone(),
        _ => "1".to_string(),
    }
}

/// A numeric operand in a form that can be compared with an out-of-range one.
/// `unit` is `None` for a plain number and for a quantity in the default unit `'1'`.
struct Operand {
    value: OutOfRangeDecimal,
    unit: Option<String>,
}

fn unit_or_none(unit: String) -> Option<String> {
    if unit == "1" { None } else { Some(unit) }
}

fn operand(value: &EvaluationResult) -> Option<Operand> {
    let number = |value| Operand { value, unit: None };
    match singleton(value) {
        EvaluationResult::OutOfRangeDecimal(d, _, _) => Some(number(d.clone())),
        EvaluationResult::Decimal(d, _, _) => Some(number(OutOfRangeDecimal::from_decimal(*d))),
        EvaluationResult::Integer(i, _, _) | EvaluationResult::Integer64(i, _, _) => {
            Some(number(OutOfRangeDecimal::from_integer(*i)))
        }
        EvaluationResult::Quantity(d, unit, _, _) => Some(Operand {
            value: OutOfRangeDecimal::from_decimal(*d),
            unit: unit_or_none(unit.clone()),
        }),
        EvaluationResult::Object { map, type_info }
            if is_fhir_quantity_type(type_info.as_ref()) =>
        {
            let value = match map.get("value")? {
                EvaluationResult::OutOfRangeDecimal(d, _, _) => d.clone(),
                EvaluationResult::Decimal(d, _, _) => OutOfRangeDecimal::from_decimal(*d),
                EvaluationResult::Integer(i, _, _) => OutOfRangeDecimal::from_integer(*i),
                _ => return None,
            };
            Some(Operand {
                value,
                unit: unit_or_none(object_unit(map)),
            })
        }
        _ => None,
    }
}

/// Compares two operands when at least one carries an out-of-range decimal.
///
/// Returns `None` when neither does, or when the other operand is not a number or
/// quantity; the caller's ordinary rules then apply. `op` is one of `=`, `!=`, `~`,
/// `!~`, `<`, `<=`, `>`, `>=`.
pub(crate) fn compare(
    left: &EvaluationResult,
    op: &str,
    right: &EvaluationResult,
) -> Option<EvaluationResult> {
    if !is_out_of_range(left) && !is_out_of_range(right) {
        return None;
    }
    let (l, r) = (operand(left)?, operand(right)?);
    let same_unit = match (&l.unit, &r.unit) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            crate::evaluator::normalize_unit_for_equality(a)
                == crate::evaluator::normalize_unit_for_equality(b)
        }
        _ => false,
    };
    let boolean = EvaluationResult::boolean;
    Some(match op {
        // Never converted to canonical units, so another unit is incomparable:
        // equality and ordering are empty, equivalence is false.
        "=" | "!=" | "<" | "<=" | ">" | ">=" if !same_unit => EvaluationResult::Empty,
        "~" if !same_unit => boolean(false),
        "!~" if !same_unit => boolean(true),
        "=" => boolean(l.value.numeric_cmp(&r.value) == Ordering::Equal),
        "!=" => boolean(l.value.numeric_cmp(&r.value) != Ordering::Equal),
        "~" => boolean(l.value.equivalent(&r.value)),
        "!~" => boolean(!l.value.equivalent(&r.value)),
        "<" => boolean(l.value.numeric_cmp(&r.value).is_lt()),
        "<=" => boolean(l.value.numeric_cmp(&r.value).is_le()),
        ">" => boolean(l.value.numeric_cmp(&r.value).is_gt()),
        ">=" => boolean(l.value.numeric_cmp(&r.value).is_ge()),
        _ => return None,
    })
}

/// Functions that would have to expand an out-of-range decimal. On one, as the
/// input or an argument, they give empty.
const NUMERIC_FUNCTIONS: &[&str] = &[
    "abs",
    "ceiling",
    "floor",
    "truncate",
    "round",
    "sqrt",
    "exp",
    "ln",
    "log",
    "power",
    "precision",
    "lowBoundary",
    "highBoundary",
    "toInteger",
    "toLong",
    "toQuantity",
];

/// The result of function `name` when its input or an argument carries an
/// out-of-range decimal, or `None` to evaluate the function as usual.
///
/// Numeric functions give empty. An out-of-range decimal is a Decimal, so
/// `toDecimal()` returns it and `convertsToDecimal()` is true; it is not an
/// Integer, Long or Quantity, so those `convertsTo*` functions are false.
pub(crate) fn call_function_on_out_of_range(
    name: &str,
    input: &EvaluationResult,
    args: &[EvaluationResult],
) -> Option<EvaluationResult> {
    let input_out_of_range = carried(input).is_some();
    if NUMERIC_FUNCTIONS.contains(&name) && (input_out_of_range || args.iter().any(is_out_of_range))
    {
        return Some(EvaluationResult::Empty);
    }
    // The remaining cases only apply to a bare decimal input, not a Quantity.
    let EvaluationResult::OutOfRangeDecimal(..) = singleton(input) else {
        return None;
    };
    match name {
        "toDecimal" => Some(singleton(input).clone()),
        "convertsToDecimal" => Some(EvaluationResult::boolean(true)),
        "convertsToInteger" | "convertsToLong" | "convertsToQuantity" => {
            Some(EvaluationResult::boolean(false))
        }
        _ => None,
    }
}

/// `toString()` of a FHIR Quantity whose value is out of range:
/// `1E+999999999 'kg'`. `None` for anything else.
pub(crate) fn quantity_to_string(value: &EvaluationResult) -> Option<String> {
    let EvaluationResult::Object { map, type_info } = singleton(value) else {
        return None;
    };
    if !is_fhir_quantity_type(type_info.as_ref()) {
        return None;
    }
    let Some(EvaluationResult::OutOfRangeDecimal(d, _, _)) = map.get("value") else {
        return None;
    };
    Some(format!(
        "{} {}",
        d.to_scientific_string(),
        helios_fhirpath_support::format_unit_for_display(&object_unit(map))
    ))
}

/// Unary minus on an out-of-range decimal, or on a FHIR Quantity holding one.
/// `None` when the value carries no out-of-range decimal.
pub(crate) fn negate(value: &EvaluationResult) -> Option<EvaluationResult> {
    match singleton(value) {
        EvaluationResult::OutOfRangeDecimal(d, _, _) => Some(EvaluationResult::OutOfRangeDecimal(
            d.negated(),
            Some(TypeInfoResult::new("System", "Decimal")),
            None,
        )),
        EvaluationResult::Object { map, type_info } if is_out_of_range(value) => {
            let mut map = map.clone();
            if let Some(EvaluationResult::OutOfRangeDecimal(d, t, m)) = map.get("value").cloned() {
                map.insert(
                    "value".to_string(),
                    EvaluationResult::OutOfRangeDecimal(d.negated(), t, m),
                );
            }
            Some(EvaluationResult::Object {
                map,
                type_info: type_info.clone(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn huge() -> EvaluationResult {
        EvaluationResult::fhir_out_of_range_decimal(
            OutOfRangeDecimal::parse("1e999999999").unwrap(),
        )
    }

    fn tiny() -> EvaluationResult {
        EvaluationResult::fhir_out_of_range_decimal(
            OutOfRangeDecimal::parse("1e-999999999").unwrap(),
        )
    }

    fn fhir_quantity(value: EvaluationResult, code: &str) -> EvaluationResult {
        let mut map = HashMap::new();
        map.insert("value".to_string(), value);
        map.insert(
            "unit".to_string(),
            EvaluationResult::string(code.to_string()),
        );
        map.insert(
            "code".to_string(),
            EvaluationResult::string(code.to_string()),
        );
        EvaluationResult::typed_object(map, "FHIR", "Quantity")
    }

    fn qty(value: i64, unit: &str) -> EvaluationResult {
        EvaluationResult::quantity(Decimal::from(value), unit.to_string())
    }

    #[test]
    fn decimals_compare_without_expansion() {
        let one = EvaluationResult::integer(1);
        let zero = EvaluationResult::decimal(Decimal::ZERO);
        assert_eq!(
            compare(&huge(), "=", &one),
            Some(EvaluationResult::boolean(false))
        );
        assert_eq!(
            compare(&huge(), ">", &one),
            Some(EvaluationResult::boolean(true))
        );
        assert_eq!(
            compare(&one, "<", &huge()),
            Some(EvaluationResult::boolean(true))
        );
        assert_eq!(
            compare(&huge(), "~", &one),
            Some(EvaluationResult::boolean(false))
        );
        assert_eq!(
            compare(&tiny(), "~", &zero),
            Some(EvaluationResult::boolean(true))
        );
        assert_eq!(
            compare(&tiny(), "!~", &zero),
            Some(EvaluationResult::boolean(false))
        );
        assert_eq!(
            compare(&tiny(), "=", &zero),
            Some(EvaluationResult::boolean(false))
        );
        assert_eq!(
            compare(&huge(), "=", &huge()),
            Some(EvaluationResult::boolean(true))
        );
    }

    #[test]
    fn in_range_operands_are_left_to_the_caller() {
        let one = EvaluationResult::integer(1);
        assert_eq!(compare(&one, "=", &one), None);
        // A string operand is not a number: the caller's type rules apply.
        assert_eq!(
            compare(&huge(), "=", &EvaluationResult::string("a".into())),
            None
        );
    }

    #[test]
    fn quantities_compare_only_in_the_same_unit() {
        let huge_kg = fhir_quantity(huge(), "kg");
        assert_eq!(
            compare(&huge_kg, "=", &qty(1, "kg")),
            Some(EvaluationResult::boolean(false))
        );
        assert_eq!(
            compare(&huge_kg, ">", &qty(1, "kg")),
            Some(EvaluationResult::boolean(true))
        );
        assert_eq!(
            compare(&huge_kg, "~", &qty(1, "kg")),
            Some(EvaluationResult::boolean(false))
        );
        assert_eq!(
            compare(&huge_kg, "=", &qty(1, "g")),
            Some(EvaluationResult::Empty)
        );
        assert_eq!(
            compare(&huge_kg, "<", &qty(1, "g")),
            Some(EvaluationResult::Empty)
        );
        assert_eq!(
            compare(&huge_kg, "~", &qty(1, "g")),
            Some(EvaluationResult::boolean(false))
        );
        assert_eq!(
            compare(&huge_kg, "!~", &qty(1, "g")),
            Some(EvaluationResult::boolean(true))
        );
        // A plain number is not a 'kg' quantity.
        assert_eq!(
            compare(&huge_kg, "=", &EvaluationResult::integer(1)),
            Some(EvaluationResult::Empty)
        );
        let tiny_kg = fhir_quantity(tiny(), "kg");
        assert_eq!(
            compare(&tiny_kg, "~", &qty(0, "kg")),
            Some(EvaluationResult::boolean(true))
        );
    }

    #[test]
    fn quantity_to_string_keeps_the_exponent() {
        assert_eq!(
            quantity_to_string(&fhir_quantity(huge(), "kg")).as_deref(),
            Some("1E+999999999 'kg'")
        );
        assert_eq!(
            quantity_to_string(&fhir_quantity(tiny(), "kg")).as_deref(),
            Some("1E-999999999 'kg'")
        );
        assert_eq!(quantity_to_string(&qty(1, "kg")), None);
    }

    #[test]
    fn negation_flips_the_sign() {
        let negated = negate(&huge()).unwrap();
        assert_eq!(negated.to_string_value(), "-1E+999999999");
        let negated_qty = negate(&fhir_quantity(huge(), "kg")).unwrap();
        assert_eq!(
            quantity_to_string(&negated_qty).as_deref(),
            Some("-1E+999999999 'kg'")
        );
        assert_eq!(negate(&EvaluationResult::integer(1)), None);
    }
}
