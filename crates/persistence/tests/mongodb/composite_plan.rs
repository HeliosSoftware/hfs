//! Composite, `:identifier` and compartment searches on the planner's bounded
//! plans (#1896), through the real write path and the profiler.
//!
//! The first ignored harness at the end seeds more rows than the 300,000-id
//! cap of the candidate collector, to show the planned shapes answer where the
//! collector refuses. The second times composite pages across pair-check pass
//! rates.

use super::*;
use futures::TryStreamExt;
use helios_persistence::types::CompartmentMembership;
use mongodb::bson::DateTime;

const LOINC: &str = "http://loinc.org";
const UCUM: &str = "http://unitsofmeasure.org";
const COMPOSITE_INDEX: &str = "idx_search_composite";
const TEMPLATE_TENANT: &str = "composite-plan-template";
const BASE_MILLIS: i64 = 1_704_067_200_000; // 2024-01-01T00:00:00Z

fn height(id: &str, code: &str, value: f64, status: &str) -> serde_json::Value {
    json!({
        "resourceType": "Observation", "id": id, "status": status,
        "code": {"coding": [{"system": LOINC, "code": code}]},
        "valueQuantity": {"value": value, "unit": "cm", "system": UCUM},
    })
}

/// A blood-pressure panel: each vital sign is its own `component`, hence its
/// own `composite_group`.
fn panel(id: &str, systolic: f64, diastolic: f64) -> serde_json::Value {
    json!({
        "resourceType": "Observation", "id": id, "status": "final",
        "code": {"coding": [{"system": LOINC, "code": "85354-9"}]},
        "component": [
            {"code": {"coding": [{"system": LOINC, "code": "8480-6"}]},
             "valueQuantity": {"value": systolic, "unit": "mm[Hg]", "system": UCUM}},
            {"code": {"coding": [{"system": LOINC, "code": "8462-4"}]},
             "valueQuantity": {"value": diastolic, "unit": "mm[Hg]", "system": UCUM}},
        ],
    })
}

async fn create_observation(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource: serde_json::Value,
) {
    backend
        .create(tenant, "Observation", resource, FhirVersion::default())
        .await
        .unwrap();
}

async fn raw_database(backend: &MongoBackend) -> mongodb::Database {
    raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name)
}

/// A resource written through the real write path, kept as its stored
/// documents so it can be copied under many ids.
struct Template {
    resource: Document,
    rows: Vec<Document>,
}

async fn read_template(db: &mongodb::Database, tenant: &str, id: &str) -> Template {
    let resource = db
        .collection::<Document>("resources")
        .find_one(doc! { "tenant_id": tenant, "resource_type": "Observation", "id": id })
        .await
        .unwrap()
        .expect("the template resource was written");
    let rows = db
        .collection::<Document>("search_index")
        .find(doc! { "tenant_id": tenant, "resource_type": "Observation", "resource_id": id })
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert!(!rows.is_empty(), "the template wrote search rows");
    Template { resource, rows }
}

/// Copies `template` under `tenant` once per `(id, last_updated millis)`,
/// optionally keeping only the index rows of the named parameters.
async fn replicate(
    db: &mongodb::Database,
    template: &Template,
    tenant: &str,
    entries: impl Iterator<Item = (String, i64)>,
    keep: Option<&[&str]>,
) {
    let resources = db.collection::<Document>("resources");
    let index = db.collection::<Document>("search_index");
    let mut resource_batch = Vec::new();
    let mut row_batch = Vec::new();
    for (id, millis) in entries {
        let mut resource = template.resource.clone();
        resource.remove("_id");
        resource.insert("tenant_id", tenant);
        resource.insert("id", id.as_str());
        resource.insert("created_at", DateTime::from_millis(millis));
        resource.insert("last_updated", DateTime::from_millis(millis));
        if let Ok(data) = resource.get_document_mut("data") {
            data.insert("id", id.as_str());
        }
        resource_batch.push(resource);
        for row in &template.rows {
            let name = row.get_str("param_name").unwrap_or_default();
            if keep.is_some_and(|keep| !keep.contains(&name)) {
                continue;
            }
            let mut row = row.clone();
            row.remove("_id");
            row.insert("tenant_id", tenant);
            row.insert("resource_id", id.as_str());
            row_batch.push(row);
        }
        if resource_batch.len() >= 2_500 {
            resources
                .insert_many(std::mem::take(&mut resource_batch))
                .await
                .unwrap();
            index
                .insert_many(std::mem::take(&mut row_batch))
                .await
                .unwrap();
        }
    }
    if !resource_batch.is_empty() {
        resources.insert_many(resource_batch).await.unwrap();
        index.insert_many(row_batch).await.unwrap();
    }
}

