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
use crate::backends::s3::keyspace::S3Keyspace;
use crate::backends::s3::user_settings::settings_object_id;
use crate::core::bulk_export::{ExportDataProvider, ExportRequest};
use crate::core::bulk_submit::{
    BulkProcessingOptions, BulkSubmitProvider, BulkSubmitRollbackProvider, NdjsonEntry,
    StreamingBulkSubmitProvider, SubmissionId, SubmissionStatus,
};
use crate::core::history::{
    HistoryParams, InstanceHistoryProvider, SystemHistoryProvider, TypeHistoryProvider,
};
use crate::core::transaction::{BundleEntry, BundleMethod, BundleProvider};
use crate::core::user_settings::SettingsStore;
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

/// The preconditions a single `put_object` call carried, recorded so tests can
/// assert that a write was conditional (never a blind overwrite).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedPut {
    /// Key the put targeted.
    key: String,
    /// Value of the `If-Match` precondition, if any.
    if_match: Option<String>,
    /// Value of the `If-None-Match` precondition, if any.
    if_none_match: Option<String>,
}

/// A one-shot "thief" run at the start of a `put_object` call, *before* its
/// preconditions are evaluated, simulating a concurrent writer that lands between
/// a caller's read and its write. This is what makes the compare-and-swap retry
/// paths deterministically testable.
type StealHook = Box<dyn FnOnce(&mut MockState) + Send>;

/// Shared mutable state backing `MockS3Client`.
#[derive(Default)]
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
    /// When true, every `put_object` fails its precondition, simulating a writer
    /// that loses the compare-and-swap race on every attempt.
    fail_all_puts_with_precondition: bool,
    /// Preconditions carried by each `put_object` call, in order.
    recorded_puts: Vec<RecordedPut>,
    /// A [`StealHook`] to run before the next `put_object` evaluates its
    /// preconditions, if one has been armed.
    steal_hook: Option<StealHook>,
}

// `steal_hook` holds a boxed closure, which has no `Debug`, so `MockState` is
// formatted without it.
impl std::fmt::Debug for MockState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockState")
            .field("buckets", &self.buckets)
            .field("objects", &self.objects)
            .field("put_count", &self.put_count)
            .field("recorded_puts", &self.recorded_puts)
            .finish_non_exhaustive()
    }
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

    /// Total number of `put_object` calls received so far.
    fn put_count(&self) -> u64 {
        self.state.lock().unwrap().put_count
    }

    /// The preconditions carried by every `put_object` call so far, in order.
    fn recorded_puts(&self) -> Vec<RecordedPut> {
        self.state.lock().unwrap().recorded_puts.clone()
    }

    /// Makes every subsequent `put_object` fail its precondition, simulating a
    /// writer that loses the compare-and-swap race on every single attempt.
    fn fail_all_puts_with_precondition(&self) {
        self.state.lock().unwrap().fail_all_puts_with_precondition = true;
    }

    /// Arranges for a concurrent writer to overwrite `key` with `body` exactly
    /// once, immediately before the next `put_object` call evaluates its
    /// preconditions.
    ///
    /// This lands the "thief" inside the caller's read-then-write window, which
    /// is the only way to deterministically drive the compare-and-swap conflict
    /// paths: the mock is synchronous, so genuinely concurrent tasks would
    /// otherwise run to completion one after another without interleaving.
    fn steal_once(&self, bucket: &str, key: &str, body: Vec<u8>) {
        let entry_key = (bucket.to_string(), key.to_string());
        let mut state = self.state.lock().unwrap();
        state.steal_hook = Some(Box::new(move |state: &mut MockState| {
            state.etag_counter += 1;
            let etag = format!("etag-{}", state.etag_counter);
            state.objects.insert(
                entry_key,
                MockObject {
                    body,
                    etag,
                    last_modified: Utc::now(),
                },
            );
        }));
    }
}

#[async_trait]
impl S3Api for MockS3Client {
    // The two HEAD operations below report `BucketNotFound` for a missing
    // bucket, mirroring the real client after #284. A HEAD response is bodyless,
    // so real S3 cannot tag it `<Code>NoSuchBucket</Code>` directly — but the
    // real `AwsS3Client` now disambiguates anyway: `head_bucket` treats its own
    // (unambiguous) 404 as `BucketNotFound`, and `head_object`, on a bodyless
    // 404, issues a follow-up `HeadBucket` to tell a missing object apart from a
    // missing bucket. So the mock is not inventing a distinction the real client
    // lacks — it is modelling the outcome the real client now produces.
    async fn head_bucket(&self, bucket: &str) -> Result<(), S3ClientError> {
        let state = self.state.lock().unwrap();
        if state.buckets.contains(bucket) {
            Ok(())
        } else {
            Err(S3ClientError::BucketNotFound(bucket.to_string()))
        }
    }

