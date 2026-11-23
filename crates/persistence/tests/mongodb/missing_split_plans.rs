//! #2233 plan tests: the exact total of a pure metadata `:missing=true` search
//! is two counts, and each of them has one candidate plan with fixed bounds.
//!
//! The commands are captured with the profiler (so they are the ones the
//! product really issues), then re-run through `explain` with
//! `executionStats`. Every assertion reads the explain output, never the
//! profiler entries.
//!
//! * stamped count: no `$lookup`; a single name is a `COUNT_SCAN` that
//!   examines no document; the bound on `missing_params` is the name itself;
//! * unstamped count: bound `[null, null]` on `missing_params`, and no more
//!   `$lookup` executions than there are unstamped resources;
//! * no plan is bounded `missing_params: [MinKey, MaxKey]`, including the two
//!   full-scan cases of today's pipeline: a name no resource lacks in a type
//!   with unstamped resources, and pairs of names in a fully stamped type;
//! * each command has a single candidate plan (no rejected plans).

use super::*;

const NAMES: [&str; 3] = ["sp-a", "sp-b", "sp-c"];
const URLS: [&str; 3] = ["urn:hfs:sp:a", "urn:hfs:sp:b", "urn:hfs:sp:c"];
const MISSING_INDEX: &str = "idx_resources_missing_params";
const DOCS: usize = 300;

/// Rows of resource `n`: `sp-a` is absent from every fifth, `sp-b` is present
/// on every fifth only, `sp-c` is present everywhere except every hundredth
/// (and nowhere absent at all when `narrow_c` is false).
fn rows(n: usize, narrow_c: bool) -> [bool; 3] {
    [
        !n.is_multiple_of(5),
        n.is_multiple_of(5),
        !narrow_c || !n.is_multiple_of(100),
    ]
}

/// Every fiftieth resource (2%) loses its `missing_params` array.
fn unstamped(n: usize) -> bool {
    n % 50 == 7
}

fn id_of(n: usize) -> String {
    format!("o{n:04}")
}

fn observation(n: usize, narrow_c: bool) -> serde_json::Value {
    let extensions: Vec<_> = rows(n, narrow_c)
        .iter()
        .enumerate()
        .filter(|(_, present)| **present)
        .map(|(i, _)| json!({"url": URLS[i], "valueString": "present"}))
        .collect();
    json!({
        "resourceType": "Observation", "id": id_of(n), "status": "final",
        "code": {"coding": [{"code": "fixture"}]}, "extension": extensions,
    })
}

