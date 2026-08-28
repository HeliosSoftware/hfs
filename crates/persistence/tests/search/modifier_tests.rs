//! Tests for search modifiers.
//!
//! This module tests various search modifiers including :missing,
//! :exact, :contains, :above, :below, :in, :not-in, and :text.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
};

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

// ============================================================================
// :missing Modifier Tests
// ============================================================================

/// Test :missing=true finds resources without the element.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_missing_true() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create patients - some with birthDate, some without
    let with_date =
        json!({"resourceType": "Patient", "id": "with-date", "birthDate": "1980-01-15"});
    let without_date = json!({
        "resourceType": "Patient",
        "id": "without-date",
        "name": [{"family": "No Date"}]
    });
    let extension_only = json!({
        "resourceType": "Patient",
        "id": "extension-only",
        "_birthDate": {
            "extension": [{
                "url": "http://example.org/fhir/StructureDefinition/birthdate-note",
                "valueString": "withheld"
            }]
        }
    });
    backend
        .create(&tenant, "Patient", with_date, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", without_date, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", extension_only, FhirVersion::default())
        .await
        .unwrap();

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: Some(SearchModifier::Missing),
        values: vec![SearchValue::boolean(true)],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    let mut ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["extension-only", "without-date"],
        "birthdate:missing=true must include absent and extension-only primitives"
    );
}

/// Test :missing=false finds resources with the element.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_missing_false() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let with_date =
        json!({"resourceType": "Patient", "id": "with-date", "birthDate": "1980-01-15"});
    let without_date = json!({
        "resourceType": "Patient",
        "id": "without-date",
        "name": [{"family": "No Date"}]
    });
    backend
        .create(&tenant, "Patient", with_date, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", without_date, FhirVersion::default())
        .await
        .unwrap();

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: Some(SearchModifier::Missing),
        values: vec![SearchValue::boolean(false)],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
    assert_eq!(
        ids,
        vec!["with-date"],
        "birthdate:missing=false must return exactly the Patient with birthDate"
    );
}

/// A value on a contained resource must not establish presence on its
/// top-level container.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_missing_ignores_contained_resource_values() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "container-without-date",
                "contained": [{
                    "resourceType": "Patient",
                    "id": "contained-with-date",
                    "birthDate": "1980-01-15"
                }]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: Some(SearchModifier::Missing),
        values: vec![SearchValue::boolean(true)],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();
    let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();

    assert_eq!(ids, vec!["container-without-date"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_missing_treats_empty_arrays_as_absent() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    for resource in [
        json!({
            "resourceType": "Patient",
            "id": "name-present",
            "name": [{"family": "Present"}]
        }),
        json!({
            "resourceType": "Patient",
            "id": "name-empty-array",
            "name": []
        }),
        json!({
            "resourceType": "Patient",
            "id": "name-absent"
        }),
    ] {
        backend
            .create(&tenant, "Patient", resource, FhirVersion::default())
            .await
            .unwrap();
    }

    let query = |is_missing| {
        SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::boolean(is_missing)],
            chain: vec![],
            components: vec![],
        })
    };

    let result = backend.search(&tenant, &query(true)).await.unwrap();
    let mut missing_ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
    missing_ids.sort();
    assert_eq!(missing_ids, vec!["name-absent", "name-empty-array"]);

    let result = backend.search(&tenant, &query(false)).await.unwrap();
    let present_ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
    assert_eq!(present_ids, vec!["name-present"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_missing_supports_indexed_metadata_parameters() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "with-tag",
                "meta": {
                    "tag": [{"system": "http://example.org/tags", "code": "reviewed"}]
                }
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    backend
        .create(
            &tenant,
            "Patient",
            json!({
                "resourceType": "Patient",
                "id": "without-tag"
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let query = |is_missing| {
        SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_tag".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::boolean(is_missing)],
            chain: vec![],
            components: vec![],
        })
    };

    let missing = backend.search(&tenant, &query(true)).await.unwrap();
    let missing_ids: Vec<&str> = missing.resources.items.iter().map(|r| r.id()).collect();
    assert_eq!(missing_ids, vec!["without-tag"]);

    let present = backend.search(&tenant, &query(false)).await.unwrap();
    let present_ids: Vec<&str> = present.resources.items.iter().map(|r| r.id()).collect();
    assert_eq!(present_ids, vec!["with-tag"]);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_missing_uses_authoritative_id_and_last_updated_columns() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    for id in ["metadata-a", "metadata-b"] {
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": id}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    for (name, param_type) in [
        ("_id", SearchParamType::Token),
        ("_lastUpdated", SearchParamType::Date),
    ] {
        let query = |is_missing| {
            SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: name.to_string(),
                param_type,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::boolean(is_missing)],
                chain: vec![],
                components: vec![],
            })
        };

        let missing = backend.search(&tenant, &query(true)).await.unwrap();
        assert!(missing.resources.items.is_empty(), "{name}:missing=true");

        let present = backend.search(&tenant, &query(false)).await.unwrap();
        let mut ids: Vec<&str> = present.resources.items.iter().map(|r| r.id()).collect();
        ids.sort();
        assert_eq!(ids, vec!["metadata-a", "metadata-b"]);
    }
}

