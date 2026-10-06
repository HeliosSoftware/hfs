//! SQLite in-DB SQL-on-FHIR runner.
//!
//! [`SqliteInDbRunner`] compiles a ViewDefinition to a parameterised SQLite
//! `SELECT` statement and executes it directly against the `resources` table,
//! bypassing in-process FHIRPath evaluation entirely.
//!
//! ## Streaming
//!
//! Rows are sent one-by-one through a bounded `tokio::sync::mpsc` channel
//! (buffer: 256) so the HTTP layer can begin flushing to the client before the
//! full result set is read.  The blocking SQLite iteration runs in a dedicated
//! `spawn_blocking` thread so it never stalls the async runtime. Its
//! `JoinHandle` is watched by [`watch_row_producer`](crate::core::sof_runner::watch_row_producer)
//! so a panic inside the blocking thread reaches the consumer as an `Err`
//! item instead of a silent end of stream.

use helios_fhir::FhirVersion;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, trace};

use crate::core::sof_runner::{
    RowStream, SofError, SofRunner, ViewFilters, ViewRow, watch_row_producer,
};
use crate::tenant::TenantContext;

use super::compiler::{SqlDialect, attach_runtime_conditions, compile_view_definition_dialect};
use super::decode::{ColumnDecode, decode_text};

/// Channel buffer depth (rows that can be queued ahead of the consumer).
const CHANNEL_BUFFER: usize = 256;

/// SQL-on-FHIR runner that compiles ViewDefinitions to SQLite SQL.
pub struct SqliteInDbRunner {
    pool: Pool<SqliteConnectionManager>,
    fhir_version: FhirVersion,
}

impl SqliteInDbRunner {
    /// Creates a new runner backed by the given connection pool. Uses the
    /// default FHIR version (R4) for compile-time cardinality lookups; call
    /// [`Self::with_fhir_version`] to override.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self {
            pool,
            fhir_version: FhirVersion::default_enabled(),
        }
    }

    /// Returns a runner that consults the given FHIR version's field-type
    /// table when validating `collection: false` columns.
    pub fn with_fhir_version(mut self, version: FhirVersion) -> Self {
        self.fhir_version = version;
        self
    }
}

#[async_trait::async_trait]
impl SofRunner for SqliteInDbRunner {
    fn runner_name(&self) -> &'static str {
        "sqlite-indb"
    }

    async fn run_view(
        &self,
        tenant: &TenantContext,
        view_definition: Value,
        mut filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        // Compile synchronously (cheap, no I/O)
        let compiled = compile_view_definition_dialect(
            &view_definition,
            SqlDialect::Sqlite,
            self.fhir_version,
        )?;

        debug!(
            runner = "sqlite-indb",
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );
        trace!(
            runner = "sqlite-indb",
            sql = %compiled.sql,
            columns = ?compiled.columns,
            constants = compiled.constants.len(),
            "compiled ViewDefinition SQL"
        );

        let tenant_id = tenant.tenant_id().to_string();
        let resource_type = view_definition
            .get("resource")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Spec-correct `group` handling: resolve each Group/{id} to its
        // `member.entity` Patient references and fold them into the patient
        // filter, mirroring the inline path's behavior. Group resolution
        // is an extra DB read per group ref; once done we clear the
        // group_refs so build_sqlite_sql doesn't double-apply.
        if !filters.group.is_empty() {
            let resolved =
                resolve_group_refs_to_patient_refs(&self.pool, &tenant_id, &filters.group)?;
            // A group that resolves to no Patient members (absent, empty, or
            // listing only other types) selects nothing (#1701). Without this
            // the merged patient list is empty and the query would run unfiltered.
            if resolved.is_empty() && filters.patient.is_empty() {
                return Ok(Box::pin(futures::stream::empty()));
            }
            for p in resolved {
                if !filters.patient.iter().any(|existing| existing == &p) {
                    filters.patient.push(p);
                }
            }
            filters.group.clear();
        }

        let limit = filters.limit;
        let columns = compiled.columns.clone();
        let decodes = compiled.column_decodes.clone();
        let pool = self.pool.clone();

        // Inject runtime filter conditions (since, patient/group). The
        // compiled query already reserves `?3..?N` for ViewDefinition
        // constants; runtime filters allocate from the next free slot.
        let (sql, extra_params) = build_sqlite_sql(
            &compiled.sql,
            &compiled.constants,
            &filters,
            self.fhir_version,
            &resource_type,
        )?;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);
        let guard_tx = tx.clone();

        let producer = tokio::task::spawn_blocking(move || {
            stream_sqlite_rows(
                &pool,
                &sql,
                &tenant_id,
                &resource_type,
                extra_params,
                &columns,
                &decodes,
                limit,
                tx,
            );
        });
        watch_row_producer(self.runner_name(), guard_tx, producer);

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

