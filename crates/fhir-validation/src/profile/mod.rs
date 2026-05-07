//! Profile validation: StructureDefinition extraction and conformance checking.
//!
//! This subtree implements **runtime profile conformance** on top of the crate’s
//! FHIRPath and structural checks. Work flows in two phases:
//!
//! 1. **Extract** [`types::ExtractedProfile`] and [`types::ExtractedElementRule`] from
//!    [`StructureDefinition`](https://hl7.org/fhir/structuredefinition.html) JSON via
//!    [`extract::extract_structure_definition_profile_from_json`] (or version helpers in
//!    [`extract`]).
//! 2. **Validate** a serialized resource instance against those rules using
//!    [`validate::validate_profile`], optionally combined with base-resource validation at the
//!    crate root (`validate_resource_with_profiles`, etc.).
//!
//! # Submodule map
//!
//! | Module | Role |
//! |--------|------|
//! | [`types`] | Normalized structs produced by extraction (`ExtractedProfile`, element rules, slicing metadata, type constraints). |
//! | [`extract`] / [`extract_core`] | Snapshot-first parsing of `StructureDefinition` JSON into extracted types; shared across FHIR versions. |
//! | [`structure_definition_extract`] | Structured error variants when extraction fails (`ValidationError::InvalidStructureDefinition`). |
//! | [`validate`] | Main pipeline: invariants, cardinality, mustSupport, slicing, fixed/pattern, bounds, bindings, `type.profile` recursion, declared profiles. |
//! | [`cardinality`] | Min/max cardinality and mustSupport using JSON path counts relative to the instance (see module for slice caveats). |
//! | [`slicing`] | Slice membership, discriminator matching (`value`, `type`, `exists`, `position`, partial `profile`), openness, ordering. |
//! | [`element_bounds`] | `maxLength`, `minValue[x]`, `maxValue[x]` against instance JSON with FHIR precision types. |
//! | [`helpers`] | Dotted-path traversal over JSON (arrays, choice `[x]` segments) shared by cardinality, slicing, and bounds. |
//! | [`profile_registry`] | **URL → extracted profile** map for [`crate::validation_context::ValidationContext::runtime_profile_registry`] and `meta.profile` validation. |
//! | [`base_definition_fetch_url`] | Rewrites canonical `baseDefinition` URLs to fetchable static JSON paths (HL7 NPM layout, NDHM). |
//!
//! # Typical integration
//!
//! - Load or build a [`ProfileRegistry`](crate::profile::profile_registry::ProfileRegistry) with `StructureDefinition`s your server publishes or
//!   ships (IG snapshot JSON).
//! - Pass `Some(&registry)` as [`crate::validation_context::ValidationContext::runtime_profile_registry`]
//!   so nested `type.profile` / **`meta.profile`** validation can resolve URLs without HTTP.
//! - Optional: enable [`crate::ValidationConfig::enable_base_definition_url_lookup`] so missing
//!   base definitions can be fetched (subject to URL allowlists and size limits); URL rewriting
//!   is handled by [`base_definition_fetch_url`].
//!
//! # Reading order for contributors
//!
//! 1. [`types`] — data model for what extraction produces.
//! 2. [`extract`] module docs — differential vs snapshot, what is intentionally not extracted.
//! 3. [`validate`] module docs — invariant focus rules and validation pipeline order.
//! 4. [`slicing`] — discriminator behavior and known gaps (`resolve()`).
//! 5. [`cardinality`] — how JSON counting interacts with **sliced** paths.

pub mod base_definition_fetch_url;
pub mod cardinality;
pub mod element_bounds;
pub mod extract;
pub mod extract_core;
pub mod helpers;
pub mod profile_registry;
pub mod slicing;
pub mod structure_definition_extract;
pub mod types;
pub mod validate;
