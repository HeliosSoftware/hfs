//! Common test utilities for REST API testing.
//!
//! This module provides test infrastructure including:
//!
//! - [`harness`] - REST API test harness
//! - [`fixtures`] - Test data fixtures
//! - [`assertions`] - HTTP response assertions
//! - [`spec_loader`] - JSON test specification loader
//! - [`backend_config`] - Backend configuration for tests

// Allow dead code in test utilities - they may be used by future tests
#![allow(dead_code)]

pub mod assertions;
pub mod backend_config;
pub mod everything;
pub mod fixtures;
pub mod harness;
pub mod spec_loader;
