# MongoDB schema version

HFS records its MongoDB schema version in the `schema_version` collection of each database it initializes. Since #2352 a build refuses to start on a database that records a newer version than its own, and there is no override. This page describes the record, what each version adds, and how to roll back to an older build on purpose.

## The schema_version record

The collection holds exactly one document:

```javascript
{ _id: "schema_version", version: 16, search_indexes: { generation: 4, completed_at: ISODate("..."), contained_rows_moved: true } }
```

`search_indexes.empty_at_generation` appears only before the first build records a generation.

Every boot does this, in order:

- It reads `version`. Any whole number counts, a mongosh double included.
- If the value is above the build's `SCHEMA_VERSION`, startup fails before anything is written. The error names the database and both versions, for example "MongoDB database 'helios' has schema_version 17, newer than the 16 this HFS build supports: the database was written by a newer HFS".
- Otherwise it runs every schema step: it creates its collections and indexes and drops the indexes a later version replaced. It does this on every boot, not only for versions above the recorded one, and each step changes nothing when its result already exists.
- Then it sets `version` to its own `SCHEMA_VERSION` unless the document already records a newer one. HFS never lowers the recorded version.

No step rewrites documents.

`version` and `search_indexes.generation` are separate records. `version` is what the startup check compares. `search_indexes.generation` tells the `search_index` builder which value indexes it built; a build sets it to its own generation, up or down, with no check; see [Downgrading](search-indexes.md#downgrading). Lowering `version` does not change the generation, and the generation rollback scripts do not change `version`.

A database audit sink with its own MongoDB database (`HFS_AUDIT_MONGODB_DATABASE` or `HFS_AUDIT_DATABASE_URL` set) is initialized the same way and has its own `schema_version` document.

## Schema versions

Version 4 is the first MongoDB schema; a database with no record counts as 0. Every later version adds collections or indexes or replaces an index, and none rewrites documents.

MongoDB keeps every index current on writes whichever build runs. An older build ignores indexes and collections it does not know, and an unknown index costs only its write overhead. Dropping one is optional; the newer build creates it again at its next start (one index build, at boot).

| Version | Adds or changes | First release | An older build without it |
|---|---|---|---|
| 4 | `resources`, `resource_history`, `search_index` and their indexes | v0.2.0 | – |
| 5 | `user_settings` (web UI settings), `idx_user_settings_key` | v0.2.2 | ignores it |
| 6 | `tenants` (tenant registry), `idx_tenants_id` | v0.2.2 | ignores it |
| 7 | `$bulk-submit` job store: `bulk_submissions`, `bulk_manifests`, `bulk_entry_results`, `bulk_submission_changes`, `bulk_submit_files` and their indexes | v0.2.2 | v0.2.0 and v0.2.1 have no MongoDB `$bulk-submit` store and ignore them |
| 8 | `idx_bulk_submit_files_manifest` (artifact identity under its manifest) replaces `idx_bulk_submit_files_part` | v0.2.2 | no release recorded 5 to 9 |
| 9 | `resources.idx_resources_type_scan` replaces `idx_resources_type_deleted` (#1021) | v0.2.2 | v0.2.0 and v0.2.1 create `idx_resources_type_deleted` again at boot (one `resources` index build before it serves); the newer build drops it again at its next start |
| 10 | `bulk_entry_results.idx_bulk_entry_results_outcome_line` replaces `idx_bulk_entry_results_outcome` (#1046) | v0.2.2 | – |
| 11 | `login_sessions` (web UI login sessions, #1481), `idx_login_sessions_expires` | v0.2.4 | ignores it; the sessions stay as they are |
| 12 | `resources.idx_resources_live_tenant` (#1910, see [tenant-count-index.md](tenant-count-index.md)) | v0.2.5 | ignores it |
| 13 | `search_index_pending`, the ledger of a large transaction Bundle's deferred search rows, and `idx_search_index_pending_due` (#1975); such resources carry `search_rows_pending` | v0.2.5 | runs no deferred-index repair. Resources named by a ledger document left by a crash, a cancelled request or a failed batch have no search rows while it runs; check before rolling back. The newer build's repair finishes what is left after the re-upgrade. |
| 14 | `sof_terminology_registry` and `sof_terminology_rows`, the stored ValueSet and ConceptMap tables of SQL-on-FHIR Libraries (#2185) | v0.2.6 | does not read or write them. SQLQuery and SQLView Libraries created or updated while it runs are not reflected; write them again after the re-upgrade. A dependency with no stored table is expanded through the terminology server on demand. |
| 15 | `bulk_manifests.idx_bulk_manifests_index_pending` (#1283); a manifest whose deferred search-index rebuild is unfinished carries `index_pending: true` and `index_pending_types` | v0.2.6 | neither sets nor reads the marker, so it does not resume a rebuild a stopped server left outstanding; check before rolling back. The newer build resumes marked rebuilds at its next start. |
| 16 | `bulk_submission_changes.idx_bulk_changes_resource`, used by a resource or type purge (#2279) | after v0.2.6 | ignores it |

A change that raises `SCHEMA_VERSION` adds its row here. The `SCHEMA_VERSION` doc comment in `crates/persistence/src/backends/mongodb/schema.rs` lists the versions too.

The version does not cover:

- The `search_index` generations and the contained-row move; see [Downgrading](search-indexes.md#downgrading).
- `missing_params` (from v0.2.5) and `missing_params_fp` (from v0.2.6) on `resources`, and the `missing_params_fingerprints` collection. An older writer can leave stale metadata ([Upgrade and definition maintenance](search-indexes.md#upgrade-and-definition-maintenance)), so after re-upgrading from v0.2.5 or earlier, if it took writes, run the [metadata-only reindex](search-indexes.md#metadata-only-reindex) without `skipExistingMetadata`, which rewrites every resource's metadata (native `mongodb` only).

### Releases

| Release | `version` | `search_indexes.generation` | Refuses a newer `version` |
|---|---|---|---|
| v0.2.0, v0.2.1 | 4 | not recorded (generation 1) | no |
| v0.2.2, v0.2.3 | 10 | 3 | no |
| v0.2.4 | 11 | 4 | no |
| v0.2.5 | 13 | 4 | no |
| v0.2.6 | 15 | 4 | no |

Builds from #2352 on refuse a newer version; at #2352 `SCHEMA_VERSION` is 16. There are two ways to read another build's value:

- From a checkout, read its source:

      git show v0.2.6:crates/persistence/src/backends/mongodb/schema.rs | grep 'pub const SCHEMA_VERSION'

- A build from #2352 on prints it when it refuses: in "newer than the 16 this HFS build supports", 16 is its version. The refused start stops before schema init writes anything (#2352).

## Rolling back to an older build

Builds from before #2352 (every release up to v0.2.6) do not check `version`. They start on a database a newer build upgraded, run their own schema steps and record their own, lower version (#2352). Rolling back to one of them needs no mongosh step, but the other steps apply, and once it has started the record no longer shows that a newer build wrote the database.

A rollback crosses version N when the older build's version is below N and the recorded version is N or higher.

1. Check what the rollback crosses. Read the record:

   ```bash
   mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" --quiet --eval 'printjson(db.schema_version.findOne({ _id: "schema_version" }))'
   ```

   Find the older build's version (see [Releases](#releases)). Read the table rows above it, up to the recorded version, and the notes under the table. For a version not in the table, read that release's `SCHEMA_VERSION` doc comment.

2. Only when crossing 13 or 15: let the newer build finish the work an older one does not resume, while it still runs.
   - `db.search_index_pending.countDocuments({})` must be 0. The repair runs at startup and every `HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL` seconds and takes only documents past a minimum age; see [Deferred search indexing](../../crates/persistence/README.md#deferred-search-indexing-for-large-mongodb-transaction-bundles).
   - `db.bulk_manifests.countDocuments({ index_pending: true })` must be 0.
   - If either stays above 0, `$reindex` the affected types after the older build starts: the resources named in the ledger documents, and the manifest's `index_pending_types`.

3. Stop every HFS instance that uses the database, including one that uses it only as a dedicated audit database. A newer instance started after step 5 records its version again, and the older build then refuses at its next start.

4. Back up the database:

   ```bash
   mongodump --uri="$HFS_MONGODB_URL" --db="$HFS_MONGODB_DATABASE" --archive="hfs-before-rollback.archive"
   ```

   This is what you restore if the rollback goes wrong; restoring it discards every write made since.

5. Lower the recorded version to the older build's version, in each database it initializes (also a dedicated audit database). The numbers below are an example, 17 to 16:

   ```bash
   mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" --quiet --eval 'printjson(db.schema_version.updateOne({ _id: "schema_version", version: 17 }, { $set: { version: NumberInt(16) } }))'
   ```

   - The result must show `matchedCount: 1`. `matchedCount: 0` means the database does not record 17: read it again.
   - The filter names the recorded value, so nothing changes if it moved meanwhile.
   - `NumberInt` stores a 32-bit integer, the type HFS writes.
   - Change only `version`; `search_indexes` belongs to the generation steps.

6. If the rollback also crosses a `search_index` generation, run the matching script from [Downgrading](search-indexes.md#downgrading). Builds from #2352 on are all generation 4 so far.

7. Start the older build. Its schema steps create nothing new, except that v0.2.0 and v0.2.1 build `idx_resources_type_deleted` and the generation-1 indexes. It records its own version and ignores what the table lists above its version.

## Upgrading again

Stop the older build(s) and start the newer one. Its startup check passes because the recorded version is lower. It runs every schema step again: indexes it finds are left as they are, and superseded ones it finds are dropped. Then it records its own version. The `search_index` builder handles the recorded generation as [search-indexes.md](search-indexes.md#downgrading) describes.

Then follow up:

- Crossed 13: the repair finishes leftover ledger documents; no action.
- Crossed 14: write the SQLQuery and SQLView Libraries created or updated meanwhile again.
- Crossed 15: marked rebuilds resume at start; no action.
- The older build was v0.2.5 or earlier and took writes: run the metadata-only reindex without `skipExistingMetadata`.

CI pins the boot behaviour this relies on in `crates/persistence/tests/mongodb/schema_version_guard.rs`.
