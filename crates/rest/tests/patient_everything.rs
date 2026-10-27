//! Router-level tests for `Patient/$everything` over SQLite in-memory.

mod common;

use axum::http::StatusCode;
use helios_auth::{LaunchContext, Principal, ScopeSet};
use serde_json::{Value, json};

use common::everything::*;

/// A SMART app token: patient-context scopes plus the given launch context. The scopes are
/// inert in these tests: `server_with_principal` injects the `Principal` after
/// authentication, so `authz_middleware` never evaluates them, and narrowing does not
/// depend on the scope type.
fn smart_principal(context: Option<LaunchContext>) -> Principal {
    Principal {
        subject: "smart-app".to_string(),
        issuer: "https://idp.example.com".to_string(),
        scopes: ScopeSet::parse("patient/*.rs"),
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        launch_context: context,
        ..Default::default()
    }
}

fn patient_context(p: &str) -> Option<LaunchContext> {
    Some(LaunchContext {
        patient: Some(p.into()),
        ..Default::default()
    })
}

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

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[tokio::test]
async fn type_level_with_patient_launch_context_returns_only_that_patient() {
    let server = server_with_principal(10_000, smart_principal(patient_context("p1"))).await;
    seed(&server).await;
    let resp = server.get("/Patient/$everything").await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
    let b: Value = resp.json();
    let m = entries(&b, "match");
    assert_eq!(m[0], "Patient/p1");
    assert!(!m.contains(&"Patient/p2".to_string()), "{m:?}");
    assert!(!m.contains(&"Observation/other".to_string()), "{m:?}");
    assert_eq!(b["total"].as_u64(), Some(m.len() as u64));

    let instance: Value = server.get("/Patient/p1/$everything").await.json();
    assert_eq!(sorted(m), sorted(entries(&instance, "match")));
    assert_eq!(
        sorted(entries(&b, "include")),
        sorted(entries(&instance, "include"))
    );

    let self_url = b["link"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["relation"] == "self")
        .expect("self link")["url"]
        .as_str()
        .unwrap();
    let self_path = path_of(self_url);
    assert!(
        self_path.starts_with("/Patient/$everything")
            || (self_path.starts_with("/Patient/%24everything")),
        "self link still echoes the type-level request: {self_path}"
    );
    assert!(!self_path.starts_with("/Patient/p1"), "{self_path}");
}

#[tokio::test]
async fn type_level_launch_context_accepts_a_patient_reference_and_pages_within_it() {
    let server =
        server_with_principal(10_000, smart_principal(patient_context("Patient/p1"))).await;
    seed(&server).await;
    let (paged, pages) = walk(&server, "/Patient/$everything?_count=2").await;
    let (instance, _) = walk(&server, "/Patient/p1/$everything").await;
    assert_eq!(paged.len(), instance.len(), "no duplicates: {paged:?}");
    assert_eq!(sorted(paged.clone()), sorted(instance));
    assert!(pages.len() > 1, "expected several pages");
    for page in &pages {
        if let Some(next) = next_link(page) {
            let path = path_of(&next);
            assert!(
                path.starts_with("/Patient/$everything")
                    || path.starts_with("/Patient/%24everything"),
                "next link stays type-level: {path}"
            );
        }
    }
    assert!(!paged.contains(&"Patient/p2".to_string()), "{paged:?}");
    assert!(
        !paged.contains(&"Observation/other".to_string()),
        "{paged:?}"
    );
}

#[tokio::test]
async fn type_level_without_usable_launch_context_walks_every_patient() {
    let principals = [
        smart_principal(None),
        smart_principal(Some(LaunchContext {
            encounter: Some("e1".into()),
            ..Default::default()
        })),
        smart_principal(patient_context("Group/g1")),
        smart_principal(patient_context("https://other.example/fhir/Patient/p1")),
    ];
    for principal in principals {
        let label = format!("{:?}", principal.launch_context);
        let server = server_with_principal(10_000, principal).await;
        seed(&server).await;
        let (all, _) = walk(&server, "/Patient/$everything?_count=3").await;
        assert_eq!(all.len(), 9, "{label}: {all:?}");
        assert!(all.contains(&"Patient/p2".to_string()), "{label}");
        assert!(all.contains(&"Observation/other".to_string()), "{label}");
    }
}

#[tokio::test]
async fn type_level_launch_patient_missing_or_deleted_answers_like_instance_level() {
    let server = server_with_principal(10_000, smart_principal(patient_context("nope"))).await;
    seed(&server).await;
    let type_level = server.get("/Patient/$everything").await;
    let instance = server.get("/Patient/nope/$everything").await;
    assert_eq!(type_level.status_code(), instance.status_code());
    assert_eq!(type_level.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(
        type_level.json::<Value>()["resourceType"],
        "OperationOutcome"
    );

    let server = server_with_principal(10_000, smart_principal(patient_context("p2"))).await;
    seed(&server).await;
    let del = server.delete("/Patient/p2").await;
    assert!(del.status_code().is_success(), "{}", del.text());
    let type_level = server.get("/Patient/$everything").await;
    let instance = server.get("/Patient/p2/$everything").await;
    assert_eq!(type_level.status_code(), StatusCode::GONE);
    assert_eq!(instance.status_code(), StatusCode::GONE);
    assert_eq!(
        type_level.json::<Value>()["resourceType"],
        "OperationOutcome"
    );
}

#[tokio::test]
async fn type_level_post_with_patient_launch_context_is_narrowed() {
    let server = server_with_principal(10_000, smart_principal(patient_context("p1"))).await;
    seed(&server).await;
    let resp = server
        .post("/Patient/$everything")
        .json(&json!({"resourceType": "Parameters"}))
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
    let post: Value = resp.json();
    let instance: Value = server.get("/Patient/p1/$everything").await.json();
    let matches = entries(&post, "match");
    assert!(!matches.contains(&"Patient/p2".to_string()), "{matches:?}");
    assert_eq!(sorted(matches), sorted(entries(&instance, "match")));
}

#[tokio::test]
async fn type_level_cursor_does_not_cross_launch_context() {
    let narrowed = server_with_principal(10_000, smart_principal(patient_context("p1"))).await;
    let plain = server_with(10_000).await;
    seed(&narrowed).await;
    seed(&plain).await;

    let narrowed_page: Value = narrowed.get("/Patient/$everything?_count=2").await.json();
    let plain_page: Value = plain.get("/Patient/$everything?_count=2").await.json();
    let narrowed_next = path_of(&next_link(&narrowed_page).expect("narrowed next link"));
    let plain_next = path_of(&next_link(&plain_page).expect("plain next link"));

    // The cursor fingerprint is an unkeyed hash of params+patient+tenant+version, so a
    // cursor replayed on the other server is otherwise valid.
    for (label, resp) in [
        ("narrowed cursor on plain", plain.get(&narrowed_next).await),
        ("plain cursor on narrowed", narrowed.get(&plain_next).await),
    ] {
        assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST, "{label}");
        assert!(
            resp.text().contains("issued for a different request"),
            "{label}: {}",
            resp.text()
        );
    }
}
