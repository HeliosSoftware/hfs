These snapshots contain the complete generated SQLite/PostgreSQL query,
typed bindings in slot order, output columns/decodes and client cap. Inline
views cover flat/forEach, nullable/nested/chained/cartesian, legacy
union/repeat/indexed, collections/join/first picks, `exists(criteria)` over
a path with several repeating steps, constants and runtime
filters including deterministically resolved Group documents.

Normal tests compare every byte and never update files. Regenerate explicitly
from the repository root, then review the whole snapshot diff:

```sh
HFS_UPDATE_SOF_GOLDEN=1 cargo test -p helios-persistence --lib --features postgres sof::golden_tests
```

Run the same command without `HFS_UPDATE_SOF_GOLDEN` to verify. The internal
unit module is included by ordinary workspace CI; no database is required.

`unbound/` holds frozen main-branch copies of the PostgreSQL goldens whose
statements now bind the resource document once per row (a
`jsonb_extract_path(..) AS rdoc(doc)` lateral). The test
`postgres_sql_without_document_binding_is_main_x_sql` switches that binding
off and requires every PostgreSQL case to reproduce the main-branch bytes:
the `unbound/` copy when one exists, otherwise the ordinary golden. The update
flag never writes `unbound/`. Change those files only deliberately, together
with the bound golden, when a change is meant to alter the unbound SQL too. A
new case whose PostgreSQL golden binds the document needs its own `unbound/`
copy, and a copy whose golden no longer binds the document must be deleted:
the test rejects it. The rejoin of a `repeat` is never bound, so the `repeat`
goldens are main-branch bytes themselves and have no copy.
