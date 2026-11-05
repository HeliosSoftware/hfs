//! Covered token membership must preserve results and bounded candidate work.

use super::*;
use futures::TryStreamExt;

const TOKEN_INDEX: &str = "idx_search_token_v2";
const ID_INDEX: &str = "idx_search_composite";
const ROWS: usize = 512;

fn token(name: &str, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.into(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    }
}

fn query(parameters: &[(&str, &str)], total: TotalMode) -> SearchQuery {
    let mut query = SearchQuery::new("Observation")
        .with_count(1000)
        .with_sort(SortDirective::parse("_id"));
    query.parameters = parameters
        .iter()
        .map(|(name, value)| token(name, value))
        .collect();
    query.total = Some(total);
    query
}

fn index_row(
    tenant: &str,
    kind: &str,
    id: &str,
    name: &str,
    value: &str,
    system: &str,
) -> Document {
    let system = if name == "status" { "" } else { system };
    doc! {
        "tenant_id": tenant, "resource_type": kind, "resource_id": id,
        "param_name": name, "value_token_code": value, "value_token_system": system,
    }
}

fn resource(tenant: &str, kind: &str, id: &str, number: usize, deleted: bool) -> Document {
    let now = mongodb::bson::DateTime::now();
    let mut coding = vec![doc! { "system": "urn:membership", "code": "base" }];
    if number.is_multiple_of(4) {
        coding.push(doc! { "system": "urn:membership", "code": "second" });
    }
    doc! {
        "tenant_id": tenant, "resource_type": kind, "id": id,
        "version_id": "1", "created_at": now, "last_updated": now,
        "is_deleted": deleted, "fhir_version": "4.0",
        "data": { "resourceType": kind, "id": id,
            "status": if number.is_multiple_of(2) { "final" } else { "preliminary" },
            "code": { "coding": coding },
            "category": [{ "coding": [{ "system": "urn:membership",
                "code": if number % 4 < 2 { "lab" } else { "other" } }] }],
            "interpretation": [{ "coding": [{ "system": "urn:membership",
                "code": if number % 8 < 4 { "normal" } else { "abnormal" } }] }],
        },
    }
}

fn values(number: usize) -> [(&'static str, &'static str); 4] {
    [
        ("code", "base"),
        (
            "status",
            if number.is_multiple_of(2) {
                "final"
            } else {
                "preliminary"
            },
        ),
        ("category", if number % 4 < 2 { "lab" } else { "other" }),
        (
            "interpretation",
            if number % 8 < 4 { "normal" } else { "abnormal" },
        ),
    ]
}

struct Fixture {
    backend: MongoBackend,
    tenant: TenantContext,
    db: mongodb::Database,
}

impl Fixture {
    async fn new(label: &str) -> Option<Self> {
        let backend = build_backend(MongoBackendConfig {
            connection_string: shared_mongo::connection_string().await?,
            database_name: build_test_database_name(label),
            data_dir: Some(repo_data_dir()),
            probe_timeout_ms: 1000,
            ..Default::default()
        })
        .await?;
        let tenant = create_tenant("covered-membership");
        let db = backend.get_database().await.unwrap();
        let scope = tenant.tenant_id().as_str();
        let mut resources = Vec::new();
        let mut index = Vec::new();
        for number in 0..ROWS {
            let id = format!("r{number:05}");
            resources.push(resource(scope, "Observation", &id, number, number == 0));
            for (name, value) in values(number) {
                let row = index_row(scope, "Observation", &id, name, value, "urn:membership");
                index.push(row.clone());
                if number % 17 == 0 {
                    index.push(row); // Duplicate index values contribute membership once.
                }
            }
            if number % 4 == 0 {
                index.push(index_row(
                    scope,
                    "Observation",
                    &id,
                    "code",
                    "second",
                    "urn:membership",
                ));
            }
            // Same ID, different scope: these rows must never fill in a missing predicate.
            for (other_tenant, kind) in [("foreign", "Observation"), (scope, "Patient")] {
                for (name, value) in values(0) {
                    index.push(index_row(
                        other_tenant,
                        kind,
                        &id,
                        name,
                        value,
                        "urn:membership",
                    ));
                }
            }
        }
        // A deleted resource and an absent resource still have matching index rows.
        for (name, value) in values(0) {
            index.push(index_row(
                scope,
                "Observation",
                "stale",
                name,
                value,
                "urn:membership",
            ));
        }
        index.push(index_row(
            scope,
            "Observation",
            "stale",
            "code",
            "second",
            "urn:membership",
        ));
        for (other_tenant, kind, id) in [
            ("foreign", "Observation", "foreign-only"),
            (scope, "Patient", "wrong-type"),
        ] {
            resources.push(resource(other_tenant, kind, id, 0, false));
            for (name, value) in values(0) {
                index.push(index_row(
                    other_tenant,
                    kind,
                    id,
                    name,
                    value,
                    "urn:membership",
                ));
            }
        }
        db.collection::<Document>("resources")
            .insert_many(resources)
            .await
            .unwrap();
        db.collection::<Document>("search_index")
            .insert_many(index)
            .await
            .unwrap();
        Some(Self {
            backend,
            tenant,
            db,
        })
    }

