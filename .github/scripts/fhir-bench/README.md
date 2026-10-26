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

## Report outputs

Each run produces, in this order:

- **Job summary text.** The comparison tables (unchanged), then a collapsed `### Charts` section: one for the compare job (`md_charts.py compare --in comparison.md`) and one per leg (`md_charts.py leg --box ... --leg ... --run-dir ...`). Both are display only and never fail the job.
- **Run history block** (`history.py`): the last 5 runs of the branch plus the latest `main` run, as tables and Mermaid charts. Old and new row labels are joined (see `LABEL_ALIASES`), except the old `index rebuild` row, which is a different quantity from `search indexing` and stays apart.
- **Standalone HTML report**, artifact `fhir-bench-report-<run id>`: one dark file (`report_html.py`, template `report_template.html`), uploaded with `archive: false` so it is not zipped. It opens in the browser through the `Full report (HTML)` link on the last line of the summary; that link works for signed-in viewers of the repository only.
  Sections: run summary, 1 Is this run valid?, 2 Results, 3 Detail (timeline and host load, Bulk Import and Batch/Transaction Import by phase, import comparison, `$sql-run` by view, trend), 4 Full data. CI passes `--no-shapes`, so there is no "Search by query shape" section.
- **Report data**, artifact `fhir-bench-report-data-<run id>` (`report-data.json`). Later runs read it (`history.py --data-dir`) for the trend in the HTML report.

All four artifacts and the summary are best effort (`continue-on-error`); a failure here never fails the benchmark.

## Run the generators locally

```bash
gh run download <run id> -D DIR          # DIR/<artifact name>/...
cd .github/scripts/fhir-bench
python3 md_charts.py compare --in DIR/fhir-bench-comparison-<run id>/comparison.md > charts.md
python3 report_html.py --run-dir DIR --run-id <run id> --no-shapes \
  --out report.html --dump-json report-data.json
```

Drop `--no-shapes` to include the search-by-shape section when `search-points.json` is present. `md_charts.py leg` also needs the leg's box Markdown (`--box`), which CI captures inside the leg job.

## Test GitHub rendering cheaply

Push a throwaway `tmp/` branch whose workflow is triggered by `push`, with `permissions: actions: read`, and have it download the artifacts of an earlier run (`actions/download-artifact` with `run-id` and `github-token`). It then runs the generators above and appends to `$GITHUB_STEP_SUMMARY`, with no servers and no benchmark. Delete the branch afterwards.

## Known limits

- The job summary cannot be previewed locally; check it on GitHub.
- Artifact links, including the HTML report, need a signed-in session with access to the repository.
