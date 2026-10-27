//! In-process cache invalidation contract for terminology backends.
//!
//! Every backend memoises derived answers in process memory — assembled
//! `$lookup` / `$validate-code` responses, concept indexes, resolved CodeSystem
//! metadata. Those memos are correct only for as long as the underlying
//! normalized tables do not change, so any code path that mutates terminology
//! content has to tell the backend to drop them.
//!
//! # Why this is its own trait
//!
//! The obvious home for this hook is [`BundleImportBackend`], since that is
//! where eviction used to live. It is the wrong home: the SQLite CRUD path
//! deliberately bypasses `import_bundle` and writes through its own pooled
//! connection, so hanging invalidation off the import trait is exactly how a
//! whole backend ended up never evicting anything on `POST`/`PUT`/`DELETE`
//! (issue #304). Invalidation belongs to the *mutation*, not to the mechanism
//! that happened to perform it, so it gets a trait of its own that
//! [`TerminologyBackend`] requires unconditionally.
//!
//! # Why there is no default implementation
//!
//! A defaulted no-op would let a new backend inherit silence: it would compile,
//! serve stale answers, and nothing would point at the omission. Implementors
//! must write the method, even if the honest body is empty with a comment
//! explaining why that backend holds no state.
//!
//! [`BundleImportBackend`]: crate::import::BundleImportBackend
//! [`TerminologyBackend`]: crate::traits::TerminologyBackend

/// Drops every in-process cache a backend maintains.
pub trait TerminologyCaches: Send + Sync {
    /// Evict all in-process caches, so the next read re-derives from storage.
    ///
    /// Called after any successful mutation of terminology content — CRUD
    /// create/update/delete and bundle import alike — and required to be
    /// idempotent, because paths that already evict internally (the PostgreSQL
    /// importer, for one) are also reached through the generic CRUD seam.
    ///
    /// This is deliberately total rather than scoped to the resource that
    /// changed. An expansion answer is derived from a transitive closure —
    /// ValueSet includes ValueSet includes CodeSystem, plus supplements and
    /// hierarchy — and no cache entry records the set of resources it was
    /// derived from, so there is no key material to invalidate against. The
    /// implicit-ValueSet case makes the point concretely: creating a
    /// *CodeSystem* with url `X` has to un-cache a 404 recorded against the
    /// *ValueSet* url `X?fhir_vs=`, a different string entirely. A URL-matched
    /// eviction would look precise and still serve the stale 404.
    ///
    /// Writes to a terminology server are rare (content publication) relative
    /// to reads, so paying a cold cache per write is the right trade. See the
    /// module docs for why this is not a defaulted method.
    fn invalidate_caches(&self);
}
