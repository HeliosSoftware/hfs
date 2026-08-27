//! PostgreSQL search index writer implementation.

use std::sync::LazyLock;

use chrono::{DateTime, Utc};

use crate::backends::postgres::cached::execute_cached;
use crate::backends::postgres::schema::IndexLayout;
use crate::error::{BackendError, StorageResult};
use crate::search::{converters::IndexValue, extractor::ExtractedValue};

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
fn insert_plan(rows: &[IndexRow]) -> InsertPlan {
    let mut plan = InsertPlan {
        columns: Vec::with_capacity(28),
        casts: Vec::with_capacity(28),
        params: Vec::with_capacity(28),
    };
    let p = &mut plan;

    column!(p, rows, "last_updated", "timestamptz[]", |r: &IndexRow| r
        .last_updated);
    column!(p, rows, "param_name", "text[]", |r: &IndexRow| r
        .param_name
        .clone());
    column!(p, rows, "param_url", "text[]", |r: &IndexRow| r
        .param_url
        .clone());
    column!(p, rows, "composite_group", "int4[]", |r: &IndexRow| r
        .composite_group);
    column!(p, rows, "value_string", "text[]", |r: &IndexRow| r
        .value_string
        .clone());
    column!(p, rows, "value_string_folded", "text[]", |r: &IndexRow| r
        .value_string_folded
        .clone());
    column!(p, rows, "value_token_system", "text[]", |r: &IndexRow| r
        .value_token_system
        .clone());
    column!(p, rows, "value_token_code", "text[]", |r: &IndexRow| r
        .value_token_code
        .clone());
    column!(p, rows, "value_token_display", "text[]", |r: &IndexRow| r
        .value_token_display
        .clone());
    column!(p, rows, "value_token_system_2", "text[]", |r: &IndexRow| r
        .value_token_system_2
        .clone());
    column!(p, rows, "value_token_code_2", "text[]", |r: &IndexRow| r
        .value_token_code_2
        .clone());
    column!(p, rows, "value_date", "timestamptz[]", |r: &IndexRow| r
        .value_date);
    column!(p, rows, "value_date_precision", "text[]", |r: &IndexRow| r
        .value_date_precision
        .clone());
    column!(p, rows, "value_number", "float8[]", |r: &IndexRow| r
        .value_number);
    column!(p, rows, "value_number_2", "float8[]", |r: &IndexRow| r
        .value_number_2);
    column!(
        p,
        rows,
        "value_quantity_value",
        "float8[]",
        |r: &IndexRow| r.value_quantity_value
    );
    column!(p, rows, "value_quantity_unit", "text[]", |r: &IndexRow| r
        .value_quantity_unit
        .clone());
    column!(
        p,
        rows,
        "value_quantity_system",
        "text[]",
        |r: &IndexRow| r.value_quantity_system.clone()
    );
    column!(
        p,
        rows,
        "value_quantity_canonical_value",
        "float8[]",
        |r: &IndexRow| r.value_quantity_canonical_value
    );
    column!(
        p,
        rows,
        "value_quantity_canonical_unit",
        "text[]",
        |r: &IndexRow| r.value_quantity_canonical_unit.clone()
    );
    column!(p, rows, "value_reference", "text[]", |r: &IndexRow| r
        .value_reference
        .clone());
    column!(
        p,
        rows,
        "value_reference_display",
        "text[]",
        |r: &IndexRow| r.value_reference_display.clone()
    );
    column!(
        p,
        rows,
        "value_identifier_type_system",
        "text[]",
        |r: &IndexRow| r.value_identifier_type_system.clone()
    );
    column!(
        p,
        rows,
        "value_identifier_type_code",
        "text[]",
        |r: &IndexRow| r.value_identifier_type_code.clone()
    );
    column!(p, rows, "value_uri", "text[]", |r: &IndexRow| r
        .value_uri
        .clone());
    column!(p, rows, "is_contained", "bool[]", |r: &IndexRow| r
        .is_contained);
    column!(p, rows, "contained_type", "text[]", |r: &IndexRow| r
        .contained_type
        .clone());
    column!(p, rows, "contained_local_id", "text[]", |r: &IndexRow| r
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

/// Rows per statement.
///
/// With the `unnest` form this is no longer a bind-parameter limit — 128 rows
/// cost the same 31 parameters as one row does — so it exists only to bound the
/// array a single statement has to marshal. It is well above the 24.2 index rows
/// an average resource produces; what it changes is the tail. `Provenance.target`
/// alone writes 1,626 rows for one resource, which the old 128-row cap split
/// into 13 statements and 13 round trips.
const BATCH_ROWS: usize = 1024;

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
                row.value_reference = Some(reference.clone());
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
        let (plain, composites) = Self::split_for_layout(values, layout);

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
    pub(crate) fn build_contained_rows(
        container: (&str, &str),
        contained: (&str, &str),
        values: &[ExtractedValue],
    ) -> Vec<IndexRow> {
        values
            .iter()
            .filter_map(|value| IndexRow::from_contained(value, container, contained))
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
        for chunk in rows.chunks(BATCH_ROWS) {
            let plan = insert_plan(chunk);
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

            execute_cached(client, INSERT_SQL.as_str(), &param_refs)
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
            let chunk: Vec<IndexRow> = (0..rows)
                .map(|_| row_of(IndexValue::String("x".to_string())))
                .collect();
            let plan = insert_plan(&chunk);
            assert_eq!(plan.params.len(), insert_plan(&[]).params.len());
            assert_eq!(plan.columns, insert_plan(&[]).columns);
        }
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
