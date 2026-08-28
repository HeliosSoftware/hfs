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
- `metadata` / `import`: parts (`parameterUrl` + `parameterValue`, both required; `parameterUrl` SHALL be absolute).

At least one of `submissionStatus` or `manifestUrl` must be populated.

## Pre-coordinated `import` / `metadata` Directives

Both are persisted with the manifest they accompany and applied at ingestion. On a
status-only kick-off (no `manifestUrl`) they have nothing to attach to and are ignored with a warning.

| Directive | `parameterUrl` | Values | Effect |
|---|---|---|---|
| import mode | `https://helios.software/import-mode` | `replace` (default), `merge` | How a submitted resource is applied when one with the same id already exists |

- `replace`: upsert-by-id, last-write-wins — the submitted resource replaces the stored one wholesale.
- `merge`: RFC 7396 JSON Merge Patch of the submission onto the stored resource — elements absent from the submission are retained, present elements overwrite, arrays are replaced wholesale, and a `null` member removes the stored element. The stored `id` is always preserved.
- A recognized directive with an unusable value (e.g. `import-mode=upsert`) is always `400`.
- Unrecognized `import` `parameterUrl`s are `400` under `Prefer: handling=strict` and ignored with a warning otherwise.
- `metadata` parts carry no processing semantics: HFS retains all of them verbatim on the manifest and logs them at ingestion, so none are rejected under strict handling. They are not echoed into the status manifest, whose schema defines no slot for them.

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
| `HFS_BULK_SUBMIT_MANIFEST_PAGE_SIZE` | `1000` | Max entries per status-manifest page; `0` disables pagination |
| `HFS_BULK_SUBMIT_CLIENT_ID` | none | OAuth client_id for fetching protected provider files |
| `HFS_BULK_SUBMIT_PRIVATE_KEY` | none | PEM key for `private_key_jwt` client assertion |
| `HFS_BULK_SUBMIT_SIGNING_ALG` | `ES384` | `ES384` or `RS384` |
| `HFS_BULK_SUBMIT_OUTBOUND_SCOPE` | `system/*.rs` | Read scope requested for file-retrieval tokens; never `system/bulk-submit` |
| `HFS_BULK_SUBMIT_DECRYPTION_KEY` | none | P-256/P-384 private key(s) for `ECDH-ES*` `fileEncryptionKey` unwrapping — PEM (PKCS#8/SEC1) or a JWK / JWK Set |

Job state reuses the same backend as the FHIR resources — unlike bulk *export*, which sidecars its job store on MongoDB and S3. Every backend that runs `$bulk-submit` hosts its own: SQLite shares `./data/hfs.db`, PostgreSQL shares `HFS_DATABASE_URL`, MongoDB uses its own `bulk_*` collections, and S3 keeps the lease and artifact state in the same objects its ingestion engine already writes (compare-and-swapped against the object ETag). Bulk submit is therefore available on `sqlite`, `postgres`, `mongodb`, `s3`, and their `-elasticsearch` composites; other backends return `501`.

The backend capability splits into `BulkSubmitIngest` (the synchronous `BulkSubmitProvider` ingestion engine) and `BulkSubmitRestWorker` (full `$bulk-submit` REST worker/job-store). All four advertise both, with one exception: an S3 backend in `BucketPerTenant` mode with no `default_system_bucket` has nowhere tenant-independent to keep the worker's claim queue and poll-token index, so it advertises only `BulkSubmitIngest` and `$bulk-submit` reports `501` — the same axis that gates the per-user settings store.

## Behavior Notes

- HFS is the Data Consumer: it fetches the provider's `manifestUrl` and files; it does not receive pushed data inline.
- For `requiresAccessToken` files, HFS acquires a read-scoped token via SMART Backend Services using `client_credentials` and `private_key_jwt` when `HFS_BULK_SUBMIT_CLIENT_ID` and `HFS_BULK_SUBMIT_PRIVATE_KEY` are set.
- If credentials are absent for `requiresAccessToken` files, fetches record a manifest-level error.
- `deleted` files, either transaction Bundles or resource refs, are applied as deletes.
- Partial success remains `200` with a populated `error[]` array of OperationOutcome NDJSON.
- Per-resource issues carry the `artifact-relatedArtifact` extension.
- Resources are ingested per the submission's import mode (`replace` by default); see the directives section above.
- NDJSON files stream to the ingestion engine; JWE-encrypted files are the exception and are buffered whole, since the authentication tag trails the ciphertext.
- JWE decryption for `fileEncryptionKey` is built unconditionally; the `bulk-submit-jwe` feature is a deprecated no-op.
- Both the manifest and each output/deleted file are decrypted. A plaintext file is rejected when a key was supplied; a plaintext manifest is tolerated with a warning.
- Supported `alg`: `dir`, `A128KW`/`A192KW`/`A256KW`, `A128GCMKW`/`A192GCMKW`/`A256GCMKW`, `ECDH-ES` and `ECDH-ES+A128KW`/`+A192KW`/`+A256KW`.
- Supported `enc`: `A128GCM`/`A192GCM`/`A256GCM`, `A128CBC-HS256`/`A192CBC-HS384`/`A256CBC-HS512`. `zip: "DEF"` is inflated. Compact plus flattened/general JSON serializations are accepted.
- `RSA-OAEP`/`RSA-OAEP-256` are deliberately rejected: the only pure-Rust RSA implementation carries RUSTSEC-2023-0071 (Marvin Attack timing sidechannel) with no fix. Use `ECDH-ES` for asymmetric CEK delivery. RSA private keys are rejected by the config loader too.
- `RSA1_5` and `PBES2-*` are also rejected; every error names the algorithm and the reason.
- `fileEncryptionKey.value` may be base64url key material, an `oct` JWK, or itself a JWE delivering the CEK. The last form needs `HFS_BULK_SUBMIT_DECRYPTION_KEY` (P-256/P-384 PEM or JWK/JWK Set) — as do `ECDH-ES*` files.
- Status `link` and pagination: the status manifest is paginated at `HFS_BULK_SUBMIT_MANIFEST_PAGE_SIZE`
  entries (`output` + `outcome` + `deleted` combined). When more remain, `link` carries a single
  `{relation: next, url: .../bulk-submit-status/{token}?page=N}` entry; every other manifest field repeats
  identically on each page. Fetch pages from the status URL with `?page=N` (1-based) — out of range is `404`,
  malformed is `400`. Page size `0` disables pagination and yields one manifest with an empty `link`.
- Cleanup periodically removes status artifacts for submissions whose `updated_at` exceeds `HFS_BULK_SUBMIT_OUTPUT_TTL`.
