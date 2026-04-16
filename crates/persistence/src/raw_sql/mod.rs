//! Concrete [`RawSqlRunner`](crate::core::RawSqlRunner) implementations.
//!
//! Each implementation targets one database backend and is gated behind the
//! corresponding feature flag:
//!
//! | Module | Backend | Feature |
//! |--------|---------|---------|
//! | [`sqlite`] | SQLite | `sqlite` |
//! | [`postgres`] | PostgreSQL | `postgres` |

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteRawRunner;

#[cfg(feature = "postgres")]
pub use postgres::PgRawRunner;
