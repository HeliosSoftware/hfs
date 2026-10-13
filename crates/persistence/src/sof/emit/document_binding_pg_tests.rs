//! PostgreSQL plan-shape gates for the resource document binding (#1866).
//!
//! Each statement is compiled twice through the production run preamble
//! ([`SqlRunPlan::compile`], then `finish` or `finish_paged`): the *candidate*
//! as shipped, and the *baseline* with marking switched off by
//! [`without_document_binding`] around the synchronous `compile`, which is the
//! main-branch SQL (the committed goldens and their frozen `unbound/` copies
//! pin that where a golden exists). Both arms are explained with
//! `EXPLAIN (FORMAT JSON)` and executed with bound parameters, in one session
//! with identical settings, against a generated fixture in a PostgreSQL 16
//! testcontainer whose schema comes from [`PostgresBackend::init_schema`].
//!
//! # Fixture
//!
//! Three tenants in one database of their own:
//!
//! - `pad`: 5000 Patients (exactly 4 names, given names, telecoms and
//!   addresses), 5000 Observations (exactly 4 components) and 2000
//!   QuestionnaireResponses (nested `item` / `answer.item`), every document
//!   padded with poorly compressible text so it is stored out of line in
//!   TOAST. A uniform fan-out of 4 is what makes PostgreSQL 16 choose a
//!   `Memoize` for expansion sources rooted at the bound document alone.
//! - `plain`: the same Patients and Observations without padding (stored
//!   inline), for the binding cost log.
//! - `mix`: smaller and irregular documents (missing, empty and 1–4 element
//!   arrays, absent subjects and codings, deleted rows, `last_updated` ties),
//!   half of them padded.
//!
//! The fixture stays below the 30 000 rows `ANALYZE` samples, so its
//! statistics, and therefore the plans, do not depend on sampling.
//! Initialisation asserts that every padded document has exactly one
//! out-of-line TOAST value and that unpadded documents stay inline; without
//! that the gates would not exercise de-TOAST work, so every gate fails.
//!
//! # Gates (per statement, tenant and page)
//!
//! 1. The binding lateral occurs exactly `k` times in the candidate SQL, where
//!    `k` is the number of bound scan sites (cross-checked against
//!    [`scan_sites`]), and never in the baseline; the arms differ iff `k > 0`.
//! 2. The candidate plan has `k` `Function Scan`s on `jsonb_extract_path`
//!    (alias `rdoc`, which `EXPLAIN` shows as `rdoc_1`, ... when a `unionAll`
//!    binds more than one branch); the baseline has none.
//! 3. `Gather` + `Gather Merge` counts are equal. Non-vacuity: the baseline
//!    has a Gather-class node for the designated statements on `pad`.
//! 4. `Memoize`: candidate ≤ baseline.
//! 5. Unmarked statements (`k = 0`): identical SQL and identical plan JSON.
//!    Marked statements: every other node-type count difference is logged,
//!    and differences in Sort-, Gather- or Hash/Merge-join-class nodes are
//!    flagged in the log for review; they are not asserted.
//! 6. Rows: multisets always equal; select statements (total `ORDER BY`)
//!    equal as ordered arrays; `unionAll` and `repeat` statements (`ORDER BY
//!    1`) equal in their key sequence, each tie group as a multiset.
//!
//! The `repeat` statements are unmarked (`k = 0`): the rejoin of a traversal
//! to `resources` is never marked, even when it reads the document, so gate 5
//! holds them to the main-branch SQL and plan.
//!
//! Also logged per statement, as measured data for review, not asserted:
//! **placement**, i.e. whether a binding function scan runs in the leader
//! above the Gather of its own `resources` scan, and whether fewer expansion
//! (`jsonb_array_elements`) function scans run inside the parallel part than
//! in the baseline. Both are planner cost choices with the shipped binding too
//! (it may gather the bound rows before the fan-out), so placement is reviewed
//! with its measured cost rather than asserted.
//!
//! # Teeth
//!
//! 7. (a) A binding whose expansion sources read `rdoc.doc` without the
//!    `COALESCE(.., r.data)` steer makes the planner add a `Memoize`, which
//!    gate 4 rejects. (b) **Weaker:** the `(SELECT r.data #> '{}' OFFSET 0)`
//!    subquery form does not lose a Gather-class node on this fixture, so
//!    gate 3 does not reject it. It moves the fan-out above the Gather, which
//!    the logged placement measurement shows; the test asserts only that.
//! 8. `jsonb_extract_path` is parallel safe (`proparallel = 's'`), the premise
//!    that lets the binding run inside parallel workers.
//!
//! Run with `--nocapture` to see the per-statement JSON log lines
//! (`N5-GATE`, `N5-TEETH`, `N5-COST`, `N5-FIXTURE`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use helios_fhir::FhirVersion;
use serde_json::{Value, json};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, NoTls, Row};

use super::emit_plan;
use crate::backends::postgres::{PostgresBackend, PostgresConfig};
use crate::core::sof_runner::ViewFilters;
use crate::sof::compile_view::build_plan;
use crate::sof::compiler::{CompileTarget, SqlDialect};
use crate::sof::dialect::{Dialect, PgDialect, ResourceDocumentBinding};
use crate::sof::ir::document_reads::{ScanSite, scan_sites, without_document_binding};
use crate::sof::ir::{JsonType, LitValue, SqlType};
use crate::sof::runtime::{PreparedSqlRun, RuntimeParam, SqlRunPlan};

// ----------------------------------------------------------------------------
// Container, database and fixture
// ----------------------------------------------------------------------------

/// Database of this test binary inside the container.
const DATABASE: &str = "hfs1866_document_binding";

/// Session settings, identical for both arms: custom plans for bound
/// parameters, and parallel plans made cheap so the small fixture plans like
/// a large table.
const SESSION_SETTINGS: &str = "SET plan_cache_mode = force_custom_plan;
SET max_parallel_workers_per_gather = 2;
SET parallel_setup_cost = 0;
SET parallel_tuple_cost = 0;
SET min_parallel_table_scan_size = 0;
SET min_parallel_index_scan_size = 0;";

/// `TOAST_TUPLE_THRESHOLD` with the default 8 kB block: a datum above it is
/// compressed or moved out of line.
const INLINE_LIMIT_BYTES: i32 = 2032;

struct Fixture {
    host: String,
    port: u16,
    /// Kept alive for the test binary; `container_cleanup` removes it at exit.
    _container: ContainerAsync<Postgres>,
}

static FIXTURE: OnceCell<Fixture> = OnceCell::const_new();

