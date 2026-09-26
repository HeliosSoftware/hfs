//! The one grammar for number and quantity search values.
//!
//! The numeric sibling of [`super::date_value`]. Every backend used to read a
//! number with `f64::from_str` and each did something different when that
//! failed: PostgreSQL skipped the value, so `probability=abc` was an
//! unconstrained search (#1319); Elasticsearch still did, because a handler
//! that returns `None` is filtered out of the query; SQLite matched nothing;
//! and only MongoDB returned an error. `f64::from_str` also reads `inf`,
//! `infinity` and `nan`, and rounds `1e999` to infinity — and those are not
//! just invalid, they widen: `probability=ltinf` matched every row on SQLite,
//! MongoDB and Elasticsearch. This module is the single answer to what a
//! number is.
//!
//! # Grammar
//!
//! The FHIR `decimal` grammar
//! (`-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`), read as widely as is
//! still unambiguous, because a false rejection turns a working search into a
//! `400`:
//!
//! ```text
//! [+|-] (digits [. [digits]] | . digits) [(e|E) [+|-] digits]
//! ```
//!
//! - a leading `+`, leading zeros (`007`) and a bare leading or trailing point
//!   (`.5`, `5.`) are tolerated;
//! - the value must be finite once parsed: `1e999` is rejected, and so are the
//!   words `f64::from_str` accepts (`inf`, `infinity`, `nan`, in any case). A
//!   literal that underflows (`1e-999`) is zero, which is finite;
//! - hexadecimal (`0x10`), digit separators (`1_000`, `1,000`) and anything
//!   after the number are rejected. Surrounding whitespace is trimmed.
//!
//! # `+` decoded to a space
//!
//! As for a date's zone offset (#1296), `application/x-www-form-urlencoded`
//! decoding turns an unencoded `+` into a space, so `value-quantity=1e+3`
//! arrives as `1e 3`. The grammar has no legal space, so a space in the
//! exponent's sign position can only be a decoded `+` and is read as one. A
//! decoded leading `+` (`gt+5`) is covered by the trimming above.
//! [`FhirNumberValue::text`] returns the repaired text; backends must use it,
//! or the parsed value, and never the raw value.
//!
//! # Quantity
//!
//! `number`, `number|code` or `number|system|code`. Only an *unescaped* `|`
//! separates — `\|` is a literal pipe inside a part — and anything after the
//! second separator belongs to the code. Only the number part is validated;
//! system and code are free text, and an empty one is absent.
//!
//! # Ranges and prefixes
//!
//! Not here: the implicit-precision range of a number, and the way each prefix
//! compares against it, are [`super::range`]'s.
//! [`FhirNumberValue::implicit_range`] calls it with the text it was parsed
//! from.

use crate::error::{SearchError, StorageError, StorageResult};
use crate::types::{SearchModifier, SearchParamType, SearchParameter, SearchPrefix, SearchQuery};

/// The forms a number search value may take, for error messages.
const EXPECTED: &str = "expected a decimal number with an optional exponent, such as 5, -5.4 or \
                        1.5e3; a quantity is number, number|code or number|system|code";

/// Why a value is not a number search value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberValueErrorReason {
    /// The value, or the number part of a quantity, is empty.
    Empty,
    /// The value does not have the shape of a number at all.
    Syntax,
    /// `inf`, `infinity` or `nan`, which a float parser would accept.
    NotFinite,
    /// A well-formed literal too large to represent (`1e999`).
    OutOfRange,
}

impl std::fmt::Display for NumberValueErrorReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NumberValueErrorReason::Empty => "the number is empty",
            NumberValueErrorReason::Syntax => "not a number",
            NumberValueErrorReason::NotFinite => "infinity and NaN are not search values",
            NumberValueErrorReason::OutOfRange => "too large to represent",
        })
    }
}

/// A value that is not a FHIR number or quantity search value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{value}' is not a valid number ({reason}); {}", EXPECTED)]
pub struct NumberValueError {
    /// The rejected value, as received.
    pub value: String,
    /// What is wrong with it.
    pub reason: NumberValueErrorReason,
}

/// A parsed number search value: the finite value, and the text it was
/// written as, which carries its implicit precision.
#[derive(Debug, Clone, PartialEq)]
pub struct FhirNumberValue {
    /// The value. Always finite.
    pub value: f64,
    /// Whether a space in the exponent's sign position was read as `+`.
    pub repaired_plus: bool,
    text: String,
}

