//! The resource metadata operations (#2562): `$meta`, `$meta-add` and
//! `$meta-delete`.
//!
//! `$meta` answers the `Meta` of the current version or of a version named
//! by `_history/[vid]`. `$meta-add` and `$meta-delete` change the profiles,
//! tags and security labels of that version in place: its version id stays,
//! as the Meta.tag and Meta.security definitions allow ("can be updated
//! without changing the stated version"), and its `lastUpdated` moves to now,
//! as the Resource Metadata table requires for any operation that changes
//! meta. Both write only where storage serves
//! [`ResourceStorage::rewrite_meta`]; elsewhere they answer `501`.
//!
//! System- and type-level `$meta` (a summary of every label in use) are not
//! implemented; the OperationDefinition allows a `4xx` for them.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use helios_fhir::FhirVersion;
use helios_fhir_validator::{ErrorKind, Severity, ValidationError, ValidationOptions, Validator};
use helios_persistence::core::{
    MetaChange, MetaTarget, ResourceStorage, SearchProvider, VersionedStorage, WRITE_ATTEMPTS,
    WriteKind,
};
use helios_persistence::error::{ConcurrencyError, StorageError};
use helios_persistence::types::StoredResource;
use serde_json::{Map, Value, json};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirResource, FhirVersionExtractor, TenantExtractor};
use crate::middleware::content_type::{FhirContentType, FhirFormat, negotiate_format};
use crate::patient_compartment::PatientCompartment;
use crate::responses::format_resource_response_for_version;
use crate::state::AppState;

/// The id of this server's `$meta` OperationDefinition: instance and
/// history-version levels only.
pub const META_DEFINITION_ID: &str = "hfs-meta";

/// `GET|POST [base]/$meta` and `[base]/[type]/$meta`: not implemented.
pub async fn meta_unsupported_handler(_tenant: TenantExtractor) -> RestError {
    RestError::NotSupported {
        feature: "$meta at the system and type level is not supported; use \
                  [type]/[id]/$meta or [type]/[id]/_history/[vid]/$meta"
            .to_string(),
    }
}

/// `GET|POST [base]/[type]/[id]/$meta`: the current version's `Meta`.
pub async fn meta_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    compartment: PatientCompartment,
    req_headers: HeaderMap,
) -> RestResult<Response>
where
    S: ResourceStorage + VersionedStorage + SearchProvider + Send + Sync,
{
    let stored = read_current(&state, &tenant, &resource_type, &id).await?;
    compartment.assert_member(
        || state.storage().search_param_registry(tenant.context()),
        stored.fhir_version(),
        stored.content(),
        "read",
    )?;
    meta_response(&stored, &req_headers, &version)
}

/// `GET|POST [base]/[type]/[id]/_history/[vid]/$meta`: that version's `Meta`.
pub async fn meta_version_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id, version_id)): Path<(String, String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    compartment: PatientCompartment,
    req_headers: HeaderMap,
) -> RestResult<Response>
where
    S: ResourceStorage + VersionedStorage + SearchProvider + Send + Sync,
{
    let stored = super::vread::execute_vread(
        &state,
        &tenant,
        &compartment,
        &resource_type,
        &id,
        &version_id,
    )
    .await?;
    meta_response(&stored, &req_headers, &version)
}

/// `POST [base]/[type]/[id]/$meta-add`.
#[allow(clippy::too_many_arguments)]
pub async fn meta_add_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    compartment: PatientCompartment,
    req_headers: HeaderMap,
    FhirResource(body): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + VersionedStorage + SearchProvider + Send + Sync,
{
    let request = MetaRequest::new(&tenant, &version, &resource_type, &id, None);
    request
        .run(&state, &compartment, &req_headers, &body, true)
        .await
}

