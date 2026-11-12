// Shared bulk-submit regressions for #2554, run against every backend's own
// bulk-write implementation. The including module aliases the library as
// `persistence`.
//
// A submitted resource is written without empty objects, empty arrays and
// `null` members, and the cleanup runs on what is written: under the merge
// import mode a `null` member still removes the stored element and an empty
// array still clears it.
use helios_fhir::FhirVersion;
use persistence::core::bulk_submit::{
    BulkProcessingOptions, BulkSubmitProvider, ImportMode, NdjsonEntry, SubmissionId,
};
use persistence::tenant::TenantContext;
use serde_json::{Value, json};

async fn seed_manifest<B: BulkSubmitProvider>(
    backend: &B,
    tenant: &TenantContext,
) -> (SubmissionId, String) {
    let id = SubmissionId::generate("empty-elements-provider");
    backend.create_submission(tenant, &id, None).await.unwrap();
    let manifest = backend
        .add_manifest(
            tenant,
            &id,
            Some("https://provider.example/manifest.json"),
            None,
        )
        .await
        .unwrap();
    (id, manifest.manifest_id)
}

async fn stored<B: BulkSubmitProvider>(backend: &B, tenant: &TenantContext, id: &str) -> Value {
    let mut content = backend
        .read(tenant, "Patient", id)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("Patient/{id} is stored"))
        .content()
        .clone();
    if let Some(object) = content.as_object_mut() {
        object.remove("meta");
    }
    content
}

async fn seed<B: BulkSubmitProvider>(backend: &B, tenant: &TenantContext, resource: Value) {
    backend
        .create(tenant, "Patient", resource, FhirVersion::default())
        .await
        .unwrap();
}

async fn ingest<B: BulkSubmitProvider>(
    backend: &B,
    tenant: &TenantContext,
    entries: Vec<Value>,
    options: &BulkProcessingOptions,
) -> Vec<persistence::core::bulk_submit::BulkEntryResult> {
    let (submission, manifest) = seed_manifest(backend, tenant).await;
    let entries = entries
        .into_iter()
        .enumerate()
        .map(|(index, resource)| NdjsonEntry::new(index as u64 + 1, "Patient", resource))
        .collect();
    let results = backend
        .process_entries(tenant, &submission, &manifest, entries, options)
        .await
        .unwrap();
    assert!(results.iter().all(|r| r.is_success()), "{results:?}");
    results
}

/// Merge mode, in one batch that updates a stored resource and creates two:
/// `null` removes the stored element, `[]` clears it, omitted elements remain,
/// and the new resources (with and without an id) are written clean.
pub async fn merge_import_applies_the_mode_before_dropping_empty_elements<B>(
    backend: &B,
    tenant: &TenantContext,
) where
    B: BulkSubmitProvider,
{
    seed(
        backend,
        tenant,
        json!({
            "resourceType": "Patient", "id": "ee-merge", "gender": "female",
            "birthDate": "1970-01-01",
            "telecom": [{"system": "phone", "value": "1"}],
            "name": [{"family": "Stale"}]
        }),
    )
    .await;

    let results = ingest(
        backend,
        tenant,
        vec![
            json!({
                "resourceType": "Patient", "id": "ee-merge",
                "telecom": null, "name": [], "address": [{}], "active": true
            }),
            json!({
                "resourceType": "Patient", "id": "ee-merge-new",
                "active": true, "name": [{}], "gender": null
            }),
            json!({"resourceType": "Patient", "active": false, "telecom": []}),
        ],
        &BulkProcessingOptions::new().with_import_mode(ImportMode::Merge),
    )
    .await;

    assert_eq!(
        stored(backend, tenant, "ee-merge").await,
        json!({
            "resourceType": "Patient", "id": "ee-merge", "gender": "female",
            "birthDate": "1970-01-01", "active": true
        })
    );
    assert_eq!(
        stored(backend, tenant, "ee-merge-new").await,
        json!({"resourceType": "Patient", "id": "ee-merge-new", "active": true})
    );
    let assigned = results[2].resource_id.clone().expect("server-assigned id");
    assert_eq!(
        stored(backend, tenant, &assigned).await,
        json!({"resourceType": "Patient", "id": assigned, "active": false})
    );
}

/// Replace mode, in one batch that updates a stored resource and creates one:
/// both are written clean.
pub async fn replace_import_drops_empty_elements<B>(backend: &B, tenant: &TenantContext)
where
    B: BulkSubmitProvider,
{
    seed(
        backend,
        tenant,
        json!({"resourceType": "Patient", "id": "ee-replace", "gender": "female"}),
    )
    .await;

    ingest(
        backend,
        tenant,
        vec![
            json!({
                "resourceType": "Patient", "id": "ee-replace",
                "active": true, "name": [{"given": []}], "telecom": null
            }),
            json!({
                "resourceType": "Patient", "id": "ee-replace-new",
                "active": true, "name": [{"family": "Doe", "given": ["Ann", null]}]
            }),
        ],
        &BulkProcessingOptions::new(),
    )
    .await;

    assert_eq!(
        stored(backend, tenant, "ee-replace").await,
        json!({"resourceType": "Patient", "id": "ee-replace", "active": true})
    );
    assert_eq!(
        stored(backend, tenant, "ee-replace-new").await,
        json!({
            "resourceType": "Patient", "id": "ee-replace-new",
            "active": true, "name": [{"family": "Doe", "given": ["Ann"]}]
        })
    );
}

/// With `skip_unchanged`, a resubmission that differs from the stored
/// resource only by empty elements compares equal after cleanup, so nothing
/// is written; for the backends that honour `skip_unchanged` (SQLite and
/// PostgreSQL).
#[allow(dead_code)]
pub async fn skip_unchanged_compares_the_clean_candidate<B>(backend: &B, tenant: &TenantContext)
where
    B: BulkSubmitProvider,
{
    seed(
        backend,
        tenant,
        json!({"resourceType": "Patient", "id": "ee-skip", "active": true}),
    )
    .await;

    let results = ingest(
        backend,
        tenant,
        vec![json!({
            "resourceType": "Patient", "id": "ee-skip",
            "active": true, "name": [], "telecom": [{}]
        })],
        &BulkProcessingOptions::new().with_skip_unchanged(true),
    )
    .await;

    assert!(results[0].unchanged, "{results:?}");
    let version = backend
        .read(tenant, "Patient", "ee-skip")
        .await
        .unwrap()
        .unwrap()
        .version_id()
        .to_string();
    assert_eq!(version, "1");
}
