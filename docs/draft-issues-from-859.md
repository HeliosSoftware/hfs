# Draft issues surfaced while implementing #859 — for review before filing

Found while resolving `PUT/DELETE [type]?[criteria]` inside transactions (branch
`feat/859-transaction-conditional-url`). None is fixed by that PR unless stated. Line numbers
are current-tree at the time of writing.

## D1. Transactional `DELETE [type]/[id]` never reaches a composite's secondaries

`CompositeStorage::sync_bundle_results` (`crates/persistence/src/composite/storage.rs`) syncs
a transaction's entries to secondaries by reading each `BundleEntryResult.resource`. An
instance-addressed `DELETE` answers `204` with no body and no `location`, so it is skipped:
on `sqlite-elasticsearch` a resource deleted inside a transaction stays searchable in
Elasticsearch until the next reindex. #859 fixes the *conditional* delete (its `204` now
carries `location`, and the sync emits `SyncEvent::Delete` for a `204` with one), but the
explicit form still answers a bare `204`.

Fix: have the three executors set `location` on every delete result (the deleted version's
URL), or have the composite derive the identity from the entry's URL, which it does not see
today. Test: `composite_conformance_sync` — a transaction `DELETE Patient/p1` followed by an
Elasticsearch search that must not return `p1`.

## D2. The spec fixture's `POST ValueSet/$lookup` entry is not a create

`crates/fhir/tests/data/json/R4/bundle-transaction.json` entry 7 is `POST ValueSet/$lookup`
with a `Parameters` body. Nothing in the bundle path recognises an operation URL: the REST
layer admits it as a mutation of type `ValueSet`, and the backends' `parse_url` would take
`$lookup` as an id. Today the entry fails on the resource-type mismatch (`Parameters` body under
`ValueSet`), which is at least a refusal; but the fixture as a whole cannot be replayed until
`$op` entries are either executed or declined with a message that names the operation.

Fix: detect `[type]/$op` and `[type]/[id]/$op` in `parse_bundle_entry`, and either dispatch
to the operation router or return `501` naming the operation. Then the fixture (minus the GET
entries, which #478 covers) becomes an end-to-end test.

## D3. `test-hfs` skill lacks the backend-specific test commands

`.claude/skills/test-hfs/SKILL.md` names testcontainers and the ES heap cap but not the
commands the suites actually need: `cargo test -p helios-persistence --features postgres --
postgres_integration`, `--features mongodb -- mongodb_integration`, the
`HFS_TEST_MONGODB_URL` escape hatch, or that MongoDB transaction tests skip on a standalone
topology. Every backend suite spells these out in its module docs instead.

## D4. `BundleError` index refers to the sorted entry order

`process_transaction` sorts entries DELETE → POST → PUT → GET before calling the backend, and
`TransactionError::BundleError { index }` (rendered as "Transaction failed at entry N") is the
index in *that* order, not the client's. The #859 overlap message names both entries by the
same sorted index. Mapping back to the original index is one lookup in the REST layer
(`indexed_entries[index].0`). Pre-existing; noticed because the overlap message makes it
visible.
