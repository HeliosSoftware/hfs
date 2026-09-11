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
| poll or manifest | GET | `/bulk-submit-status/{poll_token}` | `202` in-progress with `X-Progress` and `Retry-After`; `200` plus status manifest when done; `429` plus `Retry-After` when rate-limited; `404` after delete |
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
| `HFS_BULK_SUBMIT_DEFER_INDEXING` | `true` | Bulk fast-load (#903): ingest without search-index/FTS writes, then rebuild with an automatic per-type reindex when each manifest finishes. Default since #946; honoured on MongoDB only since #1000, where it was silently inert. Read once at startup, not per submission. A restart before that rebuild lands leaves the data stored but unsearchable; set `false` to close that window — see Ingest performance |
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
| `HFS_BULK_SUBMIT_RETRY_AFTER` | `120` | `Retry-After` seconds advertised on an in-progress status poll once ingestion has started |
| `HFS_BULK_SUBMIT_PRE_INGEST_RETRY_AFTER` | `10` | `Retry-After` seconds advertised while pre-ingest (queued / reading manifest / sizing / downloading); clamped to `RETRY_AFTER` and to `POLL_RATE_WINDOW / POLL_RATE_LIMIT` |
| `HFS_BULK_SUBMIT_POLL_RATE_LIMIT` | `10` | Status polls per client, per submission, per window; `0` disables |
| `HFS_BULK_SUBMIT_POLL_RATE_WINDOW` | `60` | Sliding window for the poll rate limit, in seconds |
| `HFS_BULK_SUBMIT_BLOCK_CONCURRENT_SUBMISSION` | `false` | Reject a new submission while one is in-progress; returns `429` |
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
- Status-poll pacing: the `202` advertises `HFS_BULK_SUBMIT_RETRY_AFTER` once bytes or entries are being counted, and the shorter `HFS_BULK_SUBMIT_PRE_INGEST_RETRY_AFTER` before that (so the #953 phase reports `Queued - starting shortly` / `reading manifest` / `sizing N of M files` / `downloading file N of M` are seen rather than slept through), and a client that polls past `HFS_BULK_SUBMIT_POLL_RATE_LIMIT` within the window gets `429` plus a `Retry-After` pointing at the end of that window. Buckets are keyed by poll token plus principal, falling back to peer address; the check runs before any job-store work, so throttled polls stay cheap.
- File fan-out is backend-aware. `HFS_BULK_SUBMIT_FILE_CONCURRENCY` is honoured as configured on the concurrent-writer backends (PostgreSQL, MongoDB, S3), but file fan-out is **not supported on SQLite**: `effective_file_concurrency` returns `1` there whatever the operator configured, and a `WARN` at startup names the configured and effective values. SQLite serialises writers, so any fan-out above one queues each batch's writes behind a single exclusive lock until they outlast `busy_timeout` and abort the manifest outright. Any file fan-out at all requires PostgreSQL.
- The manifest bookkeeping and resource writes retry with bounded exponential backoff when SQLite reports the database busy or locked, instead of failing the ingest. The retry budget is an elapsed-time deadline bounded by the manifest lease, so a retrying write can never outlive the lease it holds. Every other error still surfaces on the first attempt.
- With `HFS_BULK_SUBMIT_DEFER_INDEXING=true` (bulk fast-load, #903 — **the default since #946**) ingestion skips the search-index and FTS writes and an automatic per-type reindex rebuilds them when each manifest finishes. Reads and history are complete throughout; search sees a manifest's resources once its reindex lands. That rebuild is started *after* the manifest is already terminal and is fire-and-forget (`bulk_submit_worker.rs` → `reindex.rs`, `tokio::spawn`), so `$bulk-submit-status` answers `200` while search is still incomplete, and the job lives only in an in-memory map — no column on `bulk_manifests` records that indexing is outstanding and nothing re-fires it at startup. A restart in that window is not recoverable on its own.
- MongoDB ingests a batch, not an entry: one `find` resolves which of the batch's ids already exist, then one `insert` or `update` command per collection writes the whole batch (`backends/mongodb/bulk_ingest.rs`). Before #1000 each entry cost ~9 round trips of its own — a `read`, `create`'s second existence probe, the resource and history inserts, a search-index delete and insert, a transaction commit, the rollback record and the receipt — which pinned ingest at ~60–76 resources/s with `mongod` two-thirds idle. The batch flush is a sequence of commands rather than one transaction, on purpose: the per-entry path was not atomic across a batch either, and a batch-wide transaction would turn one transient error into a whole batch of lost entries (#1001). Commands are ordered resources → history → search index → rollback log → receipts, so an interrupted batch is re-processed rather than falsely reported done.
- **On a composite deployment (primary + Elasticsearch), the ingest engine does not reach the secondary by itself.** Ingestion runs on the *primary's* engine, and the primary deliberately skips its own indexing when search is offloaded — so `main.rs` wraps the primary's job store in `CompositeSubmitJobs`, which syncs each manifest's ingested resources into the secondary. Every composite mode must call `composite_submit_jobs(...)`; `mongo-es` and `s3-es` did not, and a completed import there was readable by id and invisible to every search — 15.27M of 15.28M resources on the reported deployment, with `GET` by id passing every smoke test (#1021). `crates/hfs/tests/bulk_submit/run_composite_es_index_check.sh` asserts the searchable count, not just readability, and is the guard against a fourth composite backend repeating it.
- The sync itself runs *before* the manifest's receipt is written (#1007), as an explicit worker step — not at `finish_manifest`, which no longer syncs by itself, so a manifest that already reached a terminal state is never re-synced by a restart; repair it with `$reindex`. A resource the secondary still rejects after its retries gets an entry result of `processing-error` in the receipt, carrying an OperationOutcome (`incomplete`) that names the `Type/id`, the rejecting backend, and `POST /{type}/$reindex` as the repair; the resource itself stays stored and readable by id, and the status's `failed_entries` counts it.
- After that copy, for every resource type the manifest ingested, the worker compares the primary's tenant-wide resource count against each secondary's. A mismatch is recorded as a `warning` OperationOutcome (also `incomplete`, naming both counts) in the manifest's `error` artifact and logged on the server. This check only runs when `HFS_COMPOSITE_SYNC_MODE` is `synchronous` or `hybrid`; under the default `asynchronous` mode the secondary's count reflects whatever had already drained from its queue rather than this manifest's own sync, so the check is skipped and the receipt then guarantees only that the resources committed on the primary.
- Without `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for`, a small count difference can be a write that has not become visible yet rather than a real gap; reconfirm with `GET /{type}?_summary=count` before treating it as drift. See "Verifying and repairing search drift" below.
- **`HFS_BULK_SUBMIT_DEFER_INDEXING` is read once, at startup, not per submission.** `spawn_submit_workers` hands the configured value to each worker, and the worker applies it to every manifest it ingests from then on; nothing about the flag is persisted on a submission or manifest record. Restarting with a different value changes only manifests ingested *after* the restart.
- Cleanup periodically removes status artifacts for submissions whose `updated_at` exceeds `HFS_BULK_SUBMIT_OUTPUT_TTL`.
- Abort (`submissionStatus=stopped`) means **stop soon** — neither "stop this instant" nor "stop after the current file". It marks the submission `aborted`, moves its `pending`/`processing` manifests to `failed`, and bars further claims. A manifest already being ingested is stopped cooperatively: the lease keeper re-reads the submission's status on its flush cadence (a few seconds) and trips a cancel token the streaming engine checks between persisted batches, so the worst case is one keeper tick plus one batch, never mid-transaction (#968).
- A manifest stopped that way keeps the counts it had already recorded — the entries it ingested are **not** rolled back. It writes no `output`/`error` receipts and does not restate its own status, since the abort already owns the outcome.
- Once an abort has settled a manifest at `failed`, nothing an in-flight worker does may move it back. Three writes enforce that: `finish_manifest` and `fail_manifest` are guarded on `status = 'processing'` (a late verdict returns `LeaseLost` and is a no-op), and both `mark_manifest_processing` and the per-batch status stamp inside the ingest are guarded on `status IN ('pending','processing')` so they only ever *promote* a manifest. Without the last of those the very next batch after an abort silently reset `failed` to `processing` and the abort read as if it had never happened.

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
(the dashboard in the UI, or the backend's `count`). Note this is the
type's tenant-wide count, not just this manifest's entries — a drift can
predate the manifest that surfaced it.

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

## Ingest performance

The lever depends on the backend, and the two are not interchangeable. On SQLite
and PostgreSQL the write path was already batched and the lever is
`HFS_BULK_SUBMIT_DEFER_INDEXING`. On MongoDB the binding constraint until #1000
was round trips, not work — see *MongoDB* below before quoting any of the SQLite
numbers at a MongoDB deployment.

`HFS_BULK_SUBMIT_DEFER_INDEXING=true` relocates search indexing to a post-ingest
reindex and is by far the biggest lever on ingestion alone; the stored resources,
history, receipts and rollback records are identical either way.

Which number you quote depends on where you stop the clock, and the two differ by
a lot. The `bulk_submit_bench` example runs **no reindex at all**, so its ~6.7x is
the cost of ingestion with the indexing work removed, not the cost of arriving at
a searchable database. Measured end to end against a running server — kick-off
until a search returns the full count — the gain is far smaller, because the
deferred arm still has to pay for the rebuild:

| stop the clock at | `false` | `true` | ratio | rounds won by `true` |
|---|---|---|---|---|
| `$bulk-submit-status` returns `200` | 26.4s | 7.9s | 3.34x | 15 of 15 (p<0.0001) |
| search returns every ingested resource | 26.8s | 22.0s | **1.22x** | 15 of 15 (p<0.0001) |

(15 interleaved rounds, 10 000 Patients per arm, release build, SQLite, on an
otherwise idle machine; columns are medians and the ratio divides them. Comparing
the two arms within each round instead and taking the median of those 15 paired
ratios gives 1.19x, range 1.06x–1.42x — the sounder statistic, since it never
compares across rounds. Reproduce with
`crates/hfs/tests/bulk_submit/run_defer_indexing_benchmark.sh`.)

Quote **~1.2x**, not 6.7x and not the 1.31x an earlier contended run produced.
That earlier run had six servers from other worktrees on the box, identical
work varied ~8x round to round, and `t_search` came out 7 of 9 — inconclusive.
Re-running idle collapsed the spread to ~1.5x and made both directions
unambiguous while revising the end-to-end gain slightly *down*. Absolute times
are not comparable between the two runs; only ratios within an interleaved run
are.

The same run also prices the window this buys, which is the other half of the
trade. Time between `$bulk-submit-status` answering `200` and search actually
being complete, at 10 000 resources:

| | median | range |
|---|---|---|
| `false` | 0.4s | 0.3–0.5s |
| `true` | **14.0s** | 0.5–18.7s |

That window is when the API reports success while search is still wrong, and it
is not load-sensitive in kind, only in width: `run_defer_indexing_crash_check.sh`
restarts the server inside it and finds 16 000 of 20 000 resources stored,
readable by id, and permanently absent from search.

**The default is `true`** — that trade was decided on #946 in favour of import
speed, with the window accepted as a documented caveat. Report both numbers when
asked about it: the gain is ~1.2x end to end, not 6.7x, and the cost is a
restart window that operators live with unless they set
`HFS_BULK_SUBMIT_DEFER_INDEXING=false`. Making the rebuild durable — persisted
on `bulk_manifests`, re-fired at startup, surfaced in `$bulk-submit-status` —
would close the window without giving up the speed, and is not done.

### MongoDB

MongoDB was a different problem, and the SQLite numbers above never applied to
it. Its ingest made ~9 round trips per resource, so it ran at a rate set by
network latency rather than by how much work the server had to do — #1000
measured ~60 resources/s with `mongod` at 32 % CPU and HFS idle. #1000 batches
the whole path: one `find` to resolve existing ids, then one command per
collection.

Measured on a MongoDB 7.0 single-node replica set, 4 GB WiredTiger cache, 3 000
Synthea `Condition` + 3 000 `Patient`, release build, batch 1 000:

| | resources/s | vs. before |
|---|---|---|
| before #1000 | 76 | — |
| batched | 720 | 9.5x |
| batched + `DEFER_INDEXING=true` | 3 137 | 41x |

Document counts per resource are identical before and after (1 `resources`, 1
`resource_history`, ~21 `search_index`, 1 receipt, 1 rollback record): the win is
entirely round trips removed, not work skipped. The third row is the one that
skips work, and it is the row that only exists after #1000 — the switch reached
`create`/`update`, which have no way to be told to skip indexing, so on MongoDB
it did nothing. A MongoDB deployment on the default `true` was therefore indexing
inline *and* rebuilding the same index in the post-manifest reindex: the index
work twice, for a switch whose whole point is to do it once.

Two things do **not** follow from those numbers:

- **The decay is a separate problem.** #1000 also measured throughput falling
  with corpus size — 300 resources/s at 2.5 M, 96 at 11 M — because 21
  `search_index` documents times 11 non-sparse indexes is ~230 index-key
  insertions per resource, and the resulting index set outgrows the WiredTiger
  cache (50.7 GB of `search_index` indexes against a 7 GB cache at 8.5 M
  resources). Batching does not touch that; it is a write-volume problem, and the
  fix is a narrower index set (`partialFilterExpression` on the value indexes —
  `value_number` is populated on 0 % of documents, `value_date` on 7 %,
  `value_uri` on 4 %; two of the eleven, `idx_search_composite` and
  `idx_search_identifier_type`, index fields the MongoDB search implementation
  never queries at all). That is a schema migration that rebuilds indexes on the
  largest collection in the deployment, so it is not in #1000.
- **`DEFER_INDEXING=true` buys less end to end than 41x**, for the same reason it
  does on SQLite: the rebuild still has to run. Quote the ingest number as an
  ingest number.

To find out where the rest of the time goes, profile the write path with the
phase counters in `helios_persistence::perf` and the `bulk_submit_bench`
example — both gated behind `--cfg perf_phases`, which keeps them out of
released binaries (`ci.yml` builds those with `--all-features`, so a cargo
feature could not have). See `/test-hfs` for the invocation and for the two
traps that have produced wrong numbers here (the search-parameter data
directory, and comparing runs across sessions instead of interleaving arms).
