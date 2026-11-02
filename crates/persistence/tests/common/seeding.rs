//! Seeding writes for a test's setup (#2435): the `ResourceStorage` write, run
//! again after an eviction rollback and after nothing else.
//!
//! `#[path]`-included as `mod seeding;` at the crate root of every test binary
//! that compiles a converted file (`mongodb_tests`, `sqlite_tests`,
//! `search_suite`, `transactions_suite`, `multitenancy_suite`, `postgres_tests`,
//! `elasticsearch_tests` and `minio_s3_tests`), because the cross-backend
//! suites reach it as `crate::seeding` from wherever they are mounted.
//!
//! The rule is backend-agnostic in data: an error is run again only when it is
//! the `Unavailable` of backend `"mongodb"` with the exact eviction-rollback
//! message, so on SQLite, PostgreSQL, Elasticsearch and S3 any error returns
//! from the first attempt, with no pause and no second run.
//!
//! This is for a test's setup only, never for the write a test asserts about.
//! The pure and failpoint tests of it are `mongodb/seeding_retry.rs`.

// Each test binary uses a subset of these (the SQLite binaries have no use for
// the retry counter's readers, `minio_s3_tests` only seeds creates and deletes).
#![allow(dead_code)]

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::error::{BackendError, StorageError};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::StoredResource;
use serde_json::Value;

/// The single-resource writes whose eviction rollback `replay_aborted_writes`
/// (`src/backends/mongodb/storage.rs`) answers with a retryable `Unavailable`
/// and, on purpose, does not run again (#1895).
pub(crate) const EVICTION_ROLLBACK_WRITES: [&str; 4] = ["create", "update", "delete", "restore"];

/// Attempts a seeding write makes before its eviction rollback is let through
/// (the first attempt included).
pub(crate) const SEED_EVICTION_ATTEMPTS: u32 = 5;

/// The wait after a seeding write's first eviction rollback. It doubles after
/// each attempt: 200, 400, 800, 1,600 ms.
const SEED_EVICTION_BACKOFF: Duration = Duration::from_millis(200);

/// How many times a seeding write was run again after an eviction rollback in
/// this process (#2435). `db_cleanup` prints it on its end-of-run line.
static SEED_EVICTION_RETRIES: AtomicU64 = AtomicU64::new(0);

/// The number of seeding writes run again after an eviction rollback in this
/// process so far, less the ones a test forced and took back out with
/// [`forget_forced_seed_eviction_retries`].
pub(crate) fn seed_eviction_retries() -> u64 {
    SEED_EVICTION_RETRIES.load(Ordering::Relaxed)
}

/// Takes `forced` retries back out of [`seed_eviction_retries`]: a test that
/// makes the server roll a seeding write back on purpose (a failpoint) calls
/// this after its asserts, so the count the end-of-run line prints is how often
/// eviction really hit a test's setup.
pub(crate) fn forget_forced_seed_eviction_retries(forced: u64) {
    SEED_EVICTION_RETRIES.fetch_sub(forced, Ordering::Relaxed);
}

/// True when `err` is the `Unavailable` a single-resource write is answered
/// with after the database rolled it back under cache pressure.
///
/// The error carries no driver source and no typed marker (`write_unavailable`
/// builds it from a message), so the variant, the backend and the whole message
/// are matched. Any other error is not one, including another "retry the
/// request" 503 (a replay that ran out of attempts, no usable server).
pub(crate) fn is_eviction_rollback_unavailable(err: &StorageError) -> bool {
    let StorageError::Backend(BackendError::Unavailable {
        backend_name,
        message,
    }) = err
    else {
        return false;
    };
    backend_name == "mongodb" && EVICTION_ROLLBACK_WRITES.iter().any(|what| {
        *message
            == format!(
                "The {what} was rolled back by the database under cache pressure; retry the request"
            )
    })
}