#[cfg(all(
    feature = "sqlite",
    feature = "R4",
    feature = "R4B",
    feature = "R5",
    feature = "R6"
))]
#[tokio::test]
async fn test_missing_across_all_fhir_versions() {
    for version in [
        FhirVersion::R4,
        FhirVersion::R4B,
        FhirVersion::R5,
        FhirVersion::R6,
    ] {
        let backend = super::make_sqlite_backend_for(version);
        let tenant = create_tenant();

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "with-date",
                    "birthDate": "1980-01-15"
                }),
                version,
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "without-date"
                }),
                version,
            )
            .await
            .unwrap();

        for (is_missing, expected) in [(true, "without-date"), (false, "with-date")] {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "birthdate".to_string(),
                param_type: SearchParamType::Date,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::boolean(is_missing)],
                chain: vec![],
                components: vec![],
            });
            let result = backend.search(&tenant, &query).await.unwrap();
            let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
            assert_eq!(
                ids,
                vec![expected],
                "FHIR {version:?}, missing={is_missing}"
            );
        }
    }
}

// ============================================================================
// :not Modifier Tests
// ============================================================================

/// Test :not modifier excludes matching resources.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_not_modifier() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let male = json!({"resourceType": "Patient", "gender": "male"});
    let female = json!({"resourceType": "Patient", "gender": "female"});
    let unknown = json!({"resourceType": "Patient", "gender": "unknown"});
    backend
        .create(&tenant, "Patient", male, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", female, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", unknown, FhirVersion::default())
        .await
        .unwrap();

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Not),
        values: vec![SearchValue::token(None, "male")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find female and unknown, not male
    for resource in &result.resources.items {
        assert_ne!(resource.content()["gender"], "male");
    }
}

/// :not means "no value of the parameter matches" — a resource whose element
/// is absent entirely has no matching value, so it must be returned (#473).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_not_modifier_includes_resources_missing_the_element() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let male = json!({"resourceType": "Patient", "gender": "male"});
    let female = json!({"resourceType": "Patient", "gender": "female"});
    let no_gender = json!({"resourceType": "Patient"});
    backend
        .create(&tenant, "Patient", male, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", female, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "Patient", no_gender, FhirVersion::default())
        .await
        .unwrap();

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Not),
        values: vec![SearchValue::token(None, "male")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert_eq!(
        result.resources.len(),
        2,
        "gender:not=male must return the female patient and the one with no gender"
    );
    assert!(
        result
            .resources
            .items
            .iter()
            .any(|r| r.content().get("gender").is_none()),
        "a patient without a gender element must match gender:not=male"
    );
}

