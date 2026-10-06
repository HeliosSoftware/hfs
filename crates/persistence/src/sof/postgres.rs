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
//! connection open until the consumer drops the receiver. That task's
//! `JoinHandle` is watched by [`watch_row_producer`](crate::core::sof_runner::watch_row_producer)
//! so a panic or cancellation reaches the consumer as an `Err` item instead
//! of a silent end of stream.

use deadpool_postgres::Pool;
use futures::StreamExt as _;
use helios_fhir::FhirVersion;
use serde_json::{Map, Value};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, trace};

use crate::core::sof_runner::{
    RowStream, SofError, SofRunner, ViewFilters, ViewRow, watch_row_producer,
};
use crate::tenant::TenantContext;

use super::compiler::{SqlDialect, SqlViewPlan, append_output_limit};
use super::decode::{ColumnDecode, decode_text};
use super::emit::ResourcePredicates;

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
        // Build the plan synchronously (cheap, no I/O) so uncompilable views
        // fail before any database access.
        let view_plan =
            SqlViewPlan::build(&view_definition, SqlDialect::Postgres, self.fhir_version)?;

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

        // Lower runtime filters into every resource scan and collect typed
        // params. Constants occupy `$3..`; runtime filters allocate once
        // from the next free slot.
        let (sql, columns, decodes, params) = build_pg_statement(
            &view_plan,
            tenant_id,
            resource_type,
            &filters,
            self.fhir_version,
        )?;

        debug!(
            runner = "postgres-indb",
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );
        trace!(
            runner = "postgres-indb",
            sql = %sql,
            columns = ?columns,
            constants = view_plan.constants().len(),
            "compiled ViewDefinition SQL"
        );

        let limit = filters.limit;
        let pool = self.pool.clone();

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);
        let guard_tx = tx.clone();

        let producer = tokio::spawn(async move {
            stream_pg_rows(pool, sql, params, columns, decodes, limit, tx).await;
        });
        watch_row_producer(self.runner_name(), guard_tx, producer);

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
// SQL runtime-filter lowering
// ============================================================================

/// Renders the final SQL (runtime filters lowered into every resource scan,
/// plus the output limit) and returns it with the visible columns and typed
/// params.
///
/// Params: `$1 = tenant_id`, `$2 = resource_type`, `$3..` constants, then
/// runtime filter values allocated once from
/// [`SqlViewPlan::first_runtime_param`].
fn build_pg_statement(
    view_plan: &SqlViewPlan,
    tenant_id: String,
    resource_type: String,
    filters: &ViewFilters,
    fhir_version: FhirVersion,
) -> Result<(String, Vec<String>, Vec<ColumnDecode>, Vec<PgParam>), SofError> {
    let (predicates, runtime_params) = pg_resource_predicates(
        view_plan.first_runtime_param(),
        &resource_type,
        filters,
        fhir_version,
    );
    let compiled = view_plan.emit(&predicates)?;
    let mut sql = compiled.sql;

    // Every shape's final ORDER BY is total, so one output LIMIT after it
    // returns exactly the unlimited statement's prefix, for flat views,
    // expansions, unions and recursion alike. Oversized public usize limits
    // keep the client-side cap in the fetch loop as their only bound.
    append_output_limit(&mut sql, filters.limit);

    let mut all_params = vec![PgParam::Text(tenant_id), PgParam::Text(resource_type)];
    all_params.extend(compiled.constants.iter().map(PgParam::from_lit));
    all_params.extend(runtime_params);

    Ok((sql, compiled.columns, compiled.column_decodes, all_params))
}

/// Allocates the runtime filter slots once, from `first_param`, and builds
/// the resource predicates (`_since`, Patient/Group compartment) the emitter
/// attaches to every resource scan. Returns them with their bound values, in
/// slot order.
fn pg_resource_predicates(
    first_param: usize,
    resource_type: &str,
    filters: &ViewFilters,
    fhir_version: FhirVersion,
) -> (ResourcePredicates, Vec<PgParam>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<PgParam> = Vec::new();
    let mut next_param = first_param;

    if let Some(since) = filters.since {
        conditions.push(format!("r.last_updated >= ${next_param}"));
        params.push(PgParam::Timestamp(since));
        next_param += 1;
    }

    if let Some(c) = compartment_filter_sql(
        fhir_version,
        "Patient",
        resource_type,
        &filters.patient,
        &mut next_param,
        &mut params,
    ) {
        conditions.push(c);
    }

    if let Some(c) = compartment_filter_sql(
        fhir_version,
        "Group",
        resource_type,
        &filters.group,
        &mut next_param,
        &mut params,
    ) {
        conditions.push(c);
    }

    (
        ResourcePredicates::new(first_param, next_param - first_param, conditions),
        params,
    )
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
    decodes: Vec<ColumnDecode>,
    limit: Option<usize>,
    tx: tokio::sync::mpsc::Sender<Result<ViewRow, SofError>>,
) {
    if let Err(e) = stream_pg_rows_inner(pool, sql, params, columns, decodes, limit, &tx).await {
        let _ = tx.send(Err(e)).await;
    }
}

