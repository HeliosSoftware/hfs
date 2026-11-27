//! Running a SQL Query Library inside the primary PostgreSQL database (#2902).
//!
//! The statement comes from [`super::native_query::compose`]. It runs in a
//! `READ ONLY` transaction with `statement_timeout` set to the query's time
//! limit, the layer below the SQL check of `helios_sof::sqlquery::native`:
//! whatever the statement contains, PostgreSQL refuses a write and stops it
//! at the limit. At most `max_rows` rows are fetched, through a portal, so a
//! larger result is not sent by the server.

use std::error::Error;
use std::time::Duration;

use base64::Engine as _;
use deadpool_postgres::Pool;
use helios_fhir::FhirVersion;
use helios_sof::sqlquery::{ColumnFhirType, QueryResult, SqlQueryError, SqlValue};
use serde_json::Value;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::{FromSql, ToSql, Type};
use tracing::{debug, trace};

use crate::core::sof_runner::NativeSqlQuery;
use crate::tenant::TenantContext;

use super::compiler::SqlDialect;
use super::decode::{ColumnDecode, decode_text};
use super::native_query::{NativeParam, compose};
use super::postgres::{PgParam, box_params, load_group_documents};

/// Runs `query` for `tenant` as one read-only statement.
pub(super) async fn run_sql_query(
    pool: &Pool,
    fhir_version: FhirVersion,
    tenant: &TenantContext,
    query: NativeSqlQuery,
) -> Result<QueryResult, SqlQueryError> {
    let tenant_id = tenant.tenant_id().to_string();
    let stored_version = query.filters.fhir_version.map(|v| v.as_mime_param());
    let group_tenant = tenant_id.as_str();
    let composed = compose(
        SqlDialect::Postgres,
        fhir_version,
        &tenant_id,
        &query,
        |refs| async move { load_group_documents(pool, group_tenant, stored_version, &refs).await },
    )
    .await?;
    debug!(runner = "postgres-indb", tenant = %tenant_id, relations = query.relations.len(), "running SQL query in the primary database");
    trace!(runner = "postgres-indb", sql = %composed.sql, "composed SQL query");

    let secs = query.timeout.as_secs();
    let mut client = pool.get().await.map_err(|e| {
        SqlQueryError::SourceUnavailable(format!("failed to acquire Postgres connection: {e}"))
    })?;
    let tx = client
        .build_transaction()
        .read_only(true)
        .start()
        .await
        .map_err(|e| query_error(&e, secs))?;
    let result = execute(
        &tx,
        &composed.sql,
        composed.params,
        query.max_rows,
        query.timeout,
    )
    .await;
    let _ = tx.rollback().await;
    result.map_err(|e| match e {
        Failure::Postgres(e) => query_error(&e, secs),
        Failure::Query(e) => e,
    })
}

enum Failure {
    Postgres(tokio_postgres::Error),
    Query(SqlQueryError),
}

impl From<tokio_postgres::Error> for Failure {
    fn from(e: tokio_postgres::Error) -> Self {
        Failure::Postgres(e)
    }
}

async fn execute(
    tx: &tokio_postgres::Transaction<'_>,
    sql: &str,
    params: Vec<NativeParam>,
    max_rows: usize,
    timeout: Duration,
) -> Result<QueryResult, Failure> {
    // At least 1 ms: `0` would lift the limit.
    let ms = u64::try_from(timeout.as_millis())
        .unwrap_or(u64::MAX)
        .max(1);
    tx.batch_execute(&format!("SET LOCAL statement_timeout = {ms}"))
        .await?;

    let statement = tx.prepare(sql).await?;
    let mut columns = Vec::with_capacity(statement.columns().len());
    let mut column_types = Vec::with_capacity(statement.columns().len());
    for column in statement.columns() {
        let Some(fhir_type) = fhir_type_of(column.type_()) else {
            return Err(Failure::Query(SqlQueryError::Unsupported(format!(
                "result column '{}' has PostgreSQL type '{}', which a SQL Query cannot return; \
                 cast it in the query, for example to text or with to_jsonb()",
                column.name(),
                column.type_()
            ))));
        };
        columns.push(column.name().to_string());
        column_types.push(fhir_type);
    }

    let boxed = bind(params);
    let refs: Vec<&(dyn ToSql + Sync)> = boxed
        .iter()
        .map(|p| p.as_ref() as &(dyn ToSql + Sync))
        .collect();
    let portal = tx.bind(&statement, &refs).await?;
    let fetch = i32::try_from(max_rows).unwrap_or(i32::MAX).max(1);
    let fetched = tx.query_portal(&portal, fetch).await?;

    let mut rows = Vec::with_capacity(fetched.len().min(max_rows));
    for row in fetched.iter().take(max_rows) {
        let mut values = Vec::with_capacity(columns.len());
        for (index, name) in columns.iter().enumerate() {
            let cell: Cell = row.try_get(index).map_err(|e| {
                Failure::Query(SqlQueryError::Internal(format!(
                    "failed to read result column '{name}': {e}"
                )))
            })?;
            values.push(cell.0);
        }
        rows.push(values);
    }
    Ok(QueryResult {
        columns,
        column_types,
        rows,
    })
}

