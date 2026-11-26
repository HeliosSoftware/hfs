//! SQL Query Libraries run inside the primary database (#2902): one suite for
//! every backend that speaks SQL. `SofRunner::run_sql_query` gets the Library's
//! SQL and its dependency graph and must answer from the backend's own tables,
//! in one read-only statement.
//!
//! The backend adapters (`sof_sqlite_runner.rs`, `sof_pg_runner.rs`) create
//! the storage and pass it here; every scenario seeds its own pair of tenants.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::sof_runner::{
    NativeRelation, NativeRelationSource, NativeSqlQuery, SofRunner, ViewFilters,
};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_sof::sqlquery::{
    BoundParam, ColumnFhirType, NativeDialect, QueryResult, SqlQueryError, SqlValue,
};
use serde_json::{Value, json};

fn tenant(name: &str) -> TenantContext {
    TenantContext::new(TenantId::new(name), TenantPermissions::full_access())
}

fn patient_view() -> Value {
    json!({
        "resourceType": "ViewDefinition", "name": "patient_flat", "resource": "Patient",
        "status": "active",
        "select": [{"column": [
            {"name": "id", "path": "id", "type": "id"},
            {"name": "gender", "path": "gender", "type": "code"},
            {"name": "active", "path": "active", "type": "boolean"},
            {"name": "birthDate", "path": "birthDate", "type": "date"}
        ]}]
    })
}

fn observation_view() -> Value {
    json!({
        "resourceType": "ViewDefinition", "name": "observation_flat", "resource": "Observation",
        "status": "active",
        "select": [{"column": [
            {"name": "id", "path": "id", "type": "id"},
            {"name": "patient_id", "path": "subject.getReferenceKey(Patient)", "type": "string"},
            {"name": "code", "path": "code.coding.first().code", "type": "code"},
            {"name": "value", "path": "value.ofType(Quantity).value", "type": "decimal"},
            {"name": "rank", "path": "extension('http://example.org/rank').value.ofType(integer)", "type": "integer"}
        ]}]
    })
}

fn view(name: &str, labels: &[&str], view: Value) -> NativeRelation {
    NativeRelation {
        name: name.to_string(),
        display: view["name"].as_str().unwrap_or(name).to_string(),
        labels: labels.iter().map(|l| (*l).to_string()).collect(),
        source: NativeRelationSource::View(view),
    }
}

fn standard_relations() -> Vec<NativeRelation> {
    vec![
        view("__sof_node_0", &["p"], patient_view()),
        view("__sof_node_1", &["o"], observation_view()),
    ]
}

fn query(relations: Vec<NativeRelation>, sql: &str) -> NativeSqlQuery {
    NativeSqlQuery {
        relations,
        sql: sql.to_string(),
        bindings: Vec::new(),
        filters: ViewFilters::default(),
        max_rows: 1000,
        timeout: Duration::from_secs(30),
    }
}

/// Rows as JSON objects, in result order.
fn objects(result: &QueryResult) -> Vec<Value> {
    result
        .rows
        .iter()
        .map(|row| {
            Value::Object(
                result
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(name, cell)| (name.clone(), cell.clone().unwrap_or(Value::Null)))
                    .collect(),
            )
        })
        .collect()
}

