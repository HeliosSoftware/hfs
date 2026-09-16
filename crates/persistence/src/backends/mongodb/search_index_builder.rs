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

/// One `listIndexes` reading, classified against the catalog.
#[derive(Debug, Default)]
struct Inspection {
    /// Background specs absent from the collection.
    missing: Vec<SearchIndexSpec>,
    /// Our names that exist but carry a `buildUUID`: someone else is building them.
    in_progress: Vec<String>,
    /// Our names that exist with a different key or partial filter.
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
        // build of the same index.
        while !inspection.in_progress.is_empty() {
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
                Some(actual) => {
                    // Numeric literals from mongosh land as doubles even when
                    // the catalog spec's keys/partial filter are `1_i32`;
                    // normalise before comparing so a pre-built database
                    // (e.g. from the shipped mongosh script) is never
                    // reported as a conflict.
                    let same_keys = normalize_numbers(&actual.key) == normalize_numbers(&spec.keys);
                    let same_partial = actual.partial.as_ref().map(normalize_numbers)
                        == spec.partial.as_ref().map(normalize_numbers);
                    if !(same_keys && same_partial) {
                        inspection
                            .conflicting
                            .push((spec.name.to_string(), actual.clone()));
                    } else if actual.in_progress {
                        inspection.in_progress.push(spec.name.to_string());
                    }
                }
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
        let (source, in_progress) = match entry.get_document("spec") {
            Ok(spec) => (spec, entry.contains_key("buildUUID")),
            Err(_) => (entry, false),
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
        out.push(ListedIndex {
            name,
            key,
            partial,
            in_progress,
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
                    // A superseded generation-1 index: also top-level fields.
                    {
                        "v": 2,
                        "key": { "tenant_id": 1 },
                        "name": "idx_search_string",
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
                },
                ListedIndex {
                    name: "idx_search_date_v2".to_string(),
                    key: doc! { "tenant_id": 1 },
                    partial: None,
                    in_progress: true,
                },
                ListedIndex {
                    name: "idx_search_string".to_string(),
                    key: doc! { "tenant_id": 1 },
                    partial: None,
                    in_progress: false,
                },
            ]
        );
    }

    #[test]
    fn listed_indexes_errors_loudly_without_cursor_first_batch() {
        let reply = doc! { "ok": 1.0 };
        assert!(listed_indexes(&reply).is_err());
    }
}
