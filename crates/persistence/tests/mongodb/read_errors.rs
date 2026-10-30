//! A single-document read whose driver call fails (#2271). `read`, `vread` and
//! `exists` used to flatten every driver error into a stringified `Internal`
//! error, a 500, so a dropped connection, a primary stepdown or a pool timeout
//! could not be told from a real fault. A transient failure is now
//! `Unavailable` (a retryable 503); any other keeps its driver error as the
//! source of `Internal`.
//!
//! Child module of the `mongodb_tests` root, like `write_abort_retry.rs`:
//! `use super::*;` reaches its harness. Every test drives the server's
//! `failCommand` failpoint, scoped to the backend's `appName` and to one
//! collection, against a backend with `retryWrites=false` and `retryReads=false`
//! so the driver's own retry of a read does not muddy the counts. One test
//! leaves the driver's read retry on, on purpose: it shows a state-change error
//! that outlasts that one retry (#2302). The same module covers the other
//! single-document reads that used to stringify the driver error (#2301): the
//! tenant lookup, `delete_with_match`'s load, `$purge`'s existence check and
//! the `$reindex` resource-type list and newest-resource probe. One test
//! covers a read that goes through `or_query_error`, `$purge`'s history count,
//! which answers a state-change error the same way since #2334. Since #2335 the
//! module also covers a cursor's later batch (`getMore`) of a history query and
//! the bulk export and `$bulk-submit` job-store reads, which go through
//! `or_query_error` too. No test asserts timing: after a dropped connection the
//! driver rediscovers the server before the next read succeeds.

use super::*;

use crate::bulk_submit::FailPoint;
use helios_persistence::core::bulk_export::{ExportDataProvider, ExportRequest};
use helios_persistence::core::{BulkSubmitProvider, SubmissionStatus, SubmitWorkerStorage};
use helios_persistence::search::ReindexSource;

/// A backend whose driver does not retry, with one Observation `obs-1` at
/// version 1 (seeded before any failpoint is on).
async fn backend_without_driver_retries(
    test: &str,
    app: &str,
) -> Option<(MongoBackend, TenantContext)> {
    seeded_backend(test, app, false).await
}

