//! PostgreSQL search index writer implementation.
//!
//! # Where `search_index` rows come from, and what each one costs
//!
//! `INSERT INTO search_index` is the largest statement in the benchmark on both
//! write suites — **86% of import's Postgres execution time** and 4,906.5 s of
//! crud's 10,935 s (45%). Its cost is arithmetic:
//!
//! ```text
//! rows  x  indexes each row enters  x  cost per btree insert
//! ```
//!
//! Earlier rounds attacked the second and third factors (partial predicates,
//! index replacement, batching the wire form). This is a census of the first,
//! plus a measured cost for the other two, so a future round can price a change
//! before making it.
//!
//! ## The census
//!
//! Measured on a locally reproduced slice of the benchmark's Synthea corpus —
//! `bulk_1k`, the two `*Information` seed bundles plus the first 40 patient
//! bundles, imported through the ordinary transaction path into PostgreSQL 18.6
//! at schema v34. **59,167 resources, 822,992 index rows, 13.91 rows per
//! resource**, which is the same 13.9 the full 1,632,067-resource corpus
//! reports, so the shape carries.
//!
//! By row kind:
//!
//! ```text
//! kind        rows      share   indexes entered
//! reference   289,225   35.1%   2   (resource, reference_pattern)
//! token       274,654   33.4%   3   (resource, token_code, token_code_recent)
//! date         75,068    9.1%   3   (resource, date, date_recent)
//! composite    66,186    8.0%   5   (resource, token_code, token_code_recent, composite_*)
//! uri          42,353    5.2%   2   (resource, uri)
//! quantity     39,576    4.8%   4   (resource, quantity, quantity_recent, quantity_canonical)
//! string       35,930    4.4%   4   (resource, string, string_folded_pattern, string_trgm)
//! contained         0    0.0%
//! ```
//!
//! By resource type (rows / share / rows per resource):
//!
//! ```text
//! Observation      365,220  44.4%  15.5      Practitioner      19,251   2.3%  23.0
//! DiagnosticReport  69,447   8.4%  13.5      Condition         18,682   2.3%  10.8
//! EOB               64,999   7.9%  11.0      Organization      12,555   1.5%  15.0
//! Provenance        56,058   6.8%  1401.5    Location          10,883   1.3%  13.0
//! Claim             43,808   5.3%   7.4      PractitionerRole   9,207   1.1%  11.0
//! DocumentReference 39,120   4.8%  16.0      Immunization       3,948   0.5%   6.0
//! Procedure         34,925   4.2%   9.2      CarePlan           1,606   0.2%  13.4
//! Encounter         34,734   4.2%  14.2      Patient            1,314   0.2%  32.9
//! MedicationRequest 34,640   4.2%  10.0      (24 types in all)
//! ```
//!
//! The ten largest single `(resource_type, param_name)` cells:
//!
//! ```text
//! Provenance  target                   55,778  6.78%   1,394 per resource
//! Observation combo-code               34,683  4.21%
//! Observation code                     23,624  2.87%
//! Observation patient                  23,586  2.87%
//! Observation category                 23,586  2.87%
//! Observation subject                  23,586  2.87%
//! Observation encounter                23,586  2.87%
//! Observation date                     23,586  2.87%
//! Observation status                   23,586  2.87%
//! Observation _profile                 20,188  2.45%
//! ```
//!
//! ## What the census says
//!
//! Nearly all of it is load-bearing, and the reason is that this backend reads
//! `search_index` by `param_name` string equality taken straight from the query
//! string. Every row whose `param_name` a client can type is reachable by
//! construction, so "unreachable rows" is a much smaller category than the
//! `13.9` invites one to assume. The three things that are *not* load-bearing,
//! all removed:
//!
//! - `_id` / `_lastUpdated`, restatements of `resources` columns — removed
//!   earlier, see [`PARAMS_ANSWERED_FROM_RESOURCES`]; ~1.63M rows on the full
//!   corpus, and none remain in the census above.
//! - Rows a resource repeats verbatim — 8,265 rows, **1.00%**. See
//!   [`dedup_rows`].
//! - `phonetic`, which stored the same strings as `name` and matched them the
//!   same way — 2,649 rows, **0.32%** here (Patient 138, Practitioner 1,674,
//!   Organization 837), but 5 of the 132 rows the crud suite's nine seed
//!   resources produce, i.e. **3.8% of that suite's index write**. See
//!   `helios_fhir::search::loader::UNIMPLEMENTED_SPEC_PARAM_CODES`.
//!
//! Everything else in the table is a parameter a search can name. `combo-code`
//! duplicates `code` for a component-less Observation and `patient` duplicates
//! `subject` when the subject is a Patient, but each is separately queryable, so
//! collapsing them is a read-path change (rewriting `combo-code=X` into a union
//! over `code` and `component-code`), not a write-path one.
//!
//! ## What a row costs
//!
//! Measured on the same database, inserting 35,000 real rows of each kind into a
//! copy of `search_index` carrying the exact v34 index set dumped from
//! `pg_indexes`, five interleaved rounds, medians:
//!
//! ```text
//! kind        indexed    heap-only   index-only   us/row   indexes   us/index
//! uri          221.7 ms    31.6 ms     190.1 ms     5.43       2       2.72
//! reference    229.0       31.2        197.8        5.65       2       2.83
//! date         354.4       27.6        326.8        9.34       3       3.11
//! token        376.8       32.3        344.5        9.84       3       3.28
//! quantity     513.9       30.9        483.0       13.80       4       3.45
//! string       530.7       28.5        502.2       14.35       4       3.59
//! composite    767.9       34.7        733.2       20.95       5       4.19
//! ```
//!
//! So: **~0.85 us of heap per row, and ~3.2 us per index entry**. The corpus mix
//! above averages 2.85 index entries per row, predicting 10.0 us/row; a mixed
//! 200,000-row sample measures 10.3. A removed row is worth its own kind's
//! number, which is why a `string` row is worth 2.5 `reference` rows.
//!
//! ## The predicate floor is not a lever — measured
//!
//! `search_index` carries 18 indexes, 17 of them partial, so every inserted row
//! evaluates 17 predicates it will usually fail. Inserting 200,000 rows whose
//! value columns are all NULL — rows that enter `idx_search_resource` and
//! nothing else — into three arms, five interleaved rounds, medians:
//!
//! ```text
//! arm                                     total     delta      per row
//! heap only (0 indexes)                   120.5 ms
//! + idx_search_resource only              643.6 ms  +523.1 ms   2.62 us
//! + the other 17 partial indexes          673.4 ms   +29.8 ms   0.15 us
//! ```
//!
//! **0.15 us per row for all 17 predicates** — 8.8 ns each, and 1.5% of the
//! 10.3 us a real row costs. Dropping an index to save its predicate evaluation
//! is not worth doing; dropping one is worth doing only for the ~3.2 us of entry
//! it charges the rows that actually enter it. That includes the two indexes
//! whose expression is `COALESCE(value_string_folded, lower(value_string))`:
//! `lower()` is strict, so a row with no string never calls it, and a row
//! written by any build since v24 has `value_string_folded` populated and never
//! reaches the second `COALESCE` arm.
//!
//! The one index every row enters, `idx_search_resource`, is therefore 2.62 of
//! the 10.3 us — 25% of the whole statement, more than any other single index.
//! It is not removable: it is the seek for the re-index `DELETE`
//! (`INSERT_SQL_REPLACE`), `delete_search_index` and the purge paths, and no
//! other index leads with `(tenant_id, resource_type, resource_id)`.
//!
//! All figures above are from a 4-core WSL2 box shared with other builds. The
//! per-index and per-kind *ratios* are what transfer; the absolute microseconds
//! are not, and a same-binary noise floor of ~13% has been measured on this
//! statement elsewhere.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};

use crate::backends::postgres::cached::execute_cached;
use crate::backends::postgres::schema::IndexLayout;
use crate::error::{BackendError, StorageResult};
use crate::search::{converters::IndexValue, extractor::ExtractedValue};
use crate::types::strip_reference_version;

fn internal_error(message: String) -> crate::error::StorageError {
    crate::error::StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

/// Parses an extracted date value into the UTC timestamp stored in `value_date`.
///
/// Returns `None` when the value cannot be parsed, so the caller can skip the
/// index row. This previously fell back to `Utc::now()`, which turned a parse
/// failure into a plausible-looking timestamp: the row was silently indexed at
/// ingestion time, so `date=gt<any past date>` matched it and `date=lt…` did
/// not. Nothing was logged, and the resource itself read back correctly, so the
/// corruption was visible only by querying `search_index` directly (#494).
///
/// A missing index row makes the parameter behave as absent for that resource —
/// still a gap, but a silent under-match is recoverable and a silent *wrong*
/// match is not.
fn parse_index_date(value: &str) -> Option<DateTime<Utc>> {
    let normalized = normalize_date_for_pg(value);
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| normalized.parse::<DateTime<Utc>>())
        .ok()
}

