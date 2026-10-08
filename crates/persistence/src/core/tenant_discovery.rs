//! Cross-tenant data discovery (#1672, #1848).
//!
//! [`ResourceStorage::discover_tenants`](crate::core::ResourceStorage::discover_tenants)
//! answers "which tenants hold data?" for background consumers such as the
//! web UI's Tenants inventory. It keeps three facts apart that
//! [`count_by_tenant`](crate::core::ResourceStorage::count_by_tenant) cannot:
//!
//! - **evidence**: per tenant, either an exact number with the population it
//!   counts ([`TenantDataEvidence::Counted`]) or presence only, with no number
//!   at all ([`TenantDataEvidence::Present`]);
//! - **coverage**: whether the listed tenants are everything in scope
//!   ([`DiscoveryCoverage::Complete`]), a budget-limited slice whose absent ids
//!   are unknown ([`DiscoveryCoverage::Partial`]), or a capability boundary
//!   that must never be read as an empty store
//!   ([`DiscoveryCoverage::Unsupported`]);
//! - **budget and resumption**: [`DiscoveryRequest`] bounds the round trips a
//!   backend that needs many of them (S3) may spend, and carries the
//!   [`DiscoveryCursor`] of a previous partial result.
//!
//! The types here hold no cache and no scheduler: the caller owns
//! single-flight, TTL and invalidation.

use std::num::NonZeroU32;

/// The population a per-tenant number counts.
///
/// Describes *what* was counted, not how fresh it is; freshness belongs to
/// the consumer's cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CountBasis {
    /// Exact count of non-deleted current resources in the authoritative
    /// store, as of one query (the SQLite, PostgreSQL and MongoDB
    /// `is_deleted = false` grouped count).
    LiveResources,
    /// S3 current-pointer objects, delete tombstones included (what the S3
    /// `count_by_tenant` returns). Proves purgeable data; it is not a live
    /// total. S3 `discover_tenants` never produces it, because that would need
    /// an exhaustive walk of every tenant's resources.
    CurrentPointersInclTombstones,
    /// Live documents in a search index. Exact relative to the index only: the
    /// index may lag or omit the primary. Never produced by cross-tenant
    /// discovery (there is no cross-tenant search-index aggregate).
    IndexedLiveDocuments,
}

/// What a presence-only finding proves. Carries no number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresenceBasis {
    /// At least one object exists under the tenant's `resources/` namespace
    /// (a live pointer, a history version or a tombstone): the tenant has
    /// purgeable data.
    ResourceObjects,
}

/// Evidence that one tenant holds data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TenantDataEvidence {
    /// An exact number of `basis` items.
    Counted {
        /// How many `basis` items the tenant holds; always greater than zero.
        resources: u64,
        /// The population `resources` counts.
        basis: CountBasis,
    },
    /// Data is present but no count was obtained. Consumers must never render
    /// or derive a number from this.
    Present {
        /// What the presence finding proves.
        basis: PresenceBasis,
    },
}

/// One tenant found by discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredTenant {
    /// The raw stored tenant id. It may be the internal system tenant, a
    /// reserved id or a non-canonical legacy id; filtering is the consumer's
    /// job, as it is for `count_by_tenant`.
    pub id: String,
    /// Why this tenant is listed.
    pub evidence: TenantDataEvidence,
}

/// Opaque resume point of a budget-limited discovery.
///
/// Only valid for the backend instance that issued it, and not a snapshot:
/// tenants created or purged between slices may be missed or seen twice. (S3:
/// the last enumerated tenant group, used as `StartAfter`.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryCursor(String);

impl DiscoveryCursor {
    /// Wraps a backend-specific resume token.
    pub fn new(opaque: impl Into<String>) -> Self {
        Self(opaque.into())
    }

    /// The backend-specific resume token.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How much of the backend's data scope a [`TenantDiscovery`] covers.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiscoveryCoverage {
    /// Every tenant with data in scope is listed. An absent id has no data
    /// for the reported basis (a measured zero for
    /// [`CountBasis::LiveResources`]).
    Complete,
    /// Discovery stopped at the request budget. Absent ids are **unknown**,
    /// not empty. `resume` continues where this slice ended; `None` means the
    /// slice cannot be resumed.
    Partial {
        /// Where the next call should continue, if the backend can resume.
        resume: Option<DiscoveryCursor>,
    },
    /// This backend or configuration cannot discover data-only tenants (the
    /// trait default, search-index backends, S3 bucket-per-tenant). Never
    /// equivalent to an empty store.
    Unsupported {
        /// What this backend cannot do, for logs and diagnostics.
        capability: &'static str,
    },
}

