//! Which tenants hold data, for the cross-tenant admin endpoints (#1913).
//!
//! `GET /admin/tenants` and `GET /console/metrics/tenants` used to call
//! [`ResourceStorage::count_by_tenant`] on every backend. On S3 that lists
//! every current pointer of every tenant, so the request grew with stored
//! resources and timed out on large stores. Both now read
//! [`ResourceStorage::discover_tenants`] instead, whose S3 cost is one
//! delimiter page per 1,000 tenant groups plus one `MaxKeys=1` probe per group.
//!
//! What the discovery proves decides the response shape:
//!
//! - [`TenantInventory::Counted`]: an exact live-resource count per tenant
//!   (SQLite, PostgreSQL, MongoDB, and composites over them). The endpoints
//!   return the numeric payload they always did, unchanged.
//! - [`TenantInventory::Presence`]: data presence without a number (S3
//!   prefix-per-tenant). Rows carry `resources: null` and `has_data`, and the
//!   response carries `resources_evidence: "presence"`.
//! - [`TenantInventory::Unsupported`]: the backend cannot discover data-only
//!   tenants (S3 bucket-per-tenant). Data is unknown, never empty:
//!   `resources: null`, `has_data: null`, `resources_evidence: "unsupported"`.
//!
//! A presence finding is never turned into a number, and an S3 pointer count
//! is never reported as a live total.

use std::collections::BTreeSet;

use helios_persistence::core::{
    CountBasis, DiscoveryCoverage, DiscoveryRequest, ResourceStorage, TenantDataEvidence,
};
use serde_json::Value;

use crate::error::RestResult;

/// Per-tenant data evidence for one admin response.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TenantInventory {
    /// Exact live-resource counts from a complete discovery. A tenant that is
    /// not listed has a measured zero.
    Counted(Vec<(String, u64)>),
    /// Tenants proven to hold data, without counts. Raw ids, unfiltered,
    /// deduplicated and sorted.
    Presence {
        /// Ids with data.
        holding: BTreeSet<String>,
        /// Whether every tenant with data is listed. If not, an id outside
        /// `holding` is unknown rather than empty.
        complete: bool,
    },
    /// The backend cannot discover which tenants hold data.
    Unsupported,
}

impl TenantInventory {
    /// Discovers every tenant with data, resuming a budget-limited
    /// [`DiscoveryCoverage::Partial`] answer until it is complete or cannot
    /// advance. No request budget is set, so S3 walks its tenant groups to
    /// completion in one call; the loop is for backends that still slice.
    pub(crate) async fn discover<S>(storage: &S) -> RestResult<Self>
    where
        S: ResourceStorage + ?Sized,
    {
        let mut request = DiscoveryRequest::default();
        let mut found = Vec::new();
        let complete = loop {
            let slice = storage.discover_tenants(&request).await?;
            found.extend(slice.tenants);
            match slice.coverage {
                DiscoveryCoverage::Complete => break true,
                DiscoveryCoverage::Unsupported { .. } => return Ok(Self::Unsupported),
                DiscoveryCoverage::Partial {
                    resume: Some(cursor),
                } if request.resume.as_ref() != Some(&cursor) => request.resume = Some(cursor),
                // Not resumable, or no progress since the last slice.
                _ => break false,
            }
        };

        let all_live_counts = found.iter().all(|tenant| {
            matches!(
                tenant.evidence,
                TenantDataEvidence::Counted {
                    basis: CountBasis::LiveResources,
                    ..
                }
            )
        });
        if complete && all_live_counts {
            return Ok(Self::Counted(
                found
                    .into_iter()
                    .filter_map(|tenant| match tenant.evidence {
                        TenantDataEvidence::Counted { resources, .. } => {
                            Some((tenant.id, resources))
                        }
                        TenantDataEvidence::Present { .. } => None,
                    })
                    .collect(),
            ));
        }

        // Presence, or a count with any other basis (S3 pointers including
        // tombstones, search-index documents): neither is a live total, so
        // only the fact that data exists is reported.
        Ok(Self::Presence {
            holding: found.into_iter().map(|tenant| tenant.id).collect(),
            complete,
        })
    }

