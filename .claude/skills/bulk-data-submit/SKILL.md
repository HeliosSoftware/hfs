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
| `HFS_BULK_SUBMIT_BATCH_SIZE` | `100` | Resources per ingestion batch, one database transaction each. Reaches the worker since #1127; before that it was parsed and ignored, and every run used `100` whatever it said. Do not raise it without measuring: with index-during-ingest on, `1000` ingested slower than `100` (382 s against 252 s, 1 % cut) |
| `HFS_BULK_SUBMIT_FETCH_READ_TIMEOUT` | `60` | Seconds the input-file fetcher waits for the next bytes of a response before treating the body as broken and resuming it with `Range`; connecting is capped separately at 10 s. Must be `> 0` |
| `HFS_BULK_SUBMIT_SKIP_UNCHANGED` | `false` | SQLite and PostgreSQL: leave a stored resource untouched when the submitted one is identical to it apart from `meta.versionId`/`meta.lastUpdated`, so replaying a manifest writes no new versions, history rows or index rows. The entry's receipt still reads `success` |
| `HFS_BULK_SUBMIT_INDEX_DURING_INGEST` | `false` | Composite deployments with an Elasticsearch secondary: index each committed batch into the secondary while ingesting, instead of leaving it to the post-manifest rebuild. The deferred reindex then runs only for resource types the secondary rejected something from. No effect without a search secondary. See Ingest performance |
| `HFS_BULK_SUBMIT_INDEX_QUEUE` | `16` | Committed batches each index-during-ingest writer may hold queued. Measured: a queue of `32` on top of coalesce `16` made ingest a further 65 % slower |
| `HFS_BULK_SUBMIT_INDEX_CONCURRENCY` | `4` | Index-during-ingest writer tasks. A resource always goes to the same writer, chosen by a hash of type and id, so its versions reach the secondary in order |
| `HFS_BULK_SUBMIT_INDEX_COALESCE` | `4` | Queued batches one writer merges into a single write to the secondary. Measured: `16` made ingest 34 % slower |
| `HFS_BULK_SUBMIT_INDEX_MAX_WAIT` | `30` | Seconds the ingest waits for room in a writer's queue. Past it the batch is marked unindexed and left to the deferred reindex, so a slow secondary never stalls the writer or starves the lease |
| `HFS_BULK_SUBMIT_DEFER_INDEXING` | `true` | Bulk fast-load (#903): ingest without search-index/FTS writes, then rebuild with an automatic per-type reindex when each manifest finishes. Default since #946; honoured on MongoDB only since #1000, where it was silently inert. Read once at startup, not per submission. A restart before that rebuild lands leaves the data stored but unsearchable; set `false` to close that window — see Ingest performance |
| `HFS_BULK_SUBMIT_BULK_INDEX_REBUILD` | `false` | SQLite: the deferred rebuild drops the `search_index` value indexes for its duration and builds them once, sorted, at the end. 1.3x at 72k resources, 1.6x at 312k, widening with size — but search on that database is unindexed for every tenant while a rebuild runs, and the final build holds the write lock. For the initial load of a large corpus on a server not serving traffic. Self-heals at startup if a process died inside the window |
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
- A status-only kick-off (no `manifestUrl`, no `replacesManifestUrl`) that restates the terminal status a submission already has — `completed` on a `complete` submission, `stopped` on an `aborted` one — answers `200` again (#998): a Data Provider whose first close-out timed out client-side while the server committed it can only re-send, and a `409` would read as a refusal. Anything else against a terminal submission (a manifest, a replacement, the other terminal status) is still `409`.
- `BulkSubmitProvider::get_submission_status` is a point read of the submission row/document on every backend (PostgreSQL since #1154, MongoDB and SQLite since #998); `get_submission` is the one that aggregates receipts, so never call it on a status poll, a status-only kick-off, or the lease keeper's abort watch. `complete_submission` returns `()` for the same reason — it no longer re-reads the summary. Measured at 11M receipts on MongoDB, the aggregation was ~26s per call.
- The Import page (`crates/ui/src/bulk_import.rs`) keeps a status change the recipient never *answered* (timeout, refused connection) as `pendingStatus` on the submission and re-sends it from the status card's refresh every 15s, up to 10 attempts, while the page is open; a refusal (any non-2xx answer) is never retried. Operator writes go through a compare-and-swap that re-loads and re-applies on a version conflict, so a *Mark completed* pressed during a live ingest no longer loses the race to the 5s status poll.
- For `requiresAccessToken` files, HFS acquires a read-scoped token via SMART Backend Services using `client_credentials` and `private_key_jwt` when `HFS_BULK_SUBMIT_CLIENT_ID` and `HFS_BULK_SUBMIT_PRIVATE_KEY` are set.
- If credentials are absent for `requiresAccessToken` files, fetches record a manifest-level error.
- `deleted` files, either transaction Bundles or resource refs, are applied as deletes.
- Partial success remains `200` with a populated `error[]` array of OperationOutcome NDJSON. Per-entry errors leave the manifest `completed`; a file that could not be read to its end makes it `failed` (see below).
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
- Status-poll pacing: the `202` advertises `HFS_BULK_SUBMIT_RETRY_AFTER` once bytes or entries are being counted, and the shorter `HFS_BULK_SUBMIT_PRE_INGEST_RETRY_AFTER` before that (so the #953 phase reports `Queued - starting shortly` / `Reading manifest` / `Sizing N of M files` / `Downloading file N of M` are seen rather than slept through; once every file is pulled the counter line carries a trailing ` - Downloaded N of N files`, #1218), and a client that polls past `HFS_BULK_SUBMIT_POLL_RATE_LIMIT` within the window gets `429` plus a `Retry-After` pointing at the end of that window. Buckets are keyed by poll token plus principal, falling back to peer address; the check runs before any job-store work, so throttled polls stay cheap.
- File fan-out is backend-aware. `HFS_BULK_SUBMIT_FILE_CONCURRENCY` is honoured as configured on the concurrent-writer backends (PostgreSQL, MongoDB, S3), but file fan-out is **not supported on SQLite**: `effective_file_concurrency` returns `1` there whatever the operator configured, and a `WARN` at startup names the configured and effective values. SQLite serialises writers, so any fan-out above one queues each batch's writes behind a single exclusive lock until they outlast `busy_timeout` and abort the manifest outright. Any file fan-out at all requires PostgreSQL.
- **A file that cannot be read to its end fails the manifest (#1127).** Before #1127 a body that broke mid-stream abandoned the rest of that file, wrote one file-level `error` line, and still published the manifest `completed`, with nothing logged at `WARN` — measured at 4–8 truncated files per run against `python -m http.server`. Now any `output` or `deleted` file that cannot be fetched, or cannot be read to its end after the retries below, publishes the manifest as `failed`. The batches committed before the break stay stored, the receipts and the `error` artifact are still published, and the deferred reindex still runs, so what was stored becomes searchable. Per-entry problems (validation errors, processing errors, resources the secondary did not index) do not fail the manifest; they stay partial success. Consumers that read the manifest's status must treat `failed` as "some files are incomplete", not as "nothing was imported".
- **A broken body is resumed with `Range` before the file is given up.** The fetcher counts the bytes it has received on the wire. On a body error or read timeout it re-requests the file with `Range: bytes=<n>-` and `If-Range` set to the first response's `ETag` or `Last-Modified`, at most 3 times, backing off 1, 2 and 4 s. A `206` whose `Content-Range` starts at `n` continues the file where it broke. A `200` to a request that carried no validator is read from the start and its first `n` bytes are skipped. A `200` answering an `If-Range` means the file changed and fails it, as does a `416`. A response sent with `Content-Encoding` is not resumed by `Range`, since the offset would not match; it is retried whole only when no bytes had been handed to the ingest yet. JWE files are buffered whole anyway, so they retry the whole `GET`. Every break is logged at `WARN` with the redacted URL, the bytes consumed, the attempt and the error's full source chain — the plain `error decoding response body` that reached the artifact before hid the actual `ConnectionReset`.
- **Input URLs are redacted wherever HFS reports them (#1127).** The query string, the fragment and any `user:password@` are stripped before a manifest or file URL goes into an error message, the manifest's `error` artifact or the log, so a presigned URL's signature does not leak into artifacts other principals can read.
- **A lost lease is never silent (#1127).** On SQLite the worker renews the lease *before* the between-files WAL checkpoint, which now runs `PRAGMA wal_checkpoint(PASSIVE)` and then `TRUNCATE` under a 1 s busy timeout, logging WAL frames, bytes and duration at `info` (`WARN` past 5 s). Each heartbeat attempt runs on a blocking thread, so the lease keeper's timeout can fire while SQLite is locked. `LeaseLost`, a heartbeat that timed out and a manifest reclaimed from another worker's expired lease are each logged at `WARN`. Before #1127 a multi-gigabyte `TRUNCATE` or a slow secondary could hold the write lock until the lease expired unseen, and the sibling worker re-read every file from the first: +11 h on the 19M-resource run (measured, older code).
- **Re-walking a file does not inflate the manifest counters (#1127).** Progress is recorded per file and the manifest's counters are the sum over its files, so a file ingested again — after a reclaim, or on a replay — counts its entries once instead of once per pass (#998 saw `total_entries` 17.9 M for 11.2 M resources; the 19M run reported 37,911,730 for 18,955,865 receipts). Combined with `HFS_BULK_SUBMIT_SKIP_UNCHANGED` a replay leaves the database size and `resource_history` unchanged; without it every identical resource still becomes a new version.
- **The submission summary no longer scans the receipts (#1127, #998).** `get_submission` — read on every `$bulk-submit-status` poll and every few seconds by each running worker's lease keeper — is served from the manifest counters on SQLite, PostgreSQL and MongoDB instead of aggregating `bulk_entry_results`, which grows to one row per ingested resource. Its cost no longer grows with the import. The per-entry detail stays in the receipts and the status manifest's artifacts, which remain authoritative.
- **Elasticsearch `_bulk` requests are capped by bytes as well as operations.** A request holds at most 500 operations and 10 MiB, whichever comes first; an operation larger than the cap goes alone. Before #1127 only the count was capped, and a page of large `Provenance` documents failed chunk-wide as `backend unavailable` (most plausibly the client's 30 s request timeout), which made the deferred rebuild of the 1 % cut give up with 2,500 resources unindexed.
- The manifest bookkeeping and resource writes retry with bounded exponential backoff when SQLite reports the database busy or locked, instead of failing the ingest. The retry budget is an elapsed-time deadline bounded by the manifest lease, so a retrying write can never outlive the lease it holds. Every other error still surfaces on the first attempt.
- With `HFS_BULK_SUBMIT_DEFER_INDEXING=true` (bulk fast-load, #903 — **the default since #946**) ingestion skips the search-index and FTS writes and an automatic per-type reindex rebuilds them when each manifest finishes. Reads and history are complete throughout; search sees a manifest's resources once its reindex lands. That rebuild is started *after* the manifest is already terminal and is fire-and-forget (`bulk_submit_worker.rs` → `reindex.rs`, `tokio::spawn`), so `$bulk-submit-status` answers `200` while search is still incomplete, and the job lives only in an in-memory map — no column on `bulk_manifests` records that indexing is outstanding and nothing re-fires it at startup. A restart in that window is not recoverable on its own.
- MongoDB ingests a batch, not an entry: one `find` resolves which of the batch's ids already exist, then one `insert` or `update` command per collection writes the whole batch (`backends/mongodb/bulk_ingest.rs`). Before #1000 each entry cost ~9 round trips of its own — a `read`, `create`'s second existence probe, the resource and history inserts, a search-index delete and insert, a transaction commit, the rollback record and the receipt — which pinned ingest at ~60–76 resources/s with `mongod` two-thirds idle. The batch flush is a sequence of commands rather than one transaction. Every command is retried on a transient driver error (`RetryableError`/`RetryableWriteError` label, I/O error, cleared pool — not a server-selection timeout) with 100 ms doubling backoff capped at 1 s over six attempts, checking the submission's cancel token before each sleep; a retry never duplicates what an earlier attempt landed (resources are re-read and matched on version + the batch's own `last_updated` + content, history and rollback rows dedupe on their unique keys, the search index is cleared before re-insert). When a stage outlives its retries the batch's entries get `processing-error` receipts with issue code `transient` and the file continues with its next batch, so `max_errors`/`continue_on_error` govern backend failures too (#1001); re-submitting the file converges. Only a receipt write that itself fails after retries still aborts the file. The manifest counters are a `$inc` and may over-count one batch if a retried attempt had actually landed — the receipts are authoritative.
- **On a composite deployment (primary + Elasticsearch), the ingest engine does not reach the secondary by itself.** Ingestion runs on the *primary's* engine, and the primary deliberately skips its own indexing when search is offloaded — so `main.rs` wraps the primary's job store in `CompositeSubmitJobs`, which syncs each manifest's ingested resources into the secondary. Under the default deferred path (`HFS_BULK_SUBMIT_DEFER_INDEXING=true` with a reindex hook) the wrapper stays in the chain in `with_ingest_sync(false)` mode: its per-resource manifest sync is off, because the post-manifest rebuild fills Elasticsearch, but `rollback_change` still mirrors each revert to the secondaries and the #1125 ledger methods reach the primary. Before #1161 that path handed the worker the raw primary, so a rolled-back create stayed in Elasticsearch as an orphan that search matched and `GET` by id answered `404`. Abort does not delete ingested resources from the primary (#968), so it does not touch Elasticsearch either. Every composite mode must call `composite_submit_jobs(...)`; `mongo-es` and `s3-es` did not, and a completed import there was readable by id and invisible to every search — 15.27M of 15.28M resources on the reported deployment, with `GET` by id passing every smoke test (#1021). `crates/hfs/tests/bulk_submit/run_composite_es_index_check.sh` asserts the searchable count, not just readability, and is the guard against a fourth composite backend repeating it.
- The sync itself runs *before* the manifest's receipt is written (#1007), as an explicit worker step — not at `finish_manifest`, which no longer syncs by itself, so a manifest that already reached a terminal state is never re-synced by a restart; repair it with `$reindex`. A resource the secondary still rejects after its retries gets an entry result of `processing-error` in the receipt, carrying an OperationOutcome (`incomplete`) that names the `Type/id`, the rejecting backend, and `POST /{type}/$reindex` as the repair; the resource itself stays stored and readable by id, and the status's `failed_entries` counts it. If the rejection is Elasticsearch's nested-object limit (`The number of nested documents has exceeded the allowed limit`), `$reindex` fails the same way until `HFS_ELASTICSEARCH_NESTED_OBJECTS_LIMIT` (default 50000, raised on existing indices at startup) is above that resource's nested value count (#1050). Raising the limit is necessary but not sufficient: `$reindex` sends the same documents through the same `_bulk` path, and on `sqlite-elasticsearch` before #1125 every 500-document `Provenance` request failed as a whole at the transport level (`backend unavailable: elasticsearch`, `retryable: true`), so `$reindex` indexed none of them. Treat `$reindex` as the repair only once its status reports `errorCount` 0 for the type. Each failed resource is listed, with its error and whether it is retryable, in `GET /$reindex-status/{job_id}`.
- **Deferred rebuild survives a restart (#1125).** A manifest ingested with `HFS_BULK_SUBMIT_DEFER_INDEXING=true` records that it still owes a search-index rebuild (`bulk_manifests.index_pending`, SQLite schema v30), set in the transaction that publishes the manifest and cleared when the rebuild finishes. On startup the server scans for those manifests and re-fires the same hook, logging `resuming search-index rebuilds left outstanding by an earlier run`. Other backends do not record it and so do not resume.
- **Rebuild knobs (#1125).** `HFS_ELASTICSEARCH_BULK_CONCURRENCY` (default `1`) sends several `_bulk` requests of one page at once; `HFS_REINDEX_BATCH_BYTES` (default `0` = off) caps a page by bytes so ~108 KB `Provenance` resources do not make a ~108 MB page. On `sqlite-es` the recommended pair for an import is `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for` with `HFS_ELASTICSEARCH_REINDEX_REFRESH=false`: measured on a 228,580-resource cut the rebuild went from 806 s to 145 s, complete either way.
- **Elasticsearch `_bulk` shape (#1125).** The ingest sync and every rebuild (`$reindex` and the deferred post-import rebuild) send documents through one `_bulk` path, capped per request at 500 operations *and* `HFS_ELASTICSEARCH_BULK_MAX_BYTES` (default 10 MiB). A request that exceeds `HFS_ELASTICSEARCH_REQUEST_TIMEOUT_MS` (default 30000) or is answered `413` (or a proxy's `408`/`504`) is split in half and resent, recursively, down to one document; once a single document times out, the rest of the page fails as transient instead of being split further. A `429`, for the request or per item, is retried with bounded exponential back-off for the rejected items only. A per-document `4xx` stays permanent, and an `ensure_index` transport failure is transient. A connection error, a whole-request `5xx`, or a `429` that outlasts its retries fails every document of that request as transient. Every failed document is recorded by `Type/id`, with Elasticsearch's `error.type`/`reason` when it gave one. Before #1125 there was no byte cap: 500 Synthea `Provenance` resources (~108 KB each) made a ~54 MB request that timed out, all 11,704 `Provenance` of the 1 % cut failed as `backend unavailable: elasticsearch`, and the automatic retry re-ran every type and failed identically.
- A client-side timeout does not cancel the `_bulk` Elasticsearch is already executing, so a resource reported as failed may still have indexed. Confirm with `GET /{type}?_summary=count` before rebuilding again.
- On `sqlite-elasticsearch` the rebuild writes only to Elasticsearch. Before #1125 it also wrote the offloaded SQLite `search_index`/FTS, which no query there reads, and because the matching delete was a no-op those rows accumulated on every run (at least 11 KB per resource).
- A deferred rebuild generation whose resource errors include *transient* ones (#1125) retries only those resources, by id, once; a failure of the job itself (a page fetch error, a panic) still retries its types. Before #1125 any transient error re-ran every type of the generation. Every resource the rebuild fails to index is logged at `warn` with `Type/id` and the reason, rate-limited per type with a per-type summary, for transient and permanent failures alike, and both closing failure lines name up to five `Type/id`s. A SQLite row the source cannot parse is recorded as a permanent error for that resource instead of silently ending the rebuild of its type.
- After that copy, for every resource type the manifest ingested, the worker compares the primary's tenant-wide resource count against each secondary's. A mismatch is recorded as a `warning` OperationOutcome (also `incomplete`, naming both counts) in the manifest's `error` artifact and logged on the server. This check only runs when `HFS_COMPOSITE_SYNC_MODE` is `synchronous` or `hybrid`; under the default `asynchronous` mode the secondary's count reflects whatever had already drained from its queue rather than this manifest's own sync, so the check is skipped and the receipt then guarantees only that the resources committed on the primary.
- Without `HFS_ELASTICSEARCH_WRITE_REFRESH=wait_for`, a small count difference can be a write that has not become visible yet rather than a real gap; reconfirm with `GET /{type}?_summary=count` before treating it as drift. See "Verifying and repairing search drift" below.
- **Under deferred indexing the rebuild is the import.** On SQLite the fast-load
  ingest of a 72k-resource Synthea mix takes ~3s and the rebuild that follows
  took ~42s, so the rebuild is where import time goes, and it is the path the
  SQLite ingest work targets. What it does now (`storage.rs`,
  `prepare_index` / `write_prepared_index`): each page's FHIRPath extraction,
  value normalisation and parameter marshalling run on a thread pool
  (`HFS_INDEX_THREADS`, default: the machine's parallelism) *before* the
  page's transaction opens, so the single SQLite writer runs statements only;
  `search_index` rows go eight per INSERT as on the inline path; the page is
  1,000 resources (`DEFERRED_REINDEX_BATCH_SIZE`, overridable with `HFS_REINDEX_BATCH_SIZE`), not `$reindex`'s 100, since
  each page is one COMMIT; `idx_search_string_folded` is partial (schema v28)
  so 87% of rows skip a b-tree it never found them by; and `param_url` is no
  longer written (11% of a bulk-loaded database, read by nothing). The inline
  path (`HFS_BULK_SUBMIT_DEFER_INDEXING=false`) prepares each batch the same
  way. Measured with `bulk_submit_bench` on the same 72k mix, three
  interleaved rounds against `main`, medians: fast-load + rebuild 41.8s ->
  21.5s end to end (1.94x, 3 of 3 rounds), inline ingest 36.4s -> 25.0s
  (1.45x, 3 of 3), database 15% smaller; at 312k resources of the same mix,
  180.7s -> 110.1s (1.64x) — the gain narrows as the b-trees outgrow the page
  cache, which is the regime the full 19M-resource corpus lives in. Two
  later additions: composite groups missing a component are no longer
  written (`drop_incomplete_composites`; they were 46% of composite rows and
  can never match — PostgreSQL already skipped them), and the opt-in
  `HFS_BULK_SUBMIT_BULK_INDEX_REBUILD` above, which is the lever for that
  regime: with it, 72k resources went 21.0s -> 16.3s and 312k went 110.1s ->
  69.7s end to end (2.6x over the pre-work numbers at both sizes), because
  `CREATE INDEX` builds each index from one sort instead of one random
  b-tree insertion per row per index. Priced and rejected on the same
  harness: narrowing `idx_search_composite`, FTS5 `automerge`/`pgsz`
  tuning, `wal_autocheckpoint=0`, `synchronous=OFF` (4%), dropping the two
  display-text indexes (5%, at a search cost). `perf_phases`
  says what is left is SQLite b-tree maintenance of `search_index` and its
  indexes plus the `search_index_fts` triggers (~20% of the rebuild, priced by
  dropping them; kept, since `:text-advanced` needs the FTS index).
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
a lot. The `bulk_submit_bench` example without `--reindex` runs **no reindex at
all**, so its ~6.7x is the cost of ingestion with the indexing work removed, not
the cost of arriving at a searchable database (`--batch 100 --defer-index
--reindex` is the server's default path end to end, and it reports the two stages
separately). Measured end to end against a running server — kick-off until a
search returns the full count — the gain is far smaller, because the deferred arm
still has to pay for the rebuild:

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

### Index during ingest (#1127)

`HFS_BULK_SUBMIT_INDEX_DURING_INGEST=true` is the lever for composite
deployments with an Elasticsearch secondary, and it is **opt-in**. Under the
default fast-load path the secondary receives nothing while the manifest
ingests: `composite_submit_jobs` hands the worker `CompositeSubmitJobs` with its
ingest sync turned off (`with_ingest_sync(false)`, #1161), and search is filled
only by the deferred rebuild that fires after the manifest is terminal. The
wrapper stays there so rollbacks still reach the secondary. With this switch
on, the job store wraps the primary in an indexing sink instead:

- The ingest engine hands each batch to the sink **right after its transaction
  commits**, never before, so the secondary only ever sees what the primary
  durably holds.
- `HFS_BULK_SUBMIT_INDEX_CONCURRENCY` writer tasks take the batches from
  bounded queues (`_INDEX_QUEUE`). Each resource is routed by a hash of its
  type and id, so a late duplicate cannot overwrite a newer version. A writer
  merges up to `_INDEX_COALESCE` queued batches into one `_bulk` write.
- Handing a batch over waits at most `_INDEX_MAX_WAIT`. Past that, and for
  anything the secondary rejects, the entries are marked unindexed
  (`processing-error`, `incomplete`, naming `POST /{type}/$reindex`) and the
  ingest moves on. A slow or saturated secondary degrades to unindexed
  entries; it never blocks the writer, and so never starves the lease.
- Before writing receipts the worker drains the sink, within the same bound
  (#1007), so a receipt never reads `success` for a resource search cannot
  find. The deferred reindex then runs only for the resource types with
  rejected entries, and not at all when nothing was rejected.

Measured on the 1 % Synthea cut (228,580 resources, release R4-only build,
isolated Elasticsearch 8.15.0 with a 4 GB heap, on `7967a483e`, i.e. `main`
before #1109):

| Run | Ingest | Rebuild | Search complete | ES root docs at end |
|---|---|---|---|---|
| Deferred rebuild (default) | 151 s | 403 s + 441 s retry, both failed | never: gives up at 995 s | 226,080 / 228,580 |
| Index during ingest | 225 s, indexing included | none | **225 s** | **228,580 / 228,580** |

The ingest itself is slower because it now includes the indexing; the rebuild
it replaces was what never finished. The failed rebuild is the `_bulk` sizing
defect described under Behavior Notes, now capped by bytes. On older code the
same switch took the end-to-end import from ~37 min to ~4 min.

Keep the shaping knobs at their defaults unless you measure. Raising coalescing
from 4 to 16 took ingest from 245 s to 329 s, and a queue of 32 on top took it
to 548 s; raising `HFS_BULK_SUBMIT_BATCH_SIZE` to 1000 took it from 252 s to
382 s (all measured, older code). An unbounded flush into a saturated
secondary is exactly what expired the lease of the 19M-resource run.

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