/// [`backend_without_driver_retries`], optionally leaving the driver's read
/// retry (`retryReads`) on. Writes are never retried by the driver here.
async fn seeded_backend(
    test: &str,
    app: &str,
    driver_read_retries: bool,
) -> Option<(MongoBackend, TenantContext)> {
    let connection_string = shared_mongo::connection_string().await?;
    let connection_string = append_query_param(&connection_string, "retryWrites=false");
    let connection_string = if driver_read_retries {
        connection_string
    } else {
        append_query_param(&connection_string, "retryReads=false")
    };
    let backend = build_backend(MongoBackendConfig {
        connection_string,
        database_name: build_test_database_name(test),
        app_name: app.to_string(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    })
    .await?;
    let tenant = create_tenant(test);
    let (stored, created) = backend
        .create_or_update(
            &tenant,
            "Observation",
            "obs-1",
            json!({
                "resourceType": "Observation",
                "id": "obs-1",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
            }),
            FhirVersion::default(),
        )
        .await
        .expect("seeding the Observation");
    assert!(created);
    assert_eq!(stored.version_id(), "1");
    Some((backend, tenant))
}

/// The failpoint's `data` for a `command` on `collection` that closes the
/// connection mid-command, the way a dropped connection or a stepdown does.
fn dropped_connection_on(backend: &MongoBackend, command: &str, collection: &str) -> Document {
    doc! {
        "failCommands": [command],
        "namespace": format!("{}.{collection}", backend.config().database_name),
        "closeConnection": true,
    }
}

/// `read` whose connection drops: `Unavailable`, with the driver text kept out
/// of the client-facing message. It used to be a 500 "Failed to read resource".
#[tokio::test]
async fn a_dropped_connection_on_read_is_unavailable() {
    let app = "fp-read-dropped-connection";
    let Some((backend, tenant)) = backend_without_driver_retries("read_dropped", app).await else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "find", "resources"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend.read(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to read resource: ") && !message.contains("Kind:"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(entered, 1);
    let again = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .expect("the next read succeeds")
        .expect("the resource is still there");
    assert_eq!(again.version_id(), "1");
}

/// A server error that is not transient stays a 500, and now keeps the driver
/// error as its source, so a classifier can read its code.
#[tokio::test]
async fn a_non_transient_read_failure_stays_internal_and_keeps_its_driver_error() {
    let app = "fp-read-non-transient";
    let Some((backend, tenant)) = backend_without_driver_retries("read_non_transient", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["find"],
            "namespace": format!("{}.resources", backend.config().database_name),
            "errorCode": 2,
        },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend.read(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Internal {
        message,
        source: Some(source),
        ..
    })) = &result
    else {
        panic!("expected Internal with its driver error kept, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to read resource: "),
        "{message}"
    );
    let driver = source
        .downcast_ref::<mongodb::error::Error>()
        .expect("the driver error is the source");
    assert!(
        matches!(
            driver.kind.as_ref(),
            mongodb::error::ErrorKind::Command(command) if command.code == 2
        ),
        "{driver:?}"
    );
    assert_eq!(entered, 1);
}

/// `vread` of a history version whose connection drops.
#[tokio::test]
async fn a_dropped_connection_on_vread_is_unavailable() {
    let app = "fp-read-dropped-vread";
    let Some((backend, tenant)) = backend_without_driver_retries("read_dropped_vread", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "find", "resource_history"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend.vread(&tenant, "Observation", "obs-1", "1").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to read historical version: ") && !message.contains("Kind:"),
        "{message}"
    );
    assert_eq!(entered, 1);
    let again = backend
        .vread(&tenant, "Observation", "obs-1", "1")
        .await
        .expect("the next vread succeeds");
    assert!(again.is_some());
}

/// `exists` runs `count_documents`, which is an `aggregate` on the server.
#[tokio::test]
async fn a_dropped_connection_on_exists_is_unavailable() {
    let app = "fp-read-dropped-exists";
    let Some((backend, tenant)) = backend_without_driver_retries("read_dropped_exists", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "aggregate", "resources"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = backend.exists(&tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to check resource existence: ") && !message.contains("Kind:"),
        "{message}"
    );
    assert_eq!(entered, 1);
    assert!(
        backend
            .exists(&tenant, "Observation", "obs-1")
            .await
            .expect("the next existence check succeeds")
    );
}

/// The ids on every page of a `$reindex` walk of `resource_type`.
async fn reindex_walk_ids(
    backend: &MongoBackend,
    tenant: &TenantContext,
    resource_type: &str,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let page = ReindexSource::fetch_resources_page(
            backend,
            tenant,
            resource_type,
            cursor.as_deref(),
            10,
        )
        .await
        .expect("the reindex page is fetched");
        ids.extend(page.resources.iter().map(|r| r.id().to_string()));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    ids
}

/// The tenant registry lookup (#2301): a dropped connection is `Unavailable`
/// and the lookup is not mistaken for a missing tenant.
#[tokio::test]
async fn a_dropped_connection_on_the_tenant_lookup_is_unavailable() {
    let app = "fp-read-dropped-get-tenant";
    let Some((backend, _tenant)) =
        backend_without_driver_retries("read_dropped_get_tenant", app).await
    else {
        return;
    };
    backend
        .register_tenant("fp-known-tenant", None)
        .await
        .expect("registering a tenant");
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "find", "tenants"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = ResourceStorage::get_tenant(&backend, "fp-known-tenant").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        message.starts_with("query get_tenant: ") && !message.contains("Kind:"),
        "{message}"
    );
    assert_eq!(entered, 1);
    assert!(
        ResourceStorage::get_tenant(&backend, "fp-known-tenant")
            .await
            .expect("the next lookup succeeds")
            .is_some()
    );
    assert!(
        ResourceStorage::get_tenant(&backend, "fp-unknown-tenant")
            .await
            .expect("a lookup of an unknown tenant succeeds")
            .is_none()
    );
}

/// A tenant lookup that fails for a reason that is not transient stays a 500
/// and keeps the driver error as its source.
#[tokio::test]
async fn a_non_transient_tenant_lookup_failure_stays_internal_and_keeps_its_driver_error() {
    let app = "fp-read-non-transient-get-tenant";
    let Some((backend, _tenant)) =
        backend_without_driver_retries("read_non_transient_get_tenant", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["find"],
            "namespace": format!("{}.tenants", backend.config().database_name),
            "errorCode": 2,
        },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = ResourceStorage::get_tenant(&backend, "fp-any-tenant").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Internal {
        message,
        source: Some(source),
        ..
    })) = &result
    else {
        panic!("expected Internal with its driver error kept, got {result:?}");
    };
    assert!(message.starts_with("query get_tenant: "), "{message}");
    let driver = source
        .downcast_ref::<mongodb::error::Error>()
        .expect("the driver error is the source");
    assert!(
        matches!(
            driver.kind.as_ref(),
            mongodb::error::ErrorKind::Command(command) if command.code == 2
        ),
        "{driver:?}"
    );
    assert_eq!(entered, 1);
}

