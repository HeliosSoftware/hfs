# FHIR benchmark scripts

Scripts for `.github/workflows/fhir-benchmark.yml`. The workflow runs one job per storage leg; these files start the backends, run the suites and build the summary.

## Legs

Six legs: `sqlite`, `postgres`, `mongodb`, `sqlite-elasticsearch`, `postgres-elasticsearch`, `mongodb-elasticsearch` (`resolve-matrix.sh` maps the `backend` input to legs).

## Suites, in run order

1. `prewarm`
2. `import`
3. `crud`
4. `search`
5. `sql-run` (HFS's own read-only SQL-on-FHIR suite, `k6/sql-run.js`)
6. `insert`
7. `bulk-import` (`bulk-import.sh` and `bulk_import.py`; not a k6 suite; `$bulk-submit` into a fresh database; always last)

The `indexing` (`$reindex`) suite was removed. Recover it from git history if needed.

## Indexing mode per leg (`bulk-import`)

| Leg | Mode |
|---|---|
| `sqlite`, `postgres`, `mongodb` | deferred (`HFS_BULK_SUBMIT_DEFER_INDEXING=true`): ingest, then one index rebuild |
| the three `*-elasticsearch` legs | inline (`HFS_BULK_SUBMIT_DEFER_INDEXING=false`) with file concurrency 4, batch size 1000, Elasticsearch bulk concurrency 4 |

`bulk-import.sh` sets these; a variable already set in the environment wins. Reasons and measurements: [`$bulk-submit` indexing mode per storage backend](../../../crates/rest/README.md#bulk-submit-indexing-mode-per-storage-backend).

## Report rows

On the Elasticsearch legs the rows `↳ search indexing (seconds)` read `(= ingest)`: the index was built while ingesting, so the value is the ingest time, not a separate rebuild.