async fn connect(host: &str, port: u16, dbname: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(
        &format!("host={host} port={port} user=postgres password=postgres dbname={dbname}"),
        NoTls,
    )
    .await
    .expect("connect to the gate container");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

/// Starts the container once per test binary, creates the schema with the
/// production `init_schema`, loads the fixture and checks its TOAST storage.
async fn fixture() -> &'static Fixture {
    FIXTURE
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            let container = crate::container_cleanup::with_cleanup_label(
                Postgres::default()
                    .with_tag("16-alpine")
                    .with_label("github.run_id", &run_id),
            )
            .start()
            .await
            .expect("start PostgreSQL 16 testcontainer");
            let host = container.get_host().await.expect("host").to_string();
            let port = container.get_host_port_ipv4(5432).await.expect("port");

            connect(&host, port, "postgres")
                .await
                .batch_execute(&format!("CREATE DATABASE {DATABASE}"))
                .await
                .expect("create the gate database");

            let data_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("data"))
                .unwrap_or_else(|| PathBuf::from("data"));
            let backend = PostgresBackend::new(PostgresConfig {
                host: host.clone(),
                port,
                dbname: DATABASE.into(),
                user: "postgres".into(),
                password: Some("postgres".into()),
                max_connections: 5,
                data_dir: Some(data_dir),
                ..Default::default()
            })
            .await
            .expect("backend");
            backend.init_schema().await.expect("init_schema");
            drop(backend);

            let client = connect(&host, port, DATABASE).await;
            client
                .batch_execute(&fixture_sql())
                .await
                .expect("load the fixture");
            client
                .batch_execute("VACUUM ANALYZE resources")
                .await
                .expect("vacuum analyze");
            let toast = toast_report(&client).await;
            println!("N5-FIXTURE {}", json!({"toast": toast}));
            assert_eq!(
                toast["ok"],
                json!(true),
                "fixture documents are not stored as planned, gates cannot run: {toast}"
            );
            Fixture {
                host,
                port,
                _container: container,
            }
        })
        .await
}

/// A session on the fixture database with [`SESSION_SETTINGS`].
async fn session() -> Client {
    let fixture = fixture().await;
    let client = connect(&fixture.host, fixture.port, DATABASE).await;
    client
        .batch_execute(SESSION_SETTINGS)
        .await
        .expect("session settings");
    client
}

/// `text.div` of `128` deterministic md5 hex strings (4096 characters): lz4
/// barely compresses it, so the document is stored out of line.
fn padding(prefix: &str) -> String {
    format!(
        "jsonb_build_object('text', jsonb_build_object('status', 'generated', 'div', '<div>' || \
         (SELECT string_agg(md5('{prefix}' || i || '-' || k), '' ORDER BY k) FROM generate_series(1, 128) k) \
         || '</div>'))"
    )
}

fn fixture_sql() -> String {
    let columns = "INSERT INTO resources (tenant_id, resource_type, id, version_id, data, last_updated, is_deleted)";
    let pad_patient = padding("p");
    let pad_observation = padding("o");
    let pad_response = padding("q");
    let pad_mix = padding("m");
    format!(
        "
-- pad / plain: uniform fan-out 4, same documents, padded or not.
{columns}
SELECT t.tenant, 'Patient', 'p' || lpad(i::text, 6, '0'), '1',
  jsonb_build_object(
    'resourceType', 'Patient', 'id', 'p' || lpad(i::text, 6, '0'), 'active', true,
    'gender', CASE WHEN i % 2 = 0 THEN 'female' ELSE 'male' END,
    'name', (SELECT jsonb_agg(jsonb_build_object('use', 'official', 'family', 'Fam' || i || '_' || k,
               'given', jsonb_build_array('G' || i || '_' || k, 'H' || i || '_' || k)) ORDER BY k)
             FROM generate_series(1, 4) k),
    'telecom', (SELECT jsonb_agg(jsonb_build_object('system', 'phone', 'value', 'tel' || i || '_' || k) ORDER BY k)
                FROM generate_series(1, 4) k),
    'address', (SELECT jsonb_agg(jsonb_build_object('city', 'City' || i || '_' || k) ORDER BY k)
                FROM generate_series(1, 4) k))
  || CASE WHEN t.padded THEN {pad_patient} ELSE '{{}}'::jsonb END,
  timestamptz '2025-01-01 00:00:00+00' + make_interval(secs => i), false
FROM (VALUES ('pad', true), ('plain', false)) AS t(tenant, padded), generate_series(1, 5000) AS i;

{columns}
SELECT t.tenant, 'Observation', 'o' || lpad(i::text, 6, '0'), '1',
  jsonb_build_object(
    'resourceType', 'Observation', 'id', 'o' || lpad(i::text, 6, '0'), 'status', 'final',
    'code', jsonb_build_object('coding', jsonb_build_array(jsonb_build_object(
              'system', 'http://loinc.org', 'code', '85354-9')), 'text', 'Blood pressure panel'),
    'subject', jsonb_build_object('reference', 'Patient/p' || lpad(i::text, 6, '0')),
    'component', (SELECT jsonb_agg(jsonb_build_object(
                    'code', jsonb_build_object('coding', jsonb_build_array(jsonb_build_object(
                              'system', 'http://loinc.org', 'code', 'c' || k)), 'text', 'component ' || k),
                    'valueQuantity', jsonb_build_object('value', (i % 97) + k, 'unit', 'mm[Hg]')) ORDER BY k)
                  FROM generate_series(1, 4) k))
  || CASE WHEN t.padded THEN {pad_observation} ELSE '{{}}'::jsonb END,
  timestamptz '2025-01-01 00:00:00+00' + make_interval(secs => i), false
FROM (VALUES ('pad', true), ('plain', false)) AS t(tenant, padded), generate_series(1, 5000) AS i;

{columns}
SELECT 'pad', 'QuestionnaireResponse', 'q' || lpad(i::text, 6, '0'), '1',
  jsonb_build_object(
    'resourceType', 'QuestionnaireResponse', 'id', 'q' || lpad(i::text, 6, '0'), 'status', 'completed',
    'item', (SELECT jsonb_agg(jsonb_build_object(
               'linkId', 'l' || k,
               'answer', jsonb_build_array(jsonb_build_object(
                 'valueString', CASE WHEN k = 2 THEN 'skip' ELSE 'a' || i || '_' || k END,
                 'item', jsonb_build_array(jsonb_build_object('linkId', 'l' || k || '.a',
                   'answer', jsonb_build_array(jsonb_build_object('valueString', 'n' || i || '_' || k)))))),
               'item', jsonb_build_array(jsonb_build_object('linkId', 'l' || k || '.i'))) ORDER BY k)
             FROM generate_series(1, 4) k))
  || {pad_response},
  timestamptz '2025-01-01 00:00:00+00' + make_interval(secs => i), false
FROM generate_series(1, 2000) AS i;

-- mix: irregular shapes, ties in last_updated, deleted rows, half padded.
{columns}
SELECT 'mix', 'Patient', 'p' || lpad(i::text, 6, '0'), '1',
  jsonb_strip_nulls(jsonb_build_object(
    'resourceType', 'Patient', 'id', 'p' || lpad(i::text, 6, '0'), 'active', i % 3 <> 0,
    'gender', CASE i % 3 WHEN 0 THEN 'female' WHEN 1 THEN 'male' END,
    'name', CASE WHEN i % 8 = 0 THEN NULL WHEN i % 8 = 4 THEN '[]'::jsonb ELSE
              (SELECT jsonb_agg(jsonb_strip_nulls(jsonb_build_object(
                 'use', CASE WHEN k = 1 THEN 'official' ELSE 'usual' END,
                 'family', CASE WHEN (i + k) % 5 = 0 THEN NULL ELSE 'Fam' || i || '_' || k END,
                 'given', (SELECT jsonb_agg('G' || i || '_' || k || '_' || g ORDER BY g)
                           FROM generate_series(1, (i + k) % 3) g))) ORDER BY k)
               FROM generate_series(1, 1 + i % 3) k) END,
    'telecom', (SELECT jsonb_agg(jsonb_build_object('system', CASE WHEN k % 2 = 0 THEN 'email' ELSE 'phone' END,
                  'value', 't' || i || '_' || k) ORDER BY k) FROM generate_series(1, i % 3) k),
    'address', (SELECT jsonb_agg(jsonb_build_object('city', 'City' || (i % 17)) ORDER BY k)
                FROM generate_series(1, (i / 2) % 3) k)))
  || CASE WHEN i % 2 = 0 THEN {pad_mix} ELSE '{{}}'::jsonb END,
  timestamptz '2025-01-01 00:00:00+00' + make_interval(secs => i / 3), i % 50 = 0
FROM generate_series(1, 2000) AS i;

{columns}
SELECT 'mix', 'Observation', 'o' || lpad(i::text, 6, '0'), '1',
  jsonb_strip_nulls(jsonb_build_object(
    'resourceType', 'Observation', 'id', 'o' || lpad(i::text, 6, '0'), 'status', 'final',
    'code', jsonb_build_object('text', 'Panel ' || (i % 7)),
    'subject', CASE WHEN i % 7 = 0 THEN NULL
                    ELSE jsonb_build_object('reference', 'Patient/p' || lpad((i % 1500)::text, 6, '0')) END,
    'component', (SELECT jsonb_agg(jsonb_strip_nulls(jsonb_build_object(
                    'code', jsonb_build_object(
                      'coding', CASE WHEN k = 2 THEN NULL ELSE jsonb_build_array(jsonb_build_object(
                                  'system', 'http://loinc.org', 'code', 'c' || k)) END,
                      'text', 'component ' || k),
                    'valueQuantity', jsonb_build_object('value', (i % 97) + k))) ORDER BY k)
                  FROM generate_series(1, i % 5) k)))
  || CASE WHEN i % 2 = 1 THEN {pad_mix} ELSE '{{}}'::jsonb END,
  timestamptz '2025-01-01 00:00:00+00' + make_interval(secs => i / 4), i % 61 = 0
FROM generate_series(1, 2000) AS i;

{columns}
SELECT 'mix', 'QuestionnaireResponse', 'q' || lpad(i::text, 6, '0'), '1',
  jsonb_strip_nulls(jsonb_build_object(
    'resourceType', 'QuestionnaireResponse', 'id', 'q' || lpad(i::text, 6, '0'), 'status', 'completed',
    'item', (SELECT jsonb_agg(jsonb_strip_nulls(jsonb_build_object(
               'linkId', 'm' || k,
               'answer', CASE WHEN k % 2 = 1 THEN jsonb_build_array(jsonb_build_object(
                           'valueString', 'v' || i || '_' || k,
                           'item', jsonb_build_array(jsonb_build_object('linkId', 'm' || k || '.a')))) END,
               'item', CASE WHEN (i + k) % 3 = 0 THEN jsonb_build_array(jsonb_build_object(
                         'linkId', 'm' || k || '.i',
                         'item', jsonb_build_array(jsonb_build_object('linkId', 'm' || k || '.i.i')))) END))
               ORDER BY k)
             FROM generate_series(1, i % 4) k)))
  || CASE WHEN i % 3 = 0 THEN {pad_mix} ELSE '{{}}'::jsonb END,
  timestamptz '2025-01-01 00:00:00+00' + make_interval(secs => i / 2), false
FROM generate_series(1, 1000) AS i;
"
    )
}

