//! Value Converters for Search Index.
//!
//! Converts FHIRPath evaluation results into index-friendly values.
//! Each FHIR data type is mapped to appropriate index columns.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{DatePrecision, SearchParamType};

use super::errors::ExtractionError;

/// A value extracted and converted for the search index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IndexValue {
    /// String value for string parameters.
    String(String),

    /// Token value (code with optional system).
    Token {
        /// Code system URI (e.g., "http://loinc.org").
        system: Option<String>,
        /// Code value.
        code: String,
        /// Display text (Coding.display or CodeableConcept.text) for :text modifier.
        display: Option<String>,
        /// Identifier type system (for :of-type modifier).
        identifier_type_system: Option<String>,
        /// Identifier type code (for :of-type modifier).
        identifier_type_code: Option<String>,
    },

    /// Date/DateTime value with precision tracking.
    Date {
        /// ISO 8601 formatted date/time.
        value: String,
        /// The precision of the original value.
        precision: DatePrecision,
    },

    /// Numeric value.
    Number(f64),

    /// Quantity value with optional unit.
    Quantity {
        /// Numeric value.
        value: f64,
        /// Unit string (e.g., "kg", "mmHg").
        unit: Option<String>,
        /// Unit system URI (e.g., "http://unitsofmeasure.org").
        system: Option<String>,
        /// Unit code (e.g., "kg").
        code: Option<String>,
    },

    /// Reference to another resource.
    Reference {
        /// Reference string (e.g., "Patient/123").
        reference: String,
        /// Resource type if known.
        resource_type: Option<String>,
        /// Resource ID if extractable.
        resource_id: Option<String>,
        /// Display text (`Reference.display`), for the `:text`/`:code-text`/
        /// `:text-advanced` modifiers on reference parameters.
        display: Option<String>,
    },

    /// URI value.
    Uri(String),
}

impl IndexValue {
    /// Creates a string index value.
    pub fn string(s: impl Into<String>) -> Self {
        IndexValue::String(s.into())
    }

    /// Creates a token index value with system and code.
    pub fn token(system: Option<String>, code: impl Into<String>) -> Self {
        IndexValue::Token {
            system,
            code: code.into(),
            display: None,
            identifier_type_system: None,
            identifier_type_code: None,
        }
    }

    /// Creates a token index value with code only.
    pub fn token_code(code: impl Into<String>) -> Self {
        IndexValue::Token {
            system: None,
            code: code.into(),
            display: None,
            identifier_type_system: None,
            identifier_type_code: None,
        }
    }

    /// Creates a token index value with display text for :text modifier.
    pub fn token_with_display(
        system: Option<String>,
        code: impl Into<String>,
        display: Option<String>,
    ) -> Self {
        IndexValue::Token {
            system,
            code: code.into(),
            display,
            identifier_type_system: None,
            identifier_type_code: None,
        }
    }

    /// Creates a token index value for identifiers with type information for :of-type modifier.
    pub fn identifier_with_type(
        system: Option<String>,
        value: impl Into<String>,
        type_system: Option<String>,
        type_code: Option<String>,
    ) -> Self {
        IndexValue::Token {
            system,
            code: value.into(),
            display: None,
            identifier_type_system: type_system,
            identifier_type_code: type_code,
        }
    }

    /// Creates a token index value for display-only text (e.g., CodeableConcept.text).
    /// This is used when there's only display text without a code.
    pub fn token_display_only(display: impl Into<String>) -> Self {
        IndexValue::Token {
            system: None,
            code: String::new(), // Empty code for display-only
            display: Some(display.into()),
            identifier_type_system: None,
            identifier_type_code: None,
        }
    }

    /// Creates a date index value.
    pub fn date(value: impl Into<String>) -> Self {
        let value = value.into();
        let precision = DatePrecision::from_date_string(&value);
        IndexValue::Date { value, precision }
    }

    /// Creates a number index value.
    pub fn number(value: f64) -> Self {
        IndexValue::Number(value)
    }

