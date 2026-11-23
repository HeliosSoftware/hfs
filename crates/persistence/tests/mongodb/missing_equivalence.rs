//! #2233 characterization net: `:missing=true` searches answered from
//! `missing_params` metadata, pinned before the exact total is rewritten.
//!
//! Every document is written through the real write path (so rows and stamps
//! are genuine) and is then moved into one state by a raw update:
//!
//! * `missing_params`: as written / absent / `[]` / `null` / lists every
//!   parameter (contradicting the rows, #2002);
//! * `search_rows_pending` present or not (#1975, #2257);
//! * value rows present or not, per parameter;
//! * `is_deleted`, another tenant, another resource type.
//!
//! Each query is read through every route (pages in every sort and both
//! cursor directions, `_total=accurate|estimate`, `_summary=count`,
//! `search_count`, `_contained=both`) and compared with
//!
//! 1. an independent model of the documented semantics ([`Doc::matches`]),
//! 2. the number of ids obtained by walking the pages,
//! 3. a test-only BSON copy of the exact-total pipeline as it is today
//!    ([`reference_total`]), so a rewritten pipeline is compared with the old one,
//! 4. the metadata-off route (value rows only) on the same database.
//!
//! The net is green on `main` and must stay green after the exact total is
//! split in two counts. The single assertion that records today's behavior of
//! a state HFS cannot write (a boolean `resource_id`) is marked `STAGE1-FLIP`.

use super::*;
use futures::TryStreamExt;
use helios_persistence::types::{ContainedMode, SummaryMode};
use mongodb::bson::DateTime as BsonDateTime;
use std::collections::BTreeSet;

const NAMES: [&str; 3] = ["eq-a", "eq-b", "eq-c"];
const URLS: [&str; 3] = ["urn:hfs:eq:a", "urn:hfs:eq:b", "urn:hfs:eq:c"];
const MISSING_INDEX: &str = "idx_resources_missing_params";
const PAGE: u32 = 25;

/// Day (of January 2026) of every `last_updated`; `day` repeats on purpose so
/// that `_lastUpdated` ties are the rule, not the exception (#2654).
fn instant(day: u32) -> String {
    format!("2026-01-{day:02}T12:00:00Z")
}

