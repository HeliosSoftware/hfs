//! Decimals outside `rust_decimal`'s range, held without expanding them.
//!
//! `rust_decimal::Decimal` covers about ±7.9e28 with at most 28 fractional digits.
//! A FHIR decimal such as `1e999999999` or `1e-999999999` is valid but does not fit,
//! and written out in full it is about a billion digits. [`OutOfRangeDecimal`] keeps
//! such a value as its significand digits and a base-10 exponent, taken from the
//! source text, so it can be ordered, compared, tested for equivalence and printed
//! without ever being expanded (#2716).
//!
//! Only those operations are supported. Arithmetic, unit conversion and boundaries
//! on these values give empty in the evaluator.

use rust_decimal::Decimal;
use std::cmp::Ordering;

/// A decimal value written with an exponent outside `rust_decimal`'s range.
///
/// The value is `±digits × 10^exponent`, where `digits` is the significand as an
/// integer. Trailing zeros of the significand are kept, since they carry precision:
/// `1.50e999999999` is `150 × 10^999999997`.
///
/// The type also holds ordinary values converted from a [`Decimal`] (see
/// [`OutOfRangeDecimal::from_decimal`]), so an out-of-range value can be compared
/// with an in-range one through a single representation.
#[derive(Debug, Clone)]
pub struct OutOfRangeDecimal {
    negative: bool,
    /// Significand digits, ASCII `0`-`9`, without leading zeros. `"0"` for zero.
    digits: Box<str>,
    /// Power of ten of the last significand digit.
    exponent: i64,
}