    async fn head_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectMetadata>, S3ClientError> {
        let state = self.state.lock().unwrap();
        // A missing bucket is an error, never `Ok(None)`: the real client reaches
        // this verdict via a follow-up `HeadBucket` on a bodyless 404; the mock
        // reaches it directly from its in-memory bucket set. Either way, a
        // misconfigured store must not read as an absent object.
        if !state.buckets.contains(bucket) {
            return Err(S3ClientError::BucketNotFound(bucket.to_string()));
        }
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
        state.recorded_puts.push(RecordedPut {
            key: key.to_string(),
            if_match: if_match.map(str::to_string),
            if_none_match: if_none_match.map(str::to_string),
        });
        if let Some(fail_after) = state.fail_put_after {
            if state.put_count > fail_after {
                return Err(S3ClientError::Internal("forced put failure".to_string()));
            }
        }

        if state.fail_all_puts_with_precondition {
            return Err(S3ClientError::PreconditionFailed);
        }

        // A concurrent writer landing between the caller's read and this write.
        // It runs before the preconditions below are evaluated, so a caller whose
        // ETag is now stale correctly sees `PreconditionFailed`.
        if let Some(steal) = state.steal_hook.take() {
            steal(&mut state);
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

    async fn list_common_prefixes(
        &self,
        bucket: &str,
        prefix: &str,
        delimiter: &str,
    ) -> Result<Vec<String>, S3ClientError> {
        let state = self.state.lock().unwrap();
        if !state.buckets.contains(bucket) {
            return Err(S3ClientError::BucketNotFound(bucket.to_string()));
        }
        let mut prefixes: Vec<String> = state
            .objects
            .keys()
            .filter(|(b, key)| b == bucket && key.starts_with(prefix))
            .filter_map(|(_, key)| {
                key[prefix.len()..]
                    .split_once(delimiter)
                    .map(|(segment, _)| format!("{prefix}{segment}{delimiter}"))
            })
            .collect();
        prefixes.sort();
        prefixes.dedup();
        Ok(prefixes)
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

    let result = backend
        .process_batch(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();
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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .unwrap();
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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await;
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

    let result = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await;
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
// The GET / PUT / LIST paths tell the two apart from the error body's
// `NoSuchBucket` code. The HEAD paths cannot — a HEAD response is bodyless — so
// #284's fix has the real client disambiguate for itself: `head_bucket` treats
// its own 404 as `BucketNotFound`, and `head_object` issues a follow-up
// `HeadBucket` on a 404. The tests below therefore assert the existence checks
// fail at the *check* (no write attempted), not merely because a later write
// happens to fail.

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
/// `create` now fails at its `head_object` existence check, before any write:
/// the check confirms the bucket and gets `BucketNotFound`, so no `PutObject` is
/// ever attempted. The `put_count == 0` assertion is what distinguishes this
/// from the old behaviour, where the check reported `Ok(None)` and the error
/// only surfaced because the following write failed.
#[tokio::test]
async fn s3_missing_bucket_create_and_count_are_errors() {
    let mock = Arc::new(MockS3Client::default());
    let backend = make_prefix_backend(mock.clone());
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
    assert_eq!(
        mock.put_count(),
        0,
        "create into a nonexistent bucket must fail at the existence check, not \
         at a subsequent write — no PutObject should have been attempted"
    );

    let counted = backend.count(&tenant, Some("Patient")).await;
    assert!(
        !matches!(counted, Ok(0)),
        "count against a nonexistent bucket returned Ok(0) — 'zero resources' is a \
         claim about data we never reached"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// #284: a missing bucket must never masquerade as a missing object on the HEAD
// paths. These pin the client seam (so the mock stays in step with the real
// `AwsS3Client`) and both `head_object` existence-check callers. The real
// client is exercised end-to-end against a live server in `minio_s3_tests.rs`.
// ─────────────────────────────────────────────────────────────────────────────

/// `head_object` against a bucket that does not exist is a bucket-level error,
/// not `Ok(None)`. (Acceptance criterion 1.)
#[tokio::test]
async fn head_object_missing_bucket_is_bucket_not_found() {
    let mock = MockS3Client::default();
    let result = mock.head_object("test-bucket", "some-key").await;
    assert!(
        matches!(&result, Err(S3ClientError::BucketNotFound(b)) if b == "test-bucket"),
        "head_object against a nonexistent bucket must surface BucketNotFound, got {result:?}"
    );
}

/// A genuinely absent object in an existing bucket is still `Ok(None)` — the
/// #284 fix must not turn an ordinary miss into an error.
#[tokio::test]
async fn head_object_absent_object_in_existing_bucket_is_ok_none() {
    let mock = MockS3Client::with_buckets(&["test-bucket"]);
    let result = mock.head_object("test-bucket", "absent-key").await;
    assert!(
        matches!(result, Ok(None)),
        "an absent object in an existing bucket must remain Ok(None), got {result:?}"
    );
}

/// A present object still reports its metadata — guards against the new bucket
/// check short-circuiting a real hit.
#[tokio::test]
async fn head_object_present_object_is_ok_some() {
    let mock = MockS3Client::with_buckets(&["test-bucket"]);
    mock.put_object("test-bucket", "k", b"body".to_vec(), None, None, None)
        .await
        .expect("seed object");
    let result = mock.head_object("test-bucket", "k").await;
    assert!(
        matches!(result, Ok(Some(_))),
        "a present object must report Ok(Some(_)), got {result:?}"
    );
}

/// `head_bucket` against a nonexistent bucket is `BucketNotFound`, so
/// `validate_buckets` can name the bucket. (Underpins acceptance criterion 2.)
#[tokio::test]
async fn head_bucket_missing_is_bucket_not_found() {
    let mock = MockS3Client::default();
    let result = mock.head_bucket("test-bucket").await;
    assert!(
        matches!(&result, Err(S3ClientError::BucketNotFound(b)) if b == "test-bucket"),
        "head_bucket against a nonexistent bucket must be BucketNotFound, got {result:?}"
    );
}

#[tokio::test]
async fn head_bucket_existing_is_ok() {
    let mock = MockS3Client::with_buckets(&["test-bucket"]);
    assert!(mock.head_bucket("test-bucket").await.is_ok());
}

/// `validate_buckets` against a nonexistent bucket reports the bucket as
/// missing, not "resource not found in S3". (Acceptance criterion 2.)
#[tokio::test]
async fn s3_validate_buckets_missing_bucket_is_bucket_flavored_error() {
    let backend = make_prefix_backend(Arc::new(MockS3Client::default()));
    let err = backend
        .validate_buckets()
        .await
        .expect_err("validate_buckets must fail against a nonexistent bucket");
    match &err {
        StorageError::Backend(BackendError::Unavailable { message, .. }) => {
            assert!(
                message.contains("bucket"),
                "expected a bucket-flavored message, got {message:?}"
            );
            assert!(
                !message.contains("resource not found in S3"),
                "validate_buckets must not report a missing bucket as a missing resource, \
                 got {message:?}"
            );
        }
        other => panic!("expected Backend(Unavailable), got {other:?}"),
    }
}

/// The bulk-submit duplicate check (the second `head_object` existence-check
/// caller) also errors at the *check* against a missing bucket, before writing
/// any state. (Acceptance criterion 4, for `create_submission`.)
#[tokio::test]
async fn s3_bulk_submit_missing_bucket_errors_at_the_check() {
    let mock = Arc::new(MockS3Client::default());
    let backend = make_prefix_backend(mock.clone());
    let tenant = tenant("tenant-a");
    let id = SubmissionId::new("client-a", "sub-1");

    let result = backend.create_submission(&tenant, &id, None).await;
    assert!(
        matches!(result, Err(StorageError::Backend(_))),
        "create_submission into a nonexistent bucket must be a backend error, got {result:?}"
    );
    assert_eq!(
        mock.put_count(),
        0,
        "the duplicate check must fail before any submission state is written"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-user settings store
//
// The settings store maps the trait's read-modify-write + monotonic-version
// contract onto S3 conditional writes. These tests cover both the semantics
// shared with the SQLite/PostgreSQL/MongoDB suites and the S3-specific
// compare-and-swap and object-naming behaviour that has no analogue there.
// ─────────────────────────────────────────────────────────────────────────────

/// Builds a settings-store backend over a mock holding `test-bucket`.
fn settings_backend() -> (S3Backend, Arc<MockS3Client>) {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    (make_prefix_backend(mock.clone()), mock)
}

/// The S3 key a user's settings object lives at, for tests that need to reach
/// past the store and manipulate the raw object.
fn settings_key(user_key: &str) -> String {
    S3Keyspace::new(None).user_settings_key(&settings_object_id(user_key))
}

/// Serialises a settings object exactly as the store does, for seeding the mock
/// with a "concurrent writer's" document.
fn settings_body(user_key: &str, document: serde_json::Value, version: i64) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "user_key": user_key,
        "document": document,
        "version": version,
        "updated_at": Utc::now().to_rfc3339(),
    }))
    .expect("serialize settings object")
}

#[tokio::test]
async fn settings_get_returns_none_for_unknown_user() {
    let (backend, _mock) = settings_backend();
    assert!(backend.get_settings("ghost").await.unwrap().is_none());
}

#[tokio::test]
async fn settings_put_then_get_round_trips_document() {
    let (backend, _mock) = settings_backend();
    let doc = json!({"theme": "dark", "recentQueries": {"Patient": ["name=smith"]}});

    let stored = backend.put_settings("u1", doc.clone(), None).await.unwrap();
    assert_eq!(stored.version, 1);

    let fetched = backend.get_settings("u1").await.unwrap().unwrap();
    assert_eq!(fetched.document, doc);
    assert_eq!(fetched.version, 1);
    assert_eq!(fetched.user_key, "u1");
}

#[tokio::test]
async fn settings_version_increments_on_each_write() {
    let (backend, _mock) = settings_backend();
    backend
        .put_settings("u1", json!({"a": 1}), None)
        .await
        .unwrap();
    let second = backend
        .put_settings("u1", json!({"a": 2}), None)
        .await
        .unwrap();
    assert_eq!(second.version, 2);
}

#[tokio::test]
async fn settings_patch_merges_and_deletes_keys() {
    let (backend, _mock) = settings_backend();
    backend
        .put_settings(
            "u1",
            json!({"theme": "dark", "defaultTenant": "acme"}),
            None,
        )
        .await
        .unwrap();

    let patched = backend
        .patch_settings("u1", json!({"theme": "light", "defaultTenant": null}), None)
        .await
        .unwrap();

    assert_eq!(patched.document, json!({"theme": "light"}));
    assert_eq!(patched.version, 2);
}

#[tokio::test]
async fn settings_patch_on_missing_user_creates_document() {
    let (backend, _mock) = settings_backend();
    let patched = backend
        .patch_settings("u1", json!({"theme": "dark"}), None)
        .await
        .unwrap();
    assert_eq!(patched.document, json!({"theme": "dark"}));
    assert_eq!(patched.version, 1);
}

#[tokio::test]
async fn settings_stale_if_match_is_rejected() {
    let (backend, _mock) = settings_backend();
    backend
        .put_settings("u1", json!({"a": 1}), None)
        .await
        .unwrap(); // version 1

    // `Some(0)` asserts "does not exist yet", which is now false.
    let err = backend
        .put_settings("u1", json!({"a": 2}), Some(0))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })
    ));
}

#[tokio::test]
async fn settings_matching_if_match_succeeds() {
    let (backend, _mock) = settings_backend();
    // `Some(0)` asserts "does not exist yet" for the first write.
    backend
        .put_settings("u1", json!({"a": 1}), Some(0))
        .await
        .unwrap();
    let updated = backend
        .put_settings("u1", json!({"a": 2}), Some(1))
        .await
        .unwrap();
    assert_eq!(updated.version, 2);
}

/// Every settings write must carry a precondition. An unconditional `PutObject`
/// on this path is exactly what a silent lost update looks like, so this is a
/// regression guard, not a style check.
#[tokio::test]
async fn settings_writes_are_never_unconditional() {
    let (backend, mock) = settings_backend();
    backend
        .put_settings("u1", json!({"a": 1}), None)
        .await
        .unwrap();
    backend
        .patch_settings("u1", json!({"b": 2}), None)
        .await
        .unwrap();

    let puts = mock.recorded_puts();
    assert_eq!(puts.len(), 2);

    // The create asserts the object does not exist; the update pins the ETag it
    // read. Neither is a blind overwrite.
    assert_eq!(puts[0].if_none_match.as_deref(), Some("*"));
    assert!(puts[0].if_match.is_none());
    assert!(puts[1].if_match.is_some());
    assert!(puts[1].if_none_match.is_none());

    for put in puts {
        assert!(
            put.if_match.is_some() || put.if_none_match.is_some(),
            "settings write to {} carried no precondition",
            put.key
        );
    }
}

/// An unconditional write that loses the compare-and-swap must re-read and retry
/// on top of the winner's document — never clobber it.
#[tokio::test]
async fn settings_unconditional_write_retries_and_preserves_concurrent_update() {
    let (backend, mock) = settings_backend();
    backend
        .put_settings("u1", json!({"a": 1}), None)
        .await
        .unwrap(); // version 1

    // A concurrent writer lands version 2 inside our read-then-write window.
    mock.steal_once(
        "test-bucket",
        &settings_key("u1"),
        settings_body("u1", json!({"a": 1, "thief": true}), 2),
    );

    let patched = backend
        .patch_settings("u1", json!({"mine": true}), None)
        .await
        .unwrap();

    // We lost the first CAS, re-read the thief's document, and merged onto it.
    assert_eq!(patched.version, 3);
    assert_eq!(patched.document["thief"], json!(true), "lost update");
    assert_eq!(patched.document["mine"], json!(true));
}

/// A *conditional* write that loses the race must fail the precondition rather
/// than retry: the caller asked to write only if the version was still `n`.
#[tokio::test]
async fn settings_conditional_write_does_not_retry_on_lost_race() {
    let (backend, mock) = settings_backend();
    backend
        .put_settings("u1", json!({"a": 1}), None)
        .await
        .unwrap(); // version 1
    let puts_before = mock.put_count();

    mock.steal_once(
        "test-bucket",
        &settings_key("u1"),
        settings_body("u1", json!({"a": 99}), 2),
    );

    let err = backend
        .put_settings("u1", json!({"a": 2}), Some(1))
        .await
        .unwrap_err();

    match err {
        StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
            expected_etag,
            actual_etag,
            ..
        }) => {
            assert_eq!(expected_etag, "W/\"1\"");
            assert_eq!(actual_etag.as_deref(), Some("W/\"2\""));
        }
        other => panic!("expected an optimistic-lock failure, got {other:?}"),
    }

    // Exactly one write attempt: a conditional write must not loop.
    assert_eq!(mock.put_count() - puts_before, 1);
}

