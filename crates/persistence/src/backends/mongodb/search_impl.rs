//! Search and conditional-operation implementation for MongoDB backend.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use helios_fhir::FhirVersion;
use mongodb::{
    Cursor,
    bson::{self, Bson, DateTime as BsonDateTime, Document, doc},
};
use regex::escape as regex_escape;
use serde_json::Value;

use crate::core::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalStorage, ConditionalUpdateResult,
    IncludeProvider, ResourceStorage, RevincludeProvider, SearchProvider, SearchResult,
};
use crate::error::{BackendError, QueryErrorExt, SearchError, StorageError, StorageResult};
use crate::search::{DatePredicate, FhirDateValue, RangeCondition, StorageResolution};
use crate::tenant::TenantContext;
use crate::types::{
    CompartmentMembership, CursorDirection, CursorValue, IncludeDirective, IncludeType, Page,
    PageCursor, PageInfo, SearchModifier, SearchParamType, SearchParameter, SearchPrefix,
    SearchQuery, SearchValue, StoredResource, strip_reference_version,
};

use super::MongoBackend;
use super::search_index_catalog::{
    COMPOSITE_SLOT_PROBE_INDEX, CONTAINED_COMPOSITE_SLOT_PROBE_INDEX,
};

/// Candidate ids per `$in` chunk when the parameter-sort aggregation is
/// bounded to a matched set (#1040). Keeps each aggregate command well under
/// the 16 MB BSON limit (10 000 UUID-length ids ≈ 0.5 MB).
const SORT_ID_CHUNK: usize = 10_000;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message,
        source: None,
    })
}

fn serialization_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::SerializationError { message })
}

fn bson_to_chrono(dt: &BsonDateTime) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap_or_else(Utc::now)
}

fn chrono_to_bson(dt: DateTime<Utc>) -> BsonDateTime {
    BsonDateTime::from_millis(dt.timestamp_millis())
}

/// The `search_index` field a date row stores the end of its range in
/// (#1391); `value_date` holds the start.
const VALUE_DATE_END: &str = "value_date_end";

/// Reads a date search value with the grammar every backend shares.
///
/// A value that is not a date is an error here, never a filter. The search
/// gate (`validate_date_values`) reports it first on every ordinary path;
/// this is what the in-transaction conditional paths, which build filters
/// without passing the gate, fall back on.
fn parse_date_search_value(value: &SearchValue, param: &str) -> StorageResult<FhirDateValue> {
    FhirDateValue::parse(&value.value).map_err(|error| {
        StorageError::Search(SearchError::InvalidDateValue {
            param: param.to_string(),
            value: value.value.clone(),
            reason: error.to_string(),
        })
    })
}

/// The date filter document for one search value against a stored *point*
/// in `field` (#519): `_lastUpdated` on the resources collection, and a date
/// component of a composite parameter. A free function so the semantics are
/// unit-testable without a live MongoDB.
///
/// The value is read by [`FhirDateValue`], the grammar and precision range
/// every backend shares. A search value names a *range*, never an instant:
/// `1995` is the whole year, `1995-10-02T08:30` the whole minute,
/// `…T08:30:00Z` the whole second. `eq` on a value with a time used to be
/// `$eq` on its first instant, so a second-precision search missed a stored
/// `…:00.123` that SQLite and Elasticsearch found (#1297). BSON dates hold
/// milliseconds, so the range is clamped to that: a microsecond search value
/// still finds the millisecond-truncated date stored for it.
///
/// `ap` accepts a point inside [`FhirDateValue::approx_window`], the window
/// every backend shares (#1391; it used to be ±12h around the start here).
fn build_date_filter_doc(value: &SearchValue, param: &str, field: &str) -> StorageResult<Document> {
    let parsed = parse_date_search_value(value, param)?;

    let Some(predicate) = parsed.predicate(value.prefix, StorageResolution::Millis) else {
        let (low, high) = parsed.approx_window(StorageResolution::Millis);
        return Ok(doc! { field: { "$gte": chrono_to_bson(low), "$lt": chrono_to_bson(high) } });
    };

    Ok(match predicate {
        DatePredicate::Within { ge, lt } => {
            doc! { field: { "$gte": chrono_to_bson(ge), "$lt": chrono_to_bson(lt) } }
        }
        DatePredicate::Outside { lt, ge } => doc! {
            "$or": [
                { field: { "$lt": chrono_to_bson(lt) } },
                { field: { "$gte": chrono_to_bson(ge) } },
            ]
        },
        DatePredicate::AtOrAfter(bound) => doc! { field: { "$gte": chrono_to_bson(bound) } },
        DatePredicate::Before(bound) => doc! { field: { "$lt": chrono_to_bson(bound) } },
    })
}

/// The date filter document for one search value against the *range* a
/// `search_index` row stores, `[value_date, value_date_end)` (#1391): a
/// `Period` is one row, not two unrelated points, and every prefix follows
/// the FHIR rule for a range target ([`FhirDateValue::range_predicate`]).
///
/// A row indexed before #1391 has no `value_date_end` and fails every
/// condition on the end until it is reindexed (`$reindex`); treating the
/// missing end as open would make it match `gt` for any date.
fn build_date_range_filter_doc(value: &SearchValue, param: &str) -> StorageResult<Document> {
    let parsed = parse_date_search_value(value, param)?;
    let predicate = parsed.range_predicate(value.prefix, StorageResolution::Millis);

    let mut arms: Vec<Document> = predicate
        .any_of
        .iter()
        .map(|group| {
            let mut arm = Document::new();
            let mut bound_on = |field: &str, op: &str, bound| {
                if let Ok(ops) = arm.get_document_mut(field) {
                    ops.insert(op, chrono_to_bson(bound));
                } else {
                    arm.insert(field, doc! { op: chrono_to_bson(bound) });
                }
            };
            for condition in group {
                match *condition {
                    RangeCondition::StartAtOrAfter(bound) => bound_on("value_date", "$gte", bound),
                    RangeCondition::StartBefore(bound) => bound_on("value_date", "$lt", bound),
                    RangeCondition::EndAfter(bound) => bound_on(VALUE_DATE_END, "$gt", bound),
                    RangeCondition::EndAtOrBefore(bound) => {
                        // Implied, since every stored range is at least one
                        // unit wide; it lets `eq` and `eb` seek on
                        // `idx_search_date_v3`, which leads with `value_date`.
                        bound_on("value_date", "$lt", bound);
                        bound_on(VALUE_DATE_END, "$lte", bound);
                    }
                }
            }
            if arm.contains_key("value_date") {
                arm
            } else {
                // `idx_search_date_v3` is partial on `value_date` existing; a
                // filter on the end alone would not imply it and would scan
                // every row of the resource type instead. `$ne: null` rather
                // than `$exists: true`, which the index cannot answer without
                // reading the document.
                let mut with_start = doc! { "value_date": { "$ne": null } };
                with_start.extend(arm);
                with_start
            }
        })
        .collect();

    Ok(if arms.len() == 1 {
        arms.remove(0)
    } else {
        doc! { "$or": arms }
    })
}

/// Flattens the date filters of one parameter's values into a single
/// top-level `$or` whose arms each repeat the `scope` conjuncts (tenant,
/// resource type, parameter name), so that every arm is a plain conjunction
/// the `idx_search_date_v3` index can seek on, covered (#1391).
///
/// A value filter is either one conjunction or `{ "$or": [conjunction, ..] }`
/// (see [`build_date_range_filter_doc`]); comma-separated values are OR'd, so
/// the union of all arms is the same set of rows as the nested form matched.
fn scoped_date_alternatives(scope: &Document, value_filters: Vec<Document>) -> Document {
    let mut arms: Vec<Bson> = Vec::new();
    for value_filter in value_filters {
        let alternatives = match value_filter.get_array("$or") {
            Ok(alternatives) if value_filter.len() == 1 => alternatives.clone(),
            _ => vec![Bson::Document(value_filter)],
        };
        for alternative in alternatives {
            if let Bson::Document(conditions) = alternative {
                let mut arm = scope.clone();
                arm.extend(conditions);
                arms.push(Bson::Document(arm));
            }
        }
    }
    doc! { "$or": arms }
}

/// Drops the `tenant_id` / `resource_type` scope from a filter built by
/// `build_search_index_filter`: at the top and, for a date filter, in each arm
/// of its top-level `$or` ([`scoped_date_alternatives`]).
fn strip_index_scope(filter: &mut Document, param_type: SearchParamType) {
    filter.remove("tenant_id");
    filter.remove("resource_type");
    if param_type != SearchParamType::Date {
        return;
    }
    if let Ok(arms) = filter.get_array_mut("$or") {
        for arm in arms {
            if let Bson::Document(arm) = arm {
                arm.remove("tenant_id");
                arm.remove("resource_type");
            }
        }
    }
}

/// The error for a number or quantity search value whose number is not one.
///
/// As for dates, such a value is an error here, never a filter — not even one
/// that matches nothing. The search gate (`validate_numeric_values`) reports
/// it first on every ordinary path; this is what the in-transaction
/// conditional paths, which build filters without passing the gate, fall back
/// on. Before the shared grammar this was a `QueryParseError`, and only for
/// what `f64::from_str` refused: `ltinf` was `{"$lt": Infinity}`, which every
/// indexed row satisfies (#1340).
fn invalid_number_value(
    param: &str,
    value: &str,
    error: &crate::search::NumberValueError,
) -> StorageError {
    StorageError::Search(SearchError::InvalidNumberValue {
        param: param.to_string(),
        value: value.to_string(),
        reason: error.to_string(),
    })
}

const CANDIDATE_BATCH_SIZE: usize = 512;
const PROBE_ROW_LIMIT: u64 = 100_000;
// 300k × ~45 bytes/UUID ≈ 13.5 MB — safely under the 16 MB BSON document cap.
const MAX_RESULT_ID_SET: usize = 300_000;

/// Targets a reference `:identifier` search may resolve to (#1408). They
/// travel in one `$in`, two entries each; 10 000 stays near 1 MB, well under
/// the 16 MB BSON limit. Past it the search is refused, never truncated.
const MAX_IDENTIFIER_TARGETS: usize = 10_000;

async fn collect_documents(mut cursor: Cursor<Document>) -> StorageResult<Vec<Document>> {
    let mut docs = Vec::new();
    while cursor
        .advance()
        .await
        .or_query_error("Failed to advance MongoDB cursor")?
    {
        let doc = cursor
            .deserialize_current()
            .or_query_error("Failed to deserialize MongoDB document")?;
        docs.push(doc);
    }
    Ok(docs)
}

/// Orders `(resource_id, key)` pairs the way the server's
/// `{ key: <order>, _id: 1 }` sort would: by key in the requested direction,
/// ties broken by id ascending in both directions. Returns the ids.
fn order_sort_keys(
    mut keyed: Vec<(String, Bson)>,
    direction: crate::types::SortDirection,
) -> Vec<String> {
    use crate::types::SortDirection;
    keyed.sort_by(|(id_a, key_a), (id_b, key_b)| {
        let by_key = compare_sort_keys(key_a, key_b);
        let by_key = match direction {
            SortDirection::Ascending => by_key,
            SortDirection::Descending => by_key.reverse(),
        };
        by_key.then_with(|| id_a.cmp(id_b))
    });
    keyed.into_iter().map(|(id, _)| id).collect()
}

/// Total order over the sort-key values the search index writes: dates by
/// instant, strings byte-wise (the server's default, collation-free order),
/// numbers numerically across the integer/double representations. Mixed
/// types within one parameter do not occur (the writer emits one field per
/// parameter type); they fall back to a fixed type rank so the order is
/// still total.
fn compare_sort_keys(a: &Bson, b: &Bson) -> std::cmp::Ordering {
    match (a, b) {
        (Bson::DateTime(x), Bson::DateTime(y)) => x.cmp(y),
        (Bson::String(x), Bson::String(y)) => x.cmp(y),
        _ => match (sort_key_as_f64(a), sort_key_as_f64(b)) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            _ => sort_key_type_rank(a)
                .cmp(&sort_key_type_rank(b))
                .then_with(|| a.to_string().cmp(&b.to_string())),
        },
    }
}

fn sort_key_as_f64(value: &Bson) -> Option<f64> {
    match value {
        Bson::Double(d) => Some(*d),
        Bson::Int32(i) => Some(f64::from(*i)),
        Bson::Int64(i) => Some(*i as f64),
        Bson::Decimal128(d) => d.to_string().parse().ok(),
        _ => None,
    }
}

fn sort_key_type_rank(value: &Bson) -> u8 {
    match value {
        Bson::Null => 0,
        Bson::Double(_) | Bson::Int32(_) | Bson::Int64(_) | Bson::Decimal128(_) => 1,
        Bson::String(_) => 2,
        Bson::DateTime(_) => 3,
        _ => 4,
    }
}

async fn read_cursor_batch(
    cursor: &mut Cursor<Document>,
    limit: usize,
) -> StorageResult<Vec<Document>> {
    let mut docs = Vec::with_capacity(limit);
    while docs.len() < limit {
        if !cursor
            .advance()
            .await
            .or_query_error("Failed to advance cursor")?
        {
            break;
        }
        let doc = cursor
            .deserialize_current()
            .or_query_error("Failed to deserialize cursor document")?;
        docs.push(doc);
    }
    Ok(docs)
}

/// Drops the probe row a cursor page over-fetches (`limit(page_size + 1)`)
/// and, on a backward page, restores the requested sort order. Returns
/// `(has_next, has_previous)`.
///
/// Forward: the probe row is the last one fetched and proves a next page;
/// `has_previous` is not knowable from the rows, so the caller's
/// `forward_has_previous` (cursor present or offset > 0) is passed through.
///
/// Backward: the filter selects rows *newer* than the cursor with the sort
/// flipped, so the driver returns nearest-newer first. The probe row is still
/// the last one fetched, but it is the *farthest* from the cursor — it belongs
/// to page N-2, not to the page being returned — and it proves a previous
/// page. It must be dropped *before* `reverse()` restores the sort order:
/// popping after the reverse discards the wanted adjacent row and keeps the
/// unwanted one, shifting the window by one on every backward hop (#1057).
/// A backward hop always has a next page (the one it came from) as long as it
/// returned any row. Mirrors the SQLite and PostgreSQL backward branches
/// (#1079).
fn trim_probe_row<T>(
    rows: &mut Vec<T>,
    page_size: usize,
    previous_mode: bool,
    forward_has_previous: bool,
) -> (bool, bool) {
    if previous_mode {
        let has_previous = rows.len() > page_size;
        if has_previous {
            rows.pop();
        }
        rows.reverse();
        (!rows.is_empty(), has_previous)
    } else {
        let has_next = rows.len() > page_size;
        if has_next {
            rows.pop();
        }
        (has_next, forward_has_previous)
    }
}

/// Finds the `contained[]` entry with the given local `id` in a container's
/// content.
fn extract_contained_resource(content: &Value, local_id: &str) -> Option<Value> {
    content
        .get("contained")?
        .as_array()?
        .iter()
        .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(local_id))
        .cloned()
}

/// One contained match: the container and, for `_containedType=contained`,
/// the local id of the contained entity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ContainedKey {
    rtype: String,
    rid: String,
    lid: Option<String>,
}

/// A server-side page of contained matches.
struct ContainedPage {
    keys: Vec<ContainedKey>,
    /// Only when `_total` was requested.
    total: Option<u64>,
}

/// Builds a `StoredResource` for a contained resource, inheriting the
/// container's version/tenant/timestamps. Used for `_containedType=contained`.
fn build_contained_stored(
    container: &StoredResource,
    contained_type: &str,
    local_id: &str,
    content: Value,
) -> StoredResource {
    StoredResource::from_storage(
        contained_type.to_string(),
        local_id.to_string(),
        container.version_id().to_string(),
        container.tenant_id().clone(),
        content,
        container.created_at(),
        container.last_modified(),
        None,
        container.fhir_version(),
    )
}

/// The `search_index` field a parameter type's value lives in. `None` for
/// `Composite`/`Special`, which have no single value field of their own
/// (composite rows carry each sub-parameter's own field; special params like
/// `_id`/`_lastUpdated` are not stored in `search_index` at all).
pub(super) fn value_field_for(param_type: SearchParamType) -> Option<&'static str> {
    match param_type {
        SearchParamType::String => Some("value_string"),
        SearchParamType::Token => Some("value_token_code"),
        SearchParamType::Date => Some("value_date"),
        SearchParamType::Number => Some("value_number"),
        SearchParamType::Quantity => Some("value_quantity_value"),
        SearchParamType::Reference => Some("value_reference"),
        SearchParamType::Uri => Some("value_uri"),
        SearchParamType::Composite | SearchParamType::Special => None,
    }
}

/// What a parameter sort aggregates per resource: the value field, except
/// that a descending date sort reads the end of each stored range (#1391) —
/// a `Period` sorts by where it ends, as the SQL backends' `MAX` over the end
/// column. A row indexed before #1391 has no end and falls back to its start.
fn sort_key_expression(value_field: &str, direction: crate::types::SortDirection) -> Bson {
    if value_field == "value_date" && direction == crate::types::SortDirection::Descending {
        Bson::Document(doc! { "$ifNull": [format!("${VALUE_DATE_END}"), "$value_date"] })
    } else {
        Bson::String(format!("${value_field}"))
    }
}

/// The envelope filter for a `:missing` presence check
/// (`{tenant_id, resource_type, param_name}`, matching every row `search_index`
/// carries for the parameter regardless of value), plus — when the parameter
/// type has a value field — a `{value_field: {"$ne": null}}` conjunct.
///
/// The extra conjunct is not redundant with the envelope: every generation-2
/// value index (`idx_search_*_v2`) is a *partial* index built with
/// `partialFilterExpression: {value_field: {"$exists": true}}`, so MongoDB
/// only considers it for a query it can prove is a subset of that filter.
/// The bare envelope has no predicate on any value field at all, so no
/// partial index qualifies and the planner falls back to a full scan of the
/// `(tenant_id, resource_type)` slice on `idx_search_composite` — every
/// parameter, every row of the type. Adding the value-field conjunct lets
/// the planner pick that parameter's own partial index and, since
/// `distinct_resource_ids` reads only `resource_id` (the index's trailing
/// key), serves the scan fully covered. Measured on MongoDB 7.0.40.
pub(super) fn missing_presence_filter(
    tenant_id: &str,
    resource_type: &str,
    param: &SearchParameter,
) -> Document {
    let mut filter = doc! {
        "tenant_id": tenant_id,
        "resource_type": resource_type,
        "param_name": &param.name,
    };
    if let Some(value_field) = value_field_for(param.param_type) {
        filter.insert(value_field, doc! { "$ne": Bson::Null });
    }
    filter
}

/// Rejects the `_contained=true|both` constraints that select *top-level*
/// resources, which a contained resource never is (#1383, #1407).
///
/// Kept as a separate gate from `matching_contained`'s per-parameter refusals
/// because these constraints live outside `query.parameters` and `both` can
/// fill a page without entering `matching_contained`. Supported composites
/// pair their components per contained entity over `search_index_contained`.
fn reject_contained_composite(query: &SearchQuery) -> StorageResult<()> {
    if query.contained == crate::types::ContainedMode::Off {
        return Ok(());
    }
    // `_has` and `_list` live outside `query.parameters` and select
    // *top-level* resources, which a contained resource never is: nothing
    // outside its container can reference it (#1383).
    for (present, name) in [
        (!query.reverse_chains.is_empty(), "_has"),
        (!query.list.is_empty(), "_list"),
    ] {
        if present {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "'{name}' cannot be combined with _contained=true or both: it selects \
                     top-level resources, which a contained resource is not"
                ),
            }));
        }
    }
    // `_sort` orders by the *contained* resource's values, which
    // `matching_contained` only matches on: it lists matches by container
    // type, id and local id, after the top-level page for `both`. Refused
    // rather than answered in that order (#1407).
    if !query.sort.is_empty() {
        return Err(StorageError::Search(SearchError::QueryParseError {
            message: "'_sort' cannot be combined with _contained=true or both: sorting \
                      contained matches is not supported on MongoDB"
                .to_string(),
        }));
    }
    // In `both` mode the top-level page can be full, so matching_contained
    // may never run. Refuse composites the contained index cannot interpret
    // before either branch is executed.
    if let Some((param, reason)) = query
        .parameters
        .iter()
        .filter(|param| param.param_type == SearchParamType::Composite)
        .find_map(|param| contained_unsupported_reason(param).map(|reason| (param, reason)))
    {
        return Err(reject_contained_parameter(param, &reason));
    }
    Ok(())
}

/// Why `matching_contained` cannot apply `param`, if it cannot (#1363).
///
/// `search_index_contained` holds what the extractor found in the contained
/// resource itself, one document per value. That answers every ordinary
/// parameter, the `meta`-derived `_`-parameters and — through
/// `contained_local_id` — `_id`. It does not answer:
///
/// - `_lastUpdated`: a contained resource has no `meta.lastUpdated` of its
///   own, and the container's is not on these documents;
/// - `_text`, `_content` and the other `_`-parameters resolved against
///   `resources`, which only knows the container;
/// - chains (composite components are paired per contained entity instead);
/// - `:not` and `:missing`, which the standard path resolves as a complement
///   over *resources* (`matching_resource_ids_complement_only`), never as a
///   `search_index` filter. There is no such complement over contained
///   entities yet.
///
/// Other modifiers on ordinary parameters go to their value filter builder.
/// Composite modifiers are refused because `component_param` removes them
/// before building the typed predicates.
fn contained_unsupported_reason(param: &SearchParameter) -> Option<String> {
    if param.param_type == SearchParamType::Composite {
        if let Some(modifier) = &param.modifier {
            // component_param removes the composite modifier before building
            // typed predicates; no modifier can be honoured on this path.
            return Some(format!("the ':{modifier}' composite modifier is"));
        }
    }
    if !param.chain.is_empty() {
        return Some("chained parameters are".to_string());
    }
    if param.name == "_id" {
        return param
            .modifier
            .as_ref()
            .map(|m| format!("the ':{m}' modifier on _id is"));
    }
    if param.name.starts_with('_')
        && !matches!(
            param.name.as_str(),
            "_tag" | "_profile" | "_security" | "_source" | "_language"
        )
    {
        return Some("this parameter is".to_string());
    }
    match (&param.modifier, param.param_type) {
        (_, SearchParamType::Special) => Some("special parameters are".to_string()),
        (Some(m @ (SearchModifier::Not | SearchModifier::Missing)), _) => {
            Some(format!("the ':{m}' modifier is"))
        }
        _ => None,
    }
}

/// The error for a criterion `_contained` matching cannot apply, naming it:
/// dropping it instead would answer a wider question than the one asked.
fn reject_contained_parameter(param: &SearchParameter, reason: &str) -> StorageError {
    let message = format!(
        "search parameter '{}' cannot be combined with _contained=true or both: {reason} not \
         supported for contained resources on MongoDB",
        param.name
    );
    StorageError::Search(match param.param_type {
        SearchParamType::Composite => SearchError::InvalidComposite { message },
        _ => SearchError::QueryParseError { message },
    })
}

/// The resource-document field a cursor pages over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CursorKeysetField {
    /// `last_updated`, carried in the cursor as an RFC 3339 string.
    LastUpdated,
    /// `id`, carried as is. Unique within a `(tenant, type)` slice of the
    /// resources collection, so it needs no tie-break.
    Id,
}

impl CursorKeysetField {
    fn as_str(self) -> &'static str {
        match self {
            Self::LastUpdated => "last_updated",
            Self::Id => "id",
        }
    }
}

/// The keyset a `_cursor` compares against: the sorted field and its
/// forward direction (#1058). Mirrors the SQLite / PostgreSQL
/// `primary_keyset_key`: the sort itself is re-derived from the request's
/// `_sort` (which the `next` / `previous` links preserve), and the cursor
/// carries only the boundary value of that field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CursorKeyset {
    field: CursorKeysetField,
    direction: crate::types::SortDirection,
}

impl CursorKeyset {
    /// The keyset for `query`, or `None` when its sort cannot be keyset-paged:
    /// more than one directive, or a directive [`Self`] has no field for
    /// (a search parameter, whose key lives in the search index).
    fn for_query(query: &SearchQuery) -> Option<Self> {
        match query.sort.as_slice() {
            [] => Some(Self {
                field: CursorKeysetField::LastUpdated,
                direction: crate::types::SortDirection::Descending,
            }),
            [directive] => {
                let field = match directive.parameter.as_str() {
                    "_lastUpdated" => CursorKeysetField::LastUpdated,
                    "_id" | "id" => CursorKeysetField::Id,
                    _ => return None,
                };
                Some(Self {
                    field,
                    direction: directive.direction,
                })
            }
            _ => None,
        }
    }

    /// `resource`'s value of the keyset field, as the cursor carries it.
    fn value_of(self, resource: &StoredResource) -> CursorValue {
        match self.field {
            CursorKeysetField::LastUpdated => {
                CursorValue::String(resource.last_modified().to_rfc3339())
            }
            CursorKeysetField::Id => CursorValue::String(resource.id().to_string()),
        }
    }
}

#[async_trait]
impl SearchProvider for MongoBackend {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        reject_contained_composite(query)?;

        // `_contained` search uses a dedicated path (separate index rows and
        // heterogeneous result types); standard search handles `_contained=false`
        // (contained rows carry the container's resource_type, so the standard
        // type-scoped filter naturally excludes them).
        if query.contained != crate::types::ContainedMode::Off {
            return self.search_contained(tenant, query).await;
        }

        self.validate_query_support(query)?;

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let cursor = if let Some(cursor_str) = &query.cursor {
            Some(PageCursor::decode(cursor_str).map_err(|_| {
                StorageError::Search(SearchError::InvalidCursor {
                    cursor: cursor_str.clone(),
                })
            })?)
        } else {
            None
        };

