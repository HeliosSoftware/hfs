//! PostgreSQL in-DB SQL-on-FHIR runner.
//!
//! [`PgInDbRunner`] compiles a ViewDefinition to a parameterised PostgreSQL
//! `SELECT` statement and executes it directly against the `resources` table,
//! bypassing in-process FHIRPath evaluation entirely.
//!
//! ## Streaming
//!
//! Rows are fetched lazily via `tokio_postgres::Client::query_raw` and sent
//! through a bounded `tokio::sync::mpsc` channel (buffer: 256) so the HTTP
//! layer can begin flushing before the full result set has been transferred.
//! The async fetch loop runs in a `tokio::spawn` task that holds the pooled
//! connection open until the consumer drops the receiver.

use deadpool_postgres::Pool;
use futures::StreamExt as _;
use helios_fhir::FhirVersion;
use serde_json::{Map, Value};
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::core::sof_runner::{RowStream, SofError, SofRunner, ViewFilters, ViewRow};
use crate::tenant::TenantContext;

use super::compiler::{SqlDialect, compile_view_definition_dialect};

/// Channel buffer depth (rows that can be queued ahead of the consumer).
const CHANNEL_BUFFER: usize = 256;

/// SQL-on-FHIR runner that compiles ViewDefinitions to PostgreSQL SQL.
pub struct PgInDbRunner {
    pool: Pool,
    fhir_version: FhirVersion,
}