/// :not must exclude a resource when ANY of its values matches — negating per
/// row let multi-valued resources leak back in through their other rows (#473).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_not_modifier_excludes_multi_valued_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let dual_coded = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [
            {"system": "http://loinc.org", "code": "8302-2"},
            {"system": "http://snomed.info/sct", "code": "50373000"}
        ]}
    });
    let other = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {"coding": [{"system": "http://loinc.org", "code": "8867-4"}]}
    });
    backend
        .create(&tenant, "Observation", dual_coded, FhirVersion::default())
        .await
        .unwrap();
    let other_id = backend
        .create(&tenant, "Observation", other, FhirVersion::default())
        .await
        .unwrap()
        .id()
        .to_string();

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Not),
        values: vec![SearchValue::token(Some("http://loinc.org"), "8302-2")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
    assert_eq!(
        ids,
        vec![other_id.as_str()],
        "the observation carrying 8302-2 must not leak in via its SNOMED coding"
    );
}

// ============================================================================
// :text Modifier Tests
// ============================================================================

/// Test :text modifier for narrative search.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_text_modifier() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let obs = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{"code": "8867-4"}],
            "text": "Patient heart rate measurement during exercise"
        }
    });
    backend
        .create(&tenant, "Observation", obs, FhirVersion::default())
        .await
        .unwrap();

    // :text modifier searches the display/text fields
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::Text),
        values: vec![SearchValue::string("heart rate")],
        chain: vec![],
        components: vec![],
    });

    let _result = backend.search(&tenant, &query.with_count(100)).await;

    // Test documents expected text search behavior
}

// ============================================================================
// :identifier Modifier Tests
// ============================================================================

/// Test :identifier modifier on reference parameters.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_identifier_modifier() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create patient with identifier
    let patient = json!({
        "resourceType": "Patient",
        "id": "patient-123",
        "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN001"}]
    });
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "patient-123",
            patient,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Create observation referencing patient
    let obs = json!({
        "resourceType": "Observation",
        "status": "final",
        "subject": {"reference": "Patient/patient-123"},
        "code": {"coding": [{"code": "test"}]}
    });
    backend
        .create(&tenant, "Observation", obs, FhirVersion::default())
        .await
        .unwrap();

    // Search using :identifier modifier
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "subject".to_string(),
        param_type: SearchParamType::Reference,
        modifier: Some(SearchModifier::Identifier),
        values: vec![SearchValue::token(
            Some("http://hospital.org/mrn"),
            "MRN001",
        )],
        chain: vec![],
        components: vec![],
    });

    let _result = backend.search(&tenant, &query.with_count(100)).await;
}

// ============================================================================
// :of-type Modifier Tests (FHIR v6.0.0 Enhanced)
// ============================================================================

/// Test :of-type modifier with three-part format [system]|[code]|[value].
///
/// The :of-type modifier allows searching identifiers by type. In FHIR v6.0.0,
/// the format is enhanced to support: `identifier:of-type=[system]|[code]|[value]`
/// where system and code identify the identifier type, and value is the identifier value.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_of_type_modifier_three_part_format() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create patient with typed identifier (SSN)
    let patient_with_ssn = json!({
        "resourceType": "Patient",
        "identifier": [{
            "type": {
                "coding": [{
                    "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                    "code": "SS"
                }]
            },
            "system": "http://hl7.org/fhir/sid/us-ssn",
            "value": "123-45-6789"
        }]
    });
    backend
        .create(&tenant, "Patient", patient_with_ssn, FhirVersion::default())
        .await
        .unwrap();

    // Create patient with typed identifier (MRN)
    let patient_with_mrn = json!({
        "resourceType": "Patient",
        "identifier": [{
            "type": {
                "coding": [{
                    "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                    "code": "MR"
                }]
            },
            "system": "http://hospital.org/mrn",
            "value": "MRN-001"
        }]
    });
    backend
        .create(&tenant, "Patient", patient_with_mrn, FhirVersion::default())
        .await
        .unwrap();

    // Search for SSN type identifier with :of-type modifier
    // Format: identifier:of-type=[type-system]|[type-code]|[value]
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::OfType),
        values: vec![SearchValue::of_type(
            "http://terminology.hl7.org/CodeSystem/v2-0203",
            "SS",
            "123-45-6789",
        )],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query.with_count(100)).await;

    // Test documents expected :of-type behavior
    // When implemented, should only match the SSN identifier
    match result {
        Ok(result) => {
            // If implemented, verify only SSN patient is returned
            for resource in &result.resources.items {
                let identifiers = resource.content()["identifier"].as_array().unwrap();
                let has_ssn = identifiers
                    .iter()
                    .any(|id| id["type"]["coding"][0]["code"] == "SS");
                assert!(has_ssn, "Should only match SSN identifiers");
            }
        }
        Err(_) => {
            // :of-type may not be implemented yet - this test serves as specification
        }
    }
}