/// `POST [base]/[type]/[id]/_history/[vid]/$meta-add`.
#[allow(clippy::too_many_arguments)]
pub async fn meta_add_version_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id, version_id)): Path<(String, String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    compartment: PatientCompartment,
    req_headers: HeaderMap,
    FhirResource(body): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + VersionedStorage + SearchProvider + Send + Sync,
{
    let request = MetaRequest::new(&tenant, &version, &resource_type, &id, Some(&version_id));
    request
        .run(&state, &compartment, &req_headers, &body, true)
        .await
}

/// `POST [base]/[type]/[id]/$meta-delete`.
#[allow(clippy::too_many_arguments)]
pub async fn meta_delete_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id)): Path<(String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    compartment: PatientCompartment,
    req_headers: HeaderMap,
    FhirResource(body): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + VersionedStorage + SearchProvider + Send + Sync,
{
    let request = MetaRequest::new(&tenant, &version, &resource_type, &id, None);
    request
        .run(&state, &compartment, &req_headers, &body, false)
        .await
}

/// `POST [base]/[type]/[id]/_history/[vid]/$meta-delete`.
#[allow(clippy::too_many_arguments)]
pub async fn meta_delete_version_handler<S>(
    State(state): State<AppState<S>>,
    Path((resource_type, id, version_id)): Path<(String, String, String)>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    compartment: PatientCompartment,
    req_headers: HeaderMap,
    FhirResource(body): FhirResource,
) -> RestResult<Response>
where
    S: ResourceStorage + VersionedStorage + SearchProvider + Send + Sync,
{
    let request = MetaRequest::new(&tenant, &version, &resource_type, &id, Some(&version_id));
    request
        .run(&state, &compartment, &req_headers, &body, false)
        .await
}

/// One `$meta-add` / `$meta-delete` request.
struct MetaRequest<'a> {
    tenant: &'a TenantExtractor,
    version: &'a FhirVersionExtractor,
    resource_type: &'a str,
    id: &'a str,
    version_id: Option<&'a str>,
}

impl<'a> MetaRequest<'a> {
    fn new(
        tenant: &'a TenantExtractor,
        version: &'a FhirVersionExtractor,
        resource_type: &'a str,
        id: &'a str,
        version_id: Option<&'a str>,
    ) -> Self {
        Self {
            tenant,
            version,
            resource_type,
            id,
            version_id,
        }
    }

    async fn run<S>(
        &self,
        state: &AppState<S>,
        compartment: &PatientCompartment,
        req_headers: &HeaderMap,
        body: &Value,
        adding: bool,
    ) -> RestResult<Response>
    where
        S: ResourceStorage + VersionedStorage + SearchProvider + Send + Sync,
    {
        let operation = if adding { "$meta-add" } else { "$meta-delete" };
        if !state.storage().supports_meta_rewrite() {
            return Err(RestError::NotImplemented {
                feature: format!(
                    "{operation} is not available on this storage configuration; tags and \
                     security labels can be removed with PATCH"
                ),
            });
        }
        // A response this build cannot write is refused before anything
        // changes.
        response_format(req_headers)?;
        // AuditEvent resources are immutable, as on every other write.
        if self.resource_type == "AuditEvent" {
            return Err(RestError::MethodNotAllowed {
                method: "POST".to_string(),
                resource_type: self.resource_type.to_string(),
            });
        }
        debug!(
            resource_type = %self.resource_type,
            id = %self.id,
            version_id = ?self.version_id,
            tenant = %self.tenant.tenant_id(),
            operation,
            "Processing metadata operation"
        );

        let rewrite = match self.version_id {
            Some(version_id) => {
                let target = super::vread::execute_vread(
                    state,
                    self.tenant,
                    compartment,
                    self.resource_type,
                    self.id,
                    version_id,
                )
                .await?;
                answerable_in(self.version, target.fhir_version())?;
                let change = self.change(body, adding, target.fhir_version())?;
                state
                    .storage()
                    .rewrite_meta(
                        self.tenant.context(),
                        self.resource_type,
                        self.id,
                        &MetaTarget::Historical {
                            version_id: version_id.to_string(),
                        },
                        &change,
                    )
                    .await?
            }
            None => {
                let mut attempt = 0;
                loop {
                    attempt += 1;
                    let current =
                        read_current(state, self.tenant, self.resource_type, self.id).await?;
                    compartment.assert_member(
                        || state.storage().search_param_registry(self.tenant.context()),
                        current.fhir_version(),
                        current.content(),
                        "update",
                    )?;
                    answerable_in(self.version, current.fhir_version())?;
                    let change = self.change(body, adding, current.fhir_version())?;
                    let target = MetaTarget::Current {
                        expected_version: current.version_id().to_string(),
                        expected_meta_seq: current.meta_seq(),
                    };
                    match state
                        .storage()
                        .rewrite_meta(
                            self.tenant.context(),
                            self.resource_type,
                            self.id,
                            &target,
                            &change,
                        )
                        .await
                    {
                        Err(StorageError::Concurrency(ConcurrencyError::VersionConflict {
                            ..
                        })) if attempt < WRITE_ATTEMPTS => continue,
                        rewrite => break rewrite?,
                    }
                }
            }
        };

        if rewrite.current_changed {
            super::write_event::report(
                state,
                self.tenant.context(),
                rewrite.resource.fhir_version(),
                self.resource_type,
                0,
                Some(super::write_event::stored_notice(
                    WriteKind::Update,
                    &rewrite.resource,
                )),
            );
        }
        meta_response(&rewrite.resource, req_headers, self.version)
    }
}