#[tokio::test]
async fn settings_get_surfaces_backend_error_for_corrupt_object() {
    let (backend, mock) = settings_backend();
    mock.put_object(
        "test-bucket",
        &settings_key("u1"),
        b"not json".to_vec(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    match backend.get_settings("u1").await.unwrap_err() {
        StorageError::Backend(BackendError::SerializationError { message }) => assert!(
            message.starts_with("read user_settings:"),
            "error should name the operation, got: {message}"
        ),
        other => panic!("expected a serialization error, got {other:?}"),
    }
}

/// The object body records its owner. If the key derivation ever regressed so
/// that two users shared an object, the read must fail rather than hand back
/// another user's settings.
#[tokio::test]
async fn settings_get_rejects_object_owned_by_another_user() {
    let (backend, mock) = settings_backend();
    mock.put_object(
        "test-bucket",
        &settings_key("u1"),
        settings_body("someone-else", json!({"theme": "dark"}), 1),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    match backend.get_settings("u1").await.unwrap_err() {
        StorageError::Backend(BackendError::Internal { message, .. }) => assert!(
            message.contains("belongs to a different user"),
            "the tripwire should say why it fired, got: {message}"
        ),
        other => panic!("expected an internal error, got {other:?}"),
    }
}

/// The user key is hashed, never embedded. Keys that a lossy sanitiser would
/// collapse together must map to distinct objects — a collision here would let
/// one user read and overwrite another's settings.
#[test]
fn settings_object_id_is_injective_for_keys_a_sanitiser_would_collide() {
    // `sanitize()` maps both '/' and ' ' to '_', so these three would collide.
    let a = settings_object_id("https://idp.example.com/realms/x|alice");
    let b = settings_object_id("https:__idp.example.com_realms_x|alice");
    let c = settings_object_id("https://idp.example.com realms x|alice");
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);

    // Fixed-length, path-safe: no separator can be smuggled into the key, and no
    // user key — however long — can exceed S3's 1024-byte key limit.
    for id in [&a, &b, &c] {
        assert_eq!(id.len(), 64);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }

    // A traversal attempt cannot escape the settings namespace.
    let traversal = settings_key("../../resources/Patient/p1/current");
    assert!(!traversal.contains(".."));
    assert!(traversal.starts_with("_system.user-settings/"));
}

/// Settings are user-global: the key must not sit under any tenant prefix, and
/// must not collide with the FHIR resource or history keyspaces.
#[tokio::test]
async fn settings_key_is_tenant_independent_and_outside_the_fhir_keyspace() {
    let (backend, mock) = settings_backend();
    backend
        .put_settings("u1", json!({"theme": "dark"}), None)
        .await
        .unwrap();

    let keyspace = S3Keyspace::new(None);
    let key = settings_key("u1");

    assert!(!key.starts_with(&keyspace.resources_prefix()));
    assert!(!key.starts_with("history/"));
    assert!(!key.starts_with("bulk/"));

    // The object really is where we think it is, and it is the only one written.
    assert_eq!(mock.bucket_object_count("test-bucket"), 1);
    assert!(
        mock.get_object("test-bucket", &key)
            .await
            .unwrap()
            .is_some()
    );

    // The adversarial case: even a tenant named after the settings namespace
    // itself cannot reach these objects, because its keys live under a
    // `resources/`/`history/` sub-prefix while a settings object is a
    // `{digest}.json` leaf. This is the structural invariant the key shape
    // relies on — it does NOT depend on tenant-ID validation.
    let hostile = keyspace.with_tenant_prefix("_system.user-settings");
    assert!(!key.starts_with(&hostile.resources_prefix()));
    assert!(!key.starts_with(&hostile.history_type_prefix("Patient")));
    let benign = keyspace.with_tenant_prefix("default");
    assert!(!key.starts_with(&benign.resources_prefix()));
}

/// In bucket-per-tenant mode the settings object has no tenant to key off, so it
/// lands in the tenant-independent system bucket.
#[tokio::test]
async fn settings_bucket_per_tenant_uses_the_system_bucket() {
    let mock = Arc::new(MockS3Client::with_buckets(&[
        "bucket-a",
        "bucket-b",
        "system-bucket",
    ]));
    let backend = make_bucket_backend(mock.clone());

    backend
        .put_settings("u1", json!({"theme": "dark"}), None)
        .await
        .unwrap();

    assert_eq!(mock.bucket_object_count("system-bucket"), 1);
    assert_eq!(mock.bucket_object_count("bucket-a"), 0);
    assert_eq!(mock.bucket_object_count("bucket-b"), 0);
}

/// Bucket-per-tenant without a system bucket has nowhere tenant-independent to
/// put settings. That must be an actionable configuration error, not a panic and
/// not a silent write into some arbitrary tenant's bucket.
#[tokio::test]
async fn settings_bucket_per_tenant_without_system_bucket_is_a_config_error() {
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
    let backend = S3Backend::with_client(config, mock.clone()).expect("backend");

    match backend
        .put_settings("u1", json!({"theme": "dark"}), None)
        .await
        .unwrap_err()
    {
        StorageError::Backend(BackendError::Internal { message, .. }) => assert!(
            message.contains("default_system_bucket"),
            "the error must name the config key to set, got: {message}"
        ),
        other => panic!("expected an internal error, got {other:?}"),
    }
    assert_eq!(mock.bucket_object_count("bucket-a"), 0);

    // And the server must not advertise the store at all in this configuration,
    // so `/_user/settings` reports the explained 501 instead of failing.
    assert!(!backend.supports_user_settings());
}

/// A create that loses the race must retry and update the winner's document
/// rather than fail — the `If-None-Match: *` branch of the compare-and-swap,
/// which the update-path tests never reach.
#[tokio::test]
async fn settings_unconditional_create_retries_when_another_writer_creates_first() {
    let (backend, mock) = settings_backend();

    // No object exists, so our first attempt will be a create. A concurrent
    // writer creates one first, inside our read-then-write window.
    mock.steal_once(
        "test-bucket",
        &settings_key("u1"),
        settings_body("u1", json!({"first": true}), 1),
    );

    let patched = backend
        .patch_settings("u1", json!({"second": true}), None)
        .await
        .unwrap();

    // We lost the create, re-read, and merged onto the winner's document.
    assert_eq!(patched.version, 2);
    assert_eq!(patched.document["first"], json!(true), "lost update");
    assert_eq!(patched.document["second"], json!(true));

    let puts = mock.recorded_puts();
    assert_eq!(puts[0].if_none_match.as_deref(), Some("*"));
    assert!(
        puts[1].if_match.is_some(),
        "retry must pin the winner's ETag"
    );
}

/// A `Some(0)` ("must not exist") create that loses the race must surface an
/// optimistic-lock failure and must not overwrite the winner.
#[tokio::test]
async fn settings_create_only_write_losing_the_race_does_not_overwrite() {
    let (backend, mock) = settings_backend();

    mock.steal_once(
        "test-bucket",
        &settings_key("u1"),
        settings_body("u1", json!({"winner": true}), 1),
    );

    let err = backend
        .put_settings("u1", json!({"loser": true}), Some(0))
        .await
        .unwrap_err();

    match err {
        StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
            expected_etag,
            actual_etag,
            ..
        }) => {
            assert_eq!(expected_etag, "W/\"0\"");
            assert_eq!(actual_etag.as_deref(), Some("W/\"1\""));
        }
        other => panic!("expected an optimistic-lock failure, got {other:?}"),
    }

    // The winner's document is intact.
    let stored = backend.get_settings("u1").await.unwrap().unwrap();
    assert_eq!(stored.document, json!({"winner": true}));
    assert_eq!(stored.version, 1);
}

