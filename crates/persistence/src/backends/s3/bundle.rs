//! Transaction bundle handling for the S3 backend.
//!
//! S3 has no atomic multi-object operation, so it cannot honour the
//! all-or-nothing contract of a FHIR `transaction` Bundle and declines them
//! outright — see [`supports_atomic_transactions`] (#489).
//!
//! `batch` Bundles are unaffected and remain fully supported. They are executed
//! entry-by-entry by the REST layer against `ResourceStorage`, which is where
//! per-entry authorization and audit live; see the note on [`BundleProvider`].
//!
//! [`supports_atomic_transactions`]: BundleProvider::supports_atomic_transactions

use async_trait::async_trait;

use crate::core::{BundleEntry, BundleProvider, BundleResult};
use crate::error::TransactionError;
use crate::tenant::TenantContext;

use super::backend::S3Backend;

#[async_trait]
impl BundleProvider for S3Backend {
    /// S3 has no multi-object atomicity, and a compensation log cannot
    /// substitute for one.
    ///
    /// Two independent reasons, both established by #489 against the
    /// compensating-delete implementation that used to live here:
    ///
    /// 1. **The rollback was unreachable on the failure mode that matters.**
    ///    The HTTP layer applies a `TimeoutLayer`, which on expiry *drops* the
    ///    handler future. Async cancellation stops the task at its current await
    ///    point without propagating an error, so every `Err`/`status >= 400` arm
    ///    holding compensation logic is skipped by construction.
    /// 2. **The compensation list was built after the writes.** Entries executed
    ///    concurrently and the list was populated only once all of them had
    ///    resolved, so at the moment of cancellation it was still empty — there
    ///    was nothing to undo *with*, even given a cancellation-safe unwind.
    ///
    /// The observed result was 466 of 473 entries durably committed, with no
    /// tombstone or compensating delete, while the client received a 408.
    ///
    /// This is also what the architecture already says. `crates/persistence`'s
    /// README describes S3 as "intentionally storage-focused … archive/history
    /// storage", and design discussion #28 places ACID on the relational tier.
    /// Refusing here matches the documented role instead of approximating a
    /// guarantee the tier was never meant to offer.
    fn supports_atomic_transactions(&self) -> bool {
        false
    }

    async fn process_transaction(
        &self,
        _tenant: &TenantContext,
        _entries: Vec<BundleEntry>,
        _fhir_version: helios_fhir::FhirVersion,
    ) -> Result<BundleResult, TransactionError> {
        // Refused before any work. A partial commit is worse than a rejection:
        // the caller cannot distinguish 408-with-466-writes from 408-with-none,
        // and a retry double-writes every POST entry (server-assigned ids, so
        // no idempotency). See `supports_atomic_transactions`.
        Err(TransactionError::AtomicityUnsupported {
            backend_name: "s3".to_string(),
        })
    }
}