impl MetaRequest<'_> {
    /// The change, its `Meta` validated against the FHIR version of the
    /// resource it is applied to. Input that declares another version
    /// (`Content-Type: ...; fhirVersion=`) is refused: it could carry
    /// elements the resource's version does not define.
    fn change(&self, body: &Value, adding: bool, target: FhirVersion) -> RestResult<MetaChange> {
        if let Some(declared) = self.version.content_version()
            && declared != target
        {
            return Err(RestError::BadRequest {
                message: format!(
                    "the input is FHIR {} but {}/{} is FHIR {}",
                    declared.as_mime_param(),
                    self.resource_type,
                    self.id,
                    target.as_mime_param()
                ),
            });
        }
        let meta = validate_meta_parameters(body, target)?;
        Ok(if adding {
            MetaChange::Add(meta)
        } else {
            MetaChange::Delete(meta)
        })
    }
}

async fn read_current<S>(
    state: &AppState<S>,
    tenant: &TenantExtractor,
    resource_type: &str,
    id: &str,
) -> RestResult<StoredResource>
where
    S: ResourceStorage + Send + Sync,
{
    state
        .storage()
        .read(tenant.context(), resource_type, id)
        .await?
        .ok_or_else(|| RestError::NotFound {
            resource_type: resource_type.to_string(),
            id: id.to_string(),
        })
}

/// The operations' output: `Parameters` with one `return` of `valueMeta`,
/// the meta the resource itself would be read with.
/// The response is written in the stored resource's FHIR version, labelled
/// with it, and refused (`406`) when `Accept` asks for another version.
fn meta_response(
    stored: &StoredResource,
    req_headers: &HeaderMap,
    version: &FhirVersionExtractor,
) -> RestResult<Response> {
    answerable_in(version, stored.fhir_version())?;
    let format = response_format(req_headers)?;
    let content = stored.content_with_meta();
    let meta = content.get("meta").cloned().unwrap_or_else(|| json!({}));
    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [{"name": "return", "valueMeta": meta}]
    });
    let content_type = FhirContentType::with_version(format, stored.fhir_version());
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        content_type
            .to_header_value()
            .parse()
            .expect("a FHIR content type is a valid header value"),
    );
    format_resource_response_for_version(
        StatusCode::OK,
        headers,
        &parameters,
        format,
        stored.fhir_version(),
    )
}

/// `406` when `Accept` asks for another FHIR version than the resource's.
fn answerable_in(version: &FhirVersionExtractor, stored: FhirVersion) -> RestResult<()> {
    match version.accept_version() {
        Some(requested) if requested != stored => Err(RestError::NotAcceptable {
            message: format!(
                "Resource is FHIR {} but {} was requested",
                stored.as_mime_param(),
                requested.as_mime_param()
            ),
        }),
        _ => Ok(()),
    }
}

/// The negotiated response format, or `406` for XML in a build without it.
fn response_format(req_headers: &HeaderMap) -> RestResult<FhirFormat> {
    let format = negotiate_format(req_headers, None).format;
    #[cfg(not(feature = "xml"))]
    if format == FhirFormat::Xml {
        return Err(RestError::NotAcceptable {
            message: "XML format is not supported (xml feature not enabled)".to_string(),
        });
    }
    Ok(format)
}

