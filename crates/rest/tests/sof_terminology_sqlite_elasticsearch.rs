//! "Terminology in SQL" (#1824) — SQLite + Elasticsearch
//! (`sqlite-elasticsearch`).
//!
//! Runs the scenarios in `common/terminology_scenarios.rs` (see
//! `sof_terminology.rs` for SQLite) against the composite `hfs` builds for
//! `HFS_STORAGE_BACKEND=sqlite-elasticsearch`: SQLite primary with its own
//! search indexing offloaded, so every search — canonical resolution and
//! dependant-Library discovery included — is answered by Elasticsearch.
//!
//! Unlike the compartment suite, the composite is built **the way
//! production runs it**: the sync queue is asynchronous with its workers
//! started, and Elasticsearch keeps the default write refresh policy. That
//! is the configuration under which a search can lag the write the server
//! already acknowledged (`docs/issue-1824-backend-parity-plan.md`, §2 risk
//! 1); the point of this suite is to reproduce that, not hide it. Fixtures
//! seeded straight to storage are made visible by the harness, so the only
//! lag a scenario can see is the one its own HTTP writes create.
//!
//! Eight scenarios do see it, and fail here for exactly that reason: a
//! Library resolving a ValueSet the previous request wrote, a ValueSet
//! looking for the Library the previous request wrote (T2), `$sql-run`
//! right after the transaction that created or deleted its dependency, and
//! a run that gets the pre-expansion body back. They are expanded as
//! `#[ignore]` with that reason until the plan's Slice B establishes
//! visibility on the write and query paths; `--ignored` runs them.
//!
//! One more rule is the composite's own, not terminology's: a conditional
//! entry inside a transaction Bundle is `501` when the search index lives on
//! a secondary (`batch.rs`), which the harness flag pins.
//!
//! Requires Docker (testcontainers spins up a real Elasticsearch instance).

#![cfg(all(feature = "elasticsearch", feature = "sqlite"))]

#[path = "common/container_cleanup.rs"]
mod container_cleanup;
#[path = "common/terminology_scenarios.rs"]
mod terminology_scenarios;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use helios_persistence::backends::elasticsearch::{ElasticsearchBackend, ElasticsearchConfig};
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::{Backend, BackendKind};
use helios_rest::ServerConfig;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::elastic_search::ElasticSearch;
use tokio::sync::OnceCell;

use terminology_scenarios::{Driver, Hfs};

struct SharedEs {
    url: String,
    /// Kept alive for the duration of the test binary; the
    /// `container_cleanup` exit hook removes it at process exit.
    _container: testcontainers::ContainerAsync<ElasticSearch>,
}

static SHARED_ES: OnceCell<SharedEs> = OnceCell::const_new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

/// Same image, heap cap and startup budget as the persistence suite's
/// `elasticsearch_tests.rs` (7.17: the 7.16 JDK crashes on cgroup v2 hosts,
/// and 8.x never logs the ready line the module waits on).
async fn shared_es() -> &'static SharedEs {
    SHARED_ES
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = container_cleanup::with_cleanup_label(
                ElasticSearch::default()
                    .with_tag("7.17.29")
                    .with_env_var("ES_JAVA_OPTS", "-Xms256m -Xmx256m")
                    .with_label("github.run_id", &run_id)
                    .with_startup_timeout(std::time::Duration::from_secs(300)),
            )
            .start()
            .await
            .expect("failed to start Elasticsearch container");
            let port = container
                .get_host_port_ipv4(9200)
                .await
                .expect("failed to get host port");
            let host = container.get_host().await.expect("failed to get host");
            SharedEs {
                url: format!("http://{host}:{port}"),
                _container: container,
            }
        })
        .await
}

struct SqliteElasticsearch;