/// Three patients and five observations under `main`, and one patient with one
/// observation under `other`, whose values no query run as `main` may see.
async fn seed<S: ResourceStorage>(storage: &S, main: &TenantContext, other: &TenantContext) {
    for (id, gender, active, born) in [
        ("p1", "female", true, "1980-01-02"),
        ("p2", "male", false, "1990"),
        ("p3", "female", true, "2001-11"),
    ] {
        storage
            .create(
                main,
                "Patient",
                json!({"resourceType": "Patient", "id": id, "gender": gender, "active": active,
                       "birthDate": born}),
                FhirVersion::R4,
            )
            .await
            .expect("seed patient");
    }
    for (id, patient, code, value, rank) in [
        ("o1", "p1", "8302-2", 170.5, 1),
        ("o2", "p1", "8302-2", 171.0, 2),
        ("o3", "p2", "8302-2", 180.25, 3),
        ("o4", "p3", "29463-7", 62.0, 4),
        ("o5", "p3", "8302-2", 149.0, 5),
    ] {
        storage
            .create(
                main,
                "Observation",
                json!({"resourceType": "Observation", "id": id, "status": "final",
                       "subject": {"reference": format!("Patient/{patient}")},
                       "code": {"coding": [{"system": "http://loinc.org", "code": code}]},
                       "valueQuantity": {"value": value, "unit": "cm"},
                       "extension": [{"url": "http://example.org/rank", "valueInteger": rank}]}),
                FhirVersion::R4,
            )
            .await
            .expect("seed observation");
    }
    storage
        .create(
            other,
            "Patient",
            json!({"resourceType": "Patient", "id": "secret", "gender": "other", "active": true}),
            FhirVersion::R4,
        )
        .await
        .expect("seed other tenant");
    storage
        .create(
            other,
            "Observation",
            json!({"resourceType": "Observation", "id": "secret-obs", "status": "final",
                   "subject": {"reference": "Patient/secret"},
                   "code": {"coding": [{"system": "http://loinc.org", "code": "8302-2"}]},
                   "valueQuantity": {"value": 999.0, "unit": "cm"}}),
            FhirVersion::R4,
        )
        .await
        .expect("seed other tenant");
}

