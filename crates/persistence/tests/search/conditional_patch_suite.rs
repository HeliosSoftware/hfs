//! Backend-agnostic conditional patch suite (issue #1406).
//!
//! `ConditionalStorage::conditional_patch` is one provided implementation —
//! resolve the criteria, read, `If-Match`, apply the shared patch, `update`
//! with compare-and-swap — that every backend reaches through its own
//! `resolve_conditional_matches`. This drives it on each backend: the patch
//! formats, the three match outcomes, the precondition, and the refusals that
//! must leave storage untouched.
//!
//! Opens with a positive control: a backend built without the spec search
//! parameters does not know `identifier`, never matches anything, and would
//! turn every "refused" assertion below into a vacuous no-match.
//!
//! Included by `#[path]` into each backend's test binary, the same arrangement
//! as `conditional_if_match_suite.rs`.

#![allow(dead_code)]

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    ConditionalInteraction, ConditionalPatchResult, ConditionalStorage, EntityTagPrecondition,
    PatchError, PatchFormat, ResourceStorage, SearchProvider,
};
use helios_persistence::error::{ConcurrencyError, StorageError, ValidationError};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use helios_persistence::types::{SearchParamType, SearchParameter, SearchQuery, SearchValue};

const CRITERIA: &str = "identifier=ne123";
const NOBODY: &str = "identifier=nobody";
/// Both `twin-a` and `twin-b`.
const TWINS: &str = "identifier=twin";

const ABSENT: &EntityTagPrecondition = &EntityTagPrecondition::Absent;

fn patient(id: &str, family: &str, identifier: &str) -> Value {
    json!({
        "resourceType": "Patient",
        "id": id,
        "active": false,
        "name": [{"family": family}],
        "identifier": [{"system": "http://example.org/mrn", "value": identifier}]
    })
}

/// `(version, active, family)` of a Patient, or how it went missing.
async fn state_of<S: ResourceStorage>(backend: &S, tenant: &TenantContext, id: &str) -> String {
    match backend.read(tenant, "Patient", id).await {
        Ok(Some(stored)) => format!(
            "v{} {} {}",
            stored.version_id(),
            stored.content()["active"],
            stored.content()["name"][0]["family"]
                .as_str()
                .unwrap_or("?")
        ),
        Ok(None) => "<absent>".to_string(),
        Err(e) => format!("<{e}>"),
    }
}

fn patch_error<T: std::fmt::Debug>(result: Result<T, StorageError>) -> PatchError {
    match result {
        Err(StorageError::Validation(ValidationError::Patch(e))) => e,
        other => panic!("expected a typed patch error, got {other:?}"),
    }
}

