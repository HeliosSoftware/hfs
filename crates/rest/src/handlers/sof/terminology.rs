//! ValueSet and ConceptMap dependencies of SQL queries (#1824, SQL on FHIR
//! "Terminology in SQL", HL7/sql-on-fhir#407).
//!
//! A SQLQuery or SQLView Library may name a `ValueSet` or `ConceptMap` in a
//! `relatedArtifact[type=depends-on]`. Its SQL then selects from the
//! dependency's `label` as a table with the spec's fixed columns (see
//! [`helios_sof::sqlquery::terminology`]).
//!
//! **Write path.** When such a Library is created or updated (`POST`/`PUT`),
//! every dependency that is not a stored ViewDefinition or Library is looked up
//! on the terminology server (`HFS_TERMINOLOGY_SERVER`): the server expands a
//! value set (paging through `$expand`) or returns a concept map, and the
//! result is flattened into rows here. The terminology server does the
//! expansion; HFS never expands a value set itself. The rows are persisted in
//! the primary backend by its [`SofTerminologyStore`], one table per tenant and
//! canonical, shared by every Library that depends on it. A dependency that is
//! terminology but cannot be materialized fails the write with `422` before
//! anything is stored. When the last Library referencing a table is deleted,
//! or rewritten without the dependency, the table is dropped.
//!
//! **Read path.** `$sql-run` and `$sql-export` load the persisted rows and copy
//! them into the request's in-memory SQL engine under the label (see
//! [`super::graph`]). A dependency with no persisted table (an inline
//! `subjectResource` Library, which was never written) is expanded through the
//! terminology server for that request only.
//!
//! Only backends that provide a [`SofTerminologyStore`] (SQLite, and
//! composites whose primary is SQLite) serve these dependencies; elsewhere a
//! terminology dependency is `501`.
//!
//! [`SofTerminologyStore`]: helios_persistence::core::sof_terminology::SofTerminologyStore

use helios_persistence::core::ResourceStorage;
use helios_persistence::core::search::SearchProvider;
use helios_persistence::tenant::TenantContext;
use helios_sof::sqlquery::library::{LIBRARY_TYPE_CODE, LIBRARY_TYPE_CODE_SQL_VIEW};
use helios_sof::sqlquery::terminology::{ValueSetRows, concept_map_relation};
use helios_sof::sqlquery::{TerminologyFlattenError, TerminologyRelation};
use serde_json::Value;
use tracing::{debug, error};

use super::graph::{ArtifactFetcher, StorageArtifactFetcher};
use super::references::resolve_resource_canonical_or_relative;
use crate::error::RestError;
use crate::state::AppState;
use crate::terminology::{TerminologyError, TerminologyServiceClient};

/// `$expand` page size requested from the terminology server.
const EXPAND_PAGE_SIZE: usize = 1000;

/// The terminology dependencies a Library write will persist, computed before
/// the write so a failure rejects it with nothing stored.
pub(crate) struct PreparedDependencies(Vec<(String, TerminologyRelation)>);

/// Whether `resource` is a Library typed as a SQLQuery or SQLView.
fn is_sql_library(resource: &Value) -> bool {
    resource.get("resourceType").and_then(Value::as_str) == Some("Library")
        && resource
            .get("type")
            .and_then(|t| t.get("coding"))
            .and_then(Value::as_array)
            .is_some_and(|codings| {
                codings.iter().any(|c| {
                    matches!(
                        c.get("code").and_then(Value::as_str),
                        Some(LIBRARY_TYPE_CODE | LIBRARY_TYPE_CODE_SQL_VIEW)
                    )
                })
            })
}

/// Whether this deployment materializes terminology dependencies: SQL on FHIR
/// is enabled and the backend has a terminology store.
fn materializes<S: ResourceStorage>(state: &AppState<S>) -> bool {
    state.sof_runner().is_some() && state.storage().sof_terminology_store().is_some()
}

