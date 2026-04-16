//! SQLite implementation of [`RawSqlRunner`](crate::core::RawSqlRunner).
//!
//! Opens a **read-only** connection to the SQLite file on each query and
//! executes the tenant-wrapped SQL via `tokio::task::spawn_blocking`.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::core::raw_sql::{RawSqlError, RawSqlRunner, SqlRow};

/// Executes read-only SQL queries against a SQLite database.
///
/// Each `run_query` call opens a fresh read-only connection.  Tenant isolation
/// is enforced by creating a **temporary table** named `resources` that shadows
/// the real table: SQLite resolves the `temp` schema before `main`, so any
/// `FROM resources` in the user SQL automatically queries the tenant-scoped
/// view.  The temp table is populated from `main.resources` with the tenant
/// filter applied.
///
/// SQLite does **not** support CTEs that shadow a real table by the same name
/// (it reports "circular reference"), so the temp-table approach is used
/// instead of a CTE.
pub struct SqliteRawRunner {
    /// Absolute or relative path to the SQLite database file.
    db_path: String,
}

impl SqliteRawRunner {
    /// Creates a new runner that opens `db_path` in read-only mode.
    pub fn new(db_path: impl Into<String>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }
}

#[async_trait]
impl RawSqlRunner for SqliteRawRunner {
    async fn run_query(
        &self,
        tenant_id: &str,
        sql: &str,
        max_rows: usize,
        timeout_secs: u64,
    ) -> Result<Vec<SqlRow>, RawSqlError> {
        let db_path = self.db_path.clone();
        let tenant_id = tenant_id.to_string();
        let user_sql = sql.to_string();

        let blocking = tokio::task::spawn_blocking(move || {
            execute_sqlite_query(&db_path, &tenant_id, &user_sql, max_rows)
        });

        tokio::time::timeout(Duration::from_secs(timeout_secs), blocking)
            .await
            .map_err(|_| RawSqlError::Timeout { secs: timeout_secs })?
            .map_err(|e| RawSqlError::Query(format!("task join error: {e}")))?
    }

    fn runner_name(&self) -> &'static str {
        "sqlite-raw"
    }
}

// ============================================================================
// Blocking helper — runs inside spawn_blocking
// ============================================================================

fn execute_sqlite_query(
    db_path: &str,
    tenant_id: &str,
    user_sql: &str,
    max_rows: usize,
) -> Result<Vec<SqlRow>, RawSqlError> {
    // Open a fresh read-only connection via URI.
    let uri = format!("file:{}?mode=ro", db_path);
    let conn = rusqlite::Connection::open_with_flags(
        &uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| RawSqlError::Connection(e.to_string()))?;

    // Tenant isolation: create a TEMP TABLE named `resources` that shadows
    // main.resources.  SQLite searches `temp` before `main`, so every
    // `FROM resources` in user SQL automatically targets the filtered copy.
    conn.execute(
        "CREATE TEMP TABLE resources AS \
         SELECT * FROM main.resources WHERE tenant_id = ?1 AND is_deleted = 0",
        rusqlite::params![tenant_id],
    )
    .map_err(|e| RawSqlError::Query(format!("failed to create tenant isolation view: {e}")))?;

    let mut stmt = conn
        .prepare(user_sql)
        .map_err(|e| RawSqlError::Query(e.to_string()))?;

    // Collect column names before iterating rows.
    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    // No extra parameters — the tenant filter was applied above.
    let rows = stmt
        .query_map([], |row| {
            let mut obj = serde_json::Map::new();
            for (i, name) in column_names.iter().enumerate() {
                let val: rusqlite::types::Value = row.get(i)?;
                obj.insert(name.clone(), sqlite_value_to_json(val));
            }
            Ok(Value::Object(obj))
        })
        .map_err(|e| RawSqlError::Query(e.to_string()))?;

    let mut result = Vec::new();
    for row_result in rows {
        if result.len() >= max_rows {
            return Err(RawSqlError::RowLimitExceeded { max_rows });
        }
        result.push(row_result.map_err(|e| RawSqlError::Query(e.to_string()))?);
    }

    Ok(result)
}

fn sqlite_value_to_json(val: rusqlite::types::Value) -> Value {
    match val {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(n) => Value::Number(n.into()),
        rusqlite::types::Value::Real(f) => {
            serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
        }
        rusqlite::types::Value::Text(s) => {
            // If the text looks like a JSON object or array, parse it inline.
            if (s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']'))
            {
                serde_json::from_str(&s).unwrap_or(Value::String(s))
            } else {
                Value::String(s)
            }
        }
        rusqlite::types::Value::Blob(b) => {
            // Encode blobs as lowercase hex strings.
            Value::String(
                b.iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            )
        }
    }
}
