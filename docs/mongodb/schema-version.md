# MongoDB schema version

HFS records its MongoDB schema version in the `schema_version` collection of each database it initializes. Since #2352 a build refuses to start on a database that records a newer version than its own, and there is no override. This page describes the record, what each version adds, and how to roll back to an older build on purpose.

## The schema_version record

The collection holds exactly one document:

```javascript
{ _id: "schema_version", version: 16, search_indexes: { generation: 4, completed_at: ISODate("..."), contained_rows_moved: true } }
```

`search_indexes.empty_at_generation` appears only before the first build records a generation.

Every boot of a build from #2352 on does this, in order:

- It reads `version`. Any whole number counts, a mongosh double included.
- If the value is above the build's `SCHEMA_VERSION`, startup fails before anything is written. The error names the database and both versions, for example "MongoDB database 'helios' has schema_version 17, newer than the 16 this HFS build supports: the database was written by a newer HFS".
- Otherwise it runs every schema step: it creates its collections and indexes and drops the indexes a later version replaced. It does this on every boot, not only for versions above the recorded one, and each step changes nothing when its result already exists.
- Then it sets `version` to its own `SCHEMA_VERSION` unless the document already records a newer one. A build from #2352 on never lowers the recorded version. A build from before it records its own version, lower or not; see [Rolling back to an older build](#rolling-back-to-an-older-build).

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
| 8 | `idx_bulk_submit_files_manifest` (artifact identity under its manifest) replaces `idx_bulk_submit_files_part` | v0.2.2 | v0.2.0 and v0.2.1 have no `$bulk-submit` store and ignore it; no release records 7 to 9. An unreleased build at 7 would create the unique `idx_bulk_submit_files_part` again, which cannot be built if two manifests of one submission share a part, and the build then fails at boot. |
| 9 | `resources.idx_resources_type_scan` replaces `idx_resources_type_deleted` (#1021) | v0.2.2 | v0.2.0 and v0.2.1 create `idx_resources_type_deleted` again at boot (one `resources` index build before it serves); the newer build drops it again at its next start |
| 10 | `bulk_entry_results.idx_bulk_entry_results_outcome_line` replaces `idx_bulk_entry_results_outcome` (#1046) | v0.2.2 | v0.2.0 and v0.2.1 ignore it; no release records 7 to 9. An unreleased build at 9 or lower would create `idx_bulk_entry_results_outcome` again (a plain index, write overhead only); the newer build drops it again. |
| 11 | `login_sessions` (web UI login sessions, #1481), `idx_login_sessions_expires` | v0.2.4 | ignores it. v0.2.0 to v0.2.3 have no interactive web UI login (it arrived in v0.2.4), so no user signs in with it while one runs. The stored sessions stay as they are and are valid again after the re-upgrade until their `expires_at`; `db.login_sessions.deleteMany({})` before restarting the newer build signs everyone out. |
| 12 | `resources.idx_resources_live_tenant` (#1910, see [tenant-count-index.md](tenant-count-index.md)) | v0.2.5 | ignores it |
| 13 | `search_index_pending`, the ledger of a large transaction Bundle's deferred search rows, and `idx_search_index_pending_due` (#1975); such resources carry `search_rows_pending` | v0.2.5 | runs no deferred-index repair. Resources named by a ledger document left by a crash, a cancelled request or a failed batch have no search rows while it runs; check before rolling back. The newer build's repair finishes what is left after the re-upgrade. |
| 14 | `sof_terminology_registry` and `sof_terminology_rows`, the stored ValueSet and ConceptMap tables of SQL-on-FHIR Libraries (#2185) | v0.2.6 | does not read or write them. While it runs, `$sql-run` and `$sql-export` of a SQLQuery or SQLView Library that depends on a ValueSet or ConceptMap fail, even if the newer build already stored tables for it: v0.2.5 answers `501` (the MongoDB terminology store is new in v0.2.6, #2185), and v0.2.4 and earlier do not accept such a dependency at all. Library writes still succeed but store no tables. |
| 15 | `bulk_manifests.idx_bulk_manifests_index_pending` (#1283); a manifest whose deferred search-index rebuild is unfinished carries `index_pending: true` and `index_pending_types` | v0.2.6 | neither sets nor reads the marker, so it does not resume a rebuild a stopped server left outstanding; check before rolling back. The newer build resumes marked rebuilds at its next start. |
| 16 | `bulk_submission_changes.idx_bulk_changes_resource`, used by a resource or type purge (#2279) | after v0.2.6 | ignores it |

A change that raises `SCHEMA_VERSION` adds its row here. The `SCHEMA_VERSION` doc comment in `crates/persistence/src/backends/mongodb/schema.rs` lists the versions from 7 on too.

The version does not cover:

- The `search_index` generations and the contained-row move; see [Downgrading](search-indexes.md#downgrading). A build of generation 3 or lower (v0.2.3 or earlier) that takes writes also leaves date and composite rows the newer build cannot match until a full `$reindex`; see [Upgrading again](#upgrading-again).
- `missing_params` (from v0.2.5) and `missing_params_fp` (from v0.2.6) on `resources`, and the `missing_params_fingerprints` collection. An older writer can leave stale metadata ([Upgrade and definition maintenance](search-indexes.md#upgrade-and-definition-maintenance)), so after re-upgrading from v0.2.4 or v0.2.5, if it took writes, run the [metadata-only reindex](search-indexes.md#metadata-only-reindex) without `skipExistingMetadata`, which rewrites every resource's metadata (native `mongodb` only). After v0.2.3 or earlier the full `$reindex` above rewrites it too.

### Releases

| Release | `version` | `search_indexes.generation` | Refuses a newer `version` |
|---|---|---|---|
| v0.2.0, v0.2.1 | 4 | not recorded (generation 1) | no |
| v0.2.2, v0.2.3 | 10 | 3 | no |
| v0.2.4 | 11 | 4 | no |
| v0.2.5 | 13 | 4 | no |
| v0.2.6 | 15 | 4 | no |

Builds from #2352 on refuse a newer version; the build that introduced the refusal (#2352) has `SCHEMA_VERSION` 16, and a later build may have a higher one. There are two ways to read another build's value:

- From a checkout, read its source:

      git show v0.2.6:crates/persistence/src/backends/mongodb/schema.rs | grep 'pub const SCHEMA_VERSION'

- A build from #2352 on prints it when it refuses: in "newer than the 16 this HFS build supports", 16 is its version. The refused start stops before schema init writes anything (#2352). Only use this for a build you know is from #2352 on: a build from before it does not refuse, it starts on the database and records its own version. When in doubt, read the source as above.

## Rolling back to an older build

Builds from before #2352 (every release up to v0.2.6) do not check `version`. They start on a database a newer build upgraded, run their own schema steps and record their own, lower version (#2352). Rolling back to one of them needs no mongosh step, but the other steps apply, and once it has started the record no longer shows that a newer build wrote the database.

A rollback crosses version N when the older build's version is below N and the recorded version is N or higher.

The commands below read `HFS_MONGODB_URL` and `HFS_MONGODB_DATABASE`. Set them to the values HFS uses. HFS also takes the URL from `HFS_MONGODB_URI` or `HFS_DATABASE_URL`, and with no database name set it uses `helios`:

```bash
export HFS_MONGODB_DATABASE="${HFS_MONGODB_DATABASE:-helios}"
```

1. Check what the rollback crosses. Read the record:

   ```bash
   mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" --quiet --eval 'printjson(db.schema_version.findOne({ _id: "schema_version" }))'
   ```

   `null` means the command reached a database HFS never initialized: check the URL and the database name.

   Find the older build's version (see [Releases](#releases)). Read the table rows above it, up to the recorded version, and the notes under the table. For a version not in the table, read that release's `SCHEMA_VERSION` doc comment.

2. Only when crossing 13 or 15: let the newer build finish the work an older one does not resume, while it still runs.
   - `db.search_index_pending.countDocuments({})` must be 0. The repair runs at startup and every `HFS_MONGODB_DEFERRED_INDEX_REPAIR_INTERVAL` seconds (default 60) and takes only documents past a minimum age; see [Deferred search indexing](../../crates/persistence/README.md#deferred-search-indexing-for-large-mongodb-transaction-bundles). With the interval set to `0` the repair is off and the count does not drop on its own: set a positive interval and restart the newer build, or skip the wait and plan the `$reindex` below.
   - `db.bulk_manifests.countDocuments({ index_pending: true })` must be 0.
   - Clients can still write during this check. Step 3 runs both counts again once every instance has stopped.
   - If either is above 0 then, `$reindex` the affected types after the older build starts: the resources named in the ledger documents (`db.search_index_pending.find({}, { refs: 1 })`; each ref is `Type/id`), and the manifest's `index_pending_types` (`db.bulk_manifests.find({ index_pending: true }, { index_pending_types: 1 })`).

3. Stop every HFS instance that uses the database, including one that uses it only as a dedicated audit database. A newer instance started after step 5 records its version again, and the older build then refuses at its next start.

   When step 2 applies, run its two counts again now that nothing writes. A large transaction Bundle, a `$bulk-submit`, or a request the shutdown cut off can have left a ledger document or a marker after the first check. If either count is above 0, plan the `$reindex` from step 2.

4. Back up the database:

   ```bash
   mongodump --uri="$HFS_MONGODB_URL" --db="$HFS_MONGODB_DATABASE" --archive="hfs-before-rollback.archive"
   ```

   This is what you restore if the rollback goes wrong; restoring it discards every write made since:

   ```bash
   mongorestore --uri="$HFS_MONGODB_URL" --nsInclude="$HFS_MONGODB_DATABASE.*" --drop --archive="hfs-before-rollback.archive"
   ```

   If a dedicated audit database is in use (`HFS_AUDIT_MONGODB_DATABASE` or `HFS_AUDIT_DATABASE_URL` is set), back it up the same way. Its URL is `HFS_AUDIT_DATABASE_URL`, or the primary URL when that is unset; its name is `HFS_AUDIT_MONGODB_DATABASE`, or `HFS_MONGODB_DATABASE` when that is unset. Where both resolve to the primary's, it is the primary database and there is nothing more to do.

   ```bash
   mongodump --uri="${HFS_AUDIT_DATABASE_URL:-$HFS_MONGODB_URL}" --db="${HFS_AUDIT_MONGODB_DATABASE:-$HFS_MONGODB_DATABASE}" --archive="hfs-audit-before-rollback.archive"
   ```

   Its restore command:

   ```bash
   mongorestore --uri="${HFS_AUDIT_DATABASE_URL:-$HFS_MONGODB_URL}" --nsInclude="${HFS_AUDIT_MONGODB_DATABASE:-$HFS_MONGODB_DATABASE}.*" --drop --archive="hfs-audit-before-rollback.archive"
   ```

5. Only when the older build is from #2352 on, so that it refuses a newer version: lower the recorded version to the older build's version, in each database it initializes. A build from before #2352 does not check the version and records its own at start, so skip this step for it. The numbers below are an example, 17 to 16:

   ```bash
   mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" --quiet --eval 'printjson(db.schema_version.updateOne({ _id: "schema_version", version: 17 }, { $set: { version: NumberInt(16) } }))'
   ```

   Do the same in a dedicated audit database, with the URL and name step 4 resolves for it. Read its own record first (the command from step 1 pointed at it) and put the value it shows in the filter:

   ```bash
   mongosh "${HFS_AUDIT_DATABASE_URL:-$HFS_MONGODB_URL}/${HFS_AUDIT_MONGODB_DATABASE:-$HFS_MONGODB_DATABASE}" --quiet --eval 'printjson(db.schema_version.updateOne({ _id: "schema_version", version: 17 }, { $set: { version: NumberInt(16) } }))'
   ```

   - The result must show `matchedCount: 1`. `matchedCount: 0` means the database does not record 17: read it again.
   - The filter names the recorded value, so nothing changes if it moved meanwhile.
   - `NumberInt` stores a 32-bit integer, the type HFS writes.
   - Change only `version`; `search_indexes` belongs to the generation steps.

6. If the rollback also crosses a `search_index` generation, run the matching script from [Downgrading](search-indexes.md#downgrading). Builds from #2352 on are all generation 4 so far.

7. Start the older build. Its schema steps create nothing new, except that v0.2.0 and v0.2.1 build `idx_resources_type_deleted` and the generation-1 indexes. It records its own version and ignores what the table lists above its version. v0.2.0 and v0.2.1 also replace the whole `schema_version` document at each start, which removes `search_indexes` (the generation and `contained_rows_moved`) along with the old version. The newer build then treats the generation as unrecorded: it moves the contained rows again and logs the `$reindex` reminder for #1391.

## Upgrading again

Stop the older build(s) and start the newer one. Its startup check passes because the recorded version is lower. It runs every schema step again: indexes it finds are left as they are, and superseded ones it finds are dropped. Then it records its own version. The `search_index` builder handles the recorded generation as [search-indexes.md](search-indexes.md#downgrading) describes. It does not rewrite rows the older build wrote; the last two follow-ups below cover that.

Then follow up:

- Crossed 13: the repair finishes leftover ledger documents; no action.
- Crossed 14: write the SQLQuery and SQLView Libraries created or updated meanwhile again. Until they are written again, the newer build expands the dependencies they lack through the terminology server (`HFS_TERMINOLOGY_SERVER`) on demand, when one is set. A Library deleted meanwhile keeps its stored tables, because the older build never released them. Writing a Library again refreshes the tables it uses; to release the deleted one's tables, create it again with the same id and delete it again, or leave them.
- Crossed 15: marked rebuilds resume at start; no action.
- The older build was v0.2.3 or earlier (`search_indexes.generation` 3 or lower, or none recorded for v0.2.0 and v0.2.1, see [Releases](#releases)) and took writes: run a full `POST /$reindex`. These builds write date rows without `value_date_end` (#1391) and composite rows without `composite_slot`. Until the rows are rewritten, date searches with the `eq`, `ne`, `gt`, `ge`, `le`, `eb` and `ap` prefixes miss them, and a same-type composite search fails with an error asking for `$reindex`. The newer build logs a `warn` naming `$reindex` at that first start. See [Upgrading: reindex the date rows](search-indexes.md#upgrading-reindex-the-date-rows) and, for the tenant-wide route the composite rows need, [Composite parameters](search-indexes.md#composite-parameters). The full `$reindex` also rewrites `missing_params`, so the metadata-only reindex below is not needed then.
- The older build was v0.2.4 or v0.2.5 and took writes: run the metadata-only reindex without `skipExistingMetadata`.

CI pins the boot behaviour this relies on in `crates/persistence/tests/mongodb/schema_version_guard.rs`.
