//! Backend-agnostic `Resource`-level meta-parameter search suite (#523).
//!
//! `_source`, `_tag`, `_profile` and `_security` are ordinary indexed
//! parameters over `meta.*`, and every backend is supposed to filter on them.
//! Two defects with opposite symptoms have hit this set:
//!
//!  * **Never indexed** (#523) — `_source` is the only `Resource`-level
//!    parameter with no embedded definition, so it kept the spec file's
//!    `Resource.meta.source`. Evaluated against a concrete resource, FHIRPath
//!    matches the leading identifier against the resource's own type and
//!    returns nothing, so no index row was ever written and `_source` matched
//!    **nothing** on every index-backed backend.
//!  * **Dropped at query time** (#474, fixed separately) — SQLite routed all
//!    four to the special-parameter path, which returned `None` for them. A
//!    `None` condition is dropped rather than rejected, so the filter silently
//!    vanished and the search returned **everything**.
//!
//! Both directions therefore have to be asserted, and on more than one engine.
//! A positive-only test passes against a backend that ignores the filter; a
//! negative-only test passes against one that never indexed it; and a
//! single-backend test cannot tell a dropped filter from a correct filter over
//! a missing row. Each parameter is checked both ways, on SQLite and
//! PostgreSQL.
//!
//! Like `fts_purge_suite.rs`, this file is `#[path]`-included by each backend's
//! test binary rather than living in `tests/common/`, which no test target
//! declares and cargo therefore never compiles (issue #306).
//!
//! ## Shared-database backends
//!
//! The PostgreSQL binary runs against one long-lived container database, so
//! callers must pass a **distinct tenant per scenario**. The scenario seeds
//! fixed logical ids within that tenant.

#![allow(dead_code)]

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

/// The resource carrying every meta value.
const CARRIER_ID: &str = "meta-carrier";
/// The resource carrying none of them, which every query must exclude.
const BYSTANDER_ID: &str = "meta-bystander";

const SOURCE: &str = "http://example.org/meta-suite/src";
const PROFILE: &str = "http://example.org/meta-suite/StructureDefinition/tagged";
const TAG_SYSTEM: &str = "http://example.org/meta-suite/tags";
const SECURITY_SYSTEM: &str = "http://example.org/meta-suite/labels";

/// One parameter's matching and non-matching query values.
struct Case {
    name: &'static str,
    param_type: SearchParamType,
    hit: String,
    miss: String,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "_source",
            param_type: SearchParamType::Uri,
            hit: SOURCE.to_string(),
            miss: "http://example.org/meta-suite/other-src".to_string(),
        },
        Case {
            name: "_profile",
            param_type: SearchParamType::Uri,
            hit: PROFILE.to_string(),
            miss: "http://example.org/meta-suite/StructureDefinition/other".to_string(),
        },
        Case {
            name: "_tag",
            param_type: SearchParamType::Token,
            hit: format!("{TAG_SYSTEM}|carried"),
            miss: format!("{TAG_SYSTEM}|absent"),
        },
        Case {
            name: "_security",
            param_type: SearchParamType::Token,
            hit: format!("{SECURITY_SYSTEM}|R"),
            miss: format!("{SECURITY_SYSTEM}|V"),
        },
    ]
}

/// Seeds the carrier and the bystander.
async fn seed<B>(backend: &B, tenant: &TenantContext)
where
    B: ResourceStorage,
{
    backend
        .create(
            tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": CARRIER_ID,
                "meta": {
                    "source": SOURCE,
                    "profile": [PROFILE],
                    "tag": [{"system": TAG_SYSTEM, "code": "carried"}],
                    "security": [{"system": SECURITY_SYSTEM, "code": "R"}]
                },
                "name": [{"family": "Carrier"}]
            }),
            FhirVersion::default(),
        )
        .await
        .expect("seed carrier");

    backend
        .create(
            tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": BYSTANDER_ID,
                "name": [{"family": "Bystander"}]
            }),
            FhirVersion::default(),
        )
        .await
        .expect("seed bystander");
}

fn query(name: &str, param_type: SearchParamType, value: &str) -> SearchQuery {
    SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: name.to_string(),
        param_type,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    })
}

/// Each meta parameter matches the resource that carries the value, and only
/// that resource.
pub async fn meta_parameters_match_only_their_carrier<B>(backend: &B, tenant: &TenantContext)
where
    B: ResourceStorage + SearchProvider,
{
    seed(backend, tenant).await;

    // Positive control: the seeded pair is reachable at all, so a later "0
    // matches" means the filter excluded them rather than the seeding failing.
    let all = backend
        .search(tenant, &SearchQuery::new("Patient"))
        .await
        .expect("unfiltered search");
    assert_eq!(
        all.resources.items.len(),
        2,
        "both seeded Patients should be present before filtering"
    );

    for case in cases() {
        let matched = backend
            .search(tenant, &query(case.name, case.param_type, &case.hit))
            .await
            .unwrap_or_else(|e| panic!("{}={} search failed: {e}", case.name, case.hit));
        let ids: Vec<&str> = matched.resources.items.iter().map(|r| r.id()).collect();
        assert_eq!(
            ids,
            vec![CARRIER_ID],
            "{}={} should match exactly the resource carrying it",
            case.name,
            case.hit
        );

        let unmatched = backend
            .search(tenant, &query(case.name, case.param_type, &case.miss))
            .await
            .unwrap_or_else(|e| panic!("{}={} search failed: {e}", case.name, case.miss));
        let ids: Vec<&str> = unmatched.resources.items.iter().map(|r| r.id()).collect();
        assert!(
            ids.is_empty(),
            "{}={} matches nothing, but the search returned {:?} — \
             a dropped filter reads as 'everything matched'",
            case.name,
            case.miss,
            ids
        );
    }
}