/// Loads each `Group/{id}` from the `resources` table and extracts its
/// `member.entity` Patient references via the shared
/// [`helios_sof::resolve_group_members_to_patient_refs`]. Returns the
/// union of those Patient refs across all supplied group refs. Unknown
/// groups contribute no patients, and a run whose groups resolve to none
/// selects nothing (see `run_view`).
fn resolve_group_refs_to_patient_refs(
    pool: &Pool<SqliteConnectionManager>,
    tenant_id: &str,
    group_refs: &[String],
) -> Result<Vec<String>, SofError> {
    if group_refs.is_empty() {
        return Ok(Vec::new());
    }
    let conn = pool
        .get()
        .map_err(|e| SofError::Storage(format!("failed to get sqlite connection: {e}")))?;
    let mut stmt = conn
        .prepare(
            "SELECT data FROM resources \
             WHERE tenant_id = ?1 \
               AND resource_type = 'Group' \
               AND id = ?2 \
               AND is_deleted = 0",
        )
        .map_err(|e| SofError::Storage(format!("prepare failed: {e}")))?;

    let mut groups = Vec::with_capacity(group_refs.len());
    for r in group_refs {
        let id = r.strip_prefix("Group/").unwrap_or(r);
        let res: rusqlite::Result<Vec<u8>> = stmt.query_row([tenant_id, id], |row| row.get(0));
        match res {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => groups.push(v),
                Err(_) => continue,
            },
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => {
                return Err(SofError::Storage(format!(
                    "group lookup failed for {r}: {e}"
                )));
            }
        }
    }

    let set = helios_sof::resolve_group_members_to_patient_refs(group_refs, &groups);
    Ok(set.into_iter().collect())
}

// ============================================================================
// SQL runtime-filter injection
// ============================================================================

/// Appends runtime filter conditions and the final output limit to the compiled SQL
/// and returns the bound parameters that follow `tenant_id` and
/// `resource_type` (i.e. ViewDefinition constants then runtime filter values).
///
/// SQLite positional parameters are `?1`, `?2`, … The base SQL always uses
/// `?1 = tenant_id` and `?2 = resource_type`. Constants then occupy
/// `?3..?(2+constants.len())`; runtime filter conditions bind from the next
/// free slot, and are attached to every `resources` scan (see
/// [`attach_runtime_conditions`]).
fn build_sqlite_sql(
    base_sql: &str,
    constants: &[super::ir::LitValue],
    filters: &ViewFilters,
    fhir_version: FhirVersion,
    resource_type: &str,
) -> Result<(String, Vec<SqliteParam>), SofError> {
    let mut conditions: Vec<String> = Vec::new();
    let mut extra_params: Vec<SqliteParam> = constants
        .iter()
        .map(SqliteParam::from_lit)
        .collect::<Vec<_>>();
    let mut next_param = 3usize + constants.len();

    if let Some(since) = &filters.since {
        conditions.push(format!("r.last_updated >= ?{next_param}"));
        // Store as RFC 3339 string — SQLite datetime columns are TEXT
        extra_params.push(SqliteParam::Text(since.to_rfc3339()));
        next_param += 1;
    }

    if let Some(c) = compartment_filter_sql(
        fhir_version,
        "Patient",
        resource_type,
        &filters.patient,
        &mut next_param,
        &mut extra_params,
    ) {
        conditions.push(c);
    }

    if let Some(c) = compartment_filter_sql(
        fhir_version,
        "Group",
        resource_type,
        &filters.group,
        &mut next_param,
        &mut extra_params,
    ) {
        conditions.push(c);
    }

    let mut sql = if conditions.is_empty() {
        base_sql.to_string()
    } else {
        attach_runtime_conditions(base_sql, SqlDialect::Sqlite, &conditions.join(" AND "))?
    };

    // Cap final output rows, after filters, expansion, unions, and ordering.
    // Keep oversized public usize limits on the existing client-side path.
    if let Some(limit) = filters.limit.and_then(|limit| i64::try_from(limit).ok()) {
        sql.push_str(&format!("\nLIMIT {limit}"));
    }
    Ok((sql, extra_params))
}