const SERVER_MANAGED: [&str; 6] = [
    "versionId",
    "_versionId",
    "lastUpdated",
    "_lastUpdated",
    "source",
    "_source",
];

/// Checks a `$meta-add` / `$meta-delete` input and returns its `Meta`, ready
/// for [`MetaChange`].
///
/// 1. The wrapper: a `Parameters` with exactly one parameter, `meta`,
///    carrying `valueMeta` and nothing else.
/// 2. `profile` / `_profile` as one paired array: each an array, padded to
///    the same length, no slot empty on both sides.
/// 3. Structural validation of the whole `Parameters` against the core
///    pack of `fhir_version`: JSON types, primitive syntax, cardinality and
///    the `value[x]` names and shapes of every Extension.
/// 4. Unknown members inside `valueMeta` are dropped by the exact location
///    the validator reports (FHIR JSON lets a server ignore them); any other
///    issue, or an unknown member elsewhere, is a `400`.
/// 5. `versionId`, `lastUpdated` and `source` are dropped: the server sets
///    them.
/// 6. ext-1 on every Extension left: exactly one populated `value[x]` choice
///    (a value and its own `_value` companion are one choice) or nested
///    extensions, never both or neither.
fn validate_meta_parameters(body: &Value, fhir_version: FhirVersion) -> RestResult<Value> {
    let mut issues: Vec<ValidationError> = Vec::new();
    let issue = |kind: ErrorKind, path: &str, message: String| {
        ValidationError::new(kind, path.to_string(), message)
    };

    if body.get("resourceType").and_then(Value::as_str) != Some("Parameters") {
        return Err(input_error(vec![issue(
            ErrorKind::Type,
            "Parameters",
            "the input must be a Parameters resource".to_string(),
        )]));
    }
    let parameters = match body.get("parameter") {
        Some(Value::Array(parameters)) => parameters,
        _ => {
            return Err(input_error(vec![issue(
                ErrorKind::NotArray,
                "Parameters.parameter",
                "Parameters.parameter must be an array with the one `meta` parameter".to_string(),
            )]));
        }
    };
    if parameters.len() != 1 {
        return Err(input_error(vec![issue(
            ErrorKind::Max,
            "Parameters.parameter",
            format!(
                "exactly one parameter, `meta`, is expected; {} were sent",
                parameters.len()
            ),
        )]));
    }
    let parameter = parameters[0].as_object();
    let name = parameter
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str);
    if name != Some("meta") {
        return Err(input_error(vec![issue(
            ErrorKind::Required,
            "Parameters.parameter.0.name",
            "the parameter must be named `meta`".to_string(),
        )]));
    }
    let parameter = parameter.expect("named parameter is an object");
    for key in parameter.keys() {
        let other_value =
            (key.starts_with("value") && key != "valueMeta") || key.starts_with("_value");
        if other_value || key == "resource" || key == "part" {
            issues.push(issue(
                ErrorKind::Choice,
                &format!("Parameters.parameter.0.{key}"),
                format!("the `meta` parameter carries only valueMeta, not {key}"),
            ));
        }
    }
    if !parameter.get("valueMeta").is_some_and(Value::is_object) {
        issues.push(issue(
            ErrorKind::Required,
            "Parameters.parameter.0.valueMeta",
            "the `meta` parameter must carry a valueMeta object".to_string(),
        ));
    }
    if !issues.is_empty() {
        return Err(input_error(issues));
    }

    let mut input = body.clone();
    let meta = input["parameter"][0]["valueMeta"]
        .as_object_mut()
        .expect("valueMeta checked above");
    pair_profiles(meta, &mut issues);
    if !issues.is_empty() {
        return Err(input_error(issues));
    }

    let validator = Validator::new(helios_fhir_validator::packs::core_registry(fhir_version));
    let options = ValidationOptions {
        use_meta_profiles: false,
        ..Default::default()
    };
    let mut unknown: Vec<String> = Vec::new();
    for error in validator.validate_sync(&input, &options).errors {
        if error.severity != Severity::Error {
            continue;
        }
        if error.kind == ErrorKind::UnknownElement
            && error.pointer.starts_with("/parameter/0/valueMeta/")
        {
            unknown.push(error.pointer.clone());
        } else {
            issues.push(error);
        }
    }
    if !issues.is_empty() {
        return Err(input_error(issues));
    }

    // Deepest first: removing a member never moves another one's location.
    unknown.sort_by_key(|pointer| std::cmp::Reverse(pointer.matches('/').count()));
    for pointer in unknown {
        if !remove_at_pointer(&mut input, &pointer) {
            return Err(input_error(vec![issue(
                ErrorKind::UnknownElement,
                &pointer,
                format!("unknown element at {pointer} could not be removed"),
            )]));
        }
    }

    let Value::Object(mut meta) = input["parameter"][0]["valueMeta"].take() else {
        unreachable!("valueMeta checked above");
    };
    for key in SERVER_MANAGED {
        meta.remove(key);
    }
    let meta = Value::Object(meta);
    check_extensions(&meta, "Parameters.parameter.0.valueMeta", &mut issues);
    if !issues.is_empty() {
        return Err(input_error(issues));
    }
    Ok(meta)
}

