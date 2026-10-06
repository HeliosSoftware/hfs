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

use super::compiler::{SqlDialect, SqlViewPlan, append_output_limit};
use super::decode::{ColumnDecode, decode_text};
use super::emit::ResourcePredicates;

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
        // Build the plan synchronously (cheap, no I/O) so uncompilable views
        // fail before any database access.
        let view_plan =
            SqlViewPlan::build(&view_definition, SqlDialect::Sqlite, self.fhir_version)?;

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
        // group_refs so build_sqlite_statement doesn't double-apply.
        if !filters.group.is_empty() {
            let resolved =
                resolve_group_refs_to_patient_refs(&self.pool, &tenant_id, &filters.group)?;
            for p in resolved {
                if !filters.patient.iter().any(|existing| existing == &p) {
                    filters.patient.push(p);
                }
            }
            filters.group.clear();
        }

        // Lower runtime filter conditions (since, patient/group) into every
        // resource scan. Constants occupy `?3..`; runtime filters allocate
        // once from the next free slot.
        let (sql, columns, decodes, extra_params) =
            build_sqlite_statement(&view_plan, &filters, self.fhir_version, &resource_type)?;

        debug!(
            runner = "sqlite-indb",
            tenant = %tenant.tenant_id(),
            "executing compiled ViewDefinition"
        );
        trace!(
            runner = "sqlite-indb",
            sql = %sql,
            columns = ?columns,
            constants = view_plan.constants().len(),
            "compiled ViewDefinition SQL"
        );

        let limit = filters.limit;
        let pool = self.pool.clone();

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
/// groups are silently skipped (matches the inline path; absent-target
/// warning is audit item #5).
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
// SQL runtime-filter lowering
// ============================================================================

/// Renders the final SQL (runtime filters lowered into every resource scan,
/// plus the output limit) and returns it with the visible columns and the
/// bound parameters that follow `tenant_id` and `resource_type` (i.e.
/// ViewDefinition constants then runtime filter values).
///
/// SQLite positional parameters are `?1`, `?2`, … `?1 = tenant_id`,
/// `?2 = resource_type`; constants occupy `?3..?(2+constants.len())`;
/// runtime filter values are allocated once from
/// [`SqlViewPlan::first_runtime_param`].
fn build_sqlite_statement(
    view_plan: &SqlViewPlan,
    filters: &ViewFilters,
    fhir_version: FhirVersion,
    resource_type: &str,
) -> Result<(String, Vec<String>, Vec<ColumnDecode>, Vec<SqliteParam>), SofError> {
    let (predicates, runtime_params) = sqlite_resource_predicates(
        view_plan.first_runtime_param(),
        resource_type,
        filters,
        fhir_version,
    );
    let compiled = view_plan.emit(&predicates)?;
    let mut sql = compiled.sql;

    // Cap final output rows, after filters, expansion, unions, and ordering.
    // Keep oversized public usize limits on the existing client-side path.
    append_output_limit(&mut sql, filters.limit);

    let mut extra_params: Vec<SqliteParam> = compiled
        .constants
        .iter()
        .map(SqliteParam::from_lit)
        .collect();
    extra_params.extend(runtime_params);
    Ok((sql, compiled.columns, compiled.column_decodes, extra_params))
}

