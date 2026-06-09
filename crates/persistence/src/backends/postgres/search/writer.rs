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

/// PostgreSQL implementation of SearchIndexWriter.
pub struct PostgresSearchIndexWriter;

impl PostgresSearchIndexWriter {
    /// Writes a single search index entry to PostgreSQL.
    ///
    /// Accepts any type that can be dereferenced to a `tokio_postgres::Client`,
    /// including `deadpool_postgres::Client` and `&deadpool_postgres::Client`.
    pub async fn write_entry(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        extracted: &ExtractedValue,
    ) -> StorageResult<()> {
        match &extracted.value {
            IndexValue::String(s) => {
                let folded = crate::search::fold_text(s);
                client
                    .execute(
                        "INSERT INTO search_index (
                            tenant_id, resource_type, resource_id, param_name, param_url,
                            value_string, composite_group, value_string_folded
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                        &[
                            &tenant_id,
                            &resource_type,
                            &resource_id,
                            &extracted.param_name.as_str(),
                            &extracted.param_url.as_str(),
                            &Some(s.as_str()),
                            &extracted.composite_group.map(|g| g as i32),
                            &folded.as_str(),
                        ],
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!("Failed to insert string search index entry: {}", e))
                    })?;
            }
            IndexValue::Token {
                system,
                code,
                display,
                identifier_type_system,
                identifier_type_code,
            } => {
                client
                    .execute(
                        "INSERT INTO search_index (
                            tenant_id, resource_type, resource_id, param_name, param_url,
                            value_token_system, value_token_code, value_token_display,
                            composite_group, value_identifier_type_system, value_identifier_type_code
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                        &[
                            &tenant_id,
                            &resource_type,
                            &resource_id,
                            &extracted.param_name.as_str(),
                            &extracted.param_url.as_str(),
                            &system.as_deref(),
                            &code.as_str(),
                            &display.as_deref(),
                            &extracted.composite_group.map(|g| g as i32),
                            &identifier_type_system.as_deref(),
                            &identifier_type_code.as_deref(),
                        ],
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!("Failed to insert token search index entry: {}", e))
                    })?;
            }
            IndexValue::Date { value, precision } => {
                let precision_str = precision.to_string();
                let normalized = normalize_date_for_pg(value);
                let timestamp: DateTime<Utc> = DateTime::parse_from_rfc3339(&normalized)
                    .map(|dt| dt.with_timezone(&Utc))
                    .or_else(|_| normalized.parse::<DateTime<Utc>>())
                    .unwrap_or_else(|_| Utc::now());
                client
                    .execute(
                        "INSERT INTO search_index (
                            tenant_id, resource_type, resource_id, param_name, param_url,
                            value_date, value_date_precision, composite_group
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                        &[
                            &tenant_id,
                            &resource_type,
                            &resource_id,
                            &extracted.param_name.as_str(),
                            &extracted.param_url.as_str(),
                            &timestamp,
                            &precision_str.as_str(),
                            &extracted.composite_group.map(|g| g as i32),
                        ],
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!("Failed to insert date search index entry: {}", e))
                    })?;
            }
            IndexValue::Number(n) => {
                client
                    .execute(
                        "INSERT INTO search_index (
                            tenant_id, resource_type, resource_id, param_name, param_url,
                            value_number, composite_group
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                        &[
                            &tenant_id,
                            &resource_type,
                            &resource_id,
                            &extracted.param_name.as_str(),
                            &extracted.param_url.as_str(),
                            n,
                            &extracted.composite_group.map(|g| g as i32),
                        ],
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!("Failed to insert number search index entry: {}", e))
                    })?;
            }
            IndexValue::Quantity {
                value,
                unit,
                system,
                code,
            } => {
                // Canonicalize using the UCUM code (else the unit display) so
                // quantity search can match equivalent units (g ⇄ mg).
                let (canonical_value, canonical_unit) = code
                    .as_deref()
                    .or(unit.as_deref())
                    .and_then(|u| helios_fhirpath::ucum::canonicalize_quantity(*value, u))
                    .map(|(v, u)| (Some(v), Some(u)))
                    .unwrap_or((None, None));
                client
                    .execute(
                        "INSERT INTO search_index (
                            tenant_id, resource_type, resource_id, param_name, param_url,
                            value_quantity_value, value_quantity_unit, value_quantity_system, composite_group,
                            value_quantity_canonical_value, value_quantity_canonical_unit
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                        &[
                            &tenant_id,
                            &resource_type,
                            &resource_id,
                            &extracted.param_name.as_str(),
                            &extracted.param_url.as_str(),
                            value,
                            &unit.as_deref(),
                            &system.as_deref(),
                            &extracted.composite_group.map(|g| g as i32),
                            &canonical_value,
                            &canonical_unit.as_deref(),
                        ],
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!(
                            "Failed to insert quantity search index entry: {}",
                            e
                        ))
                    })?;
            }
            IndexValue::Reference {
                reference,
                resource_type: _,
                resource_id: _,
                display,
            } => {
                client
                    .execute(
                        "INSERT INTO search_index (
                            tenant_id, resource_type, resource_id, param_name, param_url,
                            value_reference, composite_group, value_reference_display
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                        &[
                            &tenant_id,
                            &resource_type,
                            &resource_id,
                            &extracted.param_name.as_str(),
                            &extracted.param_url.as_str(),
                            &reference.as_str(),
                            &extracted.composite_group.map(|g| g as i32),
                            &display.as_deref(),
                        ],
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!(
                            "Failed to insert reference search index entry: {}",
                            e
                        ))
                    })?;
            }
            IndexValue::Uri(uri) => {
                client
                    .execute(
                        "INSERT INTO search_index (
                            tenant_id, resource_type, resource_id, param_name, param_url,
                            value_uri, composite_group
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
                        &[
                            &tenant_id,
                            &resource_type,
                            &resource_id,
                            &extracted.param_name.as_str(),
                            &extracted.param_url.as_str(),
                            &uri.as_str(),
                            &extracted.composite_group.map(|g| g as i32),
                        ],
                    )
                    .await
                    .map_err(|e| {
                        internal_error(format!("Failed to insert URI search index entry: {}", e))
                    })?;
            }
        }

        Ok(())
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
                let normalized = normalize_date_for_pg(value);
                value_date = Some(
                    DateTime::parse_from_rfc3339(&normalized)
                        .map(|dt| dt.with_timezone(&Utc))
                        .or_else(|_| normalized.parse::<DateTime<Utc>>())
                        .unwrap_or_else(|_| Utc::now()),
                );
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
fn normalize_date_for_pg(value: &str) -> String {
    if value.contains('T') {
        // Already has time component - ensure timezone
        if value.contains('+') || value.contains('Z') || value.ends_with("-00:00") {
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