        // Keyset for cursor pagination (#1058): the default sort or a single
        // `_id` / `_lastUpdated` directive. `None` for multi-field sorts,
        // which page by offset only — no cursor is minted for them below, so
        // an inbound one can only be a client's own construction.
        let keyset = CursorKeyset::for_query(query);
        if cursor.is_some() && keyset.is_none() {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: "MongoDB cursor pagination supports the default sort or a single \
                          _id / _lastUpdated sort directive"
                    .to_string(),
            }));
        }

        let previous_mode = cursor
            .as_ref()
            .is_some_and(|c| c.direction() == CursorDirection::Previous);

        let matched_ids = self
            .matching_resource_ids(&db, tenant_id, &query.resource_type, query)
            .await?;

        // Sorting by an indexed search parameter (#881): the sort key lives
        // in the search index, not on the resource documents, so the ordered
        // id list is computed there and the page fetched by id. Offset-paged;
        // it has no keyset, so a cursor is already rejected above.
        let param_sort = query
            .sort
            .iter()
            .find(|d| !matches!(d.parameter.as_str(), "_id" | "_lastUpdated" | "_score"));
        if let Some(directive) = param_sort {
            if query.sort.len() > 1 {
                return Err(StorageError::Search(SearchError::QueryParseError {
                    message: "MongoDB supports a single _sort directive when sorting by a                               search parameter"
                        .to_string(),
                }));
            }
            return self
                .search_param_sorted(tenant, query, &db, tenant_id, matched_ids, directive)
                .await;
        }

        let filter = self.build_resource_filter(
            tenant_id,
            &query.resource_type,
            query,
            matched_ids.as_ref(),
            cursor.as_ref().zip(keyset.as_ref()),
        )?;

        let sort = self.build_sort_document(query, previous_mode)?;
        let page_size = query.count.unwrap_or(100).max(1) as usize;

        let mut find_action = resources
            .find(filter)
            .sort(sort)
            .limit((page_size + 1) as i64);

        if cursor.is_none() {
            if let Some(offset) = query.offset {
                find_action = find_action.skip(offset as u64);
            }
        }

        let docs = collect_documents(
            find_action
                .await
                .or_query_error("Failed to execute MongoDB search")?,
        )
        .await?;

        let mut resources = docs
            .into_iter()
            .map(|doc| self.document_to_stored_resource(tenant, &query.resource_type, doc))
            .collect::<StorageResult<Vec<_>>>()?;

        let (has_next, has_previous) = trim_probe_row(
            &mut resources,
            page_size,
            previous_mode,
            cursor.is_some() || query.offset.unwrap_or(0) > 0,
        );

        // Cursors carry the value of the field the page is sorted on; a sort
        // with no keyset (multi-field) gets none and pages by offset, which
        // is what the REST layer's `next` link falls back to. Minting one
        // here would advertise a link the entry point above rejects (#1058).
        let next_cursor = match (&keyset, has_next) {
            (Some(k), true) => resources.last().map(|resource| {
                PageCursor::new(vec![k.value_of(resource)], resource.id()).encode()
            }),
            _ => None,
        };

        let previous_cursor = match (&keyset, has_previous) {
            (Some(k), true) => resources.first().map(|resource| {
                PageCursor::previous(vec![k.value_of(resource)], resource.id()).encode()
            }),
            _ => None,
        };

        let total = if query.total.is_some() {
            Some(self.search_count(tenant, query).await?)
        } else {
            None
        };

        let page_info = PageInfo {
            next_cursor,
            previous_cursor,
            total,
            has_next,
            has_previous,
        };

        let page = Page::new(resources, page_info);
        let mut included: Vec<StoredResource> = Vec::new();

        if !query.includes.is_empty() {
            let forward: Vec<IncludeDirective> = query
                .includes
                .iter()
                .filter(|i| i.include_type == IncludeType::Include)
                .cloned()
                .collect();
            if !forward.is_empty() {
                let resolved = self
                    .resolve_forward_includes_capped(tenant, &page.items, &forward)
                    .await?;
                Self::merge_unique(&mut included, resolved);
            }

            let reverse: Vec<IncludeDirective> = query
                .includes
                .iter()
                .filter(|i| i.include_type == IncludeType::Revinclude)
                .cloned()
                .collect();
            if !reverse.is_empty() {
                let resolved = self
                    .resolve_revincludes(tenant, &page.items, &reverse)
                    .await?;
                Self::merge_unique(&mut included, resolved);
            }
        }

        Ok(SearchResult {
            resources: page,
            included,
            total,
            scores: Default::default(),
        })
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        reject_contained_composite(query)?;
        self.validate_query_support(query)?;

        // Under `_contained` the count is of what `search` returns (#1383),
        // not of the top-level resources matching the same criteria: ask the
        // contained path for its total.
        if query.contained != crate::types::ContainedMode::Off {
            let mut counted = query.clone();
            counted.count = Some(1);
            counted.offset = None;
            counted.total = Some(crate::types::TotalMode::Accurate);
            return self
                .search_contained(tenant, &counted)
                .await?
                .total
                .ok_or_else(|| internal_error("contained search returned no total".to_string()));
        }

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let matched_ids = self
            .matching_resource_ids(&db, tenant_id, &query.resource_type, query)
            .await?;

        let filter = self.build_resource_filter(
            tenant_id,
            &query.resource_type,
            query,
            matched_ids.as_ref(),
            None,
        )?;

        resources
            .count_documents(filter)
            .await
            .or_query_error("Failed to count MongoDB search results")
    }

    fn search_param_registry(
        &self,
        tenant: &crate::tenant::TenantContext,
    ) -> std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        self.tenant_registry(tenant.tenant_id().as_str())
    }

    fn supports_contained_search(&self) -> bool {
        true
    }

    fn modifiers_for_param_type(
        &self,
        param_type: crate::types::SearchParamType,
    ) -> Vec<&'static str> {
        Self::modifiers_for_type(param_type)
    }
}

#[async_trait]
impl ConditionalStorage for MongoBackend {
    fn supports_conditional(&self, interaction: crate::core::ConditionalInteraction) -> bool {
        // One declaration: the capability list the contract test pins (#1384).
        crate::core::Backend::supports(self, interaction.capability())
    }

    async fn conditional_create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalCreateResult> {
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                let created = self
                    .create(tenant, resource_type, resource, fhir_version)
                    .await?;
                Ok(ConditionalCreateResult::Created(created))
            }
            1 => Ok(ConditionalCreateResult::Exists(
                matches.into_iter().next().expect("single match must exist"),
            )),
            n => Ok(ConditionalCreateResult::MultipleMatches(n)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
        if_match: &crate::core::EntityTagPrecondition,
    ) -> StorageResult<ConditionalUpdateResult> {
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                // `If-Match` names a version; nothing matched, so nothing
                // can carry it and the create below must not run (#1381).
                crate::core::conditional_if_match_gate(if_match, resource_type, None)?;
                if upsert {
                    let created = self
                        .create(tenant, resource_type, resource, fhir_version)
                        .await?;
                    Ok(ConditionalUpdateResult::Created(created))
                } else {
                    Ok(ConditionalUpdateResult::NoMatch)
                }
            }
            1 => {
                // `update` compares-and-swaps on `current`'s version, the one
                // `If-Match` is evaluated against here.
                let current = matches.into_iter().next().expect("single match must exist");
                crate::core::conditional_if_match_gate(if_match, resource_type, Some(&current))?;
                let updated = self.update(tenant, &current, resource).await?;
                Ok(ConditionalUpdateResult::Updated(updated))
            }
            n => Ok(ConditionalUpdateResult::MultipleMatches(n)),
        }
    }

    async fn conditional_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
        if_match: &crate::core::EntityTagPrecondition,
    ) -> StorageResult<ConditionalDeleteResult> {
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                // A supplied `If-Match` fails against no match, as it does on
                // `DELETE [type]/[id]` for a missing resource.
                crate::core::conditional_if_match_gate(if_match, resource_type, None)?;
                Ok(ConditionalDeleteResult::NoMatch)
            }
            1 => {
                let current = matches.into_iter().next().expect("single match must exist");
                crate::core::conditional_if_match_gate(if_match, resource_type, Some(&current))?;
                crate::core::delete_under_precondition(self, tenant, if_match, &current).await?;
                Ok(ConditionalDeleteResult::Deleted(current))
            }
            n => Ok(ConditionalDeleteResult::MultipleMatches(n)),
        }
    }

    /// The criteria resolver the provided
    /// [`ConditionalStorage::conditional_patch`] is written in terms of
    /// (#1406).
    async fn resolve_conditional_matches(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        self.find_matching_resources(tenant, resource_type, search_params)
            .await
    }
}

/// One composite component's scoped `search_index` filter, plus whether its
/// parsed value used the `Ne` prefix. `negated` components are
/// existence-bounded only (see `MongoBackend::composite_component_filters`),
/// so they must never be probed or chosen as a composite's driver arm —
/// `composite_driver_probe` filters on this; `composite_pair_check` uses
/// `filter` alone, same as before this flag existed.
#[derive(Debug)]
struct ComponentFilter {
    filter: Document,
    negated: bool,
}

/// One composite component's `search_index_contained` filter, scoped to
/// `(tenant_id, contained_type, param_name)` instead of
/// `(tenant_id, resource_type, param_name)`: contained rows carry the
/// *container's* `resource_type`, so scoping by the searched type would match
/// nothing. Built by [`MongoBackend::contained_composite_component_filters`].
#[derive(Debug)]
struct ContainedComponentFilter {
    filter: Document,
}

/// Builds the aggregation stages matching one composite value's entities over
/// `search_index_contained` (#1407).
///
/// Takes one value's already-scoped component filters (see
/// [`MongoBackend::contained_composite_component_filters`]) and returns the
/// stages matching every contained entity whose rows pair all components
/// within a single `composite_group`: one `$match` arm per component, tagged
/// with its `component_idx`, joined by `$unionWith`, then grouped by
/// `(resource_type, resource_id, contained_local_id, composite_group)` with
/// all component indices required, and finally collapsed to the entity shape
/// `_id = {rtype, rid, lid}` the contained pipeline groups on.
///
/// Each arm keeps its full scoped filter (tenant, contained type, parameter
/// name, typed predicate), so no arm can leak rows across tenants or
/// parameters. `Ne` components need no special casing here: their filter is
/// the same existence-bounded predicate the top-level pair check runs, and
/// these arms only ever feed the grouped pair check, never a driver probe.
/// An empty component list is a closed failure, never a vacuous match.
fn contained_composite_value_stages(
    components: &[ContainedComponentFilter],
) -> StorageResult<Vec<Document>> {
    if components.is_empty() {
        return Err(StorageError::Search(SearchError::InvalidComposite {
            message: "composite value has no components to match".to_string(),
        }));
    }
    let arm = |index: usize, filter: &Document| {
        vec![
            doc! { "$match": filter.clone() },
            doc! { "$addFields": { "component_idx": index as i32 } },
        ]
    };
    let required: Vec<Bson> = (0..components.len())
        .map(|index| Bson::Int32(index as i32))
        .collect();
    let mut first = components
        .first()
        .map(|component| arm(0, &component.filter))
        .unwrap_or_default();
    for (index, component) in components.iter().enumerate().skip(1) {
        first.push(doc! { "$unionWith": {
            "coll": MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION,
            "pipeline": arm(index, &component.filter),
        }});
    }
    first.push(doc! { "$group": {
        "_id": {
            "rtype": "$resource_type",
            "rid": "$resource_id",
            "lid": "$contained_local_id",
            "grp": "$composite_group",
        },
        "components": { "$addToSet": "$component_idx" },
    }});
    first.push(doc! { "$match": { "components": { "$all": required } } });
    first.push(doc! { "$group": {
        "_id": {
            "rtype": "$_id.rtype",
            "rid": "$_id.rid",
            "lid": "$_id.lid",
        },
    }});
    Ok(first)
}

impl MongoBackend {
    /// Builds every component's scoped `search_index_contained` filter for a
    /// composite parameter under `_contained` (#1407) — outer index is the
    /// (comma-OR'd) value, inner index is the component, in declaration order.
    ///
    /// Reuses [`Self::composite_component_filters`] so composite value
    /// splitting, per-type prefix handling, quantity/date/number predicates
    /// and the `{value_field: {"$ne": null}}` scoping conjunct stay on one
    /// code path. Each returned filter is then re-scoped from the top-level
    /// `(tenant_id, resource_type)` slice to the contained
    /// `(tenant_id, contained_type)` slice: `resource_type` is removed (a
    /// contained row carries its container's type, never the searched type)
    /// and the composite's own `param_name` is kept, since every component
    /// row shares it.
    /// Modifier and chain validation happens before this builder; unsupported value types are checked by
    /// `composite_component_filters`.
    fn contained_composite_component_filters(
        &self,
        tenant_id: &str,
        contained_type: &str,
        param: &SearchParameter,
    ) -> StorageResult<Vec<Vec<ContainedComponentFilter>>> {
        let per_value = self.composite_component_filters(tenant_id, contained_type, param)?;
        let mut result = Vec::with_capacity(per_value.len());
        for per_component in per_value {
            let mut rescoped = Vec::with_capacity(per_component.len());
            for component in per_component {
                let mut filter = component.filter;
                filter.remove("resource_type");
                filter.insert("tenant_id", tenant_id);
                filter.insert("contained_type", contained_type);
                filter.insert("param_name", param.name.clone());
                rescoped.push(ContainedComponentFilter { filter });
            }
            result.push(rescoped);
        }
        Ok(result)
    }

    /// Builds the aggregation stages matching one composite parameter's entities
    /// over `search_index_contained` (#1407): one value pipeline per
    /// comma-separated value (see `contained_composite_value_stages`), joined
    /// by `$unionWith` and de-duplicated by a final `$group` on `$_id`.
    ///
    /// Comma is OR: an entity matching any value matches the parameter. The
    /// final `$group` collapses entities matched by several values (e.g. an
    /// entity pairing both `A$gt5` and `B$gt5` groups) to one slot, in the
    /// same `_id = {rtype, rid, lid}` shape `matching_contained` groups on.
    /// No values is a closed failure, never a vacuous match.
    fn contained_composite_stages(
        &self,
        tenant_id: &str,
        contained_type: &str,
        param: &SearchParameter,
    ) -> StorageResult<Vec<Document>> {
        let per_value =
            self.contained_composite_component_filters(tenant_id, contained_type, param)?;
        if per_value.is_empty() {
            return Err(StorageError::Search(SearchError::InvalidComposite {
                message: format!("composite search parameter '{}' has no values", param.name),
            }));
        }
        let mut values = per_value.iter();
        let first = values
            .next()
            .map(|value| contained_composite_value_stages(value))
            .transpose()?
            .unwrap_or_default();
        let mut stages = first;
        for value in values {
            let pipeline = contained_composite_value_stages(value)?;
            stages.push(doc! { "$unionWith": {
                "coll": MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION,
                "pipeline": pipeline,
            }});
        }
        stages.push(doc! { "$group": { "_id": "$_id" } });
        Ok(stages)
    }

    /// Executes a `_contained=true|both` search (see the SQLite backend's
    /// `search_contained` for shared semantics). Returns containers (default) or
    /// contained resources (`_containedType=contained`); `both` merges top-level
    /// matches first. Paged on the server over `idx_search_contained` (#1059).
    async fn search_contained(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        use crate::types::{ContainedMode, ContainedReturn, TotalMode};

        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let contained_type = query.resource_type.as_str();
        self.preflight_legacy_composites(
            &db,
            tenant_id,
            contained_type,
            &query.parameters,
            true,
            None,
        )
        .await?;
        let count = query.count.unwrap_or(100).max(1) as usize;
        let offset = query.offset.unwrap_or(0) as usize;
        let want_total = query.wants_total();

        let (mut items, total) = match query.contained {
            ContainedMode::Both => {
                // Top-level matches come first, contained matches second. The
                // standard search is asked for its total so the boundary is
                // known, and each source is paged on the server.
                let mut top_query = query.clone();
                top_query.contained = ContainedMode::Off;
                top_query.contained_return = ContainedReturn::Container;
                top_query.total = Some(TotalMode::Accurate);
                let top = self.search(tenant, &top_query).await?;
                let top_total = top.total.ok_or_else(|| {
                    internal_error(
                        "standard search returned no total for _contained=both".to_string(),
                    )
                })? as usize;
                let mut items = top.resources.items;

                // A container of the searched type can also satisfy the
                // top-level query. Resolve that query's full predicate, not
                // just the current top-level page, so the contained pipeline
                // can discard overlaps before its offset and count stages.
                let top_filter = if query.contained_return == ContainedReturn::Container {
                    let matched_ids = self
                        .matching_resource_ids(&db, tenant_id, contained_type, &top_query)
                        .await?;
                    Some(self.build_resource_filter(
                        tenant_id,
                        contained_type,
                        &top_query,
                        matched_ids.as_ref(),
                        None,
                    )?)
                } else {
                    None
                };

                let (c_offset, c_limit) = if offset < top_total {
                    (0, count.saturating_sub(items.len()))
                } else {
                    (offset - top_total, count)
                };
                let mut contained_total = None;
                if c_limit > 0 {
                    let page = self
                        .matching_contained(
                            &db,
                            tenant_id,
                            contained_type,
                            query.contained_return,
                            query,
                            c_offset,
                            c_limit,
                            want_total,
                            top_filter.as_ref(),
                        )
                        .await?;
                    contained_total = page.total;
                    let contained = self
                        .materialize_contained(
                            &db,
                            tenant,
                            contained_type,
                            query.contained_return,
                            &page.keys,
                        )
                        .await?;
                    items.extend(contained);
                } else if want_total {
                    // No room left on this page for contained items, but the
                    // caller still wants a total: fetch the count only.
                    let page = self
                        .matching_contained(
                            &db,
                            tenant_id,
                            contained_type,
                            query.contained_return,
                            query,
                            c_offset,
                            1,
                            true,
                            top_filter.as_ref(),
                        )
                        .await?;
                    contained_total = page.total;
                }
                let total = if want_total {
                    Some(top_total as u64 + contained_total.unwrap_or(0))
                } else {
                    None
                };
                (items, total)
            }
            _ => {
                let page = self
                    .matching_contained(
                        &db,
                        tenant_id,
                        contained_type,
                        query.contained_return,
                        query,
                        offset,
                        count,
                        want_total,
                        None,
                    )
                    .await?;
                let items = self
                    .materialize_contained(
                        &db,
                        tenant,
                        contained_type,
                        query.contained_return,
                        &page.keys,
                    )
                    .await?;
                (items, page.total)
            }
        };

        items.truncate(count);
        let page = Page::new(items, PageInfo::end());
        let mut result = SearchResult::new(page);
        if let Some(t) = total {
            result = result.with_total(t);
        }
        Ok(result)
    }

    /// Resolves one server-side page of `_contained` matches over the
    /// `search_index_contained` collection (#1160), via its `idx_search_contained`
    /// index (#1059): `$match` in the index's key order,
    /// then a two-stage grouping (#1059 review N1). The first `$group` is
    /// always per contained *entity* — `{ rtype, rid, lid }` — because a
    /// multi-parameter AND (the `names: $all` `$match` that follows it) must
    /// hold within one contained entity, not across every entity a container
    /// happens to hold: grouping straight to the container would let one
    /// contained Patient matching `name=Smith` and a different contained
    /// Patient matching `gender=male` in the same container satisfy
    /// `name=Smith&gender=male` together, which is wrong. Only for the
    /// default `_containedType=container` is there a second `$group`, which
    /// collapses the surviving per-entity slots down to one slot per
    /// container (dropping `lid`) so a container is one page slot, `_total`
    /// counts containers, and a page cannot straddle a container with
    /// multiple internal matches. Then `$sort` for a stable page order, then
    /// `$skip`/`$limit`, with a `$facet` count alongside when `_total` is
    /// requested.
    ///
    /// The set of matched *names* proves every criterion held only while each
    /// name occurs once. When a name repeats (`date=ge2020&date=le2020`) the
    /// per-entity stage is instead one `$unionWith` arm per occurrence, and an
    /// entity must come back from all of them (#1362). A criterion this path
    /// cannot apply is refused, never skipped (#1363) — see
    /// [`contained_unsupported_reason`]. Composite parameters use their own
    /// grouped component checks, then join this per-occurrence intersection.
    #[allow(clippy::too_many_arguments)]
    async fn matching_contained(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        contained_type: &str,
        contained_return: crate::types::ContainedReturn,
        query: &SearchQuery,
        offset: usize,
        limit: usize,
        want_total: bool,
        exclude_top_level: Option<&Document>,
    ) -> StorageResult<ContainedPage> {
        use crate::types::ContainedReturn;
        let contained_rows =
            db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION);

        let mut entity_scope = doc! { "tenant_id": tenant_id, "contained_type": contained_type };
        // One entry per parameter *occurrence*; values inside an occurrence
        // are ORed by `build_search_index_filter`.
        let mut branches: Vec<Document> = Vec::new();
        let mut distinct_names: Vec<String> = Vec::new();
        let mut composite_branches: Vec<Vec<Document>> = Vec::new();
        // `_id` is the contained resource's local id, a field of every row.
        let mut id_clauses: Vec<Bson> = Vec::new();
        let mut composite_id_clauses: Vec<Bson> = Vec::new();
        for param in &query.parameters {
            if let Some(reason) = contained_unsupported_reason(param) {
                return Err(reject_contained_parameter(param, &reason));
            }
            if param.name == "_id" {
                let ids: Vec<&str> = param.values.iter().map(|v| v.value.as_str()).collect();
                id_clauses.push(Bson::Document(
                    doc! { "contained_local_id": { "$in": ids.clone() } },
                ));
                composite_id_clauses.push(Bson::Document(doc! { "_id.lid": { "$in": ids } }));
                continue;
            }
            if param.param_type == SearchParamType::Composite {
                composite_branches.push(self.contained_composite_stages(
                    tenant_id,
                    contained_type,
                    param,
                )?);
                continue;
            }
            // Reuse the standard per-param value filter, dropping the tenant /
            // resource_type scoping (handled by the pipeline's top `$match`).
            let mut branch = self.build_search_index_filter("", "", param)?;
            strip_index_scope(&mut branch, param.param_type);
            branches.push(branch);
            if !distinct_names.contains(&param.name) {
                distinct_names.push(param.name.clone());
            }
        }
        // Compartment membership is a criterion on the contained resource like
        // any other: it references the compartment through ANY of the
        // membership parameters (#1383). That one branch spans several
        // parameter names, so it is left out of `distinct_names` — which sends
        // the pipeline down the per-occurrence path below.
        if let Some(comp) = &query.compartment {
            if !comp.params.is_empty() && !comp.reference.is_empty() {
                let base = strip_reference_version(&comp.reference);
                let params: Vec<Bson> = comp.params.iter().cloned().map(Bson::String).collect();
                branches.push(doc! {
                    "param_name": { "$in": Bson::Array(params) },
                    "$or": [
                        { "value_reference": &base },
                        { "value_reference": {
                            "$regex": format!("^{}/_history/", regex_escape(base))
                        }},
                    ],
                });
            }
        }
        // With no criterion at all, every contained resource of the type
        // matches (#1383): the grouping below lists each of them once.
        if !id_clauses.is_empty() {
            entity_scope.insert("$and", id_clauses);
        }
        let has_plain_branches = !branches.is_empty();

        let entity = doc! {
            "rtype": "$resource_type",
            "rid": "$resource_id",
            "lid": "$contained_local_id",
        };
        let mut pipeline = if distinct_names.len() == branches.len() {
            // Every name occurs once, so the set of names an entity matched
            // proves every branch did.
            let mut first = entity_scope;
            if !branches.is_empty() {
                first.insert(
                    "$or",
                    branches.into_iter().map(Bson::Document).collect::<Vec<_>>(),
                );
            }
            let mut stages = vec![
                doc! { "$match": first },
                // Always per entity: the AND below must hold within one
                // contained resource, not across every entity a container holds.
                doc! { "$group": {
                    "_id": entity,
                    "names": { "$addToSet": "$param_name" },
                }},
            ];
            if distinct_names.len() > 1 {
                stages.push(doc! { "$match": { "names": { "$all": distinct_names } } });
            }
            stages
        } else {
            // A name repeats (`date=ge2020&date=le2020`): one row can satisfy
            // only some of its occurrences, and the names an entity matched no
            // longer tell them apart (#1362). A branch is a query document, not
            // an aggregation expression, so it cannot tag rows in place;
            // instead each occurrence selects its entities in a pipeline of its
            // own, tagged with the occurrence's index, and an entity must come
            // back from every one of them.
            let occurrence = |index: usize, branch: Document| {
                let mut filter = entity_scope.clone();
                filter.extend(branch);
                vec![
                    doc! { "$match": filter },
                    doc! { "$group": { "_id": entity.clone() } },
                    doc! { "$addFields": { "occurrence": index as i32 } },
                ]
            };
            let required: Vec<i32> = (0..branches.len() as i32).collect();
            let mut occurrences = branches.into_iter().enumerate();
            let mut stages = occurrences
                .next()
                .map(|(index, branch)| occurrence(index, branch))
                .unwrap_or_default();
            for (index, branch) in occurrences {
                stages.push(doc! { "$unionWith": {
                    "coll": MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION,
                    "pipeline": occurrence(index, branch),
                }});
            }
            stages.push(doc! { "$group": {
                "_id": "$_id",
                "occurrences": { "$addToSet": "$occurrence" },
            }});
            stages.push(doc! { "$match": { "occurrences": { "$all": required } } });
            stages
        };
        if !composite_branches.is_empty() {
            // Each composite has already paired its components within one
            // contained entity. Intersect those entities with every plain
            // criterion, including repeated names and compartment membership.
            // `_id` is a local contained id and must constrain composite-only
            // searches too; the plain arm applies it in `entity_scope`.
            let mut occurrence_count = 0;
            if has_plain_branches {
                pipeline.push(doc! { "$addFields": { "occurrence": occurrence_count } });
                occurrence_count += 1;
            } else {
                pipeline.clear();
            }
            for mut composite in composite_branches {
                if !composite_id_clauses.is_empty() {
                    composite.push(doc! { "$match": { "$and": composite_id_clauses.clone() } });
                }
                composite.push(doc! { "$addFields": { "occurrence": occurrence_count } });
                if occurrence_count == 0 {
                    pipeline = composite;
                } else {
                    pipeline.push(doc! { "$unionWith": {
                        "coll": MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION,
                        "pipeline": composite,
                    }});
                }
                occurrence_count += 1;
            }
            let required: Vec<i32> = (0..occurrence_count).collect();
            pipeline.push(doc! { "$group": {
                "_id": "$_id",
                "occurrences": { "$addToSet": "$occurrence" },
            }});
            pipeline.push(doc! { "$match": { "occurrences": { "$all": required } } });
        }
        let sort = match contained_return {
            ContainedReturn::Container => {
                // Collapse the surviving per-entity slots to one per
                // container now that the per-entity AND has been applied.
                pipeline.push(doc! { "$group": {
                    "_id": { "rtype": "$_id.rtype", "rid": "$_id.rid" },
                }});
                if let Some(top_filter) = exclude_top_level {
                    // Check every container against the full live top-level
                    // predicate before pagination and the total facet. A
                    // different container type may use the same id, so only
                    // containers of the searched type can be overlaps.
                    pipeline.push(doc! { "$lookup": {
                        "from": MongoBackend::RESOURCES_COLLECTION,
                        "localField": "_id.rid",
                        "foreignField": "id",
                        "pipeline": [
                            { "$match": top_filter.clone() },
                            { "$limit": 1 },
                            { "$project": { "_id": 1 } },
                        ],
                        "as": "top_overlap",
                    }});
                    pipeline.push(doc! { "$match": { "$or": [
                        { "_id.rtype": { "$ne": contained_type } },
                        { "top_overlap": { "$eq": [] } },
                    ] } });
                }
                doc! { "_id.rtype": 1, "_id.rid": 1 }
            }
            ContainedReturn::Contained => doc! { "_id.rtype": 1, "_id.rid": 1, "_id.lid": 1 },
        };
        pipeline.push(doc! { "$sort": sort });
        let page_stages = vec![
            doc! { "$skip": offset as i64 },
            doc! { "$limit": limit as i64 },
        ];
        if want_total {
            pipeline
                .push(doc! { "$facet": { "page": page_stages, "total": [ { "$count": "n" } ] } });
        } else {
            pipeline.extend(page_stages);
        }

        let cursor = contained_rows
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to aggregate contained search")?;
        let docs = collect_documents(cursor).await?;

        let (page_docs, total): (Vec<Document>, Option<u64>) = if want_total {
            let facet = docs.into_iter().next().unwrap_or_default();
            let page = facet
                .get_array("page")
                .map(|a| a.iter().filter_map(|b| b.as_document().cloned()).collect())
                .unwrap_or_default();
            let n = facet
                .get_array("total")
                .ok()
                .and_then(|a| a.first())
                .and_then(|b| b.as_document())
                .and_then(|d| {
                    d.get_i64("n")
                        .ok()
                        .or_else(|| d.get_i32("n").ok().map(i64::from))
                })
                .unwrap_or(0);
            (page, Some(n.max(0) as u64))
        } else {
            (docs, None)
        };

