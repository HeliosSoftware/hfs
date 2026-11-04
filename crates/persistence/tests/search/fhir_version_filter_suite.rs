//! Backend-agnostic FHIR version filter suite (issue #2254).
//!
//! A `SearchQuery` naming a `fhir_version` matches only the resources stored
//! under that version — on every pagination path (first page, offset, cursor
//! next and previous), in the count and `_total`, in `search_ids`, in
//! `search_reference_values`, and under `_contained`. A query naming none
//! keeps the version-blind search every backend ran before.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `total_mode_suite.rs`. Needs the `R4` and `R5` features: the same type is
//! stored under both versions.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ContainedMode, SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
    TotalMode,
};

/// R4 Patients, interleaved in creation (and so `_lastUpdated`) order with
/// the R5 ones, so a page that ignored the filter would mix them.
const R4_IDS: [&str; 5] = ["v4-0", "v4-1", "v4-2", "v4-3", "v4-4"];
const R5_IDS: [&str; 3] = ["v5-0", "v5-1", "v5-2"];

fn ids(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|id| id.to_string()).collect()
}

fn female() -> SearchParameter {
    SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        values: vec![SearchValue::new(SearchPrefix::Eq, "female")],
        ..Default::default()
    }
}

fn patients(version: Option<FhirVersion>) -> SearchQuery {
    let mut query = SearchQuery::new("Patient");
    query.fhir_version = version;
    query
}