/// Every slot as the driver sends it. A view's slots keep the encoding the
/// view runner uses. A `Library.parameter` value and a relation's rows go as
/// text: the statement casts each to the type it needs, so the type
/// PostgreSQL infers for the slot is always `text`.
fn bind(params: Vec<NativeParam>) -> Vec<Box<dyn ToSql + Sync + Send>> {
    let mut boxed: Vec<Box<dyn ToSql + Sync + Send>> = Vec::with_capacity(params.len());
    for param in params {
        match param {
            NativeParam::Runtime(value) => {
                boxed.extend(box_params(vec![PgParam::from_runtime(value)]))
            }
            NativeParam::User(value) => boxed.push(match value {
                SqlValue::Null => Box::new(None::<String>),
                SqlValue::Integer(i) => Box::new(i.to_string()),
                SqlValue::Real(f) => Box::new(f.to_string()),
                SqlValue::Text(s) => Box::new(s),
                SqlValue::Blob(b) => Box::new(base64::engine::general_purpose::STANDARD.encode(b)),
            }),
            NativeParam::Json(text) => boxed.push(Box::new(text)),
        }
    }
    boxed
}

/// The FHIR type a result column of PostgreSQL type `ty` is reported as, or
/// `None` when [`Cell`] cannot read the type.
fn fhir_type_of(ty: &Type) -> Option<ColumnFhirType> {
    Some(match *ty {
        Type::BOOL => ColumnFhirType::Boolean,
        // Every integer is `integer`; the FHIR formatter widens a value past
        // 32 bits to integer64 itself, as it does for the embedded engine.
        Type::INT2 | Type::INT4 | Type::INT8 | Type::OID => ColumnFhirType::Integer,
        Type::FLOAT4 | Type::FLOAT8 | Type::NUMERIC => ColumnFhirType::Decimal,
        Type::DATE => ColumnFhirType::Date,
        Type::TIMESTAMPTZ => ColumnFhirType::Instant,
        Type::TIME => ColumnFhirType::Time,
        Type::BYTEA => ColumnFhirType::Base64Binary,
        Type::TEXT
        | Type::VARCHAR
        | Type::BPCHAR
        | Type::NAME
        | Type::UNKNOWN
        | Type::TIMESTAMP
        | Type::JSON
        | Type::JSONB
        | Type::UUID => ColumnFhirType::String("string".to_string()),
        _ => return None,
    })
}

/// One result cell as JSON; `None` is SQL NULL.
struct Cell(Option<Value>);

type BoxError = Box<dyn Error + Sync + Send>;

impl<'a> FromSql<'a> for Cell {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, BoxError> {
        let value = match *ty {
            Type::BOOL => Value::Bool(bool::from_sql(ty, raw)?),
            Type::INT2 => Value::from(i16::from_sql(ty, raw)?),
            Type::INT4 => Value::from(i32::from_sql(ty, raw)?),
            Type::INT8 => Value::from(i64::from_sql(ty, raw)?),
            Type::OID => Value::from(u32::from_sql(ty, raw)?),
            Type::FLOAT4 => float(f64::from(f32::from_sql(ty, raw)?)),
            Type::FLOAT8 => float(f64::from_sql(ty, raw)?),
            Type::NUMERIC => decode_text(ColumnDecode::Decimal, numeric_text(raw)?),
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
                Value::String(String::from_utf8(raw.to_vec())?)
            }
            Type::DATE => Value::String(chrono::NaiveDate::from_sql(ty, raw)?.to_string()),
            Type::TIME => Value::String(chrono::NaiveTime::from_sql(ty, raw)?.to_string()),
            Type::TIMESTAMP => Value::String(
                chrono::NaiveDateTime::from_sql(ty, raw)?
                    .format("%Y-%m-%dT%H:%M:%S%.f")
                    .to_string(),
            ),
            Type::TIMESTAMPTZ => Value::String(
                chrono::DateTime::<chrono::Utc>::from_sql(ty, raw)?
                    .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            ),
            Type::BYTEA => Value::String(base64::engine::general_purpose::STANDARD.encode(raw)),
            Type::UUID => Value::String(uuid::Uuid::from_sql(ty, raw)?.to_string()),
            // A JSON scalar is its value; an array or object is JSON text,
            // as the embedded engine returns one.
            Type::JSON | Type::JSONB => match Value::from_sql(ty, raw)? {
                Value::Null => return Ok(Cell(None)),
                composite @ (Value::Array(_) | Value::Object(_)) => {
                    Value::String(composite.to_string())
                }
                scalar => scalar,
            },
            _ => return Err(format!("unsupported PostgreSQL type '{ty}'").into()),
        };
        Ok(Cell(Some(value)))
    }

    fn from_sql_null(_ty: &Type) -> Result<Self, BoxError> {
        Ok(Cell(None))
    }

    fn accepts(ty: &Type) -> bool {
        fhir_type_of(ty).is_some()
    }
}

