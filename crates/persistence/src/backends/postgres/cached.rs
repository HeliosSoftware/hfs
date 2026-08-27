//! Executing fixed-text SQL through the connection's prepared-statement cache.
//!
//! `tokio_postgres::Client::execute("SQL…", …)` looks like it sends one message.
//! It does not. `&str` implements `ToStatement` as `ToStatementType::Query`,
//! which calls `client.prepare(sql)` first — so every call is Parse + Describe +
//! Sync (one round trip, and a full raw parse, parse analysis and plan on the
//! server) followed by Bind + Execute + Sync (a second round trip). The
//! statement is then dropped and closed again.
//!
//! On the benchmark's Postgres — 4 CPUs, shared — that parse work is charged to
//! the same four cores the inserts need, and `pg_stat_statements` does not even
//! show it: `total_exec_time` excludes parsing and (with `track_planning` off,
//! the default) planning. So the measured 8,384 s of import execution time and
//! 6,882 s of crud execution time are the cost *after* every one of those
//! statements had been parsed from scratch — 3.2M times for import, 7.6M for
//! crud.
//!
//! `deadpool_postgres::Client::prepare_cached` keeps the `Statement` in a
//! per-connection cache, so the second and later executions of the same SQL text
//! send Bind + Execute + Sync only. Postgres re-uses the parse tree, and after
//! five executions promotes it to a generic plan and stops planning too.
//!
//! Two rules make this safe to use here:
//!
//! 1. **Only fixed SQL text.** The cache is unbounded and keyed by the query
//!    string, so a `format!`-built statement (everything the search query
//!    builder emits) would grow it without limit, on every connection, and leak
//!    server-side prepared statements with it. Every call site below passes a
//!    string literal or a `LazyLock`-built constant.
//! 2. **Only where a generic plan is the plan anyway.** These are primary-key
//!    lookups, single-table deletes by their index prefix, and inserts — none of
//!    them has a parameter-dependent plan choice for a generic plan to get
//!    wrong.
//!
//! Named prepared statements live for the session (protocol docs: "Named
//! prepared statements … last until explicitly destroyed or the session ends"),
//! not for the transaction, so one created inside a bundle transaction survives
//! its rollback. The pool is also configured with deadpool's default
//! `RecyclingMethod::Fast`, which runs no recycling query — nothing issues
//! `DEALLOCATE ALL` or `DISCARD ALL` behind the cache's back.

use deadpool_postgres::Client;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Error, Row};

/// `Client::execute` against the connection's cached prepared statement.
pub(crate) async fn execute_cached(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<u64, Error> {
    let statement = client.prepare_cached(sql).await?;
    client.execute(&statement, params).await
}

/// `Client::query_opt` against the connection's cached prepared statement.
pub(crate) async fn query_opt_cached(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Option<Row>, Error> {
    let statement = client.prepare_cached(sql).await?;
    client.query_opt(&statement, params).await
}

/// `Client::query` against the connection's cached prepared statement.
pub(crate) async fn query_cached(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, Error> {
    let statement = client.prepare_cached(sql).await?;
    client.query(&statement, params).await
}
