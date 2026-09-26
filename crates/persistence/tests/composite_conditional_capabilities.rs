//! #1384: what a composite says it can do conditionally, and what it does.
//!
//! With a dedicated search backend the composite resolves conditional criteria
//! itself — the primary's own index is offloaded and empty — so which
//! conditional interactions work is a property of the *composition*, not of
//! the primary alone. `supports_conditional` is what the CapabilityStatement
//! and the REST layer's `501` read; these tests hold it to what the methods
//! really do.
//!
//! The search secondary here is a second SQLite backend: the shape of the
//! production `*-elasticsearch` composites without a container.

#![cfg(feature = "sqlite")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_persistence::composite::{
    CompositeConfig, CompositeStorage, DynSearchProvider, DynStorage, SyncMode,
};
use helios_persistence::core::{
    BackendKind, ConditionalDeleteResult, ConditionalInteraction, ConditionalPatchResult,
    ConditionalStorage, PatchFormat, ResourceStorage,
};
use helios_persistence::core::{EntityTagPrecondition, SearchProvider};
use helios_persistence::error::{ConcurrencyError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};
use serde_json::{Value, json};

fn tenant() -> TenantContext {
    TenantContext::new(TenantId::new("default"), TenantPermissions::full_access())
}

fn sqlite() -> SqliteBackend {
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
    backend
}

/// A production-shaped composite: primary with its own index offloaded, a
/// dedicated search secondary, synchronous sync.
fn composite_with_search_backend(fhir_version: Option<FhirVersion>) -> CompositeStorage {
    composite_and_its_primary(fhir_version).0
}