/// Write path, before storage: expands every terminology dependency of a
/// SQLQuery/SQLView `Library` through the terminology server.
///
/// Returns `None` when nothing is to be recorded (not a Library, or the
/// deployment does not materialize terminology). Any other Library — a non-SQL
/// one included — yields `Some`, possibly empty, so an update that drops the
/// last dependency releases what the previous version held.
pub(crate) async fn prepare_library_write<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    resource: &Value,
) -> Result<Option<PreparedDependencies>, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    if resource_type != "Library" || !materializes(state) {
        return Ok(None);
    }
    if !is_sql_library(resource) {
        return Ok(Some(PreparedDependencies(Vec::new())));
    }
    // A Library the SQL parser rejects has no usable dependencies; the write
    // itself is not this module's to refuse (`$sql-run` reports the problem).
    let Ok(library) = helios_sof::sqlquery::parse_sqlquery_library(resource) else {
        return Ok(Some(PreparedDependencies(Vec::new())));
    };

    let fetcher = StorageArtifactFetcher::new(state, tenant);
    let max_rows = state.config().sof_sqlquery_max_source_rows_per_vd;
    let mut prepared: Vec<(String, TerminologyRelation)> = Vec::new();
    for dep in &library.depends_on {
        if prepared.iter().any(|(url, _)| url == &dep.url) {
            continue;
        }
        // A stored ViewDefinition or SQLView Library is not terminology.
        match fetcher.fetch(&dep.url).await {
            Ok(_) => continue,
            Err(RestError::NotFound { .. }) => {}
            Err(other) => return Err(other),
        }
        let relation = match state.terminology_server_url() {
            Some(base) => {
                let client = TerminologyServiceClient::new(base.to_string());
                fetch_relation(&client, &dep.url, max_rows)
                    .await
                    .map_err(|e| e.into_rest(&dep.label, &dep.url))?
            }
            None => None,
        };
        match relation {
            Some(relation) => {
                debug!(
                    url = %dep.url,
                    kind = relation.kind.as_str(),
                    rows = relation.rows.len(),
                    "expanded SQL query terminology dependency"
                );
                prepared.push((dep.url.clone(), relation));
            }
            // Unknown to the terminology server (or there is none). If HFS
            // itself holds a ValueSet or ConceptMap at that URL it *is* a
            // terminology dependency that cannot be materialized: refuse the
            // write rather than store a Library that cannot run. Otherwise it
            // is simply unresolved, which `$sql-run` reports as before.
            None => {
                if let Some(kind) = stored_terminology_kind(state, tenant, &dep.url).await? {
                    return Err(RestError::UnprocessableEntity {
                        message: match state.terminology_server_url() {
                            Some(_) => format!(
                                "depends-on '{}' is a {kind} ({}), but the terminology server \
                                 has no {kind} at that URL to expand",
                                dep.label, dep.url
                            ),
                            None => format!(
                                "depends-on '{}' is a {kind} ({}), and SQL query terminology \
                                 dependencies are expanded by the terminology server, which is \
                                 not configured (HFS_TERMINOLOGY_SERVER)",
                                dep.label, dep.url
                            ),
                        },
                    });
                }
            }
        }
    }
    Ok(Some(PreparedDependencies(prepared)))
}

/// Write path, after storage: records `prepared` as the complete terminology
/// dependencies of Library `library_id`, refreshing their tables and dropping
/// any table no Library references any more.
///
/// The Library is already stored, so a failure here does not fail the request:
/// it is logged, and `$sql-run` still serves the Library by expanding a
/// missing table through the terminology server on demand.
pub(crate) async fn commit_library_write<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    library_id: &str,
    prepared: Option<PreparedDependencies>,
) where
    S: ResourceStorage,
{
    let Some(PreparedDependencies(relations)) = prepared else {
        return;
    };
    let Some(store) = state.storage().sof_terminology_store() else {
        return;
    };
    if let Err(e) = store
        .set_library_dependencies(tenant, library_id, relations)
        .await
    {
        error!(
            library = %library_id,
            error = %e,
            "failed to persist SQL query terminology tables"
        );
    }
}

/// Write path, after a `Library` delete: releases the Library's references,
/// dropping every table it was the last to reference.
pub(crate) async fn release_library<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    resource_type: &str,
    library_id: &str,
) where
    S: ResourceStorage,
{
    if resource_type != "Library" {
        return;
    }
    let prepared = materializes(state).then(|| PreparedDependencies(Vec::new()));
    commit_library_write(state, tenant, library_id, prepared).await;
}