/// Runs `write` again, up to [`SEED_EVICTION_ATTEMPTS`] times with a doubling
/// pause, while it ends as an eviction rollback ([`is_eviction_rollback_unavailable`]);
/// any other outcome is returned at once. Each run again is counted in
/// [`seed_eviction_retries`].
///
/// The harness's shared mongod has a 0.25 GB cache, and the product answers a
/// write that the database rolled back under cache pressure with a 503 that the
/// client must retry, so a seeding `.unwrap()` used to panic in a test's setup
/// instead of testing anything (#2435). This is for a test's setup only, never
/// for the write a test asserts about.
pub(crate) async fn retry_eviction_rollback<T, F, Fut>(
    what: &str,
    write: F,
) -> Result<T, StorageError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, StorageError>>,
{
    retry_eviction_rollback_counting(&SEED_EVICTION_RETRIES, what, write).await
}

/// [`retry_eviction_rollback`] with its own `retries` counter, so a test can
/// assert exact numbers without the process-wide one.
pub(crate) async fn retry_eviction_rollback_counting<T, F, Fut>(
    retries: &AtomicU64,
    what: &str,
    mut write: F,
) -> Result<T, StorageError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, StorageError>>,
{
    let mut attempt = 1;
    loop {
        match write().await {
            Err(err)
                if attempt < SEED_EVICTION_ATTEMPTS && is_eviction_rollback_unavailable(&err) =>
            {
                retries.fetch_add(1, Ordering::Relaxed);
                let pause = SEED_EVICTION_BACKOFF * 2_u32.pow(attempt - 1);
                eprintln!(
                    "seeding {what}: attempt {attempt} of {SEED_EVICTION_ATTEMPTS} was rolled back for eviction ({err}); running it again in {} ms",
                    pause.as_millis()
                );
                tokio::time::sleep(pause).await;
                attempt += 1;
            }
            outcome => return outcome,
        }
    }
}

/// Seeding writes for a test's setup: the `ResourceStorage` write, run again
/// after an eviction rollback and after nothing else (#2435).
///
/// A call site converts by renaming the method (`create` to `seed_create`,
/// `update` to `seed_update`, `create_or_update` to `seed_put`, `delete` to
/// `seed_delete`); it keeps its own `.unwrap()` or `.expect()`. Never use it for
/// the write a test asserts about.
#[async_trait::async_trait]
pub(crate) trait SeedingStorage: ResourceStorage {
    /// [`ResourceStorage::create`], run again after an eviction rollback.
    async fn seed_create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> Result<StoredResource, StorageError> {
        retry_eviction_rollback(&format!("{resource_type} create"), || {
            self.create(tenant, resource_type, resource.clone(), fhir_version)
        })
        .await
    }

    /// [`ResourceStorage::update`], run again after an eviction rollback.
    async fn seed_update(
        &self,
        tenant: &TenantContext,
        current: &StoredResource,
        resource: Value,
    ) -> Result<StoredResource, StorageError> {
        retry_eviction_rollback(
            &format!("{}/{} update", current.resource_type(), current.id()),
            || self.update(tenant, current, resource.clone()),
        )
        .await
    }

    /// [`ResourceStorage::create_or_update`] (a PUT), run again after an
    /// eviction rollback. Whichever of create, update or restore the PUT
    /// reaches, the rollback is the same `Unavailable`.
    async fn seed_put(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        resource: Value,
        fhir_version: FhirVersion,
    ) -> Result<(StoredResource, bool), StorageError> {
        retry_eviction_rollback(&format!("{resource_type}/{id} PUT"), || {
            self.create_or_update(tenant, resource_type, id, resource.clone(), fhir_version)
        })
        .await
    }

    /// [`ResourceStorage::delete`], run again after an eviction rollback.
    async fn seed_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> Result<(), StorageError> {
        retry_eviction_rollback(&format!("{resource_type}/{id} delete"), || {
            self.delete(tenant, resource_type, id)
        })
        .await
    }
}

impl<S: ResourceStorage + ?Sized> SeedingStorage for S {}
