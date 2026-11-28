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
//!
//! The mixed totals of Stage 2 (a metadata `:missing=true` with `:not` or
//! `:missing=false`) are pinned the same way: the stamped stream and every
//! membership check have one bounded plan and no `$lookup`, and only the
//! unstamped count keeps the plan's stages. With a non-primary read
//! preference every total stays the single aggregation.

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

/// The `total_strategy` values the `helios_mongodb_plan` events of one
/// search recorded.
#[derive(Clone, Default)]
struct TotalStrategies(Arc<std::sync::Mutex<Vec<String>>>);

#[derive(Default)]
struct TotalVisitor(Option<String>);

impl tracing::field::Visit for TotalVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "total_strategy" {
            self.0 = Some(value.to_owned());
        }
    }
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl tracing::Subscriber for TotalStrategies {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "helios_mongodb_plan"
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::DEBUG)
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = TotalVisitor::default();
        event.record(&mut visitor);
        if let Some(total) = visitor.0 {
            self.0.lock().unwrap().push(total);
        }
    }
}

/// `search_count` of `query` on `backend`, with the total strategies it
/// recorded and the `$count` commands on `resources` the profiler saw.
async fn routed_total(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> (u64, Vec<String>, Vec<Document>) {
    pin_tracing_interest();
    let strategies = TotalStrategies::default();
    let dispatch = tracing::Dispatch::new(strategies.clone());
    let (total, operations) = {
        let _guard = tracing::dispatcher::set_default(&dispatch);
        composite_plan::profiled(db, backend.search_count(tenant, query)).await
    };
    let counts = operations
        .iter()
        .filter(|operation| operation.get_str("op") == Ok("command"))
        .map(|operation| composite_plan::command_of(operation).clone())
        .filter(is_count_on_resources)
        .collect();
    let strategies = strategies.0.lock().unwrap().clone();
    (total.unwrap(), strategies, counts)
}

/// `connection_string` with one more URI option.
fn with_uri_option(connection_string: &str, option: &str) -> String {
    match connection_string.split_once('?') {
        Some((base, query)) if !query.is_empty() => format!("{base}?{query}&{option}"),
        Some((base, _)) => format!("{base}?{option}"),
        None if connection_string.ends_with('/') => format!("{connection_string}?{option}"),
        None => format!("{connection_string}/?{option}"),
    }
}

/// With a non-primary read preference, the commands of a split total could
/// be served by different members, so the total stays today's single
/// aggregation: one `$count` command, the `exact_aggregation` strategy, and
/// the same number as the split of a primary-only backend on the same data,
/// for pure (Stage 1) and mixed (Stage 2) totals.
/// A single-node replica set serves `secondaryPreferred` from its primary.
#[tokio::test]
async fn mongodb_missing_split_secondary_reads_keep_one_aggregation() {
    let Some(backend) = create_backend("missing_split_secondary").await else {
        return;
    };
    let tenant = create_tenant("missing-split-secondary");
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    seed(&backend, &db, &tenant, false, true).await;
    let mut config = backend.config().clone();
    config.connection_string = with_uri_option(
        &config.connection_string,
        "readPreference=secondaryPreferred",
    );
    let Some(secondary) = build_backend(config).await else {
        return;
    };

    let mut mixed = missing_true(&[0]);
    mixed.parameters.push(SearchParameter {
        name: NAMES[2].to_owned(),
        param_type: SearchParamType::String,
        modifier: Some(SearchModifier::Missing),
        values: vec![SearchValue::eq("false")],
        chain: vec![],
        components: vec![],
    });
    let cases = [
        (
            "[0]",
            missing_true(&[0]),
            expected_total(&[0], false),
            "exact_missing_split",
        ),
        (
            "[0, 1]",
            missing_true(&[0, 1]),
            expected_total(&[0, 1], false),
            "exact_missing_split",
        ),
        (
            "[0] + sp-c:missing=false",
            mixed,
            (0..DOCS)
                .filter(|n| !rows(*n, false)[0] && rows(*n, false)[2])
                .count() as u64,
            "exact_missing_checks",
        ),
    ];
    for (label, query, expected, primary_strategy) in cases {
        let (primary, strategies, _) = routed_total(&backend, &db, &tenant, &query).await;
        assert_eq!(
            strategies,
            [primary_strategy.to_owned()],
            "{label}: primary-only route"
        );
        let (total, strategies, counts) = routed_total(&secondary, &db, &tenant, &query).await;
        let label = format!("{label} secondaryPreferred");
        assert_eq!(
            strategies,
            ["exact_aggregation".to_owned()],
            "{label}: total strategy"
        );
        assert_eq!(counts.len(), 1, "{label}: one aggregation: {counts:?}");
        assert!(
            format!("{:?}", counts[0]).contains("$or"),
            "{label}: the aggregation is the single `$or` count: {counts:?}"
        );
        assert_eq!(total, primary, "{label}: total differs from the split's");
        assert_eq!(total, expected, "{label}: rows");
    }
}

// ---------------------------------------------------------------------------
// #2233 Stage 2: mixed totals (stamped stream + membership checks)
// ---------------------------------------------------------------------------

const DATE_NAME: &str = "sp-d";
const DATE_URL: &str = "urn:hfs:sp:d";

/// `sp-d` of resource `n`: the first day of 2026 on every sixth resource, the
/// second day on every other third one, absent elsewhere.
fn date_of(n: usize) -> Option<&'static str> {
    match n % 6 {
        0 => Some("2026-01-01T12:00:00Z"),
        3 => Some("2026-01-02T12:00:00Z"),
        _ => None,
    }
}

