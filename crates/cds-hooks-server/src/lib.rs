//! A CDS Hooks service for many clients: a registry of the EMR instances that
//! may call it, JWT validation per client (`iss`, or `iss` and `tenant`),
//! replay protection shared across instances, discovery computed per client,
//! and shared services each client enables.

pub mod auth;
pub mod catalog;
pub mod clients;
pub mod greeter_service;
pub mod replay;
pub mod server;
