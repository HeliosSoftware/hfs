//! #2394: `search_index.value_reference` is stored normalized (version suffix
//! stripped, absolute URLs also under `value_reference_target`), and the
//! builder rewrites old-format rows once, resumably and idempotently.
//!
//! Child module of the `mongodb_tests` root; `use super::*;` reaches its
//! private harness. Each test builds a database with a backend (so the
//! resources and their search rows are real), degrades the `subject` rows to
//! the pre-#2394 shape with raw writes, removes the completion marker and
//! boots again.

use super::*;

const ABSOLUTE_VERSIONED: &str = "http://example.org/fhir/Patient/p1/_history/2";
const VERSIONED: &str = "Patient/p1/_history/3";
const RELATIVE: &str = "Patient/p1";
const ABSOLUTE: &str = "http://example.org/fhir/Patient/p1";
const IDS: [&str; 3] = ["obs-rel", "obs-ver", "obs-abs"];

fn search_index(db: &mongodb::Database) -> mongodb::Collection<Document> {
    db.collection::<Document>("search_index")
}

async fn seed_observations(backend: &MongoBackend, tenant: &TenantContext) {
    for (id, reference) in [
        ("obs-rel", RELATIVE),
        ("obs-ver", VERSIONED),
        ("obs-abs", ABSOLUTE_VERSIONED),
    ] {
        backend
            .create(
                tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": id,
                    "status": "final",
                    "subject": {"reference": reference},
                    "code": {"coding": [{"code": "8867-4"}]}
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed observation");
    }
}

async fn subject_row(db: &mongodb::Database, id: &str) -> Document {
    search_index(db)
        .find_one(doc! { "resource_id": id, "param_name": "subject" })
        .await
        .expect("read search_index")
        .unwrap_or_else(|| panic!("no subject row for {id}"))
}

/// Puts the `subject` rows of `obs-ver` and `obs-abs` back into the
/// pre-#2394 shape and clears the marker and the checkpoint.
async fn degrade(db: &mongodb::Database) {
    search_index(db)
        .update_one(
            doc! { "resource_id": "obs-ver", "param_name": "subject" },
            doc! { "$set": { "value_reference": VERSIONED } },
        )
        .await
        .unwrap();
    search_index(db)
        .update_one(
            doc! { "resource_id": "obs-abs", "param_name": "subject" },
            doc! {
                "$set": { "value_reference": ABSOLUTE_VERSIONED },
                "$unset": { "value_reference_target": "" },
            },
        )
        .await
        .unwrap();
    clear_marker(db).await;
}

async fn clear_marker(db: &mongodb::Database) {
    db.collection::<Document>("schema_version")
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$unset": {
                "search_indexes.references_normalized": "",
                "search_indexes.reference_normalize_last_id": "",
            } },
        )
        .await
        .unwrap();
}

async fn marker(db: &mongodb::Database) -> Option<bool> {
    let sv = db
        .collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await
        .unwrap()?;
    sv.get_document("search_indexes")
        .ok()?
        .get_bool("references_normalized")
        .ok()
}

fn assert_normalized(row: &Document, id: &str) {
    if id == "obs-abs" {
        assert_eq!(row.get_str("value_reference"), Ok(ABSOLUTE), "{id}");
        assert_eq!(row.get_str("value_reference_target"), Ok(RELATIVE), "{id}");
    } else {
        assert_eq!(row.get_str("value_reference"), Ok(RELATIVE), "{id}");
        assert!(!row.contains_key("value_reference_target"), "{id}: {row}");
    }
}

/// The bare id finds every form of the target; the qualified `Type/id` finds
/// the relative and the versioned ones (an absolute URL never matched it).
async fn assert_full_recall(backend: &MongoBackend, tenant: &TenantContext) {
    for (value, expected) in [
        ("Patient/p1", vec!["obs-rel", "obs-ver"]),
        ("p1", vec!["obs-abs", "obs-rel", "obs-ver"]),
    ] {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            values: vec![SearchValue::eq(value)],
            ..Default::default()
        });
        let result = backend.search(tenant, &query).await.expect("search");
        let mut ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        ids.sort_unstable();
        assert_eq!(ids, expected, "subject={value}");
    }
}