/// A writer that loses every race must give up after a bounded number of
/// attempts with a concurrency error, rather than looping forever.
///
/// `start_paused` lets tokio auto-advance its clock through the retry backoffs,
/// so this exercises all 8 attempts without actually sleeping through them.
#[tokio::test(start_paused = true)]
async fn settings_write_gives_up_after_bounded_attempts() {
    let (backend, mock) = settings_backend();
    mock.fail_all_puts_with_precondition();

    let err = backend
        .put_settings("u1", json!({"a": 1}), None)
        .await
        .unwrap_err();

    match err {
        StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
            expected_etag,
            actual_etag,
            ..
        }) => {
            assert_eq!(expected_etag, "W/\"*\"");
            assert!(actual_etag.is_none());
        }
        other => panic!("expected an optimistic-lock failure, got {other:?}"),
    }

    // Bounded: it tried, and stopped.
    assert_eq!(mock.put_count(), 8);
}

/// The settings key must sit under the configured global prefix. Without this,
/// a regression that dropped the prefix would pass every other test, since they
/// all run with no prefix configured.
#[tokio::test]
async fn settings_key_respects_the_configured_global_prefix() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let config = S3BackendConfig {
        tenancy_mode: S3TenancyMode::PrefixPerTenant {
            bucket: "test-bucket".to_string(),
        },
        prefix: Some("hfs-data".to_string()),
        validate_buckets_on_startup: false,
        ..Default::default()
    };
    let backend = S3Backend::with_client(config, mock.clone()).expect("backend");

    backend
        .put_settings("u1", json!({"theme": "dark"}), None)
        .await
        .unwrap();

    let expected = format!(
        "hfs-data/_system.user-settings/{}.json",
        settings_object_id("u1")
    );
    assert!(
        mock.get_object("test-bucket", &expected)
            .await
            .unwrap()
            .is_some(),
        "settings object not found at the prefixed key {expected}"
    );

    // Round-trips through the same prefixed key.
    let stored = backend.get_settings("u1").await.unwrap().unwrap();
    assert_eq!(stored.document, json!({"theme": "dark"}));
}

