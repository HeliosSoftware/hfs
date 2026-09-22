//! The one patch applier.
//!
//! `PATCH [type]/[id]` (applied by the REST layer) and `PATCH [type]?criteria`
//! (applied by [`ConditionalStorage::conditional_patch`]) both go through
//! [`apply_patch`], so a patch document means the same thing on either
//! endpoint and on every backend (#1406). Before, REST, SQLite and PostgreSQL
//! each carried a private copy.
//!
//! # Formats
//!
//! * JSON Patch (RFC 6902) and JSON Merge Patch (RFC 7396) are applied with
//!   the `json-patch` crate.
//! * FHIRPath Patch is **not implemented** and answers
//!   [`PatchError::UnsupportedFormat`]. The private appliers this module
//!   replaces had a stub for it that ignored every path but `Type.element`,
//!   skipped unknown operation types, and so could change nothing — while the
//!   caller still wrote a new version.
//!
//! # Invariants
//!
//! A patch cannot change or remove `resourceType` or `id`: they are the
//! identity of the resource the request named. Backends re-assert both on
//! every `update`, so such a patch could not corrupt a row — it would be
//! silently undone and answered as a success, which is why it is refused
//! here instead.
//!
//! `meta.versionId` and `meta.lastUpdated` are not guarded. They are
//! server-assigned on every write, exactly as for a `PUT` whose body carries
//! them, so a patch naming them has no lasting effect.
//!
//! [`ConditionalStorage::conditional_patch`]: super::ConditionalStorage::conditional_patch

use serde_json::Value;
use thiserror::Error;

use super::PatchFormat;

/// Why a patch was not applied. Nothing has been written when one is returned.
///
/// The variants are what a REST layer needs to choose a status: today all but
/// [`UnsupportedFormat`](Self::UnsupportedFormat) are a `400`, and a failed
/// `test` has a variant of its own so that can change on its own (#1393).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatchError {
    /// The patch document is not a document of its format — for JSON Patch,
    /// not an array of RFC 6902 operations.
    #[error("Invalid {format}: {message}")]
    MalformedDocument {
        /// The format, as a client would name it (`JSON Patch`).
        format: &'static str,
        /// The parser's complaint.
        message: String,
    },

    /// A JSON Patch `test` operation compared unequal (RFC 6902 §4.6). The
    /// document is well-formed and every path resolved; the resource is just
    /// not in the state the client expected.
    #[error("Failed to apply JSON Patch: {message}")]
    TestFailed {
        /// Which operation failed, and where.
        message: String,
    },

    /// A JSON Patch operation could not be carried out against this resource:
    /// a path that does not resolve, a `move` into its own source.
    #[error("Failed to apply JSON Patch: {message}")]
    OperationFailed {
        /// Which operation failed, and where.
        message: String,
    },

    /// The patch changes or removes an element that identifies the resource.
    #[error("Cannot change {element} via patch")]
    ImmutableElement {
        /// `resourceType` or `id`.
        element: &'static str,
    },

    /// The format is recognised but not implemented (FHIRPath Patch).
    #[error("{format} is not implemented")]
    UnsupportedFormat {
        /// The format, as a client would name it (`FHIRPath Patch`).
        format: &'static str,
    },
}

/// The elements a patch may neither change nor remove.
const IMMUTABLE_ELEMENTS: [&str; 2] = ["resourceType", "id"];

