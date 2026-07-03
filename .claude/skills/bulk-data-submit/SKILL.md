---
name: bulk-data-submit
description: Work on HFS Bulk Data Submit $bulk-submit. Use for Data Consumer ingestion, submit kick-off, status polling, manifests, file fetching, OAuth/private_key_jwt, JWE fileEncryptionKey behavior, submit worker leases, and bulk submit configuration.
---

# Bulk Data Submit

HFS implements the FHIR Bulk Data Submit operation from the Argo25 branch as the Data Consumer. A Data Provider POSTs `$bulk-submit` referencing a Bulk Export Manifest. HFS asynchronously fetches the manifest and NDJSON files, ingests them, and exposes results through a status manifest. The synchronous ingestion engine, `BulkSubmitProvider`, is reused; an async worker, lease, and fencing layer mirrors `$export`.

## Endpoints

| Operation | Method | URL | Response |
|---|---|---|---|
| kick-off | POST | `/$bulk-submit` | `200` sync accept; queues ingestion; `429` if blocking; `4XX` plus OperationOutcome on validation error |
| status kick-off | POST | `/$bulk-submit-status` | `202` plus `Content-Location` poll URL |
| poll or manifest | GET | `/bulk-submit-status/{poll_token}` | `202` in-progress with `X-Progress` and `Retry-After`; `200` plus status manifest when done; `404` after delete |
| cancel | DELETE | `/bulk-submit-status/{poll_token}` | `202`; subsequent poll returns `404` |
| HFS-served artifact | GET | `/bulk-submit-file/{poll_token}/{part}` | `200` `application/fhir+ndjson` |

All surfaces require the `system/bulk-submit` SMART scope when auth is enabled. Status, cancel, and file endpoints also enforce submission ownership through `owner_subject` or a system wildcard scope.

## Kick-off Parameters

The kick-off `Parameters` resource supports:

- `submitter`: Identifier, required.
- `submissionId`: string, required.
- `submissionStatus`: Coding `http://hl7.org/fhir/event-status`; `in-progress` default, `completed`, or `stopped`.
- `manifestUrl`.
- `replacesManifestUrl`.
- `outputFormat`.
- `fhirBaseUrl`: required when `manifestUrl` is present.
- `fileRequestHeader`: part.
- `oauthMetadataUrl`.
- `fileEncryptionKey`: part.
- `metadata` / `import`: parts.

At least one of `submissionStatus` or `manifestUrl` must be populated.

## Environment

| Variable | Default | Description |
|---|---|---|
| `HFS_BULK_SUBMIT_ENABLED` | `true` | Master switch; false returns `501` |
| `HFS_BULK_SUBMIT_OUTPUT_BACKEND` | `local-fs` | Status-artifact store: `local-fs` or `s3` |
| `HFS_BULK_SUBMIT_OUTPUT_DIR` | `${HFS_DATA_DIR}/submit` | Local filesystem artifact root |
| `HFS_BULK_SUBMIT_S3_BUCKET` | none | S3 bucket, required when output backend is s3 |
| `HFS_BULK_SUBMIT_REQUIRES_ACCESS_TOKEN` | `auto` | Manifest posture; false is invalid with local-fs |
| `HFS_BULK_SUBMIT_WORKER_CONCURRENCY` | `2` | In-process submit worker count |
| `HFS_BULK_SUBMIT_DISABLE_LOCAL_WORKER` | `false` | Disable in-pod workers |
| `HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT` | `4` | Per-tenant active submission cap; returns `429` |
| `HFS_BULK_SUBMIT_BATCH_SIZE` | `1000` | Ingestion batch size |
| `HFS_BULK_SUBMIT_LEASE_DURATION` | `60` | Manifest lease length in seconds; must exceed heartbeat |
| `HFS_BULK_SUBMIT_HEARTBEAT_INTERVAL` | `20` | Worker heartbeat cadence in seconds |
| `HFS_BULK_SUBMIT_CLEANUP_INTERVAL` | `300` | Cleanup scan interval in seconds |
| `HFS_BULK_SUBMIT_OUTPUT_TTL` | `86400` | Artifact retention in seconds |
| `HFS_BULK_SUBMIT_FILE_URL_TTL` | `3600` | Pre-signed artifact URL lifetime in seconds |
| `HFS_BULK_SUBMIT_CLIENT_ID` | none | OAuth client_id for fetching protected provider files |
| `HFS_BULK_SUBMIT_PRIVATE_KEY` | none | PEM key for `private_key_jwt` client assertion |
| `HFS_BULK_SUBMIT_SIGNING_ALG` | `ES384` | `ES384` or `RS384` |
| `HFS_BULK_SUBMIT_OUTBOUND_SCOPE` | `system/*.rs` | Read scope requested for file-retrieval tokens; never `system/bulk-submit` |

Job state reuses the same backend as the FHIR resources. SQLite shares `./data/hfs.db`; PostgreSQL shares `HFS_DATABASE_URL`. Bulk submit is available on `sqlite`, `postgres`, and their `-elasticsearch` composites. Other backends return `501`. The backend capability splits into `BulkSubmitIngest` (the synchronous `BulkSubmitProvider` ingestion engine) and `BulkSubmitRestWorker` (full `$bulk-submit` REST worker/job-store): SQLite and Postgres advertise both, while S3 advertises only `BulkSubmitIngest` and never owns REST-worker job state.

## Behavior Notes

- HFS is the Data Consumer: it fetches the provider's `manifestUrl` and files; it does not receive pushed data inline.
- For `requiresAccessToken` files, HFS acquires a read-scoped token via SMART Backend Services using `client_credentials` and `private_key_jwt` when `HFS_BULK_SUBMIT_CLIENT_ID` and `HFS_BULK_SUBMIT_PRIVATE_KEY` are set.
- If credentials are absent for `requiresAccessToken` files, fetches record a manifest-level error.
- `deleted` files, either transaction Bundles or resource refs, are applied as deletes.
- Partial success remains `200` with a populated `error[]` array of OperationOutcome NDJSON.
- Per-resource issues carry the `artifact-relatedArtifact` extension.
- `import` directives use upsert-by-id with last-write-wins, matching `replace`. `merge` semantics are a documented follow-up.
- JWE decryption for `fileEncryptionKey` supports `dir` plus `A128GCM` or `A256GCM` compact JWE files when built with `bulk-submit-jwe`.
- Build JWE support with `cargo build -p helios-hfs --features bulk-submit-jwe`.
- Without the feature, or for other JWE algorithms, encrypted files record a `not-supported` manifest-level error while unencrypted manifests proceed.
- Status `link` and pagination: HFS returns a single, non-paginated status manifest; the `link` array is always present and empty.
- Cleanup periodically removes status artifacts for submissions whose `updated_at` exceeds `HFS_BULK_SUBMIT_OUTPUT_TTL`.
