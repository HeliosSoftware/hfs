//! Post-boot builder for the generation-4 `search_index` indexes (#1059,
//! #1084, #1160, #1391). Also moves any contained rows still sitting in
//! `search_index` (a pre-#1160 database) into `search_index_contained`,
//! ahead of any index build, in every [`IndexBuildMode`].

use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// How `SearchIndexBuilder` runs relative to boot. Read from
/// `HFS_MONGODB_INDEX_BUILD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexBuildMode {
    /// Spawn the builder after boot and return immediately (default).
    #[default]
    Background,
    /// Await the builder before boot completes. Tests and small databases.
    Inline,
    /// Inspect and warn only. The operator builds out of band.
    Off,
}

impl FromStr for IndexBuildMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "background" => Ok(Self::Background),
            "inline" => Ok(Self::Inline),
            "off" => Ok(Self::Off),
            other => Err(format!(
                "HFS_MONGODB_INDEX_BUILD must be one of background, inline, off; got {other:?}"
            )),
        }
    }
}

impl IndexBuildMode {
    /// Reads `HFS_MONGODB_INDEX_BUILD` from the process environment: absent
    /// means [`IndexBuildMode::default`] (`Background`), present but invalid
    /// is the same error message [`FromStr`] produces. The single place this
    /// variable is parsed, so every caller (`MongoBackend::from_env` and the
    /// `hfs` binary) agrees on what "invalid" means.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("HFS_MONGODB_INDEX_BUILD") {
            Ok(raw) => raw.parse::<IndexBuildMode>(),
            Err(_) => Ok(IndexBuildMode::default()),
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn parses_the_three_modes_case_insensitively() {
        assert_eq!(
            "background".parse::<IndexBuildMode>(),
            Ok(IndexBuildMode::Background)
        );
        assert_eq!(
            " Inline ".parse::<IndexBuildMode>(),
            Ok(IndexBuildMode::Inline)
        );
        assert_eq!("OFF".parse::<IndexBuildMode>(), Ok(IndexBuildMode::Off));
        assert!("sometimes".parse::<IndexBuildMode>().is_err());
    }

    #[test]
    fn default_is_background() {
        assert_eq!(IndexBuildMode::default(), IndexBuildMode::Background);
    }
}

use std::time::Duration;

use futures::stream::TryStreamExt;
use mongodb::{
    Database,
    bson::{Bson, Document, doc},
};

use crate::error::{BackendError, StorageError, StorageResult};

use super::schema::{
    contained_rows_moved, drop_index_if_present, get_search_index_generation,
    set_contained_rows_moved, set_search_index_generation,
};
use super::search_index_catalog::{
    IndexBuild, SEARCH_INDEX_COLLECTION, SEARCH_INDEX_CONTAINED_COLLECTION,
    SEARCH_INDEX_GENERATION, SearchIndexSpec, create_indexes_command, current_specs,
    superseded_contained_spec, superseded_date_v2_spec, superseded_v1_specs,
};

/// What one run of the builder did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildOutcome {
    /// Every background spec present and ready, no superseded index left.
    UpToDate,
    /// Built the named indexes and dropped the named superseded ones.
    Built {
        /// Names of the background specs that were created.
        created: Vec<String>,
        /// Names of the superseded indexes that were dropped (generation-1
        /// value indexes, `idx_search_date_v2`, the old contained index).
        dropped: Vec<String>,
    },
    /// `IndexBuildMode::Off`: these background specs are missing; nothing changed.
    Skipped {
        /// Names of the background specs that are missing.
        missing: Vec<String>,
    },
    /// The run stopped. Nothing was dropped. The message is what was logged.
    Failed {
        /// What was logged.
        message: String,
    },
}

/// How long to wait between `listIndexes` polls while another process's
/// build of one of our names is in progress.
const IN_PROGRESS_POLL: Duration = Duration::from_secs(30);

/// Bound on how long `run_inner` waits for a *foreign* build (another
/// process's `createIndexes`, or a `buildUUID` that never clears) of one of
/// our generation-2 names. Our own `createIndexes` in step 2 is not subject
/// to this: that one command is bounded by a single collection scan and is
/// awaited directly, not through this poll loop. Six hours is generous for
/// even a very large collection scan and still short enough that `inline`
/// mode (which awaits this from `init_schema`) does not hang boot forever
/// (I3).
const IN_PROGRESS_MAX_WAIT: Duration = Duration::from_secs(6 * 60 * 60);

/// Page size for [`SearchIndexBuilder::move_contained_rows`]. Small enough
/// that a crash mid-move only replays one page's worth of work on the next
/// boot, large enough that moving even a sizeable backlog of contained rows
/// takes few round trips.
const MOVE_PAGE: usize = 1_000;

