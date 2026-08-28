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
//!
//! # The read path, and why rule 1 and rule 2 both had to be relaxed
//!
//! The two rules above kept the search path out of the cache entirely, so every
//! one of the search suite's 363,660 calls in a five-minute window still paid a
//! full Parse + Describe + Sync round trip and a fresh raw parse and parse
//! analysis on the server. `search_impl.rs` now uses [`query_dyn_cached`] /
//! [`query_one_dyn_cached`], which relax both rules under a specific
//! justification rather than by exception:
//!
//! **Rule 1 (fixed text) becomes a measured ceiling.** The query builder's
//! output is not one string, but it is not arbitrary either: what varies is the
//! predicate shape, the inlined `param_name` (bounded by the SearchParameter
//! registry), and the inlined `LIMIT` (`_count` is clamped to 1000 by
//! `extractors::pagination`). What is *genuinely* unbounded stays off the cached
//! path by construction — see `search_impl.rs` for the three call sites left on
//! `client.query(&str, …)` and why. Everything else is bounded by
//! `DEFAULT_STATEMENT_CACHE_MAX`.
//!
//! **Rule 2 (generic plan is the plan anyway) becomes false for search, so the
//! generic plan is taken away.** Caching a statement makes it persistent, and a
//! persistent statement is promoted to a generic plan after five executions.
//! Measured against a 21.8M-row `search_index` built to the benchmark's
//! proportions, `plan_cache_mode = force_generic_plan` versus
//! `force_custom_plan`, total buffers touched to answer the same page:
//!
//! | shape                                     | custom | generic | ratio |
//! |-------------------------------------------|--------|---------|-------|
//! | `Observation?patient=Patient/x`           |  1,499 | 6,005,788 | 4,006x |
//! | `Observation?patient:below=Patient/x`     |  1,469 | 6,005,788 | 4,088x |
//! | `Observation?category=laboratory,vitals`  |    464 | 2,453,618 | 5,288x |
//! | `Observation?code:text=…`                 | 2.2M   |    17.2M  |   7.7x |
//! | `Observation?code=sys|code` (join form)   | 69,736 |   601,668 |   8.6x |
//! | `Patient?address=Spring` (v30 range)      |  1,504 |     1,493 |   1.0x |
//! | token `=`, quantity, uri, date, `_id`, PK |   —    |     —     |   1.0x |
//!
//! The reference rows are the `value_reference = $3 OR value_reference LIKE $4
//! ESCAPE '\\'` form: under a custom plan the planner sees the pattern as a
//! `Const` and extracts prefix bounds for `idx_search_reference_pattern`; under
//! a generic plan the pattern is a `Param`, no bounds exist, and the scan reads
//! the whole `(tenant, type, param_name)` slice. This is the same defect seat J
//! removed from the string path in v30, on a predicate that still has it. The
//! OR-list row is a different mechanism with the same outcome: an `OR` of two
//! equalities cannot be an index condition either way, and only the custom plan
//! estimates it well enough to pick the `…_recent` index whose ordering lets the
//! `LIMIT` terminate early.
//!
//! End to end through this driver and pool, 8 concurrent clients, a ten-shape
//! search mix, five paired rounds on the same database (medians):
//!
//! | arm                                    | qps   | p50    |
//! |----------------------------------------|-------|--------|
//! | `client.query(&str, …)` (before)       | 6,616 |  959us |
//! | `prepare_cached`, `plan_cache_mode=auto` | *see below* | |
//! | `prepare_cached`, `force_custom_plan`  | 7,782 |  771us |
//!
//! +17.6% throughput, -19.6% p50. On the two generic-unsafe shapes alone,
//! measured separately because it does not belong in an average: 6,349 qps
//! before, **56.8 qps** with `prepare_cached` and the default `auto`, 7,626 qps
//! with `force_custom_plan`. A 112x collapse is what shipping the naive change
//! would have done to `Observation?patient=…`.
//!
//! So the pool ships `plan_cache_mode = force_custom_plan` in the startup packet
//! (`backend.rs`). Every plan stays byte-identical to the one this backend
//! produces today — today's throwaway statement is planned once, with its
//! parameter values in hand, which is a custom plan — and what the cache removes
//! is the parse, the parse analysis, the Describe and one of the two round
//! trips. Planning is still paid per execution. That is the conservative half of
//! the available win, deliberately: the other half needs the reference predicate
//! made sargable-under-generic (the same treatment v30 gave the string
//! predicate) before `auto` is safe to switch on.

use deadpool_postgres::Client;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Error, Row, Statement};

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