/// The same, with a handle on the primary so a test can write behind the
/// composite's back — what leaves the search backend's copy stale.
fn composite_and_its_primary(
    fhir_version: Option<FhirVersion>,
) -> (CompositeStorage, Arc<SqliteBackend>) {
    let mut primary = sqlite();
    primary.set_search_offloaded(true);
    let primary = Arc::new(primary);
    let index = Arc::new(sqlite());

    let mut builder = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .search_backend("search", BackendKind::Sqlite)
        .sync_mode(SyncMode::Synchronous);
    if let Some(v) = fhir_version {
        builder = builder.fhir_version(v);
    }
    let config = builder.build().expect("composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    backends.insert("search".to_string(), index.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), primary.clone() as DynSearchProvider);
    providers.insert("search".to_string(), index as DynSearchProvider);

    let composite = CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(primary.clone())
        .start_sync_workers();
    (composite, primary)
}

/// A composite that is only its primary: the primary indexes and searches.
fn composite_of_primary_only() -> CompositeStorage {
    let primary = Arc::new(sqlite());
    let config = CompositeConfig::builder()
        .primary("sqlite", BackendKind::Sqlite)
        .build()
        .expect("composite config");

    let mut backends: HashMap<String, DynStorage> = HashMap::new();
    backends.insert("sqlite".to_string(), primary.clone() as DynStorage);
    let mut providers: HashMap<String, DynSearchProvider> = HashMap::new();
    providers.insert("sqlite".to_string(), primary.clone() as DynSearchProvider);

    CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(primary)
}

fn organization(identifier: &str) -> Value {
    json!({
        "resourceType": "Organization",
        "identifier": [{"system": "urn:zzz:probe", "value": identifier}],
        "name": "ZZZ Probe Org"
    })
}

fn rename() -> PatchFormat {
    PatchFormat::JsonPatch(json!([
        {"op": "replace", "path": "/name", "value": "Patched"}
    ]))
}

/// Finds Organizations by `name` through the composite — that is, through
/// the search backend.
async fn names_found(composite: &CompositeStorage, t: &TenantContext, name: &str) -> Vec<String> {
    composite
        .search(
            t,
            &SearchQuery::new("Organization").with_parameter(SearchParameter {
                name: "name".to_string(),
                param_type: SearchParamType::String,
                values: vec![SearchValue::eq(name)],
                ..Default::default()
            }),
        )
        .await
        .expect("search through composite")
        .resources
        .items
        .iter()
        .map(|r| r.id().to_string())
        .collect()
}

/// With a dedicated search backend the composite resolves the criteria
/// itself and needs only plain CRUD from the primary, for all four
/// interactions. Patch was the exception until #1406: only the primary could
/// apply one, and it resolved the criteria against an index that is offloaded
/// and empty — a silent no-match before #1384, a refusal after. The applier is
/// now shared, so the composite resolves through the search backend, reads and
/// writes through the primary, and syncs the result like any update.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dedicated_search_backend_serves_every_conditional_interaction() {
    let composite = composite_with_search_backend(None);
    let t = tenant();

    for interaction in ConditionalInteraction::ALL {
        assert!(composite.supports_conditional(interaction), "{interaction}");
    }

    let created = composite
        .create(&t, "Organization", organization("ORG-P"), FhirVersion::R4)
        .await
        .expect("create through composite");
    composite
        .create(
            &t,
            "Organization",
            organization("ORG-OTHER"),
            FhirVersion::R4,
        )
        .await
        .expect("create decoy");
    let criteria = "identifier=urn:zzz:probe|ORG-P";

    // A stale `If-Match` is refused and nothing is written.
    let stale = EntityTagPrecondition::parse(["W/\"7\""]).expect("well-formed If-Match");
    let result = composite
        .conditional_patch(&t, "Organization", criteria, &rename(), &stale)
        .await;
    assert!(
        matches!(
            result,
            Err(StorageError::Concurrency(
                ConcurrencyError::OptimisticLockFailure { .. }
            ))
        ),
        "{result:?}"
    );

    match composite
        .conditional_patch(
            &t,
            "Organization",
            criteria,
            &rename(),
            &EntityTagPrecondition::Absent,
        )
        .await
        .expect("conditional patch")
    {
        ConditionalPatchResult::Patched(stored) => {
            assert_eq!(stored.id(), created.id());
            assert_eq!(stored.version_id(), "2");
            assert_eq!(stored.content()["name"], "Patched");
        }
        ConditionalPatchResult::NoMatch => {
            panic!("a matching resource exists: NoMatch is the silent failure this guards")
        }
        other => panic!("expected Patched, got {other:?}"),
    }

    // The primary holds the patched content ...
    let read = composite
        .read(&t, "Organization", created.id())
        .await
        .expect("read")
        .expect("still there");
    assert_eq!(read.version_id(), "2");
    assert_eq!(read.content()["name"], "Patched");
    // ... and the search backend was told: the new name is found, the old one
    // is not, the decoy is untouched.
    assert_eq!(names_found(&composite, &t, "Patched").await, [created.id()]);
    assert_eq!(names_found(&composite, &t, "ZZZ Probe Org").await.len(), 1);

    let result = composite
        .conditional_patch(
            &t,
            "Organization",
            "identifier=urn:zzz:probe|NOBODY",
            &rename(),
            &EntityTagPrecondition::Absent,
        )
        .await;
    assert!(
        matches!(result, Ok(ConditionalPatchResult::NoMatch)),
        "{result:?}"
    );

    // The same criteria resolve for the other interactions too.
    match composite
        .conditional_delete(&t, "Organization", criteria, &EntityTagPrecondition::Absent)
        .await
        .expect("conditional delete")
    {
        ConditionalDeleteResult::Deleted(deleted) => assert_eq!(deleted.id(), created.id()),
        other => panic!("expected Deleted, got {other:?}"),
    }
}

