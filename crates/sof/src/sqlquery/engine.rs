//! In-memory SQLite engine used by `$sqlquery-run`.
//!
//! One connection per request. Each depends-on ViewDefinition is materialized
//! into a named table; the user's SQL then runs against those tables.

use futures::Stream;
use futures::StreamExt;
use rusqlite::{Connection, ToSql, params_from_iter};
use serde_json::Value;
use std::pin::Pin;

use super::{BoundParam, SqlQueryError};

/// FHIR type code for a column. Mirrors the value-set used by
/// `ViewDefinition.select.column.type` so we can pick the correct value[X]
/// when rendering `_format=fhir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnFhirType {
    Boolean,
    Integer,
    Integer64,
    Decimal,
    Date,
    DateTime,
    Instant,
    Time,
    Base64Binary,
    /// Catch-all for `string`, `code`, `id`, `uri`, `canonical`, `url`,
    /// `markdown`, `oid`, etc. The exact code is preserved so the FHIR
    /// formatter can emit `valueCode` vs `valueString` correctly.
    String(String),
}

impl ColumnFhirType {
    pub fn from_code(code: &str) -> Self {
        match code {
            "boolean" => ColumnFhirType::Boolean,
            "integer" | "positiveInt" | "unsignedInt" => ColumnFhirType::Integer,
            "integer64" => ColumnFhirType::Integer64,
            "decimal" => ColumnFhirType::Decimal,
            "date" => ColumnFhirType::Date,
            "dateTime" => ColumnFhirType::DateTime,
            "instant" => ColumnFhirType::Instant,
            "time" => ColumnFhirType::Time,
            "base64Binary" => ColumnFhirType::Base64Binary,
            other => ColumnFhirType::String(other.to_string()),
        }
    }

    /// SQLite type-affinity declaration for `CREATE TABLE`.
    pub fn sqlite_affinity(&self) -> &'static str {
        match self {
            ColumnFhirType::Boolean | ColumnFhirType::Integer | ColumnFhirType::Integer64 => {
                "INTEGER"
            }
            ColumnFhirType::Decimal => "REAL",
            _ => "TEXT",
        }
    }
}

/// One column in a materialized table.
#[derive(Debug, Clone)]
pub struct ColumnSchema {
    pub name: String,
    pub fhir_type: ColumnFhirType,
}

/// Per-table schema: the column list (order matters for INSERT).
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub columns: Vec<ColumnSchema>,
}

impl TableSchema {
    /// Build a schema from a ViewDefinition's `select[].column[]` list.
    /// Walks every `select` entry (including nested `select` under `forEach`)
    /// and collects columns in document order.
    pub fn from_view_definition(view: &Value) -> Self {
        let mut columns = Vec::new();
        if let Some(selects) = view.get("select").and_then(|v| v.as_array()) {
            for s in selects {
                collect_columns(s, &mut columns);
            }
        }
        TableSchema { columns }
    }
}

fn collect_columns(select: &Value, out: &mut Vec<ColumnSchema>) {
    if let Some(cols) = select.get("column").and_then(|v| v.as_array()) {
        for col in cols {
            let Some(name) = col.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let type_code = col
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("string")
                .to_string();
            out.push(ColumnSchema {
                name: name.to_string(),
                fhir_type: ColumnFhirType::from_code(&type_code),
            });
        }
    }
    if let Some(nested) = select.get("select").and_then(|v| v.as_array()) {
        for s in nested {
            collect_columns(s, out);
        }
    }
    if let Some(union) = select.get("unionAll").and_then(|v| v.as_array()) {
        for s in union {
            collect_columns(s, out);
        }
    }
}

/// Result of running the user query.
pub struct QueryResult {
    pub columns: Vec<String>,
    /// Column FHIR types, in `columns` order. Inferred from the rusqlite
    /// declared column type plus a per-row check (NULL columns fall back to
    /// `String`).
    pub column_types: Vec<ColumnFhirType>,
    /// Each row is a Vec of optional values in `columns` order.
    pub rows: Vec<Vec<Option<Value>>>,
}

/// The in-memory SQLite engine.
pub struct InMemorySqlEngine {
    conn: Connection,
}