async fn run(
    runner: &Arc<dyn SofRunner>,
    tenant: &TenantContext,
    query: NativeSqlQuery,
) -> QueryResult {
    let sql = query.sql.clone();
    runner
        .run_sql_query(tenant, query)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn refused(
    runner: &Arc<dyn SofRunner>,
    tenant: &TenantContext,
    query: NativeSqlQuery,
) -> SqlQueryError {
    let sql = query.sql.clone();
    match runner.run_sql_query(tenant, query).await {
        Ok(result) => panic!("{sql}: expected a refusal, got {:?}", objects(&result)),
        Err(e) => e,
    }
}

/// Every scenario, against `storage` and its `runner`. `prefix` keeps this
/// run's tenants apart from any other test's in a shared database.
pub async fn run_suite<S: ResourceStorage>(storage: &S, runner: Arc<dyn SofRunner>, prefix: &str) {
    let dialect = runner
        .native_sql_dialect()
        .expect("a SQL backend runs SQL Queries itself");
    let main = tenant(&format!("{prefix}-main"));
    let other = tenant(&format!("{prefix}-other"));
    seed(storage, &main, &other).await;

    joins_aggregates_and_typed_columns(&runner, &main, dialect).await;
    parameters_are_bound(&runner, &main).await;
    only_the_callers_tenant_is_read(&runner, &main, &other).await;
    sql_views_and_supplied_rows_are_relations(&runner, &main).await;
    request_filters_narrow_every_view(&runner, &main).await;
    the_result_is_capped_not_refused(&runner, &main).await;
    stored_tables_and_unsafe_sql_are_refused(&runner, &main, dialect).await;
    a_statement_past_its_time_limit_is_stopped(&runner, &main).await;
    // Whatever the statements above left on a pooled connection, the next
    // query runs normally.
    let result = run(
        &runner,
        &main,
        query(standard_relations(), "SELECT count(*) AS n FROM p"),
    )
    .await;
    assert_eq!(objects(&result), vec![json!({"n": 3})]);
}

async fn joins_aggregates_and_typed_columns(
    runner: &Arc<dyn SofRunner>,
    main: &TenantContext,
    dialect: NativeDialect,
) {
    // A join, a filter on a decimal column, an aggregate and an ORDER BY, all
    // evaluated by the database over the two views.
    let result = run(
        runner,
        main,
        query(
            standard_relations(),
            "SELECT p.id AS id, count(*) AS n, max(o.value) AS top, sum(o.rank) AS ranks \
             FROM p JOIN o ON o.patient_id = p.id \
             WHERE o.code = '8302-2' AND o.value > 150 \
             GROUP BY p.id ORDER BY p.id",
        ),
    )
    .await;
    assert_eq!(result.columns, vec!["id", "n", "top", "ranks"]);
    assert_eq!(
        objects(&result),
        vec![
            json!({"id": "p1", "n": 2, "top": 171.0, "ranks": 3}),
            json!({"id": "p2", "n": 1, "top": 180.25, "ranks": 3}),
        ]
    );
    assert_eq!(result.column_types[1], ColumnFhirType::Integer);
    assert_eq!(result.column_types[2], ColumnFhirType::Decimal);

    // A boolean column is a boolean of the database: PostgreSQL's own type,
    // SQLite's 1/0.
    let active = match dialect {
        NativeDialect::Postgres => {
            "SELECT id FROM p WHERE active AND gender = 'female' ORDER BY id"
        }
        NativeDialect::Sqlite => {
            "SELECT id FROM p WHERE active = 1 AND gender = 'female' ORDER BY id"
        }
    };
    let result = run(runner, main, query(standard_relations(), active)).await;
    assert_eq!(
        objects(&result),
        vec![json!({"id": "p1"}), json!({"id": "p3"})]
    );

    // A date stays text (a FHIR date may be partial), and a column name keeps
    // its case.
    let result = run(
        runner,
        main,
        query(
            standard_relations(),
            "SELECT id, \"birthDate\" FROM p WHERE \"birthDate\" >= '1990' ORDER BY \"birthDate\"",
        ),
    )
    .await;
    assert_eq!(
        objects(&result),
        vec![
            json!({"id": "p2", "birthDate": "1990"}),
            json!({"id": "p3", "birthDate": "2001-11"}),
        ]
    );

    // The query's own CTEs, a recursive one included, sit beside the views.
    let result = run(
        runner,
        main,
        query(
            standard_relations(),
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3), \
             tall AS (SELECT patient_id FROM o WHERE value > 170) \
             SELECT (SELECT count(*) FROM n) AS steps, count(DISTINCT patient_id) AS patients FROM tall",
        ),
    )
    .await;
    assert_eq!(objects(&result), vec![json!({"steps": 3, "patients": 2})]);

    // A label is found whatever its case, and one view may have two labels.
    let relations = vec![view("__sof_node_0", &["Pat", "people"], patient_view())];
    let result = run(
        runner,
        main,
        query(
            relations,
            "SELECT (SELECT count(*) FROM pat) + (SELECT count(*) FROM people) AS n",
        ),
    )
    .await;
    assert_eq!(objects(&result), vec![json!({"n": 6})]);
}

async fn parameters_are_bound(runner: &Arc<dyn SofRunner>, main: &TenantContext) {
    let mut q = query(
        standard_relations(),
        "SELECT o.id FROM o WHERE o.value > :min_height AND o.code = :code AND o.rank <= :top \
         AND o.value > :min_height - 1 ORDER BY o.id",
    );
    q.bindings = vec![
        BoundParam {
            name: "unused".to_string(),
            value: SqlValue::Text("x".to_string()),
            fhir_type: "string".to_string(),
        },
        BoundParam {
            name: "min_height".to_string(),
            value: SqlValue::Real(160.0),
            fhir_type: "decimal".to_string(),
        },
        BoundParam {
            name: "code".to_string(),
            // A value that would end a string literal if it were pasted in.
            value: SqlValue::Text("8302-2".to_string()),
            fhir_type: "code".to_string(),
        },
        BoundParam {
            name: "top".to_string(),
            value: SqlValue::Integer(2),
            fhir_type: "integer".to_string(),
        },
    ];
    let result = run(runner, main, q.clone()).await;
    assert_eq!(
        objects(&result),
        vec![json!({"id": "o1"}), json!({"id": "o2"})]
    );

    q.bindings[2].value = SqlValue::Text("8302-2' OR '1'='1".to_string());
    let result = run(runner, main, q).await;
    assert!(result.rows.is_empty(), "{:?}", objects(&result));
}

