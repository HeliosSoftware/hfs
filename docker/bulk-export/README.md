# Bulk Data Export

HFS implements the [FHIR Bulk Data Access IG](https://build.fhir.org/ig/HL7/bulk-data/)
`$export` operation asynchronously: kick-off, poll, manifest, download, and
delete.

This directory contains a provided docker-compose example for running HFS with
Bulk Data Export job state in PostgreSQL and export output in S3-compatible
storage via MinIO. It is intended for local manual testing, demos, and trying
Bulk Data clients such as Inferno against a multi-instance-style topology.

This compose file is not used by the GitHub Actions bulk export or Inferno
workflow tests. Those workflows start their backing services directly so they
can control ports, artifacts, and per-job isolation.

## Stack

- HFS
- PostgreSQL for primary storage and bulk export job state
- MinIO for S3-compatible export output
- Keycloak using `docker/keycloak/realm.json`

## Endpoints

| Operation | Method | URL |
|-----------|--------|-----|
| system kick-off | GET / POST | `/$export` |
| patient kick-off | GET / POST | `/Patient/$export` |
| group kick-off | GET / POST | `/Group/{id}/$export` |
| status / manifest | GET | `/export-status/{job_id}` |
| cancel + delete | DELETE | `/export-status/{job_id}` |
| HFS-served download | GET | `/export-file/{job_id}/{type}-{part}` |

All kick-offs require `Prefer: respond-async`. The default response is
`202 Accepted` with a `Content-Location` status URL.

## Single Instance

The default HFS configuration wires embedded bulk export with SQLite job state,
local filesystem output, and an in-process worker pool.

```bash
cargo run --bin hfs
curl -i -H 'Prefer: respond-async' \
  http://localhost:8080/Patient/\$export
```

## Run

```bash
docker compose -f docker/bulk-export/docker-compose.yml up --build
```

HFS is available at `http://localhost:8080`.

## Try an Export

```bash
curl -i -H 'Prefer: respond-async' \
  http://localhost:8080/Patient/\$export
```

The response includes a `Content-Location` header for polling the export job.

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_BULK_EXPORT_ENABLED` | `true` | Master switch. When `false`, all `$export` endpoints return `501`. |
| `HFS_BULK_EXPORT_OUTPUT_BACKEND` | `local-fs` | Output store: `local-fs` or `s3`. |
| `HFS_BULK_EXPORT_OUTPUT_DIR` | `${HFS_DATA_DIR}/exports` | Local filesystem output root. |
| `HFS_BULK_EXPORT_S3_BUCKET` | none | S3 bucket. Required when `OUTPUT_BACKEND=s3`. |
| `HFS_BULK_EXPORT_S3_ENDPOINT` | AWS | S3-compatible endpoint URL, such as MinIO. |
| `HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE` | `false` | Path-style addressing for S3-compatible providers. |
| `HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN` | `auto` | Manifest posture: `auto`, `true`, or `false`. `false` is invalid with `local-fs`. |
| `HFS_BULK_EXPORT_FILE_URL_TTL` | `3600` | Pre-signed download URL lifetime in seconds. |
| `HFS_BULK_EXPORT_OUTPUT_TTL` | `86400` | Output retention after job completion in seconds. |
| `HFS_BULK_EXPORT_WORKER_CONCURRENCY` | `2` | In-process worker pool size. |
| `HFS_BULK_EXPORT_DISABLE_LOCAL_WORKER` | `false` | Disable in-process workers for separate exporter deployment. |
| `HFS_BULK_EXPORT_MAX_CONCURRENT_PER_TENANT` | `4` | Per-tenant active-job cap. |
| `HFS_BULK_EXPORT_BATCH_SIZE` | `1000` | Resources per export batch. |
| `HFS_BULK_EXPORT_LEASE_DURATION` | `60` | Initial lease length in seconds. Must be greater than the heartbeat interval. |
| `HFS_BULK_EXPORT_HEARTBEAT_INTERVAL` | `20` | Worker heartbeat cadence in seconds. |
| `HFS_BULK_EXPORT_CLEANUP_INTERVAL` | `300` | Cleanup task scan interval in seconds. |
| `HFS_BULK_EXPORT_SINCE_NEWLY_ADDED` | `include` | Group-export `_since` toggle: `include` or `exclude`. |
