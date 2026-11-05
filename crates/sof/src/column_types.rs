//! Output column types for ViewDefinitions.
//!
//! SQL-on-FHIR 3.0.0-ballot defines default mappings from FHIR types
//! (`viewdef-6`) and FHIRPath types (`viewdef-7`) to ISO/IEC 9075 SQL types, and
//! a column `tag` named `ansi/type` that overrides the default. This module is
//! the single place those rules live; every emitter (Parquet/Arrow, the
//! in-memory SQLite engine, the in-database compilers) resolves its column types
//! through [`resolve_columns`] so they cannot drift apart.
//!
//! The functions work on the raw ViewDefinition JSON because that is the form
//! every caller already has.
//!
//! Policy for the `ansi/type` tag (behaviour is undefined by the spec for an
//! unknown or incompatible value): such a value is **rejected** with
//! [`SofError::InvalidViewDefinition`] naming the column, rather than ignored,
//! so the caller never silently receives a different column type than asked.

use crate::SofError;
use serde_json::Value;
use std::collections::HashSet;

/// Name of the column tag that overrides the default SQL type.
pub const ANSI_TYPE_TAG: &str = "ansi/type";

/// An ISO/IEC 9075 type that an output column can be rendered as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnsiType {
    /// `BOOLEAN`
    Boolean,
    /// `INT`
    Int,
    /// `BIGINT`
    BigInt,
    /// `CHARACTER VARYING`
    Varchar,
    /// `BINARY`
    Binary,
    /// `TIMESTAMP WITH TIME ZONE`
    TimestampTz,
    /// `DATE` (reachable only through the `ansi/type` tag)
    Date,
    /// `TIME` (reachable only through the `ansi/type` tag)
    Time,
    /// `TIMESTAMP` (reachable only through the `ansi/type` tag)
    Timestamp,
    /// `DOUBLE PRECISION` (reachable only through the `ansi/type` tag)
    Double,
}

impl AnsiType {
    /// The canonical SQL spelling of the type.
    pub fn sql_name(self) -> &'static str {
        match self {
            AnsiType::Boolean => "BOOLEAN",
            AnsiType::Int => "INT",
            AnsiType::BigInt => "BIGINT",
            AnsiType::Varchar => "CHARACTER VARYING",
            AnsiType::Binary => "BINARY",
            AnsiType::TimestampTz => "TIMESTAMP WITH TIME ZONE",
            AnsiType::Date => "DATE",
            AnsiType::Time => "TIME",
            AnsiType::Timestamp => "TIMESTAMP",
            AnsiType::Double => "DOUBLE PRECISION",
        }
    }

    /// Parses the value of an `ansi/type` tag. Case-insensitive; whitespace is
    /// collapsed. Returns `None` for an unrecognised name.
    pub fn parse(value: &str) -> Option<AnsiType> {
        let normalized = value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase();
        Some(match normalized.as_str() {
            "BOOLEAN" => AnsiType::Boolean,
            "INT" | "INTEGER" => AnsiType::Int,
            "BIGINT" => AnsiType::BigInt,
            "VARCHAR" | "CHARACTER VARYING" | "CHAR VARYING" => AnsiType::Varchar,
            "BINARY" | "VARBINARY" | "BINARY VARYING" => AnsiType::Binary,
            "TIMESTAMP WITH TIME ZONE" | "TIMESTAMPTZ" => AnsiType::TimestampTz,
            "DATE" => AnsiType::Date,
            "TIME" => AnsiType::Time,
            "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" => AnsiType::Timestamp,
            "DOUBLE" | "DOUBLE PRECISION" => AnsiType::Double,
            _ => return None,
        })
    }

    /// Whether a column declared with FHIR type `fhir_type` can be rendered as
    /// this type. `None` (no declared type) is compatible with everything; the
    /// value itself is then checked when it is converted. The default type of a
    /// FHIR type is always compatible with it.
    pub fn is_compatible_with(self, fhir_type: Option<&str>) -> bool {
        let Some(t) = fhir_type else {
            return true;
        };
        if default_for_fhir_type(t) == self {
            return true;
        }
        match self {
            AnsiType::Varchar => true,
            AnsiType::Boolean => t == "boolean",
            AnsiType::Int => is_int32_type(t),
            AnsiType::BigInt => is_int32_type(t) || t == "integer64",
            AnsiType::Double => t == "decimal" || is_int32_type(t) || t == "integer64",
            AnsiType::Date => t == "date",
            AnsiType::Time => t == "time",
            AnsiType::Timestamp | AnsiType::TimestampTz => t == "dateTime" || t == "instant",
            AnsiType::Binary => t == "base64Binary",
        }
    }
}

