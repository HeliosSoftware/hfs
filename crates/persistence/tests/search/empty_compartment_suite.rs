//! Backend-agnostic empty compartment membership suite (issue #2509).
//!
//! `SearchQuery::compartment` restricts a search to one compartment
//! (`GET /Patient/{id}/Observation`), and it also carries patient-scope
//! confinement, so it is a security boundary. A membership with no `params`,
//! or with an empty `reference`, used to be read as "no compartment at all" on
//! every backend: the restriction was skipped and the search widened to every
//! resource of the type. The rule every backend now holds to is the one the
//! issue states: a `Some` compartment either constrains the query or makes it
//! fail, and an empty membership matches *nothing* — zero rows, a count of 0,
//! and no error. No resource is a member of a compartment through no
//! parameter, or of one with no reference.
//!
//! This suite drives each backend's *real* [`SearchProvider`] paths — `search`,
//! `search_count`, `search_ids` and `_contained=true|both` — with the three
//! shapes of an empty membership, alone and beside the criteria that route a
//! search down a different path of its own (an ordinary parameter, `:not`,
//! `:missing`, `_sort`, `_total=accurate`, paging). A control with a real
//! membership runs the same shapes and must still return its members: without
//! it, "returns nothing" would pass on a backend that returns nothing for
//! everything.
//!
//! Included by `#[path]` into each backend's test binary, like
//! `empty_value_suite.rs`. The backend must be built with the spec search
//! parameters loaded: the controls refuse to go on until the fixture is
//! searchable by `subject`, `performer`, `code` and `status`.

#![allow(dead_code)]

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    CompartmentMembership, ContainedMode, ContainedReturn, SearchModifier, SearchParamType,
    SearchParameter, SearchQuery, SearchValue, SortDirective, TotalMode,
};

use crate::seeding::SeedingStorage;

/// An Observation of `patient`, optionally performed by another patient.
fn observation(
    id: &str,
    patient: &str,
    performer: Option<&str>,
    status: &str,
    code: &str,
    date: &str,
) -> Value {
    let mut resource = json!({
        "resourceType": "Observation",
        "id": id,
        "status": status,
        "code": {"coding": [{"system": "http://loinc.org", "code": code}]},
        "effectiveDateTime": date,
        "subject": {"reference": format!("Patient/{patient}")},
    });
    if let Some(performer) = performer {
        resource["performer"] = json!([{"reference": format!("Patient/{performer}")}]);
    }
    resource
}

fn token(name: &str, value: &str, modifier: Option<SearchModifier>) -> SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type: SearchParamType::Token,
        modifier,
        values: vec![SearchValue::eq(value)],
        ..Default::default()
    }
}

/// The compartment `GET /Patient/p1/Observation` searches.
fn real_membership() -> CompartmentMembership {
    CompartmentMembership {
        params: vec!["subject".to_string(), "performer".to_string()],
        reference: "Patient/p1".to_string(),
    }
}

/// The three shapes of a membership that matches nothing.
fn empty_memberships() -> Vec<(&'static str, CompartmentMembership)> {
    vec![
        (
            "no params",
            CompartmentMembership {
                params: vec![],
                reference: "Patient/p1".to_string(),
            },
        ),
        (
            "empty reference",
            CompartmentMembership {
                params: vec!["subject".to_string(), "performer".to_string()],
                reference: String::new(),
            },
        ),
        ("neither", CompartmentMembership::default()),
    ]
}

