use super::*;
use serde_json::Value;

fn patient(id: &str, marker: &str) -> Value {
    json!({"resourceType": "Patient", "id": id,
        "identifier": [{"system": "http://example.org/transaction", "value": marker}],
        "active": true})
}

fn entry(method: BundleMethod, url: &str, resource: Option<Value>) -> BundleEntry {
    BundleEntry {
        method,
        url: url.to_owned(),
        resource,
        ..Default::default()
    }
}

/// A `Type?criteria` entry with its criteria typed, as REST sends it.
fn conditional_entry(
    backend: &MongoBackend,
    tenant: &TenantContext,
    method: BundleMethod,
    url: &str,
    resource: Option<Value>,
) -> BundleEntry {
    conditional_url_suite::with_typed_criteria(backend, tenant, entry(method, url, resource))
}

#[tokio::test]
async fn mongodb_transaction_mixed_resource_filters_never_delete_a_nonmatch() {
    let Some(backend) =
        create_backend_with_full_registry("transaction_mixed_resource_filters").await
    else {
        return;
    };
    let tenant = create_tenant("transaction-mixed-resource-filters");
    for (id, marker) in [("a", "OTHER"), ("b", "MATCH")] {
        backend
            .create(
                &tenant,
                "Patient",
                patient(id, marker),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    for criteria in [
        "Patient?_id=a&identifier=http://example.org/transaction|MATCH",
        "Patient?_lastUpdated=lt1900-01-01&identifier=http://example.org/transaction|MATCH",
    ] {
        let Some(result) = process_transaction_or_skip(
            &backend,
            &tenant,
            vec![conditional_entry(
                &backend,
                &tenant,
                BundleMethod::Delete,
                criteria,
                None,
            )],
            "mongodb_transaction_mixed_resource_filters_never_delete_a_nonmatch",
        )
        .await
        else {
            return;
        };
        assert_eq!(
            result.entries[0].effect,
            BundleEntryEffect::NotFound,
            "{criteria}"
        );
        for id in ["a", "b"] {
            assert!(
                backend
                    .read(&tenant, "Patient", id)
                    .await
                    .unwrap()
                    .is_some(),
                "{criteria} deleted {id}"
            );
        }
    }
    let deleted = backend
        .process_transaction(
            &tenant,
            vec![conditional_entry(
                &backend,
                &tenant,
                BundleMethod::Delete,
                "Patient?_id=b&identifier=http://example.org/transaction|MATCH",
                None,
            )],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.entries[0].effect, BundleEntryEffect::Deleted);
    assert!(
        backend
            .read(&tenant, "Patient", "a")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn mongodb_transaction_duplicate_index_rows_cannot_hide_a_second_match() {
    let Some(backend) =
        create_backend_with_full_registry("transaction_duplicate_driver_rows").await
    else {
        return;
    };
    let tenant = create_tenant("transaction-duplicate-driver-rows");
    backend
        .create(
            &tenant,
            "Patient",
            patient("a", "MATCH"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let db = backend.get_database().await.unwrap();
    let index = db.collection::<Document>("search_index");
    let mut duplicate = index
        .find_one(
            doc! {"tenant_id": tenant.tenant_id().as_str(), "resource_type": "Patient",
            "resource_id": "a", "param_name": "identifier"},
        )
        .await
        .unwrap()
        .unwrap();
    duplicate.remove("_id");
    index.insert_many(vec![duplicate; 129]).await.unwrap();
    backend
        .create(
            &tenant,
            "Patient",
            patient("b", "MATCH"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let Some(_) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![entry(BundleMethod::Get, "Patient/a", None)],
        "mongodb_transaction_duplicate_index_rows_cannot_hide_a_second_match",
    )
    .await
    else {
        return;
    };
    let error = backend
        .process_transaction(
            &tenant,
            vec![conditional_entry(
                &backend,
                &tenant,
                BundleMethod::Delete,
                "Patient?identifier=http://example.org/transaction|MATCH",
                None,
            )],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, TransactionError::MultipleMatches { count: 2, .. }),
        "{error:?}"
    );
    for id in ["a", "b"] {
        assert!(
            backend
                .read(&tenant, "Patient", id)
                .await
                .unwrap()
                .is_some()
        );
    }
}

#[tokio::test]
async fn mongodb_transaction_overlaps_and_changed_forward_targets_roll_back() {
    let Some(backend) = create_backend_with_full_registry("transaction_identity_consistency").await
    else {
        return;
    };
    let tenant = create_tenant("transaction-identity-consistency");
    backend
        .create(
            &tenant,
            "Patient",
            patient("a", "MATCH"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let Some(_) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![entry(BundleMethod::Get, "Patient/a", None)],
        "mongodb_transaction_overlaps_and_changed_forward_targets_roll_back",
    )
    .await
    else {
        return;
    };
    let mut changed = patient("a", "MATCH");
    changed["active"] = json!(false);
    let overlapping = vec![
        entry(BundleMethod::Put, "Patient/a", Some(changed)),
        conditional_entry(
            &backend,
            &tenant,
            BundleMethod::Put,
            "Patient?identifier=http://example.org/transaction|MATCH",
            Some(patient("a", "MATCH")),
        ),
    ];
    let error = backend
        .process_transaction(&tenant, overlapping, FhirVersion::default())
        .await
        .unwrap_err();
    assert!(
        matches!(error, TransactionError::BundleError { ref message, .. } if message.contains("overlap")),
        "{error:?}"
    );
    assert_eq!(
        backend
            .read(&tenant, "Patient", "a")
            .await
            .unwrap()
            .unwrap()
            .content()["active"],
        true
    );

    let observation = entry(
        BundleMethod::Post,
        "Observation",
        Some(json!({
            "resourceType":"Observation", "id":"observation", "status":"final", "code":{"text":"test"},
            "subject":{"reference":"urn:uuid:conditional-patient"}
        })),
    );
    let mut conditional = conditional_entry(
        &backend,
        &tenant,
        BundleMethod::Put,
        "Patient?identifier=http://example.org/transaction|MATCH",
        Some(patient("new", "MATCH")),
    );
    conditional.full_url = Some("urn:uuid:conditional-patient".into());
    let error = backend
        .process_transaction(
            &tenant,
            vec![
                observation,
                entry(
                    BundleMethod::Put,
                    "Patient/a",
                    Some(patient("a", "CHANGED")),
                ),
                conditional,
            ],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, TransactionError::BundleError { ref message, .. } if message.contains("reference") || message.contains("overlap")),
        "{error:?}"
    );
    assert!(
        backend
            .read(&tenant, "Observation", "observation")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .read(&tenant, "Patient", "new")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        backend
            .read(&tenant, "Patient", "a")
            .await
            .unwrap()
            .unwrap()
            .content()["identifier"][0]["value"],
        "MATCH"
    );
}

#[tokio::test]
async fn mongodb_transaction_replay_keeps_forward_references_and_metadata_atomic() {
    use crate::bulk_submit::FailPoint;
    use futures::TryStreamExt;
    use std::collections::HashSet;

    let app = "fp-forward-reference-metadata";
    let Some(backend) = create_backend_with_app_name("transaction_forward_replay", app).await
    else {
        return;
    };
    let tenant = create_tenant("transaction-forward-replay");
    let Some(fail_point) = FailPoint::enable(
        app,
        doc! {
            "failCommands": ["commitTransaction"],
            "errorCode": 112,
            "errorLabels": ["TransientTransactionError"],
        },
        doc! { "times": 1 },
    )
    .await
    else {
        return;
    };
    let observation = entry(
        BundleMethod::Post,
        "Observation",
        Some(json!({
            "resourceType": "Observation", "status": "final", "code": {"text": "test"},
            "subject": {"reference": "urn:uuid:later-patient"}
        })),
    );
    let mut patient = entry(
        BundleMethod::Post,
        "Patient",
        Some(json!({"resourceType": "Patient", "name": [{"family": "Replay"}]})),
    );
    patient.full_url = Some("urn:uuid:later-patient".into());
    let result = backend
        .process_transaction(&tenant, vec![observation, patient], FhirVersion::default())
        .await;
    let entered = fail_point.off_and_count().await;
    if matches!(
        result,
        Err(TransactionError::UnsupportedIsolationLevel { .. })
    ) {
        assert!(!transactions_required(), "test requires a replica set");
        return;
    }
    let result = result.unwrap();
    assert_eq!(
        entered, 1,
        "the commit aborted once before the clean replay"
    );
    assert_eq!(result.entries.len(), 2);
    let patient_reference = result.entries[1].reference().unwrap();
    assert_eq!(
        result.entries[0].resource.as_ref().unwrap()["subject"]["reference"],
        patient_reference
    );

    let db = backend.get_database().await.unwrap();
    let scope = doc! { "tenant_id": tenant.tenant_id().as_str() };
    let rows: Vec<Document> = db
        .collection("resources")
        .find(scope.clone())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "aborted resource rows must not survive");
    let mut committed = HashSet::new();
    for row in &rows {
        let kind = row.get_str("resource_type").unwrap();
        let id = row.get_str("id").unwrap();
        committed.insert(format!("{kind}/{id}"));
        if kind == "Observation" {
            assert_eq!(
                row.get_document("data")
                    .unwrap()
                    .get_document("subject")
                    .unwrap()
                    .get_str("reference")
                    .unwrap(),
                patient_reference
            );
        }
    }
    assert!(committed.contains(&patient_reference));
    assert_eq!(
        db.collection::<Document>("resource_history")
            .count_documents(scope.clone())
            .await
            .unwrap(),
        2
    );
    let index: Vec<Document> = db
        .collection("search_index")
        .find(scope)
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(!index.is_empty());
    for row in index {
        let identity = format!(
            "{}/{}",
            row.get_str("resource_type").unwrap(),
            row.get_str("resource_id").unwrap()
        );
        assert!(
            committed.contains(&identity),
            "index row survived for an aborted identity: {identity}"
        );
    }
}

#[tokio::test]
async fn mongodb_transaction_conditional_put_without_an_object_body_is_rejected_before_pinning() {
    let Some(backend) = create_backend_with_full_registry("transaction_body_before_pinning").await
    else {
        return;
    };
    let tenant = create_tenant("transaction-body-before-pinning");
    backend
        .create(
            &tenant,
            "Patient",
            patient("probe", "OTHER"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let Some(_) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![entry(BundleMethod::Get, "Patient/probe", None)],
        "mongodb_transaction_conditional_put_without_an_object_body_is_rejected_before_pinning",
    )
    .await
    else {
        return;
    };
    for (body, expected) in [
        (None, "missing required field: resource"),
        (Some(json!("not an object")), "must be a JSON object"),
    ] {
        let observation = entry(
            BundleMethod::Post,
            "Observation",
            Some(json!({
                "resourceType": "Observation", "status": "final", "code": {"text": "test"},
                "subject": {"reference": "urn:uuid:conditional-patient"}
            })),
        );
        let mut conditional = conditional_entry(
            &backend,
            &tenant,
            BundleMethod::Put,
            "Patient?identifier=http://example.org/transaction|MATCH",
            body,
        );
        conditional.full_url = Some("urn:uuid:conditional-patient".into());
        let error = backend
            .process_transaction(
                &tenant,
                vec![observation, conditional],
                FhirVersion::default(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, TransactionError::BundleError { index: 1, ref message } if message.contains(expected)),
            "{error:?}"
        );
        assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
        assert_eq!(
            backend.count(&tenant, Some("Observation")).await.unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn mongodb_transaction_conditional_create_matching_a_sibling_after_a_written_reference_rolls_back()
 {
    let Some(backend) = create_backend_with_full_registry("transaction_stale_pinned_create").await
    else {
        return;
    };
    let tenant = create_tenant("transaction-stale-pinned-create");
    backend
        .create(
            &tenant,
            "Patient",
            patient("probe", "OTHER"),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let Some(_) = process_transaction_or_skip(
        &backend,
        &tenant,
        vec![entry(BundleMethod::Get, "Patient/probe", None)],
        "mongodb_transaction_conditional_create_matching_a_sibling_after_a_written_reference_rolls_back",
    )
    .await
    else {
        return;
    };
    let observation = entry(
        BundleMethod::Post,
        "Observation",
        Some(json!({
            "resourceType": "Observation", "status": "final", "code": {"text": "test"},
            "subject": {"reference": "urn:uuid:conditional-patient"}
        })),
    );
    let plain = entry(
        BundleMethod::Post,
        "Patient",
        Some(patient("plain", "MATCH")),
    );
    let mut conditional = entry(
        BundleMethod::Post,
        "Patient",
        Some(patient("conditional", "MATCH")),
    );
    conditional.if_none_exist = Some("identifier=http://example.org/transaction|MATCH".into());
    conditional.full_url = Some("urn:uuid:conditional-patient".into());
    let error = backend
        .process_transaction(
            &tenant,
            vec![observation, plain, conditional],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            TransactionError::BundleError { index: 2, ref message }
                if message.contains("changed after a bundle reference was written")
        ),
        "{error:?}"
    );
    assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
    assert_eq!(
        backend.count(&tenant, Some("Observation")).await.unwrap(),
        0
    );
}
