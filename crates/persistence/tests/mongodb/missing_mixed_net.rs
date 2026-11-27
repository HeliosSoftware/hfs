//! #2233 Stage 2 characterization net: exact totals that combine a
//! metadata-answered `:missing=true` with `:not`, a metadata-supported
//! `:missing=false`, an unsupported `:missing` and resource-level conditions
//! (`_lastUpdated`, `_id:not`). It is pinned before those totals are
//! rewritten, and it must be green on `main` and after the rewrite alike.
//!
//! Every resource is written through the real write path (so rows and stamps
//! are genuine) and is then moved into one state by a raw update of its
//! `missing_params` / `search_rows_pending` / `is_deleted` / `last_updated`
//! fields. Seven parameters are registered on `Observation` and `Condition`:
//!
//! | index | code | type | role |
//! |---|---|---|---|
//! | `A`, `B`, `C` | `mx-a`, `mx-b`, `mx-c` | string | answered from `missing_params` |
//! | `U` | `mx-u` | string | made unsupported: its recorded definition digest is stale |
//! | `T` | `mx-t` | token (`valueCode`) | `:not` / `:missing=false` |
//! | `D` | `mx-d` | date (`valueDateTime`) | `:not` / `:missing=false` |
//! | `Q` | `mx-q` | quantity (`valueQuantity`) | `:not` / `:missing=false` |
//!
//! Values are labels (`alpha`, `beta`, `gamma`, `mNN`, `zNNN`) that no prefix
//! or range of another label matches, so string-prefix, date-range and
//! quantity-precision semantics need no modelling.
//!
//! Each query is compared with
//!
//! 1. an independent model of the documented semantics ([`Doc::matches`]),
//! 2. the ids obtained by walking every page,
//! 3. a test-only BSON copy of today's exact-total aggregation for mixed
//!    queries ([`reference_total`]), so a rewritten total is compared with the
//!    pipeline it replaces,
//! 4. the metadata-off route (value rows only) on the same database,
//!
//! and its total is routed where the cell intends: [`assert_route`] reads the
//! plan the total selected from the `helios_mongodb_plan` log events, because a
//! `:missing=false` parameter with at most 64 rows, for example, silently
//! takes the candidate collector instead. Each cell pins one total strategy:
//! [`MIXED_TOTAL`] for the totals the Stage 2 rewrite reroutes (one constant,
//! flipped on purpose by each production commit), [`PURE_TOTALS`] for pure
//! `:missing=true` totals, [`KEPT_TOTAL`] for totals no rewrite may reroute.
//!
//! The default tests read one rotating total door per shape;
//! `mongodb_missing_mixed_full_cross` (ignored) reads every door over the
//! full cross and is run once per production commit.
//!
//! The membership batch size of the rerouted totals can only be lowered from
//! inside the crate (a `cfg(test)` hook), so the batch-size cells (1, 2, 3
//! and 7 ids per batch) live in
//! `search_impl/search_plan/missing_checks_tests.rs`. The failure-injection
//! cells (a failure after every batch, in the stamped stream and in the
//! unstamped count) are in [`mongodb_missing_mixed_volume_concurrency_and_deadline`].

use super::*;
use futures::{StreamExt, TryStreamExt};
use helios_persistence::types::{ContainedMode, SummaryMode};
use mongodb::bson::DateTime as BsonDateTime;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::Duration;
use tracing::instrument::WithSubscriber;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Str,
    Tok,
    Date,
    Qty,
}

struct Param {
    code: &'static str,
    url: &'static str,
    kind: Kind,
}

const PARAMS: [Param; 7] = [
    Param {
        code: "mx-a",
        url: "urn:hfs:mx:a",
        kind: Kind::Str,
    },
    Param {
        code: "mx-b",
        url: "urn:hfs:mx:b",
        kind: Kind::Str,
    },
    Param {
        code: "mx-c",
        url: "urn:hfs:mx:c",
        kind: Kind::Str,
    },
    Param {
        code: "mx-u",
        url: "urn:hfs:mx:u",
        kind: Kind::Str,
    },
    Param {
        code: "mx-t",
        url: "urn:hfs:mx:t",
        kind: Kind::Tok,
    },
    Param {
        code: "mx-d",
        url: "urn:hfs:mx:d",
        kind: Kind::Date,
    },
    Param {
        code: "mx-q",
        url: "urn:hfs:mx:q",
        kind: Kind::Qty,
    },
];

const A: usize = 0;
const B: usize = 1;
const C: usize = 2;
const U: usize = 3;
const T: usize = 4;
const D: usize = 5;
const Q: usize = 6;

const MISSING_INDEX: &str = "idx_resources_missing_params";
const ALPHA: &str = "alpha";
const BETA: &str = "beta";
const GAMMA: &str = "gamma";

/// Day (of January 2026) of every `last_updated`; days repeat on purpose so
/// that `_lastUpdated` ties are the rule (#2654).
fn instant(day: u32) -> String {
    format!("2026-01-{day:02}T12:00:00Z")
}

fn bson_instant(day: u32) -> BsonDateTime {
    BsonDateTime::parse_rfc3339_str(instant(day)).unwrap()
}

/// The numeric suffix of an `mNN` / `zNNN` label.
fn label_number(label: &str) -> u32 {
    label[1..]
        .parse()
        .unwrap_or_else(|_| panic!("label {label}"))
}

fn date_of(label: &str) -> String {
    match label {
        ALPHA => "2026-02-01".to_owned(),
        BETA => "2026-02-02".to_owned(),
        GAMMA => "2026-02-03".to_owned(),
        other => {
            let n = label_number(other);
            if n < 31 {
                format!("2026-03-{:02}", n + 1)
            } else {
                format!("2026-04-{:02}", n - 30)
            }
        }
    }
}

fn quantity_of(label: &str) -> f64 {
    match label {
        ALPHA => 5.0,
        BETA => 50.0,
        GAMMA => 500.0,
        other if other.starts_with('m') => 1000.0 + f64::from(label_number(other)),
        other => 2000.0 + f64::from(label_number(other)),
    }
}

/// The search value that selects `label` on parameter `p`.
fn query_value(p: usize, label: &str) -> String {
    match PARAMS[p].kind {
        Kind::Str | Kind::Tok => label.to_owned(),
        Kind::Date => date_of(label),
        Kind::Qty => format!("{}", quantity_of(label) as i64),
    }
}

fn extension(p: usize, label: &str) -> serde_json::Value {
    let url = PARAMS[p].url;
    match PARAMS[p].kind {
        Kind::Str => json!({"url": url, "valueString": label}),
        Kind::Tok => json!({"url": url, "valueCode": label}),
        Kind::Date => json!({"url": url, "valueDateTime": date_of(label)}),
        Kind::Qty => json!({"url": url, "valueQuantity": {"value": quantity_of(label)}}),
    }
}

