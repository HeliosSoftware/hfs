//! # Helios Terminology Server (HTS)
//!
//! HTS is a FHIR R4 Terminology Server implementing the operations defined in
//! the FHIR specification for CodeSystem, ValueSet, and ConceptMap resources.
//! It runs as a standalone HTTP server and can optionally be integrated with
//! the main Helios FHIR Server (`hfs`) to power `$expand`-based search and
//! FHIRPath `memberOf()` / `subsumes()` calls.
//!
//! ## Core operations
//!
//! | Operation | Endpoint |
//! |-----------|----------|
//! | `$lookup` | `POST /CodeSystem/$lookup` |
//! | `$validate-code` | `POST /CodeSystem/$validate-code`, `POST /ValueSet/$validate-code` |
//! | `$subsumes` | `POST /CodeSystem/$subsumes` |
//! | `$expand` | `POST /ValueSet/$expand` |
//! | `$translate` | `POST /ConceptMap/$translate` |
//! | `$closure` | `POST /ConceptMap/$closure` |
//! | Bundle import | `POST /import` |
//! | CRUD | `GET/POST/PUT/DELETE /CodeSystem`, `/ValueSet`, `/ConceptMap` |
//!
//! ## Key types
//!
//! - [`state::AppState`] — shared application state injected into every handler.
//! - [`traits::TerminologyBackend`] — supertrait implemented by all backends.
//! - [`backends::SqliteTerminologyBackend`] — SQLite backend (default).
//! - [`error::HtsError`] — unified error type with `IntoResponse`.
//! - [`server::create_app`] — assembles the Axum router with all routes.
//!
//! ## Library vs binary
//!
//! This crate is both a library and a binary.  All modules are `pub` so that
//! integration tests can construct a test [`AppState`] and router without
//! replicating the binary's bootstrap logic.
//!
//! [`AppState`]: state::AppState
//! [`create_app`]: server::create_app

pub mod backends;
pub mod config;
pub mod ecl;
pub mod error;
pub mod import;
pub mod operations;
pub mod server;
pub mod state;
pub mod traits;
pub mod types;
