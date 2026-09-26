// Shared real-SQL consumer regressions. The including module aliases the library
// as `persistence`; no network provider or background worker service is needed.
use async_trait::async_trait;
use helios_fhir::FhirVersion;
use persistence::backends::local_fs::LocalFsOutputStore;
use persistence::composite::bulk_submit::CompositeSubmitJobs;
use persistence::composite::config::{CompositeConfig, SyncMode};
use persistence::composite::storage::{CompositeStorage, DynStorage};
use persistence::core::bulk_export_output::{ExportOutputStore, ExportPartKey};
use persistence::core::bulk_submit::{
    BulkEntryOutcome, BulkProcessingOptions, ManifestStatus, NdjsonEntry, SubmissionId,
};
use persistence::core::bulk_submit_input::{
    RemoteFile, RemoteManifest, SubmitInputFetcher, submission_output_job_id,
};
use persistence::core::bulk_submit_worker::{
    BulkSubmitJobStore, DefaultSubmitWorker, ManifestLease, SubmitWorkerStorage,
};
use persistence::core::{BackendKind, ResourceStorage, WorkerId};
use persistence::error::StorageResult;
use persistence::tenant::TenantContext;
use persistence::types::StoredResource;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;

struct MemoryFetcher {
    files: HashMap<String, Vec<u8>>,
    manifest: RemoteManifest,
}

#[async_trait]
impl SubmitInputFetcher for MemoryFetcher {
    async fn fetch_manifest(
        &self,
        _: &str,
        _: &[(String, String)],
        _: &[String],
        _: Option<&Value>,
    ) -> StorageResult<RemoteManifest> {
        Ok(self.manifest.clone())
    }

    async fn open_file_stream(
        &self,
        url: &str,
        _: &[(String, String)],
        _: bool,
        _: &[String],
        _: Option<&Value>,
    ) -> StorageResult<(Box<dyn tokio::io::AsyncBufRead + Send + Unpin>, Option<u64>)> {
        let bytes = self
            .files
            .get(url)
            .expect("manifest references a fixture file")
            .clone();
        let length = bytes.len() as u64;
        Ok((
            Box::new(tokio::io::BufReader::new(std::io::Cursor::new(bytes))),
            Some(length),
        ))
    }

    async fn file_size(
        &self,
        url: &str,
        _: &[(String, String)],
        _: bool,
        _: &[String],
    ) -> StorageResult<Option<u64>> {
        Ok(Some(self.files[url].len() as u64))
    }
}

async fn claim<B: BulkSubmitJobStore>(
    backend: &B,
    submission: &SubmissionId,
    manifest: &str,
) -> ManifestLease {
    let worker = WorkerId::new(format!("receipt-contract-{}", uuid::Uuid::new_v4()));
    let mut held = Vec::new();
    let mut found = None;
    for _ in 0..256 {
        match backend
            .claim_next_manifest(&worker, Duration::from_secs(300))
            .await
            .unwrap()
        {
            Some(lease) if lease.submission_id == *submission && lease.manifest_id == manifest => {
                found = Some(lease);
                break;
            }
            Some(lease) => held.push(lease),
            None => panic!("expected a claimable receipt fixture"),
        }
    }
    for lease in held {
        persistence::core::SubmitClaimStrategy::release(backend, lease)
            .await
            .unwrap();
    }
    found.expect("fixture manifest was not reached")
}

