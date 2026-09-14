//! FHIR `Patient/$everything` operation.
//!
//! Composed in the REST layer over the per-type [`SearchProvider::search`]
//! and the query-time compartment predicate that `GET /Patient/{id}/*` uses.
//! See `docs/superpowers/specs/2026-09-14-patient-everything-design.md`.

pub(crate) mod params;
