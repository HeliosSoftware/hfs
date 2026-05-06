//! Optional reference resolution hook for validator integrations.
//!
//! This trait is intentionally lightweight and side-effect free from the
//! perspective of `fhir-validation`. Callers can implement it with bundle-local
//! lookups, persistence-backed reads, or HTTP resolution.

/// Resolve a reference string (for example `Patient/123`) into resource JSON.
///
/// Return `None` when resolution is unavailable or the target cannot be found.
pub trait ReferenceResolver: Send + Sync {
    fn resolve_reference(&self, reference: &str) -> Option<serde_json::Value>;
}