    /// Creates a quantity index value.
    pub fn quantity(value: f64, unit: Option<String>, system: Option<String>) -> Self {
        IndexValue::Quantity {
            value,
            unit: unit.clone(),
            system,
            code: unit,
        }
    }

    /// Creates a reference index value (no display text).
    pub fn reference(reference: impl Into<String>) -> Self {
        Self::reference_with_display(reference, None)
    }

    /// Creates a reference index value with optional `Reference.display` text.
    pub fn reference_with_display(reference: impl Into<String>, display: Option<String>) -> Self {
        let reference = reference.into();
        let (resource_type, resource_id) = parse_reference(&reference);

        IndexValue::Reference {
            reference,
            resource_type,
            resource_id,
            display,
        }
    }

    /// Creates a URI index value.
    pub fn uri(uri: impl Into<String>) -> Self {
        IndexValue::Uri(uri.into())
    }

    /// Returns the string value if this is a String variant.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            IndexValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the parameter type this value is appropriate for.
    pub fn param_type(&self) -> SearchParamType {
        match self {
            IndexValue::String(_) => SearchParamType::String,
            IndexValue::Token { .. } => SearchParamType::Token,
            IndexValue::Date { .. } => SearchParamType::Date,
            IndexValue::Number(_) => SearchParamType::Number,
            IndexValue::Quantity { .. } => SearchParamType::Quantity,
            IndexValue::Reference { .. } => SearchParamType::Reference,
            IndexValue::Uri(_) => SearchParamType::Uri,
        }
    }
}

/// Parses a reference string into (resource_type, resource_id).
fn parse_reference(reference: &str) -> (Option<String>, Option<String>) {
    // Handle URL references (e.g., "http://example.com/fhir/Patient/123")
    if reference.starts_with("http://") || reference.starts_with("https://") {
        let parts: Vec<&str> = reference.rsplitn(3, '/').collect();
        if parts.len() >= 2 {
            return (Some(parts[1].to_string()), Some(parts[0].to_string()));
        }
    }

    // Handle relative references (e.g., "Patient/123")
    let parts: Vec<&str> = reference.split('/').collect();
    if parts.len() == 2 {
        return (Some(parts[0].to_string()), Some(parts[1].to_string()));
    }

    (None, None)
}

/// Converter for transforming JSON values to index values.
pub struct ValueConverter;

impl ValueConverter {
    /// Converts a JSON value to index values based on the target parameter type.
    ///
    /// May return multiple values for arrays or complex types.
    pub fn convert(
        value: &Value,
        target_type: SearchParamType,
        param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        match value {
            Value::Array(arr) => {
                let mut results = Vec::new();
                for item in arr {
                    results.extend(Self::convert_single(item, target_type, param_name)?);
                }
                Ok(results)
            }
            _ => Self::convert_single(value, target_type, param_name),
        }
    }

    /// Converts a single (non-array) JSON value.
    fn convert_single(
        value: &Value,
        target_type: SearchParamType,
        param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        match target_type {
            SearchParamType::String => Self::convert_to_string(value, param_name),
            SearchParamType::Token => Self::convert_to_token(value, param_name),
            SearchParamType::Date => Self::convert_to_date(value, param_name),
            SearchParamType::Number => Self::convert_to_number(value, param_name),
            SearchParamType::Quantity => Self::convert_to_quantity(value, param_name),
            SearchParamType::Reference => Self::convert_to_reference(value, param_name),
            SearchParamType::Uri => Self::convert_to_uri(value, param_name),
            SearchParamType::Composite => {
                // Composite parameters are handled differently
                Ok(Vec::new())
            }
            SearchParamType::Special => {
                // Special parameters have custom handling
                Self::convert_special(value, param_name)
            }
        }
    }