/// `delete_with_match` loads the resource before it compares versions. A failed
/// load is `Unavailable`; the version-conflict and not-found decisions it makes
/// from a successful load are unchanged.
#[tokio::test]
async fn a_dropped_connection_on_delete_with_match_is_unavailable_and_its_decisions_stand() {
    let app = "fp-read-dropped-delete-with-match";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_dropped_delete_with_match", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "find", "resources"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result =
        VersionedStorage::delete_with_match(&backend, &tenant, "Observation", "obs-1", "1").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to load resource for delete_with_match: ")
            && !message.contains("Kind:"),
        "{message}"
    );
    assert_eq!(entered, 1);

    let still_there = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .expect("the next read succeeds")
        .expect("the failed delete removed nothing");
    assert_eq!(still_there.version_id(), "1");
    let stale =
        VersionedStorage::delete_with_match(&backend, &tenant, "Observation", "obs-1", "9").await;
    assert!(
        matches!(
            stale,
            Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict { .. }
            ))
        ),
        "{stale:?}"
    );
    let missing =
        VersionedStorage::delete_with_match(&backend, &tenant, "Observation", "missing", "1").await;
    assert!(
        matches!(
            missing,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "{missing:?}"
    );
}

/// `$purge` checks the resource exists with `count_documents`, an `aggregate`
/// on the server. A failed check is `Unavailable` and purges nothing.
#[tokio::test]
async fn a_dropped_connection_on_the_purge_check_is_unavailable_and_purges_nothing() {
    let app = "fp-read-dropped-purge";
    let Some((backend, tenant)) = backend_without_driver_retries("read_dropped_purge", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "aggregate", "resources"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = PurgableStorage::purge(&backend, &tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to check resource: ") && !message.contains("Kind:"),
        "{message}"
    );
    assert_eq!(entered, 1);

    let still_there = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .expect("the next read succeeds")
        .expect("the failed purge removed nothing");
    assert_eq!(still_there.version_id(), "1");
    let missing = PurgableStorage::purge(&backend, &tenant, "Observation", "missing").await;
    assert!(
        matches!(
            missing,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "{missing:?}"
    );
}

/// The `$reindex` walk starts by probing the newest live resource of the type.
#[tokio::test]
async fn a_dropped_connection_on_the_reindex_probe_is_unavailable() {
    let app = "fp-read-dropped-reindex-probe";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_dropped_reindex_probe", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "find", "resources"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result =
        ReindexSource::fetch_resources_page(&backend, &tenant, "Observation", None, 10).await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to probe newest resource: ") && !message.contains("Kind:"),
        "{message}"
    );
    assert_eq!(entered, 1);
    assert!(
        reindex_walk_ids(&backend, &tenant, "Observation")
            .await
            .contains(&"obs-1".to_string()),
        "the next walk finds the resource"
    );
}

/// The `$reindex` job lists the tenant's resource types with `distinct`.
#[tokio::test]
async fn a_dropped_connection_on_the_reindex_type_list_is_unavailable() {
    let app = "fp-read-dropped-reindex-types";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_dropped_reindex_types", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "distinct", "resources"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = ReindexSource::list_resource_types(&backend, &tenant).await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert!(
        message.starts_with("Failed to list resource types: ") && !message.contains("Kind:"),
        "{message}"
    );
    assert_eq!(entered, 1);
    let types = ReindexSource::list_resource_types(&backend, &tenant)
        .await
        .expect("the next listing succeeds");
    assert!(types.contains(&"Observation".to_string()), "{types:?}");
}

