//! #2375: an instance-history page returns what the pre-#2375 algorithm returned,
//! and reads about one page of documents.
//!
//! Before #2375, `history_instance` read every version of the resource, sorted
//! them in Rust by numeric version (then `last_updated`, both descending),
//! applied the version cursor and truncated to the page size. [`old_page`] is a
//! port of that algorithm over the raw `resource_history` rows; the equivalence
//! tests compare every page of every walk with it, so they hold on the old code
//! and on the new one. The cost tests (`..._does_not_read_rows_outside_the_page`
//! and `..._examines_about_one_page_of_documents`) are the ones that fail on the
//! old code.
//!
//! #2392: ids that are not canonical numbers (`07`, `c`) page in one total order
//! (number, then id, both descending), checked against [`total_order_page`]. For
//! canonical ids that order and the cursor boundary equal [`old_page`]'s, so the
//! equivalence tests are unchanged.
//!
//! Child module of the `mongodb_tests` root: `use super::*;` reaches its private
//! harness (`create_backend`, `create_tenant`, `raw_test_client`).

use super::*;

use chrono::{DateTime, Utc};
use futures::stream::TryStreamExt;
use helios_persistence::core::{HistoryMethod, HistoryPage};
use helios_persistence::types::{CursorValue, PageCursor, Pagination, PaginationMode};
use mongodb::bson::DateTime as BsonDateTime;

fn utc_millis(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).expect("valid timestamp")
}

/// One real history row written by `backend.create`, without its `_id`, to clone
/// the fields a raw seed does not set (`fhir_version`).
async fn template_row(
    backend: &MongoBackend,
    history: &Collection<Document>,
    tenant: &TenantContext,
) -> Document {
    let created = backend
        .create(
            tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "Template"}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create the template resource");
    let mut template = history
        .find_one(doc! { "tenant_id": tenant.tenant_id().as_str(), "id": created.id() })
        .await
        .expect("read the template history row")
        .expect("the template history row exists");
    template.remove("_id");
    template
}

/// A raw `resource_history` row. The payload's family is `v<version_id>`, so a
/// page item can be checked against the row it must come from.
fn history_row(
    template: &Document,
    tenant: &str,
    id: &str,
    version_id: &str,
    at_ms: i64,
    deleted: bool,
) -> Document {
    let mut row = template.clone();
    row.insert("tenant_id", tenant);
    row.insert("resource_type", "Patient");
    row.insert("id", id);
    row.insert("version_id", version_id);
    let at = BsonDateTime::from_millis(at_ms);
    row.insert("last_updated", at);
    row.insert("created_at", at);
    row.insert("is_deleted", deleted);
    row.insert(
        "deleted_at",
        if deleted {
            Bson::DateTime(at)
        } else {
            Bson::Null
        },
    );
    row.insert(
        "data",
        doc! {
            "resourceType": "Patient",
            "id": id,
            "name": [{ "family": format!("v{version_id}") }],
        },
    );
    row
}

async fn insert_rows(history: &Collection<Document>, rows: Vec<Document>) {
    for chunk in rows.chunks(500) {
        history.insert_many(chunk).await.expect("seed history rows");
    }
}

/// What the old algorithm reads of one stored row.
#[derive(Debug, Clone)]
struct RefRow {
    version_id: String,
    last_updated: DateTime<Utc>,
    is_deleted: bool,
}

/// The raw rows of one resource, in `version_id` string order: the order an
/// unsorted `find` returns them in on the unique identity index.
async fn ref_rows(history: &Collection<Document>, tenant: &str, id: &str) -> Vec<RefRow> {
    let docs: Vec<Document> = history
        .find(doc! { "tenant_id": tenant, "resource_type": "Patient", "id": id })
        .sort(doc! { "version_id": 1_i32 })
        .await
        .expect("read the raw history rows")
        .try_collect()
        .await
        .expect("collect the raw history rows");
    docs.iter()
        .map(|row| RefRow {
            version_id: row.get_str("version_id").expect("version_id").to_string(),
            last_updated: utc_millis(
                row.get_datetime("last_updated")
                    .expect("last_updated")
                    .timestamp_millis(),
            ),
            is_deleted: row.get_bool("is_deleted").unwrap_or(false),
        })
        .collect()
}

/// A page as the caller sees it.
#[derive(Debug, PartialEq)]
struct PageView {
    versions: Vec<String>,
    has_next: bool,
    /// The version id inside the next cursor.
    next: Option<String>,
}

/// The filters of a history request.
#[derive(Debug, Clone, Copy)]
struct Scope {
    include_deleted: bool,
    since: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
}

impl Scope {
    fn all() -> Self {
        Self {
            include_deleted: true,
            since: None,
            before: None,
        }
    }
}

/// `parse_version_id`: a version that is not a number sorts as 0.
fn number_of(version_id: &str) -> i64 {
    version_id.parse::<i64>().unwrap_or(0)
}

/// The pre-#2375 algorithm, over raw rows (a port of the old `history_instance`
/// body): filter, stable sort by number then `last_updated` (both descending),
/// version cursor, truncate to `count`.
fn old_page(rows: &[RefRow], scope: Scope, count: u32, cursor: Option<i64>) -> PageView {
    let mut kept: Vec<&RefRow> = rows
        .iter()
        .filter(|row| {
            (scope.include_deleted || !row.is_deleted)
                && scope.since.is_none_or(|since| row.last_updated >= since)
                && scope.before.is_none_or(|before| row.last_updated < before)
        })
        .collect();
    kept.sort_by(|a, b| {
        number_of(&b.version_id)
            .cmp(&number_of(&a.version_id))
            .then_with(|| b.last_updated.cmp(&a.last_updated))
    });
    if let Some(cursor) = cursor {
        kept.retain(|row| number_of(&row.version_id) < cursor);
    }
    let count = count as usize;
    let has_more = kept.len() > count;
    if has_more {
        kept.truncate(count);
    }
    let next = if has_more {
        kept.last().map(|row| row.version_id.clone())
    } else {
        None
    };
    PageView {
        versions: kept.iter().map(|row| row.version_id.clone()).collect(),
        has_next: next.is_some(),
        next,
    }
}

/// What a build before #2392 read from a cursor: the number only. [`old_page`]
/// takes it; for canonical ids it is the same position as the version id.
fn cursor_version(encoded: &str) -> Option<i64> {
    let cursor = PageCursor::decode(encoded).ok()?;
    match cursor.sort_values().first()? {
        CursorValue::String(version) => version.parse::<i64>().ok(),
        CursorValue::Number(version) => Some(*version),
        _ => None,
    }
}

/// The version id inside an encoded cursor (#2392).
fn cursor_version_id(encoded: &str) -> Option<String> {
    let cursor = PageCursor::decode(encoded).ok()?;
    match cursor.sort_values() {
        [CursorValue::String(version)] => Some(version.clone()),
        _ => None,
    }
}

