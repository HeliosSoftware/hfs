//! Settings for the SQL-on-FHIR runner `$sql-export` jobs read through.
//!
//! An export job reads whole results (every row of every subject, and every
//! SQL Query dependency it materializes) and nothing about it is interactive.
//! On the request-serving pool a long export statement would hold a request
//! connection for its whole life, so a backend may give export jobs a runner
//! of their own: [`ResourceStorage::export_sof_runner`] takes these options
//! and defaults to the ordinary [`sof_runner`]. The PostgreSQL backend builds
//! a separate `hfs-export` connection pool from them; every other backend
//! ignores them.
//!
//! What bounds an export's work is its job budget (`HFS_EXPORT_JOB_TIMEOUT_SECS`):
//! the REST layer turns it into the deadline of every run of the job
//! ([`RunContext::deadline`]), and the PostgreSQL runner sends each page's
//! statement under `SET LOCAL statement_timeout` from what is left of it.
//! [`ExportRunnerOptions::statement_timeout_ms`] is only the session-level
//! backstop behind that.
//!
//! [`ResourceStorage::export_sof_runner`]: crate::core::ResourceStorage::export_sof_runner
//! [`sof_runner`]: crate::core::ResourceStorage::sof_runner
//! [`RunContext::deadline`]: crate::core::sof_runner::RunContext::deadline

/// Default number of export connections: `HFS_EXPORT_MAX_CONCURRENCY`'s
/// default, so each concurrently running job has one connection.
pub const DEFAULT_EXPORT_MAX_CONNECTIONS: usize = 4;

/// Default session `statement_timeout` of export connections, in
/// milliseconds: one hour, the default export job budget
/// (`HFS_EXPORT_JOB_TIMEOUT_SECS`).
pub const DEFAULT_EXPORT_STATEMENT_TIMEOUT_MS: u64 = 3_600_000;

/// Settings for the runner `$sql-export` jobs read through.
///
/// Only the PostgreSQL backend acts on these; every other backend returns its
/// ordinary runner and ignores them. There are deliberately no planner
/// settings here: those ship only with an isolated before/after measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRunnerOptions {
    /// Connections in the export pool. An export job runs one statement at a
    /// time, so one connection per concurrently running job is enough; HFS
    /// defaults this to `HFS_EXPORT_MAX_CONCURRENCY` (at least 1). Values
    /// below 1 are treated as 1. A job that finds every export connection
    /// busy waits for one (the export pool has no wait timeout), and its wait
    /// stays cancellable and bounded by the job's deadline.
    pub max_connections: usize,

    /// Session `statement_timeout` of export connections, in milliseconds:
    /// the export job budget, as a backstop behind the per-page `SET LOCAL
    /// statement_timeout` each run sends from its remaining deadline. Values
    /// above `i32::MAX` are capped to it (PostgreSQL's own maximum); `0`
    /// means no session limit.
    pub statement_timeout_ms: u64,
}

impl Default for ExportRunnerOptions {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_EXPORT_MAX_CONNECTIONS,
            statement_timeout_ms: DEFAULT_EXPORT_STATEMENT_TIMEOUT_MS,
        }
    }
}

/// Doubles for the tests of [`export_sof_runner`] and of the wrappers that
/// forward it.
///
/// [`export_sof_runner`]: crate::core::ResourceStorage::export_sof_runner
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use helios_fhir::FhirVersion;
    use serde_json::Value;

    use super::ExportRunnerOptions;
    use crate::core::ResourceStorage;
    use crate::core::sof_runner::{RunContext, RunOutput, RunRequest, SofError, SofRunner};
    use crate::error::StorageResult;
    use crate::tenant::TenantContext;
    use crate::types::StoredResource;

    /// The [`runner_name`](SofRunner::runner_name) of
    /// [`ExportMarkerStorage`]'s request runner.
    pub(crate) const MAIN_RUNNER: &str = "marker-main";
    /// The [`runner_name`](SofRunner::runner_name) of
    /// [`ExportMarkerStorage`]'s export runner.
    pub(crate) const EXPORT_RUNNER: &str = "marker-export";

    /// A runner that only has a name.
    pub(crate) struct NamedRunner(pub(crate) &'static str);

    #[async_trait]
    impl SofRunner for NamedRunner {
        async fn run_view(
            &self,
            _tenant: &TenantContext,
            _request: RunRequest,
            _ctx: RunContext,
        ) -> Result<RunOutput, SofError> {
            Err(SofError::Backend {
                sqlstate: None,
                message: format!("{} runs nothing", self.0),
            })
        }

        fn runner_name(&self) -> &'static str {
            self.0
        }
    }

    /// A store whose export runner differs from its request runner, and
    /// which records the options each `export_sof_runner` call received. A
    /// wrapper that forwards `export_sof_runner` returns [`EXPORT_RUNNER`];
    /// one that falls back to the trait default returns its `sof_runner`
    /// ([`MAIN_RUNNER`]).
    #[derive(Default)]
    pub(crate) struct ExportMarkerStorage {
        pub(crate) export_calls: Mutex<Vec<ExportRunnerOptions>>,
    }

    #[async_trait]
    impl ResourceStorage for ExportMarkerStorage {
        fn backend_name(&self) -> &'static str {
            "export-marker"
        }

        fn sof_runner(&self) -> Option<Arc<dyn SofRunner>> {
            Some(Arc::new(NamedRunner(MAIN_RUNNER)))
        }

        fn export_sof_runner(&self, options: &ExportRunnerOptions) -> Option<Arc<dyn SofRunner>> {
            self.export_calls.lock().unwrap().push(options.clone());
            Some(Arc::new(NamedRunner(EXPORT_RUNNER)))
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!("not used by the export runner tests")
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!("not used by the export runner tests")
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            Ok(None)
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!("not used by the export runner tests")
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            Ok(())
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            Ok(0)
        }
    }

    /// A composite whose primary is `primary` (no secondaries), as the
    /// production wrappers see it.
    pub(crate) fn composite_over(
        primary: Arc<ExportMarkerStorage>,
    ) -> crate::composite::storage::CompositeStorage {
        use crate::composite::config::CompositeConfig;
        use crate::composite::storage::{CompositeStorage, DynStorage};
        use crate::core::BackendKind;

        let config = CompositeConfig::builder()
            .primary("primary", BackendKind::Sqlite)
            .build()
            .expect("composite config");
        let mut backends = std::collections::HashMap::new();
        backends.insert("primary".to_string(), primary as DynStorage);
        CompositeStorage::new(config, backends).expect("composite storage")
    }

    /// Asserts that `storage` hands out the marker's export runner, and that
    /// the options reached the marker unchanged.
    pub(crate) fn assert_export_runner_is_forwarded(
        storage: &dyn ResourceStorage,
        marker: &ExportMarkerStorage,
    ) {
        let options = ExportRunnerOptions {
            max_connections: 7,
            statement_timeout_ms: 12_345,
        };
        let runner = storage
            .export_sof_runner(&options)
            .expect("an export runner");
        assert_eq!(
            runner.runner_name(),
            EXPORT_RUNNER,
            "the wrapper must forward export_sof_runner, not fall back to its sof_runner"
        );
        assert_eq!(
            storage
                .sof_runner()
                .expect("a request runner")
                .runner_name(),
            MAIN_RUNNER
        );
        assert_eq!(*marker.export_calls.lock().unwrap(), vec![options]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_have_one_connection_per_default_job_and_the_default_budget() {
        let options = ExportRunnerOptions::default();
        assert_eq!(options.max_connections, 4);
        assert_eq!(options.statement_timeout_ms, 3_600_000);
    }
}
