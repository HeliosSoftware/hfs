//! #2254: a search or view run that names a FHIR version reads only the
//! resources stored under it.
//!
//! Each test stores one resource type under R4 and R5 in one tenant and checks
//! that a version-restricted search pages, counts and sorts over that version's
//! resources alone (so pages stay full and totals agree), that `None` keeps the
//! version-blind result, and that the in-DB SoF runner scans only that
//! version's resources.
//!
//! Child module of the `mongodb_tests` root: `use super::*;` reaches its private
//! harness (`create_backend_with_stable_probes`, `create_tenant`, `seed_create`).

use super::*;

use helios_persistence::core::sof_runner::ViewFilters;
use helios_persistence::types::{CompartmentMembership, ContainedMode, ContainedReturn};
use tokio_stream::StreamExt;

/// R4 Patients, by id and family name; R5 names sort between them.
const R4_PATIENTS: [(&str, &str); 5] = [
    ("r4-ann", "Ann"),
    ("r4-cid", "Cid"),
    ("r4-eve", "Eve"),
    ("r4-gus", "Gus"),
    ("r4-ida", "Ida"),
];
const R5_PATIENTS: [(&str, &str); 3] = [("r5-bob", "Bob"), ("r5-dan", "Dan"), ("r5-fay", "Fay")];

async fn seed_patients(backend: &MongoBackend, tenant: &TenantContext) {
    for (patients, version) in [
        (&R4_PATIENTS[..], FhirVersion::R4),
        (&R5_PATIENTS[..], FhirVersion::R5),
    ] {
        for (id, family) in patients {
            backend
                .seed_create(
                    tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "gender": "female",
                        "name": [{ "family": family }],
                    }),
                    version,
                )
                .await
                .unwrap();
        }
    }
}

fn gender_female() -> SearchParameter {
    SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("female")],
        chain: vec![],
        components: vec![],
    }
}

fn by_name() -> SortDirective {
    SortDirective::parse("name").with_param_type(Some(SearchParamType::String))
}

