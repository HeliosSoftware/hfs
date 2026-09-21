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
use helios_persistence::error::{BackendError, StorageError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
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

    CompositeStorage::new(config, backends)
        .expect("composite")
        .with_search_providers(providers)
        .with_full_primary(primary)
        .start_sync_workers()
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

/// With a dedicated search backend the composite resolves create / update /
/// delete criteria itself, and cannot serve patch: the primary applies
/// patches, and resolves the criteria against an index that is offloaded and
/// empty. It used to delegate anyway, so every conditional patch was a silent
/// no-match; it now refuses, in step with what it declares.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dedicated_search_backend_serves_all_but_conditional_patch() {
    let composite = composite_with_search_backend(None);
    let t = tenant();

    for interaction in ConditionalInteraction::ALL {
        assert_eq!(
            composite.supports_conditional(interaction),
            interaction != ConditionalInteraction::Patch,
            "{interaction}"
        );
    }

    let created = composite
        .create(&t, "Organization", organization("ORG-P"), FhirVersion::R4)
        .await
        .expect("create through composite");

    match composite
        .conditional_patch(
            &t,
            "Organization",
            "identifier=urn:zzz:probe|ORG-P",
            &rename(),
            &helios_persistence::core::EntityTagPrecondition::Absent,
        )
        .await
    {
        Err(StorageError::Backend(BackendError::UnsupportedCapability { capability, .. })) => {
            assert_eq!(capability, "conditional_patch");
        }
        Ok(ConditionalPatchResult::NoMatch) => {
            panic!("a matching resource exists: NoMatch is the silent failure this guards")
        }
        other => panic!("expected UnsupportedCapability, got {other:?}"),
    }

    // Positive control: the same criteria do resolve on this composite, for an
    // interaction it declares.
    match composite
        .conditional_delete(
            &t,
            "Organization",
            "identifier=urn:zzz:probe|ORG-P",
            &helios_persistence::core::EntityTagPrecondition::Absent,
        )
        .await
        .expect("conditional delete")
    {
        ConditionalDeleteResult::Deleted(deleted) => assert_eq!(deleted.id(), created.id()),
        other => panic!("expected Deleted, got {other:?}"),
    }
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
            &helios_persistence::core::EntityTagPrecondition::Absent,
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
    match r4.conditional_delete(&t, "Patient", criteria).await {
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
        unset.conditional_delete(&t, "Patient", criteria).await,
        Ok(ConditionalDeleteResult::NoMatch)
    ));

    // Positive control: a type the version does have is accepted.
    assert!(matches!(
        r4.conditional_delete(&t, "Patient", "general-practitioner:Practitioner=p1")
            .await,
        Ok(ConditionalDeleteResult::NoMatch)
    ));
}
