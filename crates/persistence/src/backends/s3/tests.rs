//! Unit tests for the S3 backend using an in-process mock S3 client.
//!
//! All tests run without AWS credentials. [`MockS3Client`] provides a
//! thread-safe in-memory S3 implementation with optional fault injection
//! for concurrency and rollback scenarios.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use serde_json::json;
use tokio::io::BufReader;

use crate::backends::s3::backend::S3Backend;
use crate::backends::s3::client::{
    ListObjectItem, ListObjectsResult, ObjectData, ObjectMetadata, S3Api, S3ClientError,
};
use crate::backends::s3::config::{S3BackendConfig, S3TenancyMode};
use crate::core::bulk_export::{ExportDataProvider, ExportRequest};
use crate::core::bulk_submit::{
    BulkProcessingOptions, BulkSubmitProvider, BulkSubmitRollbackProvider, NdjsonEntry,
    StreamingBulkSubmitProvider, SubmissionId, SubmissionStatus,
};
use crate::core::history::{
    HistoryParams, InstanceHistoryProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use crate::core::transaction::{BundleEntry, BundleMethod, BundleProvider};
use crate::core::{ResourceStorage, VersionedStorage};
use crate::error::{
    BackendError, BulkSubmitError, ConcurrencyError, ResourceError, SearchError, StorageError,
    TenantError, TransactionError,
};
use crate::tenant::{TenantContext, TenantId, TenantPermissions};
use crate::types::{CursorValue, PageCursor, Pagination, PaginationMode};

/// An in-memory representation of a single S3 object.
#[derive(Debug, Clone)]
struct MockObject {
    /// Raw object body.
    body: Vec<u8>,
    /// Monotonically assigned ETag string for conditional write testing.
    etag: String,
    /// Simulated last-modified timestamp.
    last_modified: DateTime<Utc>,
}

/// Shared mutable state backing `MockS3Client`.
#[derive(Debug, Default)]
struct MockState {
    /// Set of buckets that exist in the mock store.
    buckets: HashSet<String>,
    /// Stored objects keyed by `(bucket, key)`.
    objects: HashMap<(String, String), MockObject>,
    /// Monotonic counter used to generate unique ETags.
    etag_counter: u64,
    /// Total number of `put_object` calls received.
    put_count: u64,
    /// When set, puts fail once this call count is exceeded (fault injection).
    fail_put_after: Option<u64>,
    /// When true, all `delete_object` calls return an internal error.
    fail_deletes: bool,
}

/// An in-process S3 mock implementing `S3Api`.
///
/// Designed for deterministic unit tests that exercise the backend logic
/// without an AWS account. Supports optional fault injection to simulate
/// concurrent write conflicts and network errors.
#[derive(Debug, Clone, Default)]
struct MockS3Client {
    /// Shared state, cloneable across multiple backend instances in a test.
    state: Arc<Mutex<MockState>>,
}

impl MockS3Client {
    /// Creates a mock client with the specified buckets pre-seeded.
    fn with_buckets(buckets: &[&str]) -> Self {
        let mut state = MockState::default();
        state.buckets = buckets.iter().map(|b| (*b).to_string()).collect();
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Configures the mock to fail all `put_object` calls once `put_count`
    /// successful puts have been observed. Used to simulate partial-write
    /// failures during rollback testing.
    fn set_fail_put_after(&self, put_count: u64) {
        let mut state = self.state.lock().unwrap();
        state.fail_put_after = Some(put_count);
    }

    /// Returns the number of objects currently stored in `bucket`.
    fn bucket_object_count(&self, bucket: &str) -> usize {
        let state = self.state.lock().unwrap();
        state.objects.keys().filter(|(b, _)| b == bucket).count()
    }
}

#[async_trait]
impl S3Api for MockS3Client {
    // NOTE: the two HEAD operations below deliberately do NOT report
    // `BucketNotFound`, even though a missing bucket is exactly what they are up
    // against. A HEAD response has no body, so real S3 cannot return
    // `<Code>NoSuchBucket</Code>` on these paths — the SDK sees a bare 404 and
    // synthesizes `NotFound`, which `map_sdk_error` maps to
    // `S3ClientError::NotFound`. Making the mock answer `BucketNotFound` here
    // would let it claim a distinction the real client cannot make, and these
    // tests would then vouch for behaviour that does not exist in production.
    //
    // That gap is real and tracked in #284: `head_object` in `S3Client` swallows
    // `NotFound` into `Ok(None)`, so a missing bucket still reads as a missing
    // object on the `create` existence check and on the bulk-submit state read,
    // and `validate_buckets` reports a missing bucket as "resource not found in
    // S3". Until that is fixed, the mock stays honest about it rather than
    // papering over it.
    async fn head_bucket(&self, bucket: &str) -> Result<(), S3ClientError> {
        let state = self.state.lock().unwrap();
        if state.buckets.contains(bucket) {
            Ok(())
        } else {
            Err(S3ClientError::NotFound)
        }
    }

    async fn head_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectMetadata>, S3ClientError> {
        let state = self.state.lock().unwrap();
        // No bucket check: a missing bucket has no objects, so the lookup below
        // already yields `Ok(None)` — which is exactly what `S3Client::head_object`
        // produces against real S3, since it swallows the bodyless 404's
        // `NotFound` into `Ok(None)`.
        Ok(state
            .objects
            .get(&(bucket.to_string(), key.to_string()))
            .map(|object| ObjectMetadata {
                etag: Some(object.etag.clone()),
                last_modified: Some(object.last_modified),
                size: object.body.len() as i64,
            }))
    }

    async fn get_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectData>, S3ClientError> {
        let state = self.state.lock().unwrap();
        if !state.buckets.contains(bucket) {
            return Err(S3ClientError::BucketNotFound(bucket.to_string()));
        }
        Ok(state
            .objects
            .get(&(bucket.to_string(), key.to_string()))
            .map(|object| ObjectData {
                bytes: object.body.clone(),
                metadata: ObjectMetadata {
                    etag: Some(object.etag.clone()),
                    last_modified: Some(object.last_modified),
                    size: object.body.len() as i64,
                },
            }))
    }

    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        _content_type: Option<&str>,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<ObjectMetadata, S3ClientError> {
        let mut state = self.state.lock().unwrap();
        if !state.buckets.contains(bucket) {
            return Err(S3ClientError::BucketNotFound(bucket.to_string()));
        }
        state.put_count += 1;
        if let Some(fail_after) = state.fail_put_after {
            if state.put_count > fail_after {
                return Err(S3ClientError::Internal("forced put failure".to_string()));
            }
        }

        let entry_key = (bucket.to_string(), key.to_string());
        let existing = state.objects.get(&entry_key).cloned();

        if let Some("*") = if_none_match {
            if existing.is_some() {
                return Err(S3ClientError::PreconditionFailed);
            }
        }

        if let Some(expected) = if_match {
            let Some(existing) = existing.as_ref() else {
                return Err(S3ClientError::PreconditionFailed);
            };
            if existing.etag != expected {
                return Err(S3ClientError::PreconditionFailed);
            }
        }

        state.etag_counter += 1;
        let etag = format!("etag-{}", state.etag_counter);
        let object = MockObject {
            body,
            etag: etag.clone(),
            last_modified: Utc::now(),
        };
        state.objects.insert(entry_key, object);

        Ok(ObjectMetadata {
            etag: Some(etag),
            last_modified: Some(Utc::now()),
            size: 0,
        })
    }

    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), S3ClientError> {
        let mut state = self.state.lock().unwrap();
        if state.fail_deletes {
            return Err(S3ClientError::Internal("forced delete failure".to_string()));
        }
        state.objects.remove(&(bucket.to_string(), key.to_string()));
        Ok(())
    }

    async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
        continuation: Option<&str>,
        max_keys: Option<i32>,
    ) -> Result<ListObjectsResult, S3ClientError> {
        let state = self.state.lock().unwrap();
        // Faithful to S3: listing a bucket that does not exist is `NoSuchBucket`,
        // not an empty listing. Otherwise a misconfigured bucket would report zero
        // objects, and `count` would confidently answer "no resources".
        if !state.buckets.contains(bucket) {
            return Err(S3ClientError::BucketNotFound(bucket.to_string()));
        }
        let mut keys = state
            .objects
            .iter()
            .filter(|((b, key), _)| b == bucket && key.starts_with(prefix))
            .map(|((_, key), value)| ListObjectItem {
                key: key.clone(),
                etag: Some(value.etag.clone()),
                last_modified: Some(value.last_modified),
                size: value.body.len() as i64,
            })
            .collect::<Vec<_>>();

        keys.sort_by(|a, b| a.key.cmp(&b.key));

        let start = continuation
            .and_then(|token| token.parse::<usize>().ok())
            .unwrap_or(0)
            .min(keys.len());
        let max = max_keys.unwrap_or(1000).max(1) as usize;
        let end = start.saturating_add(max).min(keys.len());

        let items = keys[start..end].to_vec();
        let next_continuation_token = if end < keys.len() {
            Some(end.to_string())
        } else {
            None
        };

        Ok(ListObjectsResult {
            items,
            next_continuation_token,
        })
    }
}