impl InMemorySqlEngine {
    pub fn open() -> Result<Self, SqlQueryError> {
        let conn = Connection::open_in_memory()?;
        // Aggressive in-memory pragmas — we never persist this DB.
        conn.execute_batch(
            "PRAGMA journal_mode = MEMORY;
             PRAGMA synchronous = OFF;
             PRAGMA temp_store = MEMORY;
             PRAGMA foreign_keys = OFF;",
        )?;
        Ok(Self { conn })
    }

    /// Returns an interrupt handle that can cancel a running statement from
    /// another thread (used by the request-level timeout watchdog).
    pub fn interrupt_handle(&self) -> rusqlite::InterruptHandle {
        self.conn.get_interrupt_handle()
    }

    /// Create a table with the given label and schema.
    pub fn create_table(&self, label: &str, schema: &TableSchema) -> Result<(), SqlQueryError> {
        validate_identifier(label)?;
        let mut columns_ddl = Vec::with_capacity(schema.columns.len());
        for col in &schema.columns {
            validate_identifier(&col.name)?;
            columns_ddl.push(format!(
                "\"{}\" {}",
                col.name,
                col.fhir_type.sqlite_affinity()
            ));
        }
        let sql = if columns_ddl.is_empty() {
            // SQLite needs at least one column.
            format!("CREATE TABLE \"{label}\" (\"_empty\" TEXT)")
        } else {
            format!("CREATE TABLE \"{}\" ({})", label, columns_ddl.join(", "))
        };
        self.conn.execute(&sql, [])?;
        Ok(())
    }

    /// Stream `rows` into `label`. Each row is a flat JSON object whose keys
    /// match column names; missing or null keys become SQL NULL.
    pub async fn insert_rows<S>(
        &mut self,
        label: &str,
        schema: &TableSchema,
        mut rows: Pin<Box<S>>,
        max_rows: usize,
    ) -> Result<usize, SqlQueryError>
    where
        S: Stream<Item = Result<Value, String>> + Send + ?Sized,
    {
        validate_identifier(label)?;
        for col in &schema.columns {
            validate_identifier(&col.name)?;
        }
        if schema.columns.is_empty() {
            // Drain the stream without inserting; nothing to persist.
            let mut n = 0usize;
            while let Some(item) = rows.next().await {
                item.map_err(SqlQueryError::MalformedLibrary)?;
                n += 1;
                if n > max_rows {
                    return Err(SqlQueryError::RowCapExceeded { max: max_rows });
                }
            }
            return Ok(n);
        }

        let placeholders = std::iter::repeat_n("?", schema.columns.len())
            .collect::<Vec<_>>()
            .join(", ");
        let cols_quoted = schema
            .columns
            .iter()
            .map(|c| format!("\"{}\"", c.name))
            .collect::<Vec<_>>()
            .join(", ");
        let insert_sql = format!("INSERT INTO \"{label}\" ({cols_quoted}) VALUES ({placeholders})");

        self.conn.execute("BEGIN", [])?;
        let mut inserted = 0usize;
        let result: Result<usize, SqlQueryError> = (|| {
            let mut stmt = self.conn.prepare(&insert_sql)?;
            while let Some(item) = futures::executor::block_on(rows.next()) {
                let row = item.map_err(SqlQueryError::MalformedLibrary)?;
                inserted += 1;
                if inserted > max_rows {
                    return Err(SqlQueryError::RowCapExceeded { max: max_rows });
                }
                let params: Vec<rusqlite::types::Value> = schema
                    .columns
                    .iter()
                    .map(|c| json_to_sqlite_value(&row, c))
                    .collect();
                let param_refs: Vec<&dyn ToSql> = params.iter().map(|v| v as &dyn ToSql).collect();
                stmt.execute(params_from_iter(param_refs))?;
            }
            Ok(inserted)
        })();
        match result {
            Ok(n) => {
                self.conn.execute("COMMIT", [])?;
                Ok(n)
            }
            Err(e) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(e)
            }
        }
    }

    /// Run a SELECT with named bindings and a row cap.
    pub fn execute_select(
        &self,
        sql: &str,
        bindings: &[BoundParam],
        max_rows: usize,
    ) -> Result<QueryResult, SqlQueryError> {
        let mut stmt = self.conn.prepare(sql)?;

        // Resolve each `:name` binding against the prepared statement's
        // parameter index. Names not referenced by the SQL are silently
        // ignored (the SQL may declare more params than it uses, or none).
        for b in bindings {
            let with_colon = format!(":{}", b.name);
            if let Some(idx) = stmt.parameter_index(&with_colon)? {
                stmt.raw_bind_parameter(idx, &b.value)?;
            }
        }

        let columns: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
        // Pre-seed with String to be overwritten per row.
        let mut column_types: Vec<ColumnFhirType> = columns
            .iter()
            .map(|_| ColumnFhirType::String("string".to_string()))
            .collect();
        let mut rows_out: Vec<Vec<Option<Value>>> = Vec::new();

        let mut rows_iter = stmt.raw_query();
        while let Some(row) = rows_iter.next()? {
            if rows_out.len() >= max_rows {
                // SoF v2: the server's hard cap silently truncates the
                // result set instead of erroring (spec PR #353: "Servers
                // MAY enforce a maximum value, silently capping
                // client-supplied limits at a smaller server-defined
                // maximum"). Caller-supplied `_limit` is also a silent
                // cap and is enforced at the handler. Source-row caps
                // applied during `insert_rows` remain hard errors
                // because truncating a depends-on table would silently
                // change query semantics (JOINs, aggregates).
                break;
            }
            let mut row_vals: Vec<Option<Value>> = Vec::with_capacity(columns.len());
            for (i, _) in columns.iter().enumerate() {
                let v: rusqlite::types::Value = row.get(i)?;
                let (json_val, inferred) = sqlite_value_to_json(v);
                if matches!(column_types[i], ColumnFhirType::String(_)) {
                    if let Some(ft) = inferred {
                        column_types[i] = ft;
                    }
                }
                row_vals.push(json_val);
            }
            rows_out.push(row_vals);
        }

        Ok(QueryResult {
            columns,
            column_types,
            rows: rows_out,
        })
    }
}

