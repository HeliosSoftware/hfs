//! Post-boot builder for the generation-2 `search_index` indexes (#1059, #1084).

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

use mongodb::{
    Database,
    bson::{Bson, Document, doc},
};

use crate::error::{BackendError, StorageError, StorageResult};

use super::schema::{
    drop_index_if_present, get_search_index_generation, set_search_index_generation,
};
use super::search_index_catalog::{
    IndexBuild, SEARCH_INDEX_COLLECTION, SEARCH_INDEX_GENERATION, SearchIndexSpec,
    create_indexes_command, generation2_specs, superseded_v1_specs,
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
        /// Names of the superseded generation-1 indexes that were dropped.
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
    /// Superseded generation-1 names still present.
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
        let mut inspection = self.inspect().await?;

        if !inspection.conflicting.is_empty() {
            let names: Vec<String> = inspection
                .conflicting
                .iter()
                .map(|(n, _)| n.clone())
                .collect();
            for (name, actual) in &inspection.conflicting {
                let expected_spec = generation2_specs().into_iter().find(|s| s.name == name);
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
            let missing: Vec<String> = inspection
                .missing
                .iter()
                .map(|s| s.name.to_string())
                .collect();
            if missing.is_empty() && inspection.superseded_present.is_empty() {
                self.record_if_needed().await?;
                return Ok(BuildOutcome::UpToDate);
            }
            for spec in &inspection.missing {
                tracing::warn!(
                    index = spec.name,
                    "HFS_MONGODB_INDEX_BUILD=off: generation-2 search_index index is missing; \
                     build it with docs/mongodb/search-index-v2.mongosh.js"
                );
            }
            for name in &inspection.superseded_present {
                tracing::warn!(
                    index = %name,
                    "HFS_MONGODB_INDEX_BUILD=off: {name} is a superseded generation-1 \
                     search_index index; drop it with db.search_index.dropIndex(\"{name}\")"
                );
            }
            return Ok(BuildOutcome::Skipped { missing });
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
            tracing::info!(indexes = ?names, "building generation-2 search_index indexes in one collection scan");
            let started = std::time::Instant::now();
            self.database
                .run_command(create_indexes_command(&refs))
                .await?;
            tracing::info!(indexes = ?names, elapsed_s = started.elapsed().as_secs(), "generation-2 search_index build complete");
            created = names.into_iter().map(String::from).collect();
            inspection = self.inspect().await?;
            if !inspection.missing.is_empty() || !inspection.in_progress.is_empty() {
                let message = format!(
                    "createIndexes returned but generation-2 indexes are still missing or in progress: {:?} / {:?}",
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
        if get_search_index_generation(&self.database).await? != Some(SEARCH_INDEX_GENERATION) {
            set_search_index_generation(&self.database, SEARCH_INDEX_GENERATION).await?;
        }
        Ok(())
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
                    missing: generation2_specs()
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
        for spec in generation2_specs()
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
        for v1 in superseded_v1_specs() {
            if existing.iter().any(|d| d.name == v1.name) {
                inspection.superseded_present.push(v1.name.to_string());
            }
        }
        Ok(inspection)
    }
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
        let spec = generation2_specs()
            .into_iter()
            .find(|s| s.name == "idx_search_date_v2")
            .expect("idx_search_date_v2 is in the catalog");

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
        let spec = generation2_specs()
            .into_iter()
            .find(|s| s.name == "idx_search_date_v2")
            .expect("idx_search_date_v2 is in the catalog");

        let actual = ListedIndex {
            name: spec.name.to_string(),
            key: spec.keys.clone(),
            partial: spec.partial.clone(),
            in_progress: false,
            extra_options: Vec::new(),
        };

        assert_eq!(classify_spec(&spec, &actual), SpecStatus::Ready);
    }
}