    /// The `resources_evidence` value of a response whose rows carry no
    /// counts, or `None` for [`Self::Counted`], whose payload is unchanged and
    /// has no such field.
    pub(crate) fn evidence_label(&self) -> Option<&'static str> {
        match self {
            Self::Counted(_) => None,
            Self::Presence { .. } => Some("presence"),
            Self::Unsupported => Some("unsupported"),
        }
    }

    /// The `has_data` value of a row in an uncounted response: `true` when
    /// discovery found data for `id`, `false` when a complete discovery did
    /// not, `null` when unknown.
    pub(crate) fn has_data(&self, id: &str) -> Value {
        match self {
            Self::Presence { holding, .. } if holding.contains(id) => Value::Bool(true),
            Self::Presence { complete: true, .. } => Value::Bool(false),
            Self::Counted(_) | Self::Presence { .. } | Self::Unsupported => Value::Null,
        }
    }

    /// Ids proven to hold data in an uncounted response, sorted.
    pub(crate) fn holding(&self) -> impl Iterator<Item = &str> {
        let holding = match self {
            Self::Presence { holding, .. } => Some(holding),
            Self::Counted(_) | Self::Unsupported => None,
        };
        holding.into_iter().flatten().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use helios_fhir::FhirVersion;
    use helios_persistence::core::{
        DiscoveredTenant, DiscoveryCursor, PresenceBasis, TenantDiscovery,
    };
    use helios_persistence::error::StorageResult;
    use helios_persistence::tenant::TenantContext;
    use helios_persistence::types::StoredResource;

    /// Answers `discover_tenants` from a script; nothing else is used.
    struct Scripted {
        slices: Vec<TenantDiscovery>,
        calls: Mutex<usize>,
    }

    impl Scripted {
        fn new(slices: Vec<TenantDiscovery>) -> Self {
            Self {
                slices,
                calls: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl ResourceStorage for Scripted {
        fn backend_name(&self) -> &'static str {
            "scripted"
        }

        async fn discover_tenants(
            &self,
            _req: &DiscoveryRequest,
        ) -> StorageResult<TenantDiscovery> {
            let mut calls = self.calls.lock().unwrap();
            let slice = self.slices[(*calls).min(self.slices.len() - 1)].clone();
            *calls += 1;
            Ok(slice)
        }

        async fn create(
            &self,
            _: &TenantContext,
            _: &str,
            _: Value,
            _: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _: &TenantContext,
            _: &str,
            _: &str,
            _: Value,
            _: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn read(
            &self,
            _: &TenantContext,
            _: &str,
            _: &str,
        ) -> StorageResult<Option<StoredResource>> {
            unimplemented!()
        }

        async fn update(
            &self,
            _: &TenantContext,
            _: &StoredResource,
            _: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(&self, _: &TenantContext, _: &str, _: &str) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(&self, _: &TenantContext, _: Option<&str>) -> StorageResult<u64> {
            unimplemented!()
        }
    }

    fn counted(id: &str, resources: u64, basis: CountBasis) -> DiscoveredTenant {
        DiscoveredTenant {
            id: id.to_string(),
            evidence: TenantDataEvidence::Counted { resources, basis },
        }
    }

    fn present(id: &str) -> DiscoveredTenant {
        DiscoveredTenant {
            id: id.to_string(),
            evidence: TenantDataEvidence::Present {
                basis: PresenceBasis::ResourceObjects,
            },
        }
    }

    fn slice(tenants: Vec<DiscoveredTenant>, coverage: DiscoveryCoverage) -> TenantDiscovery {
        TenantDiscovery { tenants, coverage }
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    async fn inventory(slices: Vec<TenantDiscovery>) -> (TenantInventory, usize) {
        let storage = Scripted::new(slices);
        let inventory = TenantInventory::discover(&storage).await.unwrap();
        let calls = *storage.calls.lock().unwrap();
        (inventory, calls)
    }

    #[tokio::test]
    async fn complete_live_counts_are_kept_as_numbers() {
        let (inv, _) = inventory(vec![TenantDiscovery::from_grouped_counts(
            vec![("a".into(), 3), ("b".into(), 1)],
            CountBasis::LiveResources,
        )])
        .await;
        assert_eq!(
            inv,
            TenantInventory::Counted(vec![("a".into(), 3), ("b".into(), 1)])
        );
        assert_eq!(inv.evidence_label(), None);
    }

    /// An empty complete discovery cannot say which basis it would have used.
    /// Every tenant then has a measured zero under either basis, so the counted
    /// shape is honest.
    #[tokio::test]
    async fn empty_complete_discovery_is_a_measured_zero() {
        let (inv, _) = inventory(vec![slice(Vec::new(), DiscoveryCoverage::Complete)]).await;
        assert_eq!(inv, TenantInventory::Counted(Vec::new()));
    }

    #[tokio::test]
    async fn presence_is_never_turned_into_a_number() {
        let (inv, _) = inventory(vec![slice(
            vec![present("b"), present("a")],
            DiscoveryCoverage::Complete,
        )])
        .await;
        assert_eq!(
            inv,
            TenantInventory::Presence {
                holding: set(&["a", "b"]),
                complete: true,
            }
        );
        assert_eq!(inv.evidence_label(), Some("presence"));
        assert_eq!(inv.has_data("a"), Value::Bool(true));
        assert_eq!(inv.has_data("z"), Value::Bool(false));
        assert_eq!(inv.holding().collect::<Vec<_>>(), ["a", "b"]);
    }

    /// Pointer counts include tombstones and index counts can lag the primary:
    /// neither is a live total, so both degrade to presence.
    #[tokio::test]
    async fn non_live_counts_are_reported_as_presence() {
        for basis in [
            CountBasis::CurrentPointersInclTombstones,
            CountBasis::IndexedLiveDocuments,
        ] {
            let (inv, _) = inventory(vec![slice(
                vec![counted("a", 7, basis)],
                DiscoveryCoverage::Complete,
            )])
            .await;
            assert_eq!(
                inv,
                TenantInventory::Presence {
                    holding: set(&["a"]),
                    complete: true,
                },
                "{basis:?}"
            );
        }
    }

    #[tokio::test]
    async fn unsupported_is_unknown_not_empty() {
        let (inv, _) = inventory(vec![TenantDiscovery::unsupported("x")]).await;
        assert_eq!(inv, TenantInventory::Unsupported);
        assert_eq!(inv.evidence_label(), Some("unsupported"));
        assert_eq!(inv.has_data("a"), Value::Null);
        assert_eq!(inv.holding().count(), 0);
    }

    #[tokio::test]
    async fn partial_slices_are_resumed_and_deduplicated() {
        let resume = |c: &str| DiscoveryCoverage::Partial {
            resume: Some(DiscoveryCursor::new(c)),
        };
        let (inv, calls) = inventory(vec![
            slice(vec![present("a")], resume("a/")),
            slice(vec![present("a"), present("b")], resume("b/")),
            slice(vec![present("c")], DiscoveryCoverage::Complete),
        ])
        .await;
        assert_eq!(calls, 3);
        assert_eq!(
            inv,
            TenantInventory::Presence {
                holding: set(&["a", "b", "c"]),
                complete: true,
            }
        );
    }

    /// Live counts that do not cover every tenant are not a complete count:
    /// absent tenants would read as zero.
    #[tokio::test]
    async fn incomplete_counts_degrade_to_unknown_presence() {
        let (inv, calls) = inventory(vec![slice(
            vec![counted("a", 2, CountBasis::LiveResources)],
            DiscoveryCoverage::Partial { resume: None },
        )])
        .await;
        assert_eq!(calls, 1);
        assert_eq!(
            inv,
            TenantInventory::Presence {
                holding: set(&["a"]),
                complete: false,
            }
        );
        assert_eq!(inv.has_data("z"), Value::Null);
    }

    #[tokio::test]
    async fn a_cursor_that_does_not_advance_stops_the_walk() {
        let stuck = DiscoveryCoverage::Partial {
            resume: Some(DiscoveryCursor::new("a/")),
        };
        let (inv, calls) = inventory(vec![slice(vec![present("a")], stuck)]).await;
        assert_eq!(calls, 2);
        assert_eq!(
            inv,
            TenantInventory::Presence {
                holding: set(&["a"]),
                complete: false,
            }
        );
    }
}
