//! Post-boot builder for `idx_resources_missing_params` (#1908, #2001) and
//! its staleness-probe companion `idx_resources_missing_params_fp` (#1999).
//!
//! The index is a multikey index over every `resources` document, so on an
//! existing deployment its first build is a full collection scan (about
//! 10.5 GB at 12M resources). It follows `HFS_MONGODB_INDEX_BUILD` like the
//! `search_index` value indexes instead of holding startup. Until this
//! builder reports the index ready, `:missing=true` searches take the
//! value-index route and never hint it (`MongoBackend::missing_params_index_ready`).
//! The fingerprint index is partial (only documents with `missing_params`),
//! but building it is a collection scan as well, so it is built here too and
//! readiness waits for both.

use mongodb::{Database, bson::doc};

use crate::error::StorageResult;

use super::MongoBackend;
use super::schema::{RESOURCES_MISSING_FP_INDEX, RESOURCES_MISSING_INDEX};
use super::search_index_builder::{
    BuildOutcome, IN_PROGRESS_MAX_WAIT, IN_PROGRESS_POLL, IndexBuildMode, ListedIndex, SpecStatus,
    classify_spec, is_namespace_not_found, list_indexes_command, listed_indexes,
};
use super::search_index_catalog::{IndexBuild, SearchIndexSpec, create_indexes_command_for};

/// The catalog entry for `idx_resources_missing_params`. Not partial: an
/// absent `missing_params` field is indexed too, so the index also serves
/// timestamp pages over resources without metadata.
pub(crate) fn missing_index_spec() -> SearchIndexSpec {
    SearchIndexSpec {
        name: RESOURCES_MISSING_INDEX,
        keys: doc! {
            "tenant_id": 1_i32,
            "resource_type": 1_i32,
            "is_deleted": 1_i32,
            "missing_params": 1_i32,
            "last_updated": 1_i32,
            "id": 1_i32,
        },
        partial: None,
        build: IndexBuild::Background,
    }
}

/// The catalog entry for `idx_resources_missing_params_fp`: the stamp of
/// each `missing_params` array. Partial on the array existing, so an array
/// without a stamp is a null key the probe finds in one seek, and resources
/// without metadata (failed extraction, pending deferred rows, written before
/// #1984) are not in it.
pub(crate) fn missing_fp_index_spec() -> SearchIndexSpec {
    SearchIndexSpec {
        name: RESOURCES_MISSING_FP_INDEX,
        keys: doc! {
            "tenant_id": 1_i32,
            "resource_type": 1_i32,
            "is_deleted": 1_i32,
            "missing_params_fp": 1_i32,
        },
        partial: Some(doc! { "missing_params": { "$exists": true } }),
        build: IndexBuild::Background,
    }
}

fn specs() -> [SearchIndexSpec; 2] {
    [missing_index_spec(), missing_fp_index_spec()]
}

fn names(specs: &[&SearchIndexSpec]) -> Vec<String> {
    specs.iter().map(|spec| spec.name.to_string()).collect()
}

pub(super) struct MissingIndexBuilder {
    database: Database,
    mode: IndexBuildMode,
}

impl MissingIndexBuilder {
    pub(super) fn new(database: Database, mode: IndexBuildMode) -> Self {
        Self { database, mode }
    }

    /// Inspect, then build when the mode allows it. Never returns an `Err`:
    /// every failure is logged and reported as `BuildOutcome::Failed`, and
    /// `:missing=true` keeps the value-index route.
    pub(super) async fn run(self) -> BuildOutcome {
        match self.run_inner().await {
            Ok(outcome) => outcome,
            Err(e) => {
                let message = format!("{RESOURCES_MISSING_INDEX} build failed: {e}");
                tracing::error!(error = %e, "{message}");
                BuildOutcome::Failed { message }
            }
        }
    }

    async fn run_inner(&self) -> StorageResult<BuildOutcome> {
        let specs = specs();
        let mut statuses = self.inspect(&specs).await?;

        if let Some((spec, _)) = specs
            .iter()
            .zip(&statuses)
            .find(|(_, status)| **status == Some(SpecStatus::Conflicting))
        {
            let message = format!(
                "resources.{} exists with a different spec; \
                 :missing=true searches use the value index until it is dropped and rebuilt",
                spec.name
            );
            tracing::error!(expected_keys = ?spec.keys, "{message}");
            return Ok(BuildOutcome::Failed { message });
        }

        if self.mode == IndexBuildMode::Off {
            return Ok(off_mode_decision(&statuses));
        }

        // Another process (an HFS node or an operator's mongosh) is building
        // one: wait rather than issue a second build of the same index.
        let wait_started = std::time::Instant::now();
        while statuses.contains(&Some(SpecStatus::InProgress)) {
            if wait_started.elapsed() >= IN_PROGRESS_MAX_WAIT {
                let message = format!(
                    "gave up waiting for a foreign build of the resources missing-metadata \
                     indexes after {} minutes",
                    IN_PROGRESS_MAX_WAIT.as_secs() / 60
                );
                tracing::error!("{message}");
                return Ok(BuildOutcome::Failed { message });
            }
            tracing::info!(
                index = RESOURCES_MISSING_INDEX,
                "waiting for in-progress resources index build"
            );
            tokio::time::sleep(IN_PROGRESS_POLL).await;
            statuses = self.inspect(&specs).await?;
        }

        let absent: Vec<&SearchIndexSpec> = specs
            .iter()
            .zip(&statuses)
            .filter(|(_, status)| **status != Some(SpecStatus::Ready))
            .map(|(spec, _)| spec)
            .collect();
        if absent.is_empty() {
            return Ok(BuildOutcome::UpToDate);
        }

        let created = names(&absent);
        tracing::info!(
            indexes = ?created,
            "building resources indexes; :missing=true searches use the value index until they are ready"
        );
        let started = std::time::Instant::now();
        self.database
            .run_command(create_indexes_command_for(
                MongoBackend::RESOURCES_COLLECTION,
                &absent,
            ))
            .await?;
        if let Some((spec, _)) = specs
            .iter()
            .zip(self.inspect(&specs).await?)
            .find(|(_, status)| *status != Some(SpecStatus::Ready))
        {
            let message = format!(
                "createIndexes returned but resources.{} is not ready",
                spec.name
            );
            tracing::error!("{message}");
            return Ok(BuildOutcome::Failed { message });
        }
        tracing::info!(
            indexes = ?created,
            elapsed_s = started.elapsed().as_secs(),
            "resources index build complete"
        );
        Ok(BuildOutcome::Built {
            created,
            dropped: Vec::new(),
        })
    }