/// Where the fixture documents are stored. Padded documents (the ones with
/// `text`) must each have exactly one out-of-line value in the TOAST relation
/// of `resources` and a stored size above the inline limit; the others must
/// stay inline. PostgreSQL 16 has no per-row TOAST pointer accessor, so the
/// first check counts distinct chunk ids.
async fn toast_report(client: &Client) -> Value {
    let toast: String = client
        .query_one(
            "SELECT reltoastrelid::regclass::text FROM pg_class WHERE oid = 'resources'::regclass",
            &[],
        )
        .await
        .expect("toast relation")
        .get(0);
    let chunk_ids: i64 = client
        .query_one(
            &format!("SELECT count(DISTINCT chunk_id) FROM {toast}"),
            &[],
        )
        .await
        .expect("toast chunks")
        .get(0);
    let mut groups = Vec::new();
    let mut padded_rows = 0i64;
    let mut sizes_ok = true;
    for row in client
        .query(
            "SELECT tenant_id, resource_type, data ? 'text' AS padded, count(*), \
             min(pg_column_size(data)), max(pg_column_size(data)), \
             round(avg(octet_length(data::text)))::bigint \
             FROM resources GROUP BY 1, 2, 3 ORDER BY 1, 2, 3",
            &[],
        )
        .await
        .expect("document sizes")
    {
        let padded: bool = row.get(2);
        let rows: i64 = row.get(3);
        let min: i32 = row.get(4);
        let max: i32 = row.get(5);
        if padded {
            padded_rows += rows;
            sizes_ok &= min > INLINE_LIMIT_BYTES;
        } else {
            sizes_ok &= max < INLINE_LIMIT_BYTES;
        }
        groups.push(json!({
            "tenant": row.get::<_, String>(0), "resource_type": row.get::<_, String>(1),
            "padded": padded, "rows": rows, "pg_column_size_min": min, "pg_column_size_max": max,
            "text_length_avg": row.get::<_, i64>(6),
        }));
    }
    json!({
        "toast_relation": toast,
        "toast_distinct_chunk_ids": chunk_ids,
        "padded_rows": padded_rows,
        "inline_limit_bytes": INLINE_LIMIT_BYTES,
        "groups": groups,
        "ok": chunk_ids == padded_rows && padded_rows > 0 && sizes_ok,
    })
}

// ----------------------------------------------------------------------------
// Statements and arms
// ----------------------------------------------------------------------------

/// The text of `PgDialect`'s binding lateral.
fn pg_binding() -> ResourceDocumentBinding {
    PgDialect
        .resource_document_binding()
        .expect("PostgreSQL binds the document")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowOrder {
    /// Select statement: `ORDER BY [r.last_updated,] r.id, <occurrence>`.
    Total,
    /// `unionAll` / `repeat`: `ORDER BY 1`, not total.
    KeyThenTies,
}

struct Statement {
    name: &'static str,
    resource: &'static str,
    select: Value,
    /// Bound scan sites.
    k: usize,
    ordering: RowOrder,
    /// Golden case with the same view and filters, unlimited.
    golden: Option<&'static str>,
    /// Golden case with the same view and filters, paged.
    paged_golden: Option<&'static str>,
}

impl Statement {
    fn view(&self) -> Value {
        json!({"resourceType":"ViewDefinition","resource":self.resource,"status":"active","select":self.select})
    }

    fn tenants(&self) -> &'static [&'static str] {
        if self.resource == "QuestionnaireResponse" {
            &["pad", "mix"]
        } else {
            &["pad", "plain", "mix"]
        }
    }
}

fn statement_named(name: &str) -> Statement {
    statements()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no statement {name}"))
}

