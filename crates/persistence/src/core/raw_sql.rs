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

    /// A supplied parameter is the wrong shape or an unsupported type.
    #[error("parameter binding error: {0}")]
    Parameter(String),
}

/// A value bound to a named parameter when executing a query.
///
/// Per the SQL-on-FHIR v2 spec MUST: parameter values must be safely bound
/// by the driver, not interpolated into the SQL string. Implementations of
/// [`RawSqlRunner`] map these variants to the backend's native bound-value
/// representation; backends MUST NOT format these into the SQL string.
#[derive(Debug, Clone)]
pub enum BoundValue {
    /// `valueBoolean`
    Bool(bool),
    /// `valueInteger` / `valuePositiveInt` / `valueUnsignedInt`
    Int(i64),
    /// `valueDecimal`
    Decimal(f64),
    /// `valueString` / `valueCode` / `valueId` / `valueUri` / `valueOid` /
    /// `valueCanonical` / `valueUrl`
    Text(String),
    /// `valueDate` (YYYY-MM-DD)
    Date(chrono::NaiveDate),
    /// `valueDateTime` / `valueInstant`
    DateTime(chrono::DateTime<chrono::Utc>),
    /// SQL NULL
    Null,
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
    ///
    /// `named_params` supplies values for any `:name` placeholders that appear
    /// in `sql`. The runner MUST bind these via the driver's parameter-binding
    /// API (never via string interpolation) to satisfy the SQL-on-FHIR v2
    /// MUST that parameters be safe against injection.
    async fn run_query(
        &self,
        tenant_id: &str,
        sql: &str,
        named_params: &[(String, BoundValue)],
        max_rows: usize,
        timeout_secs: u64,
    ) -> Result<Vec<SqlRow>, RawSqlError>;

    /// Human-readable name for log messages and diagnostics.
    fn runner_name(&self) -> &'static str;
}

/// Rewrites `:name` placeholders in `sql` to positional `$N` (Postgres) or
/// `?N` (SQLite) placeholders, returning the rewritten SQL plus the order in
/// which parameters must be bound. The `start` index is the first positional
/// index to allocate to user-supplied parameters (Postgres uses `$1` for the
/// tenant id, so user params start at `$2`).
///
/// Behaviour:
/// - `::` (Postgres cast) is skipped — it's not a placeholder.
/// - Multi-use of the same `:name` is supported and reuses the same index.
/// - `:` followed by whitespace or end-of-input is left untouched.
/// - Returns the rewritten SQL and a `Vec<String>` of param names in the
///   order their positional placeholders were assigned. The caller is
///   responsible for looking up each name in the supplied parameter map
///   and binding the value (driver-side, not via interpolation).
pub fn rewrite_named_placeholders(
    sql: &str,
    is_postgres: bool,
    start: usize,
) -> (String, Vec<String>) {
    let mut out = String::with_capacity(sql.len() + 8);
    let mut order: Vec<String> = Vec::new();
    let bytes = sql.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        // Skip Postgres `::` cast operator entirely.
        if b == b':' && i + 1 < bytes.len() && bytes[i + 1] == b':' {
            out.push_str("::");
            i += 2;
            continue;
        }
        if b == b':' {
            // Lookahead for an identifier: [A-Za-z_][A-Za-z0-9_]*
            let mut j = i + 1;
            while j < bytes.len() {
                let c = bytes[j];
                if c == b'_' || c.is_ascii_alphanumeric() {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > i + 1 {
                let name = &sql[i + 1..j];
                // First-char-must-not-be-digit check.
                if !name.as_bytes()[0].is_ascii_digit() {
                    // Find existing index or allocate a new one.
                    let idx = match order.iter().position(|n| n == name) {
                        Some(p) => start + p,
                        None => {
                            order.push(name.to_string());
                            start + order.len() - 1
                        }
                    };
                    if is_postgres {
                        out.push('$');
                    } else {
                        out.push('?');
                    }
                    out.push_str(&idx.to_string());
                    i = j;
                    continue;
                }
            }
        }
        // Default: copy one byte through.
        // The source is valid UTF-8; pushing one byte at a time may split a
        // multi-byte sequence, so we step by char boundary instead.
        let ch = sql[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    (out, order)
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

    #[test]
    fn test_rewrite_placeholders_postgres() {
        let (sql, order) = rewrite_named_placeholders(
            "SELECT * FROM resources WHERE id = :pid AND tenant = :pid",
            true,
            2,
        );
        assert_eq!(sql, "SELECT * FROM resources WHERE id = $2 AND tenant = $2");
        assert_eq!(order, vec!["pid".to_string()]);
    }

    #[test]
    fn test_rewrite_placeholders_sqlite_distinct_params() {
        let (sql, order) = rewrite_named_placeholders(
            "SELECT * FROM resources WHERE id = :pid AND last_updated > :since",
            false,
            2,
        );
        assert_eq!(
            sql,
            "SELECT * FROM resources WHERE id = ?2 AND last_updated > ?3"
        );
        assert_eq!(order, vec!["pid".to_string(), "since".to_string()]);
    }

    #[test]
    fn test_rewrite_placeholders_skips_pg_cast() {
        let (sql, _order) =
            rewrite_named_placeholders("SELECT id::text FROM resources WHERE id = :pid", true, 2);
        assert!(sql.contains("id::text"));
        assert!(sql.contains("$2"));
    }
}
