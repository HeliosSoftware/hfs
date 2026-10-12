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

## Notes and limits

- With the default `row_group_size_mb` of 256 the flat peak is correspondingly
  larger (hundreds of MB per request). Pass a smaller `rowGroupSize` to bound it.
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