fn bson_instant(day: u32) -> BsonDateTime {
    BsonDateTime::parse_rfc3339_str(instant(day)).unwrap()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stamp {
    /// As the real write path stored it.
    Real,
    Absent,
    Empty,
    Null,
    /// Lists every parameter whatever the rows say.
    All,
}

#[derive(Clone, Debug)]
struct Doc {
    id: String,
    rows: [bool; 3],
    stamp: Stamp,
    pending: bool,
    deleted: bool,
    day: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Crit {
    /// `name:missing=true`
    True(usize),
    /// `name:missing=false`
    False(usize),
    /// `name:not=present`
    Not(usize),
}

#[derive(Clone, Debug)]
enum Extra {
    None,
    UpdatedGe(u32),
    UpdatedList(Vec<u32>),
    UpdatedRange(u32, u32),
    IdNot(String),
}

#[derive(Clone, Debug)]
struct Shape {
    crits: Vec<Crit>,
    extra: Extra,
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

#[derive(Clone, Copy, PartialEq)]
enum Route {
    /// `missing_params` answers the `:missing=true` parameters.
    Metadata,
    /// `HFS_MONGODB_MISSING_METADATA=false`: value rows answer them.
    ValueRows,
}

impl Doc {
    fn extra_ok(&self, extra: &Extra) -> bool {
        match extra {
            Extra::None => true,
            Extra::UpdatedGe(day) => self.day >= *day,
            Extra::UpdatedList(days) => days.contains(&self.day),
            Extra::UpdatedRange(low, high) => self.day >= *low && self.day <= *high,
            Extra::IdNot(id) => &self.id != id,
        }
    }

    /// The documented semantics of one document under one query.
    fn matches(&self, shape: &Shape, route: Route) -> bool {
        if self.deleted || !self.extra_ok(&shape.extra) {
            return false;
        }
        shape.crits.iter().all(|crit| match *crit {
            Crit::True(i) => match route {
                Route::ValueRows => !self.rows[i],
                Route::Metadata => match self.stamp {
                    Stamp::Real => !self.rows[i],
                    Stamp::All => true,
                    Stamp::Empty | Stamp::Null => false,
                    Stamp::Absent => !self.pending && !self.rows[i],
                },
            },
            Crit::False(i) => self.rows[i],
            Crit::Not(i) => !self.rows[i],
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

fn criterion(crit: Crit) -> SearchParameter {
    let (index, modifier, value) = match crit {
        Crit::True(i) => (i, Some(SearchModifier::Missing), "true"),
        Crit::False(i) => (i, Some(SearchModifier::Missing), "false"),
        Crit::Not(i) => (i, Some(SearchModifier::Not), "present"),
    };
    SearchParameter {
        name: NAMES[index].to_owned(),
        param_type: SearchParamType::String,
        modifier,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
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

fn query(shape: &Shape, sort: Sort, count: u32) -> SearchQuery {
    let mut query = SearchQuery::new("Observation").with_count(count);
    for crit in &shape.crits {
        query = query.with_parameter(criterion(*crit));
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
    }
    match sort {
        Sort::Default => query,
        Sort::IdAsc => query.with_sort(SortDirective::parse("_id")),
        Sort::IdDesc => query.with_sort(SortDirective::parse("-_id")),
        Sort::UpdatedAsc => query.with_sort(SortDirective::parse("_lastUpdated")),
        Sort::UpdatedDesc => query.with_sort(SortDirective::parse("-_lastUpdated")),
    }
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

async fn register_parameters(backend: &MongoBackend, tenant: &TenantContext) {
    for (name, url) in NAMES.iter().zip(URLS) {
        backend
            .seed_create(
                tenant,
                "SearchParameter",
                json!({
                    "resourceType": "SearchParameter", "id": name,
                    "url": format!("urn:hfs:test:{name}"), "name": name,
                    "status": "active", "code": name, "base": ["Observation"],
                    "type": "string",
                    "expression": format!(
                        "Observation.extension.where(url = '{url}').value"
                    ),
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
    }
}

fn observation(doc: &Doc) -> serde_json::Value {
    let extensions: Vec<_> = doc
        .rows
        .iter()
        .enumerate()
        .filter(|(_, present)| **present)
        .map(|(i, _)| json!({"url": URLS[i], "valueString": "present"}))
        .collect();
    json!({
        "resourceType": "Observation", "id": doc.id, "status": "final",
        "code": {"coding": [{"code": "fixture"}]}, "extension": extensions,
    })
}

async fn raw_db(backend: &MongoBackend) -> mongodb::Database {
    raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name)
}

fn scope(tenant: &TenantContext, kind: &str, id: &str) -> Document {
    doc! { "tenant_id": tenant.tenant_id().as_str(), "resource_type": kind, "id": id }
}

/// Writes `doc` through the real path, then moves it into its state.
async fn write(backend: &MongoBackend, db: &mongodb::Database, tenant: &TenantContext, doc: &Doc) {
    backend
        .seed_create(
            tenant,
            "Observation",
            observation(doc),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let mut set = doc! {
        "last_updated": bson_instant(doc.day),
        "is_deleted": doc.deleted,
    };
    let mut unset = Document::new();
    match doc.stamp {
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
            set.insert("missing_params", NAMES.to_vec());
        }
    }
    if doc.pending {
        set.insert("search_rows_pending", true);
    }
    let mut update = doc! { "$set": set };
    if !unset.is_empty() {
        update.insert("$unset", unset);
    }
    let result = db
        .collection::<Document>("resources")
        .update_one(scope(tenant, "Observation", &doc.id), update)
        .await
        .unwrap();
    assert_eq!(result.matched_count, 1, "fixture {}", doc.id);
}

/// The exhaustive state space: 8 row sets x 5 stamps x pending x deleted.
fn exhaustive_docs() -> Vec<Doc> {
    let stamps = [
        Stamp::Real,
        Stamp::Absent,
        Stamp::Empty,
        Stamp::Null,
        Stamp::All,
    ];
    let mut docs = Vec::new();
    for mask in 0..8u32 {
        for (s, stamp) in stamps.iter().enumerate() {
            for pending in [false, true] {
                for deleted in [false, true] {
                    let n = docs.len() as u32;
                    docs.push(Doc {
                        id: format!("x{n:03}-m{mask}s{s}p{}d{}", pending as u8, deleted as u8),
                        rows: [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0],
                        stamp: *stamp,
                        pending,
                        deleted,
                        day: 1 + n % 6,
                    });
                }
            }
        }
    }
    docs
}

/// Deterministic generator for the seeded differential.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

fn random_docs(seed: u64, count: usize) -> Vec<Doc> {
    let mut rng = Lcg(seed);
    (0..count)
        .map(|n| {
            let stamp = match rng.next(10) {
                0..=4 => Stamp::Real,
                5 | 6 => Stamp::Absent,
                7 => Stamp::Empty,
                8 => Stamp::Null,
                _ => Stamp::All,
            };
            Doc {
                id: format!("r{n:04}"),
                rows: [rng.next(2) == 1, rng.next(3) == 0, rng.next(5) == 0],
                stamp,
                pending: rng.next(6) == 0,
                deleted: rng.next(8) == 0,
                day: 1 + rng.next(6) as u32,
            }
        })
        .collect()
}

/// Rows other tenants and other types own: they must never be counted.
async fn write_noise(backend: &MongoBackend, db: &mongodb::Database, tenant: &TenantContext) {
    let foreign = create_tenant(&format!("{}-foreign", tenant.tenant_id().as_str()));
    register_parameters(backend, &foreign).await;
    for mask in 0..8u32 {
        let doc = Doc {
            id: format!("f{mask}"),
            rows: [mask & 1 != 0, mask & 2 != 0, mask & 4 != 0],
            stamp: Stamp::Real,
            pending: false,
            deleted: false,
            day: 3,
        };
        write(backend, db, &foreign, &doc).await;
    }
    for id in ["c0", "c1"] {
        backend
            .seed_create(
                tenant,
                "Condition",
                json!({"resourceType": "Condition", "id": id,
                    "subject": {"reference": "Patient/p"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        db.collection::<Document>("resources")
            .update_one(
                scope(tenant, "Condition", id),
                doc! { "$set": { "missing_params": NAMES.to_vec() } },
            )
            .await
            .unwrap();
    }
}

struct Fixture {
    metadata: MongoBackend,
    /// Same database, `missing_metadata = false`.
    rows: MongoBackend,
    db: mongodb::Database,
    tenant: TenantContext,
    docs: Vec<Doc>,
}

async fn fixture(name: &str, docs: Vec<Doc>) -> Option<Fixture> {
    let metadata = create_backend(name).await?;
    let tenant = create_tenant(name);
    register_parameters(&metadata, &tenant).await;
    let db = raw_db(&metadata).await;
    for doc in &docs {
        write(&metadata, &db, &tenant, doc).await;
    }
    write_noise(&metadata, &db, &tenant).await;
    let mut config = metadata.config().clone();
    config.missing_metadata = false;
    let rows = build_backend(config).await?;
    Some(Fixture {
        metadata,
        rows,
        db,
        tenant,
        docs,
    })
}

// ---------------------------------------------------------------------------
// The exact total as it is today, kept as test-only code
// ---------------------------------------------------------------------------

/// A copy of today's exact-total aggregation for a pure `:missing=true`
/// query: one `$match` with the `$or` of every name, one fallback `$lookup`
/// per criterion, then `$count`. It must keep returning what the product
/// returns, whatever the product does internally.
async fn reference_total(
    db: &mongodb::Database,
    tenant: &TenantContext,
    shape: &Shape,
) -> Option<u64> {
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
        Extra::IdNot(id) => conditions.push(doc! { "id": { "$ne": id } }),
        // Resource-level shapes without a plain BSON mirror are covered by
        // the model and the page walk only.
        _ => return None,
    }
    let mut stages = Vec::new();
    for crit in &shape.crits {
        let Crit::True(i) = crit else { return None };
        conditions.push(doc! { "$or": [
            { "missing_params": NAMES[*i] },
            { "missing_params": { "$exists": false },
              "search_rows_pending": { "$exists": false } },
        ] });
        stages.push(doc! { "$lookup": {
            "from": "search_index",
            "let": { "candidate_id": { "$cond": [
                { "$eq": [{ "$type": "$missing_params" }, "missing"] },
                "$id",
                false,
            ] } },
            "pipeline": [
                { "$match": { "$and": [
                    { "tenant_id": tenant.tenant_id().as_str(),
                      "resource_type": "Observation",
                      "param_name": NAMES[*i],
                      "value_string": { "$ne": Bson::Null } },
                    { "$expr": { "$eq": ["$resource_id", "$$candidate_id"] } },
                ] } },
                { "$limit": 1 },
                { "$project": { "_id": 1 } },
            ],
            "as": "_hfs_missing_present",
        } });
        stages.push(doc! { "$match": { "_hfs_missing_present.0": { "$exists": false } } });
        stages.push(doc! { "$unset": "_hfs_missing_present" });
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
// Reads
// ---------------------------------------------------------------------------

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

/// Forward by `next` cursors, then back by `previous` cursors.
async fn walk(
    backend: &MongoBackend,
    tenant: &TenantContext,
    shape: &Shape,
    sort: Sort,
) -> (Vec<String>, Vec<String>) {
    let mut forward = Vec::new();
    let mut q = query(shape, sort, PAGE);
    let mut pages = 0;
    let mut last_len;
    let last = loop {
        let (ids, info) = page(backend, tenant, &q).await;
        last_len = ids.len();
        forward.extend(ids);
        pages += 1;
        assert!(pages < 100, "{shape:?} {sort:?}: paging never ended");
        match (info.has_next, info.next_cursor.clone()) {
            (true, Some(cursor)) => q = query(shape, sort, PAGE).with_cursor(cursor),
            (true, None) => panic!("{shape:?} {sort:?}: has_next without a cursor"),
            _ => break info,
        }
    };
    let mut backward: Vec<String> = forward[forward.len() - last_len..].to_vec();
    let mut previous = last.previous_cursor.clone();
    let mut back_pages = 0;
    while let Some(cursor) = previous {
        let (ids, info) = page(
            backend,
            tenant,
            &query(shape, sort, PAGE).with_cursor(cursor),
        )
        .await;
        back_pages += 1;
        assert!(
            back_pages < 100,
            "{shape:?} {sort:?}: paging back never ended"
        );
        let mut joined = ids;
        joined.extend(backward);
        backward = joined;
        previous = match (info.has_previous, info.previous_cursor) {
            (true, Some(cursor)) => Some(cursor),
            _ => None,
        };
    }
    (forward, backward)
}

/// Every way a caller can ask for the exact total.
async fn totals(
    backend: &MongoBackend,
    tenant: &TenantContext,
    shape: &Shape,
) -> Vec<(&'static str, u64)> {
    let mut out = Vec::new();
    for (label, mode) in [
        ("_total=accurate", TotalMode::Accurate),
        ("_total=estimate", TotalMode::Estimate),
    ] {
        let mut q = query(shape, Sort::Default, 1);
        q.total = Some(mode);
        let result = backend.search(tenant, &q).await.unwrap();
        out.push((label, result.total.expect("total requested")));
    }
    let mut q = query(shape, Sort::Default, 1);
    q.total = Some(TotalMode::Accurate);
    q.summary = Some(SummaryMode::Count);
    let result = backend.search(tenant, &q).await.unwrap();
    out.push(("_summary=count", result.total.expect("count requested")));

    let mut q = query(shape, Sort::Default, 1);
    q.total = Some(TotalMode::Accurate);
    out.push((
        "search_count",
        backend.search_count(tenant, &q).await.unwrap(),
    ));

    let mut q = query(shape, Sort::Default, PAGE);
    q.total = Some(TotalMode::Accurate);
    q.contained = ContainedMode::Both;
    // MongoDB rejects `:missing` / `:not` next to `_contained=both`, so those
    // shapes never reach the exact total through this door; only that exact
    // rejection is tolerated, any other error is a failure.
    match backend.search(tenant, &q).await {
        Ok(result) => out.push(("_contained=both", result.total.expect("total requested"))),
        Err(err) => assert!(
            err.to_string()
                .contains("cannot be combined with _contained"),
            "_contained=both failed unexpectedly: {err}"
        ),
    }
    out
}

fn order_ok(docs: &[Doc], ids: &[String], sort: Sort, label: &str) {
    let day = |id: &String| docs.iter().find(|d| &d.id == id).unwrap().day;
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

/// All the equalities for one query on one route.
async fn check(
    backend: &MongoBackend,
    db: &mongodb::Database,
    tenant: &TenantContext,
    docs: &[Doc],
    shape: &Shape,
    route: Route,
    sorts: &[Sort],
) {
    let want = expected(docs, shape, route);
    let label = format!("{shape:?}");

    for (read, total) in totals(backend, tenant, shape).await {
        assert_eq!(total, want.len() as u64, "{label}: {read}");
    }

    if route == Route::Metadata {
        if let Some(old) = reference_total(db, tenant, shape).await {
            assert_eq!(old, want.len() as u64, "{label}: reference pipeline");
        }
    }

    for sort in sorts {
        let (forward, backward) = walk(backend, tenant, shape, *sort).await;
        let unique: BTreeSet<_> = forward.iter().collect();
        assert_eq!(unique.len(), forward.len(), "{label} {sort:?}: repeated id");
        let mut seen = forward.clone();
        seen.sort();
        assert_eq!(seen, want, "{label} {sort:?}: page walk");
        assert_eq!(backward, forward, "{label} {sort:?}: walk back");
        order_ok(docs, &forward, *sort, &format!("{label} {sort:?}"));
    }
}

fn shape(crits: Vec<Crit>, extra: Extra) -> Shape {
    Shape { crits, extra }
}

fn fixed_shapes() -> Vec<Shape> {
    use Crit::{False, Not, True};
    let mut shapes: Vec<Shape> = [
        vec![True(0)],
        vec![True(1)],
        vec![True(2)],
        vec![True(0), True(1)],
        vec![True(1), True(0)],
        vec![True(0), True(2)],
        vec![True(0), True(0)],
        vec![True(0), True(1), True(2)],
        vec![True(2), True(1), True(0)],
        vec![True(1), True(1), True(2)],
        // Mixed: never part of a pure total.
        vec![True(0), False(1)],
        vec![False(1), True(0)],
        vec![True(0), False(0)],
        vec![True(0), Not(1)],
        vec![True(0), True(1), Not(2)],
        vec![True(0), True(1), False(2)],
        vec![False(0)],
        vec![Not(0)],
        vec![False(0), Not(1)],
    ]
    .into_iter()
    .map(|crits| shape(crits, Extra::None))
    .collect();
    for crits in [
        vec![True(0)],
        vec![True(0), True(1)],
        vec![True(1), True(0), True(2)],
    ] {
        for extra in [
            Extra::UpdatedGe(4),
            Extra::UpdatedList(vec![2, 5]),
            Extra::UpdatedRange(3, 5),
            Extra::IdNot("x000-m0s0p0d0".into()),
            Extra::IdNot("no-such-id".into()),
        ] {
            shapes.push(shape(crits.clone(), extra));
        }
    }
    shapes.push(shape(vec![True(0), Not(1)], Extra::UpdatedGe(3)));
    shapes.push(shape(vec![True(0), False(1)], Extra::IdNot("x001".into())));
    shapes
}

fn random_shape(rng: &mut Lcg, docs: &[Doc]) -> Shape {
    let crits = (0..1 + rng.next(3))
        .map(|_| {
            let i = rng.next(3) as usize;
            match rng.next(8) {
                0..=4 => Crit::True(i),
                5 => Crit::False(i),
                _ => Crit::Not(i),
            }
        })
        .collect();
    let extra = match rng.next(6) {
        0 => Extra::UpdatedGe(1 + rng.next(6) as u32),
        1 => Extra::UpdatedList(vec![1 + rng.next(6) as u32, 1 + rng.next(6) as u32]),
        2 => Extra::UpdatedRange(1 + rng.next(3) as u32, 3 + rng.next(4) as u32),
        3 => Extra::IdNot(docs[rng.next(docs.len() as u64) as usize].id.clone()),
        _ => Extra::None,
    };
    shape(crits, extra)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mongodb_missing_equivalence_exhaustive_states() {
    let Some(fx) = fixture("missing_eq_exhaustive", exhaustive_docs()).await else {
        return;
    };

    // The fixture must tell the states apart, or a wrong rewrite would pass:
    // dropping the pending check, the unstamped count or the stamped count
    // each changes the expected total of a single-parameter query.
    let single = shape(vec![Crit::True(0)], Extra::None);
    let want = expected(&fx.docs, &single, Route::Metadata).len();
    let without_pending = fx
        .docs
        .iter()
        .filter(|d| {
            let mut d = (*d).clone();
            d.pending = false;
            d.matches(&single, Route::Metadata)
        })
        .count();
    let stamped_only = fx
        .docs
        .iter()
        .filter(|d| d.stamp != Stamp::Absent && d.matches(&single, Route::Metadata))
        .count();
    let unstamped_only = want - stamped_only;
    assert!(
        without_pending > want,
        "fixture must contain pending states"
    );
    assert!(stamped_only > 0 && unstamped_only > 0);
    assert_ne!(
        want,
        expected(&fx.docs, &single, Route::ValueRows).len(),
        "metadata and value routes must differ on this fixture"
    );

    for shape in fixed_shapes() {
        check(
            &fx.metadata,
            &fx.db,
            &fx.tenant,
            &fx.docs,
            &shape,
            Route::Metadata,
            &SORTS,
        )
        .await;
        // The metadata-off route answers from value rows alone.
        check(
            &fx.rows,
            &fx.db,
            &fx.tenant,
            &fx.docs,
            &shape,
            Route::ValueRows,
            &[Sort::Default, Sort::IdAsc],
        )
        .await;
    }
}

#[tokio::test]
async fn mongodb_missing_equivalence_seeded_differential() {
    let docs = random_docs(0x2233, 300);
    let Some(fx) = fixture("missing_eq_random", docs).await else {
        return;
    };
    let mut rng = Lcg(0x5eed_2233);
    for _ in 0..120 {
        let shape = random_shape(&mut rng, &fx.docs);
        let sort = SORTS[rng.next(SORTS.len() as u64) as usize];
        check(
            &fx.metadata,
            &fx.db,
            &fx.tenant,
            &fx.docs,
            &shape,
            Route::Metadata,
            &[sort],
        )
        .await;
        if rng.next(4) == 0 {
            check(
                &fx.rows,
                &fx.db,
                &fx.tenant,
                &fx.docs,
                &shape,
                Route::ValueRows,
                &[Sort::Default],
            )
            .await;
        }
    }
}

fn bundle_post(id: &str, criteria: &str) -> BundleEntry {
    BundleEntry {
        method: BundleMethod::Post,
        url: "Observation".into(),
        resource: Some(observation(&Doc {
            id: id.into(),
            rows: [true, true, true],
            stamp: Stamp::Real,
            pending: false,
            deleted: false,
            day: 1,
        })),
        if_match: None,
        if_none_match: None,
        if_none_exist: Some(criteria.into()),
        full_url: None,
        criteria: None,
    }
}

/// `ifNoneExist` with `:missing=true` reads the same states the search does.
#[tokio::test]
async fn mongodb_missing_equivalence_conditional_bundles() {
    let doc = |id: &str, stamp, pending| Doc {
        id: id.into(),
        rows: [false, true, true],
        stamp,
        pending,
        deleted: false,
        day: 1,
    };
    let docs = vec![
        doc("stamped", Stamp::Real, false),
        doc("unstamped", Stamp::Absent, false),
        doc("pending", Stamp::Absent, true),
        Doc {
            rows: [true, true, true],
            ..doc("present", Stamp::Real, false)
        },
    ];
    let Some(fx) = fixture("missing_eq_bundle", docs).await else {
        return;
    };
    let resources = fx.db.collection::<Document>("resources");
    let delete = |id: &'static str| {
        let resources = resources.clone();
        let filter = scope(&fx.tenant, "Observation", id);
        async move {
            resources
                .update_one(filter, doc! { "$set": { "is_deleted": true } })
                .await
                .unwrap();
        }
    };

    // Two resources lack eq-a (stamped, unstamped); pending and present do not.
    fx.metadata
        .process_transaction(
            &fx.tenant,
            vec![bundle_post("created-1", "eq-a:missing=true")],
            FhirVersion::default(),
        )
        .await
        .unwrap_err();
    assert!(
        fx.metadata
            .read(&fx.tenant, "Observation", "created-1")
            .await
            .unwrap()
            .is_none(),
        "a multi-match conditional create must change nothing"
    );

    delete("stamped").await;
    let result = fx
        .metadata
        .process_transaction(
            &fx.tenant,
            vec![bundle_post("created-2", "eq-a:missing=true")],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 200);
    assert_eq!(
        result.entries[0].reference().as_deref(),
        Some("Observation/unstamped")
    );

    // The pending resource is left out until its batch writes the metadata.
    delete("unstamped").await;
    let result = fx
        .metadata
        .process_transaction(
            &fx.tenant,
            vec![bundle_post("created-3", "eq-a:missing=true")],
            FhirVersion::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.entries[0].status, 201);
    assert!(
        fx.metadata
            .read(&fx.tenant, "Observation", "created-3")
            .await
            .unwrap()
            .is_some()
    );
}

/// A `search_index` row whose `resource_id` is the boolean `false`: HFS cannot
/// write it (every writer stores a string), so this pins only what the
/// pipeline does today. In that state the lookup of every stamped resource
/// matches the row, so the total leaves all of them out while the pages list
/// them.
#[tokio::test]
async fn mongodb_missing_equivalence_boolean_resource_id_row() {
    let doc = |id: &str, stamp| Doc {
        id: id.into(),
        rows: [false, true, true],
        stamp,
        pending: false,
        deleted: false,
        day: 1,
    };
    let docs = vec![
        doc("stamped-1", Stamp::Real),
        doc("stamped-2", Stamp::Real),
        doc("unstamped", Stamp::Absent),
    ];
    let Some(fx) = fixture("missing_eq_boolean_id", docs).await else {
        return;
    };
    let single = shape(vec![Crit::True(0)], Extra::None);
    let ids = || expected(&fx.docs, &single, Route::Metadata);
    assert_eq!(ids().len(), 3);
    for (read, total) in totals(&fx.metadata, &fx.tenant, &single).await {
        assert_eq!(total, 3, "{read} before the odd row");
    }

    fx.db
        .collection::<Document>("search_index")
        .insert_one(doc! {
            "tenant_id": fx.tenant.tenant_id().as_str(),
            "resource_type": "Observation",
            "resource_id": false,
            "param_name": NAMES[0],
            "value_string": "odd",
        })
        .await
        .unwrap();

    // The pages do not consult rows for stamped resources: all three.
    let (walked, _) = walk(&fx.metadata, &fx.tenant, &single, Sort::Default).await;
    let mut walked_sorted = walked.clone();
    walked_sorted.sort();
    assert_eq!(walked_sorted, ids());

    for (read, total) in totals(&fx.metadata, &fx.tenant, &single).await {
        // STAGE1-FLIP (done): before the split count the stamped resources
        // vanished from the total (only the unstamped one was counted). The
        // split stamped count never consults rows, so the total equals the
        // page walk.
        assert_eq!(
            total,
            walked.len() as u64,
            "{read}: total equals the page walk in the boolean resource_id state"
        );
    }
}
