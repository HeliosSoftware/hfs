//! A single-resource create in a transaction of an id the backend minted
//! itself (a `POST`) sends neither its existence check nor its search-index
//! delete: nothing can be stored under an id nobody had, and an earlier
//! attempt of the same write that did not commit left nothing behind. A
//! client-supplied id keeps both, since rows under it may survive a write that
//! ran without a transaction. The `resources` identity index still answers a
//! duplicate as `AlreadyExists` (pinned by `storage.rs`'s
//! `create_command_tests`, which reaches the attempt directly).
//!
//! The command counts scope their failpoint to one collection
//! ([`counted_create`]), so a lookup mongod itself runs for the session does
//! not count.
//!
//! These tests pin what stays the same: the resource reads back and is found
//! by search, a duplicate create of a client id is `AlreadyExists` with nothing
//! extra written, and two concurrent creates of one id end as exactly one
//! success and one `AlreadyExists`.
//!
//! Child module of the `mongodb_tests` root: `use super::*;` reaches its
//! harness. The shared server is a single-node replica set, so a create here
//! runs in a transaction.

use super::*;

use crate::bulk_submit::FailPoint;
use helios_persistence::error::StorageResult;
use helios_persistence::types::StoredResource;

/// The collections a create's counted commands go to: its existence check
/// (`find` on `resources`) and its two search-index deletes.
const COUNTED: [(&str, &str); 3] = [
    ("find", "resources"),
    ("delete", "search_index"),
    ("delete", "search_index_contained"),
];

/// Runs one create of `resource` under a `failCommand` that matches, without
/// failing, the scoped client's `command` on `collection` of the backend's
/// database, and returns the create's result and how many commands matched.
///
/// The failpoint is scoped to one namespace, not only to the client's
/// `appName`: when a session runs its first transaction, mongod looks the
/// session up in `config.transactions` with a `find` under the client's own
/// operation, so an unscoped `find` count also includes that lookup. Whether
/// the counted create reuses a session that already ran a transaction depends
/// on the driver's session pool, so such a count is 1 higher at random.
async fn counted_create(
    backend: &MongoBackend,
    tenant: &TenantContext,
    app: &str,
    (command, collection): (&str, &str),
    resource: serde_json::Value,
) -> Option<(StorageResult<StoredResource>, i64)> {
    let data = doc! {
        "failCommands": [command],
        "namespace": format!("{}.{collection}", backend.config().database_name),
        "blockConnection": true,
        "blockTimeMS": 0_i32,
    };
    let fail_point = FailPoint::enable(app, data, doc! { "times": 1000 }).await?;
    let result = backend
        .create(tenant, "Observation", resource, FhirVersion::default())
        .await;
    Some((result, fail_point.off_and_count().await))
}

fn observation(id: Option<&str>, status: &str) -> serde_json::Value {
    let mut resource = json!({
        "resourceType": "Observation",
        "status": status,
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
    });
    if let Some(id) = id {
        resource["id"] = json!(id);
    }
    resource
}

fn status_query(status: &str) -> SearchQuery {
    SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "status".to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq(status)],
        chain: vec![],
        components: vec![],
    })
}

/// The rows stored under `id` in `tenant`: `resources`, `resource_history`,
/// `search_index`.
async fn rows(backend: &MongoBackend, tenant: &TenantContext, id: &str) -> (u64, u64, u64) {
    let resource = doc! { "tenant_id": tenant.tenant_id().as_str(), "id": id };
    let index = doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_id": id };
    (
        count_docs(backend, "resources", resource.clone()).await,
        count_docs(backend, "resource_history", resource).await,
        count_docs(backend, "search_index", index).await,
    )
}

/// One create in `tenant` before the counting starts, so the tenant's one-time
/// reads (its stored SearchParameters, which the first write of a tenant
/// loads with a `find`) are not counted as the create's own commands.
async fn warm_tenant(backend: &MongoBackend, tenant: &TenantContext) {
    backend
        .create(
            tenant,
            "Observation",
            observation(Some("obs-warm"), "preliminary"),
            FhirVersion::default(),
        )
        .await
        .expect("the warm-up create commits");
}

/// A `POST` sends no `find` on `resources` and no `delete` on either search
/// index collection; it reads back as written, its history row and search
/// rows are stored, and search finds it. One create per counted command.
#[tokio::test]
async fn a_post_create_sends_no_existence_check_and_no_index_delete() {
    let app = "fp-create-commands-post";
    let Some(backend) = create_backend_with_app_name("create_commands_post", app).await else {
        return;
    };
    let tenant = create_tenant("create_commands_post");
    warm_tenant(&backend, &tenant).await;

    let mut ids = Vec::new();
    for counted in COUNTED {
        let Some((result, entered)) =
            counted_create(&backend, &tenant, app, counted, observation(None, "final")).await
        else {
            return;
        };
        let stored = result.expect("the create commits");
        assert_eq!(stored.version_id(), "1");
        assert_eq!(
            entered, 0,
            "a create of a minted id in a transaction sends no {counted:?}"
        );

        let id = stored.id().to_string();
        let read = backend
            .read(&tenant, "Observation", &id)
            .await
            .unwrap()
            .expect("the created resource reads back");
        assert_eq!(read.content(), stored.content());
        assert_eq!(read.content()["status"], "final");
        assert_eq!(read.version_id(), "1");

        let (resources, history, index) = rows(&backend, &tenant, &id).await;
        assert_eq!((resources, history), (1, 1));
        assert!(index > 0, "the create stored its search rows");
        ids.push(id);
    }

    let found = backend
        .search(&tenant, &status_query("final"))
        .await
        .expect("search");
    let mut found: Vec<&str> = found.resources.items.iter().map(|r| r.id()).collect();
    found.sort_unstable();
    ids.sort_unstable();
    assert_eq!(found, ids);
}