/// Searches of the top-level Observations, none of them with a compartment:
/// each takes a path of its own in at least one backend.
fn top_level_variants() -> Vec<(&'static str, SearchQuery)> {
    let base = || SearchQuery::new("Observation").with_count(100);
    let mut by_date = base();
    by_date.sort.push(SortDirective::parse("date"));
    let mut newest_first = base();
    newest_first
        .sort
        .push(SortDirective::parse("-_lastUpdated"));
    let mut accurate = base();
    accurate.total = Some(TotalMode::Accurate);
    let mut paged = base();
    paged.count = Some(2);
    let mut everything = base()
        .with_parameter(token("code", "X", None))
        .with_parameter(token("status", "amended", Some(SearchModifier::Not)));
    everything.sort.push(SortDirective::parse("date"));
    everything.total = Some(TotalMode::Accurate);
    vec![
        ("alone", base()),
        ("code=X", base().with_parameter(token("code", "X", None))),
        (
            "status:not=amended",
            base().with_parameter(token("status", "amended", Some(SearchModifier::Not))),
        ),
        (
            "status:missing=false",
            base().with_parameter(token("status", "false", Some(SearchModifier::Missing))),
        ),
        ("_sort=date", by_date),
        ("_sort=-_lastUpdated", newest_first),
        ("_total=accurate", accurate),
        ("_count=2", paged),
        (
            "code=X&status:not=amended&_sort=date&_total=accurate",
            everything,
        ),
    ]
}

/// `_contained=true|both` searches, in both `_containedType` forms, each asking
/// for a total.
fn contained_variants() -> Vec<(String, SearchQuery)> {
    let mut variants = Vec::new();
    for (mode_name, mode) in [("true", ContainedMode::On), ("both", ContainedMode::Both)] {
        for (return_name, returned) in [
            ("container", ContainedReturn::Container),
            ("contained", ContainedReturn::Contained),
        ] {
            for (criteria_name, parameters) in [
                ("alone", vec![]),
                ("code=X", vec![token("code", "X", None)]),
            ] {
                let mut query = SearchQuery::new("Observation").with_count(100);
                query.contained = mode;
                query.contained_return = returned;
                query.parameters = parameters;
                query.total = Some(TotalMode::Accurate);
                variants.push((
                    format!("_contained={mode_name}&_containedType={return_name} {criteria_name}"),
                    query,
                ));
            }
        }
    }
    variants
}

/// What one query answered through every read path.
#[derive(Debug, PartialEq)]
struct Seen {
    /// Ids of `search`, sorted, unless the query sorts (then in order).
    ids: Vec<String>,
    total: Option<u64>,
    has_next: bool,
    counted: u64,
    id_page: Vec<String>,
}