/// #2302: a state-change error that outlasts the driver's read retry. The driver
/// retries a read once on these codes (`retryReads`), so the failpoint fails
/// both attempts (`times: 2`); the error then reaches `read` with no label and
/// used to answer 500. It is now `Unavailable`, a retryable 503. 91 and 11600
/// (the shutdown codes) are left to the unit table in `storage.rs`: they clear
/// the driver's connection pool, which would turn the retry into a different
/// error.
#[tokio::test]
async fn a_state_change_that_outlasts_the_driver_read_retry_is_unavailable() {
    let app = "fp-read-state-change";
    let Some((backend, tenant)) = seeded_backend("read_state_change", app, true).await else {
        return;
    };

    for (code, name) in [
        (11602, "InterruptedDueToReplStateChange"),
        (10107, "NotWritablePrimary"),
        (189, "PrimarySteppedDown"),
    ] {
        let Some(fail_point) = FailPoint::enable(
            app,
            doc! {
                "failCommands": ["find"],
                "namespace": format!("{}.resources", backend.config().database_name),
                "errorCode": code,
            },
            doc! { "times": 2 },
        )
        .await
        else {
            return;
        };

        let result = backend.read(&tenant, "Observation", "obs-1").await;
        let entered = fail_point.off_and_count().await;

        let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
            panic!("{name} ({code}): expected Unavailable, got {result:?}");
        };
        assert!(
            message.starts_with("Failed to read resource: ") && !message.contains("Kind:"),
            "{name} ({code}): {message}"
        );
        assert_eq!(
            entered, 2,
            "{name} ({code}): the driver retried the read once and the failure outlasted it"
        );
        let again = backend
            .read(&tenant, "Observation", "obs-1")
            .await
            .unwrap_or_else(|e| panic!("{name} ({code}): the next read succeeds: {e:?}"))
            .expect("the resource is still there");
        assert_eq!(again.version_id(), "1");
    }
}