fn param_type(p: usize) -> SearchParamType {
    match PARAMS[p].kind {
        Kind::Str => SearchParamType::String,
        Kind::Tok => SearchParamType::Token,
        Kind::Date => SearchParamType::Date,
        Kind::Qty => SearchParamType::Quantity,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stamp {
    /// As the real write path stored it.
    Real,
    Absent,
    Empty,
    Null,
    /// Lists every parameter whatever the rows say (#2002).
    All,
}

const STAMPS: [Stamp; 5] = [
    Stamp::Real,
    Stamp::Absent,
    Stamp::Empty,
    Stamp::Null,
    Stamp::All,
];

#[derive(Clone, Debug)]
struct Doc {
    id: String,
    /// The labels of each parameter's values, in [`PARAMS`] order; a label
    /// may repeat (duplicate rows).
    values: Vec<Vec<String>>,
    stamp: Stamp,
    pending: bool,
    deleted: bool,
    day: u32,
}

fn labels(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn blank_values() -> Vec<Vec<String>> {
    vec![Vec::new(); PARAMS.len()]
}

#[derive(Clone, Debug, PartialEq)]
enum Crit {
    /// `code:missing=true` on a metadata-answered parameter (`A`, `B`, `C`).
    True(usize),
    /// `code:missing=false` on a metadata-supported parameter.
    False(usize),
    /// `code:not=v1,v2,...`
    Not(usize, Vec<String>),
    /// `mx-u:missing=true`, with `mx-u` unsupported by the metadata.
    UnsupTrue,
    /// `mx-u:missing=false`, with `mx-u` unsupported by the metadata.
    UnsupFalse,
    /// `code=value`, a positive criterion with more than 64 rows.
    Pos(usize, String),
}

#[derive(Clone, Debug)]
enum Extra {
    None,
    UpdatedGe(u32),
    UpdatedList(Vec<u32>),
    UpdatedRange(u32, u32),
    IdNot(String),
    /// A positive `_id=v1,v2,...`.
    Ids(Vec<String>),
}

#[derive(Clone, Debug)]
struct Shape {
    crits: Vec<Crit>,
    extra: Extra,
}

fn shape(crits: Vec<Crit>, extra: Extra) -> Shape {
    Shape { crits, extra }
}

fn not(p: usize, values: &[&str]) -> Crit {
    Crit::Not(p, labels(values))
}

/// `mx-a:not` with 300 values, one of which (`alpha`) exists.
fn not_huge() -> Crit {
    let mut values = vec![ALPHA.to_owned()];
    values.extend((0..299).map(|n| format!("z{n:03}")));
    Crit::Not(A, values)
}

fn label(shape: &Shape) -> String {
    let crits: Vec<String> = shape
        .crits
        .iter()
        .map(|crit| match crit {
            Crit::True(p) => format!("{}:missing=true", PARAMS[*p].code),
            Crit::False(p) => format!("{}:missing=false", PARAMS[*p].code),
            Crit::Not(p, values) if values.len() > 4 => {
                format!("{}:not=<{} values>", PARAMS[*p].code, values.len())
            }
            Crit::Not(p, values) => format!("{}:not={}", PARAMS[*p].code, values.join(",")),
            Crit::UnsupTrue => "mx-u:missing=true(unsupported)".to_owned(),
            Crit::UnsupFalse => "mx-u:missing=false(unsupported)".to_owned(),
            Crit::Pos(p, value) => format!("{}={value}", PARAMS[*p].code),
        })
        .collect();
    format!("{} {:?}", crits.join("&"), shape.extra)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Route {
    /// `missing_params` answers `A`, `B`, `C`.
    Metadata,
    /// `HFS_MONGODB_MISSING_METADATA=false`: value rows answer them.
    ValueRows,
}

impl Doc {
    fn has_any(&self, p: usize) -> bool {
        !self.values[p].is_empty()
    }

    fn has_one_of(&self, p: usize, wanted: &[String]) -> bool {
        self.values[p].iter().any(|value| wanted.contains(value))
    }

    fn extra_ok(&self, extra: &Extra) -> bool {
        match extra {
            Extra::None => true,
            Extra::UpdatedGe(day) => self.day >= *day,
            Extra::UpdatedList(days) => days.contains(&self.day),
            Extra::UpdatedRange(low, high) => self.day >= *low && self.day <= *high,
            Extra::IdNot(id) => &self.id != id,
            Extra::Ids(ids) => ids.contains(&self.id),
        }
    }

    /// The documented semantics of one document under one query.
    fn matches(&self, shape: &Shape, route: Route) -> bool {
        if self.deleted || !self.extra_ok(&shape.extra) {
            return false;
        }
        shape.crits.iter().all(|crit| match crit {
            Crit::True(p) => match route {
                Route::ValueRows => !self.has_any(*p),
                Route::Metadata => match self.stamp {
                    Stamp::Real => !self.has_any(*p),
                    Stamp::All => true,
                    Stamp::Empty | Stamp::Null => false,
                    Stamp::Absent => !self.pending && !self.has_any(*p),
                },
            },
            // Rows decide `:missing=false` and `:not`, never the metadata.
            Crit::False(p) => self.has_any(*p),
            Crit::Not(p, values) => !self.has_one_of(*p, values),
            // An unsupported `:missing` reads rows too: neither the stamp nor
            // the pending marker matters.
            Crit::UnsupTrue => !self.has_any(U),
            Crit::UnsupFalse => self.has_any(U),
            Crit::Pos(p, value) => self.values[*p].contains(value),
        })
    }
}

fn expected(docs: &[Doc], shape: &Shape, route: Route) -> Vec<String> {
    let mut ids: Vec<String> = docs
        .iter()
        .filter(|doc| doc.matches(shape, route))
        .map(|doc| doc.id.clone())
        .collect();
    ids.sort();
    ids
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

fn criterion(crit: &Crit) -> SearchParameter {
    let missing = |p: usize, value: &str| SearchParameter {
        name: PARAMS[p].code.to_owned(),
        param_type: param_type(p),
        modifier: Some(SearchModifier::Missing),
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    };
    match crit {
        Crit::True(p) => missing(*p, "true"),
        Crit::False(p) => missing(*p, "false"),
        Crit::UnsupTrue => missing(U, "true"),
        Crit::UnsupFalse => missing(U, "false"),
        Crit::Not(p, values) => SearchParameter {
            name: PARAMS[*p].code.to_owned(),
            param_type: param_type(*p),
            modifier: Some(SearchModifier::Not),
            values: values
                .iter()
                .map(|value| SearchValue::eq(query_value(*p, value)))
                .collect(),
            chain: vec![],
            components: vec![],
        },
        Crit::Pos(p, value) => SearchParameter {
            name: PARAMS[*p].code.to_owned(),
            param_type: param_type(*p),
            modifier: None,
            values: vec![SearchValue::eq(query_value(*p, value))],
            chain: vec![],
            components: vec![],
        },
    }
}

fn updated(values: Vec<SearchValue>) -> SearchParameter {
    SearchParameter {
        name: "_lastUpdated".into(),
        param_type: SearchParamType::Date,
        modifier: None,
        values,
        chain: vec![],
        components: vec![],
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sort {
    Default,
    IdAsc,
    IdDesc,
    UpdatedAsc,
    UpdatedDesc,
}

const SORTS: [Sort; 5] = [
    Sort::Default,
    Sort::IdAsc,
    Sort::IdDesc,
    Sort::UpdatedAsc,
    Sort::UpdatedDesc,
];

fn query(shape: &Shape, count: u32) -> SearchQuery {
    let mut query = SearchQuery::new("Observation").with_count(count);
    for crit in &shape.crits {
        query = query.with_parameter(criterion(crit));
    }
    match &shape.extra {
        Extra::None => {}
        Extra::UpdatedGe(day) => {
            query = query.with_parameter(updated(vec![SearchValue::new(
                SearchPrefix::Ge,
                instant(*day),
            )]));
        }
        Extra::UpdatedList(days) => {
            query = query.with_parameter(updated(
                days.iter()
                    .map(|day| SearchValue::eq(instant(*day)))
                    .collect(),
            ));
        }
        Extra::UpdatedRange(low, high) => {
            query = query
                .with_parameter(updated(vec![SearchValue::new(
                    SearchPrefix::Ge,
                    instant(*low),
                )]))
                .with_parameter(updated(vec![SearchValue::new(
                    SearchPrefix::Le,
                    instant(*high),
                )]));
        }
        Extra::IdNot(id) => {
            query = query.with_parameter(SearchParameter {
                name: "_id".into(),
                param_type: SearchParamType::Token,
                modifier: Some(SearchModifier::Not),
                values: vec![SearchValue::eq(id.clone())],
                chain: vec![],
                components: vec![],
            });
        }
        Extra::Ids(ids) => {
            query = query.with_parameter(SearchParameter {
                name: "_id".into(),
                param_type: SearchParamType::Token,
                modifier: None,
                values: ids.iter().map(|id| SearchValue::eq(id.clone())).collect(),
                chain: vec![],
                components: vec![],
            });
        }
    }
    query
}

fn sorted(query: SearchQuery, sort: Sort) -> SearchQuery {
    match sort {
        Sort::Default => query,
        Sort::IdAsc => query.with_sort(SortDirective::parse("_id")),
        Sort::IdDesc => query.with_sort(SortDirective::parse("-_id")),
        Sort::UpdatedAsc => query.with_sort(SortDirective::parse("_lastUpdated")),
        Sort::UpdatedDesc => query.with_sort(SortDirective::parse("-_lastUpdated")),
    }
}

// ---------------------------------------------------------------------------
// Route: which plan the exact total selected
// ---------------------------------------------------------------------------

/// Records the `helios_mongodb_plan` events of one total: the filter plan a
/// count selected (`span_field = "total_filter_strategy"`) and the total
/// strategy it executed.
#[derive(Clone, Default)]
struct PlanLog {
    filters: Arc<Mutex<Vec<String>>>,
    totals: Arc<Mutex<Vec<String>>>,
}

impl PlanLog {
    fn new() -> Self {
        pin_tracing_interest();
        Self::default()
    }
}

#[derive(Default)]
struct PlanVisitor {
    span_field: Option<String>,
    filter: Option<String>,
    total: Option<String>,
}

impl tracing::field::Visit for PlanVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "span_field" => self.span_field = Some(value.to_owned()),
            "filter_strategy" => self.filter = Some(value.to_owned()),
            "total_strategy" => self.total = Some(value.to_owned()),
            _ => {}
        }
    }
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl tracing::Subscriber for PlanLog {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "helios_mongodb_plan"
    }
    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::DEBUG)
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = PlanVisitor::default();
        event.record(&mut visitor);
        if visitor.span_field.as_deref() == Some("total_filter_strategy")
            && let Some(filter) = visitor.filter
        {
            self.filters.lock().unwrap().push(filter);
        }
        if let Some(total) = visitor.total {
            self.totals.lock().unwrap().push(total);
        }
    }
}

/// The total strategy of the mixed totals the Stage 2 rewrite reroutes: a
/// metadata `:missing=true` with `:not`, a metadata-supported
/// `:missing=false` or an unsupported `:missing=true`, and no positive
/// criterion. It was the single aggregation (`exact_aggregation`) until the
/// Stage 2 commit flipped it, on purpose, to the stamped/unstamped split
/// with batched row checks.
const MIXED_TOTAL: &str = "exact_missing_checks";

/// Pure metadata `:missing=true` totals: the single aggregation on `main`,
/// the stamped/unstamped split since Stage 1. Both trees must stay green.
const PURE_TOTALS: [&str; 2] = ["exact_aggregation", "exact_missing_split"];

/// Totals no rewrite of #2233 may reroute: a positive criterion next to the
/// `:missing=true` (a require-stage of the same driverless plan), an
/// unsupported `:missing=false` (the index driver), and the unsorted count
/// behind a parameter sort over the id cap. Never flipped.
const KEPT_TOTAL: &str = "exact_aggregation";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Expect {
    /// [`MIXED_TOTAL`] over the resource-lookup plan.
    Mixed,
    /// One of [`PURE_TOTALS`] over the resource-lookup plan.
    Pure,
    /// [`KEPT_TOTAL`] over the resource-lookup plan.
    Kept,
    /// The candidate collector (`resource_filter` / `exact_resources`).
    Collector,
    /// Not pinned: no metadata `:missing=true`, so not the #2233 route.
    Any,
}

/// The route a metadata-route cell is meant to take.
fn intended_route(shape: &Shape) -> Expect {
    let metadata = shape.crits.iter().any(|crit| matches!(crit, Crit::True(_)));
    let others = shape
        .crits
        .iter()
        .any(|crit| !matches!(crit, Crit::True(_)));
    let kept = shape
        .crits
        .iter()
        .any(|crit| matches!(crit, Crit::UnsupFalse | Crit::Pos(..)));
    match (metadata, matches!(shape.extra, Extra::Ids(_))) {
        // A positive `_id` seeds the candidates from the ids instead.
        (false, _) | (_, true) => Expect::Any,
        _ if kept => Expect::Kept,
        _ if others => Expect::Mixed,
        _ => Expect::Pure,
    }
}

fn assert_route(label: &str, expect: Expect, filters: &[String], totals: &[String]) {
    let lookups = || {
        assert!(
            !filters.is_empty() && filters.iter().all(|f| f == "resource_lookups"),
            "{label}: total filter plan {filters:?}, expected resource_lookups"
        );
    };
    match expect {
        Expect::Any => {}
        Expect::Mixed => {
            lookups();
            assert_eq!(totals, [MIXED_TOTAL.to_owned()], "{label}: mixed total");
        }
        Expect::Kept => {
            lookups();
            assert_eq!(totals, [KEPT_TOTAL.to_owned()], "{label}: kept total");
        }
        Expect::Pure => {
            lookups();
            assert!(
                totals.len() == 1 && PURE_TOTALS.contains(&totals[0].as_str()),
                "{label}: pure total strategy {totals:?}"
            );
        }
        Expect::Collector => {
            assert_eq!(
                totals,
                ["exact_resources".to_owned()],
                "{label}: total strategy, expected the candidate collector"
            );
            assert_eq!(
                filters,
                ["resource_filter".to_owned()],
                "{label}: filter plan"
            );
        }
    }
}

/// `search_count` with its plan events recorded.
async fn routed_count(
    backend: &MongoBackend,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> (u64, Vec<String>, Vec<String>) {
    let log = PlanLog::new();
    let total = backend
        .search_count(tenant, query)
        .with_subscriber(log.clone())
        .await
        .unwrap_or_else(|e| panic!("search_count {query:?} failed: {e}"));
    let filters = log.filters.lock().unwrap().clone();
    let totals = log.totals.lock().unwrap().clone();
    (total, filters, totals)
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

async fn register_parameters(backend: &MongoBackend, tenant: &TenantContext) {
    for param in &PARAMS {
        let kind = match param.kind {
            Kind::Str => "string",
            Kind::Tok => "token",
            Kind::Date => "date",
            Kind::Qty => "quantity",
        };
        let url = param.url;
        backend
            .seed_create(
                tenant,
                "SearchParameter",
                json!({
                    "resourceType": "SearchParameter", "id": param.code,
                    "url": format!("urn:hfs:test:{}", param.code), "name": param.code,
                    "status": "active", "code": param.code,
                    "base": ["Observation", "Condition"], "type": kind,
                    "expression": format!(
                        "Observation.extension.where(url = '{url}').value \
                         | Condition.extension.where(url = '{url}').value"
                    ),
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
}

fn resource_json(kind: &str, doc: &Doc) -> serde_json::Value {
    let extensions: Vec<_> = doc
        .values
        .iter()
        .enumerate()
        .flat_map(|(p, values)| values.iter().map(move |value| extension(p, value)))
        .collect();
    if kind == "Condition" {
        json!({
            "resourceType": "Condition", "id": doc.id,
            "subject": {"reference": "Patient/p"}, "extension": extensions,
        })
    } else {
        json!({
            "resourceType": "Observation", "id": doc.id, "status": "final",
            "code": {"coding": [{"code": "fixture"}]}, "extension": extensions,
        })
    }
}

async fn raw_db(backend: &MongoBackend) -> mongodb::Database {
    raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name)
}

/// Writes `docs` through the real path, then moves each into its state with
/// one raw update per distinct state.
async fn write_docs(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &TenantContext,
    kind: &str,
    docs: &[Doc],
) {
    futures::stream::iter(docs)
        .for_each_concurrent(4, |doc| async move {
            backend
                .seed_create(
                    tenant,
                    kind,
                    resource_json(kind, doc),
                    FhirVersion::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("fixture {kind}/{}: {e}", doc.id));
        })
        .await;
    let mut groups: BTreeMap<(Stamp, bool, bool, u32), Vec<String>> = BTreeMap::new();
    for doc in docs {
        groups
            .entry((doc.stamp, doc.pending, doc.deleted, doc.day))
            .or_default()
            .push(doc.id.clone());
    }
    let all_codes: Vec<&str> = PARAMS.iter().map(|param| param.code).collect();
    for ((stamp, pending, deleted, day), ids) in groups {
        let mut set = doc! { "last_updated": bson_instant(day), "is_deleted": deleted };
        let mut unset = Document::new();
        match stamp {
            Stamp::Real => {}
            Stamp::Absent => {
                unset.insert("missing_params", "");
            }
            Stamp::Empty => {
                set.insert("missing_params", Vec::<Bson>::new());
            }
            Stamp::Null => {
                set.insert("missing_params", Bson::Null);
            }
            Stamp::All => {
                set.insert("missing_params", all_codes.clone());
            }
        }
        if pending {
            set.insert("search_rows_pending", true);
        }
        let mut update = doc! { "$set": set };
        if !unset.is_empty() {
            update.insert("$unset", unset);
        }
        let result = db
            .collection::<Document>("resources")
            .update_many(
                doc! {
                    "tenant_id": tenant.tenant_id().as_str(),
                    "resource_type": kind,
                    "id": { "$in": ids.clone() },
                },
                update,
            )
            .await
            .unwrap();
        assert_eq!(
            result.matched_count as usize,
            ids.len(),
            "fixture state update"
        );
    }
}

/// The same ids with the opposite rows (a parameter without a value gets two,
/// one with values gets none) and the same stamp, live: they must never be
/// counted, so a total that drops its tenant or type condition fails.
fn twins(docs: &[Doc]) -> Vec<Doc> {
    docs.iter()
        .map(|doc| Doc {
            values: doc
                .values
                .iter()
                .map(|values| {
                    if values.is_empty() {
                        labels(&[ALPHA, BETA])
                    } else {
                        Vec::new()
                    }
                })
                .collect(),
            deleted: false,
            ..doc.clone()
        })
        .collect()
}

/// Makes `mx-u` unsupported by the metadata while `A`..`Q` stay answered from
/// it: one live stamped resource gets a fingerprint whose recorded catalog
/// entry differs from the current one in the digest of `mx-u` only.
/// Freshness is per code (#1999), so only `mx-u` leaves the metadata route.
async fn make_u_stale(db: &mongodb::Database, tenant: &TenantContext, carrier: &str) {
    let resources = db.collection::<Document>("resources");
    let filter = doc! {
        "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation", "id": carrier,
    };
    let current = resources
        .find_one(filter.clone())
        .await
        .unwrap()
        .expect("carrier resource")
        .get_str("missing_params_fp")
        .expect("carrier has a stamp")
        .to_owned();
    let catalog = db.collection::<Document>("missing_params_fingerprints");
    let started = std::time::Instant::now();
    let entry = loop {
        if let Some(entry) = catalog.find_one(doc! { "_id": &current }).await.unwrap() {
            break entry;
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "fingerprint {current} never reached the catalog"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let mut changed = 0;
    let params: Vec<Document> = entry
        .get_array("params")
        .unwrap()
        .iter()
        .map(|param| {
            let mut param = param.as_document().unwrap().clone();
            if param.get_str("code") == Ok(PARAMS[U].code) {
                param.insert("digest", "stale-2233-net");
                changed += 1;
            }
            param
        })
        .collect();
    assert_eq!(changed, 1, "mx-u must be in the current fingerprint");
    let stale = format!("stale-2233-{}", uuid::Uuid::new_v4().simple());
    catalog
        .insert_one(doc! { "_id": &stale, "params": params })
        .await
        .unwrap();
    let result = resources
        .update_one(filter, doc! { "$set": { "missing_params_fp": &stale } })
        .await
        .unwrap();
    assert_eq!(result.matched_count, 1);
}

struct Net {
    metadata: MongoBackend,
    /// Same database, `missing_metadata = false`.
    rows: MongoBackend,
    db: mongodb::Database,
    tenant: TenantContext,
    docs: Vec<Doc>,
}

async fn net(name: &str, docs: Vec<Doc>, with_twins: bool, stale_u: bool) -> Option<Net> {
    let metadata = create_backend(name).await?;
    let tenant = create_tenant(name);
    register_parameters(&metadata, &tenant).await;
    let db = raw_db(&metadata).await;
    write_docs(&metadata, &db, &tenant, "Observation", &docs).await;
    if with_twins {
        let opposite = twins(&docs);
        let foreign = create_tenant(&format!("{name}-foreign"));
        register_parameters(&metadata, &foreign).await;
        write_docs(&metadata, &db, &foreign, "Observation", &opposite).await;
        write_docs(&metadata, &db, &tenant, "Condition", &opposite).await;
    }
    if stale_u {
        let carrier = docs
            .iter()
            .find(|doc| doc.stamp == Stamp::Real && !doc.deleted)
            .expect("a live stamped resource");
        make_u_stale(&db, &tenant, &carrier.id).await;
    }
    let mut config = metadata.config().clone();
    config.missing_metadata = false;
    let rows = build_backend(config).await?;
    Some(Net {
        metadata,
        rows,
        db,
        tenant,
        docs,
    })
}

// ---------------------------------------------------------------------------
// Today's exact total, kept as test-only code
// ---------------------------------------------------------------------------

fn presence_field(p: usize) -> &'static str {
    match PARAMS[p].kind {
        Kind::Str => "value_string",
        Kind::Tok => "value_token_code",
        Kind::Date => "value_date",
        Kind::Qty => "value_quantity_value",
    }
}

fn presence(tenant: &TenantContext, p: usize) -> Document {
    doc! {
        "tenant_id": tenant.tenant_id().as_str(),
        "resource_type": "Observation",
        "param_name": PARAMS[p].code,
        presence_field(p): { "$ne": Bson::Null },
    }
}

/// The row condition that selects `label` on `p`, for this fixture's values.
fn value_condition(p: usize, label: &str) -> Document {
    match PARAMS[p].kind {
        Kind::Str => doc! { "value_string": label },
        Kind::Tok => doc! { "value_token_code": label },
        Kind::Date => {
            let start =
                BsonDateTime::parse_rfc3339_str(format!("{}T00:00:00Z", date_of(label))).unwrap();
            let end = BsonDateTime::from_millis(start.timestamp_millis() + 86_400_000);
            doc! { "value_date": { "$gte": start, "$lt": end } }
        }
        Kind::Qty => {
            let value = quantity_of(label);
            doc! { "value_quantity_value": { "$gte": value - 0.5, "$lt": value + 0.5 } }
        }
    }
}

fn existence_triplet(filter: Document, exclude: bool) -> Vec<Document> {
    vec![
        doc! { "$lookup": {
            "from": "search_index",
            "let": { "candidate_id": "$id" },
            "pipeline": [
                { "$match": { "$and": [
                    filter,
                    { "$expr": { "$eq": ["$resource_id", "$$candidate_id"] } },
                ] } },
                { "$limit": 1 },
                { "$project": { "_id": 1 } },
            ],
            "as": "_hfs_search_match",
        } },
        doc! { "$match": { "_hfs_search_match.0": { "$exists": !exclude } } },
        doc! { "$unset": "_hfs_search_match" },
    ]
}

fn fallback_triplet(presence: Document) -> Vec<Document> {
    vec![
        doc! { "$lookup": {
            "from": "search_index",
            "let": { "candidate_id": { "$cond": [
                { "$eq": [{ "$type": "$missing_params" }, "missing"] },
                "$id",
                false,
            ] } },
            "pipeline": [
                { "$match": { "$and": [
                    presence,
                    { "$expr": { "$eq": ["$resource_id", "$$candidate_id"] } },
                ] } },
                { "$limit": 1 },
                { "$project": { "_id": 1 } },
            ],
            "as": "_hfs_missing_present",
        } },
        doc! { "$match": { "_hfs_missing_present.0": { "$exists": false } } },
        doc! { "$unset": "_hfs_missing_present" },
    ]
}

/// A copy of today's exact-total aggregation for a query with at least one
/// metadata `:missing=true`: one `$match` with the `$or` of every metadata
/// name, then, in query order, one fallback triplet per metadata name and one
/// existence triplet per other criterion, then `$count`. It must keep
/// returning what the product returns, whatever the product does internally.
async fn reference_total(
    db: &mongodb::Database,
    tenant: &TenantContext,
    shape: &Shape,
) -> Option<u64> {
    if !shape.crits.iter().any(|crit| matches!(crit, Crit::True(_))) {
        return None;
    }
    let mut conditions = vec![doc! {
        "tenant_id": tenant.tenant_id().as_str(),
        "resource_type": "Observation",
        "is_deleted": false,
    }];
    match &shape.extra {
        Extra::None => {}
        Extra::UpdatedGe(day) => {
            conditions.push(doc! { "last_updated": { "$gte": bson_instant(*day) } });
        }
        Extra::UpdatedList(days) => {
            let any: Vec<Document> = days
                .iter()
                .map(|day| doc! { "last_updated": bson_instant(*day) })
                .collect();
            conditions.push(doc! { "$or": any });
        }
        Extra::UpdatedRange(low, high) => conditions.push(doc! {
            "last_updated": { "$gte": bson_instant(*low), "$lte": bson_instant(*high) },
        }),
        Extra::IdNot(id) => conditions.push(doc! { "id": { "$ne": id } }),
        Extra::Ids(ids) => conditions.push(doc! { "id": { "$in": ids.clone() } }),
    }
    let mut stages = Vec::new();
    for crit in &shape.crits {
        match crit {
            Crit::True(p) => {
                conditions.push(doc! { "$or": [
                    { "missing_params": PARAMS[*p].code },
                    { "missing_params": { "$exists": false },
                      "search_rows_pending": { "$exists": false } },
                ] });
                stages.extend(fallback_triplet(presence(tenant, *p)));
            }
            Crit::False(p) => stages.extend(existence_triplet(presence(tenant, *p), false)),
            Crit::UnsupTrue => stages.extend(existence_triplet(presence(tenant, U), true)),
            Crit::UnsupFalse => stages.extend(existence_triplet(presence(tenant, U), false)),
            Crit::Not(p, values) => {
                let mut filter = doc! {
                    "tenant_id": tenant.tenant_id().as_str(),
                    "resource_type": "Observation",
                    "param_name": PARAMS[*p].code,
                };
                let any: Vec<Document> = values
                    .iter()
                    .map(|value| value_condition(*p, value))
                    .collect();
                filter.insert("$or", any);
                stages.extend(existence_triplet(filter, true));
            }
            Crit::Pos(p, value) => {
                let mut filter = doc! {
                    "tenant_id": tenant.tenant_id().as_str(),
                    "resource_type": "Observation",
                    "param_name": PARAMS[*p].code,
                };
                filter.extend(value_condition(*p, value));
                stages.extend(existence_triplet(filter, false));
            }
        }
    }
    let mut pipeline = vec![doc! { "$match": { "$and": conditions } }];
    pipeline.extend(stages);
    pipeline.push(doc! { "$count": "total" });
    let mut cursor = db
        .collection::<Document>("resources")
        .aggregate(pipeline)
        .hint(mongodb::options::Hint::Name(MISSING_INDEX.to_owned()))
        .await
        .unwrap();
    let total = match cursor.try_next().await.unwrap() {
        Some(row) => u64::try_from(row.get_i32("total").unwrap()).unwrap(),
        None => 0,
    };
    Some(total)
}

// ---------------------------------------------------------------------------
// Reads and checks
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Stats {
    shapes: usize,
    comparisons: usize,
    nonzero: usize,
    largest: usize,
    routed: usize,
}

impl Stats {
    fn report(&self, test: &str, docs: usize) {
        eprintln!(
            "[2233-net2] {test}: docs={docs} shapes={} comparisons={} nonzero_totals={} \
             largest_total={} routed_cells={} mismatches=0",
            self.shapes, self.comparisons, self.nonzero, self.largest, self.routed
        );
    }
}

async fn page(
    backend: &MongoBackend,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> (Vec<String>, helios_persistence::types::PageInfo) {
    let result = backend
        .search(tenant, query)
        .await
        .unwrap_or_else(|e| panic!("search {query:?} failed: {e}"));
    let ids = result
        .resources
        .items
        .iter()
        .map(|row| row.id().to_owned())
        .collect();
    (ids, result.resources.page_info)
}

/// Forward by `next` cursors; with `back`, then back by `previous` cursors.
async fn walk(
    backend: &MongoBackend,
    tenant: &TenantContext,
    shape: &Shape,
    sort: Sort,
    count: u32,
    back: bool,
) -> (Vec<String>, Option<Vec<String>>) {
    let label = label(shape);
    let mut forward = Vec::new();
    let mut q = sorted(query(shape, count), sort);
    let mut pages = 0;
    let mut last_len;
    let last = loop {
        let (ids, info) = page(backend, tenant, &q).await;
        last_len = ids.len();
        forward.extend(ids);
        pages += 1;
        assert!(pages < 400, "{label} {sort:?}: paging never ended");
        match (info.has_next, info.next_cursor.clone()) {
            (true, Some(cursor)) => q = sorted(query(shape, count), sort).with_cursor(cursor),
            (true, None) => panic!("{label} {sort:?}: has_next without a cursor"),
            _ => break info,
        }
    };
    if !back {
        return (forward, None);
    }
    let mut backward: Vec<String> = forward[forward.len() - last_len..].to_vec();
    let mut previous = last.previous_cursor.clone();
    let mut back_pages = 0;
    while let Some(cursor) = previous {
        let (ids, info) = page(
            backend,
            tenant,
            &sorted(query(shape, count), sort).with_cursor(cursor),
        )
        .await;
        back_pages += 1;
        assert!(
            back_pages < 400,
            "{label} {sort:?}: paging back never ended"
        );
        let mut joined = ids;
        joined.extend(backward);
        backward = joined;
        previous = match (info.has_previous, info.previous_cursor) {
            (true, Some(cursor)) => Some(cursor),
            _ => None,
        };
    }
    (forward, Some(backward))
}

/// The ways a caller can ask for the exact total besides `search_count`,
/// including the totals of two sorted searches.
const DOORS: [&str; 6] = [
    "_total=accurate",
    "_total=estimate",
    "_sort=_id&_total=accurate",
    "_sort=-_lastUpdated&_total=accurate",
    "_summary=count",
    "_contained=both",
];

/// The total through door `door` of [`DOORS`], or `None` for a door that
/// rightly refuses the query.
async fn door_total(
    backend: &MongoBackend,
    tenant: &TenantContext,
    shape: &Shape,
    door: usize,
) -> Option<u64> {
    let name = DOORS[door];
    let mut q = match door {
        2 => sorted(query(shape, 1), Sort::IdAsc),
        3 => sorted(query(shape, 1), Sort::UpdatedDesc),
        5 => query(shape, 25),
        _ => query(shape, 1),
    };
    q.total = Some(if door == 1 {
        TotalMode::Estimate
    } else {
        TotalMode::Accurate
    });
    if door == 4 {
        q.summary = Some(SummaryMode::Count);
    }
    if door == 5 {
        q.contained = ContainedMode::Both;
    }
    match backend.search(tenant, &q).await {
        Ok(result) => Some(result.total.expect("total requested")),
        // MongoDB rejects `:missing` / `:not` next to `_contained=both`; only
        // that exact rejection is tolerated, any other error is a failure.
        Err(err)
            if door == 5
                && err
                    .to_string()
                    .contains("cannot be combined with _contained") =>
        {
            None
        }
        Err(err) => panic!("{} {name}: {err}", label(shape)),
    }
}

fn order_ok(days: &HashMap<String, u32>, ids: &[String], sort: Sort, label: &str) {
    let day = |id: &String| days[id];
    for pair in ids.windows(2) {
        match sort {
            Sort::Default => {}
            Sort::IdAsc => assert!(pair[0] < pair[1], "{label}: _id order {pair:?}"),
            Sort::IdDesc => assert!(pair[0] > pair[1], "{label}: -_id order {pair:?}"),
            // #2654: equal instants are ordered by ascending id.
            Sort::UpdatedAsc => assert!(
                (day(&pair[0]), &pair[0]) < (day(&pair[1]), &pair[1]),
                "{label}: _lastUpdated order {pair:?}"
            ),
            Sort::UpdatedDesc => assert!(
                day(&pair[0]) >= day(&pair[1]),
                "{label}: -_lastUpdated order {pair:?}"
            ),
        }
    }
}

struct Walk {
    sort: Sort,
    count: u32,
    back: bool,
    /// The one door of [`DOORS`] read besides `search_count`, rotating per
    /// shape; `None` reads every door (the full cross).
    door: Option<usize>,
}

/// All the equalities for one query on one route.
async fn check(net: &Net, shape: &Shape, route: Route, walk_with: &Walk, stats: &mut Stats) {
    let backend = match route {
        Route::Metadata => &net.metadata,
        Route::ValueRows => &net.rows,
    };
    let want = expected(&net.docs, shape, route);
    let label = format!("{} [{route:?}]", label(shape));
    stats.shapes += 1;
    if !want.is_empty() {
        stats.nonzero += 1;
    }
    stats.largest = stats.largest.max(want.len());

    let (count, filters, strategies) = routed_count(backend, &net.tenant, &query(shape, 1)).await;
    assert_eq!(count, want.len() as u64, "{label}: search_count");
    stats.comparisons += 1;
    if route == Route::Metadata {
        let expect = intended_route(shape);
        assert_route(&label, expect, &filters, &strategies);
        if expect != Expect::Any {
            stats.routed += 1;
        }
    }

    let doors: Vec<usize> = match walk_with.door {
        // `_contained=both` refuses every `:missing` / `:not` query, so the
        // rotation leaves it to the full cross.
        Some(door) => vec![door % (DOORS.len() - 1)],
        None => (0..DOORS.len()).collect(),
    };
    for door in doors {
        if let Some(total) = door_total(backend, &net.tenant, shape, door).await {
            assert_eq!(total, want.len() as u64, "{label}: {}", DOORS[door]);
            stats.comparisons += 1;
        }
    }

    if route == Route::Metadata
        && let Some(old) = reference_total(&net.db, &net.tenant, shape).await
    {
        assert_eq!(old, want.len() as u64, "{label}: reference pipeline");
        stats.comparisons += 1;
    }

    let (forward, backward) = walk(
        backend,
        &net.tenant,
        shape,
        walk_with.sort,
        walk_with.count,
        walk_with.back,
    )
    .await;
    let unique: BTreeSet<_> = forward.iter().collect();
    assert_eq!(
        unique.len(),
        forward.len(),
        "{label} {:?}: repeated id",
        walk_with.sort
    );
    let mut seen = forward.clone();
    seen.sort();
    assert_eq!(seen, want, "{label} {:?}: page walk", walk_with.sort);
    stats.comparisons += 1;
    if let Some(backward) = backward {
        assert_eq!(backward, forward, "{label} {:?}: walk back", walk_with.sort);
        stats.comparisons += 1;
    }
    let days: HashMap<String, u32> = net
        .docs
        .iter()
        .map(|doc| (doc.id.clone(), doc.day))
        .collect();
    order_ok(
        &days,
        &forward,
        walk_with.sort,
        &format!("{label} {:?}", walk_with.sort),
    );
}

/// `_sort=mx-b&_total=accurate`: the total, and the first page in the order a
/// parameter sort documents (key ascending, ties and resources without a key
/// by ascending id, resources without a key last).
async fn check_param_sort(net: &Net, shape: &Shape, stats: &mut Stats) {
    let label = label(shape);
    let want = expected(&net.docs, shape, Route::Metadata);
    let mut q = query(shape, 20).with_sort(SortDirective::parse(PARAMS[B].code));
    q.total = Some(TotalMode::Accurate);
    let result = net
        .metadata
        .search(&net.tenant, &q)
        .await
        .unwrap_or_else(|e| panic!("{label} _sort=mx-b: {e}"));
    assert_eq!(
        result.total,
        Some(want.len() as u64),
        "{label}: _sort=mx-b total"
    );
    let ids: Vec<String> = result
        .resources
        .items
        .iter()
        .map(|row| row.id().to_owned())
        .collect();
    let by_id: HashMap<&str, &Doc> = net.docs.iter().map(|doc| (doc.id.as_str(), doc)).collect();
    let mut order: Vec<(Option<String>, String)> = want
        .iter()
        .map(|id| {
            let key = by_id[id.as_str()].values[B].iter().min().cloned();
            (key, id.clone())
        })
        .collect();
    order.sort_by(|left, right| match (&left.0, &right.0) {
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => left.cmp(right),
    });
    let first: Vec<String> = order.into_iter().take(20).map(|(_, id)| id).collect();
    assert_eq!(ids, first, "{label}: _sort=mx-b first page");
    stats.shapes += 1;
    stats.comparisons += 2;
}

fn extras(docs: &[Doc]) -> Vec<Extra> {
    let live_lacking_a = |stamp: Stamp| {
        docs.iter()
            .find(|doc| doc.stamp == stamp && !doc.pending && !doc.deleted && !doc.has_any(A))
            .map(|doc| doc.id.clone())
            .expect("an IdNot target")
    };
    vec![
        Extra::UpdatedGe(4),
        Extra::UpdatedList(vec![2, 5]),
        Extra::UpdatedRange(3, 5),
        // Counted by `mx-a:missing=true` on the stamped side.
        Extra::IdNot(live_lacking_a(Stamp::Real)),
        // Counted by `mx-a:missing=true` on the unstamped side.
        Extra::IdNot(live_lacking_a(Stamp::Absent)),
        Extra::IdNot("no-such-id".into()),
    ]
}

// ---------------------------------------------------------------------------
// Fixtures and shapes
// ---------------------------------------------------------------------------

/// The exhaustive states of the metadata, token and unsupported parameters:
/// `mx-a` none / alpha / beta / both, `mx-b` none / alpha, `mx-c` none /
/// beta, `mx-u` none / alpha, `mx-t` none / alpha / beta, crossed with every
/// stamp, the pending marker and `is_deleted` (1,920 resources), plus eight
/// live unstamped resources that alone carry day 7.
fn exhaustive_docs() -> Vec<Doc> {
    let a_states = [vec![], vec![ALPHA], vec![BETA], vec![ALPHA, BETA]];
    let two = |value: &'static str| [vec![], vec![value]];
    let t_states = [vec![], vec![ALPHA], vec![BETA]];
    let mut docs = Vec::new();
    for stamp in STAMPS {
        for pending in [false, true] {
            for deleted in [false, true] {
                for a in &a_states {
                    for b in &two(ALPHA) {
                        for c in &two(BETA) {
                            for u in &two(ALPHA) {
                                for t in &t_states {
                                    let n = docs.len() as u32;
                                    let mut values = blank_values();
                                    values[A] = labels(a);
                                    values[B] = labels(b);
                                    values[C] = labels(c);
                                    values[U] = labels(u);
                                    values[T] = labels(t);
                                    docs.push(Doc {
                                        id: format!("e{n:04}"),
                                        values,
                                        stamp,
                                        pending,
                                        deleted,
                                        day: 1 + n % 6,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // With `_lastUpdated` on day 7 the stamped side of a total is empty
    // while the unstamped side has matches.
    for k in 0..8u32 {
        let mut values = blank_values();
        if k & 1 != 0 {
            values[T] = labels(&[ALPHA]);
        }
        if k & 2 != 0 {
            values[B] = labels(&[ALPHA]);
        }
        if k & 4 != 0 {
            values[A] = labels(&[BETA]);
        }
        docs.push(Doc {
            id: format!("e9{k:03}"),
            values,
            stamp: Stamp::Absent,
            pending: false,
            deleted: false,
            day: 7,
        });
    }
    docs
}

/// The criteria crossed with every `:missing=true` set (the 300-value `:not`
/// joins them in the full cross only).
fn other_criteria() -> Vec<Crit> {
    vec![
        not(A, &[ALPHA]),
        not(A, &[BETA]),
        not(A, &[ALPHA, BETA]),
        not(A, &[ALPHA, BETA, GAMMA]),
        not(B, &[ALPHA]),
        not(C, &[BETA]),
        not(T, &[ALPHA]),
        not(T, &[ALPHA, BETA]),
        not(U, &[ALPHA]),
        Crit::False(A),
        Crit::False(B),
        Crit::False(T),
        Crit::UnsupTrue,
    ]
}

fn missing_sets() -> Vec<Vec<Crit>> {
    use Crit::True;
    vec![
        vec![True(A)],
        vec![True(B)],
        vec![True(C)],
        vec![True(A), True(B)],
        vec![True(B), True(A)],
        vec![True(A), True(C)],
        vec![True(A), True(A)],
        vec![True(A), True(B), True(C)],
        vec![True(C), True(B), True(A)],
        vec![True(B), True(B), True(C)],
    ]
}

fn pos(p: usize, value: &str) -> Crit {
    Crit::Pos(p, value.to_owned())
}

/// The ids `_id:not` targets in [`extras`]: a stamped and an unstamped match
/// of `mx-a:missing=true`.
fn id_targets(docs: &[Doc]) -> Vec<String> {
    extras(docs)
        .into_iter()
        .filter_map(|extra| match extra {
            Extra::IdNot(id) if id != "no-such-id" => Some(id),
            _ => None,
        })
        .collect()
}

fn exhaustive_shapes(docs: &[Doc], full: bool) -> Vec<Shape> {
    use Crit::{False, True, UnsupFalse, UnsupTrue};
    let mut criteria = other_criteria();
    if full {
        criteria.push(not_huge());
    }
    let mut shapes = Vec::new();
    // Each criterion after and before every `:missing=true` set.
    for missing in missing_sets() {
        for other in &criteria {
            let mut after = missing.clone();
            after.push(other.clone());
            shapes.push(shape(after, Extra::None));
            let mut before = vec![other.clone()];
            before.extend(missing.clone());
            shapes.push(shape(before, Extra::None));
        }
    }
    // Resource-level conditions: crossed with every criterion in the full
    // cross, with a `:not` and a `:missing=false` otherwise.
    let crossed = if full {
        criteria.clone()
    } else {
        vec![not(T, &[ALPHA]), False(B)]
    };
    for missing in [vec![True(A)], vec![True(B), True(A), True(C)]] {
        for other in &crossed {
            for extra in extras(docs) {
                let mut crits = missing.clone();
                crits.push(other.clone());
                shapes.push(shape(crits, extra));
            }
        }
    }
    let ids = extras(docs);
    if !full {
        // The 300-value `:not` in three shapes.
        shapes.push(shape(vec![True(A), not_huge()], Extra::None));
        shapes.push(shape(vec![not_huge(), True(B), True(A)], Extra::None));
        shapes.push(shape(vec![True(A), not_huge()], ids[4].clone()));
    }
    // Pairs of criteria.
    let pairs = [
        vec![not(A, &[ALPHA]), False(B)],
        vec![not(A, &[ALPHA]), not(A, &[BETA])],
        vec![not(A, &[ALPHA]), not(T, &[ALPHA])],
        vec![False(B), UnsupTrue],
        vec![not(T, &[ALPHA, BETA]), False(A)],
        vec![UnsupFalse],
        vec![UnsupFalse, not(T, &[ALPHA])],
    ];
    for missing in [vec![True(A)], vec![True(A), True(B)]] {
        for pair in &pairs {
            let mut crits = missing.clone();
            crits.extend(pair.clone());
            shapes.push(shape(crits, Extra::None));
        }
    }
    shapes.push(shape(
        vec![True(A), not(T, &[ALPHA]), False(B)],
        Extra::UpdatedRange(2, 4),
    ));
    shapes.push(shape(
        vec![True(A), not(T, &[ALPHA]), False(B)],
        ids[4].clone(),
    ));
    shapes.push(shape(vec![True(A), UnsupTrue, False(C)], ids[3].clone()));
    // A positive criterion next to `:missing=true` and `:not` (or
    // `:missing=false`), both orders: a require-stage of the same driverless
    // plan, which keeps the single aggregation.
    for crits in [
        vec![True(A), pos(T, ALPHA), not(B, &[ALPHA])],
        vec![not(B, &[ALPHA]), pos(T, ALPHA), True(A)],
        vec![True(A), True(C), pos(B, ALPHA), not(T, &[BETA])],
        vec![not(T, &[BETA]), pos(B, ALPHA), True(C), True(A)],
        vec![True(A), pos(B, ALPHA), False(T)],
        vec![UnsupTrue, pos(T, ALPHA), True(B)],
    ] {
        shapes.push(shape(crits, Extra::None));
    }
    shapes.push(shape(
        vec![True(A), pos(T, ALPHA), not(A, &[BETA])],
        Extra::UpdatedGe(3),
    ));
    // Day 7: no stamped resource, unstamped matches only.
    shapes.push(shape(
        vec![True(A), not(T, &[ALPHA])],
        Extra::UpdatedList(vec![7]),
    ));
    shapes.push(shape(vec![True(A), False(B)], Extra::UpdatedGe(7)));
    shapes.push(shape(
        vec![True(A), True(B), UnsupTrue],
        Extra::UpdatedRange(7, 7),
    ));
    // A positive `_id` (the ids seed the candidates instead).
    let mut targets = id_targets(docs);
    targets.push("no-such-id".into());
    shapes.push(shape(
        vec![True(A), not(T, &[ALPHA])],
        Extra::Ids(targets.clone()),
    ));
    shapes.push(shape(
        vec![True(A), False(B)],
        Extra::Ids(targets[..1].to_vec()),
    ));
    // Same-name contradictions.
    shapes.push(shape(vec![True(A), False(A)], Extra::None));
    shapes.push(shape(vec![True(A), not(A, &[ALPHA])], Extra::None));
    shapes.push(shape(
        vec![True(A), not(A, &[ALPHA]), False(A)],
        Extra::None,
    ));
    // Many criteria.
    shapes.push(shape(
        vec![
            True(A),
            True(B),
            True(C),
            not(A, &[ALPHA]),
            not(B, &[ALPHA]),
            not(T, &[BETA]),
            False(T),
        ],
        Extra::None,
    ));
    // Without a metadata `:missing=true` (not the rewritten route).
    shapes.push(shape(vec![not(A, &[ALPHA])], Extra::None));
    shapes.push(shape(vec![False(B), not(T, &[ALPHA])], Extra::None));
    shapes.push(shape(vec![UnsupTrue], Extra::None));
    shapes
}

async fn run_exhaustive(test: &str, full: bool) {
    let Some(net) = net(
        &format!(
            "missing_mixed_exhaustive{}",
            if full { "_full" } else { "" }
        ),
        exhaustive_docs(),
        true,
        true,
    )
    .await
    else {
        eprintln!("Skipping {test} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };

    // The fixture must tell the states apart, or a wrong rewrite would pass.
    let count =
        |crits: Vec<Crit>, route| expected(&net.docs, &shape(crits, Extra::None), route).len();
    let mixed = vec![Crit::True(A), not(T, &[ALPHA])];
    assert!(count(mixed.clone(), Route::Metadata) > 0);
    // Dropping the `:not` check on either side changes the total.
    let stamped_excluded = net.docs.iter().any(|doc| {
        doc.stamp == Stamp::Real
            && doc.matches(&shape(vec![Crit::True(A)], Extra::None), Route::Metadata)
            && doc.has_one_of(T, &labels(&[ALPHA]))
    });
    let unstamped_excluded = net.docs.iter().any(|doc| {
        doc.stamp == Stamp::Absent
            && doc.matches(&shape(vec![Crit::True(A)], Extra::None), Route::Metadata)
            && doc.has_one_of(T, &labels(&[ALPHA]))
    });
    assert!(stamped_excluded && unstamped_excluded);
    // Day 7: an empty stamped side next to unstamped matches.
    let day7 = shape(mixed.clone(), Extra::UpdatedList(vec![7]));
    let day7_matches: Vec<&Doc> = net
        .docs
        .iter()
        .filter(|doc| doc.matches(&day7, Route::Metadata))
        .collect();
    assert!(!day7_matches.is_empty());
    assert!(day7_matches.iter().all(|doc| doc.stamp == Stamp::Absent));
    assert!(
        net.docs
            .iter()
            .filter(|doc| doc.day == 7)
            .all(|doc| doc.stamp == Stamp::Absent)
    );
    // `mx-u` really is on the row route: the model of an unsupported
    // `:missing=true` differs from what the metadata would answer.
    assert_ne!(
        count(vec![Crit::True(A), Crit::UnsupTrue], Route::Metadata),
        count(vec![Crit::True(A), Crit::True(U)], Route::Metadata),
        "the unsupported parameter must be told apart from a metadata one"
    );
    assert_ne!(
        count(vec![Crit::True(A)], Route::Metadata),
        count(vec![Crit::True(A)], Route::ValueRows),
        "metadata and value routes must differ on this fixture"
    );

    // Positive control of the route assertion: `mx-d` has no row in this
    // type, so `mx-d:missing=false` is selective (at most 64 rows) and the
    // total silently takes the candidate collector.
    let control = shape(vec![Crit::True(A), Crit::False(D)], Extra::None);
    let (total, filters, strategies) =
        routed_count(&net.metadata, &net.tenant, &query(&control, 1)).await;
    assert_eq!(total, 0);
    assert_route(&label(&control), Expect::Collector, &filters, &strategies);
    let mut stats = Stats::default();
    stats.routed += 1;

    let shapes = exhaustive_shapes(&net.docs, full);
    for (n, shape) in shapes.iter().enumerate() {
        let sort = SORTS[n % SORTS.len()];
        check(
            &net,
            shape,
            Route::Metadata,
            &Walk {
                sort,
                count: 400,
                back: n % 7 == 0,
                door: (!full).then_some(n),
            },
            &mut stats,
        )
        .await;
        if n % 4 == 0 {
            check(
                &net,
                shape,
                Route::ValueRows,
                &Walk {
                    sort: Sort::Default,
                    count: 400,
                    back: false,
                    door: (!full).then_some(n + 1),
                },
                &mut stats,
            )
            .await;
        }
    }
    // A search-parameter sort within the id cap.
    for crits in [
        vec![Crit::True(A), not(T, &[ALPHA])],
        vec![not(T, &[ALPHA, BETA]), Crit::True(C), Crit::False(A)],
        vec![Crit::True(A), Crit::UnsupTrue],
    ] {
        check_param_sort(&net, &shape(crits, Extra::None), &mut stats).await;
    }
    stats.report(test, net.docs.len());
}

#[tokio::test]
async fn mongodb_missing_mixed_exhaustive_states() {
    run_exhaustive("mongodb_missing_mixed_exhaustive_states", false).await;
}

/// Date and quantity `:not`, duplicate rows and many rows per resource for
/// one parameter: `mx-a` none / alpha / alpha twice, `mx-d` and `mx-q` none /
/// alpha / beta / both, crossed with every stamp, the pending marker and
/// `is_deleted` (960 resources), plus 30 live resources with 40 rows of one
/// parameter.
fn typed_docs() -> Vec<Doc> {
    let a_states = [vec![], vec![ALPHA], vec![ALPHA, ALPHA]];
    let typed = [vec![], vec![ALPHA], vec![BETA], vec![ALPHA, BETA]];
    let mut docs = Vec::new();
    for stamp in STAMPS {
        for pending in [false, true] {
            for deleted in [false, true] {
                for a in &a_states {
                    for d in &typed {
                        for q in &typed {
                            let n = docs.len() as u32;
                            let mut values = blank_values();
                            values[A] = labels(a);
                            values[D] = labels(d);
                            values[Q] = labels(q);
                            docs.push(Doc {
                                id: format!("t{n:04}"),
                                values,
                                stamp,
                                pending,
                                deleted,
                                day: 1 + n % 6,
                            });
                        }
                    }
                }
            }
        }
    }
    let many = |first: Option<&str>| {
        let mut values: Vec<String> = first.into_iter().map(str::to_owned).collect();
        values.extend((0..40 - values.len()).map(|n| format!("m{n:02}")));
        values
    };
    for (s, stamp) in STAMPS.into_iter().enumerate() {
        for pending in [false, true] {
            let n = docs.len() as u32;
            let tag = format!("{s}{}", pending as u8);
            let mut values = blank_values();
            values[D] = many(Some(ALPHA));
            docs.push(Doc {
                id: format!("u-md{tag}"),
                values,
                stamp,
                pending,
                deleted: false,
                day: 1 + n % 6,
            });
            let mut values = blank_values();
            values[A] = many(None);
            docs.push(Doc {
                id: format!("u-ma{tag}"),
                values,
                stamp,
                pending,
                deleted: false,
                day: 1 + (n + 1) % 6,
            });
            let mut values = blank_values();
            values[A] = labels(&[ALPHA]);
            values[Q] = many(Some(BETA));
            docs.push(Doc {
                id: format!("u-mq{tag}"),
                values,
                stamp,
                pending,
                deleted: false,
                day: 1 + (n + 2) % 6,
            });
        }
    }
    docs
}

fn typed_shapes(docs: &[Doc], full: bool) -> Vec<Shape> {
    use Crit::{False, True};
    let kinds = vec![
        not(D, &[ALPHA]),
        not(D, &[BETA]),
        not(D, &[ALPHA, BETA]),
        not(D, &["m07"]),
        not(Q, &[ALPHA]),
        not(Q, &[BETA]),
        not(Q, &[ALPHA, BETA]),
        not(Q, &["m33"]),
        not(A, &[ALPHA]),
        not(A, &["m05"]),
        False(D),
        False(Q),
        False(A),
    ];
    let mut shapes = Vec::new();
    for missing in [vec![True(A)], vec![True(A), True(A)]] {
        for kind in &kinds {
            let mut after = missing.clone();
            after.push(kind.clone());
            shapes.push(shape(after, Extra::None));
            let mut before = vec![kind.clone()];
            before.extend(missing.clone());
            shapes.push(shape(before, Extra::None));
        }
    }
    let crossed = if full {
        kinds.clone()
    } else {
        vec![not(D, &[ALPHA]), not(Q, &[BETA])]
    };
    for kind in &crossed {
        for extra in extras(docs) {
            shapes.push(shape(vec![True(A), kind.clone()], extra));
        }
    }
    shapes.push(shape(
        vec![True(A), not(D, &[ALPHA]), not(Q, &[ALPHA])],
        Extra::None,
    ));
    shapes.push(shape(
        vec![not(D, &[ALPHA, BETA]), True(A), False(Q)],
        Extra::None,
    ));
    shapes.push(shape(
        vec![True(A), False(D), not(Q, &[BETA]), not(D, &["m07"])],
        Extra::UpdatedGe(3),
    ));
    shapes
}

/// The `search_index` rows stored for resource `id` and parameter `p`.
async fn stored_rows(net: &Net, id: &str, p: usize) -> u64 {
    net.db
        .collection::<Document>("search_index")
        .count_documents(doc! {
            "tenant_id": net.tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "resource_id": id,
            "param_name": PARAMS[p].code,
        })
        .await
        .unwrap()
}

async fn run_typed(test: &str, full: bool) {
    let Some(net) = net(
        &format!("missing_mixed_typed{}", if full { "_full" } else { "" }),
        typed_docs(),
        false,
        false,
    )
    .await
    else {
        eprintln!("Skipping {test} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    // The states exist in the database, not only in the model.
    for (id, p) in [("u-md00", D), ("u-ma00", A), ("u-mq00", Q)] {
        assert_eq!(
            stored_rows(&net, id, p).await,
            40,
            "{id}: 40 rows of one parameter"
        );
    }
    let with = |values: &[&str]| {
        net.docs
            .iter()
            .find(|doc| doc.values[A] == labels(values))
            .unwrap()
            .id
            .clone()
    };
    assert_eq!(
        stored_rows(&net, &with(&[ALPHA, ALPHA]), A).await,
        2,
        "duplicate value: two rows"
    );
    assert_eq!(stored_rows(&net, &with(&[ALPHA]), A).await, 1);
    let mut stats = Stats::default();
    for (n, shape) in typed_shapes(&net.docs, full).iter().enumerate() {
        check(
            &net,
            shape,
            Route::Metadata,
            &Walk {
                sort: SORTS[n % SORTS.len()],
                count: 200,
                back: n % 5 == 0,
                door: (!full).then_some(n),
            },
            &mut stats,
        )
        .await;
        if n % 3 == 0 {
            check(
                &net,
                shape,
                Route::ValueRows,
                &Walk {
                    sort: Sort::Default,
                    count: 200,
                    back: false,
                    door: (!full).then_some(n + 1),
                },
                &mut stats,
            )
            .await;
        }
    }
    stats.report(test, net.docs.len());
}

#[tokio::test]
async fn mongodb_missing_mixed_typed_and_many_rows() {
    run_typed("mongodb_missing_mixed_typed_and_many_rows", false).await;
}

/// The full cross of both fixtures: every door for every shape, the
/// 300-value `:not` and the resource-level conditions crossed with every
/// criterion. Run once per production commit.
#[tokio::test]
#[ignore = "the full cross (about 13 min); run once per production commit with --ignored"]
async fn mongodb_missing_mixed_full_cross() {
    const TEST: &str = "mongodb_missing_mixed_full_cross";
    run_exhaustive(TEST, true).await;
    run_typed(TEST, true).await;
}

// ---------------------------------------------------------------------------
// Volume, concurrency and deadline
// ---------------------------------------------------------------------------

const VOLUME_DOCS: u32 = 14_000;
const COUNT_BATCH: usize = 4096;
const SEARCH_DEADLINE_MS: u64 = 2_000;

/// `mx-a` absent everywhere, `mx-b` alpha on every second resource, `mx-t`
/// alpha on every sixteenth, `mx-c` gamma on every resource of days 1 and 2
/// (the head of the stamped stream, ordered by `last_updated`, `id`); 2%
/// unstamped (half of them pending), some deleted.
fn volume_docs() -> Vec<Doc> {
    (0..VOLUME_DOCS)
        .map(|n| {
            let mut values = blank_values();
            let day = 1 + n % 6;
            if n % 2 == 0 {
                values[B] = labels(&[ALPHA]);
            }
            if n % 16 == 0 {
                values[T] = labels(&[ALPHA]);
            }
            if day <= 2 {
                values[C] = labels(&[GAMMA]);
            }
            let unstamped = matches!(n % 100, 7 | 57);
            Doc {
                id: format!("v{n:05}"),
                values,
                stamp: if unstamped {
                    Stamp::Absent
                } else {
                    Stamp::Real
                },
                pending: n % 100 == 57,
                deleted: n % 250 == 3,
                day,
            }
        })
        .collect()
}

fn volume_shapes(docs: &[Doc]) -> Vec<Shape> {
    use Crit::{False, True};
    let stamped_match = docs
        .iter()
        .find(|doc| doc.stamp == Stamp::Real && !doc.deleted && !doc.has_any(T))
        .unwrap()
        .id
        .clone();
    vec![
        shape(vec![True(A), not(T, &[ALPHA])], Extra::None),
        shape(vec![True(A), not(T, &[BETA])], Extra::None),
        shape(vec![True(A), False(B)], Extra::None),
        shape(vec![True(A), not(T, &[ALPHA]), False(B)], Extra::None),
        shape(vec![not(T, &[ALPHA]), True(A)], Extra::UpdatedGe(3)),
        shape(vec![True(A), not(T, &[ALPHA])], Extra::IdNot(stamped_match)),
        // Empties the whole first batch of the stamped stream.
        shape(vec![True(A), not(C, &[GAMMA])], Extra::None),
    ]
}

/// More than `3 * 4096` stamped matches in one total; a `:not` that empties
/// the first batch; three concurrent mixed totals on a pool of two
/// connections and one on a pool of one, bounded so that a pool deadlock
/// fails instead of hanging; and a blocked or failing aggregate under a
/// search deadline, which yields an error or the exact total, never another
/// number.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mongodb_missing_mixed_volume_concurrency_and_deadline() {
    const TEST: &str = "mongodb_missing_mixed_volume_concurrency_and_deadline";
    let Some(net) = net("missing_mixed_volume", volume_docs(), false, false).await else {
        eprintln!("Skipping {TEST} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let shapes = volume_shapes(&net.docs);
    let stamped_matches = net
        .docs
        .iter()
        .filter(|doc| doc.stamp == Stamp::Real && doc.matches(&shapes[0], Route::Metadata))
        .count();
    assert!(
        stamped_matches > 3 * COUNT_BATCH,
        "{stamped_matches} stamped matches: the volume cell must cross three count batches"
    );
    let head = net
        .docs
        .iter()
        .filter(|doc| doc.stamp == Stamp::Real && !doc.deleted && doc.day <= 2)
        .count();
    assert!(
        head > COUNT_BATCH
            && net
                .docs
                .iter()
                .filter(|doc| doc.day <= 2)
                .all(|doc| doc.has_one_of(C, &labels(&[GAMMA]))),
        "{head} stamped resources of days 1-2: `mx-c:not=gamma` must empty the first batch"
    );

    let mut stats = Stats::default();
    for (n, shape) in shapes.iter().enumerate() {
        check(
            &net,
            shape,
            Route::Metadata,
            &Walk {
                sort: if n == 0 { Sort::IdAsc } else { Sort::Default },
                count: 500,
                back: n == 0,
                door: Some(n),
            },
            &mut stats,
        )
        .await;
    }
    check_param_sort(&net, &shapes[3], &mut stats).await;

    // Three concurrent mixed totals on a pool of two connections.
    let mut config = net.metadata.config().clone();
    config.max_connections = Some(2);
    let Some(small_pool) = build_backend(config).await else {
        eprintln!("Skipping {TEST} (MongoDB became unavailable)");
        return;
    };
    assert_eq!(small_pool.config().max_connections, Some(2));
    for round in 0..2 {
        let tasks: Vec<_> = [0usize, 2, 3]
            .into_iter()
            .map(|index| {
                let backend = small_pool.clone();
                let tenant = net.tenant.clone();
                let q = query(&shapes[index], 1);
                let want = expected(&net.docs, &shapes[index], Route::Metadata).len() as u64;
                tokio::spawn(async move {
                    let total = backend.search_count(&tenant, &q).await;
                    (index, total, want)
                })
            })
            .collect();
        let joined =
            tokio::time::timeout(Duration::from_secs(180), futures::future::join_all(tasks))
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "round {round}: three concurrent totals on two connections did not finish"
                    )
                });
        for task in joined {
            let (index, total, want) = task.unwrap();
            let total = total.unwrap_or_else(|e| panic!("concurrent total {index}: {e}"));
            assert_eq!(total, want, "round {round}: concurrent total {index}");
            stats.comparisons += 1;
        }
    }

    // One mixed total over more than three batches on a pool of one.
    let mut config = net.metadata.config().clone();
    config.max_connections = Some(1);
    let Some(single) = build_backend(config).await else {
        eprintln!("Skipping {TEST} (MongoDB became unavailable)");
        return;
    };
    assert_eq!(single.config().max_connections, Some(1));
    let want = expected(&net.docs, &shapes[0], Route::Metadata).len() as u64;
    let total = tokio::time::timeout(
        Duration::from_secs(120),
        single.search_count(&net.tenant, &query(&shapes[0], 1)),
    )
    .await
    .expect("a mixed total on one connection did not finish")
    .unwrap();
    assert_eq!(total, want, "mixed total on one connection");
    stats.comparisons += 1;

    // The k-th matching command is blocked beyond the search deadline, or
    // fails: the total is an error or the exact number, never another number.
    // `k` grows until the failpoint is no longer reached, so every command of
    // the total (each membership batch, the stamped stream and the unstamped
    // count) is hit once. The failpoint is scoped to this backend's own
    // `appName`, and some variants to one collection.
    let app_name = format!("hfs-2233-net2-{}", uuid::Uuid::new_v4().simple());
    let mut config = net.metadata.config().clone();
    config.app_name = Some(app_name.clone());
    config.search_timeout_ms = Some(SEARCH_DEADLINE_MS);
    let Some(deadline) = build_backend(config).await else {
        eprintln!("Skipping {TEST} (MongoDB became unavailable)");
        return;
    };
    let shape = &shapes[3];
    let want = expected(&net.docs, shape, Route::Metadata).len() as u64;
    let search_index = format!("{}.search_index", net.db.name());
    let resources = format!("{}.resources", net.db.name());
    let block = (SEARCH_DEADLINE_MS + 1_000) as i64;
    let mut outcomes = Vec::new();
    for (variant, data) in [
        (
            "block",
            doc! { "failCommands": ["aggregate"], "blockConnection": true,
            "blockTimeMS": block },
        ),
        (
            "error",
            doc! { "failCommands": ["aggregate"], "errorCode": 96_i32 },
        ),
        (
            "block-search_index",
            doc! { "failCommands": ["aggregate"], "namespace": &search_index,
            "blockConnection": true, "blockTimeMS": block },
        ),
        (
            "error-search_index",
            doc! { "failCommands": ["aggregate"], "namespace": &search_index,
            "errorCode": 96_i32 },
        ),
        (
            "error-resources-aggregate",
            doc! { "failCommands": ["aggregate"], "namespace": &resources,
            "errorCode": 96_i32 },
        ),
        (
            "error-resources-stream",
            doc! { "failCommands": ["find", "getMore"], "namespace": &resources,
            "errorCode": 96_i32 },
        ),
    ] {
        let mut errors = 0;
        let mut last_hits = None;
        for k in 0..200_i32 {
            let Some(failpoint) =
                bulk_submit::FailPoint::enable(&app_name, data.clone(), doc! { "skip": k }).await
            else {
                eprintln!("Skipping {TEST} deadline cells (no failCommand)");
                stats.report(TEST, net.docs.len());
                return;
            };
            let result = tokio::time::timeout(
                Duration::from_secs(60),
                deadline.search_count(&net.tenant, &query(shape, 1)),
            )
            .await;
            let hits = failpoint.off_and_count().await;
            let result = result.unwrap_or_else(|_| panic!("{variant} k={k}: total never ended"));
            let outcome = match &result {
                Ok(total) => {
                    assert_eq!(
                        *total, want,
                        "{variant} k={k}: a number other than the total"
                    );
                    "exact"
                }
                Err(_) => {
                    errors += 1;
                    "error"
                }
            };
            outcomes.push(format!("{variant}/k={k}/hits={hits}/{outcome}"));
            if k == 0 {
                assert!(hits > 0, "{variant} k=0: the failpoint never fired");
            }
            stats.comparisons += 1;
            last_hits = Some(hits);
            if hits == 0 {
                // Past the total's last matching command: it ran untouched.
                assert_eq!(outcome, "exact", "{variant} k={k}: nothing failed");
                break;
            }
        }
        assert_eq!(
            last_hits,
            Some(0),
            "{variant}: the sweep never got past the total's commands"
        );
        assert!(
            errors > 0,
            "{variant}: no command failure reached the total"
        );
    }
    eprintln!(
        "[2233-net2] {TEST}: stamped_matches={stamped_matches} first_batch_head={head} \
         failpoints=[{}]",
        outcomes.join(" ")
    );
    stats.report(TEST, net.docs.len());
}

// ---------------------------------------------------------------------------
// Search-parameter sort over the id cap
// ---------------------------------------------------------------------------

/// The id-set cap of a filtered search (`MAX_RESULT_ID_SET`).
const ID_CAP: u32 = 300_000;
const CLONES: u32 = ID_CAP + 10;

/// `_sort=mx-b&_total=accurate` over more matches than the id-set cap: the
/// page walks the sort index and the total is counted by the unsorted query
/// (`count_standard` with the sort cleared). That count keeps the single
/// aggregation for a mixed and for a pure `:missing=true` query alike, while
/// `search_count` of the same query takes its own route.
///
/// The `ID_CAP + 10` resources over the cap are raw copies of one resource
/// written through the real path (with its `mx-b` row) under new ids; a
/// hundred resources written through the real path carry the other states.
/// The copies leave out the rows of the other parameters, which no criterion
/// of this query reads.
#[tokio::test]
#[ignore = "heavy: writes 300,010 resources to cross the id-set cap; run with --ignored"]
async fn mongodb_missing_mixed_param_sort_over_the_id_cap() {
    const TEST: &str = "mongodb_missing_mixed_param_sort_over_the_id_cap";
    let mut base: Vec<Doc> = (0..100u32)
        .map(|n| {
            let mut values = blank_values();
            if n % 3 != 0 {
                values[B] = labels(&[ALPHA]);
            }
            if n % 5 == 0 {
                values[T] = labels(&[ALPHA]);
            }
            if n % 7 == 0 {
                values[A] = labels(&[BETA]);
            }
            Doc {
                id: format!("h{n:03}"),
                values,
                stamp: if n % 10 == 1 {
                    Stamp::Absent
                } else {
                    Stamp::Real
                },
                pending: n % 20 == 1,
                deleted: n % 25 == 2,
                day: 1 + n % 6,
            }
        })
        .collect();
    let mut template = blank_values();
    template[B] = labels(&[ALPHA]);
    base.push(Doc {
        id: "template".into(),
        values: template.clone(),
        stamp: Stamp::Real,
        pending: false,
        deleted: false,
        day: 3,
    });
    let Some(mut net) = net("missing_mixed_over_cap", base, false, false).await else {
        eprintln!("Skipping {TEST} (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };

    let tenant_id = net.tenant.tenant_id().as_str().to_owned();
    let resources = net.db.collection::<Document>("resources");
    let rows = net.db.collection::<Document>("search_index");
    let mut resource = resources
        .find_one(
            doc! { "tenant_id": &tenant_id, "resource_type": "Observation", "id": "template" },
        )
        .await
        .unwrap()
        .unwrap();
    resource.remove("_id");
    let mut row = rows
        .find_one(doc! {
            "tenant_id": &tenant_id, "resource_type": "Observation",
            "resource_id": "template", "param_name": PARAMS[B].code,
        })
        .await
        .unwrap()
        .expect("template mx-b row");
    row.remove("_id");
    let started = std::time::Instant::now();
    for chunk in (0..CLONES).collect::<Vec<_>>().chunks(10_000) {
        let mut documents = Vec::with_capacity(chunk.len());
        let mut index_rows = Vec::with_capacity(chunk.len());
        for n in chunk {
            let id = format!("k{n:06}");
            let mut copy = resource.clone();
            copy.insert("id", &id);
            copy.get_document_mut("data").unwrap().insert("id", &id);
            documents.push(copy);
            let mut copy = row.clone();
            copy.insert("resource_id", &id);
            index_rows.push(copy);
            net.docs.push(Doc {
                id,
                values: template.clone(),
                stamp: Stamp::Real,
                pending: false,
                deleted: false,
                day: 3,
            });
        }
        resources.insert_many(documents).await.unwrap();
        rows.insert_many(index_rows).await.unwrap();
    }
    eprintln!(
        "[2233-net2] {TEST}: {CLONES} copies written in {:?}",
        started.elapsed()
    );

    let mut stats = Stats::default();
    for crits in [
        vec![Crit::True(A), not(T, &[ALPHA])],
        vec![not(T, &[ALPHA]), Crit::True(A), Crit::False(B)],
        vec![Crit::True(A)],
    ] {
        let shape = shape(crits, Extra::None);
        let want = expected(&net.docs, &shape, Route::Metadata);
        assert!(
            want.len() > ID_CAP as usize,
            "the cell must cross the id cap"
        );
        let log = PlanLog::new();
        let mut q = query(&shape, 20).with_sort(SortDirective::parse(PARAMS[B].code));
        q.total = Some(TotalMode::Accurate);
        let result = net
            .metadata
            .search(&net.tenant, &q)
            .with_subscriber(log.clone())
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", label(&shape)));
        // Over the cap the total is `count_standard` of the unsorted query:
        // its plan events are there, and it stays on the single aggregation,
        // the pure `:missing=true` shape included.
        assert_route(
            &label(&shape),
            Expect::Kept,
            &log.filters.lock().unwrap(),
            &log.totals.lock().unwrap(),
        );
        assert_eq!(
            result.total,
            Some(want.len() as u64),
            "{}: total over the id cap",
            label(&shape)
        );
        let by_id: HashMap<&str, &Doc> =
            net.docs.iter().map(|doc| (doc.id.as_str(), doc)).collect();
        let mut keyed: Vec<&String> = want
            .iter()
            .filter(|id| by_id[id.as_str()].has_any(B))
            .collect();
        keyed.sort();
        let first: Vec<String> = keyed.into_iter().take(20).cloned().collect();
        let ids: Vec<String> = result
            .resources
            .items
            .iter()
            .map(|row| row.id().to_owned())
            .collect();
        assert_eq!(ids, first, "{}: first page over the id cap", label(&shape));
        let (count, filters, strategies) =
            routed_count(&net.metadata, &net.tenant, &query(&shape, 1)).await;
        assert_eq!(count, want.len() as u64, "{}: search_count", label(&shape));
        assert_route(
            &label(&shape),
            intended_route(&shape),
            &filters,
            &strategies,
        );
        stats.shapes += 1;
        stats.comparisons += 3;
        stats.largest = stats.largest.max(want.len());
        stats.nonzero += 1;
        stats.routed += 2;
    }
    stats.report(TEST, net.docs.len());
}
