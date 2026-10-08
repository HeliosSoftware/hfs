//! SQLite search index writer implementation.

use chrono::{DateTime, NaiveDateTime, Utc};

use crate::search::{
    StorageResolution, converters::IndexValue, extractor::ExtractedValue, indexed_end,
    indexed_range, parse_stored_date,
};
use crate::types::DatePrecision;

/// The text form every stored or bound SQLite date instant takes on the range
/// path (#1391): what `strftime('%Y-%m-%d %H:%M:%f', …)` produces, UTC, cut to
/// the millisecond. Fixed-width, so it compares as text in instant order.
pub(crate) const SQLITE_INSTANT_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.3f";

/// `instant` as [`SQLITE_INSTANT_FORMAT`] text.
pub(crate) fn sqlite_instant(instant: DateTime<Utc>) -> String {
    instant.format(SQLITE_INSTANT_FORMAT).to_string()
}

/// Reads the start of a stored date the way SQLite's `datetime()` does, which
/// is what the search SQL compares `value_date` through: the FHIR grammar
/// first, then an RFC 3339 instant, then a local date-time read as UTC — the
/// shape `normalize_date_for_sqlite` pads a date-only value to
/// (`2020-01-01T00:00:00`).
pub(crate) fn stored_date_start(text: &str) -> Option<DateTime<Utc>> {
    if let Some(parsed) = parse_stored_date(text) {
        return Some(parsed.start);
    }
    DateTime::parse_from_rfc3339(text)
        .map(|instant| instant.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|local| local.and_utc())
        })
}

/// The `value_date_end` a date index value is stored with (#1391): the end of
/// its range `[value_date, value_date_end)` as [`SQLITE_INSTANT_FORMAT`] text.
/// A point ends one unit of its precision later, a `Period` at the end of its
/// `end`, and an open `Period` at the end of the supported years.
///
/// The value may already be padded by `normalize_date_for_sqlite` (a date-only
/// `2020` arrives as `2020-01-01T00:00:00`), and that text reads as a
/// second-precision instant, so the precision the value carries decides the
/// end whenever it differs from the text's own; only when the two agree does
/// the exact range of the text apply (it knows a fraction's digit count,
/// where the carried precision only says "millisecond"). `None` for anything
/// but a date, and for a date whose start or end cannot be read; such a row
/// matches no comparison on its end.
pub(crate) fn stored_date_end(value: &IndexValue) -> Option<String> {
    let IndexValue::Date {
        value: text,
        precision,
        end,
    } = value
    else {
        return None;
    };
    let exact = (*precision == DatePrecision::from_date_string(text))
        .then(|| indexed_range(value, StorageResolution::Millis))
        .flatten();
    let end = match exact {
        Some((_, end)) => end,
        None => {
            let start = stored_date_start(text)?;
            indexed_end(start, *precision, end, StorageResolution::Millis)?
        }
    };
    Some(sqlite_instant(end))
}

/// SQLite implementation of SearchIndexWriter.
pub struct SqliteSearchIndexWriter;

