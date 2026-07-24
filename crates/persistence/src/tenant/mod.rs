//! Tenant management for multi-tenant FHIR storage.
//!
//! This module provides the core types for multi-tenant support in the persistence layer.
//! All storage operations require a [`TenantContext`] to ensure proper tenant isolation.
//!
//! # Core Types
//!
//! - [`TenantId`] - Opaque tenant identifier with hierarchical namespace support
//! - [`TenantContext`] - Validated context required for all storage operations
//! - [`TenantPermissions`] - Defines what operations a tenant can perform
//! - [`TenancyModel`] - Determines how resources are isolated between tenants
//!
//! # Design Philosophy
//!
//! The persistence layer scopes tenant data with a `TenantContext`: every
//! tenant-scoped storage operation requires one as its first argument, so a
//! tenant-scoped operation cannot be constructed without it. (A few operations
//! are intentionally cross-tenant — the admin aggregate `count_by_tenant`, the
//! tenant-registry calls — and take no context by design.)
//!
//! # Tenancy Model
//!
//! There is one isolation model: **shared schema with a `tenant_id`
//! discriminator**, chosen in design discussion
//! [#28](https://github.com/HeliosSoftware/hfs/discussions/28). Each backend
//! applies it in its own idiom; the S3 backend additionally offers a
//! bucket-per-tenant physical boundary. Schema-per-tenant and
//! database-per-tenant are not offered for the SQL backends — see the crate
//! root docs and [#370](https://github.com/HeliosSoftware/hfs/issues/370).
//!
//! # Examples
//!
//! ## Creating a Tenant Context
//!
//! ```
//! use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
//!
//! // Full access context
//! let ctx = TenantContext::new(
//!     TenantId::new("acme-corp"),
//!     TenantPermissions::full_access(),
//! );
//!
//! // Read-only context
//! let read_ctx = TenantContext::new(
//!     TenantId::new("acme-corp"),
//!     TenantPermissions::read_only(),
//! );
//!
//! // System tenant for shared resources
//! let system_ctx = TenantContext::system();
//! ```
//!
//! ## Hierarchical Tenants
//!
//! ```
//! use helios_persistence::tenant::TenantId;
//!
//! let parent = TenantId::new("acme");
//! let child = TenantId::new("acme/research");
//! let grandchild = TenantId::new("acme/research/oncology");
//!
//! assert!(child.is_descendant_of(&parent));
//! assert!(grandchild.is_descendant_of(&parent));
//! assert_eq!(grandchild.root().as_str(), "acme");
//! ```
//!
//! ## Custom Permissions
//!
//! ```
//! use helios_persistence::tenant::{TenantPermissions, Operation};
//!
//! let perms = TenantPermissions::builder()
//!     .allow_operations(vec![Operation::Read, Operation::Search])
//!     .allow_resource_types(vec!["Patient", "Observation"])
//!     .restrict_to_compartment("Patient", "123")
//!     .build();
//! ```

mod context;
mod id;
mod permissions;
mod tenancy;

pub use context::{TenantContext, TenantContextBuilder};
pub use id::{SYSTEM_TENANT, TenantId};
pub use permissions::{
    CompartmentRestriction, Operation, TenantPermissions, TenantPermissionsBuilder,
};
pub use tenancy::{CustomResourceTenancy, DefaultResourceTenancy, ResourceTenancy, TenancyModel};