pub async fn worker_receipts<B: BulkSubmitJobStore + 'static>(
    backend: Arc<B>,
    tenant: &TenantContext,
) {
    let submission = SubmissionId::generate("worker-receipt-contract");
    backend
        .create_submission(tenant, &submission, None)
        .await
        .unwrap();
    let manifest = backend
        .add_manifest(
            tenant,
            &submission,
            Some("http://provider/manifest.json"),
            None,
        )
        .await
        .unwrap();
    backend
        .create(
            tenant,
            "Patient",
            json!({"resourceType":"Patient", "id":"gone"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend.delete(tenant, "Patient", "gone").await.unwrap();
    let mut files = HashMap::new();
    let mut remote_files = Vec::new();
    let mut expected = BTreeMap::<String, usize>::new();
    for file in 0..3 {
        let url = format!("http://provider/part-{file}.ndjson");
        let mut bytes = Vec::new();
        for line in 0..1100 {
            let id = format!("p-{}", line % 1001);
            *expected.entry(format!("Patient/{id}")).or_default() += 1;
            bytes.extend_from_slice(
                json!({"resourceType":"Patient", "id":id})
                    .to_string()
                    .as_bytes(),
            );
            bytes.push(b'\n');
        }
        bytes.extend_from_slice(b"{\"resourceType\":\"Patient\",\"id\":\"gone\"}\nnot-json\n");
        files.insert(url.clone(), bytes);
        remote_files.push(RemoteFile {
            resource_type: Some("Patient".to_string()),
            url,
            count: Some(1102),
        });
    }
    let expected_bytes = files.values().map(|bytes| bytes.len() as u64).sum::<u64>();
    let fetcher = Arc::new(MemoryFetcher {
        files,
        manifest: RemoteManifest {
            requires_access_token: false,
            output: remote_files,
            deleted: Vec::new(),
        },
    });
    let temp = tempfile::tempdir().unwrap();
    let output = Arc::new(LocalFsOutputStore::new(
        temp.path().to_path_buf(),
        "http://localhost",
    ));
    let lease = claim(backend.as_ref(), &submission, &manifest.manifest_id).await;
    let worker = DefaultSubmitWorker::new(
        backend.clone(),
        fetcher,
        output.clone(),
        lease.worker_id.clone(),
    );
    worker.run_job(lease).await.unwrap();

    let current = backend
        .list_manifests(tenant, &submission)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(current.status, ManifestStatus::Completed);
    assert_eq!(current.processed_entries, 3300);
    assert_eq!(current.failed_entries, 6);
    assert_eq!(
        current.total_entries, 3303,
        "parse failures have no persisted entry result"
    );
    assert_eq!(current.bytes_processed, expected_bytes);
    assert_eq!(current.bytes_total, expected_bytes);
    let counts = backend
        .get_entry_counts(tenant, &submission, &manifest.manifest_id)
        .await
        .unwrap();
    assert_eq!(counts.success, 3300);
    assert_eq!(counts.total, 3303);
    assert_eq!(counts.processing_error, 3);
    assert_eq!(counts.skipped, 0);
    assert_eq!(backend.count(tenant, Some("Patient")).await.unwrap(), 1001);

    // Persisted errors must survive unchanged; parse failures get the existing
    // single summary OperationOutcome, whose severity is counted once.
    let stored_errors = backend
        .get_entry_results_page(
            tenant,
            &submission,
            &manifest.manifest_id,
            Some(BulkEntryOutcome::ProcessingError),
            10,
            None,
        )
        .await
        .unwrap();
    let mut expected_errors: Vec<_> = stored_errors
        .entries
        .into_iter()
        .map(|entry| entry.result.operation_outcome.unwrap())
        .collect();
    expected_errors.push(json!({"resourceType":"OperationOutcome", "issue":[{
        "severity":"error", "code":"processing", "diagnostics":"3 submitted resource(s) could not be parsed or did not match the declared resource type"
    }]}));
    let mut references = BTreeMap::<String, usize>::new();
    let mut errors = Vec::new();
    let rows = backend
        .list_submit_files(tenant, &submission)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let key = ExportPartKey {
            tenant_id: tenant.tenant_id().as_str().to_string(),
            job_id: submission_output_job_id(&submission),
            resource_type: row.file_path.clone(),
            file_type: row.file_type.clone(),
            part_index: row.part_index,
            fencing_token: row.fencing_token,
        };
        let mut reader = output.open_reader(&key).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(row.byte_count, bytes.len() as u64);
        let lines: Vec<Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(row.line_count, lines.len() as u64);
        match row.file_type.as_str() {
            "output" => {
                for line in lines {
                    *references
                        .entry(line["reference"].as_str().unwrap().to_string())
                        .or_default() += 1;
                }
            }
            "error" => {
                assert_eq!(row.count_severity, Some(json!({"error":4})));
                errors.extend(lines);
            }
            other => panic!("unexpected artifact kind {other}"),
        }
    }
    assert_eq!(references, expected);
    errors.sort_by_key(Value::to_string);
    expected_errors.sort_by_key(Value::to_string);
    assert_eq!(errors, expected_errors);

    // A manifest whose only entries fail to parse has no successful results at
    // all: it must publish the aggregated `error` part alone, still counted and
    // byte-exact, without an empty `output` part standing in for the types it
    // never stored.
    let error_only = SubmissionId::generate("worker-receipt-contract-error-only");
    backend
        .create_submission(tenant, &error_only, None)
        .await
        .unwrap();
    let error_manifest = backend
        .add_manifest(
            tenant,
            &error_only,
            Some("http://provider/error-only.json"),
            None,
        )
        .await
        .unwrap();
    let error_url = "http://provider/error-only.ndjson".to_string();
    let mut error_files = HashMap::new();
    error_files.insert(error_url.clone(), b"not-json\n".to_vec());
    let error_fetcher = Arc::new(MemoryFetcher {
        files: error_files,
        manifest: RemoteManifest {
            requires_access_token: false,
            output: vec![RemoteFile {
                resource_type: Some("Patient".to_string()),
                url: error_url,
                count: Some(1),
            }],
            deleted: Vec::new(),
        },
    });
    let error_lease = claim(backend.as_ref(), &error_only, &error_manifest.manifest_id).await;
    let error_worker = DefaultSubmitWorker::new(
        backend.clone(),
        error_fetcher,
        output.clone(),
        error_lease.worker_id.clone(),
    );
    error_worker.run_job(error_lease).await.unwrap();

    let current = backend
        .list_manifests(tenant, &error_only)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(current.status, ManifestStatus::Completed);
    let rows = backend
        .list_submit_files(tenant, &error_only)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "an error-only run publishes one artifact");
    let row = &rows[0];
    assert_eq!(
        (
            row.file_type.as_str(),
            row.resource_type.as_deref(),
            row.part_index,
            row.line_count
        ),
        ("error", Some("OperationOutcome"), 0, 1)
    );
    assert_eq!(row.count_severity, Some(json!({"error": 1})));
    let key = ExportPartKey {
        tenant_id: tenant.tenant_id().as_str().to_string(),
        job_id: submission_output_job_id(&error_only),
        resource_type: row.file_path.clone(),
        file_type: row.file_type.clone(),
        part_index: row.part_index,
        fencing_token: row.fencing_token,
    };
    let mut reader = output.open_reader(&key).await.unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(row.byte_count, bytes.len() as u64);
    let rows: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        rows,
        vec![json!({"resourceType": "OperationOutcome", "issue": [{
            "severity": "error",
            "code": "processing",
            "diagnostics": "1 submitted resource(s) could not be parsed or did not match the declared resource type"
        }]})]
    );
}