async fn only_the_callers_tenant_is_read(
    runner: &Arc<dyn SofRunner>,
    main: &TenantContext,
    other: &TenantContext,
) {
    let sql = "SELECT (SELECT count(*) FROM p) AS patients, (SELECT max(value) FROM o) AS top";
    let result = run(runner, main, query(standard_relations(), sql)).await;
    assert_eq!(
        objects(&result),
        vec![json!({"patients": 3, "top": 180.25})]
    );
    let result = run(runner, other, query(standard_relations(), sql)).await;
    assert_eq!(objects(&result), vec![json!({"patients": 1, "top": 999.0})]);
}

async fn sql_views_and_supplied_rows_are_relations(
    runner: &Arc<dyn SofRunner>,
    main: &TenantContext,
) {
    let mut relations = standard_relations();
    relations.push(NativeRelation {
        name: "__sof_node_2".to_string(),
        display: "female_patients".to_string(),
        labels: vec!["fp".to_string()],
        source: NativeRelationSource::Sql(
            "WITH f AS (SELECT id FROM p WHERE gender = 'female') SELECT id FROM f".to_string(),
        ),
    });
    relations.push(NativeRelation {
        name: "__sof_node_3".to_string(),
        display: "http://example.org/ValueSet/heights".to_string(),
        labels: vec!["heights".to_string()],
        source: NativeRelationSource::Rows {
            columns: vec![
                ("system".to_string(), "uri".to_string()),
                ("code".to_string(), "code".to_string()),
            ],
            rows: vec![
                json!({"system": "http://loinc.org", "code": "8302-2"}),
                json!({"system": "http://loinc.org", "code": "it's"}),
            ],
        },
    });
    let result = run(
        runner,
        main,
        query(
            relations,
            "SELECT fp.id AS id, max(o.value) AS height FROM fp \
             JOIN o ON o.patient_id = fp.id JOIN heights h ON h.code = o.code \
             GROUP BY fp.id ORDER BY fp.id",
        ),
    )
    .await;
    assert_eq!(
        objects(&result),
        vec![
            json!({"id": "p1", "height": 171.0}),
            json!({"id": "p3", "height": 149.0}),
        ]
    );
}

/// `patient` (and `group`, `_since`) narrow every view of the query exactly as
/// they narrow the view run on its own.
async fn request_filters_narrow_every_view(runner: &Arc<dyn SofRunner>, main: &TenantContext) {
    use futures::StreamExt as _;

    let filters = ViewFilters {
        patient: vec!["Patient/p1".to_string()],
        ..ViewFilters::default()
    };
    let mut expected = Vec::new();
    for view in [patient_view(), observation_view()] {
        let rows: Vec<_> = runner
            .run_view(main, view, filters.clone())
            .await
            .expect("run view")
            .collect()
            .await;
        expected.push(rows.len());
    }
    assert_eq!(expected[0], 1, "the patient view under patient=p1");

    let mut q = query(
        standard_relations(),
        "SELECT (SELECT count(*) FROM p) AS patients, (SELECT count(*) FROM o) AS observations",
    );
    q.filters = filters;
    // `limit` belongs to the result, never to a view.
    q.filters.limit = Some(1);
    let result = run(runner, main, q).await;
    assert_eq!(
        objects(&result),
        vec![json!({"patients": expected[0], "observations": expected[1]})]
    );
}

async fn the_result_is_capped_not_refused(runner: &Arc<dyn SofRunner>, main: &TenantContext) {
    let mut q = query(standard_relations(), "SELECT id FROM o ORDER BY id");
    q.max_rows = 2;
    let result = run(runner, main, q).await;
    assert_eq!(
        objects(&result),
        vec![json!({"id": "o1"}), json!({"id": "o2"})]
    );
}