fn statements() -> Vec<Statement> {
    let patient = |name, select, k, ordering, golden, paged_golden| Statement {
        name,
        resource: "Patient",
        select,
        k,
        ordering,
        golden,
        paged_golden,
    };
    vec![
        patient(
            "flat",
            json!([{"column":[{"path":"id","name":"id"},{"path":"active","name":"active","type":"boolean"}]}]),
            0,
            RowOrder::Total,
            Some("flat"),
            Some("export-page-flat"),
        ),
        patient(
            "foreach",
            json!([{"column":[{"path":"id","name":"r"}]},{"forEach":"name","column":[{"path":"family","name":"u0"}]}]),
            1,
            RowOrder::Total,
            Some("foreach"),
            Some("export-page-foreach"),
        ),
        Statement {
            name: "observation_expanded",
            resource: "Observation",
            select: json!([{"column":[{"name":"id","path":"getResourceKey()","type":"id"},
                {"name":"patient_id","path":"subject.getReferenceKey(Patient)","type":"id"}]},
                {"forEach":"component","column":[{"name":"component_code","path":"code.coding.first().code","type":"string"},
                {"name":"component_name","path":"code.text","type":"string"}]}]),
            k: 1,
            ordering: RowOrder::Total,
            golden: None,
            paged_golden: None,
        },
        patient(
            "cartesian",
            json!([{"forEach":"name","column":[{"path":"family","name":"family"}]},
            {"forEach":"address","column":[{"path":"city","name":"city"}]}]),
            1,
            RowOrder::Total,
            Some("cartesian"),
            None,
        ),
        patient(
            "nested",
            json!([{"forEach":"name","select":[{"column":[{"path":"family","name":"family"}]},
            {"forEach":"given","column":[{"path":"$this","name":"given","type":"string"}]}]}]),
            0,
            RowOrder::Total,
            Some("nested"),
            None,
        ),
        patient(
            "union_one_marked_branch",
            json!([{"column":[{"path":"id","name":"id"}],"unionAll":[
                {"forEach":"telecom","column":[{"path":"value","name":"value"}]},
                {"column":[{"path":"gender","name":"value"}]}]}]),
            1,
            RowOrder::KeyThenTies,
            None,
            None,
        ),
        patient(
            "union_two_marked_branches",
            json!([{"column":[{"name":"id","path":"getResourceKey()","type":"id"}],"unionAll":[
                {"forEach":"name","column":[{"name":"value","path":"family","type":"string"}]},
                {"forEach":"address","column":[{"name":"value","path":"city","type":"string"}]}]}]),
            2,
            RowOrder::KeyThenTies,
            None,
            None,
        ),
        // The shape of main's `union-in-foreach` golden (a `unionAll` under a
        // `forEach`, root `id` after the fan-out in both branches) over this
        // fixture's `name` elements: each branch is marked.
        patient(
            "union_in_foreach",
            json!([{"column":[{"path":"id","name":"id"}]},
            {"forEach":"name","select":[{"unionAll":[
                {"forEach":"given","column":[{"path":"$this","name":"value","type":"string"}]},
                {"column":[{"path":"family","name":"value"}]}]}]}]),
            2,
            RowOrder::KeyThenTies,
            None,
            None,
        ),
        Statement {
            name: "repeat",
            resource: "QuestionnaireResponse",
            select: json!([
                {"column":[{"path":"id","name":"id"},{"path":"item.linkId","name":"roots","collection":true}]},
                {"repeat":["item","answer.item"],"column":[{"path":"linkId","name":"link"},
                    {"path":"answer.valueString","name":"answers","collection":true},
                    {"path":"answer.valueString.join('|')","name":"joined","type":"string"},
                    {"path":"answer.where(valueString != 'skip').first().valueString","name":"picked","type":"string"}]}]),
            // The rejoin reads the document but is never marked (module docs).
            k: 0,
            ordering: RowOrder::KeyThenTies,
            golden: Some("repeat"),
            paged_golden: None,
        },
        Statement {
            name: "observation_repeat_rooted",
            resource: "Observation",
            select: json!([{"column":[{"name":"id","path":"getResourceKey()","type":"id"}]},
                {"repeat":["component"],"column":[{"name":"value","path":"code.text","type":"string"},
                {"name":"code","path":"code.coding.first().code","type":"string"}]}]),
            // Rooted rejoin, never marked (module docs).
            k: 0,
            ordering: RowOrder::KeyThenTies,
            golden: None,
            paged_golden: None,
        },
    ]
}

/// Statements gated unlimited (all) and paged (plan section 9.2).
const PAGED: &[&str] = &[
    "flat",
    "foreach",
    "observation_expanded",
    "cartesian",
    "union_two_marked_branches",
    "repeat",
];

/// Baseline must parallelise these on `pad` (gate 3 non-vacuity): unlimited.
const PARALLEL_UNLIMITED: &[&str] = &["foreach", "cartesian", "observation_expanded"];
/// ... and paged, first page.
const PARALLEL_PAGED_FIRST: &[&str] = &["foreach"];

/// Filters of the paged statements: those of the `export-page-*` goldens
/// (`since` is before every fixture row; the limit is client-side).
fn paged_filters() -> ViewFilters {
    ViewFilters {
        since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
        limit: Some(50),
        ..Default::default()
    }
}

struct Arms {
    baseline: PreparedSqlRun,
    candidate: PreparedSqlRun,
}

/// Both arms through the production compile path. The baseline compiles
/// synchronously inside [`without_document_binding`], so marking is off while
/// `build_plan` runs; `finish` runs outside (the fact is already on the plan).
fn arms(view: &Value, tenant: &str, filters: &ViewFilters, paged: bool) -> Arms {
    let compile = || SqlRunPlan::compile(view, SqlDialect::Postgres, FhirVersion::R4);
    let finish = |plan: SqlRunPlan| {
        if paged {
            plan.finish_paged(tenant, filters)
        } else {
            plan.finish(tenant, filters)
        }
        .expect("finish")
    };
    let baseline = without_document_binding(compile).expect("compile baseline");
    let candidate = compile().expect("compile candidate");
    Arms {
        baseline: finish(baseline),
        candidate: finish(candidate),
    }
}

/// Bound scan sites by the analysis: marked sites, seeds never bound.
fn analysis_k(view: &Value) -> usize {
    let (plan, _) =
        build_plan(view, &PgDialect, CompileTarget::Postgres, FhirVersion::R4).expect("build_plan");
    scan_sites(&plan)
        .iter()
        .filter(|(site, fact)| *fact && !matches!(site, ScanSite::RepeatSeed { .. }))
        .count()
}

/// The SQL part of a committed golden file.
fn golden_sql(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/sof")
        .join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    text.split("\n\n/* Runner bindings and output metadata:")
        .next()
        .unwrap()
        .to_string()
}

/// The main-branch SQL of a golden case: its frozen copy when the committed
/// golden binds the document, else the committed golden.
fn main_branch_sql(case: &str) -> String {
    let unbound = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/sof/unbound")
        .join(format!("{case}.postgres.sql"));
    if unbound.exists() {
        golden_sql(&format!("unbound/{case}.postgres.sql"))
    } else {
        golden_sql(&format!("{case}.postgres.sql"))
    }
}

// ----------------------------------------------------------------------------
// Execution
// ----------------------------------------------------------------------------

type BoxedParam = Box<dyn ToSql + Sync + Send>;

/// Same text representation as the PostgreSQL runner's parameter binding.
fn bind(params: &[RuntimeParam]) -> Vec<BoxedParam> {
    params
        .iter()
        .map(|param| -> BoxedParam {
            match param {
                RuntimeParam::Text(value) => Box::new(value.clone()),
                RuntimeParam::TextList(value) => Box::new(value.clone()),
                RuntimeParam::Timestamp(value) => Box::new(*value),
                RuntimeParam::Literal(LitValue::Null) => Box::new(None::<String>),
                RuntimeParam::Literal(LitValue::Bool(value)) => Box::new(value.to_string()),
                RuntimeParam::Literal(LitValue::Int(value)) => Box::new(value.to_string()),
                RuntimeParam::Literal(LitValue::Decimal(value) | LitValue::Str(value)) => {
                    Box::new(value.clone())
                }
            }
        })
        .collect()
}