struct RecordingSecondary {
    events: Arc<Mutex<Vec<String>>>,
    fail_on: Option<String>,
    failures: Arc<Mutex<usize>>,
}

#[async_trait]
impl ResourceStorage for RecordingSecondary {
    fn backend_name(&self) -> &'static str {
        "receipt-secondary"
    }
    async fn create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        version: FhirVersion,
    ) -> StorageResult<StoredResource> {
        let id = resource["id"].as_str().unwrap().to_string();
        if self.fail_on.as_deref() == Some(&id) {
            *self.failures.lock().unwrap() += 1;
            return Err(persistence::error::ValidationError::InvalidResource {
                message: "intentional secondary failure after earlier receipt pages".to_string(),
                details: Vec::new(),
            }
            .into());
        }
        self.events
            .lock()
            .unwrap()
            .push(format!("{resource_type}/{id}"));
        Ok(StoredResource::new(
            resource_type,
            id,
            tenant.tenant_id().clone(),
            resource,
            version,
        ))
    }
    async fn create_or_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
        mut resource: Value,
        version: FhirVersion,
    ) -> StorageResult<(StoredResource, bool)> {
        resource["id"] = Value::String(id.to_string());
        Ok((
            self.create(tenant, resource_type, resource, version)
                .await?,
            true,
        ))
    }
    async fn read(
        &self,
        _: &TenantContext,
        _: &str,
        _: &str,
    ) -> StorageResult<Option<StoredResource>> {
        Ok(None)
    }
    async fn update(
        &self,
        _: &TenantContext,
        _: &StoredResource,
        _: Value,
    ) -> StorageResult<StoredResource> {
        panic!("unexpected secondary update")
    }
    async fn delete(&self, _: &TenantContext, _: &str, _: &str) -> StorageResult<()> {
        panic!("unexpected secondary delete")
    }
    async fn count(&self, _: &TenantContext, _: Option<&str>) -> StorageResult<u64> {
        Ok(self.events.lock().unwrap().len() as u64)
    }
}

