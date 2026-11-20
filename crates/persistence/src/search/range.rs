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
//! | `ap`   | `v − m ≤ x ≤ v + m`       |
//!
//! `ap` has one margin on every backend (#1390): `m = max(0.1·|v|, p/2)`,
//! where `p` is the implicit precision. Ten percent of the value is the
//! spec's recommendation; the half-precision floor keeps `ap` at least as wide
//! as `eq`, so `ap0` is `[-0.5, 0.5]` and `ap1e2` is `[50, 150]`. See
//! [`approx_range`].

use crate::types::SearchPrefix;

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

/// Returns the closed `ap` range `[v − m, v + m]` for `value`, whose textual
/// form is `num_str`, with `m = max(0.1·|v|, implicit_precision / 2)`.
///
/// The floor makes the range contain the [`implicit_range`] of the same value,
/// so `ap` never matches less than `eq`. Neither bound is ever NaN: a NaN
/// `value` yields `[∞, ∞]`, which no finite stored value falls in.
pub fn approx_range(value: f64, num_str: &str) -> (f64, f64) {
    if value.is_nan() {
        return (f64::INFINITY, f64::INFINITY);
    }
    let margin = (value.abs() * 0.1).max(implicit_precision(num_str) / 2.0);
    (value - margin, value + margin)
}

/// Finite stand-in for "no lower bound" on a stored `Range` (#2602). Finite
/// because Elasticsearch cannot store `±Inf`; never canonicalized.
pub const QUANTITY_OPEN_LOW: f64 = f64::MIN;
/// Finite stand-in for "no upper bound" on a stored `Range` (#2602).
pub const QUANTITY_OPEN_HIGH: f64 = f64::MAX;

/// A comparison operator in a [`QuantityBound`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

impl Cmp {
    fn holds(self, stored: f64, bound: f64) -> bool {
        match self {
            Cmp::Lt => stored < bound,
            Cmp::Le => stored <= bound,
            Cmp::Gt => stored > bound,
            Cmp::Ge => stored >= bound,
        }
    }
}

/// One comparison of the stored closed interval `[L, H]` against a bound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QuantityBound {
    /// `L <cmp> bound`, on the lower end (`value_quantity_value`).
    Low(Cmp, f64),
    /// `H <cmp> bound`, on the upper end (the `high` column).
    High(Cmp, f64),
}

/// Prefix semantics of a quantity search against a stored closed interval
/// `[L, H]` (#2602), in disjunctive normal form: any group whose bounds all
/// hold matches. For a point (`L = H = x`) every prefix agrees with the
/// point rules.
#[derive(Debug, Clone, PartialEq)]
pub struct QuantityIntervalPredicate {
    /// OR of ANDs.
    pub any_of: Vec<Vec<QuantityBound>>,
}

impl QuantityIntervalPredicate {
    /// Whether the stored interval `[low, high]` matches.
    pub fn matches(&self, low: f64, high: f64) -> bool {
        self.any_of.iter().any(|group| {
            group.iter().all(|b| match *b {
                QuantityBound::Low(c, v) => c.holds(low, v),
                QuantityBound::High(c, v) => c.holds(high, v),
            })
        })
    }
}

/// Builds the interval predicate for `prefix` and the search value `value`
/// written as `num_str`.
///
/// | prefix | condition on `[L, H]` |
/// |---|---|
/// | `eq` | `s ≤ L ∧ H < e` (`[s, e)` = implicit range) |
/// | `ne` | `L < s ∨ H ≥ e` |
/// | `gt` / `ge` | `H > v` / `H ≥ v` |
/// | `lt` / `le` | `L < v` / `L ≤ v` |
/// | `sa` / `eb` | `L > v` / `H < v` |
/// | `ap` | `L ≤ v+m ∧ H ≥ v−m` |
pub fn quantity_interval_predicate(
    prefix: SearchPrefix,
    value: f64,
    num_str: &str,
) -> QuantityIntervalPredicate {
    use QuantityBound::{High, Low};
    let any_of = match prefix {
        SearchPrefix::Eq => {
            let (s, e) = implicit_range(value, num_str);
            vec![vec![Low(Cmp::Ge, s), High(Cmp::Lt, e)]]
        }
        SearchPrefix::Ne => {
            let (s, e) = implicit_range(value, num_str);
            vec![vec![Low(Cmp::Lt, s)], vec![High(Cmp::Ge, e)]]
        }
        SearchPrefix::Gt => vec![vec![High(Cmp::Gt, value)]],
        SearchPrefix::Ge => vec![vec![High(Cmp::Ge, value)]],
        SearchPrefix::Lt => vec![vec![Low(Cmp::Lt, value)]],
        SearchPrefix::Le => vec![vec![Low(Cmp::Le, value)]],
        SearchPrefix::Sa => vec![vec![Low(Cmp::Gt, value)]],
        SearchPrefix::Eb => vec![vec![High(Cmp::Lt, value)]],
        SearchPrefix::Ap => {
            let (lo, hi) = approx_range(value, num_str);
            vec![vec![Low(Cmp::Le, hi), High(Cmp::Ge, lo)]]
        }
    };
    QuantityIntervalPredicate { any_of }
}

