//! #1710: a composite with no dedicated Search backend splits a query that
//! uses a specialized feature (here a `:below` modifier, routed to a
//! Terminology-role secondary): the primary answers the whole query, sorted
//! and paged, and the secondary's matches filter that page. The secondary
//! must contribute every match, not its own page — paged in its own default
//! order, its window is a different slice of the result than the primary's,
//! and the filter dropped legitimate matches from the primary's page.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage,
};
use helios_persistence::core::search::SearchProvider;
use helios_persistence::core::{BackendKind, ResourceStorage};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{
    SearchModifier, SearchParamType, SearchParameter, SearchQuery, SearchValue, SortDirective,
};
use serde_json::{Value, json};

const MATCHES: usize = 8;
const PAGE: u32 = 3;

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("default"), TenantPermissions::full_access())
}

fn sqlite() -> Arc<SqliteBackend> {
    let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("data"))
        .expect("workspace data dir");
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        },
    )
    .expect("sqlite");
    backend.init_schema().expect("schema");
    Arc::new(backend)
}

/// Writes the same resource to both backends, as the composite's sync would.
async fn put(backends: &[&Arc<SqliteBackend>], resource_type: &str, id: &str, body: Value) {
    for backend in backends {
        backend
            .create_or_update(&tenant(), resource_type, id, body.clone(), FhirVersion::R4)
            .await
            .expect("seed");
    }
}

/// Eight matching ValueSets created in id order (so the backends' default
/// `_lastUpdated DESC` order is the reverse of `_sort=_id`), plus two whose
/// url falls outside the `:below` prefix.
async fn composite() -> CompositeStorage {
    let primary = sqlite();
    let terminology = sqlite();
    let both = [&primary, &terminology];

    for i in 1..=MATCHES {
        let id = format!("vs-{i:02}");
        put(
            &both,
            "ValueSet",
            &id,
            value_set(&id, "http://example.org/fhir"),
        )
        .await;
        // Distinct `last_updated` values, so default order is not an id tie.
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    for id in ["vs-other-1", "vs-other-2"] {
        put(
            &both,
            "ValueSet",
            id,
            value_set(id, "http://other.org/fhir"),
        )
        .await;
    }

    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .terminology_backend("terminology", BackendKind::Sqlite)
        .build()
        .expect("composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("terminology".to_string(), terminology.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), primary as DynSearchProvider);
    providers.insert("terminology".to_string(), terminology as DynSearchProvider);

    CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
}

fn value_set(id: &str, base: &str) -> Value {
    json!({
        "resourceType": "ValueSet",
        "id": id,
        "url": format!("{base}/ValueSet/{id}"),
        "status": "active"
    })
}

/// `ValueSet?url:below=http://example.org/fhir&_sort=_id&_count=3`.
fn example_value_sets_by_id() -> SearchQuery {
    SearchQuery::new("ValueSet")
        .with_parameter(SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: Some(SearchModifier::Below),
            values: vec![SearchValue::eq("http://example.org/fhir")],
            chain: vec![],
            components: vec![],
        })
        .with_sort(SortDirective::parse("_id"))
        .with_count(PAGE)
}

/// Every page of `query`, following the cursor (or offset) the composite
/// hands back, as the REST layer's `next` link would.
async fn walk(composite: &CompositeStorage, query: SearchQuery) -> Vec<Vec<String>> {
    let mut pages = Vec::new();
    let mut query = query;
    let mut offset = 0u32;
    loop {
        let result = composite
            .search(&tenant(), &query)
            .await
            .expect("composite search");
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect();
        offset += PAGE;
        pages.push(ids);
        if !result.resources.page_info.has_next || pages.len() > MATCHES {
            return pages;
        }
        match result.resources.page_info.next_cursor.clone() {
            Some(cursor) => {
                query.cursor = Some(cursor);
                query.offset = None;
            }
            None => query.offset = Some(offset),
        }
    }
}

#[tokio::test]
async fn split_query_pages_keep_every_match_in_primary_sort_order() {
    let composite = composite().await;

    let pages = walk(&composite, example_value_sets_by_id()).await;

    let expected: Vec<Vec<String>> = (1..=MATCHES)
        .map(|i| format!("vs-{i:02}"))
        .collect::<Vec<_>>()
        .chunks(PAGE as usize)
        .map(<[String]>::to_vec)
        .collect();
    assert_eq!(
        pages, expected,
        "each page must be the primary's sorted page, unfiltered by the \
         secondary's own paging window"
    );
}

#[tokio::test]
async fn split_query_offset_pages_keep_every_match() {
    let composite = composite().await;

    let mut all = Vec::new();
    for page in 0..(MATCHES as u32).div_ceil(PAGE) {
        let mut query = example_value_sets_by_id();
        query.offset = Some(page * PAGE);
        let result = composite
            .search(&tenant(), &query)
            .await
            .expect("composite search");
        all.extend(result.resources.items.iter().map(|r| r.id().to_string()));
    }

    let expected: Vec<String> = (1..=MATCHES).map(|i| format!("vs-{i:02}")).collect();
    assert_eq!(all, expected);
}