/// One `search_index` row, flattened to every column any write path can set.
///
/// The three columns that identify the resource — `tenant_id`, `resource_type`,
/// `resource_id` — are deliberately absent: they are constant across every row
/// one write produces (a contained entry is stored under its *container*'s type
/// and id), so [`INSERT_SQL`] binds them once per statement rather than once per
/// row.
///
/// `param_url` and the `is_contained` / `contained_*` trio are here so that the
/// rows extracted from `contained[]` share this shape and this statement.
/// Before, they had a single-row `INSERT` of their own: 311,630 of them in one
/// 5-minute crud run, 419 s of Postgres execution time and a round trip each.
#[derive(Default)]
pub(crate) struct IndexRow {
    last_updated: Option<DateTime<Utc>>,
    param_name: String,
    param_url: Option<String>,
    composite_group: Option<i32>,
    value_string: Option<String>,
    value_string_folded: Option<String>,
    value_token_system: Option<String>,
    value_token_code: Option<String>,
    value_token_display: Option<String>,
    value_token_system_2: Option<String>,
    value_token_code_2: Option<String>,
    value_date: Option<DateTime<Utc>>,
    value_date_precision: Option<String>,
    value_number: Option<f64>,
    value_number_2: Option<f64>,
    value_quantity_value: Option<f64>,
    value_quantity_unit: Option<String>,
    value_quantity_system: Option<String>,
    value_quantity_canonical_value: Option<f64>,
    value_quantity_canonical_unit: Option<String>,
    value_reference: Option<String>,
    value_reference_display: Option<String>,
    value_identifier_type_system: Option<String>,
    value_identifier_type_code: Option<String>,
    value_uri: Option<String>,
    is_contained: bool,
    contained_type: Option<String>,
    contained_local_id: Option<String>,
}

/// One column of the insert: its name, the array type it is bound as, and the
/// column's value for every row of the chunk.
struct InsertPlan {
    columns: Vec<&'static str>,
    casts: Vec<&'static str>,
    params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
}

/// Declares one column. Name, bind type and value extractor are written on a
/// single line and pushed together, so the column list and the bind order
/// cannot drift apart — the failure this replaces was silent, because Postgres
/// accepts a shifted value wherever the types happen to line up and the index
/// is then corrupted rather than the write rejected.
macro_rules! column {
    ($plan:expr, $rows:expr, $name:literal, $cast:literal, $value:expr) => {{
        $plan.columns.push($name);
        $plan.casts.push($cast);
        $plan
            .params
            .push(Box::new($rows.iter().map($value).collect::<Vec<_>>()));
    }};
}

/// Builds the column list and one array parameter per column for `rows`.
///
/// `rows.iter().map(..).collect()` always yields `rows.len()` elements, so every
/// array is the same length by construction and the multi-argument `unnest`
/// below never has to NULL-pad.
fn insert_plan(rows: &[&IndexRow]) -> InsertPlan {
    let mut plan = InsertPlan {
        columns: Vec::with_capacity(28),
        casts: Vec::with_capacity(28),
        params: Vec::with_capacity(28),
    };
    let p = &mut plan;

    column!(p, rows, "last_updated", "timestamptz[]", |r: &&IndexRow| r
        .last_updated);
    column!(p, rows, "param_name", "text[]", |r: &&IndexRow| r
        .param_name
        .clone());
    column!(p, rows, "param_url", "text[]", |r: &&IndexRow| r
        .param_url
        .clone());
    column!(p, rows, "composite_group", "int4[]", |r: &&IndexRow| r
        .composite_group);
    column!(p, rows, "value_string", "text[]", |r: &&IndexRow| r
        .value_string
        .clone());
    column!(p, rows, "value_string_folded", "text[]", |r: &&IndexRow| r
        .value_string_folded
        .clone());
    column!(p, rows, "value_token_system", "text[]", |r: &&IndexRow| r
        .value_token_system
        .clone());
    column!(p, rows, "value_token_code", "text[]", |r: &&IndexRow| r
        .value_token_code
        .clone());
    column!(p, rows, "value_token_display", "text[]", |r: &&IndexRow| r
        .value_token_display
        .clone());
    column!(
        p,
        rows,
        "value_token_system_2",
        "text[]",
        |r: &&IndexRow| r.value_token_system_2.clone()
    );
    column!(p, rows, "value_token_code_2", "text[]", |r: &&IndexRow| r
        .value_token_code_2
        .clone());
    column!(p, rows, "value_date", "timestamptz[]", |r: &&IndexRow| r
        .value_date);
    column!(
        p,
        rows,
        "value_date_precision",
        "text[]",
        |r: &&IndexRow| r.value_date_precision.clone()
    );
    column!(p, rows, "value_number", "float8[]", |r: &&IndexRow| r
        .value_number);
    column!(p, rows, "value_number_2", "float8[]", |r: &&IndexRow| r
        .value_number_2);
    column!(
        p,
        rows,
        "value_quantity_value",
        "float8[]",
        |r: &&IndexRow| r.value_quantity_value
    );
    column!(p, rows, "value_quantity_unit", "text[]", |r: &&IndexRow| r
        .value_quantity_unit
        .clone());
    column!(
        p,
        rows,
        "value_quantity_system",
        "text[]",
        |r: &&IndexRow| r.value_quantity_system.clone()
    );
    column!(
        p,
        rows,
        "value_quantity_canonical_value",
        "float8[]",
        |r: &&IndexRow| r.value_quantity_canonical_value
    );
    column!(
        p,
        rows,
        "value_quantity_canonical_unit",
        "text[]",
        |r: &&IndexRow| r.value_quantity_canonical_unit.clone()
    );
    column!(p, rows, "value_reference", "text[]", |r: &&IndexRow| r
        .value_reference
        .clone());
    column!(
        p,
        rows,
        "value_reference_display",
        "text[]",
        |r: &&IndexRow| r.value_reference_display.clone()
    );
    column!(
        p,
        rows,
        "value_identifier_type_system",
        "text[]",
        |r: &&IndexRow| r.value_identifier_type_system.clone()
    );
    column!(
        p,
        rows,
        "value_identifier_type_code",
        "text[]",
        |r: &&IndexRow| r.value_identifier_type_code.clone()
    );
    column!(p, rows, "value_uri", "text[]", |r: &&IndexRow| r
        .value_uri
        .clone());
    column!(p, rows, "is_contained", "bool[]", |r: &&IndexRow| r
        .is_contained);
    column!(p, rows, "contained_type", "text[]", |r: &&IndexRow| r
        .contained_type
        .clone());
    column!(p, rows, "contained_local_id", "text[]", |r: &&IndexRow| r
        .contained_local_id
        .clone());

    plan
}

/// The one and only statement the index writer sends.
///
/// It is a `SELECT` over the multi-argument form of `unnest`, not a multi-row
/// `VALUES` list, and that is the point: **its text does not depend on how many
/// rows are being written**. The previous form emitted
/// `VALUES ($1,…,$28), ($29,…,$56), …` — a different query string for every
/// batch width, averaging 26 rows and therefore ~728 placeholders, sent
/// unprepared. Postgres raw-parsed those ~5 KB, ran parse analysis over 728
/// `Param` nodes coercing each to its target column, and planned a `Values` scan
/// of 728 expressions — 1.5M times over an import, 256k times over a 5-minute
/// crud run, and never once re-used, because the text changed with the row
/// count and `execute(&str)` prepares a throwaway statement each call.
///
/// Now there is a single text with 31 parameters — three scalars and 28 arrays —
/// whatever the row count, so it is prepared once per connection and every
/// execution after the fifth runs on a cached generic plan.
///
/// `unnest(a, b, c, …)` in `FROM` expands the arrays side by side into one row
/// per element, which is exactly the row set the `VALUES` list spelled out.
static INSERT_SQL: LazyLock<String> = LazyLock::new(|| {
    let plan = insert_plan(&[]);
    let arrays: Vec<String> = plan
        .casts
        .iter()
        .enumerate()
        .map(|(i, cast)| format!("${}::{}", i + 4, cast))
        .collect();
    format!(
        "INSERT INTO search_index (tenant_id, resource_type, resource_id, {}) \
         SELECT $1::text, $2::text, $3::text, * FROM unnest({})",
        plan.columns.join(", "),
        arrays.join(", ")
    )
});

/// [`INSERT_SQL`] with the re-index `DELETE` folded in front of it.
///
/// A rewrite — `update`, `restore`, `$reindex` — has to clear the resource's
/// existing rows before writing the new ones, and did that with a statement of
/// its own:
///
/// ```text
/// DELETE FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3
/// INSERT INTO search_index (…) SELECT $1, $2, $3, * FROM unnest(…)
/// ```
///
/// Two statements, two round trips, and **the same three parameters bound
/// twice**. On run 33176893776's crud suite that `DELETE` ran 592,650 times for
/// 384.6 s; half of those (296,325) are the update path's, and they are the ones
/// that have an `INSERT` immediately behind them on the same connection. Per
/// crud iteration it is one of ~15 statements, so folding it removes ~7% of the
/// round trips the suite makes at Postgres.
///
/// The `DELETE` keeps doing the same work — it is the same index seek on
/// `idx_search_resource` over the same rows — so what is saved is the fixed cost
/// of a second statement: a bind, an executor start and stop, and a network
/// round trip whose latency is paid at 300 VUs against a pool of 32.
///
/// The two halves cannot interfere. `search_index` has no unique constraint and
/// no foreign key (v22/v23), and a data-modifying CTE and the outer statement
/// see the *same* snapshot: the `INSERT` neither sees the deleted rows nor is
/// blocked by them, and it is an append of new tuples either way. The ordering
/// that mattered — old rows gone before the new ones are visible — is a property
/// of the single statement's atomicity, which is stronger than what two
/// statements without a transaction gave.
///
/// The parameter *numbers* are shared with [`INSERT_SQL`] by construction, so a
/// caller binds the identical list for either text.
static INSERT_SQL_REPLACE: LazyLock<String> = LazyLock::new(|| {
    format!(
        "WITH cleared AS (\
             DELETE FROM search_index \
             WHERE tenant_id = $1::text AND resource_type = $2::text AND resource_id = $3::text\
         ) {}",
        INSERT_SQL.as_str()
    )
});

