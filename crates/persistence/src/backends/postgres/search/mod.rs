//! PostgreSQL search implementation.
//!
//! This module contains the search query builder and parameter handlers
//! for the PostgreSQL backend, using $N parameter placeholders,
//! ILIKE for case-insensitive matching, and native TIMESTAMPTZ comparisons.

pub mod chain_builder;
pub(crate) mod composite_rows;
pub mod query_builder;
pub mod writer;
