//! #2351: `HFS_MONGODB_TRANSACTION_BUNDLE_MAX_PAYLOAD_BYTES`
//! (`MongoBackendConfig::transaction_bundle_max_payload_bytes`) refuses a
//! transaction Bundle whose POST and PUT resources total more than the limit,
//! before it is admitted: no admission, no session, no transaction, nothing
//! written and nothing rolled back.
//!
//! Child module of the `mongodb_tests` root, like `transaction_bundle_admission`:
//! `use super::*;` reaches its harness (`create_tenant`, `count_docs`,
//! `process_transaction_or_skip`, ...).
//!
//! "No transaction started" is shown from the server's side and from the log,
//! because a server transaction begins only with a command:
//!
//! - a `failCommand` failpoint, scoped to the backend's `appName` and listing
//!   `insert`, `delete`, `commitTransaction` and `abortTransaction`, is never
//!   entered by the refused Bundle. A Bundle of POSTs sends a `delete` (its old
//!   search rows) and an `insert` at its first chunk, so any attempt would enter
//!   it. `update` and `hello` are left out on purpose: the missing-parameter
//!   fingerprint upsert and the driver's monitoring send them under the same
//!   `appName`;
//! - the debug line "transaction bundle admitted" is absent from the captured
//!   log, and the one WARN "transaction bundle refused before admission" is
//!   present with the entry count, the payload and the limit.
//!
//! These two show that nothing reached the server and that no admission was
//! logged. The admitted line is logged only after `gate.admit` returns, so a
//! refusal placed right after the admission would still pass them:
//! `an_over_limit_bundle_is_not_queued_behind_a_bundle_holding_the_room` here
//! and the lib test `a_bundle_over_the_limit_is_refused_while_the_admission_room_is_full`
//! are what pin the refusal before `gate.admit`.
//!
//! The same backend then commits an under-limit Bundle as the positive control:
//! it enters the failpoint and logs the admitted line, so the negative checks
//! above cannot pass vacuously.

use super::*;

use crate::bulk_submit::FailPoint;

/// The WARN the refusal logs.
const REFUSED: &str = "transaction bundle refused before admission";

/// The debug line a Bundle logs once it has taken its admission.
const ADMITTED: &str = "transaction bundle admitted";

/// A Bundle of `observations + 2` entries: a Patient POST (with a `urn:uuid`
/// `fullUrl`), `observations` Observation POSTs that reference it, and a PUT of
/// another Patient under a fresh id.
fn bundle(observations: usize) -> Vec<BundleEntry> {
    let patient_urn = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    let entry =
        |method: BundleMethod, url: String, resource: serde_json::Value, full_url| BundleEntry {
            method,
            url,
            resource: Some(resource),
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url,
            criteria: None,
        };
    let mut entries = vec![entry(
        BundleMethod::Post,
        "Patient".to_string(),
        json!({"resourceType": "Patient", "name": [{"family": "PayloadLimit"}]}),
        Some(patient_urn.clone()),
    )];
    for i in 0..observations {
        entries.push(entry(
            BundleMethod::Post,
            "Observation".to_string(),
            json!({
                "resourceType": "Observation",
                "status": "final",
                "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]},
                "effectiveDateTime": "2024-01-01T00:00:00Z",
                "valueQuantity": {"value": i, "unit": "bpm"},
                "subject": {"reference": patient_urn}
            }),
            Some(format!("urn:uuid:{}", uuid::Uuid::new_v4())),
        ));
    }
    let put_id = format!("pl-{}", uuid::Uuid::new_v4().simple());
    entries.push(entry(
        BundleMethod::Put,
        format!("Patient/{put_id}"),
        json!({"resourceType": "Patient", "id": put_id, "active": true}),
        None,
    ));
    entries
}