async fn search_ids<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> Vec<String>
where
    S: SearchProvider,
{
    backend
        .search(tenant, query)
        .await
        .expect("search")
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

/// Walks every page of `query` (page size `count`) by cursor, returning the
/// ids in page order and the page sizes; then walks back from the last page
/// by its previous cursor and checks it reproduces the earlier pages.
async fn walk_cursor<S>(
    backend: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
    count: u32,
) -> (Vec<String>, Vec<usize>)
where
    S: SearchProvider,
{
    let mut pages: Vec<Vec<String>> = Vec::new();
    let mut previous_of_last: Option<String>;
    let mut cursor: Option<String> = None;
    loop {
        let mut page_query = query.clone().with_count(count);
        page_query.cursor = cursor.clone();
        let page = backend
            .search(tenant, &page_query)
            .await
            .expect("cursor page")
            .resources;
        let page_ids: Vec<String> = page.items.iter().map(|r| r.id().to_string()).collect();
        assert!(
            page.items
                .iter()
                .all(|r| query.matches_fhir_version(r.fhir_version())),
            "a cursor page holds a resource of another version: {page_ids:?}"
        );
        pages.push(page_ids);
        previous_of_last = page.page_info.previous_cursor.clone();
        match page.page_info.next_cursor {
            Some(next) if page.page_info.has_next => cursor = Some(next),
            _ => break,
        }
        assert!(pages.len() < 20, "cursor paging does not terminate");
    }

    // Backward from the last page: each previous page is the one before it.
    if pages.len() > 1 {
        let mut cursor = previous_of_last.expect("the last page has a previous cursor");
        for expected in pages.iter().rev().skip(1) {
            let mut page_query = query.clone().with_count(count);
            page_query.cursor = Some(cursor.clone());
            let page = backend
                .search(tenant, &page_query)
                .await
                .expect("previous page")
                .resources;
            let got: Vec<String> = page.items.iter().map(|r| r.id().to_string()).collect();
            assert_eq!(&got, expected, "previous-cursor page differs");
            match page.page_info.previous_cursor {
                Some(previous) => cursor = previous,
                None => break,
            }
        }
    }

    let sizes = pages.iter().map(Vec::len).collect();
    (pages.into_iter().flatten().collect(), sizes)
}

/// Seeds Patients under R4 and R5 (plus one R4 male), Observations with
/// subjects under both, and `_contained` containers under both, then holds
/// every search path to the version filter.
pub async fn search_honors_fhir_version<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    let (r4, r5) = (FhirVersion::R4, FhirVersion::R5);

    // Interleave: v4-0, v5-0, v4-1, v5-1, v4-2, v5-2, v4-3, v4-4.
    let mut order: Vec<(&str, FhirVersion)> = Vec::new();
    for (i, id) in R4_IDS.iter().enumerate() {
        order.push((id, r4));
        if let Some(id) = R5_IDS.get(i) {
            order.push((id, r5));
        }
    }
    for (id, version) in &order {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": id, "gender": "female"}),
                *version,
            )
            .await
            .expect("seed patient");
        // Distinct `_lastUpdated` values keep the page order unambiguous.
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "id": "v4-male", "gender": "male"}),
            r4,
        )
        .await
        .expect("seed male patient");

    let all_r4: BTreeSet<String> = ids(&R4_IDS).into_iter().chain(["v4-male".into()]).collect();
    let all_r5 = ids(&R5_IDS);

    // Wait for eventually-consistent indexes on the version-blind control.
    let mut blind = BTreeSet::new();
    for _ in 0..50 {
        let mut q = patients(None);
        q.parameters.push(female());
        blind = search_ids(backend, &tenant, &q).await.into_iter().collect();
        if blind.len() == R4_IDS.len() + R5_IDS.len() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // ---- None: version-blind, as before -------------------------------
    let expected_blind_female: BTreeSet<String> = ids(&R4_IDS).union(&all_r5).cloned().collect();
    assert_eq!(
        blind, expected_blind_female,
        "version-blind filtered search"
    );
    let everything: BTreeSet<String> = all_r4.union(&all_r5).cloned().collect();
    let got: BTreeSet<String> = search_ids(backend, &tenant, &patients(None))
        .await
        .into_iter()
        .collect();
    assert_eq!(got, everything, "version-blind unfiltered search");
    assert_eq!(
        backend
            .search_count(&tenant, &patients(None))
            .await
            .unwrap(),
        everything.len() as u64,
        "version-blind count"
    );

    // ---- First page, unfiltered and filtered --------------------------
    for (version, expected, expected_female) in
        [(r4, &all_r4, ids(&R4_IDS)), (r5, &all_r5, all_r5.clone())]
    {
        let q = patients(Some(version));
        let got: BTreeSet<String> = search_ids(backend, &tenant, &q).await.into_iter().collect();
        assert_eq!(&got, expected, "{version:?}: first page");
        assert_eq!(
            backend.search_count(&tenant, &q).await.unwrap(),
            expected.len() as u64,
            "{version:?}: count"
        );
        let mut with_total = q.clone();
        with_total.total = Some(TotalMode::Accurate);
        assert_eq!(
            backend.search(&tenant, &with_total).await.unwrap().total,
            Some(expected.len() as u64),
            "{version:?}: _total=accurate"
        );

        let mut filtered = q.clone();
        filtered.parameters.push(female());
        let got: BTreeSet<String> = search_ids(backend, &tenant, &filtered)
            .await
            .into_iter()
            .collect();
        assert_eq!(got, expected_female, "{version:?}: gender=female");
        assert_eq!(
            backend.search_count(&tenant, &filtered).await.unwrap(),
            expected_female.len() as u64,
            "{version:?}: gender=female count"
        );

        // A one-row page (SQLite reads it `NOT INDEXED`) and a small one.
        for count in [1u32, 2] {
            let page = backend
                .search(&tenant, &filtered.clone().with_count(count))
                .await
                .unwrap()
                .resources;
            assert_eq!(
                page.items.len(),
                count as usize,
                "{version:?}: _count={count}"
            );
            assert!(
                page.items.iter().all(|r| r.fhir_version() == version),
                "{version:?}: _count={count} page holds another version"
            );
            assert!(
                page.page_info.has_next,
                "{version:?}: _count={count} has next"
            );
        }
    }

    // ---- Cursor paging over a filtered set ----------------------------
    let mut r4_female = patients(Some(r4));
    r4_female.parameters.push(female());
    let (walked, sizes) = walk_cursor(backend, &tenant, &r4_female, 2).await;
    assert_eq!(sizes, vec![2, 2, 1], "R4 female cursor page sizes");
    // Newest first.
    let newest_first: Vec<String> = R4_IDS.iter().rev().map(|s| s.to_string()).collect();
    assert_eq!(walked, newest_first, "R4 female cursor order");

    let (walked, sizes) = walk_cursor(backend, &tenant, &patients(Some(r5)), 2).await;
    assert_eq!(sizes, vec![2, 1], "R5 cursor page sizes");
    let newest_first: Vec<String> = R5_IDS.iter().rev().map(|s| s.to_string()).collect();
    assert_eq!(walked, newest_first, "R5 cursor order");

    // ---- Offset paging over a filtered set ----------------------------
    let mut offset_page = r4_female.clone().with_count(2);
    offset_page.offset = Some(2);
    let got = search_ids(backend, &tenant, &offset_page).await;
    assert_eq!(
        got,
        vec!["v4-2".to_string(), "v4-1".to_string()],
        "R4 offset 2"
    );
    offset_page.offset = Some(4);
    let got = search_ids(backend, &tenant, &offset_page).await;
    assert_eq!(got, vec!["v4-0".to_string()], "R4 offset 4");

    // ---- search_ids (the id-only page) --------------------------------
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let mut q = r4_female.clone().with_count(2);
        q.cursor = cursor;
        let page = backend.search_ids(&tenant, &q).await.expect("search_ids");
        seen.extend(page.items.iter().cloned());
        if !page.page_info.has_next {
            break;
        }
        cursor = page.page_info.next_cursor.clone();
        assert!(seen.len() < 50, "search_ids does not terminate");
    }
    let seen: BTreeSet<String> = seen.into_iter().collect();
    assert_eq!(seen, ids(&R4_IDS), "search_ids under R4");

    // ---- search_reference_values --------------------------------------
    for (id, subject, version) in [("obs-4", "Patient/v4-0", r4), ("obs-5", "Patient/v5-0", r5)] {
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": id,
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "1-1"}]},
                    "subject": {"reference": subject},
                }),
                version,
            )
            .await
            .expect("seed observation");
    }
    let mut obs = SearchQuery::new("Observation");
    obs.parameters.push(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        values: vec![SearchValue::new(SearchPrefix::Eq, "1-1")],
        ..Default::default()
    });
    obs.fhir_version = Some(r4);
    // `None` (the paged fallback) is an allowed answer; a list must be R4's.
    if let Some(values) = backend
        .search_reference_values(&tenant, &obs, "subject")
        .await
        .expect("search_reference_values")
    {
        assert!(
            values.iter().all(|v| !v.contains("v5-0")),
            "R4 reference values include the R5 subject: {values:?}"
        );
        assert!(
            values.iter().any(|v| v.contains("v4-0")),
            "R4 reference values miss the R4 subject: {values:?}"
        );
    }

    // ---- _contained ----------------------------------------------------
    for (id, version) in [("dr-4", r4), ("dr-5", r5)] {
        backend
            .create(
                &tenant,
                "DiagnosticReport",
                json!({
                    "resourceType": "DiagnosticReport",
                    "id": id,
                    "status": "final",
                    "code": {"text": "panel"},
                    "contained": [{
                        "resourceType": "Observation",
                        "id": "inner",
                        "status": "final",
                        "code": {"coding": [{"system": "http://loinc.org", "code": "2-2"}]},
                    }],
                }),
                version,
            )
            .await
            .expect("seed container");
    }
    let contained = |version: Option<FhirVersion>| {
        let mut q = SearchQuery::new("Observation");
        q.contained = ContainedMode::On;
        q.parameters.push(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            values: vec![SearchValue::new(SearchPrefix::Eq, "2-2")],
            ..Default::default()
        });
        q.fhir_version = version;
        q
    };
    let mut blind = BTreeSet::new();
    for _ in 0..50 {
        blind = search_ids(backend, &tenant, &contained(None))
            .await
            .into_iter()
            .collect();
        if blind.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(blind, ids(&["dr-4", "dr-5"]), "version-blind _contained");
    for (version, expected) in [(r4, "dr-4"), (r5, "dr-5")] {
        let q = contained(Some(version));
        let got: BTreeSet<String> = search_ids(backend, &tenant, &q).await.into_iter().collect();
        assert_eq!(got, ids(&[expected]), "{version:?}: _contained");
        assert_eq!(
            backend.search_count(&tenant, &q).await.unwrap(),
            1,
            "{version:?}: _contained count"
        );
    }
}
