//! `sof-server` loads a `$sql-run` `source` only under a URL prefix its
//! operator lists in `SOF_SERVER_SOURCE_PREFIXES`. It used to load any
//! `file:`, `http(s):` or cloud URL a caller named, so an unauthenticated
//! caller could read the server's files or make it fetch internal URLs.

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_sof::app::{ServerConfig, create_app_with_config};
use helios_sof::data_source::SourcePolicy;
use serde_json::{Value, json};
use url::Url;

fn server(prefixes: &str) -> TestServer {
    server_with(SourcePolicy::parse(prefixes).expect("prefixes"))
}

fn server_with(source_prefixes: SourcePolicy) -> TestServer {
    let config = ServerConfig {
        source_prefixes,
        ..ServerConfig::default()
    };
    TestServer::new(create_app_with_config(&config)).expect("test server")
}

fn run_with_source(source: &str) -> Value {
    json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "_format", "valueCode": "json"},
            {"name": "source", "valueString": source},
            {"name": "subjectResource", "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{"column": [{"name": "id", "path": "id"}]}]
            }}
        ]
    })
}

fn file_url(path: &std::path::Path) -> String {
    Url::from_file_path(path).expect("file url").to_string()
}

/// A scratch directory with `allowed/patient.json` and `outside.json`.
fn scratch() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("allowed")).unwrap();
    let patient = json!({"resourceType": "Patient", "id": "p1"}).to_string();
    std::fs::write(dir.path().join("allowed/patient.json"), &patient).unwrap();
    std::fs::write(dir.path().join("outside.json"), &patient).unwrap();
    dir
}

#[tokio::test]
async fn without_prefixes_no_source_is_loaded() {
    let dir = scratch();
    let source = file_url(&dir.path().join("allowed/patient.json"));
    let response = server("")
        .post("/$sql-run")
        .json(&run_with_source(&source))
        .await;
    assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);
    assert!(response.text().contains("SOF_SERVER_SOURCE_PREFIXES"));
}

#[tokio::test]
async fn a_file_under_a_prefix_is_loaded_and_others_are_refused() {
    let dir = scratch();
    let prefix = file_url(&dir.path().join("allowed")) + "/";
    let server = server(&prefix);

    let inside = server
        .post("/$sql-run")
        .json(&run_with_source(&file_url(
            &dir.path().join("allowed/patient.json"),
        )))
        .await;
    assert_eq!(inside.status_code(), StatusCode::OK, "{}", inside.text());
    assert!(inside.text().contains("p1"), "{}", inside.text());

    let escaping = format!("{prefix}../outside.json");
    for source in [
        file_url(&dir.path().join("outside.json")),
        escaping,
        file_url(&dir.path().join("allowed/missing.json")),
        "https://example.org/allowed/patient.json".to_string(),
    ] {
        let response = server
            .post("/$sql-run")
            .json(&run_with_source(&source))
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::BAD_REQUEST,
            "{source}: {}",
            response.text()
        );
        assert!(
            !response.text().contains(&*dir.path().to_string_lossy()),
            "the refusal names no server path: {}",
            response.text()
        );
    }
}

#[test]
fn prefixes_match_scheme_host_port_and_whole_path_segments() {
    let policy =
        SourcePolicy::parse("https://data.example.org/exports/, s3://bucket/fhir").unwrap();
    let admits = |url: &str| policy.admits(&Url::parse(url).unwrap());
    assert!(admits("https://data.example.org/exports/a.ndjson"));
    assert!(admits("https://data.example.org:443/exports/a.ndjson"));
    assert!(admits("s3://bucket/fhir/a.ndjson"));
    assert!(!admits("http://data.example.org/exports/a.ndjson"));
    assert!(!admits("https://data.example.org:8443/exports/a.ndjson"));
    assert!(!admits(
        "https://data.example.org.evil.test/exports/a.ndjson"
    ));
    assert!(!admits("https://data.example.org/exports-old/a.ndjson"));
    assert!(!admits("https://data.example.org/exports/../secret"));
    assert!(!admits("s3://bucket/fhirdata/a.ndjson"));
    assert!(!admits("s3://other/fhir/a.ndjson"));
    assert!(!admits("http://169.254.169.254/latest/meta-data/"));
}