/// The BSON bytes of the POST and PUT resources of `entries`, summed here from
/// the driver's own encoding and independent of the backend's counter.
fn expected_payload(entries: &[BundleEntry]) -> u64 {
    entries
        .iter()
        .filter(|entry| matches!(entry.method, BundleMethod::Post | BundleMethod::Put))
        .filter_map(|entry| entry.resource.as_ref())
        .map(|resource| {
            let document = mongodb::bson::to_document(resource).expect("a resource is a document");
            mongodb::bson::RawDocumentBuf::from_document(&document)
                .expect("the document encodes")
                .as_bytes()
                .len() as u64
        })
        .sum()
}

/// A scoped (not global) subscriber for one test that records the storage
/// module's debug lines into a fresh buffer; the same arrangement as
/// `transaction_retry::capture_bundle_warnings`, one level lower. `set_default`
/// is thread-local, so this needs a current-thread runtime (`#[tokio::test]`),
/// and the Bundle's events are emitted on the test thread. Keep both returned
/// guards alive until the log assertions are done.
///
/// `tracing` caches, per callsite, whether any subscriber wants it. While only
/// one dispatcher is registered, it asks just the default of the thread that
/// reaches the callsite first, so the other test of this module, running on its
/// own thread without a subscriber and sending its own Bundles through the same
/// "transaction bundle admitted" callsite, could cache "never" for this test
/// too and hide the line (seen as a flake). A second live dispatcher, created
/// before this test's, makes `tracing` evaluate every callsite against all
/// registered dispatchers, and registering this test's rebuilds the cache.
fn capture_storage_debug() -> (
    &'static std::sync::Mutex<Vec<u8>>,
    tracing::dispatcher::DefaultGuard,
    tracing::Dispatch,
) {
    let second_dispatcher = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let buf: &'static std::sync::Mutex<Vec<u8>> =
        Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
    let writer = tracing_test::internal::MockWriter::new(buf);
    let dispatch = tracing_test::internal::get_subscriber(
        writer,
        "helios_persistence::backends::mongodb::storage=debug",
    );
    let guard = tracing::dispatcher::set_default(&dispatch);
    (buf, guard, second_dispatcher)
}

/// The captured lines that contain `needle`.
fn captured_lines(buf: &std::sync::Mutex<Vec<u8>>, needle: &str) -> Vec<String> {
    let buf = buf.lock().unwrap();
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|line| line.contains(needle))
        .map(str::to_string)
        .collect()
}

/// A backend with the default admission room (the gate is on, so an admitted
/// Bundle logs its line) and `limit` as its payload limit.
async fn backend_with_limit(test: &str, app: &str, limit: Option<u64>) -> Option<MongoBackend> {
    let defaults = MongoBackendConfig::default();
    super::transaction_retry::create_backend_with_app_name_and_gate(
        test,
        app,
        defaults.max_concurrent_transaction_bundles,
        defaults.transaction_bundle_weight_entries,
        |config| config.transaction_bundle_max_payload_bytes = limit,
    )
    .await
}