/// Maps both bounds of a stored interval through `canonicalize` (UCUM
/// conversion), leaving the open-side sentinels untouched: scaling
/// `f64::MAX` would overflow to `inf`.
pub fn canonical_range(low: f64, high: f64, canonicalize: impl Fn(f64) -> f64) -> (f64, f64) {
    let conv = |v: f64| {
        if v == QUANTITY_OPEN_LOW || v == QUANTITY_OPEN_HIGH {
            v
        } else {
            canonicalize(v)
        }
    };
    (conv(low), conv(high))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approx_range_is_ten_percent_with_a_precision_floor() {
        let cases: &[(&str, f64, f64)] = &[
            ("100", 90.0, 110.0),
            ("-100", -110.0, -90.0),
            ("0", -0.5, 0.5),
            ("0.0", -0.05, 0.05),
            ("1e2", 50.0, 150.0),
            ("2", 1.5, 2.5),
            ("-2", -2.5, -1.5),
        ];
        for &(text, lo, hi) in cases {
            let value: f64 = text.parse().unwrap();
            assert_eq!(approx_range(value, text), (lo, hi), "{text}");
        }
        let (lo, hi) = approx_range(-5.4, "-5.4");
        assert!(
            (lo + 5.94).abs() < 1e-9 && (hi + 4.86).abs() < 1e-9,
            "{lo} {hi}"
        );
    }

    #[test]
    fn approx_range_contains_the_implicit_range() {
        for text in [
            "0", "7", "100", "100.0", "1e2", "1.00e2", "-5.4", "0.001", "1e-3", "12345",
        ] {
            let value: f64 = text.parse().unwrap();
            let (ap_lo, ap_hi) = approx_range(value, text);
            let (eq_lo, eq_hi) = implicit_range(value, text);
            assert!(ap_lo <= eq_lo && eq_hi <= ap_hi, "{text}");
        }
        assert_eq!(approx_range(f64::NAN, "1"), (f64::INFINITY, f64::INFINITY));
    }

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

    #[test]
    fn interval_predicate_on_a_point_equals_the_point_rules() {
        let prefixes = [
            SearchPrefix::Eq,
            SearchPrefix::Ne,
            SearchPrefix::Gt,
            SearchPrefix::Lt,
            SearchPrefix::Ge,
            SearchPrefix::Le,
            SearchPrefix::Sa,
            SearchPrefix::Eb,
            SearchPrefix::Ap,
        ];
        let texts = ["45", "5.4", "0", "-2", "100", "1e2"];
        for text in texts {
            let v: f64 = text.parse().unwrap();
            let (s, e) = implicit_range(v, text);
            let (alo, ahi) = approx_range(v, text);
            let mut xs = vec![v, s, e, alo, ahi, v - 1.0, v + 1.0, 0.0];
            xs.push(f64::from_bits(e.to_bits() + 1));
            for prefix in prefixes {
                let pred = quantity_interval_predicate(prefix, v, text);
                for &x in &xs {
                    let point = match prefix {
                        SearchPrefix::Eq => x >= s && x < e,
                        SearchPrefix::Ne => x < s || x >= e,
                        SearchPrefix::Gt | SearchPrefix::Sa => x > v,
                        SearchPrefix::Lt | SearchPrefix::Eb => x < v,
                        SearchPrefix::Ge => x >= v,
                        SearchPrefix::Le => x <= v,
                        SearchPrefix::Ap => x >= alo && x <= ahi,
                    };
                    assert_eq!(pred.matches(x, x), point, "{prefix:?} {text} x={x}");
                }
            }
        }
    }

    #[test]
    fn interval_predicate_on_ranges() {
        let p = |prefix, v: f64| quantity_interval_predicate(prefix, v, &v.to_string());
        // [40, 50]
        assert!(p(SearchPrefix::Gt, 45.0).matches(40.0, 50.0));
        assert!(p(SearchPrefix::Lt, 45.0).matches(40.0, 50.0));
        assert!(!p(SearchPrefix::Sa, 45.0).matches(40.0, 50.0));
        assert!(!p(SearchPrefix::Eb, 45.0).matches(40.0, 50.0));
        assert!(p(SearchPrefix::Ap, 45.0).matches(40.0, 50.0));
        assert!(!p(SearchPrefix::Eq, 45.0).matches(40.0, 50.0));
        assert!(p(SearchPrefix::Ne, 45.0).matches(40.0, 50.0));
        // open high: [60, ∞)
        assert!(p(SearchPrefix::Gt, 1000.0).matches(60.0, QUANTITY_OPEN_HIGH));
        assert!(!p(SearchPrefix::Lt, 50.0).matches(60.0, QUANTITY_OPEN_HIGH));
        // open low: (-∞, 10]
        assert!(p(SearchPrefix::Lt, -1000.0).matches(QUANTITY_OPEN_LOW, 10.0));
        assert!(p(SearchPrefix::Eb, 20.0).matches(QUANTITY_OPEN_LOW, 10.0));
    }

    #[test]
    fn canonical_range_keeps_open_sentinels() {
        let (l, h) = canonical_range(QUANTITY_OPEN_LOW, 2.0, |v| v * 1e300);
        assert_eq!(l, QUANTITY_OPEN_LOW);
        assert_eq!(h, 2.0e300);
    }
}
