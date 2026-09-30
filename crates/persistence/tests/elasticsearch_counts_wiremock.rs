//! The Elasticsearch backend's count aggregates — `count_all_types`,
//! `count_by_day`, `latest_write_marker` — against an in-process HTTP stub
//! (#1280). They are what the web UI's Home dashboard reads on a composite
//! whose primary keeps no counts (an S3 primary with Elasticsearch), so each
//! test asserts both the figures parsed and the request the backend sent:
//! the aggregation asked for, and the filters that keep deleted and
//! contained documents out of the totals.
//!
//! Run with:
//! `cargo test -p helios-persistence --features elasticsearch --test elasticsearch_counts_wiremock`

#![cfg(feature = "elasticsearch")]

use std::sync::{Arc, Mutex};

use chrono::{NaiveDate, TimeZone, Utc};
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::core::ResourceStorage;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TENANT_SEARCH_PATH: &str = "/hfs_count-stub_*/_search";
const PATIENT_SEARCH_PATH: &str = "/hfs_count-stub_patient/_search";

const INDEX_NOT_FOUND: &str = r#"{"error":{"root_cause":[{"type":"index_not_found_exception","reason":"no such index [hfs_count-stub_patient]","resource.type":"index_or_alias","resource.id":"hfs_count-stub_patient","index_uuid":"_na_","index":"hfs_count-stub_patient"}],"type":"index_not_found_exception","reason":"no such index [hfs_count-stub_patient]","resource.type":"index_or_alias","resource.id":"hfs_count-stub_patient","index_uuid":"_na_","index":"hfs_count-stub_patient"},"status":404}"#;

fn tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("count-stub"),
        TenantPermissions::full_access(),
    )
}

fn backend(server: &MockServer) -> ElasticsearchBackend {
    let config = ElasticsearchConfig {
        nodes: vec![server.uri()],
        request_timeout_ms: 2_000,
        ..Default::default()
    };
    ElasticsearchBackend::new(config).expect("client construction is lazy")
}

/// Answers `POST {url_path}` with `body`, recording every request body sent.
async fn answer(server: &MockServer, url_path: &str, body: Value) -> Arc<Mutex<Vec<Value>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Mock::given(method("POST"))
        .and(path(url_path))
        .respond_with(move |request: &Request| {
            let sent: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            record.lock().unwrap().push(sent);
            ResponseTemplate::new(200).set_body_json(body.clone())
        })
        .mount(server)
        .await;
    seen
}

fn filters_of(sent: &Value) -> Vec<Value> {
    sent.pointer("/query/bool/filter")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn excludes_contained(sent: &Value) -> bool {
    sent.pointer("/query/bool/must_not")
        .and_then(Value::as_array)
        .is_some_and(|m| m.contains(&json!({ "term": { "is_contained": true } })))
}

#[tokio::test]
async fn count_all_types_aggregates_live_resources_per_type_across_the_tenant() {
    let server = MockServer::start().await;
    let seen = answer(
        &server,
        TENANT_SEARCH_PATH,
        json!({
            "hits": { "total": { "value": 11846 } },
            "aggregations": { "types": { "buckets": [
                { "key": "Patient", "doc_count": 11704 },
                { "key": "Organization", "doc_count": 140 },
                { "key": "Group", "doc_count": 2 }
            ]}}
        }),
    )
    .await;
    let backend = backend(&server);

    assert!(backend.supports_type_counts());
    let mut counts = backend.count_all_types(&tenant()).await.unwrap();
    counts.sort();
    assert_eq!(
        counts,
        vec![
            ("Group".to_string(), 2),
            ("Organization".to_string(), 140),
            ("Patient".to_string(), 11704),
        ]
    );

    let sent = seen.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["size"], 0);
    assert_eq!(
        sent[0].pointer("/aggs/types/terms/field"),
        Some(&json!("resource_type"))
    );
    let filters = filters_of(&sent[0]);
    assert!(filters.contains(&json!({ "term": { "tenant_id": "count-stub" } })));
    assert!(filters.contains(&json!({ "term": { "is_deleted": false } })));
    assert!(excludes_contained(&sent[0]));
}

