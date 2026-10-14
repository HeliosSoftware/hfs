//! Tests for date search parameters.
//!
//! This module tests date-type search parameters including
//! eq, ne, lt, le, gt, ge prefixes and date precision handling.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
};

use helios_fhir::FhirVersion;

#[cfg(feature = "sqlite")]
use helios_persistence::backends::sqlite::SqliteBackend;

// ============================================================================
// Helper Functions
// ============================================================================

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
async fn seed_test_patients(backend: &SqliteBackend, tenant: &TenantContext) {
    let patients = vec![
        json!({"resourceType": "Patient", "birthDate": "1980-01-15"}),
        json!({"resourceType": "Patient", "birthDate": "1990-06-30"}),
        json!({"resourceType": "Patient", "birthDate": "2000-12-25"}),
        json!({"resourceType": "Patient", "birthDate": "1975-03-10"}),
        json!({"resourceType": "Patient", "birthDate": "2010-08-05"}),
    ];

    for patient in patients {
        backend
            .create(tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
    }
}

// ============================================================================
// Date Search Tests - Equality
// ============================================================================

/// Test date search with exact date.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_eq() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, "1980-01-15")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    assert!(!result.resources.is_empty());
    for resource in &result.resources.items {
        assert_eq!(resource.content()["birthDate"], "1980-01-15");
    }
}

// ============================================================================
// Date Search Tests - Comparison Operators
// ============================================================================

/// Test date search with less than.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_lt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Lt, "1985-01-01")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find patients born before 1985
    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(
            birth_date < "1985-01-01",
            "Birth date {} should be < 1985-01-01",
            birth_date
        );
    }
}

/// Test date search with greater than.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_gt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Gt, "2000-01-01")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(
            birth_date > "2000-01-01",
            "Birth date {} should be > 2000-01-01",
            birth_date
        );
    }
}

/// Test date search with less than or equal.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_le() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Le, "1990-06-30")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(birth_date <= "1990-06-30");
    }
}

/// Test date search with greater than or equal.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_ge() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Ge, "2000-01-01")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(birth_date >= "2000-01-01");
    }
}

// ============================================================================
// Date Search Tests - Range Queries
// ============================================================================

/// Test date search with range (between two dates).
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_range() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Find patients born between 1985 and 2005
    let query = SearchQuery::new("Patient")
        .with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "1985-01-01")],
            chain: vec![],
            components: vec![],
        })
        .with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Le, "2005-12-31")],
            chain: vec![],
            components: vec![],
        });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(("1985-01-01"..="2005-12-31").contains(&birth_date));
    }
}

// ============================================================================
// Date Search Tests - Year Precision
// ============================================================================

/// Test date search with year precision.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_search_year_precision() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_test_patients(&backend, &tenant).await;

    // Search for anyone born in 1990
    let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "birthdate".to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, "1990")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find patient with birthDate 1990-06-30
    for resource in &result.resources.items {
        let birth_date = resource.content()["birthDate"].as_str().unwrap();
        assert!(birth_date.starts_with("1990"));
    }
}

/// The shared sub-day precision and validation table (#1293, #1295, #1296,
/// #1297); PostgreSQL, MongoDB and Elasticsearch run the same one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_date_precision_and_validation_suite() {
    let backend = super::make_sqlite_backend();
    super::date_precision_suite::sub_day_precision_and_validation(&backend, "date-precision").await;
}

/// The shared number/quantity `ap` tables (#1390);
/// PostgreSQL, MongoDB and Elasticsearch run the same ones.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_ap_prefix_suite() {
    let backend = super::make_sqlite_backend();
    super::ap_prefix_suite::ap_prefix(&backend, "ap-prefix", true).await;
}

/// `ap` in the quantity component of a composite (#1390);
/// PostgreSQL, MongoDB and Elasticsearch run the same tables.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_ap_composite_suite() {
    let backend = super::make_sqlite_backend();
    super::ap_relations_suite::ap_composite(&backend, "ap-composite").await;
}