/// Builds a SQLite `WHERE` fragment that filters `r` to resources in the
/// named compartment of any of `compartment_refs`. Drives the lookup off
/// the spec's `CompartmentDefinition` via [`helios_fhir::compartment_params`]
/// and queries the pre-populated `search_index` table — no FHIRPath
/// evaluation at query time. Returns `None` when there are no compartment
/// refs to filter by (skip the clause entirely).
///
/// Two cases:
///
/// 1. **Resource = compartment owner** (e.g. `compartment_type="Patient"`
///    and `resource_type="Patient"`): match `r.id` against the id portion
///    of each compartment ref.
/// 2. **Other resource types**: look up
///    [`helios_fhir::compartment_params`] to get the linking search-param
///    names, then emit an `EXISTS (SELECT 1 FROM search_index …)` clause
///    that joins on `(tenant_id, resource_type, resource_id)` and matches
///    any of those param names against any of the compartment refs. If
///    the resource type isn't in the compartment at all, emit `1=0` so
///    the result set is empty (spec-correct).
fn compartment_filter_sql(
    fhir_version: FhirVersion,
    compartment_type: &str,
    resource_type: &str,
    compartment_refs: &[String],
    next_param: &mut usize,
    extra_params: &mut Vec<SqliteParam>,
) -> Option<String> {
    if compartment_refs.is_empty() {
        return None;
    }

    let canonical_prefix = format!("{}/", compartment_type);

    // Case 1: the view's resource is the compartment owner itself.
    if resource_type == compartment_type {
        let mut ors: Vec<String> = Vec::with_capacity(compartment_refs.len());
        for r in compartment_refs {
            let id = r.strip_prefix(canonical_prefix.as_str()).unwrap_or(r);
            let p = *next_param;
            ors.push(format!("r.id = ?{p}"));
            extra_params.push(SqliteParam::Text(id.to_string()));
            *next_param += 1;
        }
        return Some(format!("({})", ors.join(" OR ")));
    }

    // Case 2: look up the search-param names that link `resource_type`
    // to the compartment.
    let names = helios_fhir::compartment_params(fhir_version, compartment_type, resource_type);
    if names.is_empty() {
        // Spec: "Server SHALL NOT return resources from patient compartments
        // outside provided list." This resource type isn't a member of the
        // compartment, so no rows can match.
        return Some("1=0".to_string());
    }

    let mut name_placeholders = Vec::with_capacity(names.len());
    for n in names {
        let p = *next_param;
        name_placeholders.push(format!("?{p}"));
        extra_params.push(SqliteParam::Text((*n).to_string()));
        *next_param += 1;
    }

    let mut ref_placeholders = Vec::with_capacity(compartment_refs.len());
    for r in compartment_refs {
        let canonical = if r.starts_with(canonical_prefix.as_str()) {
            r.clone()
        } else {
            format!("{}{}", canonical_prefix, r)
        };
        let p = *next_param;
        ref_placeholders.push(format!("?{p}"));
        extra_params.push(SqliteParam::Text(canonical));
        *next_param += 1;
    }

    // `?1` and `?2` are tenant_id and resource_type (bound by the outer
    // query); we reuse them inside the EXISTS subquery so the search_index
    // join stays tenant-isolated and resource-typed.
    Some(format!(
        "EXISTS (SELECT 1 FROM search_index si \
         WHERE si.tenant_id = ?1 \
           AND si.resource_type = ?2 \
           AND si.resource_id = r.id \
           AND si.param_name IN ({}) \
           AND si.value_reference IN ({}))",
        name_placeholders.join(","),
        ref_placeholders.join(",")
    ))
}

