//! PostgreSQL search index writer implementation.

use chrono::{DateTime, Utc};

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

/// One `search_index` row, flattened to every column the non-contained write
/// paths can set.
///
/// The per-value `write_entry` path issues one `INSERT` per extracted value,
/// which costs a network round trip per row. A Synthea import writes ~113 index
/// rows per resource, so a 1,000-bundle import spends most of its wall clock in
/// round trips rather than in Postgres. Flattening every value to a common row
/// shape lets `write_values` send them as multi-row `INSERT`s instead.
///
/// Columns not listed here (`is_contained`, `contained_type`,
/// `contained_local_id`) are left to their table defaults, exactly as the
/// per-value inserts left them.
#[derive(Default)]
struct IndexRow {
    param_name: String,
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
}

/// Column list for the batched insert, in bind order. `COLUMNS.len() + 3`
/// (tenant/type/resource id are bound once per row too) must stay under
/// Postgres' 65535-parameter ceiling for `BATCH_ROWS` rows — see `BATCH_ROWS`.
///
/// `param_url` is deliberately absent. The column still exists and existing rows
/// keep their values, but nothing in the workspace ever reads it — there is no
/// SELECT, WHERE, or join on it in any backend, and `ReindexOptions`'
/// `search_param_urls` (the one plausible consumer) has no consumers of its own.
/// It held the SearchParameter's canonical URL, ~50-60 bytes, on every one of
/// ~60M index rows: roughly 3 GB of an 8.5 GB heap, plus the WAL to write it,
/// on an 11 GB Docker host. Left unbound it is NULL, which costs one bit in the
/// null bitmap.
///
/// The contained-resource path below still binds it. That path inserts one row
/// per value for `contained[]` entries only — under 5 MB of index across the
/// whole corpus — so it is left alone rather than churned for no measurable
/// gain.
///
/// Not dropped from the table: that DDL would run against real databases and
/// cannot be undone, and it buys nothing over simply not writing the column.
const ROW_COLUMNS: &[&str] = &[
    "tenant_id",
    "resource_type",
    "resource_id",
    "last_updated",
    "param_name",
    "composite_group",
    "value_string",
    "value_string_folded",
    "value_token_system",
    "value_token_code",
    "value_token_display",
    "value_token_system_2",
    "value_token_code_2",
    "value_date",
    "value_date_precision",
    "value_number",
    "value_number_2",
    "value_quantity_value",
    "value_quantity_unit",
    "value_quantity_system",
    "value_quantity_canonical_value",
    "value_quantity_canonical_unit",
    "value_reference",
    "value_reference_display",
    "value_identifier_type_system",
    "value_identifier_type_code",
    "value_uri",
];

