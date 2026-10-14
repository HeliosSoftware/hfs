//! Incremental Parquet and Arrow IPC encoders.
//!
//! The writers take a schema fixed up front and accept rows as they arrive,
//! so memory stays bounded by about one row group instead of the whole
//! result. They are synchronous and generic over [`std::io::Write`]; callers
//! that serve HTTP run them on a blocking thread behind a channel-backed
//! writer.
//!
//! Row groups are cut by encoded size: after every batch the writer flushes
//! once the in-progress row group reaches [`ParquetOptions::row_group_size_mb`].

use std::io::Write;
use std::sync::Arc;

use arrow::datatypes::Schema;
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::{BrotliLevel, Compression, GzipLevel, ZstdLevel};
use parquet::file::properties::WriterProperties;

use crate::{ParquetOptions, ProcessedRow, SofError, parquet_schema};

/// Rows per Arrow `RecordBatch` built by the incremental writers.
pub const STREAM_BATCH_ROWS: usize = 8192;

/// Parquet writer properties for `options` (compression is matched
/// case-insensitively; unknown names fall back to Snappy).
///
/// The row-group limit is deliberately left at the library default (a row
/// cap): the size-based cut is done by the writers, since the library setting
/// counts rows, not bytes.
pub fn writer_properties(options: &ParquetOptions) -> WriterProperties {
    let compression = match options.compression.to_ascii_lowercase().as_str() {
        "none" => Compression::UNCOMPRESSED,
        "gzip" => Compression::GZIP(GzipLevel::default()),
        "lz4" => Compression::LZ4,
        "brotli" => Compression::BROTLI(BrotliLevel::default()),
        "zstd" => Compression::ZSTD(ZstdLevel::default()),
        _ => Compression::SNAPPY,
    };
    WriterProperties::builder()
        .set_compression(compression)
        .set_data_page_row_count_limit(20_000) // Optimal for predicate pushdown
        .set_data_page_size_limit((options.page_size_kb as usize) * 1024)
        .set_write_batch_size(STREAM_BATCH_ROWS)
        .build()
}

fn to_batch(schema: &Arc<Schema>, rows: &[ProcessedRow]) -> Result<RecordBatch, SofError> {
    let arrays = parquet_schema::process_to_arrow_arrays(schema, &[], rows)?;
    RecordBatch::try_new(schema.clone(), arrays)
        .map_err(|e| SofError::ArrowConversionError(format!("Failed to create RecordBatch: {e}")))
}

fn check_schema(expected: &Arc<Schema>, batch: &RecordBatch) -> Result<(), SofError> {
    if batch.schema() == *expected {
        Ok(())
    } else {
        Err(SofError::ArrowConversionError(format!(
            "RecordBatch schema does not match the writer schema: batch {:?}, writer {:?}",
            batch.schema().fields(),
            expected.fields()
        )))
    }
}

/// Writes one Parquet file incrementally.
pub struct ParquetStreamWriter<W: Write + Send> {
    writer: ArrowWriter<W>,
    schema: Arc<Schema>,
    row_group_bytes: usize,
    rows_written: usize,
}

impl<W: Write + Send> ParquetStreamWriter<W> {
    /// Creates the writer; the schema is fixed here, before any row is seen.
    pub fn new(sink: W, schema: Arc<Schema>, options: &ParquetOptions) -> Result<Self, SofError> {
        let writer = ArrowWriter::try_new(sink, schema.clone(), Some(writer_properties(options)))
            .map_err(|e| {
            SofError::ParquetConversionError(format!("Failed to create Parquet writer: {e}"))
        })?;
        Ok(Self {
            writer,
            schema,
            row_group_bytes: (options.row_group_size_mb as usize) * 1024 * 1024,
            rows_written: 0,
        })
    }