/// Constructs a `PrefixPerTenant` backend backed by the given mock client.
fn make_prefix_backend(mock: Arc<MockS3Client>) -> S3Backend {
    let config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: "test-bucket".to_string(),
        },
        validate_buckets_on_startup: false,
        ..Default::default()
    };

    S3Backend::with_client(config, mock).expect("backend")
}

/// Constructs a `BucketPerTenant` backend backed by the given mock client
/// with `tenant-a → bucket-a`, `tenant-b → bucket-b`, and a system bucket.
fn make_bucket_backend(mock: Arc<MockS3Client>) -> S3Backend {
    let mut tenant_bucket_map = HashMap::new();
    tenant_bucket_map.insert("tenant-a".to_string(), "bucket-a".to_string());
    tenant_bucket_map.insert("tenant-b".to_string(), "bucket-b".to_string());

    let config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::BucketPerTenant {
            tenant_bucket_map,
            default_system_bucket: Some("system-bucket".to_string()),
        },
        validate_buckets_on_startup: false,
        ..Default::default()
    };

    S3Backend::with_client(config, mock).expect("backend")
}

/// Creates a full-access `TenantContext` for the given tenant ID string.
fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

#[tokio::test]
async fn crud_happy_path_and_count() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"p1","active":true}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let read = backend
        .read(&tenant, "Patient", "p1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.id(), created.id());

    let updated = backend
        .update(
            &tenant,
            &read,
            json!({"resourceType":"Patient","id":"p1","active":false}),
        )
        .await
        .unwrap();
    assert_eq!(updated.version_id(), "2");

    let count_before_delete = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count_before_delete, 1);

    backend.delete(&tenant, "Patient", "p1").await.unwrap();

    let count_after_delete = backend.count(&tenant, Some("Patient")).await.unwrap();
    assert_eq!(count_after_delete, 0);
}