/// With the limit one byte under the Bundle's payload, the Bundle is refused
/// with its own cause in well under a second, before anything reaches the
/// server and before the admission, and nothing is stored. The same backend
/// then runs an under-limit Bundle as it always did.
#[tokio::test]
async fn a_bundle_over_the_payload_limit_is_refused_before_admission() {
    const TEST: &str = "txn_payload_limit_refused";
    const APP: &str = "txn-payload-limit-2351";
    const TENANT: &str = "txn-payload-limit-refused";

    let big = bundle(20);
    let big_entries = big.len();
    assert_eq!(big_entries, 22);
    let payload = expected_payload(&big);
    let limit = payload - 1;

    let Some(backend) = backend_with_limit(TEST, APP, Some(limit)).await else {
        return;
    };
    let tenant = create_tenant(TENANT);

    // Every command that opens, fills or ends a bundle transaction. The
    // failpoint is enabled after the backend is built, so schema setup is not
    // in its path.
    let Some(fail_point) = FailPoint::enable(
        APP,
        doc! {
            "failCommands": ["insert", "delete", "commitTransaction", "abortTransaction"],
            "blockConnection": true,
            "blockTimeMS": 1,
        },
        doc! { "times": 1_000_000 },
    )
    .await
    else {
        return;
    };
    let (logs, _guard, _second_dispatcher) = capture_storage_debug();

    let started = std::time::Instant::now();
    let result = backend
        .process_transaction(&tenant, big, FhirVersion::default())
        .await;
    let elapsed = started.elapsed();
    let sent = fail_point.entered_within(1, 500).await;

    match result {
        Err(TransactionError::TooLargeForCache {
            cause:
                TooLargeForCacheCause::PayloadOverLimit {
                    payload_bytes,
                    limit_bytes,
                },
            entries,
            reason,
        }) => {
            assert_eq!(payload_bytes, payload, "the backend counts the BSON bytes");
            assert_eq!(limit_bytes, limit);
            assert_eq!(entries, big_entries);
            assert!(reason.starts_with("Refused before admission"), "{reason}");
        }
        Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
            panic!("the refusal must come before the session is started")
        }
        other => panic!("expected the payload refusal, got {other:?}"),
    }
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "the refusal took {elapsed:?}"
    );
    assert!(
        !sent,
        "an insert, delete or transaction command reached the server for a refused Bundle"
    );
    assert!(
        captured_lines(logs, ADMITTED).is_empty(),
        "a refused Bundle was admitted: {:?}",
        captured_lines(logs, ADMITTED)
    );
    let refused_lines = captured_lines(logs, REFUSED);
    assert_eq!(refused_lines.len(), 1, "{refused_lines:?}");
    for needle in [
        format!("entries={big_entries}"),
        format!("payload_bytes={payload}"),
        format!("limit_bytes={limit}"),
    ] {
        assert!(
            refused_lines[0].contains(&needle),
            "missing {needle:?}: {}",
            refused_lines[0]
        );
    }
    for collection in ["resources", "resource_history"] {
        assert_eq!(
            count_docs(&backend, collection, doc! { "tenant_id": TENANT }).await,
            0,
            "{collection}: nothing may be stored for a refused Bundle"
        );
    }

    // Positive control, same backend, tenant and failpoint: a Bundle under the
    // limit runs. It enters the failpoint and logs its admission, which is what
    // makes the negative checks above mean something.
    let small = bundle(1);
    assert_eq!(small.len(), 3);
    assert!(expected_payload(&small) <= limit);
    let Some(done) = process_transaction_or_skip(&backend, &tenant, small, TEST).await else {
        fail_point.off().await;
        return;
    };
    assert_eq!(done.entries.len(), 3);
    assert!(
        done.entries.iter().all(|entry| entry.status == 201),
        "{:?}",
        done.entries.iter().map(|e| e.status).collect::<Vec<_>>()
    );
    assert!(
        fail_point.entered_within(1, 10_000).await,
        "the committed Bundle must enter the failpoint, or the negative check proves nothing"
    );
    fail_point.off().await;
    assert_eq!(captured_lines(logs, ADMITTED).len(), 1);
    assert_eq!(captured_lines(logs, REFUSED).len(), 1);
    for collection in ["resources", "resource_history"] {
        assert_eq!(
            count_docs(&backend, collection, doc! { "tenant_id": TENANT }).await,
            3,
            "{collection}: the under-limit Bundle is stored"
        );
    }
}

/// How long the holding Bundle of
/// [`an_over_limit_bundle_is_not_queued_behind_a_bundle_holding_the_room`] is
/// kept inside its first `insert`.
const HOLD_MS: i64 = 3_000;