    async fn assert_query(&self, parameters: &[(&str, &str)], total: TotalMode, repeated: bool) {
        let query = query(parameters, total);
        let expected: Vec<_> = (1..ROWS)
            .filter(|number| {
                parameters.iter().all(|(name, wanted)| {
                    let wanted = wanted.rsplit('|').next().unwrap();
                    if *name == "code" && wanted == "second" {
                        number % 4 == 0
                    } else {
                        values(*number)
                            .iter()
                            .any(|(key, value)| key == name && *value == wanted)
                    }
                })
            })
            .map(|number| format!("r{number:05}"))
            .collect();
        assert!(!expected.is_empty());
        let result = self.backend.search(&self.tenant, &query).await.unwrap();
        let actual: Vec<_> = result
            .resources
            .items
            .iter()
            .map(|row| row.id().to_owned())
            .collect();
        assert_eq!(
            actual, expected,
            "parameters={parameters:?}, total={total:?}, repeated={repeated}"
        );
        let count = expected.len() as u64 + if total == TotalMode::Estimate { 2 } else { 0 };
        assert_eq!(
            result.total,
            if total == TotalMode::None {
                None
            } else {
                Some(count)
            }
        );
        assert_eq!(
            self.backend
                .search_count(&self.tenant, &query)
                .await
                .unwrap(),
            count
        );
    }
}

fn number(document: &Document, key: &str) -> i64 {
    document
        .get_i64(key)
        .or_else(|_| document.get_i32(key).map(i64::from))
        .unwrap()
}

fn candidate_bound(value: &Bson) -> bool {
    match value {
        Bson::Document(document) => {
            document
                .get_document("resource_id")
                .is_ok_and(|ids| ids.contains_key("$in"))
                || document.values().any(candidate_bound)
        }
        Bson::Array(values) => values.iter().any(candidate_bound),
        _ => false,
    }
}

fn named_system_predicate(value: &Bson) -> bool {
    match value {
        Bson::Document(document) => {
            document.contains_key("value_token_system")
                || document.values().any(named_system_predicate)
        }
        Bson::Array(values) => values.iter().any(named_system_predicate),
        _ => false,
    }
}

fn parameter_predicate(value: &Bson, name: &str) -> bool {
    match value {
        Bson::Document(document) => {
            document.get_str("param_name") == Ok(name)
                || document
                    .values()
                    .any(|value| parameter_predicate(value, name))
        }
        Bson::Array(values) => values.iter().any(|value| parameter_predicate(value, name)),
        _ => false,
    }
}

async fn profile_start(db: &mongodb::Database) {
    db.run_command(doc! { "profile": 2 })
        .await
        .expect("membership plan tests require database profiling");
}

async fn profiled_search_index_aggregates(db: &mongodb::Database) -> Vec<Document> {
    db.run_command(doc! { "profile": 0 }).await.unwrap();
    db.collection::<Document>("system.profile")
        .find(doc! { "command.aggregate": "search_index" })
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
}

async fn profiled_memberships(db: &mongodb::Database) -> Vec<Document> {
    profiled_search_index_aggregates(db)
        .await
        .into_iter()
        .filter(|row| {
            row.get_document("command")
                .unwrap()
                .get_array("pipeline")
                .unwrap()
                .iter()
                .any(candidate_bound)
        })
        .collect()
}

async fn explain_source(db: &mongodb::Database, command: &Document) -> Document {
    let explained = db
        .run_command(doc! {
            "explain": command_for_explain(command), "verbosity": "executionStats",
        })
        .await
        .unwrap();
    explained
        .get_array("stages")
        .ok()
        .and_then(|stages| {
            stages
                .iter()
                .find_map(|stage| stage.as_document()?.get_document("$cursor").ok().cloned())
        })
        .unwrap_or(explained)
}

