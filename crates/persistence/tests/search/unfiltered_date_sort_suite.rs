//! Backend-agnostic parity suite for the unfiltered `_sort=date` /
//! `_sort=-date` (issue #2731).
//!
//! An unfiltered sort by a date search parameter is served by an ordered walk
//! of the date index on the SQL backends. The general path (a sort key
//! computed per resource of the type) stays as the fallback, and a query that
//! sets `fhir_version` always takes it. So this suite runs every request twice,
//! once as the walk would see it (`fhir_version` unset) and once forced onto
//! the general path (`fhir_version` set to the version the data was seeded
//! with), and requires the two to agree on ids, order, `has_next`,
//! `has_previous`, cursors and the total.
//!
//! The data has what the walk has to settle, in two scenarios. Observations:
//! points, a Period (a range), a Period with no end and resources without a
//! value. MedicationRequests: resources with several `date` rows (one per
//! `dosageInstruction.timing.event`). Both have a deleted resource, a deleted
//! and recreated one, a tie group larger than any walk batch, and another
//! tenant's resources interleaved in date.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `total_mode_suite.rs`. The backend must be built with the spec search
//! parameters loaded; the positive controls fail loudly if it was not.

#![allow(dead_code)]

use std::collections::BTreeSet;

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider, SearchResult};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchQuery, SortDirection, SortDirective, TotalMode,
};

use crate::seeding::SeedingStorage;

/// Resources that share one instant, more than any walk batch reads at once.
const TIE_GROUP: usize = 40;

/// Which path a request is meant to take.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Path {
    /// `fhir_version` unset: eligible for the ordered date-index walk.
    Walk,
    /// `fhir_version` set: always the general path.
    General,
}

/// One page as the suite compares it.
#[derive(Debug, PartialEq, Eq)]
struct PageView {
    ids: Vec<String>,
    has_next: bool,
    has_previous: bool,
    next_cursor: Option<String>,
    previous_cursor: Option<String>,
}

impl PageView {
    fn of(result: &SearchResult) -> Self {
        Self {
            ids: result
                .resources
                .items
                .iter()
                .map(|r| r.id().to_string())
                .collect(),
            has_next: result.has_next(),
            has_previous: result.has_previous(),
            next_cursor: result.next_cursor().cloned(),
            previous_cursor: result.previous_cursor().cloned(),
        }
    }
}

/// What differs between the two scenarios: the resource type, how a resource
/// carries its date values, and the resources with a known place in the order.
struct Scenario {
    resource_type: &'static str,
    /// (id, date values) of the dated resources: each value is a point.
    dated: Vec<(String, Value)>,
    /// Ids of resources with no date value.
    undated: [&'static str; 3],
    /// The resource with the lowest / highest key in ascending order.
    lowest: &'static str,
    highest: &'static str,
    /// An id with a date, to delete and recreate with another date.
    recreated: &'static str,
}

fn body(resource_type: &str, id: &str, dates: &Value) -> Value {
    match resource_type {
        "Observation" => {
            let mut resource = json!({
                "resourceType": "Observation",
                "id": id,
                "status": "final",
                "code": {"text": "unfiltered-date-sort"},
            });
            if let (Some(extra), Some(map)) = (dates.as_object(), resource.as_object_mut()) {
                map.extend(extra.clone());
            }
            resource
        }
        _ => {
            // `dates` is an array of instants, one dosage instruction each.
            let dosage: Vec<Value> = dates
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|d| json!({"timing": {"event": [d]}}))
                .collect();
            json!({
                "resourceType": "MedicationRequest",
                "id": id,
                "status": "active",
                "intent": "order",
                "medicationCodeableConcept": {"text": "unfiltered-date-sort"},
                "subject": {"reference": "Patient/ud-patient"},
                "dosageInstruction": dosage,
            })
        }
    }
}

fn at(instant: &str) -> Value {
    json!({"effectiveDateTime": instant})
}

fn period(start: Option<&str>, end: Option<&str>) -> Value {
    let mut period = serde_json::Map::new();
    if let Some(start) = start {
        period.insert("start".into(), json!(start));
    }
    if let Some(end) = end {
        period.insert("end".into(), json!(end));
    }
    json!({"effectivePeriod": period})
}