impl OutOfRangeDecimal {
    /// Parses decimal text such as `1e999999999`, `-2.50E-999999999` or `12.5`.
    ///
    /// Accepts an optional sign, digits with an optional fractional part, and an
    /// optional `e`/`E` exponent. Returns `None` for malformed text and for an
    /// exponent that does not fit in an `i64` once the fractional digits are
    /// accounted for. The text is not required to be out of range; callers decide
    /// when to use this type.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (negative, rest) = match text.as_bytes().first()? {
            b'-' => (true, &text[1..]),
            b'+' => (false, &text[1..]),
            _ => (false, text),
        };
        let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
            Some((m, e)) => (m, parse_exponent(e)?),
            None => (rest, 0),
        };
        let (int_part, frac_part) = match mantissa.split_once('.') {
            Some((i, f)) => (i, f),
            None => (mantissa, ""),
        };
        if int_part.is_empty()
            || !int_part.bytes().all(|b| b.is_ascii_digit())
            || !frac_part.bytes().all(|b| b.is_ascii_digit())
            || (mantissa.contains('.') && frac_part.is_empty())
        {
            return None;
        }
        let frac_len = i64::try_from(frac_part.len()).ok()?;
        let exponent = exponent.checked_sub(frac_len)?;
        let all_digits = format!("{int_part}{frac_part}");
        let significant = all_digits.trim_start_matches('0');
        let (digits, negative) = if significant.is_empty() {
            ("0", false)
        } else {
            (significant, negative)
        };
        Some(Self {
            negative,
            digits: digits.into(),
            exponent,
        })
    }

    /// Represents an in-range [`Decimal`] in the same form, keeping its scale.
    pub fn from_decimal(value: Decimal) -> Self {
        let mantissa = value.mantissa();
        let digits = mantissa.unsigned_abs().to_string();
        Self {
            negative: mantissa < 0,
            digits: digits.into(),
            exponent: -i64::from(value.scale()),
        }
    }

    /// Represents an integer in the same form.
    pub fn from_integer(value: i64) -> Self {
        Self {
            negative: value < 0,
            digits: value.unsigned_abs().to_string().into(),
            exponent: 0,
        }
    }

    /// True when the value is zero.
    pub fn is_zero(&self) -> bool {
        self.digits.as_ref() == "0"
    }

    /// True when the value is below zero.
    pub fn is_negative(&self) -> bool {
        self.negative
    }

    /// The power of ten of the least significant digit: `1e999999999` is
    /// `999999999`, `1.5e-999999999` is `-1000000000`, `12.30` is `-2`.
    pub fn precision_exponent(&self) -> i64 {
        self.exponent
    }

    /// The value with its sign flipped. Zero stays zero.
    pub fn negated(&self) -> Self {
        Self {
            negative: !self.negative && !self.is_zero(),
            digits: self.digits.clone(),
            exponent: self.exponent,
        }
    }

    /// Power of ten of the leading digit, i.e. the exponent in scientific notation.
    /// `None` for zero.
    fn magnitude(&self) -> Option<i128> {
        if self.is_zero() {
            None
        } else {
            Some(i128::from(self.exponent) + self.digits.len() as i128 - 1)
        }
    }

    /// Significand digits with trailing zeros removed (`"0"` for zero).
    fn significant_digits(&self) -> &str {
        if self.is_zero() {
            "0"
        } else {
            self.digits.trim_end_matches('0')
        }
    }

    /// Numeric comparison, ignoring precision: `1.0` and `1` compare equal.
    pub fn numeric_cmp(&self, other: &Self) -> Ordering {
        let sign = |d: &Self| -> i8 {
            if d.is_zero() {
                0
            } else if d.negative {
                -1
            } else {
                1
            }
        };
        let (sa, sb) = (sign(self), sign(other));
        if sa != sb {
            return sa.cmp(&sb);
        }
        if sa == 0 {
            return Ordering::Equal;
        }
        let magnitude_order = match self.magnitude().cmp(&other.magnitude()) {
            Ordering::Equal => {
                // Same leading power of ten: compare digit by digit, padding the
                // shorter significand with zeros.
                let (a, b) = (self.significant_digits(), other.significant_digits());
                let len = a.len().max(b.len());
                let padded_a = a.bytes().chain(std::iter::repeat(b'0')).take(len);
                let padded_b = b.bytes().chain(std::iter::repeat(b'0')).take(len);
                padded_a.cmp(padded_b)
            }
            other_order => other_order,
        };
        if sa < 0 {
            magnitude_order.reverse()
        } else {
            magnitude_order
        }
    }

    /// The value rounded half away from zero so that its last digit is the
    /// `10^exponent` place. Returned unchanged when it is already at least that
    /// coarse.
    pub fn rounded_to(&self, exponent: i64) -> Self {
        if self.exponent >= exponent || self.is_zero() {
            return self.clone();
        }
        // Number of digits to drop from the right. Both exponents are i64, so the
        // difference fits in i128; any count past the significand's length zeroes it.
        let drop = i128::from(exponent) - i128::from(self.exponent);
        let len = self.digits.len() as i128;
        let zero = Self {
            negative: false,
            digits: "0".into(),
            exponent,
        };
        if drop > len {
            return zero;
        }
        let keep = (len - drop) as usize;
        let bytes = self.digits.as_bytes();
        let round_up = bytes[keep] >= b'5';
        let mut kept: Vec<u8> = bytes[..keep].to_vec();
        if round_up {
            let mut i = kept.len();
            loop {
                if i == 0 {
                    kept.insert(0, b'1');
                    break;
                }
                i -= 1;
                if kept[i] == b'9' {
                    kept[i] = b'0';
                } else {
                    kept[i] += 1;
                    break;
                }
            }
        }
        let text = String::from_utf8(kept).unwrap_or_default();
        let trimmed = text.trim_start_matches('0');
        if trimmed.is_empty() {
            return zero;
        }
        Self {
            negative: self.negative,
            digits: trimmed.into(),
            exponent,
        }
    }

    /// FHIRPath decimal equivalence: both values rounded to the precision of the
    /// less precise one compare equal. `1e-999999999 ~ 0` is true; `1e999999999 ~ 1`
    /// is false.
    pub fn equivalent(&self, other: &Self) -> bool {
        let exponent = self.exponent.max(other.exponent);
        self.rounded_to(exponent)
            .numeric_cmp(&other.rounded_to(exponent))
            == Ordering::Equal
    }

    /// Scientific notation with an explicit exponent sign, keeping every
    /// significand digit: `1E+999999999`, `-2.50E-999999999`, `0E+5`.
    pub fn to_scientific_string(&self) -> String {
        let magnitude = match self.magnitude() {
            Some(m) => m,
            None => i128::from(self.exponent),
        };
        let (lead, rest) = self.digits.split_at(1);
        let mut out = String::with_capacity(self.digits.len() + 24);
        if self.negative {
            out.push('-');
        }
        out.push_str(lead);
        if !rest.is_empty() {
            out.push('.');
            out.push_str(rest);
        }
        out.push('E');
        out.push(if magnitude < 0 { '-' } else { '+' });
        out.push_str(&magnitude.unsigned_abs().to_string());
        out
    }
}

/// Parses an exponent such as `999999999`, `+5` or `-12`.
fn parse_exponent(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Numeric equality, so `1.0E+999999999` equals `1E+999999999` and hashes the same.
impl PartialEq for OutOfRangeDecimal {
    fn eq(&self, other: &Self) -> bool {
        self.numeric_cmp(other) == Ordering::Equal
    }
}

impl Eq for OutOfRangeDecimal {}

impl PartialOrd for OutOfRangeDecimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OutOfRangeDecimal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.numeric_cmp(other)
    }
}

impl std::hash::Hash for OutOfRangeDecimal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Consistent with numeric equality: sign, leading power of ten and the
        // significand without trailing zeros identify the value.
        if self.is_zero() {
            0u8.hash(state);
        } else {
            self.negative.hash(state);
            self.magnitude().hash(state);
            self.significant_digits().hash(state);
        }
    }
}