async fn observe<S>(
    backend: &S,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> Result<Seen, String>
where
    S: ResourceStorage + SearchProvider,
{
    let found = backend
        .search(tenant, query)
        .await
        .map_err(|e| format!("search: {e}"))?;
    let counted = backend
        .search_count(tenant, query)
        .await
        .map_err(|e| format!("search_count: {e}"))?;
    let id_page = backend
        .search_ids(tenant, query)
        .await
        .map_err(|e| format!("search_ids: {e}"))?;
    let mut ids: Vec<String> = found
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect();
    let mut id_page: Vec<String> = id_page.items;
    // Compared as sets: the sort variants exist to send the search down their
    // own paths, and the fixture's `_lastUpdated` values are not distinct.
    ids.sort();
    id_page.sort();
    Ok(Seen {
        ids,
        total: found.total,
        has_next: found.resources.page_info.has_next,
        counted,
        id_page,
    })
}

/// What a search that matches `expected` must have answered.
fn expected_seen(query: &SearchQuery, expected: &[&str]) -> Seen {
    let n = expected.len() as u64;
    let mut ids: Vec<String> = expected.iter().map(|id| id.to_string()).collect();
    ids.sort();
    Seen {
        id_page: ids.clone(),
        ids,
        total: (query.total == Some(TotalMode::Accurate)).then_some(n),
        has_next: false,
        counted: n,
    }
}

/// Collects one line per scenario, so a failing backend reports the whole
/// table rather than its first row.
#[derive(Default)]
struct Report {
    lines: Vec<String>,
    failures: usize,
}

impl Report {
    fn record(&mut self, scenario: &str, expected: &Seen, actual: Result<Seen, String>) {
        let ok = actual.as_ref() == Ok(expected);
        if !ok {
            self.failures += 1;
        }
        self.lines.push(format!(
            "{} | {scenario} | expected {expected:?} | actual {actual:?}",
            if ok { "ok  " } else { "FAIL" }
        ));
    }

    fn finish(self, what: &str) {
        for line in &self.lines {
            eprintln!("[empty_compartment_suite] {line}");
        }
        assert!(
            self.failures == 0,
            "{what}: {} scenario(s) failed\n{}",
            self.failures,
            self.lines.join("\n")
        );
    }
}

/// Seeds the fixture and waits for the controls (eventually-consistent
/// indexes).
///
/// Top-level Observations:
/// - `ec-p1-final`: subject `Patient/p1`, final, code X, 2024-01-01
/// - `ec-p1-amended`: subject `Patient/p1`, amended, code Y, 2024-01-02
/// - `ec-p2-final`: subject `Patient/p2`, final, code X, 2024-01-03
/// - `ec-perf-p1`: subject `Patient/p2`, performed by `Patient/p1`, final,
///   code Y, 2024-01-04
///
/// Containers (DiagnosticReport → contained Observations):
/// - `ec-dr1`: `c1` (subject `Patient/p1`, code X) and `c2` (`Patient/p2`, Y)
/// - `ec-dr2`: `c3` (subject `Patient/p2`, code X)
async fn seed<S>(backend: &S, tenant: &TenantContext)
where
    S: ResourceStorage + SearchProvider,
{
    for (id, patient, performer, status, code, date) in [
        ("ec-p1-final", "p1", None, "final", "X", "2024-01-01"),
        ("ec-p1-amended", "p1", None, "amended", "Y", "2024-01-02"),
        ("ec-p2-final", "p2", None, "final", "X", "2024-01-03"),
        ("ec-perf-p1", "p2", Some("p1"), "final", "Y", "2024-01-04"),
    ] {
        backend
            .seed_create(
                tenant,
                "Observation",
                observation(id, patient, performer, status, code, date),
                FhirVersion::default(),
            )
            .await
            .expect("seed observation");
    }
    for (id, contained) in [
        (
            "ec-dr1",
            vec![
                observation("c1", "p1", None, "final", "X", "2024-02-01"),
                observation("c2", "p2", None, "final", "Y", "2024-02-02"),
            ],
        ),
        (
            "ec-dr2",
            vec![observation("c3", "p2", None, "final", "X", "2024-02-03")],
        ),
    ] {
        backend
            .seed_create(
                tenant,
                "DiagnosticReport",
                json!({
                    "resourceType": "DiagnosticReport",
                    "id": id,
                    "status": "final",
                    "code": {"text": "panel"},
                    "contained": contained,
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed container");
    }

    // The positive controls: the reference rows membership reads, and the
    // contained ones, are all indexed.
    let mut top = SearchQuery::new("Observation").with_count(100);
    top.compartment = Some(real_membership());
    let mut contained = SearchQuery::new("Observation").with_count(100);
    contained.contained = ContainedMode::On;
    contained.compartment = Some(real_membership());
    for (label, query, expected) in [
        (
            "Patient/p1/Observation",
            top,
            vec!["ec-p1-amended", "ec-p1-final", "ec-perf-p1"],
        ),
        (
            "Patient/p1/Observation [_contained=true]",
            contained,
            vec!["ec-dr1"],
        ),
    ] {
        for attempt in 0..60 {
            let got = observe(backend, tenant, &query).await.map(|seen| seen.ids);
            if got.as_ref() == Ok(&expected.iter().map(|id| id.to_string()).collect::<Vec<_>>()) {
                break;
            }
            assert!(
                attempt < 59,
                "positive control {label} never held: got {got:?}, expected {expected:?} — \
                 is the backend built with the spec search parameters?"
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}

/// An empty compartment membership matches nothing on every search path, and a
/// real membership still returns its members on the same ones.
pub async fn empty_membership_matches_nothing_on_every_path<S>(backend: &S, tenant_base: &str)
where
    S: ResourceStorage + SearchProvider,
{
    let tenant = TenantContext::new(TenantId::new(tenant_base), TenantPermissions::full_access());
    seed(backend, &tenant).await;

    let mut report = Report::default();

    // Top-level, `_contained=false`.
    for (name, variant) in top_level_variants() {
        for (shape, membership) in empty_memberships() {
            let mut query = variant.clone();
            query.compartment = Some(membership);
            let actual = observe(backend, &tenant, &query).await;
            report.record(
                &format!("{shape} + {name}"),
                &expected_seen(&query, &[]),
                actual,
            );
        }
    }

    // `_contained=true|both`.
    for (name, variant) in contained_variants() {
        for (shape, membership) in empty_memberships() {
            let mut query = variant.clone();
            query.compartment = Some(membership);
            let actual = observe(backend, &tenant, &query).await;
            report.record(
                &format!("{shape} + {name}"),
                &expected_seen(&query, &[]),
                actual,
            );
        }
    }

    // Controls: the real membership, on the same paths.
    let members = ["ec-p1-amended", "ec-p1-final", "ec-perf-p1"];
    for (name, variant) in top_level_variants() {
        let expected: Vec<&str> = match name {
            // Code X: the one Patient/p1 observation with it.
            "code=X" => vec!["ec-p1-final"],
            // Not amended: the two final ones.
            "status:not=amended" => vec!["ec-p1-final", "ec-perf-p1"],
            "code=X&status:not=amended&_sort=date&_total=accurate" => vec!["ec-p1-final"],
            // Every member has a status.
            _ => members.to_vec(),
        };
        let mut query = variant.clone();
        query.compartment = Some(real_membership());
        // A page of two reports the first two members, and that more follow.
        if name == "_count=2" {
            let actual = observe(backend, &tenant, &query).await;
            let ok = actual
                .as_ref()
                .is_ok_and(|seen| seen.ids.len() == 2 && seen.has_next && seen.counted == 3);
            if !ok {
                report.failures += 1;
            }
            report.lines.push(format!(
                "{} | control + {name} | expected 2 of 3 members and a next page | actual {actual:?}",
                if ok { "ok  " } else { "FAIL" }
            ));
            continue;
        }
        let actual = observe(backend, &tenant, &query).await;
        report.record(
            &format!("control + {name}"),
            &expected_seen(&query, &expected),
            actual,
        );
    }
    for (name, variant) in contained_variants() {
        let mut query = variant.clone();
        query.compartment = Some(real_membership());
        let expected: Vec<&str> = match (query.contained, query.contained_return) {
            (ContainedMode::On, ContainedReturn::Container) => vec!["ec-dr1"],
            (ContainedMode::On, ContainedReturn::Contained) => vec!["c1"],
            (ContainedMode::Both, ContainedReturn::Container) => {
                vec!["ec-dr1", "ec-p1-amended", "ec-p1-final", "ec-perf-p1"]
            }
            (ContainedMode::Both, ContainedReturn::Contained) => {
                vec!["c1", "ec-p1-amended", "ec-p1-final", "ec-perf-p1"]
            }
            (ContainedMode::Off, _) => unreachable!("contained variants are never `Off`"),
        };
        let expected: Vec<&str> = if query.parameters.is_empty() {
            expected
        } else {
            // `code=X` keeps the Patient/p1 observations with code X: the
            // top-level `ec-p1-final` and the contained `c1`.
            expected
                .into_iter()
                .filter(|id| matches!(*id, "ec-dr1" | "c1" | "ec-p1-final"))
                .collect()
        };
        let actual = observe(backend, &tenant, &query).await;
        report.record(
            &format!("control + {name}"),
            &expected_seen(&query, &expected),
            actual,
        );
    }

    report.finish("an empty compartment membership");
}