/// The match is found in the search backend; what gets patched is the
/// primary's content. When the two disagree on the version — a write the
/// search backend has not seen — the criteria were judged against content that
/// is no longer current, so the patch is a `VersionConflict` and writes
/// nothing, as conditional update and delete are through the primary's
/// compare-and-swap.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_search_copy_is_a_conflict_not_a_patch_over_unseen_content() {
    let (composite, primary) = composite_and_its_primary(None);
    let t = tenant();
    let criteria = "identifier=urn:zzz:probe|ORG-S";

    let created = composite
        .create(&t, "Organization", organization("ORG-S"), FhirVersion::R4)
        .await
        .expect("create through composite");

    // Behind the composite's back: the primary moves to version 2 and no
    // longer carries the identifier; the search backend still has version 1.
    let mut moved_on = created.content().clone();
    moved_on["identifier"] = json!([{"system": "urn:zzz:probe", "value": "ORG-ELSEWHERE"}]);
    primary
        .update(&t, &created, moved_on)
        .await
        .expect("direct primary update");

    let result = composite
        .conditional_patch(
            &t,
            "Organization",
            criteria,
            &rename(),
            &EntityTagPrecondition::Absent,
        )
        .await;
    assert!(
        matches!(
            result,
            Err(StorageError::Concurrency(
                ConcurrencyError::VersionConflict { .. }
            ))
        ),
        "{result:?}"
    );

    let read = composite
        .read(&t, "Organization", created.id())
        .await
        .expect("read")
        .expect("still there");
    assert_eq!(read.version_id(), "2");
    assert_eq!(read.content()["name"], "ZZZ Probe Org");
}

/// Without a dedicated search backend every conditional interaction is the
/// primary's, so the composite declares exactly what the primary does — and
/// the patch it declares works.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_search_backend_the_composite_follows_its_primary() {
    let composite = composite_of_primary_only();
    let t = tenant();

    for interaction in ConditionalInteraction::ALL {
        assert!(composite.supports_conditional(interaction), "{interaction}");
    }

    let created = composite
        .create(&t, "Organization", organization("ORG-Q"), FhirVersion::R4)
        .await
        .expect("create through composite");

    match composite
        .conditional_patch(
            &t,
            "Organization",
            "identifier=urn:zzz:probe|ORG-Q",
            &rename(),
            &EntityTagPrecondition::Absent,
        )
        .await
        .expect("conditional patch")
    {
        ConditionalPatchResult::Patched(stored) => {
            assert_eq!(stored.id(), created.id());
            assert_eq!(stored.content()["name"], "Patched");
        }
        other => panic!("expected Patched, got {other:?}"),
    }
}

/// A composite told its FHIR version judges a `:[type]` qualifier in
/// conditional criteria against that version, on the delete path that carries
/// none of its own. Only a multi-version build can tell the versions apart:
/// ActorDefinition is new in R5.
#[cfg(all(feature = "R4", feature = "R5"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_configured_version_scopes_the_type_qualifier_of_conditional_criteria() {
    let t = tenant();
    let criteria = "general-practitioner:ActorDefinition=a1";

    let r4 = composite_with_search_backend(Some(FhirVersion::R4));
    match r4
        .conditional_delete(&t, "Patient", criteria, &EntityTagPrecondition::Absent)
        .await
    {
        Err(e) => assert!(
            e.to_string().contains("nor a resource type of FHIR R4"),
            "{e}"
        ),
        Ok(other) => panic!("an R5 type must be refused by an R4 composite, got {other:?}"),
    }

    // Left unset, the fallback stands: a type of any enabled version passes,
    // and the criteria simply match nothing.
    let unset = composite_with_search_backend(None);
    assert!(matches!(
        unset
            .conditional_delete(&t, "Patient", criteria, &EntityTagPrecondition::Absent)
            .await,
        Ok(ConditionalDeleteResult::NoMatch)
    ));

    // Positive control: a type the version does have is accepted.
    assert!(matches!(
        r4.conditional_delete(
            &t,
            "Patient",
            "general-practitioner:Practitioner=p1",
            &EntityTagPrecondition::Absent,
        )
        .await,
        Ok(ConditionalDeleteResult::NoMatch)
    ));
}