impl FhirNumberValue {
    /// Parses a number search value whose comparator prefix has already been
    /// removed.
    pub fn parse(raw: &str) -> Result<Self, NumberValueError> {
        parse(raw.trim()).map_err(|reason| NumberValueError {
            value: raw.to_string(),
            reason,
        })
    }

    /// The number as the client meant it: trimmed, with a form-decoded `+`
    /// restored. This is the text the implicit precision is read from.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The half-open implicit-precision range `[lo, hi)` that `eq` and `ne`
    /// compare against — see [`super::range`].
    pub fn implicit_range(&self) -> (f64, f64) {
        super::range::implicit_range(self.value, &self.text)
    }

    /// The closed range `[lo, hi]` that `ap` compares against — see
    /// [`super::range::approx_range`].
    pub fn approx_range(&self) -> (f64, f64) {
        super::range::approx_range(self.value, &self.text)
    }
}

/// A parsed quantity search value.
#[derive(Debug, Clone, PartialEq)]
pub struct FhirQuantityValue {
    /// The number part.
    pub number: FhirNumberValue,
    /// The unit system, unescaped; `None` when absent or empty.
    pub system: Option<String>,
    /// The unit code, unescaped; `None` when absent or empty.
    pub code: Option<String>,
}

impl FhirQuantityValue {
    /// Parses a quantity search value — `number`, `number|code` or
    /// `number|system|code` — whose comparator prefix has already been removed.
    pub fn parse(raw: &str) -> Result<Self, NumberValueError> {
        let mut parts = split_unescaped(raw, '|', 3).into_iter();
        let number = parts.next().unwrap_or_default();
        let number = parse(number.trim()).map_err(|reason| NumberValueError {
            value: raw.to_string(),
            reason,
        })?;
        let non_empty = |s: String| (!s.is_empty()).then_some(s);
        let (system, code) = match (parts.next(), parts.next()) {
            (Some(system), Some(code)) => (non_empty(system), non_empty(code)),
            (Some(code), None) => (None, non_empty(code)),
            _ => (None, None),
        };
        Ok(Self {
            number,
            system,
            code,
        })
    }
}

/// Splits `raw` on unescaped `separator`s into at most `max_parts` parts,
/// unescaping `\separator` inside each. Anything after the last permitted
/// separator stays in the last part; every other escape is left as written,
/// for whoever reads the part next.
///
/// The REST layer leaves `\|` and `\$` intact for this purpose — see
/// [`super::split_unescaped_commas`].
fn split_unescaped(raw: &str, separator: char, max_parts: usize) -> Vec<String> {
    let mut parts: Vec<String> = vec![String::new()];
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        let last = parts.len() - 1;
        match c {
            '\\' if chars.peek() == Some(&separator) => {
                chars.next();
                parts[last].push(separator);
            }
            c if c == separator && parts.len() < max_parts => parts.push(String::new()),
            _ => parts[last].push(c),
        }
    }
    parts
}

fn parse(text: &str) -> Result<FhirNumberValue, NumberValueErrorReason> {
    use NumberValueErrorReason as Reason;

    if text.is_empty() {
        return Err(Reason::Empty);
    }
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());

    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if ["inf", "infinity", "nan"]
        .iter()
        .any(|word| unsigned.eq_ignore_ascii_case(word))
    {
        return Err(Reason::NotFinite);
    }

    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (unsigned, None),
    };
    // A space where the exponent's sign belongs is a `+` that form-decoding
    // turned into a space.
    let mut repaired_plus = false;
    if let Some(exponent) = exponent {
        let magnitude = match exponent.strip_prefix(['+', '-']) {
            Some(magnitude) => magnitude,
            None => match exponent.strip_prefix(' ') {
                Some(magnitude) => {
                    repaired_plus = true;
                    magnitude
                }
                None => exponent,
            },
        };
        if magnitude.is_empty() || !digits(magnitude) {
            return Err(Reason::Syntax);
        }
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (whole.is_empty() && fraction.is_empty()) || !digits(whole) || !digits(fraction) {
        return Err(Reason::Syntax);
    }

    let text = if repaired_plus {
        text.replacen(' ', "+", 1)
    } else {
        text.to_string()
    };
    // The shape is a decimal literal, so this only fails to be finite by
    // overflowing.
    let value: f64 = text.parse().map_err(|_| Reason::Syntax)?;
    if !value.is_finite() {
        return Err(Reason::OutOfRange);
    }
    Ok(FhirNumberValue {
        value,
        repaired_plus,
        text,
    })
}