/// The refusal comes before the admission, not just before the session. While
/// another Bundle holds the whole admission room (a 3-entry Bundle in a room of
/// one Bundle of 2 entries, kept inside a delayed `insert`), an over-limit
/// Bundle is still refused at once instead of queueing behind it: a refusal
/// placed after `gate.admit` would wait out the hold and miss the timeout below.
#[tokio::test]
async fn an_over_limit_bundle_is_not_queued_behind_a_bundle_holding_the_room() {
    const TEST: &str = "txn_payload_limit_room";
    const APP: &str = "txn-payload-limit-2351-room";
    const TENANT: &str = "txn-payload-limit-room";

    let over = bundle(20);
    let over_entries = over.len();
    let holder = bundle(1);
    assert_eq!(holder.len(), 3);
    let limit = expected_payload(&over) - 1;
    assert!(expected_payload(&holder) <= limit);

    let Some(backend) = super::transaction_retry::create_backend_with_app_name_and_gate(
        TEST,
        APP,
        1,
        2,
        |config| config.transaction_bundle_max_payload_bytes = Some(limit),
    )
    .await
    else {
        return;
    };
    let tenant = create_tenant(TENANT);

    // Only the holder's first `insert` is delayed, so it keeps its room for
    // `HOLD_MS` and then runs on.
    let Some(fail_point) = FailPoint::enable(
        APP,
        doc! {
            "failCommands": ["insert"],
            "blockConnection": true,
            "blockTimeMS": HOLD_MS,
        },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };

    let holding = backend.process_transaction(&tenant, holder, FhirVersion::default());
    let refusing = async {
        // The holder is inside the delayed `insert`: it has taken the room.
        let holding_the_room = fail_point.entered_within(1, 10_000).await;
        let started = std::time::Instant::now();
        let refused = tokio::time::timeout(
            std::time::Duration::from_millis(HOLD_MS as u64 / 2),
            backend.process_transaction(&tenant, over, FhirVersion::default()),
        )
        .await;
        (holding_the_room, refused, started.elapsed())
    };
    let (held, (holding_the_room, refused, elapsed)) = tokio::join!(holding, refusing);
    fail_point.off().await;

    let held = match held {
        Ok(done) => done,
        Err(TransactionError::UnsupportedIsolationLevel { .. }) if !transactions_required() => {
            eprintln!("Skipping {TEST} (MongoDB topology does not support transactions)");
            return;
        }
        Err(error) => panic!("{TEST}: the holding Bundle failed: {error}"),
    };
    assert!(
        holding_the_room,
        "the holding Bundle never reached its delayed insert, so the room was not held"
    );
    assert_eq!(held.entries.len(), 3);
    assert!(held.entries.iter().all(|entry| entry.status == 201));

    match refused.expect("the over-limit Bundle was queued behind the holder instead of refused") {
        Err(TransactionError::TooLargeForCache {
            cause: TooLargeForCacheCause::PayloadOverLimit { limit_bytes, .. },
            entries,
            ..
        }) => {
            assert_eq!(limit_bytes, limit);
            assert_eq!(entries, over_entries);
        }
        other => panic!("expected the payload refusal, got {other:?}"),
    }
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "the refusal took {elapsed:?}"
    );
    // Only the holder's three entries were stored.
    for collection in ["resources", "resource_history"] {
        assert_eq!(
            count_docs(&backend, collection, doc! { "tenant_id": TENANT }).await,
            3,
            "{collection}"
        );
    }
}

/// With the setting unset, or `Some(0)` (off), a Bundle of any size runs as it
/// did before the setting existed.
#[tokio::test]
async fn an_unset_or_zero_payload_limit_refuses_nothing() {
    for (name, limit) in [
        ("txn_payload_limit_unset", None),
        ("txn_payload_limit_zero", Some(0)),
    ] {
        let Some(backend) =
            backend_with_limit(name, &format!("txn-payload-limit-{name}"), limit).await
        else {
            return;
        };
        assert_eq!(backend.config().transaction_bundle_max_payload_bytes, limit);

        let tenant = create_tenant(name);
        let Some(done) = process_transaction_or_skip(&backend, &tenant, bundle(20), name).await
        else {
            return;
        };
        assert_eq!(done.entries.len(), 22, "{name}");
        assert!(
            done.entries.iter().all(|entry| entry.status == 201),
            "{name}: {:?}",
            done.entries.iter().map(|e| e.status).collect::<Vec<_>>()
        );
        for collection in ["resources", "resource_history"] {
            assert_eq!(
                count_docs(&backend, collection, doc! { "tenant_id": name }).await,
                22,
                "{name}: {collection}"
            );
        }
    }
}
