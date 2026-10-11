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

use super::compiler::SqlDialect;
use super::decode::{ColumnDecode, decode_text};
use super::runtime::{RuntimeParam, prepare_sql_run};

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
        filters: ViewFilters,
    ) -> Result<RowStream, SofError> {
        let tenant_id = tenant.tenant_id().to_string();
        let group_pool = self.pool.clone();
        let group_tenant = tenant_id.clone();
        let Some(prepared) = prepare_sql_run(
            &view_definition,
            SqlDialect::Sqlite,
            self.fhir_version,
            &tenant_id,
            filters,
            move |refs| load_group_documents(group_pool, group_tenant, refs),
        )
        .await?
        else {
            return Ok(Box::pin(futures::stream::empty()));
        };
        let compiled = prepared.query;
        debug!(runner = "sqlite-indb", tenant = %tenant_id, "executing compiled ViewDefinition");
        trace!(
            runner = "sqlite-indb", sql = %compiled.sql, columns = ?compiled.columns,
            constants = compiled.constants.len(), "compiled ViewDefinition SQL"
        );
        let params = prepared
            .params
            .into_iter()
            .map(SqliteParam::from_runtime)
            .collect();
        let pool = self.pool.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<ViewRow, SofError>>(CHANNEL_BUFFER);
        let guard_tx = tx.clone();
        let producer = tokio::task::spawn_blocking(move || {
            stream_sqlite_rows(
                &pool,
                &compiled.sql,
                params,
                &compiled.columns,
                &compiled.column_decodes,
                prepared.client_limit,
                tx,
            );
        });
        watch_row_producer(self.runner_name(), guard_tx, producer);
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

/// Backend-only Group document loading. Acquisition, query preparation,
/// reads and JSON decoding all run outside the async runtime.
async fn load_group_documents(
    pool: Pool<SqliteConnectionManager>,
    tenant_id: String,
    group_refs: Vec<String>,
) -> Result<Vec<Value>, SofError> {
    tokio::task::spawn_blocking(move || {
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
        for reference in &group_refs {
            let id = reference.strip_prefix("Group/").unwrap_or(reference);
            let res: rusqlite::Result<Vec<u8>> =
                stmt.query_row([tenant_id.as_str(), id], |row| row.get(0));
            match res {
                Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) => groups.push(value),
                    Err(_) => continue,
                },
                Err(rusqlite::Error::QueryReturnedNoRows) => continue,
                Err(e) => {
                    return Err(SofError::Storage(format!(
                        "group lookup failed for {reference}: {e}"
                    )));
                }
            }
        }
        Ok(groups)
    })
    .await
    .map_err(|e| SofError::Storage(format!("sqlite group lookup task failed: {e}")))?
}

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
    fn from_runtime(value: RuntimeParam) -> Self {
        match value {
            RuntimeParam::Text(value) => Self::Text(value),
            RuntimeParam::Literal(value) => Self::from_lit(&value),
            RuntimeParam::Timestamp(value) => Self::Text(value.to_rfc3339()),
            RuntimeParam::TextList(values) => Self::Text(
                serde_json::to_string(&values).expect("a list of strings always serialises"),
            ),
        }
    }

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
    params: Vec<SqliteParam>,
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

    let row_iter = {
        match stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
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
    use super::super::runtime::SqlRunPlan;
    use super::*;
    use serde_json::json;

    fn runtime_sql(view: &Value, filters: &ViewFilters) -> (String, Vec<String>) {
        let run = SqlRunPlan::compile(view, SqlDialect::Sqlite, FhirVersion::default_enabled())
            .expect("compile test view")
            .finish("tenant", filters)
            .expect("runtime sql");
        let sql = run.query.sql;
        let params: Vec<_> = run
            .params
            .into_iter()
            .skip(2)
            .map(SqliteParam::from_runtime)
            .collect();
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
        assert!(
            unlimited.contains("r.id IN (SELECT value FROM json_each(?5))"),
            "{unlimited}"
        );
        assert_eq!(bindings[0], "text:male");
        assert_eq!(bindings.last().unwrap(), r#"text:["p-eligible"]"#);
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
            (qr(json!([{"unionAll":[repeat_item.clone()]}])), 1),
        ];
        let filters = ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p1".to_string()],
            ..Default::default()
        };
        for (view, scans) in views {
            let (sql, _) = runtime_sql(&view, &filters);
            assert!(!sql.contains("FROM rec_0 AND"), "{sql}");
            assert_eq!(sql.matches("r.last_updated >= ?3").count(), scans, "{sql}");
            let membership = if view["resource"] == "Patient" {
                "r.id IN (SELECT value FROM json_each(?4))"
            } else {
                "si.value_reference IN (SELECT value FROM json_each("
            };
            assert_eq!(sql.matches(membership).count(), scans, "{sql}");
        }
    }

    #[test]
    fn test_sqlite_compartment_filter_binds_one_parameter_for_any_number_of_refs() {
        let version = FhirVersion::default_enabled();
        for resource in ["Patient", "Observation"] {
            let view = json!({"resourceType":"ViewDefinition", "resource":resource,
                "select":[{"column":[{"path":"id","name":"id"}]}]});
            let filters = ViewFilters {
                patient: (0..5_000).map(|i| format!("Patient/p{i}")).collect(),
                ..Default::default()
            };
            let run = SqlRunPlan::compile(&view, SqlDialect::Sqlite, version)
                .unwrap()
                .finish("tenant", &filters)
                .unwrap();
            let sql = run.query.sql;
            let params: Vec<_> = run
                .params
                .into_iter()
                .skip(2)
                .map(SqliteParam::from_runtime)
                .collect();
            assert!(!sql.contains(" OR "), "{sql}");
            let (expected_params, first) = if resource == "Patient" {
                assert!(
                    sql.contains("r.id IN (SELECT value FROM json_each(?3))"),
                    "{sql}"
                );
                (1, "p0")
            } else {
                assert!(
                    sql.contains("si.value_reference IN (SELECT value FROM json_each(?"),
                    "{sql}"
                );
                (
                    helios_fhir::compartment_params(version, "Patient", resource).len() + 1,
                    "Patient/p0",
                )
            };
            assert_eq!(params.len(), expected_params, "{sql}");
            let Some(SqliteParam::Text(json)) = params.last() else {
                panic!("last param must be the JSON array: {params:?}");
            };
            let refs: Vec<String> = serde_json::from_str(json).unwrap();
            assert_eq!(refs.len(), 5_000);
            assert_eq!(refs[0], first);
        }
    }

    fn group_test_pool() -> Pool<SqliteConnectionManager> {
        let pool = Pool::builder()
            .max_size(1)
            .connection_timeout(std::time::Duration::from_secs(1))
            .build(SqliteConnectionManager::memory())
            .unwrap();
        pool.get()
            .unwrap()
            .execute_batch(
                "CREATE TABLE resources (
                tenant_id TEXT NOT NULL, resource_type TEXT NOT NULL, id TEXT NOT NULL,
                data BLOB NOT NULL, last_updated TEXT NOT NULL, is_deleted INTEGER NOT NULL
            )",
            )
            .unwrap();
        pool
    }

    #[tokio::test(flavor = "current_thread")]
    async fn group_pool_acquisition_does_not_block_the_async_runtime() {
        use futures::StreamExt;

        let pool = group_test_pool();
        let connection = pool.get().unwrap();
        for resource in [
            json!({"resourceType":"Patient","id":"p1"}),
            json!({"resourceType":"Group","id":"g1","member":[{"entity":{"reference":"Patient/p1"}}]}),
        ] {
            connection
                .execute(
                    "INSERT INTO resources VALUES (?1,?2,?3,?4,?5,0)",
                    rusqlite::params![
                        "tenant",
                        resource["resourceType"].as_str().unwrap(),
                        resource["id"].as_str().unwrap(),
                        serde_json::to_vec(&resource).unwrap(),
                        "2024-01-01T00:00:00+00:00"
                    ],
                )
                .unwrap();
        }
        let runner = SqliteInDbRunner::new(pool);
        let task = tokio::spawn(async move {
            let tenant = TenantContext::new(
                crate::tenant::TenantId::new("tenant"),
                crate::tenant::TenantPermissions::full_access(),
            );
            runner
                .run_view(
                    &tenant,
                    flat_view(),
                    ViewFilters {
                        group: vec!["Group/g1".into()],
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await
        });
        // Let run_view reach group acquisition while this test owns the pool's
        // only connection. A synchronous pool.get would exhaust its timeout
        // before the current-thread runtime could execute this timer.
        tokio::task::yield_now().await;
        let started = std::time::Instant::now();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert!(
            !task.is_finished(),
            "group lookup is waiting for our connection"
        );
        drop(connection);
        let rows = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].as_ref().unwrap(), &json!({"id":"p1"}));
    }

    #[tokio::test]
    async fn group_documents_are_tenant_scoped_and_deleted_groups_are_ignored() {
        let pool = group_test_pool();
        {
            let connection = pool.get().unwrap();
            for (tenant, id, deleted) in [
                ("tenant", "live", 0),
                ("other", "foreign", 0),
                ("tenant", "deleted", 1),
            ] {
                let group = json!({"resourceType":"Group","id":id,"member":[{"entity":{"reference":"Patient/p1"}}]});
                connection
                    .execute(
                        "INSERT INTO resources VALUES (?1,'Group',?2,?3,'2024-01-01',?4)",
                        rusqlite::params![tenant, id, serde_json::to_vec(&group).unwrap(), deleted],
                    )
                    .unwrap();
            }
        }
        let groups = load_group_documents(
            pool,
            "tenant".into(),
            vec![
                "live".into(),
                "Group/foreign".into(),
                "Group/deleted".into(),
                "Group/missing".into(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["id"], "live");
    }
}