/// Upper bound on how many statements one connection may keep prepared.
///
/// `deadpool_postgres`' `StatementCache` is a plain `HashMap` keyed by the query
/// text: nothing evicts, so a caller that feeds it an unbounded set of texts
/// leaks on both sides of the wire — a `Statement` handle in the client and a
/// `CachedPlanSource` in the server backend. The read path *does* feed it a set
/// that is only organically bounded (see the module note above), so it needs a
/// ceiling rather than a promise.
///
/// Sized from measurement, not from taste. 500 of the search fast path's
/// statements prepared in one PostgreSQL 18.6 backend cost, per
/// `pg_backend_memory_contexts`:
///
/// | state                                    | total   | per statement |
/// |------------------------------------------|---------|---------------|
/// | baseline (idle backend)                  |  1.5 MB |               |
/// | 500 prepared, `force_custom_plan`        |   19 MB |     ~38 KB    |
/// | 500 prepared, `auto`, 7 executions each  |   37 MB |     ~74 KB    |
///
/// (The second row is what this backend runs; the third is what a cached
/// *generic* plan would add on top, and is another reason not to want one.)
///
/// At 38 KB a statement the ceiling costs at most `64 * 38 KB` = 2.4 MB per
/// connection. Against the default pool — `available_parallelism * 4`, clamped
/// to 16..=64 — that is at most 156 MB of Postgres backend memory; against the
/// benchmark's `HFS_PG_MAX_CONNECTIONS=256` it is at most 622 MB, on a host with
/// 11 GB. The *realistic* figure is far lower, because a connection only caches
/// the shapes it actually executes and the benchmark fires on the order of ten:
/// ~10 x 38 KB x 256 = ~97 MB. That is not free — it is page cache the 15.7 GB
/// working set would otherwise have — so the ceiling is deliberately tight and
/// `HFS_PG_STATEMENT_CACHE_MAX` exists to make it tighter without a rebuild.
///
/// `0` disables statement caching entirely: a kill switch that returns the
/// backend to `client.query(&str, …)` behaviour without a redeploy.
const DEFAULT_STATEMENT_CACHE_MAX: usize = 64;

/// The ceiling, from an `HFS_PG_STATEMENT_CACHE_MAX` value.
///
/// A value that does not parse is ignored rather than treated as `0`: a typo
/// must not silently switch statement caching off.
fn parse_statement_cache_max(raw: Option<&str>) -> usize {
    raw.and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_STATEMENT_CACHE_MAX)
}

/// `HFS_PG_STATEMENT_CACHE_MAX`, read once.
fn statement_cache_max() -> usize {
    static MAX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *MAX.get_or_init(|| {
        parse_statement_cache_max(std::env::var("HFS_PG_STATEMENT_CACHE_MAX").ok().as_deref())
    })
}

/// Prepares `sql` through the connection's cache, keeping that cache bounded.
///
/// The eviction is a flush, not an LRU, because `StatementCache` exposes only
/// `size()`, `clear()` and `remove(query, types)` — there is no ordering to
/// evict by, and reconstructing one outside the cache would mean keying
/// per-connection state off an `Arc` pointer that the pool may recycle out from
/// under it. A flush degrades gracefully: the worst case is that every call
/// re-prepares, which is exactly what this backend did before, plus one `clear`
/// of a `HashMap`. It cannot degrade into unbounded memory, which is the failure
/// that actually matters.
///
/// Returns `None` when caching is disabled, so the caller executes the text
/// directly.
async fn prepare_bounded(client: &Client, sql: &str) -> Result<Option<Statement>, Error> {
    let max = statement_cache_max();
    if max == 0 {
        return Ok(None);
    }
    if client.statement_cache.size() >= max {
        client.statement_cache.clear();
    }
    client.prepare_cached(sql).await.map(Some)
}

/// `Client::query` against the connection's cached prepared statement, for SQL
/// whose text is *built* rather than literal.
///
/// Separate from [`query_cached`] because the two carry different obligations:
/// that one is for a fixed string and needs no ceiling, this one is for the
/// query builder's output and does.
pub(crate) async fn query_dyn_cached(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, Error> {
    match prepare_bounded(client, sql).await? {
        Some(statement) => client.query(&statement, params).await,
        None => client.query(sql, params).await,
    }
}

/// `Client::query_one` against the connection's cached prepared statement, for
/// built SQL. See [`query_dyn_cached`].
pub(crate) async fn query_one_dyn_cached(
    client: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Row, Error> {
    match prepare_bounded(client, sql).await? {
        Some(statement) => client.query_one(&statement, params).await,
        None => client.query_one(sql, params).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_defaults_to_the_measured_one() {
        assert_eq!(parse_statement_cache_max(None), DEFAULT_STATEMENT_CACHE_MAX);
        assert_eq!(
            parse_statement_cache_max(Some("")),
            DEFAULT_STATEMENT_CACHE_MAX
        );
    }

    #[test]
    fn the_ceiling_is_overridable_and_zero_is_the_kill_switch() {
        assert_eq!(parse_statement_cache_max(Some("16")), 16);
        assert_eq!(parse_statement_cache_max(Some(" 0 ")), 0);
    }

    #[test]
    fn an_unparseable_ceiling_does_not_disable_caching() {
        // The failure mode to avoid is a typo parsing as 0 and silently
        // reverting the backend to a re-prepare per call.
        assert_eq!(
            parse_statement_cache_max(Some("sixty-four")),
            DEFAULT_STATEMENT_CACHE_MAX
        );
        assert_eq!(
            parse_statement_cache_max(Some("-1")),
            DEFAULT_STATEMENT_CACHE_MAX
        );
    }

    /// `DEFAULT_STATEMENT_CACHE_MAX` is a memory budget, not a taste. 500
    /// statements of the search fast path measured 19 MB of PostgreSQL 18.6
    /// backend memory (~38 KB each) under `force_custom_plan`; against the
    /// benchmark's 256-connection pool the ceiling is worth ~622 MB of worst
    /// case on an 11 GB host. Raising it is a decision about that host, so this
    /// pins the number against an absent-minded edit.
    #[test]
    fn the_ceiling_stays_where_the_memory_measurement_put_it() {
        assert_eq!(DEFAULT_STATEMENT_CACHE_MAX, 64);
    }
}
