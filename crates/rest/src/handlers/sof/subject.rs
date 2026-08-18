//! Subject naming and resolution for `$sql-run` and `$sql-export`.
//!
//! SQL on FHIR 3.0.0-ballot consolidated four operations into two. Both act on
//! a **subject** — a ViewDefinition, a [SQLQuery] Library or a [SQLView]
//! Library — named by a parameter rather than by the request path, so one
//! operation serves all three artifact kinds and both operations are invoked at
//! the system level.
//!
//! [SQLQuery]: http://hl7.org/fhir/uv/sql-on-fhir/StructureDefinition-SQLQuery.html
//! [SQLView]: http://hl7.org/fhir/uv/sql-on-fhir/StructureDefinition-SQLView.html
//!
//! ## The three naming parameters
//!
//! | Parameter          | Names the subject by                                     |
//! |--------------------|----------------------------------------------------------|
//! | `subjectCanonical` | Canonical URL, optionally `\|version`-pinned              |
//! | `subjectReference` | Literal location — relative on this server, or absolute  |
//! | `subjectResource`  | Inline resource (requires POST)                          |
//!
//! Exactly one is supplied. Supplying none, or more than one, is rejected with
//! `400 Bad Request`.
//!
//! `subjectCanonical` and `subjectReference` are deliberately distinct. A
//! canonical URL is an *identity* the server looks up in its index; a reference
//! is a *location* it reads directly. The pre-ballot `viewReference` parameter
//! accepted both shapes in one field, which meant a server could not tell which
//! lookup a client intended.
//!
//! ## Status codes
//!
//! A subject that cannot be resolved yields `404 Not Found`: it is the thing the
//! operation is about, so the operation cannot proceed without it. This is
//! distinct from an unresolvable `patient` or `group`, which merely *scopes* the
//! data and yields `400 Bad Request` (see operations-common.html#filter-resolution-errors).

use helios_persistence::core::search::SearchProvider;
use helios_persistence::tenant::TenantContext;
use serde_json::Value;

use super::references::resolve_resource_canonical_or_relative;
use crate::error::RestError;
use crate::state::AppState;

/// `Library.type.coding.code` identifying a Library as a reusable SQL view.
const LIBRARY_TYPE_SQL_VIEW: &str = "sql-view";

/// Which kind of artifact a subject turned out to be.
///
/// The operation does not change based on the kind — the same `$sql-run`
/// evaluates all three — but what happens downstream does: a ViewDefinition is
/// projected directly, while a Library's dependency graph is resolved first and
/// its SQL executed against the resulting tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    /// A ViewDefinition: flattens FHIR resources into one table.
    ViewDefinition,
    /// A `SQLQuery` Library: one logical SQL query over view tables.
    SqlQuery,
    /// A `SQLView` Library: a query whose result serves as a table for others.
    SqlView,
}

impl SubjectKind {
    /// The FHIR resource type this kind is stored as.
    pub fn resource_type(self) -> &'static str {
        match self {
            SubjectKind::ViewDefinition => "ViewDefinition",
            SubjectKind::SqlQuery | SubjectKind::SqlView => "Library",
        }
    }

    /// Whether this kind accepts the `parameters` input. Only a Library
    /// declares parameters; supplying them for a ViewDefinition is a 400.
    pub fn accepts_parameters(self) -> bool {
        !matches!(self, SubjectKind::ViewDefinition)
    }
}

/// A subject that has been named, fetched, and classified.
#[derive(Debug, Clone)]
pub struct ResolvedSubject {
    /// Which of the three artifact kinds this turned out to be.
    pub kind: SubjectKind,
    /// The artifact itself.
    pub resource: Value,
}

/// How the caller named the subject. Built from a `Parameters` body or a query
/// string, then resolved by [`resolve_subject`].
#[derive(Debug, Default, Clone)]
pub struct SubjectRef {
    /// `subjectCanonical` — an identity, resolved through the canonical index.
    pub canonical: Option<String>,
    /// `subjectReference` — a location, read directly.
    pub reference: Option<String>,
    /// `subjectResource` — the artifact itself, supplied inline.
    pub resource: Option<Value>,
}

