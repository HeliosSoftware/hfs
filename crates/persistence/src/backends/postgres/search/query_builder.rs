//! PostgreSQL search query builder.
//!
//! Builds SQL queries for FHIR search operations using PostgreSQL syntax
//! with $N parameter placeholders, ILIKE for case-insensitive matching,
//! and native TIMESTAMPTZ comparisons.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use super::REFERENCE_TARGET_ID_EXPR;
use crate::backends::postgres::schema::IndexLayout;
use crate::error::SearchError;
use crate::search::IMPLICIT_TOKEN_SYSTEM;
use crate::search::fold_text;
use crate::search::{
    DatePredicate, FhirDateValue, FhirNumberValue, RangeCondition, StorageResolution,
};
use crate::types::{
    CompartmentMembership, ContainedMode, SearchModifier, SearchParamType, SearchParameter,
    SearchPrefix, SearchQuery, SearchValue, strip_reference_version,
};

// Keep generated SQL and bind counts small well before PostgreSQL's 65,535-parameter limit.
const LARGE_ID_SET_THRESHOLD: usize = 1_000;

/// Returns the implicit precision of a decimal search value from its string form
/// (e.g. `"100"` → 1.0, `"100.0"` → 0.1), used to build `eq` ranges.
fn quantity_implicit_precision(num_str: &str) -> f64 {
    crate::search::implicit_precision(num_str)
}

/// Escapes LIKE metacharacters in a user-supplied search value.
///
/// Without this, `?name=100%` is a wildcard rather than a literal — the value is
/// interpolated straight into the pattern. Paired with `ESCAPE '\'` at the call
/// site. (`chain_builder.rs` already does this; the main builder did not.)
fn like_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// The SQL expression a string search matches against.
///
/// `value_string_folded` (case- and accent-folded, added in schema v10) is
/// populated on write but **never backfilled**, so rows written before the v10
/// upgrade have NULL there and must fall back to the raw column. `lower()` on that
/// fallback reproduces exactly what the previous `ILIKE` did — both sides of the
/// comparison end up lowercased — while keeping the whole expression indexable.
/// Plain `ILIKE` can never use a btree; this expression, matched by
/// `idx_search_string_folded_pattern` under `text_pattern_ops`, can.
///
/// Must stay character-for-character identical to the index definition in
/// `schema.rs::migrate_v32_to_v33`, or Postgres will not use the index.
///
/// ## Why the default (starts-with) form carries no `value_string IS NOT NULL`
///
/// v25 added that conjunct so the pattern index — then partial on
/// `WHERE value_string IS NOT NULL` — could be *proved* usable, because
/// `COALESCE(a, b) LIKE …` does not imply it (COALESCE is not strict). It did
/// make the index legal. It also made it unreachable on cost, and v34 measured
/// why: the conjunct is a second, independently-multiplied selectivity factor on
/// a table where only 0.9% of rows have a string value, and `param_name` already
/// determines that column. Postgres estimated the `(Patient, address)` slice at
/// 25 rows instead of 5,000 and picked the physically smallest index that could
/// supply the `(tenant, type, param)` prefix — `idx_search_string`, 50 MB —
/// filtering the rest. See `schema.rs::migrate_v32_to_v33` for the plans.
///
/// v34 moves the reachability proof into the operator instead: `~>=~` and `~~`
/// are both strict in the expression, so either implies the index's new
/// `COALESCE(…) IS NOT NULL` predicate on its own, and the conjunct — with its
/// wrong estimate — is gone from every modifier that reads this expression.
/// `:exact` never had it (`value_string = $n` is strict on the bare column).
const FOLDED_STRING_EXPR: &str = "COALESCE(value_string_folded, lower(value_string))";

/// The exclusive upper bound of the byte-ordered range that holds exactly the
/// strings beginning with `prefix`, or `None` when no such bound exists.
///
/// `~>=~`/`~<~` (the `text_pattern_ops` operators) compare bytewise, and UTF-8
/// byte order is code-point order, so `prefix <= x < next(prefix)` selects
/// exactly the values whose byte prefix — equivalently, whose character prefix,
/// UTF-8 being self-synchronizing — is `prefix`. `next` is `prefix` with its
/// last character replaced by the next code point.
///
/// This is the same construction `match_pattern_prefix` (`indxpath.c`) performs
/// when it can see a `LIKE` pattern as a `Const`; doing it here is what makes
/// the seek survive a *parameterized* pattern, which the planner cannot rewrite
/// at all — see `migrate_v32_to_v33`.
///
/// Two cases have no bound and fall back to `LIKE`:
/// - an empty prefix (every value matches, and there is nothing to increment);
/// - a prefix that is entirely `char::MAX`, for which nothing sorts higher.
///
/// The UTF-16 surrogate block is skipped because it holds no scalar value: no
/// text a Postgres UTF-8 database can store falls inside it, so widening the
/// bound across it cannot admit a row.
fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = next_char(last) {
            let mut bound: String = chars.into_iter().collect();
            bound.push(next);
            return Some(bound);
        }
        // `last` is `char::MAX`: nothing sorts after it in this position, so the
        // increment has to carry into the preceding character.
    }
    None
}

/// The next Unicode scalar value after `c`, skipping the surrogate block.
fn next_char(c: char) -> Option<char> {
    let mut next = c as u32 + 1;
    if next == 0xD800 {
        next = 0xE000;
    }
    char::from_u32(next)
}

/// Builds the numeric comparison SQL for `col` per the FHIR prefix semantics,
/// advancing `next` and returning the SQL plus its bound params. `eq`/`ne`
/// compare against the implicit-precision range `[lo, hi)` derived from the
/// search value as written. `gt`/`lt`/`ge`/`le`/`sa`/`eb` ignore precision and
/// compare against the exact value `num`, per the FHIR number search spec
/// (<https://hl7.org/fhir/R4/search.html#number>): *"the implicit precision
/// of the number is ignored, and they are treated as if they have arbitrarily
/// high precision."* `num` is also used for the `ap` margin.
fn numeric_predicate(
    col: &str,
    prefix: SearchPrefix,
    number: &FhirNumberValue,
    next: &mut usize,
) -> (String, Vec<SqlParam>) {
    let num = number.value;
    let (lo, hi) = number.implicit_range();
    match prefix {
        SearchPrefix::Eq => {
            *next += 1;
            let a = *next;
            *next += 1;
            (
                format!("{col} >= ${a} AND {col} < ${next}"),
                vec![SqlParam::Float(lo), SqlParam::Float(hi)],
            )
        }
        SearchPrefix::Ne => {
            *next += 1;
            let a = *next;
            *next += 1;
            (
                format!("({col} < ${a} OR {col} >= ${next})"),
                vec![SqlParam::Float(lo), SqlParam::Float(hi)],
            )
        }
        SearchPrefix::Gt | SearchPrefix::Sa => {
            *next += 1;
            (format!("{col} > ${next}"), vec![SqlParam::Float(num)])
        }
        SearchPrefix::Lt | SearchPrefix::Eb => {
            *next += 1;
            (format!("{col} < ${next}"), vec![SqlParam::Float(num)])
        }
        SearchPrefix::Ge => {
            *next += 1;
            (format!("{col} >= ${next}"), vec![SqlParam::Float(num)])
        }
        SearchPrefix::Le => {
            *next += 1;
            (format!("{col} <= ${next}"), vec![SqlParam::Float(num)])
        }
        SearchPrefix::Ap => {
            let (lo, hi) = number.approx_range();
            *next += 1;
            let a = *next;
            *next += 1;
            (
                format!("{col} BETWEEN ${a} AND ${next}"),
                vec![SqlParam::Float(lo), SqlParam::Float(hi)],
            )
        }
    }
}

/// Parses the number part of a `number` or `quantity` search value (prefix
/// already split off), or returns `None` when it is not a finite decimal.
///
/// The grammar is the one every backend shares,
/// [`FhirNumberValue`](crate::search::FhirNumberValue): an optional sign,
/// digits with an optional fraction, an optional exponent, finite once parsed.
/// It began here (#1319) and moved there (#1340); see that module for the
/// leniencies and for why `inf`, `nan` and `1e999` are rejected — as a bound
/// they are not just invalid, they widen: `value_number < 'Infinity'` is true
/// of every row, and Postgres orders `NaN` above every number, so
/// `lt`/`le`/`ne` with `nan` would match everything.
#[cfg(test)]
pub(crate) fn parse_search_number(raw: &str) -> Option<f64> {
    crate::search::FhirNumberValue::parse(raw)
        .ok()
        .map(|number| number.value)
}

/// `None` for a number or quantity value the shared grammar refused. The search
/// gate (`validate_numeric_values`) rejects such a value before a query is
/// built, so reaching this is worth a warning.
fn unvalidated<T>(parsed: Result<T, crate::search::NumberValueError>) -> Option<T> {
    parsed
        .inspect_err(|error| {
            tracing::warn!(
                "unvalidated number search value reached the PostgreSQL builder: {error}"
            )
        })
        .ok()
}

/// Builds the comparison of one `number` search value (prefix already split
/// off) against the numeric column `col`, advancing `next` by exactly the
/// number of binds returned.
///
/// The unchained `number` builder and the chain builder's number terminal
/// (`chain_builder.rs`, #1306) both call this, so they share
/// [`numeric_predicate`]'s per-prefix table and its implicit-precision range.
///
/// Returns `None`, leaving `next` untouched, when `raw` is not a number (see
/// [`parse_search_number`]). Every caller must turn that into
/// [`match_nothing`] rather than skipping the value: a dropped constraint
/// over-matches (#1319).
pub(crate) fn number_predicate(
    col: &str,
    prefix: SearchPrefix,
    raw: &str,
    next: &mut usize,
) -> Option<(String, Vec<SqlParam>)> {
    let number = unvalidated(FhirNumberValue::parse(raw))?;
    Some(numeric_predicate(col, prefix, &number, next))
}

/// Builds the comparison of one `quantity` search value —
/// `number`, `number|code` or `number|system|code`, prefix already split off —
/// against the `value_quantity_*` columns, advancing `next` by exactly the
/// number of binds returned.
///
/// `table` qualifies the columns: empty for the unchained builder, whose
/// subquery has a single unaliased `search_index`, and `"si2."`-style for the
/// chain builder's terminals (`chain_builder.rs`, #1306). Both call this, so a
/// chained quantity means what the unchained one does, units included.
///
/// Returns `None`, leaving `next` untouched, when the number part is not a
/// number (see [`parse_search_number`]); as with [`number_predicate`], every
/// caller must turn that into [`match_nothing`]. The returned predicate is
/// parenthesized.
pub(crate) fn quantity_predicate(
    table: &str,
    prefix: SearchPrefix,
    raw_value: &str,
    next: &mut usize,
) -> Option<(String, Vec<SqlParam>)> {
    /// Binds one float and returns its placeholder number.
    fn bind(v: f64, params: &mut Vec<SqlParam>, next: &mut usize) -> usize {
        *next += 1;
        params.push(SqlParam::Float(v));
        *next
    }

    // Parse quantity: number|system|code (or number|code, or number), by the
    // grammar every backend shares — only an unescaped `|` separates.
    let quantity = unvalidated(crate::search::FhirQuantityValue::parse(raw_value))?;
    let (num_str, num) = (quantity.number.text(), quantity.number.value);
    let (system, code) = (quantity.system.as_deref(), quantity.code.as_deref());

    // Raw branch: value comparison (exact for comparators, implicit-precision
    // range for eq/ne, the shared window for ap) + the stored unit/system.
    let (mut raw, mut params) = numeric_predicate(
        &format!("{table}value_quantity_value"),
        prefix,
        &quantity.number,
        next,
    );
    if let Some(c) = code {
        *next += 1;
        params.push(SqlParam::text(c));
        raw.push_str(&format!(" AND {table}value_quantity_unit = ${next}"));
    }
    if let Some(s) = system {
        *next += 1;
        params.push(SqlParam::text(s));
        raw.push_str(&format!(" AND {table}value_quantity_system = ${next}"));
    }

    // Canonical branch on the canonical columns so unit equivalents
    // match (g ⇄ mg). Bounds are canonicalized in the search unit before
    // comparison. Skipped for non-convertible units; `ne` also uses this
    // branch (negated range) so it matches the raw-OR-canonical `eq`
    // semantics inverted, same as SQLite's `build_canonical_condition`.
    let mut predicate = format!("({raw})");
    if let Some(c) = code {
        if let Some((_, cunit)) = helios_fhirpath::ucum::canonicalize_quantity(num, c) {
            let canon = |x: f64| helios_fhirpath::ucum::canonicalize_quantity(x, c).map(|(v, _)| v);
            // Both ends of a window, canonicalized and put in order (a
            // conversion may be decreasing).
            let canon_window = |a: f64, b: f64| match (canon(a), canon(b)) {
                (Some(a), Some(b)) => Some(if a <= b { (a, b) } else { (b, a) }),
                _ => None,
            };
            let col = format!("{table}value_quantity_canonical_value");
            // Comparators match the exact canonicalized value:
            // gt/sa → > canon(num), lt/eb → < canon(num),
            // ge → ≥ canon(num), le → ≤ canon(num). Precision
            // only bounds `eq`/`ne` (below).
            let range: Option<String> = match prefix {
                SearchPrefix::Gt | SearchPrefix::Sa => {
                    canon(num).map(|b| format!("{col} > ${}", bind(b, &mut params, next)))
                }
                SearchPrefix::Lt | SearchPrefix::Eb => {
                    canon(num).map(|b| format!("{col} < ${}", bind(b, &mut params, next)))
                }
                SearchPrefix::Ge => {
                    canon(num).map(|b| format!("{col} >= ${}", bind(b, &mut params, next)))
                }
                SearchPrefix::Le => {
                    canon(num).map(|b| format!("{col} <= ${}", bind(b, &mut params, next)))
                }
                SearchPrefix::Ap => {
                    let (lo, hi) = quantity.number.approx_range();
                    canon_window(lo, hi).map(|(lo, hi)| {
                        let lo_p = bind(lo, &mut params, next);
                        let hi_p = bind(hi, &mut params, next);
                        format!("{col} BETWEEN ${lo_p} AND ${hi_p}")
                    })
                }
                // Ne: negated implicit-precision range, mirroring the raw
                // branch's `numeric_predicate` shape for `Ne`.
                SearchPrefix::Ne => {
                    let half = quantity_implicit_precision(num_str) / 2.0;
                    canon_window(num - half, num + half).map(|(lo, hi)| {
                        let lo_p = bind(lo, &mut params, next);
                        let hi_p = bind(hi, &mut params, next);
                        format!("({col} < ${lo_p} OR {col} >= ${hi_p})")
                    })
                }
                // Eq: implicit-precision range.
                SearchPrefix::Eq => {
                    let half = quantity_implicit_precision(num_str) / 2.0;
                    canon_window(num - half, num + half).map(|(lo, hi)| {
                        let lo_p = bind(lo, &mut params, next);
                        let hi_p = bind(hi, &mut params, next);
                        format!("{col} >= ${lo_p} AND {col} < ${hi_p}")
                    })
                }
            };
            if let Some(range) = range {
                *next += 1;
                params.push(SqlParam::text(&cunit));
                predicate = format!(
                    "(({raw}) OR ({range} AND {table}value_quantity_canonical_unit = ${next}))"
                );
            }
        }
    }

    Some((predicate, params))
}

/// Returns the `[start, end)` timestamp range for a date search value at its
/// own precision — a year, a month, a day, a minute, a second, or a fraction
/// of one — as [`FhirDateValue`] defines it for every backend, clamped to the
/// microseconds a `TIMESTAMPTZ` holds. `end` is always after `start`.
///
/// A value with a time used to get a degenerate range (`start == end`) and a
/// scalar comparison, so `eq…T23:30:00-04:00` missed a stored `…:00.123` that
/// SQLite and Elasticsearch found (#1297), and a client echoing a millisecond
/// `meta.lastUpdated` could not find the microsecond value stored for it.
///
/// Returns `None` when the value is not a date (`not-a-date`, `2024-13-45`).
/// Every caller must turn that into [`match_nothing`] rather than skipping the
/// value: a dropped constraint over-matches. This path once fell back to
/// `Utc::now()` instead, which turned a parse failure into a plausible-looking
/// bound — `lt`/`le` matched essentially every indexed row, and the client got
/// an ordinary `200` (#1289). It is also what hid #1288: a mishandled negative
/// offset became "compare against now" instead of a visible failure.
///
/// The value is read by the grammar every backend shares. It used to be
/// normalized with the index writer's `normalize_date_for_pg` and judged by
/// chrono (`parse_date_value`, now gone), which kept a search value and a
/// stored value zoned alike (#1288) but accepted what chrono accepts rather
/// than what FHIR does. That invariant is now held by test: see
/// `search_and_index_agree_on_every_valid_value` in `writer.rs`.
pub(crate) fn date_precision_range(value: &str) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    FhirDateValue::parse(value)
        .ok()
        .map(|date| date.range_at(StorageResolution::Micros))
}

/// The condition emitted for a search value that cannot be interpreted: it
/// matches no row and binds no parameter, so it costs the surrounding builder no
/// `$N` placeholder and the numbering stays gap-free.
///
/// Used under *every* prefix, `ne` included. `ne` normally widens a result set,
/// but "not equal to something unparseable" is not a question the client asked;
/// invalid input must never return more than valid input could.
pub(crate) fn match_nothing() -> SqlFragment {
    SqlFragment::new("FALSE")
}

/// Builds the comparison of one date search value against a `TIMESTAMPTZ`
/// column, advancing `next` by exactly the number of binds returned.
///
/// This is the single place a date search value becomes SQL on Postgres. The
/// unchained `date` and `_lastUpdated` builders, the composite date component
/// and the chain builder's terminals (`chain_builder.rs`, #1290) all call it,
/// so they cannot drift:
///
/// - the value is read by [`FhirDateValue`], the grammar and precision range
///   every backend shares, and compared through the [`DatePredicate`] it
///   yields — only `>=` and `<` against the bounds of `[start, end)`, at every
///   precision. There is no scalar fallback for a value with a time: a second
///   is a range too (#1297);
/// - binds are always [`SqlParam::Timestamp`] — a text bind against
///   `TIMESTAMPTZ` fails serialization in tokio-postgres (#871, #1290), and a
///   `$N::timestamptz` cast does not help, it only restates the inferred type;
/// - `ap` has no approximation window here: the shared layer leaves it to the
///   backend, and this one keeps what it always did — a scalar `=` (via
///   [`PostgresQueryBuilder::prefix_to_operator`]) on the start of the range.
///
/// `col` is interpolated verbatim and must be a trusted column expression
/// (`value_date`, `si2.value_date`, `si2.last_updated`), never user input.
///
/// A value that is not a date yields [`match_nothing`]'s `FALSE` with no binds
/// and leaves `next` untouched, under every prefix, `ne` included (#1289). The
/// search gate (`validate_date_values`) rejects such a value before a query is
/// built, so that branch means a caller skipped the gate.
pub(crate) fn date_predicate(
    col: &str,
    prefix: SearchPrefix,
    value: &str,
    next: &mut usize,
) -> (String, Vec<SqlParam>) {
    let parsed = match FhirDateValue::parse(value) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!("unvalidated date search value reached the PostgreSQL builder: {error}");
            return (match_nothing().sql, Vec::new());
        }
    };

    let mut params = Vec::new();
    let mut bind = |instant: DateTime<Utc>| {
        *next += 1;
        params.push(SqlParam::Timestamp(instant));
        format!("${next}")
    };
    let sql = match parsed.predicate(prefix, StorageResolution::Micros) {
        Some(DatePredicate::Within { ge, lt }) => {
            format!("{col} >= {} AND {col} < {}", bind(ge), bind(lt))
        }
        Some(DatePredicate::Outside { lt, ge }) => {
            format!("({col} < {} OR {col} >= {})", bind(lt), bind(ge))
        }
        Some(DatePredicate::AtOrAfter(bound)) => format!("{col} >= {}", bind(bound)),
        Some(DatePredicate::Before(bound)) => format!("{col} < {}", bind(bound)),
        // `ap`: scalar comparison with the start of the range, as before.
        None => {
            let op = PostgresQueryBuilder::prefix_to_operator(&prefix);
            let (start, _) = parsed.range_at(StorageResolution::Micros);
            format!("{col} {op} {}", bind(start))
        }
    };
    (sql, params)
}

/// Builds the comparison of one date search value against an indexed *range*
/// `[start_col, end_col)`, advancing `next` by exactly the number of binds
/// returned (#1391).
///
/// This is what a `date` parameter means against `search_index`: every row
/// carries the range it covers — a point one unit of its own precision, a
/// `Period` its real `[start, end)` with open ends stored as the supported
/// limits — and the prefix is decided by the [`RangePredicate`] the shared
/// layer derives from the FHIR table (`eq` contains, `sa` starts after, `ap`
/// overlaps within the precision's margin, …). `_lastUpdated` and composite
/// date components are points and keep [`date_predicate`].
///
/// One extra bound is emitted that the predicate does not state: a group that
/// requires `end <= b` also gets `start < b`. It is implied — every stored
/// range is at least one unit wide, so `start < end` — and it is what lets
/// `eq` and `eb` keep seeking on `idx_search_date`, keyed on `value_date`,
/// instead of filtering the whole parameter slice on `value_date_end`.
///
/// The result is always safe to splice after an `AND`: a disjunction is
/// parenthesized, a single conjunction is left bare as [`date_predicate`]'s
/// `eq` always was. The same failure mode applies: a value that is not a date
/// yields [`match_nothing`]'s `FALSE` with no binds.
///
/// `start_col` and `end_col` are interpolated verbatim and must be trusted
/// column expressions (`value_date`, `si1.value_date_end`), never user input.
pub(crate) fn date_range_predicate(
    start_col: &str,
    end_col: &str,
    prefix: SearchPrefix,
    value: &str,
    next: &mut usize,
) -> (String, Vec<SqlParam>) {
    let parsed = match FhirDateValue::parse(value) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!("unvalidated date search value reached the PostgreSQL builder: {error}");
            return (match_nothing().sql, Vec::new());
        }
    };

    let mut params = Vec::new();
    let mut bind = |instant: DateTime<Utc>| {
        *next += 1;
        params.push(SqlParam::Timestamp(instant));
        format!("${next}")
    };
    let predicate = parsed.range_predicate(prefix, StorageResolution::Micros);
    let groups: Vec<String> = predicate
        .any_of
        .iter()
        .map(|group| {
            let mut terms = Vec::with_capacity(group.len() + 1);
            for condition in group {
                match *condition {
                    RangeCondition::StartAtOrAfter(b) => {
                        terms.push(format!("{start_col} >= {}", bind(b)))
                    }
                    RangeCondition::StartBefore(b) => {
                        terms.push(format!("{start_col} < {}", bind(b)))
                    }
                    RangeCondition::EndAfter(b) => terms.push(format!("{end_col} > {}", bind(b))),
                    RangeCondition::EndAtOrBefore(b) => {
                        // One bind for both: the implied start bound is the
                        // same instant.
                        let p = bind(b);
                        terms.push(format!("{start_col} < {p}"));
                        terms.push(format!("{end_col} <= {p}"));
                    }
                }
            }
            terms.join(" AND ")
        })
        .collect();
    let sql = match groups.as_slice() {
        [single] => single.clone(),
        _ => format!(
            "({})",
            groups
                .iter()
                .map(|g| if g.contains(" AND ") {
                    format!("({g})")
                } else {
                    g.clone()
                })
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
    };
    (sql, params)
}

/// How a sort key's value is typed for cursor (keyset) binding and comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortValueKind {
    /// Text column (string/token/uri/reference/`_id`).
    Text,
    /// Floating-point column (number/quantity).
    Number,
    /// Timestamp column (`_lastUpdated`, date).
    Timestamp,
}

/// A single keyset sort key: the SQL value expression, its direction, and the
/// value kind used to bind/read the cursor boundary value.
#[derive(Debug, Clone)]
pub struct KeysetKey {
    /// SQL expression yielding the sort value (column or correlated subquery).
    pub expr: String,
    /// Sort direction.
    pub direction: crate::types::SortDirection,
    /// How the value is typed for binding/reading.
    pub kind: SortValueKind,
}

/// Determines the value kind for a sort parameter.
pub(crate) fn sort_value_kind(
    parameter: &str,
    param_type: Option<SearchParamType>,
) -> SortValueKind {
    match parameter {
        "_id" => SortValueKind::Text,
        "_lastUpdated" => SortValueKind::Timestamp,
        _ => match param_type {
            Some(SearchParamType::Number) | Some(SearchParamType::Quantity) => {
                SortValueKind::Number
            }
            Some(SearchParamType::Date) => SortValueKind::Timestamp,
            _ => SortValueKind::Text,
        },
    }
}

/// Maps a search-parameter type to the `search_index` value column used when
/// sorting on that parameter. Returns `None` for types that are not sortable via
/// a single value column (composite, special).
pub(crate) fn sort_value_column(param_type: SearchParamType) -> Option<&'static str> {
    match param_type {
        SearchParamType::String => Some("value_string"),
        SearchParamType::Token => Some("value_token_code"),
        SearchParamType::Date => Some("value_date"),
        SearchParamType::Number => Some("value_number"),
        SearchParamType::Quantity => Some("value_quantity_value"),
        SearchParamType::Reference => Some("value_reference"),
        SearchParamType::Uri => Some("value_uri"),
        SearchParamType::Composite | SearchParamType::Special => None,
    }
}

/// A SQL fragment with associated parameters.
#[derive(Debug, Clone)]
pub struct SqlFragment {
    /// The SQL string with $N placeholders.
    pub sql: String,
    /// The parameter values.
    pub params: Vec<SqlParam>,
}

/// A SQL parameter value.
#[derive(Debug, Clone)]
pub enum SqlParam {
    /// Text parameter.
    Text(String),
    /// Array of text values for large id sets.
    TextArray(Vec<String>),
    /// Floating point parameter.
    Float(f64),
    /// Integer parameter.
    Integer(i64),
    /// Boolean parameter.
    Bool(bool),
    /// Timestamp parameter.
    Timestamp(DateTime<Utc>),
    /// Null parameter.
    Null,
}

impl SqlParam {
    /// Creates a text parameter.
    pub fn text(s: &str) -> Self {
        SqlParam::Text(s.to_string())
    }
}

impl SqlFragment {
    /// Creates a new fragment with no parameters.
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            params: Vec::new(),
        }
    }

    /// Creates a fragment with parameters.
    pub fn with_params(sql: impl Into<String>, params: Vec<SqlParam>) -> Self {
        Self {
            sql: sql.into(),
            params,
        }
    }

    /// Combines two fragments with AND.
    pub fn and(self, other: SqlFragment) -> SqlFragment {
        SqlFragment {
            sql: format!("({}) AND ({})", self.sql, other.sql),
            params: [self.params, other.params].concat(),
        }
    }

    /// Combines two fragments with OR.
    pub fn or(self, other: SqlFragment) -> SqlFragment {
        SqlFragment {
            sql: format!("({}) OR ({})", self.sql, other.sql),
            params: [self.params, other.params].concat(),
        }
    }
}

/// PostgreSQL search query builder.
pub struct PostgresQueryBuilder;

impl PostgresQueryBuilder {
    /// Builds a search query for finding matching resource IDs.
    ///
    /// Returns a SQL fragment that selects DISTINCT resource_ids from search_index
    /// matching the given search parameters.
    pub fn build_search_query(query: &SearchQuery, param_offset: usize) -> Option<SqlFragment> {
        Self::build_search_query_for(query, param_offset, IndexLayout::Denormalized)
    }

    /// As [`Self::build_search_query`], but emitting the form that matches the
    /// database's actual `search_index` layout.
    ///
    /// Only composite search differs. Under [`IndexLayout::Denormalized`] every
    /// component of one composite instance shares a row, so the match is a plain
    /// conjunction. A database that predates v18 still stores one row per
    /// component, where that conjunction matches nothing at all — the search
    /// silently returns an empty bundle rather than failing.
    pub fn build_search_query_for(
        query: &SearchQuery,
        param_offset: usize,
        layout: IndexLayout,
    ) -> Option<SqlFragment> {
        // Repeated occurrences of one parameter name are independent candidate
        // sets ANDed together (`?date=ge2020&date=le2021`). Built as one
        // `id IN (…)` per occurrence the conjunction is two semi-joins, and
        // PostgreSQL may execute the pair as a nested loop that re-runs one
        // branch's scan once per candidate from the other — on the manual-QA
        // corpus 37.5M discarded rows and a ~20 s page (#1416). The same
        // occurrences built as one membership test over their `INTERSECT` are a
        // set operation evaluated once per arm, so the rescanning shape is not
        // in the plan space at all.
        let grouped = Self::foldable_groups(query, param_offset, layout);

        let mut conditions = Vec::new();
        let mut current_offset = param_offset;

        for (index, param) in query.parameters.iter().enumerate() {
            if let Some(members) = grouped[index].as_ref() {
                if members[0] != index {
                    // Emitted with the group's first occurrence, which is where
                    // the intersection is written.
                    continue;
                }

                // Rebuild the group at its emission offsets: the fold moves its
                // occurrences together (`?date=a&gender=m&date=b` emits both date
                // arms before `gender`), so the numbering the plan pass saw no
                // longer holds. Rebuilding in emission order is what keeps the
                // placeholders ascending and `SqlFragment::params` in the order
                // the caller binds them.
                let mut built: Vec<(usize, SqlFragment)> = Vec::with_capacity(members.len());
                for &member in members {
                    let fragment = Self::build_parameter_condition(
                        &query.parameters[member],
                        current_offset,
                        layout,
                    )
                    .expect("a foldable occurrence builds: the plan pass built it");
                    current_offset += fragment.params.len();
                    built.push((member, fragment));
                }

                // The arms are the group's own selects, concatenated verbatim
                // under one `id IN (…)`: nothing is re-derived, so every scoping
                // and every modifier the per-occurrence builders applied
                // survives unchanged.
                let arms: Option<Vec<String>> = built
                    .iter()
                    .map(|(member, fragment)| {
                        Self::simple_membership_arm(&fragment.sql, &query.parameters[*member].name)
                            .map(str::to_string)
                    })
                    .collect();

                match arms {
                    Some(arms) => {
                        let params: Vec<SqlParam> =
                            built.into_iter().flat_map(|(_, f)| f.params).collect();
                        conditions.push(SqlFragment::with_params(
                            format!(
                                "{}{})",
                                Self::INDEX_MEMBERSHIP_OPEN,
                                arms.join(" INTERSECT ")
                            ),
                            params,
                        ));
                    }
                    // Unreachable: an offset only numbers placeholders, so a
                    // member cannot stop being the test the plan pass saw. If one
                    // somehow did, the conjunction it was built from is kept
                    // rather than a constraint dropped.
                    None => conditions.extend(built.into_iter().map(|(_, fragment)| fragment)),
                }
                continue;
            }

            if let Some(condition) = Self::build_parameter_condition(param, current_offset, layout)
            {
                current_offset += condition.params.len();
                conditions.push(condition);
            }
        }

        // Compartment membership: match resources that reference the compartment
        // through ANY of the membership params (OR), per the CompartmentDefinition.
        if let Some(comp) = &query.compartment {
            // Last condition appended, so no need to advance `current_offset`.
            if let Some(condition) = Self::build_compartment_condition(comp, current_offset) {
                conditions.push(condition);
            }
        }

        if conditions.is_empty() {
            return None;
        }

        // AND all conditions together
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.and(cond);
        }