/// A create of a client-supplied id still sends its existence check and both
/// search-index deletes: one of each, one create per counted command.
#[tokio::test]
async fn a_create_of_a_client_id_keeps_its_existence_check_and_index_delete() {
    let app = "fp-create-commands-client";
    let Some(backend) = create_backend_with_app_name("create_commands_client", app).await else {
        return;
    };
    let tenant = create_tenant("create_commands_client");
    warm_tenant(&backend, &tenant).await;

    for (n, counted) in COUNTED.into_iter().enumerate() {
        let id = format!("obs-client-{n}");
        let Some((result, entered)) = counted_create(
            &backend,
            &tenant,
            app,
            counted,
            observation(Some(&id), "final"),
        )
        .await
        else {
            return;
        };
        result.expect("the create commits");
        assert_eq!(entered, 1, "a create of a client id sends one {counted:?}");
        let (resources, history, index) = rows(&backend, &tenant, &id).await;
        assert_eq!((resources, history), (1, 1));
        assert!(index > 0);
    }
}

/// A second create of a stored client id is `AlreadyExists`, and writes
/// nothing: the resource, its history and its search rows are the first
/// create's.
#[tokio::test]
async fn a_duplicate_create_is_already_exists_and_writes_nothing() {
    let Some(backend) = create_backend("create_commands_duplicate").await else {
        return;
    };
    let tenant = create_tenant("create_commands_duplicate");
    backend
        .create(
            &tenant,
            "Observation",
            observation(Some("obs-dup"), "final"),
            FhirVersion::default(),
        )
        .await
        .expect("the first create commits");
    let before = rows(&backend, &tenant, "obs-dup").await;

    let second = backend
        .create(
            &tenant,
            "Observation",
            observation(Some("obs-dup"), "amended"),
            FhirVersion::default(),
        )
        .await;

    assert!(
        matches!(
            second,
            Err(StorageError::Resource(ResourceError::AlreadyExists { ref id, .. }))
                if id == "obs-dup"
        ),
        "{second:?}"
    );
    assert_eq!(rows(&backend, &tenant, "obs-dup").await, before);
    let read = backend
        .read(&tenant, "Observation", "obs-dup")
        .await
        .unwrap()
        .expect("the first create's resource");
    assert_eq!(read.content()["status"], "final");
    assert_eq!(read.version_id(), "1");
    assert_eq!(
        backend
            .search_count(&tenant, &status_query("amended"))
            .await
            .unwrap(),
        0,
        "the refused create left no search row"
    );
}

/// Two creates of one client id at once: exactly one commits and the other is
/// `AlreadyExists`, and the rows are one create's. Run over several ids, so
/// the two meet at different points of their transactions.
#[tokio::test]
async fn two_concurrent_creates_of_one_id_are_one_success_and_one_already_exists() {
    let Some(backend) = create_backend("create_commands_concurrent").await else {
        return;
    };
    let tenant = create_tenant("create_commands_concurrent");
    let backend = std::sync::Arc::new(backend);
    let single = {
        backend
            .create(
                &tenant,
                "Observation",
                observation(Some("obs-reference"), "final"),
                FhirVersion::default(),
            )
            .await
            .expect("a lone create commits");
        rows(&backend, &tenant, "obs-reference").await.2
    };

    for round in 0..10 {
        let id = format!("obs-race-{round}");
        let spawn = |status: &'static str| {
            let backend = backend.clone();
            let tenant = tenant.clone();
            let id = id.clone();
            tokio::spawn(async move {
                backend
                    .create(
                        &tenant,
                        "Observation",
                        observation(Some(&id), status),
                        FhirVersion::default(),
                    )
                    .await
            })
        };
        let (first, second) = tokio::join!(spawn("final"), spawn("final"));
        let outcomes = [first.unwrap(), second.unwrap()];

        let won = outcomes.iter().filter(|o| o.is_ok()).count();
        let lost = outcomes
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    Err(StorageError::Resource(ResourceError::AlreadyExists { .. }))
                )
            })
            .count();
        assert_eq!((won, lost), (1, 1), "round {round}: {outcomes:?}");
        assert_eq!(
            rows(&backend, &tenant, &id).await,
            (1, 1, single),
            "round {round}: one create's rows"
        );
    }
}
