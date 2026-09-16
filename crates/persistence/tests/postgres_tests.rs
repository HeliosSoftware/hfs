//! PostgreSQL backend integration tests.
//!
//! These tests verify the PostgreSQL backend implementation.
//! Tests that require a running PostgreSQL instance use testcontainers
//! to spin up real PostgreSQL instances in Docker.
//!
//! Run with: `cargo test -p helios-persistence --features postgres -- postgres`

#![cfg(feature = "postgres")]

use helios_persistence::backends::postgres::PostgresConfig;
use helios_persistence::core::BackendKind;

/// The backend-agnostic `ifMatch` scenarios (issue #311), shared verbatim with
/// the SQLite suite that owns the file.
///
/// Declared at the top level rather than inside `mod postgres_integration`
/// because `#[path]` on a module nested in an *inline* module resolves relative
/// to `tests/postgres_tests/postgres_integration/`, which does not exist. At the
/// crate root it resolves relative to `tests/`, which does.
#[path = "transactions/if_match_suite.rs"]
mod if_match_suite;

/// The backend-agnostic tenant-id fidelity scenarios (issue #447), shared
/// verbatim with the SQLite and MongoDB suites. Declared at the top level for
/// the same `#[path]` resolution reason as `if_match_suite` above.
#[path = "multitenancy/tenant_id_fidelity_suite.rs"]
mod tenant_id_fidelity_suite;

/// The backend-agnostic full-text purge-completeness scenarios (issue #386),
/// shared verbatim with the SQLite suite that owns the file.
///
/// PostgreSQL already deleted `resource_fts` in its purge paths — the defect was
/// SQLite-only — so these lock the *reference* backend's behaviour in place so a
/// future change cannot silently regress it. The `$reindex` scenarios are a
/// different matter: those failed on PostgreSQL too, because
/// `write_search_entries` never rebuilt the full-text row.
///
/// Declared at the top level for the same `#[path]` resolution reason as
/// `if_match_suite` above.
#[path = "search/fts_purge_suite.rs"]
mod fts_purge_suite;

/// The backend-agnostic `Resource`-level meta-parameter scenarios (#523),
/// shared verbatim with the SQLite suite that owns the file.
///
/// `_source` matched nothing on every index-backed backend until the extractor
/// stopped evaluating the spec's `Resource.meta.source` verbatim. Running one
/// scenario on both engines is what keeps that honest — the SQLite-only
/// `meta_params_tests.rs` from #474 cannot see an extraction bug, because a
/// dropped filter and a correct filter over a missing index row look identical
/// from there.
///
/// Declared at the top level for the same `#[path]` resolution reason as
/// `if_match_suite` above.
#[path = "search/meta_params_suite.rs"]
mod meta_params_suite;

/// The backend-agnostic day-precision date-boundary suite (issue #519):
/// the #456 boundary table, which #463 fixed and pinned for SQLite only.
/// Declared at the top level for the same `#[path]` resolution reason as
/// `if_match_suite` above.
#[path = "search/date_boundary_suite.rs"]
mod date_boundary_suite;

// ============================================================================
// Backend Configuration Tests (no PostgreSQL instance required)
// ============================================================================

#[test]
fn test_postgres_config_defaults() {
    let config = PostgresConfig::default();
    assert_eq!(config.host, "localhost");
    assert_eq!(config.port, 5432);
    assert_eq!(config.dbname, "helios");
    assert_eq!(config.user, "helios");
    assert!(config.password.is_none());
    // Derived from the core count (cores * 4, clamped) rather than a fixed 10, which
    // throttled search badly under concurrent load — see #224. Assert the contract
    // (the clamp bounds), not the machine-dependent value.
    assert!(
        (16..=64).contains(&config.max_connections),
        "pool size {} outside the 16..=64 clamp",
        config.max_connections
    );
    assert_eq!(config.connect_timeout_secs, 5);
    assert_eq!(config.statement_timeout_ms, 30000);
    assert!(!config.search_offloaded);
}

#[test]
fn test_postgres_config_serialization() {
    let config = PostgresConfig {
        host: "pg-server".to_string(),
        port: 5433,
        dbname: "test_db".to_string(),
        user: "test_user".to_string(),
        password: Some("secret".to_string()),
        ..Default::default()
    };

    let json = serde_json::to_string(&config).unwrap();
    let deserialized: PostgresConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.host, "pg-server");
    assert_eq!(deserialized.port, 5433);
    assert_eq!(deserialized.dbname, "test_db");
    assert_eq!(deserialized.user, "test_user");
    assert_eq!(deserialized.password, Some("secret".to_string()));
}

// ============================================================================
// Backend Capability Tests (no PostgreSQL instance required)
// ============================================================================

// NOTE: capability *declarations* are asserted in
// `tests/backend_capability_contract.rs`, against the constructor-free
// `PostgresBackend::declared_capabilities()`. They live there rather than here
// because a `PostgresBackend` cannot be constructed without a real database
// (the constructor connects immediately), and because the assertions are
// cross-backend.
//
// A `test_postgres_expected_capabilities` used to live here. It listed the
// capabilities by hand and then asserted `!expected.is_empty()` — which passes
// for any non-empty list, so it verified nothing while reading like a contract.
// Worse, its hand list was a third copy of the false `SchemaPerTenant` /
// `DatabasePerTenant` claim corrected in #369. Deleted rather than repaired.

#[test]
fn test_postgres_config_backend_kind() {
    // Verify BackendKind::Postgres exists and is usable
    let kind = BackendKind::Postgres;
    assert_eq!(format!("{}", kind), "postgres");
}

// ============================================================================
// Query Builder Unit Tests (no PostgreSQL instance required)
// ============================================================================

mod query_builder_tests {
    use helios_persistence::backends::postgres::search::query_builder::{
        PostgresQueryBuilder, SqlParam,
    };
    use helios_persistence::types::{
        SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
    };

    #[test]
    fn test_empty_query_returns_none() {
        let query = SearchQuery::new("Patient");
        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_none());
    }

    #[test]
    fn test_id_parameter() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("123")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("id = $"));
        assert_eq!(fragment.params.len(), 1);
        match &fragment.params[0] {
            SqlParam::Text(s) => assert_eq!(s, "123"),
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_string_parameter_default() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Default string search is starts-with, emitted since schema v34 as an
        // explicit bytewise range on the folded expression rather than a `LIKE`:
        // a `LIKE` whose pattern is a bind parameter cannot be turned into an
        // index range by the planner, so the bounds are derived in Rust. Never
        // `ILIKE`, which cannot use a btree at all (#224); both the stored column
        // and the bound values are already case-folded by `fold_text`, and the
        // raw-column fallback is wrapped in `lower()` so un-backfilled rows keep
        // matching case-insensitively.
        assert!(
            fragment
                .sql
                .contains("COALESCE(value_string_folded, lower(value_string)) ~>=~ $3")
                && fragment
                    .sql
                    .contains("COALESCE(value_string_folded, lower(value_string)) ~<~ $4"),
            "string search must target the indexed folded expression: {}",
            fragment.sql
        );
        assert!(!fragment.sql.contains("ILIKE"));
        assert!(fragment.sql.contains("param_name = 'name'"));
        // Two bounds, both folded: the prefix itself and the exclusive successor
        // of its last character.
        assert_eq!(fragment.params.len(), 2);
        match (&fragment.params[0], &fragment.params[1]) {
            (SqlParam::Text(lo), SqlParam::Text(hi)) => {
                assert_eq!(lo, "smith");
                assert_eq!(hi, "smiti");
            }
            other => panic!("Expected two Text params, got {:?}", other),
        }
    }

    #[test]
    fn test_string_parameter_exact() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Exact),
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Exact match should use = not ILIKE
        assert!(fragment.sql.contains("value_string = $"));
    }

    #[test]
    fn test_string_parameter_contains() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Contains),
            values: vec![SearchValue::eq("mit")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(
            fragment
                .sql
                .contains("COALESCE(value_string_folded, lower(value_string)) LIKE")
        );
        assert!(!fragment.sql.contains("ILIKE"));
        // Parameter should be "%mit%"
        match &fragment.params[0] {
            SqlParam::Text(s) => {
                assert!(s.starts_with('%'));
                assert!(s.ends_with('%'));
            }
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_token_parameter_code_text() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::CodeText),
            values: vec![SearchValue::eq("Heart")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_display ILIKE"));
        // starts-with: param is "Heart%"
        match &fragment.params[0] {
            SqlParam::Text(s) => {
                assert!(!s.starts_with('%'));
                assert!(s.ends_with('%'));
            }
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_token_parameter_text_contains() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Text),
            values: vec![SearchValue::eq("Heart")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_display ILIKE"));
        // contains: param is "%Heart%"
        match &fragment.params[0] {
            SqlParam::Text(s) => {
                assert!(s.starts_with('%'));
                assert!(s.ends_with('%'));
            }
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_string_parameter_text() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: Some(SearchModifier::Text),
            values: vec![SearchValue::eq("mit")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Accent-folded substring match against the indexed folded expression.
        assert!(fragment.sql.contains("value_string_folded"));
        assert!(
            fragment
                .sql
                .contains("COALESCE(value_string_folded, lower(value_string)) LIKE")
        );
        assert!(!fragment.sql.contains("ILIKE"));
        // Substring match: param wrapped as %mit%
        match &fragment.params[0] {
            SqlParam::Text(s) => {
                assert!(s.starts_with('%'));
                assert!(s.ends_with('%'));
            }
            _ => panic!("Expected Text param"),
        }
    }

    #[test]
    fn test_reference_parameter_below_above() {
        use helios_persistence::types::SearchModifier;

        for modifier in [SearchModifier::Below, SearchModifier::Above] {
            let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                modifier: Some(modifier),
                values: vec![SearchValue::eq("http://x.org/Questionnaire/q")],
                chain: vec![],
                components: vec![],
            });
            let fragment = PostgresQueryBuilder::build_search_query(&query, 2).unwrap();
            assert!(fragment.sql.contains("value_reference"));
            assert!(fragment.sql.contains("|| '/%'"));
        }
    }

    #[test]
    fn test_reference_parameter_identifier() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Identifier),
            values: vec![SearchValue::eq("http://hospital.org|12345")],
            chain: vec![],
            components: vec![],
        });

        let fragment = PostgresQueryBuilder::build_search_query(&query, 2).unwrap();
        // The identifier lookup drives and yields the target's `Type/id`; the
        // reference index is seeked with it. The correlated-EXISTS form this
        // replaced measured 285.6 ms against 1.4 ms on the same replica — see
        // `build_reference_identifier_condition`.
        assert!(fragment.sql.contains("param_name = 'identifier'"));
        assert!(fragment.sql.contains("idx.value_token_system"));
        assert!(fragment.sql.contains("idx.value_token_code"));
        assert!(
            !fragment.sql.contains("EXISTS") && !fragment.sql.contains("SUBSTRING"),
            "{}",
            fragment.sql
        );
    }

    #[test]
    fn test_uri_parameter_contains() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("ValueSet").with_parameter(SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: Some(SearchModifier::Contains),
            values: vec![SearchValue::eq("example.org")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_uri ILIKE"));
    }

    #[test]
    fn test_reference_parameter_text_and_code_text() {
        use helios_persistence::types::SearchModifier;

        for (modifier, expect_leading_pct) in [
            (SearchModifier::Text, true),
            (SearchModifier::CodeText, false),
        ] {
            let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                modifier: Some(modifier),
                values: vec![SearchValue::eq("John")],
                chain: vec![],
                components: vec![],
            });

            let fragment = PostgresQueryBuilder::build_search_query(&query, 2).unwrap();
            assert!(fragment.sql.contains("value_reference_display ILIKE"));
            // :text wraps as %John%; :code-text is starts-with John%
            assert_eq!(
                fragment.sql.contains("'%' || $3 || '%'"),
                expect_leading_pct
            );
        }
    }

    #[test]
    fn test_reference_parameter_contains() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Contains),
            values: vec![SearchValue::eq("patient-1")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_reference ILIKE"));
    }

    #[test]
    fn test_token_system_and_code() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://loinc.org|8867-4")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_system"));
        assert!(fragment.sql.contains("value_token_code"));
        assert_eq!(fragment.params.len(), 2);
    }

    #[test]
    fn test_token_code_only() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("male")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_code"));
        assert_eq!(fragment.params.len(), 1);
    }

    #[test]
    fn test_token_system_only() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://loinc.org|")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_token_system"));
        // The `system|` form binds the system and nothing else — still exactly
        // one parameter. It does carry `value_token_code IS NOT NULL` as of
        // schema v31, which is not a second binding: it is the conjunct that
        // makes the partial `idx_search_token_code_recent` reachable for this
        // shape, and it excludes no row (`IndexValue::Token.code` is a
        // non-optional `String`). What must stay true is that no code VALUE is
        // compared, which is what would make the search wrong.
        assert!(fragment.sql.contains("value_token_code IS NOT NULL"));
        assert!(!fragment.sql.contains("value_token_code ="));
        assert_eq!(fragment.params.len(), 1);
    }

    #[test]
    fn test_date_parameter() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Gt, "2000-01-01")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_date"));
        // gt now matches strictly after the day → value_date >= (next day).
        assert!(fragment.sql.contains(">= $"));
    }

    #[test]
    fn test_number_parameter() {
        let query = SearchQuery::new("RiskAssessment").with_parameter(SearchParameter {
            name: "probability".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "0.5")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_number"));
        assert!(fragment.sql.contains(">= $"));
        // ge ignores implicit precision and matches the exact search value.
        match &fragment.params[0] {
            SqlParam::Float(f) => assert!((f - 0.5).abs() < 1e-9),
            _ => panic!("Expected Float param"),
        }
    }

    #[test]
    fn test_quantity_parameter() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::eq("5.4||mg")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_quantity_value"));
        assert!(fragment.sql.contains("value_quantity_unit"));
    }

    #[test]
    fn test_reference_parameter() {
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Patient/123")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_reference"));
    }

    #[test]
    fn test_uri_parameter() {
        let query = SearchQuery::new("ValueSet").with_parameter(SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: None,
            values: vec![SearchValue::eq("http://example.org/fhir/ValueSet/123")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("value_uri"));
    }

    #[test]
    fn test_uri_below_modifier() {
        use helios_persistence::types::SearchModifier;

        let query = SearchQuery::new("ValueSet").with_parameter(SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: Some(SearchModifier::Below),
            values: vec![SearchValue::eq("http://example.org/fhir")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("LIKE"));
    }

    #[test]
    fn test_last_updated_parameter() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Ge, "2024-01-01")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        assert!(fragment.sql.contains("last_updated"));
        assert!(fragment.sql.contains(">= $"));
    }

    #[test]
    fn test_multiple_values_or() {
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("123"), SearchValue::eq("456")],
            chain: vec![],
            components: vec![],
        });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Multiple _id values should be OR'd
        assert!(fragment.sql.contains("OR"));
        assert_eq!(fragment.params.len(), 2);
    }

    #[test]
    fn test_multiple_parameters_and() {
        let query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "name".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::eq("Smith")],
                chain: vec![],
                components: vec![],
            })
            .with_parameter(SearchParameter {
                name: "gender".to_string(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: vec![SearchValue::eq("male")],
                chain: vec![],
                components: vec![],
            });

        let result = PostgresQueryBuilder::build_search_query(&query, 2);
        assert!(result.is_some());
        let fragment = result.unwrap();
        // Different parameters should be AND'd
        assert!(fragment.sql.contains("AND"));
    }

    #[test]
    fn test_prefix_operators() {
        // Day-precision prefixes compare against the value's [day, day+1)
        // range (#871), matching the date-parameter semantics from #463: eq
        // must mean "inside the named day", gt must exclude it entirely.
        let cases = vec![
            (
                SearchPrefix::Eq,
                "last_updated >= $1 AND last_updated < $2",
                2,
            ),
            (
                SearchPrefix::Ne,
                "(last_updated < $1 OR last_updated >= $2)",
                2,
            ),
            (SearchPrefix::Gt, "last_updated >= $1", 1),
            (SearchPrefix::Lt, "last_updated < $1", 1),
            (SearchPrefix::Ge, "last_updated >= $1", 1),
            (SearchPrefix::Le, "last_updated < $1", 1),
        ];

        for (prefix, expected_sql, expected_params) in cases {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "_lastUpdated".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::new(prefix, "2024-01-01")],
                chain: vec![],
                components: vec![],
            });

            let result = PostgresQueryBuilder::build_search_query(&query, 0);
            assert!(result.is_some(), "Failed for prefix {:?}", prefix);
            let fragment = result.unwrap();
            assert!(
                fragment.sql.contains(expected_sql),
                "Expected '{}' for prefix {:?}, got SQL: {}",
                expected_sql,
                prefix,
                fragment.sql
            );
            assert_eq!(
                fragment.params.len(),
                expected_params,
                "param count for prefix {:?}",
                prefix
            );
        }

        // A full-precision instant is a degenerate range and falls back to
        // scalar comparison.
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Gt, "2024-01-01T10:00:00Z")],
            chain: vec![],
            components: vec![],
        });
        let fragment = PostgresQueryBuilder::build_search_query(&query, 0)
            .expect("full-precision instant must build");
        assert!(
            fragment.sql.contains("last_updated > $1"),
            "full-precision gt must stay scalar, got SQL: {}",
            fragment.sql
        );
    }
}

// ============================================================================
// Integration Tests (requires Docker for testcontainers)
// ============================================================================

/// Integration tests that require a real PostgreSQL instance via testcontainers.
///
/// These tests are behind `#[cfg(feature = "postgres")]` and require Docker.
/// They mirror the patterns in sqlite_tests.rs.
///
/// Run with:
///   cargo test -p helios-persistence --features postgres -- postgres_integration
///
/// Skip if no Docker:
///   cargo test -p helios-persistence --features postgres -- --skip postgres_integration
#[cfg(test)]
mod postgres_integration {
    use std::path::PathBuf;

    use helios_fhir::FhirVersion;
    use serde_json::json;

    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::SettingsStore;
    use helios_persistence::core::history::{HistoryParams, InstanceHistoryProvider};
    use helios_persistence::core::{Backend, BackendCapability, BackendKind, ResourceStorage};
    use helios_persistence::error::{
        BackendError, BulkExportError, ConcurrencyError, ResourceError, StorageError,
    };
    use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use tokio::sync::{Mutex, OnceCell};