/// [`seed`] plus a date parameter `sp-d` ([`date_of`]), so a date `:not`
/// (whose filter is a top-level `$or` of scoped arms) is pinned too.
async fn seed_mixed(backend: &MongoBackend, db: &mongodb::Database, tenant: &TenantContext) {
    register_parameters(backend, tenant).await;
    backend
        .seed_create(
            tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter", "id": DATE_NAME,
                "url": format!("urn:hfs:test:{DATE_NAME}"), "name": DATE_NAME,
                "status": "active", "code": DATE_NAME, "base": ["Observation"],
                "type": "date",
                "expression": format!("Observation.extension.where(url = '{DATE_URL}').value"),
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    for n in 0..DOCS {
        let mut body = observation(n, false);
        if let Some(date) = date_of(n) {
            body["extension"]
                .as_array_mut()
                .unwrap()
                .push(json!({"url": DATE_URL, "valueDateTime": date}));
        }
        backend
            .seed_create(tenant, "Observation", body, FhirVersion::default())
            .await
            .unwrap();
    }
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

fn modified(
    name: &str,
    kind: SearchParamType,
    modifier: SearchModifier,
    value: &str,
) -> SearchParameter {
    SearchParameter {
        name: name.to_owned(),
        param_type: kind,
        modifier: Some(modifier),
        values: vec![SearchValue::parse(value)],
        chain: vec![],
        components: vec![],
    }
}

/// `sp-a:missing=true&sp-c:missing=false&sp-d:not=2026-01-01&sp-b:not=other`.
/// The date `:not` excludes some resources that lack `sp-a`; no resource has
/// `sp-b=other`, so that check keeps every candidate and still runs.
fn mixed_query() -> SearchQuery {
    let mut query = missing_true(&[0]);
    query.parameters.push(modified(
        NAMES[2],
        SearchParamType::String,
        SearchModifier::Missing,
        "false",
    ));
    query.parameters.push(modified(
        DATE_NAME,
        SearchParamType::Date,
        SearchModifier::Not,
        "2026-01-01",
    ));
    query.parameters.push(modified(
        NAMES[1],
        SearchParamType::String,
        SearchModifier::Not,
        "other",
    ));
    query
}

/// The criteria of [`mixed_query`] other than the metadata `:missing=true`.
const MIXED_CRITERIA: i64 = 3;

fn mixed_expected() -> u64 {
    (0..DOCS)
        .filter(|n| {
            let present = rows(*n, false);
            !present[0] && present[2] && *n % 6 != 0
        })
        .count() as u64
}

/// Every `IXSCAN` of an explained plan.
fn index_scans(planner: &Document) -> Vec<Document> {
    let mut stages = Vec::new();
    plan_stages(planner.get("winningPlan").unwrap(), &mut stages);
    stages
        .into_iter()
        .filter(|stage| stage.get_str("stage") == Ok("IXSCAN"))
        .collect()
}

fn planner_of(label: &str, explained: &Document) -> Document {
    let source = source(explained);
    let planner = source
        .get_document("queryPlanner")
        .unwrap_or_else(|_| panic!("{label}: no queryPlanner in {explained:?}"))
        .clone();
    assert_eq!(
        planner.get_array("rejectedPlans").map_or(0, Vec::len),
        0,
        "{label}: more than one candidate plan: {planner:?}"
    );
    planner
}

/// Point intervals `[v, v]` of one key of an index scan, or `None` when a
/// bound is a range.
fn points(stage: &Document, key: &str) -> Option<usize> {
    let bounds = stage
        .get_document("indexBounds")
        .ok()?
        .get_array(key)
        .ok()?;
    bounds
        .iter()
        .all(|bound| {
            let text = bound.as_str().unwrap_or("");
            let inner = text.trim_start_matches('[').trim_end_matches(']');
            match inner.split_once(", ") {
                Some((low, high)) => {
                    low == high && !low.contains("MinKey") && !low.contains("MaxKey")
                }
                None => false,
            }
        })
        .then_some(bounds.len())
}

/// The mixed total streams the stamped ids on the missing-params index with
/// one bounded plan, checks them with membership commands hinted onto
/// `idx_search_composite` (point bounds on tenant, type, parameter and ids,
/// including the date `:not`'s top-level `$or`), runs no `$lookup` on the
/// stamped side, and keeps the plan's stages on the unstamped count alone.
#[tokio::test]
async fn mongodb_missing_checks_commands_have_bounded_single_plans() {
    let Some(backend) = create_backend("missing_checks_plans").await else {
        return;
    };
    let tenant = create_tenant("missing-checks-plans");
    let tenant_id = tenant.tenant_id().as_str().to_owned();
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    seed_mixed(&backend, &db, &tenant).await;
    let unstamped_resources = (0..DOCS).filter(|n| unstamped(*n)).count() as i64;
    let stamped_resources = DOCS as i64 - unstamped_resources;

    let query = mixed_query();
    let (total, strategies, _) = routed_total(&backend, &db, &tenant, &query).await;
    assert_eq!(strategies, ["exact_missing_checks".to_owned()], "route");
    assert_eq!(total, mixed_expected(), "total differs from the rows");
    // `sp-a` is absent on every fifth resource (60), and the date `:not`
    // excludes every thirtieth (10).
    assert_eq!(total, 50, "total differs from the rows");
    let (_, operations) =
        composite_plan::profiled(&db, async { backend.search_count(&tenant, &query).await }).await;
    let commands: Vec<Document> = operations
        .iter()
        .filter(|operation| matches!(operation.get_str("op"), Ok("command" | "query")))
        .map(|operation| composite_plan::command_of(operation).clone())
        .collect();

    // Stamped stream: one find on resources that reads ids (the freshness
    // probes read the fingerprint), on the missing-params index.
    let finds: Vec<&Document> = commands
        .iter()
        .filter(|command| {
            command.get_str("find") == Ok("resources")
                && command.get_document("projection").ok() == Some(&doc! { "_id": 0, "id": 1 })
        })
        .collect();
    assert_eq!(finds.len(), 1, "one stamped stream: {finds:?}");
    assert!(
        format!("{:?}", finds[0].get("hint")).contains(MISSING_INDEX),
        "stream without the missing_params hint: {:?}",
        finds[0]
    );
    let stream = inspect("stream", &explain(&db, finds[0]).await);
    assert!(
        stream.bounds.iter().all(|bound| bound.contains(NAMES[0])),
        "stream bound is not the name: {:?}",
        stream.bounds
    );
    assert!(stream.lookups.is_empty());

    // No command of the stamped side (or any planning probe) on
    // `search_index` runs a `$lookup`.
    for command in commands
        .iter()
        .filter(|command| command.get_str("aggregate") == Ok("search_index"))
    {
        assert!(
            !format!("{command:?}").contains("$lookup"),
            "$lookup on search_index: {command:?}"
        );
    }
    // Every search_index aggregate of the total is a membership check
    // (`$match` on the criterion and the batch ids, then `$group` by
    // `resource_id`) or the planner's selectivity probe of the
    // `:missing=false` (a `$limit`ed count): one check per criterion for the
    // one batch, and one probe.
    let aggregates: Vec<&Document> = commands
        .iter()
        .filter(|command| command.get_str("aggregate") == Ok("search_index"))
        .collect();
    let is_membership = |command: &&Document| {
        command.get_array("pipeline").is_ok_and(|pipeline| {
            pipeline.len() == 2
                && pipeline[1].as_document() == Some(&doc! { "$group": { "_id": "$resource_id" } })
                && format!("{:?}", pipeline[0]).contains("\"resource_id\": Document({\"$in\"")
        })
    };
    let is_probe = |command: &&Document| {
        command.get_array("pipeline").is_ok_and(|pipeline| {
            pipeline
                .iter()
                .filter_map(Bson::as_document)
                .any(|stage| stage.contains_key("$limit"))
                && format!("{:?}", pipeline[0]).contains(NAMES[2])
        })
    };
    let memberships: Vec<&Document> = aggregates.iter().copied().filter(is_membership).collect();
    let probes: Vec<&Document> = aggregates.iter().copied().filter(is_probe).collect();
    assert_eq!(
        (memberships.len(), probes.len(), aggregates.len()),
        (MIXED_CRITERIA as usize, 1, MIXED_CRITERIA as usize + 1),
        "search_index aggregates are {MIXED_CRITERIA} membership checks and one probe: {aggregates:?}"
    );
    let mut params = std::collections::BTreeSet::new();
    for command in &memberships {
        let text = format!("{command:?}");
        assert!(
            !text.contains("$lookup"),
            "stamped side runs $lookup: {command:?}"
        );
        assert!(
            format!("{:?}", command.get("hint")).contains("idx_search_composite"),
            "membership without the composite hint: {command:?}"
        );
        let explained = explain(&db, command).await;
        let planner = planner_of("membership", &explained);
        let scans = index_scans(&planner);
        assert!(
            !scans.is_empty(),
            "membership without an index scan: {planner:?}"
        );
        for scan in &scans {
            assert_eq!(
                scan.get_str("indexName"),
                Ok("idx_search_composite"),
                "{scan:?}"
            );
            for key in ["tenant_id", "resource_type", "param_name"] {
                assert_eq!(
                    points(scan, key),
                    Some(1),
                    "{key} is not one point: {scan:?}"
                );
            }
            let ids = points(scan, "resource_id")
                .unwrap_or_else(|| panic!("resource_id is not point intervals: {scan:?}"));
            assert!(ids as i64 <= stamped_resources, "{ids} id points: {scan:?}");
            let bounds = format!("{:?}", scan.get_document("indexBounds").unwrap());
            assert!(bounds.contains(&tenant_id), "{bounds}");
            params.insert(
                scan.get_document("indexBounds")
                    .unwrap()
                    .get_array("param_name")
                    .unwrap()[0]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
    }
    assert!(
        params.iter().any(|bound| bound.contains(DATE_NAME)),
        "no membership check of the date :not: {params:?}"
    );

    // Unstamped count: the only count on resources, all plan stages.
    let counts: Vec<&Document> = commands
        .iter()
        .filter(|c| is_count_on_resources(c))
        .collect();
    assert_eq!(counts.len(), 1, "one unstamped count: {counts:?}");
    assert!(is_unstamped_count(counts[0]));
    let unstamped = inspect("unstamped", &explain(&db, counts[0]).await);
    assert!(
        unstamped.bounds.iter().all(|bound| bound.contains("null")),
        "unstamped bound is not [null, null]: {:?}",
        unstamped.bounds
    );
    assert_eq!(
        unstamped.lookups.len() as i64,
        1 + MIXED_CRITERIA,
        "unstamped lookups"
    );
    for executed in &unstamped.lookups {
        assert!(*executed <= unstamped_resources, "{executed} lookups");
    }
}