        let mut keys = Vec::with_capacity(page_docs.len());
        for doc in page_docs {
            let Ok(id) = doc.get_document("_id") else {
                continue;
            };
            let rtype = id.get_str("rtype").unwrap_or_default().to_string();
            let rid = id.get_str("rid").unwrap_or_default().to_string();
            if rtype.is_empty() || rid.is_empty() {
                continue;
            }
            let lid = id.get_str("lid").ok().map(ToString::to_string);
            keys.push(ContainedKey { rtype, rid, lid });
        }
        Ok(ContainedPage { keys, total })
    }

    /// Fetches the containers for `keys` in one `find` on `resources`, in
    /// `keys` order, and shapes them per `contained_return`. Deleted or
    /// missing containers are skipped.
    async fn materialize_contained(
        &self,
        db: &mongodb::Database,
        tenant: &TenantContext,
        contained_type: &str,
        contained_return: crate::types::ContainedReturn,
        keys: &[ContainedKey],
    ) -> StorageResult<Vec<StoredResource>> {
        use crate::types::ContainedReturn;
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);

        let mut pairs: Vec<(String, String)> = keys
            .iter()
            .map(|k| (k.rtype.clone(), k.rid.clone()))
            .collect();
        pairs.sort();
        pairs.dedup();
        let or: Vec<Bson> = pairs
            .iter()
            .map(|(t, i)| Bson::Document(doc! { "resource_type": t, "id": i }))
            .collect();
        let cursor = resources
            .find(doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "is_deleted": { "$ne": true },
                "$or": or,
            })
            .hint(mongodb::options::Hint::Name(
                "idx_resources_identity".to_string(),
            ))
            .await
            .or_query_error("Failed to fetch contained-search containers")?;
        let mut by_key: HashMap<(String, String), StoredResource> = HashMap::new();
        for doc in collect_documents(cursor).await? {
            let rtype = doc.get_str("resource_type").unwrap_or_default().to_string();
            let stored = super::storage::document_to_stored_resource(&doc, tenant, &rtype)?;
            by_key.insert(
                (stored.resource_type().to_string(), stored.id().to_string()),
                stored,
            );
        }

        let mut items = Vec::with_capacity(keys.len());
        let mut seen: HashSet<String> = HashSet::new();
        for key in keys {
            let Some(container) = by_key.get(&(key.rtype.clone(), key.rid.clone())) else {
                continue;
            };
            match contained_return {
                ContainedReturn::Container => {
                    if seen.insert(format!("{}/{}", key.rtype, key.rid)) {
                        items.push(container.clone());
                    }
                }
                ContainedReturn::Contained => {
                    let Some(local_id) = &key.lid else {
                        continue;
                    };
                    if !seen.insert(format!("{}/{}#{}", key.rtype, key.rid, local_id)) {
                        continue;
                    }
                    if let Some(c) = extract_contained_resource(container.content(), local_id) {
                        items.push(build_contained_stored(
                            container,
                            contained_type,
                            local_id,
                            c,
                        ));
                    }
                }
            }
        }
        Ok(items)
    }

    fn validate_query_support(&self, query: &SearchQuery) -> StorageResult<()> {
        if query.parameters.iter().any(|param| !param.chain.is_empty()) {
            return Err(StorageError::Search(
                SearchError::ChainedSearchNotSupported {
                    chain: "forward chain".to_string(),
                },
            ));
        }

        if !query.reverse_chains.is_empty() {
            return Err(StorageError::Search(SearchError::ReverseChainNotSupported));
        }

        for param in &query.parameters {
            // `:in`/`:not-in` are unsupported for every parameter type.
            // `:above`/`:below` are served for `uri` by `build_uri_filter`
            // (segment-aware, mirroring SQLite/Elasticsearch, #1002) and for
            // `reference` by `build_reference_filter` (the same URL/path
            // hierarchy on the stored reference, #1408) but stay rejected for
            // token: token `:above`/`:below` need terminology subsumption,
            // which is not implemented here.
            // Every modifier on a composite (#1206) is rejected here except
            // `:missing`: `composite_search::component_param` hardcodes
            // `modifier: None` when building each component's filter, so any
            // other modifier (`:not`, `:exact`, `:above`, ...) would be
            // silently dropped rather than honoured if it reached the
            // composite planner. `:not` specifically also has no defined
            // composite semantics yet — unlike token `:not`, there is no
            // single positive filter whose complement is the right answer.
            // `:missing` is the one exception: `matching_resource_ids` routes
            // it into the separate `missing` list before `normal` params
            // (which is what feeds the composite driver/pair-check planner
            // this function guards) are even considered, so it is handled by
            // `missing_presence_filter` and never reaches this composite
            // guard's concern.
            let modifier_unsupported = matches!(
                param.modifier,
                Some(SearchModifier::In) | Some(SearchModifier::NotIn)
            ) || (matches!(
                param.modifier,
                Some(SearchModifier::Above) | Some(SearchModifier::Below)
            ) && !matches!(
                param.param_type,
                SearchParamType::Uri | SearchParamType::Reference
            )) || (param.param_type == SearchParamType::Composite
                && param
                    .modifier
                    .as_ref()
                    .is_some_and(|m| !matches!(m, SearchModifier::Missing)));
            if modifier_unsupported {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: param
                        .modifier
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    param_type: param.param_type.to_string(),
                }));
            }

            // `_id`/`_lastUpdated` are lowered outside the generic modifier
            // dispatch (see `build_resource_id_condition` /
            // `build_resource_last_updated_conditions`), which only knows
            // how to honour a narrow set of modifiers on them. Reject
            // anything else explicitly here rather than letting it fall
            // through to a builder that would silently misinterpret it
            // (#1055).
            let metadata_param_honoured = match param.name.as_str() {
                "_id" => matches!(
                    param.modifier,
                    None | Some(SearchModifier::Not) | Some(SearchModifier::Missing)
                ),
                "_lastUpdated" => {
                    matches!(param.modifier, None | Some(SearchModifier::Missing))
                }
                _ => true,
            };
            if !metadata_param_honoured {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: param
                        .modifier
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    param_type: param.param_type.to_string(),
                }));
            }
        }

        // The shared date gate: the same values are invalid here as on every
        // other backend, reported the same way (#1295).
        crate::search::validate_date_values(query)?;
        // And its numeric sibling (#1340).
        crate::search::validate_numeric_values(query)?;
        // And a value that is empty, or has an empty alternative: `family=Zzz,`
        // is a prefix match on `""`, which is every family name (#1380).
        crate::search::validate_value_presence(query)
    }

    /// Search with `_sort` on an indexed parameter (#881): pages over the id
    /// ordering computed in the search index, then fetches the page by id and
    /// restores the order. Offset-paginated; no page cursors are issued —
    /// cursor pagination with a custom sort is rejected at the entry point.
    ///
    /// The rows, `has_next` and `total` of one page all derive from a single
    /// id sequence: the ordering, narrowed by the search-index matches and by
    /// the resource-level predicates (`_id`, `_lastUpdated`, live-only) that
    /// `resource_level_ids` resolves (#1056).
    async fn search_param_sorted(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
        db: &mongodb::Database,
        tenant_id: &str,
        matched_ids: Option<HashSet<String>>,
        directive: &crate::types::SortDirective,
    ) -> StorageResult<SearchResult> {
        // Any filtered sort resolves its candidate set through the resources
        // collection: that folds in the resource-level predicates (`_id`,
        // `_lastUpdated`) AND makes the set live-only by construction, so a
        // stale search-index row can never occupy a page slot (#1056/#1040).
        // With no filter at all `allowed` stays `None` and the ordering is
        // computed over the whole type.
        let allowed: Option<HashSet<String>> = match matched_ids {
            Some(set) if set.is_empty() => Some(set),
            Some(set) => Some(
                self.resource_level_ids(db, tenant_id, &query.resource_type, query, Some(&set))
                    .await?,
            ),
            None if Self::has_resource_level_params(query) => Some(
                self.resource_level_ids(db, tenant_id, &query.resource_type, query, None)
                    .await?,
            ),
            None => None,
        };

        // Nothing can match: skip the ordering aggregation entirely.
        let ordered: Vec<String> = if allowed.as_ref().is_some_and(|set| set.is_empty()) {
            Vec::new()
        } else {
            self.param_sorted_ids(
                db,
                tenant_id,
                &query.resource_type,
                directive,
                allowed.as_ref(),
            )
            .await?
        };

        let page_size = query.count.unwrap_or(100).max(1) as usize;
        let offset = query.offset.unwrap_or(0) as usize;
        let has_previous = offset > 0;
        let mut page_ids: Vec<String> = ordered
            .iter()
            .skip(offset)
            .take(page_size + 1)
            .cloned()
            .collect();
        let has_next = page_ids.len() > page_size;
        page_ids.truncate(page_size);

        // Fetch the page's documents and restore the computed order.
        let id_set: HashSet<String> = page_ids.iter().cloned().collect();
        let filter = self.build_resource_filter(
            tenant_id,
            &query.resource_type,
            query,
            Some(&id_set),
            None,
        )?;
        let resources_coll = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let docs = collect_documents(
            resources_coll
                .find(filter)
                .await
                .or_query_error("Failed to execute MongoDB search")?,
        )
        .await?;
        let mut by_id: std::collections::HashMap<String, StoredResource> = docs
            .into_iter()
            .map(|doc| self.document_to_stored_resource(tenant, &query.resource_type, doc))
            .collect::<StorageResult<Vec<_>>>()?
            .into_iter()
            .map(|r| (r.id().to_string(), r))
            .collect();
        let resources: Vec<StoredResource> =
            page_ids.iter().filter_map(|id| by_id.remove(id)).collect();

        // `ordered` is exactly the sequence the page was cut from, so its
        // length is the total by construction — no second resolution.
        let total = if query.wants_total() {
            Some(ordered.len() as u64)
        } else {
            None
        };
        let page_info = PageInfo {
            next_cursor: None,
            previous_cursor: None,
            total,
            has_next,
            has_previous,
        };
        let page = Page::new(resources, page_info);

        let mut included: Vec<StoredResource> = Vec::new();
        if !query.includes.is_empty() {
            let forward: Vec<IncludeDirective> = query
                .includes
                .iter()
                .filter(|i| i.include_type == IncludeType::Include)
                .cloned()
                .collect();
            if !forward.is_empty() {
                let resolved = self
                    .resolve_forward_includes_capped(tenant, &page.items, &forward)
                    .await?;
                Self::merge_unique(&mut included, resolved);
            }
            let reverse: Vec<IncludeDirective> = query
                .includes
                .iter()
                .filter(|i| i.include_type == IncludeType::Revinclude)
                .cloned()
                .collect();
            if !reverse.is_empty() {
                let resolved = self
                    .resolve_revincludes(tenant, &page.items, &reverse)
                    .await?;
                Self::merge_unique(&mut included, resolved);
            }
        }

        Ok(SearchResult {
            resources: page,
            included,
            total,
            scores: Default::default(),
        })
    }

    /// True when the query carries a predicate that lives on the resource
    /// document rather than in the search index.
    fn has_resource_level_params(query: &SearchQuery) -> bool {
        query
            .parameters
            .iter()
            .any(|p| matches!(p.name.as_str(), "_id" | "_lastUpdated"))
    }

    /// Ids of the live resources that satisfy the resource-level predicates
    /// (`_id`, `_lastUpdated`) within `matched_ids` — also the liveness
    /// filter for every filtered parameter sort. This is the same predicate
    /// `search_count` counts, so a page cut from this set and its `total`
    /// agree by construction (#1056).
    async fn resource_level_ids(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        query: &SearchQuery,
        matched_ids: Option<&HashSet<String>>,
    ) -> StorageResult<HashSet<String>> {
        let filter =
            self.build_resource_filter(tenant_id, resource_type, query, matched_ids, None)?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let cursor = resources
            .find(filter)
            .projection(doc! { "_id": 0, "id": 1 })
            .await
            .or_query_error("Failed to resolve _id/_lastUpdated filters")?;
        let docs = collect_documents(cursor).await?;
        Ok(docs
            .into_iter()
            .filter_map(|d| d.get_str("id").ok().map(ToString::to_string))
            .collect())
    }

    /// Distinct `resource_id`s in the search index matching `filter`.
    async fn distinct_resource_ids(
        &self,
        search_index: &mongodb::Collection<Document>,
        filter: Document,
    ) -> StorageResult<HashSet<String>> {
        Ok(search_index
            .distinct("resource_id", filter)
            .await
            .or_query_error("Failed to query search_index")?
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect())
    }

    /// Every live resource id of the type — the universe `:missing=true` and
    /// `:not` complement against (#881).
    async fn all_resource_ids(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
    ) -> StorageResult<HashSet<String>> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        Ok(resources
            .distinct(
                "id",
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "is_deleted": false,
                },
            )
            .await
            .or_query_error("Failed to enumerate resource ids")?
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect())
    }

    /// Resource ids ordered by an indexed search parameter's value (#881):
    /// grouped per resource in the search index taking the smallest value
    /// for ascending sorts and the largest for descending (the SQL backends'
    /// MIN/MAX), with resources that have no value for the parameter
    /// appended last in id order.
    ///
    /// With `allowed` — the ids the query matched — the aggregation is
    /// bounded to that set (`resource_id: {$in: chunk}`, hinted onto
    /// `idx_search_composite`), the per-resource keys are ordered client-side
    /// and the unkeyed tail is taken from `allowed` itself, so the cost is
    /// proportional to the result set rather than to the resource type
    /// (#1040). Without `allowed` (no filter at all) the ordering is still
    /// computed over the whole type.
    async fn param_sorted_ids(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        directive: &crate::types::SortDirective,
        allowed: Option<&HashSet<String>>,
    ) -> StorageResult<Vec<String>> {
        use crate::types::SortDirection;

        // The writer stores quantities in `value_quantity_value`, not
        // `value_number`; mapping them to the latter made every quantity
        // sort degrade silently to id order (#1040). `value_field_for`
        // covers that; a missing/composite/special param type falls back to
        // `value_string`, matching the old catch-all arm.
        let value_field = directive
            .param_type
            .and_then(value_field_for)
            .unwrap_or("value_string");
        let (accumulator, order) = match directive.direction {
            SortDirection::Ascending => ("$min", 1),
            SortDirection::Descending => ("$max", -1),
        };
        let sort_key = sort_key_expression(value_field, directive.direction);
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);

        if let Some(allowed) = allowed {
            // Bounded path (#1040): one `$match`+`$group` per chunk of the
            // candidate set, then order the keys client-side. Chunks are cut
            // from a sorted view of the set so the sequence of commands is
            // deterministic for a given query.
            let mut candidates: Vec<&String> = allowed.iter().collect();
            candidates.sort();
            let mut keyed: Vec<(String, Bson)> = Vec::with_capacity(allowed.len());
            for chunk in candidates.chunks(SORT_ID_CHUNK) {
                let ids: Vec<Bson> = chunk.iter().map(|id| Bson::String((*id).clone())).collect();
                let pipeline = vec![
                    doc! { "$match": {
                        "tenant_id": tenant_id,
                        "resource_type": resource_type,
                        "resource_id": { "$in": Bson::Array(ids) },
                        "param_name": &directive.parameter,
                        value_field: { "$ne": Bson::Null },
                    }},
                    doc! { "$group": {
                        "_id": "$resource_id",
                        "key": { accumulator: sort_key.clone() },
                    }},
                ];
                let cursor = search_index
                    .aggregate(pipeline)
                    .hint(mongodb::options::Hint::Name(
                        "idx_search_composite".to_string(),
                    ))
                    .await
                    .or_query_error("Failed to sort by search parameter")?;
                for d in collect_documents(cursor).await? {
                    if let (Ok(id), Some(key)) = (d.get_str("_id"), d.get("key")) {
                        keyed.push((id.to_string(), key.clone()));
                    }
                }
            }
            let mut ordered = order_sort_keys(keyed, directive.direction);
            let keyed_set: HashSet<String> = ordered.iter().cloned().collect();
            let mut unkeyed: Vec<String> = allowed
                .iter()
                .filter(|id| !keyed_set.contains(*id))
                .cloned()
                .collect();
            unkeyed.sort();
            ordered.extend(unkeyed);
            // `allowed` is live-only by construction — `search_param_sorted`
            // resolves every filtered candidate set through the resources
            // collection with `is_deleted: false` — so no liveness pass is
            // needed here.
            return Ok(ordered);
        }

        // Unfiltered sort: the ordering is still computed over the whole
        // type. Unchanged from #881/#1056 apart from the value-field mapping.
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "param_name": &directive.parameter,
                value_field: { "$ne": Bson::Null },
            }},
            doc! { "$group": {
                "_id": "$resource_id",
                "key": { accumulator: sort_key.clone() },
            }},
            doc! { "$sort": { "key": order, "_id": 1 } },
            doc! { "$project": { "_id": 1 } },
        ];
        let cursor = search_index
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to sort by search parameter")?;
        let docs = collect_documents(cursor).await?;
        let mut ordered: Vec<String> = docs
            .into_iter()
            .filter_map(|d| d.get_str("_id").ok().map(ToString::to_string))
            .collect();

        // A search-index row whose resource is gone (deleted, or a stale
        // entry) must not occupy a slot in the sequence: the page fetch would
        // drop it and the page would come back short (#1056).
        let live = self.all_resource_ids(db, tenant_id, resource_type).await?;
        ordered.retain(|id| live.contains(id));

        // Resources without a value for the parameter sort last.
        let keyed: HashSet<String> = ordered.iter().cloned().collect();
        let mut unkeyed: Vec<String> = live.into_iter().filter(|id| !keyed.contains(id)).collect();
        unkeyed.sort();
        ordered.extend(unkeyed);
        Ok(ordered)
    }

    async fn matching_resource_ids(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        query: &SearchQuery,
    ) -> StorageResult<Option<HashSet<String>>> {
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);

        // Defence in depth behind `validate_value_presence` (#1380): an empty
        // value is a prefix of every string, so a parameter carrying one
        // matches nothing — and with it the search, parameters being ANDed —
        // rather than whatever the filters below would make of it (`:not`
        // included: "nothing" negates into "everything").
        if query.parameters.iter().any(crate::search::has_empty_value) {
            return Ok(Some(HashSet::new()));
        }

        self.preflight_legacy_composites(
            db,
            tenant_id,
            resource_type,
            &query.parameters,
            false,
            None,
        )
        .await?;

        let mut normal: Vec<&SearchParameter> = Vec::new();
        let mut missing: Vec<&SearchParameter> = Vec::new();
        let mut not_params: Vec<&SearchParameter> = Vec::new();

        for param in &query.parameters {
            if matches!(param.name.as_str(), "_id" | "_lastUpdated") {
                continue;
            }
            match &param.modifier {
                Some(SearchModifier::Missing) => missing.push(param),
                Some(SearchModifier::Not) => not_params.push(param),
                _ => normal.push(param),
            }
        }

        let has_compartment = query
            .compartment
            .as_ref()
            .is_some_and(|c| !c.params.is_empty() && !c.reference.is_empty());

        if normal.is_empty() && missing.is_empty() && not_params.is_empty() && !has_compartment {
            return Ok(None);
        }

        // :missing=true and :not require complementing against the full resource
        // universe. When no normal params exist to drive paging, fall back to the
        // complement-only path that materialises the universe via distinct().
        if normal.is_empty() {
            return self
                .matching_resource_ids_complement_only(
                    db,
                    &search_index,
                    tenant_id,
                    resource_type,
                    &missing,
                    &not_params,
                    query,
                )
                .await;
        }

        // Reference `:identifier` (#1408) names its targets by their
        // identifier, which a filter document cannot join on: each such
        // parameter is resolved to its complete filter here, once, and no
        // target at all empties the search (parameters are ANDed).
        let mut identifier_filters: HashMap<usize, Document> = HashMap::new();
        for (i, param) in normal.iter().enumerate() {
            if param.param_type == SearchParamType::Reference
                && matches!(param.modifier, Some(SearchModifier::Identifier))
            {
                match self
                    .resolve_reference_identifier(&search_index, tenant_id, resource_type, param)
                    .await?
                {
                    Some(filter) => identifier_filters.insert(i, filter),
                    None => return Ok(Some(HashSet::new())),
                };
            }
        }
        let normal_filter = |i: usize| -> StorageResult<Document> {
            match identifier_filters.get(&i) {
                Some(filter) => Ok(filter.clone()),
                None => self.build_search_index_filter(tenant_id, resource_type, normal[i]),
            }
        };

        // #1206: a composite's probe must run over its component filters
        // regardless of how many normal params there are — unlike a plain
        // param, its own filter isn't a single document to count, and the
        // most-selective-arm choice inside the composite still matters even
        // when it is the only normal param. Probes are cached here so the
        // composite driver, if chosen, doesn't re-run them below.
        let mut composite_probes: HashMap<usize, (Document, u64)> = HashMap::new();

        let driver_idx = if normal.len() == 1 && normal[0].param_type != SearchParamType::Composite
        {
            0
        } else {
            let mut best: Option<(usize, u64)> = None;
            for (i, param) in normal.iter().enumerate() {
                let count = if param.param_type == SearchParamType::Composite {
                    match self
                        .composite_driver_probe(
                            &search_index,
                            tenant_id,
                            resource_type,
                            param,
                            PROBE_ROW_LIMIT,
                            None,
                        )
                        .await?
                    {
                        None => return Ok(Some(HashSet::new())),
                        Some((filter, count)) => {
                            composite_probes.insert(i, (filter, count));
                            count
                        }
                    }
                } else {
                    let filter = normal_filter(i)?;
                    let count = search_index
                        .count_documents(filter)
                        .limit(PROBE_ROW_LIMIT)
                        .await
                        .or_query_error("Failed to probe search_index for driver selection")?;
                    if count == 0 {
                        return Ok(Some(HashSet::new()));
                    }
                    count
                };
                if best.is_none_or(|(_, prev)| count < prev) {
                    best = Some((i, count));
                }
            }
            best.map(|(i, _)| i).unwrap_or(0)
        };

        // Every composite index visited by the loop above has its probe
        // result cached, so `driver_idx` pointing at a composite always finds
        // an entry here; a plain param never has one and falls through to
        // the ordinary per-value filter builder.
        let driver_filter = if let Some((filter, _)) = composite_probes.remove(&driver_idx) {
            filter
        } else {
            normal_filter(driver_idx)?
        };

        let mut driver_cursor = search_index
            .find(driver_filter)
            .projection(doc! { "resource_id": 1, "_id": 0 })
            .await
            .or_query_error("Failed to open driver cursor")?;

        let mut confirmed: HashSet<String> = HashSet::new();

        loop {
            let batch_docs = read_cursor_batch(&mut driver_cursor, CANDIDATE_BATCH_SIZE).await?;
            let docs_read = batch_docs.len();

            if docs_read == 0 {
                break;
            }

            let mut candidates: HashSet<String> = HashSet::new();
            for doc in &batch_docs {
                if let Ok(rid) = doc.get_str("resource_id") {
                    candidates.insert(rid.to_string());
                }
            }

            for (i, param) in normal.iter().enumerate() {
                if candidates.is_empty() {
                    continue;
                }
                // A composite's driver arm only proves ONE component
                // matched (its most selective one) — every composite in
                // `normal`, including the driver, still needs the grouped
                // pair check to confirm every component matched within the
                // same `composite_group` (#1206).
                if param.param_type == SearchParamType::Composite {
                    let passing = self
                        .composite_pair_check(
                            &search_index,
                            tenant_id,
                            resource_type,
                            param,
                            &candidates,
                            None,
                        )
                        .await?;
                    candidates.retain(|id| passing.contains(id));
                    continue;
                }
                if i == driver_idx {
                    continue;
                }
                let param_filter = normal_filter(i)?;
                let bounded = doc! {
                    "$and": [
                        param_filter,
                        { "resource_id": { "$in": candidates.iter().cloned().collect::<Vec<_>>() } }
                    ]
                };
                let passing: HashSet<String> = search_index
                    .distinct("resource_id", bounded)
                    .await
                    .or_query_error("Failed to intersect search_index")?
                    .into_iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect();
                candidates.retain(|id| passing.contains(id));
            }

            // :missing — check per batch against surviving candidates.
            for param in &missing {
                if candidates.is_empty() {
                    break;
                }
                let wants_missing = param
                    .values
                    .first()
                    .map(|v| v.value == "true")
                    .unwrap_or(false);
                let with_entry: HashSet<String> = search_index
                    .distinct(
                        "resource_id",
                        doc! {
                            "tenant_id": tenant_id,
                            "resource_type": resource_type,
                            "param_name": &param.name,
                            "resource_id": { "$in": candidates.iter().cloned().collect::<Vec<_>>() },
                        },
                    )
                    .await
                    .or_query_error("Failed to check :missing")?
                    .into_iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect();
                if wants_missing {
                    candidates.retain(|id| !with_entry.contains(id));
                } else {
                    candidates.retain(|id| with_entry.contains(id));
                }
            }

            // :not — per the spec, includes resources with no value for the
            // parameter at all (#881). Check per batch against surviving candidates.
            for param in &not_params {
                if candidates.is_empty() {
                    break;
                }
                let mut positive = (*param).clone();
                positive.modifier = None;
                let pos_filter =
                    self.build_search_index_filter(tenant_id, resource_type, &positive)?;
                let bounded = doc! {
                    "$and": [
                        pos_filter,
                        { "resource_id": { "$in": candidates.iter().cloned().collect::<Vec<_>>() } }
                    ]
                };
                let matching: HashSet<String> = search_index
                    .distinct("resource_id", bounded)
                    .await
                    .or_query_error("Failed to check :not")?
                    .into_iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect();
                candidates.retain(|id| !matching.contains(id));
            }

            // Compartment — checked per batch against surviving candidates.
            if has_compartment {
                if let Some(comp) = &query.compartment {
                    if !candidates.is_empty() {
                        let base = strip_reference_version(&comp.reference);
                        let params: Vec<Bson> =
                            comp.params.iter().cloned().map(Bson::String).collect();
                        let comp_filter = doc! {
                            "tenant_id": tenant_id,
                            "resource_type": resource_type,
                            "param_name": { "$in": Bson::Array(params) },
                            "resource_id": { "$in": candidates.iter().cloned().collect::<Vec<_>>() },
                            "$or": [
                                { "value_reference": &base },
                                { "value_reference": {
                                    "$regex": format!("^{}/_history/", regex_escape(base))
                                }},
                            ],
                        };
                        let in_comp: HashSet<String> = search_index
                            .distinct("resource_id", comp_filter)
                            .await
                            .or_query_error("Failed to check compartment membership")?
                            .into_iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect();
                        candidates.retain(|id| in_comp.contains(id));
                    }
                }
            }

            confirmed.extend(candidates);

            if confirmed.len() > MAX_RESULT_ID_SET {
                return Err(StorageError::Search(SearchError::TooManyResults {
                    count: confirmed.len(),
                    max: MAX_RESULT_ID_SET,
                }));
            }

            if docs_read < CANDIDATE_BATCH_SIZE {
                break;
            }
        }

        Ok(Some(confirmed))
    }

    #[allow(clippy::too_many_arguments)]
    async fn matching_resource_ids_complement_only(
        &self,
        db: &mongodb::Database,
        search_index: &mongodb::Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
        missing: &[&SearchParameter],
        not_params: &[&SearchParameter],
        query: &SearchQuery,
    ) -> StorageResult<Option<HashSet<String>>> {
        let has_compartment = query
            .compartment
            .as_ref()
            .is_some_and(|c| !c.params.is_empty() && !c.reference.is_empty());
        let mut matched: Option<HashSet<String>> = None;

        for param in missing {
            let wants_missing = param
                .values
                .first()
                .map(|v| v.value == "true")
                .unwrap_or(false);
            let with_entry = self
                .distinct_resource_ids(
                    search_index,
                    missing_presence_filter(tenant_id, resource_type, param),
                )
                .await?;
            let ids = if wants_missing {
                let all = self.all_resource_ids(db, tenant_id, resource_type).await?;
                all.difference(&with_entry).cloned().collect::<HashSet<_>>()
            } else {
                with_entry
            };
            if ids.is_empty() {
                return Ok(Some(HashSet::new()));
            }
            matched = Some(match matched {
                Some(current) => current.intersection(&ids).cloned().collect(),
                None => ids,
            });
            if matched.as_ref().is_some_and(|s| s.is_empty()) {
                return Ok(matched);
            }
        }

        for param in not_params {
            let mut positive = (*param).clone();
            positive.modifier = None;
            let filter = self.build_search_index_filter(tenant_id, resource_type, &positive)?;
            let matching = self.distinct_resource_ids(search_index, filter).await?;
            let all = self.all_resource_ids(db, tenant_id, resource_type).await?;
            let ids = all.difference(&matching).cloned().collect::<HashSet<_>>();
            if ids.is_empty() {
                return Ok(Some(HashSet::new()));
            }
            matched = Some(match matched {
                Some(current) => current.intersection(&ids).cloned().collect(),
                None => ids,
            });
            if matched.as_ref().is_some_and(|s| s.is_empty()) {
                return Ok(matched);
            }
        }

        if has_compartment {
            if let Some(comp) = &query.compartment {
                if let Some(ids) = self
                    .compartment_resource_ids(search_index, tenant_id, resource_type, comp)
                    .await?
                {
                    if ids.is_empty() {
                        return Ok(Some(HashSet::new()));
                    }
                    matched = Some(match matched {
                        Some(current) => current.intersection(&ids).cloned().collect(),
                        None => ids,
                    });
                }
            }
        }

        Ok(matched)
    }

    /// Returns the resource IDs that are members of the compartment described by
    /// `comp`: resources that reference `comp.reference` through ANY of the
    /// membership params (logical OR). Reference matching is version-agnostic
    /// (mirrors the reference handler): the stored reference must equal the base
    /// reference or carry a `/_history/<vid>` suffix.
    ///
    /// Returns `Ok(None)` when `comp` carries no params or no reference (no
    /// restriction to apply), otherwise `Ok(Some(ids))` (possibly empty).
    async fn compartment_resource_ids(
        &self,
        search_index: &mongodb::Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
        comp: &CompartmentMembership,
    ) -> StorageResult<Option<HashSet<String>>> {
        if comp.params.is_empty() || comp.reference.is_empty() {
            return Ok(None);
        }

        let base = strip_reference_version(&comp.reference);
        let params: Vec<Bson> = comp.params.iter().cloned().map(Bson::String).collect();

        let filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "param_name": { "$in": Bson::Array(params) },
            "$or": [
                { "value_reference": &base },
                { "value_reference": { "$regex": format!("^{}/_history/", regex_escape(base)) } },
            ],
        };

        let ids = search_index
            .distinct("resource_id", filter)
            .await
            .or_query_error("Failed to query search_index")?
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect::<HashSet<_>>();

        Ok(Some(ids))
    }

    pub(super) fn build_search_index_filter(
        &self,
        tenant_id: &str,
        resource_type: &str,
        param: &SearchParameter,
    ) -> StorageResult<Document> {
        if param.param_type == SearchParamType::Composite {
            return Err(internal_error(format!(
                "build_search_index_filter must never be called with a composite parameter \
                 ('{}'); composite parameters are planned by composite_component_filters/ \
                 composite_driver_probe/composite_pair_check instead",
                param.name
            )));
        }

        if param.values.is_empty() {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!("Search parameter '{}' has no values", param.name),
            }));
        }

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "param_name": &param.name,
        };

        // #1083: resolve the parameter's declared reference target types once,
        // up front, so `build_reference_filter` can emit index-bounded `$in`/
        // anchored-regex arms for the bare-id form instead of an unanchored
        // scan. Only the bare-id branch (no modifier) needs this — every other
        // reference modifier (`:contains`, `:text`, ...) never reaches it. The
        // read lock is dropped here (the `Vec` is owned) before the value loop.
        let targets: Vec<String> =
            if param.param_type == SearchParamType::Reference && param.modifier.is_none() {
                let registry = self.tenant_registry(tenant_id);
                let registry = registry.read();
                crate::search::resolve_param_targets(&registry, resource_type, &param.name)
            } else {
                Vec::new()
            };

        let value_filters = param
            .values
            .iter()
            .map(|value| self.build_index_value_filter(param, value, &targets))
            .collect::<StorageResult<Vec<_>>>()?;

        // #1391: a date value can be several alternatives of its own (`ge`,
        // `le`, `ne`), and a comma list adds more. MongoDB 5.0 answers an
        // `$or` nested under the shared tenant/type/param conjuncts by
        // reading the documents, not as a covered scan; an `$or` at the top,
        // every arm carrying the scope itself, plans one index scan per arm.
        // The alternatives and their OR are unchanged; only where they sit.
        if param.param_type == SearchParamType::Date
            && (value_filters.len() > 1 || value_filters.iter().any(|f| f.contains_key("$or")))
        {
            return Ok(scoped_date_alternatives(&filter, value_filters));
        }

        if value_filters.len() == 1 {
            if let Some(single) = value_filters.into_iter().next() {
                for (key, value) in single {
                    filter.insert(key, value);
                }
            }
            return Ok(filter);
        }

        // #1062: FHIR comma-separated values are OR for every parameter type
        // (https://build.fhir.org/search.html#combining) — Date and Number
        // used to get `$and` here, which was not merely stricter but wrong
        // in a way that emptied the whole result. This filter is evaluated
        // against a single `search_index` document (one value of one
        // parameter, for one resource), so a disjoint AND can never be
        // satisfied by any row: `distinct_resource_ids` then returns
        // nothing, and `matching_resource_ids`'s empty-set short circuit
        // empties the *entire* search result, not just this parameter. The
        // AND semantics callers actually want is the *repeated*-parameter
        // form (`?date=ge2020&date=le2021`), which arrives as separate
        // `SearchParameter`s and is intersected in `matching_resource_ids` —
        // unaffected by this change. Behavior change: a range-shaped comma
        // list (`?date=ge2020,le2021`) widens from "in 2020-2021" to "any
        // date >= 2020 OR any date <= 2021"; use the repeated form above for
        // a closed range.
        filter.insert(
            "$or",
            Bson::Array(value_filters.into_iter().map(Bson::Document).collect()),
        );

        Ok(filter)
    }

    /// Builds every component's scoped `search_index` filter for a composite
    /// parameter (#1206) — outer index is the (comma-OR'd) value, inner index
    /// is the component, in declaration order.
    ///
    /// Each returned document is a *full* filter (`tenant_id`,
    /// `resource_type`, `param_name` = the composite's own name, plus the
    /// component's typed predicate) — every row for every component of a
    /// composite shares `param_name` with the composite itself. Repeated
    /// component types also require their declared `composite_slot`.
    ///
    /// Every predicate is additionally ANDed with `{value_field: {"$ne":
    /// null}}` for the component's own value field (review finding: an
    /// absence-shaped predicate like `Ne` — `build_quantity_filter`/
    /// `build_number_filter` emit `{field: {"$not": {...}}}`,
    /// `build_date_filter_doc` emits `{field: {"$ne": start}}` for an
    /// instant — is satisfied by a document where `field` does not exist at
    /// all. Unlike a plain parameter (where `param_name` alone already
    /// scopes a filter to rows of one value type, see
    /// `ne_filter_stays_scoped_to_tenant_resource_and_param`), every
    /// component of a composite shares the *same* `param_name`, so without
    /// this conjunct a `ne` component filter would also be satisfied by a
    /// sibling component's row in the same `composite_group` — e.g.
    /// `code-value-quantity=http://loinc.org|8302-2$ne150` would match the
    /// token row for a resource whose value IS 150, because that token row
    /// has no `value_quantity_value` field to fail the `$not`. `$ne: null`
    /// (not `$exists`) is required: it is what the generation-2 partial
    /// value indexes are built to cover, and applying it uniformly (not only
    /// for `Ne`) keeps every component type on one code path. A component
    /// type with no indexed value field (`Composite`, `Special` — neither
    /// occurs today, since the registry does not nest composites and
    /// `Special` cannot be a composite's declared sub-parameter type) is
    /// rejected explicitly rather than silently skipping the conjunct.
    ///
    /// Errors from the typed builders (e.g. a malformed quantity component)
    /// propagate: a malformed composite value is a 400, same as a malformed
    /// plain value.
    ///
    /// Also returns, per component, whether its parsed value used the `Ne`
    /// prefix (`negated`) — quantity/number `Ne` builds `{field: {"$not":
    /// ...}}` and date `Ne` builds `{field: {"$ne": ...}}`, both of which are
    /// bounded only by "field exists", not by the value itself. Callers use
    /// this to keep an unbounded `ne` arm out of the driver-probe role; see
    /// `composite_driver_probe`.
    fn composite_component_filters(
        &self,
        tenant_id: &str,
        resource_type: &str,
        param: &SearchParameter,
    ) -> StorageResult<Vec<Vec<ComponentFilter>>> {
        if param.values.is_empty() {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!("Search parameter '{}' has no values", param.name),
            }));
        }

        let mut result = Vec::with_capacity(param.values.len());
        // Match the extractor's per-type, declaration-order slot numbering.
        // A unique type does not need a slot predicate, so old rows for
        // composites with distinct component types remain searchable.
        let mut counts = HashMap::<SearchParamType, usize>::new();
        for component in &param.components {
            *counts.entry(component.param_type).or_default() += 1;
        }
        for value in &param.values {
            let component_values =
                super::composite_search::split_composite_value(&value.value, &param.components)?;

            let mut per_component = Vec::with_capacity(param.components.len());
            let mut seen = HashMap::<SearchParamType, i32>::new();
            for (component, component_value) in param.components.iter().zip(component_values) {
                let negated = component_value.prefix == SearchPrefix::Ne;
                let value_field = value_field_for(component.param_type).ok_or_else(|| {
                    StorageError::Search(SearchError::InvalidComposite {
                        message: format!(
                            "composite component '{}' of parameter '{}' has type {:?}, which \
                             has no indexed value field and cannot be scoped as a composite \
                             predicate",
                            component.param_name, param.name, component.param_type
                        ),
                    })
                })?;

                let targets: Vec<String> = if component.param_type == SearchParamType::Reference {
                    let registry = self.tenant_registry(tenant_id);
                    let registry = registry.read();
                    crate::search::resolve_param_targets(
                        &registry,
                        resource_type,
                        &component.param_name,
                    )
                } else {
                    Vec::new()
                };

                let synthetic = super::composite_search::component_param(
                    param,
                    component,
                    component_value.clone(),
                );
                // A composite's date component is compared as a point on
                // `value_date`, as on every backend: only a standalone date
                // parameter is range-aware (#1391).
                let predicate = if component.param_type == SearchParamType::Date {
                    self.build_date_filter(&component_value, &synthetic.name, "value_date")?
                } else {
                    self.build_index_value_filter(&synthetic, &component_value, &targets)?
                };

                let mut scoped = doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "param_name": &param.name,
                };
                let slot = seen.entry(component.param_type).or_default();
                *slot += 1;
                if counts[&component.param_type] > 1 {
                    scoped.insert("composite_slot", *slot);
                }
                scoped.insert(
                    "$and",
                    vec![
                        Bson::Document(doc! { value_field: { "$ne": Bson::Null } }),
                        Bson::Document(predicate),
                    ],
                );
                per_component.push(ComponentFilter {
                    filter: scoped,
                    negated,
                });
            }
            result.push(per_component);
        }
        Ok(result)
    }

    /// Reject a repeated-type composite if an older matching component row
    /// lacks its slot. Without this probe a slot-constrained query can report
    /// a false negative until the tenant's index is rebuilt with `$reindex`.
    pub(super) async fn preflight_legacy_composites(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        parameters: &[SearchParameter],
        contained: bool,
        mut session: Option<&mut mongodb::ClientSession>,
    ) -> StorageResult<()> {
        let collection = db.collection::<Document>(if contained {
            MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION
        } else {
            MongoBackend::SEARCH_INDEX_COLLECTION
        });
        let probe_index = if contained {
            CONTAINED_COMPOSITE_SLOT_PROBE_INDEX
        } else {
            COMPOSITE_SLOT_PROBE_INDEX
        };
        for param in parameters
            .iter()
            .filter(|p| p.param_type == SearchParamType::Composite && p.modifier.is_none())
        {
            let mut seen = HashSet::new();
            if !param
                .components
                .iter()
                .any(|component| !seen.insert(component.param_type))
            {
                continue;
            }
            let normal_filters =
                self.composite_component_filters(tenant_id, resource_type, param)?;
            if !contained
                && normal_filters
                    .iter()
                    .any(|value| value.iter().all(|c| c.negated))
            {
                // The driver planner's existing all-ne error takes precedence.
                continue;
            }
            let filters = if contained {
                self.contained_composite_component_filters(tenant_id, resource_type, param)?
                    .into_iter()
                    .map(|value| value.into_iter().map(|c| c.filter).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            } else {
                normal_filters
                    .into_iter()
                    .map(|value| value.into_iter().map(|c| c.filter).collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            };
            for value in filters {
                for mut filter in value {
                    if filter.remove("composite_slot").is_none() {
                        continue;
                    }
                    // Keep the typed value predicate: a row for another
                    // candidate value does not require this query to fail.
                    let legacy = doc! { "$and": [
                        filter,
                        { "composite_slot": { "$exists": false } },
                        { "composite_group": { "$exists": true } },
                    ] };
                    let found = match session.as_deref_mut() {
                        Some(s) => {
                            collection
                                .find_one(legacy)
                                .hint(mongodb::options::Hint::Name(probe_index.to_string()))
                                .session(s)
                                .await
                        }
                        None => {
                            collection
                                .find_one(legacy)
                                .hint(mongodb::options::Hint::Name(probe_index.to_string()))
                                .await
                        }
                    }
                    .or_query_error("Failed to probe legacy MongoDB composite rows")?;
                    if found.is_some() {
                        return Err(StorageError::Search(SearchError::InvalidComposite {
                            message: format!(
                                "composite search parameter '{}' has rows without component slots; run $reindex for this tenant",
                                param.name
                            ),
                        }));
                    }
                }
            }
        }
        Ok(())
    }

    /// Counts documents matching `filter`, bounded by `limit`, using
    /// `count_documents` outside a transaction or an aggregate
    /// `$match/$limit/$group/$count` probe inside one — MongoDB's `count`
    /// command cannot run inside a multi-document transaction, which is
    /// exactly why the ifNoneExist matcher in `storage.rs` already uses the
    /// aggregate form when it has a session.
    ///
    /// The two branches count different things and are NOT interchangeable:
    /// the session branch `$group`s by `$resource_id` before `$count`, so it
    /// counts distinct *resource ids* — matching what the ifNoneExist
    /// matcher's own probe (`storage.rs`, same `$group`/`$count` shape)
    /// compares against; the no-session branch's `count_documents` counts
    /// `search_index` *rows*, matching what `matching_resource_ids`'s plain
    /// per-param probe (`search_index.count_documents(filter)`, no
    /// grouping) compares against. A composite's driver-arm probe can
    /// return more rows than resources (a composite_group's several
    /// component rows share one resource_id), so each caller must compare
    /// this count only against its own kind of probe, never mix the two.
    async fn probe_count(
        &self,
        search_index: &mongodb::Collection<Document>,
        filter: Document,
        limit: u64,
        session: Option<&mut mongodb::ClientSession>,
    ) -> StorageResult<u64> {
        match session {
            Some(s) => {
                let pipeline = vec![
                    doc! { "$match": filter },
                    doc! { "$limit": limit as i64 },
                    doc! { "$group": { "_id": "$resource_id" } },
                    doc! { "$count": "n" },
                ];
                let cursor = search_index
                    .aggregate(pipeline)
                    .session(&mut *s)
                    .await
                    .or_query_error(
                        "Failed to probe search_index for composite driver (session)",
                    )?;
                let docs = super::storage::collect_session_documents(cursor, s).await?;
                Ok(docs
                    .first()
                    .and_then(|d| d.get_i32("n").ok())
                    .map(|n| n as u64)
                    .unwrap_or(0))
            }
            None => search_index
                .count_documents(filter)
                .limit(limit)
                .await
                .or_query_error("Failed to probe search_index for composite driver"),
        }
    }

    /// Selects the most selective *non-`ne`* component filter for each value
    /// of a composite parameter and returns the combined driver filter plus
    /// its total probe count — or `Ok(None)` when every value has at least
    /// one zero-count component, meaning the composite can match nothing at
    /// all.
    ///
    /// Only non-negated (`prefix != Ne`) components are probed and eligible
    /// as a value's driver arm. Measured on a 228M-row corpus: an unbounded
    /// `ne` component predicate is index-bounded only by "field exists" and
    /// needs a FETCH per row, so scanning it as a driver arm (or even just
    /// probing it with a limit) took 18.7 minutes over 5.9M keys/docs; the
    /// same predicate costs 11ms once bounded to a batch of candidate ids in
    /// `composite_pair_check`. A value's driver arm is otherwise its
    /// lowest-count eligible component filter: if that minimum is zero, no
    /// `composite_group` can satisfy every component of that value (one
    /// component never occurs at all), so the value contributes nothing and
    /// is dropped. If a value has no non-negated component at all (every
    /// component of it uses `Ne`, e.g. `ne5$ne7` on a number+number
    /// composite), there is no bounded arm to drive from and this returns
    /// `SearchError::InvalidComposite`. The composite's overall driver filter
    /// is the `$or` of the surviving values' arms (or the single arm when
    /// there is one); the returned count is the sum of their probe counts,
    /// used only to compete with other parameters for the driver slot in
    /// `matching_resource_ids`.
    pub(super) async fn composite_driver_probe(
        &self,
        search_index: &mongodb::Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
        param: &SearchParameter,
        probe_limit: u64,
        mut session: Option<&mut mongodb::ClientSession>,
    ) -> StorageResult<Option<(Document, u64)>> {
        let per_value_filters =
            self.composite_component_filters(tenant_id, resource_type, param)?;

        let mut arms: Vec<Document> = Vec::new();
        let mut total: u64 = 0;
        for component_filters in per_value_filters {
            let mut best: Option<(Document, u64)> = None;
            let mut has_driver_candidate = false;
            for component_filter in component_filters {
                if component_filter.negated {
                    continue;
                }
                has_driver_candidate = true;
                let count = self
                    .probe_count(
                        search_index,
                        component_filter.filter.clone(),
                        probe_limit,
                        session.as_deref_mut(),
                    )
                    .await?;
                if best.as_ref().is_none_or(|(_, prev)| count < *prev) {
                    best = Some((component_filter.filter, count));
                }
            }
            if !has_driver_candidate {
                return Err(StorageError::Search(SearchError::InvalidComposite {
                    message: format!(
                        "composite value for parameter '{}' has every component using the \
                         'ne' prefix; MongoDB needs at least one non-'ne' component to bound \
                         the search (an unbounded 'ne' arm is an existence-bounded index scan \
                         with a fetch per row)",
                        param.name
                    ),
                }));
            }
            if let Some((filter_doc, count)) = best {
                if count > 0 {
                    arms.push(filter_doc);
                    total += count;
                }
            }
        }

        if arms.is_empty() {
            return Ok(None);
        }

        let filter = if arms.len() == 1 {
            arms.into_iter().next().expect("checked non-empty above")
        } else {
            doc! { "$or": Bson::Array(arms.into_iter().map(Bson::Document).collect()) }
        };
        Ok(Some((filter, total)))
    }

    /// Checks a candidate batch against a composite parameter: for each
    /// value, for each component, fetches `(resource_id, composite_group)`
    /// pairs bounded to the batch, intersects them across components
    /// (`composite_search::intersect_component_pairs`), and unions the
    /// surviving ids across values (comma is OR). Every component is used
    /// here regardless of its `negated` flag — unlike `composite_driver_probe`,
    /// which must never probe or drive off an unbounded `ne` arm, this
    /// function only ever queries a component's filter ANDed with
    /// `resource_id: {"$in": candidates}`, so even a `ne` component's
    /// existence-bounded predicate is cheap: measured at 11ms bounded to a
    /// batch, versus 18.7 minutes unbounded on a 228M-row corpus.
    ///
    /// `candidates` bounds every query issued here — this is the per-batch
    /// check reused by both `matching_resource_ids` (no session) and the
    /// ifNoneExist matcher in `storage.rs` (with a transaction session).
    pub(super) async fn composite_pair_check(
        &self,
        search_index: &mongodb::Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
        param: &SearchParameter,
        candidates: &HashSet<String>,
        mut session: Option<&mut mongodb::ClientSession>,
    ) -> StorageResult<HashSet<String>> {
        if candidates.is_empty() {
            return Ok(HashSet::new());
        }

        let per_value_filters =
            self.composite_component_filters(tenant_id, resource_type, param)?;
        let candidate_ids: Vec<Bson> = candidates.iter().cloned().map(Bson::String).collect();
        let projection = doc! { "resource_id": 1, "composite_group": 1, "_id": 0 };

        let mut matched: HashSet<String> = HashSet::new();
        for component_filters in per_value_filters {
            let mut per_component_pairs: Vec<HashSet<(String, i32)>> =
                Vec::with_capacity(component_filters.len());

            for component_filter in component_filters {
                let bounded = doc! {
                    "$and": [
                        component_filter.filter,
                        { "resource_id": { "$in": candidate_ids.clone() } }
                    ]
                };

                let pairs: HashSet<(String, i32)> = match session.as_deref_mut() {
                    Some(s) => {
                        let mut cursor = search_index
                            .find(bounded)
                            .projection(projection.clone())
                            .session(&mut *s)
                            .await
                            .or_query_error("Failed to query composite component rows (session)")?;
                        let mut out = HashSet::new();
                        while cursor
                            .advance(&mut *s)
                            .await
                            .or_query_error("Failed to advance composite component cursor")?
                        {
                            let doc = cursor
                                .deserialize_current()
                                .or_query_error("Failed to deserialize composite component row")?;
                            if let (Ok(rid), Ok(grp)) =
                                (doc.get_str("resource_id"), doc.get_i32("composite_group"))
                            {
                                out.insert((rid.to_string(), grp));
                            }
                        }
                        out
                    }
                    None => {
                        let cursor = search_index
                            .find(bounded)
                            .projection(projection.clone())
                            .await
                            .or_query_error("Failed to query composite component rows")?;
                        collect_documents(cursor)
                            .await?
                            .into_iter()
                            .filter_map(|d| {
                                let rid = d.get_str("resource_id").ok()?.to_string();
                                let grp = d.get_i32("composite_group").ok()?;
                                Some((rid, grp))
                            })
                            .collect()
                    }
                };
                per_component_pairs.push(pairs);
            }

            let value_matches =
                super::composite_search::intersect_component_pairs(per_component_pairs);
            matched.extend(value_matches);
        }

        Ok(matched)
    }

    fn build_index_value_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
        reference_targets: &[String],
    ) -> StorageResult<Document> {
        match param.name.as_str() {
            "_text" | "_content" => {
                return Err(StorageError::Search(SearchError::TextSearchNotAvailable));
            }
            "_id" | "_lastUpdated" => {
                return Err(StorageError::Search(SearchError::QueryParseError {
                    message: format!(
                        "Special parameter '{}' should be resolved against resources, not search_index",
                        param.name
                    ),
                }));
            }
            _ => {}
        }

        match param.param_type {
            SearchParamType::String => self.build_string_filter(param, value),
            SearchParamType::Token => self.build_token_filter(param, value),
            SearchParamType::Date => build_date_range_filter_doc(value, &param.name),
            SearchParamType::Number => self.build_number_filter(&param.name, value),
            SearchParamType::Reference => {
                self.build_reference_filter(param, value, reference_targets)
            }
            SearchParamType::Uri => self.build_uri_filter(param, value),
            SearchParamType::Quantity => self.build_quantity_filter(&param.name, value),
            SearchParamType::Composite => {
                // Composite parameters are planned by `composite_component_filters`
                // (each component gets its own scoped filter document) and never
                // flow through the per-value dispatcher: there is no single
                // "composite value" predicate to build here. Reaching this arm
                // means a composite parameter escaped that planning path (e.g.
                // via `build_search_index_filter`, which now guards against it
                // too) — #1206. This is a genuine internal error (a bug in this
                // backend, not a bad request), so it maps to 500 like the guard
                // in `build_search_index_filter`, not `InvalidComposite` (400).
                Err(internal_error(format!(
                    "composite parameter '{}' reached the per-value filter dispatcher; it must \
                     be planned via composite_component_filters",
                    param.name
                )))
            }
            SearchParamType::Special => Err(StorageError::Search(
                SearchError::UnsupportedParameterType {
                    param_type: format!("special parameter {}", param.name),
                },
            )),
        }
    }

    fn build_string_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> StorageResult<Document> {
        if value.prefix != SearchPrefix::Eq {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "Unsupported prefix '{}' for string parameter '{}'",
                    value.prefix, param.name
                ),
            }));
        }

        // `value_string` holds the value as written, so the insensitive
        // variants ask Mongo for a case-insensitive regex (`$options: "i"`)
        // rather than relying on a pre-lowercased index. `:exact` compares the
        // raw value, which is what makes it case-sensitive per the spec.
        match param.modifier.as_ref() {
            None => Ok(doc! {
                "value_string": {
                    "$regex": format!("^{}", regex_escape(&value.value)),
                    "$options": "i"
                }
            }),
            Some(SearchModifier::Exact) => Ok(doc! { "value_string": value.value.as_str() }),
            // `:text` on a string is a case-insensitive partial match,
            // implemented here as a substring match (same as `:contains`).
            Some(SearchModifier::Contains | SearchModifier::Text) => Ok(doc! {
                "value_string": {
                    "$regex": regex_escape(&value.value),
                    "$options": "i"
                }
            }),
            Some(other) => Err(StorageError::Search(SearchError::UnsupportedModifier {
                modifier: other.to_string(),
                param_type: "string".to_string(),
            })),
        }
    }

    fn build_token_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> StorageResult<Document> {
        if value.prefix != SearchPrefix::Eq {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "Unsupported prefix '{}' for token parameter '{}'",
                    value.prefix, param.name
                ),
            }));
        }

        match param.modifier.as_ref() {
            None => {}
            // `:text` (contains) and `:code-text` (starts-with) match the
            // token's display text (Coding.display / CodeableConcept.text).
            Some(m @ (SearchModifier::Text | SearchModifier::CodeText)) => {
                let escaped = regex_escape(&value.value);
                let regex = if *m == SearchModifier::CodeText {
                    format!("^{}", escaped)
                } else {
                    escaped
                };
                return Ok(doc! {
                    "value_token_display": { "$regex": regex, "$options": "i" }
                });
            }
            Some(SearchModifier::OfType) => return Ok(Self::build_of_type_filter(&value.value)),
            Some(other) => {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: other.to_string(),
                    param_type: "token".to_string(),
                }));
            }
        }

        if let Some((system, code)) = value.value.split_once('|') {
            if system.is_empty() {
                // |code - match code with no system (#1388). An absent field
                // matches `null`; a `code` element has no system property
                // either, and its row carries the marker (#1379).
                Ok(doc! {
                    "value_token_system": {
                        "$in": [
                            Bson::Null,
                            Bson::String(String::new()),
                            crate::search::IMPLICIT_TOKEN_SYSTEM,
                        ]
                    },
                    "value_token_code": code,
                })
            } else if code.is_empty() {
                Ok(doc! { "value_token_system": system })
            } else {
                // The named system, or a `code` element, whose system is
                // implicit and not verifiable here (#1379).
                Ok(doc! {
                    "value_token_system": {
                        "$in": crate::search::implicit_system_candidates(system).to_vec()
                    },
                    "value_token_code": code,
                })
            }
        } else {
            Ok(doc! { "value_token_code": &value.value })
        }
    }

    /// Builds the `:of-type` predicate (#1408): `type-system|type-code|value`
    /// against the identifier row's `value_identifier_type_system` /
    /// `value_identifier_type_code` / `value_token_code`.
    ///
    /// An empty part is not compared, as on SQLite and PostgreSQL
    /// (`|MR|12345` is "typed MR, in any system"). Anything but three parts
    /// matches nothing, as on PostgreSQL and Elasticsearch: the spec requires
    /// all three, and guessing which one is absent would over-match. A row
    /// written without the type fields — an identifier with no `type`, or one
    /// indexed before they were stored — simply fails the equality.
    fn build_of_type_filter(value: &str) -> Document {
        let parts: Vec<&str> = value.splitn(3, '|').collect();
        let [type_system, type_code, identifier_value] = parts[..] else {
            return Self::match_nothing();
        };

        let mut filter = Document::new();
        if !identifier_value.is_empty() {
            filter.insert("value_token_code", identifier_value);
        }
        if !type_system.is_empty() {
            filter.insert("value_identifier_type_system", type_system);
        }
        if !type_code.is_empty() {
            filter.insert("value_identifier_type_code", type_code);
        }
        if filter.is_empty() {
            return Self::match_nothing();
        }
        filter
    }

    /// A `search_index` predicate no row satisfies.
    fn match_nothing() -> Document {
        doc! { "resource_id": { "$in": Bson::Array(Vec::new()) } }
    }

    /// The `Type/id` (or absolute URL) a `:[type]` reference search names, or
    /// `None` when the value names another type (`subject:Patient=Group/1`),
    /// which no reference can satisfy.
    fn typed_reference(type_name: &str, value: &str) -> Option<String> {
        let base = strip_reference_version(value);
        if !base.contains('/') {
            return Some(format!("{type_name}/{base}"));
        }
        let mut segments = base.rsplit('/');
        let _id = segments.next();
        (segments.next() == Some(type_name)).then(|| base.to_string())
    }

    /// The identifier-row predicate of one `:identifier` value, in the token
    /// grammar SQLite and PostgreSQL use for it: `system|value`, `system|`,
    /// `|value` (no system) or a bare value.
    fn identifier_predicate(value: &str) -> Document {
        match value.split_once('|') {
            Some(("", code)) => doc! {
                "value_token_system": { "$in": [Bson::Null, Bson::String(String::new())] },
                "value_token_code": code,
            },
            Some((system, "")) => doc! { "value_token_system": system },
            Some((system, code)) => doc! {
                "value_token_system": system,
                "value_token_code": code,
            },
            None => doc! { "value_token_code": value },
        }
    }

    /// Resolves a reference `:identifier` parameter (#1408) into its complete
    /// `search_index` filter, or `None` when no resource carries the
    /// identifier — and so nothing can match.
    ///
    /// Same meaning as SQLite and PostgreSQL give it: the reference's *target*
    /// has the identifier. Those backends express it as a sub-select on the
    /// target's `identifier` rows; MongoDB has no join a filter document can
    /// carry, so the targets are read first and the parameter becomes one
    /// `$in` over their `Type/id` — each with an anchored `_history` regex, so
    /// a versioned reference matches too and every entry stays index-bounded.
    ///
    /// The lookup is scoped to the tenant, which is load-bearing (another
    /// tenant's identifiers must not decide this tenant's matches), and to the
    /// parameter's declared target types, which is what lets it use the token
    /// index: every value index leads with `resource_type`.
    async fn resolve_reference_identifier(
        &self,
        search_index: &mongodb::Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
        param: &SearchParameter,
    ) -> StorageResult<Option<Document>> {
        let predicates: Vec<Bson> = param
            .values
            .iter()
            .map(|value| Bson::Document(Self::identifier_predicate(&value.value)))
            .collect();

        let targets: Vec<String> = {
            let registry = self.tenant_registry(tenant_id);
            let registry = registry.read();
            crate::search::resolve_param_targets(&registry, resource_type, &param.name)
        };

        let mut lookup = doc! { "tenant_id": tenant_id };
        if !targets.is_empty() {
            lookup.insert("resource_type", doc! { "$in": targets });
        }
        lookup.insert("param_name", "identifier");
        lookup.insert("$or", Bson::Array(predicates));

        let mut cursor = search_index
            .find(lookup)
            .projection(doc! { "resource_type": 1, "resource_id": 1, "_id": 0 })
            .await
            .or_query_error("Failed to resolve :identifier targets")?;

        let mut found: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        loop {
            let batch = read_cursor_batch(&mut cursor, CANDIDATE_BATCH_SIZE).await?;
            let read = batch.len();
            for row in &batch {
                if let (Ok(target_type), Ok(target_id)) =
                    (row.get_str("resource_type"), row.get_str("resource_id"))
                {
                    found.insert(format!("{target_type}/{target_id}"));
                }
            }
            if found.len() > MAX_IDENTIFIER_TARGETS {
                return Err(StorageError::Search(SearchError::TooManyResults {
                    count: found.len(),
                    max: MAX_IDENTIFIER_TARGETS,
                }));
            }
            if read < CANDIDATE_BATCH_SIZE {
                break;
            }
        }
        if found.is_empty() {
            return Ok(None);
        }

        Ok(Some(Self::identifier_targets_filter(
            tenant_id,
            resource_type,
            &param.name,
            found,
        )))
    }

    /// The filter a resolved `:identifier` parameter becomes: the referencing
    /// rows whose `value_reference` is one of `targets`, at any version.
    fn identifier_targets_filter(
        tenant_id: &str,
        resource_type: &str,
        param_name: &str,
        targets: impl IntoIterator<Item = String>,
    ) -> Document {
        let mut references: Vec<Bson> = Vec::new();
        for target in targets {
            references.push(Bson::RegularExpression(bson::Regex {
                pattern: format!("^{}/_history/", regex_escape(&target)),
                options: String::new(),
            }));
            references.push(Bson::String(target));
        }
        doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "param_name": param_name,
            "value_reference": { "$in": references },
        }
    }

    /// Builds the `search_index` filter for a `Reference`-typed parameter.
    ///
    /// #1083: bare-id search (`subject=123`) used to emit a single
    /// unanchored `$regex: "/123$"`, forcing MongoDB to scan every key in
    /// the index. When the registry declares target types
    /// (`reference_targets`, resolved once by the caller), this builds an
    /// index-bounded `$or` instead: an `$in` of the bare id plus each
    /// `Target/id`; one anchored `^Target/id/_history/` regex per target;
    /// and anchored `^https?://.*/id$` / `^https?://.*/id/_history/`
    /// regexes for absolute-URL references. Every branch must stay
    /// bounded, or MongoDB abandons the index for the whole `$or`
    /// (measured: 827,985 keys+docs / 263s with one unanchored arm vs. 53
    /// keys / 11ms with all bounded). With no declared targets, this falls
    /// back to today's unchanged two-branch filter. The qualified form
    /// (`subject=Patient/123`) also grows an anchored `_history` arm.
    ///
    /// Divergence from SQLite: SQLite's bare form matches any `Type/id`;
    /// Mongo's matches only declared target types, by design, since an
    /// unbounded per-any-type arm can't be index-bounded.
    fn build_reference_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
        reference_targets: &[String],
    ) -> StorageResult<Document> {
        if value.prefix != SearchPrefix::Eq {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "Unsupported prefix '{}' for reference parameter '{}'",
                    value.prefix, param.name
                ),
            }));
        }

        // :contains - case-insensitive substring match on the stored reference.
        if matches!(param.modifier.as_ref(), Some(SearchModifier::Contains)) {
            return Ok(doc! {
                "value_reference": {
                    "$regex": regex_escape(&value.value),
                    "$options": "i"
                }
            });
        }

        // :text (contains) / :code-text (starts-with) match Reference.display.
        if matches!(
            param.modifier.as_ref(),
            Some(SearchModifier::Text | SearchModifier::CodeText)
        ) {
            let escaped = regex_escape(&value.value);
            let regex = if matches!(param.modifier.as_ref(), Some(SearchModifier::CodeText)) {
                format!("^{}", escaped)
            } else {
                escaped
            };
            return Ok(doc! {
                "value_reference_display": { "$regex": regex, "$options": "i" }
            });
        }

        // `:below` / `:above` - URL/path hierarchy on the stored reference,
        // the shapes `build_uri_filter` uses (#1408). Canonical `|version`
        // comparison is not implemented, as on the other backends.
        if matches!(param.modifier.as_ref(), Some(SearchModifier::Below)) {
            return Ok(doc! {
                "value_reference": {
                    "$regex": format!(
                        "^{}(/|$)",
                        regex_escape(value.value.trim_end_matches('/'))
                    )
                }
            });
        }
        if matches!(param.modifier.as_ref(), Some(SearchModifier::Above)) {
            return Ok(doc! {
                "value_reference": {
                    "$in": crate::search::compute_parent_uris(&value.value)
                }
            });
        }

        // `:[type]` (#1408): `subject:Patient=123` is `subject=Patient/123`,
        // so it takes the qualified branch below and can never match
        // `Group/123`.
        let typed;
        let reference = match param.modifier.as_ref() {
            None => value.value.as_str(),
            Some(SearchModifier::Type(type_name)) => {
                match Self::typed_reference(type_name, &value.value) {
                    Some(reference) => {
                        typed = reference;
                        typed.as_str()
                    }
                    None => return Ok(Self::match_nothing()),
                }
            }
            // `:identifier` is resolved by `resolve_reference_identifier`
            // before any filter is built; a path that does not do that has
            // no way to honour it.
            Some(modifier) => {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: modifier.to_string(),
                    param_type: "reference".to_string(),
                }));
            }
        };

        if reference.contains('/') {
            // Version-agnostic, like every other backend: a versioned search
            // value names the resource, not one version of it.
            let base = strip_reference_version(reference);
            return Ok(doc! {
                "$or": [
                    { "value_reference": base },
                    {
                        "value_reference": {
                            "$regex": format!("^{}/_history/", regex_escape(base))
                        }
                    }
                ]
            });
        }

        if reference_targets.is_empty() {
            return Ok(doc! {
                "$or": [
                    { "value_reference": &value.value },
                    {
                        "value_reference": {
                            "$regex": format!("/{}$", regex_escape(&value.value))
                        }
                    }
                ]
            });
        }

        let escaped_id = regex_escape(&value.value);
        let mut in_values: Vec<Bson> = vec![Bson::String(value.value.clone())];
        let mut history_arms: Vec<Bson> = Vec::new();
        let mut seen_targets: HashSet<&str> = HashSet::new();
        for target in reference_targets {
            if !seen_targets.insert(target.as_str()) {
                continue;
            }
            in_values.push(Bson::String(format!("{target}/{}", value.value)));
            history_arms.push(Bson::Document(doc! {
                "value_reference": {
                    "$regex": format!("^{}/{escaped_id}/_history/", regex_escape(target))
                }
            }));
        }

        let mut or_arms: Vec<Bson> = vec![Bson::Document(doc! {
            "value_reference": { "$in": in_values }
        })];
        or_arms.extend(history_arms);
        // Anchored absolute-URL arms: still match `http://.../Patient/123`
        // and its `_history` versions, which the bounded `Target/id` arms
        // above cannot express. The `^https?://` prefix keeps both branches
        // index-bounded (see the doc comment above).
        or_arms.push(Bson::Document(doc! {
            "value_reference": { "$regex": format!("^https?://.*/{escaped_id}$") }
        }));
        or_arms.push(Bson::Document(doc! {
            "value_reference": { "$regex": format!("^https?://.*/{escaped_id}/_history/") }
        }));

        Ok(doc! { "$or": or_arms })
    }

    fn build_uri_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> StorageResult<Document> {
        if value.prefix != SearchPrefix::Eq {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "Unsupported prefix '{}' for uri parameter '{}'",
                    value.prefix, param.name
                ),
            }));
        }

        match param.modifier.as_ref() {
            None | Some(SearchModifier::Exact) => Ok(doc! { "value_uri": &value.value }),
            Some(SearchModifier::Contains) => Ok(doc! {
                "value_uri": {
                    "$regex": regex_escape(&value.value)
                }
            }),
            // `:below`: the value itself or anything under `value/`. One
            // anchored regex, no `$or`: MongoDB takes the literal prefix
            // for the index bounds on idx_search_uri_v2 and applies the
            // `(/|$)` tail as a filter on the keys it walks (#1002).
            Some(SearchModifier::Below) => Ok(doc! {
                "value_uri": {
                    "$regex": format!(
                        "^{}(/|$)",
                        regex_escape(value.value.trim_end_matches('/'))
                    )
                }
            }),
            // `:above`: the value or any path-segment parent of it, as a
            // point lookup per candidate (#1002).
            Some(SearchModifier::Above) => Ok(doc! {
                "value_uri": {
                    "$in": crate::search::compute_parent_uris(&value.value)
                }
            }),
            Some(other) => Err(StorageError::Search(SearchError::UnsupportedModifier {
                modifier: other.to_string(),
                param_type: "uri".to_string(),
            })),
        }
    }

    fn build_date_filter(
        &self,
        value: &SearchValue,
        param: &str,
        field: &str,
    ) -> StorageResult<Document> {
        build_date_filter_doc(value, param, field)
    }

    /// Builds a MongoDB filter for a quantity parameter.
    ///
    /// Value form: `[prefix]number[|system|code]` (or the `number|code` shorthand),
    /// read by the grammar every backend shares
    /// ([`FhirQuantityValue`](crate::search::FhirQuantityValue)): only an
    /// unescaped `|` separates. The comparison runs on `value_quantity_value`;
    /// an optional system/code further constrain `value_quantity_system` /
    /// `value_quantity_unit` (the extractor stores the quantity code under the
    /// unit field). Per the FHIR number search spec (see
    /// `crate::search::range`), `eq`/`ne` match the implicit-precision range
    /// derived from the number's textual form (`60` ⇒ `[59.5, 60.5)`), while
    /// `gt`/`lt`/`ge`/`le`/`sa`/`eb` compare against the exact value.
    ///
    /// A number part that is not a number is an error here, never a filter —
    /// see [`invalid_number_value`].
    fn build_quantity_filter(&self, param: &str, value: &SearchValue) -> StorageResult<Document> {
        let quantity = crate::search::FhirQuantityValue::parse(&value.value)
            .map_err(|error| invalid_number_value(param, &value.value, &error))?;

        let mut filter = doc! {
            "value_quantity_value": Self::numeric_condition(value.prefix, &quantity.number)?
        };
        if let Some(system) = quantity.system {
            filter.insert("value_quantity_system", system);
        }
        if let Some(code) = quantity.code {
            filter.insert("value_quantity_unit", code);
        }

        Ok(filter)
    }

    /// Builds a MongoDB filter for a number parameter, comparing against
    /// `value_number`. Per the FHIR number search spec (see
    /// `crate::search::range`), `eq`/`ne` match the implicit-precision range
    /// derived from the number's textual form (`60` ⇒ `[59.5, 60.5)`, `60.0`
    /// ⇒ `[59.95, 60.05)`), while `gt`/`lt`/`ge`/`le`/`sa`/`eb` compare
    /// against the exact value.
    ///
    /// A value that is not a number is an error here, never a filter — see
    /// [`invalid_number_value`].
    fn build_number_filter(&self, param: &str, value: &SearchValue) -> StorageResult<Document> {
        let number = crate::search::FhirNumberValue::parse(&value.value)
            .map_err(|error| invalid_number_value(param, &value.value, &error))?;
        Ok(doc! { "value_number": Self::numeric_condition(value.prefix, &number)? })
    }

    /// The comparison `prefix` makes against a numeric field, shared by the
    /// number and quantity filters.
    fn numeric_condition(
        prefix: SearchPrefix,
        number: &crate::search::FhirNumberValue,
    ) -> StorageResult<Document> {
        let parsed = number.value;
        Ok(match prefix {
            SearchPrefix::Ap => {
                let (lo, hi) = number.approx_range();
                doc! { "$gte": lo, "$lte": hi }
            }
            SearchPrefix::Eq => {
                let (lo, hi) = number.implicit_range();
                doc! { "$gte": lo, "$lt": hi }
            }
            SearchPrefix::Ne => {
                let (lo, hi) = number.implicit_range();
                doc! { "$not": { "$gte": lo, "$lt": hi } }
            }
            _ => {
                let op = Self::prefix_to_mongo_operator(prefix)?;
                doc! { op: parsed }
            }
        })
    }

    /// Maps a comparator prefix to its MongoDB query operator. The number and
    /// quantity filters only route `gt`/`lt`/`ge`/`le`/`sa`/`eb` through here:
    /// `eq`/`ne` build the implicit-precision range and `ap` its delta range
    /// directly in `build_number_filter` / `build_quantity_filter`.
    fn prefix_to_mongo_operator(prefix: SearchPrefix) -> StorageResult<&'static str> {
        match prefix {
            SearchPrefix::Eq => Ok("$eq"),
            SearchPrefix::Ne => Ok("$ne"),
            SearchPrefix::Gt | SearchPrefix::Sa => Ok("$gt"),
            SearchPrefix::Lt | SearchPrefix::Eb => Ok("$lt"),
            SearchPrefix::Ge => Ok("$gte"),
            SearchPrefix::Le => Ok("$lte"),
            SearchPrefix::Ap => Err(internal_error(
                "`ap` has no single MongoDB operator; numeric_condition builds its range"
                    .to_string(),
            )),
        }
    }

    pub(super) fn build_resource_filter(
        &self,
        tenant_id: &str,
        resource_type: &str,
        query: &SearchQuery,
        matched_ids: Option<&HashSet<String>>,
        cursor: Option<(&PageCursor, &CursorKeyset)>,
    ) -> StorageResult<Document> {
        let mut conditions = vec![doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "is_deleted": false,
        }];

        if let Some(ids) = matched_ids {
            if ids.len() > MAX_RESULT_ID_SET {
                return Err(StorageError::Search(SearchError::TooManyResults {
                    count: ids.len(),
                    max: MAX_RESULT_ID_SET,
                }));
            }
            let id_values = ids.iter().cloned().map(Bson::String).collect::<Vec<_>>();
            conditions.push(doc! {
                "id": { "$in": Bson::Array(id_values) }
            });
        }

        for param in &query.parameters {
            match param.name.as_str() {
                "_id" => {
                    conditions.push(self.build_resource_id_condition(param)?);
                }
                "_lastUpdated" => {
                    conditions.extend(self.build_resource_last_updated_conditions(param)?);
                }
                _ => {}
            }
        }

        if let Some((cursor, keyset)) = cursor {
            conditions.push(self.build_cursor_condition(cursor, keyset)?);
        }

        if conditions.len() == 1 {
            return Ok(conditions.remove(0));
        }

        Ok(doc! {
            "$and": Bson::Array(conditions.into_iter().map(Bson::Document).collect())
        })
    }

    fn build_resource_id_condition(&self, param: &SearchParameter) -> StorageResult<Document> {
        // `:missing` is a presence test on `id`, not a value to compare
        // against it — resolved before the values loop so the boolean
        // literal ("true"/"false") is never consumed as an id (#1055).
        if matches!(param.modifier, Some(SearchModifier::Missing)) {
            let wants_missing = param
                .values
                .first()
                .map(|v| v.value == "true")
                .unwrap_or(false);
            return Ok(if wants_missing {
                doc! { "id": Bson::Null }
            } else {
                doc! { "id": { "$ne": Bson::Null } }
            });
        }

        let negated = match &param.modifier {
            None => false,
            Some(SearchModifier::Not) => true,
            Some(other) => {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: other.to_string(),
                    param_type: param.param_type.to_string(),
                }));
            }
        };

        let mut ids = Vec::new();
        for value in &param.values {
            if value.prefix != SearchPrefix::Eq {
                return Err(StorageError::Search(SearchError::QueryParseError {
                    message: format!("Unsupported prefix '{}' for _id parameter", value.prefix),
                }));
            }
            ids.push(value.value.clone());
        }

        Ok(match (ids.len(), negated) {
            (1, false) => doc! { "id": ids.remove(0) },
            (1, true) => doc! { "id": { "$ne": ids.remove(0) } },
            (_, false) => doc! {
                "id": { "$in": Bson::Array(ids.into_iter().map(Bson::String).collect()) }
            },
            (_, true) => doc! {
                "id": { "$nin": Bson::Array(ids.into_iter().map(Bson::String).collect()) }
            },
        })
    }

    fn build_resource_last_updated_conditions(
        &self,
        param: &SearchParameter,
    ) -> StorageResult<Vec<Document>> {
        match &param.modifier {
            None => {
                let mut conditions = param
                    .values
                    .iter()
                    .map(|value| self.build_date_filter(value, &param.name, "last_updated"))
                    .collect::<StorageResult<Vec<_>>>()?;

                // #1062: comma-separated `_lastUpdated` values are OR, the
                // same defect and fix as `build_search_index_filter` above
                // (see the comment there). Returning a single combined
                // document - instead of one document per value - keeps
                // `build_resource_filter` unchanged: it still `extend`s
                // whatever comes back into the top-level `$and`, so two
                // *separate* `_lastUpdated` parameters (the repeated form)
                // still AND.
                if conditions.len() <= 1 {
                    return Ok(conditions);
                }
                Ok(vec![doc! {
                    "$or": Bson::Array(conditions.drain(..).map(Bson::Document).collect()),
                }])
            }
            // Presence test on `last_updated`, mirroring `_id:missing` above.
            // Every live resource carries a `last_updated`, so `:missing=true`
            // is trivially empty and `:missing=false` trivially everything -
            // but the boolean literal must never reach `build_date_filter`
            // (#1055: it previously 400'd there as an unparseable date).
            Some(SearchModifier::Missing) => {
                let wants_missing = param
                    .values
                    .first()
                    .map(|v| v.value == "true")
                    .unwrap_or(false);
                Ok(vec![if wants_missing {
                    doc! { "last_updated": Bson::Null }
                } else {
                    doc! { "last_updated": { "$ne": Bson::Null } }
                }])
            }
            Some(other) => Err(StorageError::Search(SearchError::UnsupportedModifier {
                modifier: other.to_string(),
                param_type: param.param_type.to_string(),
            })),
        }
    }

    /// The keyset predicate for `cursor` over `keyset`'s field, in the
    /// order [`Self::build_sort_document`] produces for the same query:
    /// the sorted field first, then `id` descending (ascending in previous
    /// mode) as the tie-break when the field is not `id` itself.
    fn build_cursor_condition(
        &self,
        cursor: &PageCursor,
        keyset: &CursorKeyset,
    ) -> StorageResult<Document> {
        let invalid = || {
            StorageError::Search(SearchError::InvalidCursor {
                cursor: cursor.encode(),
            })
        };
        let boundary = match (keyset.field, cursor.sort_values().first()) {
            (CursorKeysetField::LastUpdated, Some(CursorValue::String(value))) => {
                let timestamp = DateTime::parse_from_rfc3339(value)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc);
                Bson::DateTime(chrono_to_bson(timestamp))
            }
            (CursorKeysetField::Id, Some(CursorValue::String(value))) => {
                Bson::String(value.clone())
            }
            _ => return Err(invalid()),
        };

        let previous = cursor.direction() == CursorDirection::Previous;
        // Rows strictly after the boundary in page order: past it along the
        // sort direction, flipped when walking back to the previous page.
        let ascending = (keyset.direction == crate::types::SortDirection::Ascending) != previous;
        let field_op = if ascending { "$gt" } else { "$lt" };
        // `id` is the secondary key in descending order (see
        // `build_sort_document`), so the tie-break is `$lt` going forward.
        let id_op = if previous { "$gt" } else { "$lt" };

        let field = keyset.field.as_str();
        let id = cursor.resource_id().to_string();

        if keyset.field == CursorKeysetField::Id {
            return Ok(doc! { field: { field_op: boundary } });
        }

        Ok(doc! {
            "$or": [
                { field: { field_op: boundary.clone() } },
                { field: boundary, "id": { id_op: id } }
            ]
        })
    }

    fn build_sort_document(
        &self,
        query: &SearchQuery,
        previous_mode: bool,
    ) -> StorageResult<Document> {
        if query.sort.is_empty() {
            return Ok(if previous_mode {
                doc! { "last_updated": 1_i32, "id": 1_i32 }
            } else {
                doc! { "last_updated": -1_i32, "id": -1_i32 }
            });
        }

        let mut sort = Document::new();

        for directive in &query.sort {
            let field = match directive.parameter.as_str() {
                "_lastUpdated" => "last_updated",
                "_id" | "id" => "id",
                other => {
                    return Err(StorageError::Search(
                        SearchError::UnsupportedParameterType {
                            param_type: format!("sort parameter '{}'", other),
                        },
                    ));
                }
            };

            let mut dir = if directive.direction == crate::types::SortDirection::Descending {
                -1_i32
            } else {
                1_i32
            };

            if previous_mode {
                dir = -dir;
            }

            sort.insert(field, dir);
        }

        if !sort.contains_key("id") {
            sort.insert("id", if previous_mode { 1_i32 } else { -1_i32 });
        }

        Ok(sort)
    }

    fn document_to_stored_resource(
        &self,
        tenant: &TenantContext,
        fallback_resource_type: &str,
        doc: Document,
    ) -> StorageResult<StoredResource> {
        let resource_type = doc
            .get_str("resource_type")
            .ok()
            .unwrap_or(fallback_resource_type)
            .to_string();

        let id = doc
            .get_str("id")
            .map_err(|e| internal_error(format!("Missing resource id in search result: {}", e)))?
            .to_string();

        let version_id = doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing version_id in search result: {}", e)))?
            .to_string();

        let payload = doc.get_document("data").map_err(|e| {
            internal_error(format!("Missing resource payload in search result: {}", e))
        })?;

        let content = bson::from_bson::<Value>(Bson::Document(payload.clone())).map_err(|e| {
            serialization_error(format!("Failed to deserialize resource payload: {}", e))
        })?;

        let now = Utc::now();
        let created_at = doc
            .get_datetime("created_at")
            .map(bson_to_chrono)
            .unwrap_or(now);

        let last_updated = doc
            .get_datetime("last_updated")
            .map(bson_to_chrono)
            .unwrap_or(created_at);

        let deleted_at = match doc.get("deleted_at") {
            Some(Bson::DateTime(value)) => Some(bson_to_chrono(value)),
            _ => None,
        };

        let fhir_version = doc
            .get_str("fhir_version")
            .ok()
            .and_then(FhirVersion::from_storage)
            .unwrap_or_else(FhirVersion::default_enabled);

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            version_id,
            tenant.tenant_id().clone(),
            content,
            created_at,
            last_updated,
            deleted_at,
            fhir_version,
        ))
    }

    async fn find_matching_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params_str: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        let Some(query) = self.conditional_query(tenant, resource_type, search_params_str)? else {
            return Ok(Vec::new());
        };

        let result = <Self as SearchProvider>::search(self, tenant, &query).await?;
        Ok(result.resources.items)
    }

    /// Builds the search a conditional interaction's criteria describe, or
    /// `None` when they select nothing. The parsing is
    /// [`crate::search::build_conditional_query`], shared by every backend so
    /// criteria mean what they mean as a direct search (#1312).
    fn conditional_query(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        criteria: &str,
    ) -> StorageResult<Option<SearchQuery>> {
        let registry_arc = self.tenant_registry(tenant.tenant_id().as_str());
        let registry = registry_arc.read();
        crate::search::build_conditional_query(
            &registry,
            resource_type,
            criteria,
            crate::search::ResourceTypeScope::version(self.config().fhir_version),
        )
    }

    /// Types already-split criteria pairs, for the in-transaction
    /// `ifNoneExist` resolver, which drives the `search_index` collection
    /// parameter by parameter instead of running a [`SearchQuery`].
    pub(super) fn build_search_parameters(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &[(String, String)],
    ) -> StorageResult<Vec<SearchParameter>> {
        let registry_arc = self.tenant_registry(tenant.tenant_id().as_str());
        let registry = registry_arc.read();
        crate::search::build_conditional_parameters(
            &registry,
            resource_type,
            params,
            crate::search::ResourceTypeScope::version(self.config().fhir_version),
        )
    }

    fn merge_unique(target: &mut Vec<StoredResource>, additions: Vec<StoredResource>) {
        let mut seen: HashSet<String> = target
            .iter()
            .map(|r| format!("{}/{}", r.resource_type(), r.id()))
            .collect();
        for resource in additions {
            let key = format!("{}/{}", resource.resource_type(), resource.id());
            if seen.insert(key) {
                target.push(resource);
            }
        }
    }

    /// Resolves forward `_include` directives for `search()`/`search_param_sorted()`,
    /// one directive at a time, capped at `self.config().max_included_resources`
    /// per directive.
    ///
    /// `search()` only resolves hop 1 of `_include` inline — `:iterate`
    /// continuation is the REST layer's job against
    /// [`crate::core::resolve_includes_iterate_continuation`] (#1063), so each
    /// directive is copied with `iterate: false` before delegating to
    /// [`IncludeProvider::resolve_includes`] and only its own hop-1 result is
    /// counted against the cap. The cap is applied per directive rather than as
    /// one shared budget so that an earlier directive exhausting its cap cannot
    /// starve a later, unrelated one (#1061) — this mirrors the per-directive
    /// cap the previous inline extractor enforced, and lives here (a `search()`
    /// contract) rather than on the `IncludeProvider` trait itself, which has no
    /// concept of a resource-count budget.
    async fn resolve_forward_includes_capped(
        &self,
        tenant: &TenantContext,
        matches: &[StoredResource],
        forward: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        let limit = self.config().max_included_resources.max(1);

        let mut included: Vec<StoredResource> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut truncated_labels: Vec<String> = Vec::new();

        for directive in forward {
            let hop1 = IncludeDirective {
                iterate: false,
                ..directive.clone()
            };
            let resolved = self
                .resolve_includes(tenant, matches, std::slice::from_ref(&hop1))
                .await?;

            let mut directive_count = 0usize;
            for resource in resolved {
                let key = format!("{}/{}", resource.resource_type(), resource.id());
                if seen.contains(&key) {
                    continue;
                }
                if directive_count >= limit {
                    truncated_labels.push(format!(
                        "_include={}:{}",
                        directive.source_type, directive.search_param
                    ));
                    break;
                }
                seen.insert(key);
                directive_count += 1;
                included.push(resource);
            }
        }

        if !truncated_labels.is_empty() {
            included.push(crate::core::include_truncation_outcome(
                tenant,
                matches
                    .first()
                    .map(|r| r.fhir_version())
                    .unwrap_or_else(FhirVersion::default_enabled),
                limit,
                &truncated_labels.join(","),
                "increase HFS_MONGODB_MAX_INCLUDED_RESOURCES to raise the limit",
            ));
        }

        Ok(included)
    }

    /// Streams the `resource_id`s in `search_index` matching `filter`, capped
    /// at `limit` distinct ids.
    ///
    /// Replaces an unbounded `Collection::distinct("resource_id", ..)`: a
    /// `distinct` reply is a single BSON document capped at 16 MiB by mongod,
    /// which a wide reverse-reference set can exceed (error 17217, ~372k
    /// 36-char ids) — reachable on realistic corpora (#1061). A driver-paged
    /// `find` cursor can be abandoned as soon as `limit` distinct ids are
    /// seen, so neither the wire reply nor our memory scales with the
    /// referring set's true size.
    ///
    /// No `sort`: `resource_id` is not a prefix of `idx_search_reference`
    /// (`tenant_id`, `resource_type`, `param_name`, `value_reference`), so an
    /// in-memory sort would itself risk MongoDB's 32 MiB sort limit. Ids come
    /// back in index order, so which ids survive a given truncation is stable
    /// for a fixed index state (not otherwise meaningful or API-specified).
    async fn referring_resource_ids(
        search_index: &mongodb::Collection<Document>,
        filter: Document,
        limit: usize,
    ) -> StorageResult<(Vec<String>, bool)> {
        let mut cursor = search_index
            .find(filter)
            .projection(doc! { "resource_id": 1_i32, "_id": 0_i32 })
            .batch_size(1000)
            .await
            .or_query_error("Failed to query search_index for revinclude")?;

        let mut seen: HashSet<String> = HashSet::new();
        let mut ids: Vec<String> = Vec::new();
        let mut truncated = false;

        while cursor
            .advance()
            .await
            .or_query_error("Failed to advance search_index cursor")?
        {
            let doc = cursor
                .deserialize_current()
                .or_query_error("Failed to deserialize search_index document")?;
            let Ok(resource_id) = doc.get_str("resource_id") else {
                continue;
            };
            if !seen.insert(resource_id.to_string()) {
                continue;
            }
            if ids.len() >= limit {
                truncated = true;
                break;
            }
            ids.push(resource_id.to_string());
        }

        Ok((ids, truncated))
    }
}

