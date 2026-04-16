//! PostgreSQL implementation of [`RawSqlRunner`](crate::core::RawSqlRunner).
//!
//! Opens a **read-only** connection to the Postgres database on each query
//! (using `HFS_SOF_READONLY_URL`). The connection is not pooled — it is
//! purpose-built for ad-hoc queries and is closed when the query finishes.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio_postgres::NoTls;

use crate::core::raw_sql::{RawSqlError, RawSqlRunner, SqlRow, wrap_with_tenant_cte};

/// Executes read-only SQL queries against a PostgreSQL database.
pub struct PgRawRunner {
    /// Full Postgres connection string, e.g.
    /// `postgres://readonly_user:pass@host/db`.
    connection_string: String,
}

impl PgRawRunner {
    /// Creates a new runner using the given Postgres connection string.
    pub fn new(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
        }
    }
}

#[async_trait]
impl RawSqlRunner for PgRawRunner {
    async fn run_query(
        &self,
        tenant_id: &str,
        sql: &str,
        max_rows: usize,
        timeout_secs: u64,
    ) -> Result<Vec<SqlRow>, RawSqlError> {
        let conn_str = self.connection_string.clone();
        let tenant_id = tenant_id.to_string();
        let wrapped_sql = wrap_with_tenant_cte(sql, true);

        let query_fut =
            async move { execute_pg_query(&conn_str, &tenant_id, &wrapped_sql, max_rows).await };

        tokio::time::timeout(Duration::from_secs(timeout_secs), query_fut)
            .await
            .map_err(|_| RawSqlError::Timeout { secs: timeout_secs })?
    }

    fn runner_name(&self) -> &'static str {
        "postgres-raw"
    }
}

// ============================================================================
// Async helper
// ============================================================================

async fn execute_pg_query(
    conn_str: &str,
    tenant_id: &str,
    wrapped_sql: &str,
    max_rows: usize,
) -> Result<Vec<SqlRow>, RawSqlError> {
    let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
        .await
        .map_err(|e| RawSqlError::Connection(e.to_string()))?;

    // Drive the connection in the background; errors are non-fatal for the
    // caller since the query will fail on its own if the connection drops.
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let rows = client
        .query(wrapped_sql, &[&tenant_id])
        .await
        .map_err(|e| RawSqlError::Query(e.to_string()))?;

    if rows.len() > max_rows {
        return Err(RawSqlError::RowLimitExceeded { max_rows });
    }

    rows.iter().map(pg_row_to_json).collect()
}

fn pg_row_to_json(row: &tokio_postgres::Row) -> Result<SqlRow, RawSqlError> {
    let mut map = serde_json::Map::new();

    for (i, col) in row.columns().iter().enumerate() {
        let name = col.name().to_string();
        let val = pg_col_to_json(row, i, col.type_().name()).unwrap_or(Value::Null);
        map.insert(name, val);
    }

    Ok(Value::Object(map))
}

/// Converts a single Postgres column value to a `serde_json::Value`.
///
/// Unknown or unconvertible types fall back to `None` (mapped to `Null` by
/// the caller).
fn pg_col_to_json(row: &tokio_postgres::Row, idx: usize, type_name: &str) -> Option<Value> {
    match type_name {
        "bool" => row
            .try_get::<_, Option<bool>>(idx)
            .ok()
            .flatten()
            .map(Value::Bool),
        "int2" => row
            .try_get::<_, Option<i16>>(idx)
            .ok()
            .flatten()
            .map(|n| Value::Number(n.into())),
        "int4" | "oid" => row
            .try_get::<_, Option<i32>>(idx)
            .ok()
            .flatten()
            .map(|n| Value::Number(n.into())),
        "int8" => row
            .try_get::<_, Option<i64>>(idx)
            .ok()
            .flatten()
            .map(|n| Value::Number(n.into())),
        "float4" => row
            .try_get::<_, Option<f32>>(idx)
            .ok()
            .flatten()
            .and_then(|n| serde_json::Number::from_f64(n as f64).map(Value::Number)),
        "float8" | "numeric" => row
            .try_get::<_, Option<f64>>(idx)
            .ok()
            .flatten()
            .and_then(|n| serde_json::Number::from_f64(n).map(Value::Number)),
        "text" | "varchar" | "bpchar" | "char" | "name" => row
            .try_get::<_, Option<String>>(idx)
            .ok()
            .flatten()
            .map(Value::String),
        "uuid" => row
            .try_get::<_, Option<uuid::Uuid>>(idx)
            .ok()
            .flatten()
            .map(|u| Value::String(u.to_string())),
        "timestamptz" => row
            .try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(idx)
            .ok()
            .flatten()
            .map(|dt| Value::String(dt.to_rfc3339())),
        "timestamp" => row
            .try_get::<_, Option<chrono::NaiveDateTime>>(idx)
            .ok()
            .flatten()
            .map(|dt| Value::String(dt.to_string())),
        "date" => row
            .try_get::<_, Option<chrono::NaiveDate>>(idx)
            .ok()
            .flatten()
            .map(|d| Value::String(d.to_string())),
        "json" | "jsonb" => row
            .try_get::<_, Option<serde_json::Value>>(idx)
            .ok()
            .flatten(),
        _ => {
            // Generic fallback: try to get as String.
            row.try_get::<_, Option<String>>(idx)
                .ok()
                .flatten()
                .map(Value::String)
        }
    }
    .or(Some(Value::Null))
}