        Some(combined)
    }

    /// Groups the parameter occurrences that may be folded into one membership
    /// test, keyed by occurrence index.
    ///
    /// A group is foldable when a name repeats and *every* one of its
    /// occurrences builds as a simple positive `search_index` membership test on
    /// that name ([`Self::simple_membership_arm`]). One ineligible occurrence
    /// disqualifies the whole name, which then keeps the previous
    /// per-occurrence conjunction byte for byte: `?date=bogus` (the builder fails
    /// closed with a bare `FALSE`), a `:missing` presence test, a `:not`
    /// negation, an FTS or `:identifier` sub-select, a `_id` or `_lastUpdated`
    /// column predicate, an occurrence that builds no condition at all, and the
    /// legacy composite aggregate are all of that kind.
    ///
    /// Keying on the name rather than on adjacency is what catches
    /// `?date=a&gender=m&date=b`, and `members` is in occurrence order, so
    /// `members[0]` is both the group's first occurrence and the one that emits
    /// it.
    fn foldable_groups(
        query: &SearchQuery,
        offset: usize,
        layout: IndexLayout,
    ) -> Vec<Option<Vec<usize>>> {
        let mut groups: Vec<Option<Vec<usize>>> = vec![None; query.parameters.len()];
        let mut occurrences: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, param) in query.parameters.iter().enumerate() {
            occurrences
                .entry(param.name.as_str())
                .or_default()
                .push(index);
        }

        // A name that occurs once cannot fold, and that is every parameter of the
        // common query — so only a repeat is built here at all.
        for members in occurrences.values().filter(|members| members.len() >= 2) {
            let foldable = members.iter().all(|&index| {
                let param = &query.parameters[index];
                Self::build_parameter_condition(param, offset, layout).is_some_and(|fragment| {
                    Self::simple_membership_arm(&fragment.sql, &param.name).is_some()
                })
            });
            if foldable {
                for &index in members {
                    groups[index] = Some(members.clone());
                }
            }
        }

        groups
    }

    /// Builds the compartment-membership subquery: matches resources that
    /// reference `comp.reference` via ANY of `comp.params` (logical OR), mirroring
    /// the SQLite backend. Returns `None` when there are no params or reference.
    fn build_compartment_condition(
        comp: &CompartmentMembership,
        param_offset: usize,
    ) -> Option<SqlFragment> {
        if comp.params.is_empty() || comp.reference.is_empty() {
            return None;
        }

        // Membership params come from the bundled FHIR CompartmentDefinitions
        // (trusted); escape single quotes defensively before inlining.
        let in_list = comp
            .params
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");

        let base = strip_reference_version(&comp.reference).to_string();
        let p1 = param_offset + 1;

        // A plain equality. `value_reference` holds the version-agnostic base
        // (schema v33), so the `LIKE $n || '/_history/%'` arm this used to carry
        // could match nothing — and it was the worst-shaped predicate in the
        // backend: the pattern is an `OpExpr` built in SQL rather than a `Const`,
        // so no fixed prefix could be derived from it under *any* plan, and an OR
        // is index-usable only when every arm is.
        Some(SqlFragment::with_params(
            format!(
                "id IN (SELECT resource_id FROM search_index \
                 WHERE tenant_id = $1 AND resource_type = $2 AND param_name IN ({in_list}) \
                 AND value_reference = ${p1})"
            ),
            vec![SqlParam::text(&base)],
        ))
    }

    /// Builds the `_contained` match subquery (mirrors the SQLite backend's
    /// `build_contained`).
    ///
    /// Returns SQL selecting `(resource_type, resource_id, contained_local_id)`
    /// from `search_index` for contained resources (`is_contained = TRUE`) of the
    /// searched type (`contained_type = $2`) matching every parameter,
    /// keyed on the contained entity `(resource_id, contained_local_id)` via
    /// `GROUP BY ... HAVING COUNT(DISTINCT param_name) >= n`. Value predicates are
    /// the bare row predicates of the top-level builders (`*_value_predicate`,
    /// modifier-aware, #1407) for string, reference, uri and the token display
    /// and `:of-type` forms, and the column conditions shared with
    /// composite-component matching for plain token, number, quantity and date.
    ///
    /// Each occurrence of a parameter is its own AND-ed branch (values within an
    /// occurrence are ORed). Counting distinct names only proves every branch
    /// matched while the names are distinct, so once a name repeats
    /// (`date=ge2020&date=le2020`) the `HAVING` instead requires each branch
    /// with `bool_or(<branch>)`, on the same contained entity. `:not` is the
    /// same aggregate required not to hold — "no row of this entity matches" —
    /// which needs every row of the entity, so the row filter in `WHERE` is
    /// left out when one is present (#1363).
    ///
    /// `_id` is the contained resource's local id, a column of every row (the
    /// writer drops the `_id` rows themselves), and narrows the rows directly.
    /// `_tag`, `_profile`, `_security`, `_source` and `_language` are indexed
    /// from the contained resource's own `meta` like any other parameter. What
    /// the contained rows cannot answer is refused by
    /// [`Self::reject_unsupported_contained`], which every caller runs first;
    /// this function skips those parameters.
    ///
    /// Param layout: `$1` = tenant, `$2` = contained type, then value params.
    ///
    /// `query.compartment` is one more branch, on the contained resource's
    /// own references. With no criterion at all the result is every contained
    /// resource of the type (#1383), so this always returns `Some`. The rows
    /// are unordered; the caller sorts them before paging.
    pub fn build_contained(query: &SearchQuery) -> Option<SqlFragment> {
        // (branch, negated)
        let mut branches: Vec<(String, bool)> = Vec::new();
        let mut entity_filters: Vec<String> = Vec::new();
        let mut params: Vec<SqlParam> = Vec::new();
        let mut distinct_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        // $1 = tenant, $2 = contained_type are bound by the caller.
        let mut offset = 2;

        for param in &query.parameters {
            if Self::contained_unsupported_reason(param).is_some() || param.values.is_empty() {
                continue;
            }

            if param.name == "_id" {
                let placeholders: Vec<String> = param
                    .values
                    .iter()
                    .map(|value| {
                        params.push(SqlParam::text(&value.value));
                        offset += 1;
                        format!("${offset}")
                    })
                    .collect();
                entity_filters.push(format!(
                    "contained_local_id IN ({})",
                    placeholders.join(", ")
                ));
                continue;
            }

            // Defence in depth behind `validate_value_presence` (#1380), as in
            // `build_parameter_condition`: an empty value matches nothing.
            if crate::search::has_empty_value(param) {
                entity_filters.push("FALSE".to_string());
                continue;
            }

            // A composite is decided per `composite_group` of one contained
            // entity, which is not a predicate on one row, so it narrows the
            // entities like `_id` does rather than joining the branches
            // (#1407). Contained rows are always in the one-row-per-component
            // form — `build_contained_rows` does not fold them, whatever the
            // database's layout — so this is the pairing of
            // `build_composite_condition_legacy`, keyed on the contained
            // entity, with every component on the base (slot 1) columns.
            if param.param_type == SearchParamType::Composite {
                let mut alternatives: Vec<String> = Vec::new();
                for value in &param.values {
                    let parts: Vec<&str> = value.value.split('$').collect();
                    // Staged, and committed only once every component parsed.
                    let mut staged: Vec<SqlParam> = Vec::new();
                    let mut predicates: Vec<String> = Vec::new();
                    let mut ok = parts.len() == param.components.len();
                    for (part, component) in parts.iter().zip(&param.components) {
                        if !ok {
                            break;
                        }
                        let cv = Self::parse_component_value(part, component.param_type);
                        match Self::build_composite_component(
                            &cv,
                            component.param_type,
                            offset + staged.len(),
                            1,
                        ) {
                            Some((sql, ps)) => {
                                staged.extend(ps);
                                predicates.push(sql);
                            }
                            None => ok = false,
                        }
                    }
                    if !ok || predicates.is_empty() {
                        alternatives.push(match_nothing().sql);
                        continue;
                    }
                    offset += staged.len();
                    params.extend(staged);
                    let havings: Vec<String> =
                        predicates.iter().map(|p| format!("bool_or({p})")).collect();
                    let prefilter: Vec<String> =
                        predicates.iter().map(|p| format!("({p})")).collect();
                    alternatives.push(format!(
                        "(resource_type, resource_id, contained_local_id) IN \
                         (SELECT resource_type, resource_id, contained_local_id FROM search_index \
                         WHERE tenant_id = $1 AND is_contained = TRUE AND contained_type = $2 \
                         AND param_name = '{}' AND ({}) \
                         GROUP BY resource_type, resource_id, contained_local_id, composite_group \
                         HAVING {})",
                        param.name,
                        prefilter.join(" OR "),
                        havings.join(" AND ")
                    ));
                }
                entity_filters.push(format!("({})", alternatives.join(" OR ")));
                continue;
            }

            let modifier = param.modifier.as_ref();
            let mut or_parts: Vec<String> = Vec::new();
            for value in &param.values {
                // The bare row predicates of the top-level builders (#1407).
                // Each is parenthesized: some are unparenthesized conjunctions.
                let mut next = offset;
                let row_predicate = |(sql, ps): (String, Vec<SqlParam>)| (format!("({sql})"), ps);
                let predicate = match (param.param_type, modifier) {
                    (SearchParamType::Token, Some(SearchModifier::OfType)) => {
                        // A value not in the three-part form names nothing:
                        // fail closed rather than drop the criterion.
                        Some(
                            Self::of_type_value_predicate(value, &mut next)
                                .map(row_predicate)
                                .unwrap_or_else(|| (match_nothing().sql, Vec::new())),
                        )
                    }
                    (
                        SearchParamType::Token,
                        Some(m @ (SearchModifier::Text | SearchModifier::CodeText)),
                    ) => Some(row_predicate(Self::token_display_predicate(
                        matches!(m, SearchModifier::CodeText),
                        value,
                        &mut next,
                    ))),
                    (SearchParamType::String, _) => Some(row_predicate(
                        Self::string_value_predicate(modifier, value, &mut next),
                    )),
                    // A contained resource's date rows are ranges like any
                    // other's (#1391); only a composite component is a point. Already
                    // safe inside the OR: a disjunction comes parenthesized.
                    (SearchParamType::Date, _) => Some(date_range_predicate(
                        "value_date",
                        "value_date_end",
                        value.prefix,
                        &value.value,
                        &mut next,
                    )),
                    (
                        SearchParamType::Token
                        | SearchParamType::Number
                        | SearchParamType::Quantity,
                        _,
                    ) => {
                        // Not a composite: this reuses the component predicate
                        // builder for an ordinary single-valued parameter, which
                        // always lives in slot 1.
                        Self::build_composite_component(value, param.param_type, offset, 1)
                    }
                    (SearchParamType::Reference, Some(SearchModifier::Identifier)) => {
                        // The targets are top-level resources: a contained
                        // resource's identifier rows name its container.
                        let (filter, ps) = Self::reference_identifier_filter(value, &mut next);
                        Some((
                            Self::reference_identifier_predicate(
                                "value_reference",
                                &format!("idx.is_contained = FALSE AND {filter}"),
                            ),
                            ps,
                        ))
                    }
                    (SearchParamType::Reference, _) => Some(row_predicate(
                        Self::reference_value_predicate(modifier, value, &mut next),
                    )),
                    (SearchParamType::Uri, _) => Some(row_predicate(Self::uri_value_predicate(
                        modifier, value, &mut next,
                    ))),
                    (SearchParamType::Composite | SearchParamType::Special, _) => None,
                };
                if let Some((sql, ps)) = predicate {
                    offset += ps.len();
                    or_parts.push(sql);
                    params.extend(ps);
                }
            }
            if or_parts.is_empty() {
                continue;
            }
            branches.push((
                format!(
                    "(param_name = '{}' AND ({}))",
                    param.name,
                    or_parts.join(" OR ")
                ),
                matches!(param.modifier, Some(SearchModifier::Not)),
            ));
            distinct_names.insert(param.name.clone());
        }

        // Compartment membership is a criterion on the contained resource like
        // any other: it references the compartment through ANY of the
        // membership parameters (#1383). That one branch spans several
        // parameter names, so the names an entity matched no longer prove
        // every branch did.
        let mut names_prove_branches = true;
        if let Some(comp) = &query.compartment {
            if !comp.params.is_empty() && !comp.reference.is_empty() {
                let in_list = comp
                    .params
                    .iter()
                    .map(|p| format!("'{}'", p.replace('\'', "''")))
                    .collect::<Vec<_>>()
                    .join(", ");
                offset += 1;
                branches.push((
                    format!("(param_name IN ({in_list}) AND value_reference = ${offset})"),
                    false,
                ));
                params.push(SqlParam::text(strip_reference_version(&comp.reference)));
                names_prove_branches = false;
            }
        }

        // With no criterion at all, every contained resource of the type
        // matches (#1383): the grouping below lists each of them once.
        let any_negated = branches.iter().any(|(_, negated)| *negated);
        let mut sql = String::from(
            "SELECT resource_type, resource_id, contained_local_id FROM search_index \
             WHERE tenant_id = $1 AND is_contained = TRUE AND contained_type = $2",
        );
        for filter in &entity_filters {
            sql.push_str(&format!(" AND {filter}"));
        }
        if !branches.is_empty() && !any_negated {
            let positive: Vec<&str> = branches.iter().map(|(b, _)| b.as_str()).collect();
            sql.push_str(&format!(" AND ({})", positive.join(" OR ")));
        }
        sql.push_str(" GROUP BY resource_type, resource_id, contained_local_id");
        if !branches.is_empty() {
            let having =
                if !any_negated && names_prove_branches && distinct_names.len() == branches.len() {
                    format!("COUNT(DISTINCT param_name) >= {}", distinct_names.len())
                } else {
                    // A repeated name or a negation: one row can satisfy only some
                    // of the branches, so state each. The placeholders are reused,
                    // not rebound.
                    branches
                        .iter()
                        .map(|(branch, negated)| {
                            if *negated {
                                format!("bool_or{branch} IS NOT TRUE")
                            } else {
                                format!("bool_or{branch}")
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" AND ")
                };
            sql.push_str(&format!(" HAVING {having}"));
        }
        Some(SqlFragment::with_params(sql, params))
    }

    /// Why `_contained` matching cannot apply `param`, if it cannot.
    ///
    /// The contained rows of `search_index` hold what the extractor found in
    /// the contained resource itself, one row per value. That answers every
    /// ordinary parameter, the `meta`-derived `_`-parameters and — through
    /// `contained_local_id` — `_id`. It does not answer:
    ///
    /// - `_lastUpdated`: a contained resource has no `meta.lastUpdated` of its
    ///   own, and the container's is not on these rows;
    /// - `_text`, `_content` and the other `_`-parameters that are resolved
    ///   against `resources` or `resource_fts`, which only know the container;
    /// - chains, which it does not follow (composites are paired per
    ///   `composite_group` of the contained entity, #1407);
    /// - `:missing`, refused here since the modifier was introduced and pinned
    ///   as a 400 by the REST and PostgreSQL suites, and the modifiers that are
    ///   not a predicate on one index row (`:in`, `:not-in`, token
    ///   `:above`/`:below`, …). The row-level ones — `:exact`, `:contains`,
    ///   `:text`, `:code-text`, `:of-type`, uri `:below`/`:above`,
    ///   `:identifier`, `:Type` — reuse the top-level builders' bare
    ///   `*_value_predicate` (#1407), the same set SQLite applies.
    fn contained_unsupported_reason(param: &SearchParameter) -> Option<String> {
        if !param.chain.is_empty() {
            return Some("chained parameters are".to_string());
        }
        if param.name == "_id" {
            return param
                .modifier
                .as_ref()
                .map(|m| format!("the ':{m}' modifier on _id is"));
        }
        if param.name.starts_with('_')
            && !matches!(
                param.name.as_str(),
                "_tag" | "_profile" | "_security" | "_source" | "_language"
            )
        {
            return Some("this parameter is".to_string());
        }
        let row_level = match (&param.modifier, param.param_type) {
            // Paired per `composite_group` by `build_contained`. Without its
            // components (the REST layer resolves them) there is nothing to
            // pair, and no modifier applies to a composite.
            (None, SearchParamType::Composite) if !param.components.is_empty() => true,
            (_, SearchParamType::Composite) => {
                return Some(
                    "composite parameters with a modifier or no components are".to_string(),
                );
            }
            (_, SearchParamType::Special) => return Some("special parameters are".to_string()),
            (None | Some(SearchModifier::Not), _) => true,
            (
                Some(SearchModifier::Exact | SearchModifier::Contains | SearchModifier::Text),
                SearchParamType::String,
            ) => true,
            (
                Some(SearchModifier::Text | SearchModifier::CodeText | SearchModifier::OfType),
                SearchParamType::Token,
            ) => true,
            (
                Some(SearchModifier::Contains | SearchModifier::Below | SearchModifier::Above),
                SearchParamType::Uri,
            ) => true,
            (
                Some(SearchModifier::Identifier | SearchModifier::Type(_)),
                SearchParamType::Reference,
            ) => true,
            _ => false,
        };
        if row_level {
            return None;
        }
        param
            .modifier
            .as_ref()
            .map(|m| format!("the ':{m}' modifier is"))
    }

    /// Refuses a `_contained=true|both` search carrying a criterion
    /// [`Self::build_contained`] cannot apply, naming it (#1363). Such
    /// criteria used to be skipped — or, for a modifier, read as a plain
    /// match — so the search answered a different question than the one
    /// asked. A no-op for `_contained=false`.
    ///
    /// `_has` and `_list` live outside `query.parameters` and select
    /// *top-level* resources, which a contained resource never is: nothing
    /// outside its container can reference it. They are refused too (#1383),
    /// and so is `_sort`, which this path would otherwise ignore (#1407).
    pub fn reject_unsupported_contained(query: &SearchQuery) -> Result<(), SearchError> {
        if query.contained == ContainedMode::Off {
            return Ok(());
        }
        for (present, name) in [
            (!query.reverse_chains.is_empty(), "_has"),
            (!query.list.is_empty(), "_list"),
        ] {
            if present {
                return Err(SearchError::QueryParseError {
                    message: format!(
                        "'{name}' cannot be combined with _contained=true or both: it selects \
                         top-level resources, which a contained resource is not"
                    ),
                });
            }
        }
        // `_sort` orders by the *contained* resource's values, which live on
        // index rows this path only groups — it lists matches by container
        // type, id and local id, and `_contained=both` appends them to the
        // top-level page. Returning that order for a `_sort` the client asked
        // for is the silent ignore #1363 rules out, so it is refused (#1407).
        if !query.sort.is_empty() {
            return Err(SearchError::QueryParseError {
                message: "'_sort' cannot be combined with _contained=true or both: sorting \
                          contained matches is not supported on PostgreSQL"
                    .to_string(),
            });
        }
        for param in &query.parameters {
            if let Some(reason) = Self::contained_unsupported_reason(param) {
                let message = format!(
                    "search parameter '{}' cannot be combined with _contained=true or both: \
                     {reason} not supported for contained resources on PostgreSQL",
                    param.name
                );
                return Err(match param.param_type {
                    SearchParamType::Composite => SearchError::InvalidComposite { message },
                    _ => SearchError::QueryParseError { message },
                });
            }
        }
        Ok(())
    }

    /// Builds an `ORDER BY` clause from the query's `_sort` directives.
    ///
    /// Mirrors the SQLite backend's ordering semantics so the two backends stay
    /// at parity. Multiple directives (e.g. `_sort=_lastUpdated,-_id`) are
    /// honored in order, with an `id ASC` tie-breaker appended for stable
    /// pagination when `_id` is not already part of the sort.
    ///
    /// # Supported sort parameters
    ///
    /// - `_id` → `id`
    /// - `_lastUpdated` → `last_updated`
    ///
    /// Any other parameter currently falls back to `id`. Sorting by arbitrary
    /// search parameters would require an additional join against `search_index`
    /// and is not yet implemented (see the search spec assessment).
    ///
    /// Note: this is applied to the first-page and offset paths only. The
    /// cursor (keyset) paths keep their `(last_updated, id)` ordering, which is
    /// required by the keyset `WHERE` comparison; cursor pages therefore always
    /// use the default ordering.
    pub fn build_order_by(query: &SearchQuery) -> String {
        if query.sort.is_empty() {
            return "ORDER BY last_updated DESC, id ASC".to_string();
        }

        let mut clauses: Vec<String> = query
            .sort
            .iter()
            .map(|s| {
                let dir = match s.direction {
                    crate::types::SortDirection::Ascending => "ASC",
                    crate::types::SortDirection::Descending => "DESC",
                };
                format!("{} {}", Self::sort_expression(s), dir)
            })
            .collect();

        let sorts_by_id = query.sort.iter().any(|s| s.parameter == "_id");
        if !sorts_by_id {
            clauses.push("id ASC".to_string());
        }

        format!("ORDER BY {}", clauses.join(", "))
    }

    /// The opening of the membership wrapper every single-parameter condition is
    /// built with, whose matching `)` closes it. The wrapper's contents are the
    /// select the condition closes over, which is what a repeated parameter's
    /// occurrences are concatenated as (#1416).
    const INDEX_MEMBERSHIP_OPEN: &'static str = "id IN (";

    /// The membership wrapper every single-parameter condition is built with.
    ///
    /// `SqlFragment::and` parenthesizes both sides, so a conjunction never
    /// matches this prefix — only a lone membership test does.
    const INDEX_MEMBERSHIP_PREFIX: &'static str = "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND ";

    /// Returns the bare `search_index` predicate when `sql` is exactly one
    /// membership test, so the caller can resolve the page against
    /// `search_index` directly instead of through a subquery on `resources`.
    pub fn single_index_predicate(sql: &str) -> Option<&str> {
        let inner = sql
            .strip_prefix(Self::INDEX_MEMBERSHIP_PREFIX)?
            .strip_suffix(')')?;
        // Defensive: nothing that still mentions the table can be a lone test,
        // and the legacy composite form is an aggregate rather than a row
        // predicate — splicing it into a `SELECT DISTINCT ... ORDER BY` would be
        // nonsense. (Layouts are mutually exclusive, so this is belt and braces.)
        if inner.contains("FROM search_index") || inner.contains("GROUP BY") {
            return None;
        }
        Some(inner)
    }

    /// Returns the `SELECT` a membership test closes over, when `sql` is exactly
    /// one such test **on `name`**.
    ///
    /// [`Self::single_index_predicate`] only asks whether a lone test is there,
    /// because its caller splices the predicate into a query that is already
    /// scoped. Folding a repeated parameter needs to know *which* parameter the
    /// test is scoped to instead, because it concatenates the tests of one name:
    /// the compartment clause shares the wrapper but matches `param_name IN (…)`
    /// over the compartment's own membership params, so the name check is what
    /// keeps a compartment out of a parameter it has nothing to do with. A name
    /// that only ever appears inside a modifier's own predicate (`:identifier`,
    /// the FTS sub-selects) fails the same check, and the legacy composite
    /// aggregate fails the table and aggregate checks [`Self::single_index_predicate`]
    /// makes.
    ///
    /// A set operator of its own is refused too: `s1 UNION s2 INTERSECT s3` binds
    /// as `s1 UNION (s2 INTERSECT s3)`, so an arm that is itself a set expression
    /// cannot be concatenated. No builder emits one today — the check is what
    /// keeps that from becoming a silent mis-plan if one ever does.
    ///
    /// `name` is compared as SQL would quote it, so a name carrying a quote is
    /// left to the conjunction rather than matched loosely.
    fn simple_membership_arm<'a>(sql: &'a str, name: &str) -> Option<&'a str> {
        let select = sql
            .strip_prefix(Self::INDEX_MEMBERSHIP_OPEN)?
            .strip_suffix(')')?;
        // `Self::INDEX_MEMBERSHIP_PREFIX` is this head with the wrapper's opening
        // in front of it, so the two cannot drift apart.
        let head = Self::INDEX_MEMBERSHIP_PREFIX
            .strip_prefix(Self::INDEX_MEMBERSHIP_OPEN)
            .expect("INDEX_MEMBERSHIP_PREFIX opens with INDEX_MEMBERSHIP_OPEN");
        let predicate = select
            .strip_prefix(head)?
            .strip_prefix(&format!("param_name = '{}' AND ", name.replace('\'', "''")))?;
        if predicate.contains("FROM search_index")
            || predicate.contains("GROUP BY")
            || predicate.contains("HAVING")
            || predicate.contains(" UNION ")
            || predicate.contains(" EXCEPT ")
            || predicate.contains(" INTERSECT ")
        {
            return None;
        }
        Some(select)
    }

    /// Returns the keyset sort key for cursor pagination, or `None` when the
    /// query has multiple sort fields (those are returned as a single page
    /// rather than paged with a possibly-inconsistent keyset).
    pub fn primary_keyset_key(query: &SearchQuery) -> Option<KeysetKey> {
        match query.sort.len() {
            0 => Some(KeysetKey {
                expr: "last_updated".to_string(),
                direction: crate::types::SortDirection::Descending,
                kind: SortValueKind::Timestamp,
            }),
            1 => {
                let directive = &query.sort[0];
                Some(KeysetKey {
                    expr: Self::sort_expression(directive),
                    direction: directive.direction,
                    kind: sort_value_kind(&directive.parameter, directive.param_type),
                })
            }
            _ => None,
        }
    }

    /// Builds the ORDER BY expression for a single sort directive.
    ///
    /// `_id`/`_lastUpdated` map to `resources` columns. Any other indexed search
    /// parameter sorts on a correlated subquery into `search_index`, taking the
    /// MIN value for ascending and MAX for descending (FHIR multi-value sort).
    /// A date sorts ascending on the earliest start and descending on the
    /// latest end of the ranges it indexes.
    fn sort_expression(directive: &crate::types::SortDirective) -> String {
        match directive.parameter.as_str() {
            "_id" => return "id".to_string(),
            "_lastUpdated" => return "last_updated".to_string(),
            _ => {}
        }

        match directive.param_type.and_then(sort_value_column) {
            Some(col) => {
                let (agg, col) = match directive.direction {
                    crate::types::SortDirection::Ascending => ("MIN", col),
                    // A date row covers `[value_date, value_date_end)`, so the
                    // latest a resource reaches is the largest end (#1391): a
                    // Period sorts descending by where it ends.
                    crate::types::SortDirection::Descending if col == "value_date" => {
                        ("MAX", "value_date_end")
                    }
                    crate::types::SortDirection::Descending => ("MAX", col),
                };
                format!(
                    "(SELECT {}({}) FROM search_index si WHERE si.tenant_id = $1 AND si.resource_type = $2 AND si.resource_id = resources.id AND si.param_name = '{}')",
                    agg, col, directive.parameter
                )
            }
            // Unsortable (composite/special/unresolved) — stable fallback.
            None => "id".to_string(),
        }
    }

    /// Builds a condition for a single search parameter.
    fn build_parameter_condition(
        param: &SearchParameter,
        param_offset: usize,
        layout: IndexLayout,
    ) -> Option<SqlFragment> {
        if param.values.is_empty() {
            return None;
        }

        // The `:missing` modifier is type-agnostic and resolved purely from the
        // presence/absence of a search_index entry for the parameter.
        if let Some(SearchModifier::Missing) = param.modifier {
            return Some(Self::build_missing_condition(param));
        }

        // Defence in depth behind `validate_value_presence` (#1380): an empty
        // value is a prefix of every string, so it matches nothing here rather
        // than whatever the builder below would make of it — the whole
        // parameter, since under `:not` "nothing" negates into "everything".
        if crate::search::has_empty_value(param) {
            return Some(match_nothing());
        }

        // Handle special parameters
        match param.name.as_str() {
            "_id" => return Self::build_id_condition(param, param_offset),
            "_lastUpdated" => {
                return Self::build_last_updated_condition(&param.values, param_offset);
            }
            // Full text over the generated narrative (`_text`) and over the whole
            // serialized resource (`_content`), against the same `resource_fts`
            // tsvector columns the write path populates.
            "_text" => {
                return Self::build_fts_condition(
                    &param.values,
                    "narrative_tsvector",
                    param_offset,
                );
            }
            "_content" => {
                return Self::build_fts_condition(&param.values, "content_tsvector", param_offset);
            }
            _ => {}
        }

        // Build conditions based on parameter type
        match param.param_type {
            SearchParamType::String => Self::build_string_condition(param, param_offset),
            SearchParamType::Token => Self::build_token_condition(param, param_offset),
            SearchParamType::Date => Self::build_date_condition(param, param_offset),
            SearchParamType::Number => Self::build_number_condition(param, param_offset),
            SearchParamType::Quantity => Self::build_quantity_condition(param, param_offset),
            SearchParamType::Reference => Self::build_reference_condition(param, param_offset),
            SearchParamType::Uri => Self::build_uri_condition(param, param_offset),
            SearchParamType::Composite => match layout {
                IndexLayout::Denormalized => Self::build_composite_condition(param, param_offset),
                IndexLayout::Legacy => Self::build_composite_condition_legacy(param, param_offset),
            },
            SearchParamType::Special => None,
        }
    }

    /// Builds the `_id` condition against `resources.id`.
    ///
    /// `_id` is dispatched here by name, bypassing the generic per-type
    /// modifier handling in `build_parameter_condition` entirely, so it must
    /// honour `param.modifier` itself. Before this, `:not` was silently
    /// dropped and `_id:not=a` built the exact same `id = $n` as a bare
    /// `_id=a`, returning precisely the resource the caller asked to exclude
    /// (#1092). `:missing` is resolved earlier (the type-agnostic check in
    /// `build_parameter_condition`, above) and any other modifier is
    /// rejected before this point by the backend's search entry point, so
    /// only `None` and `Some(SearchModifier::Not)` are handled here.
    ///
    /// A single value composes to `id = $n` (or `id <> $n` when negated).
    /// Several values compose to the flat predicates `id IN ($n, $m, ...)`
    /// or `id NOT IN ($n, $m, ...)`, avoiding a left-deep `OR` tree. Wide sets
    /// use one `text[]` bind to stay within PostgreSQL's parameter limit.
    fn build_id_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        if param.values.is_empty() {
            return None;
        }

        // PostgreSQL limits bind parameters to 65,535. A resolved chain may
        // contain more ids, so send wide sets in one typed array parameter.
        if param.values.len() > LARGE_ID_SET_THRESHOLD {
            let ids = param
                .values
                .iter()
                .map(|value| value.value.clone())
                .collect();
            let sql = if matches!(param.modifier, Some(SearchModifier::Not)) {
                format!("id <> ALL(${}::text[])", offset + 1)
            } else {
                format!("id = ANY(${}::text[])", offset + 1)
            };
            return Some(SqlFragment::with_params(
                sql,
                vec![SqlParam::TextArray(ids)],
            ));
        }

        if matches!(param.modifier, Some(SearchModifier::Not)) {
            let mut params = Vec::with_capacity(param.values.len());
            let placeholders: Vec<String> = param
                .values
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    params.push(SqlParam::text(&value.value));
                    format!("${}", offset + i + 1)
                })
                .collect();
            let sql = if placeholders.len() == 1 {
                format!("id <> {}", placeholders[0])
            } else {
                format!("id NOT IN ({})", placeholders.join(", "))
            };
            return Some(SqlFragment::with_params(sql, params));
        }

        let mut params = Vec::with_capacity(param.values.len());
        let placeholders: Vec<String> = param
            .values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                params.push(SqlParam::text(&value.value));
                format!("${}", offset + i + 1)
            })
            .collect();
        let sql = if placeholders.len() == 1 {
            format!("id = {}", placeholders[0])
        } else {
            format!("id IN ({})", placeholders.join(", "))
        };
        Some(SqlFragment::with_params(sql, params))
    }

    /// Builds the `_text` / `_content` full-text condition against
    /// `resource_fts`.
    ///
    /// Without this, `SearchParamType::Special` fell through to the `None` arm
    /// below and the parameter contributed **no** condition. A search whose only
    /// parameter was `_text` therefore produced an empty filter and returned
    /// every resource of the type — a text query answered with the whole
    /// compartment, not a narrower or empty result. SQLite has handled both
    /// parameters (via FTS5 `MATCH`) all along; this is the PostgreSQL half.
    ///
    /// `plainto_tsquery('english', …)` matches `search_text`/`search_content` in
    /// `search_impl.rs`, and it parameterises the user's term rather than
    /// splicing it, so a term containing tsquery operators is data, not syntax.
    ///
    /// `tenant_id = $1` is not optional: the sub-select yields a bare
    /// `resource_id` set that the outer query intersects with *this* tenant's
    /// resources, so omitting it would let tenant B's Patient/123 select tenant
    /// A's Patient/123 — a cross-tenant match oracle. `resource_type = $2`
    /// likewise keeps an Observation's narrative from selecting a Patient of the
    /// same id. Both are already bound as tenant and resource type by every
    /// caller of `build_search_query` (`search`, `search_count`), the same
    /// invariant `build_missing_condition` and `build_compartment_condition`
    /// rely on.
    fn build_fts_condition(
        values: &[SearchValue],
        column: &str,
        offset: usize,
    ) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        // Numbered off the running count of *accepted* values, not the loop
        // index: a skipped value must not leave a gap, because the caller binds
        // this fragment's params consecutively from `offset`.
        let mut param_num = offset;
        for value in values {
            let term = value.value.trim();
            if term.is_empty() {
                continue;
            }
            param_num += 1;
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM resource_fts \
                     WHERE tenant_id = $1 AND resource_type = $2 \
                     AND {} @@ plainto_tsquery('english', ${}))",
                    column, param_num
                ),
                vec![SqlParam::text(term)],
            ));
        }
        if conditions.is_empty() {
            // Every value was blank. Returning `None` here would drop the
            // parameter and hand back the entire resource type — the defect this
            // function exists to fix — so fail closed instead. A term of nothing
            // matches nothing, which is also what a stopword-only term already
            // does through `plainto_tsquery`.
            return Some(SqlFragment::new("FALSE".to_string()));
        }
        // Repeated values of one parameter are a logical OR, as elsewhere in
        // this builder.
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    /// Builds a `last_updated` comparison against the `resources` column.
    ///
    /// Binds real timestamps — `last_updated` is `TIMESTAMPTZ`, and a text
    /// parameter fails serialization outright (#871) — and applies the same
    /// precision-range semantics as [`Self::build_date_condition`]: `eq` at
    /// day precision means `[day, day+1)`, not a scalar equality that nothing
    /// can ever hit.
    fn build_last_updated_condition(values: &[SearchValue], offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        let mut next = offset;
        for value in values {
            // Not a date: `date_predicate` fails closed with a bind-free
            // `FALSE`. Skipping the value instead would drop the constraint and
            // return every resource of the type (#1289).
            let (sql, params) =
                date_predicate("last_updated", value.prefix, &value.value, &mut next);
            conditions.push(SqlFragment::with_params(sql, params));
        }
        Self::or_values("_lastUpdated", conditions)
    }

    /// Folds the per-value conditions of ONE parameter into the parameter's
    /// condition.
    ///
    /// The values of a parameter are alternatives: `date=2013-04-05,2020-06-01`
    /// is either day. The date, `_lastUpdated`, number and quantity builders
    /// folded them with `AND`, which asks for a resource matching every value —
    /// for two distinct days, nothing (#1300). The conjunction FHIR does define,
    /// a *repeated* parameter (`date=ge2013-01-01&date=le2013-12-31`), reaches
    /// this builder as separate [`SearchParameter`]s and is ANDed one level up in
    /// [`Self::build_search_query_for`]; it never passes through here.
    ///
    /// When every condition is a membership test on `param_name`, the result is
    /// ONE sublink with the predicates OR'd inside it rather than one sublink per
    /// value OR'd together — the plan reason is on `build_token_condition`: a
    /// sublink under an `OR` is not pulled up into a semi-join. Both forms select
    /// the same resources (`∃row: a ∨ b` ≡ `(∃row: a) ∨ (∃row: b)`), and the
    /// single sublink is a row predicate, so [`Self::single_index_predicate`]
    /// may extract it — as it already does for a token or reference OR-list.
    ///
    /// Anything else — `_lastUpdated`, which compares a `resources` column
    /// directly, or a bare [`match_nothing`] — is OR'd as is. `FALSE OR x` is `x`:
    /// an uninterpretable alternative contributes nothing and no longer empties
    /// the parameter. That cannot widen a result, because an all-`FALSE` list is
    /// still `FALSE`, and for dates the search gate (`validate_date_values`)
    /// rejects the whole request before a query is built.
    ///
    /// A single condition is returned untouched. Params are concatenated in
    /// value order, so placeholder numbering is exactly what the caller assigned.
    fn or_values(param_name: &str, mut conditions: Vec<SqlFragment>) -> Option<SqlFragment> {
        if conditions.len() < 2 {
            return conditions.pop();
        }

        let membership = format!(
            "{}param_name = '{}' AND ",
            Self::INDEX_MEMBERSHIP_PREFIX,
            param_name
        );
        let single_sublink = conditions
            .iter()
            .map(|c| c.sql.strip_prefix(&membership)?.strip_suffix(')'))
            .collect::<Option<Vec<&str>>>()
            .map(|predicates| format!("{membership}(({})))", predicates.join(") OR (")));
        if let Some(sql) = single_sublink {
            let params = conditions.into_iter().flat_map(|c| c.params).collect();
            return Some(SqlFragment::with_params(sql, params));
        }

        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    /// Builds an `id IN/NOT IN` condition for the `:missing` modifier.
    ///
    /// `param:missing=true` matches resources with **no top-level** `search_index`
    /// entry for the parameter; `:missing=false` matches resources that **have**
    /// one. Index rows extracted from contained resources do not establish
    /// presence on their container.
    /// Uses only the always-present `$1`/`$2` (tenant, resource type) bind
    /// params, so it adds no parameters to the surrounding query.
    fn build_missing_condition(param: &SearchParameter) -> SqlFragment {
        let is_missing = param
            .values
            .first()
            .map(|v| v.value == "true")
            .unwrap_or(false);
        let inner = match param.name.as_str() {
            "_id" => "SELECT id FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND id IS NOT NULL".to_string(),
            "_lastUpdated" => "SELECT id FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND last_updated IS NOT NULL".to_string(),
            _ => format!(
                "SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND is_contained = FALSE AND param_name = '{}'",
                param.name
            ),
        };
        let sql = if is_missing {
            format!("id NOT IN ({})", inner)
        } else {
            format!("id IN ({})", inner)
        };
        SqlFragment::new(sql)
    }

    /// Builds the `id IN (…)` membership test for a `string` parameter.
    ///
    /// The three FHIR modifiers land on three different index shapes:
    /// - default (starts-with) — a bytewise range on `FOLDED_STRING_EXPR`, which
    ///   `idx_search_string_folded_pattern` seeks. Two bind parameters.
    /// - `:contains`/`:text` — a substring `LIKE`, which no btree can seek;
    ///   served by the trigram GIN index `idx_search_string_trgm` (v34), and by
    ///   the btree pattern index where `pg_trgm` is unavailable.
    /// - `:exact` — `value_string = $n` on the bare column, served by
    ///   `idx_search_string`.
    fn build_string_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        // A value does not always cost exactly one bind parameter — the
        // starts-with form binds a low and a high bound — so the placeholder
        // number is carried rather than derived from the value's position.
        let mut next = offset;

        for value in param.values.iter() {
            let (predicate, params) =
                Self::string_value_predicate(param.modifier.as_ref(), value, &mut next);
            conditions.push(SqlFragment::with_params(
                Self::index_membership(&param.name, &predicate),
                params,
            ));
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    /// Wraps a bare `search_index` row predicate into the top-level membership
    /// test `id IN (SELECT resource_id FROM search_index WHERE … AND
    /// param_name = '<p>' AND <predicate>)`.
    ///
    /// The `*_value_predicate` functions below build the bare predicate for one
    /// search value; the `build_*_condition` functions wrap it with this, and
    /// `build_contained` — which groups the contained rows itself and needs
    /// only the row predicate — uses it bare (#1407). The text is exactly what
    /// each builder used to inline: [`Self::single_index_predicate`] and
    /// [`Self::or_values`] recognize it by [`Self::INDEX_MEMBERSHIP_PREFIX`].
    fn index_membership(param_name: &str, predicate: &str) -> String {
        format!(
            "{}param_name = '{}' AND {})",
            Self::INDEX_MEMBERSHIP_PREFIX,
            param_name,
            predicate
        )
    }

    /// The bare row predicate for one value of a `string` parameter. See
    /// [`Self::build_string_condition`] for the three index shapes. The
    /// starts-with form is an unparenthesized conjunction of two bounds.
    fn string_value_predicate(
        modifier: Option<&SearchModifier>,
        value: &SearchValue,
        next: &mut usize,
    ) -> (String, Vec<SqlParam>) {
        match modifier {
            Some(SearchModifier::Exact) => {
                *next += 1;
                (
                    format!("value_string = ${}", next),
                    vec![SqlParam::text(&value.value)],
                )
            }
            // `:text` on a string is a case-insensitive partial match,
            // implemented here as a substring match (same as `:contains`).
            // Match the accent-folded column (falling back to the raw column
            // for not-yet-reindexed rows) against a folded pattern.
            //
            // A leading `%` is not btree-sargable, so this is served by the
            // trigram GIN index v34 adds — which is why the
            // `value_string IS NOT NULL` conjunct is absent here too. It cost
            // the same 200x row-estimate error it cost the starts-with form,
            // and on that estimate the planner never costed the GIN scan
            // competitively. `~~` is strict in the COALESCE, so it proves both
            // the trigram index's predicate and the btree pattern index's
            // without help. Where the extension is unavailable the btree
            // pattern index serves it at parity with the pre-v34 plan.
            Some(SearchModifier::Contains | SearchModifier::Text) => {
                *next += 1;
                (
                    format!("{} LIKE ${} ESCAPE '\\'", FOLDED_STRING_EXPR, next),
                    vec![SqlParam::text(&format!(
                        "%{}%",
                        like_escape(&fold_text(&value.value))
                    ))],
                )
            }
            _ => {
                // Default: starts-with (case- and accent-insensitive).
                //
                // Emitted as an explicit bytewise range rather than
                // `LIKE 'prefix%'`. The two are exactly equivalent —
                // `like_escape` makes the pattern a pure literal prefix, and
                // `prefix_upper_bound` is the same bound Postgres derives
                // itself — but a range is sargable unconditionally, whereas
                // `LIKE` is only sargable when the planner can see the
                // pattern as a `Const`. It never can here: the pattern is a
                // bind parameter, and any generic plan turns the whole
                // predicate into `~~ like_escape($n, '\')`, a function call
                // on a parameter, from which no prefix can be extracted.
                let folded = fold_text(&value.value);
                match prefix_upper_bound(&folded) {
                    Some(upper) => {
                        let lo = *next + 1;
                        let hi = *next + 2;
                        *next += 2;
                        (
                            format!(
                                "{expr} ~>=~ ${lo} AND {expr} ~<~ ${hi}",
                                expr = FOLDED_STRING_EXPR,
                            ),
                            vec![SqlParam::text(&folded), SqlParam::text(&upper)],
                        )
                    }
                    // No upper bound exists: an empty search value (which
                    // matches every indexed value) or an all-`char::MAX`
                    // prefix. Fall back to the `LIKE` form. The strict `~~`
                    // still proves the index predicate, so this needs no
                    // conjunct either.
                    None => {
                        *next += 1;
                        (
                            format!("{} LIKE ${} ESCAPE '\\'", FOLDED_STRING_EXPR, next),
                            vec![SqlParam::text(&format!("{}%", like_escape(&folded)))],
                        )
                    }
                }
            }
        }
    }

    fn build_token_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        // `:of-type` matches an Identifier by its type (system|code) and value,
        // in the three-part form `type-system|type-code|identifier-value`.
        if let Some(SearchModifier::OfType) = param.modifier {
            return Self::build_of_type_condition(param, offset);
        }

        // `:text` (contains) and `:code-text` (starts-with) match the token's
        // display text (Coding.display / CodeableConcept.text).
        if matches!(
            param.modifier,
            Some(SearchModifier::Text | SearchModifier::CodeText)
        ) {
            let starts_with = matches!(param.modifier, Some(SearchModifier::CodeText));
            let mut conditions = Vec::new();
            let mut next = offset;
            for value in param.values.iter() {
                let (predicate, params) =
                    Self::token_display_predicate(starts_with, value, &mut next);
                conditions.push(SqlFragment::with_params(
                    Self::index_membership(&param.name, &predicate),
                    params,
                ));
            }
            if conditions.is_empty() {
                return None;
            }
            let mut combined = conditions.remove(0);
            for cond in conditions {
                combined = combined.or(cond);
            }
            return Some(combined);
        }

        // An OR-list (`status=finished,in-progress`) becomes ONE subquery with the
        // values OR'd inside it, rather than one `id IN (…)` per value OR'd together.
        //
        // This is a plan fix, not cosmetics. PostgreSQL only pulls a sublink up into
        // a semi-join when it sits in the top-level AND-list of the WHERE clause.
        // Sublinks under an OR stay `SubPlan`s — hashed if the planner's estimate
        // fits in `work_mem`, and otherwise **re-executed for every outer row**. At
        // 1.3M index rows that mis-estimate is easy to hit, and it is the second
        // timeout generator after the composite aggregate. With the OR inside there
        // is a single sublink, so it becomes a semi-join the planner can drive from
        // an index scan.
        //
        // Each value binds a variable number of params: `system|code` binds 2, every
        // other form (code-only, `|code`, `system|`) binds 1. Placeholder numbers
        // must be sequential and gap-free because the caller advances the global
        // offset by `params.len()`; a fixed 2-slot stride once produced a placeholder
        // past the end of the bound params, which Postgres rejects.
        let mut predicates: Vec<String> = Vec::new();
        let mut params: Vec<SqlParam> = Vec::new();
        let mut next = offset;

        for value in param.values.iter() {
            let (predicate, value_params) = Self::token_value_predicate(value, &mut next);
            predicates.push(predicate);
            params.extend(value_params);
        }

        if predicates.is_empty() {
            return None;
        }

        let mut combined = SqlFragment::with_params(
            Self::index_membership(&param.name, &format!("({})", predicates.join(" OR "))),
            params,
        );

        // `:not` reverses the match: include resources that have no matching
        // value (including those with no value at all). Negating the single sublink
        // is equivalent to negating the OR-set of sublinks it replaced.
        if let Some(SearchModifier::Not) = param.modifier {
            let sql = format!("NOT ({})", combined.sql);
            combined = SqlFragment::with_params(sql, combined.params);
        }

        Some(combined)
    }

    /// The bare row predicate for one value of a token `:text` (contains) or
    /// `:code-text` (starts-with) search, on the token's display text.
    fn token_display_predicate(
        starts_with: bool,
        value: &SearchValue,
        next: &mut usize,
    ) -> (String, Vec<SqlParam>) {
        *next += 1;
        let pattern = if starts_with {
            format!("{}%", value.value)
        } else {
            format!("%{}%", value.value)
        };
        (
            format!("value_token_display ILIKE ${}", next),
            vec![SqlParam::text(&pattern)],
        )
    }

    /// The bare row predicate for one value of a plain token search, in its
    /// four forms: `code`, `|code`, `system|` and `system|code`.
    fn token_value_predicate(value: &SearchValue, next: &mut usize) -> (String, Vec<SqlParam>) {
        if let Some((system, code)) = value.value.split_once('|') {
            if system.is_empty() {
                // |code - match code with no system (#1388). A `code`
                // element has no system property either; its row carries
                // the marker (#1379). Same set as the SQLite, Elasticsearch
                // and chain builders.
                *next += 1;
                (
                    format!(
                        "((value_token_system IS NULL OR value_token_system IN ('', '{}')) \
                         AND value_token_code = ${})",
                        IMPLICIT_TOKEN_SYSTEM, next
                    ),
                    vec![SqlParam::text(code)],
                )
            } else if code.is_empty() {
                // system| - match any code in system.
                //
                // `value_token_code IS NOT NULL` is a deliberate,
                // row-set-preserving conjunct, not a filter: it is what makes
                // the partial `idx_search_token_code_recent` (v22, `WHERE
                // value_token_code IS NOT NULL`) a legal candidate for this
                // shape, so a *broad* system streams recent-first and stops
                // at the LIMIT instead of heap-fetching and sorting its whole
                // match set. v31 measures 1074 buffers -> 26 for a 66,667-row
                // system, and it is the reason v31 could replace the 2,283 MB
                // `idx_search_token` with a seek-only index.
                //
                // As of v32 it is not merely helpful, it is LOAD-BEARING:
                // v32 dropped `idx_search_token_system` (the planner pointed
                // `system|code` at it — 80,089,347 tuples read, 358 ms p99),
                // so `idx_search_token_code_recent` is now the ONLY index a
                // `system|` predicate can reach. Remove this conjunct and the
                // form falls back to a sequential-scale scan of the
                // (tenant, type) slice.
                //
                // It excludes nothing. `IndexValue::Token` declares
                // `code: String`, and both writer paths set the column
                // unconditionally beside the system —
                // `IndexRow::from_extracted` and `CompositeRow::place` — so
                // no row this backend has ever written has a system without
                // a code. An empty code is a non-NULL empty string.
                *next += 1;
                (
                    format!(
                        "(value_token_code IS NOT NULL AND value_token_system = ${})",
                        next
                    ),
                    vec![SqlParam::text(system)],
                )
            } else {
                // system|code - exact match, or a `code` element, whose
                // system is implicit and not verifiable here (#1379). The
                // marker is a constant, inlined so this form still binds 2.
                let s = *next + 1;
                let c = *next + 2;
                *next += 2;
                (
                    format!(
                        "(value_token_system IN (${}, '{}') AND value_token_code = ${})",
                        s, IMPLICIT_TOKEN_SYSTEM, c
                    ),
                    vec![SqlParam::text(system), SqlParam::text(code)],
                )
            }
        } else {
            // code only - match any system
            *next += 1;
            (
                format!("value_token_code = ${}", next),
                vec![SqlParam::text(&value.value)],
            )
        }
    }

    /// Builds an `:of-type` identifier condition (token modifier).
    ///
    /// Value form: `type-system|type-code|identifier-value`. Empty parts are
    /// dropped, so e.g. `||MR12345` matches by identifier value only.
    fn build_of_type_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut value_conditions = Vec::new();
        let mut all_params: Vec<SqlParam> = Vec::new();
        let mut current = offset;

        for value in &param.values {
            let Some((predicate, params)) = Self::of_type_value_predicate(value, &mut current)
            else {
                continue;
            };
            all_params.extend(params);
            value_conditions.push(Self::index_membership(&param.name, &predicate));
        }

        if value_conditions.is_empty() {
            return None;
        }
        Some(SqlFragment::with_params(
            value_conditions.join(" OR "),
            all_params,
        ))
    }

    /// The bare row predicate for one `:of-type` value — an unparenthesized
    /// conjunction over the identifier columns of one row — or `None` when the
    /// value is not in the three-part form or names nothing.
    fn of_type_value_predicate(
        value: &SearchValue,
        next: &mut usize,
    ) -> Option<(String, Vec<SqlParam>)> {
        let parts: Vec<&str> = value.value.splitn(3, '|').collect();
        if parts.len() != 3 {
            return None;
        }
        let (type_system, type_code, identifier_value) = (parts[0], parts[1], parts[2]);

        let mut conds = Vec::new();
        let mut params: Vec<SqlParam> = Vec::new();
        // Identifier value (matched against the token code column).
        if !identifier_value.is_empty() {
            *next += 1;
            conds.push(format!("value_token_code = ${}", next));
            params.push(SqlParam::text(identifier_value));
        }
        if !type_system.is_empty() {
            *next += 1;
            conds.push(format!("value_identifier_type_system = ${}", next));
            params.push(SqlParam::text(type_system));
        }
        if !type_code.is_empty() {
            *next += 1;
            conds.push(format!("value_identifier_type_code = ${}", next));
            params.push(SqlParam::text(type_code));
        }
        if conds.is_empty() {
            return None;
        }
        Some((conds.join(" AND "), params))
    }

    /// Builds a composite-parameter condition.
    ///
    /// Composite values join their components with `$` (e.g.
    /// `http://loinc.org|8480-6$lt60`). Each component of a composite instance
    /// is indexed as its own `search_index` row sharing a `composite_group`, so
    /// a resource matches when there is a group in which every component is
    /// satisfied by some row. This is expressed with
    /// `GROUP BY resource_id, composite_group HAVING <every component present>`.
    fn build_composite_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        if param.components.is_empty() {
            return None;
        }

        let mut value_conditions = Vec::new();
        let mut all_params: Vec<SqlParam> = Vec::new();
        let mut current = offset;

        // Slot each component within its column family, in the parameter's own
        // component order — the identical rule the extractor uses when it writes
        // the denormalized row (`ExtractedValue::composite_slot`). Both sides
        // derive it from the same registry ordering, so no mapping is stored.
        let component_slots: Vec<u8> = {
            let mut seen: std::collections::HashMap<SearchParamType, u8> =
                std::collections::HashMap::new();
            param
                .components
                .iter()
                .map(|c| {
                    let slot = seen.entry(c.param_type).or_insert(0);
                    *slot += 1;
                    *slot
                })
                .collect()
        };

        for value in &param.values {
            let parts: Vec<&str> = value.value.split('$').collect();
            if parts.len() != param.components.len() {
                value_conditions.push("1 = 0".to_string());
                continue;
            }

            // Stage this value's params locally and commit them only once every
            // component has parsed. A component that fails midway used to leave the
            // params of its predecessors bound while emitting `1 = 0`, so those
            // params had no placeholder in the final statement and Postgres rejected
            // the whole query ("bind message supplies N parameters, but prepared
            // statement requires M"). e.g. `?code-value-quantity=8867-4$abc` → 500.
            let mut staged_params: Vec<SqlParam> = Vec::new();
            let mut predicates: Vec<String> = Vec::new();
            let mut next = current;
            let mut ok = true;

            for ((part, component), slot) in parts
                .iter()
                .zip(param.components.iter())
                .zip(component_slots.iter())
            {
                let cv = Self::parse_component_value(part, component.param_type);
                match Self::build_composite_component(&cv, component.param_type, next, *slot) {
                    Some((sql, params)) => {
                        next += params.len();
                        staged_params.extend(params);
                        predicates.push(sql);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }

            if !ok || predicates.is_empty() {
                value_conditions.push("1 = 0".to_string());
                continue;
            }

            current = next;
            all_params.extend(staged_params);

            // Each component of a composite instance is a separate `search_index`
            // row, so a resource matches only when every component is satisfied by
            // some row *within the same* `composite_group`. That grouping is what
            // stops a blood-pressure panel's systolic value from pairing with its
            // diastolic code, so the GROUP BY / HAVING structure is load-bearing and
            // is kept exactly as-is.
            //
            // What is new is the WHERE prefilter. Previously the subquery had no
            // value predicate at all: it read every composite row for the parameter
            // and only then filtered inside the aggregate, which meant a full
            // aggregation over the resource type's entire composite slice on every
            // request — the `combo-code-value-quantity` timeout.
            //
            // Dropping rows that satisfy *no* component is provably result-
            // preserving: such a row contributes 0 to every `MAX(CASE …)` in the
            // HAVING, so it can never turn a 0 into a 1, and a group left empty by
            // the prefilter could not have satisfied the HAVING anyway.
            //
            // Measured on the real benchmark dataset (656,737 Observations,
            // 1.8M `code-value-quantity` index rows), which is what these numbers
            // refer to — an earlier, smaller replica gave badly misleading answers
            // here and every alternative below was ranked wrong by it.
            //
            // Three alternatives were tried and REJECTED against real data:
            //
            //   • Dropping the prefilter entirely (the pre-#224 form) and reading the
            //     whole parameter slice index-only from a covering index: 16.5 s. The
            //     prefilter is load-bearing, not incidental.
            //
            //   • A flat self-join of the components correlated on `composite_group`,
            //     driving from the token. Postgres flattens it, then drives from an
            //     ordered scan of `resources` and probes — see below for why that is
            //     fatal here.
            //
            //   • A correlated `EXISTS` (`s.resource_id = resources.id`, grouped per
            //     resource). Only 222 of 656,737 Observations match this composite, so
            //     an ordered scan that probes per row must walk ~190k resources to
            //     collect 21 hits: 8.9 s, and 20 s when nothing matches at all.
            //
            // The common thread: the composite is extremely *sparse*, so the plan must
            // build the small candidate set first and probe `resources` by primary key
            // — never walk `resources` and test each row. The grouped subquery below
            // cannot be flattened or parameterized, which forces exactly that shape.
            // Its unflattenability is a feature; do not "optimize" it into an EXISTS.
            //
            // What remains: this shape still costs ~1 s cold because the Bitmap Heap
            // Scan fetches ~113k rows in ~112k buffer reads — close to one random page
            // per row, since a parameter's rows are scattered across a ~10M-row table.
            // That is I/O, not CPU, and it is why the shape is still ~12 s at 30 VUs.
            // A covering index does not fix it (the OR-prefilter forces heap access by
            // construction). The real fix is a storage change: write one row per
            // composite group with every component's value column populated, so a
            // single index answers "code = X AND value > Y within one group" directly.
            // Tracked separately — it needs a writer change, a migration and a backfill.
            // With the denormalized layout every component of one composite
            // instance lives in a single row, so "code = X AND value > Y within
            // the same group" is a plain conjunction — no prefilter, no
            // GROUP BY, no HAVING. The covering index
            // `idx_search_composite_flat` answers it directly and the LIMIT can
            // stop early, which is what removes the ~110k-row / ~108k-heap-block
            // scan the grouped form performed to return 21 rows.
            //
            // Parenthesize each component: a token component emits
            // `sys = $n AND code = $m`, and relying on AND-before-OR precedence
            // is a trap for the next component type someone adds.
            let conjunction = predicates
                .iter()
                .map(|p| format!("({})", p))
                .collect::<Vec<_>>()
                .join(" AND ");

            value_conditions.push(format!(
                "id IN (SELECT resource_id FROM search_index \
                 WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' \
                 AND composite_group IS NOT NULL AND {})",
                param.name, conjunction
            ));
        }

        if value_conditions.is_empty() {
            return None;
        }
        Some(SqlFragment::with_params(
            value_conditions.join(" OR "),
            all_params,
        ))
    }

    /// Composite search against a pre-v18 `search_index`, where each component of
    /// a composite instance is its own row.
    ///
    /// A resource matches only when every component is satisfied by some row
    /// *within the same* `composite_group` — that grouping is what stops a blood
    /// pressure panel's systolic value from pairing with its diastolic code, so
    /// the GROUP BY / HAVING structure is load-bearing.
    ///
    /// The WHERE prefilter is result-preserving: a row satisfying no component
    /// contributes 0 to every `MAX(CASE ...)` in the HAVING, so it can never turn
    /// a 0 into a 1, and a group the prefilter empties could not have satisfied
    /// the HAVING anyway. Without it the subquery aggregates the resource type's
    /// entire composite slice on every request (#224's timeout).
    ///
    /// This is the pre-#279 form, restored verbatim in behaviour. It is slow —
    /// that slowness is exactly why the denormalized layout exists — but it is
    /// correct against rows the denormalized conjunction cannot match at all.
    /// Components read the base value columns (slot 1): the `_2` columns only
    /// exist to pack two same-type components into one denormalized row.
    fn build_composite_condition_legacy(
        param: &SearchParameter,
        offset: usize,
    ) -> Option<SqlFragment> {
        if param.components.is_empty() {
            return None;
        }

        let mut value_conditions = Vec::new();
        let mut all_params: Vec<SqlParam> = Vec::new();
        let mut current = offset;

        for value in &param.values {
            let parts: Vec<&str> = value.value.split('$').collect();
            if parts.len() != param.components.len() {
                value_conditions.push("1 = 0".to_string());
                continue;
            }

            // Stage this value's params and commit them only once every component
            // has parsed, so a component that fails midway cannot leave its
            // predecessors bound with no placeholder in the final statement.
            let mut staged_params: Vec<SqlParam> = Vec::new();
            let mut predicates: Vec<String> = Vec::new();
            let mut next = current;
            let mut ok = true;

            for (idx, (part, component)) in parts.iter().zip(param.components.iter()).enumerate() {
                let cv = Self::parse_component_value(part, component.param_type);
                // 1-based, and the same order the extractor assigns slots from.
                let slot = (idx + 1).min(u8::MAX as usize) as u8;
                match Self::build_composite_component_transitional(
                    &cv,
                    component.param_type,
                    next,
                    slot,
                ) {
                    Some((sql, params)) => {
                        next += params.len();
                        staged_params.extend(params);
                        predicates.push(sql);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }

            if !ok || predicates.is_empty() {
                value_conditions.push("1 = 0".to_string());
                continue;
            }

            current = next;
            all_params.extend(staged_params);

            let havings = predicates
                .iter()
                .map(|p| format!("MAX(CASE WHEN {} THEN 1 ELSE 0 END) = 1", p))
                .collect::<Vec<_>>()
                .join(" AND ");
            // Parenthesize each component before OR-ing: a token component emits
            // `sys = $n AND code = $m`, and relying on AND-before-OR precedence is
            // a trap for the next component type someone adds.
            let prefilter = predicates
                .iter()
                .map(|p| format!("({})", p))
                .collect::<Vec<_>>()
                .join(" OR ");

            value_conditions.push(format!(
                "id IN (SELECT resource_id FROM search_index \
                 WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' \
                 AND ({}) \
                 GROUP BY resource_id, composite_group HAVING {})",
                param.name, prefilter, havings
            ));
        }

        if value_conditions.is_empty() {
            return None;
        }
        Some(SqlFragment::with_params(
            value_conditions.join(" OR "),
            all_params,
        ))
    }

    /// Parses a composite component value, stripping a comparison prefix.
    ///
    /// Comparison prefixes (`ne`/`gt`/`lt`/`ge`/`le`/`sa`/`eb`/`ap`/`eq`) exist
    /// only for number, date and quantity search values, so they are recognised
    /// **only** for those component types. A token, string, reference or uri
    /// component is taken verbatim — otherwise a code that merely begins with one
    /// of those letter pairs (`left`, `negative`, `ge123`) would be mangled into
    /// `ft`/`gative`/`123` and never match (#1236). Matches Elasticsearch and
    /// MongoDB, which parse prefixes only in their Number/Date/Quantity arms.
    fn parse_component_value(part: &str, param_type: SearchParamType) -> SearchValue {
        if !matches!(
            param_type,
            SearchParamType::Number | SearchParamType::Date | SearchParamType::Quantity
        ) {
            return SearchValue {
                prefix: SearchPrefix::Eq,
                value: part.to_string(),
            };
        }

        let prefixes = [
            ("ne", SearchPrefix::Ne),
            ("gt", SearchPrefix::Gt),
            ("lt", SearchPrefix::Lt),
            ("ge", SearchPrefix::Ge),
            ("le", SearchPrefix::Le),
            ("sa", SearchPrefix::Sa),
            ("eb", SearchPrefix::Eb),
            ("ap", SearchPrefix::Ap),
            ("eq", SearchPrefix::Eq),
        ];
        for (prefix_str, prefix) in prefixes {
            if let Some(stripped) = part.strip_prefix(prefix_str) {
                return SearchValue {
                    prefix,
                    value: stripped.to_string(),
                };
            }
        }
        SearchValue {
            prefix: SearchPrefix::Eq,
            value: part.to_string(),
        }
    }

    /// The column a composite component's value lives in, for a given slot.
    ///
    /// Slot 1 is the ordinary value column. Slot 2 is the `_2` variant, used
    /// when a composite has two components of the same type (24 of the 46 R4
    /// composites do — see `ExtractedValue::composite_slot`). The writer
    /// assigns slots from the same registry component order, so both sides
    /// agree without storing a mapping.
    fn composite_col(base: &str, slot: u8) -> String {
        if slot >= 2 {
            format!("{base}_{slot}")
        } else {
            base.to_string()
        }
    }

    /// A component predicate that matches **both** composite layouts, for use
    /// while a database is being promoted from legacy to denormalized.
    ///
    /// Promotion rewrites resources one page at a time, so for the length of the
    /// run the table holds both shapes at once and reads must still be correct
    /// against each. For most component types the two shapes are already
    /// indistinguishable to a read: only Token and Number have `_2` columns
    /// (`CompositeRow::place`), so a slot-2 component of any other type writes
    /// the base columns either way and needs nothing special.
    ///
    /// Token and Number in slot 2 are the exception, and getting them wrong is
    /// silent. A promoted token+token row puts component 2 in `value_token_code_2`,
    /// which the legacy `HAVING` never reads — so the plain legacy form stops
    /// matching a resource the moment it is rewritten, and 24 of the 46 R4
    /// composites pair two components of the same type. That is a false
    /// *negative*: the resource simply disappears from composite search until
    /// the marker flips.
    ///
    /// The repair is to read the `_2` column when it is populated and fall back
    /// to the base column when it is not:
    ///
    /// ```sql
    /// (code_2 = $1 OR (code_2 IS NULL AND system_2 IS NULL AND code = $2))
    /// ```
    ///
    /// The `IS NULL` guard is what keeps this from introducing a false
    /// *positive*. Simply OR-ing the two columns would let a denormalized row
    /// satisfy a query with its components **swapped** — component 1 matching via
    /// `_2` and component 2 via the base column — which is a different clinical
    /// fact than the one asked for. Component 1 therefore always reads the base
    /// columns only, and component 2 reads the base columns only on rows that
    /// have no slot-2 payload at all, i.e. legacy rows.
    fn build_composite_component_transitional(
        value: &SearchValue,
        param_type: SearchParamType,
        offset: usize,
        slot: u8,
    ) -> Option<(String, Vec<SqlParam>)> {
        // Slot 1 is the base columns under either layout.
        if slot < 2 {
            return Self::build_composite_component(value, param_type, offset, 1);
        }
        // Only Token and Number ever occupy a `_2` column.
        let guard = match param_type {
            SearchParamType::Token => "value_token_code_2 IS NULL AND value_token_system_2 IS NULL",
            SearchParamType::Number => "value_number_2 IS NULL",
            _ => return Self::build_composite_component(value, param_type, offset, 1),
        };

        let (denorm_sql, denorm_params) =
            Self::build_composite_component(value, param_type, offset, slot)?;
        let (legacy_sql, legacy_params) =
            Self::build_composite_component(value, param_type, offset + denorm_params.len(), 1)?;

        let sql = format!("(({denorm_sql}) OR ({guard} AND ({legacy_sql})))");
        let mut params = denorm_params;
        params.extend(legacy_params);
        Some((sql, params))
    }

    /// Builds a single composite component as a bare column predicate (no
    /// `id IN (...)` wrapper — the caller scopes it to the composite row).
    /// Returns `None` if the value cannot be parsed for the component type.
    fn build_composite_component(
        value: &SearchValue,
        param_type: SearchParamType,
        offset: usize,
        slot: u8,
    ) -> Option<(String, Vec<SqlParam>)> {
        let token_code = Self::composite_col("value_token_code", slot);
        let token_system = Self::composite_col("value_token_system", slot);
        let number = Self::composite_col("value_number", slot);
        match param_type {
            SearchParamType::Token => {
                if let Some((system, code)) = value.value.split_once('|') {
                    if system.is_empty() {
                        Some((
                            // No system, or the marker of a `code` component
                            // (#1388); see `token_value_predicate`.
                            format!(
                                "({token_system} IS NULL OR {token_system} IN ('', '{IMPLICIT_TOKEN_SYSTEM}')) \
                                 AND {token_code} = ${}",
                                offset + 1
                            ),
                            vec![SqlParam::text(code)],
                        ))
                    } else if code.is_empty() {
                        Some((
                            format!("{token_system} = ${}", offset + 1),
                            vec![SqlParam::text(system)],
                        ))
                    } else {
                        Some((
                            // Or a `code` component, whose system is
                            // implicit (#1379); see `build_token_condition`.
                            format!(
                                "{token_system} IN (${}, '{IMPLICIT_TOKEN_SYSTEM}') AND {token_code} = ${}",
                                offset + 1,
                                offset + 2
                            ),
                            vec![SqlParam::text(system), SqlParam::text(code)],
                        ))
                    }
                } else {
                    Some((
                        format!("{token_code} = ${}", offset + 1),
                        vec![SqlParam::text(&value.value)],
                    ))
                }
            }
            SearchParamType::String => Some((
                format!("value_string ILIKE ${}", offset + 1),
                vec![SqlParam::text(&format!("{}%", value.value))],
            )),
            SearchParamType::Number => {
                // The standalone `number` predicate on this slot's column, so a
                // component means what the parameter does: the implicit-precision
                // range for `eq`/`ne`, the shared window for `ap`. Not a number:
                // a bare `FALSE` predicate with no params, for the reason given
                // on the Date arm below (#1319). Left unparenthesized, as the
                // quantity arm is: every caller parenthesizes a component or
                // joins it where `AND` already binds tighter than `OR`.
                let mut next = offset;
                Some(
                    number_predicate(&number, value.prefix, &value.value, &mut next)
                        .unwrap_or_else(|| (match_nothing().sql, Vec::new())),
                )
            }
            SearchParamType::Quantity => {
                // As for Number: `FALSE`, never `None` (#1319). Split by the
                // shared grammar, so an escaped `|` stays in the code and the
                // `number|code` shorthand names a code here too. The value is
                // compared as the standalone raw branch compares it; the unit,
                // when given, as stored.
                let Some(quantity) =
                    unvalidated(crate::search::FhirQuantityValue::parse(&value.value))
                else {
                    return Some((match_nothing().sql, Vec::new()));
                };
                let mut next = offset;
                let (mut sql, mut params) = numeric_predicate(
                    "value_quantity_value",
                    value.prefix,
                    &quantity.number,
                    &mut next,
                );
                if let Some(code) = quantity.code.as_deref() {
                    next += 1;
                    params.push(SqlParam::text(code));
                    sql = format!("{sql} AND value_quantity_unit = ${next}");
                }
                Some((sql, params))
            }
            SearchParamType::Date => {
                // The same precision range a standalone date parameter gets;
                // this used to be a scalar comparison with the start of the
                // value, so `eq2024-01-15` meant midnight rather than the day.
                // Not a date: a bare `FALSE` predicate with no params, rather
                // than `None`. Composite callers treat `None` as match-nothing,
                // but `build_contained` skips it, which would drop the
                // constraint and over-match (#1289).
                let mut next = offset;
                let (sql, params) =
                    date_predicate("value_date", value.prefix, &value.value, &mut next);
                // Parenthesized, because callers join predicates with AND/OR;
                // the bind-free `FALSE` stays bare.
                if params.is_empty() {
                    Some((sql, params))
                } else {
                    Some((format!("({sql})"), params))
                }
            }
            _ => None,
        }
    }

    fn build_date_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        let mut next = offset;

        for value in &param.values {
            // Not a date: fail closed with a bare `FALSE`, outside the
            // `id IN (…)` wrapper as before. `continue` would drop the
            // constraint and return every resource of the type (#1289).
            if date_precision_range(&value.value).is_none() {
                conditions.push(match_nothing());
                continue;
            }
            let (sql, params) = date_range_predicate(
                "value_date",
                "value_date_end",
                value.prefix,
                &value.value,
                &mut next,
            );
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {})",
                    param.name, sql
                ),
                params,
            ));
        }

        Self::or_values(&param.name, conditions)
    }

    fn build_number_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        let mut next = offset;

        for value in &param.values {
            // Not a number: fail closed with a bare `FALSE`, as the date
            // builder does. Skipping the value would drop the constraint and
            // return every resource of the type (#1319).
            let Some((sql, params)) =
                number_predicate("value_number", value.prefix, &value.value, &mut next)
            else {
                conditions.push(match_nothing());
                continue;
            };
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {})",
                    param.name, sql
                ),
                params,
            ));
        }

        Self::or_values(&param.name, conditions)
    }

    fn build_quantity_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        // Running placeholder counter: the outer builder advances by the
        // fragment's params.len(), so numbering must be sequential and gap-free.
        let mut next = offset;

        for value in &param.values {
            // Number part is not a number: fail closed, never skip (#1319).
            let Some((predicate, params)) =
                quantity_predicate("", value.prefix, &value.value, &mut next)
            else {
                conditions.push(match_nothing());
                continue;
            };

            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND {})",
                    param.name, predicate
                ),
                params,
            ));
        }

        Self::or_values(&param.name, conditions)
    }

    /// Builds the `:identifier` condition: match references whose target
    /// resource has an identifier equal to the supplied `system|value`.
    ///
    /// # Direction
    ///
    /// The identifier lookup **drives**; the reference index is seeked with what
    /// it produces. The sub-select yields the target's `Type/id` — the exact
    /// form the writer stores (schema v33 also strips any `/_history/<vid>`, so
    /// there is no other stored form to consider) — and the enclosing
    /// `value_reference` is compared against that set. This is the shape the
    /// SQLite backend has always used (`build_identifier_condition`), reached
    /// there for a correctness reason rather than a performance one.
    ///
    /// It used to be written the other way round: a correlated `EXISTS` that
    /// pulled the target id out of each reference row with
    /// `SUBSTRING(ref.value_reference FROM POSITION('/' IN …) + 1)` and looked
    /// it up. That form has no seekable predicate on `ref` at all, so the whole
    /// parameter slice has to be materialized before anything can be discarded,
    /// and the inner lookup bound only `tenant_id` and `param_name` — never
    /// `resource_type` — so it could not seek any index on `search_index` past
    /// its first key column either.
    ///
    /// Measured on a 3.4M-row replica (PostgreSQL 18.6, warm), one
    /// `Observation.subject` identifier search against a 1.34M-row slice and
    /// 490,000 `identifier` rows:
    ///
    /// ```text
    /// correlated EXISTS (before)          Parallel Seq Scan, 80,393 buffers   285.6 ms
    ///   + resource_type bound in EXISTS   Parallel Seq Scan, 81,069 buffers   338.2 ms
    /// identifier drives (this)            Index Scan,           845 buffers     1.4 ms
    /// ```
    ///
    /// The middle row is the point. Binding `resource_type` inside the
    /// correlated `EXISTS` — the obvious repair, and the one the missing bind
    /// invites — makes it **slower**: the inner lookup was never the cost, and
    /// the extra join key only widens the hash. The cost is that the outer
    /// `ref` scan cannot be restricted at all, and only reversing the direction
    /// removes it. 209x, and 95x fewer buffers.
    ///
    /// # What changes semantically
    ///
    /// Nothing that a valid FHIR reference can express. `Reference.reference` is
    /// a relative `Type/id`, an absolute URL, or a `#fragment`. The old
    /// `SUBSTRING` never resolved an absolute URL either — it splits on the
    /// *first* `/`, so `http://ex.org/fhir/Patient/1` yielded
    /// `/ex.org/fhir/Patient/1` and matched no resource id — and a `#fragment`
    /// resolved to itself, which is not a resource id. Both are unmatched before
    /// and after. The one input that behaved differently is a bare stored id
    /// (`"reference": "123"`), which the old form matched against **any**
    /// resource type's identifier rows in the tenant; that is invalid FHIR and
    /// was a cross-type collision rather than a feature.
    ///
    /// # The inner lookup still does not bind `resource_type`
    ///
    /// It cannot: the target's type is what the sub-select is discovering. On
    /// PostgreSQL 18 the btree skip scan handles the gap (`Index Searches: 9` in
    /// the measurement above). On 13-17 it degrades to scanning the tenant's
    /// slice of `idx_search_token_code` with the type filtered — but that is now
    /// **one** scan feeding a seek, not a scan per row of the reference slice.
    fn build_reference_identifier_condition(
        param: &SearchParameter,
        offset: usize,
    ) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        let mut next = offset; // running 0-based param offset

        for value in &param.values {
            let (filter, params) = Self::reference_identifier_filter(value, &mut next);
            conditions.push(SqlFragment::with_params(
                format!(
                    "id IN (SELECT ref.resource_id FROM search_index ref \
                     WHERE ref.tenant_id = $1 AND ref.resource_type = $2 AND ref.param_name = '{}' \
                     AND {})",
                    param.name,
                    Self::reference_identifier_predicate("ref.value_reference", &filter)
                ),
                params,
            ));
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    /// The filter on the target's `identifier` rows (`idx`) for one
    /// `:identifier` value, in its four forms.
    fn reference_identifier_filter(
        value: &SearchValue,
        next: &mut usize,
    ) -> (String, Vec<SqlParam>) {
        match value.value.split_once('|') {
            Some(("", code)) => {
                *next += 1;
                (
                    format!(
                        "(idx.value_token_system IS NULL OR idx.value_token_system = '') AND idx.value_token_code = ${next}"
                    ),
                    vec![SqlParam::text(code)],
                )
            }
            Some((system, "")) => {
                *next += 1;
                (
                    format!("idx.value_token_system = ${next}"),
                    vec![SqlParam::text(system)],
                )
            }
            Some((system, code)) => {
                let s = *next + 1;
                let c = *next + 2;
                *next += 2;
                (
                    format!("idx.value_token_system = ${s} AND idx.value_token_code = ${c}"),
                    vec![SqlParam::text(system), SqlParam::text(code)],
                )
            }
            None => {
                *next += 1;
                (
                    format!("idx.value_token_code = ${next}"),
                    vec![SqlParam::text(&value.value)],
                )
            }
        }
    }

    /// The bare row predicate of an `:identifier` search: `column` — the
    /// reference row's `value_reference`, qualified as the caller needs — is
    /// among the `Type/id` of the resources whose `identifier` rows pass
    /// `filter`.
    ///
    /// `idx.tenant_id = $1` is load-bearing, not defensive: without it the
    /// sub-select yields any tenant's target, and this tenant's rows match
    /// on the strength of another tenant's identifiers.
    fn reference_identifier_predicate(column: &str, filter: &str) -> String {
        format!(
            "{column} IN \
               (SELECT idx.resource_type || '/' || idx.resource_id \
                  FROM search_index idx \
                 WHERE idx.tenant_id = $1 AND idx.param_name = 'identifier' \
                   AND {filter})"
        )
    }

    /// Builds the condition for a reference parameter.
    ///
    /// # Bare ids
    ///
    /// Per [FHIR R4 search on references](https://hl7.org/fhir/R4/search.html#reference),
    /// `[parameter]=[id]` — a bare logical id — is the *primary* form, with
    /// `[type]/[id]` an additional one. References are indexed as their
    /// version-agnostic base, so the common `Patient/<id>` literal is what sits
    /// in `value_reference`. A bare id is compared with the last `/`-delimited
    /// segment. This preserves polymorphic matching across relative and absolute
    /// references while using the v42 expression index. Comparing only the raw
    /// value made `Observation?patient=<id>` return an empty Bundle (#490), while
    /// `patient=Patient/<id>` worked.
    ///
    /// A `:Type` modifier resolves the ambiguity up front: `subject:Patient=<id>`
    /// is normalized to `Patient/<id>` and matched as a type-prefixed reference,
    /// mirroring the SQLite handler.
    ///
    /// Matching stays version-agnostic throughout, but from **both** ends now:
    /// the search value is stripped of any `/_history/<vid>` here, and the stored
    /// value was stripped by the writer (schema v33). The `OR value_reference
    /// LIKE '<base>/\_history/%'` arm this used to carry is therefore gone —
    /// there is no longer a stored form for it to find. That arm was the second
    /// index probe of every reference search in the benchmark and it forced the
    /// whole predicate into a `BitmapOr`, which costs a bitmap build and a
    /// re-check of the full disjunction (a `LIKE` per row) on every matching
    /// heap tuple. Measured on a 3.4M-row replica over 300 searches, warm:
    /// **1.50 ms/call -> 0.47 ms/call**.
    ///
    /// A plain bare id emits one equality on [`REFERENCE_TARGET_ID_EXPR`]. The
    /// `value_reference IS NOT NULL` conjunct matches the v42 partial-index
    /// predicate. `%` and `_` remain literal id characters because this path no
    /// longer constructs a `LIKE` pattern. The `:contains`, `:below`, and
    /// `:above` modifier paths retain their existing pattern behavior.
    fn build_reference_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        if matches!(param.modifier.as_ref(), Some(SearchModifier::Identifier)) {
            return Self::build_reference_identifier_condition(param, offset);
        }

        let mut conditions: Vec<String> = Vec::new();
        let mut params: Vec<SqlParam> = Vec::new();
        // Each plain reference value binds once, but the modifier paths share
        // this loop, so `param_num` runs as a counter rather than `offset + i`.
        // `build_search_query` advances the next parameter's offset by
        // `params.len()`, so this stays consistent.
        let mut param_num = offset;
        for value in &param.values {
            let (predicate, value_params) =
                Self::reference_value_predicate(param.modifier.as_ref(), value, &mut param_num);
            conditions.push(predicate);
            params.extend(value_params);
        }

        if conditions.is_empty() {
            return None;
        }

        // One sublink with the values OR'd inside, not one sublink per value OR'd
        // together — see `build_token_condition` for why (an OR'd sublink is left as
        // a re-executable SubPlan instead of being pulled up into a semi-join). The
        // benchmark fires 2- and 3-id reference OR-lists, so this path is hot.
        Some(SqlFragment::with_params(
            format!(
                "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = '{}' AND ({}))",
                param.name,
                conditions.join(" OR ")
            ),
            params,
        ))
    }

    /// The bare row predicate for one value of a reference search (every form
    /// but `:identifier`, which spans two index rows — see
    /// [`Self::reference_identifier_predicate`]).
    fn reference_value_predicate(
        modifier: Option<&SearchModifier>,
        value: &SearchValue,
        param_num: &mut usize,
    ) -> (String, Vec<SqlParam>) {
        // :contains - case-insensitive substring on the stored reference.
        // :text (contains) / :code-text (starts-with) match the indexed
        // Reference.display text.
        let type_modifier = match modifier {
            Some(SearchModifier::Type(type_name)) => Some(type_name.as_str()),
            _ => None,
        };
        let mut params: Vec<SqlParam> = Vec::new();

        let predicate = match modifier {
            Some(SearchModifier::Text) => {
                *param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!("value_reference_display ILIKE '%' || ${} || '%'", param_num)
            }
            Some(SearchModifier::CodeText) => {
                *param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!("value_reference_display ILIKE ${} || '%'", param_num)
            }
            Some(SearchModifier::Contains) => {
                *param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!("value_reference ILIKE '%' || ${} || '%'", param_num)
            }
            Some(SearchModifier::Below) => {
                // URL/path-prefix hierarchy (canonical |version not handled).
                *param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!(
                    "(value_reference = ${0} OR value_reference LIKE ${0} || '/%')",
                    param_num
                )
            }
            Some(SearchModifier::Above) => {
                *param_num += 1;
                params.push(SqlParam::text(&value.value));
                format!(
                    "(${0} = value_reference OR ${0} LIKE value_reference || '/%')",
                    param_num
                )
            }
            _ => {
                // Plain reference match. Normalize `:Type` + bare id to `Type/id`,
                // then match version-agnostically off the version-stripped base.
                let stripped = strip_reference_version(&value.value);
                let base = match type_modifier {
                    Some(type_name) if !stripped.contains('/') => {
                        format!("{}/{}", type_name, stripped)
                    }
                    _ => stripped.to_string(),
                };
                if base.contains('/') {
                    // `Type/id` or an absolute URL. One equality: the stored
                    // value is the version-agnostic base (schema v33), so a
                    // stored version cannot hide a match from it.
                    let exact = *param_num + 1;
                    *param_num += 1;
                    params.push(SqlParam::text(&base));
                    format!("value_reference = ${exact}")
                } else {
                    // Bare logical id: compare with the final reference segment.
                    // The explicit NULL check matches the v42 partial index.
                    *param_num += 1;
                    params.push(SqlParam::text(&base));
                    format!(
                        "(value_reference IS NOT NULL AND {REFERENCE_TARGET_ID_EXPR} = ${param_num})"
                    )
                }
            }
        };
        (predicate, params)
    }

    fn build_uri_condition(param: &SearchParameter, offset: usize) -> Option<SqlFragment> {
        let mut conditions = Vec::new();
        let mut next = offset;

        for value in param.values.iter() {
            let (predicate, params) =
                Self::uri_value_predicate(param.modifier.as_ref(), value, &mut next);
            conditions.push(SqlFragment::with_params(
                Self::index_membership(&param.name, &predicate),
                params,
            ));
        }

        if conditions.is_empty() {
            return None;
        }
        let mut combined = conditions.remove(0);
        for cond in conditions {
            combined = combined.or(cond);
        }
        Some(combined)
    }

    /// The bare row predicate for one value of a `uri` search: equality, or
    /// the `:contains` / `:below` / `:above` string forms.
    fn uri_value_predicate(
        modifier: Option<&SearchModifier>,
        value: &SearchValue,
        next: &mut usize,
    ) -> (String, Vec<SqlParam>) {
        *next += 1;
        let predicate = match modifier {
            Some(SearchModifier::Contains) => format!("value_uri ILIKE '%' || ${} || '%'", next),
            Some(SearchModifier::Below) => format!("value_uri LIKE ${} || '%'", next),
            Some(SearchModifier::Above) => format!("${} LIKE value_uri || '%'", next),
            _ => format!("value_uri = ${}", next),
        };
        (predicate, vec![SqlParam::text(&value.value)])
    }

    /// Converts a FHIR search prefix to a SQL comparison operator.
    fn prefix_to_operator(prefix: &SearchPrefix) -> &'static str {
        match prefix {
            SearchPrefix::Eq => "=",
            SearchPrefix::Ne => "!=",
            SearchPrefix::Gt => ">",
            SearchPrefix::Lt => "<",
            SearchPrefix::Ge => ">=",
            SearchPrefix::Le => "<=",
            SearchPrefix::Sa => ">", // starts after
            SearchPrefix::Eb => "<", // ends before
            SearchPrefix::Ap => "=", // approximately (simplified)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompositeSearchComponent, SearchModifier, SearchQuery};

    fn date_param(name: &str, prefix: SearchPrefix, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(prefix, value)],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn single_membership_test_is_extractable() {
        let query = SearchQuery::new("Encounter").with_parameter(date_param(
            "date",
            SearchPrefix::Gt,
            "2010-01-01",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(pred, "param_name = 'date' AND value_date_end > $3");
        // The extracted predicate must stand alone against `search_index`.
        assert!(!pred.contains("resources"));
        assert!(!pred.contains("SELECT"));
    }

    #[test]
    fn conjunction_is_not_extractable() {
        // Two parameters AND together, so no single index predicate resolves the
        // page — taking either one alone would over-match.
        let query = SearchQuery::new("Encounter")
            .with_parameter(date_param("date", SearchPrefix::Gt, "2010-01-01"))
            .with_parameter(date_param("date", SearchPrefix::Lt, "2020-01-01"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(PostgresQueryBuilder::single_index_predicate(&frag.sql).is_none());
    }

    fn multi_value_param(
        name: &str,
        param_type: SearchParamType,
        values: Vec<SearchValue>,
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier: None,
            values,
            chain: vec![],
            components: vec![],
        }
    }

    /// Every `$N` in `sql`, in order of appearance.
    fn placeholders(sql: &str) -> Vec<usize> {
        let mut found = Vec::new();
        let mut rest = sql;
        while let Some(at) = rest.find('$') {
            rest = &rest[at + 1..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse() {
                found.push(n);
            }
        }
        found
    }

    /// The caller-owned `$1`/`$2` ignored, the order in which the rest first
    /// appear — so "sequential and gap-free" is checkable from the text alone.
    fn bind_numbers(sql: &str, offset: usize) -> Vec<usize> {
        let mut seen: Vec<usize> = Vec::new();
        for n in placeholders(sql) {
            if n > offset && !seen.contains(&n) {
                seen.push(n);
            }
        }
        seen
    }

    #[test]
    fn date_or_list_is_a_single_extractable_sublink() {
        // `date=2013-04-05,2020-06-01`: the values of one parameter are
        // alternatives (#1300). This test used to build the same parameter and
        // assert it was NOT extractable, on the reading that the values AND at
        // resource level; that reading was the bug. As an OR the match is one row
        // satisfying either range, which is exactly what a single-row predicate
        // expresses — the same shape a token OR-list has always extracted to.
        let query = SearchQuery::new("Procedure").with_parameter(multi_value_param(
            "date",
            SearchParamType::Date,
            vec![
                SearchValue::new(SearchPrefix::Eq, "2013-04-05"),
                SearchValue::new(SearchPrefix::Eq, "2020-06-01"),
            ],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert_eq!(
            frag.sql,
            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = 'date' AND ((value_date >= $3 AND value_date < $4 AND value_date_end <= $4) OR (value_date >= $5 AND value_date < $6 AND value_date_end <= $6)))"
        );
        assert_eq!(frag.sql.matches("FROM search_index").count(), 1);
        assert_eq!(frag.params.len(), 4);
        match frag.params.as_slice() {
            [
                SqlParam::Timestamp(a),
                SqlParam::Timestamp(b),
                SqlParam::Timestamp(c),
                SqlParam::Timestamp(d),
            ] => {
                assert_eq!(a.to_rfc3339(), "2013-04-05T00:00:00+00:00");
                assert_eq!(b.to_rfc3339(), "2013-04-06T00:00:00+00:00");
                assert_eq!(c.to_rfc3339(), "2020-06-01T00:00:00+00:00");
                assert_eq!(d.to_rfc3339(), "2020-06-02T00:00:00+00:00");
            }
            other => panic!("expected the two day ranges in value order: {other:?}"),
        }

        // The extracted predicate is spliced after `AND` on the fast path, so the
        // alternatives must arrive parenthesized or the `OR` would escape the
        // tenant/type scoping in front of it.
        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("an OR list over one parameter is a single-row predicate");
        assert_eq!(
            pred,
            "param_name = 'date' AND ((value_date >= $3 AND value_date < $4 AND value_date_end <= $4) OR (value_date >= $5 AND value_date < $6 AND value_date_end <= $6))"
        );
    }

    #[test]
    fn prefixed_date_or_list_keeps_each_prefix() {
        // `date=lt2014-01-01,gt2020-01-01` — outside the window, which no single
        // value can say. Under AND it was unsatisfiable for a single-valued date.
        let query = SearchQuery::new("Procedure").with_parameter(multi_value_param(
            "date",
            SearchParamType::Date,
            vec![
                SearchValue::new(SearchPrefix::Lt, "2014-01-01"),
                SearchValue::new(SearchPrefix::Gt, "2020-01-01"),
            ],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(
            frag.sql
                .ends_with("AND ((value_date < $3) OR (value_date_end > $4)))"),
            "{}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn last_updated_or_list_is_a_disjunction() {
        // `_lastUpdated` compares a `resources` column, so there is no sublink to
        // merge: the ranges are OR'd directly, each side parenthesized.
        let query = SearchQuery::new("Patient").with_parameter(special_param(
            "_lastUpdated",
            vec![
                SearchValue::new(SearchPrefix::Eq, "2013-04-05"),
                SearchValue::new(SearchPrefix::Eq, "2020-06-01"),
            ],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert_eq!(
            frag.sql,
            "(last_updated >= $3 AND last_updated < $4) OR (last_updated >= $5 AND last_updated < $6)"
        );
        assert_eq!(frag.params.len(), 4);
        assert!(PostgresQueryBuilder::single_index_predicate(&frag.sql).is_none());
    }

    #[test]
    fn number_and_quantity_or_lists_are_single_sublinks_with_gap_free_placeholders() {
        // The same fold sat at the end of the number and quantity builders.
        // Quantity is the stress case for numbering: a value with a convertible
        // unit binds raw bounds, unit, system, canonical bounds and canonical
        // unit, and the next value must continue from wherever that ended.
        for (name, param_type, a, b) in [
            ("probability", SearchParamType::Number, "0.2", "0.8"),
            (
                "value-quantity",
                SearchParamType::Quantity,
                "5|http://unitsofmeasure.org|mg",
                "50",
            ),
        ] {
            let query = SearchQuery::new("Observation").with_parameter(multi_value_param(
                name,
                param_type,
                vec![
                    SearchValue::new(SearchPrefix::Eq, a),
                    SearchValue::new(SearchPrefix::Eq, b),
                ],
            ));
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

            assert!(!frag.sql.contains(") AND (id IN"), "{name}: {}", frag.sql);
            assert_eq!(
                frag.sql.matches("FROM search_index").count(),
                1,
                "{name}: {}",
                frag.sql
            );
            assert!(frag.sql.contains(") OR ("), "{name}: {}", frag.sql);
            assert!(
                PostgresQueryBuilder::single_index_predicate(&frag.sql).is_some(),
                "{name}: {}",
                frag.sql
            );

            // `$1`/`$2` are the caller's; everything after runs 3..=2+len with no
            // gap, and first appearances are in order.
            let mut seen: Vec<usize> = Vec::new();
            for n in placeholders(&frag.sql) {
                if n > 2 && !seen.contains(&n) {
                    seen.push(n);
                }
            }
            let expected: Vec<usize> = (3..3 + frag.params.len()).collect();
            assert_eq!(seen, expected, "{name}: {}", frag.sql);
        }
    }

    #[test]
    fn single_value_conditions_are_unchanged_by_the_or_fold() {
        // One value has nothing to fold; its SQL is byte-for-byte what it was.
        let query = SearchQuery::new("Procedure").with_parameter(date_param(
            "date",
            SearchPrefix::Eq,
            "2013-04-05",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
        assert_eq!(
            frag.sql,
            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = $2 AND param_name = 'date' AND value_date >= $3 AND value_date < $4 AND value_date_end <= $4)"
        );
    }

    /// A repeated parameter is two occurrences of one name, and a repeat is an
    /// AND of two independent candidate sets. Built as two `id IN (…)` sublinks
    /// that is two semi-joins, and PostgreSQL may run the pair as a nested loop
    /// that re-runs one arm's scan once per candidate from the other — 37.5M
    /// discarded rows and a ~20 s page on the manual-QA corpus (#1416). One
    /// membership test over the occurrences' `INTERSECT` is a set operation
    /// evaluated once per arm, so that shape is not in the plan space at all.
    #[test]
    fn repeated_date_occurrences_become_one_intersection() {
        // `date=2013-04-05,2020-06-01&date=gt2019-01-01`: the first occurrence
        // is itself an OR-list, and its ranges keep their parentheses — the `OR`
        // must not escape the scoping the sublink carries.
        let query = SearchQuery::new("Procedure")
            .with_parameter(multi_value_param(
                "date",
                SearchParamType::Date,
                vec![
                    SearchValue::new(SearchPrefix::Eq, "2013-04-05"),
                    SearchValue::new(SearchPrefix::Eq, "2020-06-01"),
                ],
            ))
            .with_parameter(date_param("date", SearchPrefix::Gt, "2019-01-01"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert_eq!(
            frag.sql,
            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND \
             resource_type = $2 AND param_name = 'date' AND ((value_date >= $3 AND \
             value_date < $4 AND value_date_end <= $4) OR (value_date >= $5 AND \
             value_date < $6 AND value_date_end <= $6)) INTERSECT \
             SELECT resource_id FROM search_index WHERE tenant_id = $1 AND \
             resource_type = $2 AND param_name = 'date' AND value_date_end > $7)"
        );
        // One test, one row predicate, one scan per occurrence — and no `AND` of
        // two sublinks left for the planner to nest.
        assert_eq!(frag.sql.matches("id IN (").count(), 1);
        assert_eq!(frag.sql.matches("FROM search_index").count(), 2);
        assert_eq!(frag.sql.matches(" INTERSECT ").count(), 1);
        assert!(!frag.sql.contains(") AND ("), "{}", frag.sql);

        // The arms are the occurrences' own binds, in occurrence order: the
        // OR-list's two day ranges, then `gt`'s first instant after the day.
        assert_eq!(bind_numbers(&frag.sql, 2), vec![3, 4, 5, 6, 7]);
        assert_eq!(
            timestamps(&frag.params),
            [
                "2013-04-05T00:00:00+00:00",
                "2013-04-06T00:00:00+00:00",
                "2020-06-01T00:00:00+00:00",
                "2020-06-02T00:00:00+00:00",
                "2019-01-02T00:00:00+00:00",
            ]
        );

        // A set operation is still not a single row predicate, so the page keeps
        // falling back to the join-everything path — as it did for the
        // conjunction this replaces.
        assert!(PostgresQueryBuilder::single_index_predicate(&frag.sql).is_none());
    }

    /// The fold moves a name's occurrences together, so it renumbers their
    /// placeholders: `?date=a&gender=m&date=b` emits both date arms first, and the
    /// binds must follow the text rather than the request, or `$4` would receive
    /// the value written for `$5`.
    #[test]
    fn an_intervening_parameter_follows_its_group_with_binds_in_emit_order() {
        let query = SearchQuery::new("Encounter")
            .with_parameter(date_param("date", SearchPrefix::Ge, "2020-01-01"))
            .with_parameter(token_param("status", None, "finished"))
            .with_parameter(date_param("date", SearchPrefix::Le, "2021-01-01"));

        for offset in [2, 4] {
            let frag = PostgresQueryBuilder::build_search_query(&query, offset).expect("condition");

            // The intersection is written where the group's first occurrence is,
            // and the parameter that sat in between follows it.
            let (fused, rest) = frag
                .sql
                .split_once(" INTERSECT ")
                .unwrap_or_else(|| panic!("offset {offset}: {}", frag.sql));
            assert!(
                fused.starts_with("(id IN (SELECT"),
                "offset {offset}: {}",
                frag.sql
            );
            assert!(
                rest.contains("param_name = 'date' AND (value_date < "),
                "offset {offset}: {}",
                frag.sql
            );
            assert!(
                rest.contains(") AND (id IN (SELECT"),
                "offset {offset}: {}",
                frag.sql
            );
            assert!(
                rest.contains("param_name = 'status'"),
                "offset {offset}: {}",
                frag.sql
            );
            assert_eq!(rest.matches(" INTERSECT ").count(), 0, "{}", frag.sql);

            // Every bind, in the order its placeholder first appears.
            let expected: Vec<usize> = (offset + 1..=offset + frag.params.len()).collect();
            assert_eq!(bind_numbers(&frag.sql, offset), expected, "{}", frag.sql);
            // Each range arm binds three (#1391): `ge` is "ends after the
            // day, or lies within it", `le` "starts before it, or lies within
            // it".
            let (dates, status) = frag.params.split_at(6);
            assert_eq!(
                timestamps(dates),
                [
                    "2020-01-02T00:00:00+00:00",
                    "2020-01-01T00:00:00+00:00",
                    "2020-01-02T00:00:00+00:00",
                    "2021-01-01T00:00:00+00:00",
                    "2021-01-01T00:00:00+00:00",
                    "2021-01-02T00:00:00+00:00",
                ],
                "offset {offset}: both date arms first"
            );
            match status {
                [SqlParam::Text(status)] => assert_eq!(status, "finished"),
                other => panic!("offset {offset}: expected `status` last, got {other:?}"),
            }
        }
    }

    /// Three occurrences, and a name whose values repeat: each occurrence is one
    /// arm, in occurrence order, with no placeholder left over or reused.
    #[test]
    fn every_occurrence_of_a_name_becomes_its_own_arm() {
        for (label, values) in [
            ("three days", ["2020-01-01", "2021-01-01", "2022-01-01"]),
            ("the same day three times", ["2020-01-01"; 3]),
        ] {
            let mut query = SearchQuery::new("Encounter");
            for value in values {
                query = query.with_parameter(date_param("date", SearchPrefix::Eq, value));
            }
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

            assert_eq!(
                frag.sql.matches("id IN (").count(),
                1,
                "{label}: {}",
                frag.sql
            );
            assert_eq!(
                frag.sql.matches("FROM search_index").count(),
                3,
                "{label}: {}",
                frag.sql
            );
            assert_eq!(
                frag.sql.matches(" INTERSECT ").count(),
                2,
                "{label}: {}",
                frag.sql
            );
            assert_eq!(
                frag.sql.matches("param_name = 'date' AND").count(),
                3,
                "{label}: {}",
                frag.sql
            );
            // `eq` on a whole day is a half-open range, so each arm binds two.
            assert_eq!(
                bind_numbers(&frag.sql, 2),
                (3..=8).collect::<Vec<_>>(),
                "{label}"
            );
            assert_eq!(frag.params.len(), 6, "{label}");
        }
    }

    /// Each repeated name gets its own membership test, and names keep away from
    /// each other: only a repeat of one name is a set operation.
    #[test]
    fn repeated_names_fold_independently_of_each_other() {
        let query = SearchQuery::new("Encounter")
            .with_parameter(date_param("date", SearchPrefix::Ge, "2020-01-01"))
            .with_parameter(token_param("status", None, "finished"))
            .with_parameter(date_param("date", SearchPrefix::Le, "2021-01-01"))
            .with_parameter(token_param("status", None, "in-progress"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert_eq!(frag.sql.matches(" INTERSECT ").count(), 2, "{}", frag.sql);
        assert_eq!(frag.sql.matches("id IN (").count(), 2, "{}", frag.sql);
        assert_eq!(frag.sql.matches(") AND (").count(), 1, "{}", frag.sql);
        assert_eq!(
            bind_numbers(&frag.sql, 2),
            (3..=10).collect::<Vec<_>>(),
            "{}",
            frag.sql
        );

        // The dates' arms come first, from the first occurrence's position, then
        // the status arms — and the binds follow the text, not the request order.
        let (dates, status) = frag.sql.split_once(") AND (").expect("two names");
        assert_eq!(dates.matches("param_name = 'date'").count(), 2, "{dates}");
        assert_eq!(
            status.matches("param_name = 'status'").count(),
            2,
            "{status}"
        );
        assert!(dates.contains("$3") && dates.contains("$8"), "{dates}");
        assert!(status.contains("$9") && status.contains("$10"), "{status}");
        // Each date range arm binds three (#1391); see
        // `an_intervening_parameter_follows_its_group_with_binds_in_emit_order`.
        let (date_binds, status_binds) = frag.params.split_at(6);
        assert_eq!(
            timestamps(date_binds),
            [
                "2020-01-02T00:00:00+00:00",
                "2020-01-01T00:00:00+00:00",
                "2020-01-02T00:00:00+00:00",
                "2021-01-01T00:00:00+00:00",
                "2021-01-01T00:00:00+00:00",
                "2021-01-02T00:00:00+00:00",
            ]
        );
        match status_binds {
            [SqlParam::Text(finished), SqlParam::Text(in_progress)] => {
                assert_eq!(finished, "finished");
                assert_eq!(in_progress, "in-progress");
            }
            other => panic!("expected both statuses after the dates, got {other:?}"),
        }
    }

    /// One occurrence is untouched: the fold is a repeat's shape, and a lone test
    /// is exactly what the fast path extracts.
    #[test]
    fn one_occurrence_keeps_the_extractable_form() {
        let query = SearchQuery::new("Encounter").with_parameter(date_param(
            "date",
            SearchPrefix::Ge,
            "2020-01-01",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(!frag.sql.contains("INTERSECT"), "{}", frag.sql);
        assert_eq!(
            frag.sql,
            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 AND \
             resource_type = $2 AND param_name = 'date' AND (value_date_end > $3 OR \
             (value_date >= $4 AND value_date < $5 AND value_date_end <= $5)))"
        );
        assert_eq!(
            PostgresQueryBuilder::single_index_predicate(&frag.sql),
            Some(
                "param_name = 'date' AND (value_date_end > $3 OR \
                 (value_date >= $4 AND value_date < $5 AND value_date_end <= $5))"
            )
        );
    }

    /// One occurrence the fold cannot express disqualifies the whole *name*, in
    /// either order: the conjunction it came from is kept rather than a constraint
    /// dropped, a presence test read as a match, or a negation read as a positive
    /// membership test.
    #[test]
    fn one_ineligible_occurrence_keeps_the_whole_name_a_conjunction() {
        let mut missing_true = date_param("date", SearchPrefix::Eq, "true");
        missing_true.modifier = Some(SearchModifier::Missing);
        let mut missing_false = date_param("date", SearchPrefix::Eq, "false");
        missing_false.modifier = Some(SearchModifier::Missing);
        let eligible_date = || date_param("date", SearchPrefix::Ge, "2020-01-01");
        let eligible_status = || token_param("status", None, "finished");

        for (label, eligible, ineligible, marker) in [
            (
                "a value that is not a date",
                eligible_date(),
                date_param("date", SearchPrefix::Eq, "not-a-date"),
                "param_name = 'date' AND (value_date_end > ",
            ),
            (
                ":missing=true",
                eligible_date(),
                missing_true,
                "param_name = 'date' AND (value_date_end > ",
            ),
            (
                ":missing=false",
                eligible_date(),
                missing_false,
                "param_name = 'date' AND (value_date_end > ",
            ),
            // `:not` is `NOT (id IN (…))`. It reaches the builder on a token; the
            // REST layer refuses it on a date before a query is built.
            (
                ":not",
                eligible_status(),
                token_param("status", Some(SearchModifier::Not), "in-progress"),
                "param_name = 'status' AND (value_token_code = ",
            ),
        ] {
            for ineligible_first in [false, true] {
                let pair = if ineligible_first {
                    vec![ineligible.clone(), eligible.clone()]
                } else {
                    vec![eligible.clone(), ineligible.clone()]
                };
                let mut query = SearchQuery::new("Encounter");
                for param in pair {
                    query = query.with_parameter(param);
                }
                let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
                let at = if ineligible_first { "first" } else { "last" };
                assert!(
                    !frag.sql.contains("INTERSECT"),
                    "{label} ({at}): {}",
                    frag.sql
                );
                assert!(frag.sql.contains(") AND ("), "{label} ({at}): {}", frag.sql);
                assert!(
                    frag.sql.contains(marker),
                    "{label} ({at}): the eligible occurrence survives: {}",
                    frag.sql
                );
            }
        }
    }

    /// Two occurrences of two different names are not a repeat: they stay two
    /// tests ANDed, and neither is intersected with the other.
    #[test]
    fn different_names_are_never_intersected() {
        let query = SearchQuery::new("Encounter")
            .with_parameter(date_param("date", SearchPrefix::Ge, "2020-01-01"))
            .with_parameter(date_param("birthdate", SearchPrefix::Le, "2000-01-01"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(!frag.sql.contains("INTERSECT"), "{}", frag.sql);
        assert_eq!(frag.sql.matches("id IN (").count(), 2, "{}", frag.sql);
        assert_eq!(frag.sql.matches(") AND (").count(), 1, "{}", frag.sql);
        // Three binds per range arm (#1391).
        assert_eq!(
            bind_numbers(&frag.sql, 2),
            (3..=8).collect::<Vec<_>>(),
            "{}",
            frag.sql
        );
    }

    /// The fold is decided by the arm's shape, not by the parameter's type or the
    /// database's layout: a date occurrence is the same `search_index` test under
    /// either layout, a denormalized composite is one row predicate and folds, and
    /// the legacy composite is an aggregate — no row predicate for `INTERSECT` to
    /// take as an arm — so it keeps its conjunction.
    #[test]
    fn composite_occurrences_fold_only_in_the_form_that_is_a_row_predicate() {
        let composite = || {
            SearchQuery::new("Observation")
                .with_parameter(composite_param("combo-code-value-quantity", "8867-4$gt100"))
                .with_parameter(composite_param("combo-code-value-quantity", "8867-4$lt200"))
        };
        let flat = PostgresQueryBuilder::build_search_query_for(
            &composite(),
            2,
            IndexLayout::Denormalized,
        )
        .expect("condition");
        assert_eq!(flat.sql.matches(" INTERSECT ").count(), 1, "{}", flat.sql);
        assert_eq!(
            flat.sql.matches("FROM search_index").count(),
            2,
            "{}",
            flat.sql
        );
        assert!(!flat.sql.contains("GROUP BY"), "{}", flat.sql);

        let legacy =
            PostgresQueryBuilder::build_search_query_for(&composite(), 2, IndexLayout::Legacy)
                .expect("condition");
        assert!(!legacy.sql.contains("INTERSECT"), "{}", legacy.sql);
        assert_eq!(
            legacy
                .sql
                .matches("GROUP BY resource_id, composite_group HAVING")
                .count(),
            2,
            "{}",
            legacy.sql
        );

        let dated = || {
            SearchQuery::new("Encounter")
                .with_parameter(date_param("date", SearchPrefix::Ge, "2020-01-01"))
                .with_parameter(date_param("date", SearchPrefix::Le, "2021-01-01"))
        };
        for layout in [IndexLayout::Denormalized, IndexLayout::Legacy] {
            let frag = PostgresQueryBuilder::build_search_query_for(&dated(), 2, layout)
                .expect("condition");
            assert_eq!(
                frag.sql.matches(" INTERSECT ").count(),
                1,
                "{layout:?}: {}",
                frag.sql
            );
        }
    }

    /// The gate, on hand-built strings: the shapes that share the wrapper but are
    /// not one test scoped to the name. The query-level tests above cover the
    /// eligible forms and representative ineligible ones: an unparseable date,
    /// both `:missing` values, `:not`, and the legacy composite aggregate.
    /// Wrapper-like exclusions that have no query-level case, such as the
    /// compartment clause's `param_name IN (...)` list and a set-expression
    /// arm, are pinned by shape here.
    #[test]
    fn only_one_test_scoped_to_the_name_is_an_arm() {
        let prefix = PostgresQueryBuilder::INDEX_MEMBERSHIP_PREFIX;
        let arm =
            |name: &str, predicate: &str| format!("{prefix}param_name = '{name}' AND {predicate})");
        let plain = arm("date", "value_date >= $3");

        assert_eq!(
            PostgresQueryBuilder::simple_membership_arm(&plain, "date"),
            Some(
                "SELECT resource_id FROM search_index WHERE tenant_id = $1 AND resource_type = \
                 $2 AND param_name = 'date' AND value_date >= $3"
            )
        );

        for (label, sql, name) in [
            // Another parameter's test.
            ("another name", arm("birthdate", "value_date >= $3"), "date"),
            // The compartment clause, which matches `param_name IN (…)` over the
            // compartment's own membership params.
            (
                "a compartment list",
                format!("{prefix}param_name IN ('subject', 'encounter') AND value_reference = $3)"),
                "date",
            ),
            // A conjunction of two tests: everything after the first predicate is
            // another sub-select, which is not one arm.
            ("a conjunction", format!("({plain}) AND ({plain})"), "date"),
            // A negation, from `:not` or `:missing=true`.
            ("a negation", format!("NOT ({plain})"), "date"),
            // Already folded, so not one test either.
            (
                "an intersection",
                arm("date", "value_date >= $3 INTERSECT SELECT 1"),
                "date",
            ),
            // An arm that is itself a set expression: concatenating it would bind
            // as `s1 UNION (s2 INTERSECT s3)`.
            (
                "a set expression",
                arm(
                    "date",
                    "value_date IN (SELECT d FROM a UNION SELECT d FROM b)",
                ),
                "date",
            ),
        ] {
            assert!(
                PostgresQueryBuilder::simple_membership_arm(&sql, name).is_none(),
                "{label} is not an arm: {sql}"
            );
        }
    }

    #[test]
    fn a_list_of_only_unparseable_dates_still_matches_nothing() {
        // `FALSE OR FALSE`: the OR fold must not turn defense in depth into a
        // dropped constraint.
        for name in ["date", "_lastUpdated"] {
            let values = vec![
                SearchValue::new(SearchPrefix::Lt, "not-a-date"),
                SearchValue::new(SearchPrefix::Ne, "also-not"),
            ];
            let param = if name == "date" {
                multi_value_param(name, SearchParamType::Date, values)
            } else {
                special_param(name, values)
            };
            let query = SearchQuery::new("Procedure").with_parameter(param);
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
            assert_eq!(frag.sql, "(FALSE) OR (FALSE)", "{name}");
            assert!(frag.params.is_empty(), "{name}");
        }
    }

    #[test]
    fn missing_modifier_is_not_extractable() {
        // `:missing=true` is an absence test, so the rows it selects are exactly
        // the ones `search_index` does not hold.
        let mut param = date_param("date", SearchPrefix::Eq, "true");
        param.modifier = Some(SearchModifier::Missing);
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(PostgresQueryBuilder::single_index_predicate(&frag.sql).is_none());
    }

    /// A slot-2 token component must read the `_2` column **and** fall back to
    /// the base column, so the legacy read form keeps matching a resource that
    /// promotion has already rewritten. Without this, 24 of the 46 R4
    /// composites silently vanish from search for the length of the run.
    #[test]
    fn transitional_legacy_form_reads_both_layouts_for_token_token() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code-value-concept".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::eq("8480-6$271649006")],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".into(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "value-concept".into(),
                },
            ],
        });

        let frag = PostgresQueryBuilder::build_search_query_for(&query, 2, IndexLayout::Legacy)
            .expect("fragment");

        assert!(
            frag.sql.contains("value_token_code_2 = $4"),
            "component 2 must read the denormalized _2 column: {}",
            frag.sql
        );
        assert!(
            frag.sql
                .contains("value_token_code_2 IS NULL AND value_token_system_2 IS NULL"),
            "the fallback must be guarded on the row having no slot-2 payload: {}",
            frag.sql
        );
        // Component 1 must NEVER read `_2`. If it did, a denormalized row whose
        // components are SWAPPED would satisfy the query — a different clinical
        // fact than the one asked for, returned as a match.
        let having = frag.sql.split("HAVING ").nth(1).expect("a HAVING clause");
        let comp_one_arm = having
            .split(" AND MAX(")
            .next()
            .expect("component 1's MAX arm");
        assert!(
            !comp_one_arm.contains("_2"),
            "component 1's HAVING arm must read only the base columns, got `{}` in: {}",
            comp_one_arm,
            frag.sql
        );
        // Still the grouped form — this is the legacy layout.
        assert!(
            frag.sql
                .contains("GROUP BY resource_id, composite_group HAVING")
        );
    }

    /// A component type with no `_2` column is identical under both layouts, so
    /// the transitional form must not widen it — a quantity component writes
    /// `value_quantity_value` whatever slot it occupies.
    #[test]
    fn transitional_legacy_form_is_unchanged_where_no_slot_2_column_exists() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::eq("8480-6$ge100")],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".into(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value-quantity".into(),
                },
            ],
        });

        let frag = PostgresQueryBuilder::build_search_query_for(&query, 2, IndexLayout::Legacy)
            .expect("fragment");

        assert!(
            !frag.sql.contains("_2"),
            "token+quantity needs no transitional widening: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2, "and binds no extra parameters");
    }

    #[test]
    fn legacy_layout_keeps_the_grouped_composite_form() {
        // A database that predates v18 stores one row per composite component.
        // The denormalized conjunction matches none of them, and would return an
        // empty bundle rather than an error — so the layout must select the form.
        let param = SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://loinc.org|8480-6$lt60",
            )],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value".to_string(),
                },
            ],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);

        let legacy = PostgresQueryBuilder::build_search_query_for(&query, 2, IndexLayout::Legacy)
            .expect("legacy composite condition");
        assert!(
            legacy
                .sql
                .contains("GROUP BY resource_id, composite_group HAVING"),
            "grouping is what confines components to one composite instance: {}",
            legacy.sql
        );
        assert!(
            legacy.sql.contains("MAX(CASE WHEN"),
            "per-component HAVING must survive: {}",
            legacy.sql
        );
        // Components read the base columns; the `_2` columns exist only to pack
        // two same-type components into one denormalized row.
        assert!(!legacy.sql.contains("value_token_code_2"), "{}", legacy.sql);

        let flat =
            PostgresQueryBuilder::build_search_query_for(&query, 2, IndexLayout::Denormalized)
                .expect("denormalized composite condition");
        assert!(!flat.sql.contains("GROUP BY"), "{}", flat.sql);
        assert!(
            flat.sql.contains("composite_group IS NOT NULL"),
            "{}",
            flat.sql
        );

        // Both forms must bind the same values, or the two layouts would disagree
        // about what was searched for.
        assert_eq!(legacy.params.len(), flat.params.len());
    }

    #[test]
    fn the_legacy_composite_form_is_never_taken_as_a_fast_path() {
        // It is an aggregate, not a row predicate; splicing it into a
        // `SELECT DISTINCT ... ORDER BY` would be nonsense.
        let param = SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://loinc.org|8480-6$lt60",
            )],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value".to_string(),
                },
            ],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let legacy = PostgresQueryBuilder::build_search_query_for(&query, 2, IndexLayout::Legacy)
            .expect("condition");
        assert!(PostgresQueryBuilder::single_index_predicate(&legacy.sql).is_none());
    }

    fn number_param(prefix: SearchPrefix, value: &str) -> SearchParameter {
        SearchParameter {
            name: "value-number".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::new(prefix, value)],
            chain: vec![],
            components: vec![],
        }
    }

    fn quantity_param(prefix: SearchPrefix, value: &str) -> SearchParameter {
        SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::new(prefix, value)],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn number_comparators_use_exact_value() {
        // Per the FHIR number search spec, gt/lt/ge/le/sa/eb ignore the search
        // value's implicit precision and compare against the exact value.
        for (prefix, op) in [
            (SearchPrefix::Gt, ">"),
            (SearchPrefix::Ge, ">="),
            (SearchPrefix::Lt, "<"),
            (SearchPrefix::Le, "<="),
            (SearchPrefix::Sa, ">"),
            (SearchPrefix::Eb, "<"),
        ] {
            let query = SearchQuery::new("Observation").with_parameter(number_param(prefix, "60"));
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

            assert!(
                frag.sql.contains(&format!("value_number {op} $3")),
                "{prefix:?} must emit `{op}`: {}",
                frag.sql
            );
            assert_eq!(frag.params.len(), 1);
            match &frag.params[0] {
                SqlParam::Float(n) => assert_eq!(*n, 60.0, "{prefix:?} must bind the exact value"),
                other => panic!("expected a float param, got {other:?}"),
            }
        }
    }

    #[test]
    fn quantity_comparator_ignores_trailing_zero_precision() {
        // "gt60" and "gt60.0" must produce identical SQL and params: the exact
        // comparators no longer derive a range from the search value's
        // implicit precision.
        let query60 =
            SearchQuery::new("Observation").with_parameter(quantity_param(SearchPrefix::Gt, "60"));
        let frag60 = PostgresQueryBuilder::build_search_query(&query60, 2).expect("condition");

        let query60_0 = SearchQuery::new("Observation")
            .with_parameter(quantity_param(SearchPrefix::Gt, "60.0"));
        let frag60_0 = PostgresQueryBuilder::build_search_query(&query60_0, 2).expect("condition");

        assert_eq!(frag60.sql, frag60_0.sql);
        assert_eq!(frag60.params.len(), frag60_0.params.len());
        for (a, b) in frag60.params.iter().zip(frag60_0.params.iter()) {
            match (a, b) {
                (SqlParam::Float(x), SqlParam::Float(y)) => assert_eq!(x, y),
                _ => panic!("expected both params to be floats: {a:?} vs {b:?}"),
            }
        }
    }

    #[test]
    fn quantity_canonical_comparator_uses_exact_canonical_value() {
        let query = SearchQuery::new("Observation").with_parameter(quantity_param(
            SearchPrefix::Gt,
            "60|http://unitsofmeasure.org|kg",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(
            frag.sql.contains("value_quantity_canonical_value > $"),
            "{}",
            frag.sql
        );
        let expected = helios_fhirpath::ucum::canonicalize_quantity(60.0, "kg")
            .expect("kg must canonicalize")
            .0;
        let canonical_param = frag
            .params
            .iter()
            .find_map(|p| match p {
                SqlParam::Float(n) if (*n - expected).abs() < f64::EPSILON => Some(*n),
                _ => None,
            })
            .expect("the canonical bound must be bound as the exact canonical value");
        assert_eq!(canonical_param, expected);
    }

    #[test]
    fn quantity_ne_includes_canonical_branch() {
        // `ne` must also negate the canonical range, so a resource stored in
        // an equivalent unit (55000 g vs 60 kg) is correctly excluded/included
        // by unit conversion rather than only by the raw stored unit.
        let query = SearchQuery::new("Observation").with_parameter(quantity_param(
            SearchPrefix::Ne,
            "60|http://unitsofmeasure.org|kg",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(
            frag.sql.contains(
                "(value_quantity_canonical_value < $7 OR value_quantity_canonical_value >= $8)"
            ),
            "expected a negated canonical range: {}",
            frag.sql
        );
        assert!(
            !frag
                .sql
                .contains("value_quantity_canonical_value >= $7 AND"),
            "the canonical branch must not use the eq inclusive-range shape: {}",
            frag.sql
        );

        let (expected_lo, _) =
            helios_fhirpath::ucum::canonicalize_quantity(59.5, "kg").expect("kg must canonicalize");
        let (expected_hi, _) =
            helios_fhirpath::ucum::canonicalize_quantity(60.5, "kg").expect("kg must canonicalize");
        let has_lo = frag
            .params
            .iter()
            .any(|p| matches!(p, SqlParam::Float(f) if (*f - expected_lo).abs() < 1e-9));
        let has_hi = frag
            .params
            .iter()
            .any(|p| matches!(p, SqlParam::Float(f) if (*f - expected_hi).abs() < 1e-9));
        assert!(
            has_lo && has_hi,
            "expected canonical bounds canon(59.5, \"kg\") = {expected_lo} and \
             canon(60.5, \"kg\") = {expected_hi}, got {:?}",
            frag.params
        );
    }

    #[test]
    fn quantity_eq_keeps_text_precision_range() {
        // eq is unaffected by this change: it still ranges over the
        // implicit-precision window derived from the value as written.
        let query = SearchQuery::new("Observation")
            .with_parameter(quantity_param(SearchPrefix::Eq, "60.0"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(
            frag.sql
                .contains("value_quantity_value >= $3 AND value_quantity_value < $4"),
            "{}",
            frag.sql
        );
        match (&frag.params[0], &frag.params[1]) {
            (SqlParam::Float(lo), SqlParam::Float(hi)) => {
                assert_eq!(*lo, 59.95);
                assert_eq!(*hi, 60.05);
            }
            other => panic!("expected two float params, got {other:?}"),
        }
    }

    #[test]
    fn composite_token_quantity_sql() {
        let param = SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://loinc.org|8480-6$lt60",
            )],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value".to_string(),
                },
            ],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        // Non-cursor search binds $1=tenant, $2=type, so params start at $3.
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("composite should produce a condition");

        assert!(frag.sql.contains("param_name = 'code-value-quantity'"));
        assert!(frag.sql.contains("value_token_system IN ($3, "));
        assert!(frag.sql.contains("value_token_code = $4"));
        assert!(frag.sql.contains("value_quantity_value < $5"));
        // token (system+code) = 2 params, quantity (no unit) = 1 param.
        assert_eq!(frag.params.len(), 3);
    }

    /// The two composite spellings, pinned. These are the exact texts the v28
    /// composite indexes are shaped for, and the exact texts whose partial
    /// predicates `predtest.c` has to prove:
    ///
    /// - `composite_group IS NOT NULL` appears literally, so
    ///   `WHERE composite_group IS NOT NULL` is provable;
    /// - the quantity component is a strict operator over
    ///   `value_quantity_value`, so `WHERE value_quantity_value IS NOT NULL` is
    ///   provable;
    /// - the token component is an equality on `value_token_code`, which is why
    ///   that column leads the index key ahead of the sort key (v21's rule).
    ///
    /// If any of these change, the v28 indexes silently stop being reachable
    /// and composite search falls back to a full sort. Reachability is not
    /// something a `#[test]` can observe, so it is pinned here instead.
    fn composite_param(name: &str, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, value)],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value".to_string(),
                },
            ],
        }
    }

    /// A number component of a composite gets the ranges a standalone number
    /// parameter gets (#1390): `ap` the shared window, `eq` the
    /// implicit-precision range. The shape is `MolecularSequence`'s
    /// `chromosome-variant-coordinate` (token, number, number), so the second
    /// number also covers the component in the second slot.
    #[test]
    fn composite_number_components_use_the_shared_ranges() {
        let param = SearchParameter {
            name: "chromosome-variant-coordinate".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "1$ap100$1e2")],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "chromosome".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Number,
                    param_name: "variant-start".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Number,
                    param_name: "variant-end".to_string(),
                },
            ],
        };
        let query = SearchQuery::new("MolecularSequence").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
        assert!(
            frag.sql.contains("(value_number BETWEEN $4 AND $5)"),
            "{}",
            frag.sql
        );
        assert!(
            frag.sql
                .contains("(value_number_2 >= $6 AND value_number_2 < $7)"),
            "{}",
            frag.sql
        );
        let floats: Vec<f64> = frag
            .params
            .iter()
            .filter_map(|p| match p {
                SqlParam::Float(f) => Some(*f),
                _ => None,
            })
            .collect();
        // `ap100` is [90, 110]; `1e2` has one significant figure, so its
        // `eq` range is [50, 150).
        assert_eq!(floats, [90.0, 110.0, 50.0, 150.0]);
    }

    #[test]
    fn composite_bare_code_emits_what_the_v27_index_is_keyed_for() {
        // `combo-code-value-quantity=8867-4$gt100` — 21 of the 27 composite
        // values in `k6/searchConfig.js` are this spelling.
        let query = SearchQuery::new("Observation")
            .with_parameter(composite_param("combo-code-value-quantity", "8867-4$gt100"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert_eq!(
            frag.sql,
            "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 \
             AND resource_type = $2 AND param_name = 'combo-code-value-quantity' \
             AND composite_group IS NOT NULL \
             AND (value_token_code = $3) AND (value_quantity_value > $4))",
            "{}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn composite_system_qualified_code_keeps_the_system_in_the_predicate() {
        // The other 6 values, and the second `pg_stat_statements` entry (7,767
        // calls). `value_token_system` is payload on the v28 index so this
        // stays index-only rather than fetching the heap per candidate.
        let query = SearchQuery::new("Observation").with_parameter(composite_param(
            "combo-code-value-quantity",
            "http://loinc.org|8480-6$gt140",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        // The named system, or the marker of a `code` component (#1379).
        assert_eq!(
            frag.sql,
            format!(
                "id IN (SELECT resource_id FROM search_index WHERE tenant_id = $1 \
                 AND resource_type = $2 AND param_name = 'combo-code-value-quantity' \
                 AND composite_group IS NOT NULL \
                 AND (value_token_system IN ($3, '{IMPLICIT_TOKEN_SYSTEM}') AND value_token_code = $4) \
                 AND (value_quantity_value > $5))"
            ),
            "{}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 3);
    }

    #[test]
    fn composite_prefix_is_only_parsed_for_numeric_component_types() {
        // A token/string/reference/uri code that begins with a prefix's letters
        // must be taken verbatim (#1236): `left` is not `le` + `ft`.
        for (code, ty) in [
            ("left", SearchParamType::Token),
            ("negative", SearchParamType::String),
            ("ge123", SearchParamType::Token),
            ("http://x/y", SearchParamType::Uri),
        ] {
            let cv = PostgresQueryBuilder::parse_component_value(code, ty);
            assert!(
                matches!(cv.prefix, SearchPrefix::Eq),
                "{code}: prefix stripped"
            );
            assert_eq!(cv.value, code, "{code}: value corrupted");
        }

        // …while number, date and quantity still parse theirs.
        let q = PostgresQueryBuilder::parse_component_value("lt60", SearchParamType::Quantity);
        assert!(matches!(q.prefix, SearchPrefix::Lt));
        assert_eq!(q.value, "60");
        let d = PostgresQueryBuilder::parse_component_value("ge2024", SearchParamType::Date);
        assert!(matches!(d.prefix, SearchPrefix::Ge));
        assert_eq!(d.value, "2024");
    }

    #[test]
    fn every_composite_quantity_prefix_is_a_strict_operator() {
        // `WHERE value_quantity_value IS NOT NULL` on the composite index is
        // only provable from a STRICT operator over that column. Every prefix
        // the benchmark can send — and every prefix `parse_component_value`
        // recognises — must therefore emit one: a comparison, a conjunction or
        // disjunction of comparisons, or a `BETWEEN`. `IS DISTINCT FROM` or a
        // `COALESCE` here would strand the index without any test failing.
        //
        // The value is compared as a standalone quantity compares it (#1390):
        // `eq`/`ne` over the implicit-precision range, `ap` over the shared
        // window, `[90, 110]` for `ap100`.
        let v = "value_quantity_value";
        for (spelling, predicate, binds) in [
            ("gt100", format!("({v} > $4)"), vec![100.0]),
            ("lt100", format!("({v} < $4)"), vec![100.0]),
            ("ge100", format!("({v} >= $4)"), vec![100.0]),
            ("le100", format!("({v} <= $4)"), vec![100.0]),
            ("sa100", format!("({v} > $4)"), vec![100.0]),
            ("eb100", format!("({v} < $4)"), vec![100.0]),
            (
                "ne100",
                format!("(({v} < $4 OR {v} >= $5))"),
                vec![99.5, 100.5],
            ),
            (
                "ap100",
                format!("({v} BETWEEN $4 AND $5)"),
                vec![90.0, 110.0],
            ),
            (
                "100",
                format!("({v} >= $4 AND {v} < $5)"),
                vec![99.5, 100.5],
            ),
        ] {
            let query = SearchQuery::new("Observation").with_parameter(composite_param(
                "combo-code-value-quantity",
                &format!("8867-4${spelling}"),
            ));
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
            assert!(
                frag.sql.contains(&predicate),
                "{spelling} must emit a strict predicate: {}",
                frag.sql
            );
            let floats: Vec<f64> = frag
                .params
                .iter()
                .filter_map(|p| match p {
                    SqlParam::Float(f) => Some(*f),
                    _ => None,
                })
                .collect();
            assert_eq!(floats, binds, "{spelling}");
        }
    }

    #[test]
    fn the_composite_fast_path_predicate_is_extractable() {
        // The whole point of keying the composite index on the fast path's sort
        // key is that composite search TAKES the fast path. It does, because a
        // lone composite parameter is exactly one membership test — this is the
        // gate, and it is the reason `ORDER BY last_updated DESC, resource_id
        // ASC` is what the index has to serve.
        let query = SearchQuery::new("Observation")
            .with_parameter(composite_param("combo-code-value-quantity", "8867-4$gt100"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone composite test is extractable");

        assert_eq!(
            pred,
            "param_name = 'combo-code-value-quantity' AND composite_group IS NOT NULL \
             AND (value_token_code = $3) AND (value_quantity_value > $4)"
        );
        // It has to stand alone against `search_index`.
        assert!(!pred.contains("resources"));
        assert!(!pred.contains("SELECT"));
    }

    fn special_param(name: &str, values: Vec<SearchValue>) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Special,
            modifier: None,
            values,
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn text_search_filters_instead_of_returning_everything() {
        // Regression: `SearchParamType::Special` fell through to `None`, so a
        // `_text`-only query built no filter at all and `search` answered a
        // full-text query with every resource of the type.
        let query = SearchQuery::new("Patient").with_parameter(special_param(
            "_text",
            vec![SearchValue::eq("Zebracrossingdiagnosis")],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_text must produce a condition, not an empty filter");

        assert!(
            frag.sql.contains("FROM resource_fts"),
            "must resolve through the full-text table: {}",
            frag.sql
        );
        assert!(
            frag.sql
                .contains("narrative_tsvector @@ plainto_tsquery('english', $3)"),
            "_text matches the narrative column: {}",
            frag.sql
        );
        // Tenant and type scoping keep the sub-select from selecting another
        // tenant's — or another resource type's — row of the same id.
        assert!(frag.sql.contains("tenant_id = $1"), "{}", frag.sql);
        assert!(frag.sql.contains("resource_type = $2"), "{}", frag.sql);
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn content_search_uses_the_content_column() {
        let query = SearchQuery::new("Patient").with_parameter(special_param(
            "_content",
            vec![SearchValue::eq("Quokkaflavoured")],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_content must produce a condition");

        assert!(
            frag.sql
                .contains("content_tsvector @@ plainto_tsquery('english', $3)"),
            "{}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn last_updated_binds_timestamps_with_precision_ranges() {
        // #871: a text param bound against the TIMESTAMPTZ column fails
        // serialization, and day-precision eq must mean [day, day+1).
        let query = SearchQuery::new("Patient").with_parameter(special_param(
            "_lastUpdated",
            vec![SearchValue::eq("2026-09-01")],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_lastUpdated must produce a condition");

        assert!(
            frag.sql
                .contains("last_updated >= $3 AND last_updated < $4"),
            "{}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2);
        assert!(
            frag.params
                .iter()
                .all(|p| matches!(p, SqlParam::Timestamp(_))),
            "must bind timestamps, not text: {:?}",
            frag.params
        );
    }

    #[test]
    fn last_updated_gt_excludes_the_named_day() {
        let query = SearchQuery::new("Patient").with_parameter(special_param(
            "_lastUpdated",
            vec![SearchValue {
                prefix: SearchPrefix::Gt,
                value: "2026-09-01".to_string(),
            }],
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_lastUpdated gt must produce a condition");

        assert!(frag.sql.contains("last_updated >= $3"), "{}", frag.sql);
        assert_eq!(frag.params.len(), 1);
        match &frag.params[0] {
            SqlParam::Timestamp(ts) => {
                assert_eq!(ts.to_rfc3339(), "2026-09-02T00:00:00+00:00");
            }
            other => panic!("must bind the period end as a timestamp: {other:?}"),
        }
    }

    /// A search value carrying a non-UTC offset must bind the instant it
    /// names. The query-side zone test recognized only `+`, `Z` and `-00:00`,
    /// so any other negative offset was mangled into invalid RFC3339 and the
    /// parse fell back to `Utc::now()`: `date=2013-04-05T09:20:00-04:00`
    /// compared against the current time and matched nothing. Found by the
    /// Inferno US Quality Core suite, where it hid as six *skipped* (not
    /// failed) date-search tests on the postgres leg only.
    #[test]
    fn date_with_negative_offset_binds_the_named_instant() {
        let query = SearchQuery::new("Procedure").with_parameter(date_param(
            "date",
            SearchPrefix::Eq,
            "2013-04-05T09:20:00-04:00",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        // The whole second it names, not a scalar `=` on its first instant:
        // a stored `…T09:20:00.123-04:00` is inside it (#1297).
        assert!(
            frag.sql.contains("value_date >= $3 AND value_date < $4"),
            "{}",
            frag.sql
        );
        assert_eq!(
            timestamps(&frag.params),
            ["2013-04-05T13:20:00+00:00", "2013-04-05T13:20:01+00:00"]
        );
    }

    /// The bound timestamps of a fragment, in order; panics on any other bind.
    fn timestamps(params: &[SqlParam]) -> Vec<String> {
        params
            .iter()
            .map(|param| match param {
                SqlParam::Timestamp(ts) => ts.to_rfc3339(),
                other => panic!("must bind a timestamp: {other:?}"),
            })
            .collect()
    }

    /// #1297: every prefix compares against the bounds of the range the value
    /// names, at every precision — on `value_date` and on `last_updated`.
    #[test]
    fn date_predicate_maps_every_prefix_onto_the_range_bounds() {
        let (start, end) = ("2013-04-06T03:30:00+00:00", "2013-04-06T03:30:01+00:00");
        for col in ["value_date", "last_updated"] {
            for (prefix, sql, binds) in [
                (SearchPrefix::Eq, "{c} >= $3 AND {c} < $4", vec![start, end]),
                (
                    SearchPrefix::Ne,
                    "({c} < $3 OR {c} >= $4)",
                    vec![start, end],
                ),
                (SearchPrefix::Gt, "{c} >= $3", vec![end]),
                (SearchPrefix::Sa, "{c} >= $3", vec![end]),
                (SearchPrefix::Lt, "{c} < $3", vec![start]),
                (SearchPrefix::Eb, "{c} < $3", vec![start]),
                (SearchPrefix::Ge, "{c} >= $3", vec![start]),
                (SearchPrefix::Le, "{c} < $3", vec![end]),
                // `ap` keeps its scalar equality with the start of the range.
                (SearchPrefix::Ap, "{c} = $3", vec![start]),
            ] {
                let mut next = 2;
                let (got, params) =
                    date_predicate(col, prefix, "2013-04-05T23:30:00-04:00", &mut next);
                assert_eq!(got, sql.replace("{c}", col), "{prefix}");
                assert_eq!(timestamps(&params), binds, "{prefix}");
                assert_eq!(next, 2 + params.len(), "{prefix}: next counts the binds");
            }
        }
    }

    #[test]
    fn date_predicate_ranges_follow_the_precision_supplied() {
        for (value, start, end) in [
            // Minute precision is valid in FHIR search.
            (
                "2013-04-05T09:20",
                "2013-04-05T09:20:00+00:00",
                "2013-04-05T09:21:00+00:00",
            ),
            // A millisecond value finds the microsecond timestamp stored for
            // it: `_lastUpdated=eq<meta.lastUpdated>` used to be a scalar `=`.
            (
                "2026-09-06T08:44:27.804Z",
                "2026-09-06T08:44:27.804+00:00",
                "2026-09-06T08:44:27.805+00:00",
            ),
            (
                "2021-11-10T16:48:57.246958-08:00",
                "2021-11-11T00:48:57.246958+00:00",
                "2021-11-11T00:48:57.246959+00:00",
            ),
            // #1296: a `+` that form decoding turned into a space.
            (
                "2013-04-05T18:50:00 05:30",
                "2013-04-05T13:20:00+00:00",
                "2013-04-05T13:20:01+00:00",
            ),
            (
                "2013-12",
                "2013-12-01T00:00:00+00:00",
                "2014-01-01T00:00:00+00:00",
            ),
        ] {
            let mut next = 0;
            let (_, params) = date_predicate("value_date", SearchPrefix::Eq, value, &mut next);
            assert_eq!(timestamps(&params), [start, end], "{value}");
        }
    }

    /// A value that is not a date binds nothing and leaves the numbering of
    /// the binds around it alone, under every prefix.
    #[test]
    fn date_predicate_fails_closed_without_binds() {
        for value in ["not-a-date", "2024-02-30", "2013-04-05T10", ""] {
            for prefix in [
                SearchPrefix::Eq,
                SearchPrefix::Ne,
                SearchPrefix::Lt,
                SearchPrefix::Ap,
            ] {
                let mut next = 7;
                let (sql, params) = date_predicate("value_date", prefix, value, &mut next);
                assert_eq!(sql, "FALSE", "{prefix}{value}");
                assert!(params.is_empty());
                assert_eq!(next, 7);
            }
        }
    }

    /// Every prefix against the indexed range `[value_date, value_date_end)`
    /// (#1391): the FHIR table the shared layer derives, each `end <= b` with
    /// its implied `start < b` on the same bind, a disjunction parenthesized.
    #[test]
    fn date_range_predicate_maps_every_prefix_onto_the_indexed_range() {
        let (s, e) = ("2013-04-06T03:30:00+00:00", "2013-04-06T03:30:01+00:00");
        let contained = "value_date >= $A AND value_date < $B AND value_date_end <= $B";
        for (prefix, sql, binds) in [
            (
                SearchPrefix::Eq,
                "value_date >= $3 AND value_date < $4 AND value_date_end <= $4".to_string(),
                vec![s, e],
            ),
            (
                SearchPrefix::Ne,
                "(value_date < $3 OR value_date_end > $4)".to_string(),
                vec![s, e],
            ),
            (SearchPrefix::Gt, "value_date_end > $3".to_string(), vec![e]),
            (SearchPrefix::Lt, "value_date < $3".to_string(), vec![s]),
            (
                SearchPrefix::Ge,
                format!(
                    "(value_date_end > $3 OR ({}))",
                    contained.replace("$A", "$4").replace("$B", "$5")
                ),
                vec![e, s, e],
            ),
            (
                SearchPrefix::Le,
                format!(
                    "(value_date < $3 OR ({}))",
                    contained.replace("$A", "$4").replace("$B", "$5")
                ),
                vec![s, s, e],
            ),
            (SearchPrefix::Sa, "value_date >= $3".to_string(), vec![e]),
            (
                SearchPrefix::Eb,
                "value_date < $3 AND value_date_end <= $3".to_string(),
                vec![s],
            ),
            // Overlap within the precision's margin: ten seconds for a second.
            (
                SearchPrefix::Ap,
                "value_date < $3 AND value_date_end > $4".to_string(),
                vec!["2013-04-06T03:30:11+00:00", "2013-04-06T03:29:50+00:00"],
            ),
        ] {
            let mut next = 2;
            let (got, params) = date_range_predicate(
                "value_date",
                "value_date_end",
                prefix,
                "2013-04-05T23:30:00-04:00",
                &mut next,
            );
            assert_eq!(got, sql, "{prefix}");
            assert_eq!(timestamps(&params), binds, "{prefix}");
            assert_eq!(next, 2 + params.len(), "{prefix}: next counts the binds");
        }
    }

    /// The range form fails closed exactly as the point form does.
    #[test]
    fn date_range_predicate_fails_closed_without_binds() {
        for value in ["not-a-date", "2024-02-30", "2013-04-05T10", ""] {
            for prefix in [
                SearchPrefix::Eq,
                SearchPrefix::Ne,
                SearchPrefix::Gt,
                SearchPrefix::Ap,
            ] {
                let mut next = 7;
                let (sql, params) =
                    date_range_predicate("value_date", "value_date_end", prefix, value, &mut next);
                assert_eq!(sql, "FALSE", "{prefix}{value}");
                assert!(params.is_empty());
                assert_eq!(next, 7);
            }
        }
    }

    /// `_sort=-date` orders by where each resource's latest range ends, so a
    /// Period sorts by its end; ascending and other types are unchanged.
    #[test]
    fn descending_date_sort_reads_the_range_end() {
        let directive = |direction, param_type| crate::types::SortDirective {
            parameter: "date".to_string(),
            direction,
            param_type: Some(param_type),
        };
        let desc = PostgresQueryBuilder::sort_expression(&directive(
            crate::types::SortDirection::Descending,
            SearchParamType::Date,
        ));
        assert!(
            desc.starts_with("(SELECT MAX(value_date_end) FROM"),
            "{desc}"
        );
        let asc = PostgresQueryBuilder::sort_expression(&directive(
            crate::types::SortDirection::Ascending,
            SearchParamType::Date,
        ));
        assert!(asc.starts_with("(SELECT MIN(value_date) FROM"), "{asc}");
        let number = PostgresQueryBuilder::sort_expression(&directive(
            crate::types::SortDirection::Descending,
            SearchParamType::Number,
        ));
        assert!(
            number.starts_with("(SELECT MAX(value_number) FROM"),
            "{number}"
        );
    }

    /// A composite's date component gets the same precision range a standalone
    /// date parameter does. It used to be a scalar comparison with the start
    /// of the value, so `2024-01-15` meant midnight rather than the day.
    #[test]
    fn composite_date_component_is_a_precision_range() {
        let (sql, params) = PostgresQueryBuilder::build_composite_component(
            &SearchValue::new(SearchPrefix::Eq, "2024-01-15"),
            SearchParamType::Date,
            4,
            1,
        )
        .expect("a date component");
        assert_eq!(sql, "(value_date >= $5 AND value_date < $6)");
        assert_eq!(
            timestamps(&params),
            ["2024-01-15T00:00:00+00:00", "2024-01-16T00:00:00+00:00"]
        );
    }

    /// The first instant of the range a search value names.
    fn start_of(value: &str) -> Option<DateTime<Utc>> {
        date_precision_range(value).map(|(start, _)| start)
    }

    #[test]
    fn date_range_start_honors_every_zone_form() {
        let cases = [
            ("2013-04-05T09:20:00-04:00", "2013-04-05T13:20:00+00:00"),
            ("2013-04-05T09:20:00+05:30", "2013-04-05T03:50:00+00:00"),
            ("2013-04-05T09:20:00Z", "2013-04-05T09:20:00+00:00"),
            ("2013-04-05T09:20:00-00:00", "2013-04-05T09:20:00+00:00"),
            // Sub-second precision with a negative offset, as Inferno sends.
            (
                "2021-11-10T16:48:57.246958-08:00",
                "2021-11-11T00:48:57.246958+00:00",
            ),
            // Zone-less and partial values are read as UTC.
            ("2013-04-05T09:20:00", "2013-04-05T09:20:00+00:00"),
            ("2013-04-05", "2013-04-05T00:00:00+00:00"),
            ("2013-04", "2013-04-01T00:00:00+00:00"),
            ("2013", "2013-01-01T00:00:00+00:00"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                start_of(input).map(|ts| ts.to_rfc3339()),
                Some(expected.to_string()),
                "input {input}"
            );
        }
    }

    /// Every prefix a date search value can carry. An unparseable value must
    /// behave identically under all of them.
    const DATE_PREFIXES: [SearchPrefix; 9] = [
        SearchPrefix::Eq,
        SearchPrefix::Ne,
        SearchPrefix::Gt,
        SearchPrefix::Lt,
        SearchPrefix::Ge,
        SearchPrefix::Le,
        SearchPrefix::Sa,
        SearchPrefix::Eb,
        SearchPrefix::Ap,
    ];

    /// Values that reach the builder but are not dates: free text, an
    /// impossible calendar date, an impossible day, and a mangled zone.
    const NOT_DATES: [&str; 4] = [
        "not-a-date",
        "2024-13-45",
        "2024-02-30T10:00:00Z",
        "2013-04-05T09:20:00-99",
    ];

    /// Fails if any bound param is a timestamp. An unparseable value has no
    /// legitimate timestamp to bind, so the type alone catches the old
    /// `Utc::now()` fallback; a near-now instant is called out in the message so
    /// a regression names its own cause.
    fn assert_binds_no_timestamp(frag: &SqlFragment, context: &str) {
        let now = Utc::now();
        for p in &frag.params {
            if let SqlParam::Timestamp(ts) = p {
                let near_now = (now - *ts).num_seconds().abs() < 3600;
                panic!(
                    "{context}: bound timestamp {ts} (near now: {near_now}) in {}",
                    frag.sql
                );
            }
        }
    }

    #[test]
    fn date_range_rejects_what_is_not_a_date() {
        // #1289: these used to come back as `Utc::now()`.
        for input in NOT_DATES {
            assert_eq!(date_precision_range(input), None, "input {input}");
        }
    }

    #[test]
    fn unparseable_date_matches_nothing_under_every_prefix() {
        // #1289: `date=ltnot-a-date` bound the current time and so matched
        // essentially every indexed row; `gt`/`ge`/`eq` matched nothing. All of
        // them — `ne` included — must now match nothing and bind nothing.
        for prefix in DATE_PREFIXES {
            for input in NOT_DATES {
                let context = format!("{prefix:?} {input}");
                let query =
                    SearchQuery::new("Procedure").with_parameter(date_param("date", prefix, input));
                let frag = PostgresQueryBuilder::build_search_query(&query, 2)
                    .expect("an invalid date must still constrain the query");

                assert_eq!(frag.sql, "FALSE", "{context}");
                assert!(frag.params.is_empty(), "{context}: {:?}", frag.params);
                assert_binds_no_timestamp(&frag, &context);
            }
        }
    }

    #[test]
    fn unparseable_last_updated_matches_nothing_under_every_prefix() {
        for prefix in DATE_PREFIXES {
            for input in NOT_DATES {
                let context = format!("{prefix:?} {input}");
                let query = SearchQuery::new("Patient").with_parameter(special_param(
                    "_lastUpdated",
                    vec![SearchValue::new(prefix, input)],
                ));
                let frag = PostgresQueryBuilder::build_search_query(&query, 2)
                    .expect("an invalid _lastUpdated must still constrain the query");

                assert_eq!(frag.sql, "FALSE", "{context}");
                assert!(frag.params.is_empty(), "{context}: {:?}", frag.params);
                assert_binds_no_timestamp(&frag, &context);
            }
        }
    }

    #[test]
    fn unparseable_date_beside_a_valid_one_keeps_placeholders_gap_free() {
        // The values of one parameter are alternatives (#1300), so a bad value
        // is an alternative that matches nothing and the valid one next to it
        // decides the result: `FALSE OR x`. This used to assert `(FALSE) AND (`
        // — "one bad value empties the result" — which was the AND fold's
        // behaviour, not a safety property: the builder is not what stands
        // between a client and a bad date. `validate_date_values` rejects the
        // whole request before any query is built (pinned against a real
        // database by `postgres_integration_invalid_date_in_or_list_is_rejected`),
        // and what reaches here regardless still cannot widen the result past
        // what the valid value alone selects.
        //
        // The bad value must not consume a placeholder number it never binds:
        // the valid value after it still starts at `$3`.
        let mut param = date_param("date", SearchPrefix::Lt, "not-a-date");
        param
            .values
            .push(SearchValue::new(SearchPrefix::Eq, "2024-01-15"));
        let query = SearchQuery::new("Procedure").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(frag.sql.starts_with("(FALSE) OR ("), "{}", frag.sql);
        assert!(
            frag.sql.contains("value_date >= $3 AND value_date < $4"),
            "{}",
            frag.sql
        );
        assert!(!frag.sql.contains("$5"), "{}", frag.sql);
        // Only the valid value's range is bound.
        match frag.params.as_slice() {
            [SqlParam::Timestamp(start), SqlParam::Timestamp(end)] => {
                assert_eq!(start.to_rfc3339(), "2024-01-15T00:00:00+00:00");
                assert_eq!(end.to_rfc3339(), "2024-01-16T00:00:00+00:00");
            }
            other => panic!("must bind exactly the valid day's range: {other:?}"),
        }
    }

    #[test]
    fn unparseable_composite_date_component_matches_nothing() {
        // The date component becomes a bare `FALSE` inside the conjunction, so
        // the token component still binds its code at `$3` and the row
        // predicate can never hold.
        for prefix in ["", "eq", "ne", "gt", "lt", "ge", "le", "sa", "eb", "ap"] {
            for input in NOT_DATES {
                let context = format!("{prefix}{input}");
                let param = SearchParameter {
                    name: "code-value-date".to_string(),
                    param_type: SearchParamType::Composite,
                    modifier: None,
                    values: vec![SearchValue::new(
                        SearchPrefix::Eq,
                        format!("8867-4${prefix}{input}"),
                    )],
                    chain: vec![],
                    components: vec![
                        CompositeSearchComponent {
                            param_type: SearchParamType::Token,
                            param_name: "code".to_string(),
                        },
                        CompositeSearchComponent {
                            param_type: SearchParamType::Date,
                            param_name: "value".to_string(),
                        },
                    ],
                };
                let query = SearchQuery::new("Observation").with_parameter(param);
                let frag = PostgresQueryBuilder::build_search_query(&query, 2)
                    .expect("an invalid composite date must still constrain the query");

                assert!(
                    frag.sql.contains("(value_token_code = $3) AND (FALSE)"),
                    "{context}: {}",
                    frag.sql
                );
                assert!(!frag.sql.contains("value_date"), "{context}: {}", frag.sql);
                assert!(!frag.sql.contains("$4"), "{context}: {}", frag.sql);
                assert_eq!(frag.params.len(), 1, "{context}: {:?}", frag.params);
                assert_binds_no_timestamp(&frag, &context);
            }
        }
    }

    #[test]
    fn unparseable_contained_date_is_not_dropped() {
        // `build_contained` skips a component that yields `None`, which would
        // drop the parameter and match every contained resource of the type.
        let query = SearchQuery::new("Procedure").with_parameter(date_param(
            "date",
            SearchPrefix::Lt,
            "not-a-date",
        ));
        let frag = PostgresQueryBuilder::build_contained(&query)
            .expect("an invalid date must still constrain the contained match");

        assert!(
            frag.sql.contains("(param_name = 'date' AND (FALSE))"),
            "{}",
            frag.sql
        );
        assert!(frag.params.is_empty(), "{:?}", frag.params);
    }

    #[test]
    fn contained_distinct_names_count_names() {
        // Distinct names: the name count proves every branch matched, and the
        // SQL is what it was before occurrences were told apart (#1336).
        let query = SearchQuery::new("Observation")
            .with_parameter(token_param("code", None, "X"))
            .with_parameter(date_param("date", SearchPrefix::Ge, "2020-01-01"));
        let frag = PostgresQueryBuilder::build_contained(&query).unwrap();

        assert!(
            frag.sql.ends_with("HAVING COUNT(DISTINCT param_name) >= 2"),
            "{}",
            frag.sql
        );
        assert!(!frag.sql.contains("bool_or"), "{}", frag.sql);
    }

    #[test]
    fn contained_repeated_name_requires_every_occurrence() {
        // `code=X&date=sa2019-12-31&date=lt2021-01-01,2019`: a name count of
        // 2 is met by a contained resource matching only one date bound
        // (#1336). `sa` and `lt` are single comparisons on a stored range
        // (#1391), unlike `ge`/`le`, so any OR below comes from the comma.
        let mut upper = date_param("date", SearchPrefix::Lt, "2021-01-01");
        upper
            .values
            .push(SearchValue::new(SearchPrefix::Eq, "2019"));
        let query = SearchQuery::new("Observation")
            .with_parameter(token_param("code", None, "X"))
            .with_parameter(date_param("date", SearchPrefix::Sa, "2019-12-31"))
            .with_parameter(upper);
        let frag = PostgresQueryBuilder::build_contained(&query).unwrap();

        let (filter, having) = frag.sql.split_once(" HAVING ").expect("a HAVING clause");
        assert!(!having.contains("COUNT("), "{having}");
        let required: Vec<&str> = having.split(" AND bool_or(").collect();
        assert_eq!(required.len(), 3, "one bool_or per occurrence: {having}");
        assert!(required[0].starts_with("bool_or(param_name = 'code'"));
        assert!(required[1].starts_with("param_name = 'date'"), "{having}");
        assert!(required[2].starts_with("param_name = 'date'"), "{having}");
        // The comma list stays a disjunction inside its own occurrence.
        assert!(!required[1].contains(" OR "), "{having}");
        assert!(required[2].contains(" OR "), "{having}");

        // HAVING re-reads the WHERE placeholders: gap-free from $3, none new.
        // A date `end <= b` reuses its bind for the implied `start < b`
        // (#1391), hence the dedup.
        let mut in_filter = placeholders(filter);
        in_filter.dedup();
        let mut expected: Vec<usize> = vec![1, 2];
        expected.extend(3..3 + frag.params.len());
        assert_eq!(in_filter, expected, "{}", frag.sql);
        let mut in_having = placeholders(having);
        in_having.dedup();
        assert_eq!(in_having, in_filter[2..], "{}", frag.sql);
    }

    fn contained_query(parameters: Vec<SearchParameter>) -> SearchQuery {
        let mut query = SearchQuery::new("Observation");
        query.contained = ContainedMode::On;
        query.parameters = parameters;
        query
    }

    #[test]
    fn contained_negation_is_decided_over_every_row_of_the_entity() {
        // `:not` means "no row of this contained resource matches", which a
        // WHERE row filter would make unanswerable (#1363).
        let query = contained_query(vec![
            token_param("code", None, "X"),
            token_param("category", Some(SearchModifier::Not), "cat1"),
        ]);
        assert!(PostgresQueryBuilder::reject_unsupported_contained(&query).is_ok());
        let frag = PostgresQueryBuilder::build_contained(&query).unwrap();

        let (filter, having) = frag.sql.split_once(" HAVING ").expect("a HAVING clause");
        assert!(!filter.contains("param_name"), "{filter}");
        assert_eq!(
            having,
            "bool_or(param_name = 'code' AND (value_token_code = $3)) AND \
             bool_or(param_name = 'category' AND (value_token_code = $4)) IS NOT TRUE"
        );
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn contained_id_is_the_local_id_and_needs_no_other_criterion() {
        let mut id = token_param("_id", None, "a");
        id.values.push(SearchValue::new(SearchPrefix::Eq, "b"));
        let frag =
            PostgresQueryBuilder::build_contained(&contained_query(vec![id.clone()])).unwrap();
        assert!(
            frag.sql.ends_with(
                "contained_type = $2 AND contained_local_id IN ($3, $4) \
                 GROUP BY resource_type, resource_id, contained_local_id"
            ),
            "{}",
            frag.sql
        );

        // Beside a value parameter the numbering carries on after the ids.
        let frag = PostgresQueryBuilder::build_contained(&contained_query(vec![
            id,
            token_param("_tag", None, "foo"),
        ]))
        .unwrap();
        assert_eq!(placeholders(&frag.sql), vec![1, 2, 3, 4, 5]);
        assert_eq!(frag.params.len(), 3);
        assert!(
            frag.sql
                .contains("(param_name = '_tag' AND (value_token_code = $5))")
                && frag.sql.ends_with("HAVING COUNT(DISTINCT param_name) >= 1"),
            "{}",
            frag.sql
        );
    }

    /// #1383: no criterion is every contained resource of the type, and
    /// compartment membership is one explicit branch over several names.
    #[test]
    fn contained_without_criteria_and_with_a_compartment() {
        let frag = PostgresQueryBuilder::build_contained(&contained_query(vec![])).unwrap();
        assert!(
            frag.sql.ends_with(
                "contained_type = $2 GROUP BY resource_type, resource_id, contained_local_id"
            ),
            "{}",
            frag.sql
        );
        assert!(frag.params.is_empty());

        let mut query = contained_query(vec![token_param("code", None, "X")]);
        query.compartment = Some(CompartmentMembership {
            params: vec!["subject".to_string(), "performer".to_string()],
            reference: "Patient/p1/_history/2".to_string(),
        });
        let frag = PostgresQueryBuilder::build_contained(&query).unwrap();
        // The `HAVING` reuses the `WHERE` placeholders; none is bound twice.
        let filter = frag.sql.split(" HAVING ").next().unwrap();
        assert_eq!(placeholders(filter), vec![1, 2, 3, 4]);
        assert_eq!(frag.params.len(), 2);
        let having = frag.sql.split(" HAVING ").nth(1).expect(&frag.sql);
        // Counting names would let `subject` stand in for `code`.
        assert_eq!(
            having,
            "bool_or(param_name = 'code' AND (value_token_code = $3)) AND \
             bool_or(param_name IN ('subject', 'performer') AND value_reference = $4)"
        );
        assert!(
            matches!(&frag.params[1], SqlParam::Text(s) if s == "Patient/p1"),
            "{:?}",
            frag.params
        );
    }

    /// #1383: `_has` and `_list` select top-level resources.
    #[test]
    fn contained_refuses_has_and_list_by_name() {
        let mut has = contained_query(vec![]);
        has.reverse_chains
            .push(crate::types::ReverseChainedParameter::terminal(
                "Provenance",
                "target",
                "agent",
                SearchValue::new(SearchPrefix::Eq, "Practitioner/x"),
            ));
        let mut list = contained_query(vec![]);
        list.list.push("l1".to_string());
        // `_sort` would be ignored by the contained path, so it is refused too
        // (#1407).
        let mut sorted = contained_query(vec![]);
        sorted
            .sort
            .push(crate::types::SortDirective::parse("-date"));
        for (query, name) in [(has, "'_has'"), (list, "'_list'"), (sorted, "'_sort'")] {
            let message = PostgresQueryBuilder::reject_unsupported_contained(&query)
                .unwrap_err()
                .to_string();
            assert!(message.contains(name), "{message}");
            let mut off = query.clone();
            off.contained = ContainedMode::Off;
            assert!(PostgresQueryBuilder::reject_unsupported_contained(&off).is_ok());
        }
    }

    #[test]
    fn contained_refuses_what_it_cannot_apply_by_name() {
        let mut chained = reference_param("subject", None, "x");
        chained.chain = vec![crate::types::ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }];
        let mut exact_token = token_param("code", Some(SearchModifier::Exact), "hello");
        exact_token.param_type = SearchParamType::Token;
        let refused = [
            date_param("_lastUpdated", SearchPrefix::Gt, "2020"),
            special_param("_text", vec![SearchValue::new(SearchPrefix::Eq, "x")]),
            token_param("_id", Some(SearchModifier::Not), "a"),
            // A composite whose components were never resolved cannot be paired.
            SearchParameter {
                components: vec![],
                ..composite_param("code-value-quantity", "X$5")
            },
            // Not a predicate on one index row, or not a modifier of the type:
            // before #1363 these were read as a plain match.
            token_param("code", Some(SearchModifier::In), "http://vs"),
            token_param("code", Some(SearchModifier::Below), "X"),
            token_param("code", Some(SearchModifier::Missing), "true"),
            exact_token,
            chained,
        ];
        for param in refused {
            let name = param.name.clone();
            let mut query = contained_query(vec![token_param("status", None, "final"), param]);
            let error = PostgresQueryBuilder::reject_unsupported_contained(&query)
                .expect_err(&name)
                .to_string();
            assert!(error.contains(&format!("'{name}'")), "{error}");

            // The same query without `_contained` is none of this gate's business.
            query.contained = ContainedMode::Off;
            assert!(PostgresQueryBuilder::reject_unsupported_contained(&query).is_ok());
        }
    }

    /// The row-level modifiers reuse the top-level builders' bare value
    /// predicates under `_contained` (#1407): same text, without the
    /// `id IN (…)` wrapper, numbered after `$1`/`$2`.
    #[test]
    fn contained_row_level_modifiers_reuse_the_bare_value_predicates() {
        let branch = |param: SearchParameter| {
            let query = contained_query(vec![param]);
            assert!(PostgresQueryBuilder::reject_unsupported_contained(&query).is_ok());
            let frag = PostgresQueryBuilder::build_contained(&query).unwrap();
            let sql = frag
                .sql
                .split_once("contained_type = $2 AND (")
                .and_then(|(_, rest)| rest.split_once(") GROUP BY"))
                .map(|(branch, _)| branch.to_string())
                .unwrap_or_else(|| panic!("no branch in {}", frag.sql));
            (sql, frag.params.len())
        };
        let string = |modifier, value| string_param("name", modifier, value);

        assert_eq!(
            branch(string(Some(SearchModifier::Exact), "Ward 7")),
            (
                "(param_name = 'name' AND ((value_string = $3)))".to_string(),
                1
            )
        );
        assert_eq!(
            branch(string(Some(SearchModifier::Contains), "ard")),
            (
                format!("(param_name = 'name' AND (({FOLDED_STRING_EXPR} LIKE $3 ESCAPE '\\')))"),
                1
            )
        );
        // The default is the accent-folded, wildcard-safe range, two binds a value.
        assert_eq!(
            branch(SearchParameter {
                values: vec![
                    SearchValue::new(SearchPrefix::Eq, "wa"),
                    SearchValue::new(SearchPrefix::Eq, "x%"),
                ],
                ..string(None, "")
            }),
            (
                format!(
                    "(param_name = 'name' AND (({e} ~>=~ $3 AND {e} ~<~ $4) OR ({e} ~>=~ $5 AND {e} ~<~ $6)))",
                    e = FOLDED_STRING_EXPR
                ),
                4
            )
        );
        assert_eq!(
            branch(token_param("code", Some(SearchModifier::Text), "gluc")),
            (
                "(param_name = 'code' AND ((value_token_display ILIKE $3)))".to_string(),
                1
            )
        );
        assert_eq!(
            branch(token_param(
                "identifier",
                Some(SearchModifier::OfType),
                "http://t|MR|123"
            )),
            (
                "(param_name = 'identifier' AND ((value_token_code = $3 AND \
                 value_identifier_type_system = $4 AND value_identifier_type_code = $5)))"
                    .to_string(),
                3
            )
        );
        // A malformed `:of-type` value fails closed instead of being dropped.
        assert_eq!(
            branch(token_param(
                "identifier",
                Some(SearchModifier::OfType),
                "123"
            )),
            ("(param_name = 'identifier' AND (FALSE))".to_string(), 0)
        );
        // Reference: bare id, `:Type`, absolute URL.
        assert_eq!(
            branch(reference_param("subject", None, "p1")),
            (
                format!(
                    "(param_name = 'subject' AND (((value_reference IS NOT NULL AND \
                     {REFERENCE_TARGET_ID_EXPR} = $3))))"
                ),
                1
            )
        );
        assert_eq!(
            branch(reference_param(
                "subject",
                Some(SearchModifier::Type("Patient".to_string())),
                "p1"
            )),
            (
                "(param_name = 'subject' AND ((value_reference = $3)))".to_string(),
                1
            )
        );
        let (identifier, binds) = branch(reference_param(
            "subject",
            Some(SearchModifier::Identifier),
            "http://mrn|42",
        ));
        assert!(
            identifier.starts_with(
                "(param_name = 'subject' AND (value_reference IN (SELECT idx.resource_type || '/' || idx.resource_id FROM search_index idx WHERE idx.tenant_id = $1 AND idx.param_name = 'identifier' AND idx.is_contained = FALSE AND idx.value_token_system = $3 AND idx.value_token_code = $4)"
            ),
            "{identifier}"
        );
        assert_eq!(binds, 2);
        let uri = |modifier| SearchParameter {
            param_type: SearchParamType::Uri,
            ..token_param("url", modifier, "http://x/")
        };
        assert_eq!(
            branch(uri(Some(SearchModifier::Below))),
            (
                "(param_name = 'url' AND ((value_uri LIKE $3 || '%')))".to_string(),
                1
            )
        );
        assert_eq!(
            branch(uri(None)),
            ("(param_name = 'url' AND ((value_uri = $3)))".to_string(), 1)
        );
    }

    /// A composite narrows the contained entities by a per-`composite_group`
    /// pairing over the unfolded contained rows (#1407); its placeholders
    /// continue the numbering, and an unparseable component fails closed.
    #[test]
    fn contained_composite_is_paired_per_group_of_the_entity() {
        let query = contained_query(vec![
            token_param("status", None, "final"),
            SearchParameter {
                values: vec![
                    SearchValue::new(SearchPrefix::Eq, "X$gt5"),
                    SearchValue::new(SearchPrefix::Eq, "X$abc"),
                ],
                ..composite_param("code-value-quantity", "")
            },
            token_param("category", None, "lab"),
        ]);
        assert!(PostgresQueryBuilder::reject_unsupported_contained(&query).is_ok());
        let frag = PostgresQueryBuilder::build_contained(&query).unwrap();
        assert!(
            frag.sql.contains(
                "AND ((resource_type, resource_id, contained_local_id) IN (SELECT resource_type, \
                 resource_id, contained_local_id FROM search_index WHERE tenant_id = $1 AND \
                 is_contained = TRUE AND contained_type = $2 AND param_name = \
                 'code-value-quantity' AND ((value_token_code = $4) OR ("
            ),
            "{}",
            frag.sql
        );
        assert!(
            frag.sql.contains(
                "GROUP BY resource_type, resource_id, contained_local_id, composite_group \
                 HAVING bool_or(value_token_code = $4) AND bool_or("
            ),
            "{}",
            frag.sql
        );
        // `X$abc`: the quantity component is not a number and can never hold.
        assert!(frag.sql.contains("AND bool_or(FALSE))"), "{}", frag.sql);
        let mut seen = placeholders(&frag.sql);
        seen.sort_unstable();
        seen.dedup();
        let expected: Vec<usize> = (1..=frag.params.len() + 2).collect();
        assert_eq!(seen, expected, "{}", frag.sql);
    }

    /// Values that reach the number and quantity builders but are not numbers:
    /// free text, a dangling exponent, a bare prefix remainder, a number with a
    /// tail — and the words `f64::from_str` takes for non-finite values.
    const NOT_NUMBERS: [&str; 14] = [
        "abc",
        "",
        " ",
        "1e",
        "1e+",
        "e5",
        ".",
        "-",
        "0.2abc",
        "1,5",
        "0x10",
        "inf",
        "-Infinity",
        "NaN",
    ];

    #[test]
    fn parse_search_number_follows_the_fhir_decimal_grammar() {
        // FHIR `decimal`: -?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?
        for (input, expected) in [
            ("0", 0.0),
            ("5", 5.0),
            ("5.4", 5.4),
            ("-5.4", -5.4),
            ("100.00", 100.0),
            ("1e3", 1000.0),
            ("1E3", 1000.0),
            ("1e+3", 1000.0),
            ("1.5E-2", 0.015),
            ("-1.5e-2", -0.015),
            // Tolerated beyond the grammar: `+`, leading zeros, a bare point.
            ("+5.4", 5.4),
            ("007", 7.0),
            (".5", 0.5),
            ("5.", 5.0),
            ("-.5", -0.5),
        ] {
            assert_eq!(
                parse_search_number(input),
                Some(expected),
                "input {input:?}"
            );
        }
        for input in NOT_NUMBERS {
            assert_eq!(parse_search_number(input), None, "input {input:?}");
        }
        // Surrounding whitespace is trimmed by the shared grammar: `gt+5`
        // arrives form-decoded as `gt 5`.
        assert_eq!(parse_search_number(" 5 "), Some(5.0));
        // Parses as a float but overflows to infinity; inner whitespace;
        // doubled signs; a second point; digit separators.
        for input in [
            "1e999", "-1e999", "5 4", "- 5", "--5", "+-5", "1.2.3", "1e5.5", "1_000", "infinity",
            "nan", "+inf",
        ] {
            assert_eq!(parse_search_number(input), None, "input {input:?}");
        }
    }

    #[test]
    fn unparseable_number_matches_nothing_under_every_prefix() {
        // #1319: the value used to be skipped, so `probability=abc` was an
        // unconstrained search. Every prefix — `ne` included — must now match
        // nothing and bind nothing.
        for prefix in DATE_PREFIXES {
            for input in NOT_NUMBERS {
                let context = format!("{prefix:?} {input:?}");
                let query =
                    SearchQuery::new("RiskAssessment").with_parameter(number_param(prefix, input));
                let frag = PostgresQueryBuilder::build_search_query(&query, 2)
                    .expect("an invalid number must still constrain the query");

                assert_eq!(frag.sql, "FALSE", "{context}");
                assert!(frag.params.is_empty(), "{context}: {:?}", frag.params);
            }
        }
    }

    #[test]
    fn unparseable_quantity_matches_nothing_under_every_prefix() {
        for prefix in DATE_PREFIXES {
            for input in NOT_NUMBERS {
                for suffix in ["", "|mg", "||mg", "|http://unitsofmeasure.org|mg"] {
                    let value = format!("{input}{suffix}");
                    let context = format!("{prefix:?} {value:?}");
                    let query = SearchQuery::new("Observation")
                        .with_parameter(quantity_param(prefix, &value));
                    let frag = PostgresQueryBuilder::build_search_query(&query, 2)
                        .expect("an invalid quantity must still constrain the query");

                    assert_eq!(frag.sql, "FALSE", "{context}");
                    assert!(frag.params.is_empty(), "{context}: {:?}", frag.params);
                }
            }
        }
    }

    #[test]
    fn valid_quantity_forms_still_build() {
        // The number part decides validity; system and code are free text.
        for (value, binds) in [
            ("5.4", 2),
            ("-5.4", 2),
            ("+5.4", 2),
            ("1e3", 2),
            ("1.5E-2", 2),
            // Non-convertible unit: raw branch only (value range + unit).
            ("5.4|a\\|b", 3),
            ("5.4|http://unitsofmeasure.org|a\\|b", 4),
        ] {
            let query = SearchQuery::new("Observation")
                .with_parameter(quantity_param(SearchPrefix::Eq, value));
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
            assert!(frag.sql.contains("value_quantity_value >= $3"), "{value}");
            assert_eq!(frag.params.len(), binds, "{value}: {:?}", frag.params);
        }

        // The escaped pipe reaches the bind unescaped.
        let query = SearchQuery::new("Observation")
            .with_parameter(quantity_param(SearchPrefix::Eq, "5.4||a\\|b"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
        assert!(
            matches!(frag.params.get(2), Some(SqlParam::Text(unit)) if unit == "a|b"),
            "{:?}",
            frag.params
        );
    }

    #[test]
    fn unparseable_number_beside_a_valid_one_keeps_placeholders_gap_free() {
        // The values of one parameter are alternatives (#1300), so a bad value
        // is an alternative that matches nothing and the valid one next to it
        // decides the result: `FALSE OR x` — never more than the valid value
        // alone selects. This asserted `(FALSE) AND (` while the fold was AND
        // (#1319); see `unparseable_date_beside_a_valid_one_keeps_placeholders_gap_free`
        // for the same change on dates. Over REST such a list never gets here:
        // the number check in `parse_search_parameter` answers 400.
        //
        // The bad value must not consume a placeholder number it never binds:
        // the valid value after it still starts at `$3`.
        let mut param = number_param(SearchPrefix::Lt, "abc");
        param.values.push(SearchValue::new(SearchPrefix::Gt, "0.5"));
        let query = SearchQuery::new("RiskAssessment").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(frag.sql.starts_with("(FALSE) OR ("), "{}", frag.sql);
        assert!(frag.sql.contains("value_number > $3"), "{}", frag.sql);
        assert!(!frag.sql.contains("$4"), "{}", frag.sql);
        assert!(
            matches!(frag.params.as_slice(), [SqlParam::Float(f)] if *f == 0.5),
            "{:?}",
            frag.params
        );

        let mut param = quantity_param(SearchPrefix::Ne, "abc||mg");
        param.values.push(SearchValue::new(SearchPrefix::Gt, "6"));
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");

        assert!(frag.sql.starts_with("(FALSE) OR ("), "{}", frag.sql);
        assert!(
            frag.sql.contains("value_quantity_value > $3"),
            "{}",
            frag.sql
        );
        assert!(!frag.sql.contains("$4"), "{}", frag.sql);
        assert_eq!(frag.params.len(), 1, "{:?}", frag.params);
    }

    #[test]
    fn unparseable_composite_numeric_component_matches_nothing() {
        // As for the Date component: a bare `FALSE` inside the conjunction, so
        // the token component still binds its code at `$3` and the row
        // predicate can never hold.
        for component_type in [SearchParamType::Number, SearchParamType::Quantity] {
            for prefix in ["", "eq", "ne", "gt", "lt", "ge", "le", "sa", "eb", "ap"] {
                for input in NOT_NUMBERS {
                    let context = format!("{component_type:?} {prefix}{input}");
                    let param = SearchParameter {
                        name: "code-value-quantity".to_string(),
                        param_type: SearchParamType::Composite,
                        modifier: None,
                        values: vec![SearchValue::new(
                            SearchPrefix::Eq,
                            format!("8480-6${prefix}{input}"),
                        )],
                        chain: vec![],
                        components: vec![
                            CompositeSearchComponent {
                                param_type: SearchParamType::Token,
                                param_name: "code".to_string(),
                            },
                            CompositeSearchComponent {
                                param_type: component_type,
                                param_name: "value-quantity".to_string(),
                            },
                        ],
                    };
                    let query = SearchQuery::new("Observation").with_parameter(param);
                    let frag = PostgresQueryBuilder::build_search_query(&query, 2)
                        .expect("an invalid composite number must still constrain the query");

                    // A component with nothing in it (`8480-6$`) is an empty
                    // value: the whole parameter matches nothing (#1380).
                    if prefix.is_empty() && input.trim().is_empty() {
                        assert_eq!(frag.sql, "FALSE", "{context}");
                        continue;
                    }

                    assert!(
                        frag.sql.contains("(value_token_code = $3) AND (FALSE)"),
                        "{context}: {}",
                        frag.sql
                    );
                    assert!(
                        !frag.sql.contains("value_quantity"),
                        "{context}: {}",
                        frag.sql
                    );
                    assert!(
                        !frag.sql.contains("value_number"),
                        "{context}: {}",
                        frag.sql
                    );
                    assert!(!frag.sql.contains("$4"), "{context}: {}", frag.sql);
                    assert_eq!(frag.params.len(), 1, "{context}: {:?}", frag.params);
                }
            }
        }
    }

    #[test]
    fn unparseable_contained_number_is_not_dropped() {
        // `build_contained` skips a component that yields `None`, which would
        // drop the parameter — and lower the `HAVING COUNT` with it — so a
        // contained resource matching only the *other* parameter came back.
        for (numeric, name) in [
            (number_param(SearchPrefix::Ne, "abc"), "value-number"),
            (
                quantity_param(SearchPrefix::Lt, "inf||mg"),
                "value-quantity",
            ),
        ] {
            let query = SearchQuery::new("Observation")
                .with_parameter(token_param("code", None, "8480-6"))
                .with_parameter(numeric);
            let frag = PostgresQueryBuilder::build_contained(&query)
                .expect("an invalid number must still constrain the contained match");

            assert!(
                frag.sql
                    .contains(&format!("(param_name = '{name}' AND (FALSE))")),
                "{}",
                frag.sql
            );
            assert!(
                frag.sql.contains("HAVING COUNT(DISTINCT param_name) >= 2"),
                "{}",
                frag.sql
            );
            assert_eq!(frag.params.len(), 1, "{:?}", frag.params);
        }
    }

    #[test]
    fn text_or_list_placeholders_are_gap_free() {
        // Two terms OR together, and a blank one must not consume a placeholder
        // number it never binds — the caller binds this fragment's params
        // consecutively.
        //
        // Driven at the FTS builder: through `build_search_query` a parameter
        // with a blank term matches nothing as a whole (#1380).
        let values = vec![
            SearchValue::eq("   "),
            SearchValue::eq("fracture"),
            SearchValue::eq("sprain"),
        ];
        let frag = PostgresQueryBuilder::build_fts_condition(&values, "narrative_tsvector", 2)
            .expect("_text OR-list should produce a condition");

        assert_eq!(frag.params.len(), 2, "the blank term binds nothing");
        assert!(frag.sql.contains("$3"), "{}", frag.sql);
        assert!(frag.sql.contains("$4"), "{}", frag.sql);
        assert!(
            !frag.sql.contains("$5"),
            "placeholder numbering must be gap-free: {}",
            frag.sql
        );
        assert!(frag.sql.contains(" OR "), "{}", frag.sql);
    }

    #[test]
    fn blank_text_term_fails_closed() {
        // Dropping the parameter would return the whole resource type, which is
        // the exact failure mode being fixed.
        let query = SearchQuery::new("Patient")
            .with_parameter(special_param("_text", vec![SearchValue::eq("")]));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("a blank _text must still constrain the query");

        assert_eq!(frag.sql, "FALSE");
        assert!(frag.params.is_empty());
    }

    #[test]
    fn token_or_list_placeholders_are_gap_free() {
        // Regression: `status=finished,in-progress` — two code-only token values,
        // each binding one param. With offset 2 the placeholders must be $3 and $4
        // with exactly 2 params bound. The old fixed 2-slot stride emitted $3 and
        // $5 while binding only 2 params, which Postgres rejected at execute time
        // ("Failed to execute search: db error").
        let param = SearchParameter {
            name: "status".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "finished"),
                SearchValue::new(SearchPrefix::Eq, "in-progress"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("token OR-list should produce a condition");

        assert_eq!(frag.params.len(), 2, "two code-only values bind two params");
        assert!(frag.sql.contains("value_token_code = $3"));
        assert!(frag.sql.contains("value_token_code = $4"));
        assert!(
            !frag.sql.contains("$5"),
            "placeholder numbering must be gap-free: {}",
            frag.sql
        );
    }

    #[test]
    fn token_or_list_mixed_forms_placeholders_sequential() {
        // code-only (1 param) then system|code (2 params): placeholders must run
        // $3, $4, $5 with 3 params bound — no gaps across differing per-value arity.
        let param = SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "8302-2"),
                SearchValue::new(SearchPrefix::Eq, "http://loinc.org|29463-7"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("mixed token OR-list should produce a condition");

        assert_eq!(frag.params.len(), 3);
        assert!(frag.sql.contains("value_token_code = $3"));
        assert!(frag.sql.contains("value_token_system IN ($4, "));
        assert!(frag.sql.contains("value_token_code = $5"));
        assert!(!frag.sql.contains("$6"));
    }

    #[test]
    fn composite_returns_none_without_components() {
        let param = SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "a$b")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        assert!(PostgresQueryBuilder::build_search_query(&query, 2).is_none());
    }

    #[test]
    fn composite_confines_components_to_one_group() {
        // The invariant under test is unchanged from the grouped form: a
        // resource may only match when BOTH components are satisfied within the
        // SAME composite instance — a blood-pressure panel's systolic value must
        // never pair with its diastolic code.
        //
        // What changed is how that is enforced. The old form read one row per
        // component and used GROUP BY resource_id, composite_group + HAVING
        // MAX(CASE …) to require both within a group. The denormalized layout
        // (#279) stores one row per (resource, composite_group) carrying every
        // component's value, so a plain conjunction over ONE row enforces the
        // same thing by construction — components cannot come from different
        // groups because they cannot come from different rows.
        //
        // So this asserts the new mechanism, and deliberately asserts the
        // absence of the old one: if GROUP BY reappears here alongside the flat
        // predicate, the two layouts have been mixed and the result is wrong.
        let param = SearchParameter {
            name: "combo-code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://loinc.org|8480-6$gt140",
            )],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value".to_string(),
                },
            ],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag =
            PostgresQueryBuilder::build_search_query(&query, 2).expect("composite condition");

        // One row per composite instance: the predicate is scoped to composite
        // rows and every component is ANDed within that single row.
        assert!(
            frag.sql.contains("composite_group IS NOT NULL"),
            "the subquery must be confined to composite rows: {}",
            frag.sql
        );
        assert!(
            frag.sql.contains(") AND (value_quantity_value"),
            "components must be ANDed within one row, not OR-prefiltered: {}",
            frag.sql
        );
        // The old aggregate form must be gone — mixing the two layouts would
        // aggregate over already-denormalized rows and match across groups.
        assert!(
            !frag.sql.contains("GROUP BY resource_id, composite_group"),
            "the denormalized layout must not re-aggregate: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("MAX(CASE WHEN"),
            "the denormalized layout must not use the HAVING form: {}",
            frag.sql
        );
    }

    #[test]
    fn composite_unparseable_component_binds_no_params() {
        // `8867-4$abc` — the quantity component fails to parse. Originally the
        // token component's params were pushed before the quantity component
        // bailed, leaving them bound with no placeholder to reference them, which
        // Postgres rejects outright ("bind message supplies N parameters...") — a
        // 500 on a malformed query rather than an empty result. The component now
        // comes back as a bare `FALSE` (#1319), so the token's bind stays, with
        // its placeholder; what must hold is that binds and placeholders agree.
        let param = SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "8867-4$abc")],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value".to_string(),
                },
            ],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("unparseable composite still yields a condition");

        assert_eq!(
            frag.params.len(),
            1,
            "an unparseable composite value must leave no orphaned bind params: {}",
            frag.sql
        );
        assert!(frag.sql.contains("$3") && !frag.sql.contains("$4"));
        assert!(frag.sql.contains("AND (FALSE)"));
    }

    #[test]
    fn token_or_list_is_a_single_sublink() {
        // OR'd sublinks (`id IN (..) OR id IN (..)`) are not pulled up into a
        // semi-join by Postgres; they stay SubPlans that can be re-executed per
        // outer row. The OR must live INSIDE one sublink.
        let param = SearchParameter {
            name: "status".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "finished"),
                SearchValue::new(SearchPrefix::Eq, "in-progress"),
                SearchValue::new(SearchPrefix::Eq, "planned"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token OR-list");

        assert_eq!(
            frag.sql.matches("id IN (SELECT").count(),
            1,
            "a 3-value OR-list must produce exactly one sublink, not three: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 3);
    }

    #[test]
    fn string_search_is_sargable_and_escapes_wildcards() {
        // `ILIKE` can never use a btree, and the raw `COALESCE(folded, value_string)`
        // could not be matched by any index. v34 goes further than `LIKE`: the
        // starts-with form is emitted as an explicit bytewise range, because a
        // `LIKE` whose pattern is a bind parameter cannot be turned into an
        // index range by the planner at all (`match_pattern_prefix` needs a
        // `Const`). See `migrate_v32_to_v33`.
        let param = SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            // A literal '%' in the search value is not a wildcard, and in the
            // range form it never becomes one — there is no pattern to escape.
            values: vec![SearchValue::new(SearchPrefix::Eq, "50%Off")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Organization").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");

        assert!(
            frag.sql.contains(
                "COALESCE(value_string_folded, lower(value_string)) ~>=~ $3 AND \
                 COALESCE(value_string_folded, lower(value_string)) ~<~ $4"
            ),
            "starts-with must be a bytewise range on the indexed folded expression: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("ILIKE"),
            "ILIKE is not btree-sargable: {}",
            frag.sql
        );
        let bounds: Vec<&str> = frag
            .params
            .iter()
            .map(|p| match p {
                SqlParam::Text(s) => s.as_str(),
                other => panic!("expected a text bound, got {:?}", other),
            })
            .collect();
        assert_eq!(
            bounds,
            vec!["50%off", "50%ofg"],
            "the bounds are the folded value and its successor, and carry no              LIKE escaping because there is no pattern"
        );
    }

    #[test]
    fn prefix_upper_bound_covers_exactly_the_prefix() {
        // ASCII: increment the last character.
        assert_eq!(prefix_upper_bound("emilia").as_deref(), Some("emilib"));
        assert_eq!(prefix_upper_bound("a").as_deref(), Some("b"));
        // Multi-byte: the successor is the next code point, and UTF-8 byte order
        // is code-point order, so `~<~` (bytewise) still bounds the prefix.
        assert_eq!(
            prefix_upper_bound("caf\u{e9}").as_deref(),
            Some("caf\u{ea}")
        );
        // The surrogate block holds no scalar value, so it is stepped over.
        assert_eq!(prefix_upper_bound("\u{d7ff}").as_deref(), Some("\u{e000}"));
        // `char::MAX` has no successor: the increment carries left.
        assert_eq!(prefix_upper_bound("a\u{10ffff}").as_deref(), Some("b"));
        // Nothing sorts above an all-`char::MAX` prefix, and an empty prefix
        // matches everything. Both fall back to `LIKE`.
        assert_eq!(prefix_upper_bound("\u{10ffff}"), None);
        assert_eq!(prefix_upper_bound(""), None);
    }

    /// Every value the FHIR benchmark's `searchConfig.js` sends for a `string`
    /// parameter must take the seeking path, not the `LIKE` fallback.
    ///
    /// Run 33179839720 left 24,865 calls on the `LIKE` statement, and the first
    /// hypothesis was that `prefix_upper_bound` was returning `None` more often
    /// than expected. It was not — the remainder is `:contains`, which the
    /// script sends for half of every string request — but the question is
    /// worth closing permanently rather than re-deriving.
    #[test]
    fn every_benchmark_search_term_gets_a_bound() {
        // Verbatim from HealthSamurai/fhir-server-performance-benchmark,
        // k6/searchConfig.js, the "string" block.
        let terms = [
            "NON-EXISTS",
            "Emilia",
            "Carolynn",
            "Stefan",
            "Linh",
            "Harold",
            "Pilar",
            "Ron",
            "Garfield",
            "Margaretta",
            "Giovanna",
            "Dione",
            "Arron",
            "Lanny",
            "Harvey",
            "Beatriz",
            "Donovan",
            "Reyes",
            "Santiago",
            "Kyong",
            "Curtis",
            "Raynham",
            "Springfield",
            "Lowell",
            "Southwick",
            "Mashpee",
            "Holbrook",
            "Falmouth",
            "Revere",
            "Sturbridge",
            "Blackstone",
            "Westport",
            "Walpole",
            "Northampton",
            "Fall River",
            "Waltham",
            "Acushnet Center",
            "Newton",
            "Winchester",
            "Maynard",
            "ORLEANS MEDICAL CENTER, P.C.",
            "ENCOMPASS HEALTH BRAINTREE HOSPITAL OF BRAINTREE",
            "STEWARD HOLY FAMILY HOSPITAL INC",
            "THE NORTHEAST HEALTH GROUP, INC",
            "PLYMOUTH BAY INTERNAL MEDICINE",
            "ART OF CARE INC",
            "T MASSACHUSETTS, LLC",
            "RIVERBEND OF SOUTH NATICK",
            "NEW ENGLAND PROFESSIONAL HOME HEALTH CARE LLC",
            "HDH CORPORATION",
            // The script escapes `\ , $ |` before sending; if any survives to
            // here it is still a literal prefix character, in the range exactly
            // as it was in the `LIKE` pattern after `like_escape`.
            "ORLEANS MEDICAL CENTER\\, P.C.",
        ];
        for term in terms {
            let query =
                SearchQuery::new("Patient").with_parameter(string_param("name", None, term));
            let frag =
                PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");
            assert!(
                frag.sql.contains("~>=~ $3 AND") && frag.sql.contains("~<~ $4)"),
                "{:?} fell back to the LIKE form: {}",
                term,
                frag.sql
            );
            assert!(
                !frag.sql.contains("IS NOT NULL"),
                "{:?} kept the conjunct: {}",
                term,
                frag.sql
            );
        }
    }

    #[test]
    fn empty_string_value_falls_back_to_the_like_form() {
        // With no prefix to bound, the string builder emits the `LIKE '%'`
        // form, which matches every indexed value. The strict `~~` proves the
        // index predicate on its own, so it carries no conjunct either.
        //
        // That is the builder on its own. A search never gets here with an
        // empty value: the gate refuses it, and `build_search_query` answers
        // such a parameter with `FALSE` (#1380).
        let param = string_param("name", None, "");
        let query = SearchQuery::new("Patient").with_parameter(param.clone());
        let guarded = PostgresQueryBuilder::build_search_query(&query, 2).expect("condition");
        assert_eq!(guarded.sql, "FALSE");

        let frag =
            PostgresQueryBuilder::build_string_condition(&param, 2).expect("string condition");

        assert!(
            frag.sql
                .contains("COALESCE(value_string_folded, lower(value_string)) LIKE $3 ESCAPE"),
            "{}",
            frag.sql
        );
        assert!(!frag.sql.contains("IS NOT NULL"), "{}", frag.sql);
        match &frag.params[0] {
            SqlParam::Text(p) => assert_eq!(p, "%"),
            other => panic!("expected a text param, got {:?}", other),
        }
        assert_eq!(frag.params.len(), 1);
    }

    #[test]
    fn multi_value_string_search_numbers_both_bounds_of_every_value() {
        // The starts-with form binds two parameters per value, so a two-value
        // OR-list must be numbered $3..$6 — not $3,$4 — or every later
        // parameter in the query is bound to the wrong placeholder.
        let param = SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "ann"),
                SearchValue::new(SearchPrefix::Eq, "bob"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Patient").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");

        assert!(frag.sql.contains("~>=~ $3 AND"), "{}", frag.sql);
        assert!(frag.sql.contains("~<~ $4)"), "{}", frag.sql);
        assert!(frag.sql.contains("~>=~ $5 AND"), "{}", frag.sql);
        assert!(frag.sql.contains("~<~ $6)"), "{}", frag.sql);
        assert_eq!(frag.params.len(), 4);
    }

    /// Builds a string `SearchParameter` with an optional modifier.
    fn string_param(name: &str, modifier: Option<SearchModifier>, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::String,
            modifier,
            values: vec![SearchValue::new(SearchPrefix::Eq, value)],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn folded_string_search_implies_the_partial_index_predicate() {
        // `idx_search_string_folded_pattern` is partial, and Postgres only uses
        // a partial index when the query implies its predicate.
        //
        // v25 got there with an explicit `value_string IS NOT NULL` conjunct,
        // because `COALESCE(a, b) LIKE …` does not imply it (COALESCE is not
        // strict). v34 rewords the index predicate onto the COALESCE itself and
        // lets the strict operator do the proving — which also takes the
        // conjunct, and the 200x row-estimate error it caused, out of the
        // starts-with plan. See `migrate_v32_to_v33`.
        let query =
            SearchQuery::new("Patient").with_parameter(string_param("name", None, "Emilia"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");
        assert!(
            frag.sql
                .contains("COALESCE(value_string_folded, lower(value_string)) ~>=~ $3"),
            "starts-with must prove the predicate through the strict operator: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("IS NOT NULL"),
            "the conjunct is what made the planner estimate 25 rows for a \
             5,000-row slice; it must not come back: {}",
            frag.sql
        );

        // `:contains`/`:text` lose it too. `~~` is strict in the COALESCE, so
        // it proves the predicate of both the trigram GIN index and the btree
        // pattern index; carrying the conjunct as well only reintroduced the
        // estimate error, and on that estimate the planner never costed the GIN
        // scan competitively (measured: 1,709 buffers on the btree slice scan
        // versus 58 once the conjunct is gone and the GIN index exists).
        for modifier in [Some(SearchModifier::Contains), Some(SearchModifier::Text)] {
            let query = SearchQuery::new("Patient").with_parameter(string_param(
                "name",
                modifier.clone(),
                "Emilia",
            ));
            let frag =
                PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");
            assert!(
                frag.sql
                    .contains("COALESCE(value_string_folded, lower(value_string)) LIKE $3 ESCAPE"),
                "modifier {:?} must match the folded expression: {}",
                modifier,
                frag.sql
            );
            assert!(
                !frag.sql.contains("IS NOT NULL"),
                "modifier {:?} must not carry the conjunct — it is what kept the \
                 planner off the trigram index: {}",
                modifier,
                frag.sql
            );
        }
    }

    #[test]
    fn exact_string_search_keeps_the_bare_column_predicate() {
        // `:exact` reads `value_string` directly, which is already strict, so it
        // must not acquire the redundant conjunct — `idx_search_string` serves it.
        let query = SearchQuery::new("Patient").with_parameter(string_param(
            "name",
            Some(SearchModifier::Exact),
            "Emilia",
        ));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");

        assert!(frag.sql.contains("value_string = $3"), "{}", frag.sql);
        assert!(!frag.sql.contains("IS NOT NULL"), "{}", frag.sql);
        assert!(!frag.sql.contains("COALESCE"), "{}", frag.sql);
    }

    /// Builds a token `SearchParameter` with an optional modifier.
    fn token_param(name: &str, modifier: Option<SearchModifier>, value: &str) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Token,
            modifier,
            values: vec![SearchValue::new(SearchPrefix::Eq, value)],
            chain: vec![],
            components: vec![],
        }
    }

    /// Builds a reference `SearchParameter` with an optional modifier.
    fn reference_param(
        name: &str,
        modifier: Option<SearchModifier>,
        value: &str,
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Reference,
            modifier,
            values: vec![SearchValue::new(SearchPrefix::Eq, value)],
            chain: vec![],
            components: vec![],
        }
    }

    /// v28 dropped `idx_search_string_folded`, the btree on the bare
    /// `value_string_folded` column, because no emitted predicate can seek it:
    /// the column is only ever read through `COALESCE(value_string_folded,
    /// lower(value_string))`, which an index on the bare column cannot match,
    /// and `sort_value_column` maps `String` to `value_string`.
    ///
    /// If a bare predicate on the folded column is ever added back it will get a
    /// sequential scan and nobody will notice, so pin the rule here.
    #[test]
    fn the_folded_column_is_only_ever_read_through_the_coalesce() {
        for modifier in [
            None,
            Some(SearchModifier::Contains),
            Some(SearchModifier::Text),
            Some(SearchModifier::Exact),
        ] {
            let query = SearchQuery::new("Patient").with_parameter(string_param(
                "name",
                modifier.clone(),
                "Emilia",
            ));
            let frag =
                PostgresQueryBuilder::build_search_query(&query, 2).expect("string condition");
            assert_eq!(
                frag.sql.matches("value_string_folded").count(),
                frag.sql.matches(FOLDED_STRING_EXPR).count(),
                "modifier {:?} reads the folded column outside the indexed \
                 COALESCE, which no index in the schema can serve: {}",
                modifier,
                frag.sql
            );
        }
        assert_eq!(
            sort_value_column(SearchParamType::String),
            Some("value_string")
        );
    }

    /// v28 dropped `idx_search_token_display` and `idx_search_reference_display`.
    /// Both were btrees in the default operator class, and `ILIKE` is not
    /// sargable against one at any prefix — so they were only ever scanners of
    /// their `(tenant_id, resource_type, param_name)` slice, which
    /// `idx_search_token_code` and `idx_search_reference_pattern` cover over a
    /// superset of the rows.
    ///
    /// The drop is safe exactly as long as `ILIKE` stays the only operator on
    /// these columns. An `=` or a prefix `LIKE` added later WOULD be sargable
    /// and would then want an index that no longer exists.
    #[test]
    fn display_columns_are_only_ever_matched_with_ilike() {
        let cases: Vec<(SearchQuery, &str)> = vec![
            (
                SearchQuery::new("Observation").with_parameter(token_param(
                    "code",
                    Some(SearchModifier::Text),
                    "Blood",
                )),
                "value_token_display",
            ),
            (
                SearchQuery::new("Observation").with_parameter(token_param(
                    "code",
                    Some(SearchModifier::CodeText),
                    "Blood",
                )),
                "value_token_display",
            ),
            (
                SearchQuery::new("Observation").with_parameter(reference_param(
                    "subject",
                    Some(SearchModifier::Text),
                    "Emilia",
                )),
                "value_reference_display",
            ),
            (
                SearchQuery::new("Observation").with_parameter(reference_param(
                    "subject",
                    Some(SearchModifier::CodeText),
                    "Emilia",
                )),
                "value_reference_display",
            ),
        ];

        for (query, column) in cases {
            let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("display shape");
            let mentions = frag.sql.matches(column).count();
            assert!(mentions > 0, "expected a {column} predicate: {}", frag.sql);
            assert_eq!(
                frag.sql.matches(&format!("{column} ILIKE")).count(),
                mentions,
                "{column} must only ever be matched with ILIKE — anything \
                 sargable needs an index this schema no longer has: {}",
                frag.sql
            );
        }
    }

    /// v28 dropped `idx_search_reference` and kept its `text_pattern_ops` twin
    /// `idx_search_reference_pattern`, which has the same key columns and the
    /// same partial predicate. That family carries `=` but NOT the ordering
    /// operators, so the drop holds only while every predicate on
    /// `value_reference` is equality or a `LIKE` — and it is the pattern index,
    /// not this one, that the prefix `LIKE`s need.
    #[test]
    fn reference_predicates_never_need_a_collation_ordered_index() {
        let queries = [
            SearchQuery::new("Observation").with_parameter(reference_param(
                "subject",
                None,
                "Patient/p1",
            )),
            SearchQuery::new("Observation").with_parameter(reference_param("subject", None, "p1")),
            SearchQuery::new("Observation").with_parameter(reference_param(
                "subject",
                Some(SearchModifier::Below),
                "http://h/Organization/o1",
            )),
            SearchQuery::new("Observation").with_parameter(reference_param(
                "subject",
                Some(SearchModifier::Contains),
                "p1",
            )),
        ];

        for query in queries {
            let frag =
                PostgresQueryBuilder::build_search_query(&query, 2).expect("reference shape");
            for op in [" < ", " > ", " <= ", " >= ", "ORDER BY value_reference"] {
                assert!(
                    !frag.sql.contains(op),
                    "an ordering comparison on value_reference cannot use the \
                     text_pattern_ops index that survives v28 (`{op}`): {}",
                    frag.sql
                );
            }
        }
    }

    #[test]
    fn system_qualified_token_is_one_extractable_equality_conjunction() {
        // `Encounter?class=http://…v3-ActCode|AMB`. This is the shape v25's
        // `idx_search_token` is keyed for: every column ahead of the sort key is
        // bound by equality, so the index can return the fast path's rows already
        // ordered. If this ever became a range, an OR, or a second sublink, that
        // index could no longer stream and the LIMIT would stop terminating early.
        let param = SearchParameter {
            name: "class".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://terminology.hl7.org/CodeSystem/v3-ActCode|AMB",
            )],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token condition");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(
            pred,
            format!(
                "param_name = 'class' AND ((value_token_system IN ($3, '{IMPLICIT_TOKEN_SYSTEM}') \
                 AND value_token_code = $4))"
            )
        );
        assert_eq!(frag.params.len(), 2);
        match (&frag.params[0], &frag.params[1]) {
            (SqlParam::Text(system), SqlParam::Text(code)) => {
                assert_eq!(system, "http://terminology.hl7.org/CodeSystem/v3-ActCode");
                assert_eq!(code, "AMB");
            }
            other => panic!("expected two text params, got {:?}", other),
        }
    }

    #[test]
    fn bare_token_predicate_never_mentions_the_system_column() {
        // The counterpart to the test above, and the reason `Encounter?status` is
        // 4x cheaper than `Encounter?class` on identical data: a bare code is not
        // strict in `value_token_system`, so it cannot reach the partial
        // `idx_search_token` at all and is served by the code-first indexes.
        let param = SearchParameter {
            name: "status".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "finished")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token condition");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(pred, "param_name = 'status' AND (value_token_code = $3)");
        assert!(!pred.contains("value_token_system"));
    }

    /// `|code` means "the code, with no system" (#1388), not "any system". A
    /// `code` element's row carries the implicit marker (#1379) and has no
    /// explicit system either, so it counts as "no system" too.
    #[test]
    fn empty_system_token_requires_no_system() {
        let query =
            SearchQuery::new("Observation").with_parameter(token_param("code", None, "|1234-5"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token condition");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(
            pred,
            format!(
                "param_name = 'code' AND (((value_token_system IS NULL \
                 OR value_token_system IN ('', '{IMPLICIT_TOKEN_SYSTEM}')) \
                 AND value_token_code = $3))"
            )
        );
        assert_eq!(frag.params.len(), 1);
        match &frag.params[0] {
            SqlParam::Text(code) => assert_eq!(code, "1234-5"),
            other => panic!("expected a text param, got {:?}", other),
        }
    }

    /// `:not` stays the exact negation of the positive `|code` predicate.
    #[test]
    fn not_empty_system_token_negates_the_no_system_predicate() {
        let positive = PostgresQueryBuilder::build_search_query(
            &SearchQuery::new("Observation").with_parameter(token_param("code", None, "|1234-5")),
            2,
        )
        .expect("token condition");
        let negated = PostgresQueryBuilder::build_search_query(
            &SearchQuery::new("Observation").with_parameter(token_param(
                "code",
                Some(SearchModifier::Not),
                "|1234-5",
            )),
            2,
        )
        .expect("token condition");

        assert_eq!(negated.sql, format!("NOT ({})", positive.sql));
        assert_eq!(negated.params.len(), 1);
    }

    /// The composite component builder (also used by contained-resource
    /// search) gives `|code` the same "no system" meaning, in either slot.
    #[test]
    fn composite_empty_system_component_requires_no_system() {
        let value = SearchValue::new(SearchPrefix::Eq, "|1234-5");
        let (sql, params) =
            PostgresQueryBuilder::build_composite_component(&value, SearchParamType::Token, 4, 1)
                .expect("a token component");
        assert_eq!(
            sql,
            format!(
                "(value_token_system IS NULL OR value_token_system IN ('', '{IMPLICIT_TOKEN_SYSTEM}')) \
                 AND value_token_code = $5"
            )
        );
        assert_eq!(params.len(), 1);

        let (sql, _) =
            PostgresQueryBuilder::build_composite_component(&value, SearchParamType::Token, 4, 2)
                .expect("a token component");
        assert_eq!(
            sql,
            format!(
                "(value_token_system_2 IS NULL OR value_token_system_2 IN ('', '{IMPLICIT_TOKEN_SYSTEM}')) \
                 AND value_token_code_2 = $5"
            )
        );
    }

    /// v31 replaced `idx_search_token` (2,283 MB, system-first, and unable to
    /// give this shape the sort key because `value_token_code` sat between the
    /// system and `last_updated`) with a seek-only `idx_search_token_system`;
    /// v32 dropped that too, because `system|code` is strict in
    /// `value_token_system` as well and the planner pointed it there —
    /// 80,089,347 tuples read for a 358 ms p99.
    ///
    /// So `idx_search_token_code_recent` is now the ONLY index the `system|`
    /// form can reach, and the `value_token_code IS NOT NULL` conjunct below is
    /// the only reason it can: that index is partial on exactly that predicate.
    /// Drop the conjunct and this shape has no index at all; turn it into a
    /// comparison against a code VALUE and the search becomes wrong.
    #[test]
    fn system_only_token_implies_the_code_partial_predicate() {
        let param = SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "http://loinc.org|")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token condition");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(
            pred,
            "param_name = 'code' AND ((value_token_code IS NOT NULL AND value_token_system = $3))"
        );
        assert_eq!(frag.params.len(), 1);
        match &frag.params[0] {
            SqlParam::Text(system) => assert_eq!(system, "http://loinc.org"),
            other => panic!("expected one text param, got {:?}", other),
        }
    }

    /// The `system|` conjunct is only row-set-preserving while every token row
    /// that carries a system also carries a code. `IndexValue::Token` makes that
    /// a type-level fact (`code: String`), but an OR-list mixing spellings must
    /// still put each arm's conjunct inside its own parentheses, or `AND`
    /// binding tighter than `OR` would be the only thing keeping the grouping
    /// correct.
    #[test]
    fn mixed_token_or_list_keeps_each_arm_parenthesized() {
        let param = SearchParameter {
            name: "class".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "AMB"),
                SearchValue::new(
                    SearchPrefix::Eq,
                    "http://terminology.hl7.org/CodeSystem/v3-ActCode|",
                ),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Encounter").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("token OR-list");

        let pred = PostgresQueryBuilder::single_index_predicate(&frag.sql)
            .expect("a lone membership test is extractable");
        assert_eq!(
            pred,
            concat!(
                "param_name = 'class' AND (value_token_code = $3 OR ",
                "(value_token_code IS NOT NULL AND value_token_system = $4))"
            )
        );
        assert_eq!(frag.params.len(), 2);
    }

    #[test]
    fn reference_or_list_is_a_single_sublink() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "Patient/a"),
                SearchValue::new(SearchPrefix::Eq, "Patient/b"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("reference OR-list");

        assert_eq!(
            frag.sql.matches("id IN (SELECT").count(),
            1,
            "reference OR-list must be one sublink: {}",
            frag.sql
        );
        // One param per type-prefixed value since v33: the base. The stored
        // value is the base too, so there is no second form to match.
        assert_eq!(frag.params.len(), 2);
        assert!(
            !frag.sql.contains("LIKE"),
            "the OR-list must be pure equalities: {}",
            frag.sql
        );
    }

    #[test]
    fn reference_or_list_mixes_bare_and_type_prefixed_values() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "patient-1"),
                SearchValue::new(SearchPrefix::Eq, "Patient/patient-2"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("reference OR-list");

        assert_eq!(
            frag.sql.matches("id IN (SELECT").count(),
            1,
            "reference OR-list must remain one subquery: {}",
            frag.sql
        );
        assert!(
            frag.sql.contains(&format!(
                "(value_reference IS NOT NULL AND {REFERENCE_TARGET_ID_EXPR} = $3) OR value_reference = $4"
            )),
            "each OR arm must use its assigned placeholder: {}",
            frag.sql
        );
        let params: Vec<&str> = frag
            .params
            .iter()
            .map(|param| match param {
                SqlParam::Text(value) => value.as_str(),
                other => panic!("expected text params, got {other:?}"),
            })
            .collect();
        assert_eq!(params, ["patient-1", "Patient/patient-2"]);
    }

    /// A bare logical id is the primary form of a reference search
    /// (`Observation?patient=<id>`), and must match a stored `Patient/<id>`.
    /// Postgres previously compared the raw value only, so a bare id matched
    /// nothing while `patient=Patient/<id>` worked — #490.
    #[test]
    fn reference_bare_id_matches_type_prefixed_reference() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "patient-1")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("bare-id reference");

        let params: Vec<&str> = frag
            .params
            .iter()
            .map(|p| match p {
                SqlParam::Text(t) => t.as_str(),
                other => panic!("expected text params, got {:?}", other),
            })
            .collect();
        assert_eq!(
            params,
            vec!["patient-1"],
            "a bare id must use one equality bind: {}",
            frag.sql
        );
        assert!(
            frag.sql.contains(&format!(
                "value_reference IS NOT NULL AND {REFERENCE_TARGET_ID_EXPR} = $3"
            )),
            "the predicate must match the v42 expression index: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("LIKE"),
            "a bare id must not be turned into a pattern: {}",
            frag.sql
        );
        assert_eq!(
            frag.sql.matches("id IN (SELECT").count(),
            1,
            "still a single sublink: {}",
            frag.sql
        );
    }

    /// `subject:Patient=<id>` resolves the bare id to `Patient/<id>` and matches
    /// it as a type-prefixed reference, mirroring the SQLite handler.
    #[test]
    fn reference_type_modifier_normalizes_bare_id() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Type("Patient".to_string())),
            values: vec![SearchValue::new(SearchPrefix::Eq, "patient-1")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect(":Type reference");

        let params: Vec<&str> = frag
            .params
            .iter()
            .map(|p| match p {
                SqlParam::Text(t) => t.as_str(),
                other => panic!("expected text params, got {:?}", other),
            })
            .collect();
        assert_eq!(
            params,
            vec!["Patient/patient-1"],
            "the type modifier pins the reference to one equality: {}",
            frag.sql
        );
    }

    /// v33's read-side claim, pinned. The `Type/id` form — every reference
    /// search the benchmark sends, and 18% of the search suite's Postgres time —
    /// must emit ONE equality. A disjunction here forces a `BitmapOr` and a
    /// `Bitmap Heap Scan` whose `Filter` re-evaluates the whole predicate on
    /// every matching heap tuple: measured 1.50 ms/call against 0.47 ms.
    #[test]
    fn reference_type_prefixed_emits_one_sargable_equality() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "Patient/patient-1")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("Type/id reference");

        assert!(
            frag.sql.contains("value_reference = $3"),
            "must be an equality on the bound base: {}",
            frag.sql
        );
        assert!(
            !frag.sql.contains("LIKE") && !frag.sql.contains(" OR "),
            "no disjunction: an OR is index-usable only when every arm is, and \
             the `/_history/%` arm was the one that never matched: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 1);
    }

    /// A versioned SEARCH value is still stripped, and now meets a stored value
    /// that was stripped by the writer — so the two ends agree without a
    /// pattern.
    #[test]
    fn reference_type_prefixed_version_is_stripped_from_the_search_value() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "Patient/patient-1/_history/7",
            )],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("versioned Type/id");
        match &frag.params[0] {
            SqlParam::Text(t) => assert_eq!(t, "Patient/patient-1"),
            other => panic!("expected a text param, got {:?}", other),
        }
        assert_eq!(frag.params.len(), 1);
    }

    /// The compartment predicate carried the worst-shaped arm in the backend:
    /// `LIKE $n || '/_history/%'` builds the pattern in SQL, so it is an
    /// `OpExpr` rather than a `Const` and no fixed prefix can be derived from it
    /// under *any* plan — while still costing the whole predicate its index,
    /// because an OR needs every arm indexable.
    #[test]
    fn compartment_membership_is_one_equality_per_param_set() {
        let mut query = SearchQuery::new("Observation");
        query.compartment = Some(CompartmentMembership {
            reference: "Patient/patient-1/_history/3".to_string(),
            params: vec!["subject".to_string(), "performer".to_string()],
        });
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect("compartment");
        assert!(
            !frag.sql.contains("LIKE"),
            "no pattern arm survives: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 1, "one bind, not two: {}", frag.sql);
        match &frag.params[0] {
            SqlParam::Text(t) => assert_eq!(t, "Patient/patient-1"),
            other => panic!("expected a text param, got {:?}", other),
        }
    }

    /// `:identifier` must be driven by the identifier lookup, not by a
    /// correlated `EXISTS` over the reference slice. The old form measured
    /// 285.6 ms / 80,393 buffers against 1.4 ms / 845 on the same replica, and
    /// binding `resource_type` into it — the repair the missing bind invites —
    /// measured 338.2 ms, i.e. slower still.
    #[test]
    fn reference_identifier_is_driven_by_the_identifier_lookup() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Identifier),
            values: vec![SearchValue::new(SearchPrefix::Eq, "http://ex.org/mrn|A1")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2).expect(":identifier");

        assert!(
            !frag.sql.contains("SUBSTRING") && !frag.sql.contains("EXISTS"),
            "the correlated form must be gone: {}",
            frag.sql
        );
        assert!(
            frag.sql.contains("ref.value_reference IN")
                && frag
                    .sql
                    .contains("idx.resource_type || '/' || idx.resource_id"),
            "the sub-select must yield the target's Type/id: {}",
            frag.sql
        );
        // Both levels stay tenant-scoped. Dropping `idx.tenant_id` would let
        // another tenant's identifier select this tenant's rows.
        assert_eq!(
            frag.sql.matches("tenant_id = $1").count(),
            2,
            "both levels scoped: {}",
            frag.sql
        );
        assert_eq!(frag.params.len(), 2);
    }

    /// A versioned search value is stripped before the bare-id suffix match, so
    /// `subject=patient-1/_history/2` still matches a stored `Patient/patient-1`.
    #[test]
    fn reference_bare_id_is_version_stripped() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "patient-1/_history/2")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("versioned bare-id reference");

        match &frag.params[0] {
            SqlParam::Text(t) => assert_eq!(t, "patient-1"),
            other => panic!("expected a text param, got {:?}", other),
        }
        assert_eq!(frag.params.len(), 1, "version-stripped back to a bare id");
        assert!(frag.sql.contains(&format!(
            "value_reference IS NOT NULL AND {REFERENCE_TARGET_ID_EXPR} = $3"
        )));
    }

    /// Bare ids use equality, so SQL pattern metacharacters remain literal.
    #[test]
    fn reference_bare_id_treats_like_metacharacters_literally() {
        let param = SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![
                SearchValue::new(SearchPrefix::Eq, "%"),
                SearchValue::new(SearchPrefix::Eq, "_"),
            ],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Observation").with_parameter(param);
        let frag =
            PostgresQueryBuilder::build_search_query(&query, 2).expect("metacharacter references");

        let params: Vec<&str> = frag
            .params
            .iter()
            .map(|param| match param {
                SqlParam::Text(value) => value.as_str(),
                other => panic!("expected text params, got {other:?}"),
            })
            .collect();
        assert_eq!(params, ["%", "_"]);
        assert!(
            !frag.sql.contains("LIKE") && !frag.sql.contains("ESCAPE"),
            "metacharacters must be compared as data: {}",
            frag.sql
        );
        assert!(
            frag.sql
                .contains(&format!("{REFERENCE_TARGET_ID_EXPR} = $3"))
        );
        assert!(
            frag.sql
                .contains(&format!("{REFERENCE_TARGET_ID_EXPR} = $4"))
        );
    }

    #[test]
    fn reference_bare_id_advances_the_next_parameter_offset_once() {
        let query = SearchQuery::new("Observation")
            .with_parameter(reference_param("subject", None, "patient-1"))
            .with_parameter(token_param("status", None, "final"));
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("reference followed by token");

        assert!(
            frag.sql
                .contains(&format!("{REFERENCE_TARGET_ID_EXPR} = $3"))
        );
        assert!(
            frag.sql.contains("value_token_code = $4"),
            "the later parameter must start after the bare-id bind: {}",
            frag.sql
        );
        let params: Vec<&str> = frag
            .params
            .iter()
            .map(|param| match param {
                SqlParam::Text(value) => value.as_str(),
                other => panic!("expected text params, got {other:?}"),
            })
            .collect();
        assert_eq!(params, ["patient-1", "final"]);
    }

    #[test]
    fn of_type_identifier_sql() {
        let param = SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::OfType),
            values: vec![SearchValue::new(
                SearchPrefix::Eq,
                "http://terminology.hl7.org/CodeSystem/v2-0203|MR|12345",
            )],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Patient").with_parameter(param);
        let frag = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect(":of-type should produce a condition");

        assert!(frag.sql.contains("param_name = 'identifier'"));
        assert!(frag.sql.contains("value_token_code = $3"));
        assert!(frag.sql.contains("value_identifier_type_system = $4"));
        assert!(frag.sql.contains("value_identifier_type_code = $5"));
        assert_eq!(frag.params.len(), 3);
    }

    fn id_param_text(value: &SqlParam) -> &str {
        match value {
            SqlParam::Text(s) => s.as_str(),
            other => panic!("expected a text param, got {:?}", other),
        }
    }

    /// #1092: `_id` is dispatched through `build_id_condition`, bypassing the
    /// generic per-type modifier handling in `build_parameter_condition`
    /// entirely, so `_id:not=a` used to build the exact same `id = $n` as a
    /// bare `_id=a` and return precisely the resource the caller asked to
    /// exclude.
    #[test]
    fn id_no_modifier_control_is_a_positive_match() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        });

        let fragment = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_id must produce a condition");

        assert_eq!(fragment.sql, "id = $3");
        assert_eq!(fragment.params.len(), 1);
        assert_eq!(id_param_text(&fragment.params[0]), "a");
    }

    #[test]
    fn large_id_set_uses_one_array_bind() {
        let values: Vec<SearchValue> = (0..65_536)
            .map(|i| SearchValue::eq(format!("id-{i}")))
            .collect();
        for (modifier, expected_sql) in [
            (None, "id = ANY($3::text[])"),
            (Some(SearchModifier::Not), "id <> ALL($3::text[])"),
        ] {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "_id".to_string(),
                param_type: SearchParamType::Token,
                modifier,
                values: values.clone(),
                ..Default::default()
            });
            let fragment = PostgresQueryBuilder::build_search_query(&query, 2).unwrap();
            assert_eq!(fragment.sql, expected_sql);
            assert!(
                matches!(fragment.params.as_slice(), [SqlParam::TextArray(ids)] if ids.len() == 65_536 && ids[65_535] == "id-65535")
            );
        }
    }

    #[test]
    fn id_not_single_value_excludes_instead_of_matching() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("a")],
            chain: vec![],
            components: vec![],
        });

        let fragment = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_id:not must produce a condition");

        assert_eq!(fragment.sql, "id <> $3");
        assert_eq!(fragment.params.len(), 1);
        assert_eq!(id_param_text(&fragment.params[0]), "a");
    }

    #[test]
    fn id_not_two_values_excludes_both() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("a"), SearchValue::eq("b")],
            chain: vec![],
            components: vec![],
        });

        let fragment = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect("_id:not must produce a condition");

        assert_eq!(fragment.sql, "id NOT IN ($3, $4)");
        assert_eq!(fragment.params.len(), 2);
        assert_eq!(id_param_text(&fragment.params[0]), "a");
        assert_eq!(id_param_text(&fragment.params[1]), "b");
    }

    #[test]
    fn missing_ignores_index_rows_from_contained_resources() {
        let param = SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("false")],
            chain: vec![],
            components: vec![],
        };
        let query = SearchQuery::new("Patient").with_parameter(param);
        let fragment = PostgresQueryBuilder::build_search_query(&query, 2)
            .expect(":missing must produce a condition");

        assert!(
            fragment.sql.contains("is_contained = FALSE"),
            "contained-resource index rows must not establish presence: {}",
            fragment.sql
        );
    }

    #[test]
    fn missing_metadata_uses_authoritative_resource_columns() {
        for (name, param_type, column) in [
            ("_id", SearchParamType::Token, "id"),
            ("_lastUpdated", SearchParamType::Date, "last_updated"),
        ] {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: name.to_string(),
                param_type,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::eq("false")],
                chain: vec![],
                components: vec![],
            });
            let fragment = PostgresQueryBuilder::build_search_query(&query, 2)
                .expect(":missing must produce a condition");

            assert!(fragment.sql.contains("FROM resources"));
            assert!(fragment.sql.contains(&format!("{column} IS NOT NULL")));
            assert!(!fragment.sql.contains("FROM search_index"));
        }
    }

    /// #1380: `family=Zzz,` reached the builder as the values `Zzz` and `""`,
    /// and a prefix match on `""` is every row. The search gate
    /// (`validate_value_presence`) rejects it before a query is built; if one
    /// is built anyway, the parameter matches nothing — the whole parameter, or
    /// `:not` would negate it into everything.
    #[test]
    fn a_parameter_with_an_empty_value_matches_nothing() {
        use SearchModifier as M;
        use SearchParamType as T;
        let cases: Vec<(&str, SearchParamType, Option<SearchModifier>, Vec<&str>)> = vec![
            ("family", T::String, None, vec!["Zzz", ""]),
            ("family", T::String, None, vec![""]),
            ("family", T::String, Some(M::Contains), vec!["", "Zzz"]),
            ("family", T::String, Some(M::Text), vec![" "]),
            ("gender", T::Token, None, vec![""]),
            ("gender", T::Token, Some(M::Not), vec!["female", ""]),
            ("gender", T::Token, Some(M::Text), vec![""]),
            ("identifier", T::Token, Some(M::OfType), vec![""]),
            ("_id", T::Token, None, vec!["a", ""]),
            ("_tag", T::Token, None, vec![""]),
            ("general-practitioner", T::Reference, None, vec![""]),
            ("url", T::Uri, Some(M::Below), vec![""]),
            ("url", T::Uri, Some(M::Contains), vec!["", "x"]),
        ];
        for (name, param_type, modifier, values) in cases {
            let context = format!("{name} {modifier:?} {values:?}");
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: name.to_string(),
                param_type,
                modifier,
                values: values.into_iter().map(SearchValue::eq).collect(),
                chain: vec![],
                components: vec![],
            });
            let fragment = PostgresQueryBuilder::build_search_query(&query, 2)
                .unwrap_or_else(|| panic!("{context}: a dropped condition matches everything"));
            assert_eq!(fragment.sql, "FALSE", "{context}");
            assert!(fragment.params.is_empty(), "{context}");
        }
    }
}
