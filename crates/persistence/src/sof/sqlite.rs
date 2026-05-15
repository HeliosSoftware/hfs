//! SQLite in-DB SQL-on-FHIR runner.
//!
//! [`SqliteInDbRunner`] compiles a ViewDefinition to a parameterised SQLite
//! `SELECT` statement and executes it directly against the `resources` table,
//! bypassing in-process FHIRPath evaluation entirely.
//!
//! ## Streaming
//!
//! Rows are sent one-by-one through a bounded `tokio::sync::mpsc` channel
//! (buffer: 256) so the HTTP layer can begin flushing to the client before the
//! full result set is read.  The blocking SQLite iteration runs in a dedicated
//! `spawn_blocking` thread so it never stalls the async runtime.

use helios_fhir::FhirVersion;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::core::sof_runner::{RowStream, SofError, SofRunner, ViewFilters, ViewRow};
use crate::tenant::TenantContext;

use super::compiler::{SqlDialect, compile_view_definition_dialect};

/// Channel buffer depth (rows that can be queued ahead of the consumer).
const CHANNEL_BUFFER: usize = 256;

/// SQL-on-FHIR runner that compiles ViewDefinitions to SQLite SQL.
pub struct SqliteInDbRunner {
    pool: Pool<SqliteConnectionManager>,
    fhir_version: FhirVersion,
}

impl SqliteInDbRunner {
    /// Creates a new runner backed by the given connection pool. Uses the
    /// default FHIR version (R4) for compile-time cardinality lookups; call
    /// [`Self::with_fhir_version`] to override.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self {
            pool,
            fhir_version: FhirVersion::default(),
        }
    }

    /// Returns a runner that consults the given FHIR version's field-type
    /// table when validating `collection: false` columns.
    pub fn with_fhir_version(mut self, version: FhirVersion) -> Self {
        self.fhir_version = version;
        self
    }
}

#[async_trait::async_trait]
impl SofRunner for SqliteInDbRunner {
    fn runner_name(&self) -> &'static str {
        "sqlite-indb"
    }

    async fn run_view(
        &self,
        tenant: &TenantContext,
        view_definition: Value,
        filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        // Compile synchronously (cheap, no I/O)
        let compiled = compile_view_definition_dialect(
            &view_definition,
            SqlDialect::Sqlite,
            self.fhir_version,
        )?;

        debug!(
            runner = "sqlite-indb",
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );

        let tenant_id = tenant.tenant_id().to_string();
        let resource_type = view_definition
            .get("resource")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let limit = filters.limit;
        let columns = compiled.columns.clone();
        let pool = self.pool.clone();

        // Inject runtime filter conditions (since, patient/group). The
        // compiled query already reserves `?3..?N` for ViewDefinition
        // constants; runtime filters allocate from the next free slot.
        let (sql, extra_params) = build_sqlite_sql(&compiled.sql, &compiled.constants, &filters);

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);

        tokio::task::spawn_blocking(move || {
            stream_sqlite_rows(
                &pool,
                &sql,
                &tenant_id,
                &resource_type,
                extra_params,
                &columns,
                limit,
                tx,
            );
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

// ============================================================================
// SQL runtime-filter injection
// ============================================================================

/// Appends runtime filter conditions (`since`, `patient`) to the compiled SQL
/// and returns the bound parameters that follow `tenant_id` and
/// `resource_type` (i.e. ViewDefinition constants then runtime filter values).
///
/// SQLite positional parameters are `?1`, `?2`, … The base SQL always uses
/// `?1 = tenant_id` and `?2 = resource_type`. Constants then occupy
/// `?3..?(2+constants.len())`; runtime filter conditions bind from the next
/// free slot.
fn build_sqlite_sql(
    base_sql: &str,
    constants: &[super::ir::LitValue],
    filters: &ViewFilters,
) -> (String, Vec<SqliteParam>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut extra_params: Vec<SqliteParam> = constants
        .iter()
        .map(SqliteParam::from_lit)
        .collect::<Vec<_>>();
    let mut next_param = 3usize + constants.len();

    if let Some(since) = &filters.since {
        conditions.push(format!("r.last_updated >= ?{next_param}"));
        // Store as RFC 3339 string — SQLite datetime columns are TEXT
        extra_params.push(SqliteParam::Text(since.to_rfc3339()));
        next_param += 1;
    }

    if !filters.patient.is_empty() {
        // Multi-value: OR across all patient references, each checked against
        // both subject.reference and patient.reference paths.
        let mut ors: Vec<String> = Vec::with_capacity(filters.patient.len());
        for patient in &filters.patient {
            let p = next_param;
            ors.push(format!(
                "(json_extract(r.data,'$.subject.reference')=?{p} \
                 OR json_extract(r.data,'$.patient.reference')=?{p})"
            ));
            extra_params.push(SqliteParam::Text(patient.clone()));
            next_param += 1;
        }
        conditions.push(format!("({})", ors.join(" OR ")));
    }

    if !filters.group.is_empty() {
        let mut ors: Vec<String> = Vec::with_capacity(filters.group.len());
        for group in &filters.group {
            let p = next_param;
            ors.push(format!("json_extract(r.data,'$.group.reference')=?{p}"));
            extra_params.push(SqliteParam::Text(group.clone()));
            next_param += 1;
        }
        conditions.push(format!("({})", ors.join(" OR ")));
    }

    if conditions.is_empty() {
        return (base_sql.to_string(), extra_params);
    }

    let joined = conditions.join(" AND ");
    let sql = inject_before_order_by(base_sql, &format!(" AND {joined}"));
    (sql, extra_params)
}

/// Inserts `extra` before the trailing `ORDER BY` in `sql`, or appends it.
///
/// The compiler emits `\nORDER BY …` (newline-prefixed), so we search for
/// that pattern first; the space-prefixed variant is checked as a fallback for
/// any hand-crafted SQL.
fn inject_before_order_by(sql: &str, extra: &str) -> String {
    // Try newline-prefixed ORDER BY first (what the compiler generates).
    let search = ["\nORDER BY", " ORDER BY"];
    for pat in search {
        if let Some(pos) = sql.rfind(pat) {
            let mut s = sql.to_string();
            s.insert_str(pos, extra);
            return s;
        }
    }
    format!("{sql}{extra}")
}

// ============================================================================
// Typed parameter — same role as `PgParam` on the PostgreSQL runner.
// ============================================================================

/// Bound-parameter value for the SQLite runner. Mirrors [`super::ir::LitValue`]
/// plus a Text variant for runtime filter strings.
#[derive(Clone, Debug)]
enum SqliteParam {
    Text(String),
    Bool(bool),
    Int(i64),
    /// Decimal preserved as text — SQLite is dynamic-typed and accepts text
    /// for numeric comparisons.
    Decimal(String),
    Null,
}

impl SqliteParam {
    fn from_lit(v: &super::ir::LitValue) -> Self {
        match v {
            super::ir::LitValue::Null => SqliteParam::Null,
            super::ir::LitValue::Bool(b) => SqliteParam::Bool(*b),
            super::ir::LitValue::Int(n) => SqliteParam::Int(*n),
            super::ir::LitValue::Decimal(s) => SqliteParam::Decimal(s.clone()),
            super::ir::LitValue::Str(s) => SqliteParam::Text(s.clone()),
        }
    }
}

impl rusqlite::ToSql for SqliteParam {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, Value};
        Ok(match self {
            SqliteParam::Text(s) => ToSqlOutput::Borrowed(s.as_str().into()),
            SqliteParam::Bool(b) => ToSqlOutput::Owned(Value::Integer(if *b { 1 } else { 0 })),
            SqliteParam::Int(n) => ToSqlOutput::Owned(Value::Integer(*n)),
            // Bind as REAL so SQLite's type-affinity rules let the value
            // compare numerically against `json_extract` results (which are
            // INTEGER/REAL for JSON numbers). Binding as TEXT puts the
            // value in a different storage class and SQLite ranks any TEXT
            // as greater than any numeric value, breaking `<` / `>`.
            SqliteParam::Decimal(s) => match s.parse::<f64>() {
                Ok(n) => ToSqlOutput::Owned(Value::Real(n)),
                Err(_) => ToSqlOutput::Owned(Value::Text(s.clone())),
            },
            SqliteParam::Null => ToSqlOutput::Owned(Value::Null),
        })
    }
}