    /// Encodes `rows` (in batches of [`STREAM_BATCH_ROWS`]), cutting a row
    /// group whenever the in-progress one reaches the configured size.
    /// Encodes `rows` as record batches of at most [`STREAM_BATCH_ROWS`].
    pub fn write_rows(&mut self, rows: &[ProcessedRow]) -> Result<(), SofError> {
        for chunk in rows.chunks(STREAM_BATCH_ROWS) {
            let batch = to_batch(&self.schema, chunk)?;
            self.writer.write(&batch).map_err(|e| {
                SofError::ParquetConversionError(format!("Failed to write RecordBatch: {e}"))
            })?;
            self.rows_written += chunk.len();
            if self.writer.in_progress_size() >= self.row_group_bytes {
                self.writer.flush().map_err(|e| {
                    SofError::ParquetConversionError(format!("Failed to flush row group: {e}"))
                })?;
            }
        }
        Ok(())
    }

    /// Encodes an already built batch (in slices of at most
    /// [`STREAM_BATCH_ROWS`]) with the same row-group rule as
    /// [`write_rows`](Self::write_rows). The batch schema must equal the
    /// writer's schema.
    pub fn write_batch(&mut self, batch: &RecordBatch) -> Result<(), SofError> {
        check_schema(&self.schema, batch)?;
        let mut offset = 0;
        while offset < batch.num_rows() {
            let len = (batch.num_rows() - offset).min(STREAM_BATCH_ROWS);
            let slice = batch.slice(offset, len);
            self.writer.write(&slice).map_err(|e| {
                SofError::ParquetConversionError(format!("Failed to write RecordBatch: {e}"))
            })?;
            self.rows_written += len;
            if self.writer.in_progress_size() >= self.row_group_bytes {
                self.writer.flush().map_err(|e| {
                    SofError::ParquetConversionError(format!("Failed to flush row group: {e}"))
                })?;
            }
            offset += len;
        }
        Ok(())
    }

    /// Rows encoded so far.
    pub fn rows_written(&self) -> usize {
        self.rows_written
    }

    /// Bytes already handed to the sink plus the estimated encoded size of the
    /// open row group — what the file would weigh if closed now.
    pub fn estimated_size(&self) -> usize {
        self.writer.bytes_written() + self.writer.in_progress_size()
    }

    /// Writes the footer and returns the sink.
    pub fn finish(self) -> Result<W, SofError> {
        self.writer.into_inner().map_err(|e| {
            SofError::ParquetConversionError(format!("Failed to close Parquet writer: {e}"))
        })
    }
}

/// Writes a sequence of Parquet files, starting a new one once the current
/// file's estimated size reaches `max_file_bytes`.
///
/// Splitting happens between batches, so a file can overshoot the limit by up
/// to one batch plus the footer. Files are opened lazily through `open`
/// (called with the file index); at least one file is always produced, even
/// for zero rows.
pub struct RollingParquetWriter<W: Write + Send, F: FnMut(usize) -> std::io::Result<W>> {
    open: F,
    schema: Arc<Schema>,
    options: ParquetOptions,
    max_file_bytes: usize,
    current: Option<ParquetStreamWriter<W>>,
    finished: Vec<W>,
    opened: usize,
}

impl<W: Write + Send, F: FnMut(usize) -> std::io::Result<W>> RollingParquetWriter<W, F> {
    /// `open(index)` supplies the sink for file `index`; a new file is started
    /// once the current one reaches `max_file_bytes` (checked per batch).
    pub fn new(
        open: F,
        schema: Arc<Schema>,
        options: &ParquetOptions,
        max_file_bytes: usize,
    ) -> Self {
        Self {
            open,
            schema,
            options: options.clone(),
            max_file_bytes,
            current: None,
            finished: Vec::new(),
            opened: 0,
        }
    }

    fn open_next(&mut self) -> Result<(), SofError> {
        let sink = (self.open)(self.opened).map_err(|e| {
            SofError::ParquetConversionError(format!("Failed to open Parquet output: {e}"))
        })?;
        self.opened += 1;
        self.current = Some(ParquetStreamWriter::new(
            sink,
            self.schema.clone(),
            &self.options,
        )?);
        Ok(())
    }

    /// Encodes `rows`, opening the next file when the size limit was reached.
    pub fn write_rows(&mut self, rows: &[ProcessedRow]) -> Result<(), SofError> {
        for chunk in rows.chunks(STREAM_BATCH_ROWS) {
            if self.current.is_none() {
                self.open_next()?;
            }
            let current = self.current.as_mut().expect("opened above");
            current.write_rows(chunk)?;
            if current.estimated_size() >= self.max_file_bytes {
                let done = self.current.take().expect("present").finish()?;
                self.finished.push(done);
            }
        }
        Ok(())
    }

