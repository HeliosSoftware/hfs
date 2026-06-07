//! Implicit-precision ranges for ordered (number/quantity) search values.
//!
//! FHIR treats a decimal search value as a range determined by its significant
//! figures: `100` ⇒ `[99.5, 100.5)`, `100.0` ⇒ `[99.95, 100.05)`. Comparator
//! prefixes are then defined against that range's boundaries (per
//! <https://build.fhir.org/search.html#prefix>):
//!
//! | prefix | match (target value `x`, search range `[lo, hi)`) |
//! |--------|---------------------------------------------------|
//! | `eq`   | `lo ≤ x < hi`                                     |
//! | `ne`   | `x < lo OR x ≥ hi`                                 |
//! | `ge`   | `x ≥ lo`                                           |
//! | `le`   | `x < hi`                                           |
//! | `gt` / `sa` | `x ≥ hi`                                      |
//! | `lt` / `eb` | `x < lo`                                      |

/// Returns the implicit precision (ULP of the least significant digit) of a
/// decimal value from its string form: `"100"` → 1.0, `"100.0"` → 0.1.
pub fn implicit_precision(num_str: &str) -> f64 {
    match num_str.find('.') {
        Some(dot) => {
            let decimals = num_str.len() - dot - 1;
            10f64.powi(-(decimals as i32))
        }
        None => 1.0,
    }
}

/// Returns the half-open implicit-precision range `[lo, hi)` for `value`, whose
/// textual form is `num_str` (used to derive the precision).
pub fn implicit_range(value: f64, num_str: &str) -> (f64, f64) {
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
}