/// Conditional patch, end to end, on one backend.
pub async fn conditional_patch_resolves_gates_applies_and_swaps<S>(backend: &S, base: &str)
where
    S: ResourceStorage + ConditionalStorage + SearchProvider,
{
    assert!(
        backend.supports_conditional(ConditionalInteraction::Patch),
        "the backend must declare what this suite shows it serves"
    );

    let t = TenantContext::new(
        TenantId::new(format!("{base}-patch")),
        TenantPermissions::full_access(),
    );
    for (id, family, identifier) in [
        ("target", "Neal", "ne123"),
        ("decoy", "Smith", "mrn-1"),
        ("twin-a", "Twin", "twin"),
        ("twin-b", "Twin", "twin"),
    ] {
        backend
            .create(
                &t,
                "Patient",
                patient(id, family, identifier),
                FhirVersion::default(),
            )
            .await
            .expect("seed patient");
    }

    // Positive control: a plain search on the criteria's parameter finds the
    // target, so a `NoMatch` below is about the criteria, not the registry.
    let found = backend
        .search(
            &t,
            &SearchQuery::new("Patient").with_parameter(SearchParameter {
                name: "identifier".to_string(),
                param_type: SearchParamType::Token,
                values: vec![SearchValue::eq("ne123")],
                ..Default::default()
            }),
        )
        .await
        .expect("positive control search");
    let ids: Vec<&str> = found.resources.items.iter().map(|r| r.id()).collect();
    assert_eq!(
        ids,
        ["target"],
        "positive control: is the backend built with the spec search parameters?"
    );

    let untouched = "v1 false Neal";

    // ---- refusals: nothing is written --------------------------------------
    let activate = PatchFormat::JsonPatch(json!([
        {"op": "replace", "path": "/active", "value": true}
    ]));

    let result = backend
        .conditional_patch(&t, "Patient", NOBODY, &activate, ABSENT)
        .await;
    assert!(
        matches!(result, Ok(ConditionalPatchResult::NoMatch)),
        "{result:?}"
    );

    let result = backend
        .conditional_patch(&t, "Patient", TWINS, &activate, ABSENT)
        .await;
    assert!(
        matches!(result, Ok(ConditionalPatchResult::MultipleMatches(2))),
        "{result:?}"
    );
    assert_eq!(state_of(backend, &t, "twin-a").await, "v1 false Twin");
    assert_eq!(state_of(backend, &t, "twin-b").await, "v1 false Twin");

    let stale = EntityTagPrecondition::parse(["W/\"7\""]).expect("well-formed If-Match");
    let result = backend
        .conditional_patch(&t, "Patient", CRITERIA, &activate, &stale)
        .await;
    assert!(
        matches!(
            result,
            Err(StorageError::Concurrency(
                ConcurrencyError::OptimisticLockFailure { .. }
            ))
        ),
        "stale If-Match: {result:?}"
    );

    for (element, patch) in [
        (
            "resourceType",
            PatchFormat::JsonPatch(
                json!([{"op": "replace", "path": "/resourceType", "value": "Person"}]),
            ),
        ),
        (
            "resourceType",
            PatchFormat::MergePatch(json!({"resourceType": "Person"})),
        ),
        (
            "id",
            PatchFormat::JsonPatch(json!([{"op": "replace", "path": "/id", "value": "other"}])),
        ),
        ("id", PatchFormat::MergePatch(json!({"id": "other"}))),
    ] {
        let result = backend
            .conditional_patch(&t, "Patient", CRITERIA, &patch, ABSENT)
            .await;
        assert_eq!(
            patch_error(result),
            PatchError::ImmutableElement { element },
            "{patch:?}"
        );
    }

    // A failed `test` stops the whole document: the `replace` before it is
    // not written either.
    let result = backend
        .conditional_patch(
            &t,
            "Patient",
            CRITERIA,
            &PatchFormat::JsonPatch(json!([
                {"op": "replace", "path": "/name/0/family", "value": "Half"},
                {"op": "test", "path": "/active", "value": true}
            ])),
            ABSENT,
        )
        .await;
    assert!(
        matches!(patch_error(result), PatchError::TestFailed { .. }),
        "failed test op"
    );

    let result = backend
        .conditional_patch(
            &t,
            "Patient",
            CRITERIA,
            &PatchFormat::JsonPatch(json!({"not": "a patch"})),
            ABSENT,
        )
        .await;
    assert!(
        matches!(patch_error(result), PatchError::MalformedDocument { .. }),
        "malformed document"
    );

    let result = backend
        .conditional_patch(
            &t,
            "Patient",
            CRITERIA,
            &PatchFormat::JsonPatch(json!([{"op": "replace", "path": "/nope/deeper", "value": 1}])),
            ABSENT,
        )
        .await;
    assert!(
        matches!(patch_error(result), PatchError::OperationFailed { .. }),
        "unresolvable path"
    );

    // FHIRPath Patch used to be a stub that changed nothing and still wrote a
    // new version. It is refused, and no version is written.
    let result = backend
        .conditional_patch(
            &t,
            "Patient",
            CRITERIA,
            &PatchFormat::FhirPathPatch(json!({
                "resourceType": "Parameters",
                "parameter": [{"name": "operation", "part": [
                    {"name": "type", "valueCode": "replace"},
                    {"name": "path", "valueString": "Patient.name[0].family"},
                    {"name": "value", "valueString": "Changed"}
                ]}]
            })),
            ABSENT,
        )
        .await;
    assert!(
        matches!(patch_error(result), PatchError::UnsupportedFormat { .. }),
        "FHIRPath Patch"
    );

    assert_eq!(state_of(backend, &t, "target").await, untouched);

    // ---- one match: JSON Patch, with a satisfied If-Match -------------------
    let current = EntityTagPrecondition::parse(["W/\"1\""]).expect("well-formed If-Match");
    let result = backend
        .conditional_patch(
            &t,
            "Patient",
            CRITERIA,
            &PatchFormat::JsonPatch(json!([
                {"op": "test", "path": "/active", "value": false},
                {"op": "replace", "path": "/active", "value": true}
            ])),
            &current,
        )
        .await;
    match &result {
        Ok(ConditionalPatchResult::Patched(stored)) => {
            assert_eq!(stored.id(), "target");
            assert_eq!(stored.version_id(), "2");
            assert_eq!(stored.content()["active"], json!(true));
            assert_eq!(stored.content()["resourceType"], json!("Patient"));
            assert_eq!(stored.content()["id"], json!("target"));
        }
        other => panic!("JSON Patch on one match: {other:?}"),
    }
    assert_eq!(state_of(backend, &t, "target").await, "v2 true Neal");

    // ---- one match: Merge Patch. The criteria still resolve after the first
    // patch, so the index followed the write. -------------------------------
    let result = backend
        .conditional_patch(
            &t,
            "Patient",
            CRITERIA,
            &PatchFormat::MergePatch(json!({"name": [{"family": "Merged"}], "active": null})),
            ABSENT,
        )
        .await;
    assert!(
        matches!(&result, Ok(ConditionalPatchResult::Patched(s)) if s.version_id() == "3"),
        "Merge Patch on one match: {result:?}"
    );
    assert_eq!(state_of(backend, &t, "target").await, "v3 null Merged");

    assert_eq!(state_of(backend, &t, "decoy").await, "v1 false Smith");
}