/// Rows per `INSERT`. 28 columns x 128 rows = 3,584 bind parameters, well under
/// Postgres' 65535 ceiling, and roughly one statement per imported resource.
const BATCH_ROWS: usize = 128;

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
    ) -> Option<Self> {
        let mut row = IndexRow {
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
                row.value_date_precision = Some(precision.to_string());
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

    /// Flattens one denormalized composite row (#279).
    ///
    /// Deliberately does not populate `value_string_folded`, `value_token_display`,
    /// the identifier-type columns or the canonical quantity columns: the
    /// composite insert never set them either, and a composite search reads none
    /// of them.
    fn from_composite(row: &super::composite_rows::CompositeRow) -> Self {
        IndexRow {
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
        let (plain, composites) =
            Self::split_for_layout(Self::drop_resources_backed(values), layout);

        let mut rows: Vec<IndexRow> = Vec::with_capacity(plain.len() + composites.len());
        for value in &plain {
            if let Some(row) = IndexRow::from_extracted(value, resource_type, resource_id) {
                rows.push(row);
            }
        }
        for row in &composites {
            rows.push(IndexRow::from_composite(row));
        }

        Self::insert_rows(
            client,
            tenant_id,
            resource_type,
            resource_id,
            last_updated,
            &rows,
        )
        .await?;
        Ok(rows.len())
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

    /// Builds one multi-row `INSERT` and its bind parameters.
    ///
    /// Split out so the correspondence between [`ROW_COLUMNS`] and the push
    /// order below is assertable without a database: a column added to one and
    /// not the other shifts every later value into the wrong column, which
    /// Postgres accepts silently wherever the types happen to line up.
    fn build_insert(
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        last_updated: DateTime<Utc>,
        chunk: &[IndexRow],
    ) -> (
        String,
        Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
    ) {
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            Vec::with_capacity(chunk.len() * ROW_COLUMNS.len());
        let mut tuples: Vec<String> = Vec::with_capacity(chunk.len());

        for row in chunk {
            let base = params.len();
            let placeholders: Vec<String> = (1..=ROW_COLUMNS.len())
                .map(|i| format!("${}", base + i))
                .collect();
            tuples.push(format!("({})", placeholders.join(", ")));

            // Push order must match ROW_COLUMNS exactly.
            params.push(Box::new(tenant_id.to_string()));
            params.push(Box::new(resource_type.to_string()));
            params.push(Box::new(resource_id.to_string()));
            params.push(Box::new(last_updated));
            params.push(Box::new(row.param_name.clone()));
            params.push(Box::new(row.composite_group));
            params.push(Box::new(row.value_string.clone()));
            params.push(Box::new(row.value_string_folded.clone()));
            params.push(Box::new(row.value_token_system.clone()));
            params.push(Box::new(row.value_token_code.clone()));
            params.push(Box::new(row.value_token_display.clone()));
            params.push(Box::new(row.value_token_system_2.clone()));
            params.push(Box::new(row.value_token_code_2.clone()));
            params.push(Box::new(row.value_date));
            params.push(Box::new(row.value_date_precision.clone()));
            params.push(Box::new(row.value_number));
            params.push(Box::new(row.value_number_2));
            params.push(Box::new(row.value_quantity_value));
            params.push(Box::new(row.value_quantity_unit.clone()));
            params.push(Box::new(row.value_quantity_system.clone()));
            params.push(Box::new(row.value_quantity_canonical_value));
            params.push(Box::new(row.value_quantity_canonical_unit.clone()));
            params.push(Box::new(row.value_reference.clone()));
            params.push(Box::new(row.value_reference_display.clone()));
            params.push(Box::new(row.value_identifier_type_system.clone()));
            params.push(Box::new(row.value_identifier_type_code.clone()));
            params.push(Box::new(row.value_uri.clone()));
        }

        let sql = format!(
            "INSERT INTO search_index ({}) VALUES {}",
            ROW_COLUMNS.join(", "),
            tuples.join(", ")
        );
        (sql, params)
    }

    /// Sends flattened rows as multi-row `INSERT`s of at most [`BATCH_ROWS`].
    ///
    /// One statement per 128 rows instead of one per row. Postgres commonly runs
    /// on a different host from the server, so the per-row form paid a network
    /// round trip for every extracted value; this is what made bulk import
    /// round-trip-bound rather than disk-bound.
    async fn insert_rows(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        last_updated: DateTime<Utc>,
        rows: &[IndexRow],
    ) -> StorageResult<()> {
        for chunk in rows.chunks(BATCH_ROWS) {
            let (sql, params) =
                Self::build_insert(tenant_id, resource_type, resource_id, last_updated, chunk);
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                .iter()
                .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            client
                .execute(sql.as_str(), &param_refs)
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
        let Some(row) = IndexRow::from_extracted(extracted, resource_type, resource_id) else {
            return Ok(());
        };
        Self::insert_rows(
            client,
            tenant_id,
            resource_type,
            resource_id,
            last_updated,
            &[row],
        )
        .await
    }

    /// Writes a single contained `ExtractedValue` for `_contained` search. The
    /// row's `resource_type` / `resource_id` identify the container; the entry is
    /// flagged `is_contained = TRUE` and carries the contained resource's type
    /// and local id. Uses one full-column INSERT (rather than the per-type
    /// inserts above) so all value variants share a single statement.
    pub async fn write_contained_entry(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        container: (&str, &str),
        contained: (&str, &str),
        extracted: &ExtractedValue,
    ) -> StorageResult<()> {
        let (container_type, container_id) = container;
        let (contained_type, contained_local_id) = contained;

        let mut value_string: Option<String> = None;
        let mut value_string_folded: Option<String> = None;
        let mut token_system: Option<String> = None;
        let mut token_code: Option<String> = None;
        let mut token_display: Option<String> = None;
        let mut value_date: Option<DateTime<Utc>> = None;
        let mut date_precision: Option<String> = None;
        let mut value_number: Option<f64> = None;
        let mut q_value: Option<f64> = None;
        let mut q_unit: Option<String> = None;
        let mut q_system: Option<String> = None;
        let mut q_canonical_value: Option<f64> = None;
        let mut q_canonical_unit: Option<String> = None;
        let mut value_reference: Option<String> = None;
        let mut reference_display: Option<String> = None;
        let mut value_uri: Option<String> = None;
        let mut id_type_system: Option<String> = None;
        let mut id_type_code: Option<String> = None;
        let composite_group = extracted.composite_group.map(|g| g as i32);

        match &extracted.value {
            IndexValue::String(s) => {
                value_string = Some(s.clone());
                value_string_folded = Some(crate::search::fold_text(s));
            }
            IndexValue::Token {
                system,
                code,
                display,
                identifier_type_system,
                identifier_type_code,
            } => {
                token_system = system.clone();
                token_code = Some(code.clone());
                token_display = display.clone();
                id_type_system = identifier_type_system.clone();
                id_type_code = identifier_type_code.clone();
            }
            IndexValue::Date { value, precision } => {
                date_precision = Some(precision.to_string());
                let Some(timestamp) = parse_index_date(value) else {
                    tracing::warn!(
                        param_name = %extracted.param_name,
                        container_type = %container_type,
                        container_id = %container_id,
                        contained_type = %contained_type,
                        value = %value,
                        "skipping contained date search index entry: unparseable date value"
                    );
                    return Ok(());
                };
                value_date = Some(timestamp);
            }
            IndexValue::Number(n) => value_number = Some(*n),
            IndexValue::Quantity {
                value,
                unit,
                system,
                code,
            } => {
                q_value = Some(*value);
                q_unit = unit.clone();
                q_system = system.clone();
                if let Some((cv, cu)) = code
                    .as_deref()
                    .or(unit.as_deref())
                    .and_then(|u| helios_fhirpath::ucum::canonicalize_quantity(*value, u))
                {
                    q_canonical_value = Some(cv);
                    q_canonical_unit = Some(cu);
                }
            }
            IndexValue::Reference {
                reference, display, ..
            } => {
                value_reference = Some(reference.clone());
                reference_display = display.clone();
            }
            IndexValue::Uri(uri) => value_uri = Some(uri.clone()),
        }

        let is_contained = true;
        let contained_type = contained_type.to_string();
        let contained_local_id = contained_local_id.to_string();

        client
            .execute(
                "INSERT INTO search_index (
                    tenant_id, resource_type, resource_id, param_name, param_url,
                    value_string, value_token_system, value_token_code, value_token_display,
                    value_date, value_date_precision, value_number,
                    value_quantity_value, value_quantity_unit, value_quantity_system,
                    value_reference, value_uri, composite_group,
                    value_identifier_type_system, value_identifier_type_code, value_reference_display,
                    value_quantity_canonical_value, value_quantity_canonical_unit, value_string_folded,
                    is_contained, contained_type, contained_local_id
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                    $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27
                )",
                &[
                    &tenant_id,
                    &container_type,
                    &container_id,
                    &extracted.param_name.as_str(),
                    &extracted.param_url.as_str(),
                    &value_string,
                    &token_system,
                    &token_code,
                    &token_display,
                    &value_date,
                    &date_precision,
                    &value_number,
                    &q_value,
                    &q_unit,
                    &q_system,
                    &value_reference,
                    &value_uri,
                    &composite_group,
                    &id_type_system,
                    &id_type_code,
                    &reference_display,
                    &q_canonical_value,
                    &q_canonical_unit,
                    &value_string_folded,
                    &is_contained,
                    &contained_type,
                    &contained_local_id,
                ],
            )
            .await
            .map_err(|e| {
                internal_error(format!(
                    "Failed to insert contained search index entry: {}",
                    e
                ))
            })?;

        Ok(())
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
        IndexRow::from_extracted(&extracted(value), "Observation", "abc")
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

    /// The push order in `build_insert` and `ROW_COLUMNS` are maintained by
    /// hand. If they drift, every value after the divergence lands in the wrong
    /// column — and Postgres accepts that silently wherever the types line up,
    /// so the index is corrupted rather than the write rejected.
    #[test]
    fn bind_count_matches_the_column_list() {
        let now = Utc::now();
        for rows in [1usize, 3, BATCH_ROWS] {
            let chunk: Vec<IndexRow> = (0..rows)
                .map(|_| row_of(IndexValue::String("x".to_string())))
                .collect();
            let (sql, params) =
                PostgresSearchIndexWriter::build_insert("t", "Observation", "abc", now, &chunk);
            assert_eq!(
                params.len(),
                rows * ROW_COLUMNS.len(),
                "bind count must equal columns x rows"
            );
            assert_eq!(
                sql.matches('$').count(),
                rows * ROW_COLUMNS.len(),
                "every bind needs a placeholder"
            );
            // Placeholders must run 1..=n with no gap or repeat.
            assert!(sql.contains(&format!("${}", rows * ROW_COLUMNS.len())));
            assert!(!sql.contains(&format!("${}", rows * ROW_COLUMNS.len() + 1)));
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

    /// An unparseable date skips its row rather than being stored at ingestion
    /// time, which would make `date=gt<any past date>` match it (#494).
    #[test]
    fn an_unparseable_date_yields_no_row() {
        let bad = IndexValue::Date {
            value: "not-a-date".to_string(),
            precision: DatePrecision::Day,
        };
        assert!(IndexRow::from_extracted(&extracted(bad), "Observation", "abc").is_none());
    }
}
