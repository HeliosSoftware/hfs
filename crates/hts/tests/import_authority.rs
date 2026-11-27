//! An import that does not own a canonical cannot replace the content of a
//! row that an owning package wrote for the same `(url, version)`.

mod common;

use std::sync::Arc;

use helios_hts::import::{BundleImportBackend, bundle_parser};
use helios_hts::traits::TerminologyBackend;
use helios_persistence::tenant::TenantContext;
use serde_json::{Value, json};

const OWNER_BASE: &str = "http://owner.example/fhir";

fn code_system(title: &str, code: &str) -> Value {
    json!({
        "resourceType": "CodeSystem",
        "id": "owned-cs",
        "url": format!("{OWNER_BASE}/CodeSystem/owned"),
        "version": "1.0.0",
        "status": "active",
        "content": "complete",
        "title": title,
        "concept": [{"code": code, "display": code}]
    })
}

fn value_set(title: &str, code: &str) -> Value {
    json!({
        "resourceType": "ValueSet",
        "id": "owned-vs",
        "url": format!("{OWNER_BASE}/ValueSet/owned"),
        "version": "1.0.0",
        "status": "active",
        "title": title,
        "compose": {"include": [{
            "system": format!("{OWNER_BASE}/CodeSystem/owned"),
            "concept": [{"code": code}]
        }]}
    })
}

fn bundle(resources: &[Value]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": resources.iter().map(|r| json!({"resource": r})).collect::<Vec<_>>()
    }))
    .unwrap()
}

async fn import_as(
    importer: &Arc<dyn BundleImportBackend>,
    resources: &[Value],
    package_canonical: Option<&str>,
) -> Vec<String> {
    let parsed =
        bundle_parser::parse_bundle_from_package(&bundle(resources), package_canonical).unwrap();
    importer
        .import_parsed(&TenantContext::system(), parsed)
        .await
        .unwrap()
        .errors
}

async fn authority_contract<B>(backend: B)
where
    B: TerminologyBackend + BundleImportBackend + Clone + 'static,
{
    let importer: Arc<dyn BundleImportBackend> = Arc::new(backend.clone());

    let owned = [
        code_system("Owner title", "a"),
        value_set("Owner title", "a"),
    ];
    assert!(
        import_as(&importer, &owned, Some(OWNER_BASE))
            .await
            .is_empty()
    );

    let errors = import_as(
        &importer,
        &[code_system("Replaced", "b"), value_set("Replaced", "b")],
        None,
    )
    .await;
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(
        errors.iter().all(|e| e.contains("authoritative source")),
        "{errors:?}"
    );
    for (kind, id) in [("CodeSystem", "owned-cs"), ("ValueSet", "owned-vs")] {
        let stored = backend.resource_by_id(kind, id).unwrap().unwrap();
        assert_eq!(stored["title"], "Owner title", "{kind} was replaced");
    }

    let errors = import_as(
        &importer,
        &[
            code_system("Owner update", "c"),
            value_set("Owner update", "c"),
        ],
        Some(OWNER_BASE),
    )
    .await;
    assert!(errors.is_empty(), "{errors:?}");
    for (kind, id) in [("CodeSystem", "owned-cs"), ("ValueSet", "owned-vs")] {
        let stored = backend.resource_by_id(kind, id).unwrap().unwrap();
        assert_eq!(stored["title"], "Owner update", "{kind} not updated");
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_unattributed_import_cannot_replace_owned_content() {
    authority_contract(helios_hts::backends::SqliteTerminologyBackend::in_memory().unwrap()).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_unattributed_import_cannot_replace_owned_content() {
    let url = common::pg_http_url().await;
    let backend = helios_hts::backends::PostgresTerminologyBackend::new(url)
        .await
        .unwrap();
    authority_contract(backend).await;
}