/// The clearing `DELETE` on its own, for a rewrite that has no rows to write.
///
/// [`INSERT_SQL_REPLACE`] carries the `DELETE` inside an `INSERT`, so it does
/// nothing at all when there are no rows — and a rewrite whose extraction
/// produced nothing is exactly the case where the old rows *must* still go, or
/// the resource keeps matching values it no longer has. Rare (the extractor has
/// to fail, which is logged) but not impossible, and silent if got wrong.
const CLEAR_SQL: &str = "DELETE FROM search_index \
                         WHERE tenant_id = $1 AND resource_type = $2 AND resource_id = $3";

/// The same statement, for rows belonging to *different* resources.
///
/// [`INSERT_SQL`] binds `resource_type` and `resource_id` once per statement
/// because one resource's rows all share them. A transaction bundle does not:
/// it writes ~1,632 resources, and sending one statement per resource costs one
/// bind, one executor start/stop and one round trip each — 1,630,685 of them on
/// run 33029355759's import, whose 1.6M index inserts averaged 24 rows and whose
/// straight-line fit puts ~0.3 ms of that per-statement cost on every one.
///
/// So this form promotes those two to arrays and keeps only `tenant_id` scalar,
/// which a transaction genuinely does hold constant (a `PostgresTransaction`
/// carries exactly one `TenantContext`). Everything else — the column list, the
/// bind order, the parameter *numbers* of the 28 value arrays — is shared with
/// [`INSERT_SQL`] by construction, because both are built from the same
/// [`insert_plan`].
static INSERT_SQL_MULTI: LazyLock<String> = LazyLock::new(|| {
    let plan = insert_plan(&[]);
    let mut arrays = vec!["$2::text[]".to_string(), "$3::text[]".to_string()];
    arrays.extend(
        plan.casts
            .iter()
            .enumerate()
            .map(|(i, cast)| format!("${}::{}", i + 4, cast)),
    );
    format!(
        "INSERT INTO search_index (tenant_id, resource_type, resource_id, {}) \
         SELECT $1::text, * FROM unnest({})",
        plan.columns.join(", "),
        arrays.join(", ")
    )
});

/// Rows per statement.
///
/// With the `unnest` form this is no longer a bind-parameter limit — 128 rows
/// cost the same 31 parameters as one row does — so it exists only to bound the
/// array a single statement has to marshal. It is well above the 24.2 index rows
/// an average resource produces; what it changes is the tail. `Provenance.target`
/// alone writes 1,626 rows for one resource, which the old 128-row cap split
/// into 13 statements and 13 round trips.
const BATCH_ROWS: usize = 1024;

/// Rows per statement for [`PostgresSearchIndexWriter::insert_rows_multi`].
///
/// Sized against the transaction buffer that feeds it: 128 resources at the
/// import corpus's 24.2 index rows per resource is ~3,100 rows, so the common
/// flush is one statement, and the cap only splits the tail.
const MULTI_BATCH_ROWS: usize = 4096;

/// Search parameters this backend answers from `resources` rather than from
/// `search_index`, and therefore does not index.
///
/// `_id` and `_lastUpdated` are the `resources.id` and `resources.last_updated`
/// columns restated as index rows. Every read path here already prefers the
/// column: `build_parameter_condition` routes `_id` to `id = $n` and
/// `_lastUpdated` to a `last_updated` comparison before it ever looks at a
/// value column; `sort_expression` maps both to the bare columns rather than
/// the correlated `search_index` subquery it uses for indexed parameters;
/// `build_missing_condition` selects from `resources`; `primary_keyset_key`
/// pages on `last_updated`; and `build_contained_condition` excludes
/// `_`-prefixed parameters outright. `ChainQueryBuilder` was the one path that
/// still read the rows, for a chained or reverse-chained terminal such as
/// `Observation?subject:Patient._id=p1`, and it now reads `resources` too.
///
/// The rows cost one insert per resource per parameter with no reader. On the
/// row census for run 33029355759, `Observation | _id` alone is 689,080 rows —
/// one for each Observation — and across all 1,632,067 resources `_id` is
/// ~1.63M of the table's 39.5M rows.
///
/// This is a write-side decision only, and it is one-directional: a database
/// written by an older build still has the rows, and nothing here reads them,
/// so both shapes answer identically. That is why it needs no schema version
/// and no migration.
pub(crate) const PARAMS_ANSWERED_FROM_RESOURCES: [&str; 2] = ["_id", "_lastUpdated"];

/// Whether [`PARAMS_ANSWERED_FROM_RESOURCES`] covers this parameter.
pub(crate) fn answered_from_resources(param_name: &str) -> bool {
    PARAMS_ANSWERED_FROM_RESOURCES.contains(&param_name)
}

impl IndexRow {
    /// Flattens one extracted value into a row.
    ///
    /// Returns `None` for an unparseable date, which is the one case the
    /// per-value path also skipped rather than inserted (#494).
    fn from_extracted(
        extracted: &ExtractedValue,
        resource_type: &str,
        resource_id: &str,
        last_updated: Option<DateTime<Utc>>,
    ) -> Option<Self> {
        let mut row = IndexRow {
            last_updated,
            param_name: extracted.param_name.to_string(),
            composite_group: extracted.composite_group.map(|g| g as i32),
            ..Default::default()
        };

        match &extracted.value {
            IndexValue::String(s) => {
                row.value_string = Some(s.clone());
                row.value_string_folded = Some(crate::search::fold_text(s));
            }
            IndexValue::Token {
                system,
                code,
                display,
                identifier_type_system,
                identifier_type_code,
            } => {
                row.value_token_system = system.clone();
                row.value_token_code = Some(code.clone());
                row.value_token_display = display.clone();
                row.value_identifier_type_system = identifier_type_system.clone();
                row.value_identifier_type_code = identifier_type_code.clone();
            }
            IndexValue::Date { value, precision } => {
                row.value_date_precision = Some(precision.to_string());
                let Some(timestamp) = parse_index_date(value) else {
                    tracing::warn!(
                        param_name = %extracted.param_name,
                        resource_type = %resource_type,
                        resource_id = %resource_id,
                        value = %value,
                        "skipping date search index entry: unparseable date value"
                    );
                    return None;
                };
                row.value_date = Some(timestamp);
            }
            IndexValue::Number(n) => {
                row.value_number = Some(*n);
            }
            IndexValue::Quantity {
                value,
                unit,
                system,
                code,
            } => {
                // Canonicalize using the UCUM code (else the unit display) so
                // quantity search can match equivalent units (g <-> mg).
                let (canonical_value, canonical_unit) = code
                    .as_deref()
                    .or(unit.as_deref())
                    .and_then(|u| helios_fhirpath::ucum::canonicalize_quantity(*value, u))
                    .map(|(v, u)| (Some(v), Some(u)))
                    .unwrap_or((None, None));
                row.value_quantity_value = Some(*value);
                row.value_quantity_unit = unit.clone();
                row.value_quantity_system = system.clone();
                row.value_quantity_canonical_value = canonical_value;
                row.value_quantity_canonical_unit = canonical_unit;
            }
            IndexValue::Reference {
                reference,
                resource_type: _,
                resource_id: _,
                display,
            } => {
                // Stored in its version-agnostic base form. See
                // `migrate_v31_to_v32`: no reader of this column has ever been
                // able to use a `/_history/<vid>` suffix, and three of them are
                // silently broken by one.
                row.value_reference = Some(strip_reference_version(reference).to_string());
                row.value_reference_display = display.clone();
            }
            IndexValue::Uri(uri) => {
                row.value_uri = Some(uri.clone());
            }
        }

        Some(row)
    }

    /// Flattens one value extracted from a `contained[]` entry.
    ///
    /// The row is stored under the *container*'s type and id — which is what
    /// [`PostgresSearchIndexWriter::insert_rows`] binds as the statement's
    /// scalars — and flagged with the contained resource's type and local id.
    ///
    /// `last_updated` stays NULL and `param_url` is populated, both exactly as
    /// the single-row insert this replaces left them. They are the only two
    /// columns where a contained row differs from a plain one, and getting
    /// either wrong would change `_contained` result ordering or a stored value.
    fn from_contained(
        extracted: &ExtractedValue,
        container: (&str, &str),
        contained: (&str, &str),
    ) -> Option<Self> {
        let (container_type, container_id) = container;
        let (contained_type, contained_local_id) = contained;
        let mut row = Self::from_extracted(extracted, container_type, container_id, None)?;
        row.param_url = Some(extracted.param_url.clone());
        row.is_contained = true;
        row.contained_type = Some(contained_type.to_string());
        row.contained_local_id = Some(contained_local_id.to_string());
        Some(row)
    }