/// The #2392 oracle: filter like [`old_page`], order by `(number, version id)`
/// descending, keep the rows whose key is strictly below the cursor's key when a
/// cursor version id is given, then truncate and build the next cursor as
/// [`old_page`] does.
fn total_order_page(rows: &[RefRow], scope: Scope, count: u32, cursor: Option<&str>) -> PageView {
    let mut kept: Vec<&RefRow> = rows
        .iter()
        .filter(|row| {
            (scope.include_deleted || !row.is_deleted)
                && scope.since.is_none_or(|since| row.last_updated >= since)
                && scope.before.is_none_or(|before| row.last_updated < before)
        })
        .collect();
    kept.sort_by(|a, b| {
        (number_of(&b.version_id), b.version_id.as_str())
            .cmp(&(number_of(&a.version_id), a.version_id.as_str()))
    });
    if let Some(cursor) = cursor {
        let boundary = (number_of(cursor), cursor);
        kept.retain(|row| (number_of(&row.version_id), row.version_id.as_str()) < boundary);
    }
    let count = count as usize;
    let has_more = kept.len() > count;
    if has_more {
        kept.truncate(count);
    }
    let next = if has_more {
        kept.last().map(|row| row.version_id.clone())
    } else {
        None
    };
    PageView {
        versions: kept.iter().map(|row| row.version_id.clone()).collect(),
        has_next: next.is_some(),
        next,
    }
}

struct Walked {
    /// The walk reached a page without a next cursor.
    ended: bool,
    /// The next cursor of every page, in order.
    cursors: Vec<String>,
    /// Every returned version, in order.
    versions: Vec<String>,
}

/// One resource under test: the backend, its identity and its raw rows.
struct Subject<'a> {
    backend: &'a MongoBackend,
    tenant: &'a TenantContext,
    id: String,
    rows: Vec<RefRow>,
}

impl<'a> Subject<'a> {
    async fn load(
        backend: &'a MongoBackend,
        tenant: &'a TenantContext,
        history: &Collection<Document>,
        id: &str,
    ) -> Self {
        Self {
            backend,
            tenant,
            id: id.to_string(),
            rows: ref_rows(history, tenant.tenant_id().as_str(), id).await,
        }
    }

    fn params(&self, scope: Scope, count: u32, cursor: Option<&str>) -> HistoryParams {
        let mut params = HistoryParams::new()
            .count(count)
            .include_deleted(scope.include_deleted);
        params.since = scope.since;
        params.before = scope.before;
        if let Some(cursor) = cursor {
            params.pagination = Pagination::with_cursor(count, cursor.to_string());
        }
        params
    }

    /// Calls the backend and checks the next cursor's format: one string sort
    /// value equal to the last returned version, and this resource's id.
    async fn page(&self, params: &HistoryParams) -> (PageView, HistoryPage) {
        let page = self
            .backend
            .history_instance(self.tenant, "Patient", &self.id, params)
            .await
            .unwrap_or_else(|e| panic!("history_instance({}) failed: {e:?}", self.id));
        let versions: Vec<String> = page
            .items
            .iter()
            .map(|entry| entry.resource.version_id().to_string())
            .collect();
        let next = page.page_info.next_cursor.as_ref().map(|encoded| {
            let cursor = PageCursor::decode(encoded).expect("the next cursor decodes");
            assert_eq!(cursor.sort_values().len(), 1, "one sort value");
            assert_eq!(
                cursor.resource_id(),
                self.id,
                "the cursor names the resource"
            );
            match &cursor.sort_values()[0] {
                CursorValue::String(version) => {
                    assert_eq!(
                        Some(version),
                        versions.last(),
                        "the cursor holds the last returned version"
                    );
                    version.clone()
                }
                other => panic!("the next cursor holds {other:?}, expected a string"),
            }
        });
        assert_eq!(
            page.page_info.has_next,
            next.is_some(),
            "has_next and the next cursor agree"
        );
        (
            PageView {
                versions,
                has_next: page.page_info.has_next,
                next,
            },
            page,
        )
    }

    fn check_items(&self, page: &HistoryPage, check_family: bool, context: &str) {
        for item in &page.items {
            let version = item.resource.version_id();
            let row = self
                .rows
                .iter()
                .find(|row| row.version_id == version)
                .unwrap_or_else(|| panic!("{context}: unknown version {version}"));
            assert_eq!(
                item.resource.is_deleted(),
                row.is_deleted,
                "{context}: v{version} deleted flag"
            );
            assert_eq!(
                item.timestamp, row.last_updated,
                "{context}: v{version} timestamp"
            );
            let method = if row.is_deleted {
                HistoryMethod::Delete
            } else if version == "1" {
                HistoryMethod::Post
            } else {
                HistoryMethod::Put
            };
            assert_eq!(item.method, method, "{context}: v{version} method");
            if check_family {
                assert_eq!(
                    item.resource.content()["name"][0]["family"],
                    json!(format!("v{version}")),
                    "{context}: v{version} content"
                );
            }
        }
    }

    /// Walks the pages from the first through `Pagination::with_cursor`, comparing
    /// each with [`old_page`] fed the cursor the walk holds, until a page has no
    /// next cursor or `max_pages` pages were read.
    async fn walk(&self, scope: Scope, count: u32, max_pages: usize, check_family: bool) -> Walked {
        self.walk_with(scope, count, max_pages, check_family, |cursor| {
            old_page(&self.rows, scope, count, cursor.and_then(cursor_version))
        })
        .await
    }

    /// [`Subject::walk`] with the expected page given by `expected`, which gets
    /// the encoded cursor the walk holds (`None` for page 1).
    async fn walk_with(
        &self,
        scope: Scope,
        count: u32,
        max_pages: usize,
        check_family: bool,
        expected: impl Fn(Option<&str>) -> PageView,
    ) -> Walked {
        let mut cursor: Option<String> = None;
        let mut walked = Walked {
            ended: false,
            cursors: Vec::new(),
            versions: Vec::new(),
        };
        for page_no in 1..=max_pages {
            let params = self.params(scope, count, cursor.as_deref());
            let (view, page) = self.page(&params).await;
            let context = format!(
                "id={} count={count} {scope:?} page={page_no} cursor={cursor:?}",
                self.id
            );
            let want = expected(cursor.as_deref());
            assert_eq!(view, want, "{context}");
            self.check_items(&page, check_family, &context);
            walked.versions.extend(view.versions.iter().cloned());
            if !view.has_next {
                walked.ended = true;
                return walked;
            }
            let next = page
                .page_info
                .next_cursor
                .clone()
                .expect("a page with has_next has a next cursor");
            walked.cursors.push(next.clone());
            cursor = Some(next);
        }
        walked
    }
}

async fn open_history(
    backend: &MongoBackend,
) -> (mongodb::Client, mongodb::Database, Collection<Document>) {
    let client = raw_test_client(&backend.config().connection_string)
        .await
        .expect("raw MongoDB client");
    let db = client.database(&backend.config().database_name);
    let history: Collection<Document> = db.collection("resource_history");
    (client, db, history)
}

/// The page sizes of the first test.
const COUNTS: [u32; 4] = [1, 2, 7, 50];

