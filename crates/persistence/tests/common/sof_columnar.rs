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
use helios_persistence::core::{RunContext, RunKind, RunRequest, SofRunner, ViewFilters};
use helios_persistence::tenant::TenantContext;
use helios_sof::arrow::array::{Array, ArrayRef};
use helios_sof::arrow::datatypes::DataType;
use helios_sof::arrow::record_batch::RecordBatch;
use helios_sof::arrow::util::display::array_value_to_string;
use serde_json::Value;

/// The context of a parity run: detached (no deadline, nothing cancels it),
/// with the kind a `$sql-run` with these filters would have.
fn ctx(filters: &ViewFilters) -> RunContext {
    RunContext::detached(if filters.limit.is_some() {
        RunKind::Preview
    } else {
        RunKind::UnlimitedRun
    })
}

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
        .run_view(
            tenant,
            RunRequest::new(view.clone(), filters.clone()),
            ctx(filters),
        )
        .await
        .map(|output| output.rows)
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
        .run_view_columnar(
            tenant,
            RunRequest::new(view.clone(), filters.clone()),
            ctx(filters),
        )
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
                    } else {
                        Some(array_value_to_string(c, r).expect("render cell"))
                    }
                })
                .collect()
        })
        .collect()
}