#[tokio::test]
async fn mongodb_covered_membership_preserves_multi_token_results_and_totals() {
    let Some(fixture) = Fixture::new("covered_membership").await else {
        return;
    };
    let parameters = [
        ("code", "base"),
        ("status", "final"),
        ("category", "lab"),
        ("interpretation", "normal"),
    ];
    profile_start(&fixture.db).await;
    for size in 2..=4 {
        for total in [TotalMode::Accurate, TotalMode::Estimate, TotalMode::None] {
            fixture
                .assert_query(&parameters[..size], total, false)
                .await;
        }
    }
    for total in [TotalMode::Accurate, TotalMode::Estimate, TotalMode::None] {
        fixture
            .assert_query(
                &[("code", "base"), ("code", "second"), ("status", "final")],
                total,
                true,
            )
            .await;
    }
    let operations = profiled_memberships(&fixture.db).await;
    assert!(
        operations.len() >= 12,
        "real searches must execute later membership checks"
    );
    let mut covered_status = false;
    for operation in operations {
        let command = operation.get_document("command").unwrap();
        assert!(
            !command.contains_key("hint"),
            "code-only membership must allow index selection: {command:?}"
        );
        let status = command
            .get_array("pipeline")
            .unwrap()
            .iter()
            .any(|stage| parameter_predicate(stage, "status"));
        if status && number(&operation, "docsExamined") == 0 {
            let source = explain_source(&fixture.db, command).await;
            assert_eq!(
                number(
                    source.get_document("executionStats").unwrap(),
                    "totalDocsExamined"
                ),
                0,
                "{source:?}"
            );
            covered_status = true;
        }
    }
    assert!(
        covered_status,
        "simple status membership must demonstrate a covered execution"
    );
}

#[tokio::test]
async fn mongodb_covered_membership_keeps_single_filter_and_named_system_controls() {
    let Some(fixture) = Fixture::new("membership_controls").await else {
        return;
    };
    profile_start(&fixture.db).await;
    fixture
        .assert_query(&[("code", "base")], TotalMode::Estimate, false)
        .await;
    let single = profiled_search_index_aggregates(&fixture.db).await;
    assert!(
        !single.is_empty(),
        "single-predicate search must execute index work"
    );
    assert!(
        single
            .iter()
            .all(|op| op.get_document("command").unwrap().get_str("hint") != Ok(TOKEN_INDEX)),
        "single-predicate search must retain its original access path: {single:?}"
    );
    fixture
        .db
        .collection::<Document>("system.profile")
        .drop()
        .await
        .unwrap();
    profile_start(&fixture.db).await;
    let named = query(
        &[
            ("status", "final"),
            ("code", "urn:membership|base"),
            ("code", "urn:membership|base"),
        ],
        TotalMode::Accurate,
    );
    let result = fixture
        .backend
        .search(&fixture.tenant, &named)
        .await
        .unwrap();
    assert_eq!(result.total, Some(255));
    // Repeating the named predicate guarantees a later named check regardless
    // of which predicate wins the probe, without constraining driver selection.
    let operations: Vec<_> = profiled_memberships(&fixture.db)
        .await
        .into_iter()
        .filter(|op| {
            op.get_document("command")
                .unwrap()
                .get_array("pipeline")
                .unwrap()
                .iter()
                .any(named_system_predicate)
        })
        .collect();
    assert!(!operations.is_empty());
    assert!(
        operations
            .iter()
            .all(
                |op| op.get_document("command").unwrap().get_str("hint") == Ok(TOKEN_INDEX)
                    && number(op, "docsExamined") == 0
            ),
        "finite named systems must use covered token membership: {operations:?}"
    );
    fixture
        .db
        .collection::<Document>("system.profile")
        .drop()
        .await
        .unwrap();
    profile_start(&fixture.db).await;
    fixture
        .assert_query(
            &[("code", "base"), ("status", "|final"), ("status", "|final")],
            TotalMode::Accurate,
            true,
        )
        .await;
    let null_system: Vec<_> = profiled_memberships(&fixture.db)
        .await
        .into_iter()
        .filter(|op| {
            op.get_document("command")
                .unwrap()
                .get_array("pipeline")
                .unwrap()
                .iter()
                .any(named_system_predicate)
        })
        .collect();
    assert!(!null_system.is_empty());
    assert!(
        null_system
            .iter()
            .all(|op| op.get_document("command").unwrap().get_str("hint") == Ok(ID_INDEX)),
        "null-system alternatives must retain candidate seeks: {null_system:?}"
    );
}