/// `listIndexes` option keys, beyond `v`/`key`/`name`/`partialFilterExpression`,
/// that change query results or index maintenance if present on a
/// generation-2 name: `unique` and `sparse` change which documents the index
/// admits, `expireAfterSeconds` deletes rows, `collation` changes string
/// comparison (and therefore sort order and equality), `hidden` removes the
/// index from planner consideration, `wildcardProjection`/`storageEngine`
/// change what the index covers or how it is stored, and `weights`/
/// `default_language` are text-index-only options that have no place on a
/// key/partial-filter index at all. Any of these on our name means a person
/// (or a script) built something under it that is not what the catalog
/// describes (I2).
const EXTRA_OPTION_KEYS: [&str; 9] = [
    "unique",
    "sparse",
    "expireAfterSeconds",
    "collation",
    "hidden",
    "wildcardProjection",
    "storageEngine",
    "weights",
    "default_language",
];

/// One `listIndexes` reading, classified against the catalog.
#[derive(Debug, Default)]
struct Inspection {
    /// Background specs absent from the collection.
    missing: Vec<SearchIndexSpec>,
    /// Our names that exist but carry a `buildUUID`: someone else is building them.
    in_progress: Vec<String>,
    /// Our names that exist with a different key, partial filter, or extra option.
    conflicting: Vec<(String, ListedIndex)>,
    /// Superseded names still present: generation-1 index names, the
    /// generation-2 `idx_search_date_v2` (#1391), plus the generation-2
    /// `search_index` contained-row index name if it is still there.
    superseded_present: Vec<String>,
}

/// One `listIndexes` `firstBatch` entry, normalised to the fields this module
/// cares about, regardless of whether the server reported it "ready" (fields
/// at the top level) or "in progress" (fields nested under `spec`, alongside
/// a top-level `buildUUID` — only reported when the command carries
/// `includeBuildUUIDs: true`). See [`listed_indexes`].
#[derive(Debug, Clone, PartialEq)]
struct ListedIndex {
    name: String,
    key: Document,
    partial: Option<Document>,
    in_progress: bool,
    /// Names of any [`EXTRA_OPTION_KEYS`] present on this entry (or its
    /// `spec`). Non-empty means a generation-2 name is conflicting even when
    /// its key and partial filter match the catalog (I2).
    extra_options: Vec<String>,
}

/// Whether moving to the current generation should warn that date rows
/// written before #1391 lack `value_date_end` (#1391), given the generation
/// recorded before this boot (`None`: never recorded) and whether
/// `search_index` is empty.
///
/// Only on the transition: a database already at the current generation has
/// nothing left to say, and an empty one (a new database, or a collection
/// nobody has written to) has no old rows to reindex.
fn unindexed_date_ranges_warning_due(recorded: Option<i32>, search_index_is_empty: bool) -> bool {
    recorded.is_none_or(|generation| generation < SEARCH_INDEX_GENERATION) && !search_index_is_empty
}

pub(super) struct SearchIndexBuilder {
    database: Database,
    mode: IndexBuildMode,
}

impl SearchIndexBuilder {
    pub(super) fn new(database: Database, mode: IndexBuildMode) -> Self {
        Self { database, mode }
    }

    /// Inspect, build, drop, record. Never returns an `Err`: every failure is
    /// logged and reported as `BuildOutcome::Failed`, because this runs
    /// detached from boot and nothing is waiting to handle an error.
    pub(super) async fn run(self) -> BuildOutcome {
        match self.run_inner().await {
            Ok(outcome) => outcome,
            Err(e) => {
                let message =
                    format!("search_index generation-{SEARCH_INDEX_GENERATION} build failed: {e}");
                tracing::error!(error = %e, "{message}");
                BuildOutcome::Failed { message }
            }
        }
    }

