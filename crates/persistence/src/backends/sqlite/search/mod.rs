//! SQLite Search Implementation.
//!
//! This module provides the SQLite-specific search implementation including:
//!
//! - Query builder for translating FHIR search queries to SQL
//! - Chain builder for forward/reverse chained parameters
//! - Parameter handlers for each search parameter type
//! - Modifier handlers for search modifiers
//! - Full-text search (FTS5) integration
//! - Filter parser for _filter parameter
//! - Search index writer implementation

pub mod chain_builder;
pub mod filter_parser;
pub mod fts;
pub mod modifier_handlers;
pub mod parameter_handlers;
pub mod query_builder;
pub mod strategy;
pub mod writer;

pub use chain_builder::{ChainError, ChainLink, ChainQueryBuilder, ParsedChain};
pub use filter_parser::{FilterExpr, FilterOp, FilterParseError, FilterParser, FilterSqlGenerator};
pub use parameter_handlers::CompositeComponentDef;
pub use query_builder::{KeysetKey, QueryBuilder, SortValueKind, SqlFragment, SqlParam};
pub use strategy::{SearchStrategyCapability, SqliteSearchStrategy};
pub use writer::{SqlValue, SqliteSearchIndexWriter};