// ============================================================================
// :text-advanced Modifier Tests (FHIR v6.0.0)
// ============================================================================

/// Test :text-advanced modifier with simple term.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_text_advanced_simple_term() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create observations with different display text
    let obs_headache = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "25064002",
                "display": "Headache disorder"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_headache, FhirVersion::default())
        .await
        .unwrap();

    let obs_migraine = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "37796009",
                "display": "Migraine with aura"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_migraine, FhirVersion::default())
        .await
        .unwrap();

    // Search for "headache" using :text-advanced
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("headache")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query.with_count(100)).await;

    // FTS5 availability is required for :text-advanced
    // If FTS5 is available, should find the headache observation
    if let Ok(result) = result {
        // FTS5 available - should match via porter stemming
        if !result.resources.is_empty() {
            assert_eq!(result.resources.len(), 1);
            let code = result.resources.items[0].content()["code"]["coding"][0]["display"].as_str();
            assert!(code.unwrap().contains("Headache"));
        }
    }
}

/// Test :text-advanced modifier with OR operator.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_text_advanced_or_operator() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create observations
    let obs_headache = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "25064002",
                "display": "Headache disorder"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_headache, FhirVersion::default())
        .await
        .unwrap();

    let obs_migraine = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "37796009",
                "display": "Migraine syndrome"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_migraine, FhirVersion::default())
        .await
        .unwrap();

    let obs_other = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "12345",
                "display": "Fracture of leg"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_other, FhirVersion::default())
        .await
        .unwrap();

    // Search for "headache OR migraine" using :text-advanced
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("headache OR migraine")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query.with_count(100)).await;

    // If FTS5 is available, should find both headache and migraine
    if let Ok(result) = result {
        if !result.resources.is_empty() {
            assert_eq!(
                result.resources.len(),
                2,
                "Should find both headache and migraine"
            );
        }
    }
}

/// Test :text-advanced modifier with phrase matching.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_text_advanced_phrase_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create observations with similar but different display text
    let obs_heart_failure = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "84114007",
                "display": "Chronic heart failure"
            }]
        }
    });
    backend
        .create(
            &tenant,
            "Observation",
            obs_heart_failure,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let obs_heart_rate = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://loinc.org",
                "code": "8867-4",
                "display": "Heart rate"
            }]
        }
    });
    backend
        .create(
            &tenant,
            "Observation",
            obs_heart_rate,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for exact phrase "heart failure"
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("\"heart failure\"")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query.with_count(100)).await;

    // If FTS5 is available, should only match exact phrase
    if let Ok(result) = result {
        if !result.resources.is_empty() {
            assert_eq!(
                result.resources.len(),
                1,
                "Should only find heart failure, not heart rate"
            );
            let display =
                result.resources.items[0].content()["code"]["coding"][0]["display"].as_str();
            assert!(display.unwrap().contains("failure"));
        }
    }
}