    async fn run_inner(&self) -> StorageResult<BuildOutcome> {
        // A correctness fix, not an index build: runs in every mode,
        // including `off`, and ahead of everything else below (#1160).
        // Bounded by the number of contained rows, which is small on every
        // deployment, so this never meaningfully delays boot even in
        // `inline` mode.
        // Wrap the move's own errors with a distinguishing prefix: this
        // whole function's errors are otherwise reported by `run`'s
        // catch-all as "search_index generation-N build failed: ...", which
        // would mislabel a move failure as an index-build failure.
        let moved = self.move_contained_rows().await.map_err(|e| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!("contained-row move failed: {e}"),
                source: Some(Box::new(e)),
            })
        })?;
        if moved > 0 {
            tracing::info!(
                moved,
                "moved contained search_index rows to search_index_contained"
            );
        }

        let mut inspection = self.inspect().await?;

        if !inspection.conflicting.is_empty() {
            let names: Vec<String> = inspection
                .conflicting
                .iter()
                .map(|(n, _)| n.clone())
                .collect();
            for (name, actual) in &inspection.conflicting {
                let expected_spec = current_specs().into_iter().find(|s| s.name == name);
                let expected_keys = expected_spec.as_ref().map(|s| s.keys.clone());
                let expected_partial = expected_spec.as_ref().and_then(|s| s.partial.clone());
                tracing::error!(
                    index = %name,
                    expected_keys = ?expected_keys,
                    expected_partial = ?expected_partial,
                    actual = ?actual,
                    "search_index index exists under a generation-2 name with a different spec; \
                     refusing to build or drop anything. Drop or rename it by hand."
                );
            }
            let message = format!(
                "conflicting index spec under generation-2 name(s): {}",
                names.join(", ")
            );
            return Ok(BuildOutcome::Failed { message });
        }

        if self.mode == IndexBuildMode::Off {
            return match off_mode_decision(&inspection) {
                None => {
                    self.record_if_needed().await?;
                    Ok(BuildOutcome::UpToDate)
                }
                Some(outcome) => Ok(outcome),
            };
        }

        // Someone else (another HFS process, or an operator's mongosh) is
        // building one of our names: wait for it rather than issue a second
        // build of the same index. Bounded (I3): in `inline` mode this is
        // awaited by `init_schema`, so an unbounded wait on a foreign build
        // that is stuck (or a `buildUUID` that never clears) would hang
        // boot forever with no way out.
        let wait_started = std::time::Instant::now();
        while !inspection.in_progress.is_empty() {
            if wait_started.elapsed() >= IN_PROGRESS_MAX_WAIT {
                let message = format!(
                    "gave up waiting for a foreign build of search_index index(es) {:?} after {} minutes; nothing was built or dropped",
                    inspection.in_progress,
                    IN_PROGRESS_MAX_WAIT.as_secs() / 60
                );
                tracing::error!("{message}");
                return Ok(BuildOutcome::Failed { message });
            }
            tracing::info!(indexes = ?inspection.in_progress, "waiting for in-progress search_index builds");
            tokio::time::sleep(IN_PROGRESS_POLL).await;
            inspection = self.inspect().await?;
        }

        let mut created = Vec::new();
        if !inspection.missing.is_empty() {
            let refs: Vec<&SearchIndexSpec> = inspection.missing.iter().collect();
            let names: Vec<&str> = refs.iter().map(|s| s.name).collect();
            tracing::info!(indexes = ?names, "building generation-{SEARCH_INDEX_GENERATION} search_index indexes in one collection scan");
            let started = std::time::Instant::now();
            self.database
                .run_command(create_indexes_command(&refs))
                .await?;
            tracing::info!(indexes = ?names, elapsed_s = started.elapsed().as_secs(), "generation-{SEARCH_INDEX_GENERATION} search_index build complete");
            created = names.into_iter().map(String::from).collect();
            inspection = self.inspect().await?;
            if !inspection.missing.is_empty() || !inspection.in_progress.is_empty() {
                let message = format!(
                    "createIndexes returned but generation-{SEARCH_INDEX_GENERATION} indexes are still missing or in progress: {:?} / {:?}",
                    inspection
                        .missing
                        .iter()
                        .map(|s| s.name)
                        .collect::<Vec<_>>(),
                    inspection.in_progress
                );
                tracing::error!("{message}");
                return Ok(BuildOutcome::Failed { message });
            }
        }

        let mut dropped = Vec::new();
        let collection = self
            .database
            .collection::<Document>(SEARCH_INDEX_COLLECTION);
        for name in &inspection.superseded_present {
            drop_index_if_present(&collection, name).await?;
            dropped.push(name.clone());
        }

        self.record_if_needed().await?;

        if created.is_empty() && dropped.is_empty() {
            tracing::info!(
                "search_index indexes are at generation {SEARCH_INDEX_GENERATION}; nothing to do"
            );
            Ok(BuildOutcome::UpToDate)
        } else {
            Ok(BuildOutcome::Built { created, dropped })
        }
    }

    async fn record_if_needed(&self) -> StorageResult<()> {
        let recorded = get_search_index_generation(&self.database).await?;
        if recorded != Some(SEARCH_INDEX_GENERATION) {
            // The one moment the upgrade is visible: once generation 4 is
            // recorded this branch is never taken again, so the check costs
            // nothing on later boots.
            if unindexed_date_ranges_warning_due(recorded, self.search_index_is_empty().await) {
                tracing::warn!(
                    from_generation = ?recorded,
                    to_generation = SEARCH_INDEX_GENERATION,
                    "search_index date rows written before #1391 have no value_date_end and will \
                     not match date searches with the eq, ne, gt, ge, le, eb or ap prefixes until \
                     they are reindexed, and a Period indexed before #1391 is still two \
                     independent point rows, so even lt and sa compare each of its ends on its \
                     own until then; run `$reindex` to rebuild them (rows written by this \
                     version are not affected)"
                );
            }
            set_search_index_generation(&self.database, SEARCH_INDEX_GENERATION).await?;
        }
        Ok(())
    }

    /// Whether `search_index` holds no rows, from collection metadata: no
    /// scan, however large the collection. A collection that does not exist
    /// counts as empty; a failed count as not empty (it only decides whether
    /// a warning is logged).
    async fn search_index_is_empty(&self) -> bool {
        self.database
            .collection::<Document>(SEARCH_INDEX_COLLECTION)
            .estimated_document_count()
            .await
            .map(|count| count == 0)
            .unwrap_or(false)
    }

    /// Raw `listIndexes`, because the driver's `IndexModel` does not expose
    /// `buildUUID`, which is how an in-progress build is recognised.
    ///
    /// `listIndexes` may page for collections with very many indexes; ours
    /// has at most 21, well under the default batch, so `firstBatch` is
    /// complete.
    async fn inspect(&self) -> StorageResult<Inspection> {
        let reply = match self.database.run_command(list_indexes_command()).await {
            Ok(reply) => reply,
            // NamespaceNotFound (26): the collection has never been written.
            // Every background spec is then "missing" and the build is instant.
            Err(e) if is_namespace_not_found(&e) => {
                return Ok(Inspection {
                    missing: current_specs()
                        .into_iter()
                        .filter(|s| s.build == IndexBuild::Background)
                        .collect(),
                    ..Default::default()
                });
            }
            Err(e) => return Err(e.into()),
        };
        let existing = listed_indexes(&reply)?;

        let mut inspection = Inspection::default();
        for spec in current_specs()
            .into_iter()
            .filter(|s| s.build == IndexBuild::Background)
        {
            match existing.iter().find(|d| d.name == spec.name) {
                None => inspection.missing.push(spec),
                Some(actual) => match classify_spec(&spec, actual) {
                    SpecStatus::Conflicting => inspection
                        .conflicting
                        .push((spec.name.to_string(), actual.clone())),
                    SpecStatus::InProgress => inspection.in_progress.push(spec.name.to_string()),
                    SpecStatus::Ready => {}
                },
            }
        }
        inspection.superseded_present = superseded_present(&existing);
        Ok(inspection)
    }

    /// One-time move of contained rows out of `search_index` (#1160). Runs
    /// in every mode: it is a correctness fix for the standard search, not
    /// an index build, and it is bounded by the number of contained rows,
    /// which is small on every deployment. Idempotent page by page, so a
    /// crash mid-way resumes on the next boot.
    async fn move_contained_rows(&self) -> StorageResult<u64> {
        if contained_rows_moved(&self.database).await? {
            return Ok(0);
        }
        let source = self
            .database
            .collection::<Document>(SEARCH_INDEX_COLLECTION);
        let target = self
            .database
            .collection::<Document>(SEARCH_INDEX_CONTAINED_COLLECTION);
        let mut moved = 0u64;
        loop {
            let mut cursor = source
                .find(doc! { "is_contained": true })
                .limit(MOVE_PAGE as i64)
                .await?;
            let mut page = Vec::with_capacity(MOVE_PAGE);
            while let Some(row) = cursor.try_next().await? {
                page.push(row);
            }
            if page.is_empty() {
                break;
            }
            let ids: Vec<Bson> = page
                .iter()
                .map(|r| {
                    r.get("_id")
                        .cloned()
                        .expect("MongoDB documents always carry _id")
                })
                .collect();
            let mut rows = page;
            for row in &mut rows {
                row.remove("is_contained");
            }
            // Keep `_id`, so a replayed page is a duplicate-key no-op.
            if let Err(e) = target.insert_many(&rows).ordered(false).await
                && !is_only_duplicate_keys(&e)
            {
                return Err(e.into());
            }
            source.delete_many(doc! { "_id": { "$in": ids } }).await?;
            moved += rows.len() as u64;
            tracing::info!(
                moved,
                "moving contained search_index rows to search_index_contained"
            );
        }
        set_contained_rows_moved(&self.database).await?;
        Ok(moved)
    }
}

