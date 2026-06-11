---
name: bulk-data-export
description: Work on HFS FHIR Bulk Data Access $export. Use for export kick-off, polling, manifests, downloads, job state, output storage, S3/local export configuration, Inferno bulk data workflow, _typeFilter, _elements, and group export behavior.
---

# Bulk Data Export

HFS implements the FHIR Bulk Data Access `$export` family asynchronously: kick-off, poll, manifest, download, delete.

## Endpoints

| Operation | Method | URL |
|---|---|---|
| system kick-off | GET/POST | `/$export` |
| patient kick-off | GET/POST | `/Patient/$export` |
| group kick-off | GET/POST | `/Group/{id}/$export` |
| status or manifest | GET | `/export-status/{job_id}` |
| cancel and delete | DELETE | `/export-status/{job_id}` |
| HFS-served download | GET | `/export-file/{job_id}/{type}-{part}` |

All kick-offs require `Prefer: respond-async`. The default response is `202 Accepted` with a `Content-Location` status URL.

## Environment

| Variable | Default | Description |
|---|---|---|
| `HFS_BULK_EXPORT_ENABLED` | `true` | Master switch; false returns `501` for all export endpoints |
| `HFS_BULK_EXPORT_OUTPUT_BACKEND` | `local-fs` | Output store: `local-fs` or `s3` |
| `HFS_BULK_EXPORT_OUTPUT_DIR` | `${HFS_DATA_DIR}/exports` | Local filesystem output root |
| `HFS_BULK_EXPORT_S3_BUCKET` | none | S3 bucket, required when output backend is s3 |
| `HFS_BULK_EXPORT_S3_ENDPOINT` | AWS | S3-compatible endpoint URL, such as MinIO |
| `HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE` | `false` | Path-style addressing for S3-compatible providers |
| `HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN` | `auto` | Manifest posture: auto, true, or false; false is invalid with local-fs |
| `HFS_BULK_EXPORT_FILE_URL_TTL` | `3600` | Pre-signed download URL lifetime in seconds |
| `HFS_BULK_EXPORT_OUTPUT_TTL` | `86400` | Output retention after job completion in seconds |
| `HFS_BULK_EXPORT_WORKER_CONCURRENCY` | `2` | In-process worker pool size |
| `HFS_BULK_EXPORT_DISABLE_LOCAL_WORKER` | `false` | Disable in-pod workers for separate exporter deployments |
| `HFS_BULK_EXPORT_MAX_CONCURRENT_PER_TENANT` | `4` | Per-tenant active job cap; kick-off returns `429` if exceeded |
| `HFS_BULK_EXPORT_BATCH_SIZE` | `1000` | Resources per `fetch_export_batch` |
| `HFS_BULK_EXPORT_LEASE_DURATION` | `60` | Initial lease length in seconds; must exceed heartbeat interval |
| `HFS_BULK_EXPORT_HEARTBEAT_INTERVAL` | `20` | Worker heartbeat cadence in seconds |
| `HFS_BULK_EXPORT_CLEANUP_INTERVAL` | `300` | Cleanup scan interval in seconds |
| `HFS_BULK_EXPORT_SINCE_NEWLY_ADDED` | `include` | Group export `_since` toggle: include or exclude |

Job-state storage reuses the same backend and connection pool that holds FHIR resources. SQLite deployments share `./data/hfs.db`. PostgreSQL deployments share `HFS_DATABASE_URL`. There is no separate job-store configuration.

Bulk export is currently available on `sqlite`, `postgres`, `sqlite-elasticsearch`, and `postgres-elasticsearch`. Other backends return `501` until job-state implementations exist.

## Single-instance Recipe

```bash
cargo run --bin hfs
```

This starts HFS with bulk export enabled, job state in the same SQLite database as FHIR resources, NDJSON output under `./data/exports/`, and an in-process worker pool.

```bash
curl -H 'Prefer: respond-async' http://localhost:8080/Patient/\$export
```

## Multi-instance Recipe

PostgreSQL plus S3 or MinIO:

```bash
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL=postgresql://hfs:hfs@localhost/hfs \
HFS_BULK_EXPORT_OUTPUT_BACKEND=s3 \
HFS_BULK_EXPORT_S3_BUCKET=hfs-export \
HFS_BULK_EXPORT_S3_ENDPOINT=http://localhost:9000 \
HFS_BULK_EXPORT_S3_FORCE_PATH_STYLE=true \
HFS_BULK_EXPORT_REQUIRES_ACCESS_TOKEN=false \
cargo run --bin hfs --features postgres,s3
```

The full local stack is in `docker/bulk-export/docker-compose.yml`: HFS, Postgres, MinIO, and Keycloak. GitHub Actions does not use this compose file for bulk export tests. The manual conformance workflow is `.github/workflows/inferno-bulk-data.yml`.

## Behavior Notes

- `_typeFilter` is parsed and applied.
- Unsupported result-control params inside `_typeFilter` are rejected with `400` regardless of `Prefer: handling`: `_sort`, `_include`, `_revinclude`, `_count`, `_elements`.
- `_elements` is implemented: subset to listed paths plus `id`, `resourceType`, and `meta`, with a `SUBSETTED` `meta.tag` added.
- Unsupported parameters `includeAssociatedData`, `organizeOutputBy`, and `allowPartialManifests` return `400` when `Prefer: handling=strict` is set. Without strict handling, or with lenient handling, they are ignored and a warning is logged.
- Group export `_since` late membership uses `include` by default, returning pre-`_since` resources for patients added after `_since`.
- `exclude` is reserved for a follow-up that requires group-membership-history tracking.
- Group export flattens nested `Group/` members iteratively with a visited-set cycle guard.
