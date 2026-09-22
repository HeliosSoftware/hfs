//! Which conditional interactions this deployment serves (#1384).
//!
//! One source, [`ConditionalStorage::supports_conditional`], answers both
//! questions a client can ask: what the CapabilityStatement advertises in
//! `rest.resource.conditional*` ([`advertised`]) and whether a conditional
//! request is served or refused ([`require`]). They used to be independent —
//! the statement was a set of literals — so a MongoDB deployment advertised a
//! conditional patch it answered `501` for, and an S3 one advertised all four.
//!
//! [`require`] is called on one line at the top of every conditional handler
//! and of every conditional Bundle entry, before anything else is decided, so
//! the refusal names the interaction the client asked for rather than whichever
//! capability the backend happened to trip over (`'search'`, for S3).

use helios_fhir::FhirVersion;
use helios_persistence::core::{ConditionalInteraction, ConditionalStorage};

use crate::error::{RestError, RestResult};

/// How the interaction reads on the wire, for the refusal's diagnostics.
fn wire_form(interaction: ConditionalInteraction) -> &'static str {
    match interaction {
        ConditionalInteraction::Create => "If-None-Exist",
        ConditionalInteraction::Update => "PUT [type]?criteria",
        ConditionalInteraction::Delete => "DELETE [type]?criteria",
        ConditionalInteraction::Patch => "PATCH [type]?criteria",
    }
}

/// `501` + `not-supported` when the storage does not serve `interaction`.
pub(crate) fn require<S>(storage: &S, interaction: ConditionalInteraction) -> RestResult<()>
where
    S: ConditionalStorage + ?Sized,
{
    if storage.supports_conditional(interaction) {
        return Ok(());
    }
    Err(RestError::NotImplemented {
        feature: format!(
            "{interaction} ({}) on this storage backend",
            wire_form(interaction)
        ),
    })
}

/// [`require`] for `POST [type]` with `If-None-Exist`.
pub(crate) fn require_create<S: ConditionalStorage + ?Sized>(storage: &S) -> RestResult<()> {
    require(storage, ConditionalInteraction::Create)
}

/// [`require`] for `PUT [type]?criteria`.
pub(crate) fn require_update<S: ConditionalStorage + ?Sized>(storage: &S) -> RestResult<()> {
    require(storage, ConditionalInteraction::Update)
}

/// [`require`] for `DELETE [type]?criteria`.
pub(crate) fn require_delete<S: ConditionalStorage + ?Sized>(storage: &S) -> RestResult<()> {
    require(storage, ConditionalInteraction::Delete)
}

/// [`require`] for `PATCH [type]?criteria`.
pub(crate) fn require_patch<S: ConditionalStorage + ?Sized>(storage: &S) -> RestResult<()> {
    require(storage, ConditionalInteraction::Patch)
}

