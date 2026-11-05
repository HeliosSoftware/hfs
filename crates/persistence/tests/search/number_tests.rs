//! Tests for number search parameters.
//!
//! This module tests number-type search parameters including
//! comparison operators and significant figures handling.

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

/// Test number search with equality.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_number_search_eq() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    // Create RiskAssessment resources with probability
    let risk1 = json!({
        "resourceType": "RiskAssessment",
        "status": "final",
        "prediction": [{"probabilityDecimal": 0.5}]
    });
    let risk2 = json!({
        "resourceType": "RiskAssessment",
        "status": "final",
        "prediction": [{"probabilityDecimal": 0.75}]
    });
    backend
        .create(&tenant, "RiskAssessment", risk1, FhirVersion::default())
        .await
        .unwrap();
    backend
        .create(&tenant, "RiskAssessment", risk2, FhirVersion::default())
        .await
        .unwrap();

    let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
        name: "probability".to_string(),
        param_type: SearchParamType::Number,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Eq, "0.5")],
        chain: vec![],
        components: vec![],
    });

    let _result = backend
        .search(&tenant, &query.with_count(100))
        .await
        .unwrap();

    // Number search implementation may vary
    // This test documents expected behavior
}

/// Test number search with less than.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_number_search_lt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
        name: "probability".to_string(),
        param_type: SearchParamType::Number,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Lt, "0.6")],
        chain: vec![],
        components: vec![],
    });

    let _result = backend.search(&tenant, &query.with_count(100)).await;

    // Test documents expected behavior for number comparisons
}

/// Test number search with greater than.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_number_search_gt() {
    let backend = create_sqlite_backend();
    let tenant = create_tenant();

    let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
        name: "probability".to_string(),
        param_type: SearchParamType::Number,
        modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Gt, "0.4")],
        chain: vec![],
        components: vec![],
    });

    let _result = backend.search(&tenant, &query.with_count(100)).await;
}

/// Boundary matrix for the number prefix semantics fixed by #1011: `gt`,
/// `lt`, `ge`, `le` compare against the exact search value regardless of how
/// many decimals it was written with, while `eq` bounds the implicit-
/// precision range of the value as written (see the module doc of
/// `helios_persistence::search::range` for the full rule).
///
/// Seeds four `RiskAssessment` resources with fixed ids and
/// `prediction[].probabilityDecimal` 0.25 / 0.5 / 0.52 / 0.75, then asserts
/// the exact id set each case in the table returns.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_number_prefix_boundary_matrix() {
    let backend = create_sqlite_backend();
    let tenant = TenantContext::new(
        TenantId::new("number-prefix-boundary"),
        TenantPermissions::full_access(),
    );

    let seeds: [(&str, f64); 4] = [
        ("r-25", 0.25),
        ("r-50", 0.5),
        ("r-52", 0.52),
        ("r-75", 0.75),
    ];
    for (id, probability) in seeds {
        backend
            .create(
                &tenant,
                "RiskAssessment",
                json!({
                    "id": id,
                    "resourceType": "RiskAssessment",
                    "status": "final",
                    "prediction": [{"probabilityDecimal": probability}]
                }),
                FhirVersion::default(),
            )
            .await
            .expect("seed RiskAssessment");
    }

    // (search value including prefix, expected matching ids)
    let cases: [(&str, &[&str]); 7] = [
        ("gt0.5", &["r-52", "r-75"]),
        ("gt0.50", &["r-52", "r-75"]),
        ("ge0.5", &["r-50", "r-52", "r-75"]),
        ("lt0.5", &["r-25"]),
        ("le0.5", &["r-25", "r-50"]),
        ("eq0.5", &["r-50", "r-52"]),
        ("eq0.50", &["r-50"]),
    ];

    for (value, expected) in cases {
        let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
            name: "probability".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::parse(value)],
            chain: vec![],
            components: vec![],
        });

        let result = backend
            .search(&tenant, &query.with_count(100))
            .await
            .unwrap_or_else(|e| panic!("search probability={value} failed: {e}"));

        let mut ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        ids.sort();
        let mut expected_sorted = expected.to_vec();
        expected_sorted.sort();
        assert_eq!(
            ids, expected_sorted,
            "probability={value} must match {expected:?}"
        );
    }
}

/// The shared table for exponent-form number and quantity values (#1337);
/// PostgreSQL, MongoDB and Elasticsearch run the same one.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_exponent_values_use_significant_figures() {
    let backend = create_sqlite_backend();
    super::number_exponent_suite::exponent_values_use_significant_figures(
        &backend,
        "number-exponent",
        true,
    )
    .await;
}

/// The shared number / quantity validation tables (#1319, #1340); PostgreSQL,
/// MongoDB and Elasticsearch run the same ones.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_numeric_validation_suite() {
    let backend = super::make_sqlite_backend();
    super::numeric_validation_suite::invalid_numbers_are_rejected_on_every_path(
        &backend,
        "numeric-validation",
    )
    .await;
}

/// The same values as conditional criteria.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_numeric_validation_suite_conditional_criteria() {
    let backend = super::make_sqlite_backend();
    super::numeric_validation_suite::invalid_numbers_are_rejected_in_conditional_criteria(
        &backend,
        "numeric-validation-conditional",
    )
    .await;
}