// ============================================================================
// Blocking row iterator → channel
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn stream_sqlite_rows(
    pool: &Pool<SqliteConnectionManager>,
    sql: &str,
    tenant_id: &str,
    resource_type: &str,
    extra_params: Vec<SqliteParam>,
    columns: &[String],
    limit: Option<usize>,
    tx: tokio::sync::mpsc::Sender<Result<ViewRow, SofError>>,
) {
    let conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.blocking_send(Err(SofError::Storage(format!(
                "failed to acquire SQLite connection: {e}"
            ))));
            return;
        }
    };

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.blocking_send(Err(SofError::Backend(format!(
                "failed to prepare SQL: {e}"
            ))));
            return;
        }
    };

    // Build the bound-parameter list: tenant_id, resource_type, then the
    // typed constants + runtime filters from `extra_params`.
    let mut all_params: Vec<SqliteParam> = Vec::with_capacity(2 + extra_params.len());
    all_params.push(SqliteParam::Text(tenant_id.to_string()));
    all_params.push(SqliteParam::Text(resource_type.to_string()));
    all_params.extend(extra_params);

    let row_iter = {
        match stmt.query_map(rusqlite::params_from_iter(all_params.iter()), |row| {
            map_sqlite_row(row, columns)
        }) {
            Ok(iter) => iter,
            Err(e) => {
                let _ = tx.blocking_send(Err(SofError::Backend(format!(
                    "query execution failed: {e}"
                ))));
                return;
            }
        }
    };

    let mut count = 0usize;
    for row_result in row_iter {
        if let Some(cap) = limit {
            if count >= cap {
                break;
            }
        }
        count += 1;

        let row = match row_result {
            Ok(map) => Ok(Value::Object(map)),
            Err(e) => Err(SofError::Backend(format!("row error: {e}"))),
        };

        if tx.blocking_send(row).is_err() {
            // Receiver dropped (client disconnected) — stop iterating
            break;
        }
    }

    debug!(
        runner = "sqlite-indb",
        rows = count,
        "in-DB view run complete"
    );
    // tx is dropped here, closing the ReceiverStream on the consumer side
}

fn map_sqlite_row(
    row: &rusqlite::Row<'_>,
    columns: &[String],
) -> rusqlite::Result<Map<String, Value>> {
    let mut map = Map::new();
    for (i, name) in columns.iter().enumerate() {
        let val = match row.get_ref(i)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => Value::from(n),
            ValueRef::Real(f) => {
                Value::from(serde_json::Number::from_f64(f).unwrap_or(serde_json::Number::from(0)))
            }
            ValueRef::Text(b) => {
                let s = String::from_utf8_lossy(b).into_owned();
                serde_json::from_str(&s).unwrap_or(Value::String(s))
            }
            ValueRef::Blob(b) => {
                let s = String::from_utf8_lossy(b).into_owned();
                serde_json::from_str(&s).unwrap_or(Value::String(s))
            }
        };
        if val != Value::Null {
            map.insert(name.clone(), val);
        }
    }
    Ok(map)
}