/// An `abfss` URL names its container in the user info, so a prefix for one
/// container admits no other; and a caller cannot add credentials to a URL.
#[test]
fn prefixes_match_user_info() {
    let policy = SourcePolicy::parse(
        "abfss://c1@acct.dfs.core.windows.net/data/, https://data.example.org/exports/",
    )
    .unwrap();
    let admits = |url: &str| policy.admits(&Url::parse(url).unwrap());
    assert!(admits("abfss://c1@acct.dfs.core.windows.net/data/a.json"));
    assert!(!admits("abfss://c2@acct.dfs.core.windows.net/data/a.json"));
    assert!(!admits("abfss://acct.dfs.core.windows.net/data/a.json"));
    assert!(!admits("https://user:pw@data.example.org/exports/a.json"));
    assert!(!admits("https://user@data.example.org/exports/a.json"));
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_under_a_prefix_to_a_file_outside_it_is_refused() {
    let dir = scratch();
    std::os::unix::fs::symlink(
        dir.path().join("outside.json"),
        dir.path().join("allowed/link.json"),
    )
    .unwrap();
    let server = server(&(file_url(&dir.path().join("allowed")) + "/"));
    let response = server
        .post("/$sql-run")
        .json(&run_with_source(&file_url(
            &dir.path().join("allowed/link.json"),
        )))
        .await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    assert!(!response.text().contains("p1"), "{}", response.text());
}

#[tokio::test]
async fn a_file_over_the_size_limit_is_refused() {
    let dir = scratch();
    let patient = std::fs::read(dir.path().join("allowed/patient.json")).unwrap();
    let prefix = file_url(&dir.path().join("allowed")) + "/";
    let source = run_with_source(&file_url(&dir.path().join("allowed/patient.json")));
    let policy = |max| SourcePolicy::parse(&prefix).unwrap().with_max_bytes(max);

    let at_limit = server_with(policy(patient.len() as u64))
        .post("/$sql-run")
        .json(&source)
        .await;
    assert_eq!(
        at_limit.status_code(),
        StatusCode::OK,
        "{}",
        at_limit.text()
    );

    let over = server_with(policy(patient.len() as u64 - 1))
        .post("/$sql-run")
        .json(&source)
        .await;
    assert_eq!(over.status_code(), StatusCode::BAD_REQUEST);
    assert!(over.text().contains("SOF_SERVER_SOURCE_MAX_BYTES"));

    let unlimited = server_with(policy(0)).post("/$sql-run").json(&source).await;
    assert_eq!(unlimited.status_code(), StatusCode::OK);
}

/// An upstream on a loopback port: `/sized` declares its length, `/chunked`
/// streams the same body with none.
async fn upstream(body: &'static str) -> String {
    use axum::{Router, body::Body, routing::get};
    let app = Router::new()
        .route("/sized", get(move || async move { body }))
        .route(
            "/chunked",
            get(move || async move {
                let halves = body.split_at(body.len() / 2);
                Body::from_stream(futures::stream::iter(
                    [halves.0, halves.1].map(Ok::<_, std::io::Error>),
                ))
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

#[tokio::test]
async fn an_http_body_over_the_size_limit_is_refused_with_or_without_a_length() {
    const BODY: &str = r#"{"resourceType":"Patient","id":"p1"}"#;
    let base = upstream(BODY).await;
    let policy = |max| SourcePolicy::parse(&base).unwrap().with_max_bytes(max);

    for path in ["sized", "chunked"] {
        let source = run_with_source(&format!("{base}{path}"));
        let at_limit = server_with(policy(BODY.len() as u64))
            .post("/$sql-run")
            .json(&source)
            .await;
        assert_eq!(
            at_limit.status_code(),
            StatusCode::OK,
            "{path}: {}",
            at_limit.text()
        );

        let over = server_with(policy(BODY.len() as u64 - 1))
            .post("/$sql-run")
            .json(&source)
            .await;
        assert_eq!(over.status_code(), StatusCode::BAD_REQUEST, "{path}");
        assert!(
            over.text().contains("SOF_SERVER_SOURCE_MAX_BYTES"),
            "{path}"
        );
    }
}
