//! A CDS Hooks 2.0 service for many clients: a registry of the EMR instances
//! that may call it, JWT validation per client, discovery computed per client,
//! and shared services each client enables.

pub mod auth;
pub mod catalog;
pub mod clients;
pub mod galleri_service;
pub mod server;