fn observations() -> Scenario {
    let mut dated: Vec<(String, Value)> = vec![
        ("ud-earliest".into(), at("1990-01-01T00:00:00Z")),
        ("ud-latest".into(), at("2030-01-01T00:00:00Z")),
        ("ud-mid-a".into(), at("2020-06-15T10:00:00Z")),
        ("ud-mid-b".into(), at("2020-06-15T10:00:00Z")),
        // A range: ascending reads its start, descending its end.
        (
            "ud-period".into(),
            period(Some("2019-01-01T00:00:00Z"), Some("2024-01-01T00:00:00Z")),
        ),
        // No end: no end value to sort descending by.
        (
            "ud-period-open".into(),
            period(Some("2021-03-01T00:00:00Z"), None),
        ),
        (
            "ud-period-wide".into(),
            period(Some("2000-01-01T00:00:00Z"), Some("2029-01-01T00:00:00Z")),
        ),
    ];
    dated.extend((0..TIE_GROUP).map(|i| (format!("ud-tie-{i:03}"), at("2018-08-08T08:08:08Z"))));
    Scenario {
        resource_type: "Observation",
        dated,
        undated: ["ud-nodate-a", "ud-nodate-b", "ud-nodate-c"],
        lowest: "ud-earliest",
        highest: "ud-latest",
        recreated: "ud-recreated",
    }
}

fn medication_requests() -> Scenario {
    let mut dated: Vec<(String, Value)> = vec![
        ("ud-earliest".into(), json!(["1990-01-01T00:00:00Z"])),
        ("ud-latest".into(), json!(["2030-01-01T00:00:00Z"])),
        // Several `date` rows each: the key is the earliest / latest of them.
        (
            "ud-multi".into(),
            json!([
                "2005-05-05T00:00:00Z",
                "2022-02-02T00:00:00Z",
                "2015-03-03T00:00:00Z"
            ]),
        ),
        (
            "ud-multi-late".into(),
            json!(["2026-01-01T00:00:00Z", "2010-01-01T00:00:00Z"]),
        ),
        // Same earliest value as `ud-multi`'s latest, to tie across rows.
        (
            "ud-multi-tie".into(),
            json!(["2022-02-02T00:00:00Z", "2022-02-02T00:00:00Z"]),
        ),
    ];
    dated.extend((0..TIE_GROUP).map(|i| {
        (
            format!("ud-tie-{i:03}"),
            json!(["2018-08-08T08:08:08Z", "2018-08-08T08:08:08Z"]),
        )
    }));
    Scenario {
        resource_type: "MedicationRequest",
        dated,
        undated: ["ud-nodate-a", "ud-nodate-b", "ud-nodate-c"],
        lowest: "ud-earliest",
        highest: "ud-latest",
        recreated: "ud-recreated",
    }
}

fn sort(direction: SortDirection) -> SortDirective {
    SortDirective {
        parameter: "date".to_string(),
        direction,
        param_type: Some(SearchParamType::Date),
    }
}

fn query(rt: &str, direction: SortDirection, path: Path, count: u32) -> SearchQuery {
    let mut query = SearchQuery::new(rt)
        .with_sort(sort(direction))
        .with_count(count);
    if path == Path::General {
        query.fhir_version = Some(FhirVersion::default());
    }
    query
}

async fn run<S>(backend: &S, tenant: &TenantContext, query: &SearchQuery) -> SearchResult
where
    S: ResourceStorage + SearchProvider,
{
    backend.search(tenant, query).await.expect("search")
}

/// One date value, in the shape the scenario's resource type carries it.
fn point(rt: &str, instant: &str) -> Value {
    if rt == "Observation" {
        at(instant)
    } else {
        json!([instant])
    }
}

/// Every live id the seeding leaves in the tenant.
fn expected_live(sc: &Scenario) -> BTreeSet<String> {
    let mut ids: BTreeSet<String> = sc.dated.iter().map(|(id, _)| id.clone()).collect();
    ids.extend(sc.undated.iter().map(|id| id.to_string()));
    ids.insert(sc.recreated.to_string());
    ids
}

async fn seed<S>(backend: &S, sc: &Scenario, tenant: &TenantContext, other: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    let v = FhirVersion::default();
    let rt = sc.resource_type;
    let create = |t: &TenantContext, id: &str, dates: Value| {
        let resource = body(rt, id, &dates);
        let t = t.clone();
        async move {
            backend
                .seed_create(&t, rt, resource, v)
                .await
                .expect("seed resource");
        }
    };

    for (id, dates) in &sc.dated {
        create(tenant, id, dates.clone()).await;
    }
    // No value at all: last in either direction, in id order. Created out of
    // order on purpose.
    for id in [sc.undated[2], sc.undated[0], sc.undated[1]] {
        create(tenant, id, Value::Null).await;
    }

    // Deleted: must not show up, and must not shorten a page.
    for (id, instant) in [
        ("ud-gone-late", "2031-01-01T00:00:00Z"),
        ("ud-gone-early", "1980-01-01T00:00:00Z"),
    ] {
        create(tenant, id, point(rt, instant)).await;
        backend
            .seed_delete(tenant, rt, id)
            .await
            .expect("delete resource");
    }

    // Deleted and recreated with another date: only the new one counts.
    create(tenant, sc.recreated, point(rt, "1999-09-09T00:00:00Z")).await;
    backend
        .seed_delete(tenant, rt, sc.recreated)
        .await
        .expect("delete resource to recreate");
    backend
        .seed_put(
            tenant,
            rt,
            sc.recreated,
            body(rt, sc.recreated, &point(rt, "2027-07-07T00:00:00Z")),
            v,
        )
        .await
        .expect("recreate resource");

    // Another tenant, interleaved in date with the first one's.
    for (id, instant) in [
        ("other-first", "1970-01-01T00:00:00Z"),
        ("other-mid", "2020-06-15T10:00:00Z"),
        ("other-tie", "2018-08-08T08:08:08Z"),
        ("other-last", "2040-01-01T00:00:00Z"),
    ] {
        create(other, id, point(rt, instant)).await;
    }
    create(other, "other-nodate", Value::Null).await;
}