fn is_int32_type(fhir_type: &str) -> bool {
    matches!(fhir_type, "integer" | "positiveInt" | "unsignedInt")
}

/// Default SQL type for a FHIR type code (`viewdef-6`). Every type the table
/// does not single out maps to `CHARACTER VARYING`; note that `decimal`, `date`,
/// `dateTime` and `time` are deliberately character data.
pub fn default_for_fhir_type(fhir_type: &str) -> AnsiType {
    match fhir_type {
        "boolean" => AnsiType::Boolean,
        "integer" | "positiveInt" | "unsignedInt" => AnsiType::Int,
        "integer64" => AnsiType::BigInt,
        "instant" => AnsiType::TimestampTz,
        "base64Binary" => AnsiType::Binary,
        _ => AnsiType::Varchar,
    }
}

/// Default SQL type for a FHIRPath (System) type name (`viewdef-7`), or `None`
/// for a name the table does not list. A column's FHIR type takes precedence
/// over this when both are known.
pub fn default_for_fhirpath_type(fhirpath_type: &str) -> Option<AnsiType> {
    match fhirpath_type {
        "Boolean" => Some(AnsiType::Boolean),
        "Integer" => Some(AnsiType::Int),
        "String" | "Decimal" | "Date" | "Time" | "DateTime" => Some(AnsiType::Varchar),
        _ => None,
    }
}

/// The resolved output type of one ViewDefinition column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnTypeInfo {
    /// Column name.
    pub name: String,
    /// Declared FHIR `type`, if any.
    pub fhir_type: Option<String>,
    /// Whether the column is declared `collection: true`.
    pub collection: bool,
    /// Resolved SQL type: the `ansi/type` tag if present, else the default for
    /// `fhir_type`. `None` when the column declares neither, in which case the
    /// emitter falls back to inspecting the values (via the FHIRPath table).
    pub ansi: Option<AnsiType>,
    /// `true` when `ansi` came from an `ansi/type` tag.
    pub overridden: bool,
}

/// Resolves the output type of every column of a ViewDefinition, in
/// declaration order. Walks `select[].column[]`, nested `select[]` and
/// `unionAll[]`; when a name is declared more than once the first declaration
/// wins (union branches contribute rows, not columns).
///
/// Fails with [`SofError::InvalidViewDefinition`] if any column carries an
/// `ansi/type` tag that is not a string, names an unknown type, or is not
/// compatible with the column's FHIR type.
pub fn resolve_columns(view: &Value) -> Result<Vec<ColumnTypeInfo>, SofError> {
    let mut columns = Vec::new();
    if let Some(selects) = view.get("select").and_then(|v| v.as_array()) {
        for select in selects {
            collect(select, &mut columns)?;
        }
    }
    let mut seen = HashSet::new();
    columns.retain(|c| seen.insert(c.name.clone()));
    Ok(columns)
}

fn collect(select: &Value, out: &mut Vec<ColumnTypeInfo>) -> Result<(), SofError> {
    if let Some(cols) = select.get("column").and_then(|v| v.as_array()) {
        for col in cols {
            if let Some(info) = resolve_column(col)? {
                out.push(info);
            }
        }
    }
    for key in ["select", "unionAll"] {
        if let Some(nested) = select.get(key).and_then(|v| v.as_array()) {
            for s in nested {
                collect(s, out)?;
            }
        }
    }
    Ok(())
}

fn resolve_column(col: &Value) -> Result<Option<ColumnTypeInfo>, SofError> {
    let Some(name) = col.get("name").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let fhir_type = col.get("type").and_then(|v| v.as_str()).map(String::from);
    let collection = col
        .get("collection")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let tagged = ansi_tag(col, name)?;
    if let Some(ansi) = tagged
        && !ansi.is_compatible_with(fhir_type.as_deref())
    {
        return Err(SofError::InvalidViewDefinition(format!(
            "column '{}': {} tag value '{}' is not compatible with type '{}'",
            name,
            ANSI_TYPE_TAG,
            ansi.sql_name(),
            fhir_type.as_deref().unwrap_or_default()
        )));
    }
    let ansi = tagged.or_else(|| fhir_type.as_deref().map(default_for_fhir_type));
    Ok(Some(ColumnTypeInfo {
        name: name.to_string(),
        fhir_type,
        collection,
        ansi,
        overridden: tagged.is_some(),
    }))
}

