//! Test-binary entry point that wires the `tests/multitenancy/` suite into the build.
//!
//! Cargo only compiles top-level `tests/*.rs` files as integration-test
//! binaries; a `tests/<dir>/` subtree is invisible until some top-level file
//! declares `mod <dir>;`. Without this file the entire `multitenancy/` suite is dead
//! code that never compiles or runs (issue #306). `mod multitenancy;` resolves to
//! `tests/multitenancy/mod.rs`.
//!
//! Gated on `sqlite`: every test in the suite is `#[cfg(feature = "sqlite")]`
//! and builds an in-memory `SqliteBackend`, so the binary is empty (and its
//! imports unused) without that feature.
#![cfg(feature = "sqlite")]

mod multitenancy;