fn input_error(issues: Vec<ValidationError>) -> RestError {
    RestError::MultiIssue {
        outcome: crate::validation::validation_outcome(&issues),
    }
}

/// Aligns `profile` and its `_profile` companion, the shorter padded with
/// the implied trailing `null`s, and rejects a slot with neither a value nor
/// a companion.
fn pair_profiles(meta: &mut Map<String, Value>, issues: &mut Vec<ValidationError>) {
    let mut lengths = [0usize; 2];
    for (index, key) in ["profile", "_profile"].into_iter().enumerate() {
        match meta.get(key) {
            None => {}
            Some(Value::Array(items)) => lengths[index] = items.len(),
            Some(_) => issues.push(ValidationError::new(
                ErrorKind::NotArray,
                format!("Parameters.parameter.0.valueMeta.{key}"),
                format!("{key} must be an array"),
            )),
        }
    }
    if !issues.is_empty() {
        return;
    }
    let len = lengths[0].max(lengths[1]);
    for key in ["profile", "_profile"] {
        if let Some(Value::Array(items)) = meta.get_mut(key) {
            items.resize(len, Value::Null);
        }
    }
    for slot in 0..len {
        let value = meta.get("profile").and_then(|p| p.get(slot));
        let companion = meta.get("_profile").and_then(|p| p.get(slot));
        if value.is_none_or(Value::is_null) && companion.is_none_or(Value::is_null) {
            issues.push(ValidationError::new(
                ErrorKind::Required,
                format!("Parameters.parameter.0.valueMeta.profile.{slot}"),
                "a profile slot needs a value or an extension".to_string(),
            ));
        }
    }
}

/// Removes the object member an RFC 6901 pointer names, resolving every
/// segment as an exact key or index. `false` when it does not resolve to an
/// object member.
fn remove_at_pointer(root: &mut Value, pointer: &str) -> bool {
    let segments: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    let Some((last, parents)) = segments.split_last() else {
        return false;
    };
    let mut node = root;
    for segment in parents {
        node = match node {
            Value::Object(object) => match object.get_mut(segment) {
                Some(child) => child,
                None => return false,
            },
            Value::Array(items) => {
                match segment.parse::<usize>().ok().and_then(|i| items.get_mut(i)) {
                    Some(child) => child,
                    None => return false,
                }
            }
            _ => return false,
        };
    }
    match node {
        Value::Object(object) => object.remove(last).is_some(),
        _ => false,
    }
}

/// ext-1 on every Extension under `value`.
fn check_extensions(value: &Value, path: &str, issues: &mut Vec<ValidationError>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                if matches!(key.as_str(), "extension" | "modifierExtension")
                    && let Value::Array(extensions) = child
                {
                    for (index, extension) in extensions.iter().enumerate() {
                        check_extension(extension, &format!("{child_path}.{index}"), issues);
                    }
                }
                check_extensions(child, &child_path, issues);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                check_extensions(item, &format!("{path}.{index}"), issues);
            }
        }
        _ => {}
    }
}

