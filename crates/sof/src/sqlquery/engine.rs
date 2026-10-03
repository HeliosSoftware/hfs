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

    /// The canonical FHIR type code for this type (#842/04): the
    /// representative code [`Self::from_code`] maps every original code in
    /// this variant's family to (`"integer"` for `integer`, `positiveInt`,
    /// and `unsignedInt` alike), except [`ColumnFhirType::String`], which
    /// returns the exact code it was built from. Used to display a
    /// resolved dependency's own declared column type (the SQL Library
    /// Columns card's *Type* cell, #842) without re-deriving it from the
    /// source `ViewDefinition` JSON a second time.
    pub fn code(&self) -> &str {
        match self {
            ColumnFhirType::Boolean => "boolean",
            ColumnFhirType::Integer => "integer",
            ColumnFhirType::Integer64 => "integer64",
            ColumnFhirType::Decimal => "decimal",
            ColumnFhirType::Date => "date",
            ColumnFhirType::DateTime => "dateTime",
            ColumnFhirType::Instant => "instant",
            ColumnFhirType::Time => "time",
            ColumnFhirType::Base64Binary => "base64Binary",
            ColumnFhirType::String(code) => code,
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

    /// The output layout of a ViewDefinition executed by an in-DB SQL runner
    /// (SQLite / PostgreSQL), for formatting its rows with explicit columns.
    ///
    /// The membership is the declared columns [`Self::from_view_definition`]
    /// collects, each name kept once (the branches of a `unionAll` declare
    /// the same columns). The order is the one the SQL compiler projects,
    /// which is not document order: within a select list, the columns of
    /// every clause (each clause's own columns, then its nested `select`s)
    /// come first, and the `unionAll` branch columns follow them, so a
    /// column declared after a `unionAll` clause precedes the branch
    /// columns. A row object from those runners lists its keys in that same
    /// order, which makes this layout equal to first-row inference whenever
    /// the first row carries every column.
    ///
    /// A SQLQuery dependency's table is created from this layout, so a
    /// `unionAll` view gets one column per name and `SELECT *` from it lists
    /// the columns as the SQL runners project them. For a view without
    /// `unionAll` (and with distinct column names) the layout is
    /// [`Self::from_view_definition`] exactly: same columns, types, and
    /// order. That function keeps document order and still repeats union
    /// declarations for its other callers.
    pub fn sql_output_layout(view: &Value) -> Self {
        let mut columns = Vec::new();
        if let Some(selects) = view.get("select").and_then(|v| v.as_array()) {
            collect_projected_list(selects, &mut columns);
        }
        let mut seen = std::collections::HashSet::new();
        columns.retain(|c: &ColumnSchema| seen.insert(c.name.clone()));
        TableSchema { columns }
    }

    /// The column names, in layout order.
    pub fn column_names(&self) -> Vec<String> {
        self.columns.iter().map(|c| c.name.clone()).collect()
    }
}

/// A select list in SQL projection order: every clause's columns, then the
/// branch columns of the list's `unionAll` clauses.
fn collect_projected_list(clauses: &[Value], out: &mut Vec<ColumnSchema>) {
    let mut unions: Vec<&Vec<Value>> = Vec::new();
    for clause in clauses {
        collect_projected_clause(clause, out);
        if let Some(branches) = clause.get("unionAll").and_then(|v| v.as_array()) {
            unions.push(branches);
        }
    }
    for branches in unions {
        collect_projected_branches(branches, out);
    }
}

/// One clause's own columns followed by its nested `select` list.
fn collect_projected_clause(clause: &Value, out: &mut Vec<ColumnSchema>) {
    push_declared_columns(clause, out);
    if let Some(nested) = clause.get("select").and_then(|v| v.as_array()) {
        collect_projected_list(nested, out);
    }
}

/// `unionAll` branches, a nested `unionAll` contributing its own branches.
fn collect_projected_branches(branches: &[Value], out: &mut Vec<ColumnSchema>) {
    for branch in branches {
        collect_projected_clause(branch, out);
        if let Some(inner) = branch.get("unionAll").and_then(|v| v.as_array()) {
            collect_projected_branches(inner, out);
        }
    }
}

fn collect_columns(select: &Value, out: &mut Vec<ColumnSchema>) {
    push_declared_columns(select, out);
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

/// A clause's own `column[]` entries; a column without a `name` is skipped.
fn push_declared_columns(select: &Value, out: &mut Vec<ColumnSchema>) {
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

    /// Creates a view named `label` that exposes every row and column of
    /// the existing table (or view) `source`, so a consumer's SQL can address
    /// that table under a second name without copying a single row. Both
    /// identifiers go through the same validation as [`Self::create_table`].
    /// The view lives in this request's in-memory database and disappears
    /// with it.
    pub fn create_view(&self, label: &str, source: &str) -> Result<(), SqlQueryError> {
        validate_identifier(label)?;
        validate_identifier(source)?;
        self.conn.execute(
            &format!("CREATE VIEW \"{label}\" AS SELECT * FROM \"{source}\""),
            [],
        )?;
        // SQLite defers a view's column/table resolution to first use (the
        // same forward-reference allowance it grants triggers), so a missing
        // `source` would otherwise go unnoticed until a caller later queries
        // `label`. Prepare (without running) a statement against the new
        // view right away so a missing `source` is reported from
        // `create_view` itself, and drop the unusable view instead of
        // leaving a dangling one behind.
        if let Err(e) = self.conn.prepare(&format!("SELECT * FROM \"{label}\"")) {
            let _ = self.conn.execute(&format!("DROP VIEW \"{label}\""), []);
            return Err(e.into());
        }
        Ok(())
    }

    /// Streams `rows` into `label` on a dedicated blocking thread, then
    /// hands the engine back to the caller.
    ///
    /// The engine (and the `rusqlite::Statement` the insert loop prepares)
    /// is not `Send` across an `.await` point, and the row stream is
    /// typically a `tokio::sync::mpsc` channel whose `recv` spends the
    /// polling task's cooperative budget on every item — draining more than
    /// 128 rows in an ordinary async task therefore starves the waker and
    /// hangs forever. Moving the whole operation into
    /// `tokio::task::spawn_blocking` sidesteps both problems: the engine
    /// crosses into a blocking-pool thread by value, the stream is driven
    /// with `Handle::block_on` (which resets the cooperative budget on every
    /// poll and is safe to use exactly because there is no scheduler to
    /// starve on that thread), and the engine is returned to the caller once
    /// the whole insert has completed.
    ///
    /// Returns `Ok((engine, n))` on success, where `n` is the number of rows
    /// inserted and the transaction has been committed. On `Err`, the
    /// transaction has already been rolled back and the engine is dropped;
    /// callers are expected to abort the current plan rather than keep using
    /// a half-populated database.
    pub async fn insert_rows<S>(
        self,
        label: &str,
        schema: &TableSchema,
        rows: Pin<Box<S>>,
        max_rows: usize,
    ) -> Result<(Self, usize), SqlQueryError>
    where
        S: Stream<Item = Result<Value, String>> + Send + 'static + ?Sized,
    {
        let label = label.to_string();
        let schema = schema.clone();
        tokio::task::spawn_blocking(move || {
            let handle = tokio::runtime::Handle::current();
            let mut engine = self;
            let inserted = engine.insert_rows_blocking(&label, &schema, rows, max_rows, &handle)?;
            Ok((engine, inserted))
        })
        .await
        .map_err(|e| SqlQueryError::Internal(format!("sqlquery worker panicked: {e}")))?
    }

    /// Synchronous body of [`Self::insert_rows`]. Runs entirely on the
    /// blocking thread `insert_rows` spawned; every `rows.next()` call
    /// (including the no-columns fast path) goes through `handle.block_on`
    /// instead of `.await`, since this function is not itself async.
    fn insert_rows_blocking<S>(
        &mut self,
        label: &str,
        schema: &TableSchema,
        mut rows: Pin<Box<S>>,
        max_rows: usize,
        handle: &tokio::runtime::Handle,
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
            while let Some(item) = handle.block_on(rows.next()) {
                item.map_err(SqlQueryError::SourceStream)?;
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
            while let Some(item) = handle.block_on(rows.next()) {
                let row = item.map_err(SqlQueryError::SourceStream)?;
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
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[
            ("id", ColumnFhirType::String("id".into())),
            ("n", ColumnFhirType::Integer),
        ]);
        engine.create_table("patients", &s).unwrap();
        let rows = stream::iter(vec![
            Ok(json!({"id": "a", "n": 1})),
            Ok(json!({"id": "b", "n": 2})),
        ]);
        let (engine, inserted) = engine
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
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[
            ("id", ColumnFhirType::String("id".into())),
            ("age", ColumnFhirType::Integer),
        ]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter(vec![Ok(json!({"id": "a"}))]); // age missing
        let (engine, _inserted) = engine
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
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("n", ColumnFhirType::Integer)]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter((0..10).map(|i| Ok(json!({"n": i}))));
        // `Result<(InMemorySqlEngine, usize), _>` doesn't implement `Debug`
        // (the engine wraps a `rusqlite::Connection`, which doesn't), so
        // `unwrap_err()` isn't available here; match instead.
        let err = match engine.insert_rows("t", &s, Box::pin(rows), 3).await {
            Ok(_) => panic!("expected RowCapExceeded"),
            Err(e) => e,
        };
        assert!(matches!(err, SqlQueryError::RowCapExceeded { max: 3 }));
    }

    #[tokio::test]
    async fn insert_rows_reports_stream_error_as_source_stream() {
        // A `SofRunner`'s row stream fails mid-materialization (storage
        // error, backend statement timeout, lost connection). This must be
        // reported as `SourceStream`, not folded into `MalformedLibrary` —
        // the Library and its ViewDefinitions are perfectly well-formed.
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("id", ColumnFhirType::String("id".into()))]);
        engine.create_table("t", &s).unwrap();
        let ok_rows = (0..200).map(|i| Ok(json!({"id": format!("p{i}")})));
        let rows = stream::iter(
            ok_rows.chain(std::iter::once(Err("connection reset by peer".to_string()))),
        );
        let err = match engine.insert_rows("t", &s, Box::pin(rows), 10_000).await {
            Ok(_) => panic!("expected SourceStream"),
            Err(e) => e,
        };
        let SqlQueryError::SourceStream(msg) = &err else {
            panic!("expected SourceStream, got {err:?}");
        };
        assert!(
            msg.contains("connection reset by peer"),
            "unexpected message: {msg}"
        );
        assert!(
            format!("{err}").starts_with("dependency source failed: "),
            "unexpected display: {err}"
        );
    }

    #[tokio::test]
    async fn execute_select_silently_truncates_at_max_rows() {
        // SoF v2 PR #353: the server's hard cap silently truncates the
        // result set; it must not error.
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("n", ColumnFhirType::Integer)]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter((1..=10).map(|i| Ok(json!({"n": i}))));
        let (engine, _inserted) = engine
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
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("n", ColumnFhirType::Integer)]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter((1..=5).map(|i| Ok(json!({"n": i}))));
        let (engine, _inserted) = engine
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

    /// Regression test for the hang this ticket fixes. A real runner (see
    /// `crates/persistence/src/sof/sqlite.rs`) feeds `insert_rows` through a
    /// `tokio::sync::mpsc` channel from a `spawn_blocking` producer. Each
    /// `recv` on that channel spends one unit of the polling task's
    /// cooperative budget (128 per poll, tokio's `coop` module); once it ran
    /// out at row 129, the old executor-blocking implementation (draining
    /// the stream via the `futures` crate's synchronous executor) never woke
    /// up again. 1,000 rows is comfortably past
    /// that threshold — `futures::stream::iter` would not reproduce this,
    /// since it never touches the cooperative budget.
    ///
    /// The consumer runs inside `tokio::spawn` (not directly `.await`ed) and
    /// the `JoinHandle` is wrapped in `tokio::time::timeout`: on a
    /// single-threaded runtime (`main`, the pre-fix binary) this test would
    /// otherwise hang forever instead of failing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn insert_rows_drains_tokio_mpsc_stream_past_coop_budget() {
        const ROW_COUNT: i64 = 1000;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Value, String>>(256);

        tokio::task::spawn_blocking(move || {
            for i in 0..ROW_COUNT {
                let row = json!({"id": format!("p{i}"), "n": i});
                if tx.blocking_send(Ok(row)).is_err() {
                    break;
                }
            }
        });

        let consumer = tokio::spawn(async move {
            let engine = InMemorySqlEngine::open().unwrap();
            let s = schema(&[
                ("id", ColumnFhirType::String("id".into())),
                ("n", ColumnFhirType::Integer),
            ]);
            engine.create_table("t", &s).unwrap();
            let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
            let (engine, inserted) = engine
                .insert_rows("t", &s, Box::pin(stream), 10_000)
                .await
                .unwrap();
            let count = engine
                .execute_select("SELECT COUNT(*) AS c FROM t", &[], 10)
                .unwrap();
            (inserted, count)
        });

        let (inserted, count) = tokio::time::timeout(std::time::Duration::from_secs(10), consumer)
            .await
            .expect("insert_rows must not hang past the cooperative budget")
            .expect("consumer task must not panic");

        assert_eq!(inserted, ROW_COUNT as usize);
        assert_eq!(count.rows[0][0], Some(Value::Number(ROW_COUNT.into())));
    }

    #[test]
    fn code_returns_the_representative_fhir_type_code() {
        assert_eq!(ColumnFhirType::Boolean.code(), "boolean");
        assert_eq!(ColumnFhirType::Integer.code(), "integer");
        assert_eq!(ColumnFhirType::Decimal.code(), "decimal");
        // `from_code`'s own family members collapse to the same
        // representative code `code()` reports back.
        assert_eq!(ColumnFhirType::from_code("positiveInt").code(), "integer");
        // `String` returns the exact code it was built from, never a
        // generic "string".
        assert_eq!(ColumnFhirType::String("id".into()).code(), "id");
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

    fn layout_names(vd: &Value) -> Vec<String> {
        TableSchema::sql_output_layout(vd).column_names()
    }

    /// The deduplicated `from_view_definition` membership, in first-seen order.
    fn declared_names(vd: &Value) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for c in TableSchema::from_view_definition(vd).columns {
            if !names.contains(&c.name) {
                names.push(c.name);
            }
        }
        names
    }

    fn sorted(mut names: Vec<String>) -> Vec<String> {
        names.sort();
        names
    }

    #[test]
    fn sql_layout_of_a_flat_view_is_document_order() {
        let vd = json!({"select": [
            {"column": [{"name": "id"}, {"name": "gender"}]},
            {"column": [{"name": "dob", "type": "date"}]}
        ]});
        assert_eq!(layout_names(&vd), vec!["id", "gender", "dob"]);
        let layout = TableSchema::sql_output_layout(&vd);
        assert!(matches!(layout.columns[2].fhir_type, ColumnFhirType::Date));
    }

    #[test]
    fn sql_layout_puts_nested_select_columns_after_their_clause_columns() {
        let vd = json!({"select": [
            {
                "forEach": "name",
                "column": [{"name": "family"}],
                "select": [
                    {"forEachOrNull": "given", "column": [{"name": "given"}]},
                    {"column": [{"name": "use"}]}
                ]
            },
            {"column": [{"name": "id"}]}
        ]});
        assert_eq!(layout_names(&vd), vec!["family", "given", "use", "id"]);
    }

    #[test]
    fn sql_layout_projects_shared_columns_before_union_branch_columns() {
        // The union clause precedes a sibling column: the compiler projects
        // every shared column (`id`, then `gender`) before the branch columns.
        let vd = json!({"select": [
            {"column": [{"name": "id"}]},
            {"unionAll": [
                {"forEach": "telecom", "column": [{"name": "value"}, {"name": "system"}]},
                {"forEach": "contact.telecom", "column": [{"name": "value"}, {"name": "system"}]}
            ]},
            {"column": [{"name": "gender"}]}
        ]});
        assert_eq!(layout_names(&vd), vec!["id", "gender", "value", "system"]);
        // Document order differs, which is why the layout is not `TableSchema`.
        assert_eq!(declared_names(&vd), vec!["id", "value", "system", "gender"]);
    }

    #[test]
    fn sql_layout_merges_columns_beside_a_union_into_the_shared_prefix() {
        let vd = json!({"select": [{
            "forEach": "contact",
            "column": [{"name": "rel"}],
            "select": [{"column": [{"name": "nested"}]}],
            "unionAll": [
                {"column": [{"name": "v"}]},
                {"unionAll": [{"column": [{"name": "v"}]}, {"column": [{"name": "v"}]}]}
            ]
        }, {"column": [{"name": "after"}]}]});
        assert_eq!(layout_names(&vd), vec!["rel", "nested", "after", "v"]);
    }

    #[test]
    fn sql_layout_stably_deduplicates_union_declarations() {
        let vd = json!({"select": [{"unionAll": [
            {"column": [{"name": "a"}, {"name": "b"}]},
            {"column": [{"name": "a"}, {"name": "b"}]},
            {"column": [{"name": "a"}, {"name": "b"}]}
        ]}]});
        assert_eq!(layout_names(&vd), vec!["a", "b"]);
        assert_eq!(TableSchema::from_view_definition(&vd).columns.len(), 6);
    }

    #[test]
    fn sql_layout_keeps_repeat_and_collection_columns() {
        let vd = json!({"select": [
            {"column": [{"name": "id"}, {"name": "names", "path": "name.given", "collection": true}]},
            {
                "repeat": ["item"],
                "column": [{"name": "linkId"}],
                "select": [{"forEach": "answer", "column": [{"name": "answer", "type": "integer"}]}]
            }
        ]});
        assert_eq!(layout_names(&vd), vec!["id", "names", "linkId", "answer"]);
    }

    #[test]
    fn sql_layout_has_the_declared_membership_and_ignores_unnamed_columns() {
        let vd = json!({"select": [
            {"column": [{"name": "a"}, {"path": "no_name"}]},
            {"unionAll": [{"column": [{"name": "b"}]}, {"column": [{"name": "b"}]}]},
            {"select": [{"column": [{"name": "c"}]}], "column": [{"name": "a"}]}
        ]});
        assert_eq!(layout_names(&vd), vec!["a", "c", "b"]);
        assert_eq!(sorted(layout_names(&vd)), sorted(declared_names(&vd)));
        assert!(layout_names(&json!({})).is_empty());
    }

    #[test]
    fn sql_layout_membership_matches_table_schema_for_the_conformance_corpus() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sql-on-fhir/tests");
        let mut views = 0usize;
        for entry in std::fs::read_dir(&dir).expect("conformance corpus") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let fixture: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
            for test in fixture["tests"].as_array().into_iter().flatten() {
                let view = &test["view"];
                assert_eq!(
                    sorted(layout_names(view)),
                    sorted(declared_names(view)),
                    "{}: {}",
                    path.display(),
                    test["title"]
                );
                views += 1;
            }
        }
        assert!(views > 100, "only {views} conformance views checked");
    }

    fn names_and_types(schema: &TableSchema) -> Vec<(String, ColumnFhirType)> {
        schema
            .columns
            .iter()
            .map(|c| (c.name.clone(), c.fhir_type.clone()))
            .collect()
    }

    /// A SQLQuery dependency's table is built from `sql_output_layout`: for a
    /// view without `unionAll` that must be `from_view_definition` exactly
    /// (names, types, order), so existing dependency tables are unchanged.
    #[test]
    fn sql_layout_equals_table_schema_for_conformance_views_without_union() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sql-on-fhir/tests");
        let mut views = 0usize;
        for entry in std::fs::read_dir(&dir).expect("conformance corpus") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let fixture: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
            for test in fixture["tests"].as_array().into_iter().flatten() {
                let view = &test["view"];
                let declared = TableSchema::from_view_definition(view);
                let mut seen = std::collections::HashSet::new();
                let distinct = declared.columns.iter().all(|c| seen.insert(&c.name));
                if view.to_string().contains("\"unionAll\"") || !distinct {
                    continue;
                }
                assert_eq!(
                    names_and_types(&TableSchema::sql_output_layout(view)),
                    names_and_types(&declared),
                    "{}: {}",
                    path.display(),
                    test["title"]
                );
                views += 1;
            }
        }
        assert!(
            views > 100,
            "only {views} non-union conformance views checked"
        );
    }

    /// A `unionAll` view's dependency table gets each column once, and union
    /// rows (inserted by column name) keep their values aligned.
    #[tokio::test]
    async fn union_dependency_table_has_unique_columns_and_aligned_rows() {
        let vd = json!({"select": [
            {"column": [{"name": "id"}]},
            {"unionAll": [
                {"forEach": "telecom", "column": [
                    {"name": "value"}, {"name": "rank", "type": "integer"}
                ]},
                {"forEach": "contact.telecom", "column": [
                    {"name": "value"}, {"name": "rank", "type": "integer"}
                ]}
            ]}
        ]});
        assert_eq!(TableSchema::from_view_definition(&vd).columns.len(), 5);
        let s = TableSchema::sql_output_layout(&vd);
        assert_eq!(
            names_and_types(&s),
            vec![
                ("id".to_string(), ColumnFhirType::String("string".into())),
                ("value".to_string(), ColumnFhirType::String("string".into())),
                ("rank".to_string(), ColumnFhirType::Integer),
            ]
        );

        let engine = InMemorySqlEngine::open().unwrap();
        engine.create_table("t", &s).unwrap();
        // Keys in a different order than the table: insertion is by name.
        let rows = futures::stream::iter(vec![
            Ok(json!({"rank": 1, "value": "a@x", "id": "p1"})),
            Ok(json!({"value": "b@x", "id": "p1", "rank": 2})),
        ]);
        let (engine, n) = engine
            .insert_rows("t", &s, Box::pin(rows), 10)
            .await
            .unwrap();
        assert_eq!(n, 2);
        let r = engine
            .execute_select("SELECT * FROM t ORDER BY rank", &[], 10)
            .unwrap();
        assert_eq!(r.columns, vec!["id", "value", "rank"]);
        assert_eq!(
            r.rows,
            vec![
                vec![Some(json!("p1")), Some(json!("a@x")), Some(json!(1))],
                vec![Some(json!("p1")), Some(json!("b@x")), Some(json!(2))],
            ]
        );
    }

    #[tokio::test]
    async fn create_view_exposes_source_rows_and_columns() {
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[
            ("id", ColumnFhirType::String("id".into())),
            ("n", ColumnFhirType::Integer),
        ]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter(vec![
            Ok(json!({"id": "a", "n": 1})),
            Ok(json!({"id": "b", "n": 2})),
            Ok(json!({"id": "c", "n": 3})),
        ]);
        let (engine, inserted) = engine
            .insert_rows("t", &s, Box::pin(rows), 10)
            .await
            .unwrap();
        assert_eq!(inserted, 3);
        engine.create_view("v", "t").unwrap();

        let from_view = engine
            .execute_select("SELECT id, n FROM v ORDER BY n", &[], 100)
            .unwrap();
        let from_table = engine
            .execute_select("SELECT id, n FROM t ORDER BY n", &[], 100)
            .unwrap();
        assert_eq!(from_view.rows.len(), 3);
        assert_eq!(from_view.rows, from_table.rows);
        assert_eq!(from_view.rows[0][0], Some(Value::String("a".into())));
        assert_eq!(from_view.rows[0][1], Some(Value::Number(1.into())));
        assert_eq!(from_view.rows[2][0], Some(Value::String("c".into())));
        assert_eq!(from_view.rows[2][1], Some(Value::Number(3.into())));
    }

    #[tokio::test]
    async fn create_view_reports_the_same_inferred_column_types_as_the_table() {
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[
            ("id", ColumnFhirType::String("id".into())),
            ("n", ColumnFhirType::Integer),
        ]);
        engine.create_table("t", &s).unwrap();
        let rows = stream::iter(vec![
            Ok(json!({"id": "a", "n": 1})),
            Ok(json!({"id": "b", "n": 2})),
            Ok(json!({"id": "c", "n": 3})),
        ]);
        let (engine, _inserted) = engine
            .insert_rows("t", &s, Box::pin(rows), 10)
            .await
            .unwrap();
        engine.create_view("v", "t").unwrap();

        let from_view = engine.execute_select("SELECT * FROM v", &[], 100).unwrap();
        let from_table = engine.execute_select("SELECT * FROM t", &[], 100).unwrap();
        assert_eq!(from_view.columns, from_table.columns);
        assert_eq!(from_view.column_types.len(), from_table.column_types.len());
        for (view_ty, table_ty) in from_view.column_types.iter().zip(&from_table.column_types) {
            assert_eq!(view_ty.code(), table_ty.code());
        }
    }

    #[test]
    fn create_view_over_empty_schema_table_works() {
        let engine = InMemorySqlEngine::open().unwrap();
        let s = TableSchema { columns: vec![] };
        engine.create_table("e", &s).unwrap();
        engine.create_view("ve", "e").unwrap();
        let result = engine
            .execute_select("SELECT COUNT(*) AS c FROM ve", &[], 100)
            .unwrap();
        assert_eq!(result.rows[0][0], Some(Value::Number(0.into())));
    }

    #[test]
    fn create_view_rejects_invalid_identifiers() {
        let engine = InMemorySqlEngine::open().unwrap();
        let s = schema(&[("a", ColumnFhirType::Integer)]);
        engine.create_table("t", &s).unwrap();

        let err = engine.create_view("bad\"name", "t").unwrap_err();
        assert!(matches!(err, SqlQueryError::InvalidIdentifier(_)));

        let err = engine.create_view("v", "").unwrap_err();
        assert!(matches!(err, SqlQueryError::InvalidIdentifier(_)));
    }

    #[test]
    fn create_view_on_missing_source_is_an_error() {
        let engine = InMemorySqlEngine::open().unwrap();
        let err = engine.create_view("v", "does_not_exist").unwrap_err();
        assert!(!matches!(err, SqlQueryError::InvalidIdentifier(_)));
    }
}