/// Rejects a query carrying a number or quantity search value that is not a
/// number.
///
/// Call this beside [`super::validate_date_values`], from every entry point
/// that builds a backend query. Every `SearchQuery` reaches one — REST
/// searches, conditional operations, batch entries, compartment and
/// `$everything` searches, and the per-hop queries the chain resolver issues
/// for chained and `_has` terminals — so an invalid number is an error on every
/// path, not only the ones a REST extractor happened to cover.
pub fn validate_numeric_values(query: &SearchQuery) -> StorageResult<()> {
    for param in &query.parameters {
        validate_numeric_parameter(param).map_err(StorageError::Search)?;
    }
    Ok(())
}

/// Rejects a parameter carrying a number or quantity search value that is not
/// a number.
///
/// Skipped for `:missing`, whose value is a boolean, and for a chained
/// parameter, whose value belongs to the chain's last link; the chain resolver
/// replaces it with a terminal query that is validated in its own right.
pub fn validate_numeric_parameter(param: &SearchParameter) -> Result<(), SearchError> {
    if matches!(param.modifier, Some(SearchModifier::Missing)) || !param.chain.is_empty() {
        return Ok(());
    }
    let invalid = |value: &str, error: NumberValueError| SearchError::InvalidNumberValue {
        param: param.name.clone(),
        value: value.to_string(),
        reason: error.to_string(),
    };
    let check = |param_type: SearchParamType, text: &str| match param_type {
        SearchParamType::Number => FhirNumberValue::parse(text).map(drop),
        SearchParamType::Quantity => FhirQuantityValue::parse(text).map(drop),
        _ => Ok(()),
    };

    if param.param_type == SearchParamType::Composite {
        // A composite value is `$`-joined, one part per component. A value
        // with the wrong number of parts is the backend's to reject.
        for value in &param.values {
            let parts = split_unescaped(&value.value, '$', usize::MAX);
            for (part, component) in parts.iter().zip(&param.components) {
                let (_, number) = SearchPrefix::extract(part);
                check(component.param_type, number).map_err(|e| invalid(&value.value, e))?;
            }
        }
    } else {
        for value in &param.values {
            check(param.param_type, &value.value).map_err(|e| invalid(&value.value, e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChainedParameter, CompositeSearchComponent, SearchValue};

    fn parsed(text: &str) -> FhirNumberValue {
        FhirNumberValue::parse(text).unwrap_or_else(|e| panic!("{text} should parse: {e}"))
    }

    #[test]
    fn valid_values() {
        for (input, value) in [
            ("5", 5.0),
            ("0", 0.0),
            ("-0", -0.0),
            ("5.4", 5.4),
            ("-5.4", -5.4),
            ("100.00", 100.0),
            ("1e3", 1000.0),
            ("1E3", 1000.0),
            ("1e+3", 1000.0),
            ("1.5E-2", 0.015),
            ("-1.5e2", -150.0),
            // Leniencies: nothing else these could mean.
            ("+5.4", 5.4),
            ("007", 7.0),
            (".5", 0.5),
            ("-.5", -0.5),
            ("5.", 5.0),
            ("5.e2", 500.0),
            // Underflow is zero, which is finite.
            ("1e-999", 0.0),
            // Surrounding whitespace is not part of the value.
            (" 5.4 ", 5.4),
        ] {
            let number = parsed(input);
            assert_eq!(number.value, value, "value of {input}");
            assert!(!number.repaired_plus, "{input} needed no repair");
            assert_eq!(number.text(), input.trim());
        }
    }

    #[test]
    fn invalid_values_and_why() {
        use NumberValueErrorReason::*;
        for (input, reason) in [
            ("", Empty),
            ("   ", Empty),
            ("abc", Syntax),
            ("true", Syntax),
            ("1e", Syntax),
            ("1e+", Syntax),
            ("e5", Syntax),
            (".", Syntax),
            ("-", Syntax),
            ("+", Syntax),
            ("--5", Syntax),
            ("+-5", Syntax),
            ("5-", Syntax),
            ("1.2.3", Syntax),
            ("1e2e3", Syntax),
            ("1e2.5", Syntax),
            ("0x10", Syntax),
            ("1_000", Syntax),
            ("1,000", Syntax),
            ("5 4", Syntax),
            ("5.4mg", Syntax),
            ("５", Syntax),
            // A comparator prefix is the caller's to remove.
            ("gt5", Syntax),
            // A quantity is the caller's to split.
            ("5.4|mg", Syntax),
            // Not the shape a decoded `+` leaves behind.
            ("1e  3", Syntax),
            ("1 e3", Syntax),
            ("inf", NotFinite),
            ("-inf", NotFinite),
            ("+inf", NotFinite),
            ("Infinity", NotFinite),
            ("-infinity", NotFinite),
            ("nan", NotFinite),
            ("NaN", NotFinite),
            ("-NAN", NotFinite),
            ("1e999", OutOfRange),
            ("-1e999", OutOfRange),
        ] {
            match FhirNumberValue::parse(input) {
                Ok(value) => panic!("{input:?} parsed as {value:?}"),
                Err(error) => {
                    assert_eq!(error.reason, reason, "reason for {input:?}");
                    assert_eq!(error.value, input);
                }
            }
        }
    }

    #[test]
    fn a_space_in_the_exponent_sign_position_is_a_decoded_plus() {
        for (decoded, meant) in [("1e 3", "1e+3"), ("-2.50E 1", "-2.50E+1")] {
            let repaired = parsed(decoded);
            assert!(repaired.repaired_plus);
            assert_eq!(repaired.text(), meant);
            assert_eq!(repaired.value, parsed(meant).value);
        }
    }

    #[test]
    fn implicit_range_is_read_from_the_text() {
        assert_eq!(parsed("100").implicit_range(), (99.5, 100.5));
        let (lo, hi) = parsed("100.0").implicit_range();
        assert!((lo - 99.95).abs() < 1e-9 && (hi - 100.05).abs() < 1e-9);
    }

    #[test]
    fn quantity_forms() {
        let some = |s: &str| Some(s.to_string());
        for (input, number, system, code) in [
            ("5.4", "5.4", None, None),
            ("5.4|mg", "5.4", None, some("mg")),
            ("5.4||mg", "5.4", None, some("mg")),
            (
                "5.4|http://unitsofmeasure.org|mg",
                "5.4",
                some("http://unitsofmeasure.org"),
                some("mg"),
            ),
            (
                "5.4|http://unitsofmeasure.org|",
                "5.4",
                some("http://unitsofmeasure.org"),
                None,
            ),
            ("5.4||", "5.4", None, None),
            ("5.4|", "5.4", None, None),
            ("1e3|mg", "1e3", None, some("mg")),
            // `\|` is a literal pipe, in either part.
            (
                "5.4|sys\\|tem|co\\|de",
                "5.4",
                some("sys|tem"),
                some("co|de"),
            ),
            ("5.4|a\\|b", "5.4", None, some("a|b")),
            // Anything after the second separator belongs to the code.
            ("5.4|sys|a|b", "5.4", some("sys"), some("a|b")),
            // Other escapes are not this layer's.
            ("5.4||a\\$b", "5.4", None, some("a\\$b")),
        ] {
            let quantity = FhirQuantityValue::parse(input)
                .unwrap_or_else(|e| panic!("{input} should parse: {e}"));
            assert_eq!(quantity.number.text(), number, "number of {input}");
            assert_eq!(quantity.system, system, "system of {input}");
            assert_eq!(quantity.code, code, "code of {input}");
        }
    }

    #[test]
    fn quantity_validates_the_number_part_only() {
        use NumberValueErrorReason::*;
        for (input, reason) in [
            ("", Empty),
            ("||mg", Empty),
            ("|http://unitsofmeasure.org|mg", Empty),
            ("abc|http://unitsofmeasure.org|mg", Syntax),
            ("mg", Syntax),
            // An escaped pipe does not end the number part.
            ("5.4\\|mg", Syntax),
            ("inf||mg", NotFinite),
            ("1e999||mg", OutOfRange),
        ] {
            let error = FhirQuantityValue::parse(input).expect_err(input);
            assert_eq!(error.reason, reason, "reason for {input:?}");
            // The whole value is reported, not just the number part.
            assert_eq!(error.value, input);
        }
    }

    fn param(name: &str, param_type: SearchParamType, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        }
    }

    fn assert_invalid(query: &SearchQuery, expected_param: &str, expected_value: &str) {
        match validate_numeric_values(query) {
            Err(StorageError::Search(SearchError::InvalidNumberValue { param, value, .. })) => {
                assert_eq!(param, expected_param);
                assert_eq!(value, expected_value);
            }
            other => panic!("expected InvalidNumberValue, got {other:?}"),
        }
    }

    #[test]
    fn gate_rejects_invalid_number_and_quantity_values() {
        for value in [
            "abc", "1e", "", "inf", "-inf", "nan", "NaN", "1e999", "0x10",
        ] {
            let query = SearchQuery::new("RiskAssessment").with_parameter(param(
                "probability",
                SearchParamType::Number,
                value,
            ));
            assert_invalid(&query, "probability", value);
            let query = SearchQuery::new("Observation").with_parameter(param(
                "value-quantity",
                SearchParamType::Quantity,
                value,
            ));
            assert_invalid(&query, "value-quantity", value);
        }
        for value in ["abc|http://unitsofmeasure.org|mg", "|sys|mg", "||mg"] {
            let query = SearchQuery::new("Observation").with_parameter(param(
                "value-quantity",
                SearchParamType::Quantity,
                value,
            ));
            assert_invalid(&query, "value-quantity", value);
        }
    }

    #[test]
    fn gate_checks_every_value_of_an_or_list() {
        let mut number = param("probability", SearchParamType::Number, "0.5");
        number
            .values
            .push(SearchValue::new(SearchPrefix::Gt, "nope"));
        let query = SearchQuery::new("RiskAssessment").with_parameter(number);
        assert_invalid(&query, "probability", "nope");
    }

    #[test]
    fn gate_accepts_valid_values() {
        let mut number = param("probability", SearchParamType::Number, "+5.4");
        number
            .values
            .push(SearchValue::new(SearchPrefix::Ge, "1e 3"));
        let mut quantity = param("value-quantity", SearchParamType::Quantity, "5.4||mg");
        quantity
            .values
            .push(SearchValue::new(SearchPrefix::Lt, ".5|a\\|b|c"));
        let query = SearchQuery::new("Observation")
            .with_parameter(number)
            .with_parameter(quantity);
        assert!(validate_numeric_values(&query).is_ok());
    }

    #[test]
    fn gate_skips_missing_chains_and_other_types() {
        // `:missing` carries a boolean, not a number.
        let mut missing = param("probability", SearchParamType::Number, "true");
        missing.modifier = Some(SearchModifier::Missing);
        // A chain's value belongs to its last link.
        let mut chained = param("subject", SearchParamType::Number, "abc");
        chained.chain = vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }];
        // Not a number parameter at all.
        let string = param("name", SearchParamType::String, "inf");
        let token = param("code", SearchParamType::Token, "abc|def");

        let query = SearchQuery::new("Observation")
            .with_parameter(missing)
            .with_parameter(chained)
            .with_parameter(string)
            .with_parameter(token);
        assert!(validate_numeric_values(&query).is_ok());
    }

    #[test]
    fn gate_checks_the_numeric_components_of_a_composite() {
        let composite = |value: &str, second: SearchParamType| SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: second,
                    param_name: "value".to_string(),
                },
            ],
        };

        for value in [
            "8480-6$abc",
            "8480-6$gtabc",
            "8480-6$ltinf",
            "8480-6$",
            "8480-6$||mm[Hg]",
        ] {
            for second in [SearchParamType::Number, SearchParamType::Quantity] {
                let query =
                    SearchQuery::new("Observation").with_parameter(composite(value, second));
                assert_invalid(&query, "code-value-quantity", value);
            }
        }
        // The token half is not a number, whatever it looks like; the numeric
        // half may carry a prefix; an escaped `$` does not start a component.
        for value in [
            "abc$5.4",
            "8480-6$gt5.4|http://unitsofmeasure.org|mm[Hg]",
            "http://loinc.org|8480-6$le1e3",
            "a\\$b$5.4",
        ] {
            let query = SearchQuery::new("Observation")
                .with_parameter(composite(value, SearchParamType::Quantity));
            assert!(validate_numeric_values(&query).is_ok(), "{value}");
        }
        // Without resolved components there is nothing to type the parts by.
        let mut untyped = composite("8480-6$abc", SearchParamType::Quantity);
        untyped.components.clear();
        let query = SearchQuery::new("Observation").with_parameter(untyped);
        assert!(validate_numeric_values(&query).is_ok());
    }
}
