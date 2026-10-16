//! Parity helpers for the columnar SoF runner path.
//!
//! Included with `#[path = "common/sof_columnar.rs"] mod sof_columnar;` by the
//! runner test binaries. Each helper runs a view through both
//! [`SofRunner::run_view`] (rows, then the buffered Arrow formatter) and
//! [`SofRunner::run_view_columnar`], and compares what comes out.
//!
//! Values are compared by their rendered text, not by Arrow type: the
//! columnar path types columns from the view, while the row path votes on the
//! first 100 values, so e.g. an all-null `integer` column is `Int32` on one
//! side and `Utf8` on the other. Use [`column_types`] to pin types explicitly.

#![allow(dead_code)]

use futures::StreamExt;
use helios_persistence::core::{SofRunner, ViewFilters};
use helios_persistence::tenant::TenantContext;
use helios_sof::arrow::array::{Array, ArrayRef};
use helios_sof::arrow::datatypes::DataType;
use helios_sof::arrow::record_batch::RecordBatch;
use helios_sof::arrow::util::display::array_value_to_string;
use serde_json::Value;
use std::sync::Arc;

/// One result cell rendered as text; `None` is null.
pub type Cell = Option<String>;

/// Rows produced by the row path, rendered through the buffered Arrow
/// formatter, with the column names in order.
pub async fn row_path_cells(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    filters: &ViewFilters,
) -> (Vec<String>, Vec<Vec<Cell>>) {
    let mut stream = runner
        .run_view(tenant, view.clone(), filters.clone())
        .await
        .expect("run_view");
    let mut rows = Vec::new();
    while let Some(item) = stream.next().await {
        rows.push(item.expect("row"));
    }
    let result = helios_sof::rows_to_processed_result(rows);
    let schema = helios_sof::parquet_schema::create_arrow_schema(&result.columns, &result.rows)
        .expect("schema");
    let arrays =
        helios_sof::parquet_schema::process_to_arrow_arrays(&schema, &result.columns, &result.rows)
            .expect("arrays");
    (result.columns.clone(), cells_of(&arrays, result.rows.len()))
}

/// Batches produced by the columnar path. Panics if the runner has no
/// columnar path for the view (`Ok(None)`).
pub async fn columnar_batches(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    filters: &ViewFilters,
) -> Vec<RecordBatch> {
    let mut stream = runner
        .run_view_columnar(tenant, view.clone(), filters.clone())
        .await
        .expect("run_view_columnar")
        .expect("runner has no columnar path for this view");
    let mut batches = Vec::new();
    while let Some(item) = stream.next().await {
        batches.push(item.expect("batch"));
    }
    batches
}

/// Asserts the columnar path yields the same column names, row count, row
/// order and cell values as the row path.
///
/// Returns the batches so callers can add type assertions.
pub async fn assert_columnar_parity(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    filters: &ViewFilters,
    ctx: &str,
) -> Vec<RecordBatch> {
    let (expected_columns, expected) = row_path_cells(runner, tenant, view, filters).await;
    let batches = columnar_batches(runner, tenant, view, filters).await;

    if expected.is_empty() {
        let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
        assert_eq!(
            rows, 0,
            "{ctx}: row path is empty, columnar has {rows} rows"
        );
        return batches;
    }

    let mut actual: Vec<Vec<Cell>> = Vec::new();
    for batch in &batches {
        let schema = batch.schema();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names, expected_columns,
            "{ctx}: column names or order differ"
        );
        actual.extend(cells_of(batch.columns(), batch.num_rows()));
    }
    if let Some(first) = batches.first() {
        let schema = first.schema();
        assert!(
            batches.iter().all(|b| b.schema() == schema),
            "{ctx}: batches do not share one schema"
        );
    }
    assert_eq!(actual.len(), expected.len(), "{ctx}: row count differs");
    for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(a, e, "{ctx}: row {i} differs");
    }
    batches
}