/// ext-1 for one Extension: one populated `value[x]` choice or nested
/// extensions. A choice is populated by a non-null value or by a companion
/// with an id or extensions; `valueX` and `_valueX` are the same choice.
fn check_extension(extension: &Value, path: &str, issues: &mut Vec<ValidationError>) {
    let Some(object) = extension.as_object() else {
        return;
    };
    let mut choices: Vec<&str> = Vec::new();
    for (key, member) in object {
        let (base, populated) = if let Some(base) = key.strip_prefix("_value") {
            (base, companion_is_populated(member))
        } else if let Some(base) = key.strip_prefix("value") {
            (base, !member.is_null())
        } else {
            continue;
        };
        if populated && base.starts_with(char::is_uppercase) && !choices.contains(&base) {
            choices.push(base);
        }
    }
    let nested = object
        .get("extension")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty());
    if choices.len() > 1 {
        issues.push(ValidationError::new(
            ErrorKind::Choice,
            path.to_string(),
            format!(
                "an extension has one value[x]; this one has {}",
                choices
                    .iter()
                    .map(|c| format!("value{c}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    } else if nested == (choices.len() == 1) {
        issues.push(ValidationError::new(
            ErrorKind::Choice,
            path.to_string(),
            "ext-1: an extension must have either extensions or a value[x], not both".to_string(),
        ));
    }
}

fn companion_is_populated(companion: &Value) -> bool {
    let Some(object) = companion.as_object() else {
        return false;
    };
    object
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
        || object
            .get("extension")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters(meta: Value) -> Value {
        json!({"resourceType": "Parameters", "parameter": [{"name": "meta", "valueMeta": meta}]})
    }

    fn check(meta: Value) -> RestResult<Value> {
        validate_meta_parameters(&parameters(meta), FhirVersion::R4)
    }

    fn rejected(meta: Value) {
        let result = check(meta.clone());
        assert!(
            matches!(result, Err(RestError::MultiIssue { .. })),
            "{meta} must be rejected, got {result:?}"
        );
    }

    fn ext(members: Value) -> Value {
        let mut extension = json!({"url": "http://example.org/e"});
        extension
            .as_object_mut()
            .unwrap()
            .extend(members.as_object().unwrap().clone());
        json!({"tag": [{"system": "s", "code": "c", "extension": [extension]}]})
    }

    #[test]
    fn the_wrapper_is_checked() {
        let tag = json!({"tag": [{"code": "a"}]});
        for body in [
            json!({"resourceType": "Patient"}),
            json!({"resourceType": "Parameters", "parameter": {"name": "meta", "valueMeta": tag}}),
            json!({"resourceType": "Parameters", "parameter": [
                {"name": "meta", "valueMeta": tag}, {"name": "meta", "valueMeta": tag}
            ]}),
            json!({"resourceType": "Parameters", "parameter": [{"name": "meta", "valueCoding": {"code": "a"}}]}),
            json!({"resourceType": "Parameters", "parameter": [{"name": "meta", "valueMeta": tag, "part": []}]}),
            json!({"resourceType": "Parameters", "parameter": [{
                "name": "meta", "valueMeta": tag,
                "_valueString": {"extension": [{"url": "http://example.org/e", "valueCode": "x"}]}
            }]}),
            json!({"resourceType": "Parameters", "parameter": [{"name": "other", "valueMeta": tag}]}),
        ] {
            assert!(
                validate_meta_parameters(&body, FhirVersion::R4).is_err(),
                "{body} must be rejected"
            );
        }
    }

    #[test]
    fn primitive_shapes_and_syntax_are_checked() {
        rejected(json!({"tag": [{"system": 1, "code": "a"}]}));
        rejected(json!({"tag": [{"code": {"value": "x"}}]}));
        rejected(json!({"tag": [{"code": "a", "userSelected": "true"}]}));
        rejected(json!({"tag": {"code": "a"}}));
        rejected(json!({"tag": [{"code": ""}]}));
        rejected(json!({"tag": [{"code": " a"}]}));
        rejected(json!({"profile": ["http://example.org/a b"]}));
    }

    #[test]
    fn a_coding_needs_neither_system_nor_code() {
        assert_eq!(
            check(json!({"tag": [{"display": "x"}]})).unwrap(),
            json!({"tag": [{"display": "x"}]})
        );
    }

    #[test]
    fn server_managed_fields_are_dropped() {
        assert_eq!(
            check(json!({
                "versionId": "9", "lastUpdated": "2020-01-01T00:00:00Z",
                "source": "http://x", "tag": [{"code": "a"}]
            }))
            .unwrap(),
            json!({"tag": [{"code": "a"}]})
        );
    }

    #[test]
    fn unknown_members_are_removed_by_exact_location() {
        let meta = check(json!({"tag": [{"code": "keep"}], "tag.0.code": false})).unwrap();
        assert_eq!(meta, json!({"tag": [{"code": "keep"}]}));

        let meta = check(json!({
            "tag": [{"system": "s", "code": "c", "a/b~c": 1}],
            "0": true
        }))
        .unwrap();
        assert_eq!(meta, json!({"tag": [{"system": "s", "code": "c"}]}));
    }

    #[test]
    fn unknown_members_outside_value_meta_are_rejected() {
        let body = json!({"resourceType": "Parameters", "bogus": 1, "parameter": [
            {"name": "meta", "valueMeta": {"tag": [{"code": "a"}]}}
        ]});
        assert!(validate_meta_parameters(&body, FhirVersion::R4).is_err());
    }

    #[test]
    fn profile_pairs_are_aligned() {
        let companion =
            json!({"extension": [{"url": "http://example.org/n", "valueString": "reviewed"}]});
        let meta = check(json!({
            "profile": ["http://example.org/StructureDefinition/p"],
            "_profile": [companion]
        }))
        .unwrap();
        assert_eq!(meta["_profile"][0], companion);

        let meta =
            check(json!({"profile": ["http://p", "http://q"], "_profile": [companion]})).unwrap();
        assert_eq!(meta["_profile"], json!([companion, null]));

        rejected(json!({"profile": ["http://p", null], "_profile": [null, null]}));
        rejected(json!({"profile": "http://p"}));
    }

    #[test]
    fn extensions_carry_exactly_one_choice() {
        assert!(check(ext(json!({"valueString": "a"}))).is_ok());
        assert!(
            check(ext(
                json!({"extension": [{"url": "http://n", "valueCode": "x"}]})
            ))
            .is_ok()
        );
        assert!(
            check(ext(
                json!({"_valueString": {"extension": [{"url": "http://n", "valueCode": "x"}]}})
            ))
            .is_ok()
        );
        assert!(
            check(ext(json!({
                "valueString": "a",
                "_valueString": {"extension": [{"url": "http://n", "valueCode": "x"}]}
            })))
            .is_ok()
        );

        rejected(ext(json!({})));
        rejected(ext(json!({
            "valueString": "a",
            "extension": [{"url": "http://n", "valueCode": "x"}]
        })));
        rejected(ext(json!({"valueBoolean": "true"})));
        rejected(ext(json!({"valueBogus": "x"})));
        rejected(ext(json!({
            "valueBoolean": true,
            "_valueString": {"extension": [{"url": "http://n", "valueCode": "x"}]}
        })));
        rejected(ext(json!({
            "valueString": "a",
            "_valueBoolean": {"extension": [{"url": "http://n", "valueCode": "x"}]}
        })));
        rejected(ext(json!({"valueString": null})));
        rejected(ext(json!({"_valueString": null})));
        rejected(ext(json!({"_valueString": {}})));
        rejected(ext(json!({"extension": []})));
        // An unknown member is dropped first; the extension is then empty.
        rejected(ext(json!({"bogus": 1})));
    }

    #[cfg(feature = "R5")]
    #[test]
    fn r5_integer64_extension_values_are_strings() {
        let meta = ext(json!({"valueInteger64": "1"}));
        assert!(validate_meta_parameters(&parameters(meta), FhirVersion::R5).is_ok());
        let meta = ext(json!({"valueInteger64": 1}));
        assert!(validate_meta_parameters(&parameters(meta), FhirVersion::R5).is_err());
    }
}
