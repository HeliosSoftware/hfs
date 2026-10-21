//! #2008: a metadata-only `$reindex` backfills `missing_params` on resources
//! written before #1984 and leaves `search_index` untouched. Child module of
//! the `mongodb_tests` root, the same arrangement as
//! `tests/mongodb/reindex_id_range.rs`.

use super::*;

use std::collections::BTreeSet;

use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexStatus};

use super::reindex_id_walk::wait_for_terminal;

/// Six Patients, every other one without a `birthDate`, and one Observation.
async fn seed(backend: &MongoBackend, tenant: &TenantContext) {
    for n in 0..6 {
        let mut patient =
            json!({"resourceType": "Patient", "id": format!("p{n}"), "gender": "female"});
        if n % 2 == 0 {
            patient["birthDate"] = json!("1970-01-01");
        }
        backend
            .create(tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }
    backend
        .create(
            tenant,
            "Observation",
            json!({"resourceType": "Observation", "id": "o1", "status": "final", "code": {"text": "metadata"}}),
            FhirVersion::default(),
        )
        .await
        .unwrap();
}

async fn resources(backend: &MongoBackend) -> mongodb::Collection<Document> {
    backend
        .get_database()
        .await
        .unwrap()
        .collection::<Document>("resources")
}

/// Removes `missing_params` from every resource of `tenant`: what a dataset
/// written before #1984 looks like, and what the definition-maintenance
/// procedure in `docs/mongodb/search-indexes.md` leaves behind.
async fn unset_metadata(backend: &MongoBackend, tenant: &TenantContext) {
    resources(backend)
        .await
        .update_many(
            doc! { "tenant_id": tenant.tenant_id().as_str() },
            doc! { "$unset": { "missing_params": "" } },
        )
        .await
        .unwrap();
}

/// Each live resource of `tenant` as `Type/id`, with its `missing_params`
/// (`None` when the field is absent).
async fn metadata(
    backend: &MongoBackend,
    tenant: &TenantContext,
) -> Vec<(String, Option<Vec<String>>)> {
    let mut cursor = resources(backend)
        .await
        .find(doc! { "tenant_id": tenant.tenant_id().as_str(), "is_deleted": false })
        .await
        .unwrap();
    let mut out = Vec::new();
    while cursor.advance().await.unwrap() {
        let row: Document = cursor.deserialize_current().unwrap();
        let names = row.get_array("missing_params").ok().map(|names| {
            names
                .iter()
                .map(|name| name.as_str().unwrap().to_string())
                .collect()
        });
        out.push((
            format!(
                "{}/{}",
                row.get_str("resource_type").unwrap(),
                row.get_str("id").unwrap()
            ),
            names,
        ));
    }
    out.sort();
    out
}

/// Every `search_index` and `search_index_contained` row of `tenant`, whole,
/// so a run that rewrote a row (new `_id`) is told apart from one that left it.
async fn index_rows(backend: &MongoBackend, tenant: &TenantContext) -> BTreeSet<String> {
    let db = backend.get_database().await.unwrap();
    let mut rows = BTreeSet::new();
    for collection in ["search_index", "search_index_contained"] {
        let mut cursor = db
            .collection::<Document>(collection)
            .find(doc! { "tenant_id": tenant.tenant_id().as_str() })
            .await
            .unwrap();
        while cursor.advance().await.unwrap() {
            let row: Document = cursor.deserialize_current().unwrap();
            rows.insert(format!("{collection}:{row}"));
        }
    }
    rows
}

/// Ids of the Patients `birthdate:missing=<missing>` finds.
async fn birthdate_missing(
    backend: &MongoBackend,
    tenant: &TenantContext,
    missing: bool,
) -> BTreeSet<String> {
    let query = SearchQuery::new("Patient")
        .with_count(100)
        .with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq(if missing { "true" } else { "false" })],
            chain: vec![],
            components: vec![],
        });
    backend
        .search(tenant, &query)
        .await
        .unwrap()
        .resources
        .items
        .iter()
        .map(|resource| resource.id().to_string())
        .collect()
}

async fn run(
    backend: &Arc<MongoBackend>,
    tenant: &TenantContext,
    request: ReindexRequest,
) -> helios_persistence::search::ReindexProgress {
    let op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
    let job = op.start(tenant.clone(), request, None).await.unwrap();
    wait_for_terminal(&op, &job).await
}