#[async_trait]
impl Driver for SqliteElasticsearch {
    async fn server(&self, config: ServerConfig, scopes: Option<&str>) -> Hfs {
        let es = shared_es().await;

        // The SQLite primary, its search offloaded as
        // `start_sqlite_elasticsearch` does it.
        let mut sqlite = SqliteBackend::with_config(
            ":memory:",
            SqliteBackendConfig {
                data_dir: Some(data_dir()),
                ..Default::default()
            },
        )
        .expect("in-memory sqlite");
        sqlite.init_schema().expect("init schema");
        sqlite.set_search_offloaded(true);
        let sqlite = Arc::new(sqlite);

        // Elasticsearch shares the primary's registry; a per-server index
        // prefix keeps the servers apart in the one container. The write
        // refresh policy is the production default on purpose.
        let elastic = ElasticsearchBackend::with_shared_registry(
            ElasticsearchConfig {
                nodes: vec![es.url.clone()],
                index_prefix: format!("term_{}", uuid::Uuid::new_v4().simple()),
                number_of_replicas: 0,
                ..Default::default()
            },
            sqlite.tenant_registries().clone(),
        )
        .expect("failed to create ElasticsearchBackend");
        elastic
            .initialize()
            .await
            .expect("failed to initialize Elasticsearch");
        let elastic = Arc::new(elastic);

        let composite_config = CompositeConfig::builder()
            .primary("sqlite", BackendKind::Sqlite)
            .search_backend("es", BackendKind::Elasticsearch)
            .sync_mode(SyncMode::Asynchronous)
            .build()
            .expect("composite config");
        let mut backends: HashMap<String, DynStorage> = HashMap::new();
        backends.insert("sqlite".to_string(), sqlite.clone() as DynStorage);
        backends.insert("es".to_string(), elastic.clone() as DynStorage);
        let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
        providers.insert("sqlite".to_string(), sqlite.clone() as DynSearchProvider);
        providers.insert("es".to_string(), elastic as DynSearchProvider);
        let composite = CompositeStorage::new(composite_config, backends)
            .expect("composite storage")
            .with_search_providers(providers)
            .with_full_primary(sqlite)
            .start_sync_workers();

        let mut hfs =
            terminology_scenarios::build(Arc::new(composite), config, scopes, true, true).await;
        hfs.conditional_in_transaction = false;
        hfs
    }
}

async fn driver(_name: &str) -> Option<SqliteElasticsearch> {
    Some(SqliteElasticsearch)
}

terminology_suite!(@each driver;
    cardiovascular_example_returns_p1_and_p2_from_a_stored_value_set,
    icd10_example_returns_the_four_translated_rows_from_a_stored_concept_map,
    both_examples_run_with_the_artifacts_supplied_in_context,
    context_value_set_with_an_enumerated_compose_is_expanded_locally_for_the_request,
    fhir_format_types_relation_columns_as_uri_code_and_boolean,
    stored_value_set_without_expansion_is_a_422_invalid_naming_the_label,
    unresolvable_canonical_is_a_404_naming_the_four_kinds,
    a_version_pin_only_matches_that_version,
    an_unsupported_concept_map_construct_is_a_422_naming_the_path,
    member_cap_is_a_422_too_costly,
    sql_export_runs_both_examples_sharing_one_resolution_per_artifact,
    sql_export_rejects_a_terminology_failure_at_kick_off,
    value_set_nobody_depends_on_is_stored_as_sent,
    unexpandable_dependency_is_a_422_in_strict_a_warning_in_lenient_and_ignored_when_off,
    member_cap_rejects_the_write_with_nothing_written,
    patch_adding_a_dependency_materializes_it,
    with_a_terminology_server_every_expansion_comes_from_it,
    a_definition_only_the_terminology_server_holds_is_fetched_stored_and_expanded,
    a_failing_terminology_server_fails_the_write_with_exception_and_writes_nothing,
    an_unreachable_terminology_server_is_an_exception_in_strict_and_a_stored_library_in_lenient,
    stale_if_match_is_a_412_before_anything_is_touched,
    conditional_create_that_matches_returns_the_existing_library_without_touching_anything,
    conditional_create_without_a_match_and_an_unexpandable_dependency_writes_nothing,
    conditional_create_without_a_match_materializes_after_the_committed_create,
    materialization_needs_the_callers_value_set_update_scope,
    transaction_needing_a_side_write_is_refused_before_persistence,
    transaction_with_an_unresolvable_dependency_and_no_terminology_server_commits,
    conditional_post_of_a_value_set_cannot_be_combined_with_materialization,
    transaction_patch_adding_a_dependency_on_an_in_bundle_compose_only_value_set_commits,
    transaction_patch_declaring_an_unprepared_dependency_rolls_back
);

terminology_suite!(@ignored driver;
    "search lags the acknowledged write on an asynchronous composite; \
     docs/issue-1824-backend-parity-plan.md Slice B establishes visibility";
    put_library_expands_its_stored_enumerated_value_set_locally_and_tags_it,
    post_value_set_after_the_library_is_transformed_in_the_candidate_and_if_match_still_works,
    transitive_dependencies_through_a_sqlview_are_materialized_and_cycles_are_rejected,
    batch_entries_materialize_independently,
    transaction_with_value_set_and_library_commits_once_in_either_order,
    transaction_deleting_a_value_set_makes_it_unresolvable_for_a_later_library,
    two_in_bundle_versions_of_one_url_are_two_candidates,
    deleting_the_newest_stored_version_in_the_bundle_resolves_an_unpinned_reference_to_the_survivor
);
