//! Peak-memory comparison of buffered and incremental Parquet encoding.
//!
//! Each arm encodes the same synthetic rows, produced lazily by an iterator.
//! A counting global allocator tracks live heap bytes; the reported figure is
//! the peak above the level at the start of the arm. Run one arm per process
//! so the arms cannot influence each other:
//!
//! ```text
//! cargo run --release -p helios-sof --example parquet_peak_memory -- buffered
//! cargo run --release -p helios-sof --example parquet_peak_memory -- incremental
//! ```
//!
//! Optional arguments: `<arm> [rows] [row_group_size_mb]` (defaults: 2000000
//! rows, 16 MB row groups).

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use helios_sof::columnar_stream::ParquetStreamWriter;
use helios_sof::parquet_schema::resolve_arrow_schema;
use helios_sof::{ParquetOptions, ProcessedResult, ProcessedRow, format_parquet};

struct CountingAlloc;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn record_alloc(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

// SAFETY: every method delegates to `System`; the counters are only bookkeeping.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            record_alloc(new_size);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const COLUMNS: [&str; 4] = ["id", "n", "v", "note"];

/// Deterministic, poorly compressible rows (xorshift noise in `note`).
fn rows(count: usize) -> impl Iterator<Item = ProcessedRow> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..count).map(move |i| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ProcessedRow {
            values: vec![
                Some(serde_json::json!(format!("patient-{i}"))),
                Some(serde_json::json!(i as i64)),
                Some(serde_json::json!((state % 100_000) as f64 / 7.0)),
                Some(serde_json::json!(format!(
                    "{state:016x}{:016x}",
                    state.rotate_left(17)
                ))),
            ],
        }
    })
}

/// Output sink that only counts bytes, like a response body already sent.
struct CountingSink(usize);

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arm = args.get(1).map(String::as_str).unwrap_or("incremental");
    let count: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let row_group_size_mb: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(16);
    let options = ParquetOptions {
        row_group_size_mb,
        ..ParquetOptions::default()
    };
    let columns: Vec<String> = COLUMNS.iter().map(|c| c.to_string()).collect();

    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let started = Instant::now();

    let output_bytes = match arm {
        "buffered" => {
            let result = ProcessedResult {
                columns,
                rows: rows(count).collect(),
            };
            format_parquet(result, Some(&options))
                .expect("buffered encode")
                .len()
        }
        "incremental" => {
            let mut iter = rows(count);
            let first: Vec<ProcessedRow> = iter.by_ref().take(100).collect();
            let schema =
                std::sync::Arc::new(resolve_arrow_schema(&columns, &[], &first).expect("schema"));
            let mut writer =
                ParquetStreamWriter::new(CountingSink(0), schema, &options).expect("writer");
            writer.write_rows(&first).expect("write prefix");
            let mut chunk = Vec::with_capacity(8192);
            for row in iter {
                chunk.push(row);
                if chunk.len() == 8192 {
                    writer.write_rows(&chunk).expect("write rows");
                    chunk.clear();
                }
            }
            writer.write_rows(&chunk).expect("write tail");
            writer.finish().expect("finish").0
        }
        other => {
            eprintln!("unknown arm {other:?}; use `buffered` or `incremental`");
            std::process::exit(2);
        }
    };

    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    println!(
        "arm={arm} rows={count} row_group_size_mb={row_group_size_mb} output_bytes={output_bytes} \
         peak_live_mb={:.1} elapsed_s={:.2}",
        peak as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64()
    );
}