#[async_trait]
impl IncludeProvider for MongoBackend {
    /// Delegates to the shared, registry-driven resolver so `_include` (and
    /// `:iterate`) follows the same search-parameter definitions (with
    /// FHIRPath expression evaluation) used to build the index, instead of a
    /// backend-specific reference extractor that looked the search parameter
    /// up as a literal JSON field name and disagreed with it whenever the
    /// parameter name differed from the element (e.g.
    /// `Encounter:service-provider` -> `serviceProvider`).
    async fn resolve_includes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        crate::core::resolve_includes_iterative(self, tenant, resources, includes).await
    }
}

#[async_trait]
impl RevincludeProvider for MongoBackend {
    async fn resolve_revincludes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        revincludes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        if resources.is_empty() || revincludes.is_empty() {
            return Ok(Vec::new());
        }

        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let resources_collection = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        // See the matching comment in `resolve_includes`: per directive, not shared.
        let limit = self.config().max_included_resources.max(1);

        let mut included: Vec<StoredResource> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut truncated_labels: Vec<String> = Vec::new();

        for revinclude in revincludes {
            if revinclude.source_type.is_empty() {
                continue;
            }

            let mut reference_values: Vec<String> = Vec::with_capacity(resources.len() * 2);
            for resource in resources {
                reference_values.push(format!("{}/{}", resource.resource_type(), resource.id()));
                reference_values.push(resource.id().to_string());
            }
            reference_values.sort();
            reference_values.dedup();

            if reference_values.is_empty() {
                continue;
            }

            let bson_values: Vec<Bson> = reference_values.into_iter().map(Bson::String).collect();
            let index_filter = doc! {
                "tenant_id": tenant_id,
                "resource_type": &revinclude.source_type,
                "param_name": &revinclude.search_param,
                "value_reference": { "$in": Bson::Array(bson_values) },
            };

            // Bounded, streamed id resolution (#1061) — see
            // `referring_resource_ids`. `reference_values` above is already
            // bounded by the page; this is what previously was not.
            let (matching_ids, index_truncated) =
                Self::referring_resource_ids(&search_index, index_filter, limit).await?;

            if matching_ids.is_empty() {
                continue;
            }

            let id_bson: Vec<Bson> = matching_ids.into_iter().map(Bson::String).collect();
            let resource_filter = doc! {
                "tenant_id": tenant_id,
                "resource_type": &revinclude.source_type,
                "is_deleted": false,
                "id": { "$in": Bson::Array(id_bson) },
            };

            let docs = collect_documents(
                resources_collection
                    .find(resource_filter)
                    .await
                    .or_query_error("Failed to fetch revinclude resources")?,
            )
            .await?;

            let mut directive_count = 0usize;
            let mut directive_truncated = index_truncated;
            for doc in docs {
                let stored =
                    self.document_to_stored_resource(tenant, &revinclude.source_type, doc)?;
                let key = format!("{}/{}", stored.resource_type(), stored.id());
                if seen.contains(&key) {
                    continue;
                }
                if directive_count >= limit {
                    directive_truncated = true;
                    break;
                }
                seen.insert(key);
                directive_count += 1;
                included.push(stored);
            }

            if directive_truncated {
                truncated_labels.push(format!(
                    "_revinclude={}:{}",
                    revinclude.source_type, revinclude.search_param
                ));
            }
        }