    /// Closes the last file and returns every sink in order.
    pub fn finish(mut self) -> Result<Vec<W>, SofError> {
        if self.current.is_none() && self.finished.is_empty() {
            self.open_next()?;
        }
        if let Some(current) = self.current.take() {
            self.finished.push(current.finish()?);
        }
        Ok(self.finished)
    }
}

/// Writes an Arrow IPC stream incrementally.
pub struct ArrowIpcStreamWriter<W: Write> {
    writer: StreamWriter<W>,
    schema: Arc<Schema>,
}

impl<W: Write> ArrowIpcStreamWriter<W> {
    /// Creates the writer and writes the schema message to `sink`.
    pub fn new(sink: W, schema: Arc<Schema>) -> Result<Self, SofError> {
        let writer = StreamWriter::try_new(sink, schema.as_ref()).map_err(|e| {
            SofError::ArrowConversionError(format!("Failed to create Arrow IPC writer: {e}"))
        })?;
        Ok(Self { writer, schema })
    }

    pub fn write_rows(&mut self, rows: &[ProcessedRow]) -> Result<(), SofError> {
        for chunk in rows.chunks(STREAM_BATCH_ROWS) {
            let batch = to_batch(&self.schema, chunk)?;
            self.writer.write(&batch).map_err(|e| {
                SofError::ArrowConversionError(format!("Failed to write Arrow IPC batch: {e}"))
            })?;
        }
        Ok(())
    }

    /// Writes an already built batch (in slices of at most
    /// [`STREAM_BATCH_ROWS`]). The batch schema must equal the writer's schema.
    pub fn write_batch(&mut self, batch: &RecordBatch) -> Result<(), SofError> {
        check_schema(&self.schema, batch)?;
        let mut offset = 0;
        while offset < batch.num_rows() {
            let len = (batch.num_rows() - offset).min(STREAM_BATCH_ROWS);
            self.writer.write(&batch.slice(offset, len)).map_err(|e| {
                SofError::ArrowConversionError(format!("Failed to write Arrow IPC batch: {e}"))
            })?;
            offset += len;
        }
        Ok(())
    }

    /// Writes the end-of-stream marker and returns the sink.
    pub fn finish(mut self) -> Result<W, SofError> {
        self.writer.finish().map_err(|e| {
            SofError::ArrowConversionError(format!("Failed to finish Arrow IPC stream: {e}"))
        })?;
        self.writer.into_inner().map_err(|e| {
            SofError::ArrowConversionError(format!("Failed to finish Arrow IPC stream: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::datatypes::{DataType, Field};
    use bytes::Bytes;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use serde_json::json;

    fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, true),
            Field::new("n", DataType::Int32, true),
        ]))
    }

    fn rows(range: std::ops::Range<usize>) -> Vec<ProcessedRow> {
        range
            .map(|i| ProcessedRow {
                values: vec![
                    Some(json!(format!("id-{i}-{}", "x".repeat(200)))),
                    Some(json!(i)),
                ],
            })
            .collect()
    }