/// The superseded names present in `existing` (a `search_index`
/// `listIndexes` reading): the generation-1 value indexes, the generation-2
/// `idx_search_date_v2` (#1391), and the generation-2 contained-row index.
/// Pure so it is unit-testable without a live server.
fn superseded_present(existing: &[ListedIndex]) -> Vec<String> {
    let mut present: Vec<String> = superseded_v1_specs()
        .into_iter()
        .chain(std::iter::once(superseded_date_v2_spec()))
        .filter(|old| existing.iter().any(|d| d.name == old.name))
        .map(|old| old.name.to_string())
        .collect();
    if contained_spec_superseded(existing) {
        present.push(superseded_contained_spec().name.to_string());
    }
    present
}

/// True when `existing` (a `search_index` `listIndexes` reading) carries the
/// generation-2 partial index under the superseded contained name (#1160):
/// same name as [`superseded_contained_spec`] and a `partialFilterExpression`
/// present. The current, non-superseded index of that name lives on
/// `search_index_contained` and is never in a `search_index` `listIndexes`
/// reply, so this can never mistake the current index for the superseded one.
/// Pure so it is unit-testable without a live server.
fn contained_spec_superseded(existing: &[ListedIndex]) -> bool {
    let contained = superseded_contained_spec();
    existing
        .iter()
        .any(|d| d.name == contained.name && d.partial.is_some())
}

