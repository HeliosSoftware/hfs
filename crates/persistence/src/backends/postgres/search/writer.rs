//! PostgreSQL search index writer implementation.

use chrono::{DateTime, Utc};

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
    param_url: String,
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
const ROW_COLUMNS: &[&str] = &[
    "tenant_id",
    "resource_type",
    "resource_id",
    "param_name",
    "param_url",
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

/// Rows per `INSERT`. 27 columns x 128 rows = 3,456 bind parameters, well under
/// Postgres' 65535 ceiling, and roughly one statement per imported resource.
const BATCH_ROWS: usize = 128;

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
            param_url: extracted.param_url.to_string(),
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
            param_url: row.param_url.clone(),
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
        values: Vec<ExtractedValue>,
    ) -> StorageResult<usize> {
        let (plain, composites) = super::composite_rows::fold_composites(values);

        let mut rows: Vec<IndexRow> = Vec::with_capacity(plain.len() + composites.len());
        for value in &plain {
            if let Some(row) = IndexRow::from_extracted(value, resource_type, resource_id) {
                rows.push(row);
            }
        }
        for row in &composites {
            rows.push(IndexRow::from_composite(row));
        }

        Self::insert_rows(client, tenant_id, resource_type, resource_id, &rows).await?;
        Ok(rows.len())
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
        rows: &[IndexRow],
    ) -> StorageResult<()> {
        for chunk in rows.chunks(BATCH_ROWS) {
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
                params.push(Box::new(row.param_name.clone()));
                params.push(Box::new(row.param_url.clone()));
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
        extracted: &ExtractedValue,
    ) -> StorageResult<()> {
        let Some(row) = IndexRow::from_extracted(extracted, resource_type, resource_id) else {
            return Ok(());
        };
        Self::insert_rows(client, tenant_id, resource_type, resource_id, &[row]).await
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
}