/// The resources the first test walks at page sizes 1 and 2: the short ones and
/// n-121 (see the test).
const TINY_PAGE_RESOURCES: [&str; 6] = ["n-1", "n-2", "n-9", "n-10", "n-11", "n-121"];

/// `(version, at_ms, deleted)` of one seeded version.
type SeedVersion = (i64, i64, bool);

/// The raw seed `n-<N>`: versions 1..=N at `base + v * 1000`, every seventh
/// deleted.
fn plain_versions(n: i64, base: i64) -> Vec<SeedVersion> {
    (1..=n).map(|v| (v, base + v * 1_000, v % 7 == 0)).collect()
}

/// #2375: every page of every walk equals the old algorithm's, for resources of
/// 1 to 121 versions (crossing 9 -> 10 and 99 -> 100), a resource with gaps and
/// one whose timestamps are not in version order, at several page sizes, with
/// and without deleted versions, and over `_since` and `before` windows. Page
/// sizes 7 and 50 walk every resource under every window; sizes 1 and 2 (a page
/// per one or two versions) walk n-1, n-2, n-9, n-10 and n-11 under every window
/// and n-121 without one, which keeps the 9 -> 10 and 99 -> 100 crossings at
/// size 1. A cursor taken from page N and a page size equal to the remaining
/// versions are checked too.
#[tokio::test]
async fn instance_history_pages_match_the_old_algorithm() {
    let test_started = std::time::Instant::now();
    let Some(backend) = create_backend("inst_hist_equiv").await else {
        eprintln!(
            "Skipping instance_history_pages_match_the_old_algorithm \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, _db, history) = open_history(&backend).await;
    let tenant = create_tenant("tenant-inst-hist-equiv");
    let tenant_id = tenant.tenant_id().as_str().to_string();
    let template = template_row(&backend, &history, &tenant).await;

    let base_ms = Utc::now().timestamp_millis() - 10 * 86_400_000;
    let mut specs: Vec<(String, Vec<SeedVersion>)> = Vec::new();
    for n in [1_i64, 2, 9, 10, 11, 99, 100, 101, 121] {
        specs.push((format!("n-{n}"), plain_versions(n, base_ms)));
    }
    // Versions 30..=34 and 50 were never written (a failed write, a purge).
    specs.push((
        "gaps".to_string(),
        plain_versions(60, base_ms)
            .into_iter()
            .filter(|(v, _, _)| ![30, 31, 32, 33, 34, 50].contains(v))
            .collect(),
    ));
    // Versions 40..=45 carry old timestamps, so a window selects a version set
    // that is not a prefix or a suffix of the numbers.
    specs.push((
        "skew".to_string(),
        plain_versions(121, base_ms)
            .into_iter()
            .map(|(v, at, deleted)| {
                if (40..=45).contains(&v) {
                    (v, base_ms + (v - 35) * 1_000, deleted)
                } else {
                    (v, at, deleted)
                }
            })
            .collect(),
    ));

    let mut rows = Vec::new();
    for (id, versions) in &specs {
        for (v, at, deleted) in versions {
            rows.push(history_row(
                &template,
                &tenant_id,
                id,
                &v.to_string(),
                *at,
                *deleted,
            ));
        }
    }
    insert_rows(&history, rows).await;

    let windows = [
        (None, None),
        (Some(utc_millis(base_ms + 95_000)), None),
        (None, Some(utc_millis(base_ms + 12_000))),
        (
            Some(utc_millis(base_ms + 8_000)),
            Some(utc_millis(base_ms + 101_000)),
        ),
    ];

    let started = std::time::Instant::now();
    let mut subjects = Vec::new();
    for (id, _) in &specs {
        subjects.push(Subject::load(&backend, &tenant, &history, id).await);
    }
    // The resources are walked side by side (the backend's pool holds a few
    // connections), which keeps the module's run time down.
    futures::future::join_all(subjects.iter().map(|subject| async move {
        for count in COUNTS {
            // A walk at page size 1 or 2 reads a page per one or two versions, so
            // it runs under every window only on the short resources (n-10 and
            // n-11 cross 9 -> 10) and, without a window, on n-121 (which crosses
            // 99 -> 100). Sizes 7 and 50 walk every resource under every window.
            let tiny_page = count <= 2;
            if tiny_page && !TINY_PAGE_RESOURCES.contains(&subject.id.as_str()) {
                continue;
            }
            for include_deleted in [true, false] {
                for (since, before) in windows {
                    if tiny_page && subject.id == "n-121" && (since.is_some() || before.is_some()) {
                        continue;
                    }
                    let scope = Scope {
                        include_deleted,
                        since,
                        before,
                    };
                    let walked = subject.walk(scope, count, 400, true).await;
                    assert!(
                        walked.ended,
                        "id={} count={count} {scope:?}: the walk did not end",
                        subject.id
                    );
                }
            }
        }
    }))
    .await;
    eprintln!(
        "instance_history_pages_match_the_old_algorithm: {} resources walked in {:?}",
        subjects.len(),
        started.elapsed()
    );

    // A page size equal to what remains below a cursor, and one on each side.
    let big = subjects
        .iter()
        .find(|subject| subject.id == "n-121")
        .expect("n-121 is seeded");
    for include_deleted in [true, false] {
        let scope = Scope {
            include_deleted,
            ..Scope::all()
        };
        let (_, first) = big.page(&big.params(scope, 7, None)).await;
        let cursor = first
            .page_info
            .next_cursor
            .clone()
            .expect("page 1 has a next cursor");
        let rest = old_page(&big.rows, scope, u32::MAX, cursor_version(&cursor));
        let remaining = rest.versions.len() as u32;
        assert!(remaining > 2, "enough versions below the cursor");

        // count == remaining: all of them, and no next page.
        let (view, _) = big.page(&big.params(scope, remaining, Some(&cursor))).await;
        assert_eq!(view.versions, rest.versions, "count == remaining");
        assert!(!view.has_next, "count == remaining ends the walk");

        // count == remaining - 1: all but the oldest, and a next page of one.
        let short = remaining - 1;
        let (view, page) = big.page(&big.params(scope, short, Some(&cursor))).await;
        assert_eq!(view.versions.len() as u32, short);
        assert_eq!(view.versions.as_slice(), &rest.versions[..short as usize]);
        assert!(view.has_next, "count == remaining - 1 has a next page");
        let next = page.page_info.next_cursor.clone().expect("next cursor");
        let (view, _) = big.page(&big.params(scope, short, Some(&next))).await;
        assert_eq!(view.versions.len(), 1, "the oldest version alone");
        assert_eq!(Some(&view.versions[0]), rest.versions.last());
        assert!(!view.has_next);

        // count == remaining + 1: all of them, and no next page.
        let (view, _) = big
            .page(&big.params(scope, remaining + 1, Some(&cursor)))
            .await;
        assert_eq!(view.versions, rest.versions, "count == remaining + 1");
        assert!(!view.has_next);

        // A cursor taken from page N, read with another page size.
        let walked = big.walk(scope, 7, 400, true).await;
        assert!(walked.ended);
        for page_no in [3_usize, 10] {
            let cursor = &walked.cursors[page_no - 1];
            let (view, _) = big.page(&big.params(scope, 50, Some(cursor))).await;
            assert_eq!(
                view,
                old_page(&big.rows, scope, 50, cursor_version(cursor)),
                "include_deleted={include_deleted}: the cursor after page {page_no}, count 50"
            );
        }
    }
    eprintln!(
        "instance_history_pages_match_the_old_algorithm: whole test took {:?}",
        test_started.elapsed()
    );
}

/// #2375: the same equivalence over versions the backend wrote itself (create,
/// update, delete and restore), plus a check against `list_versions` and `vread`.
/// Page size 1 walks without a window; sizes 7 and 50 walk under every window.
#[tokio::test]
async fn instance_history_pages_match_for_backend_written_versions() {
    let Some(backend) = create_backend("inst_hist_written").await else {
        eprintln!(
            "Skipping instance_history_pages_match_for_backend_written_versions \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, _db, history) = open_history(&backend).await;
    let tenant = create_tenant("tenant-inst-hist-written");

    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "w1"}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    let id = created.id().to_string();
    assert_eq!(created.version_id(), "1");
    for v in 2..=121_i64 {
        if v % 9 == 0 {
            backend
                .delete(&tenant, "Patient", &id)
                .await
                .unwrap_or_else(|e| panic!("delete at v{v}: {e:?}"));
        } else {
            let (stored, _) = backend
                .create_or_update(
                    &tenant,
                    "Patient",
                    &id,
                    json!({
                        "resourceType": "Patient",
                        "id": id,
                        "name": [{"family": format!("w{v}")}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap_or_else(|e| panic!("create_or_update at v{v}: {e:?}"));
            assert_eq!(stored.version_id(), v.to_string(), "the version written");
        }
    }

    let subject = Subject::load(&backend, &tenant, &history, &id).await;
    assert_eq!(subject.rows.len(), 121);
    let stamp = |version: &str| {
        subject
            .rows
            .iter()
            .find(|row| row.version_id == version)
            .expect("stored version")
            .last_updated
    };
    let windows = [
        (None, None),
        (Some(stamp("95")), None),
        (None, Some(stamp("12"))),
    ];
    // Page size 1 reads a page per version, so it walks without a window only;
    // sizes 7 and 50 walk under every window.
    let started = std::time::Instant::now();
    for count in [1_u32, 7, 50] {
        for include_deleted in [true, false] {
            for (since, before) in windows {
                if count == 1 && (since.is_some() || before.is_some()) {
                    continue;
                }
                let scope = Scope {
                    include_deleted,
                    since,
                    before,
                };
                let walked = subject.walk(scope, count, 400, false).await;
                assert!(
                    walked.ended,
                    "count={count} {scope:?}: the walk did not end"
                );
            }
        }
    }
    eprintln!(
        "instance_history_pages_match_for_backend_written_versions: walks took {:?}",
        started.elapsed()
    );

    // The full walk is list_versions reversed, and vread returns the same rows.
    let walked = subject.walk(Scope::all(), 50, 10, false).await;
    assert!(walked.ended);
    let mut listed = backend
        .list_versions(&tenant, "Patient", &id)
        .await
        .expect("list_versions");
    listed.reverse();
    assert_eq!(walked.versions, listed);
    let page = subject
        .backend
        .history_instance(
            &tenant,
            "Patient",
            &id,
            &subject.params(Scope::all(), 200, None),
        )
        .await
        .expect("one page of everything");
    assert_eq!(page.items.len(), 121);
    for version in ["1", "60", "121"] {
        let item = page
            .items
            .iter()
            .find(|entry| entry.resource.version_id() == version)
            .expect("walked version");
        let read = backend
            .vread(&tenant, "Patient", &id, version)
            .await
            .expect("vread")
            .unwrap_or_else(|| panic!("vread v{version} exists"));
        assert_eq!(read.content(), item.resource.content(), "v{version}");
        assert_eq!(read.version_id(), version);
    }
}

/// #2375, #2392: the cursor forms an instance-history page accepts. A string
/// cursor and a `Previous` cursor name the same position, an undecodable cursor
/// and an offset serve page 1, a string cursor that is not a stored version is a
/// position like any other (here below every version), and a count of 0 is an
/// empty page without a next cursor. The shapes that are refused are in
/// `instance_history_cursor_that_does_not_fit_is_invalid_cursor`.
#[tokio::test]
async fn instance_history_cursor_forms() {
    let Some(backend) = create_backend("inst_hist_cursors").await else {
        eprintln!(
            "Skipping instance_history_cursor_forms \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, _db, history) = open_history(&backend).await;
    let tenant = create_tenant("tenant-inst-hist-cursors");
    let tenant_id = tenant.tenant_id().as_str().to_string();
    let template = template_row(&backend, &history, &tenant).await;

    let base_ms = Utc::now().timestamp_millis() - 86_400_000;
    let id = "cursor-forms";
    insert_rows(
        &history,
        plain_versions(121, base_ms)
            .into_iter()
            .map(|(v, at, deleted)| {
                history_row(&template, &tenant_id, id, &v.to_string(), at, deleted)
            })
            .collect(),
    )
    .await;
    let subject = Subject::load(&backend, &tenant, &history, id).await;
    let scope = Scope::all();
    let below_60 = old_page(&subject.rows, scope, 10, Some(60));
    let first_page = old_page(&subject.rows, scope, 10, None);
    assert_eq!(below_60.versions[0], "59");

    let with_pagination = |mode: PaginationMode| {
        let mut params = subject.params(scope, 10, None);
        params.pagination = Pagination { count: 10, mode };
        params
    };
    let cursor_of = |value: CursorValue| PageCursor::new(vec![value], id);

    let (view, _) = subject
        .page(&with_pagination(PaginationMode::Cursor(Some(cursor_of(
            CursorValue::String("60".to_string()),
        )))))
        .await;
    assert_eq!(view, below_60, "a String cursor");

    let previous = PageCursor::previous(vec![CursorValue::String("60".to_string())], id);
    let (view, _) = subject
        .page(&with_pagination(PaginationMode::Cursor(Some(previous))))
        .await;
    assert_eq!(view, below_60, "a Previous cursor");

    // Page 1: an undecodable cursor, an offset, and no cursor.
    let (view, _) = subject
        .page(&subject.params(scope, 10, Some("not-a-cursor")))
        .await;
    assert_eq!(view, first_page, "an undecodable cursor");
    let (view, _) = subject
        .page(&with_pagination(PaginationMode::Offset(20)))
        .await;
    assert_eq!(view, first_page, "an offset");
    let (view, _) = subject.page(&subject.params(scope, 10, None)).await;
    assert_eq!(view, first_page, "no cursor");

    // A string cursor that is not a number is a position, `(0, "not-a-version")`,
    // which is below every version 1..=121 of this resource: the last page, empty.
    let (view, _) = subject
        .page(&with_pagination(PaginationMode::Cursor(Some(cursor_of(
            CursorValue::String("not-a-version".to_string()),
        )))))
        .await;
    assert_eq!(
        view,
        PageView {
            versions: vec![],
            has_next: false,
            next: None
        },
        "a cursor that is not a version"
    );

    // A count of 0 is an empty page and no next cursor.
    let (view, page) = subject.page(&subject.params(scope, 0, None)).await;
    assert!(page.items.is_empty());
    assert!(!view.has_next);
    assert_eq!(page.page_info.next_cursor, None);
    assert_eq!(view, old_page(&subject.rows, scope, 0, None));
}

/// #2392 (fails before): version ids that are not canonical decimal numbers page
/// in one total order, so a walk visits every version once and ends. The order
/// is the version's number (a non-numeric id counts as 0, `07` and `7` are both
/// 7), then the stored id, both descending: `list_versions` reversed. Before
/// #2392 the cursor held the number only. A non-numeric last version read as no
/// cursor, so the walk at page size 1 went 8, 7, 2, 1, c, 8, 7, ... and never
/// ended, and a cursor at `7` dropped its sibling `07`. No HFS writer produces
/// such ids; this pins how a raw or imported row is read. A canonical resource
/// is walked too, against [`old_page`], as the control that nothing changed.
#[tokio::test]
async fn non_canonical_version_ids_page_in_one_total_order() {
    let Some(backend) = create_backend("inst_hist_noncanon").await else {
        eprintln!(
            "Skipping non_canonical_version_ids_page_in_one_total_order \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, _db, history) = open_history(&backend).await;
    let tenant = create_tenant("tenant-inst-hist-noncanon");
    let tenant_id = tenant.tenant_id().as_str().to_string();
    let template = template_row(&backend, &history, &tenant).await;

    let base_ms = Utc::now().timestamp_millis() - 86_400_000;
    let id = "odd-ids";
    // `07` is newer than `7`, and `a` is the newest of the zeros: the order by
    // `last_updated` that #2375 used for ties disagrees with the order by id.
    let ids: [(&str, i64, bool); 8] = [
        ("1", 1_000, false),
        ("2", 2_000, false),
        ("7", 3_000, false),
        ("07", 4_000, false),
        ("8", 5_000, false),
        ("b", 6_000, true),
        ("c", 7_000, false),
        ("a", 8_000, false),
    ];
    insert_rows(
        &history,
        ids.iter()
            .map(|(version, offset, deleted)| {
                history_row(
                    &template,
                    &tenant_id,
                    id,
                    version,
                    base_ms + offset,
                    *deleted,
                )
            })
            .collect(),
    )
    .await;
    let subject = Subject::load(&backend, &tenant, &history, id).await;
    assert_eq!(subject.rows.len(), ids.len());

    let strings = |versions: &[&str]| -> Vec<String> {
        versions.iter().map(|version| version.to_string()).collect()
    };
    let with_deleted = strings(&["8", "7", "07", "2", "1", "c", "b", "a"]);
    let without_deleted = strings(&["8", "7", "07", "2", "1", "c", "a"]);

    // The order is `list_versions` reversed.
    let mut listed = backend
        .list_versions(&tenant, "Patient", id)
        .await
        .expect("list_versions");
    listed.reverse();
    assert_eq!(listed, with_deleted, "list_versions reversed");

    for (include_deleted, expected) in [(true, &with_deleted), (false, &without_deleted)] {
        let scope = Scope {
            include_deleted,
            since: None,
            before: None,
        };
        for count in [1_u32, 2, 3, 4, 50] {
            let walked = subject
                .walk_with(scope, count, 20, true, |cursor| {
                    total_order_page(
                        &subject.rows,
                        scope,
                        count,
                        cursor.and_then(cursor_version_id).as_deref(),
                    )
                })
                .await;
            assert!(
                walked.ended,
                "count={count} {scope:?}: the walk ends, versions so far {:?}",
                walked.versions
            );
            assert_eq!(
                &walked.versions, expected,
                "count={count} {scope:?}: every version once, in order"
            );
            assert_eq!(
                walked.cursors.len(),
                expected.len().div_ceil(count as usize) - 1,
                "count={count} {scope:?}: the last page has no next, even when it is full"
            );
        }
    }

    // A cursor an older build issued (the version id as a string) names a
    // position in that order.
    let page_after = |version: &str| {
        let encoded = PageCursor::new(vec![CursorValue::String(version.to_string())], id).encode();
        subject.params(Scope::all(), 50, Some(&encoded))
    };
    for (cursor, expected) in [
        ("8", vec!["7", "07", "2", "1", "c", "b", "a"]),
        ("7", vec!["07", "2", "1", "c", "b", "a"]),
        ("07", vec!["2", "1", "c", "b", "a"]),
        ("c", vec!["b", "a"]),
        // Not stored: a position between ids.
        ("zz", vec!["c", "b", "a"]),
        ("a", vec![]),
    ] {
        let (view, page) = subject.page(&page_after(cursor)).await;
        assert_eq!(
            view,
            PageView {
                versions: strings(&expected),
                has_next: false,
                next: None
            },
            "a cursor at {cursor:?}"
        );
        subject.check_items(&page, true, &format!("cursor at {cursor:?}"));
    }

    // The control: canonical ids keep the order and the pages of the old
    // algorithm.
    let canon = "canon";
    insert_rows(
        &history,
        plain_versions(11, base_ms)
            .into_iter()
            .map(|(v, at, deleted)| {
                history_row(&template, &tenant_id, canon, &v.to_string(), at, deleted)
            })
            .collect(),
    )
    .await;
    let canonical = Subject::load(&backend, &tenant, &history, canon).await;
    let all: Vec<String> = (1..=11).rev().map(|v| v.to_string()).collect();
    for count in 1_u32..=4 {
        let walked = canonical.walk(Scope::all(), count, 20, true).await;
        assert!(walked.ended, "canonical count={count}: the walk ends");
        assert_eq!(walked.versions, all, "canonical count={count}");
    }
}

/// #2392 (fails before): a cursor that is not exactly what `history_instance`
/// issues, one string version id of this resource, is `InvalidCursor`, not
/// page 1 again. Empty, `Number`, `Decimal`, `Null`, `Boolean`, two-value and
/// another resource's cursors, `Next` or `Previous`, and an encoded number the
/// way a client sends it back. The control is an old-style cursor, one string
/// version id, which still pages.
#[tokio::test]
async fn instance_history_cursor_that_does_not_fit_is_invalid_cursor() {
    let Some(backend) = create_backend("inst_hist_badcursor").await else {
        eprintln!(
            "Skipping instance_history_cursor_that_does_not_fit_is_invalid_cursor \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, _db, history) = open_history(&backend).await;
    let tenant = create_tenant("tenant-inst-hist-badcursor");
    let tenant_id = tenant.tenant_id().as_str().to_string();
    let template = template_row(&backend, &history, &tenant).await;

    let base_ms = Utc::now().timestamp_millis() - 86_400_000;
    let id = "fit";
    insert_rows(
        &history,
        plain_versions(5, base_ms)
            .into_iter()
            .map(|(v, at, deleted)| {
                history_row(&template, &tenant_id, id, &v.to_string(), at, deleted)
            })
            .collect(),
    )
    .await;
    let subject = Subject::load(&backend, &tenant, &history, id).await;

    let string = |version: &str| CursorValue::String(version.to_string());
    let cases: Vec<(&str, PageCursor)> = vec![
        ("empty", PageCursor::new(vec![], id)),
        ("Number", PageCursor::new(vec![CursorValue::Number(3)], id)),
        (
            "Decimal",
            PageCursor::new(vec![CursorValue::Decimal(3.0)], id),
        ),
        ("Null", PageCursor::new(vec![CursorValue::Null], id)),
        (
            "Boolean",
            PageCursor::new(vec![CursorValue::Boolean(true)], id),
        ),
        (
            "two values",
            PageCursor::new(vec![string("3"), string("2")], id),
        ),
        (
            "another instance",
            PageCursor::new(vec![string("3")], "other-id"),
        ),
        (
            "another instance, Previous",
            PageCursor::previous(vec![string("3")], "other-id"),
        ),
    ];
    let refused =
        |label: &str, result: Result<HistoryPage, StorageError>, expected_cursor: String| {
            match result {
                Err(StorageError::Search(SearchError::InvalidCursor { cursor })) => {
                    assert_eq!(
                        cursor, expected_cursor,
                        "{label}: the error names the cursor"
                    );
                }
                Err(other) => panic!("{label}: expected InvalidCursor, got {other:?}"),
                Ok(_) => panic!("{label}: expected InvalidCursor, got a page"),
            }
        };
    for (label, cursor) in &cases {
        // Set directly, not through `Pagination::with_cursor`: with serde_json's
        // `arbitrary_precision` (on in this build) an encoded `Decimal` does not
        // decode, and an undecodable cursor is no cursor.
        let params = HistoryParams {
            pagination: Pagination {
                count: 2,
                mode: PaginationMode::Cursor(Some(cursor.clone())),
            },
            ..HistoryParams::default()
        };
        let result = backend
            .history_instance(&tenant, "Patient", id, &params)
            .await;
        refused(label, result, cursor.encode());
    }

    // An encoded Number cursor, as a client sends it back.
    let number = PageCursor::new(vec![CursorValue::Number(3)], id);
    let params = HistoryParams {
        pagination: Pagination::with_cursor(2, number.encode()),
        ..HistoryParams::default()
    };
    let result = backend
        .history_instance(&tenant, "Patient", id, &params)
        .await;
    refused("an encoded Number cursor", result, number.encode());

    // The control: an old-style cursor, one string version id of this resource.
    let ok = PageCursor::new(vec![string("3")], id).encode();
    let (view, _) = subject
        .page(&subject.params(Scope::all(), 2, Some(&ok)))
        .await;
    assert_eq!(
        view,
        PageView {
            versions: vec!["2".to_string(), "1".to_string()],
            has_next: false,
            next: None
        },
        "a string cursor at 3"
    );
}

/// #2375 (fails before): a page does not read the rows outside it. Version 1 of
/// the resource has no payload; the first two pages of 50 never reach it, the
/// third does. Before #2375 every page read and parsed every row, so page 1
/// failed.
#[tokio::test]
async fn instance_history_page_does_not_read_rows_outside_the_page() {
    let Some(backend) = create_backend("inst_hist_canary").await else {
        eprintln!(
            "Skipping instance_history_page_does_not_read_rows_outside_the_page \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, _db, history) = open_history(&backend).await;
    let tenant = create_tenant("tenant-inst-hist-canary");
    let tenant_id = tenant.tenant_id().as_str().to_string();
    let template = template_row(&backend, &history, &tenant).await;

    let base_ms = Utc::now().timestamp_millis() - 86_400_000;
    let id = "canary";
    insert_rows(
        &history,
        (1..=121_i64)
            .map(|v| {
                history_row(
                    &template,
                    &tenant_id,
                    id,
                    &v.to_string(),
                    base_ms + v * 1_000,
                    false,
                )
            })
            .collect(),
    )
    .await;
    history
        .update_one(
            doc! { "tenant_id": &tenant_id, "resource_type": "Patient", "id": id, "version_id": "1" },
            doc! { "$unset": { "data": "" } },
        )
        .await
        .expect("remove the payload of version 1");

    let params = HistoryParams::new().count(50).include_deleted(true);
    let page1 = backend
        .history_instance(&tenant, "Patient", id, &params)
        .await
        .unwrap_or_else(|e| panic!("page 1 must not read version 1: {e:?}"));
    let versions: Vec<String> = page1
        .items
        .iter()
        .map(|entry| entry.resource.version_id().to_string())
        .collect();
    let expected: Vec<String> = (72..=121_i64).rev().map(|v| v.to_string()).collect();
    assert_eq!(versions, expected);

    let mut params2 = params.clone();
    params2.pagination = Pagination::with_cursor(
        50,
        page1.page_info.next_cursor.clone().expect("next cursor"),
    );
    let page2 = backend
        .history_instance(&tenant, "Patient", id, &params2)
        .await
        .unwrap_or_else(|e| panic!("page 2 must not read version 1: {e:?}"));
    let versions: Vec<String> = page2
        .items
        .iter()
        .map(|entry| entry.resource.version_id().to_string())
        .collect();
    let expected: Vec<String> = (22..=71_i64).rev().map(|v| v.to_string()).collect();
    assert_eq!(versions, expected);

    let mut params3 = params.clone();
    params3.pagination = Pagination::with_cursor(
        50,
        page2.page_info.next_cursor.clone().expect("next cursor"),
    );
    match backend
        .history_instance(&tenant, "Patient", id, &params3)
        .await
    {
        Ok(page) => panic!(
            "page 3 reaches version 1 and must fail, got {} items",
            page.items.len()
        ),
        Err(e) => assert!(
            format!("{e:?}").contains("Missing history payload"),
            "page 3 fails on the data-less row: {e:?}"
        ),
    }
}

fn docs_examined(entry: &Document) -> i64 {
    match entry.get("docsExamined") {
        Some(Bson::Int32(n)) => i64::from(*n),
        Some(Bson::Int64(n)) => *n,
        _ => 0,
    }
}

/// Runs `call` with the profiler at level 2 and returns its value, the sum of
/// `docsExamined` over the `query` and `getmore` entries on `resource_history`,
/// the number of `query` entries, and the entries (oldest first). `None` when
/// the server refuses the profiler (the caller skips).
async fn profiled_docs<T>(
    db: &mongodb::Database,
    call: impl std::future::Future<Output = T>,
) -> Option<(T, i64, usize, Vec<Document>)> {
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;
    let profile = db.collection::<Document>("system.profile");
    let _ = profile.drop().await;
    if let Err(e) = db.run_command(doc! { "profile": 2_i32 }).await {
        eprintln!("Skipping instance_history_paging: {{profile: 2}} was refused ({e})");
        return None;
    }
    let value = call.await;
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;
    let entries: Vec<Document> = profile
        .find(doc! {
            "ns": format!("{}.resource_history", db.name()),
            "op": { "$in": ["query", "getmore"] },
        })
        // A capped collection: natural order is the order the entries were written.
        .sort(doc! { "$natural": 1_i32 })
        .await
        .expect("read system.profile")
        .try_collect()
        .await
        .expect("collect system.profile");
    let docs = entries.iter().map(docs_examined).sum();
    let finds = entries
        .iter()
        .filter(|entry| entry.get_str("op").is_ok_and(|op| op == "query"))
        .count();
    Some((value, docs, finds, entries))
}

/// #2375 (fails before): a page of 7 over a resource of 121 versions examines
/// about one page of documents. The server's profiler counts the documents of the
/// `find`s and `getMore`s the backend sent: the version-id read (covered by
/// `idx_history_identity`, 0 documents) and the fetch of the page's versions.
#[tokio::test]
async fn instance_history_page_examines_about_one_page_of_documents() {
    let Some(backend) = create_backend("inst_hist_docs").await else {
        eprintln!(
            "Skipping instance_history_page_examines_about_one_page_of_documents \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, db, history) = open_history(&backend).await;
    let tenant = create_tenant("tenant-inst-hist-docs");
    let tenant_id = tenant.tenant_id().as_str().to_string();
    let template = template_row(&backend, &history, &tenant).await;

    let base_ms = Utc::now().timestamp_millis() - 86_400_000;
    let id = "docs";
    insert_rows(
        &history,
        plain_versions(121, base_ms)
            .into_iter()
            .map(|(v, at, deleted)| {
                history_row(&template, &tenant_id, id, &v.to_string(), at, deleted)
            })
            .collect(),
    )
    .await;

    let live = HistoryParams::new().count(7).include_deleted(true);
    let Some((page1, docs1, finds1, entries1)) =
        profiled_docs(&db, backend.history_instance(&tenant, "Patient", id, &live)).await
    else {
        return;
    };
    let page1 = page1.expect("page 1");
    assert_eq!(page1.items.len(), 7);
    assert!(
        docs1 <= 8,
        "page 1 (count 7) examined {docs1} documents, expected at most 8: {entries1:#?}"
    );
    assert_eq!(
        finds1, 2,
        "one version-id read and one fetch: {entries1:#?}"
    );
    let first = entries1
        .iter()
        .find(|entry| entry.get_str("op").is_ok_and(|op| op == "query"))
        .expect("a query entry");
    assert_eq!(
        docs_examined(first),
        0,
        "the version-id read examines no document: {first:#?}"
    );
    let summary = first.get_str("planSummary").unwrap_or_default();
    assert!(
        summary.contains("version_id: 1"),
        "the version-id read uses idx_history_identity, got {summary:?}"
    );

    // The page after page 1.
    let mut live2 = live.clone();
    live2.pagination =
        Pagination::with_cursor(7, page1.page_info.next_cursor.clone().expect("next cursor"));
    let (page2, docs2, _, entries2) = profiled_docs(
        &db,
        backend.history_instance(&tenant, "Patient", id, &live2),
    )
    .await
    .expect("the profiler was accepted a moment ago");
    assert_eq!(page2.expect("page 2").items.len(), 7);
    assert!(
        docs2 <= 8,
        "page 2 (count 7) examined {docs2} documents, expected at most 8: {entries2:#?}"
    );

    // Without deleted versions (every seventh is deleted) a few more candidates
    // are fetched, not another full page.
    let live_only = HistoryParams::new().count(7);
    let (page, docs, _, entries) = profiled_docs(
        &db,
        backend.history_instance(&tenant, "Patient", id, &live_only),
    )
    .await
    .expect("the profiler was accepted a moment ago");
    assert_eq!(page.expect("live page").items.len(), 7);
    assert!(
        docs <= 24,
        "a live page (count 7) examined {docs} documents, expected at most 24: {entries:#?}"
    );
}

/// Most pages [`walk_numbers`] reads before it gives up: a resource of 61 versions
/// is 21 pages at size 3, so a walk past this is a cursor that does not advance.
const MAX_WALK_PAGES: usize = 200;

/// One full walk at page size 3, as a list of version numbers.
async fn walk_numbers(backend: &MongoBackend, tenant: &TenantContext, id: &str) -> Vec<i64> {
    let mut cursor: Option<String> = None;
    let mut seen: Vec<i64> = Vec::new();
    let mut pages = 0_usize;
    loop {
        pages += 1;
        assert!(
            pages <= MAX_WALK_PAGES,
            "id={id}: the walk is still going after {MAX_WALK_PAGES} pages (a cursor that does \
             not advance?); versions seen so far: {seen:?}"
        );
        let mut params = HistoryParams::new().count(3).include_deleted(true);
        if let Some(cursor) = &cursor {
            params.pagination = Pagination::with_cursor(3, cursor.clone());
        }
        let page = backend
            .history_instance(tenant, "Patient", id, &params)
            .await
            .expect("history_instance during the writes");
        seen.extend(
            page.items
                .iter()
                .map(|entry| number_of(entry.resource.version_id())),
        );
        if !page.page_info.has_next {
            return seen;
        }
        cursor = page.page_info.next_cursor;
    }
}

/// #2375: walks run while a writer adds versions. The old single `find` was not
/// a snapshot either; what holds is that every walk is strictly decreasing, has
/// no duplicate and no gap down to version 1: a version written during the walk
/// is above every version already returned, and history rows do not change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn instance_history_walk_is_consistent_under_concurrent_updates() {
    let Some(backend) = create_backend("inst_hist_concurrent").await else {
        eprintln!(
            "Skipping instance_history_walk_is_consistent_under_concurrent_updates \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let backend = Arc::new(backend);
    let tenant = create_tenant("tenant-inst-hist-concurrent");
    let created = backend
        .create(
            &tenant,
            "Patient",
            json!({"resourceType": "Patient", "name": [{"family": "c1"}]}),
            FhirVersion::default(),
        )
        .await
        .expect("create");
    let id = created.id().to_string();

    let writer = {
        let backend = Arc::clone(&backend);
        let id = id.clone();
        tokio::spawn(async move {
            let tenant = create_tenant("tenant-inst-hist-concurrent");
            for v in 2..=61_i64 {
                let (stored, _) = backend
                    .create_or_update(
                        &tenant,
                        "Patient",
                        &id,
                        json!({
                            "resourceType": "Patient",
                            "id": id,
                            "name": [{"family": format!("c{v}")}]
                        }),
                        FhirVersion::default(),
                    )
                    .await
                    .unwrap_or_else(|e| panic!("update to v{v}: {e:?}"));
                assert_eq!(stored.version_id(), v.to_string());
            }
        })
    };

    let mut walks = 0_usize;
    loop {
        let seen = walk_numbers(&backend, &tenant, &id).await;
        walks += 1;
        let top = *seen.first().expect("a walk returns versions");
        let expected: Vec<i64> = (1..=top).rev().collect();
        assert_eq!(
            seen, expected,
            "walk {walks}: strictly decreasing, no duplicate, no gap down to 1"
        );
        if writer.is_finished() {
            break;
        }
    }
    writer.await.expect("the writer finished");
    eprintln!("instance_history_walk_is_consistent_under_concurrent_updates: {walks} walks");

    let seen = walk_numbers(&backend, &tenant, &id).await;
    let expected: Vec<i64> = (1..=61_i64).rev().collect();
    assert_eq!(seen, expected, "after the writes: 61 down to 1");
}

/// #2375: a page is scoped to its tenant on both of its commands (the version-id
/// read and the document fetches). The same Patient id holds versions 1..=5 in
/// tenant A and 1..=8 in tenant B, in the same database; each tenant's walk
/// returns only its own versions, and every page equals the old algorithm's over
/// that tenant's rows (the timestamps differ between the tenants, so a row read
/// from the other tenant fails the timestamp check).
#[tokio::test]
async fn instance_history_pages_stay_within_their_tenant() {
    let started = std::time::Instant::now();
    let Some(backend) = create_backend("inst_hist_tenants").await else {
        eprintln!(
            "Skipping instance_history_pages_stay_within_their_tenant \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, _db, history) = open_history(&backend).await;
    let tenant_a = create_tenant("tenant-inst-hist-iso-a");
    let tenant_b = create_tenant("tenant-inst-hist-iso-b");
    let template = template_row(&backend, &history, &tenant_a).await;

    let base_ms = Utc::now().timestamp_millis() - 10 * 86_400_000;
    let id = "shared";
    let mut rows = Vec::new();
    for v in 1..=5_i64 {
        rows.push(history_row(
            &template,
            tenant_a.tenant_id().as_str(),
            id,
            &v.to_string(),
            base_ms + v * 1_000,
            false,
        ));
    }
    for v in 1..=8_i64 {
        rows.push(history_row(
            &template,
            tenant_b.tenant_id().as_str(),
            id,
            &v.to_string(),
            base_ms + v * 1_000 + 500,
            false,
        ));
    }
    insert_rows(&history, rows).await;

    let a = Subject::load(&backend, &tenant_a, &history, id).await;
    let b = Subject::load(&backend, &tenant_b, &history, id).await;
    assert_eq!(a.rows.len(), 5, "tenant A's raw rows");
    assert_eq!(b.rows.len(), 8, "tenant B's raw rows");

    let expected = |top: i64| -> Vec<String> { (1..=top).rev().map(|v| v.to_string()).collect() };
    for count in [2_u32, 50] {
        let walked = a.walk(Scope::all(), count, 20, true).await;
        assert!(walked.ended, "tenant A, count {count}: the walk ended");
        assert_eq!(
            walked.versions,
            expected(5),
            "tenant A, count {count}: versions 5..=1 and none of tenant B's"
        );

        let walked = b.walk(Scope::all(), count, 20, true).await;
        assert!(walked.ended, "tenant B, count {count}: the walk ended");
        assert_eq!(
            walked.versions,
            expected(8),
            "tenant B, count {count}: versions 8..=1 and none of tenant A's"
        );
    }
    eprintln!(
        "instance_history_pages_stay_within_their_tenant: took {:?}",
        started.elapsed()
    );
}

/// #2375: a fetch never names more than `INSTANCE_HISTORY_MAX_BATCH` (1,024)
/// versions, and the batches lose and duplicate no row. One resource of 2,200
/// live versions is walked two ways, every page compared with the old algorithm.
///
/// - `before` keeps versions 1..=40 at page size 7. Inferred from the batch
///   arithmetic: the first fetch names 8 versions and matches none, so the next
///   ones double (16, 32, ..., 1,024) until 2,040 candidates were dropped; the
///   doubling then asks for 2,048, which is clamped to 1,024 and ends at the 160
///   candidates left (versions 160..=1), which hold the 40 matches.
/// - Page size 1,100 without a window needs 1,101 rows, more than one batch: the
///   first fetch of page 1 names 1,024 versions, and a second names the 77 more.
///   Page 2 holds the other 1,100.
#[tokio::test]
async fn instance_history_batches_reach_the_cap() {
    let started = std::time::Instant::now();
    let Some(backend) = create_backend("inst_hist_cap").await else {
        eprintln!(
            "Skipping instance_history_batches_reach_the_cap \
             (requires Docker or HFS_TEST_MONGODB_URL)"
        );
        return;
    };
    let (_client, db, history) = open_history(&backend).await;
    let tenant = create_tenant("tenant-inst-hist-cap");
    let tenant_id = tenant.tenant_id().as_str().to_string();
    let template = template_row(&backend, &history, &tenant).await;

    let base_ms = Utc::now().timestamp_millis() - 10 * 86_400_000;
    let id = "cap";
    insert_rows(
        &history,
        (1..=2_200_i64)
            .map(|v| {
                history_row(
                    &template,
                    &tenant_id,
                    id,
                    &v.to_string(),
                    base_ms + v * 1_000,
                    false,
                )
            })
            .collect(),
    )
    .await;
    let subject = Subject::load(&backend, &tenant, &history, id).await;
    assert_eq!(subject.rows.len(), 2_200, "the raw rows");

    // Walk 1: a window at the old end of the resource.
    let windowed = Scope {
        include_deleted: true,
        since: None,
        before: Some(utc_millis(base_ms + 41_000)),
    };
    let walked = subject.walk(windowed, 7, 20, true).await;
    assert!(walked.ended, "the windowed walk ended");
    assert_eq!(
        walked.versions,
        (1..=40_i64)
            .rev()
            .map(|v| v.to_string())
            .collect::<Vec<_>>(),
        "the window keeps versions 40..=1"
    );

    // Walk 2: a page that needs more than one batch.
    let walked = subject.walk(Scope::all(), 1_100, 5, true).await;
    assert!(walked.ended, "the 1,100-row walk ended");
    assert_eq!(walked.cursors.len(), 1, "two pages: one next cursor");
    assert_eq!(walked.versions.len(), 2_200, "1,100 items on each page");
    assert_eq!(
        walked.versions,
        (1..=2_200_i64)
            .rev()
            .map(|v| v.to_string())
            .collect::<Vec<_>>(),
        "every version once, newest first"
    );

    // Page 1 of walk 2 under the profiler: 1,101 documents over three queries
    // (the version-id read, 1,024 versions and 77 versions).
    let params = subject.params(Scope::all(), 1_100, None);
    if let Some((_, docs, finds, entries)) = profiled_docs(&db, subject.page(&params)).await {
        assert!(
            docs <= 1_101,
            "page 1 (count 1,100) examined {docs} documents, expected at most 1,101: {entries:#?}"
        );
        assert_eq!(
            finds, 3,
            "one version-id read and two fetches (1,024 + 77): {entries:#?}"
        );
    }
    eprintln!(
        "instance_history_batches_reach_the_cap: took {:?}",
        started.elapsed()
    );
}