/// True when `e` is a driver `InsertMany` error whose every per-document
/// write error is a duplicate key (11000) — i.e. every row in the batch was
/// already inserted by an earlier, interrupted attempt at the same page — and
/// no write concern error accompanies it. Any other shape (a page-level
/// error, a write error with a different code, or a write concern error) is
/// a real failure the caller must propagate.
fn is_only_duplicate_keys(e: &mongodb::error::Error) -> bool {
    matches!(
        e.kind.as_ref(),
        mongodb::error::ErrorKind::InsertMany(insert_many)
            if insert_many
                .write_errors
                .as_ref()
                .is_some_and(|errors| errors.iter().all(|e| e.code == 11000))
                // With `ordered(false)`, a batch can report duplicate-key
                // write errors for some documents alongside a write concern
                // error for the ones that did insert; treating that as "all
                // duplicates" would let the caller delete source rows whose
                // copies were never majority-acknowledged.
                && insert_many.write_concern_error.is_none()
    )
}

/// [`IndexBuildMode::Off`]'s decision for one [`Inspection`]: `None` when
/// every background spec is present and no superseded index remains (the
/// caller then records the generation and reports `UpToDate`); otherwise
/// warns about every missing spec and every superseded name — including
/// `idx_search_contained` once [`SearchIndexBuilder::inspect`] marks it
/// superseded (#1160) — and reports `Skipped`. Never builds or drops
/// anything. Pure apart from the `tracing` calls, so it is unit-testable
/// without a live server.
fn off_mode_decision(inspection: &Inspection) -> Option<BuildOutcome> {
    if inspection.missing.is_empty() && inspection.superseded_present.is_empty() {
        return None;
    }
    for spec in &inspection.missing {
        tracing::warn!(
            index = spec.name,
            "HFS_MONGODB_INDEX_BUILD=off: generation-{SEARCH_INDEX_GENERATION} search_index index is missing; \
             build it with docs/mongodb/search-index-v2.mongosh.js"
        );
    }
    for name in &inspection.superseded_present {
        tracing::warn!(
            index = %name,
            "HFS_MONGODB_INDEX_BUILD=off: {name} is a superseded search_index index; \
             drop it with db.search_index.dropIndex(\"{name}\")"
        );
    }
    Some(BuildOutcome::Skipped {
        missing: inspection
            .missing
            .iter()
            .map(|s| s.name.to_string())
            .collect(),
    })
}

/// The `listIndexes` command this module sends. `includeBuildUUIDs: true` is
/// required for the server to report `buildUUID` on an index whose build is
/// still running — without it, an in-progress build looks identical to a
/// finished one and the wait loop in `run_inner` never fires.
fn list_indexes_command() -> Document {
    doc! { "listIndexes": SEARCH_INDEX_COLLECTION, "includeBuildUUIDs": true }
}