impl SqliteSearchIndexWriter {
    /// Zero-based position of the `resource_key` value in the parameter vectors
    /// produced by [`Self::to_sql_params`] (both the base and contained shapes
    /// place it here, before the trailing contained columns). The write path
    /// patches this slot with the resolved `resources.rowid` — `to_sql_params`
    /// runs on a thread pool with no connection, so it emits a placeholder here.
    pub const RESOURCE_KEY_PARAM_IX: usize = 24;

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
            value_reference_display,
            value_quantity_canonical_value, value_quantity_canonical_unit,
            value_string_folded,
            resource_key,
            value_date_end, composite_slot
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, ?7, ?8, ?9,
            ?10, ?11,
            ?12, ?13, ?14, ?15,
            ?16, ?17, ?18,
            ?19, ?20,
            ?21,
            ?22, ?23,
            ?24,
            ?25,
            ?26, ?27
        )
        "#
    }

    /// INSERT SQL for a contained index entry: the same 27 base columns as
    /// [`Self::insert_sql`] plus `is_contained`, `contained_type`, and
    /// `contained_local_id` (`?28..?30`). The base columns' `resource_type` /
    /// `resource_id` / `resource_key` identify the *container*; `contained_type`
    /// is the nested resource's type. Bind the base params from
    /// [`Self::to_sql_params`] followed by `1`, the contained type, and the
    /// contained local id.
    pub fn insert_contained_sql() -> &'static str {
        r#"
        INSERT INTO search_index (
            tenant_id, resource_type, resource_id, param_name, param_url,
            value_string, value_token_system, value_token_code, value_token_display,
            value_date, value_date_precision,
            value_number, value_quantity_value, value_quantity_unit, value_quantity_system,
            value_reference, value_uri, composite_group,
            value_identifier_type_system, value_identifier_type_code,
            value_reference_display,
            value_quantity_canonical_value, value_quantity_canonical_unit,
            value_string_folded,
            resource_key,
            value_date_end, composite_slot,
            is_contained, contained_type, contained_local_id
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, ?7, ?8, ?9,
            ?10, ?11,
            ?12, ?13, ?14, ?15,
            ?16, ?17, ?18,
            ?19, ?20,
            ?21,
            ?22, ?23,
            ?24,
            ?25,
            ?26, ?27,
            ?28, ?29, ?30
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

    /// Multi-row variant of [`Self::insert_sql`]: one INSERT carrying eight
    /// rows (8 x 27 positional parameters). Bulk indexing executes this once
    /// per chunk instead of stepping the single-row statement eight times.
    pub fn insert_sql_rows8() -> &'static str {
        static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        SQL.get_or_init(|| {
            let base = Self::insert_sql();
            let cols = &base[..base.find("VALUES").expect("insert_sql has VALUES")];
            let group = format!("({})", ["?"; Self::COLUMNS].join(", "));
            format!("{cols}VALUES {}", vec![group; 8].join(", "))
        })
    }

    /// Number of base columns [`Self::insert_sql`] writes, and so the length of
    /// every vector [`Self::to_sql_params`] returns.
    pub const COLUMNS: usize = 27;

    /// Converts an ExtractedValue to SQL parameters.
    ///
    /// Returns a tuple of (column_values) where each value corresponds to a column.
    pub fn to_sql_params(
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource_key: i64,
        extracted: &ExtractedValue,
    ) -> Vec<SqlValue> {
        let mut params = Self::base_sql_params(
            tenant_id,
            resource_type,
            resource_id,
            resource_key,
            extracted,
        );
        // Preserve the existing `resource_key` and date-end bind positions.
        params.push(SqlValue::OptString(stored_date_end(&extracted.value)));
        params.push(SqlValue::OptInt(extracted.composite_slot.map(i64::from)));
        params
    }

    /// Every base column of [`Self::to_sql_params`] up to `resource_key`.
    fn base_sql_params(
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        resource_key: i64,
        extracted: &ExtractedValue,
    ) -> Vec<SqlValue> {
        let mut params = vec![
            SqlValue::String(tenant_id.to_string()),
            SqlValue::String(resource_type.to_string()),
            SqlValue::String(resource_id.to_string()),
            SqlValue::String(extracted.param_name.clone()),
            // `param_url` is not written. No SQLite read path consults it —
            // every lookup is by `param_name` — and at ~57 bytes on every one
            // of a resource's ~20 rows it was 11% of a bulk-loaded database.
            // The column stays so older rows and the schema are untouched.
            SqlValue::Null,
        ];

        // `value_reference_display` and the UCUM-canonical quantity columns are
        // appended as trailing columns; capture them here so the common tail
        // (and the Token early-return) can emit them.
        let mut reference_display: Option<String> = None;
        let mut canonical_value: Option<f64> = None;
        let mut canonical_unit: Option<String> = None;
        let mut string_folded: Option<String> = None;

        // Add value columns based on the IndexValue type
        match &extracted.value {
            IndexValue::String(s) => {
                string_folded = Some(crate::search::fold_text(s));
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
                params.push(SqlValue::Null); // value_quantity_canonical_value
                params.push(SqlValue::Null); // value_quantity_canonical_unit
                params.push(SqlValue::Null); // value_string_folded
                params.push(SqlValue::Int(resource_key)); // resource_key
                return params;
            }
            IndexValue::Date {
                value, precision, ..
            } => {
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
                code,
            } => {
                // Canonicalize using the UCUM code when present, else the unit
                // display string. Stored alongside the raw value so quantity
                // search can match equivalent units (e.g. g ⇄ mg).
                let ucum = code.as_deref().or(unit.as_deref());
                if let Some(u) = ucum {
                    if let Some((cv, cu)) = helios_fhirpath::ucum::canonicalize_quantity(*value, u)
                    {
                        canonical_value = Some(cv);
                        canonical_unit = Some(cu);
                    }
                }
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
        params.push(match canonical_value {
            Some(v) => SqlValue::Float(v),
            None => SqlValue::Null,
        }); // value_quantity_canonical_value
        params.push(SqlValue::OptString(canonical_unit)); // value_quantity_canonical_unit
        params.push(SqlValue::OptString(string_folded)); // value_string_folded
        params.push(SqlValue::Int(resource_key)); // resource_key

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
    use crate::search::DateEnd;
    use crate::types::{DatePrecision, SearchParamType};

    #[test]
    fn composite_slot_survives_single_batch_and_contained_inserts() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::backends::sqlite::schema::initialize_schema(&conn).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        let extracted = ExtractedValue {
            param_name: "pair".into(),
            param_url: "http://example.org/pair".into(),
            param_type: SearchParamType::Composite,
            value: IndexValue::Number(4.0),
            composite_group: Some(7),
            composite_slot: Some(2),
            composite_arity: Some(2),
        };
        let rows: Vec<Vec<SqlValue>> = (0..9)
            .map(|i| {
                SqliteSearchIndexWriter::to_sql_params(
                    "t",
                    "Observation",
                    &format!("o{i}"),
                    i + 1,
                    &extracted,
                )
            })
            .collect();
        assert_eq!(rows[0].len(), 27);
        assert!(matches!(
            rows[0][SqliteSearchIndexWriter::RESOURCE_KEY_PARAM_IX],
            SqlValue::Int(1)
        ));
        assert!(rows[0][25].is_null());
        assert!(matches!(rows[0][26], SqlValue::OptInt(Some(2))));
        let bind = |value: &SqlValue| -> rusqlite::types::Value {
            match value {
                SqlValue::String(s) | SqlValue::OptString(Some(s)) => s.clone().into(),
                SqlValue::Int(v) | SqlValue::OptInt(Some(v)) => (*v).into(),
                SqlValue::Float(v) => (*v).into(),
                _ => rusqlite::types::Value::Null,
            }
        };
        conn.execute(
            SqliteSearchIndexWriter::insert_sql_rows8(),
            rusqlite::params_from_iter(rows[..8].iter().flatten().map(bind)),
        )
        .unwrap();
        conn.execute(
            SqliteSearchIndexWriter::insert_sql(),
            rusqlite::params_from_iter(rows[8].iter().map(bind)),
        )
        .unwrap();
        let mut contained = rows[0].clone();
        contained.extend([
            SqlValue::Int(1),
            SqlValue::String("Patient".into()),
            SqlValue::String("local".into()),
        ]);
        assert_eq!(contained.len(), 30);
        conn.execute(
            SqliteSearchIndexWriter::insert_contained_sql(),
            rusqlite::params_from_iter(contained.iter().map(bind)),
        )
        .unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM search_index WHERE composite_slot = 2 AND composite_group = 7 AND value_number = 4", [], |row| row.get::<_, i64>(0)).unwrap(), 10);
        let stored: (i64, String, String, i64) = conn.query_row("SELECT resource_key, contained_type, contained_local_id, composite_slot FROM search_index WHERE is_contained = 1", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap();
        assert_eq!(stored, (1, "Patient".into(), "local".into(), 2));
    }

    #[test]
    fn test_string_value_params() {
        let extracted = ExtractedValue {
            param_name: "name".to_string(),
            param_url: "http://hl7.org/fhir/SearchParameter/Patient-name".to_string(),
            param_type: SearchParamType::String,
            value: IndexValue::String("Smith".to_string()),
            composite_group: None,
            composite_slot: None,
            composite_arity: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Patient", "123", 1, &extracted);

        assert_eq!(params.len(), SqliteSearchIndexWriter::COLUMNS);
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
            composite_slot: None,
            composite_arity: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Patient", "123", 1, &extracted);

        assert_eq!(params.len(), SqliteSearchIndexWriter::COLUMNS);
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
            composite_slot: None,
            composite_arity: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Observation", "123", 1, &extracted);

        assert_eq!(params.len(), SqliteSearchIndexWriter::COLUMNS);
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
            composite_slot: None,
            composite_arity: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Patient", "123", 1, &extracted);

        assert_eq!(params.len(), SqliteSearchIndexWriter::COLUMNS);
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
                end: DateEnd::Precision,
            },
            composite_group: None,
            composite_slot: None,
            composite_arity: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Patient", "123", 1, &extracted);

        assert!(matches!(&params[9], SqlValue::String(s) if s == "2024-01-15")); // Updated index for new column
        assert_eq!(params.len(), SqliteSearchIndexWriter::COLUMNS);
        // value_date_end, after resource_key: the end of the day.
        assert!(
            matches!(&params[25], SqlValue::OptString(Some(s)) if s == "2024-01-16 00:00:00.000")
        );
    }

    fn date_value(value: &str, precision: DatePrecision, end: DateEnd) -> IndexValue {
        IndexValue::Date {
            value: value.to_string(),
            precision,
            end,
        }
    }

    /// #1391: `value_date_end` is the end of the stored range, whether the
    /// value arrives as the resource wrote it or padded for SQLite.
    #[test]
    fn stored_date_end_is_the_end_of_the_range() {
        for (value, precision, end, expected) in [
            // Padded by `normalize_date_for_sqlite`: the precision decides.
            (
                "2020-01-01T00:00:00",
                DatePrecision::Year,
                DateEnd::Precision,
                "2021-01-01 00:00:00.000",
            ),
            (
                "2020-06-01T00:00:00",
                DatePrecision::Month,
                DateEnd::Precision,
                "2020-07-01 00:00:00.000",
            ),
            // An instant folds to UTC and ends one second later.
            (
                "2020-06-15T10:00:00+02:00",
                DatePrecision::Second,
                DateEnd::Precision,
                "2020-06-15 08:00:01.000",
            ),
            // A Period ends at the end of its own end.
            (
                "2020-01-15T00:00:00",
                DatePrecision::Day,
                DateEnd::At("2020-06".to_string()),
                "2020-07-01 00:00:00.000",
            ),
            // Open above: the last millisecond of the supported years.
            (
                "2020-01-15T00:00:00",
                DatePrecision::Day,
                DateEnd::Open,
                "9999-12-31 23:59:59.999",
            ),
            // Open below: the start is the supported minimum.
            (
                crate::search::OPEN_START,
                DatePrecision::Month,
                DateEnd::At("2020-06".to_string()),
                "2020-07-01 00:00:00.000",
            ),
        ] {
            assert_eq!(
                stored_date_end(&date_value(value, precision, end.clone())).as_deref(),
                Some(expected),
                "{value} {precision:?} {end:?}"
            );
        }
        assert_eq!(
            stored_date_end(&date_value(
                "not-a-date",
                DatePrecision::Day,
                DateEnd::Precision
            )),
            None
        );
        assert_eq!(stored_date_end(&IndexValue::String("2020".into())), None);
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
            composite_slot: None,
            composite_arity: None,
        };

        let params =
            SqliteSearchIndexWriter::to_sql_params("tenant1", "Observation", "456", 1, &extracted);

        assert!(matches!(&params[12], SqlValue::Float(f) if (*f - 5.4).abs() < 0.001)); // Updated index
        assert!(matches!(&params[13], SqlValue::OptString(Some(s)) if s == "mg")); // Updated index
    }
}