#[tokio::test]
async fn crud_duplicate_create_and_missing_read() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"dup"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let duplicate = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"dup"}),
            FhirVersion::default(),
        )
        .await;

    assert!(matches!(
        duplicate,
        Err(StorageError::Resource(ResourceError::AlreadyExists { .. }))
    ));

    let missing = backend.read(&tenant, "Patient", "missing").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn crud_concurrent_create_race() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let b1 = backend.clone();
    let b2 = backend.clone();
    let t1 = tenant.clone();
    let t2 = tenant.clone();

    let fut1 = tokio::spawn(async move {
        b1.create(
            &t1,
            "Patient",
            json!({"resourceType":"Patient","id":"race"}),
            FhirVersion::default(),
        )
        .await
    });
    let fut2 = tokio::spawn(async move {
        b2.create(
            &t2,
            "Patient",
            json!({"resourceType":"Patient","id":"race"}),
            FhirVersion::default(),
        )
        .await
    });

    let r1 = fut1.await.unwrap();
    let r2 = fut2.await.unwrap();

    let success_count = [r1.is_ok(), r2.is_ok()].into_iter().filter(|v| *v).count();
    let exists_count = [r1, r2]
        .into_iter()
        .filter(|r| {
            matches!(
                r,
                Err(StorageError::Resource(ResourceError::AlreadyExists { .. }))
            )
        })
        .count();

    assert_eq!(success_count, 1);
    assert_eq!(exists_count, 1);
}