async fn stored_tables_and_unsafe_sql_are_refused(
    runner: &Arc<dyn SofRunner>,
    main: &TenantContext,
    dialect: NativeDialect,
) {
    // The tables that hold every tenant's resources are not readable, however
    // they are named.
    for sql in [
        "SELECT * FROM resources",
        "SELECT data FROM resources WHERE tenant_id <> 'x'",
        "SELECT * FROM p WHERE EXISTS (SELECT 1 FROM resources)",
        "SELECT * FROM public.resources",
        "SELECT * FROM main.resources",
        "SELECT * FROM search_index",
        "SELECT * FROM __sof_raw_0",
        "WITH x AS (SELECT * FROM resources) SELECT * FROM x",
        "WITH a AS (SELECT * FROM resources), resources AS (SELECT 1 AS z) SELECT * FROM a",
        "SELECT * FROM p UNION ALL SELECT id, NULL, NULL, NULL FROM resources",
    ] {
        let error = refused(runner, main, query(standard_relations(), sql)).await;
        assert!(
            matches!(error, SqlQueryError::Unsupported(_)),
            "{sql}: {error:?}"
        );
    }
    // Neither is anything that is not one plain query.
    for sql in [
        "DELETE FROM p",
        "SELECT 1; SELECT 2",
        "SELECT * FROM p FOR UPDATE",
        "SELECT id FROM p WHERE id = ?1",
        "SELECT id FROM p WHERE id = $1",
        "SELECT id FROM p WHERE id = :undeclared",
    ] {
        let error = refused(runner, main, query(standard_relations(), sql)).await;
        assert!(
            matches!(
                error,
                SqlQueryError::Unsupported(_)
                    | SqlQueryError::NotSelect(_)
                    | SqlQueryError::BindParameter(_)
            ),
            "{sql}: {error:?}"
        );
    }
    let by_dialect: &[&str] = match dialect {
        NativeDialect::Postgres => &[
            "SELECT pg_sleep(0)",
            "SELECT current_setting('server_version')",
            "SELECT set_config('statement_timeout', '0', true)",
            "SELECT query_to_xml('select 1', true, false, '')",
            "SELECT * FROM pg_catalog.pg_tables",
            "SELECT * FROM pg_tables",
            "SELECT current_user",
        ],
        NativeDialect::Sqlite => &[
            "SELECT * FROM sqlite_master",
            "SELECT * FROM pragma_table_info('resources')",
            "SELECT load_extension('x')",
        ],
    };
    for sql in by_dialect {
        let error = refused(runner, main, query(standard_relations(), sql)).await;
        assert!(
            matches!(
                error,
                SqlQueryError::Unsupported(_) | SqlQueryError::NotSelect(_)
            ),
            "{sql}: {error:?}"
        );
    }

    // A label may not take the name of a stored table.
    let relations = vec![view("__sof_node_0", &["resources"], patient_view())];
    let error = refused(runner, main, query(relations, "SELECT * FROM resources")).await;
    assert!(
        matches!(error, SqlQueryError::MalformedLibrary(_)),
        "{error:?}"
    );

    // SQL the database itself rejects is the caller's error, with the
    // database's message.
    let error = refused(
        runner,
        main,
        query(standard_relations(), "SELECT nope FROM p"),
    )
    .await;
    assert!(
        matches!(error, SqlQueryError::Database(_) | SqlQueryError::Sqlite(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("nope"), "{error}");
}

async fn a_statement_past_its_time_limit_is_stopped(
    runner: &Arc<dyn SofRunner>,
    main: &TenantContext,
) {
    let mut q = query(
        standard_relations(),
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n",
    );
    q.timeout = Duration::from_secs(1);
    let start = Instant::now();
    let error = refused(runner, main, q).await;
    assert!(
        matches!(error, SqlQueryError::Timeout { secs: 1 }),
        "{error:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
}