fn validate_identifier(name: &str) -> Result<(), SqlQueryError> {
    if name.contains('"') || name.is_empty() {
        return Err(SqlQueryError::InvalidIdentifier(name.to_string()));
    }
    Ok(())
}

fn json_to_sqlite_value(row: &Value, col: &ColumnSchema) -> rusqlite::types::Value {
    use rusqlite::types::Value as RV;
    let raw = row.get(&col.name).unwrap_or(&Value::Null);
    match raw {
        Value::Null => RV::Null,
        Value::Bool(b) => RV::Integer(if *b { 1 } else { 0 }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                RV::Integer(i)
            } else if let Some(f) = n.as_f64() {
                RV::Real(f)
            } else {
                RV::Text(n.to_string())
            }
        }
        Value::String(s) => match col.fhir_type {
            ColumnFhirType::Integer | ColumnFhirType::Integer64 => s
                .parse::<i64>()
                .map(RV::Integer)
                .unwrap_or(RV::Text(s.clone())),
            ColumnFhirType::Decimal => s
                .parse::<f64>()
                .map(RV::Real)
                .unwrap_or(RV::Text(s.clone())),
            ColumnFhirType::Boolean => match s.as_str() {
                "true" | "1" => RV::Integer(1),
                "false" | "0" => RV::Integer(0),
                _ => RV::Text(s.clone()),
            },
            _ => RV::Text(s.clone()),
        },
        Value::Array(_) | Value::Object(_) => RV::Text(raw.to_string()),
    }
}

/// Maps a rusqlite value to JSON plus a best-guess `ColumnFhirType`. Useful
/// for output columns the engine produced (e.g. `SELECT COUNT(*)`).
fn sqlite_value_to_json(v: rusqlite::types::Value) -> (Option<Value>, Option<ColumnFhirType>) {
    use rusqlite::types::Value as RV;
    match v {
        RV::Null => (None, None),
        RV::Integer(i) => (Some(Value::Number(i.into())), Some(ColumnFhirType::Integer)),
        RV::Real(f) => (
            serde_json::Number::from_f64(f).map(Value::Number),
            Some(ColumnFhirType::Decimal),
        ),
        RV::Text(s) => (Some(Value::String(s)), None),
        RV::Blob(b) => (
            Some(Value::String(
                base64::engine::general_purpose::STANDARD.encode(b),
            )),
            Some(ColumnFhirType::Base64Binary),
        ),
    }
}