/// Normalises every `cursor.firstBatch` entry of a `listIndexes` reply
/// (issued with `includeBuildUUIDs: true`, see [`list_indexes_command`]) into
/// a [`ListedIndex`].
///
/// A finished index reports `name`/`key`/`partialFilterExpression` at the
/// entry's top level. An index whose build is still running reports none of
/// those at the top level at all — instead they are nested under a `spec`
/// sub-document, alongside a top-level `buildUUID`:
/// `{ "spec": { "v": 2, "key": {...}, "name": "...", "partialFilterExpression": {...} }, "buildUUID": <uuid> }`.
/// Reading `name`/`key` from the entry's top level unconditionally — as an
/// earlier version of this function did — silently misses every in-progress
/// index (it never matches its catalog spec by name, so it is reported
/// `missing` instead of `in_progress`), which left the wait loop in
/// `run_inner` dead code.
fn listed_indexes(reply: &Document) -> StorageResult<Vec<ListedIndex>> {
    let no_first_batch = || {
        StorageError::Backend(BackendError::Internal {
            backend_name: "mongodb".to_string(),
            message: format!(
                "listIndexes reply for {SEARCH_INDEX_COLLECTION} had no cursor.firstBatch: {reply:?}"
            ),
            source: None,
        })
    };
    let batch = reply
        .get_document("cursor")
        .ok()
        .and_then(|c| c.get_array("firstBatch").ok())
        .ok_or_else(no_first_batch)?;

    let mut out = Vec::with_capacity(batch.len());
    for entry in batch.iter().filter_map(|b| b.as_document()) {
        // A top-level `buildUUID` means "in progress" whichever shape the
        // rest of the entry takes: the nested-`spec` shape is the common
        // case, but nothing in the `listIndexes` contract says a server
        // could not report a top-level `buildUUID` alongside top-level
        // `name`/`key` too. Treating that as "not in progress" (as an
        // earlier version of this function did when no `spec` was present)
        // would classify it ready and let generation-1 be dropped mid-build
        // (the MUST-FIX deferred minor from the review).
        let in_progress = entry.contains_key("buildUUID");
        let source = match entry.get_document("spec") {
            Ok(spec) => spec,
            Err(_) => entry,
        };
        let no_field = |field: &str| {
            StorageError::Backend(BackendError::Internal {
                backend_name: "mongodb".to_string(),
                message: format!(
                    "listIndexes entry for {SEARCH_INDEX_COLLECTION} had no {field}: {entry:?}"
                ),
                source: None,
            })
        };
        let name = source
            .get_str("name")
            .map_err(|_| no_field("name"))?
            .to_string();
        let key = source
            .get_document("key")
            .map_err(|_| no_field("key"))?
            .clone();
        let partial = source.get_document("partialFilterExpression").ok().cloned();
        let extra_options = EXTRA_OPTION_KEYS
            .iter()
            .filter(|key| source.contains_key(**key))
            .map(|key| key.to_string())
            .collect();
        out.push(ListedIndex {
            name,
            key,
            partial,
            in_progress,
            extra_options,
        });
    }
    Ok(out)
}

/// Recursively converts `Int32`/`Int64` values to `Double`, so a catalog spec
/// built from `1_i32` literals compares equal to the same index as MongoDB
/// (or mongosh) reports it back, which uses doubles for bare numeric
/// literals. Used only for the conflict comparison in `inspect`; the actual
/// `createIndexes` command still sends the catalog's native integer types.
fn normalize_numbers(doc: &Document) -> Document {
    let mut out = Document::new();
    for (k, v) in doc.iter() {
        out.insert(k, normalize_bson(v));
    }
    out
}

fn normalize_bson(value: &Bson) -> Bson {
    match value {
        Bson::Int32(i) => Bson::Double(f64::from(*i)),
        Bson::Int64(i) => Bson::Double(*i as f64),
        Bson::Document(d) => Bson::Document(normalize_numbers(d)),
        Bson::Array(arr) => Bson::Array(arr.iter().map(normalize_bson).collect()),
        other => other.clone(),
    }
}

fn is_namespace_not_found(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(c) if c.code == 26)
}

/// How one `listIndexes` entry compares to the generation-2 spec it is named
/// for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecStatus {
    /// Different key, different partial filter, or an [`EXTRA_OPTION_KEYS`]
    /// option present (I2) — a person built something under our name.
    Conflicting,
    /// Same key and partial filter, no extra options, but still building.
    InProgress,
    /// Same key and partial filter, no extra options, build finished.
    Ready,
}

/// Classifies one existing index against the catalog spec it is named for.
/// Pure and side-effect-free so the conflict rule (I2, and the key/partial
/// comparison it already had) is unit-testable without a live server.
fn classify_spec(spec: &SearchIndexSpec, actual: &ListedIndex) -> SpecStatus {
    // Numeric literals from mongosh land as doubles even when the catalog
    // spec's keys/partial filter are `1_i32`; normalise before comparing so
    // a pre-built database (e.g. from the shipped mongosh script) is never
    // reported as a conflict.
    let same_keys = normalize_numbers(&actual.key) == normalize_numbers(&spec.keys);
    let same_partial = actual.partial.as_ref().map(normalize_numbers)
        == spec.partial.as_ref().map(normalize_numbers);
    if !(same_keys && same_partial && actual.extra_options.is_empty()) {
        SpecStatus::Conflicting
    } else if actual.in_progress {
        SpecStatus::InProgress
    } else {
        SpecStatus::Ready
    }
}

