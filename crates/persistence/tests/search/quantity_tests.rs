//! Tests for quantity search parameters.
//!
//! This module tests quantity-type search parameters including
//! value|system|code format and unit-aware comparisons.

use serde_json::json;

use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
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

#[cfg(feature = "sqlite")]
async fn seed_observations(backend: &SqliteBackend, tenant: &TenantContext) {
    let observations = vec![
        json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {"coding": [{"code": "29463-7", "display": "Body weight"}]},
            "valueQuantity": {
                "value": 70,
                "unit": "kg",
                "system": "http://unitsofmeasure.org",
                "code": "kg"
            }
        }),
        json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {"coding": [{"code": "29463-7", "display": "Body weight"}]},
            "valueQuantity": {
                "value": 154,
                "unit": "lb",
                "system": "http://unitsofmeasure.org",
                "code": "[lb_av]"
            }
        }),
        json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {"coding": [{"code": "8302-2", "display": "Body height"}]},
            "valueQuantity": {
                "value": 175,
                "unit": "cm",
                "system": "http://unitsofmeasure.org",
                "code": "cm"
            }
        }),
    ];

    for obs in observations {
        backend
            .create(tenant, "Observation", obs, FhirVersion::default())
            .await
            .unwrap();
    }
}

/// Test quantity search with value only.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_quantity_search_value_only() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_observations(&backend, &tenant).await;

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "value-quantity".to_string(),
        param_type: SearchParamType::Quantity,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, "70")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find observation with value 70
    for resource in &result.resources.items {
        if let Some(value) = resource.content()["valueQuantity"]["value"].as_f64() {
            assert!((value - 70.0).abs() < 0.1);
        }
    }
}

/// Test quantity search with value and unit.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_quantity_search_value_and_unit() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_observations(&backend, &tenant).await;

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "value-quantity".to_string(),
        param_type: SearchParamType::Quantity,
        modifier: None,
        values: vec![SearchValue::new(
            SearchPrefix::Eq,
            "70|http://unitsofmeasure.org|kg",
        )],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find the 70 kg observation
    assert!(!result.resources.is_empty());
}

/// Test quantity search with comparison.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_quantity_search_gt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();
    seed_observations(&backend, &tenant).await;

    let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "value-quantity".to_string(),
        param_type: SearchParamType::Quantity,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Gt, "100")],
        chain: vec![],
        components: vec![],
    });

    let result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Should find observations with value > 100 (154 lb, 175 cm)
    for resource in &result.resources.items {
        if let Some(value) = resource.content()["valueQuantity"]["value"].as_f64() {
            assert!(value > 100.0);
        }
    }
}

/// Boundary matrix for the quantity prefix semantics fixed by #1011: `gt`,
/// `lt`, `ge`, `le`, `sa`, `eb` compare against the exact search value
/// regardless of how many decimals it was written with (and after UCUM unit
/// conversion), while `eq`/`ne` bound the implicit-precision range of the
/// value as written (see the module doc of `helios_persistence::search::range`
/// for the full rule).
///
/// Seeds four `Observation` resources with fixed ids and `valueQuantity`
/// 55.4 / 58.5 / 60.2 / 64.5 kg, then asserts the exact id set each case in
/// the table returns, including canonical (converted) cross-unit cases in
/// grams.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_quantity_prefix_boundary_matrix() {
    let backend = create_sqlite_backend();
    let tenant = TenantContext::new(
        TenantId::new("quantity-prefix-boundary"),
        TenantPermissions::full_access(),
    );

    let seeds: [(&str, f64); 4] = [
        ("w-55-4", 55.4),
        ("w-58-5", 58.5),
        ("w-60-2", 60.2),
        ("w-64-5", 64.5),
    ];
    for (id, value) in seeds {
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": id,
                    "resourceType": "Observation",
                    "status": "final",
                    "code": {"coding": [{"code": "29463-7", "display": "Body weight"}]},
                    "valueQuantity": {
                        "value": value,
                        "unit": "kg",
                        "system": "http://unitsofmeasure.org",
                        "code": "kg"
                    }
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed Observation");
    }

    // (search value including prefix, expected matching ids)
    let cases: [(&str, &[&str]); 25] = [
        ("gt60", &["w-60-2", "w-64-5"]),
        ("gt60.0", &["w-60-2", "w-64-5"]),
        ("gt60.2", &["w-64-5"]),
        ("ge60", &["w-60-2", "w-64-5"]),
        ("ge60.2", &["w-60-2", "w-64-5"]),
        ("lt60", &["w-55-4", "w-58-5"]),
        ("lt58.5", &["w-55-4"]),
        ("le58.5", &["w-55-4", "w-58-5"]),
        ("le60.2", &["w-55-4", "w-58-5", "w-60-2"]),
        ("sa60", &["w-60-2", "w-64-5"]),
        ("eb60", &["w-55-4", "w-58-5"]),
        ("eq60", &["w-60-2"]),
        ("eq60.0", &[]),
        ("eq60.2", &["w-60-2"]),
        ("eq60.5", &[]),
        ("ne60.0", &["w-55-4", "w-58-5", "w-60-2", "w-64-5"]),
        ("ne60", &["w-55-4", "w-58-5", "w-64-5"]),
        ("gt60|http://unitsofmeasure.org|kg", &["w-60-2", "w-64-5"]),
        ("ge60200|http://unitsofmeasure.org|g", &["w-60-2", "w-64-5"]),
        ("lt60|http://unitsofmeasure.org|kg", &["w-55-4", "w-58-5"]),
        ("le58500|http://unitsofmeasure.org|g", &["w-55-4", "w-58-5"]),
        ("sa60000|http://unitsofmeasure.org|g", &["w-60-2", "w-64-5"]),
        ("eb60000|http://unitsofmeasure.org|g", &["w-55-4", "w-58-5"]),
        ("lt58500|http://unitsofmeasure.org|g", &["w-55-4"]),
        (
            "ne60|http://unitsofmeasure.org|kg",
            &["w-55-4", "w-58-5", "w-64-5"],
        ),
    ];

    for (value, expected) in cases {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::parse(value)],
            chain: vec![],
            components: vec![],
        });

        let result = backend
            .search(&tenant, &query.with_count(100))
            .await
            .unwrap_or_else(|e| panic!("search value-quantity={value} failed: {e}"));

        let mut ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        ids.sort();
        let mut expected_sorted = expected.to_vec();
        expected_sorted.sort();
        assert_eq!(
            ids, expected_sorted,
            "value-quantity={value} must match {expected:?}"
        );
    }
}