    /// Flattens one denormalized composite row (#279).
    ///
    /// Deliberately does not populate `value_string_folded`, `value_token_display`,
    /// the identifier-type columns or the canonical quantity columns: the
    /// composite insert never set them either, and a composite search reads none
    /// of them.
    fn from_composite(
        row: &super::composite_rows::CompositeRow,
        last_updated: Option<DateTime<Utc>>,
    ) -> Self {
        IndexRow {
            last_updated,
            param_name: row.param_name.clone(),
            composite_group: Some(row.composite_group),
            value_token_system: row.value_token_system.clone(),
            value_token_code: row.value_token_code.clone(),
            value_token_system_2: row.value_token_system_2.clone(),
            value_token_code_2: row.value_token_code_2.clone(),
            value_string: row.value_string.clone(),
            value_date: row.value_date.as_deref().and_then(parse_index_date),
            value_number: row.value_number,
            value_number_2: row.value_number_2,
            value_quantity_value: row.value_quantity_value,
            value_quantity_unit: row.value_quantity_unit.clone(),
            value_quantity_system: row.value_quantity_system.clone(),
            value_reference: row.value_reference.clone(),
            value_uri: row.value_uri.clone(),
            ..Default::default()
        }
    }
}


/// The identity of a `search_index` row, for the de-duplication in
/// [`dedup_rows`].
///
/// Every column the table has, in borrowed form: the three that identify the
/// resource (bound once per statement, so they are passed in rather than read
/// off the row) plus all 28 of [`IndexRow`]'s. Two rows with equal keys are the
/// same tuple, byte for byte, and Postgres would store both.
///
/// `f64` is not `Eq`/`Hash`, so the five float columns are keyed on their IEEE
/// bit patterns. That is *stricter* than `==`: `0.0` and `-0.0` compare equal as
/// floats but have different bits, and two NaNs compare unequal but may share
/// bits. Stricter is the safe direction here — the only consequence is keeping a
/// row that could have been dropped.
#[derive(PartialEq, Eq, Hash)]
struct RowKey<'a> {
    resource_type: &'a str,
    resource_id: &'a str,
    last_updated: Option<&'a DateTime<Utc>>,
    param_name: &'a str,
    param_url: Option<&'a String>,
    composite_group: Option<&'a i32>,
    value_string: Option<&'a String>,
    value_string_folded: Option<&'a String>,
    value_token_system: Option<&'a String>,
    value_token_code: Option<&'a String>,
    value_token_display: Option<&'a String>,
    value_token_system_2: Option<&'a String>,
    value_token_code_2: Option<&'a String>,
    value_date: Option<&'a DateTime<Utc>>,
    value_date_precision: Option<&'a String>,
    value_number: Option<u64>,
    value_number_2: Option<u64>,
    value_quantity_value: Option<u64>,
    value_quantity_unit: Option<&'a String>,
    value_quantity_system: Option<&'a String>,
    value_quantity_canonical_value: Option<u64>,
    value_quantity_canonical_unit: Option<&'a String>,
    value_reference: Option<&'a String>,
    value_reference_display: Option<&'a String>,
    value_identifier_type_system: Option<&'a String>,
    value_identifier_type_code: Option<&'a String>,
    value_uri: Option<&'a String>,
    is_contained: bool,
    contained_type: Option<&'a String>,
    contained_local_id: Option<&'a String>,
}

impl IndexRow {
    /// This row's [`RowKey`], under the resource the statement will bind.
    ///
    /// The body destructures `IndexRow` with an exhaustive pattern and **no**
    /// `..` rest. A column added to the row and not added here therefore fails
    /// to compile, rather than silently dropping a row that differs only in the
    /// new column — the same device [`insert_plan`]'s `column!` macro uses for
    /// the bind order, and for the same reason: the failure it replaces is
    /// silent and corrupts the index rather than erroring.
    fn key<'a>(&'a self, resource_type: &'a str, resource_id: &'a str) -> RowKey<'a> {
        let IndexRow {
            last_updated,
            param_name,
            param_url,
            composite_group,
            value_string,
            value_string_folded,
            value_token_system,
            value_token_code,
            value_token_display,
            value_token_system_2,
            value_token_code_2,
            value_date,
            value_date_precision,
            value_number,
            value_number_2,
            value_quantity_value,
            value_quantity_unit,
            value_quantity_system,
            value_quantity_canonical_value,
            value_quantity_canonical_unit,
            value_reference,
            value_reference_display,
            value_identifier_type_system,
            value_identifier_type_code,
            value_uri,
            is_contained,
            contained_type,
            contained_local_id,
        } = self;

        RowKey {
            resource_type,
            resource_id,
            last_updated: last_updated.as_ref(),
            param_name: param_name.as_str(),
            param_url: param_url.as_ref(),
            composite_group: composite_group.as_ref(),
            value_string: value_string.as_ref(),
            value_string_folded: value_string_folded.as_ref(),
            value_token_system: value_token_system.as_ref(),
            value_token_code: value_token_code.as_ref(),
            value_token_display: value_token_display.as_ref(),
            value_token_system_2: value_token_system_2.as_ref(),
            value_token_code_2: value_token_code_2.as_ref(),
            value_date: value_date.as_ref(),
            value_date_precision: value_date_precision.as_ref(),
            value_number: value_number.map(f64::to_bits),
            value_number_2: value_number_2.map(f64::to_bits),
            value_quantity_value: value_quantity_value.map(f64::to_bits),
            value_quantity_unit: value_quantity_unit.as_ref(),
            value_quantity_system: value_quantity_system.as_ref(),
            value_quantity_canonical_value: value_quantity_canonical_value.map(f64::to_bits),
            value_quantity_canonical_unit: value_quantity_canonical_unit.as_ref(),
            value_reference: value_reference.as_ref(),
            value_reference_display: value_reference_display.as_ref(),
            value_identifier_type_system: value_identifier_type_system.as_ref(),
            value_identifier_type_code: value_identifier_type_code.as_ref(),
            value_uri: value_uri.as_ref(),
            is_contained: *is_contained,
            contained_type: contained_type.as_ref(),
            contained_local_id: contained_local_id.as_ref(),
        }
    }
}

/// Drops rows that repeat a row already in the list, preserving order.
///
/// ## The rows this removes
///
/// One resource can extract the same value twice under the same parameter, and
/// every one of those became its own `search_index` row. Measured on a 40-bundle
/// slice of the benchmark's Synthea corpus (59,167 resources, 822,992 index
/// rows, 13.91 rows per resource — the same ratio the full corpus shows),
/// **8,265 rows, 1.00% of the table, are byte-for-byte repeats of a row the same
/// resource already has**:
///
/// ```text
/// resource_type      param_name               rows   repeats
/// Observation        component-value-concept  8352      3060
/// PractitionerRole   telecom                  3348      1674
/// Practitioner       email                    1674       837
/// Practitioner       telecom                  1674       837
/// PractitionerRole   phone                    1674       837
/// PractitionerRole   email                    1674       837
/// Provenance         agent                      80        40
/// Patient            phone                      80        40
/// Patient            telecom                    80        40
/// Patient            name / phonetic / given   365        51
/// AllergyIntolerance severity                   28        11
/// ```
///
/// They are not a bug in any one parameter. `Practitioner.telecom` is a `token`
/// parameter over `ContactPoint`, and a Synthea practitioner carries the same
/// address in two entries; `email` is `telecom.where(system='email')` over the
/// same two. `Observation.component-value-concept` is
/// `Observation.component.value as CodeableConcept`, and a panel whose
/// components share an interpretation code repeats it once per component. FHIR
/// has no rule against any of this — the resources are valid, and the second
/// value is genuinely present in them.
///
/// ## Why removing them cannot change a search result
///
/// Because no reader of `search_index` on this backend counts rows. Every one of
/// them is a set operation:
///
/// - `build_*_condition` (`search/query_builder.rs`) emits
///   `id IN (SELECT resource_id FROM search_index WHERE …)` for every parameter
///   type — a sublink whose result is a set. A second identical row cannot
///   change whether `id` is in it.
/// - `build_missing_condition` tests presence/absence of *any* row for the
///   parameter. De-duplication never removes the last row of a group, so
///   presence is preserved exactly.
/// - The #279 composite fast path (`search_impl.rs`) is
///   `SELECT DISTINCT resource_id, last_updated … ORDER BY … LIMIT n`. `DISTINCT`
///   runs before `LIMIT`, so duplicates could never even consume its budget —
///   they only cost it scan work.
/// - `sort_expression` correlates `(SELECT MIN(col) …)` / `MAX`. Aggregates over
///   a multiset with a repeated element give the same answer.
/// - `ChainQueryBuilder` nests the same `IN (SELECT …)` sublinks at every depth.
/// - `resolve_revincludes` is `SELECT DISTINCT` behind a `seen_ids` set;
///   `resolve_includes` reads the resource JSON and never touches this table.
///
/// What does change is [`SearchIndexWriter::count_entries`] and the
/// `$reindex` "entries" tally, which now report rows written rather than values
/// visited. That is a reporting number, and the more accurate of the two.
///
/// ## Cost
///
/// One `HashSet` insert per row, against ~3.2 us of btree maintenance per index
/// entry and 2.85 index entries per row (see the module docstring). The trade is
/// deliberate: Postgres is the wall on both write suites and the HFS process is
/// not. The rows removed here are mostly `token` and `string` — 9.8 and 14.4 us
/// respectively, the expensive half of the table.
fn dedup_rows<'a>(
    rows: impl IntoIterator<Item = (&'a str, &'a str, &'a IndexRow)>,
) -> Vec<(&'a str, &'a str, &'a IndexRow)> {
    let mut seen: HashSet<RowKey<'a>> = HashSet::new();
    let mut kept = Vec::new();
    for (resource_type, resource_id, row) in rows {
        if seen.insert(row.key(resource_type, resource_id)) {
            kept.push((resource_type, resource_id, row));
        }
    }
    kept
}