#[tokio::test]
async fn versioning_vread_and_conflict() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"v1","active":true}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = backend
        .update_with_match(
            &tenant,
            "Patient",
            "v1",
            created.version_id(),
            json!({"resourceType":"Patient","id":"v1","active":false}),
        )
        .await
        .unwrap();

    assert_eq!(updated.version_id(), "2");

    let versions = backend
        .list_versions(&tenant, "Patient", "v1")
        .await
        .unwrap();
    assert_eq!(versions, vec!["1".to_string(), "2".to_string()]);

    let first = backend
        .vread(&tenant, "Patient", "v1", "1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.version_id(), "1");

    let stale = backend
        .update_with_match(
            &tenant,
            "Patient",
            "v1",
            "1",
            json!({"resourceType":"Patient","id":"v1","active":true}),
        )
        .await;

    assert!(matches!(
        stale,
        Err(StorageError::Concurrency(
            ConcurrencyError::VersionConflict { .. }
        ))
    ));
}

#[tokio::test]
async fn versioning_parallel_updates_one_conflicts() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let current = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"parallel"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let expected = current.version_id().to_string();

    let b1 = backend.clone();
    let b2 = backend.clone();
    let t1 = tenant.clone();
    let t2 = tenant.clone();

    let f1 = tokio::spawn(async move {
        b1.update_with_match(
            &t1,
            "Patient",
            "parallel",
            &expected,
            json!({"resourceType":"Patient","id":"parallel","a":1}),
        )
        .await
    });

    let f2 = tokio::spawn(async move {
        b2.update_with_match(
            &t2,
            "Patient",
            "parallel",
            "1",
            json!({"resourceType":"Patient","id":"parallel","b":2}),
        )
        .await
    });

    let r1 = f1.await.unwrap();
    let r2 = f2.await.unwrap();

    let successes = [r1.is_ok(), r2.is_ok()].into_iter().filter(|v| *v).count();
    assert_eq!(successes, 1);
}

#[tokio::test]
async fn history_instance_type_system_and_invalid_cursor() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"h1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let updated = backend
        .update(
            &tenant,
            &created,
            json!({"resourceType":"Patient","id":"h1","active":true}),
        )
        .await
        .unwrap();

    backend.delete(&tenant, "Patient", "h1").await.unwrap();

    let history = backend
        .history_instance(
            &tenant,
            "Patient",
            "h1",
            &HistoryParams::new().include_deleted(true),
        )
        .await
        .unwrap();

    assert_eq!(history.items.len(), 3);
    assert_eq!(history.items[0].resource.version_id(), "3");
    assert_eq!(history.items[1].resource.version_id(), updated.version_id());

    let type_history = backend
        .history_type(
            &tenant,
            "Patient",
            &HistoryParams::new().include_deleted(true),
        )
        .await
        .unwrap();
    assert!(type_history.items.len() >= 3);

    let system_history = backend
        .history_system(&tenant, &HistoryParams::new().include_deleted(true))
        .await
        .unwrap();
    assert!(system_history.items.len() >= 3);

    let bad_cursor = PageCursor::new(vec![CursorValue::String("bad".to_string())], "oops").encode();
    let params = HistoryParams {
        pagination: Pagination {
            count: 10,
            mode: PaginationMode::Cursor(Some(PageCursor::decode(&bad_cursor).unwrap())),
        },
        ..HistoryParams::new()
    };

    let invalid = backend
        .history_instance(&tenant, "Patient", "h1", &params)
        .await;

    assert!(matches!(
        invalid,
        Err(StorageError::Search(SearchError::InvalidCursor { .. }))
    ));
}

#[tokio::test]
async fn bundle_batch_mixed_results() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"resourceType":"Patient","id":"b1"})),
            ..Default::default()
        },
        BundleEntry {
            method: BundleMethod::Get,
            url: "Patient/missing".to_string(),
            ..Default::default()
        },
    ];

    let result = backend.process_batch(&tenant, entries).await.unwrap();
    assert_eq!(result.entries.len(), 2);
    assert_eq!(result.entries[0].status, 201);
    assert_eq!(result.entries[1].status, 404);
}

