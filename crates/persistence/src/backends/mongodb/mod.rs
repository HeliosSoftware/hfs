//! MongoDB backend implementation.
//!
//! This module provides MongoDB backend wiring, schema bootstrap helpers,
//! and storage contract support through Phase 4.
//!
//! Phase 4 scope currently includes:
//! - backend/config wiring and health checks
//! - core [`crate::core::ResourceStorage`] contract parity for CRUD/count
//! - [`crate::core::VersionedStorage`] for vread and If-Match update/delete
//! - history providers for instance/type/system history retrieval
//! - tenant isolation and soft-delete semantics
//! - schema/index bootstrap foundations (including search index collection)
//! - basic [`crate::core::SearchProvider`] support for first-wave parameter types
//! - [`crate::core::ConditionalStorage`] support for create/update/delete
//! - composite search parameters (#1206), via grouped `(resource_id,
//!   composite_group)` pair checks — see `composite_search` and
//!   `docs/mongodb/search-indexes.md`

pub(crate) mod backend;
mod bulk_export;
mod bulk_ingest;
pub(crate) mod bulk_provider;
mod bulk_submit;
mod composite_search;
mod retry;
pub(crate) mod schema;
mod search_impl;
pub(crate) mod search_index_builder;
pub(crate) mod search_index_catalog;
mod storage;
mod user_settings;

pub use backend::{MongoBackend, MongoBackendConfig};
pub use search_index_builder::{BuildOutcome, IndexBuildMode};