impl std::fmt::Display for OutOfRangeDecimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_scientific_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> OutOfRangeDecimal {
        OutOfRangeDecimal::parse(s).unwrap()
    }

    fn dec(s: &str) -> OutOfRangeDecimal {
        OutOfRangeDecimal::from_decimal(Decimal::from_str(s).unwrap())
    }

    #[test]
    fn parses_without_expanding() {
        let huge = d("1e999999999");
        assert_eq!(huge.to_scientific_string(), "1E+999999999");
        assert_eq!(huge.precision_exponent(), 999_999_999);
        let tiny = d("1e-999999999");
        assert_eq!(tiny.to_scientific_string(), "1E-999999999");
        assert_eq!(
            d("-2.50E-999999999").to_scientific_string(),
            "-2.50E-999999999"
        );
        assert_eq!(d("1.50e999999999").precision_exponent(), 999_999_997);
        assert_eq!(d("0012.5").to_scientific_string(), "1.25E+1");
        assert_eq!(d("-0.0e5").to_scientific_string(), "0E+4");
        assert!(!d("-0.0e5").is_negative());
    }

    #[test]
    fn rejects_malformed_text_and_unrepresentable_exponents() {
        for bad in [
            "", "-", "e5", "1e", "1e+", "1.e5", ".5", "1x", "1e5.0", "1e--5", "+-1",
        ] {
            assert!(OutOfRangeDecimal::parse(bad).is_none(), "{bad:?}");
        }
        assert!(OutOfRangeDecimal::parse("1e99999999999999999999").is_none());
        // Fractional digits push the exponent below i64::MIN.
        assert!(OutOfRangeDecimal::parse("1.5e-9223372036854775808").is_none());
    }

    #[test]
    fn orders_by_sign_then_magnitude_then_digits() {
        let huge = d("1e999999999");
        let tiny = d("1e-999999999");
        assert_eq!(huge.numeric_cmp(&dec("1")), Ordering::Greater);
        assert_eq!(
            huge.numeric_cmp(&dec("79228162514264337593543950335")),
            Ordering::Greater
        );
        assert_eq!(d("-1e999999999").numeric_cmp(&dec("-1")), Ordering::Less);
        assert_eq!(tiny.numeric_cmp(&dec("0")), Ordering::Greater);
        assert_eq!(
            tiny.numeric_cmp(&dec("0.0000000000000000000000000001")),
            Ordering::Less
        );
        assert_eq!(d("-1e-999999999").numeric_cmp(&dec("0")), Ordering::Less);
        assert_eq!(
            d("1.5e999999999").numeric_cmp(&d("1.49e999999999")),
            Ordering::Greater
        );
        assert_eq!(
            d("1.50e999999999").numeric_cmp(&d("1.5e999999999")),
            Ordering::Equal
        );
        assert_eq!(
            d("-1.5e999999999").numeric_cmp(&d("-1.49e999999999")),
            Ordering::Less
        );
        assert_eq!(dec("12.30").numeric_cmp(&dec("12.3")), Ordering::Equal);
        assert_eq!(dec("0.00").numeric_cmp(&dec("0")), Ordering::Equal);
    }

    #[test]
    fn rounds_half_away_from_zero_without_expanding() {
        assert_eq!(
            d("1e-999999999").rounded_to(0).to_scientific_string(),
            "0E+0"
        );
        assert_eq!(dec("1.25").rounded_to(-1).to_scientific_string(), "1.3E+0");
        assert_eq!(
            dec("-1.25").rounded_to(-1).to_scientific_string(),
            "-1.3E+0"
        );
        assert_eq!(dec("9.96").rounded_to(-1).to_scientific_string(), "1.00E+1");
        assert_eq!(dec("0.6").rounded_to(0).to_scientific_string(), "1E+0");
        assert_eq!(dec("0.4").rounded_to(0).to_scientific_string(), "0E+0");
        // Rounding 1 to the precision of 1e999999999 drops it to zero.
        assert!(dec("1").rounded_to(999_999_999).is_zero());
        assert_eq!(
            d("1e999999999").rounded_to(0).to_scientific_string(),
            "1E+999999999"
        );
    }

    #[test]
    fn equivalence_uses_the_less_precise_operand() {
        assert!(d("1e-999999999").equivalent(&dec("0")));
        assert!(d("1e-999999999").equivalent(&dec("0.0")));
        assert!(!d("1e999999999").equivalent(&dec("1")));
        assert!(d("1e999999999").equivalent(&d("1.0e999999999")));
        assert!(dec("1.24").equivalent(&dec("1.2")));
        assert!(!dec("1.26").equivalent(&dec("1.2")));
        assert!(!d("-1e999999999").equivalent(&d("1e999999999")));
    }

    #[test]
    fn equality_and_hash_follow_numeric_value() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let hash = |v: &OutOfRangeDecimal| {
            let mut h = DefaultHasher::new();
            v.hash(&mut h);
            h.finish()
        };
        let a = d("1e999999999");
        let b = d("1.000e999999999");
        assert_eq!(a, b);
        assert_eq!(hash(&a), hash(&b));
        assert_ne!(a, d("1e999999998"));
        assert_eq!(d("-0e5"), dec("0"));
        assert_eq!(hash(&d("-0e5")), hash(&dec("0.00")));
    }

    #[test]
    fn negation_keeps_zero_unsigned() {
        assert_eq!(
            d("1e999999999").negated().to_scientific_string(),
            "-1E+999999999"
        );
        assert!(!dec("0").negated().is_negative());
    }
}