#[tokio::test]
async fn bundle_transaction_success_and_reference_resolution() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            full_url: Some("urn:uuid:patient-1".to_string()),
            url: "Patient".to_string(),
            resource: Some(json!({"resourceType":"Patient","id":"tx-p1"})),
            ..Default::default()
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".to_string(),
            resource: Some(json!({
                "resourceType":"Observation",
                "id":"obs-1",
                "subject": {"reference": "urn:uuid:patient-1"}
            })),
            ..Default::default()
        },
    ];

    let result = backend.process_transaction(&tenant, entries).await.unwrap();
    assert_eq!(result.entries.len(), 2);

    let obs = backend
        .read(&tenant, "Observation", "obs-1")
        .await
        .unwrap()
        .unwrap();
    let reference = obs
        .content()
        .pointer("/subject/reference")
        .and_then(|v| v.as_str())
        .unwrap();

    assert_eq!(reference, "Patient/tx-p1");
}

#[tokio::test]
async fn bundle_transaction_failure_rolls_back() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"resourceType":"Patient","id":"rollback-me"})),
            ..Default::default()
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"id":"missing-resource-type"})),
            ..Default::default()
        },
    ];

    let result = backend.process_transaction(&tenant, entries).await;
    assert!(matches!(result, Err(TransactionError::BundleError { .. })));

    let read = backend.read(&tenant, "Patient", "rollback-me").await;
    assert!(matches!(
        read,
        Err(StorageError::Resource(ResourceError::Gone { .. }))
    ));
}

#[tokio::test]
async fn bundle_transaction_reports_rollback_failure() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    // First create writes 4 objects (current + history + type index + system index).
    // Start failing puts after that so compensation during rollback fails.
    mock.set_fail_put_after(4);
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"resourceType":"Patient","id":"rollback-failure"})),
            ..Default::default()
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"id":"invalid"})),
            ..Default::default()
        },
    ];

    let result = backend.process_transaction(&tenant, entries).await;
    match result {
        Err(TransactionError::BundleError { message, .. }) => {
            assert!(message.contains("rollback failed"));
        }
        other => panic!("expected rollback failure bundle error, got {other:?}"),
    }
}

// `bulk_export_start_manifest_and_delete` was removed: S3 no longer
// implements `BulkExportStorage` (job state lives in SQLite or PostgreSQL).
// The remaining bulk-export surface on the S3 backend is the
// `ExportDataProvider` data-feed, exercised by
// `bulk_export_fetch_batch_cursor` below.

#[tokio::test]
async fn bulk_export_fetch_batch_cursor() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    for i in 0..3 {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":format!("p{}", i)}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let request = ExportRequest::system();
    let batch1 = backend
        .fetch_export_batch(&tenant, &request, "Patient", None, 2)
        .await
        .unwrap();
    assert_eq!(batch1.lines.len(), 2);
    assert!(!batch1.is_last);

    let batch2 = backend
        .fetch_export_batch(
            &tenant,
            &request,
            "Patient",
            batch1.next_cursor.as_deref(),
            2,
        )
        .await
        .unwrap();
    assert_eq!(batch2.lines.len(), 1);
    assert!(batch2.is_last);
}

#[tokio::test]
async fn bulk_submit_lifecycle_and_processing() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let submission_id = SubmissionId::new("client-a", "sub-1");
    let summary = backend
        .create_submission(&tenant, &submission_id, None)
        .await
        .unwrap();
    assert_eq!(summary.status, SubmissionStatus::InProgress);

    let manifest = backend
        .add_manifest(&tenant, &submission_id, None, None)
        .await
        .unwrap();

    let entries = vec![
        NdjsonEntry::new(1, "Patient", json!({"resourceType":"Patient","id":"bs1"})),
        NdjsonEntry::new(2, "Patient", json!({"resourceType":"Patient","id":"bs2"})),
    ];

    let results = backend
        .process_entries(
            &tenant,
            &submission_id,
            &manifest.manifest_id,
            entries,
            &BulkProcessingOptions::new(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.is_success()));

    let counts = backend
        .get_entry_counts(&tenant, &submission_id, &manifest.manifest_id)
        .await
        .unwrap();
    assert_eq!(counts.total, 2);
    assert_eq!(counts.success, 2);

    let completed = backend
        .complete_submission(&tenant, &submission_id)
        .await
        .unwrap();
    assert_eq!(completed.status, SubmissionStatus::Complete);
}

