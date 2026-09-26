//! Router-level tests for `Patient/$everything` over SQLite in-memory.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};

use common::everything::*;

#[tokio::test]
async fn instance_level_returns_patient_members_and_supporting_resources() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let resp = server.get("/Patient/p1/$everything").await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
    let b: Value = resp.json();
    assert_eq!(b["resourceType"], "Bundle");
    assert_eq!(b["type"], "searchset");
    let m = entries(&b, "match");
    assert_eq!(m[0], "Patient/p1", "Patient is first");
    for id in [
        "Observation/o1",
        "Observation/o2",
        "Observation/o3",
        "Encounter/e1",
        "Encounter/e2",
        "Condition/c1",
    ] {
        assert!(m.contains(&id.to_string()), "missing {id} in {m:?}");
    }
    assert!(
        !m.iter()
            .any(|x| x == "Observation/other" || x == "Patient/p2")
    );
    let inc = entries(&b, "include");
    assert!(inc.contains(&"Practitioner/dr1".to_string()), "{inc:?}");
    assert!(inc.contains(&"Organization/org1".to_string()), "{inc:?}");
    assert_eq!(
        b["total"].as_u64(),
        Some(m.len() as u64),
        "unpaged total is exact"
    );
    assert!(next_link(&b).is_none());
}

#[tokio::test]
async fn deleted_supporting_resource_is_skipped_not_410() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let del = server.delete("/Practitioner/dr1").await;
    assert!(del.status_code().is_success(), "{}", del.text());
    let resp = server.get("/Patient/p1/$everything").await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
    let b: Value = resp.json();
    let inc = entries(&b, "include");
    assert!(
        !inc.contains(&"Practitioner/dr1".to_string()),
        "deleted supporting resource must be skipped: {inc:?}"
    );
    assert!(
        inc.contains(&"Organization/org1".to_string()),
        "live supporting resource must still be included: {inc:?}"
    );
}

#[tokio::test]
async fn type_filter_restricts_members_but_keeps_patient() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let b: Value = server
        .get("/Patient/p1/$everything?_type=Encounter")
        .await
        .json();
    let m = entries(&b, "match");
    assert_eq!(m[0], "Patient/p1");
    assert_eq!(m.len(), 3, "{m:?}");
    assert!(m.iter().skip(1).all(|x| x.starts_with("Encounter/")));
}

#[tokio::test]
async fn since_and_clinical_dates_filter_members_only() {
    let server = server_with(10_000).await;
    seed(&server).await;

    let b: Value = server
        .get("/Patient/p1/$everything?start=2020-01-01&end=2020-12-31")
        .await
        .json();
    let m = entries(&b, "match");
    assert!(m.contains(&"Patient/p1".to_string()));
    assert!(m.contains(&"Observation/o2".to_string()));
    assert!(!m.contains(&"Observation/o1".to_string()));
    assert!(!m.contains(&"Observation/o3".to_string()));
    assert!(
        m.contains(&"Condition/c1".to_string()),
        "onset-date 2020-01-15 in range"
    );
    // Both encounters have a `period` with a start but no end, so each is an
    // ongoing range with an unbounded end (#1391). e1 began in 2019 and is
    // still running through 2020; e2 only begins in 2021.
    assert!(
        m.contains(&"Encounter/e1".to_string()),
        "open-ended period from 2019-06-01 overlaps 2020"
    );
    assert!(
        !m.contains(&"Encounter/e2".to_string()),
        "period starting 2021-06-01 is after 2020"
    );

    // _since: everything was created "now", so a far-future instant excludes all members.
    let b: Value = server
        .get("/Patient/p1/$everything?_since=2999-01-01T00:00:00Z")
        .await
        .json();
    assert_eq!(entries(&b, "match"), vec!["Patient/p1"]);
    let b: Value = server
        .get("/Patient/p1/$everything?_since=2000-01-01T00:00:00Z")
        .await
        .json();
    assert!(entries(&b, "match").len() > 1);
}

#[tokio::test]
async fn paging_walks_to_exhaustion_without_gaps_or_duplicates() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let (unpaged, _) = walk(&server, "/Patient/p1/$everything").await;
    let (paged, pages) = walk(&server, "/Patient/p1/$everything?_count=2").await;
    assert!(
        pages.len() >= 4,
        "expected several pages, got {}",
        pages.len()
    );
    let last = pages.len() - 1;
    let mut any_include = false;
    for (i, p) in pages.iter().enumerate() {
        assert!(p["total"].is_null(), "paged responses omit total");
        let m = entries(p, "match").len();
        if i == last {
            assert!(m <= 2);
        } else {
            assert_eq!(
                m, 2,
                "non-last page must be filled to _count=2; includes must not count against it"
            );
        }
        if !entries(p, "include").is_empty() {
            any_include = true;
        }
    }
    assert!(
        any_include,
        "expected at least one page to carry include entries (Patient/p1's managingOrganization -> Organization/org1)"
    );
    let mut sorted_a = unpaged.clone();
    sorted_a.sort();
    let mut sorted_b = paged.clone();
    sorted_b.sort();
    assert_eq!(sorted_a, sorted_b);
    assert_eq!(paged.len(), unpaged.len(), "no duplicates: {paged:?}");
}