/// After a metadata-only run on a pre-#1984 dataset every live resource has
/// `missing_params`, matching what a write would have stored, the value
/// index is byte-for-byte unchanged, `:missing` finds the same resources, and
/// another tenant is not touched.
#[tokio::test]
async fn mongodb_metadata_only_reindex_backfills_without_touching_the_index() {
    let Some(backend) = create_backend_with_full_registry("reindex_metadata_only").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("metadata-only");
    let other = create_tenant("metadata-only-other");
    seed(&backend, &tenant).await;
    seed(&backend, &other).await;

    let written = metadata(&backend, &tenant).await;
    assert!(
        written.iter().all(|(_, names)| names.is_some()),
        "writes store missing_params: {written:?}"
    );
    let found_missing = birthdate_missing(&backend, &tenant, true).await;
    let found_present = birthdate_missing(&backend, &tenant, false).await;
    assert_eq!(found_missing.len(), 3);
    assert_eq!(found_present.len(), 3);

    unset_metadata(&backend, &tenant).await;
    unset_metadata(&backend, &other).await;
    let rows = index_rows(&backend, &tenant).await;
    assert!(!rows.is_empty());
    assert_eq!(
        birthdate_missing(&backend, &tenant, true).await,
        found_missing
    );

    let progress = run(&backend, &tenant, ReindexRequest::all().metadata_only()).await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert!(progress.errors.is_empty(), "{progress:?}");
    assert_eq!(progress.processed_resources, 7);
    assert_eq!(progress.metadata_updated, 7);
    assert_eq!(progress.entries_created, 0);
    assert!(progress.metadata_only);

    assert_eq!(
        metadata(&backend, &tenant).await,
        written,
        "the backfill stores what the writes stored"
    );
    assert_eq!(
        index_rows(&backend, &tenant).await,
        rows,
        "search_index is untouched"
    );
    assert_eq!(
        birthdate_missing(&backend, &tenant, true).await,
        found_missing
    );
    assert_eq!(
        birthdate_missing(&backend, &tenant, false).await,
        found_present
    );
    assert!(
        metadata(&backend, &other)
            .await
            .iter()
            .all(|(_, names)| names.is_none()),
        "another tenant is not backfilled"
    );
}

/// `skipExistingMetadata` fills in only the resources without the field, and
/// a type-scoped run only that type.
#[tokio::test]
async fn mongodb_metadata_only_reindex_can_skip_existing_metadata() {
    let Some(backend) = create_backend_with_full_registry("reindex_metadata_skip").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("metadata-skip");
    seed(&backend, &tenant).await;
    let written = metadata(&backend, &tenant).await;
    unset_metadata(&backend, &tenant).await;
    // A sentinel no extraction produces, so a rewrite would show.
    resources(&backend)
        .await
        .update_one(
            doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Patient", "id": "p0" },
            doc! { "$set": { "missing_params": ["sentinel"] } },
        )
        .await
        .unwrap();

    let progress = run(
        &backend,
        &tenant,
        ReindexRequest::for_types(["Patient"])
            .metadata_only()
            .skip_existing_metadata(),
    )
    .await;
    assert_eq!(progress.status, ReindexStatus::Completed, "{progress:?}");
    assert_eq!(progress.processed_resources, 6);
    assert_eq!(progress.metadata_updated, 5, "p0 already had metadata");

    let after = metadata(&backend, &tenant).await;
    for ((key, names), (_, expected)) in after.iter().zip(&written) {
        match key.as_str() {
            "Patient/p0" => assert_eq!(names.as_deref(), Some(&["sentinel".to_string()][..])),
            "Observation/o1" => assert_eq!(names, &None, "out of the run's scope"),
            _ => assert_eq!(names, expected, "{key}"),
        }
    }
}

/// With search offloaded, MongoDB keeps no metadata a run could write, so a
/// metadata-only run is refused before a job starts.
#[tokio::test]
async fn mongodb_metadata_only_reindex_is_unsupported_when_search_is_offloaded() {
    let Some(backend) =
        create_backend_with_search_offloaded("reindex_metadata_offloaded", true).await
    else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let backend = Arc::new(backend);
    let op = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
    let error = op
        .start(
            create_tenant("metadata-offloaded"),
            ReindexRequest::all().metadata_only(),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            helios_persistence::search::ReindexError::Unsupported { .. }
        ),
        "{error:?}"
    );
}