/// Work bound and resume point for one
/// [`discover_tenants`](crate::core::ResourceStorage::discover_tenants) call.
///
/// Backends that answer with one aggregate query ignore the budget; their own
/// server-side limits (statement timeouts, `maxTimeMS`) still apply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveryRequest {
    /// Maximum backend round trips (for S3, LIST requests, probes included)
    /// this call may issue. `None` uses the backend default.
    pub max_requests: Option<NonZeroU32>,
    /// Continue a previous [`DiscoveryCoverage::Partial`] result.
    pub resume: Option<DiscoveryCursor>,
}

/// The result of one discovery call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TenantDiscovery {
    /// Tenants found with data, in backend order. Raw ids, unfiltered.
    pub tenants: Vec<DiscoveredTenant>,
    /// How much of the backend's data scope `tenants` covers.
    pub coverage: DiscoveryCoverage,
}

impl TenantDiscovery {
    /// No tenants and [`DiscoveryCoverage::Unsupported`]: `capability` names
    /// what this backend cannot do, for logs and diagnostics.
    pub fn unsupported(capability: &'static str) -> Self {
        Self {
            tenants: Vec::new(),
            coverage: DiscoveryCoverage::Unsupported { capability },
        }
    }

    /// Complete discovery from a grouped count that already spans every
    /// tenant in the store, such as `count_by_tenant` on the SQL and document
    /// backends. Rows with a zero count are dropped, so an absent id always
    /// means a measured zero.
    pub fn from_grouped_counts(counts: Vec<(String, u64)>, basis: CountBasis) -> Self {
        Self {
            tenants: counts
                .into_iter()
                .filter(|(_, resources)| *resources > 0)
                .map(|(id, resources)| DiscoveredTenant {
                    id,
                    evidence: TenantDataEvidence::Counted { resources, basis },
                })
                .collect(),
            coverage: DiscoveryCoverage::Complete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_is_empty_with_the_named_capability() {
        let discovery = TenantDiscovery::unsupported("tenant-discovery");
        assert!(discovery.tenants.is_empty());
        assert_eq!(
            discovery.coverage,
            DiscoveryCoverage::Unsupported {
                capability: "tenant-discovery"
            }
        );
        assert_ne!(
            discovery.coverage,
            DiscoveryCoverage::Complete,
            "unsupported must never read as a complete, empty store"
        );
    }

    #[test]
    fn grouped_counts_become_complete_counted_evidence() {
        let discovery = TenantDiscovery::from_grouped_counts(
            vec![("acme".to_string(), 3), ("__system__".to_string(), 12)],
            CountBasis::LiveResources,
        );
        assert_eq!(discovery.coverage, DiscoveryCoverage::Complete);
        assert_eq!(
            discovery.tenants,
            vec![
                DiscoveredTenant {
                    id: "acme".to_string(),
                    evidence: TenantDataEvidence::Counted {
                        resources: 3,
                        basis: CountBasis::LiveResources,
                    },
                },
                // Raw ids pass through: the system tenant is the consumer's to drop.
                DiscoveredTenant {
                    id: "__system__".to_string(),
                    evidence: TenantDataEvidence::Counted {
                        resources: 12,
                        basis: CountBasis::LiveResources,
                    },
                },
            ]
        );
    }

    #[test]
    fn grouped_counts_drop_zero_rows() {
        let discovery = TenantDiscovery::from_grouped_counts(
            vec![("empty".to_string(), 0), ("acme".to_string(), 1)],
            CountBasis::LiveResources,
        );
        let ids: Vec<&str> = discovery.tenants.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["acme"]);
    }

    #[test]
    fn no_grouped_counts_is_a_complete_empty_store() {
        let discovery = TenantDiscovery::from_grouped_counts(Vec::new(), CountBasis::LiveResources);
        assert!(discovery.tenants.is_empty());
        assert_eq!(discovery.coverage, DiscoveryCoverage::Complete);
    }

    #[test]
    fn grouped_counts_keep_the_given_basis() {
        let discovery = TenantDiscovery::from_grouped_counts(
            vec![("acme".to_string(), 5)],
            CountBasis::CurrentPointersInclTombstones,
        );
        assert_eq!(
            discovery.tenants[0].evidence,
            TenantDataEvidence::Counted {
                resources: 5,
                basis: CountBasis::CurrentPointersInclTombstones,
            }
        );
    }

    #[test]
    fn default_request_has_no_budget_and_no_resume_point() {
        let request = DiscoveryRequest::default();
        assert_eq!(request.max_requests, None);
        assert_eq!(request.resume, None);
    }

    #[test]
    fn cursor_round_trips_its_opaque_token() {
        let cursor = DiscoveryCursor::new("tenant-0999/");
        assert_eq!(cursor.as_str(), "tenant-0999/");
        assert_eq!(cursor, DiscoveryCursor::new(String::from("tenant-0999/")));
    }
}