/// The Arrow type of each column of the first batch, by name.
pub fn column_types(batches: &[RecordBatch]) -> Vec<(String, DataType)> {
    batches
        .first()
        .map(|b| {
            b.schema()
                .fields()
                .iter()
                .map(|f| (f.name().clone(), f.data_type().clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn cells_of(columns: &[ArrayRef], rows: usize) -> Vec<Vec<Cell>> {
    (0..rows)
        .map(|r| {
            columns
                .iter()
                .map(|c| {
                    if c.is_null(r) {
                        None
                    } else if let DataType::Timestamp(_, Some(_)) = c.data_type() {
                        // Arrow cannot render a named zone ("UTC") without chrono-tz.
                        use helios_sof::arrow::array::AsArray;
                        use helios_sof::arrow::datatypes::TimestampMicrosecondType;
                        Some(format!(
                            "{}us-utc",
                            c.as_primitive::<TimestampMicrosecondType>().value(r)
                        ))
                    } else {
                        Some(array_value_to_string(c, r).expect("render cell"))
                    }
                })
                .collect()
        })
        .collect()
}

/// Row path with the view's declared types applied (what the buffered
/// formatters do): returns the Arrow schema and the rendered cells.
pub async fn row_path_typed(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    filters: &ViewFilters,
) -> (helios_sof::arrow::datatypes::Schema, Vec<Vec<Cell>>) {
    let mut stream = runner
        .run_view(tenant, view.clone(), filters.clone())
        .await
        .expect("run_view");
    let mut rows = Vec::new();
    while let Some(item) = stream.next().await {
        rows.push(item.expect("row"));
    }
    let parsed =
        helios_sof::parse_view_definition_for_version(view.clone(), helios_fhir::FhirVersion::R4)
            .expect("parse view");
    let declared = parsed.try_declared_columns().expect("declared columns");
    let result = helios_sof::rows_to_processed_result(rows);
    let schema =
        helios_sof::parquet_schema::resolve_arrow_schema(&result.columns, &declared, &result.rows)
            .expect("schema");
    let arrays =
        helios_sof::parquet_schema::process_to_arrow_arrays(&schema, &result.columns, &result.rows)
            .expect("arrays");
    let cells = cells_of(&arrays, result.rows.len());
    (schema, cells)
}

/// Asserts the row path and the columnar path agree on the Arrow schema
/// (name, type and field metadata, including `helios.sof.declared`) and on
/// every cell. Returns the columnar batches.
pub async fn assert_typed_parity(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    ctx: &str,
) -> Vec<RecordBatch> {
    let filters = ViewFilters::default();
    let (row_schema, row_cells) = row_path_typed(runner, tenant, view, &filters).await;
    let batches = columnar_batches(runner, tenant, view, &filters).await;
    assert!(!batches.is_empty(), "{ctx}: no columnar batches");
    let col_schema = batches[0].schema();
    assert_eq!(
        row_schema.fields().len(),
        col_schema.fields().len(),
        "{ctx}: column count"
    );
    for (r, c) in row_schema.fields().iter().zip(col_schema.fields()) {
        assert_eq!(r.name(), c.name(), "{ctx}: column name");
        assert_eq!(r.data_type(), c.data_type(), "{ctx}: type of {}", r.name());
        assert_eq!(
            r.metadata().get("helios.sof.declared"),
            c.metadata().get("helios.sof.declared"),
            "{ctx}: declared flag of {}",
            r.name()
        );
    }
    let mut cells = Vec::new();
    for b in &batches {
        cells.extend(cells_of(b.columns(), b.num_rows()));
    }
    assert_eq!(cells, row_cells, "{ctx}: cells differ");
    batches
}

/// Resources every typed-column test seeds: one Patient and one Observation.
pub fn typed_fixture() -> Vec<Value> {
    vec![
        serde_json::json!({
            "resourceType": "Patient", "id": "tp1",
            "birthDate": "1980-05-17",
            "name": [{"family": "Smith"}],
            "photo": [{"contentType": "image/png", "data": "aGVsbG8="}]
        }),
        serde_json::json!({
            "resourceType": "Observation", "id": "to1",
            "status": "final", "code": {"text": "x"},
            "issued": "2024-01-02T03:04:05.123+02:00",
            "effectiveDateTime": "2024-01-02T03:04:05",
            "valueTime": "13:45:30",
            "component": [
                {"code": {"text": "a"}, "valueInteger": 1},
                {"code": {"text": "b"}, "valueInteger": 2}
            ],
            "referenceRange": [{"low": {"value": 1.5}}]
        }),
    ]
}

pub fn patient_typed_view() -> Value {
    serde_json::json!({
        "resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
        "select": [{"column": [
            {"path": "id", "name": "id"},
            {"path": "photo.first().data", "name": "photo", "type": "base64Binary"},
            {"path": "birthDate", "name": "dob", "type": "date"},
            {"path": "birthDate", "name": "dob_d", "type": "date",
             "tag": [{"name": "ansi/type", "value": "DATE"}]},
            {"path": "birthDate", "name": "dob_tag_only",
             "tag": [{"name": "ansi/type", "value": "DATE"}]}
        ]}]
    })
}

pub fn observation_typed_view() -> Value {
    serde_json::json!({
        "resourceType": "ViewDefinition", "resource": "Observation", "status": "active",
        "select": [{"column": [
            {"path": "id", "name": "id"},
            {"path": "issued", "name": "at", "type": "instant"},
            {"path": "value.ofType(time)", "name": "t", "type": "time",
             "tag": [{"name": "ansi/type", "value": "TIME"}]},
            {"path": "effective.ofType(dateTime)", "name": "ts", "type": "dateTime",
             "tag": [{"name": "ansi/type", "value": "TIMESTAMP"}]},
            {"path": "component.valueInteger", "name": "ints",
             "type": "integer", "collection": true},
            {"path": "component.first().value.ofType(integer)", "name": "first_int",
             "type": "integer"},
            {"path": "referenceRange.low.value", "name": "dec", "type": "decimal"},
            {"path": "referenceRange.low.value", "name": "dbl", "type": "decimal",
             "tag": [{"name": "ansi/type", "value": "DOUBLE PRECISION"}]}
        ]}]
    })
}

/// Asserts the values and types of the typed Patient view's batches.
pub fn assert_patient_typed(batches: &[RecordBatch]) {
    use helios_sof::arrow::array::{Array, AsArray};
    use helios_sof::arrow::datatypes::Date32Type;
    let b = &batches[0];
    assert_eq!(b.num_rows(), 1);
    let ty = |n: &str| b.schema().field_with_name(n).unwrap().data_type().clone();
    assert_eq!(ty("id"), DataType::Utf8);
    assert_eq!(ty("photo"), DataType::Binary);
    assert_eq!(ty("dob"), DataType::Utf8);
    assert_eq!(ty("dob_d"), DataType::Date32);
    assert_eq!(ty("dob_tag_only"), DataType::Date32);
    assert_eq!(
        b.column_by_name("photo")
            .unwrap()
            .as_binary::<i32>()
            .value(0),
        b"hello"
    );
    assert_eq!(
        b.column_by_name("dob").unwrap().as_string::<i32>().value(0),
        "1980-05-17"
    );
    // 1980-05-17 is 3789 days after the epoch.
    for n in ["dob_d", "dob_tag_only"] {
        let a = b.column_by_name(n).unwrap().as_primitive::<Date32Type>();
        assert!(!a.is_null(0));
        assert_eq!(a.value(0), 3789, "{n}");
    }
}

/// Asserts the values and types of the typed Observation view's batches.
pub fn assert_observation_typed(batches: &[RecordBatch]) {
    use helios_sof::arrow::array::AsArray;
    use helios_sof::arrow::datatypes::{
        Float64Type, Int32Type, Time64MicrosecondType, TimeUnit, TimestampMicrosecondType,
    };
    let b = &batches[0];
    assert_eq!(b.num_rows(), 1);
    let ty = |n: &str| b.schema().field_with_name(n).unwrap().data_type().clone();
    assert_eq!(
        ty("at"),
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(ty("t"), DataType::Time64(TimeUnit::Microsecond));
    assert_eq!(ty("ts"), DataType::Timestamp(TimeUnit::Microsecond, None));
    assert_eq!(
        ty("ints"),
        DataType::List(Arc::new(helios_sof::arrow::datatypes::Field::new(
            "item",
            DataType::Int32,
            true
        )))
        .clone()
    );
    assert_eq!(ty("first_int"), DataType::Int32);
    assert_eq!(ty("dec"), DataType::Utf8);
    assert_eq!(ty("dbl"), DataType::Float64);
    // 2024-01-02T01:04:05.123Z
    let at = b
        .column_by_name("at")
        .unwrap()
        .as_primitive::<TimestampMicrosecondType>();
    assert_eq!(at.value(0), 1_704_157_445_123_000);
    let t = b
        .column_by_name("t")
        .unwrap()
        .as_primitive::<Time64MicrosecondType>();
    assert_eq!(t.value(0), (13 * 3600 + 45 * 60 + 30) * 1_000_000);
    let ts = b
        .column_by_name("ts")
        .unwrap()
        .as_primitive::<TimestampMicrosecondType>();
    assert_eq!(ts.value(0), 1_704_164_645_000_000); // 2024-01-02T03:04:05
    let list = b.column_by_name("ints").unwrap().as_list::<i32>();
    let inner = list.value(0);
    let inner = inner.as_primitive::<Int32Type>();
    assert_eq!(
        inner.values().to_vec(),
        vec![1, 2],
        "{:?}",
        b.column_by_name("ints")
    );
    assert_eq!(
        b.column_by_name("first_int")
            .unwrap()
            .as_primitive::<Int32Type>()
            .value(0),
        1
    );
    assert_eq!(
        b.column_by_name("dec").unwrap().as_string::<i32>().value(0),
        "1.5"
    );
    assert_eq!(
        b.column_by_name("dbl")
            .unwrap()
            .as_primitive::<Float64Type>()
            .value(0),
        1.5
    );
}

/// Strictness and tag-rejection checks shared by both backends.
pub async fn assert_strict_and_rejected(runner: &dyn SofRunner, tenant: &TenantContext) {
    let f = ViewFilters::default();
    // Declared columns are strict: misfit fails naming the column.
    let strict = [
        // NOTE: a declared `integer` over text ("name.family") is not asserted
        // here: the in-DB runners cast in SQL, so the database decides (SQLite
        // casts text to 0, PostgreSQL fails the query without naming the
        // column). Strictness of the columnar builder itself is covered by
        // `columnar::tests::declared_column_misfit_is_an_error_but_undeclared_is_null`
        // and the REST `declared_integer_over_a_value_beyond_int32_is_an_error`.
        ("baddate", "birthDate", "date", Some("DATE")),
    ];
    for (col, path, ty, tag) in strict {
        let mut c = serde_json::json!({"path": path, "name": col, "type": ty});
        if let Some(t) = tag {
            c["tag"] = serde_json::json!([{"name": "ansi/type", "value": t}]);
        }
        let view = serde_json::json!({
            "resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [{"column": [c]}]
        });
        let msg = match runner.run_view_columnar(tenant, view, f.clone()).await {
            Err(e) => e.to_string(),
            Ok(None) => panic!("{col}: no columnar path"),
            Ok(Some(mut s)) => {
                let mut err = None;
                while let Some(item) = s.next().await {
                    if let Err(e) = item {
                        err = Some(e.to_string());
                        break;
                    }
                }
                err.unwrap_or_else(|| panic!("{col}: misfit was accepted"))
            }
        };
        assert!(
            msg.contains(col),
            "{col}: error must name the column: {msg}"
        );
    }
    // Inferred (undeclared) columns are lenient: a misfit is null.
    // Declared invalid tags are rejected on both paths, naming the column.
    let invalid = [
        ("unknown", serde_json::json!("NOT_A_TYPE")),
        ("incompat", serde_json::json!("INT")),
        ("nonstring", serde_json::json!(42)),
    ];
    for (col, value) in invalid {
        let ty = if col == "incompat" {
            "decimal"
        } else {
            "string"
        };
        let view = serde_json::json!({
            "resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
            "select": [{"column": [
                {"path": "id", "name": col, "type": ty,
                 "tag": [{"name": "ansi/type", "value": value}]}
            ]}]
        });
        let row = runner.run_view(tenant, view.clone(), f.clone()).await;
        let row_err = match row {
            Err(e) => e.to_string(),
            Ok(_) => panic!("{col}: run_view accepted an invalid tag"),
        };
        // A non-string tag value fails in serde before the column is known;
        // the other two must name it.
        if col != "nonstring" {
            assert!(row_err.contains(col), "{col}: run_view error: {row_err}");
        }
        let columnar = runner.run_view_columnar(tenant, view, f.clone()).await;
        let col_err = match columnar {
            Err(e) => e.to_string(),
            Ok(_) => panic!("{col}: run_view_columnar accepted an invalid tag"),
        };
        if col != "nonstring" {
            assert!(col_err.contains(col), "{col}: columnar error: {col_err}");
        }
    }
}

/// Runs `view` on the row path and returns its rows, or the error text of
/// the first failure (raised when starting the run or while streaming).
pub async fn row_outcome(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
) -> Result<Vec<Value>, String> {
    let mut stream = runner
        .run_view(tenant, view.clone(), ViewFilters::default())
        .await
        .map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    while let Some(item) = stream.next().await {
        rows.push(item.map_err(|e| e.to_string())?);
    }
    Ok(rows)
}

/// A Patient view declaring `name.family` (text) with the given `type`.
pub fn text_declared_as(ty: &str) -> Value {
    serde_json::json!({
        "resourceType": "ViewDefinition", "resource": "Patient", "status": "active",
        "select": [{"column": [
            {"path": "id", "name": "id"},
            {"path": "name.first().family", "name": "family", "type": ty}
        ]}]
    })
}