/// A float as a JSON number; `NaN` and the infinities, which JSON has no
/// number for, as their names.
fn float(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(value.to_string()))
}

/// The decimal text of a `numeric` in PostgreSQL's binary form: a header of
/// four 16-bit fields (digit count, weight of the first digit, sign, display
/// scale) followed by base-10000 digits.
fn numeric_text(raw: &[u8]) -> Result<String, BoxError> {
    let field = |index: usize| -> Result<i16, BoxError> {
        let bytes = raw
            .get(index * 2..index * 2 + 2)
            .ok_or("numeric value is truncated")?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    };
    let count = usize::try_from(field(0)?)?;
    let weight = i32::from(field(1)?);
    let sign = field(2)? as u16;
    let scale = usize::try_from(field(3)?)?;
    match sign {
        0xC000 => return Ok("NaN".to_string()),
        0xD000 => return Ok("Infinity".to_string()),
        0xF000 => return Ok("-Infinity".to_string()),
        _ => {}
    }
    let mut digits = Vec::with_capacity(count);
    for index in 0..count {
        digits.push(field(4 + index)?);
    }
    let digit = |position: i32| -> i16 {
        usize::try_from(position)
            .ok()
            .and_then(|p| digits.get(p).copied())
            .unwrap_or(0)
    };
    let mut text = String::new();
    if sign == 0x4000 {
        text.push('-');
    }
    if weight < 0 {
        text.push('0');
    } else {
        for position in 0..=weight {
            let group = digit(position);
            if position == 0 {
                text.push_str(&group.to_string());
            } else {
                text.push_str(&format!("{group:04}"));
            }
        }
    }
    if scale > 0 {
        text.push('.');
        let mut fraction = String::new();
        let mut position = weight + 1;
        while fraction.len() < scale {
            fraction.push_str(&format!("{:04}", digit(position)));
            position += 1;
        }
        fraction.truncate(scale);
        text.push_str(&fraction);
    }
    Ok(text)
}

/// How a PostgreSQL failure of the statement is reported.
///
/// The statement is mostly the caller's SQL, so an error PostgreSQL raises
/// about the SQL or its data (syntax, an unknown column, a failed cast,
/// division by zero) is the caller's to fix and carries PostgreSQL's message.
fn query_error(e: &tokio_postgres::Error, secs: u64) -> SqlQueryError {
    let Some(db) = e.as_db_error() else {
        return SqlQueryError::SourceUnavailable(e.to_string());
    };
    let code = db.code();
    if *code == SqlState::QUERY_CANCELED {
        return SqlQueryError::Timeout { secs };
    }
    if *code == SqlState::READ_ONLY_SQL_TRANSACTION {
        return SqlQueryError::NotSelect(format!(
            "only SELECT queries are allowed: {}",
            db.message()
        ));
    }
    let text = format!(
        "PostgreSQL error: {} (SQLSTATE {})",
        db.message(),
        code.code()
    );
    match &code.code()[..2] {
        // Syntax and access-rule errors, data exceptions, cardinality
        // violations, unsupported features, program limits.
        "42" | "22" | "21" | "0A" | "54" | "2F" => SqlQueryError::Database(text),
        // Connection failures, resource shortage, shutdown.
        "08" | "53" | "57" => SqlQueryError::SourceUnavailable(text),
        _ => SqlQueryError::Internal(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numeric(count_weight_sign_scale: [i16; 4], digits: &[i16]) -> Vec<u8> {
        let mut raw = Vec::new();
        for field in count_weight_sign_scale.iter().chain(digits) {
            raw.extend_from_slice(&field.to_be_bytes());
        }
        raw
    }

    #[test]
    fn numeric_binary_form_decodes_to_decimal_text() {
        for (header, digits, expected) in [
            ([0, 0, 0, 0], vec![], "0"),
            ([1, 0, 0, 0], vec![42], "42"),
            ([2, 1, 0, 0], vec![1, 2345], "12345"),
            ([2, 0, 0, 2], vec![170, 5000], "170.50"),
            ([1, -1, 0, 3], vec![1200], "0.120"),
            ([1, -2, 0, 6], vec![1200], "0.000012"),
            ([1, 0, 0x4000, 1], vec![3], "-3.0"),
            ([1, 1, 0, 0], vec![7], "70000"),
            ([0, 0, 0, 2], vec![], "0.00"),
        ] {
            assert_eq!(numeric_text(&numeric(header, &digits)).unwrap(), expected);
        }
        assert_eq!(
            numeric_text(&numeric([0, 0, 0xC000u16 as i16, 0], &[])).unwrap(),
            "NaN"
        );
        assert!(numeric_text(&[0, 1]).is_err());
    }
}