async fn register_parameters(backend: &MongoBackend, tenant: &TenantContext) {
    for (name, url) in NAMES.iter().zip(URLS) {
        backend
            .seed_create(
                tenant,
                "SearchParameter",
                json!({
                    "resourceType": "SearchParameter", "id": name,
                    "url": format!("urn:hfs:test:{name}"), "name": name,
                    "status": "active", "code": name, "base": ["Observation"],
                    "type": "string",
                    "expression": format!(
                        "Observation.extension.where(url = '{url}').value"
                    ),
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
}

/// Writes `DOCS` Observations through the real path. With `drop_stamps`, the
/// 2% of resources picked by [`unstamped`] then lose `missing_params`.
async fn seed(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &TenantContext,
    narrow_c: bool,
    drop_stamps: bool,
) {
    register_parameters(backend, tenant).await;
    for n in 0..DOCS {
        backend
            .seed_create(
                tenant,
                "Observation",
                observation(n, narrow_c),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    if drop_stamps {
        let ids: Vec<String> = (0..DOCS).filter(|n| unstamped(*n)).map(id_of).collect();
        let result = db
            .collection::<Document>("resources")
            .update_many(
                doc! {
                    "tenant_id": tenant.tenant_id().as_str(),
                    "resource_type": "Observation",
                    "id": { "$in": ids.clone() },
                },
                doc! { "$unset": { "missing_params": "" } },
            )
            .await
            .unwrap();
        assert_eq!(result.matched_count as usize, ids.len());
    }
}

fn missing_true(names: &[usize]) -> SearchQuery {
    let mut query = SearchQuery::new("Observation").with_count(1);
    for name in names {
        query = query.with_parameter(SearchParameter {
            name: NAMES[*name].to_owned(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("true")],
            chain: vec![],
            components: vec![],
        });
    }
    query.total = Some(TotalMode::Accurate);
    query
}

/// Resources that lack every one of `names`, by the rows they were written with.
fn expected_total(names: &[usize], narrow_c: bool) -> u64 {
    (0..DOCS)
        .filter(|n| names.iter().all(|i| !rows(*n, narrow_c)[*i]))
        .count() as u64
}

fn is_count_on_resources(command: &Document) -> bool {
    command.get_str("aggregate") == Ok("resources")
        && command.get_array("pipeline").is_ok_and(|pipeline| {
            pipeline
                .last()
                .and_then(Bson::as_document)
                .is_some_and(|stage| stage.contains_key("$count"))
        })
}

fn is_unstamped_count(command: &Document) -> bool {
    format!("{:?}", command.get_array("pipeline").unwrap()[0]).contains("search_rows_pending")
}

/// The count commands a search issues, read from the profiler.
async fn captured_counts(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &TenantContext,
    names: &[usize],
) -> (u64, Vec<Document>) {
    let query = missing_true(names);
    let (total, operations) =
        composite_plan::profiled(db, async { backend.search_count(tenant, &query).await }).await;
    let commands = operations
        .iter()
        .filter(|operation| operation.get_str("op") == Ok("command"))
        .map(|operation| composite_plan::command_of(operation).clone())
        .filter(is_count_on_resources)
        .collect();
    (total.unwrap(), commands)
}

async fn explain(db: &mongodb::Database, command: &Document) -> Document {
    db.run_command(doc! {
        "explain": command_for_explain(command), "verbosity": "executionStats",
    })
    .await
    .unwrap()
}

/// The `$cursor` stage of the explained aggregation, or the whole explain when
/// the engine answers the count without stages.
fn source(explained: &Document) -> Document {
    explained
        .get_array("stages")
        .ok()
        .and_then(|stages| {
            stages
                .iter()
                .find_map(|stage| stage.as_document()?.get_document("$cursor").ok().cloned())
        })
        .unwrap_or_else(|| explained.clone())
}

/// Every plan stage (a document with a `stage` name), whatever the nesting.
fn plan_stages(value: &Bson, out: &mut Vec<Document>) {
    match value {
        Bson::Document(document) => {
            if document.get_str("stage").is_ok() {
                out.push(document.clone());
            }
            for inner in document.values() {
                plan_stages(inner, out);
            }
        }
        Bson::Array(values) => values.iter().for_each(|inner| plan_stages(inner, out)),
        _ => {}
    }
}

fn number(document: &Document, key: &str) -> i64 {
    document
        .get_i64(key)
        .or_else(|_| document.get_i32(key).map(i64::from))
        .unwrap_or_else(|_| panic!("{key} missing in {document:?}"))
}

/// The bound a stage puts on `missing_params`, as text.
fn missing_params_bound(stage: &Document) -> Option<String> {
    match stage.get_str("stage").ok()? {
        "IXSCAN" => stage
            .get_document("indexBounds")
            .ok()?
            .get("missing_params")
            .map(|bound| format!("{bound:?}")),
        "COUNT_SCAN" => {
            let bounds = stage.get_document("indexBounds").ok()?;
            let key = |name: &str| {
                bounds
                    .get_document(name)
                    .ok()
                    .and_then(|key| key.get("missing_params"))
                    .map(|value| format!("{value:?}"))
            };
            Some(format!("{}..{}", key("startKey")?, key("endKey")?))
        }
        _ => None,
    }
}

struct Inspected {
    stages: Vec<Document>,
    bounds: Vec<String>,
    docs_examined: i64,
    lookups: Vec<i64>,
}

fn inspect(label: &str, explained: &Document) -> Inspected {
    let source = source(explained);
    let planner = source
        .get_document("queryPlanner")
        .unwrap_or_else(|_| panic!("{label}: no queryPlanner in {explained:?}"));
    let mut winning = Vec::new();
    plan_stages(planner.get("winningPlan").unwrap(), &mut winning);
    let rejected = planner.get_array("rejectedPlans").map_or(0, Vec::len);
    assert_eq!(
        rejected, 0,
        "{label}: more than one candidate plan: {planner:?}"
    );

    assert!(
        winning
            .iter()
            .all(|stage| stage.get_str("stage") != Ok("COLLSCAN")),
        "{label}: collection scan: {planner:?}"
    );
    let mut bounds = Vec::new();
    for stage in &winning {
        if matches!(stage.get_str("stage"), Ok("IXSCAN" | "COUNT_SCAN")) {
            assert_eq!(
                stage.get_str("indexName"),
                Ok(MISSING_INDEX),
                "{label}: scan outside the missing_params index: {stage:?}"
            );
        }
        if let Some(bound) = missing_params_bound(stage) {
            assert!(
                !bound.contains("MinKey") && !bound.contains("MaxKey"),
                "{label}: unbounded missing_params scan {bound}: {stage:?}"
            );
            bounds.push(bound);
        }
    }
    assert!(
        !bounds.is_empty(),
        "{label}: no missing_params bound: {planner:?}"
    );

    let docs_examined = number(
        source
            .get_document("executionStats")
            .unwrap_or_else(|_| panic!("{label}: no executionStats in {explained:?}")),
        "totalDocsExamined",
    );
    let lookups = explained
        .get_array("stages")
        .map(|stages| {
            stages
                .iter()
                .filter_map(Bson::as_document)
                .filter(|stage| stage.contains_key("$lookup"))
                .map(|stage| number(stage, "nReturned"))
                .collect()
        })
        .unwrap_or_default();
    Inspected {
        stages: winning,
        bounds,
        docs_examined,
        lookups,
    }
}

/// Runs the search, checks the two commands and returns their inspections.
async fn check_query(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &TenantContext,
    names: &[usize],
    narrow_c: bool,
    unstamped_resources: i64,
) -> (Inspected, Inspected) {
    let label = format!("{names:?}");
    let (total, commands) = captured_counts(backend, db, tenant, names).await;
    assert_eq!(
        total,
        expected_total(names, narrow_c),
        "{label}: total differs from the rows"
    );
    assert_eq!(
        commands.len(),
        2,
        "{label}: the exact total is two count commands: {commands:?}"
    );
    let stamped_command = commands.iter().find(|c| !is_unstamped_count(c)).unwrap();
    let unstamped_command = commands.iter().find(|c| is_unstamped_count(c)).unwrap();
    for command in [stamped_command, unstamped_command] {
        assert!(
            format!("{:?}", command.get("hint")).contains(MISSING_INDEX),
            "{label}: count without the missing_params hint: {command:?}"
        );
    }

    let stamped = inspect(
        &format!("{label} stamped"),
        &explain(db, stamped_command).await,
    );
    assert!(
        stamped.lookups.is_empty(),
        "{label}: stamped count runs $lookup stages {:?}",
        stamped.lookups
    );
    assert!(
        stamped
            .bounds
            .iter()
            .any(|bound| bound.contains(NAMES[names[0]])),
        "{label}: stamped bound is not the first name: {:?}",
        stamped.bounds
    );

    let unstamped = inspect(
        &format!("{label} unstamped"),
        &explain(db, unstamped_command).await,
    );
    assert!(
        unstamped
            .bounds
            .iter()
            .all(|bound| bound.contains("null") && !bound.contains(NAMES[names[0]])),
        "{label}: unstamped bound is not [null, null]: {:?}",
        unstamped.bounds
    );
    assert_eq!(unstamped.lookups.len(), names.len(), "{label}: lookups");
    for executed in &unstamped.lookups {
        assert!(
            *executed <= unstamped_resources,
            "{label}: {executed} $lookup executions for {unstamped_resources} unstamped resources"
        );
    }
    (stamped, unstamped)
}

#[tokio::test]
async fn mongodb_missing_split_single_name_counts_without_fetching() {
    let Some(backend) = create_backend("missing_split_single").await else {
        return;
    };
    let tenant = create_tenant("missing-split-single");
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    seed(&backend, &db, &tenant, false, true).await;
    let unstamped_resources = (0..DOCS).filter(|n| unstamped(*n)).count() as i64;

    for names in [vec![0usize], vec![1]] {
        let (stamped, unstamped) =
            check_query(&backend, &db, &tenant, &names, false, unstamped_resources).await;
        assert!(
            stamped
                .stages
                .iter()
                .any(|stage| stage.get_str("stage") == Ok("COUNT_SCAN")),
            "{names:?}: single-name stamped count is not a COUNT_SCAN: {:?}",
            stamped.stages
        );
        assert_eq!(stamped.docs_examined, 0, "{names:?}: stamped documents");
        assert!(
            unstamped
                .stages
                .iter()
                .any(|stage| stage.get_str("stage") == Ok("IXSCAN")),
            "{names:?}: unstamped count is not an index scan"
        );
    }
}

/// A name no resource lacks, in a type where 2% of resources have no stamp:
/// today's pipeline scans `missing_params` end to end for a total of 0.
#[tokio::test]
async fn mongodb_missing_split_name_nobody_lacks_is_not_a_full_scan() {
    let Some(backend) = create_backend("missing_split_none").await else {
        return;
    };
    let tenant = create_tenant("missing-split-none");
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    seed(&backend, &db, &tenant, false, true).await;
    let unstamped_resources = (0..DOCS).filter(|n| unstamped(*n)).count() as i64;

    let (stamped, _) = check_query(&backend, &db, &tenant, &[2], false, unstamped_resources).await;
    assert_eq!(stamped.docs_examined, 0);
}

/// Two names on a fully stamped type, narrow name first and last: no plan
/// reads `missing_params` unbounded, and nothing is `$lookup`ed.
#[tokio::test]
async fn mongodb_missing_split_name_pairs_on_a_stamped_type_are_bounded() {
    let Some(backend) = create_backend("missing_split_pairs").await else {
        return;
    };
    let tenant = create_tenant("missing-split-pairs");
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    seed(&backend, &db, &tenant, true, false).await;

    for names in [
        vec![2usize, 0],
        vec![0, 2],
        vec![2, 1],
        vec![1, 2],
        vec![0, 1],
        vec![0, 1, 2],
    ] {
        let (_, unstamped) = check_query(&backend, &db, &tenant, &names, true, 0).await;
        assert!(
            unstamped.lookups.iter().all(|executed| *executed == 0),
            "{names:?}: nothing is unstamped, so nothing is looked up"
        );
    }
}