fn ids_of(result: &helios_persistence::core::SearchResult) -> Vec<String> {
    result
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_owned())
        .collect()
}

/// The sort the documented orders mean, independent of the backend's own
/// builder: the final directive is `id`, descending unless named.
fn sort_document(sort: &str) -> Document {
    match sort {
        "" => doc! { "last_updated": -1, "id": -1 },
        "_lastUpdated" => doc! { "last_updated": 1, "id": -1 },
        "-_lastUpdated" => doc! { "last_updated": -1, "id": -1 },
        "_lastUpdated,_id" => doc! { "last_updated": 1, "id": 1 },
        "_id" => doc! { "id": 1 },
        "-_id" => doc! { "id": -1 },
        other => panic!("no documented order for {other}"),
    }
}

async fn live_in_order(
    db: &mongodb::Database,
    tenant: &str,
    expected: &[String],
    sort: &str,
) -> Vec<String> {
    db.collection::<Document>("resources")
        .find(doc! {
            "tenant_id": tenant, "resource_type": "Observation",
            "is_deleted": false, "id": { "$in": expected },
        })
        .projection(doc! { "_id": 0, "id": 1 })
        .sort(sort_document(sort))
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get_str("id").unwrap().to_owned())
        .collect()
}

fn sorted_query(query: &SearchQuery, sort: &str) -> SearchQuery {
    let mut query = query.clone();
    query.sort = sort
        .split(',')
        .filter(|part| !part.is_empty())
        .map(SortDirective::parse)
        .collect();
    query
}

fn keyset_sort(sort: &str) -> bool {
    matches!(sort, "" | "_lastUpdated" | "-_lastUpdated" | "_id" | "-_id")
}

/// Every route a composite page can take, against ids and order fixed by the
/// fixture: offset pages, cursor pages forward and back, an accurate total,
/// and the standalone count.
async fn assert_every_route(
    backend: &MongoBackend,
    tenant: &TenantContext,
    query: &SearchQuery,
    expected: &[String],
) {
    let db = raw_database(backend).await;
    assert!(!expected.is_empty());
    let count = 7;
    for sort in [
        "",
        "_lastUpdated",
        "-_lastUpdated",
        "_lastUpdated,_id",
        "_id",
        "-_id",
    ] {
        let want = live_in_order(&db, tenant.tenant_id().as_str(), expected, sort).await;
        assert_eq!(want.len(), expected.len(), "sort {sort:?}");
        let mut paged = sorted_query(query, sort).with_count(count);
        paged.total = Some(TotalMode::None);

        let mut walked = Vec::new();
        let mut offset = 0;
        loop {
            let mut next = paged.clone();
            next.offset = Some(offset);
            let page = backend.search(tenant, &next).await.unwrap();
            walked.extend(ids_of(&page));
            assert_eq!(page.total, None);
            assert_eq!(page.resources.page_info.has_previous, offset > 0);
            if !page.resources.page_info.has_next {
                break;
            }
            offset += count;
        }
        assert_eq!(walked, want, "offset pages, sort {sort:?}");

        if keyset_sort(sort) {
            let mut forward = Vec::new();
            let mut previous = Vec::new();
            let mut page = backend.search(tenant, &paged).await.unwrap();
            loop {
                forward.push(ids_of(&page));
                previous.push(page.resources.page_info.previous_cursor.clone());
                let Some(cursor) = page.resources.page_info.next_cursor.clone() else {
                    break;
                };
                page = backend
                    .search(tenant, &paged.clone().with_cursor(cursor))
                    .await
                    .unwrap();
            }
            assert_eq!(forward.concat(), want, "cursor pages, sort {sort:?}");
            for position in (1..forward.len()).rev() {
                let cursor = previous[position]
                    .clone()
                    .expect("a later page has a previous");
                let back = backend
                    .search(tenant, &paged.clone().with_cursor(cursor))
                    .await
                    .unwrap();
                assert_eq!(ids_of(&back), forward[position - 1], "back, sort {sort:?}");
                assert!(back.resources.page_info.has_next);
            }
        }

        let mut counted = sorted_query(query, sort).with_count(count);
        counted.total = Some(TotalMode::Accurate);
        let page = backend.search(tenant, &counted).await.unwrap();
        assert_eq!(page.total, Some(want.len() as u64), "total, sort {sort:?}");
    }
    assert_eq!(
        backend.search_count(tenant, query).await.unwrap(),
        expected.len() as u64
    );
}

