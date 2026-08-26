//! Tests for chained search parameters.
//!
//! This module tests chained search parameters (e.g., patient.name)
//! and reverse chaining (_has).

use serde_json::json;

use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::search::resolve_chains;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    ChainedParameter, ReverseChainedParameter, SearchParamType, SearchParameter, SearchQuery,
    SearchValue,
};

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
async fn seed_chained_data(backend: &SqliteBackend, tenant: &TenantContext) {
    // Create patients
    let patient1 = json!({
        "resourceType": "Patient",
        "id": "patient-smith",
        "name": [{"family": "Smith", "given": ["John"]}]
    });
    let patient2 = json!({
        "resourceType": "Patient",
        "id": "patient-jones",
        "name": [{"family": "Jones", "given": ["Jane"]}]
    });
    backend
        .create_or_update(
            tenant,
            "Patient",
            "patient-smith",
            patient1,
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create_or_update(
            tenant,
            "Patient",
            "patient-jones",
            patient2,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create observations for patients
    let obs1 = json!({
        "resourceType": "Observation",
        "status": "final",
        "subject": {"reference": "Patient/patient-smith"},
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]}
    });
    let obs2 = json!({
        "resourceType": "Observation",
        "status": "final",
        "subject": {"reference": "Patient/patient-smith"},
        "code": {"coding": [{"system": "http://loinc.org", "code": "8310-5"}]}
    });
    let obs3 = json!({
        "resourceType": "Observation",
        "status": "final",
        "subject": {"reference": "Patient/patient-jones"},
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]}
    });
    backend
        .create(tenant, "Observation", obs1, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(tenant, "Observation", obs2, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(tenant, "Observation", obs3, FhirVersion::default())
        .await
        .unwrap();
}

// ============================================================================
// Chained Search Tests
// ============================================================================

/// Test chained search: Observation?subject.name=Smith
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_chained_search_subject_name() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_chained_data(&backend, &tenant).await;

    // Search for observations where patient name is Smith
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }],
        components: vec![],
    });

    let rewritten = resolve_chains(&backend, &tenant, &query).await.unwrap();
    let result = backend
        .search(&tenant, &rewritten.with_count(100))
        .await
        .unwrap();

    // Should find observations for patient Smith
    for resource in &result.resources.items {
        assert_eq!(
            resource.content()["subject"]["reference"],
            "Patient/patient-smith"
        );
    }
}

/// Test chained search with type hint: Observation?subject:Patient.name=Smith
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_chained_search_with_type_hint() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_chained_data(&backend, &tenant).await;

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Smith")],
        chain: vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }],
        components: vec![],
    });

    let rewritten = resolve_chains(&backend, &tenant, &query).await.unwrap();
    let result = backend
        .search(&tenant, &rewritten.with_count(100))
        .await
        .unwrap();

    // Should find Smith's observations
    assert!(!result.resources.is_empty());
}

/// Test chained search with no results.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_chained_search_no_results() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_chained_data(&backend, &tenant).await;

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("Nonexistent")],
        chain: vec![ChainedParameter {
            reference_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "name".to_string(),
        }],
        components: vec![],
    });

    let rewritten = resolve_chains(&backend, &tenant, &query).await.unwrap();
    let result = backend
        .search(&tenant, &rewritten.with_count(100))
        .await
        .unwrap();

    assert!(result.resources.is_empty());
}

// ============================================================================
// Reverse Chaining (_has) Tests
// ============================================================================

/// Test reverse chaining: Patient?_has:Observation:subject:code=8867-4
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_reverse_chaining() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_chained_data(&backend, &tenant).await;

    // Find patients that have observations with code 8867-4
    // This is expressed as: Patient?_has:Observation:subject:code=8867-4
    let mut query = SearchQuery::new("Patient");
    query.reverse_chains.push(ReverseChainedParameter {
        source_type: "Observation".to_string(),
        reference_param: "subject".to_string(),
        search_param: "code".to_string(),
        value: Some(SearchValue::token(Some("http://loinc.org"), "8867-4")),
        nested: None,
    });

    let rewritten = resolve_chains(&backend, &tenant, &query).await.unwrap();
    let result = backend
        .search(&tenant, &rewritten.with_count(100))
        .await
        .unwrap();

    // Should find both patients (both have 8867-4 observations)
    assert!(!result.resources.is_empty());
}

/// Test reverse chaining with no matches.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_reverse_chaining_no_matches() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_chained_data(&backend, &tenant).await;

    // Find patients with nonexistent observation code
    let mut query = SearchQuery::new("Patient");
    query.reverse_chains.push(ReverseChainedParameter {
        source_type: "Observation".to_string(),
        reference_param: "subject".to_string(),
        search_param: "code".to_string(),
        value: Some(SearchValue::token(Some("http://loinc.org"), "NONEXISTENT")),
        nested: None,
    });

    let rewritten = resolve_chains(&backend, &tenant, &query).await.unwrap();
    let result = backend
        .search(&tenant, &rewritten.with_count(100))
        .await
        .unwrap();

    assert!(result.resources.is_empty());
}

/// #645: the resolver's intermediate searches must drain every page. With the
/// terminal hop left at the backend's default page size, any hop matching
/// more than 100 resources silently truncated the whole chain — both
/// `patient.gender=female` and `=male` answered exactly total=100 on a
/// Synthea set where each should have been in the thousands.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_chained_search_survives_hops_beyond_the_default_page() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // 150 female patients, one Observation each: the terminal hop
    // (Patient?gender=female) matches more than one default page.
    for i in 0..150 {
        let pid = format!("chained-page-p{i}");
        backend
            .create_or_update(
                &tenant,
                "Patient",
                &pid,
                json!({"resourceType": "Patient", "id": pid, "gender": "female"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let oid = format!("chained-page-o{i}");
        backend
            .create_or_update(
                &tenant,
                "Observation",
                &oid,
                json!({"resourceType": "Observation", "id": oid, "status": "final",
                       "code": {"text": "hr"},
                       "subject": {"reference": format!("Patient/{pid}")}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "patient".to_string(),
        param_type: SearchParamType::Reference,
        modifier: None,
        values: vec![SearchValue::eq("female")],
        chain: vec![ChainedParameter {
            reference_param: "patient".to_string(),
            target_type: Some("Patient".to_string()),
            target_param: "gender".to_string(),
        }],
        components: vec![],
    });

    let rewritten = resolve_chains(&backend, &tenant, &query).await.unwrap();
    let id_filter = rewritten
        .parameters
        .iter()
        .find(|p| p.name == "_id")
        .expect("chains rewrite into an _id filter");
    assert_eq!(
        id_filter.values.len(),
        150,
        "every matching id survives, not just the first page"
    );

    let result = backend
        .search(&tenant, &rewritten.with_count(1000))
        .await
        .unwrap();
    assert_eq!(result.resources.items.len(), 150);
}