    fn row_count(file: Vec<u8>) -> (usize, usize) {
        let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(file)).unwrap();
        let groups = builder.metadata().num_row_groups();
        let n = builder
            .build()
            .unwrap()
            .map(|b| b.unwrap().num_rows())
            .sum();
        (n, groups)
    }

    #[test]
    fn parquet_cuts_row_groups_by_bytes() {
        let opts = ParquetOptions {
            row_group_size_mb: 1,
            compression: "none".into(),
            ..ParquetOptions::default()
        };
        let mut w = ParquetStreamWriter::new(Vec::new(), schema(), &opts).unwrap();
        // ~200 B/row * 20k rows = ~4 MB uncompressed -> several 1 MB groups.
        for start in (0..20_000).step_by(5_000) {
            w.write_rows(&rows(start..start + 5_000)).unwrap();
        }
        assert_eq!(w.rows_written(), 20_000);
        let (n, groups) = row_count(w.finish().unwrap());
        assert_eq!(n, 20_000);
        assert!(groups > 1, "expected several row groups, got {groups}");
    }

    #[test]
    fn rolling_writer_splits_and_keeps_all_rows() {
        let opts = ParquetOptions {
            row_group_size_mb: 1,
            compression: "none".into(),
            ..ParquetOptions::default()
        };
        let mut w = RollingParquetWriter::new(|_| Ok(Vec::new()), schema(), &opts, 512 * 1024);
        w.write_rows(&rows(0..20_000)).unwrap();
        let files = w.finish().unwrap();
        assert!(files.len() > 1, "expected a split, got {}", files.len());
        let total: usize = files.into_iter().map(|f| row_count(f).0).sum();
        assert_eq!(total, 20_000);
    }

    #[test]
    fn rolling_writer_emits_one_file_for_zero_rows() {
        let w = RollingParquetWriter::new(
            |_| Ok(Vec::new()),
            schema(),
            &ParquetOptions::default(),
            usize::MAX,
        );
        let files = w.finish().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(row_count(files.into_iter().next().unwrap()).0, 0);
    }

    #[test]
    fn arrow_ipc_round_trips() {
        let mut w = ArrowIpcStreamWriter::new(Vec::new(), schema()).unwrap();
        w.write_rows(&rows(0..10)).unwrap();
        let bytes = w.finish().unwrap();
        let reader =
            arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None).unwrap();
        let n: usize = reader.map(|b| b.unwrap().num_rows()).sum();
        assert_eq!(n, 10);
    }

    fn batch(range: std::ops::Range<usize>) -> RecordBatch {
        to_batch(&schema(), &rows(range)).unwrap()
    }

    #[test]
    fn parquet_write_batch_round_trips_and_splits_large_batches() {
        let mut w =
            ParquetStreamWriter::new(Vec::new(), schema(), &ParquetOptions::default()).unwrap();
        w.write_batch(&batch(0..STREAM_BATCH_ROWS * 2 + 5)).unwrap();
        w.write_batch(&batch(0..3)).unwrap();
        assert_eq!(w.rows_written(), STREAM_BATCH_ROWS * 2 + 8);
        let (n, _) = row_count(w.finish().unwrap());
        assert_eq!(n, STREAM_BATCH_ROWS * 2 + 8);
    }

    #[test]
    fn parquet_write_batch_cuts_row_groups_by_bytes() {
        let opts = ParquetOptions {
            row_group_size_mb: 1,
            compression: "none".into(),
            ..ParquetOptions::default()
        };
        let mut w = ParquetStreamWriter::new(Vec::new(), schema(), &opts).unwrap();
        w.write_batch(&batch(0..20_000)).unwrap();
        let (n, groups) = row_count(w.finish().unwrap());
        assert_eq!(n, 20_000);
        assert!(groups > 1, "expected several row groups, got {groups}");
    }

    #[test]
    fn write_batch_rejects_a_different_schema() {
        let other = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, true)]));
        let bad = to_batch(&other, &rows(0..2)).unwrap();
        let mut p =
            ParquetStreamWriter::new(Vec::new(), schema(), &ParquetOptions::default()).unwrap();
        assert!(p.write_batch(&bad).is_err());
        let mut a = ArrowIpcStreamWriter::new(Vec::new(), schema()).unwrap();
        assert!(a.write_batch(&bad).is_err());
        // Same columns but a different type is also a mismatch.
        let retyped = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, true),
            Field::new("n", DataType::Int64, true),
        ]));
        let bad = to_batch(&retyped, &rows(0..2)).unwrap();
        assert!(a.write_batch(&bad).is_err());
    }

    #[test]
    fn arrow_ipc_write_batch_splits_large_batches() {
        let mut w = ArrowIpcStreamWriter::new(Vec::new(), schema()).unwrap();
        w.write_batch(&batch(0..STREAM_BATCH_ROWS + 10)).unwrap();
        let bytes = w.finish().unwrap();
        let reader =
            arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(bytes), None).unwrap();
        let sizes: Vec<usize> = reader.map(|b| b.unwrap().num_rows()).collect();
        assert_eq!(sizes, vec![STREAM_BATCH_ROWS, 10]);
    }
}