// ---------------------------------------------------------------------------
// Issue #271 — a tenant whose id names the registry namespace
// ---------------------------------------------------------------------------

/// A tenant named `tenants` writes under `tenants/resources/…`, which shares the
/// registry's list prefix. Its resources must not be read back as registry
/// records.
#[tokio::test]
async fn list_tenants_ignores_data_written_by_a_tenant_named_tenants() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);

    let acme = backend.register_tenant("acme", None).await.unwrap();

    let hostile = tenant("tenants");
    backend
        .create(
            &hostile,
            "Patient",
            json!({"resourceType":"Patient","id":"p1","active":true}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let listed = backend.list_tenants().await.unwrap();
    assert_eq!(
        listed,
        vec![acme],
        "a resource under `tenants/` must not surface as a registry record"
    );
}

/// The permanent-failure half of #271: a history index event has no
/// `created_at`, so deserializing it as a `TenantRecord` fails. That must skip
/// the object, not abort the listing — otherwise the admin tenant list and the
/// UI tenants page return 500 forever with no operator recovery path.
#[tokio::test]
async fn list_tenants_survives_history_events_under_the_registry_prefix() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);

    let acme = backend.register_tenant("acme", None).await.unwrap();

    // Create, update, and delete so both type- and system-level history index
    // events exist under `tenants/history/…`.
    let hostile = tenant("tenants");
    let created = backend
        .create(
            &hostile,
            "Patient",
            json!({"resourceType":"Patient","id":"p1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .update(
            &hostile,
            &created,
            json!({"resourceType":"Patient","id":"p1","active":true}),
        )
        .await
        .unwrap();
    backend.delete(&hostile, "Patient", "p1").await.unwrap();

    let listed = backend
        .list_tenants()
        .await
        .expect("history events under the registry prefix must not fail the listing");
    assert_eq!(listed, vec![acme]);
}

/// A bucket already poisoned before the upgrade must recover on read alone.
///
/// The hostile objects are written *raw* through the mock client rather than via
/// the backend API, so this keeps testing the real corrupted-bucket shape even if
/// the registry is later relocated.
#[tokio::test]
async fn list_tenants_recovers_from_an_already_poisoned_bucket() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));

    // A resource object: deserializes cleanly into a TenantRecord (both `id` and
    // `created_at` are present), so pre-fix it injected a phantom tenant row.
    mock.put_object(
        "test-bucket",
        "tenants/resources/Patient/p1/current.json",
        serde_json::to_vec(&json!({
            "id": "p1",
            "created_at": "2026-01-01T00:00:00Z",
            "resource": {"resourceType": "Patient", "id": "p1"}
        }))
        .unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    // A history index event: no `created_at`, so pre-fix it hard-failed the call.
    mock.put_object(
        "test-bucket",
        "tenants/history/system/1700000000000_Patient_p1_1_abc.json",
        serde_json::to_vec(&json!({
            "id": "p1",
            "resource_type": "Patient",
            "version_id": "1"
        }))
        .unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let backend = make_prefix_backend(mock);
    let acme = backend.register_tenant("acme", None).await.unwrap();

    let listed = backend
        .list_tenants()
        .await
        .expect("a poisoned bucket must recover on upgrade, without bucket surgery");
    assert_eq!(listed, vec![acme]);
}

/// `sanitize()` mapped `/` to `_`, so `a/b` and `a_b` shared one registry
/// object. That is a cross-tenant read and delete, not a cosmetic listing bug,
/// so `get_tenant` and `deregister_tenant` are asserted too.
#[tokio::test]
async fn registry_distinguishes_ids_a_sanitiser_would_collide() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);

    let slashed = backend
        .register_tenant("a/b", Some("Slashed"))
        .await
        .unwrap();
    let underscored = backend
        .register_tenant("a_b", Some("Underscored"))
        .await
        .unwrap();

    assert_eq!(backend.list_tenants().await.unwrap().len(), 2);

    let got_slashed = backend.get_tenant("a/b").await.unwrap().unwrap();
    let got_underscored = backend.get_tenant("a_b").await.unwrap().unwrap();
    assert_eq!(got_slashed, slashed);
    assert_eq!(got_underscored, underscored);
    assert_eq!(got_slashed.display_name.as_deref(), Some("Slashed"));

    // Deregistering one must leave the other intact.
    assert!(backend.deregister_tenant("a/b").await.unwrap());
    assert!(backend.get_tenant("a/b").await.unwrap().is_none());
    assert_eq!(
        backend.get_tenant("a_b").await.unwrap(),
        Some(underscored),
        "deregistering `a/b` must not remove `a_b`"
    );
}