async fn stream_pg_rows_inner(
    pool: Pool,
    sql: String,
    params: Vec<PgParam>,
    columns: Vec<String>,
    decodes: Vec<ColumnDecode>,
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
                match row_to_json(&pg_row, &columns, &decodes) {
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
/// The compiled SQL projects all columns as text via `->>`/`#>>` operators, so
/// each text value is decoded according to its column's [`ColumnDecode`]. A
/// SQL `NULL` is written as an explicit JSON `null` so the key is never lost.
fn row_to_json(
    pg_row: &tokio_postgres::Row,
    columns: &[String],
    decodes: &[ColumnDecode],
) -> Result<ViewRow, SofError> {
    let mut map = Map::new();
    for (i, name) in columns.iter().enumerate() {
        let val: Option<String> = pg_row
            .try_get(i)
            .map_err(|e| SofError::Backend(format!("failed to read column '{name}': {e}")))?;

        let json_val = match val {
            Some(s) => decode_text(decodes.get(i).copied().unwrap_or_default(), s),
            None => Value::Null,
        };
        map.insert(name.clone(), json_val);
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::super::compiler::compile_view_definition_dialect;
    use super::*;
    use serde_json::json;

    fn runtime_sql(view: &Value, filters: &ViewFilters) -> (String, Vec<String>) {
        let view_plan =
            SqlViewPlan::build(view, SqlDialect::Postgres, FhirVersion::default_enabled())
                .expect("compile test view");
        let resource_type = view["resource"].as_str().unwrap_or_default().to_string();
        let (sql, _, _, params) = build_pg_statement(
            &view_plan,
            "tenant".into(),
            resource_type,
            filters,
            FhirVersion::default_enabled(),
        )
        .expect("emit test view");
        let bindings = params
            .iter()
            .map(|param| match param {
                PgParam::Text(v) => format!("text:{v}"),
                PgParam::Bool(v) => format!("bool:{v}"),
                PgParam::Int(v) => format!("int:{v}"),
                PgParam::Decimal(v) => format!("decimal:{v}"),
                PgParam::Null => "null".into(),
                PgParam::Timestamp(v) => format!("timestamp:{v}"),
            })
            .collect();
        (sql, bindings)
    }

    fn flat_view() -> Value {
        json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"column":[{"path":"id","name":"id"}]}]})
    }

    #[test]
    fn test_pg_runtime_sql_appends_final_output_limit() {
        let view = flat_view();
        let compiled = compile_view_definition_dialect(
            &view,
            SqlDialect::Postgres,
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
    fn test_pg_limit_preserves_constant_and_runtime_bindings() {
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
        assert!(unlimited.contains("$3"), "{unlimited}");
        assert!(unlimited.contains("r.last_updated >= $4"), "{unlimited}");
        assert!(unlimited.contains("r.id = $5"), "{unlimited}");
        assert_eq!(bindings[2], "text:male");
        assert_eq!(bindings.last().unwrap(), "text:p-eligible");
        filters.limit = Some(50);
        let (limited, limited_bindings) = runtime_sql(&view, &filters);
        assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
        assert_eq!(limited_bindings, bindings);
    }

    /// Union, repeat, union-with-repeat-branch and multi-path repeat views.
    fn complex_limit_views() -> [Value; 4] {
        [
            json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "select":[{"unionAll":[
                {"column":[{"path":"id","name":"id"}]},
                {"column":[{"path":"gender","name":"id"}]}
            ]}]}),
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
                "select":[{"repeat":["item"],
                    "column":[{"path":"linkId","name":"link_id"}]}]}),
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "select":[{"unionAll":[
                {"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
                {"column":[{"path":"id","name":"v"}]}
            ]}]}),
            json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
                "select":[{"repeat":["item","answer.item"],
                    "column":[{"path":"linkId","name":"link_id"}]}]}),
        ]
    }

    #[test]
    fn test_pg_union_and_recursive_limits_append_one_final_limit() {
        for view in complex_limit_views() {
            let (unlimited, bindings) = runtime_sql(&view, &ViewFilters::default());
            assert!(!unlimited.contains("\nLIMIT "), "{unlimited}");
            let (limited, limited_bindings) = runtime_sql(
                &view,
                &ViewFilters {
                    limit: Some(50),
                    ..Default::default()
                },
            );
            // One LIMIT, after the final (outer, for unions) ORDER BY.
            assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
            assert_eq!(limited.matches("\nLIMIT ").count(), 1, "{limited}");
            let tail = &unlimited[unlimited.rfind("ORDER BY ").expect("final ORDER BY")..];
            assert_eq!(
                tail.matches('(').count(),
                tail.matches(')').count(),
                "the final ORDER BY is top-level: {unlimited}"
            );
            if unlimited.contains("\nUNION ALL\n") {
                assert!(
                    union_operands(&limited)
                        .iter()
                        .all(|operand| !operand.contains("\nLIMIT ")),
                    "never per branch: {limited}"
                );
            }
            assert_eq!(limited_bindings, bindings);
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_pg_unrepresentable_limit_keeps_existing_sql() {
        let mut views = vec![
            flat_view(),
            json!({"resourceType":"ViewDefinition","resource":"Patient",
            "select":[{"forEach":"name","column":[{"name":"family","path":"family"}]}]}),
        ];
        views.extend(complex_limit_views());
        for view in views {
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
    }

    #[test]
    fn test_pg_expansion_final_limit_preserves_filtered_sql_and_bindings() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
            "constant":[{"name":"g","valueString":"male"}],
            "where":[{"path":"gender = %g"}],
            "select":[{"forEach":"name","column":[{"path":"family","name":"family"}]}]});
        let mut filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p-eligible".into()],
            ..Default::default()
        };
        let (unlimited, bindings) = runtime_sql(&view, &filters);
        assert!(unlimited.contains("r.last_updated >= $4"));
        assert!(unlimited.contains("r.id = $5"));
        let mut limits = vec![0, 1, 50, 10_000];
        #[cfg(target_pointer_width = "64")]
        limits.push(i64::MAX as usize);
        for limit in limits {
            filters.limit = Some(limit);
            let (limited, limited_bindings) = runtime_sql(&view, &filters);
            assert_eq!(limited, format!("{unlimited}\nLIMIT {limit}"));
            assert_eq!(limited_bindings, bindings);
        }
        // A constant-filtered union with a repeat branch keeps its constant
        // and runtime-filter slots, and takes the same single final LIMIT.
        let view = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
        "constant":[{"name":"s","valueString":"completed"}],
        "where":[{"path":"status = %s"}],
        "select":[{"unionAll":[
            {"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
            {"column":[{"path":"id","name":"v"}]}
        ]}]});
        filters.limit = None;
        let (unlimited, bindings) = runtime_sql(&view, &filters);
        assert!(unlimited.contains("r.last_updated >= $4"), "{unlimited}");
        assert_eq!(bindings[2], "text:completed");
        filters.limit = Some(50);
        let (limited, limited_bindings) = runtime_sql(&view, &filters);
        assert_eq!(limited, format!("{unlimited}\nLIMIT 50"));
        assert_eq!(limited_bindings, bindings);
    }

    /// Splits a runtime statement into its top-level `UNION ALL` operands,
    /// dropping the outer visible projection and the final ordering.
    fn union_operands(sql: &str) -> Vec<&str> {
        let (_, inner) = sql
            .split_once("\nFROM (\n")
            .expect("union is wrapped in an outer SELECT");
        inner
            .split_once("\n) AS u\nORDER BY ")
            .expect("outer ORDER BY over the wrapped union")
            .0
            .split("\nUNION ALL\n")
            .collect()
    }

    /// Returns the `WITH RECURSIVE` CTE body and the outer SELECT of one
    /// recursive statement or union operand.
    fn recursive_parts(sql: &str) -> (&str, &str) {
        let start = sql.find("AS (\n").expect("recursive CTE body") + "AS (\n".len();
        let end = sql.find("\n)\nSELECT").expect("recursive CTE end");
        (&sql[start..end], &sql[end..])
    }

    fn since_and_patient() -> ViewFilters {
        ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p-eligible".into()],
            ..Default::default()
        }
    }

    #[test]
    fn test_pg_runtime_filters_reuse_slots_in_every_union_branch() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"Patient",
        "constant":[{"name":"g","valueString":"male"}],
        "where":[{"path":"gender = %g"}],
        "select":[{"unionAll":[
            {"column":[{"path":"id","name":"v"}]},
            {"forEach":"name","column":[{"path":"family","name":"v"}]},
            {"column":[{"path":"gender","name":"v"}]}
        ]}]});
        let (sql, bindings) = runtime_sql(&view, &since_and_patient());
        let operands = union_operands(&sql);
        assert_eq!(operands.len(), 3, "{sql}");
        for operand in &operands {
            assert!(operand.contains("$3"), "constant in {operand}");
            assert_eq!(
                operand.matches("r.last_updated >= $4").count(),
                1,
                "{operand}"
            );
            assert_eq!(operand.matches("(r.id = $5)").count(), 1, "{operand}");
        }
        assert!(
            !sql.contains("$6"),
            "runtime slots must be allocated once: {sql}"
        );
        assert_eq!(
            bindings,
            [
                "text:tenant",
                "text:Patient",
                "text:male",
                "timestamp:2024-01-01 00:00:00 UTC",
                "text:p-eligible"
            ]
        );
    }

    #[test]
    fn test_pg_runtime_filters_lower_into_every_recursive_seed() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "constant":[{"name":"s","valueString":"completed"}],
            "where":[{"path":"status = %s"}],
            "select":[{"repeat":["item","answer.item"],
                "column":[{"path":"linkId","name":"link_id"}]}]});
        let (sql, bindings) = runtime_sql(&view, &since_and_patient());
        let (cte, outer) = recursive_parts(&sql);
        // Two seeds (one per repeat path) each carry the resource predicates.
        assert_eq!(cte.matches("FROM resources r").count(), 2, "{sql}");
        assert_eq!(cte.matches("r.last_updated >= $4").count(), 2, "{sql}");
        assert_eq!(cte.matches("FROM search_index si").count(), 2, "{sql}");
        assert!(!outer.contains("r.last_updated"), "{sql}");
        // #1623 2C: the first-column primary key gains explicit NULL
        // placement and the resource-key/traversal-identity tie-breaks.
        assert!(
            sql.ends_with(
                "FROM rec_0\nORDER BY 1 ASC NULLS LAST, rec_0.last_updated, rec_0.rid, rec_0.ident"
            ),
            "{sql}"
        );
        assert_eq!(bindings[2], "text:completed");
        assert_eq!(bindings[3], "timestamp:2024-01-01 00:00:00 UTC");
        assert_eq!(bindings.last().unwrap(), "text:Patient/p-eligible");
        let slots = bindings.len();
        assert!(sql.contains(&format!("${slots}")), "{sql}");
        assert!(!sql.contains(&format!("${}", slots + 1)), "{sql}");
    }

    #[test]
    fn test_pg_runtime_filters_lower_into_recursive_union_branch_and_rejoin() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
        "select":[{"unionAll":[
            {"repeat":["item"],"column":[{"path":"linkId","name":"v"}]},
            {"column":[{"path":"id","name":"v"}]}
        ]}]});
        let filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let (sql, bindings) = runtime_sql(&view, &filters);
        let operands = union_operands(&sql);
        assert_eq!(operands.len(), 2, "{sql}");
        let (cte, outer) = recursive_parts(operands[0]);
        assert_eq!(cte.matches("r.last_updated >= $3").count(), 1, "{sql}");
        assert!(outer.ends_with("FROM rec_0) AS _recurse_0"), "{sql}");
        assert_eq!(
            operands[1].matches("r.last_updated >= $3").count(),
            1,
            "{sql}"
        );
        assert_eq!(bindings.len(), 3);

        // A sibling resource column rejoins `resources r`; that scan carries
        // the predicates too.
        let rejoin = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "select":[{"column":[{"path":"id","name":"id"}]},
                {"repeat":["item"],"column":[{"path":"linkId","name":"link_id"}]}]});
        let (sql, _) = runtime_sql(&rejoin, &filters);
        let (cte, outer) = recursive_parts(&sql);
        assert_eq!(cte.matches("r.last_updated >= $3").count(), 1, "{sql}");
        assert_eq!(outer.matches("r.last_updated >= $3").count(), 1, "{sql}");
        assert!(
            outer.contains("JOIN resources r ON r.id = rec_0.rid"),
            "{sql}"
        );
    }
}