        if !truncated_labels.is_empty() {
            included.push(crate::core::include_truncation_outcome(
                tenant,
                resources
                    .first()
                    .map(|r| r.fhir_version())
                    .unwrap_or_else(FhirVersion::default_enabled),
                limit,
                &truncated_labels.join(","),
                "increase HFS_MONGODB_MAX_INCLUDED_RESOURCES to raise the limit",
            ));
        }

        Ok(included)
    }
}

#[cfg(test)]
mod date_filter_tests {
    use super::*;

    fn filter(raw: &str) -> Document {
        build_date_filter_doc(&SearchValue::parse(raw), "date", "value_date").expect("valid date")
    }

    fn bounds(d: &Document) -> &Document {
        d.get_document("value_date").expect("field doc")
    }

    fn at(rfc3339: &str) -> BsonDateTime {
        chrono_to_bson(
            DateTime::parse_from_rfc3339(rfc3339)
                .expect("test instant")
                .with_timezone(&Utc),
        )
    }

    /// The two arms of an `ne` filter: before the range, or at/after its end.
    fn ne_bounds(d: &Document) -> (BsonDateTime, BsonDateTime) {
        let arms = d.get_array("$or").expect("$or");
        assert_eq!(arms.len(), 2);
        let arm = |i: usize, op: &str| {
            *arms[i]
                .as_document()
                .and_then(|a| a.get_document("value_date").ok())
                .and_then(|b| b.get_datetime(op).ok())
                .unwrap_or_else(|| panic!("arm {i} has no {op}: {d}"))
        };
        (arm(0, "$lt"), arm(1, "$gte"))
    }