/// Read path: the relation for a dependency `url` that is not a stored
/// ViewDefinition or Library. The persisted table wins; failing that, the
/// terminology server expands it for this request. `Ok(None)` means `url` is
/// not terminology this server can find.
pub(crate) async fn resolve_relation<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    url: &str,
) -> Result<Option<TerminologyRelation>, RestError>
where
    S: ResourceStorage + SearchProvider + Send + Sync + 'static,
{
    let store = state.storage().sof_terminology_store();
    if let Some(store) = &store
        && let Some(relation) = store.load(tenant, url).await?
    {
        return Ok(Some(relation));
    }
    let relation = match state.terminology_server_url() {
        Some(base) => {
            let client = TerminologyServiceClient::new(base.to_string());
            fetch_relation(
                &client,
                url,
                state.config().sof_sqlquery_max_source_rows_per_vd,
            )
            .await
            .map_err(|e| e.into_rest(url, url))?
        }
        None => None,
    };
    if store.is_none() {
        let kind = match &relation {
            Some(relation) => Some(relation.kind.as_str()),
            None => stored_terminology_kind(state, tenant, url).await?,
        };
        if let Some(kind) = kind {
            return Err(RestError::NotImplemented {
                feature: format!(
                    "SQL query dependency '{url}' is a {kind}; ValueSet and ConceptMap \
                     dependencies are only supported on the SQLite storage backend"
                ),
            });
        }
    }
    Ok(relation)
}

/// `Some("ValueSet")` / `Some("ConceptMap")` when HFS storage holds that
/// resource at canonical `url`.
async fn stored_terminology_kind<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    url: &str,
) -> Result<Option<&'static str>, RestError>
where
    S: SearchProvider + Send + Sync + 'static,
{
    for kind in ["ValueSet", "ConceptMap"] {
        match resolve_resource_canonical_or_relative(state, tenant, kind, url).await {
            Ok(_) => return Ok(Some(kind)),
            Err(RestError::NotFound { .. }) => {}
            Err(other) => return Err(other),
        }
    }
    Ok(None)
}

/// Why a terminology dependency could not be materialized.
#[derive(Debug)]
enum FetchError {
    Server(TerminologyError),
    Flatten(TerminologyFlattenError),
    TooManyRows { max: usize },
}

impl FetchError {
    fn into_rest(self, label: &str, url: &str) -> RestError {
        let message = match self {
            Self::Server(e) => format!(
                "the terminology server could not resolve depends-on '{label}' ({url}): {e}"
            ),
            Self::Flatten(e) => format!("depends-on '{label}' ({url}): {e}"),
            Self::TooManyRows { max } => format!(
                "depends-on '{label}' ({url}) has more than {max} rows; raise \
                 HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD to use it"
            ),
        };
        RestError::UnprocessableEntity { message }
    }
}

/// Asks the terminology server for `url`: first as a value set to expand,
/// then as a concept map. `Ok(None)` when it knows neither.
async fn fetch_relation(
    client: &TerminologyServiceClient,
    url: &str,
    max_rows: usize,
) -> Result<Option<TerminologyRelation>, FetchError> {
    if let Some(relation) = expand_value_set(client, url, max_rows).await? {
        return Ok(Some(relation));
    }
    let Some(concept_map) = client
        .find_concept_map(url)
        .await
        .map_err(FetchError::Server)?
    else {
        return Ok(None);
    };
    let relation = concept_map_relation(&concept_map).map_err(FetchError::Flatten)?;
    if relation.rows.len() > max_rows {
        return Err(FetchError::TooManyRows { max: max_rows });
    }
    Ok(Some(relation))
}

