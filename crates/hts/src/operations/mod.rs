//! Axum route handlers for all FHIR Terminology Service operations.
//!
//! Each sub-module owns the handlers (and supporting logic) for one operation
//! family.  All handlers are generic over `B: TerminologyBackend` so the same
//! routing layer works with any storage backend.
//!
//! ## Module layout
//!
//! | Module | Endpoint(s) |
//! |--------|-------------|
//! | [`batch`] | `POST /` — FHIR batch/transaction |
//! | [`closure`] | `POST /ConceptMap/$closure` |
//! | [`crud`] | `GET/POST/PUT/DELETE /CodeSystem`, `/ValueSet`, `/ConceptMap` |
//! | [`expand`] | `GET/POST /ValueSet/$expand` |
//! | [`format`] | Content-type negotiation and JSON→XML serialization (shared) |
//! | [`health`] | `GET /health` |
//! | [`import_bundle`] | `POST /import` |
//! | [`lookup`] | `GET/POST /CodeSystem/$lookup` |
//! | [`metadata`] | `GET /metadata` |
//! | [`params`] | FHIR Parameters parsing helpers (shared) |
//! | [`search`] | `GET /CodeSystem`, `/ValueSet`, `/ConceptMap` |
//! | [`subsumes`] | `GET/POST /CodeSystem/$subsumes` |
//! | [`translate`] | `GET/POST /ConceptMap/$translate` |
//! | [`validate_code`] | `GET/POST /CodeSystem/$validate-code`, `/ValueSet/$validate-code` |
//!
//! [`batch`]: self::batch
//! [`closure`]: self::closure
//! [`crud`]: self::crud
//! [`expand`]: self::expand
//! [`format`]: self::format
//! [`health`]: self::health
//! [`import_bundle`]: self::import_bundle
//! [`lookup`]: self::lookup
//! [`metadata`]: self::metadata
//! [`params`]: self::params
//! [`search`]: self::search
//! [`subsumes`]: self::subsumes
//! [`translate`]: self::translate
//! [`validate_code`]: self::validate_code

pub mod batch;
pub mod batch_validate;
pub mod closure;
pub mod crud;
pub mod expand;
pub mod format;
pub mod health;
pub mod import_bundle;
pub mod lookup;
pub mod metadata;
pub mod params;
pub mod search;
pub mod subsumes;
pub mod translate;
pub mod validate_code;