    /// Converts a value to string type.
    ///
    /// Values are indexed **as written**, preserving case and accents. The
    /// case- and accent-insensitive comparisons that string search defaults to
    /// are done at query time — every backend either matches the folded
    /// companion value (`value_string_folded` / `search_params.string.folded`,
    /// written by the index writers via [`crate::search::fold_text`]) or applies
    /// a case-insensitive comparison to this raw value. Folding here instead
    /// would make `:exact` — which is defined as case- and accent-sensitive —
    /// impossible to answer, because the original spelling would be gone.
    fn convert_to_string(
        value: &Value,
        _param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        let mut results = Vec::new();

        match value {
            Value::String(s) => {
                results.push(IndexValue::string(s.as_str()));
            }
            Value::Object(obj) => {
                // HumanName
                if let Some(family) = obj.get("family").and_then(|v| v.as_str()) {
                    results.push(IndexValue::string(family));
                }
                if let Some(given) = obj.get("given").and_then(|v| v.as_array()) {
                    for g in given {
                        if let Some(s) = g.as_str() {
                            results.push(IndexValue::string(s));
                        }
                    }
                }
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    results.push(IndexValue::string(text));
                }

                // Address
                if let Some(line) = obj.get("line").and_then(|v| v.as_array()) {
                    for l in line {
                        if let Some(s) = l.as_str() {
                            results.push(IndexValue::string(s));
                        }
                    }
                }
                if let Some(city) = obj.get("city").and_then(|v| v.as_str()) {
                    results.push(IndexValue::string(city));
                }
                if let Some(state) = obj.get("state").and_then(|v| v.as_str()) {
                    results.push(IndexValue::string(state));
                }
                if let Some(postal) = obj.get("postalCode").and_then(|v| v.as_str()) {
                    results.push(IndexValue::string(postal));
                }
                if let Some(country) = obj.get("country").and_then(|v| v.as_str()) {
                    results.push(IndexValue::string(country));
                }
            }
            _ => {}
        }