/// Pages through `$expand` until the expansion's `total` is reached (or, with
/// no `total`, until a short page). `Ok(None)` when the server has no value
/// set at `url`.
async fn expand_value_set(
    client: &TerminologyServiceClient,
    url: &str,
    max_rows: usize,
) -> Result<Option<TerminologyRelation>, FetchError> {
    let mut rows = ValueSetRows::new();
    let mut offset = 0usize;
    loop {
        let Some(page) = client
            .expand_value_set_page(url, offset, EXPAND_PAGE_SIZE)
            .await
            .map_err(FetchError::Server)?
        else {
            if offset == 0 {
                return Ok(None);
            }
            break;
        };
        let returned = rows.add_page(&page).map_err(FetchError::Flatten)?;
        if rows.len() > max_rows {
            return Err(FetchError::TooManyRows { max: max_rows });
        }
        offset += returned;
        let total = page
            .get("expansion")
            .and_then(|e| e.get("total"))
            .and_then(Value::as_u64)
            .map(|t| t as usize);
        let done = match total {
            Some(total) => offset >= total,
            None => returned < EXPAND_PAGE_SIZE,
        };
        // `returned == 0` also stops a server that ignores `offset` and keeps
        // reporting a larger total.
        if done || returned == 0 {
            break;
        }
        // A server that ignores `offset` would otherwise loop forever; the
        // entries read are bounded by the row cap with headroom for abstract
        // and duplicate entries.
        if rows.entries() > max_rows.saturating_mul(2).max(EXPAND_PAGE_SIZE) {
            return Err(FetchError::TooManyRows { max: max_rows });
        }
    }
    Ok(Some(rows.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_sql_typed_libraries_are_sql_libraries() {
        let lib =
            |code: &str| json!({"resourceType": "Library", "type": {"coding": [{"code": code}]}});
        assert!(is_sql_library(&lib("sql-query")));
        assert!(is_sql_library(&lib("sql-view")));
        assert!(!is_sql_library(&lib("logic-library")));
        assert!(!is_sql_library(&json!({"resourceType": "ValueSet"})));
    }

    #[tokio::test]
    async fn expansion_pages_until_total() {
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let page = |offset: usize, codes: &[&str]| {
            Mock::given(method("POST"))
                .and(path("/ValueSet/$expand"))
                .and(body_partial_json(json!({"parameter": [
                    {"name": "url"}, {"name": "offset", "valueInteger": offset}
                ]})))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "resourceType": "ValueSet",
                    "expansion": {"total": 3, "contains": codes.iter()
                        .map(|c| json!({"system": "http://s", "code": c}))
                        .collect::<Vec<_>>()}
                })))
        };
        page(0, &["a", "b"]).mount(&server).await;
        page(2, &["c"]).mount(&server).await;

        let client = TerminologyServiceClient::new(server.uri());
        let relation = expand_value_set(&client, "http://vs", 100)
            .await
            .unwrap()
            .unwrap();
        let codes: Vec<_> = relation.rows.iter().map(|r| r["code"].clone()).collect();
        assert_eq!(codes, vec![json!("a"), json!("b"), json!("c")]);
    }

    #[tokio::test]
    async fn unknown_value_set_falls_through_to_concept_map() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$expand"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ConceptMap"))
            .and(query_param("url", "http://cm"))
            .and(query_param("version", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resourceType": "Bundle",
                "entry": [{"resource": {"resourceType": "ConceptMap", "group": [{
                    "source": "http://a", "target": "http://b",
                    "element": [{"code": "1", "target": [{"code": "X", "relationship": "equivalent"}]}]
                }]}}]
            })))
            .mount(&server)
            .await;

        let client = TerminologyServiceClient::new(server.uri());
        let relation = fetch_relation(&client, "http://cm|2", 100)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            relation.kind,
            helios_sof::sqlquery::TerminologyKind::ConceptMap
        );
        assert_eq!(relation.rows[0]["target_code"], "X");

        Mock::given(method("GET"))
            .and(path("/ConceptMap"))
            .and(query_param("url", "http://nothing"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resourceType": "Bundle", "entry": []
            })))
            .mount(&server)
            .await;
        assert!(
            fetch_relation(&client, "http://nothing", 100)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn expansion_over_the_row_cap_is_rejected() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/ValueSet/$expand"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resourceType": "ValueSet",
                "expansion": {"contains": [
                    {"system": "http://s", "code": "a"}, {"system": "http://s", "code": "b"}
                ]}
            })))
            .mount(&server)
            .await;
        let client = TerminologyServiceClient::new(server.uri());
        let err = expand_value_set(&client, "http://vs", 1).await.unwrap_err();
        assert!(matches!(err, FetchError::TooManyRows { max: 1 }));
    }
}
