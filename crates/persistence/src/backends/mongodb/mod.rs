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
//!
//! Advanced search/composite behavior remains part of later phases.

pub(crate) mod backend;
mod bulk_export;
pub(crate) mod schema;
mod search_impl;
mod storage;
mod user_settings;

pub use backend::{MongoBackend, MongoBackendConfig};