/// Reads the column's `ansi/type` tag (the first one, if repeated).
fn ansi_tag(col: &Value, name: &str) -> Result<Option<AnsiType>, SofError> {
    let Some(tags) = col.get("tag").and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    for tag in tags {
        if tag.get("name").and_then(|v| v.as_str()) != Some(ANSI_TYPE_TAG) {
            continue;
        }
        let value = tag.get("value").and_then(|v| v.as_str()).ok_or_else(|| {
            SofError::InvalidViewDefinition(format!(
                "column '{}': {} tag requires a string value",
                name, ANSI_TYPE_TAG
            ))
        })?;
        return AnsiType::parse(value).map(Some).ok_or_else(|| {
            SofError::InvalidViewDefinition(format!(
                "column '{}': unsupported {} tag value '{}'",
                name, ANSI_TYPE_TAG, value
            ))
        });
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fhir_type_defaults_follow_the_spec_table() {
        use AnsiType::*;
        for (code, want) in [
            ("boolean", Boolean),
            ("integer", Int),
            ("positiveInt", Int),
            ("unsignedInt", Int),
            ("integer64", BigInt),
            ("instant", TimestampTz),
            ("base64Binary", Binary),
            // Deliberately character data, not float / date / time.
            ("decimal", Varchar),
            ("date", Varchar),
            ("dateTime", Varchar),
            ("time", Varchar),
            ("string", Varchar),
            ("code", Varchar),
            ("uri", Varchar),
            ("id", Varchar),
            ("markdown", Varchar),
        ] {
            assert_eq!(default_for_fhir_type(code), want, "{code}");
        }
    }

    #[test]
    fn fhirpath_type_defaults_follow_the_spec_table() {
        use AnsiType::*;
        assert_eq!(default_for_fhirpath_type("Boolean"), Some(Boolean));
        assert_eq!(default_for_fhirpath_type("Integer"), Some(Int));
        for t in ["String", "Decimal", "Date", "Time", "DateTime"] {
            assert_eq!(default_for_fhirpath_type(t), Some(Varchar), "{t}");
        }
        assert_eq!(default_for_fhirpath_type("Quantity"), None);
    }

    #[test]
    fn parse_accepts_aliases_case_insensitively() {
        use AnsiType::*;
        assert_eq!(AnsiType::parse("varchar"), Some(Varchar));
        assert_eq!(AnsiType::parse("Character  Varying"), Some(Varchar));
        assert_eq!(AnsiType::parse("integer"), Some(Int));
        assert_eq!(AnsiType::parse("INT"), Some(Int));
        assert_eq!(AnsiType::parse("bigint"), Some(BigInt));
        assert_eq!(AnsiType::parse("double precision"), Some(Double));
        assert_eq!(
            AnsiType::parse("timestamp with time zone"),
            Some(TimestampTz)
        );
        assert_eq!(AnsiType::parse("TIMESTAMP"), Some(Timestamp));
        assert_eq!(AnsiType::parse("DATE"), Some(Date));
        assert_eq!(AnsiType::parse("binary"), Some(Binary));
        assert_eq!(AnsiType::parse("geometry"), None);
        assert_eq!(AnsiType::parse(""), None);
    }

    #[test]
    fn sql_name_round_trips_through_parse() {
        for t in [
            AnsiType::Boolean,
            AnsiType::Int,
            AnsiType::BigInt,
            AnsiType::Varchar,
            AnsiType::Binary,
            AnsiType::TimestampTz,
            AnsiType::Date,
            AnsiType::Time,
            AnsiType::Timestamp,
            AnsiType::Double,
        ] {
            assert_eq!(AnsiType::parse(t.sql_name()), Some(t));
        }
    }

    #[test]
    fn compatibility_matrix() {
        use AnsiType::*;
        let cases = [
            (Varchar, "decimal", true),
            (Varchar, "boolean", true),
            (Boolean, "boolean", true),
            (Boolean, "integer", false),
            (Int, "positiveInt", true),
            (Int, "integer64", false),
            (Int, "decimal", false),
            (BigInt, "integer", true),
            (BigInt, "integer64", true),
            (BigInt, "decimal", false),
            (Double, "decimal", true),
            (Double, "integer64", true),
            (Double, "string", false),
            (Date, "date", true),
            (Date, "dateTime", false),
            (Time, "time", true),
            (Time, "date", false),
            (Timestamp, "dateTime", true),
            (Timestamp, "instant", true),
            (Timestamp, "date", false),
            (TimestampTz, "instant", true),
            (TimestampTz, "dateTime", true),
            (Binary, "base64Binary", true),
            (Binary, "string", false),
        ];
        for (ansi, fhir, want) in cases {
            assert_eq!(
                ansi.is_compatible_with(Some(fhir)),
                want,
                "{ansi:?} on {fhir}"
            );
        }
        // An undeclared type accepts any override.
        assert!(AnsiType::Date.is_compatible_with(None));
    }

    #[test]
    fn untyped_column_has_no_resolved_type() {
        let view = json!({"select": [{"column": [{"name": "a", "path": "id"}]}]});
        let cols = resolve_columns(&view).unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].fhir_type, None);
        assert_eq!(cols[0].ansi, None);
        assert!(!cols[0].overridden);
    }

    #[test]
    fn declared_type_wins_and_collection_is_read() {
        let view = json!({"select": [{"column": [
            {"name": "d", "path": "x", "type": "decimal"},
            {"name": "i", "path": "x", "type": "instant"},
            {"name": "b", "path": "x", "type": "integer64", "collection": true}
        ]}]});
        let cols = resolve_columns(&view).unwrap();
        assert_eq!(cols[0].ansi, Some(AnsiType::Varchar));
        assert_eq!(cols[1].ansi, Some(AnsiType::TimestampTz));
        assert_eq!(cols[2].ansi, Some(AnsiType::BigInt));
        assert!(cols[2].collection);
        assert!(cols.iter().all(|c| !c.overridden));
    }

    #[test]
    fn ansi_type_tag_overrides_the_default() {
        let view = json!({"select": [{"column": [
            {"name": "birth", "path": "birthDate", "type": "date",
             "tag": [{"name": "other", "value": "x"},
                     {"name": "ansi/type", "value": "DATE"}]}
        ]}]});
        let cols = resolve_columns(&view).unwrap();
        assert_eq!(cols[0].ansi, Some(AnsiType::Date));
        assert!(cols[0].overridden);
    }

    #[test]
    fn tag_on_untyped_column_is_accepted() {
        let view = json!({"select": [{"column": [
            {"name": "a", "path": "x", "tag": [{"name": "ansi/type", "value": "varchar"}]}
        ]}]});
        let cols = resolve_columns(&view).unwrap();
        assert_eq!(cols[0].ansi, Some(AnsiType::Varchar));
    }

    #[test]
    fn unknown_tag_value_is_rejected_with_column_name() {
        let view = json!({"select": [{"column": [
            {"name": "col1", "path": "x", "type": "string",
             "tag": [{"name": "ansi/type", "value": "GEOMETRY"}]}
        ]}]});
        let err = resolve_columns(&view).unwrap_err().to_string();
        assert!(err.contains("col1") && err.contains("GEOMETRY"), "{err}");
    }

    #[test]
    fn incompatible_tag_value_is_rejected() {
        let view = json!({"select": [{"column": [
            {"name": "dec", "path": "x", "type": "decimal",
             "tag": [{"name": "ansi/type", "value": "INT"}]}
        ]}]});
        let err = resolve_columns(&view).unwrap_err().to_string();
        assert!(
            err.contains("dec") && err.contains("not compatible"),
            "{err}"
        );
    }

    #[test]
    fn non_string_tag_value_is_rejected() {
        let view = json!({"select": [{"column": [
            {"name": "a", "path": "x", "tag": [{"name": "ansi/type", "value": 5}]}
        ]}]});
        assert!(resolve_columns(&view).is_err());
    }

    #[test]
    fn walks_nested_select_union_all_and_for_each() {
        let view = json!({"select": [
            {"forEach": "name", "column": [{"name": "a", "path": "x", "type": "integer"}],
             "select": [{"column": [{"name": "b", "path": "x", "type": "boolean"}]}]},
            {"unionAll": [
                {"column": [{"name": "c", "path": "x", "type": "instant"}]},
                {"column": [{"name": "c", "path": "y", "type": "string"}]}
            ]}
        ]});
        let cols = resolve_columns(&view).unwrap();
        let names: Vec<_> = cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        // First declaration of `c` wins.
        assert_eq!(cols[2].ansi, Some(AnsiType::TimestampTz));
    }

    #[test]
    fn column_without_name_is_skipped_and_no_select_is_empty() {
        let view = json!({"select": [{"column": [{"path": "x"}]}]});
        assert!(resolve_columns(&view).unwrap().is_empty());
        assert!(resolve_columns(&json!({})).unwrap().is_empty());
    }
}