/// Allocates the runtime filter slots once, from `first_param`, and builds
/// the resource predicates (`_since`, Patient/Group compartment) the emitter
/// attaches to every resource scan. Returns them with their bound values, in
/// slot order.
fn sqlite_resource_predicates(
    first_param: usize,
    resource_type: &str,
    filters: &ViewFilters,
    fhir_version: FhirVersion,
) -> (ResourcePredicates, Vec<SqliteParam>) {
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<SqliteParam> = Vec::new();
    let mut next_param = first_param;

    if let Some(since) = &filters.since {
        conditions.push(format!("r.last_updated >= ?{next_param}"));
        // Store as RFC 3339 string — SQLite datetime columns are TEXT
        params.push(SqliteParam::Text(since.to_rfc3339()));
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
    use super::super::compiler::compile_view_definition_dialect;
    use super::*;
    use serde_json::json;

    fn runtime_sql(view: &Value, filters: &ViewFilters) -> (String, Vec<String>) {
        let view_plan =
            SqlViewPlan::build(view, SqlDialect::Sqlite, FhirVersion::default_enabled())
                .expect("compile test view");
        let (sql, _, _, params) = build_sqlite_statement(
            &view_plan,
            filters,
            FhirVersion::default_enabled(),
            view["resource"].as_str().unwrap_or_default(),
        )
        .expect("emit test view");
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
    fn test_sqlite_limit_is_global_for_union_and_recursive_sql() {
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
    fn test_sqlite_unrepresentable_limit_keeps_existing_sql() {
        let mut views = vec![flat_view()];
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
    fn test_sqlite_runtime_filters_reuse_slots_in_every_union_branch() {
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
            assert!(operand.contains("?3"), "constant in {operand}");
            assert_eq!(
                operand.matches("r.last_updated >= ?4").count(),
                1,
                "{operand}"
            );
            assert_eq!(operand.matches("(r.id = ?5)").count(), 1, "{operand}");
        }
        assert!(
            !sql.contains("?6"),
            "runtime slots must be allocated once: {sql}"
        );
        assert_eq!(
            bindings,
            [
                "text:male",
                "text:2024-01-01T00:00:00+00:00",
                "text:p-eligible"
            ]
        );
    }

    #[test]
    fn test_sqlite_runtime_filters_lower_into_every_recursive_seed() {
        let view = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "constant":[{"name":"s","valueString":"completed"}],
            "where":[{"path":"status = %s"}],
            "select":[{"repeat":["item","answer.item"],
                "column":[{"path":"linkId","name":"link_id"}]}]});
        let (sql, bindings) = runtime_sql(&view, &since_and_patient());
        let (cte, outer) = recursive_parts(&sql);
        // Two seeds (one per repeat path) each carry the resource predicates.
        assert_eq!(cte.matches("FROM resources r").count(), 2, "{sql}");
        assert_eq!(cte.matches("r.last_updated >= ?4").count(), 2, "{sql}");
        assert_eq!(cte.matches("FROM search_index si").count(), 2, "{sql}");
        assert!(!outer.contains("r.last_updated"), "{sql}");
        // #1623 2C: the first-column primary key gains explicit NULL
        // placement and the resource-key/traversal-identity tie-breaks.
        assert!(
            sql.ends_with(
                "FROM rec_0\nORDER BY 1 ASC NULLS FIRST, rec_0.last_updated, rec_0.rid, rec_0.ident COLLATE BINARY"
            ),
            "{sql}"
        );
        assert_eq!(bindings[0], "text:completed");
        assert_eq!(bindings[1], "text:2024-01-01T00:00:00+00:00");
        assert_eq!(bindings.last().unwrap(), "text:Patient/p-eligible");
        let slots = bindings.len() + 2;
        assert!(sql.contains(&format!("?{slots}")), "{sql}");
        assert!(!sql.contains(&format!("?{}", slots + 1)), "{sql}");
    }

    #[test]
    fn test_sqlite_runtime_filters_lower_into_recursive_union_branch_and_rejoin() {
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
        assert_eq!(cte.matches("r.last_updated >= ?3").count(), 1, "{sql}");
        assert!(outer.ends_with("FROM rec_0) AS _recurse_0"), "{sql}");
        assert_eq!(
            operands[1].matches("r.last_updated >= ?3").count(),
            1,
            "{sql}"
        );
        assert_eq!(bindings.len(), 1);

        // A sibling resource column rejoins `resources r`; that scan carries
        // the predicates too.
        let rejoin = json!({"resourceType":"ViewDefinition", "resource":"QuestionnaireResponse",
            "select":[{"column":[{"path":"id","name":"id"}]},
                {"repeat":["item"],"column":[{"path":"linkId","name":"link_id"}]}]});
        let (sql, _) = runtime_sql(&rejoin, &filters);
        let (cte, outer) = recursive_parts(&sql);
        assert_eq!(cte.matches("r.last_updated >= ?3").count(), 1, "{sql}");
        assert_eq!(outer.matches("r.last_updated >= ?3").count(), 1, "{sql}");
        assert!(
            outer.contains("JOIN resources r ON r.id = rec_0.rid"),
            "{sql}"
        );
    }
}
