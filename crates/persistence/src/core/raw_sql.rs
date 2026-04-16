//! Raw SQL query execution abstraction for `$sql-query-run`.
//!
//! Only backends that advertise [`BackendCapability::RawSqlQuery`] provide a
//! runner. The handler layer validates that the submitted SQL is a plain
//! `SELECT` before calling [`RawSqlRunner::run_query`].

use async_trait::async_trait;
use serde_json::Value;

/// A single output row: a flat JSON object with column names as keys.
pub type SqlRow = Value;

/// Errors returned by [`RawSqlRunner::run_query`].
#[derive(Debug, thiserror::Error)]
pub enum RawSqlError {
    /// The runner could not connect to the read-only database.
    #[error("connection error: {0}")]
    Connection(String),

    /// The database rejected or failed to execute the query.
    #[error("query error: {0}")]
    Query(String),

    /// The query did not finish within the permitted timeout.
    #[error("query timed out after {secs}s")]
    Timeout {
        /// The timeout that was exceeded.
        secs: u64,
    },

    /// The result set exceeded the configured row cap.
    #[error("result set exceeds the {max_rows}-row limit")]
    RowLimitExceeded {
        /// The cap that was exceeded.
        max_rows: usize,
    },
}

/// Executes raw SQL queries against the FHIR resource store in read-only mode.
///
/// # Security
///
/// Implementations are responsible for:
/// - Opening a **read-only** connection (no DDL / DML privilege).
/// - Injecting a **tenant boundary** so that the caller can only see rows
///   belonging to their tenant.  The standard mechanism is a CTE that shadows
///   the `resources` table:
///   ```sql
///   WITH resources AS (
///     SELECT * FROM resources WHERE tenant_id = $1 AND is_deleted = false
///   )
///   <user_sql>
///   ```
/// - Enforcing the `max_rows` cap and `timeout_secs` deadline.
///
/// # Object safety
///
/// The trait is intentionally object-safe so it can be stored as
/// `Arc<dyn RawSqlRunner>` inside `AppState`.
#[async_trait]
pub trait RawSqlRunner: Send + Sync {
    /// Execute `sql` scoped to `tenant_id` and return at most `max_rows` rows.
    ///
    /// The SQL must already have been validated as a plain `SELECT` by the
    /// caller.  The runner wraps it in a tenant-boundary CTE before execution.
    async fn run_query(
        &self,
        tenant_id: &str,
        sql: &str,
        max_rows: usize,
        timeout_secs: u64,
    ) -> Result<Vec<SqlRow>, RawSqlError>;

    /// Human-readable name for log messages and diagnostics.
    fn runner_name(&self) -> &'static str;
}

// ============================================================================
// Shared helper
// ============================================================================

/// Wraps `user_sql` in a tenant-filtering CTE that shadows the `resources`
/// table, ensuring the query can only see rows for `tenant_id`.
///
/// - `is_postgres = true`  → uses `$1` parameter and `false` for is_deleted.
///   PostgreSQL CTEs can safely name the CTE `resources` while referencing the
///   real table inside the body — the body resolves names against the real schema.
/// - `is_postgres = false` → uses `?1` parameter and `0` for is_deleted (SQLite).
///   SQLite CTEs **cannot** shadow a real table inside their own body (it would
///   create a circular reference).  Instead we use a two-step approach:
///   ```sql
///   WITH _hfs_r AS (SELECT * FROM resources WHERE ...),
///        resources AS (SELECT * FROM _hfs_r)
///   ```
///   `_hfs_r` references the real table; `resources` shadows it for user SQL.
///
/// If `user_sql` already begins with a `WITH` clause (CTEs), the tenant CTEs
/// are prepended to the CTE list so they take effect for the entire query.
pub fn wrap_with_tenant_cte(user_sql: &str, is_postgres: bool) -> String {
    let trimmed = user_sql.trim();

    // Detect a leading WITH clause (case-insensitive).
    let starts_with_with = trimmed.len() >= 4
        && trimmed[..4].eq_ignore_ascii_case("with")
        && (trimmed
            .as_bytes()
            .get(4)
            .copied()
            .is_some_and(|b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r'));

    if is_postgres {
        let tenant_cte = "WITH resources AS (\
             SELECT * FROM resources \
             WHERE tenant_id = $1 AND is_deleted = false\
             )"
        .to_string();
        if starts_with_with {
            format!("{tenant_cte},{}", &trimmed[4..])
        } else {
            format!("{tenant_cte} {trimmed}")
        }
    } else {
        // SQLite two-step: avoid self-referential CTE.
        let tenant_ctes = "WITH _hfs_r AS (\
             SELECT * FROM resources \
             WHERE tenant_id = ?1 AND is_deleted = 0\
             ),\
             resources AS (SELECT * FROM _hfs_r)";

        if starts_with_with {
            // Prepend our two CTEs before the user's CTE list.
            // trimmed[4..] strips the leading "WITH", leaving " cte_name AS ..."
            format!("{},{}", tenant_ctes, &trimmed[4..])
        } else {
            format!("{tenant_ctes} {trimmed}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_plain_select_postgres() {
        let sql = "SELECT id FROM resources WHERE resource_type = 'Patient'";
        let wrapped = wrap_with_tenant_cte(sql, true);
        assert!(wrapped.starts_with("WITH resources AS ("));
        assert!(wrapped.contains("tenant_id = $1"));
        assert!(wrapped.contains("is_deleted = false"));
        assert!(wrapped.ends_with("SELECT id FROM resources WHERE resource_type = 'Patient'"));
    }

    #[test]
    fn test_wrap_plain_select_sqlite() {
        let sql = "SELECT id FROM resources";
        let wrapped = wrap_with_tenant_cte(sql, false);
        assert!(wrapped.contains("tenant_id = ?1"));
        assert!(wrapped.contains("is_deleted = 0"));
        // SQLite two-step: must use _hfs_r as intermediate to avoid circular reference
        assert!(
            wrapped.contains("_hfs_r"),
            "SQLite CTE should use _hfs_r alias"
        );
        assert!(wrapped.contains("resources AS (SELECT * FROM _hfs_r)"));
    }

    #[test]
    fn test_wrap_with_existing_cte() {
        let sql = "WITH obs AS (SELECT * FROM resources WHERE resource_type = 'Observation') SELECT id FROM obs";
        let wrapped = wrap_with_tenant_cte(sql, true);
        // Our tenant CTE comes first
        assert!(wrapped.starts_with("WITH resources AS ("));
        // Then user's CTE
        assert!(wrapped.contains(", obs AS ("));
        assert!(wrapped.contains("SELECT id FROM obs"));
    }

    #[test]
    fn test_wrap_with_lowercase_with() {
        let sql = "with patients as (select * from resources where resource_type = 'Patient') select * from patients";
        let wrapped = wrap_with_tenant_cte(sql, true);
        assert!(wrapped.starts_with("WITH resources AS ("));
        assert!(wrapped.contains(", patients as ("));
    }
}