fn ids(result: &helios_persistence::core::SearchResult) -> Vec<String> {
    result
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

fn expected(patients: &[(&str, &str)]) -> Vec<String> {
    sorted(patients.iter().map(|(id, _)| id.to_string()).collect())
}

/// Walks every offset page of `query` (page size `count`), asserting each
/// page but the last is full and every page reports `total`.
async fn walk_offset_pages(
    backend: &MongoBackend,
    tenant: &TenantContext,
    query: &SearchQuery,
    count: u32,
    total: u64,
) -> Vec<String> {
    let mut all = Vec::new();
    let mut offset = 0;
    loop {
        let mut page_query = query.clone();
        page_query.count = Some(count);
        page_query.offset = Some(offset);
        page_query.total = Some(TotalMode::Accurate);
        let page = backend.search(tenant, &page_query).await.unwrap();
        assert_eq!(page.total, Some(total), "total at offset {offset}");
        let page_ids = ids(&page);
        let remaining = total as usize - all.len();
        assert_eq!(
            page_ids.len(),
            remaining.min(count as usize),
            "page at offset {offset} must be full: {page_ids:?}"
        );
        assert_eq!(
            page.resources.page_info.has_next,
            remaining > count as usize
        );
        all.extend(page_ids);
        if !page.resources.page_info.has_next {
            return all;
        }
        offset += count;
    }
}

#[tokio::test]
async fn mongodb_fhir_version_filter_pages_counts_and_sorts() {
    let Some(backend) = create_backend_with_stable_probes("fhir_version_search").await else {
        eprintln!("Skipping mongodb_fhir_version_filter_pages_counts_and_sorts (no MongoDB)");
        return;
    };
    let tenant = create_tenant("tenant-fhir-version-search");
    seed_patients(&backend, &tenant).await;
    let all: Vec<(&str, &str)> = R4_PATIENTS.iter().chain(&R5_PATIENTS).copied().collect();

    // Version-blind: both versions, as before #2254.
    let blind = SearchQuery::new("Patient");
    assert_eq!(
        sorted(walk_offset_pages(&backend, &tenant, &blind, 3, 8).await),
        expected(&all)
    );
    assert_eq!(backend.search_count(&tenant, &blind).await.unwrap(), 8);

    for (version, patients) in [
        (FhirVersion::R4, &R4_PATIENTS[..]),
        (FhirVersion::R5, &R5_PATIENTS[..]),
    ] {
        let n = patients.len() as u64;
        let plain = SearchQuery::new("Patient").with_fhir_version(version);

        // Offset pages of the default sort stay full and agree with the total.
        assert_eq!(
            sorted(walk_offset_pages(&backend, &tenant, &plain, 2, n).await),
            expected(patients),
            "{version:?} offset pages"
        );
        assert_eq!(backend.search_count(&tenant, &plain).await.unwrap(), n);

        // Cursor pages walk the same set without gaps or repeats.
        let mut walked = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut q = plain.clone().with_count(2);
            q.cursor = cursor.clone();
            let page = backend.search(&tenant, &q).await.unwrap();
            assert!(
                !page.resources.items.is_empty(),
                "{version:?} cursor page must not be empty"
            );
            walked.extend(ids(&page));
            cursor = page.resources.page_info.next_cursor.clone();
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            sorted(walked),
            expected(patients),
            "{version:?} cursor pages"
        );

        // A parameter search, sorted by name: filtered, in name order, with
        // full pages.
        let named = plain
            .clone()
            .with_parameter(gender_female())
            .with_sort(by_name());
        let mut by_family: Vec<(&str, &str)> = patients.to_vec();
        by_family.sort_by_key(|(_, family)| *family);
        let in_order: Vec<String> = by_family.iter().map(|(id, _)| id.to_string()).collect();
        assert_eq!(
            walk_offset_pages(&backend, &tenant, &named, 2, n).await,
            in_order,
            "{version:?} gender + _sort=name"
        );
        assert_eq!(backend.search_count(&tenant, &named).await.unwrap(), n);

        // The UI rail's shape: `_sort=name&_count=N&_offset=M`, no parameter.
        let rail = plain.clone().with_sort(by_name());
        assert_eq!(
            walk_offset_pages(&backend, &tenant, &rail, 2, n).await,
            in_order,
            "{version:?} _sort=name"
        );

        // `_sort=_id` with an accurate total (the combined page+total route
        // when version-blind).
        let by_id = plain
            .clone()
            .with_parameter(gender_female())
            .with_sort(SortDirective::parse("_id"));
        assert_eq!(
            walk_offset_pages(&backend, &tenant, &by_id, 2, n).await,
            expected(patients),
            "{version:?} gender + _sort=_id"
        );

        // An estimated total is exact under a version filter.
        let mut estimate = plain.clone().with_parameter(gender_female()).with_count(1);
        estimate.total = Some(TotalMode::Estimate);
        let page = backend.search(&tenant, &estimate).await.unwrap();
        assert_eq!(page.total, Some(n), "{version:?} estimated total");
        assert_eq!(ids(&page).len(), 1);

        // `:missing=true` (no Patient has a birthdate).
        let missing = plain.clone().with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("true")],
            chain: vec![],
            components: vec![],
        });
        assert_eq!(
            sorted(walk_offset_pages(&backend, &tenant, &missing, 2, n).await),
            expected(patients),
            "{version:?} birthdate:missing=true"
        );
        assert_eq!(backend.search_count(&tenant, &missing).await.unwrap(), n);

        // `:not` (complement against the version's resources).
        let mut not_male = gender_female();
        not_male.modifier = Some(SearchModifier::Not);
        not_male.values = vec![SearchValue::eq("male")];
        let not = plain.clone().with_parameter(not_male);
        assert_eq!(backend.search_count(&tenant, &not).await.unwrap(), n);
    }
}

