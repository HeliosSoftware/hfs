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
| `HFS_BULK_SUBMIT_FILE_CONCURRENCY` | `1` | Files of one manifest ingested at once (fan-out); always `1` on SQLite, where fan-out is not supported |
| `HFS_BULK_SUBMIT_DISABLE_LOCAL_WORKER` | `false` | Disable in-pod workers |
| `HFS_BULK_SUBMIT_MAX_CONCURRENT_PER_TENANT` | `4` | Per-tenant active submission cap; returns `429` |
| `HFS_BULK_SUBMIT_BATCH_SIZE` | `1000` | Ingestion batch size |
| `HFS_BULK_SUBMIT_DEFER_INDEXING` | `true` | Bulk fast-load (#903): ingest without search-index/FTS writes, then rebuild with an automatic per-type reindex when each manifest finishes. Default since #946 — ~1.2x faster end to end. Honoured on MongoDB only since #1000, where it was silently inert. A restart before that rebuild lands leaves the data stored but unsearchable; set `false` to close that window |
| `HFS_REINDEX_BATCH_SIZE` | `1000` | Page size of the automatic deferred rebuild (`DEFERRED_REINDEX_BATCH_SIZE`), reaching `ReindexOnFinish`. `POST $reindex` keeps its own `batchSize` (default 100) |
| `HFS_ELASTICSEARCH_BULK_MAX_BYTES` | `10485760` | ES composites: byte cap per `_bulk` request body (10 MiB), on top of the 500-operation cap. Applies to the ingest sync and to every rebuild |
| `HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS` | `30000` | ES composites: timeout of every Elasticsearch request, `_bulk` included. A timed-out `_bulk` is split in half and resent, down to one document |
| `HFS_ELASTICSEARCH_REINDEX_REFRESH` | unset | ES composites: `refresh` for `$reindex` and deferred-rebuild writes (`false`/`wait_for`/`true`); unset follows `HFS_ELASTICSEARCH_WRITE_REFRESH`. `false` skips the per-request refresh wait during a rebuild only |
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

`HFS_COMPOSITE_SYNC_MODE` (documented in `/run-hfs-server`) also affects bulk submit on composite deployments: `asynchronous` (default) queues the secondary sync; the bulk-submit receipt then cannot reflect search-index failures and the drift check is skipped.

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
- File fan-out is backend-aware. `HFS_BULK_SUBMIT_FILE_CONCURRENCY` is honoured as configured on the concurrent-writer backends (PostgreSQL, MongoDB, S3), but file fan-out is **not supported on SQLite**: `effective_file_concurrency` returns `1` there whatever the operator configured, and a `WARN` at startup names the configured and effective values. SQLite serialises writers, so any fan-out above one queues each batch's writes behind a single exclusive lock until they outlast `busy_timeout` and abort the manifest outright. Any file fan-out at all requires PostgreSQL.
- The manifest bookkeeping and resource writes retry with bounded exponential backoff when SQLite reports the database busy or locked, instead of failing the ingest. The retry budget is an elapsed-time deadline bounded by the manifest lease, so a retrying write can never outlive the lease it holds. Every other error still surfaces on the first attempt.
- With `HFS_BULK_SUBMIT_DEFER_INDEXING=true` (bulk fast-load, #903, the default since #946), ingestion skips search-index and FTS writes. The worker requests an automatic full-type reindex after the manifest becomes terminal, so `$bulk-submit-status` can answer `200` while search is still incomplete.
- Automatic deferred reindex requests share the coordinator owned by their `ReindexOperation` (#1087). One tenant has one active generation plus one pending, deduplicated type set. The process runs at most `W` automatic generations and retains at most `2W` tenant entries, where `W` is the existing `HFS_BULK_SUBMIT_WORKER_CONCURRENCY` value. Admission applies backpressure before it adds another tenant. HFS has no separate bulk-submit reindex-concurrency variable.
- The coordinator releases and reacquires its execution permit between generations so another admitted tenant can progress. Separate `ReindexOperation` instances remain independent because they can have different writer and registry sets. Explicit `$reindex` jobs bypass this coordinator and can overlap automatic work.
- A clean automatic generation ends `Completed` with no resource errors. Failure of the job itself or a panic gets one retry with the active and pending types. A completion with *transient* resource errors (a backend that was unavailable, timed out, or answered Elasticsearch `429`/`5xx`) gets one retry that covers only those resources, by id (#1125); before #1125 it re-ran every type of the generation, which on `sqlite-elasticsearch` repeated the whole rebuild and failed identically. A completion whose resource errors are all *permanent* — a document the search backend rejects outright, such as one over Elasticsearch's nested-object limit (#1050) — is not retried, because a rerun fails identically; it logs the error count and the first failing `Type/id`s instead. A second consecutive failure abandons that generation and logs the manual `$reindex` repair. Both failure lines name up to five failing `Type/id`s, and every resource the rebuild fails to index is also logged at `warn` with `Type/id` and the reason, rate-limited per type with a per-type summary, for transient and permanent failures alike. A SQLite row the source cannot parse is recorded as a permanent error for that resource instead of silently ending the rebuild of its type. Independently queued work that arrived during the retry still runs as a new generation with its own retry budget. Cancellation does not retry the cancelled active types, but independently queued pending types still run after the cancelled task has stopped writing.
- Coordination and reindex job state are in memory and local to one HFS process. A restart loses pending work, and separate processes do not coordinate. Full-type scans remain in use for each first attempt, so a finite burst can still cause one active scan and one accumulated follow-up. Limiting work to successful manifest IDs was evaluated and deferred because the generic path lacks bounded receipt deduplication, current-resource handling for missing or deleted IDs, and consistent semantics for every composite target.
- The coordination logic is common to standalone SQLite, PostgreSQL, and MongoDB plus the Elasticsearch composites that wire reindex. Current performance evidence is PostgreSQL-only; do not claim equivalent latency or database-work improvements for the other backends without measuring them. See `docs/deferred-reindex-coordination-benchmark.md`.
- MongoDB ingests a batch, not an entry (#1000): one `find` resolves which ids already exist, then one `insert`/`update` command per collection. The per-entry path it replaced cost ~9 round trips per resource and ran at ~60–76 resources/s with the server two-thirds idle; batched it reaches ~720, and ~3 100 with indexing deferred. The flush is ordered commands, not one transaction — the per-entry path was not atomic across a batch either, and a batch-wide transaction would widen #1001 from one lost entry to a whole batch.
- Composite deployments (primary + Elasticsearch, including the `mongo-es`/`s3-es` modes) must wrap the primary's job store with `composite_submit_jobs(...)`: ingestion runs on the primary, whose own indexing is offloaded, so without the wrapper a completed import is readable by id and invisible to every search (#882, and #1021 for the modes that were missed). Guard: `crates/hfs/tests/bulk_submit/run_composite_es_index_check.sh`.
- On those composite deployments (#1007), the worker copies every manifest's ingested resources into the secondary search index *before* writing the manifest's receipt (not at `finish_manifest`, which no longer syncs — a manifest already terminal is never re-synced by a restart). A resource the secondary still rejects after its retries gets an entry result of `processing-error` in the receipt, with an OperationOutcome (`incomplete`) naming the `Type/id`, the rejecting backend, and `POST /{type}/$reindex` as the repair; the resource itself stays stored and readable by id, and `failed_entries` on the status counts it. If the rejection is Elasticsearch's nested-object limit (`The number of nested documents has exceeded the allowed limit`), `$reindex` fails the same way until `HFS_ELASTICSEARCH_NESTED_OBJECTS_LIMIT` (default 50000, raised on existing indices at startup) is above that resource's nested value count (#1050). Raising the limit is necessary but not sufficient: `$reindex` sends the same documents through the same `_bulk` path, and on `sqlite-elasticsearch` before #1125 every 500-document `Provenance` request failed as a whole at the transport level (`backend unavailable: elasticsearch`, `retryable: true`), so `$reindex` indexed none of them. Treat `$reindex` as the repair only once its status reports `errorCount` 0 for the type. Each failed resource is listed, with its error and whether it is retryable, in `GET /$reindex-status/{job_id}`.
- **Deferred rebuild survives a restart (#1125).** A manifest ingested with `HFS_BULK_SUBMIT_DEFER_INDEXING=true` records that it still owes a search-index rebuild (`bulk_manifests.index_pending`, SQLite schema v30), set in the transaction that publishes the manifest and cleared when the rebuild finishes. On startup the server scans for those manifests and re-fires the same hook, logging `resuming search-index rebuilds left outstanding by an earlier run`. Other backends do not record it and so do not resume.
- **Rebuild knobs (#1125).** `HFS_ELASTICSEARCH_BULK_CONCURRENCY` (default `1`) sends several `_bulk` requests of one page at once; `HFS_REINDEX_BATCH_BYTES` (default `0` = off) caps a page by bytes so ~108 KB `Provenance` resources do not make a ~108 MB page. On `sqlite-es` the recommended pair for an import is `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for` with `HFS_ELASTICSEARCH_REINDEX_REFRESH=false`: measured on a 228,580-resource cut the rebuild went from 806 s to 145 s, complete either way.
- **Elasticsearch `_bulk` shape (#1125).** The ingest sync and every rebuild (`$reindex` and the deferred post-import rebuild) send documents through one `_bulk` path, capped per request at 500 operations *and* `HFS_ELASTICSEARCH_BULK_MAX_BYTES` (default 10 MiB). A request that exceeds `HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS` (default 30000) or is answered `413` (or a proxy's `408`/`504`) is split in half and resent, recursively, down to one document; once a single document times out, the rest of the page fails as transient instead of being split further. A `429`, for the request or per item, is retried with bounded exponential back-off for the rejected items only. A per-document `4xx` stays permanent, and an `ensure_index` transport failure is transient. A connection error, a whole-request `5xx`, or a `429` that outlasts its retries fails every document of that request as transient. Every failed document is recorded by `Type/id`, with Elasticsearch's `error.type`/`reason` when it gave one. Before #1125 there was no byte cap: 500 Synthea `Provenance` resources (~108 KB each) made a ~54 MB request that timed out, all 11,704 `Provenance` of the 1 % cut failed as `backend unavailable: elasticsearch`, and the automatic retry re-ran every type and failed identically.
- A client-side timeout does not cancel the `_bulk` Elasticsearch is already executing, so a resource reported as failed may still have indexed. Confirm with `GET /{type}?_summary=count` before rebuilding again.
- On `sqlite-elasticsearch` the rebuild writes only to Elasticsearch. Before #1125 it also wrote the offloaded SQLite `search_index`/FTS, which no query there reads, and because the matching delete was a no-op those rows accumulated on every run (at least 11 KB per resource).
- After that copy, the worker compares the primary's tenant-wide count against each secondary's for every resource type the manifest ingested. A mismatch is recorded as a `warning` OperationOutcome (also `incomplete`, naming both counts) in the manifest's `error` artifact and logged. This only runs when `HFS_COMPOSITE_SYNC_MODE` is `synchronous` or `hybrid`; under the default `asynchronous` mode the secondary's count reflects whatever had already drained from its queue rather than this manifest's sync, so the check is skipped and the receipt then only guarantees the resources committed on the primary.
- Without `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for`, a small count difference can be a write not yet visible rather than a real gap; reconfirm with `GET /{type}?_summary=count` before treating it as drift. See "Verifying and repairing search drift" below.
- `HFS_BULK_SUBMIT_DEFER_INDEXING` is read once at startup, not per submission. Rebuild a stale or missed index with `$reindex`, then confirm `errorCount` 0 in `$reindex-status`; see the repair caveats below.
- Cleanup periodically removes status artifacts for submissions whose `updated_at` exceeds `HFS_BULK_SUBMIT_OUTPUT_TTL`.

## Verifying and repairing search drift

Applies to composite deployments (`*-elasticsearch` backends) where a primary
stores and serves resources by id while an Elasticsearch secondary serves
search. A standalone backend has a single index, so there is nothing to drift.

**Verify**: fetch the finished status manifest with `GET
/bulk-submit-status/{poll_token}` and scan its `error` artifact for
OperationOutcome issues coded `incomplete` — `severity: error` names a
resource the secondary rejected (`processing-error`, still stored and
readable by id); `severity: warning` names a resource type whose tenant-wide
count disagrees between the primary and a secondary. Confirm the count
directly against the secondary with `_summary=count`, scoped to the same
tenant:

```bash
curl -H "X-Tenant-ID: clinic-a" "http://localhost:8080/Patient?_summary=count"
```

Compare that against the primary's own count for the same type and tenant
(the dashboard in the UI, or the backend's `count`). This is the type's
tenant-wide count, not just this manifest's entries — a drift can predate
the manifest that surfaced it.

**Repair**: rebuild the affected resource type's search index with
`$reindex`, which requires the `system/reindex` scope:

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -H "X-Tenant-ID: clinic-a" \
  http://localhost:8080/Patient/\$reindex
# -> 202 Accepted, a Parameters resource with a "jobId" parameter

curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:8080/\$reindex-status/{job_id}
```

`$reindex` rebuilds the whole resource type for the tenant, not only the
manifest's resources, and its job state is per-process — poll the node you
kicked it off against. See #1007 and `HFS_COMPOSITE_SYNC_MODE` above for why
the drift check does not always run.

`$reindex` is not a guaranteed repair on an Elasticsearch-backed composite.
When the job ends, check `errorCount` in `$reindex-status`: a resource whose
own `_bulk` request still fails after splitting is listed there with
Elasticsearch's `error.type`/`reason`, and a rerun fails the same way until its
cause is fixed. Raise `HFS_ELASTICSEARCH_NESTED_OBJECTS_LIMIT` for a
nested-object rejection, or `HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS` for a single
document that times out on its own. Before #1125, `$reindex` on
`sqlite-elasticsearch` failed every large `Provenance` chunk at the transport
level and did not repair them at all. After a failed rebuild the UI rebuild
banner stays up and points at `$reindex-status` instead of disappearing.