#[tokio::test]
async fn bulk_submit_duplicate_abort_and_rollback() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let submission_id = SubmissionId::new("client-a", "sub-dup");
    backend
        .create_submission(&tenant, &submission_id, None)
        .await
        .unwrap();

    let duplicate = backend
        .create_submission(&tenant, &submission_id, None)
        .await;
    assert!(matches!(
        duplicate,
        Err(StorageError::BulkSubmit(
            BulkSubmitError::DuplicateSubmission { .. }
        ))
    ));

    let manifest = backend
        .add_manifest(&tenant, &submission_id, None, None)
        .await
        .unwrap();

    let entries = vec![NdjsonEntry::new(
        1,
        "Patient",
        json!({"resourceType":"Patient","id":"rollback-submit"}),
    )];
    backend
        .process_entries(
            &tenant,
            &submission_id,
            &manifest.manifest_id,
            entries,
            &BulkProcessingOptions::new(),
        )
        .await
        .unwrap();

    let changes = backend
        .list_changes(&tenant, &submission_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(changes.len(), 1);
    let rolled_back = backend
        .rollback_change(&tenant, &submission_id, &changes[0])
        .await
        .unwrap();
    assert!(rolled_back);

    // Keep one manifest pending so abort reports a cancellation count.
    backend
        .add_manifest(&tenant, &submission_id, None, None)
        .await
        .unwrap();

    let cancelled = backend
        .abort_submission(&tenant, &submission_id, "test abort")
        .await
        .unwrap();
    assert_eq!(cancelled, 1);
}

#[tokio::test]
async fn bulk_submit_stream_and_parallel_manifests_max_errors() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let submission_id = SubmissionId::new("client-stream", "sub-stream");
    backend
        .create_submission(&tenant, &submission_id, None)
        .await
        .unwrap();

    let m1 = backend
        .add_manifest(&tenant, &submission_id, None, None)
        .await
        .unwrap();
    let m2 = backend
        .add_manifest(&tenant, &submission_id, None, None)
        .await
        .unwrap();

    let ndjson = "{\"resourceType\":\"Patient\",\"id\":\"stream-1\"}\n";
    let reader = Box::new(BufReader::new(Cursor::new(ndjson.as_bytes().to_vec())));
    let stream_result = backend
        .process_ndjson_stream(
            &tenant,
            &submission_id,
            &m1.manifest_id,
            "Patient",
            reader,
            &BulkProcessingOptions::new(),
        )
        .await
        .unwrap();
    assert_eq!(stream_result.counts.success, 1);

    let strict = BulkProcessingOptions::new()
        .with_max_errors(1)
        .with_continue_on_error(false);

    let b1 = backend.clone();
    let b2 = backend.clone();
    let t1 = tenant.clone();
    let t2 = tenant.clone();
    let sub1 = submission_id.clone();
    let sub2 = submission_id.clone();
    let m1_id = m1.manifest_id.clone();
    let m2_id = m2.manifest_id.clone();

    let f1 = tokio::spawn(async move {
        b1.process_entries(
            &t1,
            &sub1,
            &m1_id,
            vec![
                NdjsonEntry::new(
                    1,
                    "Patient",
                    json!({"resourceType":"Observation","id":"x1"}),
                ),
                NdjsonEntry::new(2, "Patient", json!({"resourceType":"Patient","id":"x1"})),
                NdjsonEntry::new(3, "Patient", json!({"resourceType":"Patient","id":"x2"})),
            ],
            &strict,
        )
        .await
    });

    let f2 = tokio::spawn(async move {
        b2.process_entries(
            &t2,
            &sub2,
            &m2_id,
            vec![NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType":"Patient","id":"parallel-ok"}),
            )],
            &BulkProcessingOptions::new(),
        )
        .await
    });

    let r1 = f1.await.unwrap();
    let r2 = f2.await.unwrap();

    assert!(matches!(
        r1,
        Err(StorageError::BulkSubmit(
            BulkSubmitError::MaxErrorsExceeded { .. }
        ))
    ));
    assert!(r2.is_ok());
}

