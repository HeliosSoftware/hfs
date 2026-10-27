//! Tests for the meta search parameters `_tag`, `_profile`, `_security`,
//! and `_source` (#474).
//!
//! These are ordinary typed parameters indexed from `Resource.meta.*` by the
//! spec `SearchParameter` set; the query builder must apply them rather than
//! treating them as unimplemented specials and silently returning the
//! unfiltered result set.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

use helios_fhir::FhirVersion;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;

#[cfg(feature = "sqlite")]
fn create_sqlite_backend() -> SqliteBackend {
    super::make_sqlite_backend()
}

fn create_tenant() -> TenantContext {
    TenantContext::new(
        TenantId::new("test-tenant"),
        TenantPermissions::full_access(),
    )
}

#[cfg(feature = "sqlite")]
async fn seed_meta_patients(backend: &SqliteBackend, tenant: &TenantContext) {
    let tagged = json!({
        "resourceType": "Patient",
        "meta": {
            "tag": [{"system": "http://example.org/tags", "code": "test-data"}],
            "profile": ["http://example.org/StructureDefinition/custom-patient"],
            "security": [{
                "system": "http://terminology.hl7.org/CodeSystem/v3-ActCode",
                "code": "HTEST"
            }]
        },
        "gender": "female"
    });
    let untagged = json!({"resourceType": "Patient", "gender": "male"});
    backend
        .create(tenant, "Patient", tagged, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(tenant, "Patient", untagged, FhirVersion::default())
        .await
        .unwrap();
}

fn token_param(name: &str, system: Option<&str>, code: &str) -> SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::token(system, code)],
        chain: vec![],
        components: vec![],
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_tag_filters_to_tagged_resources() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_meta_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(token_param(
        "_tag",
        Some("http://example.org/tags"),
        "test-data",
    ));
    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert_eq!(
        result.resources.len(),
        1,
        "_tag must filter, not fall through to the unfiltered set"
    );
    assert_eq!(result.resources.items[0].content()["gender"], "female");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_tag_with_no_match_returns_empty() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_meta_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(token_param(
        "_tag",
        Some("http://example.org/tags"),
        "no-such-tag",
    ));
    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert_eq!(result.resources.len(), 0);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_security_filters_by_security_label() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_meta_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(token_param(
        "_security",
        Some("http://terminology.hl7.org/CodeSystem/v3-ActCode"),
        "HTEST",
    ));
    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert_eq!(result.resources.len(), 1);
    assert_eq!(result.resources.items[0].content()["gender"], "female");
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_profile_filters_by_canonical() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_meta_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "_profile".to_string(),
        param_type: SearchParamType::Uri,
        modifier: None,
        values: vec![SearchValue::string(
            "http://example.org/StructureDefinition/custom-patient",
        )],
        chain: vec![],
        components: vec![],
    });
    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert_eq!(result.resources.len(), 1);
    assert_eq!(result.resources.items[0].content()["gender"], "female");
}