pub async fn composite_receipts<B: BulkSubmitJobStore + 'static>(
    backend: Arc<B>,
    tenant: &TenantContext,
    kind: BackendKind,
    fail: bool,
    secondary_failure: bool,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let failures = Arc::new(Mutex::new(0));
    let config = CompositeConfig::builder()
        .primary("primary", kind)
        .search_backend("secondary", BackendKind::Elasticsearch)
        .sync_mode(SyncMode::Synchronous)
        .build()
        .unwrap();
    let storage = Arc::new(
        CompositeStorage::new(
            config,
            HashMap::from([
                ("primary".to_string(), backend.clone() as DynStorage),
                (
                    "secondary".to_string(),
                    Arc::new(RecordingSecondary {
                        events: events.clone(),
                        fail_on: secondary_failure.then(|| "late-2-1000".to_string()),
                        failures: failures.clone(),
                    }) as DynStorage,
                ),
            ]),
        )
        .unwrap(),
    );
    let jobs = CompositeSubmitJobs::new(backend.clone(), storage);
    let submission = SubmissionId::generate("composite-receipt-contract");
    backend
        .create_submission(tenant, &submission, None)
        .await
        .unwrap();
    let manifest = backend
        .add_manifest(
            tenant,
            &submission,
            Some("http://provider/composite.json"),
            None,
        )
        .await
        .unwrap();
    let lease = claim(backend.as_ref(), &submission, &manifest.manifest_id).await;
    backend
        .create(
            tenant,
            "Patient",
            json!({"resourceType":"Patient", "id":"gone"}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend.delete(tenant, "Patient", "gone").await.unwrap();
    let mut expected = BTreeSet::new();
    for file in 0..3 {
        let mut entries: Vec<_> = (0..1100)
            .map(|line| {
                // Each later page contains both repeated IDs and resources that
                // cannot be synchronized by stopping after an earlier page.
                let id = if line >= 1000 {
                    format!("late-{file}-{line}")
                } else {
                    format!("p-{}", line % 701)
                };
                expected.insert(format!("Patient/{id}"));
                NdjsonEntry::new(
                    line + 1,
                    "Patient",
                    json!({"resourceType":"Patient", "id":id}),
                )
            })
            .collect();
        entries.push(NdjsonEntry::new(
            1101,
            "Patient",
            json!({"resourceType":"Patient", "id":"gone"}),
        ));
        entries.push(NdjsonEntry::new(
            1102,
            "Patient",
            json!({"resourceType":"Patient", "id":"must-be-skipped"}),
        ));
        let mut options =
            BulkProcessingOptions::new().with_file_url(format!("http://provider/{file}.ndjson"));
        options.max_errors = 1;
        let results = backend
            .process_entries(
                tenant,
                &submission,
                &manifest.manifest_id,
                entries,
                &options,
            )
            .await
            .unwrap();
        assert_eq!(
            results
                .iter()
                .filter(|r| r.outcome == BulkEntryOutcome::Success)
                .count(),
            1100
        );
        assert_eq!(results.last().unwrap().outcome, BulkEntryOutcome::Skipped);
    }
    assert!(events.lock().unwrap().is_empty());
    // The worker reaches receipts through Composite's delegation too.
    let delegated = persistence::core::BulkSubmitProvider::get_entry_results_page(
        &jobs,
        tenant,
        &submission,
        &manifest.manifest_id,
        Some(BulkEntryOutcome::Success),
        1,
        None,
    )
    .await
    .unwrap();
    assert!(delegated.entries[0].stored_identity.is_some());
    assert!(matches!(
        delegated.next,
        Some(persistence::core::EntryResultContinuation::Keyset(_))
    ));
    // #1007: the sync now runs as its own explicit step, mirroring what the
    // real worker does before calling `finish_manifest`/`fail_manifest` —
    // neither of those delegates to the primary syncs anything by itself
    // anymore.
    jobs.sync_ingested(&lease).await.unwrap();
    if fail {
        jobs.fail_manifest(&lease, "intentional partial failure")
            .await
            .unwrap();
    } else {
        jobs.finish_manifest(&lease).await.unwrap();
    }
    let mut actual = events.lock().unwrap().clone();
    actual.sort();
    if secondary_failure {
        assert!(
            *failures.lock().unwrap() > 0,
            "the secondary must actually reject the late resource"
        );
        expected.remove("Patient/late-2-1000");
    } else {
        assert_eq!(*failures.lock().unwrap(), 0);
    }
    let expected: Vec<_> = expected.into_iter().collect();
    assert_eq!(
        actual, expected,
        "every available resource syncs exactly once across all pages, including resources after a secondary error"
    );
    let current = backend
        .list_manifests(tenant, &submission)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        current.status,
        if fail {
            ManifestStatus::Failed
        } else {
            ManifestStatus::Completed
        }
    );
}
