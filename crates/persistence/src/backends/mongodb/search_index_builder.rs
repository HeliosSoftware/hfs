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
    bson::{Document, doc},
};

use crate::error::StorageResult;

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
    conflicting: Vec<(String, Document)>,
    /// Superseded generation-1 names still present.
    superseded_present: Vec<String>,
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
                let expected = generation2_specs()
                    .into_iter()
                    .find(|s| s.name == name)
                    .map(|s| s.keys);
                tracing::error!(
                    index = %name,
                    expected_keys = ?expected,
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
        let reply = match self
            .database
            .run_command(doc! { "listIndexes": SEARCH_INDEX_COLLECTION })
            .await
        {
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
        let existing: Vec<Document> = reply
            .get_document("cursor")
            .ok()
            .and_then(|c| c.get_array("firstBatch").ok())
            .map(|batch| {
                batch
                    .iter()
                    .filter_map(|b| b.as_document().cloned())
                    .collect()
            })
            .unwrap_or_default();

        let mut inspection = Inspection::default();
        for spec in generation2_specs()
            .into_iter()
            .filter(|s| s.build == IndexBuild::Background)
        {
            match existing.iter().find(|d| d.get_str("name") == Ok(spec.name)) {
                None => inspection.missing.push(spec),
                Some(actual) => {
                    let same_keys = actual.get_document("key").ok() == Some(&spec.keys);
                    let same_partial = actual.get_document("partialFilterExpression").ok().cloned()
                        == spec.partial;
                    if !(same_keys && same_partial) {
                        inspection
                            .conflicting
                            .push((spec.name.to_string(), actual.clone()));
                    } else if actual.contains_key("buildUUID") {
                        inspection.in_progress.push(spec.name.to_string());
                    }
                }
            }
        }
        for v1 in superseded_v1_specs() {
            if existing.iter().any(|d| d.get_str("name") == Ok(v1.name)) {
                inspection.superseded_present.push(v1.name.to_string());
            }
        }
        Ok(inspection)
    }
}

fn is_namespace_not_found(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(c) if c.code == 26)
}
