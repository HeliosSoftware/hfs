//! Implicit-precision ranges for ordered (number/quantity) search values.
//!
//! FHIR treats a decimal search value as a range determined by its significant
//! figures: `100` ⇒ `[99.5, 100.5)`, `100.0` ⇒ `[99.95, 100.05)`, `1e2` ⇒
//! `[50, 150)` (R5's corrected example; R4's text says `[95, 105)`, which
//! contradicts its own "1 significant figure"). That range only bounds the `eq`/`ne` prefixes; every other comparator prefix compares
//! against the exact search value `v`, per the FHIR number search spec
//! (<https://hl7.org/fhir/R4/search.html#number>): *"When a comparison prefix
//! in the set gt, lt, ge, le, sa & eb is provided, the implicit precision of
//! the number is ignored, and they are treated as if they have arbitrarily
//! high precision."*
//!
//! | prefix | match (target value `x`) |
//! |--------|---------------------------|
//! | `eq`   | `lo ≤ x < hi`             |
//! | `ne`   | `x < lo OR x ≥ hi`         |
//! | `gt` / `sa` | `x > v`               |
//! | `lt` / `eb` | `x < v`               |
//! | `ge`   | `x ≥ v`                   |
//! | `le`   | `x ≤ v`                   |

/// Returns the implicit precision (ULP of the least significant digit) of a
/// decimal value from its string form: `"100"` → 1.0, `"100.0"` → 0.1.
///
/// An exponent shifts the last written digit, so the precision is
/// `10^(exponent − fraction_digits)`, counting the fraction digits of the
/// mantissa only: `"1e2"` → 100.0, `"1.00e2"` → 1.0 (three significant
/// figures, the same search as `"100"`), `"1.5E-2"` → 0.001.
///
/// `num_str` is the bare number: callers split off the search prefix and any
/// `|system|code` suffix first. The result is always finite and positive,
/// whatever the text; malformed or out-of-range values are rejected upstream.
pub fn implicit_precision(num_str: &str) -> f64 {
    let (mantissa, exponent) = match num_str.find(['e', 'E']) {
        Some(e) => (&num_str[..e], parse_exponent(&num_str[e + 1..])),
        None => (num_str, 0),
    };
    let decimals = match mantissa.find('.') {
        Some(dot) => mantissa.len() - dot - 1,
        None => 0,
    };
    let decimals = i32::try_from(decimals).unwrap_or(MAX_POWER);
    let power = exponent.saturating_sub(decimals);
    // Beyond f64's range `powi` gives 0 or infinity, which would make the
    // range empty or a bound NaN.
    10f64
        .powi(power.clamp(-MAX_POWER, MAX_POWER))
        .clamp(f64::MIN_POSITIVE, f64::MAX)
}

/// Larger than any power of ten an `f64` can hold, in either direction.
const MAX_POWER: i32 = 400;

/// Parses the text after the `e` of a number. An exponent too large for `i32`
/// saturates; anything that is not an integer counts as no exponent.
fn parse_exponent(text: &str) -> i32 {
    if let Ok(exponent) = text.parse::<i32>() {
        return exponent;
    }
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    if negative { -MAX_POWER } else { MAX_POWER }
}

