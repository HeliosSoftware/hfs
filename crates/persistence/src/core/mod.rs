//! Core storage traits and abstractions.
//!
//! This module provides the foundational traits for the persistence layer:
//!
//! - [`Backend`] - Database driver abstraction
//! - [`ResourceStorage`] - Core CRUD operations
//! - [`VersionedStorage`] - Version-aware operations
//! - [`InstanceHistoryProvider`], [`TypeHistoryProvider`], [`SystemHistoryProvider`] - History access
//! - [`SearchProvider`], [`MultiTypeSearchProvider`], [`ChainedSearchProvider`] - Search capability
//! - [`Transaction`] - ACID transaction support
//! - [`CapabilityProvider`] - Runtime capability discovery
//!
//! # Trait Hierarchy
//!
//! The traits form a progressive hierarchy where more advanced traits
//! extend simpler ones:
//!
//! ```text
//! ResourceStorage
//!     └── VersionedStorage
//!             └── InstanceHistoryProvider
//!                     └── TypeHistoryProvider
//!                             └── SystemHistoryProvider
//!
//! ResourceStorage
//!     └── SearchProvider
//!             ├── MultiTypeSearchProvider
//!             ├── IncludeProvider
//!             ├── RevincludeProvider
//!             ├── ChainedSearchProvider
//!             ├── TerminologySearchProvider
//!             └── TextSearchProvider
//!
//! ResourceStorage
//!     └── TransactionProvider
//!             └── BundleProvider
//! ```
//!
//! # Backend Capabilities
//!
//! Not all backends support all features. Use [`CapabilityProvider`] to
//! discover what a backend supports at runtime:
//!
//! ```ignore
//! use helios_persistence::core::{CapabilityProvider, Interaction};
//!
//! fn check_capabilities<S: CapabilityProvider>(storage: &S) {
//!     if storage.supports_interaction("Patient", Interaction::HistoryType) {
//!         // Use type-level history
//!     }
//!
//!     let caps = storage.capabilities();
//!     println!("Backend: {}", caps.backend_name);
//!     println!("Supports transactions: {}",
//!         caps.system_interactions.contains(&SystemInteraction::Transaction));
//! }
//! ```
//!
//! # Example: Implementing a Storage Backend
//!
//! ```ignore
//! use async_trait::async_trait;
//! use helios_persistence::core::{ResourceStorage, Backend, BackendKind};
//! use helios_persistence::tenant::TenantContext;
//! use helios_persistence::types::StoredResource;
//! use helios_persistence::error::StorageResult;
//!
//! struct MyBackend {
//!     // ... backend-specific fields
//! }
//!
//! #[async_trait]
//! impl ResourceStorage for MyBackend {
//!     fn backend_name(&self) -> &'static str {
//!         "my-backend"
//!     }
//!
//!     async fn create(
//!         &self,
//!         tenant: &TenantContext,
//!         resource_type: &str,
//!         resource: serde_json::Value,
//!     ) -> StorageResult<StoredResource> {
//!         // Implementation...
//!         todo!()
//!     }
//!
//!     // ... implement other required methods
//! }
//! ```

pub mod backend;
pub mod bulk_export;
pub mod bulk_export_output;
pub mod bulk_export_worker;
pub mod bulk_submit;
pub mod bulk_submit_input;
pub mod bulk_submit_worker;
pub mod capabilities;
pub mod history;
pub mod search;
pub mod storage;
pub mod transaction;
pub mod versioned;

// Re-export main types
pub use backend::{Backend, BackendCapability, BackendConfig, BackendKind, BackendPoolStats};
pub use bulk_export::{
    BulkExportStorage, ExpiredExportRef, ExportDataProvider, ExportFileMetadata, ExportJobId,
    ExportJobMetadata, ExportLevel, ExportManifest, ExportOutputFile, ExportProgress,
    ExportRequest, ExportStatus, GroupExportProvider, NdjsonBatch, PatientExportProvider,
    RawExportManifest, RawManifestEntry, StartExportInput, TypeExportProgress, TypeFilter,
};
pub use bulk_export_output::{
    DownloadUrl, ExportOutputStore, ExportPartKey, ExportPartWriter, FinalizedPart,
};
pub use bulk_export_worker::{
    BulkExportJobStore, DefaultExportWorker, ExportClaimStrategy, ExportJobLease,
    ExportResourceProvider, ExportWorkerStorage, LeaseError, WorkerId, WorkerJobView,
};
pub use bulk_submit::{
    BulkEntryOutcome, BulkEntryResult, BulkProcessingOptions, BulkSubmitProvider,
    BulkSubmitRollbackProvider, ChangeType, EntryCountSummary, ManifestStatus, NdjsonEntry,
    StreamProcessingResult, StreamingBulkSubmitProvider, SubmissionChange, SubmissionId,
    SubmissionManifest, SubmissionStatus, SubmissionSummary,
};
pub use bulk_submit_input::{
    FileTokenProvider, RemoteFile, RemoteManifest, SubmitInputFetcher, submission_output_job_id,
};
pub use bulk_submit_worker::{
    BulkSubmitJobStore, DefaultSubmitWorker, ManifestLease, ManifestWorkerView, PollTokenTarget,
    SubmitClaimStrategy, SubmitFileRecord, SubmitFileRow, SubmitWorkerStorage,
};
pub use capabilities::{
    CapabilityProvider, GlobalSearchCapabilities, Interaction, ResourceCapabilities,
    ResourceSearchCapabilities, SearchCapabilityProvider, SearchParamCapability,
    StorageCapabilities, SystemInteraction, UnsupportedFeatureType, UnsupportedSearchFeature,
};
pub use history::{
    DifferentialHistoryProvider, HistoryEntry, HistoryMethod, HistoryPage, HistoryParams,
    InstanceHistoryProvider, SystemHistoryProvider, TypeHistoryProvider,
};
pub use search::{
    ChainedSearchProvider, FullSearchProvider, IncludeProvider, MultiTypeSearchProvider,
    RevincludeProvider, SearchProvider, SearchResult, TerminologySearchProvider,
    TextSearchProvider, resolve_includes_iterative,
};
pub use storage::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalPatchResult, ConditionalStorage,
    ConditionalUpdateResult, PatchFormat, PurgableStorage, ResourceStorage,
};
pub use transaction::{
    BundleEntry, BundleEntryResult, BundleMethod, BundleProvider, BundleResult, BundleType,
    IsolationLevel, LockingStrategy, Transaction, TransactionOptions, TransactionProvider,
};
pub use versioned::{VersionConflictInfo, VersionedStorage, check_version_match, normalize_etag};
