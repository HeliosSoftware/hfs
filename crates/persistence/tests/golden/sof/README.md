These snapshots contain the complete generated SQLite/PostgreSQL query,
typed bindings in slot order, output columns/decodes and client cap. Inline
views cover flat/forEach, nullable/nested/chained/cartesian, legacy
union/repeat/indexed, collections/join/first picks, constants and runtime
filters including deterministically resolved Group documents.

Normal tests compare every byte and never update files. Regenerate explicitly
from the repository root, then review the whole snapshot diff:

```sh
HFS_UPDATE_SOF_GOLDEN=1 cargo test -p helios-persistence --lib --features postgres sof::golden_tests
```

Run the same command without `HFS_UPDATE_SOF_GOLDEN` to verify. The internal
unit module is included by ordinary workspace CI; no database is required.