    #[tokio::test]
    async fn postgres_publication_exact_contract() {
        use helios_persistence::core::{
            BulkSubmitProvider, LeaseError, ManifestPublicationResult, ManifestPublicationStatus,
            ManifestStatus, SubmissionId, SubmitFileRecord, SubmitWorkerStorage,
        };

        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("publication-exact-contract");
        let submission = SubmissionId::generate("publication-exact-contract");
        let manifest_url = "https://provider/publication-exact-contract.json";
        backend
            .create_submission(&tenant, &submission, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &submission, Some(manifest_url), None)
            .await
            .unwrap();
        let lease = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "publication-contract-worker-{}",
                uuid::Uuid::new_v4().simple()
            )),
            &submission,
            &manifest.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        let output = SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: "output".to_string(),
            resource_type: Some("Patient".to_string()),
            part_index: 0,
            file_path: "output/patient-0.ndjson".to_string(),
            line_count: 3,
            byte_count: 128,
            count_severity: None,
        };
        let error = SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: "error".to_string(),
            resource_type: Some("OperationOutcome".to_string()),
            part_index: 0,
            file_path: "error/outcome-0.ndjson".to_string(),
            line_count: 1,
            byte_count: 96,
            count_severity: Some(serde_json::json!({"error": 2})),
        };

        backend.record_submit_file(&lease, &output).await.unwrap();
        backend.record_submit_file(&lease, &output).await.unwrap();
        assert!(
            backend
                .list_submit_files(&tenant, &submission)
                .await
                .unwrap()
                .is_empty(),
            "staged artifacts are not publication-visible"
        );

        let mut conflicting = output.clone();
        conflicting.byte_count = 127;
        assert!(
            matches!(
                backend.record_submit_file(&lease, &conflicting).await,
                Err(LeaseError::Storage(_))
            ),
            "the same artifact identity cannot change staged byte_count"
        );

        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    &[output.clone(), error.clone()],
                    ManifestPublicationStatus::Completed,
                )
                .await
                .unwrap(),
            ManifestPublicationResult::Published
        );
        let manifest = backend
            .get_manifest(&tenant, &submission, &lease.manifest_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(manifest.status, ManifestStatus::Completed);
        let rows = backend
            .list_submit_files(&tenant, &submission)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        let row = rows
            .iter()
            .find(|row| row.file_type == "output")
            .expect("published output row");
        assert_eq!(row.manifest_url.as_deref(), Some(manifest_url));
        assert_eq!(row.resource_type.as_deref(), Some("Patient"));
        assert_eq!(row.part_index, 0);
        assert_eq!(row.fencing_token, lease.fencing_token);
        assert_eq!(row.file_path, "output/patient-0.ndjson");
        assert_eq!(row.line_count, 3);
        assert_eq!(row.byte_count, 128);
        assert_eq!(row.count_severity, None);
        let row = rows
            .iter()
            .find(|row| row.file_type == "error")
            .expect("published error row");
        assert_eq!(row.manifest_url.as_deref(), Some(manifest_url));
        assert_eq!(row.resource_type.as_deref(), Some("OperationOutcome"));
        assert_eq!(row.part_index, 0);
        assert_eq!(row.fencing_token, lease.fencing_token);
        assert_eq!(row.file_path, "error/outcome-0.ndjson");
        assert_eq!(row.line_count, 1);
        assert_eq!(row.byte_count, 96);
        assert_eq!(row.count_severity, Some(serde_json::json!({"error": 2})));

        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    &[error.clone(), output.clone()],
                    ManifestPublicationStatus::Completed,
                )
                .await
                .unwrap(),
            ManifestPublicationResult::AlreadyPublished
        );

        let mut altered = output.clone();
        altered.byte_count = 127;
        assert!(
            matches!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[altered, error.clone()],
                        ManifestPublicationStatus::Completed,
                    )
                    .await,
                Err(LeaseError::Storage(_))
            ),
            "published byte_count cannot change on replay"
        );
        assert!(
            matches!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[output.clone(), error.clone()],
                        ManifestPublicationStatus::Failed {
                            error_message: "changed terminal".to_string(),
                        },
                    )
                    .await,
                Err(LeaseError::Storage(_))
            ),
            "published terminal status cannot change on replay"
        );

        assert!(
            matches!(
                backend
                    .publish_manifest_artifacts(
                        &helios_persistence::core::ManifestLease {
                            worker_id: helios_persistence::core::WorkerId::new(
                                "interloper".to_string()
                            ),
                            ..lease.clone()
                        },
                        &[output.clone(), error.clone()],
                        ManifestPublicationStatus::Completed,
                    )
                    .await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "the same publication cannot be replayed by another worker"
        );

        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    &[output.clone(), error.clone()],
                    ManifestPublicationStatus::Completed,
                )
                .await
                .unwrap(),
            ManifestPublicationResult::AlreadyPublished
        );
        assert_eq!(
            backend
                .list_submit_files(&tenant, &submission)
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn postgres_publication_fault_rollback_and_retry() {
        use helios_persistence::core::{
            BulkSubmitProvider, LeaseError, ManifestPublicationResult, ManifestPublicationStatus,
            ManifestStatus, SubmissionId, SubmitFileRecord, SubmitWorkerStorage,
        };

        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("publication-fault-rollback");
        let submission = SubmissionId::generate("publication-fault-rollback");
        let manifest_url = "https://provider/publication-fault-rollback.json";
        backend
            .create_submission(&tenant, &submission, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &submission, Some(manifest_url), None)
            .await
            .unwrap();
        let lease = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "publication-fault-worker-{}",
                uuid::Uuid::new_v4().simple()
            )),
            &submission,
            &manifest.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        let output = SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: "output".to_string(),
            resource_type: Some("Patient".to_string()),
            part_index: 0,
            file_path: "output/patient-0.ndjson".to_string(),
            line_count: 3,
            byte_count: 128,
            count_severity: None,
        };
        let error = SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: "error".to_string(),
            resource_type: Some("OperationOutcome".to_string()),
            part_index: 0,
            file_path: "error/outcome-0.ndjson".to_string(),
            line_count: 1,
            byte_count: 96,
            count_severity: Some(serde_json::json!({"error": 2})),
        };

        let client = backend.get_client().await.unwrap();
        let identity_where = "tenant_id = $1 AND submitter = $2 AND submission_id = $3 \
                              AND manifest_id = $4";
        let tenant_id = lease.tenant.tenant_id().as_str();
        let identity: [&(dyn tokio_postgres::types::ToSql + Sync); 4] = [
            &tenant_id,
            &lease.submission_id.submitter,
            &lease.submission_id.submission_id,
            &lease.manifest_id,
        ];
        let fault_worker = format!("publication-fault-{}", uuid::Uuid::new_v4().simple());
        let insert_fn = format!(
            "publication_fault_insert_fn_{}",
            uuid::Uuid::new_v4().simple()
        );
        let insert_trigger = format!(
            "publication_fault_insert_trigger_{}",
            uuid::Uuid::new_v4().simple()
        );
        client
            .batch_execute(&format!(
                r#"
                CREATE FUNCTION public.{insert_fn}() RETURNS trigger LANGUAGE plpgsql AS $body$
                BEGIN
                    UPDATE bulk_manifests
                    SET worker_id = '{fault_worker}', fencing_token = fencing_token + 1
                    WHERE tenant_id = NEW.tenant_id AND submitter = NEW.submitter
                      AND submission_id = NEW.submission_id AND manifest_id = NEW.manifest_id;
                    RETURN NEW;
                END;
                $body$;
                CREATE TRIGGER {insert_trigger} AFTER INSERT ON bulk_submit_files
                FOR EACH ROW
                WHEN (NEW.tenant_id = '{tenant}' AND NEW.submitter = '{submitter}'
                      AND NEW.submission_id = '{submission}' AND NEW.manifest_id = '{manifest_id}')
                EXECUTE FUNCTION public.{insert_fn}();
                "#,
                tenant = lease.tenant.tenant_id(),
                submitter = lease.submission_id.submitter,
                submission = lease.submission_id.submission_id,
                manifest_id = lease.manifest_id,
            ))
            .await
            .unwrap();

        assert!(
            matches!(
                backend
                    .publish_manifest_artifacts(
                        &lease,
                        &[output.clone(), error.clone()],
                        ManifestPublicationStatus::Completed,
                    )
                    .await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "the active lease must no longer match the injected worker/token"
        );
        let raw_count = client
            .query_one(
                &format!("SELECT count(*) FROM bulk_submit_files WHERE {identity_where}"),
                &identity,
            )
            .await
            .unwrap()
            .get::<_, i64>(0);
        assert_eq!(raw_count, 0);
        let marker = client
            .query_one(
                &format!(
                    "SELECT worker_id, fencing_token, published_token, publication_status, \
                     publication_error_message FROM bulk_manifests WHERE {identity_where}"
                ),
                &identity,
            )
            .await
            .unwrap();
        assert_eq!(marker.get::<_, String>(0), lease.worker_id.as_str());
        assert_eq!(marker.get::<_, i64>(1), lease.fencing_token as i64);
        assert_eq!(marker.get::<_, Option<i64>>(2), None);
        assert_eq!(marker.get::<_, Option<String>>(3), None);
        assert_eq!(marker.get::<_, Option<String>>(4), None);
        client
            .batch_execute(&format!(
                "DROP TRIGGER IF EXISTS {insert_trigger} ON bulk_submit_files; \
                 DROP FUNCTION IF EXISTS public.{insert_fn}();"
            ))
            .await
            .unwrap();

        let update_fn = format!(
            "publication_fault_update_fn_{}",
            uuid::Uuid::new_v4().simple()
        );
        let update_trigger = format!(
            "publication_fault_update_trigger_{}",
            uuid::Uuid::new_v4().simple()
        );
        client
            .batch_execute(&format!(
                r#"
                CREATE FUNCTION public.{update_fn}() RETURNS trigger LANGUAGE plpgsql AS $body$
                BEGIN
                    IF NEW.published_token IS NOT NULL AND NEW.tenant_id = '{tenant}'
                       AND NEW.submitter = '{submitter}' AND NEW.submission_id = '{submission}'
                       AND NEW.manifest_id = '{manifest_id}' THEN
                        RAISE EXCEPTION 'publication injected terminal failure';
                    END IF;
                    RETURN NEW;
                END;
                $body$;
                CREATE TRIGGER {update_trigger} BEFORE UPDATE ON bulk_manifests
                FOR EACH ROW EXECUTE FUNCTION public.{update_fn}();
                "#,
                tenant = lease.tenant.tenant_id(),
                submitter = lease.submission_id.submitter,
                submission = lease.submission_id.submission_id,
                manifest_id = lease.manifest_id,
            ))
            .await
            .unwrap();
        let Err(LeaseError::Storage(storage_error)) = backend
            .publish_manifest_artifacts(
                &lease,
                &[output.clone(), error.clone()],
                ManifestPublicationStatus::Completed,
            )
            .await
        else {
            panic!("the BEFORE UPDATE fault must surface as a storage error");
        };
        assert!(matches!(
            storage_error,
            helios_persistence::error::StorageError::Backend(
                helios_persistence::error::BackendError::Internal { .. }
            )
        ));
        assert!(storage_error.to_string().contains("publish manifest:"));
        let raw_count = client
            .query_one(
                &format!("SELECT count(*) FROM bulk_submit_files WHERE {identity_where}"),
                &identity,
            )
            .await
            .unwrap()
            .get::<_, i64>(0);
        assert_eq!(raw_count, 0);
        let marker = client
            .query_one(
                &format!(
                    "SELECT worker_id, fencing_token, published_token, publication_status, \
                     publication_error_message FROM bulk_manifests WHERE {identity_where}"
                ),
                &identity,
            )
            .await
            .unwrap();
        assert_eq!(marker.get::<_, String>(0), lease.worker_id.as_str());
        assert_eq!(marker.get::<_, i64>(1), lease.fencing_token as i64);
        assert_eq!(marker.get::<_, Option<i64>>(2), None);
        assert_eq!(marker.get::<_, Option<String>>(3), None);
        assert_eq!(marker.get::<_, Option<String>>(4), None);
        client
            .batch_execute(&format!(
                "DROP TRIGGER IF EXISTS {update_trigger} ON bulk_manifests; \
                 DROP FUNCTION IF EXISTS public.{update_fn}();"
            ))
            .await
            .unwrap();

        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    &[output.clone(), error.clone()],
                    ManifestPublicationStatus::Completed,
                )
                .await
                .unwrap(),
            ManifestPublicationResult::Published
        );
        let rows = backend
            .list_submit_files(&tenant, &submission)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        for expected in [&output, &error] {
            let row = rows
                .iter()
                .find(|row| row.file_type == expected.file_type)
                .expect("published row");
            assert_eq!(row.file_path, expected.file_path);
            assert_eq!(row.fencing_token, lease.fencing_token);
        }
        let manifest = backend
            .get_manifest(&tenant, &submission, &lease.manifest_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(manifest.status, ManifestStatus::Completed);
        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    &[error.clone(), output],
                    ManifestPublicationStatus::Completed,
                )
                .await
                .unwrap(),
            ManifestPublicationResult::AlreadyPublished
        );
    }

    #[tokio::test]
    async fn postgres_publication_expiry_reclaim_scope_and_replay() {
        use helios_persistence::core::{
            BulkSubmitProvider, LeaseError, ManifestPublicationResult, ManifestPublicationStatus,
            ManifestStatus, SubmissionId, SubmitFileRecord, SubmitWorkerStorage,
        };

        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("publication-expiry-scope");
        let submission = SubmissionId::generate("publication-expiry-scope");
        backend
            .create_submission(&tenant, &submission, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(
                &tenant,
                &submission,
                Some("https://provider/publication-expiry-scope.json"),
                None,
            )
            .await
            .unwrap();
        let expired_unreclaimed = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "publication-expiry-worker-{}",
                uuid::Uuid::new_v4().simple()
            )),
            &submission,
            &manifest.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        let client = backend.get_client().await.unwrap();
        let expired_tenant_id = expired_unreclaimed.tenant.tenant_id().as_str();
        client
            .execute(
                "UPDATE bulk_manifests SET lease_expiry = NOW() - INTERVAL '1 second' \
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 \
                   AND manifest_id = $4",
                &[
                    &expired_tenant_id as &(dyn tokio_postgres::types::ToSql + Sync),
                    &expired_unreclaimed.submission_id.submitter,
                    &expired_unreclaimed.submission_id.submission_id,
                    &expired_unreclaimed.manifest_id,
                ],
            )
            .await
            .unwrap();
        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &expired_unreclaimed,
                    &[],
                    ManifestPublicationStatus::Completed,
                )
                .await
                .unwrap(),
            ManifestPublicationResult::Published,
            "expiry alone does not fence an unreclaimed live lease"
        );

        let stale_url = "https://provider/publication-expiry-scope-stale.json";
        let reclaim_seed = backend
            .add_manifest(&tenant, &submission, Some(stale_url), None)
            .await
            .unwrap();
        let stale = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "publication-stale-worker-{}",
                uuid::Uuid::new_v4().simple()
            )),
            &submission,
            &reclaim_seed.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        let stale_tenant_id = stale.tenant.tenant_id().as_str();
        client
            .execute(
                "UPDATE bulk_manifests SET lease_expiry = NOW() - INTERVAL '1 second' \
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3 \
                   AND manifest_id = $4",
                &[
                    &stale_tenant_id as &(dyn tokio_postgres::types::ToSql + Sync),
                    &stale.submission_id.submitter,
                    &stale.submission_id.submission_id,
                    &stale.manifest_id,
                ],
            )
            .await
            .unwrap();
        let lease = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "publication-live-worker-{}",
                uuid::Uuid::new_v4().simple()
            )),
            &submission,
            &reclaim_seed.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        assert!(lease.fencing_token > stale.fencing_token);
        let error = SubmitFileRecord {
            manifest_url: Some(stale_url.to_string()),
            file_type: "error".to_string(),
            resource_type: Some("OperationOutcome".to_string()),
            part_index: 0,
            file_path: format!("error/{}.ndjson", lease.manifest_id),
            line_count: 2,
            byte_count: 128,
            count_severity: Some(serde_json::json!({"error": 2})),
        };
        let failure_message = format!("reclaimed publication {}", lease.fencing_token);
        assert!(matches!(
            backend
                .publish_manifest_artifacts(
                    &stale,
                    std::slice::from_ref(&error),
                    ManifestPublicationStatus::Failed {
                        error_message: failure_message.clone(),
                    },
                )
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));
        assert!(
            backend
                .list_submit_files(&tenant, &submission)
                .await
                .unwrap()
                .is_empty(),
            "the stale publication must not expose its rejected set"
        );
        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    std::slice::from_ref(&error),
                    ManifestPublicationStatus::Failed {
                        error_message: failure_message.clone(),
                    },
                )
                .await
                .unwrap(),
            ManifestPublicationResult::Published
        );
        let live_manifest = backend
            .get_manifest(&tenant, &submission, &lease.manifest_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(live_manifest.status, ManifestStatus::Failed);
        let rows = backend
            .list_submit_files(&tenant, &submission)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.file_type, "error");
        assert_eq!(row.resource_type.as_deref(), Some("OperationOutcome"));
        assert_eq!(row.part_index, 0);
        assert_eq!(row.fencing_token, lease.fencing_token);
        assert_eq!(row.file_path, error.file_path);
        assert_eq!(row.line_count, 2);
        assert_eq!(row.byte_count, 128);
        assert_eq!(row.count_severity, Some(serde_json::json!({"error": 2})));
        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    std::slice::from_ref(&error),
                    ManifestPublicationStatus::Failed {
                        error_message: failure_message.clone(),
                    },
                )
                .await
                .unwrap(),
            ManifestPublicationResult::AlreadyPublished
        );
        assert!(matches!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    std::slice::from_ref(&error),
                    ManifestPublicationStatus::Failed {
                        error_message: "different terminal message".to_string(),
                    },
                )
                .await,
            Err(LeaseError::Storage(_))
        ));

        let coexist_url = "https://provider/publication-expiry-scope-coexist.json";
        let coexist_manifest = backend
            .add_manifest(&tenant, &submission, Some(coexist_url), None)
            .await
            .unwrap();
        let coexist_lease = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "publication-coexist-worker-{}",
                uuid::Uuid::new_v4().simple()
            )),
            &submission,
            &coexist_manifest.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        let mut coexist_lease = coexist_lease;
        coexist_lease.fencing_token = lease.fencing_token;
        client
            .execute(
                "UPDATE bulk_manifests SET fencing_token = $1 \
                 WHERE tenant_id = $2 AND submitter = $3 AND submission_id = $4 \
                   AND manifest_id = $5",
                &[
                    &(coexist_lease.fencing_token as i64),
                    &coexist_lease.tenant.tenant_id().as_str(),
                    &coexist_lease.submission_id.submitter,
                    &coexist_lease.submission_id.submission_id,
                    &coexist_lease.manifest_id,
                ],
            )
            .await
            .unwrap();
        let coexist = SubmitFileRecord {
            manifest_url: Some(coexist_url.to_string()),
            file_path: format!("error/{}.ndjson", coexist_lease.manifest_id),
            ..error.clone()
        };
        backend
            .record_submit_file(&coexist_lease, &coexist)
            .await
            .unwrap();
        assert_eq!(
            backend
                .publish_manifest_artifacts(
                    &coexist_lease,
                    std::slice::from_ref(&coexist),
                    ManifestPublicationStatus::Completed,
                )
                .await
                .unwrap(),
            ManifestPublicationResult::Published
        );
        let rows = backend
            .list_submit_files(&tenant, &submission)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.file_type == "error"
            && row.resource_type.as_deref() == Some("OperationOutcome")
            && row.part_index == 0
            && row.fencing_token == lease.fencing_token));
        assert_eq!(
            rows.iter()
                .map(|row| row.manifest_id.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some(lease.manifest_id.as_str()),
                Some(coexist_lease.manifest_id.as_str())
            ]
        );
        let other_tenant = create_tenant("publication-scope-other");
        assert!(
            backend
                .list_submit_files(&other_tenant, &submission)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            backend
                .list_submit_files(&tenant, &SubmissionId::generate("publication-scope-other"))
                .await
                .unwrap()
                .is_empty()
        );
        let _ = client;
    }

    #[tokio::test]
    async fn postgres_publication_concurrency_and_cleanup_rollback() {
        use helios_persistence::core::{
            BulkSubmitProvider, LeaseError, ManifestPublicationResult, ManifestPublicationStatus,
            SubmissionId, SubmitFileRecord, SubmitWorkerStorage,
        };

        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("publication-cleanup");
        let submission = SubmissionId::generate("publication-cleanup");
        let manifest_url = "https://provider/publication-cleanup.json";
        backend
            .create_submission(&tenant, &submission, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &submission, Some(manifest_url), None)
            .await
            .unwrap();
        let lease = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "publication-cleanup-worker-{}",
                uuid::Uuid::new_v4().simple()
            )),
            &submission,
            &manifest.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        let output = SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: "output".to_string(),
            resource_type: Some("Patient".to_string()),
            part_index: 0,
            file_path: "output/patient-0.ndjson".to_string(),
            line_count: 3,
            byte_count: 128,
            count_severity: None,
        };
        let error = SubmitFileRecord {
            manifest_url: Some(manifest_url.to_string()),
            file_type: "error".to_string(),
            resource_type: Some("OperationOutcome".to_string()),
            part_index: 0,
            file_path: "error/outcome-0.ndjson".to_string(),
            line_count: 1,
            byte_count: 96,
            count_severity: Some(serde_json::json!({"error": 2})),
        };
        backend.record_submit_file(&lease, &output).await.unwrap();
        backend.record_submit_file(&lease, &error).await.unwrap();
        let set = [output.clone(), error.clone()];
        let (first, second) = tokio::join!(
            backend.publish_manifest_artifacts(&lease, &set, ManifestPublicationStatus::Completed,),
            backend.publish_manifest_artifacts(&lease, &set, ManifestPublicationStatus::Completed,),
        );
        let results = [first.unwrap(), second.unwrap()];
        assert_eq!(
            results
                .iter()
                .filter(|result| **result == ManifestPublicationResult::Published)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| **result == ManifestPublicationResult::AlreadyPublished)
                .count(),
            1
        );

        let client = backend.get_client().await.unwrap();
        let tenant_id = lease.tenant.tenant_id().as_str();
        let where_clause = "tenant_id = $1 AND submitter = $2 AND submission_id = $3";
        let identity: [&(dyn tokio_postgres::types::ToSql + Sync); 3] = [
            &tenant_id,
            &lease.submission_id.submitter,
            &lease.submission_id.submission_id,
        ];
        let cleanup_fn = format!("publication_cleanup_fn_{}", uuid::Uuid::new_v4().simple());
        let cleanup_trigger = format!(
            "publication_cleanup_trigger_{}",
            uuid::Uuid::new_v4().simple()
        );
        client
            .batch_execute(&format!(
                r#"
                CREATE FUNCTION public.{cleanup_fn}() RETURNS trigger LANGUAGE plpgsql AS $body$
                BEGIN
                    RAISE EXCEPTION 'cleanup injected terminal failure';
                END;
                $body$;
                CREATE TRIGGER {cleanup_trigger} BEFORE UPDATE OF published_token ON bulk_manifests
                FOR EACH ROW
                WHEN (OLD.published_token IS NOT NULL AND NEW.published_token IS NULL
                      AND NEW.tenant_id = '{tenant_id}'
                      AND NEW.submitter = '{submitter}'
                      AND NEW.submission_id = '{submission_id}')
                EXECUTE FUNCTION public.{cleanup_fn}();
                "#,
                tenant_id = lease.tenant.tenant_id(),
                submitter = lease.submission_id.submitter,
                submission_id = lease.submission_id.submission_id,
            ))
            .await
            .unwrap();
        let Err(storage_error) = backend
            .delete_submission_artifacts(&tenant, &submission)
            .await
        else {
            panic!("cleanup marker reset must fail while the trigger is installed");
        };
        assert!(
            storage_error
                .to_string()
                .contains("clear publication markers:"),
            "PG Display must retain the backend stage context, got {storage_error}"
        );
        let raw_count = client
            .query_one(
                &format!("SELECT count(*) FROM bulk_submit_files WHERE {where_clause}"),
                &identity,
            )
            .await
            .unwrap()
            .get::<_, i64>(0);
        assert_eq!(raw_count, 2);
        let marker = client
            .query_one(
                &format!(
                    "SELECT published_token, publication_status, publication_error_message, \
                     publication_worker_id FROM bulk_manifests WHERE {where_clause}"
                ),
                &identity,
            )
            .await
            .unwrap();
        assert_eq!(
            marker.get::<_, Option<i64>>(0),
            Some(lease.fencing_token as i64)
        );
        assert_eq!(
            marker.get::<_, Option<String>>(1),
            Some("completed".to_string())
        );
        assert_eq!(marker.get::<_, Option<String>>(2), None);
        assert_eq!(
            marker.get::<_, Option<String>>(3),
            Some(lease.worker_id.as_str().to_string())
        );
        client
            .batch_execute(&format!(
                "DROP TRIGGER IF EXISTS {cleanup_trigger} ON bulk_manifests; \
                 DROP FUNCTION IF EXISTS public.{cleanup_fn}();"
            ))
            .await
            .unwrap();
        backend
            .delete_submission_artifacts(&tenant, &submission)
            .await
            .unwrap();
        let raw_count = client
            .query_one(
                &format!("SELECT count(*) FROM bulk_submit_files WHERE {where_clause}"),
                &identity,
            )
            .await
            .unwrap()
            .get::<_, i64>(0);
        assert_eq!(raw_count, 0);
        assert!(matches!(
            backend
                .publish_manifest_artifacts(
                    &lease,
                    &[output, error],
                    ManifestPublicationStatus::Completed,
                )
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));
    }

    #[tokio::test]
    async fn postgres_replaced_manifest_process_entries_does_not_revive_publication() {
        use helios_persistence::core::LeaseError;
        use helios_persistence::core::{
            BulkSubmitProvider, ManifestPublicationStatus, ManifestStatus, SubmissionId,
            SubmitWorkerStorage,
        };

        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("replacement-guard");
        let submission = SubmissionId::generate("replacement-guard");
        let manifest_url = "https://provider/replacement-guard.json";
        backend
            .create_submission(&tenant, &submission, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &submission, Some(manifest_url), None)
            .await
            .unwrap();
        let lease = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "replacement-guard-worker-{}",
                uuid::Uuid::new_v4().simple()
            )),
            &submission,
            &manifest.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        backend
            .replace_manifest_by_url(&tenant, &submission, manifest_url)
            .await
            .unwrap();

        backend
            .process_entries(
                &tenant,
                &submission,
                &manifest.manifest_id,
                Vec::new(),
                &helios_persistence::core::BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        let manifest = backend
            .get_manifest(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            manifest.status,
            ManifestStatus::Replaced,
            "an empty ingest must not revive a replaced manifest"
        );
        assert!(
            matches!(
                backend
                    .publish_manifest_artifacts(&lease, &[], ManifestPublicationStatus::Completed,)
                    .await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "the old lease must lose publication after replacement"
        );
    }

    mod receipt_paging_contract {
        use helios_persistence as persistence;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/bulk_submit/paging_contract.rs"
        ));
    }

    mod receipt_consumer_contract {
        use helios_persistence as persistence;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/bulk_submit/consumer_contract.rs"
        ));
    }

    #[tokio::test]
    async fn postgres_bulk_submit_worker_exact_artifacts_across_pages() {
        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        receipt_consumer_contract::worker_receipts(
            std::sync::Arc::new(create_backend().await),
            &create_tenant("receipt-worker"),
        )
        .await;
    }

    #[tokio::test]
    async fn postgres_bulk_submit_composite_deduplicates_all_pages_on_finish_and_failure() {
        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        for (fail, secondary_failure) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            receipt_consumer_contract::composite_receipts(
                std::sync::Arc::new(create_backend().await),
                &create_tenant("receipt-composite"),
                BackendKind::Postgres,
                fail,
                secondary_failure,
            )
            .await;
        }
    }

    #[async_trait::async_trait]
    impl receipt_paging_contract::ReceiptFixture for PostgresBackend {
        async fn seed_receipts(
            &self,
            tenant: &TenantContext,
            submission: &helios_persistence::core::SubmissionId,
            manifest: &str,
            rows: &[receipt_paging_contract::ReceiptRow],
        ) {
            let client = self.get_client().await.unwrap();
            let tid = tenant.tenant_id().as_str();
            client.execute("INSERT INTO bulk_submissions (tenant_id,submitter,submission_id,status,created_at,updated_at) VALUES ($1,$2,$3,'complete',NOW(),NOW()) ON CONFLICT DO NOTHING", &[&tid, &submission.submitter, &submission.submission_id]).await.unwrap();
            client.execute("INSERT INTO bulk_manifests (tenant_id,submitter,submission_id,manifest_id,status,added_at) VALUES ($1,$2,$3,$4,'completed',NOW()) ON CONFLICT DO NOTHING", &[&tid, &submission.submitter, &submission.submission_id, &manifest]).await.unwrap();
            for row in rows {
                let line = i32::try_from(row.line).unwrap();
                client.execute("INSERT INTO bulk_entry_results (tenant_id,submitter,submission_id,manifest_id,file_url,line_number,resource_type,resource_id,outcome) VALUES ($1,$2,$3,$4,$5,$6,'Patient',$7,$8)", &[&tid, &submission.submitter, &submission.submission_id, &manifest, &row.file, &line, &row.id, &row.outcome]).await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn postgres_bulk_submit_exact_keyset_pages() {
        receipt_paging_contract::exact_keyset_pages(
            &create_backend().await,
            &create_tenant("receipt-pages"),
            i64::from(i32::MAX),
        )
        .await;
    }

    /// Shared PostgreSQL container reused across all tests in this module.
    struct SharedPg {
        host: String,
        port: u16,
        /// Kept alive for the duration of the test binary. NOTE: a `static` is
        /// never dropped, so `Drop for ContainerAsync` — testcontainers' only
        /// container-removal path — never runs. The container outlives the test
        /// process and is reaped in CI by its `github.run_id` label.
        _container: testcontainers::ContainerAsync<Postgres>,
    }

    static SHARED_PG: OnceCell<SharedPg> = OnceCell::const_new();
    static BULK_EXPORT_TEST_LOCK: Mutex<()> = Mutex::const_new(());
    static BULK_SUBMIT_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    async fn shared_pg() -> &'static SharedPg {
        SHARED_PG
            .get_or_init(|| async {
                let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
                // Pin the major version. testcontainers-modules defaults to
                // postgres:11, which is EOL and predates `plan_cache_mode` — a GUC
                // the backend sends as a startup option, so PG 11 rejects every
                // connection FATAL. The rest of the repo runs 16.
                let container = Postgres::default()
                    .with_tag("16-alpine")
                    .with_label("github.run_id", &run_id)
                    .start()
                    .await
                    .expect("Failed to start PostgreSQL container");

                let port = container
                    .get_host_port_ipv4(5432)
                    .await
                    .expect("Failed to get host port");

                let host = container
                    .get_host()
                    .await
                    .expect("Failed to get host")
                    .to_string();

                // Initialize schema once on the shared container.
                let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.join("data"))
                    .unwrap_or_else(|| PathBuf::from("data"));

                let config = PostgresConfig {
                    host: host.clone(),
                    port,
                    dbname: "postgres".to_string(),
                    user: "postgres".to_string(),
                    password: Some("postgres".to_string()),
                    max_connections: 5,
                    data_dir: Some(data_dir),
                    ..Default::default()
                };

                let backend = PostgresBackend::new(config)
                    .await
                    .expect("Failed to create PostgresBackend");

                backend
                    .init_schema()
                    .await
                    .expect("Failed to initialize schema");

                SharedPg {
                    host,
                    port,
                    _container: container,
                }
            })
            .await
    }

    /// Creates a PostgresBackend connected to the shared testcontainers PostgreSQL instance.
    ///
    /// Schema is initialized once when the shared container starts; `init_schema()` is
    /// idempotent (uses CREATE TABLE IF NOT EXISTS).
    async fn create_backend() -> PostgresBackend {
        create_backend_with_max_connections(5).await
    }

    async fn create_backend_with_max_connections(max_connections: usize) -> PostgresBackend {
        let pg = shared_pg().await;

        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections,
            data_dir: Some(data_dir),
            ..Default::default()
        };

        PostgresBackend::new(config)
            .await
            .expect("Failed to create PostgresBackend")
    }

    /// Creates a tenant with a unique ID suffix to isolate tests sharing the same database.
    fn create_tenant(id: &str) -> TenantContext {
        let unique_id = format!("{}_{}", id, uuid::Uuid::new_v4().simple());
        TenantContext::new(TenantId::new(&unique_id), TenantPermissions::full_access())
    }

    #[tokio::test]
    async fn postgres_bulk_submit_status_uses_the_complete_identity_and_preserves_summary() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use helios_persistence::composite::{
            CompositeConfig, CompositeStorage, CompositeSubmitJobs, DynStorage,
        };
        use helios_persistence::core::{
            BulkSubmitJobStore, BulkSubmitProvider, SubmissionId, SubmissionStatus,
        };

        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = Arc::new(create_backend().await);
        let tenant = create_tenant("bulk-submit-status");
        let other_tenant = create_tenant("bulk-submit-status-other-tenant");
        let submission = SubmissionId::new("status-submitter", "status-id");
        let other_submitter = SubmissionId::new("other-submitter", "status-id");
        let other_submission = SubmissionId::new("status-submitter", "other-status-id");
        let metadata = json!({"source": "status-test"});

        backend
            .create_submission(&tenant, &submission, Some(metadata.clone()))
            .await
            .unwrap();
        backend
            .create_submission(&other_tenant, &submission, None)
            .await
            .unwrap();
        backend
            .create_submission(&tenant, &other_submitter, None)
            .await
            .unwrap();
        backend
            .create_submission(&tenant, &other_submission, None)
            .await
            .unwrap();

        let client = backend.get_client().await.unwrap();
        for (row_tenant, id, status) in [
            (&other_tenant, &submission, "complete"),
            (&tenant, &other_submitter, "aborted"),
            (&tenant, &other_submission, "complete"),
        ] {
            let tenant_id = row_tenant.tenant_id().as_str();
            client
                .execute(
                    "UPDATE bulk_submissions SET status = $4
                     WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                    &[
                        &tenant_id,
                        &id.submitter.as_str(),
                        &id.submission_id.as_str(),
                        &status,
                    ],
                )
                .await
                .unwrap();
        }

        assert_eq!(
            backend
                .get_submission_status(&tenant, &submission)
                .await
                .unwrap(),
            Some(SubmissionStatus::InProgress)
        );
        assert_eq!(
            backend
                .get_submission_status(&other_tenant, &submission)
                .await
                .unwrap(),
            Some(SubmissionStatus::Complete)
        );
        assert_eq!(
            backend
                .get_submission_status(&tenant, &other_submitter)
                .await
                .unwrap(),
            Some(SubmissionStatus::Aborted)
        );
        assert_eq!(
            backend
                .get_submission_status(&tenant, &other_submission)
                .await
                .unwrap(),
            Some(SubmissionStatus::Complete)
        );
        assert_eq!(
            backend
                .get_submission_status(&tenant, &SubmissionId::new("status-submitter", "missing"),)
                .await
                .unwrap(),
            None
        );

        let manifest = backend
            .add_manifest(
                &tenant,
                &submission,
                Some("https://provider/status.json"),
                None,
            )
            .await
            .unwrap();
        let tenant_id = tenant.tenant_id().as_str();
        for (line, outcome) in [
            (1_i32, "success"),
            (2, "validation-error"),
            (3, "processing-error"),
            (4, "skipped"),
        ] {
            let file_url = format!("https://provider/status-{line}.ndjson");
            let resource_id = format!("status-{line}");
            client
                .execute(
                    "INSERT INTO bulk_entry_results
                     (tenant_id, submitter, submission_id, manifest_id, file_url,
                      line_number, resource_type, resource_id, outcome)
                     VALUES ($1, $2, $3, $4, $5, $6, 'Patient', $7, $8)",
                    &[
                        &tenant_id,
                        &submission.submitter.as_str(),
                        &submission.submission_id.as_str(),
                        &manifest.manifest_id.as_str(),
                        &file_url,
                        &line,
                        &resource_id,
                        &outcome,
                    ],
                )
                .await
                .unwrap();
        }
        let summary = backend
            .get_submission(&tenant, &submission)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.metadata, Some(metadata));
        assert_eq!(summary.manifest_count, 1);
        assert_eq!(summary.total_entries, 4);
        assert_eq!(summary.success_count, 1);
        assert_eq!(summary.error_count, 2);
        assert_eq!(summary.skipped_count, 1);

        let config = CompositeConfig::builder()
            .primary("postgres", BackendKind::Postgres)
            .build()
            .unwrap();
        let mut backends = HashMap::new();
        backends.insert("postgres".to_string(), backend.clone() as DynStorage);
        let composite = Arc::new(CompositeStorage::new(config, backends).unwrap());
        let jobs =
            CompositeSubmitJobs::new(backend.clone() as Arc<dyn BulkSubmitJobStore>, composite);
        assert_eq!(
            jobs.get_submission_status(&tenant, &submission)
                .await
                .unwrap(),
            Some(SubmissionStatus::InProgress)
        );

        let corrupt = SubmissionId::generate("corrupt-status");
        backend
            .create_submission(&tenant, &corrupt, None)
            .await
            .unwrap();
        client
            .execute(
                "UPDATE bulk_submissions SET status = 'invalid-status'
                 WHERE tenant_id = $1 AND submitter = $2 AND submission_id = $3",
                &[
                    &tenant_id,
                    &corrupt.submitter.as_str(),
                    &corrupt.submission_id.as_str(),
                ],
            )
            .await
            .unwrap();
        let error = backend
            .get_submission_status(&tenant, &corrupt)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Invalid status: invalid-status"));
    }

    #[tokio::test]
    async fn statement_timeout_applies_to_every_pooled_connection() {
        let pg = shared_pg().await;
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));

        const POOL_SIZE: usize = 10;
        const TIMEOUT_MS: u64 = 250;

        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: POOL_SIZE,
            statement_timeout_ms: TIMEOUT_MS,
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let backend =
            std::sync::Arc::new(PostgresBackend::new(config).await.expect("create backend"));

        // Hold POOL_SIZE clients across a barrier so the pool is forced to open
        // every physical connection before any is released. deadpool creates
        // connections lazily, so a serial check could pass while exercising only
        // one connection. The regression this guards (#285): the pre-fix code ran
        // `SET statement_timeout` on the single connection borrowed inside
        // `PostgresBackend::new`, so every connection created lazily afterwards
        // inherited the server default (usually 0 = uncapped). Shipping the GUC
        // in the connection startup packet makes every connection carry it, which
        // is what each task asserts below.
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(POOL_SIZE));
        let mut handles = Vec::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            let backend = backend.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                let client = backend.get_client().await.expect("get_client");
                barrier.wait().await;
                let row = client
                    .query_one("SELECT current_setting('statement_timeout')", &[])
                    .await
                    .expect("current_setting");
                let value: String = row.get(0);
                value
            }));
        }

        for (i, h) in handles.into_iter().enumerate() {
            let value = h.await.expect("task panicked");
            assert_eq!(
                value,
                format!("{TIMEOUT_MS}ms"),
                "pooled connection #{i} reported statement_timeout={value:?}, \
                 expected {TIMEOUT_MS}ms — the GUC did not reach every connection"
            );
        }
    }

    /// A statement cancelled by `statement_timeout` must classify as
    /// [`BackendError::Timeout`] (→ HTTP 504), not `Internal` (→ 500).
    ///
    /// Regression for issue #353. `tokio_postgres::Error` has no public
    /// constructor, so the SQLSTATE-classification path can only be exercised
    /// against a live server — hence a testcontainer test rather than a unit
    /// test. `SELECT pg_sleep()` is the cheapest statement guaranteed to
    /// outlive the deadline.
    ///
    /// Note this asserts on the SQLSTATE (`57014`) reaching the classifier, not
    /// on the driver's message text: PostgreSQL localizes error messages via
    /// `lc_messages`, so matching the English string would make this test (and
    /// the classifier it guards) locale-dependent.
    #[tokio::test]
    async fn statement_timeout_cancellation_classifies_as_backend_timeout() {
        use helios_persistence::error::classify_postgres_error;

        let pg = shared_pg().await;
        const TIMEOUT_MS: u64 = 250;

        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            statement_timeout_ms: TIMEOUT_MS,
            ..Default::default()
        };
        let backend = PostgresBackend::new(config).await.expect("create backend");
        let client = backend.get_client().await.expect("get_client");

        // Sleep well past the 250ms budget so the server cancels us.
        let err = client
            .query("SELECT pg_sleep(5)", &[])
            .await
            .expect_err("pg_sleep(5) must be cancelled by a 250ms statement_timeout");

        assert_eq!(
            err.code().map(|c| c.code()),
            Some("57014"),
            "expected SQLSTATE 57014 query_canceled, got {err}"
        );

        let classified = classify_postgres_error("Failed to execute search", err);
        match classified {
            BackendError::Timeout {
                ref backend_name,
                ref message,
            } => {
                assert_eq!(backend_name, "postgres");
                assert!(
                    message.starts_with("Failed to execute search: "),
                    "caller context must survive classification, got {message:?}"
                );
            }
            other => panic!(
                "statement_timeout cancellation must classify as BackendError::Timeout \
                 (HTTP 504), got {other:?} — this is the #353 regression"
            ),
        }

        // Call sites that add no context of their own convert with a bare `?`,
        // which goes through `impl From<tokio_postgres::Error> for StorageError`
        // rather than the classifier directly. That path must classify
        // identically, or the fix would hold only for the sites that happen to
        // pass a context string.
        let err = client
            .query("SELECT pg_sleep(5)", &[])
            .await
            .expect_err("pg_sleep(5) must be cancelled by a 250ms statement_timeout");
        let converted: StorageError = err.into();
        assert!(
            matches!(
                converted,
                StorageError::Backend(BackendError::Timeout { .. })
            ),
            "the `?` conversion must classify too, got {converted:?}"
        );
    }

    // ========================================================================
    // CRUD Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_create_resource() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "Smith", "given": ["John"]}]
        });

        let result = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await;
        assert!(result.is_ok(), "Create failed: {:?}", result.err());

        let created = result.unwrap();
        assert_eq!(created.resource_type(), "Patient");
        assert!(!created.id().is_empty());
        assert_eq!(created.version_id(), "1");
    }

    #[tokio::test]
    async fn postgres_integration_create_with_id() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "patient-123",
            "name": [{"family": "Jones"}]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(created.id(), "patient-123");
    }

    #[tokio::test]
    async fn postgres_integration_create_duplicate_fails() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "duplicate-id"
        });

        backend
            .create(&tenant, "Patient", patient.clone(), FhirVersion::default())
            .await
            .unwrap();

        let result = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn postgres_integration_read_resource() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "ReadTest"}]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap();
        assert!(read.is_some());

        let resource = read.unwrap();
        assert_eq!(resource.id(), created.id());
        assert_eq!(resource.content()["name"][0]["family"], "ReadTest");
    }

    #[tokio::test]
    async fn postgres_integration_read_nonexistent() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let read = backend
            .read(&tenant, "Patient", "does-not-exist")
            .await
            .unwrap();
        assert!(read.is_none());
    }

    #[tokio::test]
    async fn postgres_integration_exists() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        assert!(
            backend
                .exists(&tenant, "Patient", created.id())
                .await
                .unwrap()
        );
        assert!(
            !backend
                .exists(&tenant, "Patient", "nonexistent")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn postgres_integration_update_resource() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "Original"}]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let updated_content = json!({
            "resourceType": "Patient",
            "name": [{"family": "Updated"}]
        });

        let updated = backend
            .update(&tenant, &created, updated_content)
            .await
            .unwrap();

        assert_eq!(updated.version_id(), "2");
        assert_eq!(updated.content()["name"][0]["family"], "Updated");
    }

    #[tokio::test]
    async fn postgres_integration_create_or_update() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Create via upsert
        let patient = json!({"resourceType": "Patient", "name": [{"family": "First"}]});
        let (resource, was_created) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "upsert-id",
                patient,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(was_created);
        assert_eq!(resource.id(), "upsert-id");

        // Update via upsert
        let patient2 = json!({"resourceType": "Patient", "name": [{"family": "Second"}]});
        let (resource2, was_created2) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "upsert-id",
                patient2,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(!was_created2);
        assert_eq!(resource2.content()["name"][0]["family"], "Second");
    }

    /// A `PUT` onto a deleted id restores the resource instead of failing.
    ///
    /// FHIR permits a deleted resource to be brought back by a subsequent
    /// update (http.html#delete). The restore continues the existing version
    /// chain — v1 create, v2 delete, v3 restore — rather than resetting to
    /// "1", and the resource is readable and searchable again afterwards.
    /// This mirrors `crud::delete_tests::test_delete_is_soft_delete`, which
    /// covers the same path on SQLite.
    #[tokio::test]
    async fn postgres_integration_create_or_update_restores_deleted() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let created = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Original"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = created.id().to_string();
        assert_eq!(created.version_id(), "1");

        backend.delete(&tenant, "Patient", &id).await.unwrap();

        let (restored, _created_new) = backend
            .create_or_update(
                &tenant,
                "Patient",
                &id,
                json!({"resourceType": "Patient", "name": [{"family": "Restored"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert_eq!(restored.content()["name"][0]["family"], "Restored");
        assert_eq!(
            restored.version_id(),
            "3",
            "restore should continue the version chain (v1 create, v2 delete, v3 restore)"
        );
        assert!(!restored.is_deleted());

        // The resource is live again: readable, and the restore is the current
        // version.
        let read = backend
            .read(&tenant, "Patient", &id)
            .await
            .unwrap()
            .expect("restored resource must be readable");
        assert_eq!(read.version_id(), "3");
        assert_eq!(read.content()["name"][0]["family"], "Restored");
        assert!(backend.exists(&tenant, "Patient", &id).await.unwrap());

        // History keeps every version, including the deletion — which is only
        // returned when `include_deleted` is set (deleted versions are filtered out
        // by default on every backend).
        let history = backend
            .history_instance(
                &tenant,
                "Patient",
                &id,
                &HistoryParams::new().include_deleted(true),
            )
            .await
            .unwrap();
        assert_eq!(
            history.items.len(),
            3,
            "history should hold create, delete and restore"
        );
        assert_eq!(history.items[0].resource.version_id(), "3");
        assert!(!history.items[0].resource.is_deleted());
        assert!(
            history.items[1].resource.is_deleted(),
            "the middle version is the deletion"
        );
    }

    /// Restoring a deleted resource requires update permission.
    #[tokio::test]
    async fn postgres_integration_restore_deleted_requires_permission() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let created = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = created.id().to_string();
        backend.delete(&tenant, "Patient", &id).await.unwrap();

        // Same tenant, read-only permissions.
        let read_only =
            TenantContext::new(tenant.tenant_id().clone(), TenantPermissions::read_only());
        let result = backend
            .create_or_update(
                &read_only,
                "Patient",
                &id,
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await;
        assert!(
            matches!(&result, Err(StorageError::Tenant(_))),
            "restore without update permission must be refused, got {:?}",
            result.as_ref().map(|(r, _)| r.version_id())
        );
    }

    /// A restored SearchParameter re-enters the tenant's registry overlay.
    ///
    /// Deleting a custom SearchParameter unregisters it; bringing it back with
    /// a PUT has to reload the stored-parameter cache the way a create does,
    /// or the parameter stays invisible to search until the process restarts.
    #[tokio::test]
    async fn postgres_integration_restored_search_parameter_reenters_registry() {
        use helios_persistence::core::SearchProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let search_param = json!({
            "resourceType": "SearchParameter",
            "id": "pg-restore-sp",
            "url": "http://example.org/fhir/SearchParameter/pg-restore-sp",
            "name": "pgrestoresp",
            "status": "active",
            "code": "pgrestoresp",
            "base": ["Observation"],
            "type": "token",
            "expression": "Observation.code"
        });

        backend
            .create_or_update(
                &tenant,
                "SearchParameter",
                "pg-restore-sp",
                search_param.clone(),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        {
            let reg = backend.search_param_registry(&tenant);
            let registry = reg.read();
            assert!(registry.get_param("Observation", "pgrestoresp").is_some());
        }

        backend
            .delete(&tenant, "SearchParameter", "pg-restore-sp")
            .await
            .unwrap();
        {
            let reg = backend.search_param_registry(&tenant);
            let registry = reg.read();
            assert!(
                registry.get_param("Observation", "pgrestoresp").is_none(),
                "deleted SearchParameter should be unregistered"
            );
        }

        backend
            .create_or_update(
                &tenant,
                "SearchParameter",
                "pg-restore-sp",
                search_param,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let reg = backend.search_param_registry(&tenant);
        let registry = reg.read();
        assert!(
            registry.get_param("Observation", "pgrestoresp").is_some(),
            "restored SearchParameter should be registered again"
        );
    }

    #[tokio::test]
    async fn postgres_integration_delete_resource() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        backend
            .delete(&tenant, "Patient", created.id())
            .await
            .unwrap();

        let read_result = backend.read(&tenant, "Patient", created.id()).await;
        match read_result {
            Err(StorageError::Resource(ResourceError::Gone { .. })) => {}
            Ok(None) => {}
            other => {
                panic!("Expected Gone error or None, got: {:?}", other);
            }
        }
    }

    /// A tombstone's version comes from the row, not from an earlier read.
    ///
    /// `delete` used to `SELECT version_id`, add one in Rust, and
    /// compare-and-swap. It now computes the increment inside the `UPDATE`'s
    /// target list, so this pins the arithmetic that replaced the Rust: after
    /// two updates the live row is v3 and the tombstone must be v4, with a
    /// contiguous, duplicate-free version chain behind it.
    #[tokio::test]
    async fn postgres_integration_delete_tombstone_version_follows_the_row() {
        use helios_persistence::core::VersionedStorage;

        let backend = create_backend().await;
        let tenant = create_tenant("delete-tombstone-version");

        let created = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "active": true}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(created.version_id(), "1");

        let v2 = backend
            .update(
                &tenant,
                &created,
                json!({"resourceType": "Patient", "active": false}),
            )
            .await
            .unwrap();
        let v3 = backend
            .update(
                &tenant,
                &v2,
                json!({"resourceType": "Patient", "active": true}),
            )
            .await
            .unwrap();
        assert_eq!(v3.version_id(), "3");

        backend
            .delete(&tenant, "Patient", created.id())
            .await
            .unwrap();

        let mut versions = backend
            .list_versions(&tenant, "Patient", created.id())
            .await
            .unwrap();
        versions.sort_by_key(|v| v.parse::<u64>().unwrap());
        assert_eq!(
            versions,
            vec!["1", "2", "3", "4"],
            "the tombstone must continue the chain at current + 1"
        );
    }

    /// Concurrent writers on one row must never produce an internal error.
    ///
    /// This is the failure the removed compare-and-swap existed to prevent: a
    /// version computed from a stale read colliding with one another writer had
    /// already inserted, tripping `resource_history`'s
    /// `PRIMARY KEY (tenant_id, resource_type, id, version_id)`. Deriving the
    /// version inside the statement makes that unreachable, because PostgreSQL
    /// re-evaluates the target list against the committed new tuple when it
    /// unblocks. Every outcome here must therefore be a success or an ordinary
    /// `NotFound`/`VersionConflict` — never a `Backend` error — and the surviving
    /// history must have no duplicate versions.
    #[tokio::test]
    async fn postgres_integration_concurrent_update_and_delete_never_collide() {
        use helios_persistence::core::VersionedStorage;
        use std::sync::Arc;

        let backend = Arc::new(create_backend().await);
        let tenant = create_tenant("concurrent-update-delete");

        for _ in 0..12 {
            let created = backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType": "Patient", "active": true}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            let id = created.id().to_string();

            let (b1, b2, t1, t2, id1, id2) = (
                backend.clone(),
                backend.clone(),
                tenant.clone(),
                tenant.clone(),
                id.clone(),
                id.clone(),
            );
            let updater = tokio::spawn(async move {
                b1.update(
                    &t1,
                    &created,
                    json!({"resourceType": "Patient", "active": false}),
                )
                .await
                .map(|_| ())
            });
            let deleter = tokio::spawn(async move { b2.delete(&t2, "Patient", &id1).await });

            for outcome in [updater.await.unwrap(), deleter.await.unwrap()] {
                match outcome {
                    Ok(())
                    | Err(StorageError::Resource(ResourceError::NotFound { .. }))
                    | Err(StorageError::Concurrency(ConcurrencyError::VersionConflict {
                        ..
                    })) => {}
                    other => panic!("concurrent update/delete produced {:?}", other),
                }
            }

            let versions = backend
                .list_versions(&tenant, "Patient", &id2)
                .await
                .unwrap();
            let mut unique = versions.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(
                unique.len(),
                versions.len(),
                "duplicate history versions for {id2}: {versions:?}"
            );
        }
    }

    #[tokio::test]
    async fn postgres_integration_delete_nonexistent_fails() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let result = backend.delete(&tenant, "Patient", "nonexistent").await;
        assert!(result.is_err());
    }

    // ========================================================================
    // Tenant Isolation Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_tenant_isolation() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Tenant A can see it
        assert!(
            backend
                .exists(&tenant_a, "Patient", created.id())
                .await
                .unwrap()
        );

        // Tenant B cannot see it
        assert!(
            !backend
                .exists(&tenant_b, "Patient", created.id())
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn postgres_integration_same_id_different_tenants() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient_a = json!({"resourceType": "Patient", "name": [{"family": "A"}]});
        let patient_b = json!({"resourceType": "Patient", "name": [{"family": "B"}]});

        backend
            .create_or_update(
                &tenant_a,
                "Patient",
                "shared-id",
                patient_a,
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create_or_update(
                &tenant_b,
                "Patient",
                "shared-id",
                patient_b,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let read_a = backend
            .read(&tenant_a, "Patient", "shared-id")
            .await
            .unwrap()
            .unwrap();
        let read_b = backend
            .read(&tenant_b, "Patient", "shared-id")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(read_a.content()["name"][0]["family"], "A");
        assert_eq!(read_b.content()["name"][0]["family"], "B");
    }

    // ========================================================================
    // Version Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_version_increments() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient"});
        let v1 = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(v1.version_id(), "1");

        let v2 = backend
            .update(&tenant, &v1, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        assert_eq!(v2.version_id(), "2");

        let v3 = backend
            .update(&tenant, &v2, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        assert_eq!(v3.version_id(), "3");
    }

    // ========================================================================
    // Count Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_count_resources() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for i in 0..5 {
            let patient = json!({"resourceType": "Patient", "id": format!("p{}", i)});
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        let count = backend.count(&tenant, Some("Patient")).await.unwrap();
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn postgres_integration_count_by_tenant() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        for _ in 0..3 {
            let patient = json!({"resourceType": "Patient"});
            backend
                .create(&tenant_a, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        for _ in 0..2 {
            let patient = json!({"resourceType": "Patient"});
            backend
                .create(&tenant_b, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 3);
        assert_eq!(backend.count(&tenant_b, Some("Patient")).await.unwrap(), 2);
    }

    // ========================================================================
    // Console Dashboard count_* Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_count_by_types() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-count-by-types");

        // Seed a small deterministic dataset: 2 Patients, 1 Observation.
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let counts = backend
            .count_by_types(&tenant, &["Patient", "Observation", "Encounter"])
            .await
            .unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get("Patient"), Some(&2));
        assert_eq!(map.get("Observation"), Some(&1));
        // A type with zero rows is ABSENT from the result, not a 0 row.
        assert!(!map.contains_key("Encounter"));
    }

    #[tokio::test]
    async fn postgres_integration_count_all_types() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-count-all-types");

        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let counts = backend.count_all_types(&tenant).await.unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get("Patient"), Some(&2));
        assert_eq!(map.get("Observation"), Some(&1));
    }

    /// #1078: the write marker is empty for a fresh tenant, changes on every
    /// create/update/delete, ignores other tenants, and counts recent rows.
    #[tokio::test]
    async fn postgres_integration_latest_write_marker() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-write-marker");
        let other = create_tenant("console-write-marker-other");
        let since = Some(chrono::Utc::now() - chrono::Duration::hours(1));

        let empty = backend
            .latest_write_marker(&tenant, since)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(empty.latest, None);
        assert_eq!(empty.recent_writes, Some(0));
        let unbounded = backend
            .latest_write_marker(&tenant, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unbounded.recent_writes, None);

        let created = backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        let after_create = backend.latest_write_marker(&tenant, since).await.unwrap();
        assert_ne!(after_create, Some(empty));
        assert_eq!(after_create.unwrap().recent_writes, Some(1));

        backend
            .update(&tenant, &created, json!({"active": true}))
            .await
            .unwrap();
        let after_update = backend.latest_write_marker(&tenant, since).await.unwrap();
        assert_ne!(after_update, after_create);

        backend
            .delete(&tenant, "Patient", created.id())
            .await
            .unwrap();
        let after_delete = backend.latest_write_marker(&tenant, since).await.unwrap();
        assert_ne!(after_delete, after_update);
        assert_eq!(after_delete.unwrap().recent_writes, Some(3));

        backend
            .create(&other, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        assert_eq!(
            backend.latest_write_marker(&tenant, since).await.unwrap(),
            after_delete
        );
        let future = Some(chrono::Utc::now() + chrono::Duration::hours(1));
        let marker = backend
            .latest_write_marker(&tenant, future)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(marker.recent_writes, Some(0));
        assert!(marker.latest.is_some());
    }

    #[tokio::test]
    async fn postgres_integration_count_by_day() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-count-by-day");

        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // `since` = start of today (UTC midnight), built the same way the handler
        // does; `today` is derived from the same clock so this stays date-robust.
        let since = chrono::Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let today = chrono::Utc::now().date_naive();

        let rows = backend
            .count_by_day(&tenant, "Patient", since)
            .await
            .unwrap();
        let today_row = rows
            .iter()
            .find(|r| r.day == today)
            .expect("today bucket should be present");
        assert_eq!(today_row.count, 2);
    }

    /// The history-backed delta rule on real Postgres: create `+1`, update `0`,
    /// delete `-1`, on epoch-aligned buckets. Mirrors the SQLite unit test, so the
    /// two backends are held to the same bucketing contract.
    #[tokio::test]
    async fn postgres_integration_count_deltas_by_bucket() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-count-deltas");

        let first = backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .update(&tenant, &first, json!({"active": true}))
            .await
            .unwrap();

        let since = chrono::Utc::now() - chrono::Duration::minutes(5);
        let rows = backend
            .count_deltas_by_bucket(&tenant, "Patient", since, 60)
            .await
            .unwrap();

        assert_eq!(
            rows.iter().map(|r| r.delta).sum::<i64>(),
            2,
            "two creates and one update net to +2"
        );
        assert!(
            rows.iter().all(|r| r.bucket_start.timestamp() % 60 == 0),
            "buckets are epoch-aligned to their width"
        );

        backend
            .delete(&tenant, "Patient", first.id())
            .await
            .unwrap();
        let rows = backend
            .count_deltas_by_bucket(&tenant, "Patient", since, 60)
            .await
            .unwrap();
        assert_eq!(rows.iter().map(|r| r.delta).sum::<i64>(), 1);
    }

    #[tokio::test]
    async fn postgres_integration_activity_histogram() {
        let backend = create_backend().await;
        let tenant = create_tenant("console-activity-histogram");

        // 3 writes for this tenant -> 3 resource_history rows.
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let since = chrono::Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();

        let cells = backend.activity_histogram(&tenant, since).await.unwrap();
        assert!(!cells.is_empty());
        // Total across returned cells equals the number of writes seeded.
        let total: u64 = cells.iter().map(|c| c.count).sum();
        assert_eq!(total, 3);
    }

    #[tokio::test]
    async fn postgres_integration_console_count_by_tenant() {
        let backend = create_backend().await;
        // Unique tenant IDs isolate this cross-tenant aggregate from every other
        // test sharing the same PostgreSQL container.
        let tenant_a = create_tenant("console-count-by-tenant-a");
        let tenant_b = create_tenant("console-count-by-tenant-b");

        // tenant-a: 3 resources, tenant-b: 2 resources.
        backend
            .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_a, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_a, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_b, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant_b, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Cross-tenant admin aggregate: takes NO TenantContext. Look up by the
        // actual (uuid-suffixed) tenant IDs so other tests' rows never interfere.
        let counts = backend.count_by_tenant().await.unwrap();
        let map: std::collections::HashMap<String, u64> = counts.into_iter().collect();
        assert_eq!(map.get(tenant_a.tenant_id().as_str()), Some(&3));
        assert_eq!(map.get(tenant_b.tenant_id().as_str()), Some(&2));
    }

    #[tokio::test]
    async fn postgres_integration_is_cluster_shared() {
        let backend = create_backend().await;
        assert!(backend.is_cluster_shared());
    }

    // ========================================================================
    // Tenant Registry Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_tenant_registry_crud() {
        let backend = create_backend().await;
        assert!(backend.supports_tenant_registry());

        // Unique ids isolate this test from others sharing the database. The
        // shared uuid base plus "-a"/"-b" suffixes make the id ASC tie-break
        // deterministic when both rows land in the same created_at second.
        let base = uuid::Uuid::new_v4().simple().to_string();
        let id_a = format!("registry-{}-a", base);
        let id_b = format!("registry-{}-b", base);

        assert!(backend.get_tenant(&id_a).await.unwrap().is_none());

        // Register two tenants, one with a display name.
        let acme = backend
            .register_tenant(&id_a, Some("Acme Corp"))
            .await
            .unwrap();
        assert_eq!(acme.id, id_a);
        assert_eq!(acme.display_name.as_deref(), Some("Acme Corp"));
        assert!(!acme.created_at.is_empty());

        let beta = backend.register_tenant(&id_b, None).await.unwrap();
        assert_eq!(beta.id, id_b);
        assert_eq!(beta.display_name, None);

        // get_tenant round-trips the registered records.
        assert_eq!(backend.get_tenant(&id_a).await.unwrap(), Some(acme));
        assert_eq!(backend.get_tenant(&id_b).await.unwrap(), Some(beta));

        // The database is shared across tests, so only assert on our own rows:
        // both are present, ordered a before b (created_at ASC, id ASC).
        let all = backend.list_tenants().await.unwrap();
        let pos_a = all.iter().position(|t| t.id == id_a);
        let pos_b = all.iter().position(|t| t.id == id_b);
        assert!(pos_a.is_some(), "registered tenant {} not listed", id_a);
        assert!(pos_b.is_some(), "registered tenant {} not listed", id_b);
        assert!(pos_a < pos_b, "expected {} to sort before {}", id_a, id_b);

        // Duplicate registration is an error (handler pre-checks for 409).
        assert!(backend.register_tenant(&id_a, None).await.is_err());

        // Deregister removes the row; repeat and unknown ids report nothing
        // removed.
        assert!(backend.deregister_tenant(&id_a).await.unwrap());
        assert!(backend.get_tenant(&id_a).await.unwrap().is_none());
        assert!(!backend.deregister_tenant(&id_a).await.unwrap());
        assert!(
            !backend
                .deregister_tenant(&format!("never-registered-{}", base))
                .await
                .unwrap()
        );

        let remaining = backend.list_tenants().await.unwrap();
        assert!(!remaining.iter().any(|t| t.id == id_a));
        assert!(remaining.iter().any(|t| t.id == id_b));

        backend.deregister_tenant(&id_b).await.unwrap();
    }

    #[tokio::test]
    async fn postgres_integration_purge_tenant_data() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("purge-tenant-a");
        let tenant_b = create_tenant("purge-tenant-b");

        let mut a_ids = Vec::new();
        for i in 0..3 {
            let patient = json!({
                "resourceType": "Patient",
                "name": [{"family": format!("Purge{}", i)}]
            });
            let created = backend
                .create(&tenant_a, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
            a_ids.push(created.id().to_string());
        }
        let b_created = backend
            .create(
                &tenant_b,
                "Patient",
                json!({"resourceType": "Patient", "name": [{"family": "Kept"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Purge removes tenant-a's data only, reporting its current-row count.
        let removed = backend
            .purge_tenant_data(tenant_a.tenant_id().as_str())
            .await
            .unwrap();
        assert_eq!(removed, 3);

        assert_eq!(backend.count(&tenant_a, Some("Patient")).await.unwrap(), 0);
        for id in &a_ids {
            assert!(
                backend
                    .read(&tenant_a, "Patient", id)
                    .await
                    .unwrap()
                    .is_none()
            );
        }

        // tenant-b's data is intact.
        assert_eq!(backend.count(&tenant_b, Some("Patient")).await.unwrap(), 1);
        assert!(
            backend
                .read(&tenant_b, "Patient", b_created.id())
                .await
                .unwrap()
                .is_some()
        );
    }

    // ========================================================================
    // Batch Read Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_read_batch() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let ids: Vec<String> = (0..3).map(|i| format!("batch-{}", i)).collect();
        for id in &ids {
            let patient = json!({"resourceType": "Patient"});
            backend
                .create_or_update(&tenant, "Patient", id, patient, FhirVersion::default())
                .await
                .unwrap();
        }

        let id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        let batch = backend
            .read_batch(&tenant, "Patient", &id_refs)
            .await
            .unwrap();

        assert_eq!(batch.len(), 3);
    }

    // ========================================================================
    // History Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_instance_history() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient", "name": [{"family": "V1"}]});
        let v1 = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let v2 = backend
            .update(
                &tenant,
                &v1,
                json!({"resourceType": "Patient", "name": [{"family": "V2"}]}),
            )
            .await
            .unwrap();

        let _v3 = backend
            .update(
                &tenant,
                &v2,
                json!({"resourceType": "Patient", "name": [{"family": "V3"}]}),
            )
            .await
            .unwrap();

        let history = backend
            .history_instance(&tenant, "Patient", v1.id(), &HistoryParams::default())
            .await
            .unwrap();

        assert!(
            history.items.len() >= 3,
            "Expected at least 3 history entries, got {}",
            history.items.len()
        );
    }

    // ========================================================================
    // Content Preservation Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_content_preserved() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "Smith", "given": ["John", "Jacob"]}],
            "birthDate": "1990-01-15",
            "gender": "male",
            "active": true,
            "identifier": [{
                "system": "http://example.org/mrn",
                "value": "MRN-001"
            }],
            "address": [{
                "city": "Springfield",
                "state": "IL"
            }]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(read.content()["name"][0]["family"], "Smith");
        assert_eq!(read.content()["name"][0]["given"][0], "John");
        assert_eq!(read.content()["name"][0]["given"][1], "Jacob");
        assert_eq!(read.content()["birthDate"], "1990-01-15");
        assert_eq!(read.content()["gender"], "male");
        assert_eq!(read.content()["active"], true);
        assert_eq!(read.content()["identifier"][0]["value"], "MRN-001");
        assert_eq!(read.content()["address"][0]["city"], "Springfield");
    }

    // ========================================================================
    // Search Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_search_by_name() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "name": [{"family": "Smith", "given": ["John"]}]
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            !result.resources.items.is_empty(),
            "Search by name should find the patient"
        );
        assert_eq!(result.resources.items[0].id(), "p1");
    }

    /// The backend-agnostic meta-parameter scenario (#523), shared verbatim
    /// with the SQLite suite that owns the file.
    #[tokio::test]
    async fn postgres_integration_search_by_meta_parameters() {
        let backend = create_backend().await;
        let tenant = create_tenant("meta-params");
        crate::meta_params_suite::meta_parameters_match_only_their_carrier(&backend, &tenant).await;
    }

    #[tokio::test]
    async fn postgres_integration_string_search_is_accent_insensitive() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "accent-pg",
                    "name": [{ "family": "Müller" }]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        for q in ["muller", "Müller", "MULLER"] {
            let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "family".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::eq(q)],
                chain: vec![],
                components: vec![],
            });
            let result = backend.search(&tenant, &query).await.unwrap();
            assert_eq!(
                result.resources.items.len(),
                1,
                "accent-insensitive family search '{q}' should match 'Müller'"
            );
        }
    }

    #[tokio::test]
    async fn postgres_integration_quantity_search_ucum_equivalence() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-mass-pg",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "x" }] },
                    "valueQuantity": { "value": 1, "unit": "g", "system": "http://unitsofmeasure.org", "code": "g" }
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::eq("1000|http://unitsofmeasure.org|mg")],
            chain: vec![],
            components: vec![],
        });
        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "UCUM-equivalent quantity (1000 mg) should match stored 1 g"
        );
        assert_eq!(result.resources.items[0].id(), "obs-mass-pg");
    }

    #[tokio::test]
    async fn postgres_integration_quantity_comparators_ignore_search_precision() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Issue #1011: `value-quantity=gt60` must exclude 60.2 kg exactly at
        // the boundary while still matching every value strictly above 60,
        // regardless of how the search value's own precision is written.
        let weights = [
            ("obs-weight-55-4", 55.4),
            ("obs-weight-58-5", 58.5),
            ("obs-weight-60-2", 60.2),
            ("obs-weight-64-5", 64.5),
        ];
        for (id, value) in weights {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": id,
                        "status": "final",
                        "code": { "coding": [{ "system": "http://loinc.org", "code": "29463-7" }] },
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
                .unwrap();
        }

        async fn search_ids(
            backend: &PostgresBackend,
            tenant: &TenantContext,
            prefix: SearchPrefix,
            value: &str,
        ) -> Vec<String> {
            let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "value-quantity".to_string(),
                param_type: SearchParamType::Quantity,
                modifier: None,
                values: vec![SearchValue::new(prefix, value)],
                chain: vec![],
                components: vec![],
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

        let gt60 = search_ids(&backend, &tenant, SearchPrefix::Gt, "60").await;
        assert_eq!(gt60, vec!["obs-weight-60-2", "obs-weight-64-5"], "gt60");

        let gt60_0 = search_ids(&backend, &tenant, SearchPrefix::Gt, "60.0").await;
        assert_eq!(
            gt60_0,
            vec!["obs-weight-60-2", "obs-weight-64-5"],
            "gt60.0 must match gt60 exactly: implicit precision is ignored"
        );

        let le60_2 = search_ids(&backend, &tenant, SearchPrefix::Le, "60.2").await;
        assert_eq!(
            le60_2,
            vec!["obs-weight-55-4", "obs-weight-58-5", "obs-weight-60-2"],
            "le60.2"
        );

        let lt58_5 = search_ids(&backend, &tenant, SearchPrefix::Lt, "58.5").await;
        assert_eq!(lt58_5, vec!["obs-weight-55-4"], "lt58.5");

        let eq60 = search_ids(&backend, &tenant, SearchPrefix::Eq, "60").await;
        assert_eq!(
            eq60,
            vec!["obs-weight-60-2"],
            "eq60 ranges over [59.5, 60.5), which contains 60.2"
        );

        let eq60_0 = search_ids(&backend, &tenant, SearchPrefix::Eq, "60.0").await;
        assert!(
            eq60_0.is_empty(),
            "eq60.0 ranges over [59.95, 60.05), which excludes 60.2: got {eq60_0:?}"
        );

        let gt60_kg = search_ids(
            &backend,
            &tenant,
            SearchPrefix::Gt,
            "60|http://unitsofmeasure.org|kg",
        )
        .await;
        assert_eq!(
            gt60_kg,
            vec!["obs-weight-60-2", "obs-weight-64-5"],
            "gt60|...|kg (raw branch)"
        );

        // Cross-unit boundary: 60.2 kg canonicalizes to exactly 60200 g, so
        // `ge60200|...|g` must match it (and everything above) through the
        // canonical branch.
        let ge60200_g = search_ids(
            &backend,
            &tenant,
            SearchPrefix::Ge,
            "60200|http://unitsofmeasure.org|g",
        )
        .await;
        assert_eq!(
            ge60200_g,
            vec!["obs-weight-60-2", "obs-weight-64-5"],
            "ge60200|...|g (canonical branch, cross-unit exact boundary)"
        );
    }

    #[tokio::test]
    async fn postgres_integration_quantity_ne_uses_canonical_values() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Issue #1011 (gate follow-up): `ne60|...|kg` must exclude resources
        // whose value equals 60 kg *by unit conversion*, not only resources
        // stored literally as kg. Mixed kg/g dataset around the 60 kg / 60000 g
        // boundary.
        let weights = [
            ("obs-ne-55-4-kg", 55.4, "kg"),
            ("obs-ne-60-2-kg", 60.2, "kg"),
            ("obs-ne-55000-g", 55000.0, "g"),
            ("obs-ne-60000-g", 60000.0, "g"),
            ("obs-ne-64500-g", 64500.0, "g"),
        ];
        for (id, value, unit) in weights {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": id,
                        "status": "final",
                        "code": { "coding": [{ "system": "http://loinc.org", "code": "29463-7" }] },
                        "valueQuantity": {
                            "value": value,
                            "unit": unit,
                            "system": "http://unitsofmeasure.org",
                            "code": unit
                        }
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        async fn search_ids(
            backend: &PostgresBackend,
            tenant: &TenantContext,
            prefix: SearchPrefix,
            value: &str,
        ) -> Vec<String> {
            let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "value-quantity".to_string(),
                param_type: SearchParamType::Quantity,
                modifier: None,
                values: vec![SearchValue::new(prefix, value)],
                chain: vec![],
                components: vec![],
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

        let ne60_kg = search_ids(
            &backend,
            &tenant,
            SearchPrefix::Ne,
            "60|http://unitsofmeasure.org|kg",
        )
        .await;
        assert_eq!(
            ne60_kg,
            vec!["obs-ne-55-4-kg", "obs-ne-55000-g", "obs-ne-64500-g"],
            "ne60|...|kg must exclude 60.2 kg and its canonical equivalent 60000 g"
        );

        let ne60000_g = search_ids(
            &backend,
            &tenant,
            SearchPrefix::Ne,
            "60000|http://unitsofmeasure.org|g",
        )
        .await;
        assert_eq!(
            ne60000_g,
            vec![
                "obs-ne-55-4-kg",
                "obs-ne-55000-g",
                "obs-ne-60-2-kg",
                "obs-ne-64500-g"
            ],
            "ne60000|...|g must exclude 60000 g and its canonical equivalent 60.2 kg"
        );

        let ne60_raw = search_ids(&backend, &tenant, SearchPrefix::Ne, "60").await;
        assert_eq!(
            ne60_raw,
            vec![
                "obs-ne-55-4-kg",
                "obs-ne-55000-g",
                "obs-ne-60000-g",
                "obs-ne-64500-g"
            ],
            "ne60 without a unit only excludes the raw value 60.2, regardless of unit"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_by_token() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "p1",
            "gender": "male"
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("male")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            !result.resources.items.is_empty(),
            "Search by gender should find the patient"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_sort_by_id() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchQuery, SortDirective};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Create in non-sorted insertion order to prove ORDER BY is applied.
        for id in ["p3", "p1", "p2"] {
            let patient = json!({ "resourceType": "Patient", "id": id });
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        // Ascending _sort=_id
        let asc = SearchQuery::new("Patient").with_sort(SortDirective::parse("_id"));
        let result = backend.search(&tenant, &asc).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["p1", "p2", "p3"],
            "_sort=_id should return ascending id order"
        );

        // Descending _sort=-_id
        let desc = SearchQuery::new("Patient").with_sort(SortDirective::parse("-_id"));
        let result = backend.search(&tenant, &desc).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["p3", "p2", "p1"],
            "_sort=-_id should return descending id order"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_cursor_with_custom_sort() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchParamType, SearchQuery, SortDirective};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Insert out of order; page size 1 to force keyset paging across pages.
        for (id, family) in [
            ("p-charlie", "Charlie"),
            ("p-alice", "Alice"),
            ("p-bob", "Bob"),
        ] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({ "resourceType": "Patient", "id": id, "name": [{ "family": family }] }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        let mut collected = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..5 {
            let mut q = SearchQuery::new("Patient")
                .with_sort(
                    SortDirective::parse("family").with_param_type(Some(SearchParamType::String)),
                )
                .with_count(1);
            q.cursor = cursor.clone();
            let result = backend.search(&tenant, &q).await.unwrap();
            for r in &result.resources.items {
                collected.push(r.id().to_string());
            }
            match result.resources.page_info.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }

        // Keyset paging must yield the full set in family order, no dups/gaps.
        assert_eq!(
            collected,
            vec!["p-alice", "p-bob", "p-charlie"],
            "cursor paging with custom sort must preserve global order"
        );
    }

    /// Creates Patients `cp-1..cp-n` (inclusive) in the given tenant.
    async fn create_cursor_paging_patients(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        n: usize,
    ) {
        for i in 1..=n {
            backend
                .create(
                    tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("cp-{i}"),
                        "name": [{"family": "CursorPaging"}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
    }

    /// Collects the resource ids of a search result's page, in page order.
    fn page_ids(result: &helios_persistence::core::SearchResult) -> Vec<String> {
        result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect()
    }

    /// Walks forward through 7 Patients 3 at a time (no explicit `_sort`),
    /// then walks all the way back via `previous_cursor` and confirms every
    /// page is reproduced exactly, including the page-3-to-page-2 hop that a
    /// naive truncate-after-reverse implementation gets wrong (#1079).
    #[tokio::test]
    async fn postgres_integration_cursor_paging_round_trip_previous() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::SearchQuery;

        let backend = create_backend().await;
        let tenant = create_tenant("cursor-prev");
        create_cursor_paging_patients(&backend, &tenant, 7).await;

        let query = SearchQuery::new("Patient").with_count(3);

        let page1 = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page1.resources.items.len(), 3);
        assert!(!page1.resources.page_info.has_previous);
        assert!(page1.resources.page_info.previous_cursor.is_none());
        assert!(page1.resources.page_info.has_next);
        assert!(page1.resources.page_info.next_cursor.is_some());

        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page2.resources.items.len(), 3);
        assert!(page2.resources.page_info.has_previous);
        assert!(page2.resources.page_info.has_next);

        let page3 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page3.resources.items.len(), 1);
        assert!(!page3.resources.page_info.has_next);
        assert!(page3.resources.page_info.next_cursor.is_none());
        assert!(page3.resources.page_info.previous_cursor.is_some());

        let page1_ids = page_ids(&page1);
        let page2_ids = page_ids(&page2);
        let page3_ids = page_ids(&page3);
        let mut all_ids = page1_ids.clone();
        all_ids.extend(page2_ids.clone());
        all_ids.extend(page3_ids.clone());
        let mut unique_ids = all_ids.clone();
        unique_ids.sort();
        unique_ids.dedup();
        assert_eq!(unique_ids.len(), 7, "all 7 ids must be distinct");

        // Walk back: page 3 -> page 2 must be exact, including order. This is
        // the case a naive truncate-after-reverse gets wrong: it drops the
        // nearest hit instead of the farthest one.
        let back2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page3.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back2), page2_ids);
        assert!(back2.resources.page_info.has_previous);
        assert!(back2.resources.page_info.has_next);
        assert!(back2.resources.page_info.next_cursor.is_some());

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), page1_ids);
        assert!(!back1.resources.page_info.has_previous);
        assert!(back1.resources.page_info.previous_cursor.is_none());
        assert!(back1.resources.page_info.has_next);

        let again2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&again2), page2_ids);
    }

    /// Same round trip, but with an explicit `_sort=_id` ascending so the
    /// exact page contents (not just their distinctness) can be asserted.
    #[tokio::test]
    async fn postgres_integration_cursor_paging_round_trip_previous_with_sort() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchQuery, SortDirective};

        let backend = create_backend().await;
        let tenant = create_tenant("cursor-prev-sort");
        create_cursor_paging_patients(&backend, &tenant, 7).await;

        let query = SearchQuery::new("Patient")
            .with_count(3)
            .with_sort(SortDirective::parse("_id"));

        let page1 = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page_ids(&page1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(!page1.resources.page_info.has_previous);
        assert!(page1.resources.page_info.previous_cursor.is_none());
        assert!(page1.resources.page_info.has_next);

        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page2), vec!["cp-4", "cp-5", "cp-6"]);
        assert!(page2.resources.page_info.has_previous);
        assert!(page2.resources.page_info.has_next);

        let page3 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page3), vec!["cp-7"]);
        assert!(!page3.resources.page_info.has_next);
        assert!(page3.resources.page_info.next_cursor.is_none());
        assert!(page3.resources.page_info.previous_cursor.is_some());

        let back2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page3.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back2), vec!["cp-4", "cp-5", "cp-6"]);
        assert!(back2.resources.page_info.has_previous);
        assert!(back2.resources.page_info.has_next);
        assert!(back2.resources.page_info.next_cursor.is_some());

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(!back1.resources.page_info.has_previous);
        assert!(back1.resources.page_info.previous_cursor.is_none());
        assert!(back1.resources.page_info.has_next);

        let again2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&again2), vec!["cp-4", "cp-5", "cp-6"]);
    }

    /// Backward from page 2 of a 4-item, count-3 listing has no extra row
    /// beyond page 1, so `has_previous` must be false and no
    /// `previous_cursor` is produced.
    #[tokio::test]
    async fn postgres_integration_cursor_paging_backward_from_page_two_has_no_previous() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchQuery, SortDirective};

        let backend = create_backend().await;
        let tenant = create_tenant("cursor-prev-short");
        create_cursor_paging_patients(&backend, &tenant, 4).await;

        let query = SearchQuery::new("Patient")
            .with_count(3)
            .with_sort(SortDirective::parse("_id"));

        let page1 = backend.search(&tenant, &query).await.unwrap();
        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page2), vec!["cp-4"]);

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(!back1.resources.page_info.has_previous);
        assert!(back1.resources.page_info.has_next);
        assert!(back1.resources.page_info.next_cursor.is_some());
    }

    #[tokio::test]
    async fn postgres_integration_search_sort_by_indexed_param() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{SearchParamType, SearchQuery, SortDirective};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for (id, family) in [
            ("p-charlie", "Charlie"),
            ("p-alice", "Alice"),
            ("p-bob", "Bob"),
        ] {
            let patient = json!({
                "resourceType": "Patient",
                "id": id,
                "name": [{ "family": family }],
            });
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }

        let collect_ids = |result: helios_persistence::core::SearchResult| {
            result
                .resources
                .items
                .iter()
                .map(|r| r.id().to_string())
                .collect::<Vec<_>>()
        };

        let asc = SearchQuery::new("Patient").with_sort(
            SortDirective::parse("family").with_param_type(Some(SearchParamType::String)),
        );
        let ids = collect_ids(backend.search(&tenant, &asc).await.unwrap());
        assert_eq!(
            ids,
            vec!["p-alice", "p-bob", "p-charlie"],
            "sort by family asc"
        );

        let desc = SearchQuery::new("Patient").with_sort(
            SortDirective::parse("-family").with_param_type(Some(SearchParamType::String)),
        );
        let ids = collect_ids(backend.search(&tenant, &desc).await.unwrap());
        assert_eq!(
            ids,
            vec!["p-charlie", "p-bob", "p-alice"],
            "sort by family desc"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_missing_modifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::error::{SearchError, StorageError};
        use helios_persistence::types::{
            ContainedMode, SearchModifier, SearchParamType, SearchParameter, SearchQuery,
            SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "with-gender", "gender": "male" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "no-gender" }),
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
                    "id": "container-no-gender",
                    "contained": [{
                        "resourceType": "Patient",
                        "id": "contained-with-gender",
                        "gender": "female"
                    }]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let missing = |present: &str| {
            SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "gender".to_string(),
                param_type: SearchParamType::Token,
                modifier: Some(SearchModifier::Missing),
                values: vec![SearchValue::eq(present)],
                chain: vec![],
                components: vec![],
            })
        };

        let result = backend.search(&tenant, &missing("true")).await.unwrap();
        let mut ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["container-no-gender", "no-gender"],
            "gender:missing=true must ignore gender indexed only from contained resources"
        );

        let result = backend.search(&tenant, &missing("false")).await.unwrap();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["with-gender"],
            "gender:missing=false → with-gender"
        );

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
            assert_eq!(
                ids,
                vec!["container-no-gender", "no-gender", "with-gender"],
                "{name}:missing=false"
            );
        }

        let mut contained_missing = missing("true");
        contained_missing.contained = ContainedMode::On;
        let error = backend
            .search(&tenant, &contained_missing)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            StorageError::Search(SearchError::QueryParseError { .. })
        ));
    }

    #[tokio::test]
    async fn postgres_integration_search_not_modifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for (id, gender) in [("male1", Some("male")), ("female1", Some("female"))] {
            let mut patient = json!({ "resourceType": "Patient", "id": id });
            if let Some(g) = gender {
                patient["gender"] = json!(g);
            }
            backend
                .create(&tenant, "Patient", patient, FhirVersion::default())
                .await
                .unwrap();
        }
        // A patient with no gender at all should also be returned by :not.
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "none1" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "gender".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Not),
            values: vec![SearchValue::eq("male")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        let mut ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["female1", "none1"],
            "gender:not=male → non-male incl. resources with no gender"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_composite_code_value_quantity() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let observation = json!({
            "resourceType": "Observation",
            "id": "obs-bp",
            "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
            "valueQuantity": { "value": 107, "unit": "mmHg", "system": "http://unitsofmeasure.org" }
        });
        backend
            .create(&tenant, "Observation", observation, FhirVersion::default())
            .await
            .unwrap();

        let query = |value: &str| {
            SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "code-value-quantity".to_string(),
                param_type: SearchParamType::Composite,
                modifier: None,
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![
                    CompositeSearchComponent {
                        param_type: SearchParamType::Token,
                        param_name: "code".to_string(),
                    },
                    CompositeSearchComponent {
                        param_type: SearchParamType::Quantity,
                        param_name: "value-quantity".to_string(),
                    },
                ],
            })
        };

        let result = backend
            .search(&tenant, &query("8480-6$ge100"))
            .await
            .unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "code + value match → 1 hit"
        );
        assert_eq!(result.resources.items[0].id(), "obs-bp");

        let result = backend
            .search(&tenant, &query("8480-6$ge200"))
            .await
            .unwrap();
        assert!(result.resources.items.is_empty(), "value too low → no hit");

        let result = backend
            .search(&tenant, &query("9999-9$ge100"))
            .await
            .unwrap();
        assert!(result.resources.items.is_empty(), "code mismatch → no hit");
    }

    /// A composite must match only when every component is satisfied *within the
    /// same* `composite_group`.
    ///
    /// `Observation.component` yields one composite group per component entry, so
    /// a blood-pressure panel indexes systolic (8480-6, 120) as group 0 and
    /// diastolic (8462-4, 80) as group 1 — each with its own code row and value
    /// row. A query for "diastolic > 100" must NOT match: the resource has a code
    /// row for 8462-4 (group 1) and a value row > 100 (group 0, the systolic 120),
    /// but never both in one group.
    ///
    /// Any rewrite that pushes the component predicates into the WHERE clause
    /// without correlating on `composite_group` returns this resource — a silent
    /// false positive that ships as a 100x speedup and returns the wrong patients.
    /// The single-group fixture in the test above cannot detect that class of bug.
    #[tokio::test]
    async fn postgres_integration_composite_components_must_share_a_group() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Blood-pressure panel: systolic 120, diastolic 80 — two composite groups.
        let observation = json!({
            "resourceType": "Observation",
            "id": "obs-bp-panel",
            "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "85354-9" }] },
            "component": [
                {
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
                    "valueQuantity": { "value": 120, "unit": "mm[Hg]", "system": "http://unitsofmeasure.org" }
                },
                {
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "8462-4" }] },
                    "valueQuantity": { "value": 80, "unit": "mm[Hg]", "system": "http://unitsofmeasure.org" }
                }
            ]
        });
        backend
            .create(&tenant, "Observation", observation, FhirVersion::default())
            .await
            .unwrap();

        let query = |value: &str| {
            SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "component-code-value-quantity".to_string(),
                param_type: SearchParamType::Composite,
                modifier: None,
                values: vec![SearchValue::eq(value)],
                chain: vec![],
                components: vec![
                    CompositeSearchComponent {
                        param_type: SearchParamType::Token,
                        param_name: "component-code".to_string(),
                    },
                    CompositeSearchComponent {
                        param_type: SearchParamType::Quantity,
                        param_name: "component-value-quantity".to_string(),
                    },
                ],
            })
        };

        // The cross-group false positive: diastolic's code (group 1) + systolic's
        // value (group 0). Must NOT match.
        let result = backend
            .search(&tenant, &query("8462-4$gt100"))
            .await
            .unwrap();
        assert!(
            result.resources.items.is_empty(),
            "diastolic is 80, not >100 — a match here means component predicates \
             leaked across composite groups (systolic's 120 satisfied the value \
             while diastolic satisfied the code)"
        );

        // Both components satisfied within group 0 (systolic 120 > 100) → match.
        let result = backend
            .search(&tenant, &query("8480-6$gt100"))
            .await
            .unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "systolic 8480-6 = 120 > 100, both components in group 0 → 1 hit"
        );
        assert_eq!(result.resources.items[0].id(), "obs-bp-panel");

        // Both components satisfied within group 1 (diastolic 80 < 100) → match.
        let result = backend
            .search(&tenant, &query("8462-4$lt100"))
            .await
            .unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "diastolic 8462-4 = 80 < 100, both components in group 1 → 1 hit"
        );
    }

    /// Each value of a composite OR-list must be satisfied on its own; components
    /// must not pair up *across* values.
    ///
    /// The fixture's only component is (8480-6, 250). For
    /// `8480-6$lt60,8462-4$gt200`: value 1 needs code 8480-6 AND value < 60 (it is
    /// 250 — no); value 2 needs code 8462-4 (absent — no). Collapsing both values
    /// into one subquery with a merged HAVING lets value 1's code satisfy the token
    /// leg while value 2's `>200` is satisfied by the same row's 250 → false positive.
    #[tokio::test]
    async fn postgres_integration_composite_or_list_values_are_isolated() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            CompositeSearchComponent, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let observation = json!({
            "resourceType": "Observation",
            "id": "obs-single-high",
            "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "85354-9" }] },
            "component": [{
                "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
                "valueQuantity": { "value": 250, "unit": "mm[Hg]", "system": "http://unitsofmeasure.org" }
            }]
        });
        backend
            .create(&tenant, "Observation", observation, FhirVersion::default())
            .await
            .unwrap();

        let multi_value = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "component-code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![
                SearchValue::eq("8480-6$lt60"),
                SearchValue::eq("8462-4$gt200"),
            ],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "component-code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "component-value-quantity".to_string(),
                },
            ],
        });

        let result = backend.search(&tenant, &multi_value).await.unwrap();
        assert!(
            result.resources.items.is_empty(),
            "neither OR-value is satisfied on its own (8480-6 is 250 not <60; there \
             is no 8462-4) — a match means components paired across OR-values"
        );
    }

    /// String search must still match rows whose `value_string_folded` is NULL.
    ///
    /// That column arrived in schema v10 and is populated **only on write** — the
    /// migration never backfilled it (there is no `UPDATE search_index` anywhere),
    /// so every row indexed before the upgrade has NULL there. The search predicate
    /// therefore falls back to the raw `value_string`, and that fallback must stay
    /// case-insensitive.
    ///
    /// This is a tripwire, not a feature test: it is green today and must stay
    /// green. It fails the moment someone "optimizes" the predicate to
    /// `value_string_folded LIKE $n` (NULL never matches → patients silently vanish
    /// from results) or to `COALESCE(value_string_folded, value_string) LIKE $n`
    /// (the raw branch loses its case-folding → `name=smith` stops finding "Smith").
    /// Both are silent wrong-results bugs, not errors.
    #[tokio::test]
    async fn postgres_integration_string_search_matches_unbackfilled_rows() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("unbackfilled");
        // `create_tenant` suffixes a UUID for isolation, so the effective tenant id
        // must be read back off the context — the literal passed in is only a prefix.
        // Seeding `search_index` with the bare literal violates the FK to `resources`.
        let tenant_id = tenant.tenant_id().as_str();

        // No `name` element, so the writer indexes no `name` row for this Patient —
        // leaving the field clear for the hand-written legacy row below.
        backend
            .create(
                &tenant,
                "Patient",
                json!({ "id": "legacy-p1", "gender": "female" }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // A pre-v10 row: raw value only, `value_string_folded` left NULL.
        insert_search_index(
            tenant_id,
            "Patient",
            "legacy-p1",
            "name",
            "value_string",
            "Smith",
        )
        .await;

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            // Lowercase query against a capitalized stored value: only the
            // case-insensitive fallback can match this.
            values: vec![SearchValue::eq("smith")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "a row with value_string='Smith' and value_string_folded=NULL (i.e. \
             written before the v10 migration) must still match name=smith — the \
             COALESCE fallback and its case-folding are load-bearing"
        );
        assert_eq!(result.resources.items[0].id(), "legacy-p1");
    }

    // ========================================================================
    // Backend Health Check Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_health_check() {
        let backend = create_backend().await;

        let result = backend.health_check().await;
        assert!(result.is_ok(), "Health check failed: {:?}", result.err());

        // The `/_readiness` probe delegates to `Backend::health_check` via the
        // `ResourceStorage::readiness_check` override; a live pg must report ready.
        let readiness = ResourceStorage::readiness_check(&backend).await;
        assert!(
            readiness.is_ok(),
            "readiness_check failed on a live postgres: {:?}",
            readiness.err()
        );
    }

    #[tokio::test]
    async fn postgres_integration_backend_kind() {
        let backend = create_backend().await;

        assert_eq!(backend.kind(), BackendKind::Postgres);
        assert_eq!(backend.name(), "postgres");
    }

    /// Instance-level tenancy + delegation contract for PostgreSQL (#369).
    ///
    /// `tests/backend_capability_contract.rs` pins the constructor-free
    /// `PostgresBackend::declared_capabilities()`. This is the other half — that
    /// the *instance* answers the same thing through `supports()`. The #369
    /// defect lived in a hand-rolled `matches!` ladder inside `supports()`, so a
    /// declaration test alone would not have caught it. PostgreSQL is the one
    /// backend whose instance assertions need a live database, and it always has
    /// one: `create_backend()` panics rather than skipping, and CI makes
    /// `DOCKER_HOST` mandatory.
    #[tokio::test]
    async fn postgres_integration_capabilities() {
        let backend = create_backend().await;

        assert!(backend.supports(BackendCapability::Crud));
        assert!(backend.supports(BackendCapability::Versioning));
        assert!(backend.supports(BackendCapability::InstanceHistory));
        assert!(backend.supports(BackendCapability::BasicSearch));
        assert!(backend.supports(BackendCapability::Transactions));
        assert!(backend.supports(BackendCapability::BulkExport));
        assert!(backend.supports(BackendCapability::BulkSubmitIngest));
        assert!(backend.supports(BackendCapability::BulkSubmitRestWorker));
        assert!(backend.supports(BackendCapability::Include));
        assert!(backend.supports(BackendCapability::Revinclude));

        // The #369 regression, asserted on the live instance rather than the
        // declaration: PostgreSQL is shared-schema only.
        assert!(backend.supports(BackendCapability::SharedSchema));
        assert!(
            !backend.supports(BackendCapability::SchemaPerTenant),
            "PostgreSQL has no SET search_path / CREATE SCHEMA path; declaring schema-per-tenant \
             overstates isolation. See #369."
        );
        assert!(
            !backend.supports(BackendCapability::DatabasePerTenant),
            "PostgreSQL has no CREATE DATABASE / per-tenant pool; declaring database-per-tenant \
             overstates isolation. See #369."
        );

        // supports() must agree with capabilities() for every declared
        // capability — the delegation the #369 fix relies on, checked live.
        for capability in backend.capabilities() {
            assert!(
                backend.supports(capability),
                "postgres capabilities() lists {capability:?} but supports() denies it — the two \
                 have drifted apart (#369)."
            );
        }
    }

    // ========================================================================
    // Additional Content Preservation Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_unicode_content() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "name": [{"family": "日本語", "given": ["名前"]}]
        });

        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        let read = backend
            .read(&tenant, "Patient", created.id())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(read.content()["name"][0]["family"], "日本語");
        assert_eq!(read.content()["name"][0]["given"][0], "名前");
    }

    // ========================================================================
    // Additional Tenant Isolation Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_tenant_isolation_read() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Tenant A can read
        let read_a = backend
            .read(&tenant_a, "Patient", created.id())
            .await
            .unwrap();
        assert!(read_a.is_some());

        // Tenant B cannot read
        let read_b = backend
            .read(&tenant_b, "Patient", created.id())
            .await
            .unwrap();
        assert!(read_b.is_none());
    }

    #[tokio::test]
    async fn postgres_integration_tenant_isolation_delete() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        // Tenant B cannot delete tenant A's resource
        let result = backend.delete(&tenant_b, "Patient", created.id()).await;
        assert!(result.is_err());

        // Resource still exists for tenant A
        assert!(
            backend
                .exists(&tenant_a, "Patient", created.id())
                .await
                .unwrap()
        );
    }

    // ========================================================================
    // Additional Batch Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_read_batch_ignores_other_tenant() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        backend
            .create_or_update(
                &tenant_a,
                "Patient",
                "a-patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create_or_update(
                &tenant_b,
                "Patient",
                "b-patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let ids = ["a-patient", "b-patient"];
        let batch = backend
            .read_batch(&tenant_a, "Patient", &ids)
            .await
            .unwrap();

        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].id(), "a-patient");
    }

    // ========================================================================
    // Detailed History Tests - Instance
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_history_instance_detailed() {
        use helios_persistence::core::history::HistoryMethod;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient", "name": [{"family": "Smith"}]});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let v2 = backend
            .update(
                &tenant,
                &created,
                json!({"resourceType": "Patient", "name": [{"family": "Jones"}]}),
            )
            .await
            .unwrap();

        let _v3 = backend
            .update(
                &tenant,
                &v2,
                json!({"resourceType": "Patient", "name": [{"family": "Brown"}]}),
            )
            .await
            .unwrap();

        let params = HistoryParams::new();
        let history = backend
            .history_instance(&tenant, "Patient", created.id(), &params)
            .await
            .unwrap();

        // Should have 3 versions, newest first
        assert_eq!(history.items.len(), 3);
        assert_eq!(history.items[0].resource.version_id(), "3");
        assert_eq!(history.items[1].resource.version_id(), "2");
        assert_eq!(history.items[2].resource.version_id(), "1");

        // Check methods
        assert_eq!(history.items[0].method, HistoryMethod::Put);
        assert_eq!(history.items[1].method, HistoryMethod::Put);
        assert_eq!(history.items[2].method, HistoryMethod::Post);

        // Check content
        assert_eq!(
            history.items[0].resource.content()["name"][0]["family"],
            "Brown"
        );
        assert_eq!(
            history.items[2].resource.content()["name"][0]["family"],
            "Smith"
        );
    }

    #[tokio::test]
    async fn postgres_integration_history_instance_count() {
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient"});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        let v2 = backend
            .update(&tenant, &created, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        let _v3 = backend
            .update(&tenant, &v2, json!({"resourceType": "Patient"}))
            .await
            .unwrap();

        let count = backend
            .history_instance_count(&tenant, "Patient", created.id())
            .await
            .unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn postgres_integration_history_with_delete() {
        use helios_persistence::core::history::HistoryMethod;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({"resourceType": "Patient", "id": "hist-patient"});
        let created = backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        let _v2 = backend
            .update(
                &tenant,
                &created,
                json!({"resourceType": "Patient", "id": "hist-patient"}),
            )
            .await
            .unwrap();
        backend
            .delete(&tenant, "Patient", "hist-patient")
            .await
            .unwrap();

        let params = HistoryParams::new().include_deleted(true);
        let history = backend
            .history_instance(&tenant, "Patient", "hist-patient", &params)
            .await
            .unwrap();

        assert_eq!(history.items.len(), 3);
        assert_eq!(history.items[0].method, HistoryMethod::Delete);
        assert_eq!(history.items[0].resource.version_id(), "3");
    }

    #[tokio::test]
    async fn postgres_integration_history_tenant_isolation() {
        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        let patient = json!({"resourceType": "Patient", "id": "hist-shared"});
        let created = backend
            .create(&tenant_a, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();
        let _v2 = backend
            .update(
                &tenant_a,
                &created,
                json!({"resourceType": "Patient", "id": "hist-shared"}),
            )
            .await
            .unwrap();

        // Tenant A sees history
        let history_a = backend
            .history_instance(&tenant_a, "Patient", "hist-shared", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_a.items.len(), 2);

        // Tenant B sees nothing
        let history_b = backend
            .history_instance(&tenant_b, "Patient", "hist-shared", &HistoryParams::new())
            .await
            .unwrap();
        assert!(history_b.items.is_empty());
    }

    // ========================================================================
    // Type History Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_history_type() {
        use helios_persistence::core::history::TypeHistoryProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "tp1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p2 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "tp2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Update p1
        let _p1_v2 = backend
            .update(
                &tenant,
                &p1,
                json!({"resourceType": "Patient", "id": "tp1"}),
            )
            .await
            .unwrap();

        // Create an observation (different type)
        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let history = backend
            .history_type(&tenant, "Patient", &HistoryParams::new())
            .await
            .unwrap();

        // Should have 3 entries for Patient (p1 v1, p1 v2, p2 v1)
        assert_eq!(history.items.len(), 3);

        for entry in &history.items {
            assert_eq!(entry.resource.resource_type(), "Patient");
        }
    }

    #[tokio::test]
    async fn postgres_integration_history_type_count() {
        use helios_persistence::core::history::TypeHistoryProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p1_v2 = backend
            .update(&tenant, &p1, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        let _p2 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let patient_count = backend
            .history_type_count(&tenant, "Patient")
            .await
            .unwrap();
        assert_eq!(patient_count, 3);

        let obs_count = backend
            .history_type_count(&tenant, "Observation")
            .await
            .unwrap();
        assert_eq!(obs_count, 1);
    }

    #[tokio::test]
    async fn postgres_integration_history_type_tenant_isolation() {
        use helios_persistence::core::history::TypeHistoryProvider;

        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        backend
            .create(
                &tenant_a,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant_a,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant_b,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let history_a = backend
            .history_type(&tenant_a, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_a.items.len(), 2);

        let history_b = backend
            .history_type(&tenant_b, "Patient", &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_b.items.len(), 1);
    }

    // ========================================================================
    // System History Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_history_system() {
        use helios_persistence::core::history::SystemHistoryProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "sp1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation", "id": "so1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Encounter",
                json!({"resourceType": "Encounter", "id": "se1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Update patient
        let _p1_v2 = backend
            .update(
                &tenant,
                &p1,
                json!({"resourceType": "Patient", "id": "sp1"}),
            )
            .await
            .unwrap();

        let history = backend
            .history_system(&tenant, &HistoryParams::new())
            .await
            .unwrap();

        // Should have 4 entries total
        assert_eq!(history.items.len(), 4);

        let types: std::collections::HashSet<_> = history
            .items
            .iter()
            .map(|e| e.resource.resource_type())
            .collect();
        assert!(types.contains("Patient"));
        assert!(types.contains("Observation"));
        assert!(types.contains("Encounter"));
    }

    #[tokio::test]
    async fn postgres_integration_history_system_count() {
        use helios_persistence::core::history::SystemHistoryProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let p1 = backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let _p1_v2 = backend
            .update(&tenant, &p1, json!({"resourceType": "Patient"}))
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let count = backend.history_system_count(&tenant).await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn postgres_integration_history_system_tenant_isolation() {
        use helios_persistence::core::history::SystemHistoryProvider;

        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        backend
            .create(
                &tenant_a,
                "Patient",
                json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant_a,
                "Observation",
                json!({"resourceType": "Observation"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant_b,
                "Encounter",
                json!({"resourceType": "Encounter"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let history_a = backend
            .history_system(&tenant_a, &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_a.items.len(), 2);

        let history_b = backend
            .history_system(&tenant_b, &HistoryParams::new())
            .await
            .unwrap();
        assert_eq!(history_b.items.len(), 1);

        assert_eq!(backend.history_system_count(&tenant_a).await.unwrap(), 2);
        assert_eq!(backend.history_system_count(&tenant_b).await.unwrap(), 1);
    }

    // ========================================================================
    // Additional Search Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_search_index_on_create() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "search-test-1",
            "identifier": [{
                "system": "http://example.org/mrn",
                "value": "MRN12345"
            }],
            "name": [{"family": "TestFamily", "given": ["TestGiven"]}],
            "birthDate": "1990-01-15"
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://example.org/mrn|MRN12345")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "search-test-1");
    }

    #[tokio::test]
    async fn postgres_integration_search_index_on_delete() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "id": "search-delete-1",
            "identifier": [{"system": "http://example.org", "value": "DEL123"}]
        });

        backend
            .create(&tenant, "Patient", patient, FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("DEL123")],
            chain: vec![],
            components: vec![],
        });

        let result_before = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result_before.resources.items.len(), 1);

        backend
            .delete(&tenant, "Patient", "search-delete-1")
            .await
            .unwrap();

        let result_after = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result_after.resources.items.len(),
            0,
            "Deleted resource should not be searchable"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_string_prefix() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "name-1",
                    "name": [{"family": "Smith", "given": ["John"]}]
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
                    "id": "name-2",
                    "name": [{"family": "Smithson", "given": ["Jane"]}]
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
                    "id": "name-3",
                    "name": [{"family": "Johnson", "given": ["Bob"]}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            2,
            "Should find 2 patients with name starting with Smith"
        );

        let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        assert!(ids.contains(&"name-1"));
        assert!(ids.contains(&"name-2"));
    }

    #[tokio::test]
    async fn postgres_integration_search_date() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "date-1",
                    "birthDate": "1990-01-15"
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
                    "id": "date-2",
                    "birthDate": "2000-06-20"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::eq("1990-01-15")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 1);
        assert_eq!(result.resources.items[0].id(), "date-1");
    }

    /// Dates carrying a negative UTC offset must index as the instant they name.
    ///
    /// The writer treated `2019-05-04T12:12:29-07:00` as zone-less, appended
    /// `+00:00`, and the resulting `...-07:00+00:00` failed to parse — at which
    /// point it silently indexed `Utc::now()`. Every date search over such a row
    /// was then answered against the ingestion time: `gt<any past date>` matched
    /// and `lt` did not (#494). The sibling test above uses a date-only
    /// `birthDate`, the one shape that always worked, which is how this survived.
    #[tokio::test]
    async fn postgres_integration_search_date_negative_utc_offset() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for (id, effective) in [
            ("obs-2019", "2019-05-04T12:12:29-07:00"),
            ("obs-2025", "2025-05-04T12:12:29-07:00"),
        ] {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": id,
                        "status": "final",
                        "code": {"coding": [{"code": "8867-4"}]},
                        "effectiveDateTime": effective
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        let date_query = |prefix: SearchPrefix, value: &str| {
            SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "date".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::new(prefix, value)],
                chain: vec![],
                components: vec![],
            })
        };
        fn ids(items: &[helios_persistence::types::StoredResource]) -> Vec<String> {
            let mut v: Vec<String> = items.iter().map(|r| r.id().to_string()).collect();
            v.sort();
            v
        }

        // Before the fix both rows carried the ingestion timestamp, so `gt` on a
        // past date matched both and `lt` matched neither.
        let after = backend
            .search(
                &tenant,
                &date_query(SearchPrefix::Gt, "2023-01-01T00:00:00+00:00"),
            )
            .await
            .unwrap();
        assert_eq!(
            ids(&after.resources.items),
            vec!["obs-2025"],
            "gt must exclude the 2019 row"
        );

        let before = backend
            .search(
                &tenant,
                &date_query(SearchPrefix::Lt, "2023-01-01T00:00:00+00:00"),
            )
            .await
            .unwrap();
        assert_eq!(
            ids(&before.resources.items),
            vec!["obs-2019"],
            "lt must find the 2019 row"
        );

        // Pin the offset arithmetic, not just parseability: 12:12:29-07:00 is
        // 19:12:29Z, so this one-hour window brackets it. A fix that merely
        // stripped the offset would store 12:12:29Z and fall outside.
        let bracketed = backend
            .search(
                &tenant,
                &date_query(SearchPrefix::Gt, "2019-05-04T19:00:00+00:00").with_parameter(
                    SearchParameter {
                        name: "date".to_string(),
                        param_type: SearchParamType::Date,
                        modifier: None,
                        values: vec![SearchValue::new(
                            SearchPrefix::Lt,
                            "2019-05-04T20:00:00+00:00",
                        )],
                        chain: vec![],
                        components: vec![],
                    },
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            ids(&bracketed.resources.items),
            vec!["obs-2019"],
            "-07:00 must convert to 19:12:29Z"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_reference() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-1",
                    "subject": {"reference": "Patient/patient-1"},
                    "code": {"coding": [{"code": "8867-4"}]},
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-2",
                    "subject": {"reference": "Patient/patient-1"},
                    "code": {"coding": [{"code": "9279-1"}]},
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs-3",
                    "subject": {"reference": "Patient/patient-2"},
                    "code": {"coding": [{"code": "8867-4"}]},
                    "status": "final"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Patient/patient-1")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(result.resources.items.len(), 2);

        let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        assert!(ids.contains(&"obs-1"));
        assert!(ids.contains(&"obs-2"));
    }

    /// A bare logical id must match a stored `Patient/<id>` reference.
    ///
    /// `Observation?patient=<id>` is the primary form in the spec and the shape
    /// Inferno uses throughout, but Postgres compared the raw search value
    /// against the stored `Patient/<id>` and so matched nothing — every clinical
    /// search returned an empty Bundle (#490). The sibling test above covers the
    /// `Type/id` form, which always worked; only that form was ever asserted,
    /// which is how the gap survived.
    #[tokio::test]
    async fn postgres_integration_search_reference_bare_id() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for (id, subject) in [
            ("obs-1", "Patient/patient-1"),
            ("obs-2", "Patient/patient-1"),
            ("obs-3", "Patient/patient-2"),
        ] {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": id,
                        "subject": {"reference": subject},
                        "code": {"coding": [{"code": "8867-4"}]},
                        "status": "final"
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        let bare_id = |modifier: Option<SearchModifier>| {
            SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                modifier,
                values: vec![SearchValue::eq("patient-1")],
                chain: vec![],
                components: vec![],
            })
        };

        for (label, query) in [
            ("bare id", bare_id(None)),
            (
                ":Type + bare id",
                bare_id(Some(SearchModifier::Type("Patient".to_string()))),
            ),
        ] {
            let result = backend.search(&tenant, &query).await.unwrap();
            let mut ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
            ids.sort_unstable();
            assert_eq!(
                ids,
                vec!["obs-1", "obs-2"],
                "{label} must match Patient/patient-1 and not the decoy patient-2"
            );
        }

        // The suffix match must not become a wildcard: `subject=%` matches the
        // literal id `%`, i.e. nothing, rather than every reference.
        let wildcard = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("%")],
            chain: vec![],
            components: vec![],
        });
        assert!(
            backend
                .search(&tenant, &wildcard)
                .await
                .unwrap()
                .resources
                .items
                .is_empty(),
            "a LIKE metacharacter must be matched literally, not as a wildcard"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_reference_identifier() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        // Patient with a known identifier, plus a decoy patient.
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "p-ident-1",
                    "identifier": [{"system": "http://hospital.org", "value": "MRN-42"}]
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
                    "id": "p-ident-2",
                    "identifier": [{"system": "http://hospital.org", "value": "MRN-99"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        for (oid, pid) in [("obs-i1", "p-ident-1"), ("obs-i2", "p-ident-2")] {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": oid,
                        "subject": {"reference": format!("Patient/{pid}")},
                        "status": "final"
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // subject:identifier matches the observation whose subject patient has
        // the given identifier.
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: Some(SearchModifier::Identifier),
            values: vec![SearchValue::eq("http://hospital.org|MRN-42")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();
        let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
        assert_eq!(ids, vec!["obs-i1"]);
    }

    #[tokio::test]
    async fn postgres_integration_search_tenant_isolation() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant_a = create_tenant("tenant-a");
        let tenant_b = create_tenant("tenant-b");

        backend
            .create(
                &tenant_a,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "tenant-iso-1",
                    "identifier": [{"system": "http://example.org", "value": "UNIQUE123"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("UNIQUE123")],
            chain: vec![],
            components: vec![],
        });

        let result_a = backend.search(&tenant_a, &query).await.unwrap();
        assert_eq!(result_a.resources.items.len(), 1);

        let result_b = backend.search(&tenant_b, &query).await.unwrap();
        assert_eq!(
            result_b.resources.items.len(),
            0,
            "Tenant B should not see tenant A's resources"
        );
    }

    #[tokio::test]
    async fn postgres_integration_search_multiple_parameters() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "multi-1",
                    "name": [{"family": "Smith"}],
                    "gender": "male"
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
                    "id": "multi-2",
                    "name": [{"family": "Smith"}],
                    "gender": "female"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "name".to_string(),
                param_type: SearchParamType::String,
                modifier: None,
                values: vec![SearchValue::eq("Smith")],
                chain: vec![],
                components: vec![],
            })
            .with_parameter(SearchParameter {
                name: "gender".to_string(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: vec![SearchValue::eq("male")],
                chain: vec![],
                components: vec![],
            });

        let result = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(
            result.resources.items.len(),
            1,
            "AND across params should find only 1 patient"
        );
        assert_eq!(result.resources.items[0].id(), "multi-1");
    }

    // ========================================================================
    // Conditional Operations Tests
    // ========================================================================

    #[tokio::test]
    async fn postgres_integration_conditional_create() {
        use helios_persistence::core::{ConditionalCreateResult, ConditionalStorage};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-12345"}],
            "name": [{"family": "Original"}]
        });

        let result = backend
            .conditional_create(
                &tenant,
                "Patient",
                patient,
                "identifier=http://hospital.org/mrn|MRN-12345",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(
            matches!(result, ConditionalCreateResult::Created(_)),
            "First conditional create should succeed"
        );
    }

    #[tokio::test]
    async fn postgres_integration_conditional_create_exists() {
        use helios_persistence::core::{ConditionalCreateResult, ConditionalStorage};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        let patient = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-EXISTS"}],
            "name": [{"family": "Original"}]
        });

        // First create
        backend
            .conditional_create(
                &tenant,
                "Patient",
                patient.clone(),
                "identifier=http://hospital.org/mrn|MRN-EXISTS",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Second conditional create - should return existing
        let patient2 = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-EXISTS"}],
            "name": [{"family": "Duplicate"}]
        });

        let result2 = backend
            .conditional_create(
                &tenant,
                "Patient",
                patient2,
                "identifier=http://hospital.org/mrn|MRN-EXISTS",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(
            matches!(result2, ConditionalCreateResult::Exists(_)),
            "Second conditional create should return existing resource"
        );
    }

    #[tokio::test]
    async fn postgres_integration_conditional_create_multiple_matches() {
        use helios_persistence::core::{ConditionalCreateResult, ConditionalStorage};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://system-a.org", "value": "SHARED-VALUE"}]
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
                    "identifier": [{"system": "http://system-b.org", "value": "SHARED-VALUE"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let result = backend
            .conditional_create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"value": "SHARED-VALUE"}]
                }),
                "identifier=SHARED-VALUE",
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(
            matches!(result, ConditionalCreateResult::MultipleMatches(_)),
            "Should report multiple matches"
        );
    }

    #[tokio::test]
    async fn postgres_integration_conditional_update() {
        use helios_persistence::core::{ConditionalStorage, ConditionalUpdateResult};

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-UPDATE-1"}],
                    "name": [{"family": "Original"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let updated_patient = json!({
            "resourceType": "Patient",
            "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-UPDATE-1"}],
            "name": [{"family": "Updated"}]
        });

        let result = backend
            .conditional_update(
                &tenant,
                "Patient",
                updated_patient,
                "identifier=http://hospital.org/mrn|MRN-UPDATE-1",
                false,
                FhirVersion::default(),
            )
            .await
            .unwrap();

        assert!(
            matches!(result, ConditionalUpdateResult::Updated(_)),
            "Conditional update should find and update resource"
        );

        if let ConditionalUpdateResult::Updated(updated) = result {
            assert_eq!(
                updated.content()["name"][0]["family"].as_str(),
                Some("Updated")
            );
        }
    }

    fn if_none_exist_entry(family: &str, full_url: &str) -> helios_persistence::core::BundleEntry {
        use helios_persistence::core::{BundleEntry, BundleMethod};
        BundleEntry {
            method: BundleMethod::Post,
            url: "Patient".to_string(),
            resource: Some(json!({
                "resourceType": "Patient",
                "identifier": [{"system": "http://example.org/mrn", "value": "MRN-TX-COND-1"}],
                "name": [{"family": family}]
            })),
            if_match: None,
            if_none_match: None,
            if_none_exist: Some("identifier=http://example.org/mrn|MRN-TX-COND-1".to_string()),
            full_url: Some(full_url.to_string()),
        }
    }

    /// `ifNoneExist` is resolved inside the transaction (#511): the same bundle
    /// twice answers 201 then 200, the 200 names the match, and one row exists.
    /// Two entries with the same criteria in one bundle see each other, since
    /// buffered creates are flushed before the search.
    #[tokio::test]
    async fn postgres_integration_transaction_if_none_exist_is_idempotent() {
        use helios_persistence::core::BundleProvider;

        let backend = create_backend().await;
        let tenant = create_tenant("tx-if-none-exist");

        let first = backend
            .process_transaction(
                &tenant,
                vec![if_none_exist_entry("First", "urn:uuid:first")],
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(first.entries[0].status, 201);

        let second = backend
            .process_transaction(
                &tenant,
                vec![if_none_exist_entry("Second", "urn:uuid:second")],
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            second.entries[0].status, 200,
            "the match is answered, not duplicated"
        );
        assert_eq!(second.entries[0].location, first.entries[0].location);
        assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);

        let tenant = create_tenant("tx-if-none-exist-same-bundle");
        let both = backend
            .process_transaction(
                &tenant,
                vec![
                    if_none_exist_entry("First", "urn:uuid:first"),
                    if_none_exist_entry("Second", "urn:uuid:second"),
                ],
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(both.entries[0].status, 201);
        assert_eq!(both.entries[1].status, 200);
        assert_eq!(both.entries[1].location, both.entries[0].location);
        assert_eq!(backend.count(&tenant, Some("Patient")).await.unwrap(), 1);
    }

    /// A `urn:uuid` reference to a matched `ifNoneExist` entry resolves to the
    /// match (R4 §3.1.0.11.2); several matches fail the entry with 412 and roll
    /// the bundle back.
    #[tokio::test]
    async fn postgres_integration_transaction_if_none_exist_resolves_references_and_rejects_ambiguity()
     {
        use helios_persistence::core::{BundleEntry, BundleMethod, BundleProvider};
        use helios_persistence::error::TransactionError;

        let backend = create_backend().await;
        let tenant = create_tenant("tx-if-none-exist-urn");

        let existing = backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://example.org/mrn", "value": "MRN-TX-COND-1"}],
                    "name": [{"family": "AlreadyThere"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let result = backend
            .process_transaction(
                &tenant,
                vec![
                    if_none_exist_entry("Duplicate", "urn:uuid:patient"),
                    BundleEntry {
                        method: BundleMethod::Post,
                        url: "Observation".to_string(),
                        resource: Some(json!({
                            "resourceType": "Observation",
                            "status": "final",
                            "code": {"text": "test"},
                            "subject": {"reference": "urn:uuid:patient"}
                        })),
                        if_match: None,
                        if_none_match: None,
                        if_none_exist: None,
                        full_url: Some("urn:uuid:observation".to_string()),
                    },
                ],
                FhirVersion::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.entries[0].status, 200);
        assert_eq!(result.entries[1].status, 201);
        let observation = result.entries[1].resource.as_ref().expect("created");
        assert_eq!(
            observation["subject"]["reference"],
            json!(format!("Patient/{}", existing.id()))
        );

        // A second identical patient makes the criteria ambiguous.
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://example.org/mrn", "value": "MRN-TX-COND-1"}],
                    "name": [{"family": "Twin"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let before = backend.count(&tenant, Some("Patient")).await.unwrap();

        let err = backend
            .process_transaction(
                &tenant,
                vec![
                    BundleEntry {
                        method: BundleMethod::Post,
                        url: "Patient".to_string(),
                        resource: Some(
                            json!({"resourceType": "Patient", "name": [{"family": "Plain"}]}),
                        ),
                        if_match: None,
                        if_none_match: None,
                        if_none_exist: None,
                        full_url: None,
                    },
                    if_none_exist_entry("Ambiguous", "urn:uuid:ambiguous"),
                ],
                FhirVersion::default(),
            )
            .await
            .expect_err("an ambiguous ifNoneExist must fail the bundle");
        match err {
            TransactionError::BundleError { index, message } => {
                assert_eq!(index, 1);
                assert!(message.contains("412"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(
            backend.count(&tenant, Some("Patient")).await.unwrap(),
            before,
            "the plain create in entry 0 must have been rolled back"
        );
    }

    #[tokio::test]
    async fn postgres_integration_conditional_delete() {
        use helios_persistence::core::{
            ConditionalDeleteResult, ConditionalStorage, SearchProvider,
        };
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "identifier": [{"system": "http://hospital.org/mrn", "value": "MRN-DELETE-1"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let result = backend
            .conditional_delete(
                &tenant,
                "Patient",
                "identifier=http://hospital.org/mrn|MRN-DELETE-1",
            )
            .await
            .unwrap();

        assert!(
            matches!(result, ConditionalDeleteResult::Deleted(_)),
            "Conditional delete should find and delete resource"
        );

        // Verify deletion by searching
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://hospital.org/mrn|MRN-DELETE-1")],
            chain: vec![],
            components: vec![],
        });

        let search_result = backend.search(&tenant, &query).await.unwrap();
        assert!(
            search_result.resources.items.is_empty(),
            "Resource should be deleted"
        );
    }

    // ========================================================================
    // Reindex Tests
    // ========================================================================

    async fn reindex_test_client_for(dbname: &str) -> tokio_postgres::Client {
        let pg = shared_pg().await;
        let conn_str = format!(
            "host={} port={} user=postgres password=postgres dbname={dbname}",
            pg.host, pg.port
        );
        let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
            .await
            .expect("connect to shared pg for reindex assertions");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
    }

    async fn reindex_test_client() -> tokio_postgres::Client {
        reindex_test_client_for("postgres").await
    }

    /// Creates a dedicated database for tests that install triggers or table locks.
    ///
    /// Unique tenant IDs isolate rows, but PostgreSQL DDL still locks the shared
    /// `search_index` and `resource_fts` tables and can deadlock parallel tests.
    async fn isolated_reindex_backend() -> (PostgresBackend, String) {
        isolated_reindex_backend_with_max_connections(5).await
    }

    async fn isolated_reindex_backend_with_max_connections(
        max_connections: usize,
    ) -> (PostgresBackend, String) {
        let pg = shared_pg().await;
        let dbname = format!("reindex_test_{}", uuid::Uuid::new_v4().simple());
        reindex_test_client()
            .await
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .expect("create isolated reindex database");

        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .map(|path| path.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));
        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: dbname.clone(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections,
            data_dir: Some(data_dir),
            ..Default::default()
        };
        let schema_backend = PostgresBackend::new(PostgresConfig {
            max_connections: 5,
            ..config.clone()
        })
        .await
        .expect("connect to isolated reindex database");
        schema_backend
            .init_schema()
            .await
            .expect("initialize isolated reindex database");
        if max_connections == 5 {
            return (schema_backend, dbname);
        }
        drop(schema_backend);
        let backend = PostgresBackend::new(config)
            .await
            .expect("connect to initialized isolated reindex database");
        (backend, dbname)
    }

    /// Exercises the production deferred-reindex hook against PostgreSQL.
    ///
    /// It uses an isolated database because it briefly takes an exclusive lock
    /// on `search_index` so generation one is known to have fetched the old
    /// resource before generation two is enqueued.
    #[tokio::test]
    async fn postgres_integration_deferred_reindex_coordination() {
        use helios_persistence::core::DeferredReindexHook;
        use helios_persistence::search::{ReindexOnFinish, ReindexOperation, ReindexStatus};
        use std::sync::Arc;

        let (backend, dbname) = isolated_reindex_backend().await;
        let backend = Arc::new(backend);
        let tenant = create_tenant("deferred-reindex-coordination");
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let resource_id = format!("coord-{}", uuid::Uuid::new_v4().simple());
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": resource_id,
                    "name": [{"family": "BeforeCoordination"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let operation = Arc::new(ReindexOperation::new(
            backend.clone(),
            backend.tenant_registries().clone(),
        ));
        let hook = ReindexOnFinish::with_max_concurrency(operation.clone(), 2);

        let mut lock_client = reindex_test_client_for(&dbname).await;
        let observer = reindex_test_client_for(&dbname).await;
        let transaction = lock_client.transaction().await.unwrap();
        let blocker_pid: i32 = transaction
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        transaction
            .batch_execute("LOCK TABLE search_index IN ACCESS EXCLUSIVE MODE")
            .await
            .unwrap();

        hook.reindex_types(&tenant, vec!["Patient".to_string()])
            .await;

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let blocked: i64 = observer
                    .query_one(
                        "SELECT COUNT(*) FROM pg_stat_activity
                         WHERE $1 = ANY(pg_blocking_pids(pid))",
                        &[&blocker_pid],
                    )
                    .await
                    .unwrap()
                    .get(0);
                if blocked > 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("first reindex generation did not block on the controlled table lock");

        let jobs_while_blocked = operation.list_jobs();
        assert_eq!(jobs_while_blocked.len(), 1);
        assert_eq!(jobs_while_blocked[0].status, ReindexStatus::InProgress);

        let updated = json!({
            "resourceType": "Patient",
            "id": resource_id,
            "name": [{"family": "FreshCoordination"}]
        });
        transaction
            .execute(
                "UPDATE resources SET data = $3
                 WHERE tenant_id = $1 AND resource_type = 'Patient' AND id = $2",
                &[&tenant_id, &resource_id, &updated],
            )
            .await
            .unwrap();

        // The second callback arrives after generation one fetched the stale
        // value. It must merge into a pending generation, not start a second
        // PostgreSQL scan while generation one is blocked.
        hook.reindex_types(&tenant, vec!["Patient".to_string()])
            .await;
        assert_eq!(
            operation.list_jobs().len(),
            1,
            "compatible automatic requests overlapped as physical jobs"
        );

        transaction.commit().await.unwrap();

        let mut jobs = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let jobs = operation.list_jobs();
                if jobs.len() == 2 && jobs.iter().all(|job| job.status.is_finished()) {
                    break jobs;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("deferred reindex generations did not finish");
        jobs.sort_by_key(|job| job.started_at.clone());
        assert!(
            jobs.iter()
                .all(|job| { job.status == ReindexStatus::Completed && !job.has_errors() }),
            "both generations must complete cleanly: {jobs:?}"
        );
        let first_completed =
            chrono::DateTime::parse_from_rfc3339(jobs[0].completed_at.as_deref().unwrap()).unwrap();
        let second_started =
            chrono::DateTime::parse_from_rfc3339(jobs[1].started_at.as_deref().unwrap()).unwrap();
        assert!(
            first_completed <= second_started,
            "same-tenant generations overlapped: {jobs:?}"
        );

        let family_rows: Vec<(String, i64)> = observer
            .query(
                "SELECT value_string, COUNT(*)
                 FROM search_index
                 WHERE tenant_id = $1 AND resource_type = 'Patient'
                   AND resource_id = $2 AND param_name = 'family'
                 GROUP BY value_string ORDER BY value_string",
                &[&tenant_id, &resource_id],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
        assert_eq!(family_rows, vec![("FreshCoordination".to_string(), 1)]);

        let fts_rows: i64 = observer
            .query_one(
                "SELECT COUNT(*) FROM resource_fts
                 WHERE tenant_id = $1 AND resource_type = 'Patient' AND resource_id = $2",
                &[&tenant_id, &resource_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(fts_rows, 1, "the follow-up generation duplicated FTS rows");
    }

    #[tokio::test]
    async fn postgres_integration_reindex_page_batches_and_scopes_writes() {
        use helios_persistence::search::{ReindexSource, ReindexTarget};

        let backend = create_backend().await;
        let tenant = create_tenant("reindex-page-batch");
        let other_tenant = create_tenant("reindex-page-bystander");
        let tenant_id = tenant.tenant_id().as_str();
        let other_tenant_id = other_tenant.tenant_id().as_str();

        for (id, family) in [
            ("page-a", "Able"),
            ("page-b", "Baker"),
            ("outside", "Clark"),
        ] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "name": [{"family": family}],
                        "contained": [{
                            "resourceType": "Practitioner",
                            "id": format!("contained-{id}"),
                            "name": [{"family": format!("Contained {family}")}]
                        }]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        backend
            .create(
                &other_tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "page-a",
                    "name": [{"family": "Bystander"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let client = reindex_test_client().await;
        for (scoped_tenant, id) in [
            (tenant_id, "page-a"),
            (tenant_id, "page-b"),
            (tenant_id, "outside"),
            (other_tenant_id, "page-a"),
        ] {
            client
                .execute(
                    "INSERT INTO search_index
                     (tenant_id, resource_type, resource_id, param_name, value_string)
                     VALUES ($1, 'Patient', $2, 'obsolete-batch-probe', 'stale')",
                    &[&scoped_tenant, &id],
                )
                .await
                .unwrap();
        }

        let page = backend
            .fetch_resources_page(&tenant, "Patient", None, 2)
            .await
            .unwrap();
        assert_eq!(
            page.resources.iter().map(|r| r.id()).collect::<Vec<_>>(),
            vec!["page-a", "page-b"]
        );

        let before: Vec<(String, serde_json::Value)> = client
            .query(
                "SELECT version_id, data FROM resources
                 WHERE tenant_id = $1 AND resource_type = 'Patient'
                   AND id = ANY($2::text[]) ORDER BY id",
                &[&tenant_id, &vec!["page-a", "page-b"]],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
        let history_before: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = 'Patient'
                   AND id = ANY($2::text[])",
                &[&tenant_id, &vec!["page-a", "page-b"]],
            )
            .await
            .unwrap()
            .get(0);

        let results = backend
            .write_search_entries_page(&tenant, &page.resources)
            .await;
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(Result::is_ok));
        assert!(results.iter().all(|result| *result.as_ref().unwrap() > 0));

        let remaining_stale: Vec<(String, String)> = client
            .query(
                "SELECT tenant_id, resource_id FROM search_index
                 WHERE param_name = 'obsolete-batch-probe'
                   AND ((tenant_id = $1 AND resource_id = ANY($2::text[]))
                     OR (tenant_id = $3 AND resource_id = 'page-a'))
                 ORDER BY tenant_id, resource_id",
                &[
                    &tenant_id,
                    &vec!["page-a", "page-b", "outside"],
                    &other_tenant_id,
                ],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
        assert_eq!(remaining_stale.len(), 2);
        assert!(remaining_stale.contains(&(tenant_id.to_string(), "outside".to_string())));
        assert!(remaining_stale.contains(&(other_tenant_id.to_string(), "page-a".to_string())));

        for resource in &page.resources {
            let contained_count: i64 = client
                .query_one(
                    "SELECT COUNT(*) FROM search_index
                     WHERE tenant_id = $1 AND resource_type = 'Patient'
                       AND resource_id = $2 AND is_contained = TRUE",
                    &[&tenant_id, &resource.id()],
                )
                .await
                .unwrap()
                .get(0);
            assert!(contained_count > 0);
        }

        let after: Vec<(String, serde_json::Value)> = client
            .query(
                "SELECT version_id, data FROM resources
                 WHERE tenant_id = $1 AND resource_type = 'Patient'
                   AND id = ANY($2::text[]) ORDER BY id",
                &[&tenant_id, &vec!["page-a", "page-b"]],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
        let history_after: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history
                 WHERE tenant_id = $1 AND resource_type = 'Patient'
                   AND id = ANY($2::text[])",
                &[&tenant_id, &vec!["page-a", "page-b"]],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(after, before);
        assert_eq!(history_after, history_before);
    }

    #[tokio::test]
    async fn postgres_integration_reindex_page_uses_one_transaction_for_all_index_writes() {
        use helios_persistence::search::{ReindexSource, ReindexTarget};

        let (backend, dbname) = isolated_reindex_backend().await;
        let tenant = create_tenant("reindex-page-single-transaction");
        let tenant_id = tenant.tenant_id().as_str();
        for id in ["tx-a", "tx-b"] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "name": [{"family": id}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let page = backend
            .fetch_resources_page(&tenant, "Patient", None, 10)
            .await
            .unwrap();

        let client = reindex_test_client_for(&dbname).await;
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let table_name = format!("reindex_tx_probe_{suffix}");
        let function_name = format!("record_reindex_tx_{suffix}");
        let search_trigger = format!("record_search_tx_{suffix}");
        let fts_trigger = format!("record_fts_tx_{suffix}");
        client
            .batch_execute(&format!(
                "CREATE TABLE {table_name} (operation text NOT NULL, transaction_id bigint NOT NULL);
                 CREATE FUNCTION {function_name}() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF TG_OP = 'DELETE' THEN
                     IF OLD.tenant_id = '{tenant_id}' THEN
                       INSERT INTO {table_name} VALUES (TG_TABLE_NAME || '_delete', txid_current());
                     END IF;
                     RETURN OLD;
                   END IF;
                   IF NEW.tenant_id = '{tenant_id}' THEN
                     INSERT INTO {table_name} VALUES (TG_TABLE_NAME || '_insert', txid_current());
                   END IF;
                   RETURN NEW;
                 END $$;
                 CREATE TRIGGER {search_trigger} AFTER INSERT OR DELETE ON search_index
                 FOR EACH ROW EXECUTE FUNCTION {function_name}();
                 CREATE TRIGGER {fts_trigger} AFTER INSERT OR DELETE ON resource_fts
                 FOR EACH ROW EXECUTE FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        let results = backend
            .write_search_entries_page(&tenant, &page.resources)
            .await;
        let probe_rows: Vec<(String, i64)> = client
            .query(
                &format!(
                    "SELECT DISTINCT operation, transaction_id FROM {table_name} ORDER BY operation"
                ),
                &[],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();

        client
            .batch_execute(&format!(
                "DROP TRIGGER {search_trigger} ON search_index;
                 DROP TRIGGER {fts_trigger} ON resource_fts;
                 DROP FUNCTION {function_name}();
                 DROP TABLE {table_name};"
            ))
            .await
            .unwrap();

        assert!(results.iter().all(Result::is_ok));
        assert_eq!(
            probe_rows
                .iter()
                .map(|(operation, _)| operation.as_str())
                .collect::<Vec<_>>(),
            vec![
                "resource_fts_delete",
                "resource_fts_insert",
                "search_index_delete",
                "search_index_insert",
            ]
        );
        assert_eq!(
            probe_rows
                .iter()
                .map(|(_, transaction_id)| *transaction_id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            1,
            "all page index writes must share one PostgreSQL transaction"
        );
    }

    #[tokio::test]
    async fn postgres_integration_reindex_page_failure_releases_single_connection_pool() {
        use helios_persistence::search::{ReindexSource, ReindexTarget};

        let (backend, dbname) = isolated_reindex_backend_with_max_connections(1).await;
        let tenant = create_tenant("reindex-page-one-connection");
        let tenant_id = tenant.tenant_id().as_str();
        for id in ["one-a", "one-fail", "one-c"] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "name": [{"family": id}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let page = backend
            .fetch_resources_page(&tenant, "Patient", None, 10)
            .await
            .unwrap();
        let client = reindex_test_client_for(&dbname).await;
        client
            .execute(
                "INSERT INTO search_index
                 (tenant_id, resource_type, resource_id, param_name, value_string)
                 VALUES ($1, 'Patient', 'one-fail', 'obsolete-one-connection', 'stale')",
                &[&tenant_id],
            )
            .await
            .unwrap();
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let function_name = format!("reject_one_connection_{suffix}");
        let trigger_name = format!("reject_one_connection_{suffix}");
        client
            .batch_execute(&format!(
                "CREATE FUNCTION {function_name}() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF NEW.tenant_id = '{tenant_id}' AND NEW.resource_id = 'one-fail' THEN
                     RAISE EXCEPTION 'deliberate one-connection reindex failure';
                   END IF;
                   RETURN NEW;
                 END $$;
                 CREATE TRIGGER {trigger_name} BEFORE INSERT ON search_index
                 FOR EACH ROW EXECUTE FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        let results = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            backend.write_search_entries_page(&tenant, &page.resources),
        )
        .await
        .expect("fallback must not deadlock a one-connection pool");

        client
            .batch_execute(&format!(
                "DROP TRIGGER {trigger_name} ON search_index;
                 DROP FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        assert_eq!(results.len(), 3);
        for (resource, result) in page.resources.iter().zip(&results) {
            if resource.id() == "one-fail" {
                let error = result.as_ref().expect_err("trigger target must fail");
                assert!(
                    error
                        .to_string()
                        .contains("deliberate one-connection reindex failure"),
                    "deliberate database cause was lost: {error}"
                );
            } else {
                assert!(result.is_ok(), "{} should be reindexed", resource.id());
            }
        }
        for table in ["search_index", "resource_fts"] {
            let stale: i64 = client
                .query_one(
                    &format!(
                        "SELECT COUNT(*) FROM {table}
                         WHERE tenant_id = $1 AND resource_type = 'Patient'
                           AND resource_id = 'one-fail'"
                    ),
                    &[&tenant_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(stale, 0, "failed resource retained stale rows in {table}");
        }
    }

    #[tokio::test]
    async fn postgres_integration_reindex_status_attributes_page_resource_error() {
        use helios_persistence::search::{ReindexOperation, ReindexRequest, ReindexStatus};
        use std::sync::Arc;

        let (backend, dbname) = isolated_reindex_backend().await;
        let backend = Arc::new(backend);
        let tenant = create_tenant("reindex-page-status-error");
        let tenant_id = tenant.tenant_id().as_str();
        for id in ["status-a", "status-fail", "status-c"] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "name": [{"family": id}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let client = reindex_test_client_for(&dbname).await;
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let function_name = format!("reject_status_reindex_{suffix}");
        let trigger_name = format!("reject_status_reindex_{suffix}");
        client
            .batch_execute(&format!(
                "CREATE FUNCTION {function_name}() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF NEW.tenant_id = '{tenant_id}' AND NEW.resource_id = 'status-fail' THEN
                     RAISE EXCEPTION 'deliberate status reindex failure';
                   END IF;
                   RETURN NEW;
                 END $$;
                 CREATE TRIGGER {trigger_name} BEFORE INSERT ON search_index
                 FOR EACH ROW EXECUTE FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        let operation = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
        let job_id = operation
            .start(
                tenant.clone(),
                ReindexRequest::for_types(["Patient"]).with_batch_size(10),
                None,
            )
            .await
            .unwrap();
        let progress = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let progress = operation.get_progress(&job_id).await.unwrap();
                if progress.status.is_finished() {
                    break progress;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("reindex status should become terminal");

        client
            .batch_execute(&format!(
                "DROP TRIGGER {trigger_name} ON search_index;
                 DROP FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        assert_eq!(progress.status, ReindexStatus::Completed);
        assert_eq!(progress.total_resources, 3);
        assert_eq!(progress.processed_resources, 3);
        assert_eq!(progress.errors.len(), 1);
        assert_eq!(progress.errors[0].resource_type, "Patient");
        assert_eq!(progress.errors[0].resource_id, "status-fail");
        assert!(
            progress.errors[0]
                .error
                .contains("deliberate status reindex failure")
        );
        let parameters = progress.to_parameters();
        let error_count = parameters["parameter"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "errorCount")
            .unwrap();
        assert_eq!(error_count["valueInteger"], json!(1));
    }

    #[tokio::test]
    async fn postgres_integration_reindex_page_handles_empty_and_extraction_failure() {
        use helios_persistence::search::ReindexTarget;
        use helios_persistence::types::StoredResource;

        let backend = create_backend().await;
        let tenant = create_tenant("reindex-page-extraction");
        let tenant_id = tenant.tenant_id().as_str();
        assert!(
            backend
                .write_search_entries_page(&tenant, &[])
                .await
                .is_empty()
        );

        let invalid = StoredResource::new(
            "Patient",
            "invalid-extraction",
            tenant.tenant_id().clone(),
            json!({"resourceType": "Observation", "id": "invalid-extraction"}),
            FhirVersion::default(),
        );
        let empty_fts = StoredResource::new(
            "Patient",
            "empty-fts",
            tenant.tenant_id().clone(),
            json!({}),
            FhirVersion::default(),
        );
        let client = reindex_test_client().await;
        client
            .execute(
                "INSERT INTO search_index
                 (tenant_id, resource_type, resource_id, param_name, value_string)
                 VALUES ($1, 'Patient', 'invalid-extraction', 'obsolete-batch-probe', 'stale')",
                &[&tenant_id],
            )
            .await
            .unwrap();
        client
            .execute(
                "INSERT INTO resource_fts
                 (tenant_id, resource_type, resource_id, narrative_tsvector, content_tsvector)
                 VALUES ($1, 'Patient', 'empty-fts', to_tsvector('english', 'stale'), to_tsvector('english', 'stale'))",
                &[&tenant_id],
            )
            .await
            .unwrap();

        let results = backend
            .write_search_entries_page(&tenant, &[invalid, empty_fts])
            .await;
        assert_eq!(results.len(), 2);
        assert!(results[0].is_err());
        assert!(results[1].is_ok());

        let stale_count: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM search_index
                 WHERE tenant_id = $1 AND resource_id = 'invalid-extraction'",
                &[&tenant_id],
            )
            .await
            .unwrap()
            .get(0);
        let empty_fts_count: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM resource_fts
                 WHERE tenant_id = $1 AND resource_id = 'empty-fts'",
                &[&tenant_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(stale_count, 0);
        assert_eq!(empty_fts_count, 0);
    }

    #[tokio::test]
    async fn postgres_integration_reindex_page_rolls_back_and_falls_back_per_resource() {
        use helios_persistence::search::{ReindexSource, ReindexTarget};

        let (backend, dbname) = isolated_reindex_backend().await;
        let tenant = create_tenant("reindex-page-fallback");
        let tenant_id = tenant.tenant_id().as_str();
        for id in ["fallback-a", "fallback-fail", "fallback-c"] {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "name": [{"family": id}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let page = backend
            .fetch_resources_page(&tenant, "Patient", None, 10)
            .await
            .unwrap();
        let client = reindex_test_client_for(&dbname).await;
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let function_name = format!("reject_reindex_{suffix}");
        let trigger_name = format!("reject_reindex_{suffix}");
        client
            .batch_execute(&format!(
                "CREATE FUNCTION {function_name}() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF NEW.tenant_id = '{tenant_id}' AND NEW.resource_id = 'fallback-fail' THEN
                     RAISE EXCEPTION 'deliberate reindex failure';
                   END IF;
                   RETURN NEW;
                 END $$;
                 CREATE TRIGGER {trigger_name} BEFORE INSERT ON search_index
                 FOR EACH ROW EXECUTE FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        let results = backend
            .write_search_entries_page(&tenant, &page.resources)
            .await;

        client
            .batch_execute(&format!(
                "DROP TRIGGER {trigger_name} ON search_index;
                 DROP FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        assert_eq!(results.len(), 3);
        for (resource, result) in page.resources.iter().zip(results) {
            if resource.id() == "fallback-fail" {
                assert!(result.is_err());
            } else {
                assert!(result.is_ok(), "{} should be reindexed", resource.id());
                let rows: i64 = client
                    .query_one(
                        "SELECT COUNT(*) FROM search_index
                         WHERE tenant_id = $1 AND resource_type = 'Patient'
                           AND resource_id = $2",
                        &[&tenant_id, &resource.id()],
                    )
                    .await
                    .unwrap()
                    .get(0);
                assert!(rows > 0);
            }
        }
    }

    #[tokio::test]
    async fn postgres_integration_reindex_page_retries_oversized_fts_individually() {
        use helios_persistence::search::ReindexTarget;
        use helios_persistence::types::StoredResource;

        let backend = create_backend().await;
        let tenant = create_tenant("reindex-page-oversized-fts");
        let tenant_id = tenant.tenant_id().as_str();
        let text = (0..100_000)
            .map(|index| format!("lexeme{index:08x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let resource = StoredResource::new(
            "Patient",
            "oversized-fts",
            tenant.tenant_id().clone(),
            json!({
                "resourceType": "Patient",
                "id": "oversized-fts",
                "text": {"status": "generated", "div": format!("<div>{text}</div>")}
            }),
            FhirVersion::default(),
        );

        let results = backend
            .write_search_entries_page(&tenant, &[resource])
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok());

        let client = reindex_test_client().await;
        let fts_rows: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM resource_fts
                 WHERE tenant_id = $1 AND resource_type = 'Patient'
                   AND resource_id = 'oversized-fts'",
                &[&tenant_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(fts_rows, 1);
    }

    #[tokio::test]
    async fn postgres_integration_reindex_clear_rolls_back_when_fts_delete_fails() {
        use helios_persistence::search::ReindexTarget;

        let (backend, dbname) = isolated_reindex_backend_with_max_connections(1).await;
        let tenant = create_tenant("reindex-clear-atomic");
        let tenant_id = tenant.tenant_id().as_str();
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "clear-atomic",
                    "name": [{"family": "Atomic"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let client = reindex_test_client_for(&dbname).await;
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let function_name = format!("reject_fts_clear_{suffix}");
        let trigger_name = format!("reject_fts_clear_{suffix}");
        client
            .batch_execute(&format!(
                "CREATE FUNCTION {function_name}() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF OLD.tenant_id = '{tenant_id}' THEN
                     RAISE EXCEPTION 'deliberate FTS clear failure';
                   END IF;
                   RETURN OLD;
                 END $$;
                 CREATE TRIGGER {trigger_name} BEFORE DELETE ON resource_fts
                 FOR EACH ROW EXECUTE FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        let failed_clear = backend.clear_search_index(&tenant).await;

        client
            .batch_execute(&format!(
                "DROP TRIGGER {trigger_name} ON resource_fts;
                 DROP FUNCTION {function_name}();"
            ))
            .await
            .unwrap();

        assert!(failed_clear.is_err());
        for table in ["search_index", "resource_fts"] {
            let rows: i64 = client
                .query_one(
                    &format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = $1"),
                    &[&tenant_id],
                )
                .await
                .unwrap()
                .get(0);
            assert!(rows > 0, "{table} deletion should have rolled back");
        }

        assert!(backend.clear_search_index(&tenant).await.unwrap() > 0);
        for table in ["search_index", "resource_fts"] {
            let rows: i64 = client
                .query_one(
                    &format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = $1"),
                    &[&tenant_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(rows, 0);
        }
    }

    #[tokio::test]
    async fn postgres_integration_reindex_list_types() {
        use helios_persistence::search::ReindexSource;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Observation",
                json!({"resourceType": "Observation", "id": "o1", "status": "final"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType": "Patient", "id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let types = backend.list_resource_types(&tenant).await.unwrap();
        assert!(types.contains(&"Patient".to_string()));
        assert!(types.contains(&"Observation".to_string()));
        assert_eq!(types.len(), 2);
    }

    #[tokio::test]
    async fn postgres_integration_reindex_count() {
        use helios_persistence::search::ReindexSource;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for i in 1..=5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("patient-{}", i)
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        let count = backend.count_resources(&tenant, "Patient").await.unwrap();
        assert_eq!(count, 5);

        let count = backend
            .count_resources(&tenant, "Observation")
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn postgres_integration_reindex_fetch_page() {
        use helios_persistence::search::ReindexSource;

        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");

        for i in 1..=10 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("patient-{:02}", i)
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // Force every resource onto the same timestamp so the cursor's id
        // tiebreaker, rather than incidental clock ordering, carries the page.
        reindex_test_client()
            .await
            .execute(
                "UPDATE resources SET last_updated = '2026-01-01T00:00:00Z'
                 WHERE tenant_id = $1 AND resource_type = 'Patient'",
                &[&tenant.tenant_id().as_str()],
            )
            .await
            .unwrap();

        // Fetch first page (5 resources)
        let page1 = backend
            .fetch_resources_page(&tenant, "Patient", None, 5)
            .await
            .unwrap();
        assert_eq!(page1.resources.len(), 5);
        assert!(page1.next_cursor.is_some());

        // Fetch second page using cursor
        let page2 = backend
            .fetch_resources_page(&tenant, "Patient", page1.next_cursor.as_deref(), 5)
            .await
            .unwrap();
        assert_eq!(page2.resources.len(), 5);

        // Ensure no duplicates between pages
        let page1_ids: Vec<&str> = page1.resources.iter().map(|r| r.id()).collect();
        let page2_ids: Vec<&str> = page2.resources.iter().map(|r| r.id()).collect();
        assert_eq!(
            page1_ids,
            vec![
                "patient-01",
                "patient-02",
                "patient-03",
                "patient-04",
                "patient-05"
            ]
        );
        assert_eq!(
            page2_ids,
            vec![
                "patient-06",
                "patient-07",
                "patient-08",
                "patient-09",
                "patient-10"
            ]
        );
        for id in &page1_ids {
            assert!(!page2_ids.contains(id), "Duplicate ID found: {}", id);
        }

        // Fetch third page (should be empty or have no more cursor)
        let page3 = backend
            .fetch_resources_page(&tenant, "Patient", page2.next_cursor.as_deref(), 5)
            .await
            .unwrap();
        assert!(page3.resources.is_empty() || page3.next_cursor.is_none());
    }

    /// Inserts a row directly into the search_index table. Mirrors what the
    /// SQLite chain tests do for the same purpose — exercises the chain SQL
    /// without depending on the FHIRPath extractor's full coverage. Connects
    /// to the shared testcontainer with its own tokio-postgres client because
    /// `PostgresBackend::get_client` is crate-private.
    async fn insert_search_index(
        tenant_id: &str,
        resource_type: &str,
        resource_id: &str,
        param_name: &str,
        column: &str,
        value: &str,
    ) {
        let pg = shared_pg().await;
        let conn_str = format!(
            "host={} port={} user=postgres password=postgres dbname=postgres",
            pg.host, pg.port,
        );
        let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
            .await
            .expect("connect to shared pg");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let sql = format!(
            "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, {col}) \
             VALUES ($1, $2, $3, $4, $5)",
            col = column,
        );
        client
            .execute(
                &sql,
                &[
                    &tenant_id,
                    &resource_type,
                    &resource_id,
                    &param_name,
                    &value,
                ],
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn postgres_integration_resolve_chain_multi_level() {
        use helios_persistence::core::ChainedSearchProvider;

        // Mirror sqlite/search_impl.rs::test_resolve_chain_multi_level for
        // Postgres. Three-level chain: Observation?subject.organization.name=Hospital.
        let backend = create_backend().await;
        let tenant = create_tenant("chain-multi");
        let tenant_id = tenant.tenant_id().as_str();

        backend
            .create(
                &tenant,
                "Organization",
                json!({"id": "org1", "name": "General Hospital"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "managingOrganization": {"reference": "Organization/org1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1", "subject": {"reference": "Patient/p1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        insert_search_index(
            tenant_id,
            "Organization",
            "org1",
            "name",
            "value_string",
            "General Hospital",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Patient",
            "p1",
            "organization",
            "value_reference",
            "Organization/org1",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Observation",
            "o1",
            "subject",
            "value_reference",
            "Patient/p1",
        )
        .await;

        let ids = backend
            .resolve_chain(
                &tenant,
                "Observation",
                "subject.organization.name",
                "Hospital",
            )
            .await
            .unwrap();

        assert_eq!(ids, vec!["o1".to_string()]);
    }

    #[tokio::test]
    async fn postgres_integration_resolve_reverse_chain_terminal() {
        use helios_persistence::core::ChainedSearchProvider;
        use helios_persistence::types::{ReverseChainedParameter, SearchValue};

        // _has:Observation:subject:code=8867-4 — find patients referenced by
        // Observations whose code matches.
        let backend = create_backend().await;
        let tenant = create_tenant("reverse-chain");
        let tenant_id = tenant.tenant_id().as_str();

        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1", "subject": {"reference": "Patient/p1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o2", "subject": {"reference": "Patient/p2"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        insert_search_index(
            tenant_id,
            "Observation",
            "o1",
            "subject",
            "value_reference",
            "Patient/p1",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Observation",
            "o2",
            "subject",
            "value_reference",
            "Patient/p2",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Observation",
            "o1",
            "code",
            "value_token_code",
            "8867-4",
        )
        .await;
        insert_search_index(
            tenant_id,
            "Observation",
            "o2",
            "code",
            "value_token_code",
            "other",
        )
        .await;

        let rc = ReverseChainedParameter::terminal(
            "Observation",
            "subject",
            "code",
            SearchValue::eq("8867-4"),
        );
        let ids = backend
            .resolve_reverse_chain(&tenant, "Patient", &rc)
            .await
            .unwrap();
        assert_eq!(ids, vec!["p1".to_string()]);
    }

    // ========================================================================
    // Bulk Export — Phase 2 multi-instance job state on Postgres.
    // ========================================================================

    use chrono::{DateTime, Utc};
    use helios_persistence::core::bulk_export::{
        BulkExportStorage, ExportDataProvider, ExportRequest, ExportStatus, GroupExportProvider,
        PatientExportProvider, StartExportInput, TypeExportProgress,
    };
    use helios_persistence::core::bulk_export_worker::{
        ExportClaimStrategy, ExportWorkerStorage, LeaseError, WorkerId,
    };
    use std::time::Duration as StdDuration;

    fn export_input(request: ExportRequest) -> StartExportInput {
        StartExportInput {
            request,
            transaction_time: Utc::now(),
            request_url: "http://localhost/$export".to_string(),
            owner_subject: Some("pg-test".to_string()),
            fhir_version: FhirVersion::default(),
        }
    }

    /// Claims jobs in a loop until the lease for `target` is returned;
    /// releases any other jobs claimed along the way. Robust to concurrent
    /// tests sharing the testcontainers PostgreSQL instance.
    async fn claim_specific(
        backend: &helios_persistence::backends::postgres::PostgresBackend,
        worker_id: &WorkerId,
        target: &helios_persistence::core::bulk_export::ExportJobId,
        lease_duration: StdDuration,
    ) -> helios_persistence::core::bulk_export_worker::ExportJobLease {
        for _ in 0..100 {
            match backend.claim_next(worker_id, lease_duration).await.unwrap() {
                Some(lease) if &lease.job_id == target => return lease,
                Some(other) => {
                    // Drain other tests' jobs out of the queue by completing
                    // them (so the claim ordering moves on instead of
                    // looping back to the same job after `release`).
                    let _ = backend
                        .finish_export_job(
                            &other.tenant,
                            &other.job_id,
                            &other.worker_id,
                            other.fencing_token,
                        )
                        .await;
                }
                None => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        }
        panic!("never claimed the expected job");
    }

    #[tokio::test]
    async fn postgres_integration_export_claim_skip_locked() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-claim");

        let job_id = backend
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker_a = WorkerId::new(format!("pg-worker-a-{}", uuid::Uuid::new_v4()));
        let lease_a =
            claim_specific(&backend, &worker_a, &job_id, StdDuration::from_secs(60)).await;
        assert!(lease_a.fencing_token >= 1);

        // Worker A finishes via the fenced ExportWorkerStorage.
        backend
            .mark_export_in_progress(&tenant, &job_id, &worker_a, lease_a.fencing_token)
            .await
            .unwrap();
        backend
            .update_export_type_progress(
                &tenant,
                &job_id,
                &worker_a,
                lease_a.fencing_token,
                &TypeExportProgress::new("Patient"),
            )
            .await
            .unwrap();
        backend
            .finish_export_job(&tenant, &job_id, &worker_a, lease_a.fencing_token)
            .await
            .unwrap();

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.status, ExportStatus::Complete);
    }

    #[tokio::test]
    async fn postgres_integration_export_missing_group_is_group_not_found() {
        let backend = create_backend().await;
        let tenant = create_tenant("export-missing-group");

        let members_err = backend
            .get_group_members(&tenant, "nope")
            .await
            .expect_err("a nonexistent group must not resolve to an empty member list");
        match members_err {
            StorageError::BulkExport(BulkExportError::GroupNotFound { group_id }) => {
                assert_eq!(group_id, "nope");
            }
            other => panic!("expected GroupNotFound, got: {other:?}"),
        }

        let patients_err = backend
            .resolve_group_patient_ids(&tenant, "nope")
            .await
            .expect_err("resolving patients for a nonexistent group must fail");
        match patients_err {
            StorageError::BulkExport(BulkExportError::GroupNotFound { group_id }) => {
                assert_eq!(group_id, "nope");
            }
            other => panic!("expected GroupNotFound, got: {other:?}"),
        }
    }

    /// Pins a stored resource's `last_updated` so a window test does not depend
    /// on wall-clock timing.
    async fn pin_last_updated(backend: &PostgresBackend, id: &str, at: DateTime<Utc>) {
        let client = backend.get_client().await.unwrap();
        client
            .execute(
                "UPDATE resources SET last_updated = $1 WHERE id = $2",
                &[&at, &id],
            )
            .await
            .unwrap();
    }

    async fn seed_patient_at(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        at: DateTime<Utc>,
    ) -> String {
        let stored = backend
            .create(
                tenant,
                "Patient",
                serde_json::json!({"resourceType": "Patient"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let id = stored.id().to_string();
        pin_last_updated(backend, &id, at).await;
        id
    }

    fn instant(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// The `Patient` branch of the compartment fetch applies `_since`, the way
    /// the non-Patient branch below it always has.
    #[tokio::test]
    async fn postgres_integration_since_bounds_the_patient_compartment_branch() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("since-patient-branch");

        let stale = seed_patient_at(&backend, &tenant, instant("2026-01-01T00:00:00Z")).await;
        let fresh = seed_patient_at(&backend, &tenant, instant("2026-07-01T00:00:00Z")).await;

        // Both named explicitly, as a group export or an explicit `patient`
        // parameter would: the id list reaching this method is NOT pre-filtered.
        let ids = vec![stale.clone(), fresh.clone()];
        let request = ExportRequest::patient().with_since(instant("2026-06-01T00:00:00Z"));

        let batch = backend
            .fetch_patient_compartment_batch(&tenant, &request, "Patient", &ids, None, 10)
            .await
            .unwrap();

        assert_eq!(
            batch.lines.len(),
            1,
            "the stale patient must be filtered out"
        );
        assert!(batch.lines[0].contains(&fresh));
    }

    /// The new bound and the keyset cursor coexist: paging a filtered set
    /// neither drops nor repeats a row, and the bound does not consume the
    /// `$4`/`$5` the cursor clause needs.
    #[tokio::test]
    async fn postgres_integration_since_and_cursor_page_the_patient_branch() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("since-patient-paging");

        let stale = seed_patient_at(&backend, &tenant, instant("2026-01-01T00:00:00Z")).await;
        let a = seed_patient_at(&backend, &tenant, instant("2026-07-01T00:00:00Z")).await;
        let b = seed_patient_at(&backend, &tenant, instant("2026-07-02T00:00:00Z")).await;
        let c = seed_patient_at(&backend, &tenant, instant("2026-07-03T00:00:00Z")).await;

        let ids = vec![stale.clone(), a.clone(), b.clone(), c.clone()];
        let request = ExportRequest::patient().with_since(instant("2026-06-01T00:00:00Z"));

        let first = backend
            .fetch_patient_compartment_batch(&tenant, &request, "Patient", &ids, None, 2)
            .await
            .unwrap();
        assert_eq!(first.lines.len(), 2);
        assert!(!first.is_last);

        let second = backend
            .fetch_patient_compartment_batch(
                &tenant,
                &request,
                "Patient",
                &ids,
                first.next_cursor.as_deref(),
                2,
            )
            .await
            .unwrap();

        let all: Vec<&String> = first.lines.iter().chain(second.lines.iter()).collect();
        for id in [&a, &b, &c] {
            let hits = all.iter().filter(|l| l.contains(id.as_str())).count();
            assert_eq!(hits, 1, "each in-window patient appears exactly once");
        }
        assert!(
            !all.iter().any(|l| l.contains(stale.as_str())),
            "the stale patient never appears on any page"
        );
    }

    /// `_until` excludes a resource modified after the bound, and the count
    /// agrees with what the fetch emits.
    #[tokio::test]
    async fn postgres_integration_export_until_bounds_count_and_fetch() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-until");

        let early = seed_patient_at(&backend, &tenant, instant("2026-01-01T00:00:00Z")).await;
        let _late = seed_patient_at(&backend, &tenant, instant("2026-03-01T00:00:00Z")).await;

        let request = ExportRequest::system().with_until(instant("2026-02-01T00:00:00Z"));

        let count = backend
            .count_export_resources(&tenant, &request, "Patient")
            .await
            .unwrap();
        let batch = backend
            .fetch_export_batch(&tenant, &request, "Patient", None, 10)
            .await
            .unwrap();

        assert_eq!(count, 1, "count must apply the upper bound");
        assert_eq!(batch.lines.len(), 1, "fetch must apply the upper bound");
        assert_eq!(
            count as usize,
            batch.lines.len(),
            "count and fetch must agree about the window"
        );
        assert!(batch.lines[0].contains(&early));
    }

    /// The bound is inclusive, matching S3.
    #[tokio::test]
    async fn postgres_integration_export_until_is_inclusive() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-until-incl");

        seed_patient_at(&backend, &tenant, instant("2026-02-01T00:00:00Z")).await;

        let request = ExportRequest::system().with_until(instant("2026-02-01T00:00:00Z"));
        let batch = backend
            .fetch_export_batch(&tenant, &request, "Patient", None, 10)
            .await
            .unwrap();

        assert_eq!(
            batch.lines.len(),
            1,
            "a resource exactly on the bound is included"
        );
    }

    /// `_since` and `_until` together bound the window at both ends.
    #[tokio::test]
    async fn postgres_integration_export_since_and_until_bound_the_window() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-window");

        let _before = seed_patient_at(&backend, &tenant, instant("2025-12-01T00:00:00Z")).await;
        let inside = seed_patient_at(&backend, &tenant, instant("2026-01-15T00:00:00Z")).await;
        let _after = seed_patient_at(&backend, &tenant, instant("2026-03-01T00:00:00Z")).await;

        let request = ExportRequest::system()
            .with_since(instant("2026-01-01T00:00:00Z"))
            .with_until(instant("2026-02-01T00:00:00Z"));

        let count = backend
            .count_export_resources(&tenant, &request, "Patient")
            .await
            .unwrap();
        let batch = backend
            .fetch_export_batch(&tenant, &request, "Patient", None, 10)
            .await
            .unwrap();

        assert_eq!(count, 1);
        assert_eq!(batch.lines.len(), 1);
        assert!(batch.lines[0].contains(&inside));
    }

    /// The patient-compartment path applies the bound too — it builds its own
    /// query, separate from `fetch_export_batch`.
    #[tokio::test]
    async fn postgres_integration_export_until_bounds_patient_compartment() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-compartment");

        let early = seed_patient_at(&backend, &tenant, instant("2026-01-01T00:00:00Z")).await;
        let late = seed_patient_at(&backend, &tenant, instant("2026-03-01T00:00:00Z")).await;

        let request = ExportRequest::patient().with_until(instant("2026-02-01T00:00:00Z"));
        let ids = vec![early.clone(), late.clone()];

        let batch = backend
            .fetch_patient_compartment_batch(&tenant, &request, "Patient", &ids, None, 10)
            .await
            .unwrap();

        assert_eq!(
            batch.lines.len(),
            1,
            "the compartment fetch applies the upper bound"
        );
        assert!(batch.lines[0].contains(&early));
    }

    #[tokio::test]
    async fn postgres_integration_export_stale_worker_fenced_out() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-fence");

        let job_id = backend
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();

        // Worker A takes a very short lease, then Worker B reclaims.
        let worker_a = WorkerId::new(format!("pg-stale-a-{}", uuid::Uuid::new_v4()));
        let lease_a =
            claim_specific(&backend, &worker_a, &job_id, StdDuration::from_millis(1)).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let worker_b = WorkerId::new(format!("pg-stale-b-{}", uuid::Uuid::new_v4()));
        let lease_b =
            claim_specific(&backend, &worker_b, &job_id, StdDuration::from_secs(60)).await;
        assert!(lease_b.fencing_token > lease_a.fencing_token);

        // Worker A is fenced out from every mutation.
        assert!(matches!(
            backend
                .mark_export_in_progress(&tenant, &job_id, &worker_a, lease_a.fencing_token)
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));
        assert!(matches!(
            backend
                .finish_export_job(&tenant, &job_id, &worker_a, lease_a.fencing_token)
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));

        // Worker B can still finish.
        backend
            .finish_export_job(&tenant, &job_id, &worker_b, lease_b.fencing_token)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn postgres_integration_export_set_current_type_persists_and_is_fenced() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-current-type");

        let job_id = backend
            .start_export(&tenant, export_input(ExportRequest::system()))
            .await
            .unwrap();

        let worker = WorkerId::new(format!("pg-current-type-{}", uuid::Uuid::new_v4()));
        let lease = claim_specific(&backend, &worker, &job_id, StdDuration::from_secs(60)).await;

        backend
            .set_export_current_type(
                &tenant,
                &job_id,
                &worker,
                lease.fencing_token,
                Some("Patient"),
                1,
                3,
            )
            .await
            .unwrap();

        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.current_type, Some("Patient".to_string()));
        assert_eq!(progress.types_done, 1);
        assert_eq!(progress.types_total, 3);

        // A stale fencing token is rejected and leaves the status unchanged.
        let stale_token = lease.fencing_token + 1000;
        assert!(matches!(
            backend
                .set_export_current_type(
                    &tenant,
                    &job_id,
                    &worker,
                    stale_token,
                    Some("Observation"),
                    2,
                    3,
                )
                .await,
            Err(LeaseError::LeaseLost { .. })
        ));
        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.current_type, Some("Patient".to_string()));
        assert_eq!(progress.types_done, 1);

        // The terminal update clears the marker but keeps the counters.
        backend
            .finish_export_job(&tenant, &job_id, &worker, lease.fencing_token)
            .await
            .unwrap();
        let progress = backend.get_export_status(&tenant, &job_id).await.unwrap();
        assert_eq!(progress.current_type, None);
        assert_eq!(progress.types_done, 1);
        assert_eq!(progress.types_total, 3);
    }

    #[tokio::test]
    async fn postgres_integration_export_count_active_and_expire() {
        let _guard = BULK_EXPORT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("export-cleanup");

        for _ in 0..2 {
            backend
                .start_export(&tenant, export_input(ExportRequest::system()))
                .await
                .unwrap();
        }
        assert_eq!(backend.count_active_exports(&tenant).await.unwrap(), 2);
        assert_eq!(
            backend
                .count_exports_by_status(&tenant, ExportStatus::Accepted)
                .await
                .unwrap(),
            2
        );

        // Nothing is expired yet.
        let expired_now = backend
            .list_expired_exports(Utc::now(), StdDuration::from_secs(3600), 100)
            .await
            .unwrap();
        // Only completed/error/cancelled jobs can expire — these are accepted.
        assert!(expired_now.is_empty());
    }

    // ========================================================================
    // Per-user settings store
    // ========================================================================

    /// A user key unique to each test, so tests sharing the database don't
    /// collide on the single-row-per-user `user_settings` table.
    fn unique_user_key(prefix: &str) -> String {
        format!("{}|{}", prefix, uuid::Uuid::new_v4().simple())
    }

    /// `delete_settings` removes the row and reports whether one existed —
    /// the primitive the #270 legacy-key migration uses to move a document
    /// rather than leave a duplicate copy behind.
    #[tokio::test]
    async fn postgres_integration_settings_delete_is_idempotent() {
        let backend = create_backend().await;
        let user = unique_user_key("delete");

        // Absent is not an error, and reports "nothing removed".
        assert!(!backend.delete_settings(&user).await.unwrap());

        backend
            .put_settings(&user, json!({"theme": "dark"}), None)
            .await
            .unwrap();
        assert!(backend.get_settings(&user).await.unwrap().is_some());

        assert!(backend.delete_settings(&user).await.unwrap());
        assert!(backend.get_settings(&user).await.unwrap().is_none());
        assert!(!backend.delete_settings(&user).await.unwrap());
    }

    #[tokio::test]
    async fn postgres_integration_settings_get_missing_is_none() {
        let backend = create_backend().await;
        let user = unique_user_key("missing");
        assert!(backend.get_settings(&user).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn postgres_integration_settings_put_get_and_version() {
        let backend = create_backend().await;
        let user = unique_user_key("round-trip");
        let doc = json!({"theme": "dark", "recentQueries": {"Patient": ["name=smith"]}});

        let stored = backend
            .put_settings(&user, doc.clone(), None)
            .await
            .unwrap();
        assert_eq!(stored.version, 1);

        let fetched = backend.get_settings(&user).await.unwrap().unwrap();
        assert_eq!(fetched.document, doc);
        assert_eq!(fetched.version, 1);

        let second = backend
            .put_settings(&user, json!({"theme": "light"}), None)
            .await
            .unwrap();
        assert_eq!(second.version, 2);
    }

    #[tokio::test]
    async fn postgres_integration_settings_patch_merges_and_deletes() {
        let backend = create_backend().await;
        let user = unique_user_key("patch");
        backend
            .put_settings(
                &user,
                json!({"theme": "dark", "defaultTenant": "acme"}),
                None,
            )
            .await
            .unwrap();

        let patched = backend
            .patch_settings(
                &user,
                json!({"theme": "light", "defaultTenant": null}),
                None,
            )
            .await
            .unwrap();
        assert_eq!(patched.document, json!({"theme": "light"}));
        assert_eq!(patched.version, 2);
    }

    #[tokio::test]
    async fn postgres_integration_settings_optimistic_lock() {
        let backend = create_backend().await;
        let user = unique_user_key("lock");
        backend
            .put_settings(&user, json!({"a": 1}), None)
            .await
            .unwrap(); // version 1

        // Stale precondition is rejected.
        let err = backend
            .put_settings(&user, json!({"a": 2}), Some(0))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure { .. })
        ));

        // Matching precondition succeeds.
        let ok = backend
            .put_settings(&user, json!({"a": 2}), Some(1))
            .await
            .unwrap();
        assert_eq!(ok.version, 2);
    }

    /// A dedicated backend with one physical connection per writer, so a
    /// concurrency test can put every racer in flight at once instead of
    /// queuing behind `create_backend`'s shared 5-connection pool. Mirrors
    /// `postgres_integration_concurrent_pool_connections_all_carry_statement_timeout`'s
    /// pattern of building a bespoke config against the shared container.
    async fn create_backend_with_pool_size(pool_size: usize) -> PostgresBackend {
        let pg = shared_pg().await;
        let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .unwrap_or_else(|| PathBuf::from("data"));
        let config = PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: pool_size,
            data_dir: Some(data_dir),
            ..Default::default()
        };
        PostgresBackend::new(config)
            .await
            .expect("Failed to create PostgresBackend")
    }

    /// Forces `count` physical connections into existence and back into the
    /// idle pool before a race starts.
    ///
    /// deadpool creates connections lazily, and establishing a fresh one
    /// (TCP handshake + PostgreSQL startup/auth) can take longer than the
    /// whole read-modify-write these concurrency tests race — so on a cold
    /// pool, a barrier-released batch can end up serialized by connection
    /// setup alone: writer 1 finishes its entire transaction while writer 2
    /// is still waiting for its connection, which defeats the test. Holding
    /// `count` clients at once (forcing that many distinct connections) and
    /// then dropping them back into the pool removes that confound.
    async fn warm_pool(backend: &PostgresBackend, count: usize) {
        let mut clients = Vec::with_capacity(count);
        for _ in 0..count {
            clients.push(backend.get_client().await.expect("warm connection"));
        }
        drop(clients);
    }

    /// A user's very first write is the case `SELECT … FOR UPDATE` cannot
    /// protect on its own: with no row yet, there is nothing for the lock to
    /// block on, so racing writers must instead serialize on
    /// `write_settings`'s `pg_advisory_xact_lock`. Without it, every racer
    /// reads `None`, computes `new_version = 1` from an empty document, and
    /// the `INSERT … ON CONFLICT DO UPDATE` losers silently overwrite the
    /// winner — a lost update on creation. Firing many unconditional
    /// single-key patches at a brand-new user key and requiring every key to
    /// survive, with the version landing on exactly the writer count, is the
    /// same shape as `mongodb_integration_settings_concurrent_patches_serialize`.
    ///
    /// Runs on a multi-thread runtime with a dedicated pool sized to the
    /// writer count and a [`tokio::sync::Barrier`] releasing every task at
    /// once — a single-threaded runtime, or a shared pool a racer might queue
    /// behind, can make the whole batch execute close enough to sequentially
    /// that even the unfixed code never actually hits the race.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn postgres_integration_settings_concurrent_first_writes_never_lose_an_update() {
        use std::sync::Arc;

        const WRITERS: usize = 8;
        let backend = Arc::new(create_backend_with_pool_size(WRITERS).await);
        warm_pool(&backend, WRITERS).await;
        let user = unique_user_key("concurrent-first-write");
        let barrier = Arc::new(tokio::sync::Barrier::new(WRITERS));

        let mut handles = Vec::with_capacity(WRITERS);
        for i in 0..WRITERS {
            let backend = backend.clone();
            let user = user.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                backend
                    .patch_settings(&user, json!({ format!("k{i}"): i }), None)
                    .await
            }));
        }
        for h in handles {
            h.await
                .expect("patch task panicked")
                .expect("patch_settings failed");
        }

        let final_doc = backend
            .get_settings(&user)
            .await
            .unwrap()
            .expect("settings document missing after concurrent first writes");
        let obj = final_doc.document.as_object().unwrap();
        for i in 0..WRITERS {
            assert_eq!(
                obj.get(&format!("k{i}")),
                Some(&json!(i)),
                "key k{i} was lost to a read-modify-write race on the user's first write"
            );
        }
        // WRITERS patches, each a distinct successful write from version 0 upward.
        assert_eq!(final_doc.version, WRITERS as i64);
    }

    /// The `if_match_version = Some(0)` half of the same race: every racer
    /// asserts "this user does not exist yet", so exactly one `put_settings`
    /// call may succeed and the rest must observe the loser's own creation as
    /// an optimistic-lock conflict — never a raw backend error, and never a
    /// second silent `Ok`. Uses the same error shape as
    /// `postgres_integration_settings_optimistic_lock`.
    ///
    /// Same multi-thread + dedicated-pool + barrier setup as
    /// `postgres_integration_settings_concurrent_first_writes_never_lose_an_update`,
    /// for the same reason: this must exercise real concurrent creates, not a
    /// batch that happens to run one at a time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn postgres_integration_settings_concurrent_creates_have_a_single_winner() {
        use std::sync::Arc;

        const WRITERS: usize = 8;
        let backend = Arc::new(create_backend_with_pool_size(WRITERS).await);
        warm_pool(&backend, WRITERS).await;
        let user = unique_user_key("concurrent-create");
        let barrier = Arc::new(tokio::sync::Barrier::new(WRITERS));

        let mut handles = Vec::with_capacity(WRITERS);
        for i in 0..WRITERS {
            let backend = backend.clone();
            let user = user.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                backend
                    .put_settings(&user, json!({"createdBy": i}), Some(0))
                    .await
            }));
        }

        let mut ok_count = 0;
        let mut conflict_count = 0;
        for h in handles {
            match h.await.expect("put task panicked") {
                Ok(stored) => {
                    ok_count += 1;
                    assert_eq!(stored.version, 1);
                }
                Err(StorageError::Concurrency(ConcurrencyError::OptimisticLockFailure {
                    ..
                })) => {
                    conflict_count += 1;
                }
                Err(other) => panic!("expected an optimistic-lock conflict, got {other:?}"),
            }
        }
        assert_eq!(ok_count, 1, "exactly one create should win the race");
        assert_eq!(conflict_count, WRITERS - 1);
    }

    /// Issue #313: a tenant purge must reach the PHI-derived query strings a
    /// client stores in its settings document. Those rows are keyed by *user*,
    /// so none of `purge_tenant_data`'s tenant-scoped deletes touch them.
    ///
    /// The PostgreSQL-specific risk this proves is that the sweep runs inside
    /// the purge's own transaction (`SELECT … FOR UPDATE`), so the offboarding
    /// commits atomically: it cannot leave a tenant's saved queries behind after
    /// its records are gone.
    #[tokio::test]
    async fn postgres_integration_purge_tenant_settings() {
        let backend = create_backend().await;
        let user = unique_user_key("tenant-purge");
        let dotted = unique_user_key("tenant-purge-dotted");

        backend
            .put_settings(
                &user,
                json!({
                    "theme": "dark",
                    "byTenant": {
                        "acme-purge": {"savedQueries": {"Patient": {"q": {"query": "name=smith"}}}},
                        "beta-keep": {"savedQueries": {"Patient": {"q": {"query": "name=jones"}}}}
                    }
                }),
                None,
            )
            .await
            .unwrap();
        // A tenant id containing `.` and `/`, both permitted by
        // `admin_tenants::validate_tenant_id` — the reason the sweep edits a
        // parsed document rather than using a `jsonb` text path.
        backend
            .put_settings(
                &dotted,
                json!({"byTenant": {"org.a/b": {"savedQueries": {"Patient": {"q": {}}}}}}),
                None,
            )
            .await
            .unwrap();

        let before = backend.get_settings(&user).await.unwrap().unwrap();

        // Driven through `purge_tenant_data`, which is the single choke point
        // both the admin API and the web UI go through.
        backend.purge_tenant_data("acme-purge").await.unwrap();

        let after = backend.get_settings(&user).await.unwrap().unwrap();
        assert_eq!(after.document["theme"], "dark");
        assert!(after.document["byTenant"].get("acme-purge").is_none());
        assert_eq!(
            after.document["byTenant"]["beta-keep"]["savedQueries"]["Patient"]["q"]["query"],
            "name=jones"
        );
        assert!(
            !serde_json::to_string(&after.document)
                .unwrap()
                .contains("smith"),
            "purged content must not survive in the stored row"
        );
        assert_eq!(
            after.version,
            before.version + 1,
            "the version must bump so a stale ETag cannot write the content back"
        );

        // A tenant whose id is a prefix of another must not take it with it.
        backend.purge_tenant_data("org.a").await.unwrap();
        let dotted_doc = backend.get_settings(&dotted).await.unwrap().unwrap();
        assert!(
            dotted_doc.document["byTenant"].get("org.a/b").is_some(),
            "purging 'org.a' must not touch the tenant named 'org.a/b'"
        );
        backend.purge_tenant_data("org.a/b").await.unwrap();
        let dotted_doc = backend.get_settings(&dotted).await.unwrap().unwrap();
        assert_eq!(dotted_doc.document, json!({}));
    }

    /// A tenant with nothing in the settings store leaves every document at its
    /// original version, so no client ETag is needlessly invalidated.
    #[tokio::test]
    async fn postgres_integration_purge_tenant_settings_is_a_no_op_when_nothing_matches() {
        let backend = create_backend().await;
        let user = unique_user_key("tenant-purge-noop");
        backend
            .put_settings(&user, json!({"theme": "dark"}), None)
            .await
            .unwrap();
        let before = backend.get_settings(&user).await.unwrap().unwrap();

        backend
            .purge_tenant_data("tenant-that-has-no-settings")
            .await
            .unwrap();

        let after = backend.get_settings(&user).await.unwrap().unwrap();
        assert_eq!(after.version, before.version);
        assert_eq!(after.document, json!({"theme": "dark"}));
    }

    // ========================================================================
    // _contained / _containedType search
    // ========================================================================

    async fn seed_contained(backend: &PostgresBackend, tenant: &TenantContext) {
        backend
            .create(
                tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs1",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                    "subject": { "reference": "#p1" },
                    "contained": [{
                        "resourceType": "Patient",
                        "id": "p1",
                        "name": [{ "family": "Smith", "given": ["Contained"] }],
                        "gender": "male"
                    }]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "top1", "name": [{ "family": "Smith" }] }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    fn contained_name_query(
        mode: helios_persistence::types::ContainedMode,
        ret: helios_persistence::types::ContainedReturn,
    ) -> helios_persistence::types::SearchQuery {
        use helios_persistence::types::{
            SearchParamType, SearchParameter, SearchQuery, SearchValue,
        };
        let mut q = SearchQuery::new("Patient");
        q.contained = mode;
        q.contained_return = ret;
        q.parameters.push(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });
        q
    }

    #[tokio::test]
    async fn postgres_integration_contained_off_excludes_contained() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, ContainedReturn};
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &contained_name_query(ContainedMode::Off, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        assert_eq!(urls, vec!["Patient/top1"]);
    }

    #[tokio::test]
    async fn postgres_integration_contained_returns_container() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, ContainedReturn};
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &contained_name_query(ContainedMode::On, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        assert_eq!(urls, vec!["Observation/obs1"]);
    }

    #[tokio::test]
    async fn postgres_integration_contained_type_contained_returns_contained() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, ContainedReturn};
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &contained_name_query(ContainedMode::On, ContainedReturn::Contained),
            )
            .await
            .unwrap();
        assert_eq!(result.resources.items.len(), 1);
        let r = &result.resources.items[0];
        assert_eq!(r.resource_type(), "Patient");
        assert_eq!(r.id(), "p1");
        assert_eq!(r.content()["name"][0]["given"][0], "Contained");
    }

    #[tokio::test]
    async fn postgres_integration_contained_both_merges() {
        use helios_persistence::core::SearchProvider;
        use helios_persistence::types::{ContainedMode, ContainedReturn};
        let backend = create_backend().await;
        let tenant = create_tenant("test-tenant");
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &contained_name_query(ContainedMode::Both, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let mut urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        urls.sort();
        assert_eq!(urls, vec!["Observation/obs1", "Patient/top1"]);
    }

    #[tokio::test]
    async fn postgres_integration_supports_contained_search() {
        use helios_persistence::core::SearchProvider;
        let backend = create_backend().await;
        assert!(backend.supports_contained_search());
    }

    // ========================================================================
    // Backend error handling — a reachable but misconfigured store
    // ========================================================================
    //
    // `tests/backend_error_handling.rs` covers every backend's *unreachable*
    // case with no server at all. PostgreSQL has a gap that cannot be closed
    // there: `PostgresBackend::new` eagerly verifies connectivity, so an
    // unreachable server fails at construction and the caller never obtains a
    // backend to drive. That leaves every per-operation error arm in
    // `postgres/storage.rs` — the `internal_error(..)` mapping behind `read`,
    // `count`, `create` and friends — completely unexercised, which is exactly
    // the class of uncovered code that motivated this work.
    //
    // Reaching those arms needs a server that answers but cannot serve the
    // query, so this test lives here, with the shared container. It mirrors the
    // SQLite `unmigrated_store_surfaces_backend_error` test: connect to a
    // database whose schema was never created (`new()` runs no DDL —
    // `init_schema()` is a separate, opt-in call) and drive the core operations.
    // Every one of them hits `relation "resources" does not exist`.

    /// Operations against a reachable database with no schema must surface a
    /// backend error — never a misleading success.
    #[tokio::test]
    async fn postgres_integration_unmigrated_store_surfaces_backend_error() {
        let pg = shared_pg().await;

        // A database of our own, so we can leave it unmigrated without disturbing
        // the schema the rest of this suite shares.
        let dbname = format!("unmigrated_{}", uuid::Uuid::new_v4().simple());
        let admin_conn = format!(
            "host={} port={} user=postgres password=postgres dbname=postgres",
            pg.host, pg.port,
        );
        let (admin, connection) = tokio_postgres::connect(&admin_conn, tokio_postgres::NoTls)
            .await
            .expect("connect to shared pg");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        // `batch_execute` uses the simple query protocol. `execute` would use the
        // extended one, which wraps the statement in an implicit transaction —
        // and CREATE DATABASE cannot run inside a transaction block.
        admin
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .expect("create an empty database");

        let backend = PostgresBackend::new(PostgresConfig {
            host: pg.host.clone(),
            port: pg.port,
            dbname,
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 2,
            ..Default::default()
        })
        .await
        .expect("the server is reachable, so construction must succeed");

        // init_schema() is deliberately NOT called.

        let tenant = create_tenant("unmigrated");

        let read = backend.read(&tenant, "Patient", "does-not-exist").await;
        assert!(
            !matches!(read, Ok(None)),
            "read against an unmigrated database returned Ok(None) — a store we \
             could not query must not be indistinguishable from one where the \
             resource is genuinely absent"
        );
        assert!(
            matches!(read, Err(StorageError::Backend(_))),
            "expected a backend error from an unmigrated database, got {read:?}"
        );

        let count = backend.count(&tenant, Some("Patient")).await;
        assert!(
            !matches!(count, Ok(0)),
            "count against an unmigrated database returned Ok(0) — 'zero resources' \
             is a claim about data we never successfully queried"
        );
        assert!(
            matches!(count, Err(StorageError::Backend(_))),
            "expected a backend error from an unmigrated database, got {count:?}"
        );

        let create = backend
            .create(
                &tenant,
                "Patient",
                json!({ "resourceType": "Patient" }),
                FhirVersion::default(),
            )
            .await;
        assert!(
            matches!(create, Err(StorageError::Backend(_))),
            "expected a backend error from an unmigrated database, got {create:?}"
        );
    }
    /// Claims manifests in a loop until the lease for `target` comes back,
    /// holding any other lease it picks up along the way (so the loop cannot
    /// re-claim the same foreign manifest) and returning those to the queue
    /// once the target is held. Robust to concurrent tests sharing the
    /// testcontainers PostgreSQL instance — the submit-side twin of
    /// `claim_specific` for exports.
    async fn claim_specific_manifest(
        backend: &PostgresBackend,
        worker_id: &helios_persistence::core::WorkerId,
        submission_id: &helios_persistence::core::SubmissionId,
        target_manifest_id: &str,
        lease_duration: std::time::Duration,
    ) -> helios_persistence::core::ManifestLease {
        use helios_persistence::core::SubmitClaimStrategy;

        let mut held = Vec::new();
        let mut found = None;
        for _ in 0..100 {
            match backend
                .claim_next_manifest(worker_id, lease_duration)
                .await
                .unwrap()
            {
                Some(lease)
                    if lease.submission_id == *submission_id
                        && lease.manifest_id == target_manifest_id =>
                {
                    found = Some(lease);
                    break;
                }
                Some(other) => held.push(other),
                None => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            }
        }
        for other in held {
            let _ = SubmitClaimStrategy::release(backend, other).await;
        }
        found.expect("never claimed the expected manifest")
    }

    /// The `import` / `metadata` kickoff directives must survive the PostgreSQL
    /// round-trip and reach the worker, and `merge` must actually merge.
    ///
    /// This is the Postgres half of the coverage in
    /// `core::bulk_submit_worker::tests` (which runs on SQLite): the plumbing is
    /// shared but the SQL is not, so a typo in the new columns would otherwise
    /// only surface in production.
    #[tokio::test]
    async fn postgres_bulk_submit_import_directives_round_trip() {
        use helios_persistence::core::{
            BulkProcessingOptions, BulkSubmitProvider, IMPORT_MODE_PARAMETER_URL, ImportMode,
            ManifestFetchParams, NdjsonEntry, SubmissionId, SubmitWorkerStorage,
        };

        // The worker claim queue is intentionally cross-tenant, so keep this
        // claim isolated from the synchronous bulk-submit test below.
        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("bulk_submit_import");
        let sub_id = SubmissionId::generate("pg-import-test");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, Some("https://provider/m.json"), None)
            .await
            .unwrap();

        let directives = vec![(IMPORT_MODE_PARAMETER_URL.to_string(), "merge".to_string())];
        let metadata = vec![("https://ex/context".to_string(), "batch-7".to_string())];
        backend
            .set_manifest_fetch_params(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                ManifestFetchParams {
                    fhir_base_url: Some("https://provider/fhir"),
                    import_directives: &directives,
                    metadata: &metadata,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Claim *this* manifest the way the worker does, then read its view
        // back. The claim queue is cross-tenant and ordered by `added_at`, and
        // the test binary shares one container database, so a plain
        // `claim_next_manifest` can hand back another test's manifest — and
        // an unleased `processing` manifest (the synchronous `process_entries`
        // path leaves one behind) is reclaimable, so "move it out of pending"
        // elsewhere is no defence. Loop until ours comes back, returning
        // anything else to the queue.
        let lease = claim_specific_manifest(
            &backend,
            &helios_persistence::core::WorkerId::new(format!(
                "pg-import-worker-{}",
                uuid::Uuid::new_v4()
            )),
            &sub_id,
            &manifest.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        let view = backend.get_manifest_for_worker(&lease).await.unwrap();
        assert_eq!(view.import_directives, directives);
        assert_eq!(view.metadata, metadata);
        assert_eq!(
            ImportMode::from_directives(&view.import_directives),
            ImportMode::Merge
        );

        // And the resolved mode changes what ingestion writes.
        backend
            .create(
                &tenant,
                "Patient",
                json!({
                    "resourceType": "Patient",
                    "id": "pg-merge-1",
                    "gender": "female",
                    "name": [{"family": "Stale"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let entry = NdjsonEntry::new(
            1,
            "Patient",
            json!({"resourceType": "Patient", "id": "pg-merge-1", "name": [{"family": "New"}]}),
        );
        let results = backend
            .process_entries(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                vec![entry],
                &BulkProcessingOptions::new().with_import_mode(ImportMode::Merge),
            )
            .await
            .unwrap();
        assert!(results[0].is_success());

        let stored = backend
            .read(&tenant, "Patient", "pg-merge-1")
            .await
            .unwrap()
            .expect("patient still stored");
        assert_eq!(stored.content()["name"], json!([{"family": "New"}]));
        assert_eq!(
            stored.content()["gender"],
            json!("female"),
            "merge must retain elements the submission omitted"
        );
    }

    #[derive(Clone)]
    struct RecordedBulkEntry {
        line_number: u64,
        resource_type: String,
        resource_id: Option<String>,
        created: bool,
        outcome: String,
    }

    #[derive(Default)]
    struct RecordingBulkSubmitBatches(std::sync::Mutex<Vec<Vec<RecordedBulkEntry>>>);

    impl helios_persistence::core::BatchCommitObserver for RecordingBulkSubmitBatches {
        fn batch_committed(&self, batch: &helios_persistence::core::BatchCommitted<'_>) {
            self.0.lock().unwrap().push(
                batch
                    .results
                    .iter()
                    .map(|result| RecordedBulkEntry {
                        line_number: result.line_number,
                        resource_type: result.resource_type.clone(),
                        resource_id: result.resource_id.clone(),
                        created: result.created,
                        outcome: result.outcome.to_string(),
                    })
                    .collect(),
            );
        }
    }

    async fn new_bulk_submit_manifest(
        backend: &PostgresBackend,
        tenant: &TenantContext,
        label: &str,
    ) -> (
        helios_persistence::core::SubmissionId,
        helios_persistence::core::SubmissionManifest,
    ) {
        use helios_persistence::core::BulkSubmitProvider;

        let submission = helios_persistence::core::SubmissionId::generate(label);
        backend
            .create_submission(tenant, &submission, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(tenant, &submission, None, None)
            .await
            .unwrap();
        (submission, manifest)
    }

    fn grouped_create_options(
        observer: std::sync::Arc<RecordingBulkSubmitBatches>,
    ) -> helios_persistence::core::BulkProcessingOptions {
        helios_persistence::core::BulkProcessingOptions::new()
            .with_defer_indexing(true)
            .with_file_url("https://provider.example/fresh.ndjson")
            .with_batch_observer(observer)
    }

    async fn assert_bulk_submit_table_count(
        client: &tokio_postgres::Client,
        table: &str,
        tenant_id: &str,
        expected: i64,
    ) {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = $1");
        let actual: i64 = client.query_one(&sql, &[&tenant_id]).await.unwrap().get(0);
        assert_eq!(actual, expected, "unexpected {table} row count");
    }

    /// #1136: a clean eligible batch keeps every observable per-entry
    /// contract while PostgreSQL flushes all 100 creates at once.
    #[tokio::test]
    async fn postgres_bulk_submit_grouped_fresh_create_preserves_full_batch_contract() {
        use helios_persistence::core::{
            BulkEntryOutcome, BulkSubmitProvider, BulkSubmitRollbackProvider, NdjsonEntry,
        };

        let (backend, dbname) = isolated_reindex_backend().await;
        let tenant = create_tenant("bulk-submit-grouped-clean");
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let (submission, manifest) =
            new_bulk_submit_manifest(&backend, &tenant, "grouped-clean").await;
        let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
        let entries: Vec<_> = (1..=100)
            .map(|line| {
                NdjsonEntry::new(
                    line,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("fresh-{line:03}"),
                        "active": line % 2 == 0
                    }),
                )
            })
            .collect();

        let results = backend
            .process_entries(
                &tenant,
                &submission,
                &manifest.manifest_id,
                entries,
                &grouped_create_options(observer.clone()),
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 100);
        for (index, result) in results.iter().enumerate() {
            let line = (index + 1) as u64;
            let id = format!("fresh-{line:03}");
            assert_eq!(result.line_number, line);
            assert_eq!(result.resource_type, "Patient");
            assert_eq!(result.resource_id.as_deref(), Some(id.as_str()));
            assert!(result.created);
            assert_eq!(result.outcome, BulkEntryOutcome::Success);
        }

        let batches = observer.0.lock().unwrap().clone();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 100);
        for (index, observed) in batches[0].iter().enumerate() {
            let line = (index + 1) as u64;
            let id = format!("fresh-{line:03}");
            assert_eq!(observed.line_number, line);
            assert_eq!(observed.resource_type, "Patient");
            assert_eq!(observed.resource_id.as_deref(), Some(id.as_str()));
            assert!(observed.created);
            assert_eq!(observed.outcome, "success");
        }

        let stored = backend
            .read(&tenant, "Patient", "fresh-042")
            .await
            .unwrap()
            .expect("grouped resource");
        assert_eq!(stored.version_id(), "1");
        assert_eq!(stored.tenant_id(), tenant.tenant_id());
        assert_eq!(stored.fhir_version(), FhirVersion::R4);
        assert_eq!(stored.content()["resourceType"], "Patient");
        assert_eq!(stored.content()["id"], "fresh-042");
        assert_eq!(stored.content()["active"], true);

        let page = backend
            .get_entry_results_page(&tenant, &submission, &manifest.manifest_id, None, 101, None)
            .await
            .unwrap();
        assert_eq!(page.entries.len(), 100);
        for (index, entry) in page.entries.iter().enumerate() {
            let line = (index + 1) as u64;
            let id = format!("fresh-{line:03}");
            assert_eq!(entry.result.line_number, line);
            assert_eq!(entry.result.resource_id.as_deref(), Some(id.as_str()));
            assert!(entry.result.created);
            assert_eq!(entry.result.outcome, BulkEntryOutcome::Success);
        }

        let changes = backend
            .list_changes(&tenant, &submission, 101, 0)
            .await
            .unwrap();
        assert_eq!(changes.len(), 100);
        assert!(changes.iter().all(|change| {
            change.change_type == helios_persistence::core::ChangeType::Create
                && change.new_version == "1"
                && change.previous_version.is_none()
        }));
        let counts = backend
            .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!((counts.total, counts.success), (100, 100));
        assert_eq!(
            (
                counts.validation_error,
                counts.processing_error,
                counts.skipped
            ),
            (0, 0, 0)
        );

        let manifest_after = backend
            .get_manifest(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(manifest_after.total_entries, 100);
        assert_eq!(manifest_after.processed_entries, 100);
        assert_eq!(manifest_after.failed_entries, 0);

        let client = reindex_test_client_for(&dbname).await;
        assert_bulk_submit_table_count(&client, "resources", &tenant_id, 100).await;
        assert_bulk_submit_table_count(&client, "resource_history", &tenant_id, 100).await;
        assert_bulk_submit_table_count(&client, "bulk_entry_results", &tenant_id, 100).await;
        assert_bulk_submit_table_count(&client, "bulk_submission_changes", &tenant_id, 100).await;
        let bad_rows: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM resources
                 WHERE tenant_id = $1 AND
                       (version_id <> '1' OR is_deleted OR fhir_version <> '4.0' OR
                        data->>'resourceType' <> resource_type OR data->>'id' <> id)",
                &[&tenant_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(bad_rows, 0);
    }

    /// A permission error after a provisional create must discard the whole
    /// grouped attempt before the per-entry replay starts.
    #[tokio::test]
    async fn postgres_bulk_submit_grouped_permission_error_rolls_back_before_replay() {
        use helios_persistence::core::{BulkEntryOutcome, BulkSubmitProvider, NdjsonEntry};
        use helios_persistence::tenant::Operation;

        let (backend, dbname) = isolated_reindex_backend().await;
        let unrestricted = create_tenant("bulk-submit-grouped-permission");
        let tenant = TenantContext::new(
            unrestricted.tenant_id().clone(),
            TenantPermissions::builder()
                .allow_operations(vec![Operation::Create])
                .allow_resource_types(vec!["Patient"])
                .build(),
        );
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let (submission, manifest) =
            new_bulk_submit_manifest(&backend, &tenant, "grouped-permission").await;
        let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
        let entries = vec![
            NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType":"Patient","id":"allowed-a"}),
            ),
            NdjsonEntry::new(
                2,
                "Observation",
                json!({"resourceType":"Observation","id":"denied"}),
            ),
            NdjsonEntry::new(
                3,
                "Patient",
                json!({"resourceType":"Patient","id":"allowed-b"}),
            ),
        ];

        let results = backend
            .process_entries(
                &tenant,
                &submission,
                &manifest.manifest_id,
                entries,
                &grouped_create_options(observer.clone()),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].is_success() && results[0].created);
        assert_eq!(results[1].outcome, BulkEntryOutcome::ProcessingError);
        assert!(results[2].is_success() && results[2].created);
        assert_eq!(observer.0.lock().unwrap().len(), 1);

        let client = reindex_test_client_for(&dbname).await;
        assert_bulk_submit_table_count(&client, "resources", &tenant_id, 2).await;
        assert_bulk_submit_table_count(&client, "resource_history", &tenant_id, 2).await;
        assert_bulk_submit_table_count(&client, "bulk_entry_results", &tenant_id, 3).await;
        assert_bulk_submit_table_count(&client, "bulk_submission_changes", &tenant_id, 2).await;
        let attempts: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM resource_history WHERE tenant_id = $1 AND id = 'allowed-a'",
                &[&tenant_id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            attempts, 1,
            "the provisional Patient must not survive replay"
        );

        let counts = backend
            .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(
            (counts.total, counts.success, counts.processing_error),
            (3, 2, 1)
        );
    }

    /// A database error for one grouped row rolls back the grouped statement,
    /// releases the only pooled connection, and replays entries under their
    /// individual savepoints.
    #[tokio::test]
    async fn postgres_bulk_submit_grouped_row_failure_replays_with_pool_size_one() {
        use helios_persistence::core::{BulkEntryOutcome, BulkSubmitProvider, NdjsonEntry};

        let (backend, dbname) = isolated_reindex_backend_with_max_connections(1).await;
        let tenant = create_tenant("bulk-submit-grouped-row-error");
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let (submission, manifest) =
            new_bulk_submit_manifest(&backend, &tenant, "grouped-row-error").await;
        let client = reindex_test_client_for(&dbname).await;
        client
            .batch_execute(&format!(
                "CREATE FUNCTION reject_bulk_submit_row() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF NEW.tenant_id = '{tenant_id}' AND NEW.id = 'bad' THEN
                     RAISE EXCEPTION 'deliberate grouped row failure';
                   END IF;
                   RETURN NEW;
                 END $$;
                 CREATE TRIGGER reject_bulk_submit_row
                 BEFORE INSERT ON resources
                 FOR EACH ROW EXECUTE FUNCTION reject_bulk_submit_row();"
            ))
            .await
            .unwrap();
        let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
        let entries = ["good-a", "bad", "good-b"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| {
                NdjsonEntry::new(
                    (index + 1) as u64,
                    "Patient",
                    json!({"resourceType":"Patient","id":id}),
                )
            })
            .collect();

        let results = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            backend.process_entries(
                &tenant,
                &submission,
                &manifest.manifest_id,
                entries,
                &grouped_create_options(observer.clone()),
            ),
        )
        .await
        .expect("replay deadlocked while waiting for the only pool connection")
        .unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].is_success());
        assert_eq!(results[1].outcome, BulkEntryOutcome::ProcessingError);
        assert!(results[2].is_success());
        assert_eq!(observer.0.lock().unwrap().len(), 1);

        assert_bulk_submit_table_count(&client, "resources", &tenant_id, 2).await;
        assert_bulk_submit_table_count(&client, "resource_history", &tenant_id, 2).await;
        assert_bulk_submit_table_count(&client, "bulk_entry_results", &tenant_id, 3).await;
        assert_bulk_submit_table_count(&client, "bulk_submission_changes", &tenant_id, 2).await;
        for id in ["good-a", "good-b"] {
            let history: i64 = client
                .query_one(
                    "SELECT COUNT(*) FROM resource_history WHERE tenant_id = $1 AND id = $2",
                    &[&tenant_id, &id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(history, 1, "{id} must be created once after replay");
        }
        let counts = backend
            .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(
            (counts.total, counts.success, counts.processing_error),
            (3, 2, 1)
        );
    }

    async fn install_resource_insert_statement_counter(client: &tokio_postgres::Client) {
        client
            .batch_execute(
                "CREATE SEQUENCE grouped_resource_insert_statements MINVALUE 0 START 0;
                 CREATE FUNCTION count_grouped_resource_insert_statements()
                   RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   PERFORM nextval('grouped_resource_insert_statements');
                   RETURN NULL;
                 END $$;
                 CREATE TRIGGER count_grouped_resource_insert_statements
                 BEFORE INSERT ON resources
                 FOR EACH STATEMENT EXECUTE FUNCTION count_grouped_resource_insert_statements();",
            )
            .await
            .unwrap();
    }

    async fn resource_insert_statements(client: &tokio_postgres::Client) -> i64 {
        client
            .query_one(
                "SELECT CASE WHEN is_called THEN last_value + 1 ELSE 0 END
                 FROM grouped_resource_insert_statements",
                &[],
            )
            .await
            .unwrap()
            .get(0)
    }

    /// An active target-tenant key must select the individual path before any
    /// grouped insert is attempted. The fresh companion therefore accounts for
    /// the only resource INSERT statement.
    #[tokio::test]
    async fn postgres_bulk_submit_active_candidate_routes_before_grouped_insert() {
        use helios_persistence::core::{
            BulkEntryOutcome, BulkSubmitProvider, BulkSubmitRollbackProvider, NdjsonEntry,
        };

        for allow_updates in [false, true] {
            let (backend, dbname) = isolated_reindex_backend().await;
            let tenant = create_tenant(if allow_updates {
                "grouped-active-update"
            } else {
                "grouped-active-skip"
            });
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType":"Patient","id":"active","name":[{"family":"Stored"}]}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            let (submission, manifest) =
                new_bulk_submit_manifest(&backend, &tenant, "grouped-active").await;
            let client = reindex_test_client_for(&dbname).await;
            install_resource_insert_statement_counter(&client).await;
            let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
            let options =
                grouped_create_options(observer.clone()).with_allow_updates(allow_updates);

            let results = backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest.manifest_id,
                    vec![
                        NdjsonEntry::new(
                            1,
                            "Patient",
                            json!({"resourceType":"Patient","id":"active","name":[{"family":"Submitted"}]}),
                        ),
                        NdjsonEntry::new(
                            2,
                            "Patient",
                            json!({"resourceType":"Patient","id":"fresh"}),
                        ),
                    ],
                    &options,
                )
                .await
                .unwrap();

            assert_eq!(results.len(), 2);
            if allow_updates {
                assert!(results[0].is_success());
                assert!(!results[0].created);
            } else {
                assert_eq!(results[0].outcome, BulkEntryOutcome::Skipped);
            }
            assert!(results[1].is_success() && results[1].created);
            assert_eq!(resource_insert_statements(&client).await, 1);
            assert_eq!(observer.0.lock().unwrap().len(), 1);

            let active = backend
                .read(&tenant, "Patient", "active")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                active.content()["name"][0]["family"],
                if allow_updates { "Submitted" } else { "Stored" }
            );
            assert_eq!(active.version_id(), if allow_updates { "2" } else { "1" });
            let changes = backend
                .list_changes(&tenant, &submission, 10, 0)
                .await
                .unwrap();
            assert_eq!(changes.len(), if allow_updates { 2 } else { 1 });
            let counts = backend
                .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
                .await
                .unwrap();
            assert_eq!(counts.total, 2);
            assert_eq!(counts.success, if allow_updates { 2 } else { 1 });
            assert_eq!(counts.skipped, if allow_updates { 0 } else { 1 });
            assert_eq!(counts.processing_error, 0);
        }
    }

    /// Soft-deleted rows are candidates too. Finding the row before the grouped
    /// attempt leaves exactly one failed individual INSERT; filtering deleted
    /// rows from the candidate query would add an earlier grouped attempt.
    #[tokio::test]
    async fn postgres_bulk_submit_deleted_candidate_routes_before_grouped_insert() {
        use helios_persistence::core::{BulkEntryOutcome, BulkSubmitProvider, NdjsonEntry};

        for allow_updates in [false, true] {
            let (backend, dbname) = isolated_reindex_backend().await;
            let tenant = create_tenant(if allow_updates {
                "grouped-deleted-update"
            } else {
                "grouped-deleted-skip"
            });
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"resourceType":"Patient","id":"deleted"}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            backend.delete(&tenant, "Patient", "deleted").await.unwrap();
            let (submission, manifest) =
                new_bulk_submit_manifest(&backend, &tenant, "grouped-deleted").await;
            let client = reindex_test_client_for(&dbname).await;
            install_resource_insert_statement_counter(&client).await;
            let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
            let options =
                grouped_create_options(observer.clone()).with_allow_updates(allow_updates);

            let results = backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest.manifest_id,
                    vec![NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType":"Patient","id":"deleted"}),
                    )],
                    &options,
                )
                .await
                .unwrap();

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].outcome, BulkEntryOutcome::ProcessingError);
            assert_eq!(resource_insert_statements(&client).await, 1);
            assert_eq!(observer.0.lock().unwrap().len(), 1);
            assert!(matches!(
                backend.read(&tenant, "Patient", "deleted").await,
                Err(StorageError::Resource(ResourceError::Gone { .. }))
            ));
            let counts = backend
                .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
                .await
                .unwrap();
            assert_eq!(
                (counts.total, counts.success, counts.processing_error),
                (1, 0, 1)
            );
        }
    }

    /// Candidate matching is scoped by both tenant and resource type. Each
    /// boundary case remains eligible for one grouped resource INSERT statement.
    #[tokio::test]
    async fn postgres_bulk_submit_candidate_matching_uses_tenant_and_resource_type() {
        use helios_persistence::core::{BulkSubmitProvider, NdjsonEntry};

        for allow_updates in [false, true] {
            for same_tenant_different_type in [false, true] {
                let (backend, dbname) = isolated_reindex_backend().await;
                let tenant = create_tenant(if same_tenant_different_type {
                    "grouped-type-boundary"
                } else {
                    "grouped-tenant-boundary"
                });
                let other_tenant = create_tenant("grouped-other-tenant");
                let collision_id = if same_tenant_different_type {
                    "same-id-different-type"
                } else {
                    "same-id-other-tenant"
                };
                let fixture_tenant = if same_tenant_different_type {
                    &tenant
                } else {
                    &other_tenant
                };
                let fixture_type = if same_tenant_different_type {
                    "Observation"
                } else {
                    "Patient"
                };
                backend
                    .create(
                        fixture_tenant,
                        fixture_type,
                        json!({"resourceType":fixture_type,"id":collision_id}),
                        FhirVersion::default(),
                    )
                    .await
                    .unwrap();
                let (submission, manifest) =
                    new_bulk_submit_manifest(&backend, &tenant, "grouped-boundary").await;
                let client = reindex_test_client_for(&dbname).await;
                install_resource_insert_statement_counter(&client).await;
                let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
                let options =
                    grouped_create_options(observer.clone()).with_allow_updates(allow_updates);

                let results = backend
                    .process_entries(
                        &tenant,
                        &submission,
                        &manifest.manifest_id,
                        vec![
                            NdjsonEntry::new(
                                1,
                                "Patient",
                                json!({"resourceType":"Patient","id":collision_id}),
                            ),
                            NdjsonEntry::new(
                                2,
                                "Patient",
                                json!({"resourceType":"Patient","id":"fresh-companion"}),
                            ),
                        ],
                        &options,
                    )
                    .await
                    .unwrap();

                assert!(
                    results
                        .iter()
                        .all(|result| result.is_success() && result.created)
                );
                assert_eq!(resource_insert_statements(&client).await, 1);
                assert_eq!(observer.0.lock().unwrap().len(), 1);
                assert!(
                    backend
                        .read(&tenant, "Patient", collision_id)
                        .await
                        .unwrap()
                        .is_some()
                );
                assert!(
                    backend
                        .read(fixture_tenant, fixture_type, collision_id)
                        .await
                        .unwrap()
                        .is_some()
                );
                let counts = backend
                    .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
                    .await
                    .unwrap();
                assert_eq!(
                    (counts.total, counts.success, counts.processing_error),
                    (2, 2, 0)
                );
            }
        }
    }

    /// The candidate query can race a competing insert. A trigger blocks the
    /// grouped insert after that query has completed, while another transaction
    /// inserts and commits the conflicting key under the same advisory lock.
    #[tokio::test]
    async fn postgres_bulk_submit_grouped_concurrent_conflict_replays_for_both_update_modes() {
        use helios_persistence::core::{
            BulkEntryOutcome, BulkSubmitProvider, BulkSubmitRollbackProvider, NdjsonEntry,
        };
        use std::sync::Arc;

        for allow_updates in [false, true] {
            let (backend, dbname) = isolated_reindex_backend().await;
            let backend = Arc::new(backend);
            let tenant = create_tenant(if allow_updates {
                "grouped-race-update"
            } else {
                "grouped-race-skip"
            });
            let tenant_id = tenant.tenant_id().as_str().to_string();
            let (submission, manifest) =
                new_bulk_submit_manifest(&backend, &tenant, "grouped-race").await;
            let client = reindex_test_client_for(&dbname).await;
            client
                .batch_execute(&format!(
                    "CREATE FUNCTION block_grouped_race() RETURNS trigger LANGUAGE plpgsql AS $$
                     BEGIN
                       IF NEW.tenant_id = '{tenant_id}' AND NEW.id = 'race' THEN
                         PERFORM pg_advisory_xact_lock(hashtext(NEW.tenant_id), hashtext(NEW.id));
                       END IF;
                       RETURN NEW;
                     END $$;
                     CREATE TRIGGER block_grouped_race
                     BEFORE INSERT ON resources
                     FOR EACH ROW EXECUTE FUNCTION block_grouped_race();"
                ))
                .await
                .unwrap();
            client.batch_execute("BEGIN").await.unwrap();
            let owner_pid: i32 = client
                .query_one("SELECT pg_backend_pid()", &[])
                .await
                .unwrap()
                .get(0);
            client
                .query_one(
                    "SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))",
                    &[&tenant_id, &"race"],
                )
                .await
                .unwrap();

            let observer = Arc::new(RecordingBulkSubmitBatches::default());
            let options =
                grouped_create_options(observer.clone()).with_allow_updates(allow_updates);
            let task_backend = backend.clone();
            let task_tenant = tenant.clone();
            let task_submission = submission.clone();
            let task_manifest_id = manifest.manifest_id.clone();
            let ingest = tokio::spawn(async move {
                task_backend
                    .process_entries(
                        &task_tenant,
                        &task_submission,
                        &task_manifest_id,
                        vec![
                            NdjsonEntry::new(
                                1,
                                "Patient",
                                json!({"resourceType":"Patient","id":"before-race"}),
                            ),
                            NdjsonEntry::new(
                                2,
                                "Patient",
                                json!({"resourceType":"Patient","id":"race","name":[{"family":"Submitted"}]}),
                            ),
                        ],
                        &options,
                    )
                    .await
            });

            let mut waiter_seen = false;
            for _ in 0..200 {
                waiter_seen = client
                    .query_one(
                        "SELECT EXISTS (
                           SELECT 1 FROM pg_stat_activity AS activity
                           WHERE $1 = ANY(pg_blocking_pids(activity.pid))
                             AND activity.datname = current_database()
                             AND activity.pid <> $1
                         )",
                        &[&owner_pid],
                    )
                    .await
                    .unwrap()
                    .get(0);
                if waiter_seen {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            assert!(
                waiter_seen,
                "grouped insert never waited on the advisory lock"
            );

            let competitor = json!({
                "resourceType": "Patient",
                "id": "race",
                "name": [{"family": "Competitor"}]
            });
            client
                .execute(
                    "WITH ins AS (
                       INSERT INTO resources
                         (tenant_id, resource_type, id, version_id, data, last_updated,
                          is_deleted, fhir_version)
                       VALUES ($1, 'Patient', 'race', '1', $2, NOW(), FALSE, '4.0')
                       RETURNING tenant_id, resource_type, id, version_id, data, last_updated,
                                 is_deleted, fhir_version
                     )
                     INSERT INTO resource_history
                       (tenant_id, resource_type, id, version_id, data, last_updated,
                        is_deleted, fhir_version)
                     SELECT tenant_id, resource_type, id, version_id, data, last_updated,
                            is_deleted, fhir_version FROM ins",
                    &[&tenant_id, &competitor],
                )
                .await
                .unwrap();
            client.batch_execute("COMMIT").await.unwrap();

            let results = tokio::time::timeout(std::time::Duration::from_secs(10), ingest)
                .await
                .expect("ingest remained blocked after the competitor committed")
                .unwrap()
                .unwrap();
            assert_eq!(results.len(), 2);
            assert!(results[0].is_success() && results[0].created);
            if allow_updates {
                assert!(results[1].is_success());
                assert!(!results[1].created);
                assert_eq!(results[1].resource_id.as_deref(), Some("race"));
            } else {
                assert_eq!(results[1].outcome, BulkEntryOutcome::Skipped);
            }
            let observed = observer.0.lock().unwrap().clone();
            assert_eq!(observed.len(), 1);
            assert_eq!(observed[0].len(), 2);
            assert_eq!(observed[0][0].resource_id.as_deref(), Some("before-race"));

            let before_history: i64 = client
                .query_one(
                    "SELECT COUNT(*) FROM resource_history
                     WHERE tenant_id = $1 AND id = 'before-race'",
                    &[&tenant_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(before_history, 1, "the provisional create survived replay");
            let race_history: i64 = client
                .query_one(
                    "SELECT COUNT(*) FROM resource_history
                     WHERE tenant_id = $1 AND id = 'race'",
                    &[&tenant_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(race_history, if allow_updates { 2 } else { 1 });
            let race = backend
                .read(&tenant, "Patient", "race")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                race.content()["name"][0]["family"],
                if allow_updates {
                    "Submitted"
                } else {
                    "Competitor"
                }
            );
            assert_eq!(race.version_id(), if allow_updates { "2" } else { "1" });

            let changes = backend
                .list_changes(&tenant, &submission, 10, 0)
                .await
                .unwrap();
            assert_eq!(changes.len(), if allow_updates { 2 } else { 1 });
            let counts = backend
                .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
                .await
                .unwrap();
            assert_eq!(counts.total, 2);
            assert_eq!(counts.success, if allow_updates { 2 } else { 1 });
            assert_eq!(counts.skipped, if allow_updates { 0 } else { 1 });
            assert_eq!(counts.processing_error, 0);
        }
    }

    async fn install_resource_attempt_counter(client: &tokio_postgres::Client, tenant_id: &str) {
        client
            .batch_execute(&format!(
                "CREATE SEQUENCE grouped_resource_attempts;
                 CREATE FUNCTION count_grouped_resource_attempts() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF NEW.tenant_id = '{tenant_id}' THEN
                     PERFORM nextval('grouped_resource_attempts');
                   END IF;
                   RETURN NEW;
                 END $$;
                 CREATE TRIGGER count_grouped_resource_attempts
                 BEFORE INSERT ON resources
                 FOR EACH ROW EXECUTE FUNCTION count_grouped_resource_attempts();"
            ))
            .await
            .unwrap();
    }

    async fn grouped_resource_attempts(client: &tokio_postgres::Client) -> i64 {
        client
            .query_one("SELECT last_value FROM grouped_resource_attempts", &[])
            .await
            .unwrap()
            .get(0)
    }

    /// Receipt and change failures happen after the grouped resource flush.
    /// The caller must roll back the transaction without replaying resources.
    #[tokio::test]
    async fn postgres_bulk_submit_post_flush_bookkeeping_failure_does_not_replay() {
        use helios_persistence::core::{BulkSubmitProvider, NdjsonEntry};

        for (fault_table, label) in [
            ("bulk_entry_results", "receipt"),
            ("bulk_submission_changes", "change"),
        ] {
            let (backend, dbname) = isolated_reindex_backend().await;
            let tenant = create_tenant(&format!("grouped-{label}-failure"));
            let tenant_id = tenant.tenant_id().as_str().to_string();
            let (submission, manifest) =
                new_bulk_submit_manifest(&backend, &tenant, &format!("grouped-{label}-failure"))
                    .await;
            let client = reindex_test_client_for(&dbname).await;
            install_resource_attempt_counter(&client, &tenant_id).await;
            client
                .batch_execute(&format!(
                    "CREATE FUNCTION reject_grouped_bookkeeping() RETURNS trigger LANGUAGE plpgsql AS $$
                     BEGIN
                       IF NEW.tenant_id = '{tenant_id}' THEN
                         RAISE EXCEPTION 'deliberate {label} failure';
                       END IF;
                       RETURN NEW;
                     END $$;
                     CREATE TRIGGER reject_grouped_bookkeeping
                     BEFORE INSERT ON {fault_table}
                     FOR EACH ROW EXECUTE FUNCTION reject_grouped_bookkeeping();"
                ))
                .await
                .unwrap();
            let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
            let entries = [format!("{label}-a"), format!("{label}-b")]
                .into_iter()
                .enumerate()
                .map(|(index, id)| {
                    NdjsonEntry::new(
                        (index + 1) as u64,
                        "Patient",
                        json!({"resourceType":"Patient","id":id}),
                    )
                })
                .collect();

            assert!(
                backend
                    .process_entries(
                        &tenant,
                        &submission,
                        &manifest.manifest_id,
                        entries,
                        &grouped_create_options(observer.clone()),
                    )
                    .await
                    .is_err()
            );
            assert!(observer.0.lock().unwrap().is_empty());
            assert_eq!(grouped_resource_attempts(&client).await, 2);
            for table in [
                "resources",
                "resource_history",
                "bulk_entry_results",
                "bulk_submission_changes",
            ] {
                assert_bulk_submit_table_count(&client, table, &tenant_id, 0).await;
            }
            let current = backend
                .get_manifest(&tenant, &submission, &manifest.manifest_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (
                    current.total_entries,
                    current.processed_entries,
                    current.failed_entries
                ),
                (0, 0, 0)
            );
        }
    }

    /// A deferred constraint failure is raised by COMMIT after resources and
    /// bookkeeping were staged. No observer or tail counters may claim that
    /// the transaction committed.
    #[tokio::test]
    async fn postgres_bulk_submit_deferred_commit_failure_does_not_replay() {
        use helios_persistence::core::{BulkSubmitProvider, NdjsonEntry};

        let (backend, dbname) = isolated_reindex_backend().await;
        let tenant = create_tenant("grouped-commit-failure");
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let (submission, manifest) =
            new_bulk_submit_manifest(&backend, &tenant, "grouped-commit-failure").await;
        let client = reindex_test_client_for(&dbname).await;
        install_resource_attempt_counter(&client, &tenant_id).await;
        client
            .batch_execute(&format!(
                "CREATE FUNCTION reject_grouped_commit() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF NEW.tenant_id = '{tenant_id}' THEN
                     RAISE EXCEPTION 'deliberate deferred commit failure';
                   END IF;
                   RETURN NULL;
                 END $$;
                 CREATE CONSTRAINT TRIGGER reject_grouped_commit
                 AFTER INSERT ON bulk_entry_results
                 DEFERRABLE INITIALLY DEFERRED
                 FOR EACH ROW EXECUTE FUNCTION reject_grouped_commit();"
            ))
            .await
            .unwrap();
        let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
        let entries = ["commit-a", "commit-b"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| {
                NdjsonEntry::new(
                    (index + 1) as u64,
                    "Patient",
                    json!({"resourceType":"Patient","id":id}),
                )
            })
            .collect();

        assert!(
            backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest.manifest_id,
                    entries,
                    &grouped_create_options(observer.clone()),
                )
                .await
                .is_err()
        );
        assert!(observer.0.lock().unwrap().is_empty());
        assert_eq!(grouped_resource_attempts(&client).await, 2);
        for table in [
            "resources",
            "resource_history",
            "bulk_entry_results",
            "bulk_submission_changes",
        ] {
            assert_bulk_submit_table_count(&client, table, &tenant_id, 0).await;
        }
        let current = backend
            .get_manifest(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                current.total_entries,
                current.processed_entries,
                current.failed_entries
            ),
            (0, 0, 0)
        );
    }

    /// The counter update runs after the resource transaction commits. Its
    /// failure must preserve the durable batch and its single observer event.
    #[tokio::test]
    async fn postgres_bulk_submit_tail_counter_failure_preserves_committed_batch() {
        use helios_persistence::core::{BulkSubmitProvider, NdjsonEntry};

        let (backend, dbname) = isolated_reindex_backend().await;
        let tenant = create_tenant("grouped-tail-failure");
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let (submission, manifest) =
            new_bulk_submit_manifest(&backend, &tenant, "grouped-tail-failure").await;
        let client = reindex_test_client_for(&dbname).await;
        install_resource_attempt_counter(&client, &tenant_id).await;
        client
            .batch_execute(&format!(
                "CREATE FUNCTION reject_grouped_tail() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF NEW.tenant_id = '{tenant_id}' THEN
                     RAISE EXCEPTION 'deliberate manifest counter failure';
                   END IF;
                   RETURN NEW;
                 END $$;
                 CREATE TRIGGER reject_grouped_tail
                 BEFORE UPDATE OF total_entries ON bulk_manifests
                 FOR EACH ROW EXECUTE FUNCTION reject_grouped_tail();"
            ))
            .await
            .unwrap();
        let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
        let entries = ["tail-a", "tail-b"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| {
                NdjsonEntry::new(
                    (index + 1) as u64,
                    "Patient",
                    json!({"resourceType":"Patient","id":id}),
                )
            })
            .collect();

        assert!(
            backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest.manifest_id,
                    entries,
                    &grouped_create_options(observer.clone()),
                )
                .await
                .is_err()
        );
        assert_eq!(observer.0.lock().unwrap().len(), 1);
        assert_eq!(grouped_resource_attempts(&client).await, 2);
        for table in [
            "resources",
            "resource_history",
            "bulk_entry_results",
            "bulk_submission_changes",
        ] {
            assert_bulk_submit_table_count(&client, table, &tenant_id, 2).await;
        }
        let current = backend
            .get_manifest(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                current.total_entries,
                current.processed_entries,
                current.failed_entries
            ),
            (0, 0, 0)
        );
    }

    /// Terminating the grouped transaction's connection makes its explicit
    /// rollback fail. The caller must return that rollback error and must not
    /// open a replay transaction.
    #[tokio::test]
    async fn postgres_bulk_submit_grouped_rollback_failure_stops_before_replay() {
        use helios_persistence::core::{BulkSubmitProvider, NdjsonEntry};

        let (backend, dbname) = isolated_reindex_backend().await;
        let tenant = create_tenant("grouped-rollback-failure");
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let (submission, manifest) =
            new_bulk_submit_manifest(&backend, &tenant, "grouped-rollback-failure").await;
        let client = reindex_test_client_for(&dbname).await;
        client
            .batch_execute(&format!(
                "CREATE SEQUENCE grouped_termination_attempts;
                 CREATE FUNCTION terminate_grouped_connection() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   IF NEW.tenant_id = '{tenant_id}'
                      AND nextval('grouped_termination_attempts') = 1 THEN
                     PERFORM pg_terminate_backend(pg_backend_pid());
                   END IF;
                   RETURN NEW;
                 END $$;
                 CREATE TRIGGER terminate_grouped_connection
                 BEFORE INSERT ON resources
                 FOR EACH ROW EXECUTE FUNCTION terminate_grouped_connection();"
            ))
            .await
            .unwrap();
        let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
        let entries = ["terminate-a", "terminate-b"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| {
                NdjsonEntry::new(
                    (index + 1) as u64,
                    "Patient",
                    json!({"resourceType":"Patient","id":id}),
                )
            })
            .collect();

        let error = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            backend.process_entries(
                &tenant,
                &submission,
                &manifest.manifest_id,
                entries,
                &grouped_create_options(observer.clone()),
            ),
        )
        .await
        .expect("connection termination left process_entries blocked")
        .expect_err("a failed rollback must surface");
        assert!(
            error.to_string().contains("roll back grouped fresh-create"),
            "unexpected error: {error}"
        );
        assert!(observer.0.lock().unwrap().is_empty());
        let attempts: i64 = client
            .query_one("SELECT last_value FROM grouped_termination_attempts", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(attempts, 1, "a second connection replayed the batch");
        for table in [
            "resources",
            "resource_history",
            "bulk_entry_results",
            "bulk_submission_changes",
        ] {
            assert_bulk_submit_table_count(&client, table, &tenant_id, 0).await;
        }
        let current = backend
            .get_manifest(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                current.total_entries,
                current.processed_entries,
                current.failed_entries
            ),
            (0, 0, 0)
        );
    }

    /// A statement timeout on the candidate query follows the same one-replay
    /// branch as grouped create and flush errors. The lock observer sees that
    /// candidate query first and an ordinary point read second, proving the
    /// fallback transaction executed before its error receipts committed.
    #[tokio::test]
    async fn postgres_bulk_submit_candidate_query_failure_replays_once() {
        use helios_persistence::core::{BulkEntryOutcome, BulkSubmitProvider, NdjsonEntry};
        use std::sync::Arc;

        let (backend, dbname) = isolated_reindex_backend_with_max_connections(1).await;
        let backend = Arc::new(backend);
        let tenant = create_tenant("grouped-candidate-query-failure");
        let tenant_id = tenant.tenant_id().as_str().to_string();
        let (submission, manifest) =
            new_bulk_submit_manifest(&backend, &tenant, "candidate-query-failure").await;
        let pooled = backend.get_client().await.unwrap();
        pooled
            .batch_execute("SET statement_timeout = '500ms'")
            .await
            .unwrap();
        drop(pooled);

        let locker = reindex_test_client_for(&dbname).await;
        locker
            .batch_execute("BEGIN; LOCK TABLE resources IN ACCESS EXCLUSIVE MODE")
            .await
            .unwrap();
        let locker_pid: i32 = locker
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        let observer = std::sync::Arc::new(RecordingBulkSubmitBatches::default());
        let entries = ["query-a", "query-b"]
            .into_iter()
            .enumerate()
            .map(|(index, id)| {
                NdjsonEntry::new(
                    (index + 1) as u64,
                    "Patient",
                    json!({"resourceType":"Patient","id":id}),
                )
            })
            .collect();
        let task_backend = backend.clone();
        let task_tenant = tenant.clone();
        let task_submission = submission.clone();
        let task_manifest_id = manifest.manifest_id.clone();
        let task_observer = observer.clone();
        let ingest = tokio::spawn(async move {
            task_backend
                .process_entries(
                    &task_tenant,
                    &task_submission,
                    &task_manifest_id,
                    entries,
                    &grouped_create_options(task_observer),
                )
                .await
        });

        let blocked_query = |row: &tokio_postgres::Row| {
            let query: String = row.get("query");
            let state: String = row.get("state");
            let wait_event_type: Option<String> = row.get("wait_event_type");
            (query, state, wait_event_type)
        };
        let mut candidate_observation = None;
        let mut last_blocked_queries = Vec::new();
        for _ in 0..400 {
            locker
                .batch_execute("SELECT pg_stat_clear_snapshot()")
                .await
                .unwrap();
            let rows = locker
                .query(
                    "SELECT query, state, wait_event_type
                     FROM pg_stat_activity
                     WHERE datname = current_database()
                       AND pid <> $1
                       AND state = 'active'
                       AND wait_event_type = 'Lock'",
                    &[&locker_pid],
                )
                .await
                .unwrap();
            last_blocked_queries = rows
                .iter()
                .map(|row| row.get::<_, String>("query"))
                .collect();
            candidate_observation = rows.into_iter().find_map(|row| {
                let observation = blocked_query(&row);
                let query = observation.0.to_ascii_lowercase();
                (query.contains("from resources as resource")
                    && query.contains("join unnest")
                    && query.contains("candidate(resource_type, id)"))
                .then_some(observation)
            });
            if candidate_observation.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let candidate_observation = candidate_observation.unwrap_or_else(|| {
            panic!(
                "the candidate query was not observed waiting on the resources table lock; \
                 last blocked queries: {last_blocked_queries:?}"
            )
        });
        assert_eq!(candidate_observation.1, "active");
        assert_eq!(candidate_observation.2.as_deref(), Some("Lock"));

        let mut replay_observation = None;
        for _ in 0..400 {
            locker
                .batch_execute("SELECT pg_stat_clear_snapshot()")
                .await
                .unwrap();
            let rows = locker
                .query(
                    "SELECT query, state, wait_event_type
                     FROM pg_stat_activity
                     WHERE datname = current_database()
                       AND pid <> $1
                       AND state = 'active'
                       AND wait_event_type = 'Lock'",
                    &[&locker_pid],
                )
                .await
                .unwrap();
            replay_observation = rows.into_iter().find_map(|row| {
                let observation = blocked_query(&row);
                (observation
                    .0
                    .contains("SELECT version_id, data, last_updated, is_deleted, fhir_version")
                    && observation.0.contains("resource_type = $2 AND id = $3"))
                .then_some(observation)
            });
            if replay_observation.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let replay_observation = replay_observation
            .expect("the replay point read was not observed after the candidate query timed out");
        assert_eq!(replay_observation.1, "active");
        assert_eq!(replay_observation.2.as_deref(), Some("Lock"));

        let result = tokio::time::timeout(std::time::Duration::from_secs(5), ingest)
            .await
            .expect("candidate-query replay exceeded its statement timeouts")
            .unwrap();
        locker.batch_execute("ROLLBACK").await.unwrap();

        let results = result.unwrap();
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|result| result.outcome == BulkEntryOutcome::ProcessingError)
        );
        assert_eq!(observer.0.lock().unwrap().len(), 1);
        let client = reindex_test_client_for(&dbname).await;
        assert_bulk_submit_table_count(&client, "resources", &tenant_id, 0).await;
        assert_bulk_submit_table_count(&client, "resource_history", &tenant_id, 0).await;
        assert_bulk_submit_table_count(&client, "bulk_entry_results", &tenant_id, 2).await;
        assert_bulk_submit_table_count(&client, "bulk_submission_changes", &tenant_id, 0).await;
        let counts = backend
            .get_entry_counts(&tenant, &submission, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(
            (counts.total, counts.success, counts.processing_error),
            (2, 0, 2)
        );
    }

    /// #872: the batched ingest commits entry writes, rollback records, and
    /// per-line receipts together, and a failing entry is contained to its
    /// savepoint — on Postgres a failed statement aborts the transaction, so
    /// without the savepoint one bad entry would poison the whole batch.
    #[tokio::test]
    async fn postgres_bulk_submit_batch_commits_bookkeeping_and_contains_errors() {
        use helios_persistence::core::{
            BulkEntryOutcome, BulkProcessingOptions, BulkSubmitProvider,
            BulkSubmitRollbackProvider, ChangeType, NdjsonEntry, SubmissionId,
        };

        // A synchronous manifest has no worker lease. Serialize it with the
        // worker claim test above so the cross-tenant queue cannot hand it to
        // that test while this one is processing the batch.
        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend_with_max_connections(1).await;
        let tenant = create_tenant("bulk_submit_batch");
        let sub_id = SubmissionId::generate("pg-batch-test");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, Some("https://provider/b.json"), None)
            .await
            .unwrap();
        // Mark the manifest `processing` right away. Note this is not a
        // defence against other tests' `claim_next_manifest` calls — an
        // unleased `processing` manifest is reclaimable, so a claimant may
        // still pick this one up transiently — which is why claiming tests use
        // `claim_specific_manifest` and hand foreign manifests back.
        backend
            .process_entries(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                Vec::new(),
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();

        // An existing resource for the update path, and a soft-deleted id
        // whose create fails with AlreadyExists inside the batch.
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":"pg-batch-upd","name":[{"family":"Old"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":"pg-batch-gone"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .delete(&tenant, "Patient", "pg-batch-gone")
            .await
            .unwrap();

        let entries = vec![
            NdjsonEntry::new(
                1,
                "Patient",
                json!({"resourceType":"Patient","id":"pg-batch-new","name":[{"family":"BatchNew"}]}),
            ),
            NdjsonEntry::new(
                2,
                "Patient",
                json!({"resourceType":"Patient","id":"pg-batch-gone"}),
            ),
            NdjsonEntry::new(
                3,
                "Patient",
                json!({"resourceType":"Patient","id":"pg-batch-upd","name":[{"family":"BatchUpd"}]}),
            ),
        ];
        let results = backend
            .process_entries(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                entries,
                &BulkProcessingOptions::new().with_file_url("https://provider/p.ndjson"),
            )
            .await
            .unwrap();

        let client = backend.get_client().await.unwrap();
        let prepared_bookkeeping: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM pg_prepared_statements
                 WHERE statement LIKE 'INSERT INTO bulk_submission_changes%'
                    OR statement LIKE 'WITH receipt_rows AS%'",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            prepared_bookkeeping, 2,
            "three receipts and two rollback changes should retain two prepared statements"
        );
        drop(client);

        // `process_entries` leaves synchronous manifests in `processing`
        // without a worker lease. Such rows remain eligible for the worker
        // claim queue, so terminate this test submission before releasing the
        // lock shared with the claim round-trip test.
        assert_eq!(
            backend
                .abort_submission(&tenant, &sub_id, "test cleanup")
                .await
                .unwrap(),
            1
        );

        assert!(results[0].is_success() && results[0].created);
        assert!(
            results[1].is_error(),
            "the soft-deleted id must fail its entry, got {:?}",
            results[1]
        );
        assert!(results[2].is_success() && !results[2].created);

        let receipts = backend
            .get_entry_results_page(&tenant, &sub_id, &manifest.manifest_id, None, 10, None)
            .await
            .unwrap();
        assert!(receipts.next.is_none());
        for paged in &receipts.entries {
            let identity = paged.stored_identity.as_ref().unwrap();
            assert_eq!(identity.file_url, "https://provider/p.ndjson");
            assert_eq!(identity.line_number, paged.result.line_number);
            assert_eq!(paged.result.resource_type, "Patient");
        }
        let receipt_facts: Vec<_> = receipts
            .entries
            .iter()
            .map(|paged| {
                let result = &paged.result;
                (
                    result.line_number,
                    result.resource_id.as_deref(),
                    result.created,
                    result.outcome,
                    result.operation_outcome.is_some(),
                )
            })
            .collect();
        assert_eq!(
            receipt_facts,
            vec![
                (
                    1,
                    Some("pg-batch-new"),
                    true,
                    BulkEntryOutcome::Success,
                    false
                ),
                (2, None, false, BulkEntryOutcome::ProcessingError, true),
                (
                    3,
                    Some("pg-batch-upd"),
                    false,
                    BulkEntryOutcome::Success,
                    false
                ),
            ]
        );

        // The failed entry did not poison the batch: both writes committed,
        // together with their receipts and rollback records.
        let created = backend
            .read(&tenant, "Patient", "pg-batch-new")
            .await
            .unwrap()
            .expect("created patient");
        assert_eq!(created.version_id(), "1");
        assert_eq!(created.content()["name"], json!([{"family":"BatchNew"}]));
        let updated = backend
            .read(&tenant, "Patient", "pg-batch-upd")
            .await
            .unwrap()
            .expect("updated patient");
        assert_eq!(updated.version_id(), "2");
        assert_eq!(updated.content()["name"], json!([{"family":"BatchUpd"}]));

        let created_history = backend
            .history_instance(
                &tenant,
                "Patient",
                "pg-batch-new",
                &HistoryParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(created_history.items.len(), 1);
        assert_eq!(created_history.items[0].resource.version_id(), "1");
        let updated_history = backend
            .history_instance(
                &tenant,
                "Patient",
                "pg-batch-upd",
                &HistoryParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(updated_history.items.len(), 2);
        assert_eq!(updated_history.items[0].resource.version_id(), "2");
        assert_eq!(updated_history.items[1].resource.version_id(), "1");

        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.total, 3);
        assert_eq!(counts.success, 2);
        assert_eq!(counts.processing_error, 1);

        let changes = backend.list_changes(&tenant, &sub_id, 10, 0).await.unwrap();
        assert_eq!(
            changes.len(),
            2,
            "one rollback record per successful write, none for the failed entry"
        );
        let create = changes
            .iter()
            .find(|change| change.resource_id == "pg-batch-new")
            .expect("create change");
        assert_eq!(create.manifest_id, manifest.manifest_id);
        assert_eq!(create.change_type, ChangeType::Create);
        assert_eq!(create.resource_type, "Patient");
        assert_eq!(create.new_version, "1");
        assert!(create.previous_version.is_none());
        assert!(create.previous_content.is_none());
        let update = changes
            .iter()
            .find(|change| change.resource_id == "pg-batch-upd")
            .expect("update change");
        assert_eq!(update.manifest_id, manifest.manifest_id);
        assert_eq!(update.change_type, ChangeType::Update);
        assert_eq!(update.resource_type, "Patient");
        assert_eq!(update.previous_version.as_deref(), Some("1"));
        assert_eq!(update.new_version, "2");
        assert_eq!(
            update
                .previous_content
                .as_ref()
                .expect("update previous content")["name"],
            json!([{"family":"Old"}])
        );

        let page = backend
            .get_entry_results_page(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                Some(BulkEntryOutcome::ProcessingError),
                1,
                None,
            )
            .await
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert!(page.entries[0].result.resource_id.is_none());
        assert!(page.entries[0].result.operation_outcome.is_some());
    }

    async fn seed_isolated_bulk_submit(
        backend: &PostgresBackend,
        label: &str,
    ) -> (
        TenantContext,
        helios_persistence::core::SubmissionId,
        String,
    ) {
        use helios_persistence::core::{BulkSubmitProvider, SubmissionId};

        let tenant = create_tenant(label);
        let submission = SubmissionId::generate(label);
        backend
            .create_submission(&tenant, &submission, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(
                &tenant,
                &submission,
                Some(&format!("https://provider/{label}.json")),
                None,
            )
            .await
            .unwrap();
        (tenant, submission, manifest.manifest_id)
    }

    async fn install_bookkeeping_statement_probe(dbname: &str) -> (tokio_postgres::Client, String) {
        let client = reindex_test_client_for(dbname).await;
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let probe = format!("bulk_bookkeeping_probe_{suffix}");
        let function = format!("record_bulk_bookkeeping_{suffix}");
        let receipt_trigger = format!("record_bulk_receipt_{suffix}");
        let change_trigger = format!("record_bulk_change_{suffix}");
        client
            .batch_execute(&format!(
                "CREATE TABLE {probe} (target text PRIMARY KEY, statements bigint NOT NULL);
                 INSERT INTO {probe} VALUES ('bulk_entry_results', 0), ('bulk_submission_changes', 0);
                 CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                   UPDATE {probe} SET statements = statements + 1 WHERE target = TG_TABLE_NAME;
                   RETURN NULL;
                 END $$;
                 CREATE TRIGGER {receipt_trigger} BEFORE INSERT ON bulk_entry_results
                   FOR EACH STATEMENT EXECUTE FUNCTION {function}();
                 CREATE TRIGGER {change_trigger} BEFORE INSERT ON bulk_submission_changes
                   FOR EACH STATEMENT EXECUTE FUNCTION {function}();"
            ))
            .await
            .expect("install bookkeeping statement probe");
        (client, probe)
    }

    async fn bookkeeping_statement_counts(
        client: &tokio_postgres::Client,
        probe: &str,
    ) -> (i64, i64) {
        let receipt = client
            .query_one(
                &format!("SELECT statements FROM {probe} WHERE target = 'bulk_entry_results'"),
                &[],
            )
            .await
            .unwrap()
            .get(0);
        let change = client
            .query_one(
                &format!("SELECT statements FROM {probe} WHERE target = 'bulk_submission_changes'"),
                &[],
            )
            .await
            .unwrap()
            .get(0);
        (receipt, change)
    }

    async fn reset_bookkeeping_statement_counts(client: &tokio_postgres::Client, probe: &str) {
        client
            .execute(&format!("UPDATE {probe} SET statements = 0"), &[])
            .await
            .unwrap();
    }

    fn mutation_entries(prefix: &str, count: usize) -> Vec<helios_persistence::core::NdjsonEntry> {
        use helios_persistence::core::NdjsonEntry;

        (0..count)
            .map(|index| {
                NdjsonEntry::new(
                    index as u64 + 1,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("{prefix}-{index}")
                    }),
                )
            })
            .collect()
    }

    /// #1137: bookkeeping statement count is bounded by processed entries,
    /// including batches where every entry is skipped and creates no rollback row.
    #[tokio::test]
    async fn postgres_bulk_submit_bookkeeping_flushes_in_processed_entry_batches() {
        use helios_persistence::core::{
            BulkProcessingOptions, BulkSubmitProvider, BulkSubmitRollbackProvider, NdjsonEntry,
            ResourceStorage,
        };

        let (backend, dbname) = isolated_reindex_backend().await;
        let (client, probe) = install_bookkeeping_statement_probe(&dbname).await;

        let (tenant, submission, manifest) =
            seed_isolated_bulk_submit(&backend, "bulk-bookkeeping-100").await;
        let results = backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                mutation_entries("bookkeeping-100", 100),
                &BulkProcessingOptions::new().with_defer_indexing(true),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 100);
        assert_eq!(bookkeeping_statement_counts(&client, &probe).await, (1, 1));

        reset_bookkeeping_statement_counts(&client, &probe).await;
        let (tenant, submission, manifest) =
            seed_isolated_bulk_submit(&backend, "bulk-bookkeeping-1002").await;
        let results = backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                mutation_entries("bookkeeping-1002", 1002),
                &BulkProcessingOptions::new().with_defer_indexing(true),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1002);
        assert_eq!(bookkeeping_statement_counts(&client, &probe).await, (2, 2));
        assert_eq!(
            backend
                .get_entry_counts(&tenant, &submission, &manifest)
                .await
                .unwrap()
                .total,
            1002
        );
        assert_eq!(
            backend
                .list_changes(&tenant, &submission, 2000, 0)
                .await
                .unwrap()
                .len(),
            1002
        );

        reset_bookkeeping_statement_counts(&client, &probe).await;
        let (tenant, submission, manifest) =
            seed_isolated_bulk_submit(&backend, "bulk-bookkeeping-skips").await;
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":"bookkeeping-existing"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let skips: Vec<_> = (0..1002)
            .map(|index| {
                NdjsonEntry::new(
                    index + 1,
                    "Patient",
                    json!({"resourceType":"Patient","id":"bookkeeping-existing"}),
                )
            })
            .collect();
        let results = backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                skips,
                &BulkProcessingOptions::create_only().with_defer_indexing(true),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1002);
        assert!(results.iter().all(|result| !result.is_success()));
        assert_eq!(bookkeeping_statement_counts(&client, &probe).await, (2, 0));
        assert_eq!(
            backend
                .get_entry_counts(&tenant, &submission, &manifest)
                .await
                .unwrap()
                .skipped,
            1002
        );
    }

    /// #1137: a receipt upsert remains last-write-wins within one array batch,
    /// across array batches, and on replay. File URL remains part of its identity.
    #[tokio::test]
    async fn postgres_bulk_submit_batched_receipts_keep_identity_and_change_fidelity() {
        use helios_persistence::core::{
            BulkEntryOutcome, BulkProcessingOptions, BulkSubmitProvider,
            BulkSubmitRollbackProvider, ChangeType, NdjsonEntry, ResourceStorage,
        };
        use std::collections::HashSet;

        let (backend, _dbname) = isolated_reindex_backend().await;
        let (tenant, submission, manifest) =
            seed_isolated_bulk_submit(&backend, "bulk-bookkeeping-identity").await;
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":"identity-skip"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let mut across = vec![NdjsonEntry::new(
            77,
            "Patient",
            json!({"resourceType":"Patient","id":"identity-across-first"}),
        )];
        across.extend((0..999).map(|index| {
            NdjsonEntry::new(
                index + 1000,
                "Patient",
                json!({"resourceType":"Patient","id":"identity-skip"}),
            )
        }));
        across.push(NdjsonEntry::new(
            77,
            "Patient",
            json!({"resourceType":"Patient","id":"identity-across-last"}),
        ));
        backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                across,
                &BulkProcessingOptions::create_only()
                    .with_file_url("across.ndjson")
                    .with_defer_indexing(true),
            )
            .await
            .unwrap();
        let cross_flush_receipt = backend
            .get_entry_results_page(
                &tenant,
                &submission,
                &manifest,
                Some(BulkEntryOutcome::Success),
                1,
                None,
            )
            .await
            .unwrap();
        assert_eq!(cross_flush_receipt.entries.len(), 1);
        assert_eq!(
            cross_flush_receipt.entries[0].result.resource_id.as_deref(),
            Some("identity-across-last")
        );

        backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                vec![
                    NdjsonEntry::new(
                        88,
                        "Patient",
                        json!({"resourceType":"Patient","id":"identity-within-first"}),
                    ),
                    NdjsonEntry::new(
                        88,
                        "Patient",
                        json!({"resourceType":"Patient","id":"identity-within-last"}),
                    ),
                ],
                &BulkProcessingOptions::new()
                    .with_file_url("within.ndjson")
                    .with_defer_indexing(true),
            )
            .await
            .unwrap();

        backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                vec![NdjsonEntry::new(
                    77,
                    "Patient",
                    json!({"resourceType":"Patient","id":"identity-replay"}),
                )],
                &BulkProcessingOptions::new()
                    .with_file_url("across.ndjson")
                    .with_defer_indexing(true),
            )
            .await
            .unwrap();
        let replay_receipt = backend
            .get_entry_results_page(
                &tenant,
                &submission,
                &manifest,
                Some(BulkEntryOutcome::Success),
                1,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            replay_receipt.entries[0].result.resource_id.as_deref(),
            Some("identity-replay")
        );
        backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                vec![NdjsonEntry::new(
                    77,
                    "Patient",
                    json!({"resourceType":"Patient","id":"identity-other-file"}),
                )],
                &BulkProcessingOptions::new()
                    .with_file_url("other.ndjson")
                    .with_defer_indexing(true),
            )
            .await
            .unwrap();
        backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                vec![NdjsonEntry::new(
                    79,
                    "Patient",
                    json!({
                        "resourceType":"Patient",
                        "id":"identity-replay",
                        "active":true
                    }),
                )],
                &BulkProcessingOptions::new()
                    .with_file_url("across.ndjson")
                    .with_defer_indexing(true),
            )
            .await
            .unwrap();

        let mut cursor = None;
        let mut successes = Vec::new();
        loop {
            let page = backend
                .get_entry_results_page(
                    &tenant,
                    &submission,
                    &manifest,
                    Some(BulkEntryOutcome::Success),
                    1,
                    cursor.as_ref(),
                )
                .await
                .unwrap();
            successes.extend(page.entries);
            let Some(next) = page.next else {
                break;
            };
            cursor = Some(next);
        }
        let success_identities: Vec<_> = successes
            .iter()
            .map(|entry| {
                (
                    entry.stored_identity.as_ref().unwrap().file_url.as_str(),
                    entry.result.line_number,
                    entry.result.resource_id.as_deref().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            success_identities,
            vec![
                ("across.ndjson", 77, "identity-replay"),
                ("across.ndjson", 79, "identity-replay"),
                ("other.ndjson", 77, "identity-other-file"),
                ("within.ndjson", 88, "identity-within-last"),
            ]
        );
        let skipped = backend
            .get_entry_results_page(
                &tenant,
                &submission,
                &manifest,
                Some(BulkEntryOutcome::Skipped),
                1,
                None,
            )
            .await
            .unwrap();
        assert_eq!(skipped.entries.len(), 1);
        assert!(skipped.entries[0].result.resource_id.is_none());
        assert!(skipped.entries[0].result.operation_outcome.is_some());

        let changes = backend
            .list_changes(&tenant, &submission, 20, 0)
            .await
            .unwrap();
        assert_eq!(
            changes.len(),
            7,
            "every successful mutation keeps its change"
        );
        assert_eq!(
            changes
                .iter()
                .map(|change| change.change_id.as_str())
                .collect::<HashSet<_>>()
                .len(),
            changes.len()
        );
        assert!(changes.iter().all(|change| {
            change.manifest_id == manifest
                && change.resource_type == "Patient"
                && !change.change_id.is_empty()
                && change.changed_at <= chrono::Utc::now()
        }));
        let update = changes
            .iter()
            .find(|change| change.change_type == ChangeType::Update)
            .expect("replay update change");
        assert_eq!(update.resource_id, "identity-replay");
        assert_eq!(update.previous_version.as_deref(), Some("1"));
        assert_eq!(update.new_version, "2");
        assert_eq!(
            update.previous_content.as_ref().unwrap()["id"],
            json!("identity-replay")
        );
        assert!(
            changes
                .iter()
                .filter(|change| change.change_type == ChangeType::Create)
                .all(|change| change.previous_version.is_none()
                    && change.previous_content.is_none()
                    && change.new_version == "1")
        );
    }

    #[derive(Default)]
    struct RecordingBatchCommitObserver {
        batches: std::sync::Mutex<Vec<Vec<helios_persistence::core::BulkEntryResult>>>,
    }

    impl helios_persistence::core::BatchCommitObserver for RecordingBatchCommitObserver {
        fn batch_committed(&self, batch: &helios_persistence::core::BatchCommitted<'_>) {
            self.batches.lock().unwrap().push(batch.results.to_vec());
        }
    }

    async fn install_second_bookkeeping_statement_failure(dbname: &str, target: &str) {
        assert!(matches!(
            target,
            "bulk_entry_results" | "bulk_submission_changes"
        ));
        let client = reindex_test_client_for(dbname).await;
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let probe = format!("bulk_failure_probe_{suffix}");
        let function = format!("fail_second_bulk_statement_{suffix}");
        let trigger = format!("fail_second_bulk_statement_trigger_{suffix}");
        client
            .batch_execute(&format!(
                "CREATE TABLE {probe} (calls bigint NOT NULL);
                 INSERT INTO {probe} VALUES (0);
                 CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$
                 DECLARE call_number bigint;
                 BEGIN
                   UPDATE {probe} SET calls = calls + 1 RETURNING calls INTO call_number;
                   IF call_number = 2 THEN
                     RAISE EXCEPTION 'forced second bookkeeping statement failure';
                   END IF;
                   RETURN NULL;
                 END $$;
                 CREATE TRIGGER {trigger} BEFORE INSERT ON {target}
                   FOR EACH STATEMENT EXECUTE FUNCTION {function}();"
            ))
            .await
            .expect("install second-statement failure trigger");
    }

    /// #1137: every flush remains inside the resource transaction. Failure of
    /// either bookkeeping table on flush two rolls flush one and all resources back.
    #[tokio::test]
    async fn postgres_bulk_submit_second_bookkeeping_flush_failure_rolls_back_everything() {
        use helios_persistence::core::{BulkProcessingOptions, BulkSubmitProvider};
        use std::sync::Arc;

        for target in ["bulk_entry_results", "bulk_submission_changes"] {
            let (backend, dbname) = isolated_reindex_backend().await;
            let (tenant, submission, manifest) =
                seed_isolated_bulk_submit(&backend, &format!("bulk-failure-{target}")).await;
            install_second_bookkeeping_statement_failure(&dbname, target).await;
            let observer = Arc::new(RecordingBatchCommitObserver::default());
            let outcome = backend
                .process_entries(
                    &tenant,
                    &submission,
                    &manifest,
                    mutation_entries(&format!("failure-{target}"), 1001),
                    &BulkProcessingOptions::new()
                        .with_defer_indexing(true)
                        .with_batch_observer(observer.clone()),
                )
                .await;
            assert!(outcome.is_err(), "{target} failure must abort the batch");
            assert!(
                observer.batches.lock().unwrap().is_empty(),
                "an uncommitted batch must not notify the observer"
            );

            let client = reindex_test_client_for(&dbname).await;
            let tenant_id = tenant.tenant_id().as_str();
            for table in [
                "resources",
                "resource_history",
                "bulk_entry_results",
                "bulk_submission_changes",
            ] {
                let count: i64 = client
                    .query_one(
                        &format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = $1"),
                        &[&tenant_id],
                    )
                    .await
                    .unwrap()
                    .get(0);
                assert_eq!(count, 0, "{target} failure left rows in {table}");
            }
            let stored_manifest = backend
                .get_manifest(&tenant, &submission, &manifest)
                .await
                .unwrap()
                .expect("manifest remains after ingest rollback");
            assert_eq!(stored_manifest.total_entries, 0);
            assert_eq!(stored_manifest.processed_entries, 0);
            assert_eq!(stored_manifest.failed_entries, 0);
        }
    }

    /// #1137: the max-error paths flush and commit exactly the processed prefix,
    /// notify only after commit, and retain the existing manifest-counter behavior.
    #[tokio::test]
    async fn postgres_bulk_submit_batched_bookkeeping_preserves_max_error_semantics() {
        use helios_persistence::core::{
            BulkEntryOutcome, BulkProcessingOptions, BulkSubmitProvider, NdjsonEntry,
            ResourceStorage,
        };
        use helios_persistence::error::{BulkSubmitError, StorageError};
        use std::sync::Arc;

        let (backend, _dbname) = isolated_reindex_backend().await;
        let (tenant, submission, manifest) =
            seed_isolated_bulk_submit(&backend, "bulk-max-errors-stop").await;
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":"max-stop-tombstone"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .delete(&tenant, "Patient", "max-stop-tombstone")
            .await
            .unwrap();
        let stop_observer = Arc::new(RecordingBatchCommitObserver::default());
        let outcome = backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                vec![
                    NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType":"Patient","id":"max-stop-tombstone"}),
                    ),
                    NdjsonEntry::new(
                        2,
                        "Patient",
                        json!({"resourceType":"Patient","id":"max-stop-unreached"}),
                    ),
                ],
                &BulkProcessingOptions::strict()
                    .with_defer_indexing(true)
                    .with_batch_observer(stop_observer.clone()),
            )
            .await;
        assert!(
            matches!(
                &outcome,
                Err(StorageError::BulkSubmit(
                    BulkSubmitError::MaxErrorsExceeded { .. }
                ))
            ),
            "expected MaxErrorsExceeded, got {outcome:?}"
        );
        {
            let stopped_batches = stop_observer.batches.lock().unwrap();
            assert_eq!(stopped_batches.len(), 1);
            assert_eq!(stopped_batches[0].len(), 1);
            assert!(stopped_batches[0][0].is_error());
        }
        let receipt_counts = backend
            .get_entry_counts(&tenant, &submission, &manifest)
            .await
            .unwrap();
        assert_eq!(receipt_counts.total, 1);
        assert_eq!(receipt_counts.processing_error, 1);
        assert!(
            backend
                .read(&tenant, "Patient", "max-stop-unreached")
                .await
                .unwrap()
                .is_none()
        );
        let stopped_manifest = backend
            .get_manifest(&tenant, &submission, &manifest)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stopped_manifest.total_entries, 0);
        assert_eq!(stopped_manifest.processed_entries, 0);
        assert_eq!(stopped_manifest.failed_entries, 0);

        let (tenant, submission, manifest) =
            seed_isolated_bulk_submit(&backend, "bulk-max-errors-continue").await;
        backend
            .create(
                &tenant,
                "Patient",
                json!({"resourceType":"Patient","id":"max-continue-tombstone"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .delete(&tenant, "Patient", "max-continue-tombstone")
            .await
            .unwrap();
        let continue_observer = Arc::new(RecordingBatchCommitObserver::default());
        let results = backend
            .process_entries(
                &tenant,
                &submission,
                &manifest,
                vec![
                    NdjsonEntry::new(
                        1,
                        "Patient",
                        json!({"resourceType":"Patient","id":"max-continue-tombstone"}),
                    ),
                    NdjsonEntry::new(
                        2,
                        "Patient",
                        json!({"resourceType":"Patient","id":"max-continue-a"}),
                    ),
                    NdjsonEntry::new(
                        3,
                        "Patient",
                        json!({"resourceType":"Patient","id":"max-continue-b"}),
                    ),
                ],
                &BulkProcessingOptions::new()
                    .with_max_errors(1)
                    .with_continue_on_error(true)
                    .with_defer_indexing(true)
                    .with_batch_observer(continue_observer.clone()),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].is_error());
        assert!(
            results[1..]
                .iter()
                .all(|result| result.outcome == BulkEntryOutcome::Skipped)
        );
        {
            let continued_batches = continue_observer.batches.lock().unwrap();
            assert_eq!(continued_batches.len(), 1);
            assert_eq!(continued_batches[0].len(), 3);
        }
        let receipt_counts = backend
            .get_entry_counts(&tenant, &submission, &manifest)
            .await
            .unwrap();
        assert_eq!(receipt_counts.total, 3);
        assert_eq!(receipt_counts.processing_error, 1);
        assert_eq!(receipt_counts.skipped, 2);
        let continued_manifest = backend
            .get_manifest(&tenant, &submission, &manifest)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(continued_manifest.total_entries, 3);
        assert_eq!(continued_manifest.processed_entries, 0);
        assert_eq!(continued_manifest.failed_entries, 1);
    }

    /// #1007: `mark_entries_unindexed` flips only the named `(type, id)`
    /// entry results to `processing-error`, leaving the rest untouched, and
    /// is a no-op on an empty entry list.
    #[tokio::test]
    async fn mark_entries_unindexed_flips_only_the_named_resources() {
        use helios_persistence::core::{
            BulkEntryOutcome, BulkProcessingOptions, BulkSubmitProvider, NdjsonEntry, SubmissionId,
            UnindexedEntry,
        };

        let backend = create_backend().await;
        let tenant = create_tenant("bulk_submit_unindexed");
        let sub_id = SubmissionId::generate("pg-unindexed-test");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, None, None)
            .await
            .unwrap();

        let entries: Vec<NdjsonEntry> = ["pg-unidx-1", "pg-unidx-2", "pg-unidx-3"]
            .iter()
            .enumerate()
            .map(|(i, id)| {
                NdjsonEntry::new(
                    (i + 1) as u64,
                    "Patient",
                    json!({"resourceType": "Patient", "id": id}),
                )
            })
            .collect();
        let results = backend
            .process_entries(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                entries,
                &BulkProcessingOptions::new(),
            )
            .await
            .unwrap();
        assert!(results.iter().all(|r| r.is_success()));

        // Empty list is a no-op.
        assert_eq!(
            backend
                .mark_entries_unindexed(&tenant, &sub_id, &manifest.manifest_id, &[])
                .await
                .unwrap(),
            0
        );

        let oo = json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "incomplete",
                "diagnostics": "Patient/pg-unidx-2 was stored but could not be indexed for \
                                search on es: timeout. Run POST /Patient/$reindex to repair."
            }]
        });
        let changed = backend
            .mark_entries_unindexed(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                &[UnindexedEntry {
                    resource_type: "Patient".to_string(),
                    resource_id: "pg-unidx-2".to_string(),
                    operation_outcome: oo.clone(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(changed, 1);

        let page = backend
            .get_entry_results_page(&tenant, &sub_id, &manifest.manifest_id, None, 10, None)
            .await
            .unwrap();
        for paged in &page.entries {
            let result = &paged.result;
            if result.resource_id.as_deref() == Some("pg-unidx-2") {
                assert_eq!(result.outcome, BulkEntryOutcome::ProcessingError);
                assert_eq!(result.operation_outcome.as_ref(), Some(&oo));
            } else {
                assert_eq!(result.outcome, BulkEntryOutcome::Success);
            }
        }
    }

    /// Wraps a reader so that `token` is tripped the first time the ingest
    /// actually reads from the stream.
    struct CancelOnFirstRead<R> {
        inner: R,
        token: helios_persistence::core::bulk_submit::CancelToken,
        tripped: bool,
    }

    impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for CancelOnFirstRead<R> {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let this = self.get_mut();
            if !this.tripped {
                this.tripped = true;
                this.token.cancel();
            }
            std::pin::Pin::new(&mut this.inner).poll_read(cx, buf)
        }
    }

    /// #968, PostgreSQL: cancelling mid-manifest stops at the next batch
    /// boundary and keeps what was already committed. Each batch is its own
    /// Postgres transaction, so this pins that stopping does not roll the
    /// committed ones back — the opposite failure to #942's poisoned batch.
    #[tokio::test]
    async fn postgres_bulk_submit_cancel_mid_stream_keeps_committed_batches() {
        use helios_persistence::core::bulk_submit::{CANCELLED_ABORT_REASON, CancelToken};
        use helios_persistence::core::{
            BulkProcessingOptions, BulkSubmitProvider, StreamingBulkSubmitProvider, SubmissionId,
        };

        // Like the batch test: a synchronous manifest has no worker lease, so
        // serialize against the claim queue.
        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("bulk_submit_cancel");
        let sub_id = SubmissionId::generate("pg-cancel-test");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, Some("https://provider/c.json"), None)
            .await
            .unwrap();

        let cancel = CancelToken::new();
        let options = BulkProcessingOptions::new()
            .with_batch_size(2)
            .with_cancel(cancel.clone());

        // Six Patients, enough for three batches of two.
        let lines = (1..=6)
            .map(|i| format!("{{\"resourceType\":\"Patient\",\"id\":\"pg-cancel-{i}\"}}\n"))
            .collect::<String>()
            .into_bytes();
        let reader = Box::new(tokio::io::BufReader::new(CancelOnFirstRead {
            inner: std::io::Cursor::new(lines),
            token: cancel,
            tripped: false,
        }));
        let result = backend
            .process_ndjson_stream(
                &tenant,
                &sub_id,
                &manifest.manifest_id,
                "Patient",
                reader,
                &options,
            )
            .await
            .unwrap();

        // Terminate the submission before releasing the shared lock, for the
        // same reason as the batch test above.
        backend
            .abort_submission(&tenant, &sub_id, "test cleanup")
            .await
            .unwrap();

        assert!(result.aborted);
        assert_eq!(result.abort_reason.as_deref(), Some(CANCELLED_ABORT_REASON));
        assert_eq!(
            result.counts.success, 2,
            "the batch already committed when the token tripped is kept"
        );
        assert_eq!(
            result.lines_processed, 2,
            "the remaining four lines were never read"
        );

        let counts = backend
            .get_entry_counts(&tenant, &sub_id, &manifest.manifest_id)
            .await
            .unwrap();
        assert_eq!(counts.total, 2, "the partial counts are durable");
        assert!(
            backend
                .read(&tenant, "Patient", "pg-cancel-2")
                .await
                .unwrap()
                .is_some(),
            "the first batch really landed"
        );
        assert!(
            backend
                .read(&tenant, "Patient", "pg-cancel-3")
                .await
                .unwrap()
                .is_none(),
            "nothing after the cancellation point was ingested"
        );
    }

    /// #968, PostgreSQL: `abort_submission` fails in-flight manifests without
    /// clearing the lease, so the worker's late verdict must lose rather than
    /// resurrect the manifest as `completed`. Postgres enforces this with an
    /// `AND status = 'processing'` clause on the fenced update, which is its
    /// own SQL and so needs its own test.
    #[tokio::test]
    async fn postgres_bulk_submit_abort_beats_a_late_finish_manifest() {
        use helios_persistence::core::{
            BulkSubmitProvider, LeaseError, ManifestStatus, SubmissionId, SubmitWorkerStorage,
            WorkerId,
        };

        let _guard = BULK_SUBMIT_TEST_LOCK.lock().await;
        let backend = create_backend().await;
        let tenant = create_tenant("bulk_submit_abort_race");
        let sub_id = SubmissionId::generate("pg-abort-race-test");
        backend
            .create_submission(&tenant, &sub_id, None)
            .await
            .unwrap();
        let manifest = backend
            .add_manifest(&tenant, &sub_id, Some("https://provider/a.json"), None)
            .await
            .unwrap();

        let lease = claim_specific_manifest(
            &backend,
            &WorkerId::new(format!("pg-abort-worker-{}", uuid::Uuid::new_v4())),
            &sub_id,
            &manifest.manifest_id,
            std::time::Duration::from_secs(60),
        )
        .await;
        backend.mark_manifest_processing(&lease).await.unwrap();

        // The submitter aborts while the worker still holds a valid lease.
        backend
            .abort_submission(&tenant, &sub_id, "user cancelled")
            .await
            .unwrap();

        // The worker's verdicts arrive too late and change nothing.
        assert!(
            matches!(
                backend.finish_manifest(&lease).await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "a finish after an abort must not win"
        );
        let stored = backend
            .get_manifest(&tenant, &sub_id, &lease.manifest_id)
            .await
            .unwrap()
            .expect("manifest");
        assert_eq!(
            stored.status,
            ManifestStatus::Failed,
            "the abort's verdict stands"
        );

        assert!(
            matches!(
                backend.fail_manifest(&lease, "worker gave up").await,
                Err(LeaseError::LeaseLost { .. })
            ),
            "a late failure verdict is equally a no-op"
        );
        let stored = backend
            .get_manifest(&tenant, &sub_id, &lease.manifest_id)
            .await
            .unwrap()
            .expect("manifest");
        assert_eq!(stored.status, ManifestStatus::Failed);
    }

    // ========================================================================
    // Issue #311 — `ifMatch` on bundle entries, on a real PostgreSQL instance
    //
    // PostgreSQL carried both #311 defects (batch arm ignored `ifMatch`; the
    // field was compared as one opaque string), and its batch and transaction
    // arms are separate code paths from SQLite's. Running the *shared* suite
    // here is what makes "the backends agree" a checked claim rather than an
    // assumption — the assertions are literally the same function bodies.
    //
    // Each test gets its own tenant, since the whole binary shares one
    // container database and the scenarios reuse fixed resource ids.
    // ========================================================================

    /// Expands to a `#[tokio::test]` running one shared scenario against the
    /// shared PostgreSQL container under an isolated tenant.
    macro_rules! pg_if_match_test {
        ($test_name:ident, $scenario:ident) => {
            #[tokio::test]
            async fn $test_name() {
                let backend = create_backend().await;
                let tenant = create_tenant(concat!("if_match_", stringify!($scenario)));
                super::if_match_suite::$scenario(&backend, &tenant).await;
            }
        };
    }

    pg_if_match_test!(
        postgres_integration_multi_valued_if_match_matches_any_member,
        multi_valued_if_match_matches_any_member
    );
    pg_if_match_test!(
        postgres_integration_multi_valued_if_match_fails_when_no_member_matches,
        multi_valued_if_match_fails_when_no_member_matches
    );
    pg_if_match_test!(
        postgres_integration_strong_form_if_match_matches_weak_etag,
        strong_form_if_match_matches_weak_etag
    );
    pg_if_match_test!(
        postgres_integration_transaction_delete_honors_stale_if_match,
        transaction_delete_honors_stale_if_match
    );
    pg_if_match_test!(
        postgres_integration_transaction_delete_accepts_matching_if_match,
        transaction_delete_accepts_matching_if_match
    );

    // ========================================================================
    // Full-text purge completeness (issue #386)
    // ========================================================================

    use crate::fts_purge_suite::{self as fts_suite, FtsProbe};

    /// Reads `resource_fts` directly over the backend's own pool.
    ///
    /// `PostgresBackend::get_client` is `#[doc(hidden)] pub` precisely so
    /// out-of-crate tests can run raw SQL; other tests in this module already
    /// do the same.
    struct PgFtsProbe(PostgresBackend);

    #[async_trait::async_trait]
    impl FtsProbe for PgFtsProbe {
        async fn fts_row_count(&self, tenant_id: &str) -> u64 {
            let client = self.0.get_client().await.expect("get_client");
            let row = client
                .query_one(
                    "SELECT COUNT(*)::bigint FROM resource_fts WHERE tenant_id = $1",
                    &[&tenant_id],
                )
                .await
                .expect("count resource_fts");
            row.get::<_, i64>(0) as u64
        }

        async fn fts_rows_containing(&self, needle: &str) -> u64 {
            let client = self.0.get_client().await.expect("get_client");
            let pattern = format!("%{needle}%");
            let row = client
                .query_one(
                    "SELECT COUNT(*)::bigint FROM resource_fts \
                     WHERE full_content LIKE $1 OR narrative_text LIKE $1",
                    &[&pattern],
                )
                .await
                .expect("count resource_fts by content");
            row.get::<_, i64>(0) as u64
        }
    }

    /// One `#[tokio::test]` per shared scenario, each on its own UUID-suffixed
    /// tenant so they cannot collide on the shared container.
    macro_rules! pg_fts_test {
        ($name:ident, $scenario:ident) => {
            #[tokio::test]
            async fn $name() {
                let backend = create_backend().await;
                let probe = PgFtsProbe(create_backend().await);
                let tenant = create_tenant(stringify!($scenario));
                fts_suite::$scenario(&backend, &probe, &tenant).await;
            }
        };
    }

    pg_fts_test!(
        postgres_integration_purge_removes_fts_rows,
        purge_removes_fts_rows
    );
    pg_fts_test!(
        postgres_integration_purge_all_removes_fts_rows,
        purge_all_removes_fts_rows
    );
    pg_fts_test!(
        postgres_integration_purge_tenant_data_removes_fts_rows,
        purge_tenant_data_removes_fts_rows
    );
    pg_fts_test!(
        postgres_integration_reuse_after_purge_does_not_resurrect_narrative,
        reuse_after_purge_does_not_resurrect_narrative
    );
    pg_fts_test!(
        postgres_integration_tenant_reuse_does_not_resurrect_narrative,
        tenant_reuse_does_not_resurrect_narrative
    );
    pg_fts_test!(
        postgres_integration_repeated_purge_and_recreate_does_not_grow_fts,
        repeated_purge_and_recreate_does_not_grow_fts
    );

    #[tokio::test]
    async fn postgres_integration_purge_tenant_data_leaves_other_tenants_intact() {
        let backend = create_backend().await;
        let probe = PgFtsProbe(create_backend().await);
        fts_suite::purge_tenant_data_leaves_other_tenants_intact(
            &backend,
            &probe,
            &create_tenant("fts_victim"),
            &create_tenant("fts_bystander"),
        )
        .await;
    }

    /// `$reindex` must rebuild full-text search, not destroy it.
    ///
    /// This failed on PostgreSQL before the fix, in both modes: `run_reindex`
    /// drops each resource's `resource_fts` row via `delete_search_entries`, and
    /// `write_search_entries` never put it back.
    #[tokio::test]
    async fn postgres_integration_reindex_preserves_full_text_search_without_clear() {
        pg_reindex_case(false, "fts_reindex_noclear").await;
    }

    #[tokio::test]
    async fn postgres_integration_reindex_preserves_full_text_search_with_clear() {
        pg_reindex_case(true, "fts_reindex_clear").await;
    }

    async fn pg_reindex_case(clear_existing: bool, tenant_label: &str) {
        use helios_persistence::search::ReindexOperation;
        use std::sync::Arc;

        let backend = Arc::new(create_backend().await);
        let probe = PgFtsProbe(create_backend().await);
        let tenant = create_tenant(tenant_label);
        let reindex = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
        fts_suite::reindex_preserves_full_text_search(
            backend.as_ref(),
            &probe,
            &tenant,
            &reindex,
            clear_existing,
        )
        .await;
    }

    #[tokio::test]
    async fn postgres_integration_repeated_reindex_does_not_duplicate_fts_rows() {
        use helios_persistence::search::ReindexOperation;
        use std::sync::Arc;

        let backend = Arc::new(create_backend().await);
        let probe = PgFtsProbe(create_backend().await);
        let tenant = create_tenant("fts_reindex_repeat");
        let reindex = ReindexOperation::new(backend.clone(), backend.tenant_registries().clone());
        fts_suite::repeated_reindex_does_not_duplicate_fts_rows(
            backend.as_ref(),
            &probe,
            &tenant,
            &reindex,
        )
        .await;
    }

    // ========================================================================
    // Issue #447 — tenant-id fidelity, on a real PostgreSQL instance
    //
    // The #447 defect is S3's: it *derives* a key prefix from the tenant id and
    // the derivation was many-to-one. PostgreSQL derives nothing — `tenant_id`
    // is a `TEXT` column in each composite primary key, bound and compared with
    // `=` — so the scoping is the identity mapping and the defect cannot occur
    // here.
    //
    // That is a code reading, and a code reading is precisely what let the same
    // defect class sit undiscovered in the two backends that *do* derive (#384
    // on Elasticsearch, #447 on S3). So it is checked rather than asserted in
    // prose, against a real server: bound parameters, collation, and index
    // behaviour are properties of the engine, not of the Rust.
    //
    // Each test takes a unique base id — the whole binary shares one container
    // database, and the scenarios derive fixed resource ids from it.
    // ========================================================================

    fn unique_base(label: &str) -> String {
        format!("{}_{}", label, uuid::Uuid::new_v4().simple())
    }

    #[tokio::test]
    async fn postgres_integration_day_precision_date_boundaries() {
        let backend = create_backend().await;
        super::date_boundary_suite::day_precision_boundaries(
            &backend,
            &unique_base("date_boundary"),
        )
        .await;
    }

    #[tokio::test]
    async fn postgres_integration_distinct_tenant_ids_never_share_data() {
        let backend = create_backend().await;
        super::tenant_id_fidelity_suite::distinct_tenant_ids_never_share_data(
            &backend,
            &unique_base("fidelity"),
        )
        .await;
    }

    #[tokio::test]
    async fn postgres_integration_purging_one_tenant_leaves_the_look_alikes_intact() {
        let backend = create_backend().await;
        super::tenant_id_fidelity_suite::purging_one_tenant_leaves_the_look_alikes_intact(
            &backend,
            &unique_base("fidelity_purge"),
        )
        .await;
    }

    /// The `organization` search parameter maps to `managingOrganization`, not
    /// a field literally named `organization` — only the registry-driven
    /// resolver (FHIRPath expression, not a literal JSON field lookup) can
    /// follow it. Same fixture and assertions as SQLite's
    /// `test_resolve_includes_renamed_param_uses_registry`.
    #[tokio::test]
    async fn postgres_include_renamed_param_resolves_via_registry() {
        use helios_persistence::core::IncludeProvider;
        use helios_persistence::types::{IncludeDirective, IncludeType};

        let backend = create_backend().await;
        let tenant = create_tenant("include_renamed_param");

        backend
            .create_or_update(
                &tenant,
                "Organization",
                "org-1",
                json!({"id": "org-1", "name": "Acme Clinic"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let (patient, _) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "p1",
                json!({
                    "id": "p1",
                    "managingOrganization": {"reference": "Organization/org-1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let include = IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: None,
            iterate: false,
        };

        let included = backend
            .resolve_includes(&tenant, std::slice::from_ref(&patient), &[include])
            .await
            .unwrap();

        assert_eq!(included.len(), 1);
        assert_eq!(included[0].resource_type(), "Organization");
        assert_eq!(included[0].id(), "org-1");

        let include_wrong_target = IncludeDirective {
            include_type: IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: Some("Practitioner".to_string()),
            iterate: false,
        };

        let included = backend
            .resolve_includes(&tenant, &[patient], &[include_wrong_target])
            .await
            .unwrap();

        assert!(included.is_empty());
    }
}