#[tokio::test]
async fn a_tenant_with_no_indices_has_no_types_and_a_broken_answer_is_an_error() {
    let server = MockServer::start().await;
    let backend = backend(&server);

    Mock::given(method("POST"))
        .and(path(TENANT_SEARCH_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_string(INDEX_NOT_FOUND))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(backend.count_all_types(&tenant()).await.unwrap(), vec![]);
    server.reset().await;

    // A 200 without the aggregation is not "no resources".
    answer(
        &server,
        TENANT_SEARCH_PATH,
        json!({ "hits": { "total": { "value": 3 } } }),
    )
    .await;
    assert!(backend.count_all_types(&tenant()).await.is_err());
}

#[tokio::test]
async fn count_by_day_buckets_one_type_by_utc_day_since_the_bound() {
    let server = MockServer::start().await;
    let seen = answer(
        &server,
        PATIENT_SEARCH_PATH,
        json!({
            "aggregations": { "days": { "buckets": [
                { "key_as_string": "2026-09-23T00:00:00.000Z", "key": 1790121600000u64, "doc_count": 5 },
                { "key_as_string": "2026-09-25T00:00:00.000Z", "key": 1790294400000u64, "doc_count": 11699 }
            ]}}
        }),
    )
    .await;
    let backend = backend(&server);
    let since = Utc.with_ymd_and_hms(2026, 9, 20, 0, 0, 0).unwrap();

    let days = backend
        .count_by_day(&tenant(), "Patient", since)
        .await
        .unwrap();
    assert_eq!(days.len(), 2);
    assert_eq!(days[0].day, NaiveDate::from_ymd_opt(2026, 9, 23).unwrap());
    assert_eq!(days[0].count, 5);
    assert_eq!(days[1].day, NaiveDate::from_ymd_opt(2026, 9, 25).unwrap());
    assert_eq!(days[1].count, 11699);

    let sent = seen.lock().unwrap();
    assert_eq!(
        sent[0].pointer("/aggs/days/date_histogram/calendar_interval"),
        Some(&json!("day"))
    );
    let filters = filters_of(&sent[0]);
    assert!(filters.contains(&json!({ "term": { "is_deleted": false } })));
    assert!(
        filters
            .iter()
            .any(|f| f.pointer("/range/last_updated/gte").is_some()),
        "{filters:?}"
    );
    assert!(excludes_contained(&sent[0]));
}

#[tokio::test]
async fn the_write_marker_is_the_newest_last_updated_plus_the_recent_count_when_asked() {
    let server = MockServer::start().await;
    let seen = answer(
        &server,
        TENANT_SEARCH_PATH,
        json!({
            "aggregations": {
                "latest": { "value": 1790330400000u64, "value_as_string": "2026-09-25T10:00:00.000Z" },
                "recent": { "doc_count": 42 }
            }
        }),
    )
    .await;
    let backend = backend(&server);
    let since = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();

    let marker = backend
        .latest_write_marker(&tenant(), Some(since))
        .await
        .unwrap()
        .expect("the search backend has a marker");
    assert_eq!(
        marker.latest,
        Some(Utc.with_ymd_and_hms(2026, 9, 25, 10, 0, 0).unwrap())
    );
    assert_eq!(marker.recent_writes, Some(42));
    {
        let sent = seen.lock().unwrap();
        assert_eq!(
            sent[0].pointer("/aggs/latest/max/field"),
            Some(&json!("last_updated"))
        );
        assert!(
            sent[0]
                .pointer("/aggs/recent/filter/range/last_updated/gte")
                .is_some()
        );
    }

    // Without a bound there is no recent count, and an empty tenant has no
    // newest write.
    server.reset().await;
    answer(
        &server,
        TENANT_SEARCH_PATH,
        json!({ "aggregations": { "latest": { "value": null } } }),
    )
    .await;
    let marker = backend
        .latest_write_marker(&tenant(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(marker.latest, None);
    assert_eq!(marker.recent_writes, None);
}
