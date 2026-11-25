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

fn concept_map(id: &str, url: &str, title: &str, target: &str) -> Value {
    json!({
        "resourceType": "ConceptMap",
        "id": id,
        "url": url,
        "status": "active",
        "title": title,
        "group": [{
            "source": format!("{OWNER_BASE}/CodeSystem/owned"),
            "target": "http://target.example/cs",
            "element": [{"code": "a", "target": [{"code": target, "equivalence": "equivalent"}]}]
        }]
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

/// The same rule for a ConceptMap, which is stored one row per canonical URL:
/// neither an unattributed import of the same map nor a bundle that only
/// shares its `id` replaces what the owning package wrote.
async fn concept_map_authority_contract<B>(backend: B)
where
    B: TerminologyBackend + BundleImportBackend + Clone + 'static,
{
    let importer: Arc<dyn BundleImportBackend> = Arc::new(backend.clone());
    let owned_url = format!("{OWNER_BASE}/ConceptMap/owned");
    let stored_title = |backend: &B| {
        backend
            .resource_by_id("ConceptMap", "owned-cm")
            .unwrap()
            .expect("the owned map is still stored")["title"]
            .clone()
    };

    let owned = [concept_map("owned-cm", &owned_url, "Owner title", "x")];
    assert!(
        import_as(&importer, &owned, Some(OWNER_BASE))
            .await
            .is_empty()
    );

    let errors = import_as(
        &importer,
        &[concept_map("owned-cm", &owned_url, "Replaced", "y")],
        None,
    )
    .await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("authoritative source"), "{errors:?}");
    assert_eq!(stored_title(&backend), "Owner title");

    let errors = import_as(
        &importer,
        &[concept_map(
            "owned-cm",
            "http://attacker.example/cm",
            "Hijacked",
            "z",
        )],
        None,
    )
    .await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        stored_title(&backend),
        "Owner title",
        "a bundle sharing only the id replaced the owned map"
    );

    let errors = import_as(
        &importer,
        &[concept_map("owned-cm", &owned_url, "Owner update", "w")],
        Some(OWNER_BASE),
    )
    .await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(stored_title(&backend), "Owner update");
}

/// A CodeSystem a native importer loaded is owned: an unattributed bundle
/// with the same `(url, version)` cannot replace it.
async fn native_import_contract<B>(backend: B)
where
    B: TerminologyBackend + BundleImportBackend + Clone + 'static,
{
    let importer: Arc<dyn BundleImportBackend> = Arc::new(backend.clone());
    let icd10 = |title: &str| {
        json!({
            "resourceType": "CodeSystem",
            "id": "icd-10-cm",
            "url": "http://hl7.org/fhir/sid/icd-10-cm",
            "version": "2026",
            "status": "active",
            "content": "complete",
            "title": title,
            "concept": [{"code": "A00", "display": "Cholera"}]
        })
    };

    let stats = importer
        .import_native_bundle(
            &TenantContext::system(),
            &bundle(&[icd10("Native title")]),
            false,
        )
        .await
        .unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);

    let errors = import_as(&importer, &[icd10("Replaced")], None).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("authoritative source"), "{errors:?}");
    let stored = backend
        .resource_by_id("CodeSystem", "icd-10-cm")
        .unwrap()
        .unwrap();
    assert_eq!(stored["title"], "Native title");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_unattributed_import_cannot_replace_a_native_import() {
    native_import_contract(helios_hts::backends::SqliteTerminologyBackend::in_memory().unwrap())
        .await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_unattributed_import_cannot_replace_a_native_import() {
    let url = common::pg_http_url().await;
    let backend = helios_hts::backends::PostgresTerminologyBackend::new(url)
        .await
        .unwrap();
    native_import_contract(backend).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_unattributed_import_cannot_replace_owned_content() {
    authority_contract(helios_hts::backends::SqliteTerminologyBackend::in_memory().unwrap()).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_unattributed_import_cannot_replace_an_owned_concept_map() {
    concept_map_authority_contract(
        helios_hts::backends::SqliteTerminologyBackend::in_memory().unwrap(),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_unattributed_import_cannot_replace_an_owned_concept_map() {
    let url = common::pg_http_url().await;
    let backend = helios_hts::backends::PostgresTerminologyBackend::new(url)
        .await
        .unwrap();
    concept_map_authority_contract(backend).await;
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