    /// #519: every prefix arm over the period a partial date names, pinned
    /// without a live MongoDB. `1995-10` spans [1995-10-01, 1995-11-01).
    #[test]
    fn partial_dates_compare_as_periods() {
        let oct = at("1995-10-01T00:00:00Z");
        let nov = at("1995-11-01T00:00:00Z");

        let eq = filter("1995-10");
        assert_eq!(bounds(&eq).get_datetime("$gte").unwrap(), &oct);
        assert_eq!(bounds(&eq).get_datetime("$lt").unwrap(), &nov);

        // ne: outside the period, either side.
        assert_eq!(ne_bounds(&filter("ne1995-10")), (oct, nov));

        // gt/sa start at the period's end; le runs to it.
        assert_eq!(
            bounds(&filter("gt1995-10")).get_datetime("$gte").unwrap(),
            &nov
        );
        assert_eq!(
            bounds(&filter("sa1995-10")).get_datetime("$gte").unwrap(),
            &nov
        );
        assert_eq!(
            bounds(&filter("le1995-10")).get_datetime("$lt").unwrap(),
            &nov
        );

        // lt/eb and ge compare against the start.
        assert_eq!(
            bounds(&filter("lt1995-10")).get_datetime("$lt").unwrap(),
            &oct
        );
        assert_eq!(
            bounds(&filter("eb1995-10")).get_datetime("$lt").unwrap(),
            &oct
        );
        assert_eq!(
            bounds(&filter("ge1995-10")).get_datetime("$gte").unwrap(),
            &oct
        );
    }

    /// Every precision derives its own range end — a value with a time
    /// included. It used to be an instant compared with `$eq`/`$ne`/`$gt`/
    /// `$lte`, so `eq…T08:30:00Z` missed a stored `…T08:30:00.123Z` (#1297).
    #[test]
    fn precision_decides_the_period_end() {
        assert_eq!(
            bounds(&filter("1995")).get_datetime("$lt").unwrap(),
            &at("1996-01-01T00:00:00Z")
        );
        assert_eq!(
            bounds(&filter("1995-10-02")).get_datetime("$lt").unwrap(),
            &at("1995-10-03T00:00:00Z")
        );

        let second = at("1995-10-02T08:30:00Z");
        let next_second = at("1995-10-02T08:30:01Z");

        let eq = filter("eq1995-10-02T08:30:00Z");
        assert_eq!(bounds(&eq).get_datetime("$gte").unwrap(), &second);
        assert_eq!(bounds(&eq).get_datetime("$lt").unwrap(), &next_second);

        assert_eq!(
            ne_bounds(&filter("ne1995-10-02T08:30:00Z")),
            (second, next_second)
        );
        // gt is strictly after the whole second; le runs to its end.
        assert_eq!(
            bounds(&filter("gt1995-10-02T08:30:00Z"))
                .get_datetime("$gte")
                .unwrap(),
            &next_second
        );
        assert_eq!(
            bounds(&filter("le1995-10-02T08:30:00Z"))
                .get_datetime("$lt")
                .unwrap(),
            &next_second
        );
        assert_eq!(
            bounds(&filter("lt1995-10-02T08:30:00Z"))
                .get_datetime("$lt")
                .unwrap(),
            &second
        );
        assert_eq!(
            bounds(&filter("ge1995-10-02T08:30:00Z"))
                .get_datetime("$gte")
                .unwrap(),
            &second
        );
    }

    /// Minute precision is valid in FHIR search and used to be a 400 here.
    #[test]
    fn minute_precision_is_a_one_minute_range() {
        let eq = filter("2013-04-05T09:20-04:00");
        assert_eq!(
            bounds(&eq).get_datetime("$gte").unwrap(),
            &at("2013-04-05T13:20:00Z")
        );
        assert_eq!(
            bounds(&eq).get_datetime("$lt").unwrap(),
            &at("2013-04-05T13:21:00Z")
        );
    }

    /// BSON holds milliseconds. A finer search value is the millisecond it
    /// falls in, so a client echoing microseconds still finds the stored date.
    #[test]
    fn fractions_are_clamped_to_the_millisecond() {
        let eq = filter("2021-11-10T16:48:57.246958-08:00");
        assert_eq!(
            bounds(&eq).get_datetime("$gte").unwrap(),
            &at("2021-11-11T00:48:57.246Z")
        );
        assert_eq!(
            bounds(&eq).get_datetime("$lt").unwrap(),
            &at("2021-11-11T00:48:57.247Z")
        );
    }

    /// #1296: a `+` offset that form decoding turned into a space.
    #[test]
    fn a_decoded_plus_is_the_same_instant() {
        assert_eq!(
            filter("2013-04-05T18:50:00 05:30"),
            filter("2013-04-05T18:50:00+05:30")
        );
        assert_eq!(
            bounds(&filter("2013-04-05T18:50:00 05:30"))
                .get_datetime("$gte")
                .unwrap(),
            &at("2013-04-05T13:20:00Z")
        );
    }

    /// ap on a point: the shared window, the range widened by one unit of a
    /// date-only precision on each side (#1391; it was ±12h around the start).
    #[test]
    fn ap_on_a_point_uses_the_shared_window() {
        let ap = filter("ap1995-10-02");
        assert_eq!(
            bounds(&ap).get_datetime("$gte").unwrap(),
            &at("1995-10-01T00:00:00Z")
        );
        assert_eq!(
            bounds(&ap).get_datetime("$lt").unwrap(),
            &at("1995-10-04T00:00:00Z")
        );
    }

    fn range_filter(raw: &str) -> Document {
        build_date_range_filter_doc(&SearchValue::parse(raw), "date").expect("valid date")
    }

    /// #1391: a standalone date parameter compares the stored range
    /// `[value_date, value_date_end)` per the FHIR rules for a range target.
    /// `2020` spans [2020-01-01, 2021-01-01).
    #[test]
    fn range_prefixes_compare_both_ends_of_the_stored_range() {
        let (s, e) = (at("2020-01-01T00:00:00Z"), at("2021-01-01T00:00:00Z"));
        let contained = doc! {
            "value_date": { "$gte": s, "$lt": e },
            "value_date_end": { "$lte": e },
        };
        let end_after = doc! {
            "value_date": { "$ne": null },
            "value_date_end": { "$gt": e },
        };
        assert_eq!(range_filter("2020"), contained);
        assert_eq!(
            range_filter("ne2020"),
            doc! { "$or": [ { "value_date": { "$lt": s } }, end_after.clone() ] }
        );
        assert_eq!(range_filter("gt2020"), end_after);
        assert_eq!(range_filter("lt2020"), doc! { "value_date": { "$lt": s } });
        assert_eq!(
            range_filter("ge2020"),
            doc! { "$or": [ end_after.clone(), contained.clone() ] }
        );
        assert_eq!(
            range_filter("le2020"),
            doc! { "$or": [ { "value_date": { "$lt": s } }, contained.clone() ] }
        );
        assert_eq!(range_filter("sa2020"), doc! { "value_date": { "$gte": e } });
        assert_eq!(
            range_filter("eb2020"),
            doc! {
                "value_date": { "$lt": s },
                "value_date_end": { "$lte": s },
            }
        );
        assert_eq!(
            range_filter("ap2020"),
            doc! {
                "value_date": { "$lt": at("2022-01-01T00:00:00Z") },
                "value_date_end": { "$gt": at("2019-01-01T00:00:00Z") },
            }
        );
    }

    /// The filters agree with the shared predicate on the cases #1391 is
    /// about: a Period straddling the search range, and open ends.
    #[test]
    fn range_filters_decide_the_issue_cases() {
        let matches = |raw: &str, ts: &str, te: &str| {
            let parsed = FhirDateValue::parse(&SearchValue::parse(raw).value).unwrap();
            parsed
                .range_predicate(SearchValue::parse(raw).prefix, StorageResolution::Millis)
                .matches(bson_to_chrono(&at(ts)), bson_to_chrono(&at(te)))
        };
        let open_end = crate::search::open_end(StorageResolution::Millis).to_rfc3339();
        let open_start = crate::search::open_start().to_rfc3339();
        // A Period from 2019-06 to the end of 2020-03 is not "in" 2020.
        assert!(!matches(
            "2020",
            "2019-06-01T00:00:00Z",
            "2020-04-01T00:00:00Z"
        ));
        // One that only ends in 2021 does not start after 2020.
        assert!(!matches(
            "sa2020",
            "2020-06-01T00:00:00Z",
            "2021-06-01T00:00:00Z"
        ));
        // Open ends are unbounded.
        assert!(matches("gt2030", "2019-01-01T00:00:00Z", &open_end));
        assert!(matches("lt1900", &open_start, "2020-01-01T00:00:00Z"));
        // Every prefix yields a filter.
        for prefix in ["", "ne", "gt", "lt", "ge", "le", "sa", "eb", "ap"] {
            range_filter(&format!("{prefix}2020-06-15T10:00Z"));
        }
    }

    /// Descending date sorts read the end of the stored range; everything
    /// else, and ascending date sorts, read the value field itself.
    #[test]
    fn descending_date_sort_reads_the_range_end() {
        use crate::types::SortDirection;
        assert_eq!(
            sort_key_expression("value_date", SortDirection::Descending),
            Bson::Document(doc! { "$ifNull": ["$value_date_end", "$value_date"] })
        );
        assert_eq!(
            sort_key_expression("value_date", SortDirection::Ascending),
            Bson::String("$value_date".to_string())
        );
        assert_eq!(
            sort_key_expression("value_string", SortDirection::Descending),
            Bson::String("$value_string".to_string())
        );
    }

    /// Garbage stays an error, not a silent full scan — under every prefix,
    /// and now for values chrono used to be the judge of.
    #[test]
    fn invalid_dates_error() {
        for raw in [
            "not-a-date",
            "ltnot-a-date",
            "ne2024-13-45",
            "lt2024-02-30",
            "2013-04-05T10",
            "2013-04-05T09:20:00z",
            "",
        ] {
            let error = build_date_filter_doc(&SearchValue::parse(raw), "date", "value_date")
                .expect_err(raw);
            assert!(
                matches!(
                    &error,
                    StorageError::Search(SearchError::InvalidDateValue { param, .. })
                        if param == "date"
                ),
                "{raw}: {error:?}"
            );
            let error =
                build_date_range_filter_doc(&SearchValue::parse(raw), "date").expect_err(raw);
            assert!(
                matches!(
                    &error,
                    StorageError::Search(SearchError::InvalidDateValue { .. })
                ),
                "{raw} (range): {error:?}"
            );
        }
    }

    /// The numeric sibling (#1340). `f64::from_str` used to be the judge, and
    /// took `inf` and `nan`: `ltinf` was `{"$lt": Infinity}`, which every
    /// indexed row satisfies, and `nenan` matched them all too.
    #[test]
    fn invalid_numbers_error() {
        let backend =
            MongoBackend::new(crate::backends::mongodb::MongoBackendConfig::default()).unwrap();
        for raw in [
            "abc",
            "gtabc",
            "",
            "gt",
            "1e",
            "inf",
            "lt-inf",
            "ltInfinity",
            "nenan",
            "NaN",
            "lt1e999",
            "0x10",
        ] {
            let value = SearchValue::parse(raw);
            for error in [
                backend
                    .build_number_filter("probability", &value)
                    .expect_err(raw),
                backend
                    .build_quantity_filter("probability", &value)
                    .expect_err(raw),
            ] {
                assert!(
                    matches!(
                        &error,
                        StorageError::Search(SearchError::InvalidNumberValue { param, .. })
                            if param == "probability"
                    ),
                    "{raw}: {error:?}"
                );
            }
        }
        for raw in ["||mg", "abc|http://unitsofmeasure.org|mg", "ltinf||mg"] {
            let error = backend
                .build_quantity_filter("value-quantity", &SearchValue::parse(raw))
                .expect_err(raw);
            assert!(
                matches!(
                    &error,
                    StorageError::Search(SearchError::InvalidNumberValue { .. })
                ),
                "{raw}: {error:?}"
            );
        }
    }

    /// A quantity's system and code are split on unescaped pipes only.
    #[test]
    fn quantity_filter_unescapes_pipes() {
        let backend =
            MongoBackend::new(crate::backends::mongodb::MongoBackendConfig::default()).unwrap();
        let filter = backend
            .build_quantity_filter(
                "value-quantity",
                &SearchValue::parse("gt5.4|http://example.org|a\\|b"),
            )
            .unwrap();
        assert_eq!(
            filter.get_str("value_quantity_system"),
            Ok("http://example.org")
        );
        assert_eq!(filter.get_str("value_quantity_unit"), Ok("a|b"));
        assert_eq!(
            filter.get_document("value_quantity_value").unwrap(),
            &doc! { "$gt": 5.4 }
        );
    }
}

#[cfg(test)]
mod query_support_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;
    use crate::types::CompositeSearchComponent;

    /// `birthdate:missing=true` carries the value `true`, which is not a
    /// date — `:missing` must be accepted as presence-only (#881) rather
    /// than falling through to type-specific value parsing.
    #[test]
    fn missing_is_supported_without_type_specific_value_parsing() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("true")],
            chain: vec![],
            components: vec![],
        });

        assert!(backend.validate_query_support(&query).is_ok());
    }

    #[test]
    fn below_is_still_rejected() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Below),
            values: vec![SearchValue::eq("http://example.org|abc")],
            chain: vec![],
            components: vec![],
        });

        let error = backend.validate_query_support(&query).unwrap_err();
        assert!(matches!(
            error,
            StorageError::Search(SearchError::UnsupportedModifier {
                ref modifier,
                ..
            }) if modifier == "below"
        ));
    }

    #[test]
    fn uri_below_and_above_pass_the_gate() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        for modifier in [SearchModifier::Below, SearchModifier::Above] {
            let query = SearchQuery::new("ValueSet").with_parameter(SearchParameter {
                name: "url".to_string(),
                param_type: SearchParamType::Uri,
                modifier: Some(modifier),
                values: vec![SearchValue::eq("http://example.org/fhir")],
                chain: vec![],
                components: vec![],
            });
            assert!(backend.validate_query_support(&query).is_ok());
        }
    }

    #[test]
    fn uri_below_is_one_anchored_regex() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let param = SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: Some(SearchModifier::Below),
            values: vec![SearchValue::eq("http://example.org/fhir")],
            chain: vec![],
            components: vec![],
        };
        let filter = backend.build_uri_filter(&param, &param.values[0]).unwrap();
        assert_eq!(
            filter,
            doc! { "value_uri": { "$regex": "^http://example\\.org/fhir(/|$)" } }
        );
    }

    #[test]
    fn uri_above_is_a_point_lookup_over_the_parents() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let param = SearchParameter {
            name: "url".to_string(),
            param_type: SearchParamType::Uri,
            modifier: Some(SearchModifier::Above),
            values: vec![SearchValue::eq("http://example.org/fhir/ValueSet/a")],
            chain: vec![],
            components: vec![],
        };
        let filter = backend.build_uri_filter(&param, &param.values[0]).unwrap();
        assert_eq!(
            filter,
            doc! { "value_uri": { "$in": [
                "http://example.org/fhir/ValueSet/a",
                "http://example.org/fhir/ValueSet",
                "http://example.org/fhir",
                "http://example.org",
            ] } }
        );
    }

    fn composite_param(modifier: Option<SearchModifier>) -> SearchParameter {
        SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier,
            values: vec![SearchValue::eq("http://loinc.org|8302-2$gt150")],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value-quantity".to_string(),
                },
            ],
        }
    }

    /// #1206 widened gate: `composite_search::component_param` hardcodes
    /// `modifier: None` for each component filter, so *any* modifier other
    /// than `:missing` on a composite would be silently dropped rather than
    /// honoured if it reached the composite planner. `:exact` stands in for
    /// the broader class (`:not` already has its own dedicated test).
    #[test]
    fn exact_on_a_composite_is_rejected() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let query = SearchQuery::new("Observation")
            .with_parameter(composite_param(Some(SearchModifier::Exact)));

        let error = backend.validate_query_support(&query).unwrap_err();
        assert!(
            matches!(
                error,
                StorageError::Search(SearchError::UnsupportedModifier { .. })
            ),
            "expected UnsupportedModifier, got {error:?}"
        );
    }

    /// `:missing` on a composite is the one modifier the gate still lets
    /// through: `matching_resource_ids` routes it into the separate
    /// `missing` list before `normal` params reach the composite
    /// driver/pair-check planner, so it never hits
    /// `composite_search::component_param`'s hardcoded `modifier: None` and
    /// is instead served by `missing_presence_filter`.
    #[test]
    fn missing_on_a_composite_passes_the_gate() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let query = SearchQuery::new("Observation")
            .with_parameter(composite_param(Some(SearchModifier::Missing)));

        assert!(backend.validate_query_support(&query).is_ok());
    }
}

/// Controller finding: a `:missing` presence filter with no value-field
/// conjunct is served by a full scan of the `(tenant, type)` slice, because
/// no generation-2 partial index has a `{tenant_id, resource_type,
/// param_name}` prefix without a value predicate. See `missing_presence_filter`.
#[cfg(test)]
mod missing_presence_filter_tests {
    use super::*;

    /// Every `SearchParamType` variant, via a `match` that is exhaustive
    /// over the enum: adding a new variant fails this test to compile
    /// (rather than silently falling through `value_field_for`'s own
    /// `Composite | Special => None` arm) until it is added here too.
    #[test]
    fn value_field_for_covers_every_variant() {
        let variants = [
            SearchParamType::String,
            SearchParamType::Uri,
            SearchParamType::Number,
            SearchParamType::Date,
            SearchParamType::Quantity,
            SearchParamType::Token,
            SearchParamType::Reference,
            SearchParamType::Composite,
            SearchParamType::Special,
        ];
        for variant in variants {
            let expected = match variant {
                SearchParamType::String => Some("value_string"),
                SearchParamType::Token => Some("value_token_code"),
                SearchParamType::Date => Some("value_date"),
                SearchParamType::Number => Some("value_number"),
                SearchParamType::Quantity => Some("value_quantity_value"),
                SearchParamType::Reference => Some("value_reference"),
                SearchParamType::Uri => Some("value_uri"),
                SearchParamType::Composite | SearchParamType::Special => None,
            };
            assert_eq!(value_field_for(variant), expected, "{variant:?}");
        }
    }

    fn param(name: &str, param_type: SearchParamType) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("false")],
            chain: vec![],
            components: vec![],
        }
    }

    #[test]
    fn missing_presence_filter_adds_value_field_conjunct_for_token() {
        let filter = missing_presence_filter(
            "tenant-1",
            "Patient",
            &param("gender", SearchParamType::Token),
        );
        assert_eq!(
            filter,
            doc! {
                "tenant_id": "tenant-1",
                "resource_type": "Patient",
                "param_name": "gender",
                "value_token_code": { "$ne": Bson::Null },
            }
        );
    }

    #[test]
    fn missing_presence_filter_is_the_bare_envelope_for_composite() {
        let filter = missing_presence_filter(
            "tenant-1",
            "Patient",
            &param("some-composite", SearchParamType::Composite),
        );
        assert_eq!(
            filter,
            doc! {
                "tenant_id": "tenant-1",
                "resource_type": "Patient",
                "param_name": "some-composite",
            }
        );
    }
}