        Ok(results)
    }

    /// Converts a value to token type.
    fn convert_to_token(
        value: &Value,
        _param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        let mut results = Vec::new();

        match value {
            Value::String(s) => {
                // Simple code
                results.push(IndexValue::token_code(s.clone()));
            }
            Value::Bool(b) => {
                results.push(IndexValue::token_code(b.to_string()));
            }
            Value::Object(obj) => {
                // Coding (has code and optionally system/display)
                if obj.contains_key("code") && !obj.contains_key("coding") {
                    let system = obj.get("system").and_then(|v| v.as_str()).map(String::from);
                    let code = obj.get("code").and_then(|v| v.as_str()).unwrap_or_default();
                    let display = obj
                        .get("display")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    if !code.is_empty() {
                        results.push(IndexValue::token_with_display(system, code, display));
                    }
                }

                // CodeableConcept (has coding array and optionally text)
                if let Some(coding) = obj.get("coding").and_then(|v| v.as_array()) {
                    let first_coding_row = results.len();
                    for c in coding {
                        if let Some(code) = c.get("code").and_then(|v| v.as_str()) {
                            let system = c.get("system").and_then(|v| v.as_str()).map(String::from);
                            let display =
                                c.get("display").and_then(|v| v.as_str()).map(String::from);
                            results.push(IndexValue::token_with_display(system, code, display));
                        }
                    }
                    // Also index CodeableConcept.text for :text modifier searches —
                    // but only when no coding row of this same concept can already
                    // carry it. See [`absorb_codeable_concept_text`].
                    if let Some(text) = obj.get("text").and_then(|v| v.as_str())
                        && !text.is_empty()
                        && !absorb_codeable_concept_text(&mut results[first_coding_row..], text)
                    {
                        results.push(IndexValue::token_display_only(text));
                    }
                }

                // Identifier (has value, may have system and type)
                if obj.contains_key("value")
                    && !obj.contains_key("code")
                    && !obj.contains_key("coding")
                {
                    let system = obj.get("system").and_then(|v| v.as_str()).map(String::from);
                    let value = obj
                        .get("value")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();

                    // Extract Identifier.type.coding for :of-type modifier
                    let (type_system, type_code) = obj
                        .get("type")
                        .and_then(|t| t.get("coding"))
                        .and_then(|c| c.as_array())
                        .and_then(|arr| arr.first())
                        .map(|coding| {
                            (
                                coding
                                    .get("system")
                                    .and_then(|v| v.as_str())
                                    .map(String::from),
                                coding
                                    .get("code")
                                    .and_then(|v| v.as_str())
                                    .map(String::from),
                            )
                        })
                        .unwrap_or((None, None));

                    if !value.is_empty() {
                        results.push(IndexValue::identifier_with_type(
                            system,
                            value,
                            type_system,
                            type_code,
                        ));
                    }
                }

                // ContactPoint (for email/phone searches)
                if let Some(val) = obj.get("value").and_then(|v| v.as_str()) {
                    if obj.contains_key("system")
                        && obj
                            .get("system")
                            .and_then(|v| v.as_str())
                            .map(|s| s == "phone" || s == "email")
                            .unwrap_or(false)
                    {
                        let system_type =
                            obj.get("system").and_then(|v| v.as_str()).map(String::from);
                        results.push(IndexValue::token(system_type, val));
                    }
                }
            }
            _ => {}
        }

        Ok(results)
    }

    /// Converts a value to date type.
    fn convert_to_date(
        value: &Value,
        _param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        let mut results = Vec::new();

        match value {
            Value::String(s) => {
                // ISO date string
                results.push(IndexValue::date(s.clone()));
            }
            Value::Object(obj) => {
                // Period
                if let Some(start) = obj.get("start").and_then(|v| v.as_str()) {
                    results.push(IndexValue::date(start));
                }
                if let Some(end) = obj.get("end").and_then(|v| v.as_str()) {
                    results.push(IndexValue::date(end));
                }

                // Timing (complex - just extract bounds for now)
                if let Some(repeat) = obj.get("repeat").and_then(|v| v.as_object()) {
                    if let Some(bounds_period) =
                        repeat.get("boundsPeriod").and_then(|v| v.as_object())
                    {
                        if let Some(start) = bounds_period.get("start").and_then(|v| v.as_str()) {
                            results.push(IndexValue::date(start));
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(results)
    }

    /// Converts a value to number type.
    fn convert_to_number(
        value: &Value,
        param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        match value {
            Value::Number(n) => {
                let f = n
                    .as_f64()
                    .ok_or_else(|| ExtractionError::ConversionFailed {
                        param_name: param_name.to_string(),
                        expected_type: "number".to_string(),
                        actual_value: n.to_string(),
                    })?;
                Ok(vec![IndexValue::number(f)])
            }
            Value::String(s) => {
                let f: f64 = s.parse().map_err(|_| ExtractionError::ConversionFailed {
                    param_name: param_name.to_string(),
                    expected_type: "number".to_string(),
                    actual_value: s.clone(),
                })?;
                Ok(vec![IndexValue::number(f)])
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Converts a value to quantity type.
    fn convert_to_quantity(
        value: &Value,
        _param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        let mut results = Vec::new();

        if let Value::Object(obj) = value {
            if let Some(val) = obj.get("value").and_then(|v| v.as_f64()) {
                let unit = obj.get("unit").and_then(|v| v.as_str()).map(String::from);
                let system = obj.get("system").and_then(|v| v.as_str()).map(String::from);
                let code = obj.get("code").and_then(|v| v.as_str()).map(String::from);

                results.push(IndexValue::Quantity {
                    value: val,
                    unit: unit.or_else(|| code.clone()),
                    system,
                    code,
                });
            }
        }

        Ok(results)
    }

    /// Converts a value to reference type.
    fn convert_to_reference(
        value: &Value,
        _param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        let mut results = Vec::new();

        match value {
            Value::String(s) => {
                results.push(IndexValue::reference(s.clone()));
            }
            Value::Object(obj) => {
                if let Some(reference) = obj.get("reference").and_then(|v| v.as_str()) {
                    let display = obj
                        .get("display")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    results.push(IndexValue::reference_with_display(reference, display));
                }
            }
            _ => {}
        }

        Ok(results)
    }

    /// Converts a value to URI type.
    fn convert_to_uri(
        value: &Value,
        _param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        match value {
            Value::String(s) => Ok(vec![IndexValue::uri(s.clone())]),
            _ => Ok(Vec::new()),
        }
    }

    /// Handles special parameter types.
    fn convert_special(
        value: &Value,
        param_name: &str,
    ) -> Result<Vec<IndexValue>, ExtractionError> {
        // For now, treat special parameters like their base type
        match param_name {
            "_id" => {
                if let Value::String(s) = value {
                    Ok(vec![IndexValue::token_code(s.clone())])
                } else {
                    Ok(Vec::new())
                }
            }
            "_lastUpdated" => Self::convert_to_date(value, param_name),
            "_tag" | "_security" => Self::convert_to_token(value, param_name),
            "_profile" | "_source" => Self::convert_to_uri(value, param_name),
            _ => Ok(Vec::new()),
        }
    }
}

/// Places a `CodeableConcept.text` onto one of the concept's own coding rows,
/// returning `true` when it found a home and needs no `search_index` row of its
/// own.
///
/// `text` is indexed so that `:text` (and its `:code-text` / `:text-advanced`
/// relatives) can match the human-readable label. Every backend answers those
/// modifiers by matching the **display column of any row** for the parameter and
/// returning that row's `resource_id` — Postgres `value_token_display ILIKE`,
/// SQLite `value_token_display COLLATE NOCASE LIKE`, MongoDB `$regex` with `i`,
/// Elasticsearch `match` / `query_string` / `match_phrase_prefix` over the
/// analyzed `search_params.token.display`. None of them is sensitive to *which*
/// row of a resource carries the text, and none of them reads the code-less
/// row's other columns: its `value_token_code` is `""`, which no well-formed
/// token search names. So the text only has to appear in some row's display for
/// the same `(resource, param_name)`; a row of its own is pure duplication.
///
/// That duplication is the largest single multiplier in the Postgres index. In
/// the row census for run 33029355759, `Observation | code` holds 1,380,384 rows
/// for 689,080 resources — exactly 2.00 per resource, one LOINC coding plus one
/// code-less `text` row, because Synthea writes `code.text` as a verbatim copy of
/// `coding[0].display`. `Observation | category` and `| status`, which carry no
/// `text`, sit at exactly 1.00. The same doubling runs through `combo-code`
/// (1,996,340), `component-code` (615,956), `value-concept`, `combo-value-concept`
/// and every other CodeableConcept parameter — and it is *squared* inside the
/// composites, which cross the token axis with the value axis: `code-value-concept`
/// is 4.00 rows per complete instance where 1.00 would do.
///
/// Two dispositions, in order:
///
/// 1. A coding whose `display` already equals the text (ASCII-case-insensitively,
///    which every matcher above is) makes the row redundant outright. This is the
///    Synthea shape and the one that pays.
/// 2. Otherwise the text is parked on the first coding that has no `display` of
///    its own — the column is documented as "Coding.display or
///    CodeableConcept.text", so this is the value it was meant to hold, and no
///    display is overwritten.
///
/// Anything else (every coding has a *different* display) keeps its own row: the
/// text is not reachable through any existing row and dropping it would lose a
/// `:text` match. Non-ASCII case differences also keep the row, which errs toward
/// writing one row too many rather than losing a result.
fn absorb_codeable_concept_text(codings: &mut [IndexValue], text: &str) -> bool {
    let already_carried = codings.iter().any(|value| match value {
        IndexValue::Token {
            display: Some(display),
            ..
        } => display.eq_ignore_ascii_case(text),
        _ => false,
    });
    if already_carried {
        return true;
    }

    for value in codings.iter_mut() {
        if let IndexValue::Token { display, .. } = value
            && display.is_none()
        {
            *display = Some(text.to_string());
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_index_value_creation() {
        let s = IndexValue::string("test");
        assert_eq!(s.as_string(), Some("test"));
        assert_eq!(s.param_type(), SearchParamType::String);

        let t = IndexValue::token(Some("http://loinc.org".to_string()), "1234-5");
        assert_eq!(t.param_type(), SearchParamType::Token);

        let d = IndexValue::date("2024-01-15");
        if let IndexValue::Date { precision, .. } = d {
            assert_eq!(precision, DatePrecision::Day);
        }

        let r = IndexValue::reference("Patient/123");
        if let IndexValue::Reference {
            resource_type,
            resource_id,
            ..
        } = r
        {
            assert_eq!(resource_type, Some("Patient".to_string()));
            assert_eq!(resource_id, Some("123".to_string()));
        }
    }

    #[test]
    fn test_parse_reference() {
        let (rt, id) = parse_reference("Patient/123");
        assert_eq!(rt, Some("Patient".to_string()));
        assert_eq!(id, Some("123".to_string()));

        let (rt, id) = parse_reference("http://example.com/fhir/Patient/456");
        assert_eq!(rt, Some("Patient".to_string()));
        assert_eq!(id, Some("456".to_string()));
    }

    #[test]
    fn test_convert_string() {
        let value = json!("Smith");
        let results = ValueConverter::convert(&value, SearchParamType::String, "name").unwrap();
        assert_eq!(results.len(), 1);
        // Indexed as written: `:exact` needs the original spelling.
        assert_eq!(results[0].as_string(), Some("Smith"));
    }

    #[test]
    fn test_convert_human_name() {
        let value = json!({
            "family": "Smith",
            "given": ["John", "Jane"],
            "text": "John Jane Smith"
        });
        let results = ValueConverter::convert(&value, SearchParamType::String, "name").unwrap();

        let values: Vec<&str> = results.iter().filter_map(|v| v.as_string()).collect();
        assert_eq!(
            values,
            vec!["Smith", "John", "Jane", "John Jane Smith"],
            "family, each given, and the rendered text are all indexed"
        );
    }

    /// An `Address` indexes every string part, each as written — `address` is a
    /// string parameter that matches against any of them.
    #[test]
    fn test_convert_address() {
        let value = json!({
            "line": ["1 Long Street", "Apt 2"],
            "city": "Springfield",
            "state": "IL",
            "postalCode": "62704",
            "country": "USA"
        });
        let results = ValueConverter::convert(&value, SearchParamType::String, "address").unwrap();

        let values: Vec<&str> = results.iter().filter_map(|v| v.as_string()).collect();
        assert_eq!(
            values,
            vec![
                "1 Long Street",
                "Apt 2",
                "Springfield",
                "IL",
                "62704",
                "USA"
            ],
            "every address part is indexed, preserving case"
        );
    }

    #[test]
    fn test_convert_token_coding() {
        let value = json!({
            "system": "http://loinc.org",
            "code": "12345-6"
        });
        let results = ValueConverter::convert(&value, SearchParamType::Token, "code").unwrap();
        assert_eq!(results.len(), 1);

        if let IndexValue::Token { system, code, .. } = &results[0] {
            assert_eq!(system.as_ref().unwrap(), "http://loinc.org");
            assert_eq!(code, "12345-6");
        }
    }

    /// Returns the `display` of every token value, in order.
    fn displays(values: &[IndexValue]) -> Vec<Option<&str>> {
        values
            .iter()
            .map(|v| match v {
                IndexValue::Token { display, .. } => display.as_deref(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_convert_codeable_concept() {
        let value = json!({
            "coding": [
                {"system": "http://snomed.info/sct", "code": "123"},
                {"system": "http://icd10.info", "code": "456"}
            ],
            "text": "Some condition"
        });
        let results = ValueConverter::convert(&value, SearchParamType::Token, "code").unwrap();
        // Two codings, neither with a display of its own, so the concept's text
        // rides on the first of them instead of claiming a third row.
        assert_eq!(results.len(), 2);
        assert_eq!(
            displays(&results),
            vec![Some("Some condition"), None],
            "`:text` still reaches the resource through the first coding"
        );
    }

    #[test]
    fn codeable_concept_text_equal_to_a_display_writes_no_extra_row() {
        // The Synthea shape, and the one that dominates the benchmark corpus:
        // `text` is a verbatim copy of `coding[0].display`.
        let value = json!({
            "coding": [{
                "system": "http://loinc.org",
                "code": "8302-2",
                "display": "Body Height"
            }],
            "text": "Body Height"
        });
        let results = ValueConverter::convert(&value, SearchParamType::Token, "code").unwrap();
        assert_eq!(results.len(), 1, "the text row would be a duplicate");
        assert_eq!(displays(&results), vec![Some("Body Height")]);
    }

    #[test]
    fn codeable_concept_text_matches_a_display_case_insensitively() {
        // Every `:text` matcher in the workspace is case-insensitive (ILIKE,
        // COLLATE NOCASE LIKE, $regex /i, a lowercasing analyzer), so a row that
        // differs only in case cannot change any result.
        let value = json!({
            "coding": [{"code": "123", "display": "BODY HEIGHT"}],
            "text": "Body Height"
        });
        let results = ValueConverter::convert(&value, SearchParamType::Token, "code").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(displays(&results), vec![Some("BODY HEIGHT")]);
    }

    #[test]
    fn codeable_concept_text_unlike_every_display_keeps_its_own_row() {
        // Dropping this one would lose a `:text` match: no existing row's
        // display contains "Raised blood pressure".
        let value = json!({
            "coding": [
                {"code": "123", "display": "Systolic BP"},
                {"code": "456", "display": "Diastolic BP"}
            ],
            "text": "Raised blood pressure"
        });
        let results = ValueConverter::convert(&value, SearchParamType::Token, "code").unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(
            displays(&results),
            vec![
                Some("Systolic BP"),
                Some("Diastolic BP"),
                Some("Raised blood pressure")
            ]
        );
        assert!(
            matches!(&results[2], IndexValue::Token { code, .. } if code.is_empty()),
            "the surviving text row is still the code-less display carrier"
        );
    }

    #[test]
    fn codeable_concept_text_without_any_coding_keeps_its_row() {
        // Nothing else carries it, and `code:missing=false` must stay true.
        let value = json!({ "coding": [], "text": "Free text only" });
        let results = ValueConverter::convert(&value, SearchParamType::Token, "code").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(displays(&results), vec![Some("Free text only")]);
    }

    #[test]
    fn codeable_concept_text_never_overwrites_a_display() {
        // Parking the text on a coding that already has a display would delete
        // that display from the index — a `:text` search for it would stop
        // matching. The first *display-less* coding takes it instead.
        let value = json!({
            "coding": [
                {"code": "123", "display": "Systolic BP"},
                {"code": "456"}
            ],
            "text": "Raised blood pressure"
        });
        let results = ValueConverter::convert(&value, SearchParamType::Token, "code").unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            displays(&results),
            vec![Some("Systolic BP"), Some("Raised blood pressure")]
        );
    }

    #[test]
    fn test_convert_identifier() {
        let value = json!({
            "system": "http://hospital.org/mrn",
            "value": "12345"
        });
        let results =
            ValueConverter::convert(&value, SearchParamType::Token, "identifier").unwrap();
        assert_eq!(results.len(), 1);

        if let IndexValue::Token { system, code, .. } = &results[0] {
            assert_eq!(system.as_ref().unwrap(), "http://hospital.org/mrn");
            assert_eq!(code, "12345");
        }
    }

    #[test]
    fn test_convert_date() {
        let value = json!("2024-01-15T10:30:00Z");
        let results = ValueConverter::convert(&value, SearchParamType::Date, "date").unwrap();
        assert_eq!(results.len(), 1);

        if let IndexValue::Date { value, precision } = &results[0] {
            assert!(value.starts_with("2024-01-15"));
            assert_eq!(*precision, DatePrecision::Second);
        }
    }

    #[test]
    fn test_convert_period() {
        let value = json!({
            "start": "2024-01-01",
            "end": "2024-01-31"
        });
        let results = ValueConverter::convert(&value, SearchParamType::Date, "date").unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_convert_quantity() {
        let value = json!({
            "value": 120.5,
            "unit": "mmHg",
            "system": "http://unitsofmeasure.org",
            "code": "mm[Hg]"
        });
        let results =
            ValueConverter::convert(&value, SearchParamType::Quantity, "value-quantity").unwrap();
        assert_eq!(results.len(), 1);

        if let IndexValue::Quantity {
            value,
            unit,
            system,
            code,
        } = &results[0]
        {
            assert!((value - 120.5).abs() < f64::EPSILON);
            assert_eq!(unit.as_ref().unwrap(), "mmHg");
            assert_eq!(system.as_ref().unwrap(), "http://unitsofmeasure.org");
            assert_eq!(code.as_ref().unwrap(), "mm[Hg]");
        }
    }

    #[test]
    fn test_convert_reference_object() {
        let value = json!({
            "reference": "Patient/123",
            "display": "John Smith"
        });
        let results =
            ValueConverter::convert(&value, SearchParamType::Reference, "subject").unwrap();
        assert_eq!(results.len(), 1);

        if let IndexValue::Reference {
            reference,
            resource_type,
            resource_id,
            display,
        } = &results[0]
        {
            assert_eq!(reference, "Patient/123");
            assert_eq!(resource_type.as_ref().unwrap(), "Patient");
            assert_eq!(resource_id.as_ref().unwrap(), "123");
            assert_eq!(display.as_ref().unwrap(), "John Smith");
        }
    }

    #[test]
    fn test_convert_array() {
        let value = json!(["one", "two", "three"]);
        let results = ValueConverter::convert(&value, SearchParamType::String, "name").unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_convert_codeable_concept_with_display() {
        // This is what would be extracted from Observation.code
        let value = json!({
            "coding": [
                {
                    "system": "http://loinc.org",
                    "code": "8867-4",
                    "display": "Heart rate"
                }
            ]
        });
        let results = ValueConverter::convert(&value, SearchParamType::Token, "code").unwrap();

        // Should have at least the coding entry
        assert!(!results.is_empty(), "Should have at least one result");

        // Find the token with code 8867-4
        let heart_rate = results
            .iter()
            .find(|v| matches!(v, IndexValue::Token { code, .. } if code == "8867-4"));
        assert!(heart_rate.is_some(), "Should have token with code 8867-4");

        // Verify display is populated
        if let Some(IndexValue::Token {
            system,
            code,
            display,
            ..
        }) = heart_rate
        {
            assert_eq!(system.as_ref().unwrap(), "http://loinc.org");
            assert_eq!(code, "8867-4");
            assert_eq!(
                display.as_ref().unwrap(),
                "Heart rate",
                "Display text should be populated"
            );
        }
    }

    #[test]
    fn test_convert_identifier_with_type() {
        // Identifier with type for :of-type modifier
        let value = json!({
            "type": {
                "coding": [
                    {
                        "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                        "code": "MR"
                    }
                ]
            },
            "system": "http://hospital.org/mrn",
            "value": "MRN12345"
        });
        let results =
            ValueConverter::convert(&value, SearchParamType::Token, "identifier").unwrap();

        assert_eq!(results.len(), 1);

        if let IndexValue::Token {
            system,
            code,
            identifier_type_system,
            identifier_type_code,
            ..
        } = &results[0]
        {
            assert_eq!(system.as_ref().unwrap(), "http://hospital.org/mrn");
            assert_eq!(code, "MRN12345");
            assert_eq!(
                identifier_type_system.as_ref().unwrap(),
                "http://terminology.hl7.org/CodeSystem/v2-0203",
                "Identifier type system should be populated"
            );
            assert_eq!(
                identifier_type_code.as_ref().unwrap(),
                "MR",
                "Identifier type code should be populated"
            );
        } else {
            panic!("Expected Token variant");
        }
    }
}
