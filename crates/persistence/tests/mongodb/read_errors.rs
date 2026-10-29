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
//! so the driver's own retry of a read does not muddy the counts. No test
//! asserts timing: after a dropped connection the driver rediscovers the server
//! before the next read succeeds.

use super::*;

use crate::bulk_submit::FailPoint;

/// A backend whose driver does not retry, with one Observation `obs-1` at
/// version 1 (seeded before any failpoint is on).
async fn backend_without_driver_retries(
    test: &str,
    app: &str,
) -> Option<(MongoBackend, TenantContext)> {
    let connection_string = shared_mongo::connection_string().await?;
    let connection_string = append_query_param(&connection_string, "retryWrites=false");
    let connection_string = append_query_param(&connection_string, "retryReads=false");
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