/// #1062: comma-separated search values are OR per FHIR
/// (https://build.fhir.org/search.html#combining), for every parameter
/// type. These pins target the exact filter document `build_search_index_filter`
/// sends to `distinct_resource_ids` (:832), because *where* the predicate is
/// evaluated is what made the old `$and` catastrophic rather than merely
/// strict: a `search_index` document holds one value of one parameter for
/// one resource, so a disjoint `$and` can never be satisfied by any single
/// row. `distinct` then returns empty, `matching_resource_ids`'s empty-set
/// short circuit fires, and the *entire* search result is emptied — not
/// just the one parameter.
///
/// This is unrelated to the repeated-parameter form
/// (`?birthdate=ge1980-01-01&birthdate=lt1990-01-01`), which arrives as two
/// separate `SearchParameter` entries and is intersected across parameters
/// in `matching_resource_ids`; that AND is correct per spec and is guarded
/// here too, so a change that widens the comma-list join cannot silently
/// widen the repeated form as well.
#[cfg(test)]
mod value_list_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    #[test]
    fn comma_separated_date_values_are_ored() {
        let backend = backend();
        let param = SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::parse("2019"), SearchValue::parse("2021")],
            chain: vec![],
            components: vec![],
        };

        let filter = backend
            .build_search_index_filter("t1", "Patient", &param)
            .expect("valid filter");

        assert!(
            filter.get("$and").is_none(),
            "date value list must not be ANDed: {filter:?}"
        );
        let arms = filter.get_array("$or").expect("$or array");
        assert_eq!(arms.len(), 2);
        for arm in arms {
            let doc = arm.as_document().expect("$or arm is a document");
            assert!(
                doc.contains_key("value_date"),
                "each $or arm constrains value_date: {doc:?}"
            );
        }
    }

    fn date_param(values: &[&str]) -> SearchParameter {
        SearchParameter {
            name: "date".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: values.iter().map(|v| SearchValue::parse(v)).collect(),
            chain: vec![],
            components: vec![],
        }
    }

    fn at_date(rfc3339: &str) -> BsonDateTime {
        chrono_to_bson(
            DateTime::parse_from_rfc3339(rfc3339)
                .expect("test instant")
                .with_timezone(&Utc),
        )
    }

    /// The arms of a date filter's top-level `$or`, as documents.
    fn top_level_arms(filter: &Document) -> Vec<Document> {
        assert_eq!(filter.len(), 1, "only the $or at the top: {filter:?}");
        filter
            .get_array("$or")
            .expect("top-level $or")
            .iter()
            .map(|arm| arm.as_document().expect("arm").clone())
            .collect()
    }

    /// #1391: a date value with alternatives (`ge`, `le`, `ne`) puts its `$or`
    /// at the top, each arm repeating tenant/type/param, so MongoDB plans one
    /// covered `idx_search_date_v3` scan per arm; nested under the shared
    /// conjuncts it reads the documents instead.
    #[test]
    fn two_branch_date_prefixes_scope_every_arm() {
        let backend = backend();
        let (s, e) = (
            at_date("2020-01-01T00:00:00Z"),
            at_date("2021-01-01T00:00:00Z"),
        );
        let scope = doc! { "tenant_id": "t1", "resource_type": "Patient", "param_name": "date" };
        let expect = |arm: Document| {
            let mut scoped = scope.clone();
            scoped.extend(arm);
            Bson::Document(scoped)
        };
        let end_after = doc! { "value_date": { "$ne": null }, "value_date_end": { "$gt": e } };
        let contained = doc! {
            "value_date": { "$gte": s, "$lt": e },
            "value_date_end": { "$lte": e },
        };
        let before = doc! { "value_date": { "$lt": s } };

        let cases = [
            ("ge2020", vec![end_after.clone(), contained.clone()]),
            ("le2020", vec![before.clone(), contained.clone()]),
            ("ne2020", vec![before.clone(), end_after.clone()]),
        ];
        for (raw, arms) in cases {
            let filter = backend
                .build_search_index_filter("t1", "Patient", &date_param(&[raw]))
                .expect(raw);
            let expected = doc! { "$or": arms.into_iter().map(expect).collect::<Vec<_>>() };
            assert_eq!(filter, expected, "date={raw}");
        }
    }

    /// One-condition and both-end prefixes keep the flat shape: the
    /// conjuncts and the value condition side by side, no `$or` at all.
    #[test]
    fn single_branch_date_prefixes_stay_flat() {
        let backend = backend();
        for raw in ["gt2020", "lt2020", "sa2020", "eb2020", "2020", "ap2020"] {
            let filter = backend
                .build_search_index_filter("t1", "Patient", &date_param(&[raw]))
                .expect(raw);
            assert!(!filter.contains_key("$or"), "date={raw}: {filter:?}");
            assert_eq!(filter.get_str("tenant_id"), Ok("t1"), "date={raw}");
            assert_eq!(filter.get_str("resource_type"), Ok("Patient"), "date={raw}");
            assert_eq!(filter.get_str("param_name"), Ok("date"), "date={raw}");
        }
    }

    /// A comma list is the OR of its values' alternatives, every one a
    /// self-contained scoped arm at the top: `ge2020,lt2019` is
    /// `[ge-end-arm, ge-contained-arm, lt-arm]`, not an `$or` of an `$or`.
    #[test]
    fn comma_list_of_date_values_flattens_into_one_top_level_or() {
        let backend = backend();
        let filter = backend
            .build_search_index_filter("t1", "Patient", &date_param(&["ge2020", "lt2019"]))
            .expect("filter");
        let arms = top_level_arms(&filter);
        assert_eq!(arms.len(), 3, "{arms:?}");
        for arm in &arms {
            assert_eq!(arm.get_str("tenant_id"), Ok("t1"), "{arm:?}");
            assert_eq!(arm.get_str("resource_type"), Ok("Patient"), "{arm:?}");
            assert_eq!(arm.get_str("param_name"), Ok("date"), "{arm:?}");
            assert!(!arm.contains_key("$or"), "no nested $or: {arm:?}");
            assert!(arm.contains_key("value_date"), "{arm:?}");
        }
        // The same alternatives, in the same order, as the values alone.
        let alone: Vec<Document> = ["ge2020", "lt2019"]
            .iter()
            .flat_map(|raw| {
                let one = backend
                    .build_search_index_filter("t1", "Patient", &date_param(&[raw]))
                    .unwrap();
                if one.contains_key("$or") {
                    top_level_arms(&one)
                } else {
                    vec![one]
                }
            })
            .collect();
        assert_eq!(arms, alone);
    }

    /// A repeated parameter (`date=ge2020&date=le2021`) is still two separate
    /// filters, one per occurrence; nothing is merged across them.
    #[test]
    fn repeated_date_parameters_stay_separate_filters() {
        let backend = backend();
        let first = backend
            .build_search_index_filter("t1", "Patient", &date_param(&["ge2020"]))
            .unwrap();
        let second = backend
            .build_search_index_filter("t1", "Patient", &date_param(&["le2021"]))
            .unwrap();
        assert_eq!(top_level_arms(&first).len(), 2);
        assert_eq!(top_level_arms(&second).len(), 2);
        assert_ne!(first, second);
    }

    /// The contained-resource search reuses the filter under its own scope:
    /// tenant and type come off every arm, the parameter name stays.
    #[test]
    fn stripping_the_scope_reaches_into_date_arms() {
        let backend = backend();
        let mut filter = backend
            .build_search_index_filter("", "", &date_param(&["ge2020"]))
            .unwrap();
        strip_index_scope(&mut filter, SearchParamType::Date);
        for arm in top_level_arms(&filter) {
            assert!(!arm.contains_key("tenant_id"), "{arm:?}");
            assert!(!arm.contains_key("resource_type"), "{arm:?}");
            assert_eq!(arm.get_str("param_name"), Ok("date"), "{arm:?}");
        }
    }

    /// A regression a partial fix could pass: dropping only `Date` from the
    /// old `matches!` would leave `Number` ANDed and this test red.
    #[test]
    fn comma_separated_number_values_are_ored() {
        let backend = backend();
        let param = SearchParameter {
            name: "factor-override".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::parse("0.25"), SearchValue::parse("1.5")],
            chain: vec![],
            components: vec![],
        };

        let filter = backend
            .build_search_index_filter("t1", "ChargeItem", &param)
            .expect("valid filter");

        assert!(
            filter.get("$and").is_none(),
            "number value list must not be ANDed: {filter:?}"
        );
        let arms = filter.get_array("$or").expect("$or array");
        assert_eq!(arms.len(), 2);
        for arm in arms {
            let doc = arm.as_document().expect("$or arm is a document");
            assert!(
                doc.contains_key("value_number"),
                "each $or arm constrains value_number: {doc:?}"
            );
        }
    }

    /// Quantity was never in the AND list. Confirmed here rather than
    /// assumed, so this module pins the join for every parameter type in
    /// one place.
    #[test]
    fn comma_separated_quantity_values_stay_ored() {
        let backend = backend();
        let param = SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::parse("5"), SearchValue::parse("10")],
            chain: vec![],
            components: vec![],
        };

        let filter = backend
            .build_search_index_filter("t1", "Observation", &param)
            .expect("valid filter");

        assert!(filter.get("$and").is_none());
        assert_eq!(filter.get_array("$or").expect("$or array").len(), 2);
    }

    /// Second site, same defect, one function away:
    /// `build_resource_last_updated_conditions` (:1508) is the `_lastUpdated`
    /// equivalent of `build_search_index_filter`, resolved against the
    /// resource document instead of `search_index`. A disjoint comma list
    /// must OR into a single combined condition.
    #[test]
    fn comma_separated_last_updated_values_are_ored() {
        let backend = backend();
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::parse("2019"), SearchValue::parse("2021")],
            chain: vec![],
            components: vec![],
        });

        let filter = backend
            .build_resource_filter("t1", "Patient", &query, None, None)
            .expect("valid filter");

        let and_arms = filter.get_array("$and").expect("$and array");
        assert_eq!(
            and_arms.len(),
            2,
            "base document plus one combined last_updated condition: {filter:?}"
        );
        let combined = and_arms[1].as_document().expect("condition is a document");
        let or_arms = combined.get_array("$or").expect("$or array");
        assert_eq!(or_arms.len(), 2);
        for arm in or_arms {
            let doc = arm.as_document().expect("$or arm is a document");
            assert!(doc.contains_key("last_updated"));
        }
    }

    /// Guard: the repeated-parameter form is a different mechanism (two
    /// separate `SearchParameter` entries, not one with two values) and
    /// must keep ANDing exactly as before — MANUAL_TESTING_MATRIX row 4.3
    /// depends on it. Must stay green before and after the fix.
    #[test]
    fn repeated_last_updated_parameters_still_and() {
        let backend = backend();
        let query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "_lastUpdated".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::parse("ge2019")],
                chain: vec![],
                components: vec![],
            })
            .with_parameter(SearchParameter {
                name: "_lastUpdated".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::parse("le2021")],
                chain: vec![],
                components: vec![],
            });

        let filter = backend
            .build_resource_filter("t1", "Patient", &query, None, None)
            .expect("valid filter");

        let and_arms = filter.get_array("$and").expect("$and array");
        assert_eq!(
            and_arms.len(),
            3,
            "base document plus one condition per repeated parameter: {filter:?}"
        );
        for arm in &and_arms[1..] {
            let doc = arm.as_document().expect("condition is a document");
            assert!(doc.contains_key("last_updated"));
            assert!(
                doc.get("$or").is_none(),
                "a single-value condition stays flat, not wrapped in $or"
            );
        }
    }
}

/// #1011: `eq`/`ne` on number/quantity match the implicit-precision range
/// derived from the value's textual form, per `crate::search::range`, while
/// every other comparator prefix compares against the exact value.
#[cfg(test)]
mod number_quantity_precision_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    fn number_bounds(raw: &str) -> Document {
        let backend = backend();
        let param = SearchParameter {
            name: "value-number".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::parse(raw)],
            chain: vec![],
            components: vec![],
        };
        let filter = backend
            .build_search_index_filter("t1", "Observation", &param)
            .expect("valid filter");
        filter
            .get_document("value_number")
            .expect("value_number condition")
            .clone()
    }

    #[test]
    fn number_eq_uses_text_precision_range() {
        let bounds = number_bounds("60");
        assert!((bounds.get_f64("$gte").unwrap() - 59.5).abs() < 1e-9);
        assert!((bounds.get_f64("$lt").unwrap() - 60.5).abs() < 1e-9);

        // A trailing zero narrows the implicit precision: "60.0" carries one
        // more significant figure than "60", so the range is ten times
        // tighter around the same center.
        let bounds = number_bounds("60.0");
        assert!((bounds.get_f64("$gte").unwrap() - 59.95).abs() < 1e-9);
        assert!((bounds.get_f64("$lt").unwrap() - 60.05).abs() < 1e-9);
    }

    #[test]
    fn number_ne_excludes_text_precision_range() {
        let bounds = number_bounds("ne60");
        let not_doc = bounds.get_document("$not").expect("$not condition");
        assert!((not_doc.get_f64("$gte").unwrap() - 59.5).abs() < 1e-9);
        assert!((not_doc.get_f64("$lt").unwrap() - 60.5).abs() < 1e-9);
    }

    #[test]
    fn quantity_eq_uses_text_precision_range() {
        let backend = backend();
        let param = SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::parse("60.0|http://unitsofmeasure.org|kg")],
            chain: vec![],
            components: vec![],
        };
        let filter = backend
            .build_search_index_filter("t1", "Observation", &param)
            .expect("valid filter");

        let bounds = filter
            .get_document("value_quantity_value")
            .expect("value_quantity_value condition");
        assert!((bounds.get_f64("$gte").unwrap() - 59.95).abs() < 1e-9);
        assert!((bounds.get_f64("$lt").unwrap() - 60.05).abs() < 1e-9);
        assert_eq!(filter.get_str("value_quantity_unit").unwrap(), "kg");
    }

    #[test]
    fn number_comparators_stay_exact() {
        let gt = number_bounds("gt60");
        assert_eq!(gt.get_f64("$gt").unwrap(), 60.0);

        let le = number_bounds("le60");
        assert_eq!(le.get_f64("$lte").unwrap(), 60.0);
    }

    /// #1011 finding 1: MongoDB's `$not` also matches documents where the
    /// field is missing, so an unscoped `ne` filter could over-match. But
    /// `build_search_index_filter` always ANDs the value condition with
    /// `tenant_id`/`resource_type`/`param_name` in the same top-level
    /// document (:1560), and a given `param_name` is indexed with exactly
    /// one `IndexValue` variant per parameter (`storage.rs`'s
    /// `index_value_to_document`, e.g. `IndexValue::Number` always inserts
    /// `value_number`). So every `search_index` document that matches this
    /// filter's `tenant_id`/`resource_type`/`param_name` already carries
    /// `value_number`, and no `search_index` document for a *different*
    /// parameter or resource type can match — `$not` never reaches into
    /// another parameter's rows or missing-field rows. No `$exists` guard is
    /// needed; this pins the sibling keys are present alongside `$not`.
    #[test]
    fn ne_filter_stays_scoped_to_tenant_resource_and_param() {
        let backend = backend();
        let param = SearchParameter {
            name: "value-number".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::parse("ne60")],
            chain: vec![],
            components: vec![],
        };
        let filter = backend
            .build_search_index_filter("t1", "Observation", &param)
            .expect("valid filter");

        assert_eq!(filter.get_str("tenant_id").unwrap(), "t1");
        assert_eq!(filter.get_str("resource_type").unwrap(), "Observation");
        assert_eq!(filter.get_str("param_name").unwrap(), "value-number");
        assert!(
            filter
                .get_document("value_number")
                .expect("value_number condition")
                .contains_key("$not"),
            "ne is a value_number-scoped $not, sitting alongside the tenant/resource/param keys: {filter:?}"
        );
    }
}

/// #1206 review finding 1: every component's scoped filter must exclude
/// rows where its own value field is absent — otherwise a `ne`-shaped
/// predicate (satisfied by absence, not merely by a not-equal value) is
/// wrongly satisfied by a *sibling* component's row, since every component
/// of a composite shares the same `param_name`. These pin the exact
/// document `composite_component_filters` builds for a token component and
/// for a quantity `ne` component.
#[cfg(test)]
mod composite_component_filter_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;
    use crate::types::CompositeSearchComponent;

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    fn code_value_quantity_param(value: &str) -> SearchParameter {
        SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value-quantity".to_string(),
                },
            ],
        }
    }

    #[test]
    fn token_component_filter_scoped_to_tenant_resource_param_and_not_null() {
        let backend = backend();
        let param = code_value_quantity_param("8302-2$150");
        let per_value = backend
            .composite_component_filters("t1", "Observation", &param)
            .expect("valid composite filters");
        assert_eq!(per_value.len(), 1, "one composite value");
        let components = &per_value[0];
        assert_eq!(components.len(), 2, "two declared components");

        assert!(
            !components[0].negated,
            "an Eq token component must not be flagged negated"
        );
        let token_filter = &components[0].filter;
        assert_eq!(token_filter.get_str("tenant_id").unwrap(), "t1");
        assert_eq!(
            token_filter.get_str("resource_type").unwrap(),
            "Observation"
        );
        // Every component's row shares `param_name` with the composite
        // itself. This distinct-type composite can also use historical rows.
        assert_eq!(
            token_filter.get_str("param_name").unwrap(),
            "code-value-quantity"
        );
        assert!(!token_filter.contains_key("composite_slot"));
        let and_arms = token_filter.get_array("$and").expect("$and conjunction");
        assert_eq!(and_arms.len(), 2, "not-null guard + typed predicate");
        let not_null = and_arms[0].as_document().expect("not-null arm");
        assert_eq!(
            not_null
                .get_document("value_token_code")
                .expect("value_token_code not-null guard")
                .get("$ne"),
            Some(&Bson::Null),
            "the not-null guard must be Bson::Null, not $exists: {not_null:?}"
        );
        let predicate = and_arms[1].as_document().expect("typed predicate arm");
        assert_eq!(predicate.get_str("value_token_code").unwrap(), "8302-2");
    }

    #[test]
    fn quantity_ne_component_filter_excludes_absent_value_field() {
        let backend = backend();
        let param = code_value_quantity_param("8302-2$ne150");
        let per_value = backend
            .composite_component_filters("t1", "Observation", &param)
            .expect("valid composite filters");
        assert!(
            per_value[0][1].negated,
            "an ne quantity component must be flagged negated"
        );
        let quantity_filter = &per_value[0][1].filter;

        assert_eq!(
            quantity_filter.get_str("param_name").unwrap(),
            "code-value-quantity"
        );
        let and_arms = quantity_filter.get_array("$and").expect("$and conjunction");
        assert_eq!(and_arms.len(), 2);

        let not_null = and_arms[0].as_document().expect("not-null arm");
        assert_eq!(
            not_null
                .get_document("value_quantity_value")
                .expect("value_quantity_value not-null guard")
                .get("$ne"),
            Some(&Bson::Null),
            "the not-null guard must be Bson::Null, not $exists: {not_null:?}"
        );

        // The typed predicate is quantity's `Ne` shape: a `$not` wrapping the
        // implicit-precision range. Without the not-null guard above, a
        // sibling row lacking `value_quantity_value` entirely would also
        // satisfy this `$not` -- this pins that the guard is present
        // alongside it, not merged into a single top-level key that a
        // sibling absence could still slip past.
        let predicate = and_arms[1].as_document().expect("typed predicate arm");
        assert!(
            predicate
                .get_document("value_quantity_value")
                .expect("value_quantity_value condition")
                .contains_key("$not"),
            "quantity ne must build a $not-wrapped range predicate: {predicate:?}"
        );
    }

    /// #1206 follow-up: `composite_driver_probe` must never probe or drive
    /// off an unbounded `ne` component arm, so `composite_component_filters`
    /// must report, per component, whether its parsed prefix was `Ne`.
    #[test]
    fn negated_flag_marks_only_the_ne_component() {
        let backend = backend();

        let ne_param = code_value_quantity_param("8302-2$ne150");
        let per_value = backend
            .composite_component_filters("t1", "Observation", &ne_param)
            .expect("valid composite filters");
        assert!(
            !per_value[0][0].negated,
            "the token component (code=8302-2) is not 'ne'"
        );
        assert!(
            per_value[0][1].negated,
            "the quantity component (ne150) is 'ne'"
        );

        let gt_param = code_value_quantity_param("8302-2$gt150");
        let per_value = backend
            .composite_component_filters("t1", "Observation", &gt_param)
            .expect("valid composite filters");
        assert!(
            !per_value[0][0].negated,
            "the token component (code=8302-2) is not 'ne'"
        );
        assert!(
            !per_value[0][1].negated,
            "'gt' is not 'ne', so the quantity component must not be flagged negated"
        );
    }

    #[test]
    fn composite_with_empty_values_errors_like_plain_param() {
        let backend = backend();
        let param = SearchParameter {
            values: vec![],
            ..code_value_quantity_param("unused")
        };
        let err = backend
            .composite_component_filters("t1", "Observation", &param)
            .expect_err("empty values must error");
        assert!(
            matches!(
                err,
                StorageError::Search(SearchError::QueryParseError { .. })
            ),
            "expected QueryParseError like build_search_index_filter's own empty-values guard, \
             got {err:?}"
        );
    }

    #[test]
    fn repeated_type_components_are_scoped_by_declared_slot() {
        let backend = backend();
        let param = SearchParameter {
            name: "code-value-concept".to_string(),
            param_type: SearchParamType::Composite,
            modifier: None,
            values: vec![SearchValue::eq("A$B")],
            chain: vec![],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "value-concept".to_string(),
                },
            ],
        };
        let filters = backend
            .composite_component_filters("t1", "Observation", &param)
            .unwrap();
        assert_eq!(filters[0][0].filter.get_i32("composite_slot"), Ok(1));
        assert_eq!(filters[0][1].filter.get_i32("composite_slot"), Ok(2));
    }
}

/// #1083: bare-id reference search must be index-bounded, and the qualified
/// form must keep matching its own `_history` versions (parity with SQLite's
/// `test_search_by_reference_does_not_match_extended_sibling_ids`). These
/// pins target the exact filter document `build_search_index_filter` sends
/// to `distinct_resource_ids`, the same way `value_list_tests` above does.
///
/// The `backend()` helper's registry (embedded fallback params only — the
/// spec bundle at `<workspace root>/data` is not reachable from this crate's
/// `./data`-relative default `data_dir` under `cargo test`) has no declared
/// targets for `Observation.subject`, so tests that need targets seed the
/// base registry explicitly via `SearchParameterDefinition::with_targets`,
/// the same builder `resolve_param_targets_returns_declared_targets` in
/// `helios_fhir::search::registry` uses.
#[cfg(test)]
mod reference_filter_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;
    use crate::search::SearchParameterDefinition;

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    /// A backend whose base registry declares `Observation.subject` targets
    /// `["Patient", "Group"]`, registry order preserved.
    fn backend_with_subject_targets() -> MongoBackend {
        let backend = backend();
        backend
            .tenant_registries()
            .base()
            .write()
            .register(
                SearchParameterDefinition::new(
                    "http://hl7.org/fhir/SearchParameter/Observation-subject",
                    "subject",
                    SearchParamType::Reference,
                    "Observation.subject",
                )
                .with_base(vec!["Observation"])
                .with_targets(vec!["Patient", "Group"]),
            )
            .expect("registering Observation.subject must not collide with a fallback param");
        backend
    }

    fn subject_param(value: &str) -> SearchParameter {
        SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        }
    }

    fn or_arms(filter: &Document) -> Vec<Document> {
        filter
            .get_array("$or")
            .expect("$or array")
            .iter()
            .map(|b| b.as_document().expect("$or arm is a document").clone())
            .collect()
    }

    fn arm_regex(arm: &Document) -> &str {
        arm.get_document("value_reference")
            .expect("value_reference field")
            .get_str("$regex")
            .expect("$regex field")
    }

    /// Bare `subject=123`: `$in` first (bare id + `Target/123` per declared
    /// target, registry order, deduplicated), one anchored `_history` regex
    /// per target, and an anchored `^https?://.*/123$` absolute-URL regex
    /// last — every branch index-bounded, per #1083 (a plain unanchored
    /// `/123$` last arm makes MongoDB abandon the index for the whole `$or`).
    #[test]
    fn bare_id_with_registry_targets_is_index_bounded() {
        let backend = backend_with_subject_targets();
        let filter = backend
            .build_search_index_filter("t1", "Observation", &subject_param("123"))
            .expect("valid filter");

        let arms = or_arms(&filter);
        // $in arm + one history-regex arm per target ["Patient", "Group"] +
        // the trailing absolute-URL regex + the trailing absolute-URL
        // _history regex.
        assert_eq!(arms.len(), 5, "unexpected arm count: {arms:?}");

        let in_values: Vec<&str> = arms[0]
            .get_document("value_reference")
            .expect("value_reference field")
            .get_array("$in")
            .expect("$in array")
            .iter()
            .map(|b| b.as_str().expect("$in entry is a string"))
            .collect();
        assert_eq!(in_values, vec!["123", "Patient/123", "Group/123"]);

        assert_eq!(arm_regex(&arms[1]), "^Patient/123/_history/");
        assert_eq!(arm_regex(&arms[2]), "^Group/123/_history/");

        // Anchored absolute-URL arms, last.
        assert_eq!(arm_regex(&arms[3]), "^https?://.*/123$");
        assert_eq!(arm_regex(&arms[4]), "^https?://.*/123/_history/");
    }

    /// No declared targets (unknown parameter): the bare form is byte-for-byte
    /// today's two-branch `$or` — the match set must not shrink or grow.
    #[test]
    fn bare_id_without_registry_targets_is_unchanged() {
        let backend = backend();
        let param = SearchParameter {
            name: "nonexistent-ref".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("123")],
            chain: vec![],
            components: vec![],
        };

        let filter = backend
            .build_search_index_filter("t1", "Observation", &param)
            .expect("valid filter");

        let arms = or_arms(&filter);
        assert_eq!(
            arms.len(),
            2,
            "today's filter has exactly two arms: {arms:?}"
        );
        assert_eq!(
            arms[0].get_str("value_reference").expect("exact-match arm"),
            "123"
        );
        assert_eq!(arm_regex(&arms[1]), "/123$");
    }

    /// Qualified `subject=Patient/123`: exact match plus an anchored
    /// `_history` regex, so `Patient/123/_history/2` keeps matching
    /// (parity with SQLite).
    #[test]
    fn qualified_reference_matches_its_own_history() {
        let backend = backend();
        let filter = backend
            .build_search_index_filter("t1", "Observation", &subject_param("Patient/123"))
            .expect("valid filter");

        let arms = or_arms(&filter);
        assert_eq!(arms.len(), 2);
        assert_eq!(
            arms[0].get_str("value_reference").expect("exact-match arm"),
            "Patient/123"
        );
        assert_eq!(arm_regex(&arms[1]), "^Patient/123/_history/");
    }

    /// Regex metacharacters in the id (`.`) must be escaped in every arm:
    /// the qualified form's `_history` regex, each target's `_history`
    /// regex in the bare form, and both trailing absolute-URL regexes.
    #[test]
    fn regex_metacharacters_are_escaped_in_every_arm() {
        let backend = backend_with_subject_targets();

        let qualified = backend
            .build_search_index_filter("t1", "Observation", &subject_param("Patient/123.5"))
            .expect("valid filter");
        let arms = or_arms(&qualified);
        assert_eq!(arm_regex(&arms[1]), "^Patient/123\\.5/_history/");

        let bare = backend
            .build_search_index_filter("t1", "Observation", &subject_param("123.5"))
            .expect("valid filter");
        let arms = or_arms(&bare);
        assert_eq!(arms.len(), 5);
        assert_eq!(arm_regex(&arms[1]), "^Patient/123\\.5/_history/");
        assert_eq!(arm_regex(&arms[2]), "^Group/123\\.5/_history/");
        assert_eq!(arm_regex(&arms[3]), "^https?://.*/123\\.5$");
        assert_eq!(arm_regex(&arms[4]), "^https?://.*/123\\.5/_history/");
    }
}