#[tokio::test]
async fn mongodb_integration_builder_normalizes_old_reference_rows_and_is_idempotent() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db_name = build_test_database_name("ref_normalize");
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    let tenant = create_tenant("ref-normalize");

    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    seed_observations(&backend, &tenant).await;
    // New writes are normalized already.
    for id in IDS {
        assert_normalized(&subject_row(&db, id).await, id);
    }
    // A contained row in the old shape, in the other collection.
    let contained = db.collection::<Document>("search_index_contained");
    contained
        .insert_one(doc! {
            "tenant_id": "t", "resource_type": "Observation", "resource_id": "holder",
            "param_name": "subject", "param_type": "reference",
            "value_reference": ABSOLUTE_VERSIONED,
            "contained_type": "Observation", "contained_local_id": "c",
        })
        .await
        .unwrap();
    degrade(&db).await;
    assert_eq!(
        subject_row(&db, "obs-ver").await.get_str("value_reference"),
        Ok(VERSIONED)
    );
    assert_eq!(marker(&db).await, None);

    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    backend.wait_for_search_index_build().await;
    for id in IDS {
        assert_normalized(&subject_row(&db, id).await, id);
    }
    let moved = contained
        .find_one(doc! { "resource_id": "holder" })
        .await
        .unwrap()
        .unwrap();
    assert_normalized_contained(&moved);
    assert_eq!(marker(&db).await, Some(true));
    assert_full_recall(&backend, &tenant).await;

    // Idempotent: a second boot changes nothing.
    let before = subject_row(&db, "obs-abs").await;
    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    assert_eq!(
        backend.wait_for_search_index_build().await,
        Some(BuildOutcome::UpToDate)
    );
    assert_eq!(subject_row(&db, "obs-abs").await, before);
    // Clearing only the marker re-runs the pass, which finds nothing to do.
    clear_marker(&db).await;
    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    backend.wait_for_search_index_build().await;
    assert_eq!(subject_row(&db, "obs-abs").await, before);
    assert_eq!(marker(&db).await, Some(true));
    assert_full_recall(&backend, &tenant).await;
}

fn assert_normalized_contained(row: &Document) {
    assert_eq!(row.get_str("value_reference"), Ok(ABSOLUTE));
    assert_eq!(row.get_str("value_reference_target"), Ok(RELATIVE));
}

/// An interrupted pass resumes after its `_id` checkpoint: rows at or before
/// it are not revisited, rows after it are rewritten.
#[tokio::test]
async fn mongodb_integration_builder_reference_normalization_resumes_from_checkpoint() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db_name = build_test_database_name("ref_normalize_resume");
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    let tenant = create_tenant("ref-normalize-resume");

    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    seed_observations(&backend, &tenant).await;
    degrade(&db).await;

    // Pretend the previous run got as far as the lower-`_id` degraded row.
    let first = subject_row(&db, "obs-ver").await;
    let second = subject_row(&db, "obs-abs").await;
    let (done, pending) = if first.get_object_id("_id").expect("ObjectId _id")
        < second.get_object_id("_id").expect("ObjectId _id")
    {
        (("obs-ver", first), ("obs-abs", second))
    } else {
        (("obs-abs", second), ("obs-ver", first))
    };
    db.collection::<Document>("schema_version")
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": {
                "search_indexes.reference_normalize_last_id.search_index":
                    done.1.get("_id").unwrap().clone(),
            } },
        )
        .await
        .unwrap();

    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    backend.wait_for_search_index_build().await;
    assert_normalized(&subject_row(&db, pending.0).await, pending.0);
    assert_eq!(
        subject_row(&db, done.0).await.get("value_reference"),
        done.1.get("value_reference"),
        "a row at the checkpoint is not revisited"
    );
    assert_eq!(marker(&db).await, Some(true));

    // Dropping the checkpoint and the marker finishes the job.
    clear_marker(&db).await;
    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    backend.wait_for_search_index_build().await;
    for id in IDS {
        assert_normalized(&subject_row(&db, id).await, id);
    }
    assert_full_recall(&backend, &tenant).await;
}

/// `off` still rewrites the rows and sets the marker, but does not build the
/// target index, so readers stay on the legacy arms and still find everything.
#[tokio::test]
async fn mongodb_integration_builder_off_mode_normalizes_rows_without_the_target_index() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db_name = build_test_database_name("ref_normalize_off");
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    let tenant = create_tenant("ref-normalize-off");

    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    seed_observations(&backend, &tenant).await;
    degrade(&db).await;
    search_index(&db)
        .drop_index("idx_search_reference_target_v2")
        .await
        .unwrap();

    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Off).await;
    backend.wait_for_search_index_build().await;
    assert!(
        !search_index_names(&db)
            .await
            .contains(&"idx_search_reference_target_v2".to_string())
    );
    for id in IDS {
        assert_normalized(&subject_row(&db, id).await, id);
    }
    assert_eq!(marker(&db).await, Some(true));
    assert_full_recall(&backend, &tenant).await;
}
