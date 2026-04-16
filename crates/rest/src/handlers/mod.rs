//! HTTP request handlers for FHIR interactions.
//!
//! This module contains handlers for all FHIR REST API interactions:
//!
//! - [`read`] - Read a resource by ID
//! - [`vread`] - Read a specific version of a resource
//! - [`create`] - Create a new resource
//! - [`update`] - Update an existing resource
//! - [`patch`] - Patch a resource
//! - [`delete`] - Delete a resource
//! - [`search`] - Search for resources
//! - [`history`] - Get resource history
//! - [`batch`] - Process a batch/transaction bundle
//! - [`capabilities`] - Get server capabilities (CapabilityStatement)
//! - [`versions`] - Get supported FHIR versions ($versions operation)
//! - [`health`] - Health check endpoint

pub mod batch;
pub mod capabilities;
pub mod compartment;
pub mod create;
pub mod delete;
pub mod health;
pub mod history;
pub mod patch;
pub mod read;
pub mod search;
pub mod smart_discovery;
#[cfg(feature = "sof")]
pub mod sof;
pub mod update;
pub mod versions;
pub mod vread;

// Re-export handlers for convenience
pub use batch::batch_handler;
pub use capabilities::capabilities_handler;
pub use compartment::compartment_search_handler;
pub use create::create_handler;
pub use delete::{conditional_delete_handler, delete_handler};
pub use health::health_handler;
pub use history::{
    delete_instance_history_handler, delete_version_handler, history_instance_handler,
    history_system_handler, history_type_handler,
};
pub use patch::patch_handler;
pub use read::{head_read_handler, read_handler};
pub use search::{search_get_handler, search_post_handler};
pub use update::{conditional_update_handler, update_handler};
pub use versions::versions_handler;
pub use vread::vread_handler;