#[tokio::test]
async fn mongodb_covered_membership_missing_token_index_uses_existing_id_index() {
    let Some(fixture) = Fixture::new("membership_fallback").await else {
        return;
    };
    fixture
        .db
        .collection::<Document>("search_index")
        .drop_index(TOKEN_INDEX)
        .await
        .unwrap();
    profile_start(&fixture.db).await;
    for total in [TotalMode::Accurate, TotalMode::Estimate, TotalMode::None] {
        fixture
            .assert_query(
                &[
                    ("code", "urn:membership|base"),
                    ("code", "urn:membership|base"),
                    ("status", "final"),
                ],
                total,
                true,
            )
            .await;
    }
    let operations = profiled_memberships(&fixture.db).await;
    assert!(
        operations
            .iter()
            .any(
                |op| op.get_document("command").unwrap().get_str("hint") == Ok(TOKEN_INDEX)
                    && op.get_i32("errCode") == Ok(2)
            ),
        "must actually attempt the missing token index: {operations:?}"
    );
    assert!(
        operations
            .iter()
            .any(
                |op| op.get_document("command").unwrap().get_str("hint") == Ok(ID_INDEX)
                    && !op.contains_key("errCode")
            ),
        "missing-index retry must use the existing candidate index: {operations:?}"
    );
}

#[tokio::test]
async fn mongodb_covered_membership_code_only_work_stays_bounded_across_many_systems() {
    let Some(backend) = create_backend("membership_many_systems").await else {
        return;
    };
    let tenant = create_tenant("membership-many-systems");
    let scope = tenant.tenant_id().as_str();
    let db = backend.get_database().await.unwrap();
    let mut resources = Vec::new();
    let mut index = Vec::new();
    for number in 0..128 {
        let id = format!("r{number:05}");
        let mut row = resource(scope, "Observation", &id, number, false);
        let data = row.get_document_mut("data").unwrap();
        data.insert("status", "final");
        data.insert(
            "code",
            doc! { "coding": [{
                "system": format!("urn:candidate:{number:05}"),
                "code": if number < 96 { "base" } else { "other" },
            }] },
        );
        resources.push(row);
        index.push(index_row(scope, "Observation", &id, "status", "final", ""));
        index.push(index_row(
            scope,
            "Observation",
            &id,
            "code",
            if number < 96 { "base" } else { "other" },
            &format!("urn:candidate:{number:05}"),
        ));
    }
    // The omitted system key precedes resource_id in the token index. A covered
    // plan must not turn a 128-ID membership check into a scan of every system.
    for number in 0..8192 {
        index.push(index_row(
            scope,
            "Observation",
            &format!("unrelated-{number:05}"),
            "code",
            "base",
            &format!("urn:other:{number:05}"),
        ));
    }
    db.collection::<Document>("resources")
        .insert_many(resources)
        .await
        .unwrap();
    db.collection::<Document>("search_index")
        .insert_many(index)
        .await
        .unwrap();
    profile_start(&db).await;
    let query = query(
        &[("status", "final"), ("code", "base")],
        TotalMode::Accurate,
    );
    assert_eq!(backend.search_count(&tenant, &query).await.unwrap(), 96);
    let operations = profiled_memberships(&db).await;
    assert!(
        !operations.is_empty(),
        "must exercise bounded membership after the selective driver"
    );
    for operation in operations {
        let command = operation.get_document("command").unwrap();
        let source = explain_source(&db, command).await;
        let stats = source.get_document("executionStats").unwrap();
        let mut original = command.clone();
        original.insert("hint", ID_INDEX);
        let original_source = explain_source(&db, &original).await;
        let original_keys = number(
            original_source.get_document("executionStats").unwrap(),
            "totalKeysExamined",
        );
        let actual_keys = number(stats, "totalKeysExamined");
        assert!(
            actual_keys <= original_keys.max(128) * 4,
            "covered membership scanned too many keys across systems: actual={actual_keys}, original={original_keys}, source={source:?}"
        );
        assert!(
            number(stats, "totalDocsExamined") <= 128 * 4,
            "candidate document work must remain bounded: {source:?}"
        );
    }
}