// ============================================================================
// Typed parameter — same role as `PgParam` on the PostgreSQL runner.
// ============================================================================

/// Bound-parameter value for the SQLite runner. Mirrors [`super::ir::LitValue`]
/// plus a Text variant for runtime filter strings.
#[derive(Clone, Debug)]
enum SqliteParam {
    Text(String),
    Bool(bool),
    Int(i64),
    /// Decimal preserved as text — SQLite is dynamic-typed and accepts text
    /// for numeric comparisons.
    Decimal(String),
    Null,
}

impl SqliteParam {
    fn from_lit(v: &super::ir::LitValue) -> Self {
        match v {
            super::ir::LitValue::Null => SqliteParam::Null,
            super::ir::LitValue::Bool(b) => SqliteParam::Bool(*b),
            super::ir::LitValue::Int(n) => SqliteParam::Int(*n),
            super::ir::LitValue::Decimal(s) => SqliteParam::Decimal(s.clone()),
            super::ir::LitValue::Str(s) => SqliteParam::Text(s.clone()),
        }
    }
}

impl rusqlite::ToSql for SqliteParam {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, Value};
        Ok(match self {
            SqliteParam::Text(s) => ToSqlOutput::Borrowed(s.as_str().into()),
            SqliteParam::Bool(b) => ToSqlOutput::Owned(Value::Integer(if *b { 1 } else { 0 })),
            SqliteParam::Int(n) => ToSqlOutput::Owned(Value::Integer(*n)),
            // Bind as REAL so SQLite's type-affinity rules let the value
            // compare numerically against `json_extract` results (which are
            // INTEGER/REAL for JSON numbers). Binding as TEXT puts the
            // value in a different storage class and SQLite ranks any TEXT
            // as greater than any numeric value, breaking `<` / `>`.
            SqliteParam::Decimal(s) => match s.parse::<f64>() {
                Ok(n) => ToSqlOutput::Owned(Value::Real(n)),
                Err(_) => ToSqlOutput::Owned(Value::Text(s.clone())),
            },
            SqliteParam::Null => ToSqlOutput::Owned(Value::Null),
        })
    }
}

// ============================================================================
// Blocking row iterator → channel
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn stream_sqlite_rows(
    pool: &Pool<SqliteConnectionManager>,
    sql: &str,
    tenant_id: &str,
    resource_type: &str,
    extra_params: Vec<SqliteParam>,
    columns: &[String],
    decodes: &[ColumnDecode],
    limit: Option<usize>,
    tx: tokio::sync::mpsc::Sender<Result<ViewRow, SofError>>,
) {
    let conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.blocking_send(Err(SofError::Storage(format!(
                "failed to acquire SQLite connection: {e}"
            ))));
            return;
        }
    };

    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.blocking_send(Err(SofError::Backend(format!(
                "failed to prepare SQL: {e}"
            ))));
            return;
        }
    };

    // Build the bound-parameter list: tenant_id, resource_type, then the
    // typed constants + runtime filters from `extra_params`.
    let mut all_params: Vec<SqliteParam> = Vec::with_capacity(2 + extra_params.len());
    all_params.push(SqliteParam::Text(tenant_id.to_string()));
    all_params.push(SqliteParam::Text(resource_type.to_string()));
    all_params.extend(extra_params);

    let row_iter = {
        match stmt.query_map(rusqlite::params_from_iter(all_params.iter()), |row| {
            map_sqlite_row(row, columns, decodes)
        }) {
            Ok(iter) => iter,
            Err(e) => {
                let _ = tx.blocking_send(Err(SofError::Backend(format!(
                    "query execution failed: {e}"
                ))));
                return;
            }
        }
    };

    let mut count = 0usize;
    for row_result in row_iter {
        if let Some(cap) = limit {
            if count >= cap {
                break;
            }
        }
        count += 1;

        let row = match row_result {
            Ok(map) => Ok(Value::Object(map)),
            Err(e) => Err(SofError::Backend(format!("row error: {e}"))),
        };

        if tx.blocking_send(row).is_err() {
            // Receiver dropped (client disconnected) — stop iterating
            break;
        }
    }

    debug!(
        runner = "sqlite-indb",
        rows = count,
        "in-DB view run complete"
    );
    // tx is dropped here, closing the ReceiverStream on the consumer side
}