impl PgInDbRunner {
    /// Creates a new runner backed by the given connection pool. Uses the
    /// default FHIR version (R4) for compile-time cardinality lookups; call
    /// [`Self::with_fhir_version`] to override.
    pub fn new(pool: Pool) -> Self {
        Self {
            pool,
            fhir_version: FhirVersion::default(),
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
impl SofRunner for PgInDbRunner {
    fn runner_name(&self) -> &'static str {
        "postgres-indb"
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
            SqlDialect::Postgres,
            self.fhir_version,
        )?;

        debug!(
            runner = "postgres-indb",
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );

        let tenant_id = tenant.tenant_id().to_string();
        let resource_type = view_definition
            .get("resource")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Spec-correct `group` handling: resolve each Group/{id} to its
        // `member.entity` Patient references and fold them into the patient
        // filter. Same pattern as the SQLite runner.
        if !filters.group.is_empty() {
            let resolved =
                resolve_group_refs_to_patient_refs(&self.pool, &tenant_id, &filters.group).await?;
            for p in resolved {
                if !filters.patient.iter().any(|existing| existing == &p) {
                    filters.patient.push(p);
                }
            }
            filters.group.clear();
        }

        let limit = filters.limit;
        let columns = compiled.columns.clone();
        let pool = self.pool.clone();

        // Build SQL with runtime filters and collect typed params. The
        // compiled query already reserves `$3..$N` for ViewDefinition
        // constants; runtime filters allocate from the next free slot.
        let (sql, params) = build_pg_sql_and_params(
            &compiled.sql,
            tenant_id,
            resource_type,
            &compiled.constants,
            &filters,
            self.fhir_version,
        );

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);

        tokio::spawn(async move {
            stream_pg_rows(pool, sql, params, columns, limit, tx).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

/// Loads each `Group/{id}` from the `resources` table and extracts its
/// `member.entity` Patient references via the shared
/// [`helios_sof::resolve_group_members_to_patient_refs`]. Returns the
/// union of those Patient refs across all supplied group refs. Unknown
/// groups are silently skipped (matches the inline path; absent-target
/// warning is audit item #5).
async fn resolve_group_refs_to_patient_refs(
    pool: &Pool,
    tenant_id: &str,
    group_refs: &[String],
) -> Result<Vec<String>, SofError> {
    if group_refs.is_empty() {
        return Ok(Vec::new());
    }
    let client = pool
        .get()
        .await
        .map_err(|e| SofError::Storage(format!("failed to get pg connection: {e}")))?;
    let stmt = client
        .prepare(
            "SELECT data FROM resources \
             WHERE tenant_id = $1 \
               AND resource_type = 'Group' \
               AND id = $2 \
               AND is_deleted = false",
        )
        .await
        .map_err(|e| SofError::Storage(format!("prepare failed: {e}")))?;

    let mut groups = Vec::with_capacity(group_refs.len());
    for r in group_refs {
        let id = r.strip_prefix("Group/").unwrap_or(r);
        match client.query_opt(&stmt, &[&tenant_id, &id]).await {
            Ok(Some(row)) => {
                let data: Value = row.get(0);
                groups.push(data);
            }
            Ok(None) => continue,
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

/// Builds the final SQL and typed params list for a PG query.
///
/// The base SQL uses `$1 = tenant_id` and `$2 = resource_type`.
/// Extra filter conditions inject `$3`, `$4`, … as needed.
fn build_pg_sql_and_params(
    base_sql: &str,
    tenant_id: String,
    resource_type: String,
    constants: &[super::ir::LitValue],
    filters: &ViewFilters,
    fhir_version: FhirVersion,
) -> (String, Vec<PgParam>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut extra: Vec<PgParam> = Vec::new();
    // Constants occupy `$3..$(2+constants.len())`; runtime filters start
    // immediately after.
    let mut constant_params: Vec<PgParam> = Vec::with_capacity(constants.len());
    for c in constants {
        constant_params.push(PgParam::from_lit(c));
    }
    let mut next_param = 3usize + constants.len();

    if let Some(since) = filters.since {
        conditions.push(format!("r.last_updated >= ${next_param}"));
        extra.push(PgParam::Timestamp(since));
        next_param += 1;
    }

    if let Some(c) = compartment_filter_sql(
        fhir_version,
        "Patient",
        &resource_type,
        &filters.patient,
        &mut next_param,
        &mut extra,
    ) {
        conditions.push(c);
    }

    if let Some(c) = compartment_filter_sql(
        fhir_version,
        "Group",
        &resource_type,
        &filters.group,
        &mut next_param,
        &mut extra,
    ) {
        conditions.push(c);
    }

    let sql = if conditions.is_empty() {
        base_sql.to_string()
    } else {
        let joined = conditions.join(" AND ");
        inject_before_order_by(base_sql, &format!(" AND {joined}"))
    };

    let mut all_params = vec![PgParam::Text(tenant_id), PgParam::Text(resource_type)];
    all_params.extend(constant_params);
    all_params.extend(extra);

    (sql, all_params)
}

/// Builds a PostgreSQL `WHERE` fragment that filters `r` to resources in
/// the named compartment of any of `compartment_refs`. Drives the lookup
/// off the spec's `CompartmentDefinition` via
/// [`helios_fhir::compartment_params`] and queries the pre-populated
/// `search_index` table — no FHIRPath evaluation at query time.
///
/// See the matching SQLite implementation for algorithm details; the only
/// difference here is `$N` parameter syntax instead of `?N`.
fn compartment_filter_sql(
    fhir_version: FhirVersion,
    compartment_type: &str,
    resource_type: &str,
    compartment_refs: &[String],
    next_param: &mut usize,
    extra_params: &mut Vec<PgParam>,
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
            ors.push(format!("r.id = ${p}"));
            extra_params.push(PgParam::Text(id.to_string()));
            *next_param += 1;
        }
        return Some(format!("({})", ors.join(" OR ")));
    }

    // Case 2: look up the search-param names that link `resource_type`
    // to the compartment.
    let names = helios_fhir::compartment_params(fhir_version, compartment_type, resource_type);
    if names.is_empty() {
        return Some("1=0".to_string());
    }

    let mut name_placeholders = Vec::with_capacity(names.len());
    for n in names {
        let p = *next_param;
        name_placeholders.push(format!("${p}"));
        extra_params.push(PgParam::Text((*n).to_string()));
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
        ref_placeholders.push(format!("${p}"));
        extra_params.push(PgParam::Text(canonical));
        *next_param += 1;
    }

    // `$1` and `$2` are tenant_id and resource_type (bound by the outer
    // query); we reuse them inside the EXISTS subquery so the search_index
    // join stays tenant-isolated and resource-typed.
    Some(format!(
        "EXISTS (SELECT 1 FROM search_index si \
         WHERE si.tenant_id = $1 \
           AND si.resource_type = $2 \
           AND si.resource_id = r.id \
           AND si.param_name IN ({}) \
           AND si.value_reference IN ({}))",
        name_placeholders.join(","),
        ref_placeholders.join(",")
    ))
}

/// Inserts `extra` before the trailing `ORDER BY` in `sql`, or appends it.
///
/// The compiler emits `\nORDER BY …` (newline-prefixed), so we search for
/// that pattern first; the space-prefixed variant is a fallback for hand-crafted SQL.
fn inject_before_order_by(sql: &str, extra: &str) -> String {
    let search = ["\nORDER BY", " ORDER BY"];
    for pat in search {
        if let Some(pos) = sql.rfind(pat) {
            let mut s = sql.to_string();
            s.insert_str(pos, extra);
            return s;
        }
    }
    format!("{sql}{extra}")
}

// ============================================================================
// Typed parameter enum — avoids the self-referential borrow issues with
// `Vec<Box<dyn ToSql>>` + `Vec<&dyn ToSql>` that arise in async tasks.
// ============================================================================

#[derive(Clone)]
enum PgParam {
    Text(String),
    Bool(bool),
    Int(i64),
    Decimal(String),
    Null,
    Timestamp(chrono::DateTime<chrono::Utc>),
}

impl PgParam {
    /// Lifts a [`super::ir::LitValue`] (used by `ViewDefinition.constant[]`)
    /// into the runtime parameter representation. Decimals bind as text and
    /// rely on PG's implicit cast to `numeric` at the call site.
    fn from_lit(v: &super::ir::LitValue) -> Self {
        match v {
            super::ir::LitValue::Null => PgParam::Null,
            super::ir::LitValue::Bool(b) => PgParam::Bool(*b),
            super::ir::LitValue::Int(n) => PgParam::Int(*n),
            super::ir::LitValue::Decimal(s) => PgParam::Decimal(s.clone()),
            super::ir::LitValue::Str(s) => PgParam::Text(s.clone()),
        }
    }
}

// ============================================================================
// Async fetch loop
// ============================================================================

async fn stream_pg_rows(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    limit: Option<usize>,
    tx: tokio::sync::mpsc::Sender<Result<ViewRow, SofError>>,
) {
    if let Err(e) = stream_pg_rows_inner(pool, sql, params, columns, limit, &tx).await {
        let _ = tx.send(Err(e)).await;
    }
}

async fn stream_pg_rows_inner(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    limit: Option<usize>,
    tx: &tokio::sync::mpsc::Sender<Result<ViewRow, SofError>>,
) -> Result<(), SofError> {
    let client = pool
        .get()
        .await
        .map_err(|e| SofError::Storage(format!("failed to acquire Postgres connection: {e}")))?;

    if std::env::var("PG_SOF_DEBUG_ALL").is_ok() {
        eprintln!("[PG_SOF_DEBUG_ALL] preparing\n--- SQL ---\n{sql}\n---");
    }
    let stmt = client.prepare(&sql).await.map_err(|e| {
        if std::env::var("PG_SOF_DEBUG").is_ok() {
            eprintln!("[PG_SOF_DEBUG] prepare failed: {e}\n--- SQL ---\n{sql}\n---");
        }
        SofError::Backend(format!("failed to prepare SQL: {e}"))
    })?;

    // Build boxed params for query_raw; these are 'static + Send
    let boxed: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = params
        .into_iter()
        .map(|p| -> Box<dyn tokio_postgres::types::ToSql + Sync + Send> {
            match p {
                PgParam::Text(s) => Box::new(s),
                // Bind Bool/Int/Decimal constants as text so they compare
                // cleanly against `->>`/`#>>` JSON-text projections without
                // a per-call PG type-mismatch. Numeric contexts apply
                // explicit `::numeric` casts via `lower_binop_dialect`;
                // boolean contexts compare against `'true'`/`'false'`.
                PgParam::Bool(b) => Box::new(if b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }),
                PgParam::Int(n) => Box::new(n.to_string()),
                PgParam::Decimal(s) => Box::new(s),
                PgParam::Null => Box::new(None::<String>),
                PgParam::Timestamp(dt) => Box::new(dt),
            }
        })
        .collect();

    // query_raw needs a slice of &dyn ToSql + Sync. Build references that borrow
    // from `boxed` — both live in this async block's stack frame, so no lifetime
    // issue (the future holds them until the stream is exhausted).
    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = boxed
        .iter()
        .map(|b| b.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();

    let raw = client
        .query_raw(&stmt, param_refs.iter().copied())
        .await
        .map_err(|e| {
            if std::env::var("PG_SOF_DEBUG").is_ok() {
                eprintln!("[PG_SOF_DEBUG] query failed: {e}\n--- SQL ---\n{sql}\n---");
            }
            SofError::Backend(format!("query execution failed: {e}"))
        })?;

    // params no longer needed after query_raw returns (data sent to DB)
    drop(param_refs);
    drop(boxed);

    futures::pin_mut!(raw);

    let mut count = 0usize;
    while let Some(row_result) = raw.next().await {
        match row_result {
            Ok(pg_row) => {
                if let Some(cap) = limit {
                    if count >= cap {
                        break;
                    }
                }
                count += 1;
                match row_to_json(&pg_row, &columns) {
                    Ok(row) => {
                        if tx.send(Ok(row)).await.is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        break;
                    }
                }
            }
            Err(e) => {
                if std::env::var("PG_SOF_DEBUG").is_ok() {
                    eprintln!("[PG_SOF_DEBUG] row error: {e}\n--- SQL ---\n{sql}\n---");
                }
                let _ = tx
                    .send(Err(SofError::Backend(format!("row error: {e}"))))
                    .await;
                break;
            }
        }
    }

    debug!(
        runner = "postgres-indb",
        rows = count,
        "in-DB view run complete"
    );
    Ok(())
    // tx dropped here, closing the ReceiverStream
}

// ============================================================================
// Row → JSON conversion
// ============================================================================

/// Converts a `tokio_postgres::Row` into a `serde_json::Value` object.
///
/// The compiled SQL projects all columns as text via `->>`/`#>>` operators.
fn row_to_json(pg_row: &tokio_postgres::Row, columns: &[String]) -> Result<ViewRow, SofError> {
    let mut map = Map::new();
    for (i, name) in columns.iter().enumerate() {
        let val: Option<String> = pg_row
            .try_get(i)
            .map_err(|e| SofError::Backend(format!("failed to read column '{name}': {e}")))?;

        if let Some(s) = val {
            let json_val = serde_json::from_str(&s).unwrap_or(Value::String(s));
            map.insert(name.clone(), json_val);
        }
    }
    Ok(Value::Object(map))
}
