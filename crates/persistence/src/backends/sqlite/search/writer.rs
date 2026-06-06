//! SQLite search index writer implementation.

use crate::search::{converters::IndexValue, extractor::ExtractedValue};

/// SQLite implementation of SearchIndexWriter.
pub struct SqliteSearchIndexWriter;

impl SqliteSearchIndexWriter {
    /// Creates a new SQLite search index writer.
    pub fn new() -> Self {
        Self
    }

    /// Generates the INSERT SQL for a single index entry.
    pub fn insert_sql() -> &'static str {
        r#"
        INSERT INTO search_index (
            tenant_id, resource_type, resource_id, param_name, param_url,
            value_string, value_token_system, value_token_code, value_token_display,
            value_date, value_date_precision,
            value_number, value_quantity_value, value_quantity_unit, value_quantity_system,
            value_reference, value_uri, composite_group,
            value_identifier_type_system, value_identifier_type_code,
            value_reference_display
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, ?7, ?8, ?9,
            ?10, ?11,
            ?12, ?13, ?14, ?15,
            ?16, ?17, ?18,
            ?19, ?20,
            ?21
        )
        "#
    }

    /// Generates the DELETE SQL for clearing a resource's index entries.
    pub fn delete_sql() -> &'static str {
        "DELETE FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3"
    }

    /// Generates the DELETE SQL for a specific parameter.
    pub fn delete_param_sql() -> &'static str {
        "DELETE FROM search_index WHERE tenant_id = ?1 AND resource_type = ?2 AND resource_id = ?3 AND param_name = ?4"
    }

    /// Converts an ExtractedValue to SQL parameters.
    ///
    /// Returns a tuple of (column_values) where each value corresponds to a column.
    pub fn to_sql_params(
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        extracted: &ExtractedValue,
    ) -> Vec<SqlValue> {
        let mut params = vec![
            SqlValue::String(tenant_id.to_string()),
            SqlValue::String(resource_type.to_string()),
            SqlValue::String(resource_id.to_string()),
            SqlValue::String(extracted.param_name.clone()),
            SqlValue::String(extracted.param_url.clone()),
        ];

        // `value_reference_display` is appended as the final column; capture it
        // here so the common tail (and the Token early-return) can emit it.
        let mut reference_display: Option<String> = None;

        // Add value columns based on the IndexValue type
        match &extracted.value {
            IndexValue::String(s) => {
                params.push(SqlValue::OptString(Some(s.clone()))); // value_string
                params.push(SqlValue::Null); // value_token_system
                params.push(SqlValue::Null); // value_token_code
                params.push(SqlValue::Null); // value_token_display
                params.push(SqlValue::Null); // value_date
                params.push(SqlValue::Null); // value_date_precision
                params.push(SqlValue::Null); // value_number
                params.push(SqlValue::Null); // value_quantity_value
                params.push(SqlValue::Null); // value_quantity_unit
                params.push(SqlValue::Null); // value_quantity_system
                params.push(SqlValue::Null); // value_reference
                params.push(SqlValue::Null); // value_uri
            }
            IndexValue::Token {
                system,
                code,
                display,
                identifier_type_system,
                identifier_type_code,
            } => {
                params.push(SqlValue::Null); // value_string
                params.push(SqlValue::OptString(system.clone())); // value_token_system
                params.push(SqlValue::String(code.clone())); // value_token_code
                params.push(SqlValue::OptString(display.clone())); // value_token_display
                params.push(SqlValue::Null); // value_date
                params.push(SqlValue::Null); // value_date_precision
                params.push(SqlValue::Null); // value_number
                params.push(SqlValue::Null); // value_quantity_value
                params.push(SqlValue::Null); // value_quantity_unit
                params.push(SqlValue::Null); // value_quantity_system
                params.push(SqlValue::Null); // value_reference
                params.push(SqlValue::Null); // value_uri
                params.push(SqlValue::OptInt(
                    extracted.composite_group.map(|g| g as i64),
                )); // composite_group
                params.push(SqlValue::OptString(identifier_type_system.clone())); // value_identifier_type_system
                params.push(SqlValue::OptString(identifier_type_code.clone())); // value_identifier_type_code
                params.push(SqlValue::Null); // value_reference_display
                return params;
            }
            IndexValue::Date { value, precision } => {
                params.push(SqlValue::Null); // value_string
                params.push(SqlValue::Null); // value_token_system
                params.push(SqlValue::Null); // value_token_code
                params.push(SqlValue::Null); // value_token_display
                params.push(SqlValue::String(value.clone())); // value_date
                params.push(SqlValue::String(precision.to_string())); // value_date_precision
                params.push(SqlValue::Null); // value_number
                params.push(SqlValue::Null); // value_quantity_value
                params.push(SqlValue::Null); // value_quantity_unit
                params.push(SqlValue::Null); // value_quantity_system
                params.push(SqlValue::Null); // value_reference
                params.push(SqlValue::Null); // value_uri
            }
            IndexValue::Number(n) => {
                params.push(SqlValue::Null); // value_string
                params.push(SqlValue::Null); // value_token_system
                params.push(SqlValue::Null); // value_token_code
                params.push(SqlValue::Null); // value_token_display
                params.push(SqlValue::Null); // value_date
                params.push(SqlValue::Null); // value_date_precision
                params.push(SqlValue::Float(*n)); // value_number
                params.push(SqlValue::Null); // value_quantity_value
                params.push(SqlValue::Null); // value_quantity_unit
                params.push(SqlValue::Null); // value_quantity_system
                params.push(SqlValue::Null); // value_reference
                params.push(SqlValue::Null); // value_uri
            }
            IndexValue::Quantity {
                value,
                unit,
                system,
                code: _,
            } => {
                params.push(SqlValue::Null); // value_string
                params.push(SqlValue::Null); // value_token_system
                params.push(SqlValue::Null); // value_token_code
                params.push(SqlValue::Null); // value_token_display
                params.push(SqlValue::Null); // value_date
                params.push(SqlValue::Null); // value_date_precision
                params.push(SqlValue::Null); // value_number
                params.push(SqlValue::Float(*value)); // value_quantity_value
                params.push(SqlValue::OptString(unit.clone())); // value_quantity_unit
                params.push(SqlValue::OptString(system.clone())); // value_quantity_system
                params.push(SqlValue::Null); // value_reference
                params.push(SqlValue::Null); // value_uri
            }
            IndexValue::Reference {
                reference,
                resource_type: _,
                resource_id: _,
                display,
            } => {
                reference_display = display.clone();
                params.push(SqlValue::Null); // value_string
                params.push(SqlValue::Null); // value_token_system
                params.push(SqlValue::Null); // value_token_code
                params.push(SqlValue::Null); // value_token_display
                params.push(SqlValue::Null); // value_date
                params.push(SqlValue::Null); // value_date_precision
                params.push(SqlValue::Null); // value_number
                params.push(SqlValue::Null); // value_quantity_value
                params.push(SqlValue::Null); // value_quantity_unit
                params.push(SqlValue::Null); // value_quantity_system
                params.push(SqlValue::String(reference.clone())); // value_reference
                params.push(SqlValue::Null); // value_uri
            }
            IndexValue::Uri(uri) => {
                params.push(SqlValue::Null); // value_string
                params.push(SqlValue::Null); // value_token_system
                params.push(SqlValue::Null); // value_token_code
                params.push(SqlValue::Null); // value_token_display
                params.push(SqlValue::Null); // value_date
                params.push(SqlValue::Null); // value_date_precision
                params.push(SqlValue::Null); // value_number
                params.push(SqlValue::Null); // value_quantity_value
                params.push(SqlValue::Null); // value_quantity_unit
                params.push(SqlValue::Null); // value_quantity_system
                params.push(SqlValue::Null); // value_reference
                params.push(SqlValue::String(uri.clone())); // value_uri
            }
        }

        // Add remaining columns for non-Token types
        params.push(SqlValue::OptInt(
            extracted.composite_group.map(|g| g as i64),
        )); // composite_group
        params.push(SqlValue::Null); // value_identifier_type_system
        params.push(SqlValue::Null); // value_identifier_type_code
        params.push(SqlValue::OptString(reference_display)); // value_reference_display

        params
    }
}