#[tokio::test]
async fn mongodb_fhir_version_filter_compartment_contained_and_includes() {
    let Some(backend) = create_backend_with_stable_probes("fhir_version_compartment").await else {
        eprintln!(
            "Skipping mongodb_fhir_version_filter_compartment_contained_and_includes (no MongoDB)"
        );
        return;
    };
    let tenant = create_tenant("tenant-fhir-version-compartment");

    // The subject is stored under R4; one Observation per version refers to it,
    // and each contains a Patient named "Inner".
    backend
        .seed_create(
            &tenant,
            "Patient",
            json!({ "resourceType": "Patient", "id": "p1", "name": [{ "family": "Outer" }] }),
            FhirVersion::R4,
        )
        .await
        .unwrap();
    for (id, version) in [("obs-r4", FhirVersion::R4), ("obs-r5", FhirVersion::R5)] {
        backend
            .seed_create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": id,
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                    "subject": { "reference": "Patient/p1" },
                    "performer": [{ "reference": "#inner" }],
                    "contained": [{
                        "resourceType": "Patient",
                        "id": "inner",
                        "name": [{ "family": "Inner" }]
                    }]
                }),
                version,
            )
            .await
            .unwrap();
    }

    // Compartment search.
    let compartment = |version: Option<FhirVersion>| {
        let mut q = SearchQuery::new("Observation");
        q.compartment = Some(CompartmentMembership {
            params: vec!["subject".to_string(), "patient".to_string()],
            reference: "Patient/p1".to_string(),
        });
        q.total = Some(TotalMode::Accurate);
        q.fhir_version = version;
        q
    };
    for (version, want) in [
        (None, vec!["obs-r4", "obs-r5"]),
        (Some(FhirVersion::R4), vec!["obs-r4"]),
        (Some(FhirVersion::R5), vec!["obs-r5"]),
    ] {
        let result = backend
            .search(&tenant, &compartment(version))
            .await
            .unwrap();
        assert_eq!(sorted(ids(&result)), want, "compartment {version:?}");
        assert_eq!(result.total, Some(want.len() as u64));
        assert_eq!(
            backend
                .search_count(&tenant, &compartment(version))
                .await
                .unwrap(),
            want.len() as u64
        );
    }

    // `_contained`: the contained Patient is of its container's version.
    let contained = |mode: ContainedMode, ret: ContainedReturn, version: Option<FhirVersion>| {
        let mut q = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Inner")],
            chain: vec![],
            components: vec![],
        });
        q.contained = mode;
        q.contained_return = ret;
        q.total = Some(TotalMode::Accurate);
        q.fhir_version = version;
        q
    };
    for (version, want) in [
        (None, vec!["Observation/obs-r4", "Observation/obs-r5"]),
        (Some(FhirVersion::R4), vec!["Observation/obs-r4"]),
        (Some(FhirVersion::R5), vec!["Observation/obs-r5"]),
    ] {
        for mode in [ContainedMode::On, ContainedMode::Both] {
            let q = contained(mode, ContainedReturn::Container, version);
            let result = backend.search(&tenant, &q).await.unwrap();
            let mut urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
            urls.sort();
            assert_eq!(urls, want, "_contained={mode:?} {version:?}");
            assert_eq!(result.total, Some(want.len() as u64));
            assert_eq!(
                backend.search_count(&tenant, &q).await.unwrap(),
                want.len() as u64
            );
        }
        let q = contained(ContainedMode::On, ContainedReturn::Contained, version);
        let result = backend.search(&tenant, &q).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            want.len(),
            "contained {version:?}"
        );
        assert_eq!(result.total, Some(want.len() as u64));
    }

    // `_include` targets are not filtered: the R5 Observation's R4 subject is
    // still included.
    let include = SearchQuery::new("Observation")
        .with_fhir_version(FhirVersion::R5)
        .with_include(IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        });
    let result = backend.search(&tenant, &include).await.unwrap();
    assert_eq!(ids(&result), vec!["obs-r5"]);
    let included: Vec<String> = result.included.iter().map(|r| r.url()).collect();
    assert_eq!(included, vec!["Patient/p1"]);
}

#[tokio::test]
async fn mongodb_fhir_version_filter_sof_runner_reads_one_version() {
    let Some(backend) = create_backend_with_stable_probes("fhir_version_sof").await else {
        eprintln!("Skipping mongodb_fhir_version_filter_sof_runner_reads_one_version (no MongoDB)");
        return;
    };
    let tenant = create_tenant("tenant-fhir-version-sof");
    for (id, version) in [
        ("obs-r4-a", FhirVersion::R4),
        ("obs-r4-b", FhirVersion::R4),
        ("obs-r5-a", FhirVersion::R5),
    ] {
        backend
            .seed_create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": id,
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                }),
                version,
            )
            .await
            .unwrap();
    }

    let runner = backend.sof_runner().expect("MongoDB provides a SOF runner");
    assert_eq!(runner.runner_name(), "mongo-indb");
    let run = |version: Option<FhirVersion>, union: bool| {
        let runner = runner.clone();
        let tenant = tenant.clone();
        async move {
            // A `unionAll` view exercises the `$facet` branches too.
            let select = if union {
                json!([{ "unionAll": [
                    { "column": [{ "path": "id", "name": "obs_id" }] },
                    { "column": [{ "path": "id", "name": "obs_id" }] }
                ] }])
            } else {
                json!([{ "column": [{ "path": "id", "name": "obs_id" }] }])
            };
            let view = json!({
                "resourceType": "ViewDefinition",
                "resource": "Observation",
                "status": "active",
                "select": select,
            });
            let filters = ViewFilters {
                fhir_version: version,
                ..Default::default()
            };
            let mut stream = runner
                .run_view(&tenant, view, filters)
                .await
                .expect("run_view must succeed");
            let mut ids = Vec::new();
            while let Some(row) = stream.next().await {
                let row = row.expect("row must not be an error");
                ids.push(row["obs_id"].as_str().unwrap().to_string());
            }
            ids.sort();
            ids
        }
    };

    assert_eq!(run(None, false).await, ["obs-r4-a", "obs-r4-b", "obs-r5-a"]);
    assert_eq!(
        run(Some(FhirVersion::R4), false).await,
        ["obs-r4-a", "obs-r4-b"]
    );
    assert_eq!(run(Some(FhirVersion::R5), false).await, ["obs-r5-a"]);
    assert_eq!(
        run(Some(FhirVersion::R4), true).await,
        ["obs-r4-a", "obs-r4-a", "obs-r4-b", "obs-r4-b"]
    );
    assert_eq!(
        run(Some(FhirVersion::R5), true).await,
        ["obs-r5-a", "obs-r5-a"]
    );
}