async fn seed_skewed_membership(db: &mongodb::Database, scope: &str, many_systems: bool) {
    let mut resources = Vec::new();
    let mut index = Vec::new();
    for number in 0..128 {
        let id = format!("r{number:05}");
        let system = if many_systems {
            format!("urn:candidate:{number:05}")
        } else {
            "urn:uniform".to_owned()
        };
        let code = if number < 96 { "base" } else { "other" };
        let mut row = resource(scope, "Observation", &id, number, false);
        let data = row.get_document_mut("data").unwrap();
        data.insert("status", "final");
        data.insert(
            "code",
            doc! { "coding": [{ "system": &system, "code": code }] },
        );
        data.insert(
            "category",
            vec![doc! { "coding": [{ "system": &system, "code": code }] }],
        );
        resources.push(row);
        index.push(index_row(scope, "Observation", &id, "status", "final", ""));
        for parameter in ["code", "category"] {
            index.push(index_row(
                scope,
                "Observation",
                &id,
                parameter,
                code,
                &system,
            ));
        }
    }
    // Both distributions have the same cardinality, so status stays the driver.
    // Only system diversity differs for the later code/category predicate.
    for number in 0..8192 {
        let system = if many_systems {
            format!("urn:other:{number:05}")
        } else {
            "urn:uniform".to_owned()
        };
        for parameter in ["code", "category"] {
            index.push(index_row(
                scope,
                "Observation",
                &format!("unrelated-{number:05}"),
                parameter,
                "base",
                &system,
            ));
        }
    }
    db.collection::<Document>("resources")
        .insert_many(resources)
        .await
        .unwrap();
    db.collection::<Document>("search_index")
        .insert_many(index)
        .await
        .unwrap();
}

#[tokio::test]
async fn mongodb_covered_membership_cached_plans_remain_bounded_across_tenant_skew() {
    let Some(connection_string) = shared_mongo::connection_string().await else {
        return;
    };
    let Some(backend) = build_backend(MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name("membership_cached_skew"),
        data_dir: Some(repo_data_dir()),
        probe_timeout_ms: 1000,
        ..Default::default()
    })
    .await
    else {
        return;
    };
    let db = backend.get_database().await.unwrap();
    let tenants = [
        create_tenant("membership-uniform"),
        create_tenant("membership-diverse"),
    ];
    for (position, tenant) in tenants.iter().enumerate() {
        seed_skewed_membership(&db, tenant.tenant_id().as_str(), position == 1).await;
    }
    for order in [[0, 0, 0, 1, 1, 1, 0, 1], [1, 1, 1, 0, 0, 0, 1, 0]] {
        db.run_command(doc! { "planCacheClear": "search_index" })
            .await
            .unwrap();
        for position in order {
            for parameter in ["code", "category"] {
                profile_start(&db).await;
                let search = query(
                    &[("status", "final"), (parameter, "base")],
                    TotalMode::Accurate,
                );
                assert_eq!(
                    backend
                        .search_count(&tenants[position], &search)
                        .await
                        .unwrap(),
                    96
                );
                let operations = profiled_memberships(&db).await;
                assert!(
                    !operations.is_empty(),
                    "must profile actual repeated membership work"
                );
                for operation in operations {
                    let command = operation.get_document("command").unwrap();
                    assert!(!command.contains_key("hint"), "{command:?}");
                    let mut original = command.clone();
                    original.insert("hint", ID_INDEX);
                    let baseline = explain_source(&db, &original).await;
                    let stats = baseline.get_document("executionStats").unwrap();
                    let baseline_keys = number(stats, "totalKeysExamined");
                    let baseline_docs = number(stats, "totalDocsExamined");
                    eprintln!(
                        "membership cache tenant={position} parameter={parameter} keys={} docs={} original_keys={baseline_keys} original_docs={baseline_docs} cache_key={:?} from_cache={:?}",
                        number(&operation, "keysExamined"),
                        number(&operation, "docsExamined"),
                        operation.get_str("planCacheKey").ok(),
                        operation.get_bool("fromPlanCache").ok(),
                    );
                    // These are the executed request's profiler counters. Explain
                    // is only the original-plan reference and cannot warm the cache.
                    assert!(
                        number(&operation, "keysExamined") <= baseline_keys.max(128) * 4,
                        "cached membership work regressed for tenant={position}, parameter={parameter}, original={stats:?}, operation={operation:?}"
                    );
                    assert!(
                        number(&operation, "docsExamined") <= baseline_docs.max(128) * 4,
                        "cached membership fetched unbounded documents: {operation:?}"
                    );
                }
                db.collection::<Document>("system.profile")
                    .drop()
                    .await
                    .unwrap();
            }
        }
    }
}