impl SubjectRef {
    /// Number of naming parameters supplied. Exactly one is valid.
    fn count(&self) -> usize {
        self.canonical.is_some() as usize
            + self.reference.is_some() as usize
            + self.resource.is_some() as usize
    }

    /// Enforces the spec's exactly-one-of rule, naming the offending parameters
    /// in the error so the client can see what it sent.
    pub fn validate(&self, operation: &str) -> Result<(), RestError> {
        match self.count() {
            1 => Ok(()),
            0 => Err(RestError::BadRequest {
                message: format!(
                    "{operation} requires a subject: supply exactly one of \
                     'subjectCanonical', 'subjectReference' or 'subjectResource'"
                ),
            }),
            _ => {
                let mut supplied = Vec::new();
                if self.canonical.is_some() {
                    supplied.push("subjectCanonical");
                }
                if self.reference.is_some() {
                    supplied.push("subjectReference");
                }
                if self.resource.is_some() {
                    supplied.push("subjectResource");
                }
                Err(RestError::BadRequest {
                    message: format!(
                        "{operation} accepts exactly one subject parameter, but {} were supplied ({}). \
                         'subjectCanonical' names an identity, 'subjectReference' a location, and \
                         'subjectResource' carries the artifact inline",
                        supplied.len(),
                        supplied.join(", ")
                    ),
                })
            }
        }
    }
}

/// Classifies an already-fetched artifact.
///
/// A ViewDefinition names its own type. A Library is a SQLQuery or a SQLView
/// depending on the `LibraryTypesCodes` code in `Library.type`, which both
/// profiles fix. Anything else is not a subject this operation can act on.
pub fn classify_subject(resource: &Value) -> Result<SubjectKind, RestError> {
    match resource.get("resourceType").and_then(|v| v.as_str()) {
        Some("ViewDefinition") => Ok(SubjectKind::ViewDefinition),
        Some("Library") => {
            let is_sql_view = resource
                .get("type")
                .and_then(|t| t.get("coding"))
                .and_then(|c| c.as_array())
                .map(|codings| {
                    codings.iter().any(|c| {
                        c.get("code").and_then(|v| v.as_str()) == Some(LIBRARY_TYPE_SQL_VIEW)
                    })
                })
                .unwrap_or(false);
            // A Library that declares neither code is treated as a SQLQuery so
            // the SQLQuery parser reports the precise profile violation, rather
            // than this function returning a vaguer "not a subject" error.
            Ok(if is_sql_view {
                SubjectKind::SqlView
            } else {
                SubjectKind::SqlQuery
            })
        }
        other => Err(RestError::BadRequest {
            message: format!(
                "a subject must be a ViewDefinition, a SQLQuery Library or a SQLView Library, \
                 got resourceType='{}'",
                other.unwrap_or("<absent>")
            ),
        }),
    }
}

/// Resolves a named subject into the artifact itself.
///
/// `subjectResource` is used as supplied. `subjectCanonical` and
/// `subjectReference` are looked up in storage; because a subject may be either
/// a ViewDefinition or a Library, and neither URL form need say which, both
/// types are tried. A subject that resolves in neither is `404 Not Found`.
pub async fn resolve_subject<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    subject: &SubjectRef,
    operation: &str,
) -> Result<ResolvedSubject, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    subject.validate(operation)?;

    if let Some(inline) = &subject.resource {
        let kind = classify_subject(inline)?;
        return Ok(ResolvedSubject {
            kind,
            resource: inline.clone(),
        });
    }

    let (url, param) = match (&subject.canonical, &subject.reference) {
        (Some(c), _) => (c.as_str(), "subjectCanonical"),
        (_, Some(r)) => (r.as_str(), "subjectReference"),
        _ => unreachable!("validate() guarantees one naming parameter is present"),
    };

    let resource = resolve_subject_url(state, tenant, url, param).await?;
    let kind = classify_subject(&resource)?;
    Ok(ResolvedSubject { kind, resource })
}

