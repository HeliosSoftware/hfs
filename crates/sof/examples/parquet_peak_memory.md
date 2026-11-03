# SQL-on-FHIR Parquet streaming: peak-memory evidence for #1812

Parquet output used to be built from the complete result held in memory. The
incremental writer in `helios_sof::columnar_stream` fixes the schema up front,
encodes rows in batches of 8,192 and cuts a row group each time the encoded
size reaches `row_group_size_mb`. This page records the peak heap of both
approaches on the same synthetic input.

## Method

- Harness: `crates/sof/examples/parquet_peak_memory.rs`. A counting global
  allocator tracks live heap bytes; the figure is the peak above the level at
  the start of the arm. Each arm runs in its own process.
- Input: 4 columns (`id` string, `n` integer, `v` decimal, `note` 32-character
  hex noise so compression does not hide the size), generated lazily by an
  iterator, so the input itself is not what is being measured in the
  incremental arm.
- Buffered arm: all rows collected into a `ProcessedResult`, then
  `format_parquet`, which returns the whole file as a `Vec<u8>`.
- Incremental arm: `ParquetStreamWriter` fed 8,192 rows at a time, writing to
  a sink that only counts bytes (standing in for a response body that has
  already been sent).
- Settings: `row_group_size_mb = 16`, Snappy, debug build (allocation counts do
  not depend on the optimisation level; elapsed time does).
- Machine: AMD Ryzen 9 9955HX, 8 logical CPUs available, rustc 1.99.0, base
  commit `bd9d8eebd` plus the #1812 working tree.

Reproduce:

```bash
cargo run -p helios-sof --example parquet_peak_memory -- buffered 2000000 16
cargo run -p helios-sof --example parquet_peak_memory -- incremental 2000000 16
```

## Results

| Rows | Output size | Buffered peak | Incremental peak |
| ---: | ---: | ---: | ---: |
| 1,000,000 | 47.3 MB | 463.0 MB | 49.9 MB |
| 2,000,000 | 94.6 MB | 898.0 MB | 50.0 MB |

The buffered peak grows with the result (about 450 bytes of heap per row). The
incremental peak stays flat at roughly three times the row group size: the
open row group, the batch being encoded and the writer's own buffers. These are
single runs, not percentiles; elapsed time was the same within noise (9.5 s
buffered, 8.8 s incremental at 2 M rows, debug build).

## Through the HTTP path

Measured on `hfs` (debug build, sqlite backend) with 200,000 stored
Observations and a typed view of five columns, calling `POST /$sql-run`. Peak
RSS is `VmHWM` after resetting it through `/proc/<pid>/clear_refs`, reported
above the RSS at the start of the request. The buffered comparison is
`_format=csv`, which still drains the whole `RowStream` into memory exactly as
the Parquet path did before this change (a fair proxy; building `main` too was
not done).

| Format | Time to first byte | Total | Peak RSS above start |
| --- | ---: | ---: | ---: |
| `parquet` (streamed, first request on a fresh process) | 9 ms | 2.6 s | 51 MB |
| `csv` (buffered) | 2,734 ms | 2.7 s | 297 MB |
| `csv` (buffered, second run) | 2,707 ms | 2.7 s | 295 MB |

The streamed response starts after the first 100 rows instead of after the
last one. The second Parquet run showed no growth at all because the process
allocator had already kept the memory from the first CSV run, so only the first
figure is a clean one. The Parquet output here is 3.4 MB, below the default
256 MB row group, so this measures the removal of the buffered copies rather
than row group cutting, which the library benchmark above covers. Single runs on
a shared machine.

## Notes and limits

- With the default `row_group_size_mb` of 256 the flat peak is correspondingly
  larger (hundreds of MB per request). `rowGroupSize` cannot go below 64 MB over REST or in `sof-cli`/`sof-server`, so a request's peak has a floor of roughly three times that (about 190 MB); the 16 MB used here is only reachable through the library.
- Splitting by `maxFileSize` has the granularity of one batch: the size is
  checked after each batch of 8,192 rows against the bytes written plus the
  encoded size of the open row group, so a file can overshoot the limit by one
  batch plus the footer.
- Before this change `row_group_size_mb` was passed to a setting that counts
  rows, so every file was a single row group and the size-based split almost
  never fired. Files now contain several row groups once the data exceeds the
  configured size.
- Not covered by these numbers: the Binary envelope, `_format=fhir`, inline
  resources and SQLQuery/Library subjects, which still build the output in
  memory.