/// PostgreSQL implementation of SearchIndexWriter.
pub struct PostgresSearchIndexWriter;

impl PostgresSearchIndexWriter {
    /// Writes every extracted value for one resource.
    ///
    /// Non-composite values keep one row each. Composite values are folded into
    /// the denormalized one-row-per-instance layout (issue #279) before insert,
    /// so a composite search is a plain conjunction over a single row instead of
    /// a grouped aggregate over one row per component.
    ///
    /// Returns the number of rows written.
    pub async fn write_values(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        last_updated: DateTime<Utc>,
        layout: IndexLayout,
        values: Vec<ExtractedValue>,
    ) -> StorageResult<usize> {
        let rows = Self::build_rows(resource_type, resource_id, last_updated, layout, values);
        Self::insert_rows(client, tenant_id, resource_type, resource_id, &rows).await?;
        Ok(rows.len())
    }

    /// Flattens a resource's extracted values into rows, without touching the
    /// database.
    ///
    /// Split out from [`Self::write_values`] so a caller that also has contained
    /// rows can append them and send everything as one statement.
    pub(crate) fn build_rows(
        resource_type: &str,
        resource_id: &str,
        last_updated: DateTime<Utc>,
        layout: IndexLayout,
        values: Vec<ExtractedValue>,
    ) -> Vec<IndexRow> {
        // `drop_resources_backed` first: `_id`/`_lastUpdated` restate columns on
        // `resources` and are answered from there, so they never become rows.
        let (plain, composites) =
            Self::split_for_layout(Self::drop_resources_backed(values), layout);

        let mut rows: Vec<IndexRow> = Vec::with_capacity(plain.len() + composites.len());
        for value in &plain {
            if let Some(row) =
                IndexRow::from_extracted(value, resource_type, resource_id, Some(last_updated))
            {
                rows.push(row);
            }
        }
        for row in &composites {
            rows.push(IndexRow::from_composite(row, Some(last_updated)));
        }
        rows
    }

    /// Flattens the values extracted from one `contained[]` entry into rows.
    ///
    /// [`Self::drop_resources_backed`] applies here too, and did not before.
    /// `build_contained` (`search/query_builder.rs`) skips every parameter whose
    /// name `starts_with('_')` outright, so a contained `_id` or `_lastUpdated`
    /// row has no reader at all — not "answered from `resources`" as on the
    /// plain path, but genuinely unreachable.
    ///
    /// The `_id` row is worse than merely unread: it is a byte-for-byte
    /// restatement of a column the same row already carries. On the benchmark's
    /// `ExplanationOfBenefit` seed, whose two `contained[]` entries are a
    /// `ServiceRequest` with `id: "referral"` and a `Coverage` with
    /// `id: "coverage"`:
    ///
    /// ```text
    /// param_name | value_token_code | contained_type  | contained_local_id
    /// _id        | referral         | ServiceRequest  | referral
    /// _id        | coverage         | Coverage        | coverage
    /// ```
    ///
    /// Each costs a heap tuple plus three index entries — `idx_search_resource`,
    /// and both token indexes, whose predicate is `value_token_code IS NOT NULL`
    /// and so admits them despite their NULL `last_updated`. Two of the 132 rows
    /// the nine crud seed resources produce, i.e. **1.5% of the crud suite's
    /// `search_index` write**, and they are the only `_`-prefixed rows the
    /// corpus produces.
    ///
    /// Write-side only and one-directional, exactly as
    /// [`PARAMS_ANSWERED_FROM_RESOURCES`] is: a database written by an older
    /// build still holds the rows, nothing reads them, and both shapes answer
    /// identically. No schema version, no migration.
    pub(crate) fn build_contained_rows(
        container: (&str, &str),
        contained: (&str, &str),
        values: &[ExtractedValue],
    ) -> Vec<IndexRow> {
        values
            .iter()
            .filter(|value| !answered_from_resources(&value.param_name))
            .filter_map(|value| IndexRow::from_contained(value, container, contained))
            .collect()
    }

    /// Drops the values this backend answers from `resources` columns.
    ///
    /// See [`PARAMS_ANSWERED_FROM_RESOURCES`]. Split out from `write_values` so
    /// the rule is assertable without a database.
    fn drop_resources_backed(values: Vec<ExtractedValue>) -> Vec<ExtractedValue> {
        values
            .into_iter()
            .filter(|v| !answered_from_resources(&v.param_name))
            .collect()
    }

    /// Splits extracted values into the row shapes the database's layout expects.
    ///
    /// A pre-v17 database is read with the grouped composite form, which only
    /// understands one row per component. Folding anyway would leave the table
    /// holding both shapes at once, matching neither reliably — and silently,
    /// since a composite miss returns an empty bundle rather than an error. So
    /// the write side follows the same marker the read side does.
    fn split_for_layout(
        values: Vec<ExtractedValue>,
        layout: IndexLayout,
    ) -> (
        Vec<ExtractedValue>,
        Vec<super::composite_rows::CompositeRow>,
    ) {
        match layout {
            IndexLayout::Denormalized => super::composite_rows::fold_composites(values),
            IndexLayout::Legacy => (values, Vec::new()),
        }
    }

    /// Sends flattened rows through [`INSERT_SQL`], at most [`BATCH_ROWS`] per
    /// statement.
    ///
    /// `tenant_id`, `resource_type` and `resource_id` are the same for every row
    /// of one write, so they are bound once per statement instead of once per
    /// row — 3 fewer values on the wire for each of the ~39.5M index rows an
    /// import writes.
    pub(crate) async fn insert_rows(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        rows: &[IndexRow],
    ) -> StorageResult<()> {
        Self::write_rows(client, tenant_id, resource_type, resource_id, rows, false).await
    }

    /// [`Self::insert_rows`], but clearing whatever this resource already has.
    ///
    /// The `DELETE` rides inside the first statement ([`INSERT_SQL_REPLACE`])
    /// rather than being one of its own. Only the *first* chunk carries it: a
    /// resource whose rows exceed [`BATCH_ROWS`] must not have chunk 2 delete
    /// what chunk 1 just wrote.
    pub(crate) async fn replace_rows(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        rows: &[IndexRow],
    ) -> StorageResult<()> {
        Self::write_rows(client, tenant_id, resource_type, resource_id, rows, true).await
    }