/// `_filter` must keep the date range and the following token bind separate.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_fractional_date_filter_with_following_criterion() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    for (id, performed, status) in [
        ("inside-completed", "2024-01-01T10:00:00.55Z", "completed"),
        ("inside-progress", "2024-01-01T10:00:00.55Z", "in-progress"),
        ("outside-completed", "2024-01-01T10:00:00.60Z", "completed"),
    ] {
        backend
            .create_or_update(
                &tenant,
                "Procedure",
                id,
                json!({
                    "resourceType": "Procedure",
                    "status": status,
                    "subject": {"reference": "Patient/filter-subject"},
                    "performedDateTime": performed,
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    let query = SearchQuery::new("Procedure").with_parameter(SearchParameter {
        name: "_filter".to_string(),
        param_type: SearchParamType::Special,
        values: vec![SearchValue::eq(
            "date eq 2024-01-01T10:00:00.5Z and status eq completed",
        )],
        ..Default::default()
    });
    let result = backend.search(&tenant, &query).await.unwrap();
    let ids: Vec<_> = result.resources.items.iter().map(|r| r.id()).collect();
    assert_eq!(ids, ["inside-completed"]);
}

/// `_filter ... ap` on a quantity column uses the shared closed `ap` range
/// (#1503), and on a string column it no longer degrades to a substring match.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_filter_ap_quantity_uses_shared_range() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    for (id, dose) in [
        ("q-89", 89.0),
        ("q-91", 91.0),
        ("q-100", 100.0),
        ("q-109", 109.0),
        ("q-111", 111.0),
        ("q-1000", 1000.0),
    ] {
        backend
            .create_or_update(
                &tenant,
                "Observation",
                id,
                json!({
                    "resourceType": "Observation",
                    "status": "final",
                    "code": {"text": "dose"},
                    "valueQuantity": {
                        "value": dose,
                        "unit": "mg",
                        "system": "http://unitsofmeasure.org",
                        "code": "mg",
                    },
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
    backend
        .create_or_update(
            &tenant,
            "Patient",
            "filter-ap-smithson",
            json!({"resourceType": "Patient", "name": [{"family": "Smithson"}]}),
            FhirVersion::default(),
        )
        .await
        .unwrap();

    let filter_ids = |resource_type: &'static str, expr: &'static str| {
        let backend = &backend;
        let tenant = &tenant;
        async move {
            let query = SearchQuery::new(resource_type).with_parameter(SearchParameter {
                name: "_filter".to_string(),
                param_type: SearchParamType::Special,
                values: vec![SearchValue::eq(expr)],
                ..Default::default()
            });
            let result = backend.search(tenant, &query).await.unwrap();
            let mut ids: Vec<String> = result
                .resources
                .items
                .iter()
                .map(|r| r.id().to_string())
                .collect();
            ids.sort();
            ids
        }
    };

    // Shared window [90, 110]: 1000 (which a substring match on "100" would
    // return) is out, 91 and 109 (which it would miss) are in.
    assert_eq!(
        filter_ids("Observation", "value-quantity ap 100").await,
        ["q-100", "q-109", "q-91"]
    );

    // `ap` has no meaning on a string: no substring match.
    assert!(filter_ids("Patient", "name ap Smith").await.is_empty());
    // Positive control: the data is there, and `co` still finds it.
    assert_eq!(
        filter_ids("Patient", "name co Smith").await,
        ["filter-ap-smithson"]
    );
}

/// The shared table for stored dateTimes with minutes but no seconds (#1315);
/// PostgreSQL and MongoDB run the same one. SQLite never had the gap — it
/// indexes the text as stored and reads the `hh:mm` start into
/// `value_date_start` — so this pins it.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_minute_precision_stored_dates_are_indexed() {
    let backend = super::make_sqlite_backend();
    super::date_minute_index_suite::minute_precision_stored_values_are_indexed(
        &backend,
        "date-minute-index",
    )
    .await;
}
