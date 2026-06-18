# SQL-on-FHIR v2 conformance fixtures

The JSON files in `tests/` are the official SQL-on-FHIR v2 declarative test
fixtures, kept here as a **verbatim** copy of upstream so they can run in CI.

- **Upstream source:** <https://github.com/FHIR/sql-on-fhir.js/tree/main/tests>
  (raw: `https://raw.githubusercontent.com/FHIR/sql-on-fhir.js/main/tests/<file>.json`)
- **Policy:** files are copied byte-for-byte. Do not hand-edit them — re-sync from
  upstream instead, so a plain `diff` against upstream stays meaningful.

To re-sync:

```sh
dir=crates/sof/tests/sql-on-fhir-v2/tests
for f in "$dir"/*.json; do
  name=$(basename "$f")
  curl -fsS "https://raw.githubusercontent.com/FHIR/sql-on-fhir.js/main/tests/$name" -o "$f"
done
# Pick up any newly-added upstream files too (e.g. a future `fn_*.json`):
# list the upstream `tests/` directory and `curl` anything missing locally.
```

## Who runs these

- `crates/sof/tests/test_runner_integration.rs` — runs every fixture against the
  in-memory `run_view_definition` engine and asserts **all** pass.
- In-DB runners, each running every fixture against a real backend via the REST
  endpoint with a `PASS_FLOOR` ratchet (some fixtures exercise constructs a given
  in-DB compiler does not yet cover):
  - `crates/rest/tests/sof_conformance.rs` — SQLite (in-memory, no Docker; floor 132).
  - `crates/rest/tests/sof_conformance_postgres.rs` — PostgreSQL via testcontainers (floor 132).
  - `crates/rest/tests/sof_conformance_mongodb.rs` — MongoDB via testcontainers
    (floor 59; the Stage-1 aggregation emitter doesn't yet cover `unionAll`,
    `repeat`, collections, and several functions).

## Notes

- `constant_types.json` omits the top-level `fhirVersion` field; the runners
  treat a missing `fhirVersion` as "applies to all versions".
- Row comparison is order-insensitive (matching the upstream compliance runner,
  which canonicalizes rows before comparing).

### `%rowIndex` support by backend

`%rowIndex` (the iteration-position environment variable) is implemented per
execution engine:

| Backend | Engine | `%rowIndex` |
|---------|--------|-------------|
| SQLite | in-DB SQL compiler | full (top-level, forEach/forEachOrNull, repeat) |
| PostgreSQL | in-DB SQL compiler | full (`WITH ORDINALITY` for forEach, pre-order `ord_path` in the repeat CTE) |
| S3 | in-process `helios-sof` engine | full (the engine's own `forEach`/`repeat` support) |
| MongoDB | in-DB aggregation (Stage 1) | top-level + forEach/forEachOrNull (`$unwind … includeArrayIndex`); `repeat`/`unionAll` are unsupported by the Stage-1 emitter regardless |
| Elasticsearch | — | no SOF runner; SOF is not served on ES |

Across SQLite and PostgreSQL, 8 of the 9 `row_index.json` fixtures pass in-DB;
the 9th (`%rowIndex in unionAll inside forEach`) depends on the orthogonal,
pre-existing "unionAll nested inside another select" gap.