/// One result row as the flat JSON object every runner emits: every
/// compiled column is present, a SQL NULL as JSON `null`. A row must not
/// drop its NULL columns — the formatters take the column list from the
/// first row, so a first row without `gender` would cut the header and
/// every later row down to its own non-null keys (#1569).
///
/// TEXT and BLOB values are decoded per column through [`decode_text`], so a
/// string column keeps `"44054006"`, `"true"` and `"null"` as strings (#1769).
/// Native INTEGER and REAL values pass through unchanged.
fn map_sqlite_row(
    row: &rusqlite::Row<'_>,
    columns: &[String],
    decodes: &[ColumnDecode],
) -> rusqlite::Result<Map<String, Value>> {
    let mut map = Map::new();
    for (i, name) in columns.iter().enumerate() {
        let val = match row.get_ref(i)? {
            ValueRef::Null => Value::Null,
            // SQLite has no boolean type: `json_extract` yields INTEGER 1/0
            // for a JSON boolean, which a boolean column must report as one.
            ValueRef::Integer(n @ (0 | 1))
                if decodes.get(i).copied() == Some(ColumnDecode::Boolean) =>
            {
                Value::Bool(n == 1)
            }
            ValueRef::Integer(n) => Value::from(n),
            ValueRef::Real(f) => {
                Value::from(serde_json::Number::from_f64(f).unwrap_or(serde_json::Number::from(0)))
            }
            ValueRef::Text(b) => {
                let s = String::from_utf8_lossy(b).into_owned();
                decode_text(decodes.get(i).copied().unwrap_or_default(), s)
            }
            ValueRef::Blob(b) => {
                let s = String::from_utf8_lossy(b).into_owned();
                decode_text(decodes.get(i).copied().unwrap_or_default(), s)
            }
        };
        map.insert(name.clone(), val);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn runtime_sql(view: &Value, filters: &ViewFilters) -> (String, Vec<String>) {
        let compiled = compile_view_definition_dialect(
            view,
            SqlDialect::Sqlite,
            FhirVersion::default_enabled(),
        )
        .expect("compile test view");
        let (sql, params) = build_sqlite_sql(
            &compiled.sql,
            &compiled.constants,
            filters,
            FhirVersion::default_enabled(),
            "Patient",
        )
        .expect("runtime sql");
        let bindings = params
            .iter()
            .map(|param| match param {
                SqliteParam::Text(v) => format!("text:{v}"),
                SqliteParam::Bool(v) => format!("bool:{v}"),
                SqliteParam::Int(v) => format!("int:{v}"),
                SqliteParam::Decimal(v) => format!("decimal:{v}"),
                SqliteParam::Null => "null".into(),
            })
            .collect();
        (sql, bindings)
    }

    fn flat_view() -> Value {
        json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"column":[{"path":"id","name":"id"}]}]})
    }

    #[test]
    fn test_sqlite_runtime_sql_appends_final_output_limit() {
        let view = flat_view();
        let compiled = compile_view_definition_dialect(
            &view,
            SqlDialect::Sqlite,
            FhirVersion::default_enabled(),
        )
        .unwrap();
        let (unlimited, bindings) = runtime_sql(&view, &ViewFilters::default());
        assert_eq!(unlimited, compiled.sql);
        let mut limits = vec![0, 1, 50, 10_000];
        #[cfg(target_pointer_width = "64")]
        limits.push(i64::MAX as usize);
        for limit in limits {
            let (sql, limited_bindings) = runtime_sql(
                &view,
                &ViewFilters {
                    limit: Some(limit),
                    ..Default::default()
                },
            );
            assert_eq!(sql, format!("{unlimited}\nLIMIT {limit}"));
            assert_eq!(limited_bindings, bindings);
        }
    }

    #[test]
    fn test_sqlite_limit_preserves_constant_and_runtime_bindings() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "constant":[{"name":"g","valueString":"male"}],
            "where":[{"path":"gender = %g"}],
            "select":[{"column":[{"path":"id","name":"id"}]}]});
        let mut filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p-eligible".into()],
            ..Default::default()
        };
        let (unlimited, bindings) = runtime_sql(&view, &filters);
        assert!(unlimited.contains("?3"), "{unlimited}");
        assert!(unlimited.contains("r.last_updated >= ?4"), "{unlimited}");
        assert!(unlimited.contains("r.id = ?5"), "{unlimited}");
        assert_eq!(bindings[0], "text:male");
        assert_eq!(bindings.last().unwrap(), "text:p-eligible");
        filters.limit = Some(50);
        let (limited, limited_bindings) = runtime_sql(&view, &filters);
        assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
        assert_eq!(limited_bindings, bindings);
    }

    #[test]
    fn test_sqlite_limit_is_global_for_union_and_recursive_sql() {
        let views = [
            json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"unionAll":[
                {"column":[{"path":"id","name":"id"}]},
                {"column":[{"path":"gender","name":"id"}]}
            ]}]}),
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
                "select":[{"repeat":["item"],
                    "column":[{"path":"linkId","name":"link_id"}]}]}),
        ];
        for view in views {
            let (unlimited, bindings) = runtime_sql(&view, &ViewFilters::default());
            let (limited, limited_bindings) = runtime_sql(
                &view,
                &ViewFilters {
                    limit: Some(50),
                    ..Default::default()
                },
            );
            assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
            assert_eq!(limited_bindings, bindings);
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_sqlite_unrepresentable_limit_keeps_existing_sql() {
        let view = flat_view();
        let unlimited = runtime_sql(&view, &ViewFilters::default());
        for limit in [i64::MAX as usize + 1, usize::MAX] {
            assert_eq!(
                runtime_sql(
                    &view,
                    &ViewFilters {
                        limit: Some(limit),
                        ..Default::default()
                    }
                ),
                unlimited
            );
        }
    }

    #[test]
    fn test_sqlite_runtime_filters_reach_every_resources_scan() {
        let qr = |select: Value| {
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
                "status":"active", "select": select})
        };
        let patient = |select: Value| {
            json!({"resourceType":"ViewDefinition", "resource":"Patient",
                "status":"active", "select": select})
        };
        let id_col = json!({"column":[{"path":"id","name":"value"}]});
        let gender_col = json!({"column":[{"path":"gender","name":"value"}]});
        let repeat_item = json!({"repeat":["item"], "column":[{"path":"linkId","name":"link_id"}]});
        let repeat_value = json!({"repeat":["item"], "column":[{"path":"linkId","name":"value"}]});
        let views = vec![
            (
                patient(json!([{"unionAll":[id_col.clone(), gender_col.clone()]}])),
                2,
            ),
            (
                patient(json!([{"unionAll":[id_col.clone(), gender_col.clone(), id_col.clone()]}])),
                3,
            ),
            (qr(json!([repeat_item.clone()])), 1),
            (
                qr(json!([{"column":[{"path":"id","name":"qr"}]}, repeat_item.clone()])),
                2,
            ),
            (
                qr(json!([{"repeat":["item","answer.item"],
                    "column":[{"path":"linkId","name":"link_id"}]}])),
                2,
            ),
            (qr(json!([{"unionAll":[repeat_value, id_col.clone()]}])), 2),
        ];
        let filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p1".to_string()],
            ..Default::default()
        };
        for (view, scans) in views {
            let (sql, _) = runtime_sql(&view, &filters);
            assert_eq!(sql.matches("r.last_updated >= ?3").count(), scans, "{sql}");
            assert_eq!(sql.matches("r.id = ?4").count(), scans, "{sql}");
        }
    }
}