/// Returns the half-open implicit-precision range `[lo, hi)` for `value`, whose
/// textual form is `num_str` (used to derive the precision).
///
/// Neither bound is ever NaN: a NaN `value` yields the empty range `[∞, ∞)`,
/// since nothing equals it.
pub fn implicit_range(value: f64, num_str: &str) -> (f64, f64) {
    if value.is_nan() {
        return (f64::INFINITY, f64::INFINITY);
    }
    let half = implicit_precision(num_str) / 2.0;
    (value - half, value + half)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_from_significant_figures() {
        assert_eq!(implicit_precision("100"), 1.0);
        assert_eq!(implicit_precision("100.0"), 0.1);
        assert_eq!(implicit_precision("100.00"), 0.01);
    }

    #[test]
    fn range_brackets_value() {
        let (lo, hi) = implicit_range(100.0, "100");
        assert_eq!(lo, 99.5);
        assert_eq!(hi, 100.5);
    }

    /// The precision is one unit of the last digit written: the power of ten
    /// `exponent - fraction_digits`. Compared bit-for-bit against `powi`, which
    /// is what the plain-decimal path has always returned.
    #[test]
    fn precision_table() {
        let cases: &[(&str, i32)] = &[
            // Plain integers and decimals.
            ("100", 0),
            ("0", 0),
            ("7", 0),
            ("-7", 0),
            ("+7", 0),
            ("100.0", -1),
            ("100.00", -2),
            ("0.0", -1),
            ("2.0", -1),
            ("0.8", -1),
            ("-5.4", -1),
            ("-0.25", -2),
            ("0.001", -3),
            (".5", -1),
            ("5.", 0),
            // Exponent forms (#1337): the exponent shifts the last digit.
            ("1e2", 2),
            ("1.0e2", 1),
            ("1.00e2", 0),
            ("1e3", 3),
            ("1E3", 3),
            ("1E+3", 3),
            ("+1e3", 3),
            ("-1e3", 3),
            ("1.5E-2", -3),
            ("1e-1", -1),
            ("8e-1", -1),
            ("1e0", 0),
            ("1.5e0", -1),
            ("12.345e1", -2),
            ("-2.50e-3", -5),
            ("0e0", 0),
            ("1e05", 5),
        ];
        for (text, power) in cases {
            assert_eq!(
                implicit_precision(text),
                10f64.powi(*power),
                "implicit_precision({text:?})"
            );
        }
    }

    /// `100` and `1.00e2` have three significant figures each, so they are the
    /// same search; `1e2` has one.
    #[test]
    fn trailing_zeros_and_exponent_agree() {
        assert_eq!(implicit_range(100.0, "100"), (99.5, 100.5));
        assert_eq!(implicit_range(100.0, "1.00e2"), (99.5, 100.5));
        assert_eq!(implicit_range(100.0, "1.0e2"), (95.0, 105.0));
        assert_eq!(implicit_range(100.0, "1e2"), (50.0, 150.0));
    }

    #[test]
    fn range_table() {
        let cases: &[(&str, f64, f64)] = &[
            ("100", 99.5, 100.5),
            ("100.00", 99.995, 100.005),
            ("2.0", 1.95, 2.05),
            ("0", -0.5, 0.5),
            ("0.0", -0.05, 0.05),
            ("-5.4", -5.45, -5.35),
            ("1e3", 500.0, 1500.0),
            ("+1e3", 500.0, 1500.0),
            ("1E+3", 500.0, 1500.0),
            ("-1e3", -1500.0, -500.0),
            ("1.5E-2", 0.0145, 0.0155),
            ("1e-1", 0.05, 0.15),
            ("8e-1", 0.75, 0.85),
        ];
        for (text, lo, hi) in cases {
            let value: f64 = text.parse().unwrap();
            let (got_lo, got_hi) = implicit_range(value, text);
            assert!(
                (got_lo - lo).abs() < 1e-12 && (got_hi - hi).abs() < 1e-12,
                "implicit_range({text:?}) = [{got_lo}, {got_hi}), expected [{lo}, {hi})"
            );
        }
    }

    /// Out-of-range and malformed text is rejected before it gets here, but
    /// whatever arrives must not panic or produce a NaN bound.
    #[test]
    fn extreme_and_malformed_input_never_panics_or_yields_nan() {
        let texts = [
            "1e400",
            "-1e400",
            "1e-400",
            "1e2147483647",
            "1e-2147483648",
            "1e99999999999999999999",
            "-1.5e-99999999999999999999",
            "1e",
            "1e+",
            "1e-",
            "e5",
            "1ee5",
            "1e5e5",
            "",
            ".",
            "abc",
            "NaN",
            "inf",
            "-infinity",
            "1e5|mg",
            "5.4|mg",
            "gt5.4",
        ];
        let long_fraction = format!("0.{}1", "0".repeat(400));
        for text in texts.iter().copied().chain([long_fraction.as_str()]) {
            let precision = implicit_precision(text);
            assert!(
                precision.is_finite() && precision > 0.0,
                "implicit_precision({text:?}) = {precision}"
            );
            let value = text.parse::<f64>().unwrap_or(0.0);
            let (lo, hi) = implicit_range(value, text);
            assert!(
                !lo.is_nan() && !hi.is_nan(),
                "implicit_range({text:?}) = [{lo}, {hi})"
            );
            assert!(lo <= hi, "implicit_range({text:?}) = [{lo}, {hi})");
        }
    }
}
