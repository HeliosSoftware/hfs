//! Transaction tests for persistence backends.
//!
//! This module contains tests for transaction handling including
//! basic transactions, FHIR bundle transactions, and rollback scenarios.

pub mod basic_tests;
pub mod bundle_tests;
pub mod rollback_tests;

/// Backend-agnostic `ifMatch` scenarios shared with the PostgreSQL suite, which
/// `#[path]`-includes this same file (see the module docs).
pub mod if_match_suite;
