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

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_unattributed_import_cannot_replace_owned_content() {
    authority_contract(helios_hts::backends::SqliteTerminologyBackend::in_memory().unwrap()).await;
}

/// An unattributed import that arrives first decides neither the id of the
/// owning package's map nor which map answers on that id: the owner takes the
/// id it declares, and a displaced map keeps its content under a fresh id.
async fn concept_map_owner_claims_its_id_contract<B>(backend: B)
where
    B: TerminologyBackend + BundleImportBackend + Clone + 'static,
{
    let importer: Arc<dyn BundleImportBackend> = Arc::new(backend.clone());
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let read = |id: &str| {
        backend
            .resource_by_id("ConceptMap", id)
            .unwrap()
            .map(|map| (map["title"].clone(), map["url"].clone(), map["id"].clone()))
    };
    let translate = |url: String| {
        let backend = backend.clone();
        async move {
            let request = helios_hts::types::TranslateRequest {
                url: Some(url),
                system: Some(format!("{OWNER_BASE}/CodeSystem/owned")),
                code: "a".into(),
                ..Default::default()
            };
            helios_hts::traits::ConceptMapOperations::translate(
                &backend,
                &TenantContext::system(),
                request,
            )
            .await
            .unwrap()
            .matches
            .into_iter()
            .map(|m| m.concept_code)
            .collect::<Vec<_>>()
        }
    };

    // The id is taken first, under another URL.
    let id = format!("squatted-id-{tag}");
    let owned_url = format!("{OWNER_BASE}/ConceptMap/id-{tag}");
    let squatter_url = format!("http://attacker.example/cm/id-{tag}");
    let squatter = [concept_map(&id, &squatter_url, "Squatter", "z")];
    assert!(import_as(&importer, &squatter, None).await.is_empty());
    let owned = [concept_map(&id, &owned_url, "Owner title", "x")];
    let errors = import_as(&importer, &owned, Some(OWNER_BASE)).await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        read(&id),
        Some((json!("Owner title"), json!(owned_url), json!(id))),
        "the owning package did not get the id it declares"
    );
    assert_eq!(translate(owned_url.clone()).await, ["x"]);
    assert_eq!(
        translate(squatter_url.clone()).await,
        ["z"],
        "the displaced map lost its content"
    );
    // Importing the squatter again neither takes the id back nor fails.
    let errors = import_as(&importer, &squatter, None).await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(read(&id).unwrap().0, "Owner title");
    assert_eq!(translate(squatter_url).await, ["z"]);

    // The URL is taken first, under another id.
    let id = format!("owned-id-{tag}");
    let squatter_id = format!("squatter-id-{tag}");
    let owned_url = format!("{OWNER_BASE}/ConceptMap/url-{tag}");
    assert!(
        import_as(
            &importer,
            &[concept_map(&squatter_id, &owned_url, "Squatter", "z")],
            None
        )
        .await
        .is_empty()
    );
    let errors = import_as(
        &importer,
        &[concept_map(&id, &owned_url, "Owner title", "x")],
        Some(OWNER_BASE),
    )
    .await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        read(&id),
        Some((json!("Owner title"), json!(owned_url), json!(id))),
        "the owner's map stayed under the id of the copy it replaced"
    );
    assert_eq!(read(&squatter_id), None);
    assert_eq!(translate(owned_url.clone()).await, ["x"]);

    // Between equal ranks the id is stable: a package that renames its map
    // keeps the stored id.
    let errors = import_as(
        &importer,
        &[concept_map(
            &format!("renamed-{tag}"),
            &owned_url,
            "Owner update",
            "w",
        )],
        Some(OWNER_BASE),
    )
    .await;
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(read(&id).unwrap().0, "Owner update");
    assert_eq!(read(&format!("renamed-{tag}")), None);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_owning_package_claims_its_concept_map_id() {
    concept_map_owner_claims_its_id_contract(
        helios_hts::backends::SqliteTerminologyBackend::in_memory().unwrap(),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn postgres_owning_package_claims_its_concept_map_id() {
    let url = common::pg_http_url().await;
    let backend = helios_hts::backends::PostgresTerminologyBackend::new(url)
        .await
        .unwrap();
    concept_map_owner_claims_its_id_contract(backend).await;
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