/// A record written under the pre-fix key shape stays readable after upgrade, so
/// the escaping change does not orphan existing registrations.
#[tokio::test]
async fn registry_reads_records_written_under_the_legacy_key() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));

    // `a/b` as the old code would have written it: sanitized to `a_b.json`.
    mock.put_object(
        "test-bucket",
        "tenants/a_b.json",
        serde_json::to_vec(&json!({
            "id": "a/b",
            "display_name": "Legacy",
            "created_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let backend = make_prefix_backend(mock);

    let found = backend
        .get_tenant("a/b")
        .await
        .unwrap()
        .expect("a pre-fix record must remain readable after upgrade");
    assert_eq!(found.display_name.as_deref(), Some("Legacy"));

    let listed = backend.list_tenants().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "a/b");

    assert!(
        backend.deregister_tenant("a/b").await.unwrap(),
        "deregister must remove a record stored under the legacy key"
    );
    assert!(backend.get_tenant("a/b").await.unwrap().is_none());
}

/// Every sibling namespace of the registry, exercised end to end: writing data
/// as a tenant with that name must never corrupt or fail the registry listing.
#[tokio::test]
async fn list_tenants_survives_tenants_named_after_control_plane_namespaces() {
    for hostile_id in [
        "tenants",
        "resources",
        "history",
        "bulk",
        "_system.user-settings",
    ] {
        let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
        let backend = make_prefix_backend(mock);

        let acme = backend.register_tenant("acme", None).await.unwrap();

        let hostile = tenant(hostile_id);
        backend
            .create(
                &hostile,
                "Patient",
                json!({"resourceType":"Patient","id":"p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend.delete(&hostile, "Patient", "p1").await.unwrap();

        let listed = backend
            .list_tenants()
            .await
            .unwrap_or_else(|e| panic!("tenant named {hostile_id:?} broke list_tenants: {e}"));
        assert_eq!(
            listed,
            vec![acme.clone()],
            "tenant named {hostile_id:?} corrupted the registry listing"
        );
    }
}

/// A malformed object sitting *directly* under the registry prefix — a foreign
/// object, a truncated write, or a future schema change — must be skipped, not
/// abort the listing. The direct-child filter cannot catch this one, so it is
/// what keeps a single bad object from making the admin tenant list permanently
/// unavailable.
#[tokio::test]
async fn list_tenants_skips_an_unreadable_record_instead_of_failing() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));

    // Well-formed JSON, but not a TenantRecord (no `created_at`).
    mock.put_object(
        "test-bucket",
        "tenants/not-a-record.json",
        serde_json::to_vec(&json!({"something": "else"})).unwrap(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    // Not JSON at all.
    mock.put_object(
        "test-bucket",
        "tenants/truncated.json",
        b"{\"id\": \"half-writ".to_vec(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let backend = make_prefix_backend(mock);
    let acme = backend.register_tenant("acme", None).await.unwrap();

    let listed = backend
        .list_tenants()
        .await
        .expect("one unreadable record must not fail the whole listing");
    assert_eq!(listed, vec![acme]);
}

/// #330: a deregistered tenant's leftover data must stay discoverable.
///  enumerates tenant prefixes with a delimiter LIST and
/// counts each tenant's current-pointer objects — LIST-only, no per-object
/// GETs, which also means delete tombstones count (they are purgeable data).
#[tokio::test]
async fn count_by_tenant_discovers_data_without_registration() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);

    // tenant-a: three current pointers — a live resource, an updated resource
    // (history versions must not inflate the count), and a delete tombstone.
    let a = tenant("tenant-a");
    backend
        .create(
            &a,
            "Patient",
            json!({"resourceType":"Patient","id":"p1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let p2 = backend
        .create(
            &a,
            "Patient",
            json!({"resourceType":"Patient","id":"p2"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .update(
            &a,
            &p2,
            json!({"resourceType":"Patient","id":"p2","active":true}),
        )
        .await
        .unwrap();
    backend
        .create(
            &a,
            "Observation",
            json!({"resourceType":"Observation","id":"o1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend.delete(&a, "Observation", "o1").await.unwrap();

    // tenant-b: data with no registration — the deregistered-tenant scenario.
    backend
        .create(
            &tenant("tenant-b"),
            "Patient",
            json!({"resourceType":"Patient","id":"q1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Registered but empty: a registry object under tenants/ with no
    // resources/ subtree — must not surface as a data-discovered tenant here
    // (the maintenance page merges registrations separately).
    backend.register_tenant("tenant-c", None).await.unwrap();

    let mut counts = backend.count_by_tenant().await.unwrap();
    counts.sort();
    assert_eq!(
        counts,
        vec![("tenant-a".to_string(), 3), ("tenant-b".to_string(), 1)]
    );
}
