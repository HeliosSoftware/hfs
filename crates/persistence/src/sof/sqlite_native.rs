//! Running a SQL Query Library inside the primary SQLite database (#2902).
//!
//! The statement comes from [`super::native_query::compose`]. It runs on a
//! pooled connection of the primary database under two per-statement hooks,
//! both removed before the connection goes back to the pool:
//!
//! - an authorizer that allows only reads (the layer below the SQL check of
//!   `helios_sof::sqlquery::native`): no write, DDL, `PRAGMA`, `ATTACH`, no
//!   read of the `sqlite_*` schema tables, no `load_extension`;
//! - a progress handler that interrupts the statement at its time limit.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use helios_fhir::FhirVersion;
use helios_sof::sqlquery::{QueryResult, SqlQueryError, read_query_result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use tracing::{debug, trace};

use crate::core::sof_runner::NativeSqlQuery;
use crate::tenant::TenantContext;

use super::compiler::SqlDialect;
use super::native_query::{ComposedQuery, NativeParam, compose};
use super::sqlite::{SqliteParam, load_group_documents};

/// SQLite virtual-machine steps between two checks of the time limit.
const PROGRESS_STEPS: i32 = 10_000;

/// Runs `query` for `tenant` as one read-only statement.
pub(super) async fn run_sql_query(
    pool: &Pool<SqliteConnectionManager>,
    fhir_version: FhirVersion,
    tenant: &TenantContext,
    query: NativeSqlQuery,
) -> Result<QueryResult, SqlQueryError> {
    let tenant_id = tenant.tenant_id().to_string();
    let stored_version = query.filters.fhir_version.map(|v| v.as_mime_param());
    let composed = compose(
        SqlDialect::Sqlite,
        fhir_version,
        &tenant_id,
        &query,
        |refs| load_group_documents(pool.clone(), tenant_id.clone(), stored_version, refs),
    )
    .await?;
    debug!(runner = "sqlite-indb", tenant = %tenant_id, relations = query.relations.len(), "running SQL query in the primary database");
    trace!(runner = "sqlite-indb", sql = %composed.sql, "composed SQL query");
    let pool = pool.clone();
    let max_rows = query.max_rows;
    let timeout = query.timeout;
    tokio::task::spawn_blocking(move || execute(&pool, composed, max_rows, timeout))
        .await
        .map_err(|e| SqlQueryError::Internal(format!("sqlquery worker panicked: {e}")))?
}

/// Removes the statement's hooks when the statement is done, however it ends,
/// so the pooled connection is returned as it was taken.
struct Hooks<'c>(&'c mut rusqlite::Connection);

impl Drop for Hooks<'_> {
    fn drop(&mut self) {
        let _ = self
            .0
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        let _ = self.0.progress_handler(0, None::<fn() -> bool>);
    }
}

fn execute(
    pool: &Pool<SqliteConnectionManager>,
    composed: ComposedQuery,
    max_rows: usize,
    timeout: Duration,
) -> Result<QueryResult, SqlQueryError> {
    let mut conn = pool.get().map_err(|e| {
        SqlQueryError::SourceUnavailable(format!("failed to acquire SQLite connection: {e}"))
    })?;

    let refused: Arc<OnceLock<String>> = Arc::default();
    let record = Arc::clone(&refused);
    let read_only = move |ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Select | AuthAction::Recursive => Authorization::Allow,
        AuthAction::Read { table_name, .. } if !table_name.starts_with("sqlite_") => {
            Authorization::Allow
        }
        AuthAction::Function { function_name }
            if !function_name.eq_ignore_ascii_case("load_extension") =>
        {
            Authorization::Allow
        }
        other => {
            let _ = record.set(refusal(&other));
            Authorization::Deny
        }
    };
    let hooks = Hooks(&mut conn);
    // Fail closed: if the authorizer cannot be installed, run nothing.
    hooks.0.authorizer(Some(read_only))?;
    let deadline = Instant::now() + timeout;
    let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&timed_out);
    hooks.0.progress_handler(
        PROGRESS_STEPS,
        Some(move || {
            let over = Instant::now() >= deadline;
            if over {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            over
        }),
    )?;

    let result = run(hooks.0, &composed, max_rows);
    drop(hooks);
    if let Some(message) = refused.get() {
        return Err(SqlQueryError::NotSelect(message.clone()));
    }
    if timed_out.load(std::sync::atomic::Ordering::Relaxed) && result.is_err() {
        return Err(SqlQueryError::Timeout {
            secs: timeout.as_secs(),
        });
    }
    result
}

fn run(
    conn: &rusqlite::Connection,
    composed: &ComposedQuery,
    max_rows: usize,
) -> Result<QueryResult, SqlQueryError> {
    let mut stmt = conn.prepare(&composed.sql)?;
    // SQLite has no authorizer action for VACUUM; a prepared statement that
    // would write reports itself as not read-only.
    if !stmt.readonly() {
        return Err(SqlQueryError::NotSelect(
            "only SELECT queries are allowed".to_string(),
        ));
    }
    for (index, param) in composed.params.iter().enumerate() {
        match param {
            NativeParam::Runtime(value) => {
                stmt.raw_bind_parameter(index + 1, SqliteParam::from_runtime(value.clone()))?
            }
            NativeParam::User(value) => stmt.raw_bind_parameter(index + 1, value)?,
            NativeParam::Json(text) => stmt.raw_bind_parameter(index + 1, text)?,
        }
    }
    read_query_result(&mut stmt, max_rows)
}

fn refusal(action: &AuthAction<'_>) -> String {
    let what = match action {
        AuthAction::Read { table_name, .. } => {
            return format!("table '{table_name}' cannot be read by a SQL Query");
        }
        AuthAction::Function { function_name } => {
            return format!("function '{function_name}' is not available to SQL Queries");
        }
        AuthAction::Insert { table_name } if table_name.starts_with("sqlite_") => "CREATE",
        AuthAction::Delete { table_name } if table_name.starts_with("sqlite_") => "DROP",
        AuthAction::Insert { .. } => "INSERT",
        AuthAction::Update { .. } => "UPDATE",
        AuthAction::Delete { .. } => "DELETE",
        AuthAction::Pragma { .. } => "PRAGMA",
        AuthAction::Attach { .. } => "ATTACH",
        AuthAction::Detach { .. } => "DETACH",
        _ => "non-SELECT",
    };
    format!("only SELECT queries are allowed; {what} statements are not permitted")
}