#[cfg(test)]
mod builder_tests {
    use super::*;

    #[test]
    fn list_indexes_command_includes_build_uuids() {
        let cmd = list_indexes_command();
        assert_eq!(cmd.get_str("listIndexes"), Ok(SEARCH_INDEX_COLLECTION));
        assert_eq!(cmd.get_bool("includeBuildUUIDs"), Ok(true));
    }

    #[test]
    fn normalize_numbers_treats_ints_and_doubles_as_equal() {
        let ints = doc! { "a": 1_i32, "b": { "c": 2_i64 } };
        let doubles = doc! { "a": 1.0, "b": { "c": 2.0 } };
        assert_eq!(normalize_numbers(&ints), normalize_numbers(&doubles));
    }

    #[test]
    fn listed_indexes_normalises_ready_in_progress_and_legacy_shapes() {
        let reply = doc! {
            "cursor": {
                "firstBatch": [
                    // A finished index: fields at the entry's top level.
                    {
                        "v": 2,
                        "key": { "tenant_id": 1 },
                        "name": "idx_search_string_v2",
                        "partialFilterExpression": { "value_string": { "$exists": true } },
                    },
                    // An in-progress build (only reported this way because
                    // the command carries `includeBuildUUIDs: true`): fields
                    // nested under `spec`, no top-level name/key at all.
                    {
                        "spec": {
                            "v": 2,
                            "key": { "tenant_id": 1 },
                            "name": "idx_search_date_v2",
                        },
                        "buildUUID": "test-build-uuid",
                    },
                    // A legacy/generation-1-shaped entry carrying a top-level
                    // `buildUUID` with no `spec` wrapper. Nothing in the
                    // `listIndexes` contract rules this out, and treating it
                    // as "not in progress" (an earlier version of this
                    // function did, by hard-coding `false` in this branch)
                    // would classify it ready and let generation-1 be
                    // dropped while it is still building (the MUST-FIX
                    // deferred minor from the review).
                    {
                        "v": 2,
                        "key": { "tenant_id": 1 },
                        "name": "idx_search_string",
                        "buildUUID": "legacy-shape-build-uuid",
                    },
                ],
            },
        };

        let listed = listed_indexes(&reply).expect("listed_indexes");
        assert_eq!(
            listed,
            vec![
                ListedIndex {
                    name: "idx_search_string_v2".to_string(),
                    key: doc! { "tenant_id": 1 },
                    partial: Some(doc! { "value_string": { "$exists": true } }),
                    in_progress: false,
                    extra_options: Vec::new(),
                },
                ListedIndex {
                    name: "idx_search_date_v2".to_string(),
                    key: doc! { "tenant_id": 1 },
                    partial: None,
                    in_progress: true,
                    extra_options: Vec::new(),
                },
                ListedIndex {
                    name: "idx_search_string".to_string(),
                    key: doc! { "tenant_id": 1 },
                    partial: None,
                    in_progress: true,
                    extra_options: Vec::new(),
                },
            ]
        );
    }

    /// #1391: the warning is for an upgrade with rows to reindex, once.
    #[test]
    fn unindexed_date_ranges_warning_is_due_only_on_an_upgrade_with_rows() {
        let current = SEARCH_INDEX_GENERATION;
        // Never recorded, rows present: a database from before generations.
        assert!(unindexed_date_ranges_warning_due(None, false));
        // Every earlier generation, rows present.
        for generation in 1..current {
            assert!(
                unindexed_date_ranges_warning_due(Some(generation), false),
                "generation {generation}"
            );
        }
        // A new database, or one nobody wrote to.
        assert!(!unindexed_date_ranges_warning_due(None, true));
        assert!(!unindexed_date_ranges_warning_due(Some(current - 1), true));
        // Already there (or newer, from a later binary): silent on every boot.
        assert!(!unindexed_date_ranges_warning_due(Some(current), false));
        assert!(!unindexed_date_ranges_warning_due(Some(current), true));
        assert!(!unindexed_date_ranges_warning_due(Some(current + 1), false));
    }

    #[test]
    fn listed_indexes_errors_loudly_without_cursor_first_batch() {
        let reply = doc! { "ok": 1.0 };
        assert!(listed_indexes(&reply).is_err());
    }