use base64::Engine as _;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use serde_json::json;

    fn schema(cols: &[(&str, ColumnFhirType)]) -> TableSchema {
        TableSchema {
            columns: cols
                .iter()
                .map(|(n, t)| ColumnSchema {
                    name: (*n).to_string(),
                    fhir_type: t.clone(),
                })
                .collect(),
        }
    }

    #[tokio::test]
    async fn round_trip_basic() {
        let mut engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[
            ("id", ColumnFhirType::String("id".into())),
            ("n", ColumnFhirType::Integer),
        ]);
        engine.create_table("patients", &s).unwrap();
        let rows = stream::iter(vec![
            Ok(json!({"id": "a", "n": 1})),
            Ok(json!({"id": "b", "n": 2})),
        ]);
        let inserted = engine
            .insert_rows("patients", &s, Box::pin(rows), 10)
            .await
            .unwrap();
        assert_eq!(inserted, 2);
        let result = engine
            .execute_select("SELECT id, n FROM patients ORDER BY n", &[], 10)
            .unwrap();
        assert_eq!(result.columns, vec!["id", "n"]);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], Some(Value::String("a".into())));
        assert_eq!(result.rows[0][1], Some(Value::Number(1.into())));
    }

    #[tokio::test]
    async fn null_handling() {
        let mut engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[
            ("id", ColumnFhirType::String("id".into())),
            ("age", ColumnFhirType::Integer),
        ]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter(vec![Ok(json!({"id": "a"}))]); // age missing
        engine
            .insert_rows("t", &s, Box::pin(rows), 10)
            .await
            .unwrap();
        let result = engine
            .execute_select("SELECT id, age FROM t", &[], 10)
            .unwrap();
        assert_eq!(result.rows[0][1], None);
    }

    #[tokio::test]
    async fn row_cap_exceeded() {
        let mut engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("n", ColumnFhirType::Integer)]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter((0..10).map(|i| Ok(json!({"n": i}))));
        let err = engine
            .insert_rows("t", &s, Box::pin(rows), 3)
            .await
            .unwrap_err();
        assert!(matches!(err, SqlQueryError::RowCapExceeded { max: 3 }));
    }

    #[tokio::test]
    async fn execute_select_silently_truncates_at_max_rows() {
        // SoF v2 PR #353: the server's hard cap silently truncates the
        // result set; it must not error.
        let mut engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("n", ColumnFhirType::Integer)]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter((1..=10).map(|i| Ok(json!({"n": i}))));
        engine
            .insert_rows("t", &s, Box::pin(rows), 100)
            .await
            .unwrap();
        let result = engine
            .execute_select("SELECT n FROM t ORDER BY n", &[], 4)
            .unwrap();
        assert_eq!(result.rows.len(), 4);
        assert_eq!(result.rows[0][0], Some(Value::Number(1.into())));
        assert_eq!(result.rows[3][0], Some(Value::Number(4.into())));
    }

    #[test]
    fn rejects_quote_in_identifier() {
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("a", ColumnFhirType::Integer)]);
        let err = engine.create_table("bad\"name", &s).unwrap_err();
        assert!(matches!(err, SqlQueryError::InvalidIdentifier(_)));
    }

    #[tokio::test]
    async fn named_bindings_filter() {
        let mut engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("n", ColumnFhirType::Integer)]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter((1..=5).map(|i| Ok(json!({"n": i}))));
        engine
            .insert_rows("t", &s, Box::pin(rows), 100)
            .await
            .unwrap();
        let bindings = vec![BoundParam {
            name: "min".to_string(),
            value: rusqlite::types::Value::Integer(3),
        }];
        let result = engine
            .execute_select("SELECT n FROM t WHERE n >= :min ORDER BY n", &bindings, 100)
            .unwrap();
        assert_eq!(result.rows.len(), 3);
    }

    #[test]
    fn schema_from_vd_select_columns() {
        let vd = json!({
            "select": [{
                "column": [
                    {"name": "id", "type": "id"},
                    {"name": "n", "type": "integer"}
                ]
            }]
        });
        let s = TableSchema::from_view_definition(&vd);
        assert_eq!(s.columns.len(), 2);
        assert_eq!(s.columns[0].name, "id");
        assert!(matches!(s.columns[1].fhir_type, ColumnFhirType::Integer));
    }

    #[test]
    fn schema_walks_nested_selects_and_union() {
        let vd = json!({
            "select": [{
                "column": [{"name": "a"}],
                "select": [{"column": [{"name": "b"}]}],
                "unionAll": [{"column": [{"name": "c"}]}]
            }]
        });
        let s = TableSchema::from_view_definition(&vd);
        assert_eq!(
            s.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }
}