/// The `rest.resource.conditionalCreate` / `conditionalUpdate` /
/// `conditionalDelete` / `conditionalPatch` elements of a CapabilityStatement
/// resource entry, each from what the storage really serves.
///
/// - `conditionalDelete` is a code, not a boolean: `single` or
///   `not-supported`. Never `multiple` — criteria that match more than one
///   resource are answered `412`, on every backend.
/// - `conditionalPatch` exists from FHIR R5 on. `PATCH [type]?criteria` is
///   served for every version, but R4 / R4B have no element to say so, and
///   emitting it there would make the statement invalid.
pub(crate) fn advertised<S>(
    storage: &S,
    version: FhirVersion,
) -> serde_json::Map<String, serde_json::Value>
where
    S: ConditionalStorage + ?Sized,
{
    use serde_json::Value;

    let supports = |interaction| storage.supports_conditional(interaction);
    let mut elements = serde_json::Map::new();
    elements.insert(
        "conditionalCreate".to_string(),
        Value::Bool(supports(ConditionalInteraction::Create)),
    );
    elements.insert(
        "conditionalUpdate".to_string(),
        Value::Bool(supports(ConditionalInteraction::Update)),
    );
    let delete = if supports(ConditionalInteraction::Delete) {
        "single"
    } else {
        "not-supported"
    };
    elements.insert(
        "conditionalDelete".to_string(),
        Value::String(delete.to_string()),
    );
    if matches!(version.as_mime_param(), "5.0" | "6.0") {
        elements.insert(
            "conditionalPatch".to_string(),
            Value::Bool(supports(ConditionalInteraction::Patch)),
        );
    }
    elements
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use axum::http::StatusCode;
    use helios_persistence::core::{
        ConditionalCreateResult, ConditionalDeleteResult, ConditionalUpdateResult, ResourceStorage,
    };
    use helios_persistence::error::StorageResult;
    use helios_persistence::tenant::TenantContext;
    use helios_persistence::types::StoredResource;
    use serde_json::{Value, json};

    /// A storage that declares exactly the listed interactions and serves
    /// nothing: only its declaration is under test.
    struct Declares(&'static [ConditionalInteraction]);

    /// SQLite, PostgreSQL, MongoDB; a composite with a dedicated search backend.
    const ALL: Declares = Declares(&ConditionalInteraction::ALL);
    /// What MongoDB and those composites declared before #1406; any storage
    /// that resolves criteria but declines patch.
    const ALL_BUT_PATCH: Declares = Declares(&[
        ConditionalInteraction::Create,
        ConditionalInteraction::Update,
        ConditionalInteraction::Delete,
    ]);
    /// S3.
    const NONE: Declares = Declares(&[]);

    #[async_trait]
    impl ResourceStorage for Declares {
        fn backend_name(&self) -> &'static str {
            "declares"
        }

        async fn create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn create_or_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
            _resource: Value,
            _fhir_version: FhirVersion,
        ) -> StorageResult<(StoredResource, bool)> {
            unimplemented!()
        }

        async fn read(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<Option<StoredResource>> {
            unimplemented!()
        }

        async fn update(
            &self,
            _tenant: &TenantContext,
            _current: &StoredResource,
            _resource: Value,
        ) -> StorageResult<StoredResource> {
            unimplemented!()
        }

        async fn delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _id: &str,
        ) -> StorageResult<()> {
            unimplemented!()
        }

        async fn count(
            &self,
            _tenant: &TenantContext,
            _resource_type: Option<&str>,
        ) -> StorageResult<u64> {
            unimplemented!()
        }
    }

    #[async_trait]
    impl ConditionalStorage for Declares {
        fn supports_conditional(&self, interaction: ConditionalInteraction) -> bool {
            self.0.contains(&interaction)
        }

        async fn conditional_create(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _search_params: &str,
            _fhir_version: FhirVersion,
        ) -> StorageResult<ConditionalCreateResult> {
            unimplemented!()
        }

        async fn conditional_update(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _resource: Value,
            _search_params: &str,
            _upsert: bool,
            _fhir_version: FhirVersion,
            _if_match: &helios_persistence::core::EntityTagPrecondition,
        ) -> StorageResult<ConditionalUpdateResult> {
            unimplemented!()
        }

        async fn conditional_delete(
            &self,
            _tenant: &TenantContext,
            _resource_type: &str,
            _search_params: &str,
            _if_match: &helios_persistence::core::EntityTagPrecondition,
        ) -> StorageResult<ConditionalDeleteResult> {
            unimplemented!()
        }
    }

    #[test]
    fn a_declared_interaction_is_served() {
        for interaction in ConditionalInteraction::ALL {
            assert!(require(&ALL, interaction).is_ok(), "{interaction}");
        }
        assert!(require_create(&ALL_BUT_PATCH).is_ok());
        assert!(require_update(&ALL_BUT_PATCH).is_ok());
        assert!(require_delete(&ALL_BUT_PATCH).is_ok());
    }

    #[test]
    fn an_undeclared_interaction_is_501_not_supported_and_named() {
        for (refusal, wording) in [
            (require_create(&NONE), "conditional create (If-None-Exist)"),
            (
                require_update(&NONE),
                "conditional update (PUT [type]?criteria)",
            ),
            (
                require_delete(&NONE),
                "conditional delete (DELETE [type]?criteria)",
            ),
            (
                require_patch(&NONE),
                "conditional patch (PATCH [type]?criteria)",
            ),
            (
                require_patch(&ALL_BUT_PATCH),
                "conditional patch (PATCH [type]?criteria)",
            ),
        ] {
            let (status, code, text) = refusal.expect_err(wording).client_response();
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{wording}");
            assert_eq!(code, "not-supported", "{wording}");
            assert!(text.contains(wording), "{text}");
        }
    }

    /// R4 has no `conditionalPatch` element, whatever the storage serves.
    #[cfg(feature = "R4")]
    #[test]
    fn r4_elements_follow_the_declaration() {
        let of = |storage: &Declares| Value::Object(advertised(storage, FhirVersion::R4));
        let served = json!({
            "conditionalCreate": true,
            "conditionalUpdate": true,
            "conditionalDelete": "single"
        });
        assert_eq!(of(&ALL), served);
        assert_eq!(of(&ALL_BUT_PATCH), served);
        assert_eq!(
            of(&NONE),
            json!({
                "conditionalCreate": false,
                "conditionalUpdate": false,
                "conditionalDelete": "not-supported"
            })
        );
    }

    /// From R5 on `conditionalPatch` is a statement of its own.
    #[cfg(feature = "R5")]
    #[test]
    fn r5_adds_conditional_patch_from_the_declaration() {
        let patch =
            |storage: &Declares| advertised(storage, FhirVersion::R5)["conditionalPatch"].clone();
        assert_eq!(patch(&ALL), json!(true));
        assert_eq!(patch(&ALL_BUT_PATCH), json!(false));
        assert_eq!(patch(&NONE), json!(false));
    }
}