    #[test]
    fn listed_indexes_collects_extra_options_beyond_key_and_partial() {
        let reply = doc! {
            "cursor": {
                "firstBatch": [
                    {
                        "v": 2,
                        "key": { "tenant_id": 1, "value_date": 1 },
                        "name": "idx_search_date_v2",
                        "partialFilterExpression": { "value_date": { "$exists": true } },
                        "collation": { "locale": "en" },
                    },
                ],
            },
        };

        let listed = listed_indexes(&reply).expect("listed_indexes");
        assert_eq!(listed[0].extra_options, vec!["collation".to_string()]);
    }

    /// I2: an index that matches the catalog's key and partial filter
    /// exactly but carries an extra option (here `collation`) must still be
    /// classified `Conflicting`, not `Ready` — a collation difference
    /// changes string comparison results, so serving off it silently would
    /// be a correctness bug, not just a performance one.
    #[test]
    fn classify_spec_treats_matching_index_with_extra_option_as_conflicting() {
        let spec = current_specs()
            .into_iter()
            .find(|s| s.name == "idx_search_date_v3")
            .expect("idx_search_date_v3 is in the catalog");

        let actual = ListedIndex {
            name: spec.name.to_string(),
            key: spec.keys.clone(),
            partial: spec.partial.clone(),
            in_progress: false,
            extra_options: vec!["collation".to_string()],
        };

        assert_eq!(classify_spec(&spec, &actual), SpecStatus::Conflicting);
    }

    #[test]
    fn classify_spec_ready_when_key_partial_match_and_no_extra_options() {
        let spec = current_specs()
            .into_iter()
            .find(|s| s.name == "idx_search_date_v3")
            .expect("idx_search_date_v3 is in the catalog");

        let actual = ListedIndex {
            name: spec.name.to_string(),
            key: spec.keys.clone(),
            partial: spec.partial.clone(),
            in_progress: false,
            extra_options: Vec::new(),
        };

        assert_eq!(classify_spec(&spec, &actual), SpecStatus::Ready);
    }

    /// #1160: a listed `idx_search_contained` with a `partialFilterExpression`
    /// is the generation-2 shape still sitting on `search_index` — it must be
    /// classified superseded so the builder drops it once the rows it used to
    /// serve have moved to `search_index_contained`.
    #[test]
    fn superseded_present_includes_the_generation2_contained_index() {
        let contained = superseded_contained_spec();
        let existing = vec![ListedIndex {
            name: contained.name.to_string(),
            key: contained.keys.clone(),
            partial: contained.partial.clone(),
            in_progress: false,
            extra_options: Vec::new(),
        }];

        assert!(contained_spec_superseded(&existing));
    }

    /// The *current* `idx_search_contained` (generation 3, no partial filter,
    /// on `search_index_contained`) never appears in a `search_index`
    /// `listIndexes` reply in the first place, but even a same-named plain
    /// index with no partial filter must not be mistaken for the superseded
    /// generation-2 shape.
    #[test]
    fn contained_spec_not_superseded_without_a_partial_filter() {
        let contained = superseded_contained_spec();
        let existing = vec![ListedIndex {
            name: contained.name.to_string(),
            key: contained.keys.clone(),
            partial: None,
            in_progress: false,
            extra_options: Vec::new(),
        }];

        assert!(!contained_spec_superseded(&existing));
    }

    /// #1391: once `idx_search_date_v3` is current, a listed
    /// `idx_search_date_v2` is superseded (dropped once the generation-4 set
    /// is ready), while the current value indexes never are.
    #[test]
    fn superseded_present_includes_the_generation2_date_index() {
        let listed = |spec: SearchIndexSpec| ListedIndex {
            name: spec.name.to_string(),
            key: spec.keys,
            partial: spec.partial,
            in_progress: false,
            extra_options: Vec::new(),
        };
        let mut existing: Vec<ListedIndex> = current_specs().into_iter().map(listed).collect();
        existing.push(listed(superseded_date_v2_spec()));

        assert_eq!(
            superseded_present(&existing),
            vec!["idx_search_date_v2".to_string()]
        );
    }

    #[test]
    fn off_mode_warns_about_the_contained_index_and_drops_nothing() {
        let inspection = Inspection {
            missing: Vec::new(),
            in_progress: Vec::new(),
            conflicting: Vec::new(),
            superseded_present: vec!["idx_search_contained".to_string()],
        };

        assert_eq!(
            off_mode_decision(&inspection),
            Some(BuildOutcome::Skipped { missing: vec![] })
        );
    }

    #[test]
    fn off_mode_decision_is_none_when_nothing_missing_or_superseded() {
        let inspection = Inspection::default();
        assert_eq!(off_mode_decision(&inspection), None);
    }
}