/// The full result list of a direction, read in one page on the general path.
async fn full_order<S>(
    backend: &S,
    rt: &str,
    tenant: &TenantContext,
    direction: SortDirection,
    path: Path,
) -> Vec<String>
where
    S: ResourceStorage + SearchProvider,
{
    PageView::of(&run(backend, tenant, &query(rt, direction, path, 500)).await).ids
}

/// Walks forward by cursor from the first page, `path_for(n)` choosing the
/// path of the n-th request, and returns every page.
async fn walk_forward<S>(
    backend: &S,
    rt: &str,
    tenant: &TenantContext,
    direction: SortDirection,
    count: u32,
    path_for: impl Fn(usize) -> Path,
) -> Vec<PageView>
where
    S: ResourceStorage + SearchProvider,
{
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    for n in 0.. {
        assert!(n < 500, "forward walk does not terminate");
        let mut q = query(rt, direction, path_for(n), count);
        q.cursor = cursor.clone();
        let page = PageView::of(&run(backend, tenant, &q).await);
        cursor = page.next_cursor.clone();
        let more = page.has_next;
        pages.push(page);
        if !more {
            break;
        }
        assert!(cursor.is_some(), "a next page without a cursor");
    }
    pages
}

/// Walks backward by `previous_cursor` from `start`, returning the pages in
/// the order they were read (nearest first).
async fn walk_backward<S>(
    backend: &S,
    rt: &str,
    tenant: &TenantContext,
    direction: SortDirection,
    count: u32,
    start: &PageView,
    path_for: impl Fn(usize) -> Path,
) -> Vec<PageView>
where
    S: ResourceStorage + SearchProvider,
{
    let mut pages = Vec::new();
    let mut cursor = start.previous_cursor.clone();
    for n in 0.. {
        assert!(n < 500, "backward walk does not terminate");
        let Some(current) = cursor.take() else { break };
        let mut q = query(rt, direction, path_for(n), count);
        q.cursor = Some(current);
        let page = PageView::of(&run(backend, tenant, &q).await);
        cursor = page.previous_cursor.clone();
        let more = page.has_previous;
        pages.push(page);
        if !more {
            break;
        }
    }
    pages
}

fn flatten(pages: &[PageView]) -> Vec<String> {
    pages.iter().flat_map(|p| p.ids.clone()).collect()
}

fn assert_no_repeats(ids: &[String], what: &str) {
    let unique: BTreeSet<&String> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "{what}: an id repeats: {ids:?}");
}

/// Seeds both scenarios, then checks the walk against the general path.
pub async fn unfiltered_date_sort_matches_the_general_path<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    for (suffix, sc) in [("obs", observations()), ("medreq", medication_requests())] {
        check_scenario(backend, &format!("{tenant_base}-{suffix}"), &sc).await;
    }
}