/// The shared suite every backend runs (#2254): first page, offset, cursor
/// next and previous, counts, `search_ids`, `search_reference_values` and
/// `_contained`.
#[tokio::test]
async fn mongodb_search_honors_fhir_version() {
    let Some(backend) = create_backend_with_stable_probes("fhir_version_suite").await else {
        eprintln!("Skipping mongodb_search_honors_fhir_version (no MongoDB)");
        return;
    };
    super::fhir_version_filter_suite::search_honors_fhir_version(&backend, "fhir-version-2254")
        .await;
}

/// The shared SoF runner check (#2254): flat scan, `unionAll`, `repeat`,
/// patient/group (the in-process fallback over `MongoResourceScan`).
#[tokio::test]
async fn mongodb_sof_runner_honors_fhir_version() {
    let Some(backend) = create_backend_with_stable_probes("fhir_version_sof_suite").await else {
        eprintln!("Skipping mongodb_sof_runner_honors_fhir_version (no MongoDB)");
        return;
    };
    let runner = backend.sof_runner().expect("MongoDB provides a SOF runner");
    let tenant = create_tenant("tenant-fhir-version-sof-suite");
    super::sof_fhir_version_filter::runner_honors_fhir_version(&backend, runner.as_ref(), &tenant)
        .await;
}

/// Patient/group runs go to the in-process fallback over `MongoResourceScan`:
/// a version-restricted run reads the Group and the scanned type under that
/// version only.
#[tokio::test]
async fn mongodb_fhir_version_filter_sof_compartment_fallback() {
    let Some(backend) = create_backend_with_stable_probes("fhir_version_sof_fallback").await else {
        eprintln!("Skipping mongodb_fhir_version_filter_sof_compartment_fallback (no MongoDB)");
        return;
    };
    let tenant = create_tenant("tenant-fhir-version-sof-fallback");
    for (id, version) in [
        ("a4", FhirVersion::R4),
        ("b4", FhirVersion::R4),
        ("a5", FhirVersion::R5),
    ] {
        backend
            .seed_create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": id, "gender": "female" }),
                version,
            )
            .await
            .unwrap();
    }
    for (id, version) in [("g4", FhirVersion::R4), ("g5", FhirVersion::R5)] {
        backend
            .seed_create(
                &tenant,
                "Group",
                json!({
                    "resourceType": "Group", "id": id, "type": "person", "actual": true,
                    "member": [
                        { "entity": { "reference": "Patient/a4" } },
                        { "entity": { "reference": "Patient/a5" } }
                    ]
                }),
                version,
            )
            .await
            .unwrap();
    }

    let runner = backend.sof_runner().expect("MongoDB provides a SOF runner");
    let view = json!({
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "status": "active",
        "select": [{ "column": [{ "path": "id", "name": "id" }] }]
    });
    let run = |filters: ViewFilters| {
        let runner = runner.clone();
        let tenant = tenant.clone();
        let view = view.clone();
        async move {
            let mut stream = runner
                .run_view(&tenant, view, filters)
                .await
                .expect("run_view must succeed");
            let mut ids = Vec::new();
            while let Some(row) = stream.next().await {
                let row = row.expect("row must not be an error");
                ids.push(row["id"].as_str().unwrap().to_string());
            }
            ids.sort();
            ids
        }
    };
    let group = |group: &str, version: Option<FhirVersion>| ViewFilters {
        group: vec![group.to_string()],
        fhir_version: version,
        ..Default::default()
    };

    // Version-blind: both members, whatever their version.
    assert_eq!(run(group("Group/g4", None)).await, ["a4", "a5"]);
    // An R5 run over an R5 Group scans R5 Patients only.
    assert_eq!(run(group("Group/g5", Some(FhirVersion::R5))).await, ["a5"]);
    // An R4 run over an R4 Group scans R4 Patients only.
    assert_eq!(run(group("Group/g4", Some(FhirVersion::R4))).await, ["a4"]);
    // A patient filter naming a Patient of the run's version.
    assert_eq!(
        run(ViewFilters {
            patient: vec!["Patient/b4".to_string()],
            fhir_version: Some(FhirVersion::R4),
            ..Default::default()
        })
        .await,
        ["b4"]
    );
}