    /// Each spec's status; `None` when that index is absent (or `resources`
    /// does not exist yet).
    async fn inspect(&self, specs: &[SearchIndexSpec]) -> StorageResult<Vec<Option<SpecStatus>>> {
        let listed = match self
            .database
            .run_command(list_indexes_command(MongoBackend::RESOURCES_COLLECTION))
            .await
        {
            Ok(reply) => listed_indexes(&reply, MongoBackend::RESOURCES_COLLECTION)?,
            Err(e) if is_namespace_not_found(&e) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(specs.iter().map(|spec| classify(spec, &listed)).collect())
    }
}

fn classify(spec: &SearchIndexSpec, listed: &[ListedIndex]) -> Option<SpecStatus> {
    listed
        .iter()
        .find(|index| index.name == spec.name)
        .map(|index| classify_spec(spec, index))
}

/// [`IndexBuildMode::Off`]'s decision: report the indexes ready only when
/// both are, otherwise warn with the commands that build them. Never builds
/// anything.
fn off_mode_decision(statuses: &[Option<SpecStatus>]) -> BuildOutcome {
    let specs = specs();
    let missing: Vec<&SearchIndexSpec> = specs
        .iter()
        .zip(statuses)
        .filter(|(_, status)| **status != Some(SpecStatus::Ready))
        .map(|(spec, _)| spec)
        .collect();
    if missing.is_empty() {
        return BuildOutcome::UpToDate;
    }
    if statuses.contains(&Some(SpecStatus::InProgress)) {
        tracing::warn!(
            indexes = ?names(&missing),
            "HFS_MONGODB_INDEX_BUILD=off: resources index is still being built; \
             :missing=true searches use the value index until the next start after it finishes"
        );
    } else {
        tracing::warn!(
            indexes = ?names(&missing),
            "HFS_MONGODB_INDEX_BUILD=off: resources index is missing; :missing=true searches \
             use the value index. Build it with db.resources.createIndex({{tenant_id: 1, \
             resource_type: 1, is_deleted: 1, missing_params: 1, last_updated: 1, id: 1}}, \
             {{name: \"{RESOURCES_MISSING_INDEX}\"}}) and db.resources.createIndex({{tenant_id: 1, \
             resource_type: 1, is_deleted: 1, missing_params_fp: 1}}, {{name: \
             \"{RESOURCES_MISSING_FP_INDEX}\", partialFilterExpression: {{missing_params: \
             {{$exists: true}}}}}}) and restart"
        );
    }
    BuildOutcome::Skipped {
        missing: names(&missing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(in_progress: bool, extra_options: Vec<String>) -> ListedIndex {
        let spec = missing_index_spec();
        ListedIndex {
            name: spec.name.to_string(),
            key: spec.keys,
            partial: spec.partial,
            in_progress,
            extra_options,
        }
    }

    #[test]
    fn classify_reads_absent_building_ready_and_conflicting() {
        let spec = missing_index_spec();
        assert_eq!(classify(&spec, &[]), None);
        assert_eq!(
            classify(&spec, &[listed(true, Vec::new())]),
            Some(SpecStatus::InProgress)
        );
        assert_eq!(
            classify(&spec, &[listed(false, Vec::new())]),
            Some(SpecStatus::Ready)
        );
        assert_eq!(
            classify(&spec, &[listed(false, vec!["hidden".to_string()])]),
            Some(SpecStatus::Conflicting)
        );
    }

    #[test]
    fn off_mode_is_up_to_date_only_when_both_are_ready() {
        let ready = Some(SpecStatus::Ready);
        assert_eq!(off_mode_decision(&[ready, ready]), BuildOutcome::UpToDate);
        for status in [None, Some(SpecStatus::InProgress)] {
            assert_eq!(
                off_mode_decision(&[status, ready]),
                BuildOutcome::Skipped {
                    missing: vec![RESOURCES_MISSING_INDEX.to_string()],
                }
            );
            assert_eq!(
                off_mode_decision(&[ready, status]),
                BuildOutcome::Skipped {
                    missing: vec![RESOURCES_MISSING_FP_INDEX.to_string()],
                }
            );
        }
    }

    #[test]
    fn fingerprint_index_is_partial_on_the_metadata_array() {
        let spec = missing_fp_index_spec();
        let entry = spec.create_indexes_entry();
        assert_eq!(
            entry.get_document("partialFilterExpression").unwrap(),
            &doc! { "missing_params": { "$exists": true } }
        );
    }
}