#[tokio::test]
async fn tenancy_prefix_and_bucket_modes() {
    let prefix_mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let prefix_backend = make_prefix_backend(prefix_mock);

    let ta = tenant("tenant-a");
    let tb = tenant("tenant-b");

    prefix_backend
        .create(
            &ta,
            "Patient",
            json!({"resourceType":"Patient","id":"same","a":1}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    prefix_backend
        .create(
            &tb,
            "Patient",
            json!({"resourceType":"Patient","id":"same","b":2}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let ra = prefix_backend
        .read(&ta, "Patient", "same")
        .await
        .unwrap()
        .unwrap();
    let rb = prefix_backend
        .read(&tb, "Patient", "same")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(ra.content()["a"], 1);
    assert_eq!(rb.content()["b"], 2);

    let bucket_mock = Arc::new(MockS3Client::with_buckets(&[
        "bucket-a",
        "bucket-b",
        "system-bucket",
    ]));
    let bucket_backend = make_bucket_backend(bucket_mock.clone());

    bucket_backend
        .create(
            &ta,
            "Patient",
            json!({"resourceType":"Patient","id":"same"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    bucket_backend
        .create(
            &tb,
            "Patient",
            json!({"resourceType":"Patient","id":"same"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    assert!(bucket_mock.bucket_object_count("bucket-a") > 0);
    assert!(bucket_mock.bucket_object_count("bucket-b") > 0);

    let missing_tenant = tenant("tenant-c");
    let missing = bucket_backend
        .create(
            &missing_tenant,
            "Patient",
            json!({"resourceType":"Patient","id":"x"}),
            FhirVersion::default(),
        )
        .await;

    assert!(matches!(
        missing,
        Err(StorageError::Tenant(TenantError::InvalidTenant { .. }))
    ));
}

#[tokio::test]
async fn tenant_registry_crud_prefix_mode() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);

    assert!(backend.supports_tenant_registry());

    let a = backend
        .register_tenant("tenant-a", Some("Tenant A"))
        .await
        .unwrap();
    assert_eq!(a.id, "tenant-a");
    assert_eq!(a.display_name.as_deref(), Some("Tenant A"));
    assert!(DateTime::parse_from_rfc3339(&a.created_at).is_ok());

    let b = backend.register_tenant("tenant-b", None).await.unwrap();
    assert_eq!(b.id, "tenant-b");
    assert!(b.display_name.is_none());

    let fetched = backend.get_tenant("tenant-a").await.unwrap().unwrap();
    assert_eq!(fetched, a);

    let listed = backend.list_tenants().await.unwrap();
    assert_eq!(listed, vec![a, b.clone()]);

    assert!(backend.deregister_tenant("tenant-a").await.unwrap());
    assert!(!backend.deregister_tenant("tenant-a").await.unwrap());
    assert!(backend.get_tenant("tenant-a").await.unwrap().is_none());
    assert_eq!(backend.list_tenants().await.unwrap(), vec![b]);
}

#[tokio::test]
async fn tenant_registry_register_duplicate_fails() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);

    backend.register_tenant("tenant-a", None).await.unwrap();

    let duplicate = backend.register_tenant("tenant-a", Some("Again")).await;
    assert!(matches!(
        duplicate,
        Err(StorageError::Backend(BackendError::QueryError { .. }))
    ));
}

#[tokio::test]
async fn tenant_registry_unsupported_without_system_bucket() {
    let mock = Arc::new(MockS3Client::with_buckets(&["bucket-a"]));
    let mut tenant_bucket_map = HashMap::new();
    tenant_bucket_map.insert("tenant-a".to_string(), "bucket-a".to_string());

    let config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::BucketPerTenant {
            tenant_bucket_map,
            default_system_bucket: None,
        },
        validate_buckets_on_startup: false,
        ..Default::default()
    };
    let backend = S3Backend::with_client(config, mock).expect("backend");

    assert!(!backend.supports_tenant_registry());
    assert!(matches!(
        backend.register_tenant("tenant-a", None).await,
        Err(StorageError::Backend(
            BackendError::UnsupportedCapability { .. }
        ))
    ));
    assert!(backend.list_tenants().await.unwrap().is_empty());
    assert!(backend.get_tenant("tenant-a").await.unwrap().is_none());
}

#[tokio::test]
async fn tenant_registry_bucket_mode_uses_system_bucket() {
    let mock = Arc::new(MockS3Client::with_buckets(&[
        "bucket-a",
        "bucket-b",
        "system-bucket",
    ]));
    let backend = make_bucket_backend(mock.clone());

    assert!(backend.supports_tenant_registry());

    let record = backend
        .register_tenant("tenant-a", Some("Tenant A"))
        .await
        .unwrap();
    assert_eq!(backend.get_tenant("tenant-a").await.unwrap(), Some(record));
    assert!(mock.bucket_object_count("system-bucket") > 0);
}

#[tokio::test]
async fn purge_tenant_data_sweeps_resources_and_history() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let ta = tenant("tenant-a");
    let tb = tenant("tenant-b");

    backend
        .create(
            &ta,
            "Patient",
            json!({"resourceType":"Patient","id":"p1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &ta,
            "Observation",
            json!({"resourceType":"Observation","id":"o1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tb,
            "Patient",
            json!({"resourceType":"Patient","id":"p2"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let removed = backend.purge_tenant_data("tenant-a").await.unwrap();
    assert_eq!(removed, 2);

    assert!(backend.read(&ta, "Patient", "p1").await.unwrap().is_none());
    assert!(
        backend
            .read(&ta, "Observation", "o1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .vread(&ta, "Patient", "p1", "1")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(backend.count(&ta, None).await.unwrap(), 0);

    assert!(backend.read(&tb, "Patient", "p2").await.unwrap().is_some());
    assert_eq!(backend.count(&tb, None).await.unwrap(), 1);
}

// ============================================================================
// Backend error handling — a misconfigured bucket is not an empty store
// ============================================================================
//
// Companion to `tests/backend_error_handling.rs`, which covers the *unreachable
// endpoint* case for every backend. This covers the case that needs a
// responsive-but-misconfigured S3, which only the mock client can express: the
// endpoint answers, but the configured bucket does not exist (a typo, a deleted
// bucket, or credentials that cannot see it).
//
// The danger is specific to S3. `read` legitimately returns `Ok(None)` when an
// object is absent, and S3 reports a missing bucket on the very same code path.
// If the two are conflated, every read against a misconfigured bucket answers
// "this resource does not exist" — a store that is merely pointed at the wrong
// place would look exactly like a store that is empty.
//
// Scope, so these tests are not read as more than they are: they cover the
// operations that reach S3 via GET / PUT / LIST, whose error responses carry a
// body and can therefore say `NoSuchBucket`. The HEAD paths cannot (#284) — see
// the note on `MockS3Client::head_bucket` — so `create` below is covered only
// because its `PutObject` fails, not because its `head_object` existence check
// notices the missing bucket. It does not.

/// A `read` against a bucket that does not exist must error, not report the
/// resource as absent.
#[tokio::test]
async fn s3_missing_bucket_read_is_an_error_not_an_empty_store() {
    // The backend is configured for "test-bucket"; the mock has no buckets at all.
    let mock = Arc::new(MockS3Client::default());
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let result = backend.read(&tenant, "Patient", "some-id").await;

    assert!(
        !matches!(result, Ok(None)),
        "read against a nonexistent bucket returned Ok(None) — a store pointed at \
         the wrong bucket must not be indistinguishable from an empty one"
    );
    assert!(
        matches!(result, Err(StorageError::Backend(_))),
        "expected a backend error from a nonexistent bucket, got {result:?}"
    );
}

/// The same applies to writes and counts: they must not silently succeed or
/// report zero against a bucket that isn't there.
///
/// `create` fails at its `PutObject`, not at its `head_object` existence check —
/// the check reports `Ok(None)` ("no such object, go ahead and create") because a
/// bodyless HEAD 404 cannot say `NoSuchBucket`. The contract holds here, but by
/// way of the write that follows rather than by the check noticing anything.
#[tokio::test]
async fn s3_missing_bucket_create_and_count_are_errors() {
    let mock = Arc::new(MockS3Client::default());
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType":"Patient"}),
            FhirVersion::default(),
        )
        .await;
    assert!(
        matches!(created, Err(StorageError::Backend(_))),
        "expected a backend error creating into a nonexistent bucket, got {created:?}"
    );

    let counted = backend.count(&tenant, Some("Patient")).await;
    assert!(
        !matches!(counted, Ok(0)),
        "count against a nonexistent bucket returned Ok(0) — 'zero resources' is a \
         claim about data we never reached"
    );
}