async fn query(client: &Client, sql: &str, params: &[RuntimeParam]) -> Vec<Row> {
    let boxed = bind(params);
    let refs: Vec<&(dyn ToSql + Sync)> = boxed
        .iter()
        .map(|b| b.as_ref() as &(dyn ToSql + Sync))
        .collect();
    client.query(sql, &refs).await.unwrap_or_else(|error| {
        let detail = error
            .as_db_error()
            .map_or_else(|| error.to_string(), |db| db.message().to_string());
        panic!("{detail}\n--- SQL ---\n{sql}")
    })
}

/// `EXPLAIN (FORMAT JSON)` with bound parameters: the top-level object.
async fn explain(client: &Client, sql: &str, params: &[RuntimeParam]) -> Value {
    let rows = query(client, &format!("EXPLAIN (FORMAT JSON) {sql}"), params).await;
    let plan: Value = rows[0].get(0);
    plan[0].clone()
}

type TextRow = Vec<Option<String>>;

async fn execute(client: &Client, sql: &str, params: &[RuntimeParam]) -> (Vec<TextRow>, f64) {
    let started = Instant::now();
    let rows = query(client, sql, params).await;
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let rows = rows
        .iter()
        .map(|row| {
            (0..row.len())
                .map(|i| row.get::<_, Option<String>>(i))
                .collect()
        })
        .collect();
    (rows, elapsed)
}

// ----------------------------------------------------------------------------
// Plan shape
// ----------------------------------------------------------------------------

/// Node counts and the placement of the function scans that matter here.
#[derive(Default)]
struct Shape {
    nodes: BTreeMap<String, usize>,
    /// Binding function scans ([`is_binding_scan`]).
    binding_scans: usize,
    /// Binding function scans evaluated above a Gather-class node that sits
    /// over their own `resources` scan: the scan runs in parallel workers but
    /// the binding runs serially in the leader.
    binding_split_from_scan: usize,
    /// `Function Scan` on `jsonb_array_elements` (expansion sources and the
    /// unnests inside scalar subqueries).
    unnest_scans: usize,
    unnest_below_gather: usize,
}

impl Shape {
    fn of(explained: &Value) -> Self {
        let mut shape = Self::default();
        shape.walk(&explained["Plan"], false, &mut Vec::new());
        shape
    }

    fn walk<'a>(&mut self, node: &'a Value, below_gather: bool, ancestors: &mut Vec<&'a Value>) {
        let kind = node["Node Type"].as_str().unwrap_or("?");
        *self.nodes.entry(kind.to_string()).or_default() += 1;
        if is_binding_scan(node) {
            self.binding_scans += 1;
            let split = enclosing_join(ancestors).is_some_and(|join| {
                other_sides(join, node).any(|side| gather_over_resources(side, false))
            });
            self.binding_split_from_scan += usize::from(split);
        } else if kind == "Function Scan" && node["Function Name"] == "jsonb_array_elements" {
            self.unnest_scans += 1;
            self.unnest_below_gather += usize::from(below_gather);
        }
        let below = below_gather || is_gather(kind);
        ancestors.push(node);
        for child in children(node) {
            self.walk(child, below, ancestors);
        }
        ancestors.pop();
    }

    fn count(&self, kind: &str) -> usize {
        self.nodes.get(kind).copied().unwrap_or(0)
    }

    fn gathers(&self) -> usize {
        self.count("Gather") + self.count("Gather Merge")
    }

    fn json(&self) -> Value {
        json!({
            "nodes": self.nodes,
            "gather_class": self.gathers(),
            "memoize": self.count("Memoize"),
            "binding_function_scans": self.binding_scans,
            "binding_split_from_scan": self.binding_split_from_scan,
            "unnest_function_scans": self.unnest_scans,
            "unnest_below_gather": self.unnest_below_gather,
        })
    }
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    node["Plans"].as_array().into_iter().flatten()
}

fn is_gather(kind: &str) -> bool {
    kind == "Gather" || kind == "Gather Merge"
}

/// The binding's `Function Scan` on `jsonb_extract_path`. `EXPLAIN` renames
/// repeated aliases (`rdoc`, `rdoc_1`, ...), one per bound `unionAll` branch.
fn is_binding_scan(node: &Value) -> bool {
    node["Node Type"] == "Function Scan"
        && node["Function Name"] == "jsonb_extract_path"
        && node["Alias"]
            .as_str()
            .is_some_and(|alias| alias == "rdoc" || alias.starts_with("rdoc_"))
}

fn enclosing_join<'a>(ancestors: &[&'a Value]) -> Option<&'a Value> {
    ancestors.iter().rev().copied().find(|a| {
        matches!(
            a["Node Type"].as_str(),
            Some("Nested Loop" | "Hash Join" | "Merge Join")
        )
    })
}

/// The children of `join` that do not contain `node`.
fn other_sides<'a>(join: &'a Value, node: &'a Value) -> impl Iterator<Item = &'a Value> {
    children(join).filter(move |child| !contains_node(child, node))
}

/// Whether a Gather-class node sits between `tree` and a `resources` scan.
fn gather_over_resources(tree: &Value, seen: bool) -> bool {
    let seen = seen || is_gather(tree["Node Type"].as_str().unwrap_or("?"));
    if tree["Relation Name"] == "resources" {
        return seen;
    }
    children(tree).any(|child| gather_over_resources(child, seen))
}

/// Gates 3 (Gather-class counts) and 4 (Memoize) for `other` against
/// `baseline`.
fn gate_3_4_failures(baseline: &Shape, other: &Shape) -> Vec<String> {
    let mut failures = Vec::new();
    if other.gathers() != baseline.gathers() {
        failures.push(format!(
            "gate 3: Gather + Gather Merge {} != baseline {}",
            other.gathers(),
            baseline.gathers()
        ));
    }
    if other.count("Memoize") > baseline.count("Memoize") {
        failures.push(format!(
            "gate 4: Memoize {} > baseline {}",
            other.count("Memoize"),
            baseline.count("Memoize")
        ));
    }
    failures
}

/// Parallel placement of `other` against `baseline`, logged for review and
/// not asserted (module docs): a binding evaluated in the leader above the
/// Gather of its own `resources` scan, or fewer expansion function scans
/// inside the parallel part than in the baseline.
fn placement_findings(baseline: &Shape, other: &Shape) -> Vec<String> {
    let mut findings = Vec::new();
    if other.binding_split_from_scan > 0 {
        findings.push(format!(
            "{} of {} binding function scans run above the Gather of their resources scan",
            other.binding_split_from_scan, other.binding_scans
        ));
    }
    if other.unnest_below_gather < baseline.unnest_below_gather {
        findings.push(format!(
            "expansion function scans below a Gather {} < baseline {}",
            other.unnest_below_gather, baseline.unnest_below_gather
        ));
    }
    findings
}

/// Node kinds whose count change on a marked statement is flagged for review
/// (gate 5's pre-declared escalation classes).
fn is_review_class(kind: &str) -> bool {
    matches!(
        kind,
        "Sort" | "Incremental Sort" | "Gather" | "Gather Merge" | "Hash Join" | "Merge Join"
    )
}