impl Default for SqliteSearchIndexWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// SQL value type for parameterized queries.
#[derive(Debug, Clone)]
pub enum SqlValue {
    /// String value.
    String(String),
    /// Optional string value.
    OptString(Option<String>),
    /// Integer value.
    Int(i64),
    /// Optional integer value.
    OptInt(Option<i64>),
    /// Float value.
    Float(f64),
    /// Null value.
    Null,
}

impl SqlValue {
    /// Returns true if this is a null value.
    pub fn is_null(&self) -> bool {
        matches!(
            self,
            SqlValue::Null | SqlValue::OptString(None) | SqlValue::OptInt(None)
        )
    }

    /// Converts to a rusqlite-compatible type.
    pub fn as_sql_string(&self) -> Option<String> {
        match self {
            SqlValue::String(s) => Some(s.clone()),
            SqlValue::OptString(Some(s)) => Some(s.clone()),
            SqlValue::Int(i) => Some(i.to_string()),
            SqlValue::OptInt(Some(i)) => Some(i.to_string()),
            SqlValue::Float(f) => Some(f.to_string()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DatePrecision, SearchParamType};

    #[test]
    fn test_string_value_params() {
        let extracted = ExtractedValue {
            param_name: "name".to_string(),
            param_url: "http://hl7.org/fhir/SearchParameter/Patient-name".to_string(),
            param_type: SearchParamType::String,
            value: IndexValue::String("Smith".to_string()),
            composite_group: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Patient", "123", &extracted);

        assert_eq!(params.len(), 21); // Updated for new columns
        assert!(matches!(&params[0], SqlValue::String(s) if s == "tenant1"));
        assert!(matches!(&params[5], SqlValue::OptString(Some(s)) if s == "Smith"));
    }

    #[test]
    fn test_token_value_params() {
        let extracted = ExtractedValue {
            param_name: "identifier".to_string(),
            param_url: "http://hl7.org/fhir/SearchParameter/Patient-identifier".to_string(),
            param_type: SearchParamType::Token,
            value: IndexValue::Token {
                system: Some("http://example.org".to_string()),
                code: "12345".to_string(),
                display: None,
                identifier_type_system: None,
                identifier_type_code: None,
            },
            composite_group: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Patient", "123", &extracted);

        assert_eq!(params.len(), 21); // Updated for new columns
        assert!(matches!(&params[6], SqlValue::OptString(Some(s)) if s == "http://example.org"));
        assert!(matches!(&params[7], SqlValue::String(s) if s == "12345"));
    }

    #[test]
    fn test_token_with_display_params() {
        let extracted = ExtractedValue {
            param_name: "code".to_string(),
            param_url: "http://hl7.org/fhir/SearchParameter/clinical-code".to_string(),
            param_type: SearchParamType::Token,
            value: IndexValue::Token {
                system: Some("http://loinc.org".to_string()),
                code: "12345-6".to_string(),
                display: Some("Test Display".to_string()),
                identifier_type_system: None,
                identifier_type_code: None,
            },
            composite_group: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Observation", "123", &extracted);

        assert_eq!(params.len(), 21);
        assert!(matches!(&params[8], SqlValue::OptString(Some(s)) if s == "Test Display")); // value_token_display
    }

    #[test]
    fn test_identifier_with_type_params() {
        let extracted = ExtractedValue {
            param_name: "identifier".to_string(),
            param_url: "http://hl7.org/fhir/SearchParameter/Patient-identifier".to_string(),
            param_type: SearchParamType::Token,
            value: IndexValue::Token {
                system: Some("http://hospital.org/mrn".to_string()),
                code: "MRN12345".to_string(),
                display: None,
                identifier_type_system: Some(
                    "http://terminology.hl7.org/CodeSystem/v2-0203".to_string(),
                ),
                identifier_type_code: Some("MR".to_string()),
            },
            composite_group: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Patient", "123", &extracted);

        assert_eq!(params.len(), 21);
        // value_identifier_type_system is at index 18
        assert!(
            matches!(&params[18], SqlValue::OptString(Some(s)) if s == "http://terminology.hl7.org/CodeSystem/v2-0203")
        );
        // value_identifier_type_code is at index 19
        assert!(matches!(&params[19], SqlValue::OptString(Some(s)) if s == "MR"));
    }

    #[test]
    fn test_date_value_params() {
        let extracted = ExtractedValue {
            param_name: "birthdate".to_string(),
            param_url: "http://hl7.org/fhir/SearchParameter/Patient-birthdate".to_string(),
            param_type: SearchParamType::Date,
            value: IndexValue::Date {
                value: "2024-01-15".to_string(),
                precision: DatePrecision::Day,
            },
            composite_group: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Patient", "123", &extracted);

        assert!(matches!(&params[9], SqlValue::String(s) if s == "2024-01-15")); // Updated index for new column
    }

    #[test]
    fn test_quantity_value_params() {
        let extracted = ExtractedValue {
            param_name: "value-quantity".to_string(),
            param_url: "http://hl7.org/fhir/SearchParameter/Observation-value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            value: IndexValue::Quantity {
                value: 5.4,
                unit: Some("mg".to_string()),
                system: Some("http://unitsofmeasure.org".to_string()),
                code: Some("mg".to_string()),
            },
            composite_group: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Observation", "456", &extracted);

        assert!(matches!(&params[12], SqlValue::Float(f) if (*f - 5.4).abs() < 0.001)); // Updated index
        assert!(matches!(&params[13], SqlValue::OptString(Some(s)) if s == "mg")); // Updated index
    }
}