async fn check_scenario<S>(backend: &S, tenant_base: &str, sc: &Scenario)
where
    S: ResourceStorage + SearchProvider,
{
    let rt = sc.resource_type;
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    let other = TenantContext::new(
        TenantId::new(format!("{tenant_base}-other")),
        TenantPermissions::full_access(),
    );
    seed(backend, sc, &tenant, &other).await;

    let live = expected_live(sc);
    let nodate: Vec<String> = sc.undated.iter().map(|id| id.to_string()).collect();

    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let label = format!("{rt} {direction:?}");

        // --- Positive controls, on the general path -------------------------
        let general = full_order(backend, rt, &tenant, direction, Path::General).await;
        assert_eq!(
            general.iter().cloned().collect::<BTreeSet<_>>(),
            live,
            "{label}: the live set (deleted and other-tenant ids stay out)"
        );
        assert_eq!(general.len(), live.len(), "{label}: no id twice");
        // A Period with no end has no end value to sort descending by, so it
        // may sit with the undated resources there; leave it out of the
        // controls on the border.
        let firm: Vec<String> = general
            .iter()
            .filter(|id| id.as_str() != "ud-period-open")
            .cloned()
            .collect();
        assert_eq!(
            &firm[firm.len() - nodate.len()..],
            nodate.as_slice(),
            "{label}: resources without a date come last, in id order"
        );
        let dated = &firm[..firm.len() - nodate.len()];
        let (first, last) = match direction {
            SortDirection::Ascending => (sc.lowest, sc.highest),
            SortDirection::Descending => (sc.highest, sc.lowest),
        };
        // The recreated resource is at 2027, so a deleted 1999 or 1980 and a
        // deleted 2031 are not in the way of the ends.
        assert_eq!(dated.first().map(String::as_str), Some(first), "{label}");
        assert_eq!(dated.last().map(String::as_str), Some(last), "{label}");

        // The same list from the walk, in one page.
        let walked = full_order(backend, rt, &tenant, direction, Path::Walk).await;
        assert_eq!(walked, general, "{label}: the walk's order");

        // --- Page by page, by cursor ----------------------------------------
        for count in [1u32, 2, 7] {
            let tag = format!("{label} count={count}");
            let on_general =
                walk_forward(backend, rt, &tenant, direction, count, |_| Path::General).await;
            let on_walk =
                walk_forward(backend, rt, &tenant, direction, count, |_| Path::Walk).await;
            assert_eq!(on_walk, on_general, "{tag}: forward cursor pages");
            assert_eq!(
                flatten(&on_walk),
                general,
                "{tag}: pages add up to the list"
            );
            assert_no_repeats(&flatten(&on_walk), &tag);

            // A cursor minted by one path continues on the other.
            for flipped in [false, true] {
                let mixed = walk_forward(backend, rt, &tenant, direction, count, |n| {
                    if (n % 2 == 0) != flipped {
                        Path::Walk
                    } else {
                        Path::General
                    }
                })
                .await;
                assert_eq!(mixed, on_general, "{tag}: cursors are interchangeable");
            }

            // Backward from the last page: both paths, then mixed.
            let last_page = on_general.last().expect("at least one page");
            let back_general =
                walk_backward(backend, rt, &tenant, direction, count, last_page, |_| {
                    Path::General
                })
                .await;
            let back_walk =
                walk_backward(backend, rt, &tenant, direction, count, last_page, |_| {
                    Path::Walk
                })
                .await;
            assert_eq!(back_walk, back_general, "{tag}: backward cursor pages");
            let back_mixed =
                walk_backward(backend, rt, &tenant, direction, count, last_page, |n| {
                    if n % 2 == 0 {
                        Path::Walk
                    } else {
                        Path::General
                    }
                })
                .await;
            assert_eq!(back_mixed, back_general, "{tag}: backward, mixed paths");

            // The last page plus what the backward walk read covers every
            // resource exactly once.
            let mut seen = last_page.ids.clone();
            seen.extend(flatten(&back_walk));
            assert_no_repeats(&seen, &format!("{tag} backward"));
            assert_eq!(
                seen.into_iter().collect::<BTreeSet<_>>(),
                live,
                "{tag}: backward walk leaves no gap"
            );
            assert!(
                !back_walk.last().is_some_and(|p| p.has_previous),
                "{tag}: the backward walk reached the start"
            );
        }

        // --- _offset, across the dated/undated border and past the end ------
        for count in [1u32, 7] {
            for offset in 0..(live.len() as u32 + 3) {
                let mut walk = query(rt, direction, Path::Walk, count);
                walk.offset = Some(offset);
                let mut on_general = query(rt, direction, Path::General, count);
                on_general.offset = Some(offset);
                let walk = PageView::of(&run(backend, &tenant, &walk).await);
                let on_general = PageView::of(&run(backend, &tenant, &on_general).await);
                assert_eq!(
                    walk, on_general,
                    "{label} count={count} offset={offset}: offset page"
                );
            }
        }
    }

    // --- The total: the same on both paths, and the count of live resources --
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        for (mode, expected) in [
            (None, None),
            (Some(TotalMode::None), None),
            (Some(TotalMode::Accurate), Some(live.len() as u64)),
            (Some(TotalMode::Estimate), Some(live.len() as u64)),
        ] {
            for path in [Path::Walk, Path::General] {
                let mut q = query(rt, direction, path, 7);
                q.total = mode;
                let result = run(backend, &tenant, &q).await;
                assert_eq!(
                    result.total, expected,
                    "{rt} {direction:?} {path:?} _total {mode:?}"
                );
                assert_eq!(result.resources.page_info.total, expected);
            }
        }
    }
}