/// Applies `patch` to `current` and returns the patched content.
///
/// `current` is the content of the resource as stored; it is not modified.
/// The result still has the `resourceType` and `id` of `current` (see the
/// module docs), and is otherwise unvalidated: the caller writes it with
/// [`ResourceStorage::update`](super::ResourceStorage::update) like any other
/// content.
pub fn apply_patch(current: &Value, patch: &PatchFormat) -> Result<Value, PatchError> {
    let mut patched = current.clone();

    match patch {
        PatchFormat::JsonPatch(operations) => {
            let operations: json_patch::Patch = serde_json::from_value(operations.clone())
                .map_err(|e| PatchError::MalformedDocument {
                    format: "JSON Patch",
                    message: e.to_string(),
                })?;

            json_patch::patch(&mut patched, &operations).map_err(|e| {
                let message = e.to_string();
                match e.kind {
                    json_patch::PatchErrorKind::TestFailed => PatchError::TestFailed { message },
                    _ => PatchError::OperationFailed { message },
                }
            })?;
        }
        PatchFormat::MergePatch(merge_doc) => json_patch::merge(&mut patched, merge_doc),
        PatchFormat::FhirPathPatch(_) => {
            return Err(PatchError::UnsupportedFormat {
                format: "FHIRPath Patch",
            });
        }
    }

    for element in IMMUTABLE_ELEMENTS {
        // Compared as JSON: a resource stored without the element (nothing
        // requires `current` to carry its id) may not gain one either.
        if patched.get(element) != current.get(element) {
            return Err(PatchError::ImmutableElement { element });
        }
    }

    Ok(patched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn patient() -> Value {
        json!({
            "resourceType": "Patient",
            "id": "p1",
            "active": false,
            "name": [{"family": "Before"}]
        })
    }

    #[test]
    fn json_patch_is_applied_and_the_input_left_alone() {
        let current = patient();
        let patched = apply_patch(
            &current,
            &PatchFormat::JsonPatch(json!([
                {"op": "test", "path": "/active", "value": false},
                {"op": "replace", "path": "/active", "value": true},
                {"op": "replace", "path": "/name/0/family", "value": "After"}
            ])),
        )
        .unwrap();

        assert_eq!(patched["active"], json!(true));
        assert_eq!(patched["name"][0]["family"], json!("After"));
        assert_eq!(current, patient());
    }

    #[test]
    fn merge_patch_sets_and_removes() {
        let patched = apply_patch(
            &patient(),
            &PatchFormat::MergePatch(json!({"active": true, "name": null, "gender": "other"})),
        )
        .unwrap();

        assert_eq!(
            patched,
            json!({"resourceType": "Patient", "id": "p1", "active": true, "gender": "other"})
        );
    }

    #[test]
    fn a_document_that_is_not_a_json_patch_is_malformed() {
        for document in [
            json!({"not": "a patch"}),
            json!([{"op": "frobnicate", "path": "/active"}]),
            json!([{"op": "replace", "path": "no-leading-slash", "value": 1}]),
        ] {
            let err = apply_patch(&patient(), &PatchFormat::JsonPatch(document.clone()));
            assert!(
                matches!(
                    err,
                    Err(PatchError::MalformedDocument {
                        format: "JSON Patch",
                        ..
                    })
                ),
                "{document}: {err:?}"
            );
        }
    }

    #[test]
    fn a_failed_test_is_told_apart_from_an_operation_that_cannot_apply() {
        let failed_test = apply_patch(
            &patient(),
            &PatchFormat::JsonPatch(json!([{"op": "test", "path": "/active", "value": true}])),
        );
        assert!(
            matches!(failed_test, Err(PatchError::TestFailed { .. })),
            "{failed_test:?}"
        );

        let bad_path = apply_patch(
            &patient(),
            &PatchFormat::JsonPatch(json!([{"op": "replace", "path": "/nope/deeper", "value": 1}])),
        );
        assert!(
            matches!(bad_path, Err(PatchError::OperationFailed { .. })),
            "{bad_path:?}"
        );
    }

    #[test]
    fn resource_type_and_id_can_be_neither_changed_nor_removed() {
        let refused = [
            (
                "resourceType",
                PatchFormat::JsonPatch(
                    json!([{"op": "replace", "path": "/resourceType", "value": "Person"}]),
                ),
            ),
            (
                "resourceType",
                PatchFormat::JsonPatch(json!([{"op": "remove", "path": "/resourceType"}])),
            ),
            (
                "resourceType",
                PatchFormat::MergePatch(json!({"resourceType": "Person"})),
            ),
            (
                "id",
                PatchFormat::JsonPatch(json!([{"op": "replace", "path": "/id", "value": "p2"}])),
            ),
            (
                "id",
                PatchFormat::JsonPatch(json!([{"op": "move", "from": "/id", "path": "/was"}])),
            ),
            ("id", PatchFormat::MergePatch(json!({"id": null}))),
            ("id", PatchFormat::MergePatch(json!({"id": "p2"}))),
        ];
        for (element, patch) in refused {
            assert_eq!(
                apply_patch(&patient(), &patch),
                Err(PatchError::ImmutableElement { element }),
                "{patch:?}"
            );
        }

        // Naming them without changing them is not a change.
        for patch in [
            PatchFormat::MergePatch(json!({"resourceType": "Patient", "id": "p1", "active": true})),
            PatchFormat::JsonPatch(json!([
                {"op": "test", "path": "/id", "value": "p1"},
                {"op": "replace", "path": "/resourceType", "value": "Patient"},
                {"op": "replace", "path": "/active", "value": true}
            ])),
        ] {
            let patched = apply_patch(&patient(), &patch).unwrap();
            assert_eq!(patched["active"], json!(true), "{patch:?}");
        }
    }

    #[test]
    fn fhirpath_patch_is_refused_rather_than_half_applied() {
        let parameters = json!({
            "resourceType": "Parameters",
            "parameter": [{"name": "operation", "part": [
                {"name": "type", "valueCode": "replace"},
                {"name": "path", "valueString": "Patient.active"},
                {"name": "value", "valueBoolean": true}
            ]}]
        });
        assert_eq!(
            apply_patch(&patient(), &PatchFormat::FhirPathPatch(parameters)),
            Err(PatchError::UnsupportedFormat {
                format: "FHIRPath Patch"
            })
        );
    }
}