#[tokio::test]
async fn cursor_from_a_different_request_is_rejected() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let b: Value = server.get("/Patient/p1/$everything?_count=2").await.json();
    let next = path_of(&next_link(&b).unwrap());
    let tampered = next.replace("_count=2", "_count=3");
    let resp = server.get(&tampered).await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.json::<Value>()["resourceType"], "OperationOutcome");
}

#[tokio::test]
async fn unpaged_ceiling_switches_to_paging_with_information_outcome() {
    let server = server_with(4).await;
    seed(&server).await;
    let b: Value = server.get("/Patient/p1/$everything").await.json();
    assert_eq!(entries(&b, "match").len(), 4);
    assert!(next_link(&b).is_some());
    let outcome = b["entry"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["resource"]["resourceType"] == "OperationOutcome")
        .expect("information outcome present");
    assert_eq!(outcome["resource"]["issue"][0]["severity"], "information");
    assert_eq!(outcome["search"]["mode"], "outcome");
    let (all, _) = walk(&server, "/Patient/p1/$everything").await;
    assert_eq!(all.len(), 7);
}

#[tokio::test]
async fn type_level_walks_every_patient_and_resumes_mid_patient() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let (all, _) = walk(&server, "/Patient/$everything?_count=3").await;
    assert!(all.contains(&"Patient/p1".to_string()));
    assert!(all.contains(&"Patient/p2".to_string()));
    assert!(all.contains(&"Observation/other".to_string()));
    assert!(all.contains(&"Observation/o1".to_string()));
    assert_eq!(all.len(), 9, "{all:?}");
    let mut s = all.clone();
    s.sort();
    s.dedup();
    assert_eq!(s.len(), all.len(), "no duplicates across patients");
}

#[tokio::test]
async fn post_parameters_matches_get() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let get: Value = server
        .get("/Patient/p1/$everything?_type=Observation&start=2020-01-01")
        .await
        .json();
    let post: Value = server
        .post("/Patient/p1/$everything")
        .json(&json!({
            "resourceType": "Parameters",
            "parameter": [
                {"name": "_type", "valueCode": "Observation"},
                {"name": "start", "valueDate": "2020-01-01"}
            ]
        }))
        .await
        .json();
    assert_eq!(entries(&get, "match"), entries(&post, "match"));
}

#[tokio::test]
async fn errors() {
    let server = server_with(10_000).await;
    seed(&server).await;
    assert_eq!(
        server.get("/Patient/nope/$everything").await.status_code(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        server
            .get("/Patient/p1/$everything?_type=Bogus")
            .await
            .status_code(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        server
            .get("/Patient/p1/$everything?_include=x")
            .await
            .status_code(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        server
            .get("/Patient/p1/$everything?_since=2020")
            .await
            .status_code(),
        StatusCode::BAD_REQUEST
    );
    let resp = server
        .post("/Patient/p1/$everything")
        .json(&json!({"resourceType": "Patient"}))
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::BAD_REQUEST,
        "POST body must be Parameters"
    );
}

#[tokio::test]
async fn deleted_patient_yields_410() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let del = server.delete("/Patient/p2").await;
    assert!(del.status_code().is_success(), "{}", del.text());
    let resp = server.get("/Patient/p2/$everything").await;
    assert_eq!(resp.status_code(), StatusCode::GONE, "{}", resp.text());
    let body: Value = resp.json();
    assert_eq!(body["resourceType"], "OperationOutcome");
}

#[tokio::test]
async fn capability_statement_declares_everything_on_patient() {
    let server = server_with(10_000).await;
    let b: Value = server.get("/metadata").await.json();
    let patient = b["rest"][0]["resource"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type"] == "Patient")
        .expect("Patient resource entry");
    let ops = patient["operation"].as_array().expect("operation array");
    assert!(
        ops.iter().any(|o| o["name"] == "everything"
            && o["definition"] == "http://hl7.org/fhir/OperationDefinition/Patient-everything"),
        "{ops:?}"
    );
}
