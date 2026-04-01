//! Helios Terminology Service — library entry point.
//!
//! Exposes all internal modules so that integration tests and other crates can
//! build a test [`AppState`] and [`create_app`] router without duplicating the
//! binary's bootstrap logic.
//!
//! [`AppState`]: state::AppState
//! [`create_app`]: server::create_app

pub mod backend;
pub mod config;
pub mod error;
pub mod import;
pub mod operations;
pub mod server;
pub mod state;
pub mod traits;
pub mod types;
