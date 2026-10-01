//! PostgreSQL patient + composite search through the HTTP parser and pagination.
#![cfg(feature = "postgres")]

use std::path::PathBuf;

use axum_test::TestServer;
use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
use helios_rest::{ServerConfig, create_app_with_config};
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;

#[path = "common/container_cleanup.rs"]
mod container_cleanup;

struct SharedPg {
    host: String,
    port: u16,
    _container: testcontainers::ContainerAsync<Postgres>,
}

static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();

async fn server() -> TestServer {
    let pg = SHARED_PG
        .get_or_init(|| async {
            let container =
                container_cleanup::with_cleanup_label(Postgres::default().with_tag("16-alpine"))
                    .start()
                    .await
                    .unwrap();
            SharedPg {
                host: container.get_host().await.unwrap().to_string(),
                port: container.get_host_port_ipv4(5432).await.unwrap(),
                _container: container,
            }
        })
        .await;
    let backend = PostgresBackend::new(PostgresConfig {
        host: pg.host.clone(),
        port: pg.port,
        dbname: "postgres".into(),
        user: "postgres".into(),
        password: Some("postgres".into()),
        max_connections: 5,
        data_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")),
        ..Default::default()
    })
    .await
    .unwrap();
    backend.init_schema().await.unwrap();
    let config = ServerConfig {
        default_tenant: format!("selective_1579_{}", uuid::Uuid::new_v4().simple()),
        ..ServerConfig::for_testing()
    };
    TestServer::new(create_app_with_config(backend, config)).unwrap()
}

fn ids(bundle: &Value) -> Vec<String> {
    bundle["entry"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .map(|e| e["resource"]["id"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn link_path(bundle: &Value, relation: &str) -> String {
    let url = bundle["link"]
        .as_array()
        .unwrap()
        .iter()
        .find(|link| link["relation"] == relation)
        .unwrap()["url"]
        .as_str()
        .unwrap();
    let url = url::Url::parse(url).unwrap();
    format!("{}?{}", url.path(), url.query().unwrap())
}

#[tokio::test]
async fn postgres_1579_patient_composite_http_literal_total_and_pagination() {
    let server = server().await;
    for (id, patient, value, system) in [
        ("a", "anchor", 164.1, "http://loinc.org"),
        ("b", "anchor", 164.1, "http://loinc.org"),
        ("c", "anchor", 164.1, "http://loinc.org"),
        ("boundary", "anchor", 160.0, "http://loinc.org"),
        ("wrong-system", "anchor", 170.0, "http://wrong.example"),
        ("wrong-patient", "other", 170.0, "http://loinc.org"),
        ("deleted", "anchor", 170.0, "http://loinc.org"),
    ] {
        let resource = json!({"resourceType": "Observation", "id": id, "status": "final",
            "code": {"coding": [{"system": system, "code": "8302-2"}]},
            "subject": {"reference": format!("Patient/{patient}")},
            "valueQuantity": {"value": value, "unit": "cm", "system": "http://unitsofmeasure.org", "code": "cm"}});
        let response = server
            .put(&format!("/Observation/{id}"))
            .json(&resource)
            .await;
        assert!(response.status_code().is_success(), "{}", response.text());
    }
    assert!(
        server
            .delete("/Observation/deleted")
            .await
            .status_code()
            .is_success()
    );
    for total in ["none", "accurate"] {
        let response = server
            .get("/Observation")
            .add_query_param("patient", "anchor")
            .add_query_param("code-value-quantity", "http://loinc.org|8302-2$gt160")
            .add_query_param("_total", total)
            .await;
        assert_eq!(
            response.status_code(),
            axum::http::StatusCode::OK,
            "{}",
            response.text()
        );
        let bundle: Value = response.json();
        let mut found = ids(&bundle);
        found.sort();
        assert_eq!(found, ["a", "b", "c"]);
        assert_eq!(
            bundle.get("total").cloned(),
            if total == "accurate" {
                Some(json!(3))
            } else {
                None
            }
        );
    }
    let first: Value = server
        .get("/Observation")
        .add_query_param("patient", "anchor")
        .add_query_param("code-value-quantity", "http://loinc.org|8302-2$gt160")
        .add_query_param("_total", "accurate")
        .add_query_param("_count", "1")
        .await
        .json();
    let second: Value = server.get(&link_path(&first, "next")).await.json();
    let third: Value = server.get(&link_path(&second, "next")).await.json();
    let mut found = [ids(&first), ids(&second), ids(&third)].concat();
    found.sort();
    assert_eq!(found, ["a", "b", "c"]);
    assert_eq!(first["total"], 3);
    assert_eq!(second["total"], 3);
    assert_eq!(third["total"], 3);
    let previous: Value = server.get(&link_path(&second, "previous")).await.json();
    assert_eq!(ids(&previous), ids(&first));
    let offset: Value = server
        .get("/Observation")
        .add_query_param("patient", "anchor")
        .add_query_param("code-value-quantity", "http://loinc.org|8302-2$gt160")
        .add_query_param("_count", "1")
        .add_query_param("_offset", "1")
        .await
        .json();
    assert_eq!(ids(&offset), ids(&second));
}