/// Node-type count differences between the arms.
fn node_diff(baseline: &Shape, candidate: &Shape) -> BTreeMap<String, (usize, usize)> {
    baseline
        .nodes
        .keys()
        .chain(candidate.nodes.keys())
        .filter_map(|kind| {
            let pair = (baseline.count(kind), candidate.count(kind));
            (pair.0 != pair.1).then(|| (kind.clone(), pair))
        })
        .collect()
}

fn contains_node(tree: &Value, target: &Value) -> bool {
    std::ptr::eq(tree, target) || children(tree).any(|child| contains_node(child, target))
}

// ----------------------------------------------------------------------------
// Row oracle
// ----------------------------------------------------------------------------

fn sorted(mut rows: Vec<TextRow>) -> Vec<TextRow> {
    rows.sort();
    rows
}

/// Consecutive rows with an equal first column.
fn tie_groups(rows: &[TextRow]) -> Vec<(Option<String>, Vec<TextRow>)> {
    let mut groups: Vec<(Option<String>, Vec<TextRow>)> = Vec::new();
    for row in rows {
        let key = row.first().cloned().flatten();
        match groups.last_mut() {
            Some((last, members)) if *last == key => members.push(row.clone()),
            _ => groups.push((key, vec![row.clone()])),
        }
    }
    groups
}

/// Gate 6. Returns the failure, if any.
fn oracle_failure(
    ordering: RowOrder,
    baseline: &[TextRow],
    candidate: &[TextRow],
) -> Option<String> {
    if sorted(baseline.to_vec()) != sorted(candidate.to_vec()) {
        return Some(format!(
            "gate 6: row multisets differ ({} vs {} rows)",
            baseline.len(),
            candidate.len()
        ));
    }
    match ordering {
        RowOrder::Total => (baseline != candidate)
            .then(|| "gate 6: row order differs under a total ORDER BY".to_string()),
        RowOrder::KeyThenTies => {
            let (b, c) = (tie_groups(baseline), tie_groups(candidate));
            let keys = |groups: &[(Option<String>, Vec<TextRow>)]| {
                groups.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()
            };
            if keys(&b) != keys(&c) {
                return Some("gate 6: key sequence differs".to_string());
            }
            b.into_iter()
                .zip(c)
                .any(|((_, x), (_, y))| sorted(x) != sorted(y))
                .then(|| "gate 6: a tie group differs as a multiset".to_string())
        }
    }
}

// ----------------------------------------------------------------------------
// Gate runner
// ----------------------------------------------------------------------------

/// A page of the paged statements, as `(label, after, through)` over the
/// live ids of the tenant's type in `id COLLATE "C"` order.
async fn pages(
    client: &Client,
    tenant: &str,
    resource: &str,
) -> Vec<(&'static str, String, String)> {
    let ids: Vec<String> = client
        .query(
            "SELECT id FROM resources WHERE tenant_id = $1 AND resource_type = $2 \
             AND is_deleted = false ORDER BY id COLLATE \"C\"",
            &[&tenant, &resource],
        )
        .await
        .expect("page ids")
        .iter()
        .map(|row| row.get(0))
        .collect();
    let at = |percent: usize| ids[ids.len() * percent / 100].clone();
    vec![("first", String::new(), at(25)), ("mid", at(40), at(60))]
}

async fn gate(
    client: &Client,
    statement: &Statement,
    tenant: &str,
    page: Option<(&str, &str, &str)>,
    must_parallelise: bool,
    failures: &mut Vec<String>,
) {
    let paged = page.is_some();
    let filters = if paged {
        paged_filters()
    } else {
        ViewFilters::default()
    };
    let view = statement.view();
    let Arms {
        mut baseline,
        mut candidate,
    } = arms(&view, tenant, &filters, paged);
    if let Some((_, after, through)) = page {
        for prepared in [&mut baseline, &mut candidate] {
            let n = prepared.params.len();
            prepared.params[n - 2] = RuntimeParam::Text(after.to_string());
            prepared.params[n - 1] = RuntimeParam::Text(through.to_string());
        }
    }
    let label = format!(
        "{}{} on {tenant}",
        statement.name,
        page.map_or(String::new(), |(p, _, _)| format!(" (paged, {p} page)"))
    );
    let failures_before = failures.len();
    let mut fail = |message: String| failures.push(format!("{label}: {message}"));
    let (base_sql, cand_sql) = (&baseline.query.sql, &candidate.query.sql);
    let lateral = pg_binding().lateral;
    let k = statement.k;

    // Gate 1.
    let analysed = analysis_k(&view);
    if analysed != k {
        fail(format!("declared k {k} != analysis {analysed}"));
    }
    let laterals = cand_sql.matches(lateral).count();
    if laterals != k {
        fail(format!("gate 1: {laterals} binding laterals, expected {k}"));
    }
    if base_sql.contains("rdoc") {
        fail("gate 1: baseline SQL binds the document".into());
    }
    if (base_sql != cand_sql) != (k > 0) {
        fail(format!(
            "gate 1: arms differ = {}, k = {k}",
            base_sql != cand_sql
        ));
    }
    let golden = if paged {
        statement.paged_golden
    } else {
        statement.golden
    };
    if let Some(case) = golden {
        if *base_sql != main_branch_sql(case) {
            fail(format!(
                "baseline SQL differs from the main-branch golden {case}"
            ));
        }
        if *cand_sql != golden_sql(&format!("{case}.postgres.sql")) {
            fail(format!(
                "candidate SQL differs from the committed golden {case}"
            ));
        }
    }

    // Gates 2-5.
    let base_plan = explain(client, base_sql, &baseline.params).await;
    let cand_plan = explain(client, cand_sql, &candidate.params).await;
    let (base_shape, cand_shape) = (Shape::of(&base_plan), Shape::of(&cand_plan));
    if cand_shape.binding_scans != k || base_shape.binding_scans != 0 {
        fail(format!(
            "gate 2: binding function scans candidate {} (expected {k}), baseline {}",
            cand_shape.binding_scans, base_shape.binding_scans
        ));
    }
    if must_parallelise && base_shape.gathers() == 0 {
        fail("gate 3: non-vacuity: the baseline has no Gather-class node".into());
    }
    for message in gate_3_4_failures(&base_shape, &cand_shape) {
        fail(message);
    }
    let placement = placement_findings(&base_shape, &cand_shape);
    let diff = node_diff(&base_shape, &cand_shape);
    let review: Vec<&String> = diff.keys().filter(|kind| is_review_class(kind)).collect();
    if k == 0 && (base_sql != cand_sql || base_plan != cand_plan) {
        fail("gate 5: unmarked statement changed its SQL or plan".into());
    }

    // Gate 6.
    let (base_rows, base_ms) = execute(client, base_sql, &baseline.params).await;
    let (cand_rows, cand_ms) = execute(client, cand_sql, &candidate.params).await;
    let oracle = oracle_failure(statement.ordering, &base_rows, &cand_rows);
    if let Some(message) = &oracle {
        fail(message.clone());
    }

    let oracle_kind = match statement.ordering {
        RowOrder::Total => "exact order",
        RowOrder::KeyThenTies => "key sequence + tie multisets",
    };
    println!(
        "N5-GATE {}",
        json!({
            "statement": statement.name,
            "tenant": tenant,
            "paged": paged,
            "page": page.map(|(p, a, t)| json!({"label": p, "after": a, "through": t})),
            "k": k,
            "laterals_in_sql": laterals,
            "golden": golden,
            "sql_identical": base_sql == cand_sql,
            "plan_identical": base_plan == cand_plan,
            "baseline": base_shape.json(),
            "candidate": cand_shape.json(),
            "node_diff": diff.iter().map(|(kind, (b, c))| json!({"node": kind, "baseline": b, "candidate": c})).collect::<Vec<_>>(),
            "review_class_diff": review,
            "placement_review": placement,
            "rows": base_rows.len(),
            "oracle": oracle_kind,
            "oracle_ok": oracle.is_none(),
            "exec_ms_indicative": {"baseline": base_ms, "candidate": cand_ms},
            "failures": &failures[failures_before..],
            "plans": (k > 0 || failures.len() > failures_before)
                .then(|| json!({"baseline": base_plan, "candidate": cand_plan})),
        })
    );
}