#[tokio::test]
async fn mongodb_composite_pages_and_totals_match_expected_on_every_route() {
    let Some(backend) = create_backend_with_full_registry("composite_plan_routes").await else {
        eprintln!("Skipping composite_plan routes (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("composite-plan-routes");
    let mut systolic_match = Vec::new();
    for number in 0..120 {
        let id = format!("panel-{number:03}");
        // Every tenth panel has a diastolic value above 130 next to a low
        // systolic one: the code and the value exist, in different components.
        let (systolic, diastolic) = if number % 10 == 3 {
            (110.0, 140.0)
        } else {
            (
                100.0 + ((number * 7) % 60) as f64,
                60.0 + ((number * 11) % 40) as f64,
            )
        };
        create_observation(&backend, &tenant, panel(&id, systolic, diastolic)).await;
        if systolic > 130.0 && !(10..15).contains(&number) {
            systolic_match.push(id);
        }
    }
    let mut tall = Vec::new();
    for number in 0..80 {
        let id = format!("height-{number:03}");
        create_observation(
            &backend,
            &tenant,
            height(&id, "8302-2", 140.0 + number as f64, "final"),
        )
        .await;
        if 140 + number > 160 && !(30..35).contains(&number) {
            tall.push(id);
        }
    }
    // A weight above 160 shares the value, not the code.
    for number in 0..30 {
        let id = format!("weight-{number:03}");
        create_observation(
            &backend,
            &tenant,
            height(&id, "29463-7", 150.0 + number as f64, "final"),
        )
        .await;
    }
    for number in 10..15 {
        backend
            .delete(&tenant, "Observation", &format!("panel-{number:03}"))
            .await
            .unwrap();
    }
    for number in 30..35 {
        backend
            .delete(&tenant, "Observation", &format!("height-{number:03}"))
            .await
            .unwrap();
    }
    assert!(systolic_match.len() > 20 && tall.len() > 20);

    assert_every_route(
        &backend,
        &tenant,
        &component_code_value_quantity_query("8480-6$gt130"),
        &systolic_match,
    )
    .await;
    assert_every_route(
        &backend,
        &tenant,
        &code_value_quantity_query("http://loinc.org|8302-2$gt160"),
        &tall,
    )
    .await;
    // A comma list is an OR of two arms that differ.
    let mut either = tall.clone();
    either.extend(
        (0..30)
            .filter(|n| 150 + n < 160)
            .map(|n| format!("weight-{n:03}")),
    );
    assert_every_route(
        &backend,
        &tenant,
        &code_value_quantity_query_values(&["8302-2$gt160", "29463-7$lt160"]),
        &either,
    )
    .await;
}

/// Profile entries of the two collections a search reads, with every getMore.
async fn profiled_search_replies(db: &mongodb::Database) -> Vec<Document> {
    db.collection::<Document>("system.profile")
        .find(doc! {
            "ns": { "$in": [
                format!("{}.search_index", db.name()),
                format!("{}.resources", db.name()),
            ] },
            "op": { "$in": ["query", "command", "getmore"] },
        })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

fn returned(operation: &Document) -> i64 {
    operation
        .get_i64("nreturned")
        .or_else(|_| operation.get_i32("nreturned").map(i64::from))
        .unwrap_or(0)
}

fn command_of(operation: &Document) -> &Document {
    operation
        .get_document("originatingCommand")
        .or_else(|_| operation.get_document("command"))
        .expect("a profile entry names its command")
}

/// The pair-check reads: `search_index` finds that project `composite_group`.
fn pair_checks(operations: &[Document]) -> Vec<&Document> {
    operations
        .iter()
        .filter(|operation| {
            operation.get_str("op") == Ok("query")
                && command_of(operation).get_str("find") == Ok("search_index")
                && command_of(operation)
                    .get_document("projection")
                    .is_ok_and(|projection| projection.contains_key("composite_group"))
        })
        .collect()
}

fn candidate_list_length(value: &Bson) -> Option<usize> {
    match value {
        Bson::Document(document) => document
            .get_document("resource_id")
            .ok()
            .and_then(|ids| ids.get_array("$in").ok())
            .map(Vec::len)
            .or_else(|| document.values().find_map(candidate_list_length)),
        Bson::Array(values) => values.iter().find_map(candidate_list_length),
        _ => None,
    }
}

fn limits_rows_to(operation: &Document, rows: i64) -> bool {
    command_of(operation)
        .get_array("pipeline")
        .is_ok_and(|pipeline| {
            pipeline.iter().filter_map(Bson::as_document).any(|stage| {
                stage
                    .get_i64("$limit")
                    .or_else(|_| stage.get_i32("$limit").map(i64::from))
                    == Ok(rows)
            })
        })
}

async fn profiled<T>(
    db: &mongodb::Database,
    work: impl std::future::Future<Output = T>,
) -> (T, Vec<Document>) {
    // A capped profile large enough to keep every command of a search. An
    // operation still finishing from before may recreate the default 1 MiB
    // collection between the drop and the create (NamespaceExists).
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    let profile = db.collection::<Document>("system.profile");
    for attempt in 1.. {
        profile.drop().await.unwrap();
        match db
            .create_collection("system.profile")
            .capped(true)
            .size(32 * 1024 * 1024)
            .await
        {
            Ok(()) => break,
            Err(error) if attempt < 10 && format!("{error}").contains("already exists") => {}
            Err(error) => panic!("recreate system.profile: {error}"),
        }
    }
    db.run_command(doc! { "profile": 2 })
        .await
        .expect("this test requires database profiling");
    let output = work.await;
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    let operations = profiled_search_replies(db).await;
    (output, operations)
}

#[tokio::test]
async fn mongodb_broad_composite_page_does_not_materialize_matches() {
    let Some(backend) = create_backend_with_stable_probes("composite_plan_broad").await else {
        eprintln!("Skipping composite_plan broad (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let template_tenant = create_tenant(TEMPLATE_TENANT);
    let tenant = create_tenant("composite-plan-broad");
    create_observation(
        &backend,
        &template_tenant,
        height("match", "8302-2", 170.0, "final"),
    )
    .await;
    create_observation(
        &backend,
        &template_tenant,
        height("fail", "8302-2", 150.0, "final"),
    )
    .await;
    let db = raw_database(&backend).await;
    let matching = read_template(&db, TEMPLATE_TENANT, "match").await;
    let failing = read_template(&db, TEMPLATE_TENANT, "fail").await;
    // One id space and one timeline for both kinds, so every order meets near
    // misses (every fifth row) among the matches: 12,000 pair matches and 3,000
    // rows whose value is 150.
    replicate(
        &db,
        &matching,
        "composite-plan-broad",
        (0..15_000)
            .filter(|n| n % 5 != 4)
            .map(|n| (format!("x{n:05}"), BASE_MILLIS + n * 5)),
        None,
    )
    .await;
    replicate(
        &db,
        &failing,
        "composite-plan-broad",
        (0..15_000)
            .filter(|n| n % 5 == 4)
            .map(|n| (format!("x{n:05}"), BASE_MILLIS + n * 5)),
        None,
    )
    .await;

    let mut default_sort = code_value_quantity_query("8302-2$gt160").with_count(20);
    default_sort.total = Some(TotalMode::None);
    let mut by_id = code_value_quantity_query("8302-2$gt160")
        .with_count(20)
        .with_sort(SortDirective::parse("_id"));
    by_id.total = Some(TotalMode::None);

    // Newest first: the highest numbered matches. Timestamps are distinct.
    let newest: Vec<String> = (0..15_000)
        .rev()
        .filter(|n| n % 5 != 4)
        .take(40)
        .map(|n| format!("x{n:05}"))
        .collect();
    let (first, operations) = profiled(&db, async {
        backend.search(&tenant, &default_sort).await.unwrap()
    })
    .await;
    assert_eq!(ids_of(&first), newest[..20].to_vec());
    assert!(first.resources.page_info.has_next);
    assert_eq!(first.total, None);
    let cursor = first.resources.page_info.next_cursor.clone().unwrap();
    let mut pages = vec![("first page", operations)];
    let (second, operations) = profiled(&db, async {
        backend
            .search(&tenant, &default_sort.clone().with_cursor(cursor))
            .await
            .unwrap()
    })
    .await;
    assert_eq!(ids_of(&second), newest[20..40].to_vec());
    pages.push(("next page", operations));
    let (by_id_page, operations) = profiled(&db, async {
        backend.search(&tenant, &by_id).await.unwrap()
    })
    .await;
    assert_eq!(
        ids_of(&by_id_page),
        (0..)
            .filter(|n| n % 5 != 4)
            .take(20)
            .map(|n| format!("x{n:05}"))
            .collect::<Vec<_>>()
    );
    pages.push(("id page", operations));
    for (label, operations) in &pages {
        let total: i64 = operations.iter().map(returned).sum();
        eprintln!("{label}: {total} documents returned by the search commands");
        assert!(total >= 20, "{label}: profiling must capture the transfer");
        assert!(
            total <= 2_048,
            "{label}: a 20-row page must transfer candidate batches, not the 15,000 \
             candidates; {total} documents were returned"
        );
        assert!(
            operations
                .iter()
                .all(|operation| !limits_rows_to(operation, 100_000)),
            "{label}: the planner must not run the collector's 100,000-row arm probe"
        );
        assert!(!pair_checks(operations).is_empty(), "{label}");
    }

    // The accurate total is proportional to the matches, not to the page.
    let mut accurate = default_sort.clone();
    accurate.total = Some(TotalMode::Accurate);
    let total = backend.search(&tenant, &accurate).await.unwrap();
    assert_eq!(total.total, Some(12_000));
    assert_eq!(
        backend.search_count(&tenant, &default_sort).await.unwrap(),
        12_000
    );
}

fn assert_pair_checks_are_hinted(operations: &[Document], max_candidates: usize, label: &str) {
    let finds = pair_checks(operations);
    assert!(!finds.is_empty(), "{label}: no pair check was profiled");
    for find in &finds {
        let command = command_of(find);
        assert_eq!(command.get_str("hint"), Ok(COMPOSITE_INDEX), "{label}");
        let summary = find.get_str("planSummary").unwrap_or_default();
        assert!(
            summary.contains("resource_id: 1, param_name: 1, composite_group: 1"),
            "{label}: {summary}"
        );
        let bound = candidate_list_length(&Bson::Document(command.clone()))
            .unwrap_or_else(|| panic!("{label}: a pair check is bounded by candidate ids"));
        assert!(
            bound > 0 && bound <= max_candidates,
            "{label}: {bound} candidate ids"
        );
    }
}

/// Item 4 of #1896: the pair check reads through `idx_search_composite` on the
/// collector's driver path as well as on the planner's.
#[tokio::test]
async fn mongodb_legacy_composite_pair_check_is_hinted() {
    let Some(backend) = create_backend_with_stable_probes("composite_plan_hint").await else {
        eprintln!("Skipping composite_plan hint (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let template_tenant = create_tenant(TEMPLATE_TENANT);
    let tenant = create_tenant("composite-plan-hint");
    create_observation(
        &backend,
        &template_tenant,
        height("match", "8302-2", 170.0, "final"),
    )
    .await;
    let db = raw_database(&backend).await;
    let template = read_template(&db, TEMPLATE_TENANT, "match").await;
    replicate(
        &db,
        &template,
        "composite-plan-hint",
        (0..600).map(|n| (format!("h{n:04}"), BASE_MILLIS + n)),
        None,
    )
    .await;

    // `:not` keeps the collector, whose driver path pair-checks 512-id batches.
    let mut cancelled = parameter_token("status", "cancelled");
    cancelled.modifier = Some(SearchModifier::Not);
    let legacy = code_value_quantity_query("8302-2$gt160")
        .with_parameter(cancelled)
        .with_count(1000);
    let (result, operations) = profiled(&db, async {
        backend.search(&tenant, &legacy).await.unwrap()
    })
    .await;
    assert_eq!(result.resources.items.len(), 600);
    assert!(
        pair_checks(&operations).len() >= 4,
        "600 candidates take two batches of two component reads"
    );
    assert_pair_checks_are_hinted(&operations, 512, "collector driver path");

    // The planner's candidates are checked in one bounded read per component.
    let planned = code_value_quantity_query("8302-2$gt160").with_count(1000);
    let (result, operations) = profiled(&db, async {
        backend.search(&tenant, &planned).await.unwrap()
    })
    .await;
    assert_eq!(result.resources.items.len(), 600);
    assert_pair_checks_are_hinted(&operations, 10_000, "planned path");
}

fn parameter_token(name: &str, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    }
}

fn subject_observation(id: &str, patient: &str, status: &str) -> serde_json::Value {
    json!({
        "resourceType": "Observation", "id": id, "status": status,
        "code": {"coding": [{"system": LOINC, "code": "8302-2"}]},
        "subject": {"reference": format!("Patient/{patient}")},
    })
}

#[tokio::test]
async fn mongodb_identifier_and_compartment_searches_plan_from_the_index() {
    let Some(backend) = create_backend_with_stable_probes("composite_plan_refs").await else {
        eprintln!("Skipping composite_plan refs (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let template_tenant = create_tenant(TEMPLATE_TENANT);
    let tenant = create_tenant("composite-plan-refs");
    for (id, identifier) in [("p1", "A1"), ("p2", "B2")] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": id,
                    "identifier": [{"system": "urn:mrn", "value": identifier}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    for (id, status) in [
        ("o1", "final"),
        ("o2", "final"),
        ("o3", "final"),
        ("o4", "preliminary"),
        ("o5", "preliminary"),
    ] {
        create_observation(&backend, &tenant, subject_observation(id, "p1", status)).await;
    }
    create_observation(
        &backend,
        &template_tenant,
        subject_observation("other", "p2", "final"),
    )
    .await;
    let db = raw_database(&backend).await;
    let template = read_template(&db, TEMPLATE_TENANT, "other").await;
    replicate(
        &db,
        &template,
        "composite-plan-refs",
        (0..1_500).map(|n| (format!("q{n:04}"), BASE_MILLIS + n)),
        None,
    )
    .await;

    let identified = |value: &str| {
        SearchQuery::new("Observation")
            .with_parameter(SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                modifier: Some(SearchModifier::Identifier),
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![],
            })
            .with_count(50)
    };
    let mut query = identified("urn:mrn|A1");
    query.total = Some(TotalMode::Accurate);
    let result = backend.search(&tenant, &query).await.unwrap();
    let mut found = ids_of(&result);
    found.sort();
    assert_eq!(found, ["o1", "o2", "o3", "o4", "o5"]);
    assert_eq!(result.total, Some(5));
    assert_eq!(backend.search_count(&tenant, &query).await.unwrap(), 5);

    let mut unknown = identified("urn:mrn|none");
    unknown.total = Some(TotalMode::Accurate);
    let result = backend.search(&tenant, &unknown).await.unwrap();
    assert!(result.resources.items.is_empty());
    assert_eq!(result.total, Some(0));
    assert_eq!(backend.search_count(&tenant, &unknown).await.unwrap(), 0);

    let mut member = SearchQuery::new("Observation").with_count(50);
    member.compartment = Some(CompartmentMembership {
        params: vec!["subject".to_string(), "performer".to_string()],
        reference: "Patient/p1".to_string(),
    });
    member.total = Some(TotalMode::Accurate);
    let result = backend.search(&tenant, &member).await.unwrap();
    assert_eq!(result.total, Some(5));
    assert_eq!(result.resources.items.len(), 5);

    // The broad sibling is checked against the compartment's five ids only.
    let broad = member
        .clone()
        .with_parameter(parameter_token("status", "final"));
    let (result, operations) = profiled(&db, async {
        backend.search(&tenant, &broad).await.unwrap()
    })
    .await;
    let mut found = ids_of(&result);
    found.sort();
    assert_eq!(found, ["o1", "o2", "o3"]);
    assert_eq!(result.total, Some(3));
    let status_checks: Vec<_> = operations
        .iter()
        .filter(|operation| {
            command_of(operation).get_str("aggregate") == Ok("search_index")
                && command_of(operation)
                    .get_array("pipeline")
                    .map(|pipeline| Bson::Array(pipeline.clone()))
                    .is_ok_and(|pipeline| {
                        mentions_param(&pipeline, "status")
                            && candidate_list_length(&pipeline).is_some()
                    })
        })
        .collect();
    assert!(
        !status_checks.is_empty(),
        "the status check must be profiled"
    );
    for check in status_checks {
        let ids = candidate_list_length(&Bson::Array(
            command_of(check).get_array("pipeline").unwrap().clone(),
        ))
        .unwrap();
        assert!(ids <= 5, "status was checked against {ids} ids");
        let keys = check
            .get_i64("keysExamined")
            .or_else(|_| check.get_i32("keysExamined").map(i64::from))
            .expect("a membership check reports its keys");
        assert!(keys <= 50, "status membership examined {keys} keys");
    }
}

fn mentions_param(value: &Bson, name: &str) -> bool {
    match value {
        Bson::Document(document) => {
            document.get_str("param_name") == Ok(name)
                || document.values().any(|value| mentions_param(value, name))
        }
        Bson::Array(values) => values.iter().any(|value| mentions_param(value, name)),
        _ => false,
    }
}

/// Measures the shapes above the collector's 300,000-id cap on the local
/// MongoDB: the collector refuses, the planned searches answer.
///
/// Run with
/// `cargo test -p helios-persistence --features mongodb --test mongodb_tests \
///  composite_over_cap_harness -- --ignored --nocapture`.
/// `HFS_TEST_COMPOSITE_CAP_ROWS` sets the row count (default 300,100) and
/// `HFS_TEST_COMPOSITE_CAP_MONGODB_URL` a server other than the shared one.
#[tokio::test]
#[ignore = "#1896 over-cap measurement harness; run with --ignored"]
async fn composite_over_cap_harness() {
    use std::time::Instant;

    let rows: i64 = std::env::var("HFS_TEST_COMPOSITE_CAP_ROWS")
        .ok()
        .and_then(|rows| rows.parse().ok())
        .unwrap_or(300_100);
    let connection_string = match std::env::var("HFS_TEST_COMPOSITE_CAP_MONGODB_URL") {
        Ok(url) => url,
        Err(_) => match shared_mongo::connection_string().await {
            Some(url) => url,
            None => {
                eprintln!("Skipping composite_over_cap_harness (no MongoDB available)");
                return;
            }
        },
    };
    let Some(backend) = build_backend(MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name("composite_over_cap"),
        data_dir: Some(repo_data_dir()),
        probe_timeout_ms: 10_000,
        ..Default::default()
    })
    .await
    else {
        eprintln!("Skipping composite_over_cap_harness (backend unavailable)");
        return;
    };
    let template_tenant = create_tenant(TEMPLATE_TENANT);
    let tenant = create_tenant("composite-over-cap");
    create_observation(
        &backend,
        &template_tenant,
        height("match", "8302-2", 170.0, "final"),
    )
    .await;
    let db = raw_database(&backend).await;
    let template = read_template(&db, TEMPLATE_TENANT, "match").await;
    let started = Instant::now();
    replicate(
        &db,
        &template,
        "composite-over-cap",
        (0..rows).map(|n| (format!("r{n:07}"), BASE_MILLIS + n)),
        Some(&["code-value-quantity", "status"]),
    )
    .await;
    eprintln!("seeded {rows} rows in {} ms", started.elapsed().as_millis());

    let mut cancelled = parameter_token("status", "cancelled");
    cancelled.modifier = Some(SearchModifier::Not);
    let control = code_value_quantity_query("8302-2$gt160").with_parameter(cancelled);
    let started = Instant::now();
    let refused = backend.search(&tenant, &control).await;
    eprintln!(
        "control (:not keeps the collector): {} ms",
        started.elapsed().as_millis()
    );
    if rows > 300_000 {
        assert!(
            matches!(
                refused,
                Err(StorageError::Search(SearchError::TooManyResults { .. }))
            ),
            "the collector must refuse above its cap, got {:?}",
            refused.map(|result| result.resources.items.len())
        );
    }

    let newest: Vec<String> = (0..rows)
        .rev()
        .take(20)
        .map(|n| format!("r{n:07}"))
        .collect();
    let oldest: Vec<String> = (0..20).map(|n| format!("r{n:07}")).collect();
    let page = code_value_quantity_query("8302-2$gt160").with_count(20);
    let started = Instant::now();
    let result = backend.search(&tenant, &page).await.unwrap();
    eprintln!("default-sort page: {} ms", started.elapsed().as_millis());
    assert_eq!(ids_of(&result), newest);
    let by_id = page.clone().with_sort(SortDirective::parse("_id"));
    let started = Instant::now();
    let result = backend.search(&tenant, &by_id).await.unwrap();
    eprintln!("_sort=_id page: {} ms", started.elapsed().as_millis());
    assert_eq!(ids_of(&result), oldest);
    let mut accurate = by_id.clone();
    accurate.total = Some(TotalMode::Accurate);
    let started = Instant::now();
    let result = backend.search(&tenant, &accurate).await.unwrap();
    eprintln!("_total=accurate page: {} ms", started.elapsed().as_millis());
    assert_eq!(ids_of(&result), oldest);
    assert_eq!(result.total, Some(rows as u64));
    let started = Instant::now();
    let count = backend.search_count(&tenant, &page).await.unwrap();
    eprintln!("search_count: {} ms", started.elapsed().as_millis());
    assert_eq!(count, rows as u64);
    db.drop().await.unwrap();
}

/// Times composite pages across pair-check pass rates on the local MongoDB
/// (#2262): one tenant per rate, where a value of 170 passes `8302-2$gt160`
/// and 150 fails it. A selective composite drives from its small value arm, so
/// this shows which arms keep the grouped page and which read ordered windows.
///
/// Seed once, then time, with
/// `HFS_TEST_COMPOSITE_RATE_SEED=1 cargo test -p helios-persistence \
///  --features mongodb --test mongodb_tests composite_pass_rate_harness -- \
///  --ignored --nocapture`, and the same without `HFS_TEST_COMPOSITE_RATE_SEED`.
/// `HFS_TEST_COMPOSITE_RATE_MONGODB_URL` names the server (required: the seeded
/// database outlives the run), `HFS_TEST_COMPOSITE_RATE_ROWS` the rows per
/// tenant (default 200,000), `HFS_TEST_COMPOSITE_RATE_REPS` the timed runs per
/// shape (default 5) and `HFS_TEST_COMPOSITE_RATE_TENANTS` a comma list of
/// tenants to time.
#[tokio::test]
#[ignore = "#2262 pass-rate measurement harness; run with --ignored"]
async fn composite_pass_rate_harness() {
    use std::time::Instant;

    let Ok(connection_string) = std::env::var("HFS_TEST_COMPOSITE_RATE_MONGODB_URL") else {
        eprintln!("Skipping composite_pass_rate_harness (no HFS_TEST_COMPOSITE_RATE_MONGODB_URL)");
        return;
    };
    let rows: i64 = std::env::var("HFS_TEST_COMPOSITE_RATE_ROWS")
        .ok()
        .and_then(|rows| rows.parse().ok())
        .unwrap_or(200_000);
    let reps: usize = std::env::var("HFS_TEST_COMPOSITE_RATE_REPS")
        .ok()
        .and_then(|reps| reps.parse().ok())
        .unwrap_or(5);
    let Some(backend) = build_backend(MongoBackendConfig {
        connection_string,
        database_name: "composite_pass_rate".into(),
        data_dir: Some(repo_data_dir()),
        probe_timeout_ms: 10_000,
        ..Default::default()
    })
    .await
    else {
        eprintln!("Skipping composite_pass_rate_harness (backend unavailable)");
        return;
    };
    let db = raw_database(&backend).await;
    // Each tenant's name is its pass rate: row `n` passes when
    // `n % modulus < passing`.
    let tenants: [(&str, i64, i64); 6] = [
        ("p100", 1, 1),
        ("p50", 2, 1),
        ("p30", 10, 3),
        ("p5", 20, 1),
        ("p1", 100, 1),
        ("p01", 1000, 1),
    ];
    if std::env::var("HFS_TEST_COMPOSITE_RATE_SEED").is_ok() {
        let template_tenant = create_tenant(TEMPLATE_TENANT);
        create_observation(
            &backend,
            &template_tenant,
            height("pass", "8302-2", 170.0, "final"),
        )
        .await;
        create_observation(
            &backend,
            &template_tenant,
            height("fail", "8302-2", 150.0, "final"),
        )
        .await;
        let pass = read_template(&db, TEMPLATE_TENANT, "pass").await;
        let fail = read_template(&db, TEMPLATE_TENANT, "fail").await;
        let keep: &[&str] = &["code-value-quantity", "status", "code", "value-quantity"];
        for (tenant, modulus, passing) in tenants {
            let started = Instant::now();
            for (template, wanted) in [(&pass, true), (&fail, false)] {
                replicate(
                    &db,
                    template,
                    tenant,
                    (0..rows)
                        .filter(|n| (n % modulus < passing) == wanted)
                        .map(|n| (format!("r{n:07}"), BASE_MILLIS + n)),
                    Some(keep),
                )
                .await;
            }
            eprintln!("seeded {tenant} in {} ms", started.elapsed().as_millis());
        }
        return;
    }

    let only = std::env::var("HFS_TEST_COMPOSITE_RATE_TENANTS").ok();
    let by_id = |query: SearchQuery, count: u32, sort: &str| {
        query
            .with_count(count)
            .with_sort(SortDirective::parse(sort))
    };
    let plain_value = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "value-quantity".to_string(),
        param_type: SearchParamType::Quantity,
        modifier: None,
        values: vec![SearchValue::parse("gt160")],
        chain: vec![],
        components: vec![],
    });
    let shapes = [
        (
            "_sort=_id",
            by_id(code_value_quantity_query("8302-2$gt160"), 20, "_id"),
        ),
        (
            "_sort=-_id",
            by_id(code_value_quantity_query("8302-2$gt160"), 20, "-_id"),
        ),
        (
            "_sort=_id _count=100",
            by_id(code_value_quantity_query("8302-2$gt160"), 100, "_id"),
        ),
        (
            "_sort=_id two values",
            by_id(
                code_value_quantity_query_values(&["8302-2$gt160", "8302-2$lt100"]),
                20,
                "_id",
            ),
        ),
        (
            "default sort",
            code_value_quantity_query("8302-2$gt160").with_count(20),
        ),
        (
            "plain value-quantity _sort=_id",
            by_id(plain_value, 20, "_id"),
        ),
    ];
    for (tenant, _, _) in tenants {
        if only
            .as_deref()
            .is_some_and(|only| !only.split(',').any(|name| name == tenant))
        {
            continue;
        }
        let context = create_tenant(tenant);
        for (name, query) in &shapes {
            let mut times = Vec::with_capacity(reps);
            let mut summary = String::new();
            for _ in 0..reps {
                let started = Instant::now();
                let result = backend.search(&context, query).await.unwrap();
                times.push(started.elapsed().as_secs_f64() * 1000.0);
                let ids = ids_of(&result);
                summary = format!(
                    "n={} first={:?} last={:?} next={}",
                    ids.len(),
                    ids.first(),
                    ids.last(),
                    result.resources.page_info.has_next
                );
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "{tenant}\t{name}\tmedian={:.1} ms\tmin={:.1} ms\t{summary}",
                times[times.len() / 2],
                times[0],
            );
        }
    }
}
