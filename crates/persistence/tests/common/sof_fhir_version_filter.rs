//! Backend-agnostic SoF runner check for the FHIR version filter (#2254).
//!
//! A run whose `ViewFilters::fhir_version` names a version reads only the
//! resources stored under it: on a flat scan, every `unionAll` branch, a
//! `repeat` seed and its join-back, `group` resolution, the columnar path and
//! the export reader. A run naming none reads every version, as before.
//!
//! Included with `#[path = "common/sof_fhir_version_filter.rs"]` by the
//! runner test binaries; needs the `R4` and `R5` features.

#![allow(dead_code)]

use std::collections::BTreeMap;

use futures::StreamExt;
use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

fn filters(version: Option<FhirVersion>) -> ViewFilters {
    ViewFilters {
        fhir_version: version,
        ..Default::default()
    }
}

/// `column` of every row, sorted (row order is not what this checks).
async fn column(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    filters: ViewFilters,
    column: &str,
) -> Vec<String> {
    let mut stream = runner
        .run_view(tenant, view.clone(), filters)
        .await
        .expect("run_view");
    let mut values = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.expect("row must succeed");
        values.push(row[column].as_str().unwrap_or("<null>").to_string());
    }
    values.sort();
    values
}

/// A run whose `patient`/`group` names a resource stored under another
/// version than the run's: that resource is absent from the run's data, so
/// the run answers the way the runner answers any absent compartment target.
/// The SQL runners select nothing; the in-process runner (MongoDB's
/// patient/group fallback, S3) refuses the run as naming a missing resource.
/// Either way it must not read the other version's rows.
async fn absent_target_run(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    filters: ViewFilters,
) -> Result<Vec<Value>, String> {
    let mut stream = match runner.run_view(tenant, view.clone(), filters).await {
        Ok(stream) => stream,
        Err(e) => return Err(e.to_string()),
    };
    let mut rows = Vec::new();
    while let Some(row) = stream.next().await {
        match row {
            Ok(row) => rows.push(row),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(rows)
}

fn strings(values: &[&str]) -> Vec<String> {
    let mut values: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    values.sort();
    values
}

/// Seeds the same types under R4 and R5 in `tenant`, then holds `runner`
/// to the version filter on every scan shape.
pub async fn runner_honors_fhir_version<S>(
    backend: &S,
    runner: &dyn SofRunner,
    tenant: &TenantContext,
) where
    S: ResourceStorage,
{
    let (r4, r5) = (FhirVersion::R4, FhirVersion::R5);
    for (id, version) in [("a4", r4), ("b4", r4), ("a5", r5)] {
        backend
            .create(
                tenant,
                "Patient",
                json!({"resourceType":"Patient","id":id,"gender":"female"}),
                version,
            )
            .await
            .expect("seed patient");
    }
    for (id, members, version) in [("g4", ["a4", "a5"], r4), ("g5", ["a4", "a5"], r5)] {
        let member: Vec<Value> = members
            .iter()
            .map(|m| json!({"entity":{"reference":format!("Patient/{m}")}}))
            .collect();
        backend
            .create(
                tenant,
                "Group",
                json!({"resourceType":"Group","id":id,"type":"person","actual":true,
                    "membership":"enumerated","member":member}),
                version,
            )
            .await
            .expect("seed group");
    }
    for (id, version) in [("qr4", r4), ("qr5", r5)] {
        backend
            .create(
                tenant,
                "QuestionnaireResponse",
                json!({"resourceType":"QuestionnaireResponse","id":id,"status":"completed",
                    "item":[{"linkId":format!("{id}-1"),
                        "item":[{"linkId":format!("{id}-1.1")}]}]}),
                version,
            )
            .await
            .expect("seed questionnaire response");
    }

    // ---- Flat scan -------------------------------------------------------
    let flat = json!({"resourceType":"ViewDefinition","resource":"Patient","status":"active",
        "select":[{"column":[{"path":"id","name":"id"}]}]});
    assert_eq!(
        column(runner, tenant, &flat, filters(None), "id").await,
        strings(&["a4", "a5", "b4"]),
        "version-blind flat run"
    );
    assert_eq!(
        column(runner, tenant, &flat, filters(Some(r4)), "id").await,
        strings(&["a4", "b4"]),
        "R4 flat run"
    );
    assert_eq!(
        column(runner, tenant, &flat, filters(Some(r5)), "id").await,
        strings(&["a5"]),
        "R5 flat run"
    );

    // ---- Every unionAll branch ------------------------------------------
    let union = json!({"resourceType":"ViewDefinition","resource":"Patient","status":"active",
    "select":[{"unionAll":[
        {"column":[{"path":"id","name":"value"}]},
        {"column":[{"path":"gender","name":"value"}]}
    ]}]});
    assert_eq!(
        column(runner, tenant, &union, filters(Some(r5)), "value").await,
        strings(&["a5", "female"]),
        "R5 unionAll run"
    );
    assert_eq!(
        column(runner, tenant, &union, filters(None), "value")
            .await
            .len(),
        6,
        "version-blind unionAll run"
    );

    // ---- repeat seed and join-back --------------------------------------
    let repeat = json!({"resourceType":"ViewDefinition","resource":"QuestionnaireResponse",
        "status":"active",
        "select":[{"column":[{"path":"id","name":"qr"}]},
            {"repeat":["item"],"column":[{"path":"linkId","name":"link_id"}]}]});
    assert_eq!(
        column(runner, tenant, &repeat, filters(Some(r4)), "link_id").await,
        strings(&["qr4-1", "qr4-1.1"]),
        "R4 repeat run"
    );
    assert_eq!(
        column(runner, tenant, &repeat, filters(Some(r5)), "qr").await,
        strings(&["qr5", "qr5"]),
        "R5 repeat run"
    );
    assert_eq!(
        column(runner, tenant, &repeat, filters(None), "link_id")
            .await
            .len(),
        4,
        "version-blind repeat run"
    );

    // ---- patient and group ----------------------------------------------
    let mut by_patient = filters(Some(r5));
    by_patient.patient = vec!["Patient/a5".into()];
    assert_eq!(
        column(runner, tenant, &flat, by_patient, "id").await,
        strings(&["a5"]),
        "R5 run restricted to an R5 patient"
    );
    // a4 and g4 are stored under R4: an R5 run does not see them.
    let mut by_patient = filters(Some(r5));
    by_patient.patient = vec!["Patient/a4".into()];
    if let Ok(rows) = absent_target_run(runner, tenant, &flat, by_patient).await {
        assert!(rows.is_empty(), "an R5 run must not read an R4 Patient");
    }
    let mut by_group = filters(Some(r5));
    by_group.group = vec!["Group/g4".into()];
    if let Ok(rows) = absent_target_run(runner, tenant, &flat, by_group).await {
        assert!(rows.is_empty(), "an R5 run must not resolve an R4 Group");
    }
    let mut by_group = filters(Some(r5));
    by_group.group = vec!["Group/g5".into()];
    assert_eq!(
        column(runner, tenant, &flat, by_group, "id").await,
        strings(&["a5"]),
        "R5 run over an R5 Group reads R5 members only"
    );
    let mut by_group = filters(None);
    by_group.group = vec!["Group/g4".into()];
    assert_eq!(
        column(runner, tenant, &flat, by_group, "id").await,
        strings(&["a4", "a5"]),
        "version-blind group run"
    );

    // ---- Columnar path ---------------------------------------------------
    let mut counts = BTreeMap::new();
    for (label, version) in [("none", None), ("r4", Some(r4)), ("r5", Some(r5))] {
        let Some(mut batches) = runner
            .run_view_columnar(tenant, flat.clone(), filters(version))
            .await
            .expect("run_view_columnar")
        else {
            continue; // the runner has no columnar path for this view
        };
        let mut rows = 0;
        while let Some(batch) = batches.next().await {
            rows += batch.expect("batch must succeed").num_rows();
        }
        counts.insert(label, rows);
    }
    // Runners without a columnar path (MongoDB) leave `counts` empty.
    if !counts.is_empty() {
        assert_eq!(
            counts,
            BTreeMap::from([("none", 3), ("r4", 2), ("r5", 1)]),
            "columnar row counts"
        );
    }

    // ---- Export reader (paged on PostgreSQL) ----------------------------
    if let Some(export) = runner.for_export_job() {
        assert_eq!(
            column(export.as_ref(), tenant, &flat, filters(Some(r5)), "id").await,
            strings(&["a5"]),
            "R5 export run"
        );
        assert_eq!(
            column(export.as_ref(), tenant, &flat, filters(Some(r4)), "id").await,
            strings(&["a4", "b4"]),
            "R4 export run"
        );
    }
}
