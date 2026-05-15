//! FHIR topic-based Subscriptions engine for HFS.
//!
//! This crate implements the [FHIR Subscriptions Framework](https://build.fhir.org/subscriptions.html)
//! for the Helios FHIR Server. It supports topic-based subscriptions across all
//! FHIR versions (R4, R4B, R5, R6), with R4 using the
//! [Subscriptions R5 Backport IG](https://build.fhir.org/ig/HL7/fhir-subscription-backport-ig/)
//! and R4B/R5/R6 using native subscription resources.
//!
//! # Architecture
//!
//! The subscription system decomposes into five primary concerns:
//!
//! - **Topic Registry** — stores `SubscriptionTopic` definitions and evaluates triggers
//! - **Subscription Manager** — CRUD lifecycle and status tracking for `Subscription` resources
//! - **Event Evaluator** — matches resource write events against active subscriptions
//! - **Notification Builder** — constructs version-specific notification bundles
//! - **Channel Dispatcher** — delivers notifications via rest-hook, WebSocket, etc.
//!
//! The [`SubscriptionEngine`] orchestrates all five concerns and is the main
//! entry point, invoked asynchronously after each resource write.

pub mod channels;
pub mod config;
pub mod engine;
pub mod error;
pub mod evaluator;
pub mod event;
pub mod manager;
pub mod notification;
pub mod topics;

// Re-export key types for convenience.
pub use channels::messaging::MessagingChannel;
pub use channels::ws_manager::WebSocketManager;
pub use channels::ws_token::WsBindingTokenManager;
pub use config::{MessagingSettings, SubscriptionConfig};
pub use engine::SubscriptionEngine;
pub use error::SubscriptionError;
pub use event::{ResourceEvent, ResourceEventType};