/// #2334: the same state-change error on a read that goes through `or_query_error`, not
/// `read_driver_error`: `$purge`'s `resource_history` count, which runs right after the
/// `resources` count. During an election the two counts used to answer differently (503, then
/// 500); `classify_mongodb_error` now answers a state-change code `Unavailable` too, with the
/// driver text in the log only. `count_documents` is an `aggregate`, and the `resources` count
/// before it is on another namespace, so the failpoint does not fire on it. 11602 clears no
/// connection pool, unlike 91 and 11600 (see above), so the driver's own retry still runs.
#[tokio::test]
async fn a_state_change_on_the_purge_history_count_is_unavailable_and_purges_nothing() {
    let app = "fp-read-state-change-purge-history";
    let Some((backend, tenant)) =
        seeded_backend("read_state_change_purge_history", app, true).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["aggregate"],
            "namespace": format!("{}.resource_history", backend.config().database_name),
            "errorCode": 11602,
        },
        doc! { "times": 2 },
    )
    .await
    else {
        return;
    };

    let result = PurgableStorage::purge(&backend, &tenant, "Observation", "obs-1").await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert_eq!(
        message,
        "Failed to check resource history: the database is temporarily unavailable; retry the request"
    );
    assert!(
        !message.contains("Kind:") && !message.contains("InterruptedDueToReplStateChange"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(
        entered, 2,
        "the driver retried the count once and the failure outlasted it"
    );

    let still_there = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .expect("the next read succeeds")
        .expect("the failed purge removed nothing");
    assert_eq!(still_there.version_id(), "1");

    // The 503 invites a retry, and the retry purges.
    PurgableStorage::purge(&backend, &tenant, "Observation", "obs-1")
        .await
        .expect("the retried purge succeeds");
    let again = PurgableStorage::purge(&backend, &tenant, "Observation", "obs-1").await;
    assert!(
        matches!(
            again,
            Err(StorageError::Resource(ResourceError::NotFound { .. }))
        ),
        "{again:?}"
    );
}

/// #2335: a dropped connection on a cursor's later batch. A type history drains its
/// cursor with `collect_documents`; the `find` that opens it was already classified, but the
/// `getMore` that fetches the next batch was not, and answered 500. A `find` returns 101
/// documents in its first batch, so 110 history rows make the drain send a `getMore`, which
/// the failpoint closes. The client-facing message names the step and leaves the driver error
/// to the log.
#[tokio::test]
async fn a_dropped_connection_on_a_history_get_more_is_unavailable() {
    let app = "fp-read-dropped-history-get-more";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_dropped_history_get_more", app).await
    else {
        return;
    };
    // `obs-1` is seeded; 109 more make 110 rows in the type history.
    for n in 2..=110 {
        let id = format!("obs-{n}");
        backend
            .create_or_update(
                &tenant,
                "Observation",
                &id,
                json!({
                    "resourceType": "Observation",
                    "id": id.clone(),
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seeding an Observation");
    }
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "getMore", "resource_history"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let params = HistoryParams::new().count(200);
    let result = TypeHistoryProvider::history_type(&backend, &tenant, "Observation", &params).await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert_eq!(
        message,
        "Failed to advance MongoDB cursor: the database is temporarily unavailable; retry the request"
    );
    assert!(
        !message.contains("Kind:"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(entered, 1);
    let again = TypeHistoryProvider::history_type(&backend, &tenant, "Observation", &params)
        .await
        .expect("the next history query succeeds");
    assert_eq!(again.items.len(), 110);
}

/// #2335: a bulk export data read whose connection drops. The export type list is a
/// `distinct`; it used to answer 500 with the driver text in the message.
#[tokio::test]
async fn a_dropped_connection_on_the_export_type_list_is_unavailable() {
    let app = "fp-read-dropped-export-types";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_dropped_export_types", app).await
    else {
        return;
    };
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "distinct", "resources"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let request = ExportRequest::system();
    let result = ExportDataProvider::list_export_types(&backend, &tenant, &request).await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert_eq!(
        message,
        "distinct types: the database is temporarily unavailable; retry the request"
    );
    assert!(
        !message.contains("Kind:"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(entered, 1);
    let types = ExportDataProvider::list_export_types(&backend, &tenant, &request)
        .await
        .expect("the next call succeeds");
    assert!(types.iter().any(|t| t == "Observation"), "{types:?}");
}

/// #2335: the `$bulk-submit` status read whose connection drops. This is the second read of
/// `GET /bulk-submit-status/{poll_token}`, after the poll-token lookup (covered by
/// `a_dropped_connection_on_the_poll_token_lookup_is_unavailable` below). A Bulk Data client
/// polls it and expects to retry a 503; it used to get a 500.
#[tokio::test]
async fn a_dropped_connection_on_the_submission_status_read_is_unavailable() {
    let app = "fp-read-dropped-submit-status";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_dropped_submit_status", app).await
    else {
        return;
    };
    let (id, _) = crate::bulk_submit::seed(&backend, &tenant).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "find", "bulk_submissions"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = BulkSubmitProvider::get_submission_status(&backend, &tenant, &id).await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert_eq!(
        message,
        "read submission status: the database is temporarily unavailable; retry the request"
    );
    assert!(
        !message.contains("Kind:"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(entered, 1);
    let again = BulkSubmitProvider::get_submission_status(&backend, &tenant, &id)
        .await
        .expect("the next call succeeds");
    assert_eq!(again, Some(SubmissionStatus::InProgress));
}

/// #2335: the poll-token lookup whose connection drops. `GET /bulk-submit-status/{poll_token}`
/// calls `resolve_poll_token` before it reads the submission's status, so a dropped connection
/// here still answered 500 with the driver text after the status read itself was classified.
/// The lookup is a `find` on `bulk_submissions` by `poll_token`.
#[tokio::test]
async fn a_dropped_connection_on_the_poll_token_lookup_is_unavailable() {
    let app = "fp-read-dropped-poll-token";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_dropped_poll_token", app).await
    else {
        return;
    };
    let (id, _) = crate::bulk_submit::seed(&backend, &tenant).await;
    let token = SubmitWorkerStorage::ensure_poll_token(&backend, &tenant, &id)
        .await
        .expect("minting the poll token");
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "find", "bulk_submissions"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = SubmitWorkerStorage::resolve_poll_token(&backend, &token).await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert_eq!(
        message,
        "resolve poll token: the database is temporarily unavailable; retry the request"
    );
    assert!(
        !message.contains("Kind:"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(entered, 1);
    let target = SubmitWorkerStorage::resolve_poll_token(&backend, &token)
        .await
        .expect("the next call succeeds")
        .expect("the token still resolves");
    assert_eq!(target.submission_id, id);
}

/// #2335: the manifest counter aggregate whose connection drops. A submission summary
/// (`get_submission`) reads the submission document, then sums its manifests' counters with an
/// `aggregate` on `bulk_manifests`; only that second read fails here, and it used to answer 500
/// with the driver text. A dropped connection fails the `aggregate` command itself, so this
/// covers the call's own `or_query_error`; the `getMore` of its cursor drain is the shared
/// `collect`, covered by the history test above.
#[tokio::test]
async fn a_dropped_connection_on_the_manifest_counter_aggregate_is_unavailable() {
    let app = "fp-read-dropped-manifest-aggregate";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_dropped_manifest_aggregate", app).await
    else {
        return;
    };
    let (id, _) = crate::bulk_submit::seed(&backend, &tenant).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        dropped_connection_on(&backend, "aggregate", "bulk_manifests"),
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = BulkSubmitProvider::get_submission(&backend, &tenant, &id).await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Unavailable { message, .. })) = &result else {
        panic!("expected Unavailable, got {result:?}");
    };
    assert_eq!(
        message,
        "sum manifest counters: the database is temporarily unavailable; retry the request"
    );
    assert!(
        !message.contains("Kind:"),
        "the client-facing message leaves the driver error to the log: {message}"
    );
    assert_eq!(entered, 1);
    let summary = BulkSubmitProvider::get_submission(&backend, &tenant, &id)
        .await
        .expect("the next call succeeds")
        .expect("the submission is still there");
    assert_eq!(summary.manifest_count, 1);
}

/// #2335: any other failure of a job-store read stays `Internal`, with the driver error kept as
/// its source, as the single-document reads above do.
#[tokio::test]
async fn a_non_transient_submission_status_read_stays_internal_and_keeps_its_driver_error() {
    let app = "fp-read-non-transient-submit-status";
    let Some((backend, tenant)) =
        backend_without_driver_retries("read_non_transient_submit_status", app).await
    else {
        return;
    };
    let (id, _) = crate::bulk_submit::seed(&backend, &tenant).await;
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["find"],
            "namespace": format!("{}.bulk_submissions", backend.config().database_name),
            "errorCode": 2,
        },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let result = BulkSubmitProvider::get_submission_status(&backend, &tenant, &id).await;
    let entered = fail_point.off_and_count().await;

    let Err(StorageError::Backend(BackendError::Internal {
        message,
        source: Some(source),
        ..
    })) = &result
    else {
        panic!("expected Internal with its driver error kept, got {result:?}");
    };
    assert!(message.starts_with("read submission status: "), "{message}");
    let driver = source
        .downcast_ref::<mongodb::error::Error>()
        .expect("the driver error is the source");
    assert!(
        matches!(
            driver.kind.as_ref(),
            mongodb::error::ErrorKind::Command(command) if command.code == 2
        ),
        "{driver:?}"
    );
    assert_eq!(entered, 1);
}