/// #1055: `_id`/`_lastUpdated` are lowered outside the generic modifier
/// dispatch (`matching_resource_ids` skips them and hands them to
/// `build_resource_id_condition` / `build_resource_last_updated_conditions`
/// instead), so these two builders must honour `param.modifier` themselves.
/// Pinned at the filter-shape level, without a live MongoDB, because the
/// integration test cannot fully discriminate `_id:missing=true` (an
/// accidental pass either way — see the sibling integration test).
#[cfg(test)]
mod metadata_param_modifier_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    fn id_param(modifier: Option<SearchModifier>, values: &[&str]) -> SearchParameter {
        SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            chain: vec![],
            components: vec![],
        }
    }

    fn last_updated_param(modifier: Option<SearchModifier>, value: &str) -> SearchParameter {
        SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        }
    }

    /// The metadata-param condition alone: `build_resource_filter`'s first
    /// `$and` arm is always the tenant/resource_type/is_deleted seed, so with
    /// a single parameter and no matched-id set the second arm is the
    /// condition under test.
    fn condition(backend: &MongoBackend, param: SearchParameter) -> Document {
        let query = SearchQuery::new("Patient").with_parameter(param);
        let filter = backend
            .build_resource_filter("t", "Patient", &query, None, None)
            .expect("filter should build");
        filter.get_array("$and").expect("$and array")[1]
            .as_document()
            .expect("condition document")
            .clone()
    }

    #[test]
    fn plain_id_match_is_unchanged() {
        let backend = backend();
        let cond = condition(&backend, id_param(None, &["pat-a"]));
        assert_eq!(cond.get_str("id").unwrap(), "pat-a");
    }

    #[test]
    fn id_not_negates_instead_of_matching() {
        let backend = backend();

        let single = condition(&backend, id_param(Some(SearchModifier::Not), &["pat-a"]));
        assert_eq!(
            single.get_document("id").unwrap().get_str("$ne").unwrap(),
            "pat-a"
        );

        let multi = condition(
            &backend,
            id_param(Some(SearchModifier::Not), &["pat-a", "pat-b"]),
        );
        let nin = multi.get_document("id").unwrap().get_array("$nin").unwrap();
        let values: Vec<&str> = nin.iter().map(|b| b.as_str().unwrap()).collect();
        assert_eq!(values, vec!["pat-a", "pat-b"]);
    }

    #[test]
    fn id_missing_is_a_presence_test_on_the_id_field() {
        let backend = backend();

        let missing_true = condition(&backend, id_param(Some(SearchModifier::Missing), &["true"]));
        assert!(matches!(missing_true.get("id"), Some(Bson::Null)));

        let missing_false = condition(
            &backend,
            id_param(Some(SearchModifier::Missing), &["false"]),
        );
        assert!(matches!(
            missing_false.get_document("id").unwrap().get("$ne"),
            Some(Bson::Null)
        ));
    }

    /// Control for #519: the refactor of `build_resource_last_updated_conditions`
    /// into a modifier match must leave the no-modifier (period-comparison) arm
    /// byte-identical.
    #[test]
    fn last_updated_no_modifier_control_still_builds_a_date_range() {
        let backend = backend();
        let cond = condition(&backend, last_updated_param(None, "2024"));
        let bounds = cond.get_document("last_updated").expect("field doc");
        assert!(bounds.get_datetime("$gte").is_ok());
        assert!(bounds.get_datetime("$lt").is_ok());
    }

    #[test]
    fn last_updated_missing_does_not_parse_the_boolean_as_a_date() {
        let backend = backend();

        let missing_true = condition(
            &backend,
            last_updated_param(Some(SearchModifier::Missing), "true"),
        );
        assert!(matches!(missing_true.get("last_updated"), Some(Bson::Null)));

        let missing_false = condition(
            &backend,
            last_updated_param(Some(SearchModifier::Missing), "false"),
        );
        assert!(matches!(
            missing_false
                .get_document("last_updated")
                .unwrap()
                .get("$ne"),
            Some(Bson::Null)
        ));
    }

    #[test]
    fn unhonoured_modifiers_on_metadata_params_are_rejected() {
        let backend = backend();

        let id_text = SearchQuery::new("Patient")
            .with_parameter(id_param(Some(SearchModifier::Text), &["abc"]));
        let err = backend.validate_query_support(&id_text).unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                if modifier == "text"
        ));

        let last_updated_not = SearchQuery::new("Patient")
            .with_parameter(last_updated_param(Some(SearchModifier::Not), "2024"));
        let err = backend
            .validate_query_support(&last_updated_not)
            .unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                if modifier == "not"
        ));

        // The three honoured combinations must not be rejected.
        assert!(
            backend
                .validate_query_support(
                    &SearchQuery::new("Patient")
                        .with_parameter(id_param(Some(SearchModifier::Not), &["pat-a"]))
                )
                .is_ok()
        );
        assert!(
            backend
                .validate_query_support(
                    &SearchQuery::new("Patient")
                        .with_parameter(id_param(Some(SearchModifier::Missing), &["true"]))
                )
                .is_ok()
        );
        assert!(
            backend
                .validate_query_support(
                    &SearchQuery::new("Patient")
                        .with_parameter(last_updated_param(Some(SearchModifier::Missing), "true"))
                )
                .is_ok()
        );
    }
}

/// #1040: `order_sort_keys` must reproduce the server's `{ key: <order>,
/// _id: 1 }` sort — including the tie-break — now that the bounded path
/// orders the keyed candidates client-side instead of server-side.
#[cfg(test)]
mod sort_key_order_tests {
    use super::*;
    use crate::types::SortDirection;

    fn dt(millis: i64) -> Bson {
        Bson::DateTime(mongodb::bson::DateTime::from_millis(millis))
    }

    #[test]
    fn dates_ascending_order_by_instant() {
        let keyed = vec![
            ("c".to_string(), dt(300)),
            ("a".to_string(), dt(100)),
            ("b".to_string(), dt(200)),
        ];
        assert_eq!(
            order_sort_keys(keyed, SortDirection::Ascending),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn dates_descending_reverses_by_instant_with_id_ascending_ties() {
        let keyed = vec![
            ("c".to_string(), dt(300)),
            ("a".to_string(), dt(100)),
            ("b".to_string(), dt(200)),
        ];
        assert_eq!(
            order_sort_keys(keyed.clone(), SortDirection::Descending),
            vec!["c", "b", "a"]
        );

        // Equal keys keep id-ascending order in BOTH directions — this is
        // the server's `{key: -1, _id: 1}` semantics.
        let tied = vec![("z".to_string(), dt(100)), ("y".to_string(), dt(100))];
        assert_eq!(
            order_sort_keys(tied.clone(), SortDirection::Ascending),
            vec!["y", "z"]
        );
        assert_eq!(
            order_sort_keys(tied, SortDirection::Descending),
            vec!["y", "z"]
        );
    }

    #[test]
    fn strings_order_byte_wise() {
        let keyed = vec![
            ("id-c".to_string(), Bson::String("c".to_string())),
            ("id-ba".to_string(), Bson::String("ba".to_string())),
            ("id-b".to_string(), Bson::String("b".to_string())),
        ];
        assert_eq!(
            order_sort_keys(keyed, SortDirection::Ascending),
            vec!["id-b", "id-ba", "id-c"]
        );

        let case_sensitive = vec![
            ("id-a".to_string(), Bson::String("a".to_string())),
            ("id-B".to_string(), Bson::String("B".to_string())),
        ];
        assert_eq!(
            order_sort_keys(case_sensitive, SortDirection::Ascending),
            vec!["id-B", "id-a"]
        );
    }

    #[test]
    fn numbers_order_numerically_across_representations() {
        let keyed = vec![
            ("id-7".to_string(), Bson::Int32(7)),
            ("id-6.5".to_string(), Bson::Double(6.5)),
            ("id-8".to_string(), Bson::Int64(8)),
        ];
        assert_eq!(
            order_sort_keys(keyed, SortDirection::Ascending),
            vec!["id-6.5", "id-7", "id-8"]
        );
    }
}

/// #1057: the probe row of a cursor page is dropped in driver order, before a
/// backward page is reversed back into sort order.
#[cfg(test)]
mod trim_probe_row_tests {
    use super::trim_probe_row;

    // Sort order is `last_updated desc, id desc`, so the listing is
    // 7,6,5,4,3,2,1 and a page holds 3 rows.

    #[test]
    fn forward_drops_the_trailing_probe_and_reports_next() {
        let mut rows = vec![7, 6, 5, 4];
        assert_eq!(trim_probe_row(&mut rows, 3, false, false), (true, false));
        assert_eq!(rows, vec![7, 6, 5]);
    }

    #[test]
    fn forward_without_probe_has_no_next_and_passes_previous_through() {
        let mut rows = vec![4, 3, 2];
        assert_eq!(trim_probe_row(&mut rows, 3, false, true), (false, true));
        assert_eq!(rows, vec![4, 3, 2]);
    }

    #[test]
    fn backward_drops_the_farthest_row_before_restoring_order() {
        // Back from page 3 (cursor at row 1): the driver returns the
        // nearest-newer rows first, plus the probe row 5 from page 1.
        let mut rows = vec![2, 3, 4, 5];
        assert_eq!(trim_probe_row(&mut rows, 3, true, true), (true, true));
        // Page 2 exactly — not [5, 4, 3], which is what popping after the
        // reverse produced.
        assert_eq!(rows, vec![4, 3, 2]);
    }

    #[test]
    fn backward_without_probe_has_no_previous_but_still_has_next() {
        // Back from page 2 (cursor at row 4): rows 5..7 are all that exist.
        let mut rows = vec![5, 6, 7];
        assert_eq!(trim_probe_row(&mut rows, 3, true, true), (true, false));
        assert_eq!(rows, vec![7, 6, 5]);
    }

    #[test]
    fn backward_with_no_rows_has_neither_link() {
        let mut rows: Vec<i32> = Vec::new();
        assert_eq!(trim_probe_row(&mut rows, 3, true, true), (false, false));
        assert!(rows.is_empty());
    }
}

/// #1206 review: `build_search_parameters` must resolve the parameter type
/// from the *parsed* value for anything the registry doesn't declare as
/// Composite, so the registry-miss fallback still sees a stripped
/// comparator prefix, exactly as it did before the composite path existed.
#[cfg(test)]
mod build_search_parameters_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;
    use crate::tenant::{TenantId, TenantPermissions};

    fn tenant() -> TenantContext {
        TenantContext::new(TenantId::new("t1"), TenantPermissions::full_access())
    }

    /// A parameter the registry has never heard of is refused: conditional
    /// criteria guard a write, so an unknown name is neither searched for
    /// literally nor ignored (#1323). With the default (embedded-only)
    /// registry, `foo` is unregistered; `_lastUpdated` is a date, read with
    /// prefix `Gt` and value `2020-01-01`.
    #[test]
    fn unregistered_parameter_is_refused_and_a_date_keeps_its_prefix() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        backend
            .build_search_parameters(
                &tenant(),
                "Patient",
                &[("foo".to_string(), "gt2020-01-01".to_string())],
            )
            .expect_err("an unregistered criterion must be refused");

        let params = backend
            .build_search_parameters(
                &tenant(),
                "Patient",
                &[("_lastUpdated".to_string(), "gt2020-01-01".to_string())],
            )
            .expect("criteria build");

        assert_eq!(params.len(), 1);
        let param = &params[0];
        assert_eq!(param.param_type, SearchParamType::Date);
        assert!(param.components.is_empty());
        assert_eq!(param.values.len(), 1);
        assert_eq!(param.values[0].prefix, SearchPrefix::Gt);
        assert_eq!(param.values[0].value, "2020-01-01");
    }

    /// With the full R4 registry loaded, a registry-declared composite
    /// parameter is the one case that keeps the raw `$`-joined string
    /// (`parse` would strip a prefix off the whole composite when its
    /// first component is itself ordered) and populates `components` from
    /// the registry's declared sub-parameters.
    #[test]
    fn registered_composite_parameter_keeps_the_raw_value_and_components() {
        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"))
            .expect("workspace data dir");
        let backend = MongoBackend::new(MongoBackendConfig {
            data_dir: Some(data_dir),
            ..Default::default()
        })
        .unwrap();

        let params = backend
            .build_search_parameters(
                &tenant(),
                "Observation",
                &[(
                    "code-value-quantity".to_string(),
                    "http://loinc.org|8302-2$gt150".to_string(),
                )],
            )
            .expect("criteria build");

        assert_eq!(params.len(), 1);
        let param = &params[0];
        assert_eq!(param.param_type, SearchParamType::Composite);
        assert_eq!(param.values.len(), 1);
        assert_eq!(param.values[0].prefix, SearchPrefix::Eq);
        assert_eq!(param.values[0].value, "http://loinc.org|8302-2$gt150");

        assert_eq!(param.components.len(), 2);
        assert_eq!(param.components[0].param_type, SearchParamType::Token);
        assert_eq!(param.components[0].param_name, "code");
        assert_eq!(param.components[1].param_type, SearchParamType::Quantity);
        assert_eq!(param.components[1].param_name, "value-quantity");
    }
}

/// #1058: the cursor a page mints must be one the same query accepts, and
/// its predicate must compare the field the page was sorted on.
#[cfg(test)]
mod cursor_keyset_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;
    use crate::types::{SortDirection, SortDirective};

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    fn keyset(sorts: &[&str]) -> Option<CursorKeyset> {
        let mut query = SearchQuery::new("Patient");
        for sort in sorts {
            query = query.with_sort(SortDirective::parse(sort));
        }
        CursorKeyset::for_query(&query)
    }

    #[test]
    fn default_sort_pages_over_last_updated_descending() {
        assert_eq!(
            keyset(&[]),
            Some(CursorKeyset {
                field: CursorKeysetField::LastUpdated,
                direction: SortDirection::Descending,
            })
        );
    }

    #[test]
    fn single_id_or_last_updated_sorts_have_a_keyset() {
        assert_eq!(
            keyset(&["_id"]),
            Some(CursorKeyset {
                field: CursorKeysetField::Id,
                direction: SortDirection::Ascending,
            })
        );
        assert_eq!(
            keyset(&["-_id"]),
            Some(CursorKeyset {
                field: CursorKeysetField::Id,
                direction: SortDirection::Descending,
            })
        );
        assert_eq!(
            keyset(&["-_lastUpdated"]),
            Some(CursorKeyset {
                field: CursorKeysetField::LastUpdated,
                direction: SortDirection::Descending,
            })
        );
        assert_eq!(
            keyset(&["_lastUpdated"]),
            Some(CursorKeyset {
                field: CursorKeysetField::LastUpdated,
                direction: SortDirection::Ascending,
            })
        );
    }

    /// Multi-field sorts and parameter sorts page by offset only: no keyset,
    /// so no cursor is minted and an inbound one is rejected.
    #[test]
    fn multi_field_and_parameter_sorts_have_no_keyset() {
        assert_eq!(keyset(&["_lastUpdated", "_id"]), None);
        assert_eq!(keyset(&["birthdate"]), None);
        assert_eq!(keyset(&["_id", "birthdate"]), None);
    }

    fn resource(id: &str, last_updated: &str) -> StoredResource {
        let ts = DateTime::parse_from_rfc3339(last_updated)
            .unwrap()
            .with_timezone(&Utc);
        StoredResource::from_storage(
            "Patient".to_string(),
            id.to_string(),
            "1".to_string(),
            crate::tenant::TenantId::new("t"),
            serde_json::json!({"resourceType": "Patient", "id": id}),
            ts,
            ts,
            None,
            FhirVersion::default(),
        )
    }

    /// The minted value is the sorted field's, and decoding it back through
    /// `build_cursor_condition` yields a predicate over that same field.
    #[test]
    fn id_sort_cursor_compares_id() {
        let k = keyset(&["_id"]).unwrap();
        let r = resource("p-5", "2026-01-02T03:04:05Z");
        let cursor =
            PageCursor::decode(&PageCursor::new(vec![k.value_of(&r)], r.id()).encode()).unwrap();
        assert!(matches!(
            cursor.sort_values().first(),
            Some(CursorValue::String(v)) if v == "p-5"
        ));

        let next = backend().build_cursor_condition(&cursor, &k).unwrap();
        assert_eq!(next, doc! { "id": { "$gt": "p-5" } });

        let prev = PageCursor::decode(&PageCursor::previous(vec![k.value_of(&r)], r.id()).encode())
            .unwrap();
        let previous = backend().build_cursor_condition(&prev, &k).unwrap();
        assert_eq!(previous, doc! { "id": { "$lt": "p-5" } });

        let desc = keyset(&["-_id"]).unwrap();
        let next_desc = backend().build_cursor_condition(&cursor, &desc).unwrap();
        assert_eq!(next_desc, doc! { "id": { "$lt": "p-5" } });
    }

    /// `_lastUpdated` keeps `id` (descending) as the tie-break, in both
    /// explicit directions and for the default sort.
    #[test]
    fn last_updated_sort_cursor_compares_last_updated_then_id() {
        let r = resource("p-5", "2026-01-02T03:04:05Z");
        let ts = Bson::DateTime(chrono_to_bson(r.last_modified()));

        let default = keyset(&[]).unwrap();
        let cursor =
            PageCursor::decode(&PageCursor::new(vec![default.value_of(&r)], r.id()).encode())
                .unwrap();
        assert!(matches!(
            cursor.sort_values().first(),
            Some(CursorValue::String(v)) if v == "2026-01-02T03:04:05+00:00"
        ));
        assert_eq!(
            backend().build_cursor_condition(&cursor, &default).unwrap(),
            doc! { "$or": [
                { "last_updated": { "$lt": ts.clone() } },
                { "last_updated": ts.clone(), "id": { "$lt": "p-5" } }
            ]}
        );

        let explicit_desc = keyset(&["-_lastUpdated"]).unwrap();
        assert_eq!(
            backend()
                .build_cursor_condition(&cursor, &explicit_desc)
                .unwrap(),
            backend().build_cursor_condition(&cursor, &default).unwrap()
        );

        let asc = keyset(&["_lastUpdated"]).unwrap();
        assert_eq!(
            backend().build_cursor_condition(&cursor, &asc).unwrap(),
            doc! { "$or": [
                { "last_updated": { "$gt": ts.clone() } },
                { "last_updated": ts.clone(), "id": { "$lt": "p-5" } }
            ]}
        );

        let prev =
            PageCursor::decode(&PageCursor::previous(vec![asc.value_of(&r)], r.id()).encode())
                .unwrap();
        assert_eq!(
            backend().build_cursor_condition(&prev, &asc).unwrap(),
            doc! { "$or": [
                { "last_updated": { "$lt": ts.clone() } },
                { "last_updated": ts, "id": { "$gt": "p-5" } }
            ]}
        );
    }

    /// A cursor whose value is not a timestamp is invalid for a
    /// `_lastUpdated` keyset — e.g. an `_id`-sort cursor replayed against a
    /// query whose `_sort` was changed by hand.
    #[test]
    fn cursor_value_must_match_the_keyset_field() {
        let cursor = PageCursor::new(vec![CursorValue::String("p-5".into())], "p-5");
        let err = backend()
            .build_cursor_condition(&cursor, &keyset(&[]).unwrap())
            .unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::InvalidCursor { .. })
        ));

        let numeric = PageCursor::new(vec![CursorValue::Number(5)], "p-5");
        let err = backend()
            .build_cursor_condition(&numeric, &keyset(&["_id"]).unwrap())
            .unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::InvalidCursor { .. })
        ));
    }
}

/// #1408: token `:of-type`, reference `:[type]`, `:identifier`, `:above` and
/// `:below` — refused as unsupported modifiers before.
#[cfg(test)]
mod modifier_parity_filter_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;

    const V2_0203: &str = "http://terminology.hl7.org/CodeSystem/v2-0203";

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    fn param(
        name: &str,
        param_type: SearchParamType,
        modifier: SearchModifier,
        values: &[&str],
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier: Some(modifier),
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            chain: vec![],
            components: vec![],
        }
    }

    fn filter(param: &SearchParameter) -> Document {
        backend()
            .build_search_index_filter("t1", "Observation", param)
            .unwrap()
    }

    fn subject(modifier: SearchModifier, value: &str) -> Document {
        filter(&param(
            "subject",
            SearchParamType::Reference,
            modifier,
            &[value],
        ))
    }

    fn patient() -> SearchModifier {
        SearchModifier::Type("Patient".to_string())
    }

    /// The envelope every filter carries, so a predicate is never evaluated
    /// against another tenant's, type's or parameter's rows.
    fn envelope(name: &str) -> Document {
        doc! { "tenant_id": "t1", "resource_type": "Observation", "param_name": name }
    }

    fn with(mut envelope: Document, predicate: Document) -> Document {
        envelope.extend(predicate);
        envelope
    }

    #[test]
    fn the_gate_admits_the_new_modifiers_and_still_refuses_terminology() {
        let backend = backend();
        let admitted = [
            param(
                "identifier",
                SearchParamType::Token,
                SearchModifier::OfType,
                &["s|c|v"],
            ),
            param("subject", SearchParamType::Reference, patient(), &["1"]),
            param(
                "subject",
                SearchParamType::Reference,
                SearchModifier::Identifier,
                &["s|v"],
            ),
            param(
                "subject",
                SearchParamType::Reference,
                SearchModifier::Above,
                &["http://example.org/fhir/Patient/1"],
            ),
            param(
                "subject",
                SearchParamType::Reference,
                SearchModifier::Below,
                &["http://example.org/fhir"],
            ),
        ];
        for param in admitted {
            let shown = format!("{param:?}");
            let query = SearchQuery::new("Observation").with_parameter(param);
            assert!(backend.validate_query_support(&query).is_ok(), "{shown}");
        }

        for modifier in [
            SearchModifier::In,
            SearchModifier::NotIn,
            SearchModifier::Above,
            SearchModifier::Below,
        ] {
            let query = SearchQuery::new("Observation").with_parameter(param(
                "code",
                SearchParamType::Token,
                modifier.clone(),
                &["http://loinc.org|1234-5"],
            ));
            assert!(
                matches!(
                    backend.validate_query_support(&query),
                    Err(StorageError::Search(
                        SearchError::UnsupportedModifier { .. }
                    ))
                ),
                "token :{modifier} must stay refused"
            );
        }
    }

    #[test]
    fn of_type_compares_type_system_type_code_and_value() {
        let of_type = |value: &str| {
            filter(&param(
                "identifier",
                SearchParamType::Token,
                SearchModifier::OfType,
                &[value],
            ))
        };

        assert_eq!(
            of_type(&format!("{V2_0203}|MR|12345")),
            with(
                envelope("identifier"),
                doc! {
                    "value_token_code": "12345",
                    "value_identifier_type_system": V2_0203,
                    "value_identifier_type_code": "MR",
                }
            )
        );
        // An empty part is not compared.
        assert_eq!(
            of_type("|MR|12345"),
            with(
                envelope("identifier"),
                doc! { "value_token_code": "12345", "value_identifier_type_code": "MR" }
            )
        );
        // The identifier value may itself contain a pipe.
        assert_eq!(
            of_type("|MR|a|b"),
            with(
                envelope("identifier"),
                doc! { "value_token_code": "a|b", "value_identifier_type_code": "MR" }
            )
        );
    }

    /// Fewer than three parts — or three empty ones — must match nothing:
    /// dropping the condition instead would return every resource of the type.
    #[test]
    fn a_malformed_of_type_value_matches_nothing() {
        for value in ["12345", "MR|12345", "||"] {
            let built = filter(&param(
                "identifier",
                SearchParamType::Token,
                SearchModifier::OfType,
                &[value],
            ));
            assert_eq!(
                built,
                with(envelope("identifier"), MongoBackend::match_nothing()),
                "{value}"
            );
        }
    }

    #[test]
    fn of_type_values_are_ored() {
        let built = filter(&param(
            "identifier",
            SearchParamType::Token,
            SearchModifier::OfType,
            &["|MR|1", "|SS|2"],
        ));
        assert_eq!(
            built.get_array("$or").unwrap(),
            &vec![
                Bson::Document(
                    doc! { "value_token_code": "1", "value_identifier_type_code": "MR" }
                ),
                Bson::Document(
                    doc! { "value_token_code": "2", "value_identifier_type_code": "SS" }
                ),
            ]
        );
    }

    /// `subject:Patient=123` is `subject=Patient/123`: the same filter, and so
    /// never `Group/123`.
    #[test]
    fn type_modifier_is_the_qualified_reference() {
        let plain = filter(&SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("Patient/123")],
            chain: vec![],
            components: vec![],
        });
        assert_eq!(
            plain,
            with(
                envelope("subject"),
                doc! { "$or": [
                    { "value_reference": "Patient/123" },
                    { "value_reference": { "$regex": "^Patient/123/_history/" } },
                ]}
            )
        );
        assert_eq!(subject(patient(), "123"), plain);
        assert_eq!(subject(patient(), "Patient/123"), plain);
        // Version-agnostic, with or without the modifier.
        assert_eq!(subject(patient(), "123/_history/2"), plain);
        assert_eq!(subject(patient(), "Patient/123/_history/2"), plain);
        assert_eq!(
            filter(&SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                modifier: None,
                values: vec![SearchValue::eq("Patient/123/_history/2")],
                chain: vec![],
                components: vec![],
            }),
            plain
        );
    }

    #[test]
    fn type_modifier_keeps_an_absolute_url_of_that_type() {
        assert_eq!(
            subject(patient(), "http://example.org/fhir/Patient/123"),
            with(
                envelope("subject"),
                doc! { "$or": [
                    { "value_reference": "http://example.org/fhir/Patient/123" },
                    { "value_reference": {
                        "$regex": "^http://example\\.org/fhir/Patient/123/_history/"
                    } },
                ]}
            )
        );
    }

    #[test]
    fn type_modifier_and_a_value_of_another_type_match_nothing() {
        for value in ["Group/123", "http://example.org/fhir/Group/123"] {
            assert_eq!(
                subject(patient(), value),
                with(envelope("subject"), MongoBackend::match_nothing()),
                "{value}"
            );
        }
    }

    #[test]
    fn type_modifier_escapes_regex_metacharacters() {
        let built = subject(patient(), "1.5");
        let arms = built.get_array("$or").unwrap();
        assert_eq!(
            arms[1].as_document().unwrap(),
            &doc! { "value_reference": { "$regex": "^Patient/1\\.5/_history/" } }
        );
    }

    #[test]
    fn reference_below_and_above_mirror_the_uri_shapes() {
        assert_eq!(
            subject(SearchModifier::Below, "http://example.org/fhir/"),
            with(
                envelope("subject"),
                doc! { "value_reference": { "$regex": "^http://example\\.org/fhir(/|$)" } }
            )
        );
        let above = subject(SearchModifier::Above, "http://example.org/fhir/Patient/1");
        let parents = above
            .get_document("value_reference")
            .unwrap()
            .get_array("$in")
            .unwrap();
        assert!(parents.contains(&Bson::String(
            "http://example.org/fhir/Patient/1".to_string()
        )));
        assert!(parents.contains(&Bson::String("http://example.org/fhir".to_string())));
    }

    /// `:identifier` needs the database; a path that did not resolve it must
    /// refuse rather than treat the identifier as a reference.
    #[test]
    fn an_unresolved_identifier_modifier_is_refused_by_the_builder() {
        let error = backend()
            .build_search_index_filter(
                "t1",
                "Observation",
                &param(
                    "subject",
                    SearchParamType::Reference,
                    SearchModifier::Identifier,
                    &["http://example.org/mrn|12345"],
                ),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                if modifier == "identifier"
        ));
    }

    #[test]
    fn identifier_predicates_follow_the_token_grammar() {
        assert_eq!(
            MongoBackend::identifier_predicate("http://example.org/mrn|12345"),
            doc! { "value_token_system": "http://example.org/mrn", "value_token_code": "12345" }
        );
        assert_eq!(
            MongoBackend::identifier_predicate("http://example.org/mrn|"),
            doc! { "value_token_system": "http://example.org/mrn" }
        );
        assert_eq!(
            MongoBackend::identifier_predicate("12345"),
            doc! { "value_token_code": "12345" }
        );
        assert_eq!(
            MongoBackend::identifier_predicate("|12345"),
            doc! {
                "value_token_system": { "$in": [Bson::Null, Bson::String(String::new())] },
                "value_token_code": "12345",
            }
        );
    }

    /// `|code` means "no system" (#1388): absent, empty, or the implicit
    /// marker of a `code` element — never any system, as a bare `code` does.
    #[test]
    fn token_without_system_matches_only_rows_without_one() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let param = SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![],
            chain: vec![],
            components: vec![],
        };
        assert_eq!(
            backend
                .build_token_filter(&param, &SearchValue::eq("|1234-5"))
                .unwrap(),
            doc! {
                "value_token_system": {
                    "$in": [
                        Bson::Null,
                        Bson::String(String::new()),
                        crate::search::IMPLICIT_TOKEN_SYSTEM,
                    ]
                },
                "value_token_code": "1234-5",
            }
        );
        assert_eq!(
            backend
                .build_token_filter(&param, &SearchValue::eq("1234-5"))
                .unwrap(),
            doc! { "value_token_code": "1234-5" }
        );
    }

    /// Each resolved target is matched exactly or at any version — never as a
    /// prefix of another id (`Patient/p1` must not admit `Patient/p10`).
    #[test]
    fn resolved_identifier_targets_are_one_bounded_in() {
        let built = MongoBackend::identifier_targets_filter(
            "t1",
            "Observation",
            "subject",
            vec!["Patient/p.1".to_string()],
        );
        assert_eq!(
            built,
            with(
                envelope("subject"),
                doc! { "value_reference": { "$in": [
                    Bson::RegularExpression(bson::Regex {
                        pattern: "^Patient/p\\.1/_history/".to_string(),
                        options: String::new(),
                    }),
                    Bson::String("Patient/p.1".to_string()),
                ]}}
            )
        );
    }
}