    async fn write_rows(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        rows: &[IndexRow],
        clear_first: bool,
    ) -> StorageResult<()> {
        if clear_first && rows.is_empty() {
            // Nothing to fold the `DELETE` into, and it still has to happen.
            execute_cached(
                client,
                CLEAR_SQL,
                &[&tenant_id, &resource_type, &resource_id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to clear search index rows: {}", e)))?;
            return Ok(());
        }

        // One choke point for the de-duplication: every row this backend writes
        // for a single resource passes through here. See [`dedup_rows`].
        let kept = dedup_rows(rows.iter().map(|row| (resource_type, resource_id, row)));

        let mut first = true;
        for chunk in kept.chunks(BATCH_ROWS) {
            let sql = if clear_first && first {
                INSERT_SQL_REPLACE.as_str()
            } else {
                INSERT_SQL.as_str()
            };
            first = false;
            let chunk: Vec<&IndexRow> = chunk.iter().map(|(_, _, row)| *row).collect();
            let plan = insert_plan(&chunk);
            let mut param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                Vec::with_capacity(3 + plan.params.len());
            param_refs.push(&tenant_id);
            param_refs.push(&resource_type);
            param_refs.push(&resource_id);
            param_refs.extend(
                plan.params
                    .iter()
                    .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync)),
            );

            execute_cached(client, sql, &param_refs)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to insert search index rows: {}", e))
                })?;
        }

        Ok(())
    }

    /// Sends the rows of *several* resources through [`INSERT_SQL_MULTI`].
    ///
    /// `batches` is `(resource_type, resource_id, rows)` in the order the
    /// resources were created, and the flattened row order follows it, so the
    /// table is appended to in exactly the order the unbatched path appended in.
    /// `search_index` has no unique constraint and no foreign key (v22/v23), so
    /// nothing about this insert can conflict with, or wait on, a concurrent
    /// writer's rows: it is a pure append of rows keyed by a resource id that
    /// only this transaction is creating.
    ///
    /// Chunked at [`MULTI_BATCH_ROWS`] so one caller's flush is normally one
    /// statement, and a single pathological resource (`Provenance.target` writes
    /// 1,626 rows) cannot make the arrays unbounded.
    pub(crate) async fn insert_rows_multi(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        batches: &[(&str, &str, &[IndexRow])],
    ) -> StorageResult<()> {
        let total: usize = batches.iter().map(|(_, _, rows)| rows.len()).sum();
        if total == 0 {
            return Ok(());
        }

        let mut flat: Vec<(&str, &str, &IndexRow)> = Vec::with_capacity(total);
        for (resource_type, resource_id, rows) in batches {
            for row in rows.iter() {
                flat.push((resource_type, resource_id, row));
            }
        }

        // The other choke point. Here the key's `resource_type` / `resource_id`
        // are load-bearing rather than constant: this flush carries several
        // resources, and two of them may legitimately hold the identical value
        // under the identical parameter. See [`dedup_rows`].
        let flat = dedup_rows(flat);

        for chunk in flat.chunks(MULTI_BATCH_ROWS) {
            let rows: Vec<&IndexRow> = chunk.iter().map(|(_, _, row)| *row).collect();
            let resource_types: Vec<&str> = chunk.iter().map(|(t, _, _)| *t).collect();
            let resource_ids: Vec<&str> = chunk.iter().map(|(_, i, _)| *i).collect();

            let plan = insert_plan(&rows);
            let mut param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                Vec::with_capacity(3 + plan.params.len());
            param_refs.push(&tenant_id);
            param_refs.push(&resource_types);
            param_refs.push(&resource_ids);
            param_refs.extend(
                plan.params
                    .iter()
                    .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync)),
            );

            execute_cached(client, INSERT_SQL_MULTI.as_str(), &param_refs)
                .await
                .map_err(|e| {
                    internal_error(format!("Failed to insert search index rows: {}", e))
                })?;
        }

        Ok(())
    }

    /// Writes a single search index entry to PostgreSQL.
    ///
    /// Shares [`IndexRow`] with the batched path so both agree on which column
    /// each `IndexValue` variant populates.
    pub async fn write_entry(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        last_updated: DateTime<Utc>,
        extracted: &ExtractedValue,
    ) -> StorageResult<()> {
        if answered_from_resources(&extracted.param_name) {
            return Ok(());
        }
        let Some(row) =
            IndexRow::from_extracted(extracted, resource_type, resource_id, Some(last_updated))
        else {
            return Ok(());
        };
        Self::insert_rows(client, tenant_id, resource_type, resource_id, &[row]).await
    }
}
/// Normalize a date string for PostgreSQL TIMESTAMPTZ.
///
/// Converts partial dates to full timestamps:
/// - "2024" -> "2024-01-01T00:00:00+00:00"
/// - "2024-01" -> "2024-01-01T00:00:00+00:00"
/// - "2024-01-15" -> "2024-01-15T00:00:00+00:00"
/// - "2024-01-15T10:30:00" -> "2024-01-15T10:30:00+00:00"
/// - "2024-01-15T10:30:00-07:00" -> unchanged (already zoned)
fn normalize_date_for_pg(value: &str) -> String {
    if let Some((_, time_part)) = value.split_once('T') {
        // Already has a time component — append UTC only if it carries no zone.
        //
        // The zone test must look at the *time* component alone. Testing the
        // whole value for `-` would match the date's own `YYYY-MM-DD`
        // separators, and testing only for `+`/`Z`/`-00:00` (as this did) misses
        // every other negative offset: `2019-05-04T12:12:29-07:00` was treated
        // as zone-less and became `...-07:00+00:00`, which is not valid RFC3339.
        // Per the FHIR `dateTime`/`instant` grammar the only `+` or `-` that can
        // appear after `T` is the offset sign, so their presence is decisive.
        let has_zone = time_part.ends_with('Z')
            || time_part.ends_with('z')
            || time_part.contains('+')
            || time_part.contains('-');
        if has_zone {
            value.to_string()
        } else {
            format!("{}+00:00", value)
        }
    } else if value.len() == 10 {
        // YYYY-MM-DD
        format!("{}T00:00:00+00:00", value)
    } else if value.len() == 7 {
        // YYYY-MM
        format!("{}-01T00:00:00+00:00", value)
    } else if value.len() == 4 {
        // YYYY
        format!("{}-01-01T00:00:00+00:00", value)
    } else {
        // Best effort
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two statements are built from one [`insert_plan`], and
    /// [`PostgresSearchIndexWriter::insert_rows`] and
    /// [`PostgresSearchIndexWriter::insert_rows_multi`] push their three scalars
    /// or arrays in the same three slots before extending with the identical
    /// `plan.params`. If the value arrays ever stopped starting at `$4` in both,
    /// one of the two would bind every value one column off — a shift Postgres
    /// accepts wherever the types line up, corrupting the index rather than
    /// failing the write.
    #[test]
    fn both_statements_number_the_value_arrays_from_the_same_slot() {
        let columns = insert_plan(&[]).columns;
        for sql in [INSERT_SQL.as_str(), INSERT_SQL_MULTI.as_str()] {
            assert!(
                sql.contains("$4::timestamptz[]"),
                "the first value array must be $4 in `{sql}`"
            );
            assert_eq!(
                sql.matches("::").count(),
                // three scalars/arrays plus one cast per value column
                3 + columns.len(),
                "one cast per bound parameter and no more, in `{sql}`"
            );
        }
    }

    /// Both write the same columns in the same order; only the way
    /// `resource_type`/`resource_id` are supplied differs.
    #[test]
    fn both_statements_write_the_same_column_list() {
        let list = format!(
            "INSERT INTO search_index (tenant_id, resource_type, resource_id, {})",
            insert_plan(&[]).columns.join(", ")
        );
        assert!(INSERT_SQL.starts_with(&list));
        assert!(INSERT_SQL_MULTI.starts_with(&list));
    }

    /// The multi form binds the tenant once and everything else per row, which
    /// is what lets one statement carry several resources.
    #[test]
    fn the_multi_statement_binds_only_the_tenant_as_a_scalar() {
        assert!(INSERT_SQL.contains("SELECT $1::text, $2::text, $3::text, * FROM unnest("));
        assert!(
            INSERT_SQL_MULTI.contains("SELECT $1::text, * FROM unnest($2::text[], $3::text[],")
        );
    }


    /// A row that repeats one already in the list is dropped, and the survivors
    /// keep their original order — the order the unbatched path appended in.
    #[test]
    fn an_identical_row_is_written_once() {
        let row = |code: &str| IndexRow {
            param_name: "code".to_string(),
            value_token_code: Some(code.to_string()),
            ..Default::default()
        };
        let rows = vec![row("a"), row("b"), row("a"), row("c"), row("b")];
        let kept = dedup_rows(rows.iter().map(|r| ("Observation", "obs1", r)));
        let codes: Vec<&str> = kept
            .iter()
            .map(|(_, _, r)| r.value_token_code.as_deref().unwrap())
            .collect();
        assert_eq!(codes, ["a", "b", "c"]);
    }

    /// The last row of a group is never removed, so `:missing` — which tests
    /// presence of *any* row for the parameter — answers identically.
    #[test]
    fn a_repeated_value_still_leaves_one_row() {
        let row = || IndexRow {
            param_name: "telecom".to_string(),
            value_token_code: Some("mailto:x@example.org".to_string()),
            ..Default::default()
        };
        let rows = vec![row(), row(), row()];
        let kept = dedup_rows(rows.iter().map(|r| ("Practitioner", "p1", r)));
        assert_eq!(kept.len(), 1);
    }

    /// Two resources may legitimately hold the same value under the same
    /// parameter. The multi-resource flush keys on the resource as well, so one
    /// of them is not silently dropped — which would make the *other* resource
    /// unfindable by that value.
    #[test]
    fn the_same_value_on_two_resources_is_kept_twice() {
        let row = IndexRow {
            param_name: "code".to_string(),
            value_token_code: Some("8302-2".to_string()),
            ..Default::default()
        };
        let kept = dedup_rows([
            ("Observation", "obs1", &row),
            ("Observation", "obs2", &row),
            ("DiagnosticReport", "dr1", &row),
        ]);
        assert_eq!(kept.len(), 3);
    }

    /// Every column participates in the key. A pair of rows differing in
    /// exactly one column is two rows, not one — including the columns a search
    /// never reads (`param_url`, `value_reference_display`), because they are
    /// still stored values and dropping one would change what is in the table.
    ///
    /// The key itself is built by destructuring `IndexRow` exhaustively, so a
    /// new column that is not added to it fails to compile; this covers the
    /// columns that exist today.
    #[test]
    fn every_column_distinguishes_two_rows() {
        let base = IndexRow::default();
        let mutators: Vec<(&str, fn(&mut IndexRow))> = vec![
            ("last_updated", |r| r.last_updated = Some(Utc::now())),
            ("param_name", |r| r.param_name = "x".into()),
            ("param_url", |r| r.param_url = Some("u".into())),
            ("composite_group", |r| r.composite_group = Some(1)),
            ("value_string", |r| r.value_string = Some("s".into())),
            ("value_string_folded", |r| {
                r.value_string_folded = Some("s".into())
            }),
            ("value_token_system", |r| {
                r.value_token_system = Some("s".into())
            }),
            ("value_token_code", |r| r.value_token_code = Some("c".into())),
            ("value_token_display", |r| {
                r.value_token_display = Some("d".into())
            }),
            ("value_token_system_2", |r| {
                r.value_token_system_2 = Some("s".into())
            }),
            ("value_token_code_2", |r| {
                r.value_token_code_2 = Some("c".into())
            }),
            ("value_date", |r| r.value_date = Some(Utc::now())),
            ("value_date_precision", |r| {
                r.value_date_precision = Some("day".into())
            }),
            ("value_number", |r| r.value_number = Some(1.0)),
            ("value_number_2", |r| r.value_number_2 = Some(1.0)),
            ("value_quantity_value", |r| {
                r.value_quantity_value = Some(1.0)
            }),
            ("value_quantity_unit", |r| {
                r.value_quantity_unit = Some("mg".into())
            }),
            ("value_quantity_system", |r| {
                r.value_quantity_system = Some("ucum".into())
            }),
            ("value_quantity_canonical_value", |r| {
                r.value_quantity_canonical_value = Some(1.0)
            }),
            ("value_quantity_canonical_unit", |r| {
                r.value_quantity_canonical_unit = Some("g".into())
            }),
            ("value_reference", |r| r.value_reference = Some("P/1".into())),
            ("value_reference_display", |r| {
                r.value_reference_display = Some("d".into())
            }),
            ("value_identifier_type_system", |r| {
                r.value_identifier_type_system = Some("s".into())
            }),
            ("value_identifier_type_code", |r| {
                r.value_identifier_type_code = Some("MR".into())
            }),
            ("value_uri", |r| r.value_uri = Some("http://x".into())),
            ("is_contained", |r| r.is_contained = true),
            ("contained_type", |r| r.contained_type = Some("Coverage".into())),
            ("contained_local_id", |r| {
                r.contained_local_id = Some("cov".into())
            }),
        ];
        assert_eq!(
            mutators.len(),
            insert_plan(&[]).columns.len(),
            "one mutator per inserted column"
        );
        for (name, mutate) in mutators {
            let mut other = IndexRow::default();
            mutate(&mut other);
            let kept = dedup_rows([("Patient", "p1", &base), ("Patient", "p1", &other)]);
            assert_eq!(kept.len(), 2, "`{name}` must distinguish two rows");
        }
    }

    /// The defect behind #494: a negative UTC offset was not recognised as a
    /// zone, so `+00:00` was appended and the result stopped being valid
    /// RFC3339. US/Americas data is overwhelmingly negative-offset, so this was
    /// the common case rather than an edge case.
    #[test]
    fn negative_offsets_are_recognised_as_zoned() {
        for value in [
            "2019-05-04T12:12:29-07:00",
            "1941-09-05T01:11:45-04:00",
            "2021-11-10T16:48:57.246958-08:00",
            "2024-01-15T10:30:00-00:00",
        ] {
            assert_eq!(
                normalize_date_for_pg(value),
                value,
                "an already-zoned value must be left alone"
            );
            assert!(
                parse_index_date(value).is_some(),
                "{value} must parse rather than be dropped"
            );
        }
    }

    /// A negative offset must survive as an *instant*, not just parse. Appending
    /// `+00:00` to `...-07:00` happened to be unparseable, but the failure mode
    /// worth pinning is the resulting timestamp being wrong.
    #[test]
    fn negative_offset_converts_to_the_right_instant() {
        let parsed = parse_index_date("2019-05-04T12:12:29-07:00").expect("parses");
        assert_eq!(
            parsed.to_rfc3339(),
            "2019-05-04T19:12:29+00:00",
            "-07:00 is seven hours behind UTC"
        );
    }

    #[test]
    fn positive_offsets_and_z_are_still_recognised() {
        for value in [
            "2024-01-15T10:30:00Z",
            "2024-01-15T10:30:00+05:30",
            "2024-01-15T10:30:00.123Z",
        ] {
            assert_eq!(normalize_date_for_pg(value), value);
            assert!(parse_index_date(value).is_some());
        }
    }

    #[test]
    fn zone_less_and_partial_values_are_completed_as_utc() {
        for (input, expected) in [
            ("2024-01-15T10:30:00", "2024-01-15T10:30:00+00:00"),
            ("2024-01-15", "2024-01-15T00:00:00+00:00"),
            ("2024-01", "2024-01-01T00:00:00+00:00"),
            ("2024", "2024-01-01T00:00:00+00:00"),
        ] {
            assert_eq!(normalize_date_for_pg(input), expected);
            assert!(parse_index_date(input).is_some(), "{input} must parse");
        }
    }

    /// An unparseable value yields `None` so the caller skips the row. It must
    /// never resolve to a timestamp — the old `unwrap_or_else(|_| Utc::now())`
    /// wrote ingestion time and made every date search over the row wrong.
    #[test]
    fn unparseable_values_are_dropped_not_substituted() {
        for value in ["", "not-a-date", "2024-13-45T99:99:99", "T00:00:00"] {
            assert!(
                parse_index_date(value).is_none(),
                "{value:?} must not resolve to a timestamp"
            );
        }
    }

    use crate::search::extractor::ExtractedValue;
    use crate::types::DatePrecision;

    fn extracted(value: IndexValue) -> ExtractedValue {
        ExtractedValue {
            param_name: "p".to_string(),
            param_url: "http://example.org/p".to_string(),
            param_type: crate::types::SearchParamType::String,
            value,
            composite_group: None,
            composite_slot: None,
            composite_arity: None,
        }
    }

    fn row_of(value: IndexValue) -> IndexRow {
        IndexRow::from_extracted(&extracted(value), "Observation", "abc", Some(Utc::now()))
            .expect("value should map to a row")
    }

    /// A pre-v17 database is read with the grouped composite form, which only
    /// understands one row per component. If the write path folded anyway, the
    /// table would hold both shapes and the read form would match neither
    /// reliably — silently, since a composite miss returns an empty bundle.
    #[test]
    fn the_layout_decides_the_composite_row_shape() {
        let component = |code: &str, slot: u8| ExtractedValue {
            param_name: "code-value-quantity".to_string(),
            param_url: "http://example.org/cvq".to_string(),
            param_type: crate::types::SearchParamType::Composite,
            value: IndexValue::Token {
                system: Some("http://loinc.org".to_string()),
                code: code.to_string(),
                display: None,
                identifier_type_system: None,
                identifier_type_code: None,
            },
            composite_group: Some(1),
            composite_slot: Some(slot),
            // `code-value-quantity` is two-component; a group that reaches that
            // arity is what the fold keeps.
            composite_arity: Some(2),
        };
        let values = vec![component("8480-6", 1), component("8462-4", 2)];

        let (plain, folded) =
            PostgresSearchIndexWriter::split_for_layout(values.clone(), IndexLayout::Denormalized);
        assert_eq!(folded.len(), 1, "the pair folds into one row");
        assert!(plain.is_empty(), "nothing is left unfolded");

        let (plain, folded) =
            PostgresSearchIndexWriter::split_for_layout(values, IndexLayout::Legacy);
        assert_eq!(plain.len(), 2, "one row per component");
        assert!(folded.is_empty(), "nothing is folded on a legacy layout");
    }

    /// `_id` and `_lastUpdated` restate `resources.id` and
    /// `resources.last_updated`; nothing on this backend reads their index rows
    /// (`PARAMS_ANSWERED_FROM_RESOURCES`), so writing them is one insert per
    /// resource per parameter for no reader.
    #[test]
    fn resources_backed_params_are_not_indexed() {
        let named = |name: &str, value: IndexValue| ExtractedValue {
            param_name: name.to_string(),
            ..extracted(value)
        };
        let kept = PostgresSearchIndexWriter::drop_resources_backed(vec![
            named("_id", IndexValue::token_code("abc")),
            named(
                "_lastUpdated",
                IndexValue::date("2024-01-15T10:30:00Z".to_string()),
            ),
            named("code", IndexValue::token_code("8302-2")),
            // Not on the list: a real parameter whose name merely starts with
            // an underscore must keep its rows.
            named("_profile", IndexValue::uri("http://example.org/p")),
            named("_tag", IndexValue::token_code("t")),
        ]);
        let names: Vec<&str> = kept.iter().map(|v| v.param_name.as_str()).collect();
        assert_eq!(names, vec!["code", "_profile", "_tag"]);
    }

    /// The column list and the bind order are now produced by one `column!` per
    /// column, so a name can no longer drift away from the value bound under it.
    /// What is still worth pinning is that nothing else creeps in between: the
    /// statement must bind exactly three scalars plus one array per column, and
    /// its placeholders must run `$1..=$n` with no gap or repeat. A drift here
    /// used to shift every later value into the wrong column, which Postgres
    /// accepts silently wherever the types line up — the index is corrupted
    /// rather than the write rejected.
    #[test]
    fn the_statement_binds_three_scalars_and_one_array_per_column() {
        let plan = insert_plan(&[]);
        let expected = 3 + plan.columns.len();

        assert_eq!(plan.casts.len(), plan.columns.len());
        assert_eq!(plan.params.len(), plan.columns.len());

        let sql = INSERT_SQL.as_str();
        assert_eq!(
            sql.matches('$').count(),
            expected,
            "every bind needs exactly one placeholder"
        );
        for n in 1..=expected {
            assert!(sql.contains(&format!("${}", n)), "missing ${}", n);
        }
        assert!(
            !sql.contains(&format!("${}", expected + 1)),
            "no placeholder beyond the last bind"
        );

        // The INSERT column list is the three scalars plus every planned column,
        // in order.
        let column_list = sql
            .split_once("(tenant_id, resource_type, resource_id, ")
            .expect("insert names its columns")
            .1
            .split_once(") SELECT ")
            .expect("column list is closed")
            .0;
        assert_eq!(column_list, plan.columns.join(", "));
    }

    /// The row count is a property of the arrays, not of the SQL: the same
    /// statement text has to serve one row and a full batch, or the
    /// prepared-statement cache buys nothing.
    #[test]
    fn the_statement_text_does_not_depend_on_the_row_count() {
        for rows in [0usize, 1, 3, BATCH_ROWS] {
            let owned: Vec<IndexRow> = (0..rows)
                .map(|_| row_of(IndexValue::String("x".to_string())))
                .collect();
            let chunk: Vec<&IndexRow> = owned.iter().collect();
            let plan = insert_plan(&chunk);
            assert_eq!(plan.params.len(), insert_plan(&[]).params.len());
            assert_eq!(plan.columns, insert_plan(&[]).columns);
        }
    }

    /// The load-bearing half of schema v30. `build_token_condition` emits
    /// `value_token_code IS NOT NULL AND value_token_system = $n` for the
    /// `system|` form, which is row-set-preserving only while no written row has
    /// a system without a code — and v30 dropped the 2,283 MB `idx_search_token`
    /// on the strength of that. `IndexValue::Token` makes it a type-level fact
    /// (`code: String`, not `Option<String>`), and both writer paths carry it
    /// through; this pins the behaviour so a later `Option` would fail here
    /// rather than silently narrow every `system|` search.
    #[test]
    fn a_token_row_never_has_a_system_without_a_code() {
        for code in ["8302-2", ""] {
            let row = row_of(IndexValue::Token {
                system: Some("http://loinc.org".to_string()),
                code: code.to_string(),
                display: None,
                identifier_type_system: None,
                identifier_type_code: None,
            });
            assert!(row.value_token_system.is_some());
            assert!(
                row.value_token_code.is_some(),
                "a system-bearing row with a NULL code would make the v30 \
                 `system|` predicate lose matches"
            );
        }

        // The composite path is the other producer of these columns.
        let composite = crate::backends::postgres::search::composite_rows::CompositeRow {
            value_token_system: Some("http://loinc.org".to_string()),
            value_token_code: Some("8480-6".to_string()),
            ..Default::default()
        };
        let row = IndexRow::from_composite(&composite, None);
        assert!(row.value_token_system.is_some() && row.value_token_code.is_some());
    }

    #[test]
    fn each_value_kind_lands_in_its_own_columns() {
        let string = row_of(IndexValue::String("Smith".to_string()));
        assert_eq!(string.value_string.as_deref(), Some("Smith"));
        assert!(
            string.value_string_folded.is_some(),
            "string is folded on write"
        );
        assert!(string.value_token_code.is_none());

        let token = row_of(IndexValue::Token {
            system: Some("http://loinc.org".to_string()),
            code: "8302-2".to_string(),
            display: Some("Body height".to_string()),
            identifier_type_system: Some("http://ts".to_string()),
            identifier_type_code: Some("MR".to_string()),
        });
        assert_eq!(
            token.value_token_system.as_deref(),
            Some("http://loinc.org")
        );
        assert_eq!(token.value_token_code.as_deref(), Some("8302-2"));
        assert_eq!(token.value_token_display.as_deref(), Some("Body height"));
        assert_eq!(token.value_identifier_type_code.as_deref(), Some("MR"));
        assert!(token.value_string.is_none());

        let reference = row_of(IndexValue::Reference {
            reference: "Patient/1".to_string(),
            resource_type: Some("Patient".to_string()),
            resource_id: Some("1".to_string()),
            display: Some("Jane".to_string()),
        });
        assert_eq!(reference.value_reference.as_deref(), Some("Patient/1"));
        assert_eq!(reference.value_reference_display.as_deref(), Some("Jane"));

        let uri = row_of(IndexValue::Uri("http://x".to_string()));
        assert_eq!(uri.value_uri.as_deref(), Some("http://x"));

        let number = row_of(IndexValue::Number(4.0));
        assert_eq!(number.value_number, Some(4.0));
        assert!(number.value_quantity_value.is_none());
    }

    /// The whole of schema v32 rests on this: `value_reference` holds the
    /// version-agnostic base, so `build_reference_condition` can emit one
    /// equality instead of `= OR LIKE '<base>/\_history/%'`, and so the three
    /// readers that split the column on `/` (`_revinclude`, `ChainQueryBuilder`,
    /// `:identifier`) see a `Type/id` they can resolve. A later change that
    /// stored `Reference.reference` verbatim again would not fail anywhere
    /// except in results — a versioned reference would simply stop matching —
    /// so it fails here instead.
    #[test]
    fn a_stored_reference_is_normalized_to_its_version_agnostic_base() {
        for (stored, expected) in [
            ("Patient/1/_history/4", "Patient/1"),
            ("Patient/1", "Patient/1"),
            (
                "http://ex.org/fhir/Patient/1/_history/2",
                "http://ex.org/fhir/Patient/1",
            ),
            // No suffix to strip, and nothing that merely contains the text.
            (
                "Patient/_history-is-not-a-version",
                "Patient/_history-is-not-a-version",
            ),
            ("#contained-1", "#contained-1"),
        ] {
            let row = row_of(IndexValue::Reference {
                reference: stored.to_string(),
                resource_type: None,
                resource_id: None,
                display: None,
            });
            assert_eq!(
                row.value_reference.as_deref(),
                Some(expected),
                "stored reference {stored:?}"
            );
        }
    }

    /// A plain row and a contained row now travel in the same statement, so the
    /// four columns that tell them apart have to be set per row. Getting
    /// `last_updated` wrong would reorder `_contained` results (the search key is
    /// `last_updated DESC`, and NULLs sort first under `DESC`); getting
    /// `is_contained` wrong would make a contained value answer an ordinary
    /// search.
    #[test]
    fn a_contained_row_differs_from_a_plain_row_in_exactly_four_columns() {
        let value = extracted(IndexValue::String("Smith".to_string()));

        let plain =
            IndexRow::from_extracted(&value, "Patient", "p1", Some(Utc::now())).expect("row");
        assert!(!plain.is_contained);
        assert!(plain.param_url.is_none(), "the batched path leaves it NULL");
        assert!(plain.contained_type.is_none());
        assert!(plain.contained_local_id.is_none());
        assert!(plain.last_updated.is_some());

        let contained =
            IndexRow::from_contained(&value, ("Patient", "p1"), ("Practitioner", "prac1"))
                .expect("row");
        assert!(contained.is_contained);
        assert_eq!(
            contained.param_url.as_deref(),
            Some("http://example.org/p"),
            "the contained path has always stored param_url"
        );
        assert_eq!(contained.contained_type.as_deref(), Some("Practitioner"));
        assert_eq!(contained.contained_local_id.as_deref(), Some("prac1"));
        assert!(
            contained.last_updated.is_none(),
            "the single-row insert never bound last_updated for contained rows"
        );

        // Everything else is the value, and the value is flattened identically.
        assert_eq!(plain.value_string, contained.value_string);
        assert_eq!(plain.value_string_folded, contained.value_string_folded);
        assert_eq!(plain.param_name, contained.param_name);
    }

    /// `build_contained` (`query_builder.rs`) skips every `_`-prefixed
    /// parameter, so a contained `_id` row is not "answered elsewhere" — it is
    /// unreachable. The plain path has dropped these since
    /// `PARAMS_ANSWERED_FROM_RESOURCES`; the contained path did not, and wrote
    /// two per `ExplanationOfBenefit` whose value duplicated
    /// `contained_local_id` exactly.
    #[test]
    fn contained_rows_drop_the_parameters_answered_from_resources() {
        let mut id_value = extracted(IndexValue::Token {
            system: None,
            code: "referral".to_string(),
            display: None,
            identifier_type_system: None,
            identifier_type_code: None,
        });
        id_value.param_name = "_id".to_string();
        let mut kept = extracted(IndexValue::String("Smith".to_string()));
        kept.param_name = "family".to_string();

        let rows = PostgresSearchIndexWriter::build_contained_rows(
            ("ExplanationOfBenefit", "eob1"),
            ("ServiceRequest", "referral"),
            &[id_value, kept],
        );

        let names: Vec<&str> = rows.iter().map(|r| r.param_name.as_str()).collect();
        assert_eq!(names, vec!["family"], "contained rows: {names:?}");
    }

    /// The re-index `DELETE` is folded into the insert, so the two texts have to
    /// agree about parameter numbering: a caller binds one list and picks the
    /// text. If the fold ever stopped being a pure prefix, the bind order would
    /// drift silently and Postgres would accept whatever lined up by type.
    #[test]
    fn the_replacing_insert_is_the_plain_insert_behind_a_delete_cte() {
        let plain = INSERT_SQL.as_str();
        let replacing = INSERT_SQL_REPLACE.as_str();

        assert!(
            replacing.ends_with(plain),
            "replacing form must be the plain INSERT verbatim: {replacing}"
        );
        assert!(replacing.starts_with("WITH cleared AS (DELETE FROM search_index "));
        for placeholder in ["$1::text", "$2::text", "$3::text"] {
            assert!(
                replacing
                    .split_once(plain)
                    .expect("prefix")
                    .0
                    .contains(placeholder),
                "the DELETE reuses the INSERT's own scalars: {replacing}"
            );
        }
        // Nothing between $1..$3 and the value arrays: the DELETE introduces no
        // parameter of its own, so `$4` is still the first array either way.
        assert!(replacing.contains("$4::timestamptz[]"));
    }

    /// An unparseable date skips its row rather than being stored at ingestion
    /// time, which would make `date=gt<any past date>` match it (#494) — on the
    /// contained path too.
    #[test]
    fn an_unparseable_date_yields_no_row() {
        let bad = || IndexValue::Date {
            value: "not-a-date".to_string(),
            precision: DatePrecision::Day,
        };
        assert!(
            IndexRow::from_extracted(&extracted(bad()), "Observation", "abc", Some(Utc::now()))
                .is_none()
        );
        assert!(
            IndexRow::from_contained(&extracted(bad()), ("Observation", "abc"), ("Patient", "p1"))
                .is_none()
        );
    }
}
