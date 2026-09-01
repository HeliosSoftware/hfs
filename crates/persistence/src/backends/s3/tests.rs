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
use crate::core::{Backend, BackendCapability, ResourceStorage, VersionedStorage};
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

    /// Returns the number of objects currently stored in `bucket`.
    fn bucket_object_count(&self, bucket: &str) -> usize {
        let state = self.state.lock().unwrap();
        state.objects.keys().filter(|(b, _)| b == bucket).count()
    }

    /// Every object key currently stored in `bucket`, sorted.
    ///
    /// Used to assert the *literal* key layout, which
    /// `bucket_object_count` cannot express — the tenant-prefix fidelity tests
    /// (issue #447) need to prove that a safe tenant id still lands on exactly
    /// its pre-fix key, not merely that some object exists.
    fn keys(&self, bucket: &str) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut keys: Vec<String> = state
            .objects
            .keys()
            .filter(|(b, _)| b == bucket)
            .map(|(_, k)| k.clone())
            .collect();
        keys.sort();
        keys
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

/// Regression test for #787 on the S3 backend specifically: creating a
/// SearchParameter must make it immediately visible in that tenant's
/// registry — S3's `create()`/`update()`/`delete()` never invalidated any
/// overlay at all before this fix (S3 had no real per-tenant registry to
/// invalidate; the composite handed Elasticsearch a `base_only()` container
/// whose loader unconditionally returned an empty overlay for every tenant).
#[tokio::test]
async fn search_parameter_write_updates_the_tenant_registry_immediately() {
    use crate::core::search::SearchProvider;

    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    // Not yet registered: creating a Patient under a code the registry
    // doesn't know about must not resolve.
    assert!(
        backend
            .search_param_registry(&tenant)
            .read()
            .get_param("Patient", "nickname-s3")
            .is_none()
    );

    let sp = backend
        .create(
            &tenant,
            "SearchParameter",
            json!({
                "resourceType": "SearchParameter",
                "id": "patient-nickname-s3",
                "url": "http://acme.health/fhir/SearchParameter/patient-nickname-s3",
                "name": "nickname-s3",
                "status": "active",
                "code": "nickname-s3",
                "base": ["Patient"],
                "type": "string",
                "expression": "Patient.name.where(use = 'nickname').given"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Immediately visible — no separate refresh call, no TTL wait.
    let registered = backend
        .search_param_registry(&tenant)
        .read()
        .get_param("Patient", "nickname-s3")
        .expect("SearchParameter should be registered immediately after create");
    assert_eq!(
        registered.url,
        "http://acme.health/fhir/SearchParameter/patient-nickname-s3"
    );

    // Another tenant never sees it (per-tenant overlay, not a global cache).
    let other_tenant =
        TenantContext::new(TenantId::new("tenant-b"), TenantPermissions::full_access());
    assert!(
        backend
            .search_param_registry(&other_tenant)
            .read()
            .get_param("Patient", "nickname-s3")
            .is_none()
    );

    // Deleting it removes it from the registry immediately too.
    backend
        .delete(&tenant, "SearchParameter", sp.id())
        .await
        .unwrap();
    assert!(
        backend
            .search_param_registry(&tenant)
            .read()
            .get_param("Patient", "nickname-s3")
            .is_none(),
        "a deleted SearchParameter must drop out of the registry immediately"
    );
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

// The three `bundle_transaction_*` tests were removed here.
//
// They covered `S3Backend::process_transaction`'s happy path, its
// compensation-log rollback, and its rollback-failure reporting. That whole
// path is now unreachable: S3 answers `false` to
// `supports_atomic_transactions` and the method refuses before doing any work,
// because the rollback could never run on the failure mode that mattered — an
// HTTP timeout drops the request future, so cancellation skips every error
// branch (#489).
//
// Replaced by `transaction_bundle_is_refused_without_writing_anything` below,
// which asserts the refusal *and* that nothing was written. Batch is not
// re-asserted here: #501 removed `BundleProvider::process_batch` as
// unreachable, so batch has no S3-level entry point to test. It is covered
// over HTTP in `helios-rest` (`batch_conformance.rs`, `batch_if_match.rs`) and
// end-to-end by the `s3-elasticsearch` Inferno leg.

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

/// Two output files of one manifest, both starting at line 1, must each keep
/// their own entry result and raw archive (issue #457).
///
/// The S3 shape of that bug is quieter than the SQL backends': a `PutObject`
/// has no primary key to violate, so the second file's line 1 landed on the
/// first file's key and silently replaced it — no error, just an entry result
/// and an audit payload gone, and counts reporting one file's worth of lines
/// instead of the sum.
#[tokio::test]
async fn bulk_submit_entry_results_are_keyed_by_their_output_file() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock.clone());
    let tenant = tenant("tenant-a");

    let submission_id = SubmissionId::new("client-a", "sub-multifile");
    backend
        .create_submission(&tenant, &submission_id, None)
        .await
        .unwrap();
    let manifest = backend
        .add_manifest(&tenant, &submission_id, None, None)
        .await
        .unwrap();

    for (file, family) in [
        ("http://provider/Patient.ndjson", "FromPatients"),
        ("http://provider/Practitioner.ndjson", "FromPractitioners"),
    ] {
        let entries = vec![NdjsonEntry::new(
            1,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": family}]}),
        )];
        let results = backend
            .process_entries(
                &tenant,
                &submission_id,
                &manifest.manifest_id,
                entries,
                &BulkProcessingOptions::new().with_file_url(file),
            )
            .await
            .unwrap_or_else(|e| panic!("file {file} must ingest: {e}"));
        assert!(results.iter().all(|r| r.is_success()), "{file}");
    }

    let counts = backend
        .get_entry_counts(&tenant, &submission_id, &manifest.manifest_id)
        .await
        .unwrap();
    assert_eq!(counts.success, 2, "one stored entry result per file");

    let stored = backend
        .get_entry_results(&tenant, &submission_id, &manifest.manifest_id, None, 10, 0)
        .await
        .unwrap();
    assert_eq!(stored.len(), 2, "both files' line 1 must survive");

    // The raw NDJSON archive is discriminated too, so the auditable copy of the
    // first file's payload is not replaced by the second's.
    let raw_keys: Vec<String> = mock
        .recorded_puts()
        .into_iter()
        .map(|put| put.key)
        .filter(|key| key.contains("/raw/") && key.ends_with("line-1.ndjson"))
        .collect();
    assert_eq!(raw_keys.len(), 2, "one raw archive put per file");
    assert_ne!(
        raw_keys[0], raw_keys[1],
        "raw archives must not share a key"
    );
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
// Tenant prefix fidelity (issue #447)
// ============================================================================
//
// `S3Keyspace::with_tenant_prefix` used `trim_matches('/')`, so `acme`, `/acme`,
// `acme/` and `//acme` all resolved to the data prefix `acme` while the registry
// (injective since #271) held them as four separate tenants. A second defect in
// the same derivation let a tenant named `acme/resources` nest its entire
// keyspace inside the prefix tenant `acme` sweeps and lists.
//
// `keyspace.rs` proves the properties over an adversarial corpus of ids. These
// drive the consequences through the real storage API instead, because that is
// what an operator experiences and what the issue reported: a purge that deletes
// a different tenant than the one named, and a read that returns another
// tenant's resources.

/// Issue #447, consequence 1: `DELETE /admin/tenants/%2Facme?purge=true` used to
/// delete tenant `acme`'s resources *and its version history*, while leaving
/// `acme`'s registry record in place — a still-registered, still-listed tenant
/// whose data was gone, reported to the caller as success.
#[tokio::test]
async fn purging_a_slash_padded_tenant_leaves_the_bare_tenant_intact() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let bare = tenant("acme");

    let created = backend
        .create(
            &bare,
            "Patient",
            json!({"resourceType":"Patient","id":"p1"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .update(
            &bare,
            &created,
            json!({"resourceType":"Patient","id":"p1","active":true}),
        )
        .await
        .unwrap();

    // Every id that used to collapse onto `acme`.
    for padded in ["/acme", "acme/", "//acme", "/acme/"] {
        let removed = backend.purge_tenant_data(padded).await.unwrap();
        assert_eq!(removed, 0, "purging {padded:?} must not reach any resource");
    }

    assert!(
        backend
            .read(&bare, "Patient", "p1")
            .await
            .unwrap()
            .is_some(),
        "tenant `acme`'s current resource must survive"
    );
    assert!(
        backend
            .vread(&bare, "Patient", "p1", "1")
            .await
            .unwrap()
            .is_some(),
        "tenant `acme`'s version history must survive"
    );
    assert_eq!(backend.count(&bare, None).await.unwrap(), 1);
}

/// Issue #447: slash-padded ids are distinct tenants in the registry, so they
/// must also be distinct in the data keyspace — writes under one must be
/// invisible to the others.
#[tokio::test]
async fn slash_padded_tenants_do_not_share_resources() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);

    let ids = ["acme", "/acme", "acme/", "//acme"];
    for (i, id) in ids.iter().enumerate() {
        backend
            .create(
                &tenant(id),
                "Patient",
                json!({"resourceType":"Patient","id": format!("p{i}")}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    for (i, id) in ids.iter().enumerate() {
        let ctx = tenant(id);
        assert_eq!(
            backend.count(&ctx, None).await.unwrap(),
            1,
            "tenant {id:?} must see only its own resource"
        );
        assert!(
            backend
                .read(&ctx, "Patient", &format!("p{i}"))
                .await
                .unwrap()
                .is_some()
        );
        for (j, _) in ids.iter().enumerate() {
            if i == j {
                continue;
            }
            assert!(
                backend
                    .read(&ctx, "Patient", &format!("p{j}"))
                    .await
                    .unwrap()
                    .is_none(),
                "tenant {id:?} must not see p{j}"
            );
        }
    }
}

/// The second defect in the same derivation: a tenant whose id carries a
/// keyspace namespace as a later segment used to store its objects *inside* the
/// parent id's radius. `acme` sweeps `acme/resources/` on purge and enumerates
/// it on list, which is exactly where `acme/resources` kept everything — a
/// cross-tenant delete and a cross-tenant read.
///
/// Reachable through the admin API today: `RESERVED_TENANT_IDS` is compared
/// against the whole id, so `acme/resources` provisions cleanly.
#[tokio::test]
async fn a_tenant_named_after_a_sweep_root_survives_its_parents_purge() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let parent = tenant("acme");

    for nested_id in ["acme/resources", "acme/history", "acme/bulk"] {
        let nested = tenant(nested_id);
        backend
            .create(
                &nested,
                "Patient",
                json!({"resourceType":"Patient","id":"nested"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // The parent must not be able to see it...
        assert_eq!(
            backend.count(&parent, None).await.unwrap(),
            0,
            "tenant `acme` must not enumerate {nested_id:?}'s resources"
        );
        assert!(
            backend
                .read(&parent, "Patient", "nested")
                .await
                .unwrap()
                .is_none()
        );

        // ...nor destroy it by purging itself.
        backend.purge_tenant_data("acme").await.unwrap();
        assert!(
            backend
                .read(&nested, "Patient", "nested")
                .await
                .unwrap()
                .is_some(),
            "purging `acme` must not delete {nested_id:?}'s data"
        );

        backend.purge_tenant_data(nested_id).await.unwrap();
    }
}

/// Hierarchical and ordinary ids must keep their exact prefix, so upgrading
/// relocates nothing that was already stored safely. Asserted through the API:
/// a resource written before the fix is still readable after it.
///
/// `__system__` matters most — it holds the shared terminology resources, and
/// silently orphaning those would be far worse than the defect being fixed.
#[tokio::test]
async fn safe_tenant_ids_still_resolve_to_their_original_prefix() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock.clone());

    for id in ["default", "acme", "acme/research", "__system__"] {
        let ctx = tenant(id);
        backend
            .create(
                &ctx,
                "Patient",
                json!({"resourceType":"Patient","id":"p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert!(backend.read(&ctx, "Patient", "p1").await.unwrap().is_some());
    }

    // The object keys themselves are unchanged — this is what makes the upgrade
    // a no-op for these tenants rather than a silent relocation.
    let keys = mock.keys("test-bucket");
    for expected in [
        "default/resources/Patient/p1/current.json",
        "acme/resources/Patient/p1/current.json",
        "acme/research/resources/Patient/p1/current.json",
        "__system__/resources/Patient/p1/current.json",
    ] {
        assert!(
            keys.iter().any(|k| k == expected),
            "expected unchanged key {expected:?}; got {keys:?}"
        );
    }
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

// ── Tenancy capability declaration (issue #369) ──────────────────────────────
//
// S3 is the one backend whose tenant-placement topology is a property of the
// *instance* rather than of the backend type, so these assert the composition
// `capabilities(&self)` performs — the mode-parameterised list is covered
// separately, without a client, in `tests/backend_capability_contract.rs`.

/// `PrefixPerTenant` shares one bucket across tenants, so it declares
/// `SharedSchema` and must not claim the physical isolation it does not have.
#[test]
fn prefix_per_tenant_instance_declares_shared_schema_only() {
    let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));

    assert!(backend.supports(BackendCapability::SharedSchema));
    assert!(!backend.supports(BackendCapability::DatabasePerTenant));
    assert!(!backend.supports(BackendCapability::SchemaPerTenant));

    let declared = backend.capabilities();
    assert!(declared.contains(&BackendCapability::SharedSchema));
    assert!(!declared.contains(&BackendCapability::DatabasePerTenant));
}

/// `BucketPerTenant` gives each tenant a dedicated bucket, so it declares
/// `DatabasePerTenant` — and must not *also* claim `SharedSchema` merely
/// because a `default_system_bucket` exists for cross-tenant state.
#[test]
fn bucket_per_tenant_instance_declares_database_per_tenant_only() {
    let backend = make_bucket_backend(Arc::new(MockS3Client::with_buckets(&[
        "bucket-a",
        "bucket-b",
        "system-bucket",
    ])));

    assert!(backend.supports(BackendCapability::DatabasePerTenant));
    assert!(!backend.supports(BackendCapability::SharedSchema));
    assert!(!backend.supports(BackendCapability::SchemaPerTenant));

    let declared = backend.capabilities();
    assert!(declared.contains(&BackendCapability::DatabasePerTenant));
    assert!(!declared.contains(&BackendCapability::SharedSchema));
}

/// The behavioural warrant for the two declarations above: bucket-per-tenant
/// really does resolve distinct buckets, and prefix-per-tenant really does
/// share one bucket while separating tenants by key prefix.
#[test]
fn tenant_location_matches_the_declared_tenancy_topology() {
    let bucket_backend = make_bucket_backend(Arc::new(MockS3Client::with_buckets(&[
        "bucket-a",
        "bucket-b",
        "system-bucket",
    ])));
    let a = bucket_backend
        .tenant_location(&tenant("tenant-a"))
        .expect("tenant-a location");
    let b = bucket_backend
        .tenant_location(&tenant("tenant-b"))
        .expect("tenant-b location");
    assert_ne!(
        a.bucket, b.bucket,
        "BucketPerTenant must resolve a dedicated bucket per tenant"
    );

    let prefix_backend =
        make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
    let a = prefix_backend
        .tenant_location(&tenant("tenant-a"))
        .expect("tenant-a location");
    let b = prefix_backend
        .tenant_location(&tenant("tenant-b"))
        .expect("tenant-b location");
    assert_eq!(
        a.bucket, b.bucket,
        "PrefixPerTenant must share one bucket across tenants"
    );
    assert_ne!(
        a.keyspace.resources_prefix(),
        b.keyspace.resources_prefix(),
        "PrefixPerTenant must separate tenants by key prefix"
    );
}

/// A transaction bundle must be refused outright, and refused *before* any
/// object is written.
///
/// S3 has no atomic multi-object operation, and the compensation log cannot
/// stand in for one: the HTTP `TimeoutLayer` drops the request future, so
/// cancellation skips every error branch that would trigger a rollback, and the
/// compensation list is only populated after all writes resolve — empty at the
/// moment it would be needed. In production that left 466 of 473 entries
/// durably committed while the client was told the transaction failed (#489).
///
/// The `count` assertion is the point of the test: a refusal that still wrote
/// something would reproduce the original defect in miniature.
#[tokio::test]
async fn transaction_bundle_is_refused_without_writing_anything() {
    let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
    let backend = make_prefix_backend(mock);
    let tenant = tenant("tenant-a");

    assert!(
        !backend.supports_atomic_transactions(),
        "S3 must not claim atomicity it cannot provide"
    );

    let entries = vec![
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({"resourceType": "Patient", "active": true})),
            full_url: Some("urn:uuid:11111111-1111-1111-1111-111111111111".to_string()),
            ..Default::default()
        },
        BundleEntry {
            method: BundleMethod::Post,
            url: "Observation".to_string(),
            resource: Some(json!({"resourceType": "Observation", "status": "final"})),
            full_url: Some("urn:uuid:22222222-2222-2222-2222-222222222222".to_string()),
            ..Default::default()
        },
    ];

    let err = backend
        .process_transaction(&tenant, entries, FhirVersion::default())
        .await
        .expect_err("transaction must be refused");

    match err {
        TransactionError::AtomicityUnsupported { backend_name } => {
            assert_eq!(backend_name, "s3");
        }
        other => panic!("expected AtomicityUnsupported, got {other:?}"),
    }

    for resource_type in ["Patient", "Observation"] {
        assert_eq!(
            backend.count(&tenant, Some(resource_type)).await.unwrap(),
            0,
            "{resource_type}: refusal must happen before any write"
        );
    }
}

// ===========================================================================
// $bulk-submit job store
//
// S3 hosts the REST worker's job state itself — leases compare-and-swapped
// against the manifest object's ETag, plus a cross-tenant index for the three
// lookups that have no tenant in hand. These exercise that layer against the
// mock client, whose conditional `put_object` enforces real precondition
// semantics.
// ===========================================================================

mod bulk_submit_worker {
    use super::*;

    use std::time::Duration;

    use crate::core::bulk_export_worker::WorkerId;
    use crate::core::bulk_submit::{ManifestStatus, SubmissionManifest};
    use crate::core::bulk_submit_worker::{
        ManifestFetchParams, SubmitClaimStrategy, SubmitFileRecord, SubmitWorkerStorage,
    };

    fn lease_duration() -> Duration {
        Duration::from_secs(60)
    }

    /// Creates a submission with one fetchable manifest — the shape the REST
    /// kickoff handler produces.
    async fn seed(backend: &S3Backend, tenant_ctx: &TenantContext) -> (SubmissionId, String) {
        let id = SubmissionId::new("client-a", uuid::Uuid::new_v4().to_string());
        backend
            .create_submission(tenant_ctx, &id, None)
            .await
            .expect("create submission");
        let manifest = backend
            .add_manifest(
                tenant_ctx,
                &id,
                Some("https://provider.example/manifest.json"),
                None,
            )
            .await
            .expect("add manifest");
        (id, manifest.manifest_id)
    }

    #[tokio::test]
    async fn claim_leases_a_queued_manifest_and_excludes_other_workers() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let (id, manifest_id) = seed(&backend, &t).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
            .await
            .expect("claim")
            .expect("the seeded manifest is claimable");
        assert_eq!(lease.manifest_id, manifest_id);
        assert_eq!(lease.fencing_token, 1, "the first claim bumps 0 -> 1");
        assert_eq!(lease.tenant.tenant_id().as_str(), "tenant-a");

        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-2"), lease_duration())
                .await
                .expect("claim")
                .is_none(),
            "a live lease must not be claimable by a second worker"
        );

        backend.heartbeat(&lease).await.expect("heartbeat");
        backend.finish_manifest(&lease).await.expect("finish");

        let manifests = backend.list_manifests(&t, &id).await.expect("manifests");
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .expect("claim")
                .is_none(),
            "a completed manifest must leave the claim queue"
        );
    }

    /// An expired lease is reclaimable, and the reclaim must invalidate the
    /// previous holder — this is the whole point of the fencing token.
    #[tokio::test]
    async fn an_expired_lease_is_reclaimed_and_fences_out_the_previous_holder() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let (_id, _manifest_id) = seed(&backend, &t).await;

        // A zero-length lease is expired the moment it is taken.
        let stale = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), Duration::from_secs(0))
            .await
            .expect("claim")
            .expect("claimable");
        let fresh = backend
            .claim_next_manifest(&WorkerId::new("worker-2"), lease_duration())
            .await
            .expect("claim")
            .expect("an expired lease is reclaimable");
        assert!(fresh.fencing_token > stale.fencing_token);

        for outcome in [
            backend
                .heartbeat(&stale)
                .await
                .err()
                .map(|e| format!("{e:?}")),
            backend
                .update_manifest_progress(&stale, 5, 0, 5)
                .await
                .err()
                .map(|e| format!("{e:?}")),
            backend
                .finish_manifest(&stale)
                .await
                .err()
                .map(|e| format!("{e:?}")),
            backend
                .get_manifest_for_worker(&stale)
                .await
                .err()
                .map(|e| format!("{e:?}")),
        ] {
            let message = outcome.expect("a fenced-out worker's write must fail");
            assert!(
                message.contains("LeaseLost"),
                "expected LeaseLost, got {message}"
            );
        }

        // The live holder is unaffected.
        backend
            .heartbeat(&fresh)
            .await
            .expect("the live lease survives");
    }

    #[tokio::test]
    async fn fetch_params_round_trip_to_the_worker_view() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let (id, manifest_id) = seed(&backend, &t).await;

        let headers = vec![("Authorization".to_string(), "Bearer x".to_string())];
        let oauth = vec!["https://provider.example/.well-known/smart".to_string()];
        let key = json!({"alg": "dir", "value": "abc"});
        let import = vec![(
            crate::core::bulk_submit::IMPORT_MODE_PARAMETER_URL.to_string(),
            "merge".to_string(),
        )];
        let metadata = vec![("https://example.org/note".to_string(), "hello".to_string())];

        backend
            .set_manifest_fetch_params(
                &t,
                &id,
                &manifest_id,
                ManifestFetchParams {
                    fhir_base_url: Some("https://provider.example/fhir"),
                    output_format: Some("application/fhir+ndjson; fhirVersion=4.0"),
                    file_request_headers: &headers,
                    oauth_metadata_urls: &oauth,
                    file_encryption_key: Some(&key),
                    import_directives: &import,
                    metadata: &metadata,
                },
            )
            .await
            .expect("set fetch params");

        let lease = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
            .await
            .expect("claim")
            .expect("claimable");
        let view = backend
            .get_manifest_for_worker(&lease)
            .await
            .expect("worker view");

        assert_eq!(
            view.manifest_url.as_deref(),
            Some("https://provider.example/manifest.json")
        );
        assert_eq!(
            view.fhir_base_url.as_deref(),
            Some("https://provider.example/fhir")
        );
        assert_eq!(view.file_request_headers, headers);
        assert_eq!(view.oauth_metadata_urls, oauth);
        assert_eq!(view.file_encryption_key, Some(key));
        assert_eq!(view.import_directives, import);
        assert_eq!(view.metadata, metadata);
        assert_eq!(view.fhir_version, FhirVersion::R4);
    }

    #[tokio::test]
    async fn poll_token_resolves_only_while_the_submission_holds_it() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let (id, _manifest_id) = seed(&backend, &t).await;

        backend
            .set_submission_kickoff_meta(
                &t,
                &id,
                Some("client-subject"),
                "https://hfs.example/$bulk-submit",
                true,
            )
            .await
            .expect("kickoff meta");

        let token = backend
            .ensure_poll_token(&t, &id)
            .await
            .expect("mint token");
        assert_eq!(
            backend.ensure_poll_token(&t, &id).await.expect("re-mint"),
            token,
            "ensure_poll_token must be idempotent"
        );

        let target = backend
            .resolve_poll_token(&token)
            .await
            .expect("resolve")
            .expect("the token resolves");
        assert_eq!(target.submission_id, id);
        assert_eq!(target.tenant.tenant_id().as_str(), "tenant-a");
        assert_eq!(
            target.owner_subject.as_deref(),
            Some("client-subject"),
            "the owner must come from the submission, which the ownership check trusts"
        );

        assert!(
            backend
                .resolve_poll_token("not-a-token")
                .await
                .expect("resolve")
                .is_none()
        );

        backend.clear_poll_token(&t, &id).await.expect("clear");
        assert!(
            backend
                .resolve_poll_token(&token)
                .await
                .expect("resolve")
                .is_none(),
            "a cleared token must stop resolving, so a deleted submission 404s"
        );
    }

    /// A transaction time is minted once and then never moves — the status
    /// manifest's `transactionTime` must be stable across polls.
    #[tokio::test]
    async fn transaction_time_is_minted_once() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let (id, _manifest_id) = seed(&backend, &t).await;

        let first = backend
            .ensure_transaction_time(&t, &id)
            .await
            .expect("transaction time");
        assert_eq!(
            backend
                .ensure_transaction_time(&t, &id)
                .await
                .expect("transaction time"),
            first
        );
    }

    #[tokio::test]
    async fn submit_files_are_recorded_idempotently_and_deletable() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let (id, _manifest_id) = seed(&backend, &t).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
            .await
            .expect("claim")
            .expect("claimable");

        let output = SubmitFileRecord {
            manifest_url: Some("https://provider.example/manifest.json".to_string()),
            file_type: "output".to_string(),
            resource_type: Some("Patient".to_string()),
            part_index: 0,
            file_path: "tenant-a/job/output/Patient-0.ndjson".to_string(),
            line_count: 3,
            byte_count: 120,
            count_severity: None,
        };
        let error = SubmitFileRecord {
            manifest_url: Some("https://provider.example/manifest.json".to_string()),
            file_type: "error".to_string(),
            resource_type: Some("OperationOutcome".to_string()),
            part_index: 0,
            file_path: "tenant-a/job/error/OperationOutcome-0.ndjson".to_string(),
            line_count: 1,
            byte_count: 40,
            count_severity: Some(json!({"error": 1})),
        };
        backend
            .record_submit_file(&lease, &output)
            .await
            .expect("output");
        backend
            .record_submit_file(&lease, &error)
            .await
            .expect("error");
        // A retry of the same artifact must replace its row, not add one.
        backend
            .record_submit_file(&lease, &output)
            .await
            .expect("retry");

        let rows = backend
            .list_submit_files(&t, &id)
            .await
            .expect("list files");
        assert_eq!(rows.len(), 2, "record_submit_file must be idempotent");
        let error_row = rows
            .iter()
            .find(|r| r.file_type == "error")
            .expect("error row");
        assert_eq!(error_row.count_severity, Some(json!({"error": 1})));
        assert!(rows.iter().all(|r| r.fencing_token == lease.fencing_token));

        backend
            .delete_submission_artifacts(&t, &id)
            .await
            .expect("delete artifacts");
        assert!(
            backend
                .list_submit_files(&t, &id)
                .await
                .expect("list files")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn active_counts_and_expiry_scan_track_the_submission() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let other = tenant("tenant-b");
        let (id, _manifest_id) = seed(&backend, &t).await;

        assert_eq!(
            backend
                .count_active_submissions(&t)
                .await
                .expect("count active"),
            1
        );
        assert_eq!(
            backend
                .count_active_submissions(&other)
                .await
                .expect("count active"),
            0,
            "the cap is per tenant, and the index spans tenants"
        );

        backend
            .complete_submission(&t, &id)
            .await
            .expect("complete submission");
        assert_eq!(
            backend
                .count_active_submissions(&t)
                .await
                .expect("count active"),
            0,
            "a completed submission must free its slot"
        );

        let expired = backend
            .list_expired_submissions(Utc::now(), Duration::from_secs(0), 10)
            .await
            .expect("expiry scan");
        assert!(expired.iter().any(|(_, sub)| sub == &id));
        let fresh = backend
            .list_expired_submissions(Utc::now(), Duration::from_secs(86_400), 10)
            .await
            .expect("expiry scan");
        assert!(!fresh.iter().any(|(_, sub)| sub == &id));
    }

    #[tokio::test]
    async fn aborted_and_replaced_manifests_leave_the_queue() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");

        let (aborted, _) = seed(&backend, &t).await;
        backend
            .abort_submission(&t, &aborted, "provider cancelled")
            .await
            .expect("abort");
        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .expect("claim")
                .is_none(),
            "an aborted submission's manifests must not be picked up"
        );

        let (replaced, manifest_id) = seed(&backend, &t).await;
        let superseded = backend
            .replace_manifest_by_url(&t, &replaced, "https://provider.example/manifest.json")
            .await
            .expect("replace");
        assert_eq!(superseded, vec![manifest_id]);
        assert_eq!(
            backend
                .list_manifests(&t, &replaced)
                .await
                .expect("manifests")[0]
                .status,
            ManifestStatus::Replaced
        );
        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .expect("claim")
                .is_none(),
            "a replaced manifest is no longer work"
        );
    }

    /// A status-only kickoff registers no fetchable manifest, so it never
    /// becomes work — the same predicate the SQL job stores encode as
    /// `WHERE manifest_url IS NOT NULL`.
    #[tokio::test]
    async fn a_manifest_without_a_url_is_never_claimed() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let id = SubmissionId::new("client-a", "status-only");
        backend
            .create_submission(&t, &id, None)
            .await
            .expect("create submission");
        backend
            .add_manifest(&t, &id, None, None)
            .await
            .expect("add manifest");

        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .expect("claim")
                .is_none()
        );
    }

    /// The claim queue spans tenants, since a worker has no tenant in hand.
    #[tokio::test]
    async fn the_claim_queue_reaches_every_tenant() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let a = tenant("tenant-a");
        let b = tenant("tenant-b");
        seed(&backend, &a).await;
        seed(&backend, &b).await;

        let mut claimed = Vec::new();
        for worker in ["worker-1", "worker-2"] {
            let lease = backend
                .claim_next_manifest(&WorkerId::new(worker), lease_duration())
                .await
                .expect("claim")
                .expect("both tenants' manifests are claimable");
            claimed.push(lease.tenant.tenant_id().as_str().to_string());
        }
        claimed.sort();
        assert_eq!(claimed, vec!["tenant-a", "tenant-b"]);
    }

    /// Bucket-per-tenant with no system bucket has nowhere to keep the
    /// cross-tenant index, so it must not advertise the worker and must not
    /// claim anything — rather than writing the index into some tenant's bucket.
    #[tokio::test]
    async fn a_configuration_without_cross_tenant_storage_hosts_no_worker() {
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
        let backend =
            S3Backend::with_client(config, Arc::new(MockS3Client::with_buckets(&["bucket-a"])))
                .expect("backend");

        assert!(!backend.supports_bulk_submit_worker());

        // Ingestion still works — it is entirely tenant-scoped.
        let t = tenant("tenant-a");
        let (id, manifest_id) = seed(&backend, &t).await;
        let results = backend
            .process_entries(
                &t,
                &id,
                &manifest_id,
                vec![NdjsonEntry::new(
                    1,
                    "Patient",
                    json!({"resourceType": "Patient", "id": "p1"}),
                )],
                &BulkProcessingOptions::new(),
            )
            .await
            .expect("ingest");
        assert!(results[0].is_success());

        // The worker surfaces simply find nothing.
        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .expect("claim")
                .is_none()
        );
        assert!(
            backend
                .resolve_poll_token("anything")
                .await
                .expect("resolve")
                .is_none()
        );
        assert!(
            backend
                .list_expired_submissions(Utc::now(), Duration::from_secs(0), 10)
                .await
                .expect("expiry scan")
                .is_empty()
        );
    }

    /// A leased manifest's terminal status belongs to the worker: ingesting a
    /// file must leave it `processing`, or a worker that dies on the next file
    /// would never be reclaimed.
    #[tokio::test]
    async fn ingesting_a_file_does_not_settle_a_leased_manifest() {
        let backend = make_prefix_backend(Arc::new(MockS3Client::with_buckets(&["test-bucket"])));
        let t = tenant("tenant-a");
        let (id, manifest_id) = seed(&backend, &t).await;

        let lease = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
            .await
            .expect("claim")
            .expect("claimable");

        // A malformed payload: `process_entries` records the failure but must
        // not mark the manifest failed while the worker still holds it.
        backend
            .process_entries(
                &t,
                &id,
                &manifest_id,
                vec![NdjsonEntry::new(
                    1,
                    "Patient",
                    json!({"resourceType": "Observation", "id": "wrong"}),
                )],
                &BulkProcessingOptions::new().with_file_url("a.ndjson"),
            )
            .await
            .expect("ingest");

        assert_eq!(
            backend.list_manifests(&t, &id).await.expect("manifests")[0].status,
            ManifestStatus::Processing
        );
        // The lease survived the ingest, so the worker can still settle it.
        backend.finish_manifest(&lease).await.expect("finish");
        assert_eq!(
            backend.list_manifests(&t, &id).await.expect("manifests")[0].status,
            ManifestStatus::Completed
        );
    }

    /// A queue entry whose manifest object is gone must be swept, not retried
    /// forever.
    #[tokio::test]
    async fn a_stale_queue_entry_is_swept() {
        let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
        let backend = make_prefix_backend(mock.clone());
        let t = tenant("tenant-a");
        let (id, manifest_id) = seed(&backend, &t).await;

        let location = backend.tenant_location(&t).expect("location");
        let key =
            location
                .keyspace
                .submit_manifest_key(&id.submitter, &id.submission_id, &manifest_id);
        mock.delete_object(&location.bucket, &key)
            .await
            .expect("delete manifest object");

        assert!(
            backend
                .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
                .await
                .expect("claim")
                .is_none()
        );
        // Seeding a fresh manifest still works, proving the queue was not left
        // wedged on the missing entry.
        let (_id2, manifest_id2) = seed(&backend, &t).await;
        let lease = backend
            .claim_next_manifest(&WorkerId::new("worker-1"), lease_duration())
            .await
            .expect("claim")
            .expect("the new manifest is claimable");
        assert_eq!(lease.manifest_id, manifest_id2);
    }

    /// The index keys are digests of client-supplied identifiers, so two
    /// submissions whose ids differ only where a lossy sanitiser would collapse
    /// them must still get distinct queue entries.
    #[test]
    fn index_object_ids_are_injective_across_part_boundaries() {
        use crate::backends::s3::keyspace::submit_index_object_id;

        assert_ne!(
            submit_index_object_id(&["a", "b/c"]),
            submit_index_object_id(&["a/b", "c"]),
        );
        assert_ne!(
            submit_index_object_id(&["ab", "c"]),
            submit_index_object_id(&["a", "bc"]),
        );
        assert_eq!(
            submit_index_object_id(&["a", "b"]),
            submit_index_object_id(&["a", "b"]),
        );
    }

    /// A manifest object written before the job store existed carries none of
    /// the lease fields; it must still deserialize (and be claimable).
    #[tokio::test]
    async fn a_pre_worker_manifest_object_still_deserializes() {
        let mock = Arc::new(MockS3Client::with_buckets(&["test-bucket"]));
        let backend = make_prefix_backend(mock.clone());
        let t = tenant("tenant-a");
        let id = SubmissionId::new("client-a", "legacy");
        backend
            .create_submission(&t, &id, None)
            .await
            .expect("create submission");

        // The pre-#521 on-disk shape: `{"manifest": {...}}` and nothing else.
        let manifest = SubmissionManifest::new("legacy-manifest")
            .with_url("https://provider.example/manifest.json");
        let location = backend.tenant_location(&t).expect("location");
        let key = location.keyspace.submit_manifest_key(
            &id.submitter,
            &id.submission_id,
            &manifest.manifest_id,
        );
        let body = serde_json::to_vec(&json!({ "manifest": manifest })).expect("encode");
        mock.put_object(
            &location.bucket,
            &key,
            body,
            Some("application/json"),
            None,
            None,
        )
        .await
        .expect("write legacy manifest");

        let manifests = backend.list_manifests(&t, &id).await.expect("manifests");
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].manifest_id, "legacy-manifest");
    }

    /// End-to-end through the shared worker: claim → fetch → ingest → artifacts.
    ///
    /// This is also the compile-level proof that `S3Backend` satisfies
    /// [`BulkSubmitJobStore`], the bound `build_bulk_submit` takes — the whole
    /// point of the issue this module closes.
    #[tokio::test]
    async fn the_shared_worker_drives_an_s3_manifest_to_completion() {
        use crate::backends::local_fs::LocalFsOutputStore;
        use crate::core::bulk_submit_input::{RemoteFile, RemoteManifest, SubmitInputFetcher};
        use crate::core::bulk_submit_worker::{BulkSubmitJobStore, DefaultSubmitWorker};
        use crate::error::StorageResult;

        struct MockFetcher {
            manifest: RemoteManifest,
            files: HashMap<String, Vec<u8>>,
        }

        #[async_trait]
        impl SubmitInputFetcher for MockFetcher {
            async fn fetch_manifest(
                &self,
                _url: &str,
                _headers: &[(String, String)],
                _oauth: &[String],
                _key: Option<&serde_json::Value>,
            ) -> StorageResult<RemoteManifest> {
                Ok(self.manifest.clone())
            }

            async fn open_file_stream(
                &self,
                url: &str,
                _headers: &[(String, String)],
                _requires_access_token: bool,
                _oauth: &[String],
                _key: Option<&serde_json::Value>,
            ) -> StorageResult<Box<dyn tokio::io::AsyncBufRead + Send + Unpin>> {
                let data = self.files.get(url).cloned().unwrap_or_default();
                Ok(Box::new(BufReader::new(Cursor::new(data))))
            }
        }

        let backend = Arc::new(make_prefix_backend(Arc::new(MockS3Client::with_buckets(
            &["test-bucket"],
        ))));
        let t = tenant("tenant-a");
        let (id, _manifest_id) = seed(&backend, &t).await;

        let ndjson = concat!(
            "{\"resourceType\":\"Patient\",\"id\":\"w1\",\"gender\":\"female\"}\n",
            "not-json\n"
        );
        let mut files = HashMap::new();
        files.insert(
            "https://provider.example/patient.ndjson".to_string(),
            ndjson.as_bytes().to_vec(),
        );
        let fetcher = Arc::new(MockFetcher {
            manifest: RemoteManifest {
                requires_access_token: false,
                output: vec![RemoteFile {
                    resource_type: Some("Patient".to_string()),
                    url: "https://provider.example/patient.ndjson".to_string(),
                    count: Some(2),
                }],
                deleted: vec![],
            },
            files,
        });

        let tmp = tempfile::tempdir().expect("tempdir");
        let output = Arc::new(LocalFsOutputStore::new(
            tmp.path().to_path_buf(),
            "http://localhost:8080",
        ));
        let worker_id = WorkerId::new("e2e-worker");
        let jobs: Arc<dyn BulkSubmitJobStore> = backend.clone();
        let worker = DefaultSubmitWorker::new(jobs.clone(), fetcher, output, worker_id.clone());

        let lease = jobs
            .claim_next_manifest(&worker_id, lease_duration())
            .await
            .expect("claim")
            .expect("claimable");
        worker.run_job(lease).await.expect("run job");

        // Partial success: the good line ingested, the malformed one counted.
        assert!(
            backend
                .read(&t, "Patient", "w1")
                .await
                .expect("read")
                .is_some(),
            "the valid NDJSON line must be ingested"
        );
        let manifests = backend.list_manifests(&t, &id).await.expect("manifests");
        assert_eq!(manifests[0].status, ManifestStatus::Completed);
        assert!(manifests[0].failed_entries >= 1);

        let artifacts = backend.list_submit_files(&t, &id).await.expect("files");
        assert!(
            artifacts
                .iter()
                .any(|f| f.file_type == "output" && f.resource_type.as_deref() == Some("Patient")),
            "an output receipt must be recorded, got {artifacts:?}"
        );
        assert!(
            artifacts.iter().any(|f| f.file_type == "error"),
            "the malformed line must surface in the error artifact, got {artifacts:?}"
        );
    }
}