#[tokio::test]
async fn unlimited_statements_keep_their_plan_shape_and_rows() {
    let client = session().await;
    let mut failures = Vec::new();
    for statement in statements() {
        for tenant in statement.tenants() {
            let must = *tenant == "pad" && PARALLEL_UNLIMITED.contains(&statement.name);
            gate(&client, &statement, tenant, None, must, &mut failures).await;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn paged_statements_keep_their_plan_shape_and_rows() {
    let client = session().await;
    let mut failures = Vec::new();
    for name in PAGED {
        let statement = statement_named(name);
        for tenant in statement.tenants() {
            for (label, after, through) in pages(&client, tenant, statement.resource).await {
                let must = *tenant == "pad"
                    && label == "first"
                    && PARALLEL_PAGED_FIRST.contains(&statement.name);
                gate(
                    &client,
                    &statement,
                    tenant,
                    Some((label, after.as_str(), through.as_str())),
                    must,
                    &mut failures,
                )
                .await;
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ----------------------------------------------------------------------------
// Teeth (gates 7 and 8)
// ----------------------------------------------------------------------------

/// `PgDialect` with a different resource document binding: the wrong
/// variants the gates must reject. `name()` stays `"postgres"`, so the emitter
/// treats it as PostgreSQL everywhere else.
struct WrongBinding(ResourceDocumentBinding);

impl Dialect for WrongBinding {
    fn name(&self) -> &'static str {
        PgDialect.name()
    }
    fn placeholder(&self, idx: usize) -> String {
        PgDialect.placeholder(idx)
    }
    fn string_literal(&self, s: &str) -> String {
        PgDialect.string_literal(s)
    }
    fn json_field(&self, base: &str, key: &str) -> String {
        PgDialect.json_field(base, key)
    }
    fn json_field_text(&self, base: &str, key: &str) -> String {
        PgDialect.json_field_text(base, key)
    }
    fn json_path(&self, base: &str, segments: &[&str]) -> String {
        PgDialect.json_path(base, segments)
    }
    fn json_path_text(&self, base: &str, segments: &[&str]) -> String {
        PgDialect.json_path_text(base, segments)
    }
    fn json_path_repr(&self, base: &str, segments: &[&str]) -> String {
        PgDialect.json_path_repr(base, segments)
    }
    fn unnest_array(&self, expr: &str) -> String {
        PgDialect.unnest_array(expr)
    }
    fn coalesce_array(&self, expr: &str) -> String {
        PgDialect.coalesce_array(expr)
    }
    fn json_type(&self, expr: &str) -> String {
        PgDialect.json_type(expr)
    }
    fn json_agg(&self, expr: &str, occurrence_order: &[String]) -> String {
        PgDialect.json_agg(expr, occurrence_order)
    }
    fn string_agg(&self, expr: &str, sep_param: &str, occurrence_order: &[String]) -> String {
        PgDialect.string_agg(expr, sep_param, occurrence_order)
    }
    fn bool_true(&self) -> &'static str {
        PgDialect.bool_true()
    }
    fn bool_false(&self) -> &'static str {
        PgDialect.bool_false()
    }
    fn lateral_keyword(&self) -> &'static str {
        PgDialect.lateral_keyword()
    }
    fn cast(&self, inner: &str, ty: SqlType) -> String {
        PgDialect.cast(inner, ty)
    }
    fn has_json_type(&self, expr: &str, ty: JsonType) -> String {
        PgDialect.has_json_type(expr, ty)
    }
    fn truthy_predicate(&self, expr: &str) -> String {
        PgDialect.truthy_predicate(expr)
    }
    fn last_path_segment(&self, s: &str) -> String {
        PgDialect.last_path_segment(s)
    }
    fn resource_document_binding(&self) -> Option<ResourceDocumentBinding> {
        Some(self.0)
    }
}

/// Expansion sources read the bound document alone: the planner sees an
/// opaque function-scan output and may cache the fan-out per document.
fn without_coalesce() -> ResourceDocumentBinding {
    let pg = pg_binding();
    ResourceDocumentBinding {
        expansion_root: pg.read_root,
        ..pg
    }
}

/// The lateral subquery form: an `OFFSET 0` fence instead of a function scan.
const OFFSET_ZERO_LATERAL: &str =
    "CROSS JOIN LATERAL (SELECT r.data #> '{}' AS doc OFFSET 0) AS rdoc";

fn offset_zero() -> ResourceDocumentBinding {
    ResourceDocumentBinding {
        lateral: OFFSET_ZERO_LATERAL,
        ..pg_binding()
    }
}

/// Teeth statements: unlimited, tenant `pad` (uniform fan-out 4, TOASTed).
const TEETH: &[&str] = &["foreach", "observation_expanded", "cartesian"];

/// Explains the baseline, the candidate and the `wrong` variant of each teeth
/// statement and returns, per statement, `(name, baseline, candidate, wrong)`.
async fn teeth(
    client: &Client,
    wrong: ResourceDocumentBinding,
    expected_sql: impl Fn(&str) -> String,
) -> Vec<(&'static str, Shape, Shape, Shape)> {
    let mut out = Vec::new();
    for name in TEETH {
        let statement = statement_named(name);
        let view = statement.view();
        let arms = arms(&view, "pad", &ViewFilters::default(), false);
        let (plan, _) = build_plan(&view, &PgDialect, CompileTarget::Postgres, FhirVersion::R4)
            .expect("build_plan");
        let wrong_sql = emit_plan(&plan, &WrongBinding(wrong)).expect("emit").sql;
        assert_eq!(
            wrong_sql,
            expected_sql(&arms.candidate.query.sql),
            "{name}: the wrong variant must differ from the candidate only in the binding"
        );
        let base =
            Shape::of(&explain(client, &arms.baseline.query.sql, &arms.baseline.params).await);
        let cand =
            Shape::of(&explain(client, &arms.candidate.query.sql, &arms.candidate.params).await);
        let bad = Shape::of(&explain(client, &wrong_sql, &arms.candidate.params).await);
        out.push((statement.name, base, cand, bad));
    }
    out
}

/// Gate 7 (a): without the `COALESCE(rdoc.doc, r.data)` steer, PostgreSQL
/// adds a `Memoize`, and gate 4 rejects that variant.
#[tokio::test]
async fn teeth_expansion_sources_without_coalesce_get_a_memoize() {
    let client = session().await;
    let pg = pg_binding();
    let results = teeth(&client, without_coalesce(), |sql| {
        sql.replace(pg.expansion_root, pg.read_root)
    })
    .await;
    let mut rejected = 0;
    for (name, base, cand, bad) in &results {
        let gate4 = gate_3_4_failures(base, bad)
            .into_iter()
            .filter(|f| f.starts_with("gate 4"))
            .collect::<Vec<_>>();
        rejected += usize::from(!gate4.is_empty());
        println!(
            "N5-TEETH {}",
            json!({"teeth": "7a without COALESCE", "statement": name, "tenant": "pad",
                   "baseline": base.json(), "candidate": cand.json(), "wrong": bad.json(),
                   "gate4_rejects_wrong": gate4})
        );
        assert!(
            gate_3_4_failures(base, cand).is_empty(),
            "{name}: the candidate itself fails gates 3-4"
        );
    }
    assert!(
        rejected >= 1,
        "no statement got a Memoize without the COALESCE steer: gate 7 (a) has no teeth on this fixture"
    );
}

/// Gate 7 (b), weaker (module docs): the `OFFSET 0` lateral keeps the
/// Gather-class node count on this fixture, so gate 3 does not reject it; the
/// logged placement measurement shows that it evaluates the fan-out above the
/// Gather.
#[tokio::test]
async fn teeth_offset_zero_binding_is_seen_by_placement_only() {
    let client = session().await;
    let lateral = pg_binding().lateral;
    let results = teeth(&client, offset_zero(), |sql| {
        sql.replace(lateral, OFFSET_ZERO_LATERAL)
    })
    .await;
    let mut seen = 0;
    for (name, base, cand, bad) in &results {
        let placement = placement_findings(base, bad);
        seen += usize::from(!placement.is_empty());
        println!(
            "N5-TEETH {}",
            json!({"teeth": "7b OFFSET 0 subquery", "statement": name, "tenant": "pad",
                   "baseline": base.json(), "candidate": cand.json(), "wrong": bad.json(),
                   "gate_3_4_reject_wrong": gate_3_4_failures(base, bad),
                   "placement_of_wrong": placement,
                   "placement_of_candidate": placement_findings(base, cand)})
        );
        assert!(
            gate_3_4_failures(base, cand).is_empty(),
            "{name}: the candidate itself fails gates 3-4"
        );
    }
    assert!(
        seen >= 1,
        "the OFFSET 0 form kept the fan-out inside the parallel part everywhere: \
         the placement measurement has no teeth on this fixture"
    );
}

/// Gate 8 (DR4): the binding function is parallel safe.
#[tokio::test]
async fn jsonb_extract_path_is_parallel_safe() {
    let client = session().await;
    let flags: Vec<String> = client
        .query(
            "SELECT proparallel::text FROM pg_proc WHERE proname = 'jsonb_extract_path'",
            &[],
        )
        .await
        .expect("pg_proc")
        .iter()
        .map(|row| row.get(0))
        .collect();
    println!(
        "N5-TEETH {}",
        json!({"teeth": "8 proparallel", "jsonb_extract_path": flags})
    );
    assert!(!flags.is_empty(), "jsonb_extract_path not found");
    assert!(flags.iter().all(|flag| flag == "s"), "{flags:?}");
}

// ----------------------------------------------------------------------------
// Fixture evidence
// ----------------------------------------------------------------------------

/// Dumps the `resources` columns and indexes created by `init_schema`, for
/// comparison with the main-branch before-state, and re-checks the TOAST
/// storage of the fixture.
#[tokio::test]
async fn fixture_schema_and_toast_storage() {
    let client = session().await;
    let columns: Vec<Value> = client
        .query(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull, \
             pg_get_expr(d.adbin, d.adrelid), a.attstorage::text, \
             coalesce(nullif(a.attcompression::text, ''), '-') \
             FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attrelid = 'resources'::regclass AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            &[],
        )
        .await
        .expect("columns")
        .iter()
        .map(|row| {
            json!({"column": row.get::<_, String>(0), "type": row.get::<_, String>(1),
                   "not_null": row.get::<_, bool>(2), "default": row.get::<_, Option<String>>(3),
                   "storage": row.get::<_, String>(4), "compression": row.get::<_, String>(5)})
        })
        .collect();
    let indexes: Vec<String> = client
        .query(
            "SELECT indexname || ' | ' || indexdef FROM pg_indexes \
             WHERE tablename = 'resources' ORDER BY indexname",
            &[],
        )
        .await
        .expect("indexes")
        .iter()
        .map(|row| row.get(0))
        .collect();
    let version: Option<i32> = client
        .query_one("SELECT max(version) FROM schema_version", &[])
        .await
        .expect("schema_version")
        .get(0);
    let server: String = client
        .query_one("SHOW server_version", &[])
        .await
        .expect("server_version")
        .get(0);
    let toast = toast_report(&client).await;
    println!(
        "N5-FIXTURE {}",
        json!({"server_version": server, "schema_version": version, "columns": columns,
               "indexes": indexes, "toast": toast})
    );
    assert_eq!(toast["ok"], json!(true), "{toast}");
    let data = columns
        .iter()
        .find(|c| c["column"] == "data")
        .expect("data column");
    assert_eq!(data["storage"], "x", "resources.data must be TOASTable");
}

/// Cost of the binding, logged only: `EXPLAIN (ANALYZE, BUFFERS)` of both
/// arms on out-of-line (`pad`) and inline (`plain`) documents, one warm-up and
/// three measured runs. The paged `foreach` first page is the shape whose
/// placement changes (module docs). Container timings are indicative.
#[tokio::test]
async fn binding_cost_is_logged() {
    let client = session().await;
    let cases = [
        ("foreach", false),
        ("observation_expanded", false),
        ("foreach", true),
    ];
    for (name, paged) in cases {
        let statement = statement_named(name);
        for tenant in ["pad", "plain"] {
            let filters = if paged {
                paged_filters()
            } else {
                ViewFilters::default()
            };
            let Arms {
                mut baseline,
                mut candidate,
            } = arms(&statement.view(), tenant, &filters, paged);
            let page = if paged {
                let (_, after, through) = pages(&client, tenant, statement.resource)
                    .await
                    .swap_remove(0);
                for prepared in [&mut baseline, &mut candidate] {
                    let n = prepared.params.len();
                    prepared.params[n - 2] = RuntimeParam::Text(after.clone());
                    prepared.params[n - 1] = RuntimeParam::Text(through.clone());
                }
                Some("first")
            } else {
                None
            };
            let mut measured = serde_json::Map::new();
            for (arm, prepared) in [("baseline", &baseline), ("candidate", &candidate)] {
                let mut exec_ms = Vec::new();
                let mut blocks = Vec::new();
                for run in 0..4 {
                    let rows = query(
                        &client,
                        &format!(
                            "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {}",
                            prepared.query.sql
                        ),
                        &prepared.params,
                    )
                    .await;
                    let plan: Value = rows[0].get(0);
                    if run == 0 {
                        continue;
                    }
                    exec_ms.push(plan[0]["Execution Time"].as_f64().unwrap_or_default());
                    let top = &plan[0]["Plan"];
                    blocks.push(
                        top["Shared Hit Blocks"].as_u64().unwrap_or_default()
                            + top["Shared Read Blocks"].as_u64().unwrap_or_default(),
                    );
                }
                let mut sorted_ms = exec_ms.clone();
                sorted_ms.sort_by(f64::total_cmp);
                blocks.sort_unstable();
                measured.insert(
                    arm.to_string(),
                    json!({"exec_ms": exec_ms, "exec_ms_median": sorted_ms[1], "shared_blocks_median": blocks[1]}),
                );
            }
            println!(
                "N5-COST {}",
                json!({"statement": name, "tenant": tenant, "paged": page, "measured": measured})
            );
        }
    }
}