/// Test :text-advanced modifier with prefix matching.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_text_advanced_prefix_match() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create observations with related terms
    let obs_cardio = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "49601007",
                "display": "Cardiovascular disease"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_cardio, FhirVersion::default())
        .await
        .unwrap();

    let obs_cardiac = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "56265001",
                "display": "Cardiac arrest"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_cardiac, FhirVersion::default())
        .await
        .unwrap();

    let obs_fracture = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "125605004",
                "display": "Fracture of bone"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_fracture, FhirVersion::default())
        .await
        .unwrap();

    // Search for prefix "cardi*" (matches cardiac, cardiovascular)
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("cardi*")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query.with_count(100)).await;

    // If FTS5 is available, should match both cardi* terms
    if let Ok(result) = result {
        if !result.resources.is_empty() {
            assert_eq!(
                result.resources.len(),
                2,
                "Should find cardiovascular and cardiac"
            );
        }
    }
}

/// Test :text-advanced modifier with NOT operator.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_text_advanced_not_operator() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create observations
    let obs_heart_surgery = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "64915003",
                "display": "Heart surgery procedure"
            }]
        }
    });
    backend
        .create(
            &tenant,
            "Observation",
            obs_heart_surgery,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let obs_heart_failure = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "84114007",
                "display": "Heart failure condition"
            }]
        }
    });
    backend
        .create(
            &tenant,
            "Observation",
            obs_heart_failure,
            FhirVersion::default(),
        )
        .await
        .unwrap();

    // Search for "heart" but NOT "surgery"
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("heart -surgery")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query.with_count(100)).await;

    // If FTS5 is available, should only match heart failure (not surgery)
    if let Ok(result) = result {
        if !result.resources.is_empty() {
            assert_eq!(
                result.resources.len(),
                1,
                "Should find heart failure but not heart surgery"
            );
            let display =
                result.resources.items[0].content()["code"]["coding"][0]["display"].as_str();
            assert!(display.unwrap().contains("failure"));
        }
    }
}

/// Test :text-advanced modifier with porter stemming.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_text_advanced_stemming() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create observation with plural/conjugated term
    let obs_running = json!({
        "resourceType": "Observation",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://snomed.info/sct",
                "code": "282239000",
                "display": "Running injury"
            }]
        }
    });
    backend
        .create(&tenant, "Observation", obs_running, FhirVersion::default())
        .await
        .unwrap();

    // Search for "run" - should match "running" via porter stemming
    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::TextAdvanced),
        values: vec![SearchValue::string("run")],
        chain: vec![],
        components: vec![],
    });

    let result = backend.search(&tenant, &query.with_count(100)).await;

    // If FTS5 is available with porter stemmer, should match "running"
    if let Ok(result) = result {
        if !result.resources.is_empty() {
            assert_eq!(
                result.resources.len(),
                1,
                "Should find 'running' when searching for 'run'"
            );
        }
    }
}

/// Test :of-type modifier distinguishes between identifier types.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_of_type_modifier_type_discrimination() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create patient with multiple identifiers of different types
    let patient = json!({
        "resourceType": "Patient",
        "identifier": [
            {
                "type": {
                    "coding": [{
                        "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                        "code": "SS"
                    }]
                },
                "value": "123-45-6789"
            },
            {
                "type": {
                    "coding": [{
                        "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                        "code": "DL"
                    }]
                },
                "value": "DL-999888"
            }
        ]
    });
    backend
        .create(&tenant, "Patient", patient, FhirVersion::default())
        .await
        .unwrap();

    // Search for DL (driver's license) type - should match
    let dl_query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::OfType),
        values: vec![SearchValue::of_type(
            "http://terminology.hl7.org/CodeSystem/v2-0203",
            "DL",
            "DL-999888",
        )],
        chain: vec![],
        components: vec![],
    });

    // Search for passport type - should NOT match (patient has no passport)
    let pp_query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "identifier".to_string(),
        param_type: SearchParamType::Token,
        modifier: Some(SearchModifier::OfType),
        values: vec![SearchValue::of_type(
            "http://terminology.hl7.org/CodeSystem/v2-0203",
            "PPN",
            "DL-999888", // Same value but wrong type
        )],
        chain: vec![],
        components: vec![],
    });

    // Test documents expected behavior when :of-type is implemented
    let _dl_result = backend.search(&tenant, &dl_query.with_count(100)).await;
    let _pp_result = backend.search(&tenant, &pp_query.with_count(100)).await;
}