/// Looks a subject up by URL, trying whichever resource types the URL could
/// name.
///
/// A relative reference states its own type (`ViewDefinition/x`, `Library/y`),
/// so only that type is tried. A canonical URL states nothing, so both are
/// tried; ViewDefinition first, since it is the more common subject.
async fn resolve_subject_url<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    url: &str,
    param: &str,
) -> Result<Value, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    let trimmed = url.trim();
    let candidate_types: &[&str] = if trimmed.starts_with("ViewDefinition/") {
        &["ViewDefinition"]
    } else if trimmed.starts_with("Library/") {
        &["Library"]
    } else {
        &["ViewDefinition", "Library"]
    };

    let mut last_internal: Option<RestError> = None;
    for resource_type in candidate_types {
        match resolve_resource_canonical_or_relative(state, tenant, resource_type, trimmed).await {
            Ok(resource) => return Ok(resource),
            // Not this type — keep trying the others.
            Err(RestError::NotFound { .. }) => continue,
            // A malformed reference or a storage failure is the same whichever
            // type we ask for, so stop rather than repeating it.
            Err(other) => {
                last_internal = Some(other);
                break;
            }
        }
    }

    if let Some(err) = last_internal {
        return Err(err);
    }
    // `RestError::NotFound` renders as "Resource {type}/{id} not found", so the
    // id carries the URL alone; naming the parameter goes in `resource_type`'s
    // place would read as a path. Keep both legible.
    Err(RestError::NotFound {
        resource_type: format!("{param} subject ({})", candidate_types.join(" or ")),
        id: trimmed.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn library(code: &str) -> Value {
        json!({
            "resourceType": "Library",
            "type": {"coding": [{
                "system": helios_sof::canonical::LIBRARY_TYPES_CODE_SYSTEM,
                "code": code
            }]}
        })
    }

    #[test]
    fn classifies_view_definition() {
        let vd = json!({"resourceType": "ViewDefinition", "resource": "Patient"});
        assert_eq!(classify_subject(&vd).unwrap(), SubjectKind::ViewDefinition);
    }

    #[test]
    fn classifies_sql_query_and_sql_view_libraries() {
        assert_eq!(
            classify_subject(&library("sql-query")).unwrap(),
            SubjectKind::SqlQuery
        );
        assert_eq!(
            classify_subject(&library("sql-view")).unwrap(),
            SubjectKind::SqlView
        );
    }

    #[test]
    fn library_without_a_type_code_falls_through_to_sql_query() {
        // The SQLQuery parser reports the precise profile violation; this
        // function only needs to route it there.
        let bare = json!({"resourceType": "Library"});
        assert_eq!(classify_subject(&bare).unwrap(), SubjectKind::SqlQuery);
    }

    #[test]
    fn a_patient_is_not_a_subject() {
        let err = classify_subject(&json!({"resourceType": "Patient"})).unwrap_err();
        assert!(matches!(err, RestError::BadRequest { .. }));
    }

    #[test]
    fn exactly_one_naming_parameter_is_required() {
        let none = SubjectRef::default();
        assert!(none.validate("$sql-run").is_err());

        let one = SubjectRef {
            canonical: Some("http://example.org/vd".into()),
            ..Default::default()
        };
        assert!(one.validate("$sql-run").is_ok());
    }

    #[test]
    fn two_naming_parameters_are_rejected_and_both_are_named() {
        let two = SubjectRef {
            canonical: Some("http://example.org/vd".into()),
            reference: Some("ViewDefinition/x".into()),
            ..Default::default()
        };
        let err = two.validate("$sql-run").unwrap_err();
        let RestError::BadRequest { message } = err else {
            panic!("expected 400");
        };
        assert!(message.contains("subjectCanonical"), "{message}");
        assert!(message.contains("subjectReference"), "{message}");
    }

    #[test]
    fn view_definition_declares_no_parameters() {
        assert!(!SubjectKind::ViewDefinition.accepts_parameters());
        assert!(SubjectKind::SqlQuery.accepts_parameters());
        assert!(SubjectKind::SqlView.accepts_parameters());
    }

    #[test]
    fn subject_kinds_map_to_their_storage_type() {
        assert_eq!(
            SubjectKind::ViewDefinition.resource_type(),
            "ViewDefinition"
        );
        assert_eq!(SubjectKind::SqlQuery.resource_type(), "Library");
        assert_eq!(SubjectKind::SqlView.resource_type(), "Library");
    }
}
