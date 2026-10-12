//! #1813 measurement gate: how much of a large Parquet `$sql-run` over a
//! ViewDefinition is spent turning database values into `serde_json::Value`
//! rows and copying them around before they reach Arrow?
//!
//! Each stage of today's path is timed on its own, with the peak RSS reached
//! during that stage:
//!
//! 1. `run_view` drained into `Vec<Value>` (SQL + driver + JSON row build)
//! 2. `rows_to_processed_result` (copy into `ProcessedRow`s)
//! 3. `create_arrow_schema` + `process_to_arrow_arrays` (Arrow arrays)
//! 4. Parquet write (`format_output` total minus stage 3)
//!
//! A "driver floor" pass runs a hand-written equivalent `SELECT` and only
//! touches each cell, so stage 1 minus the floor approximates the JSON row
//! build. The floor is an approximation: it does not run the compiled SQL.
//!
//! Run with (release is right here, this is a measurement, not a check):
//!
//! ```text
//! cargo test --release -p helios-persistence --features postgres \
//!   --test sof_columnar_profile -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Run from the repository root so the search-parameter registry loads.
//! Postgres needs Docker; if the container cannot start the test skips.
//!
//! Knobs (defaults in parentheses):
//! - `HFS_TEST_COLUMNAR_PROFILE_ROWS` (200000): resources per view
//! - `HFS_TEST_COLUMNAR_PROFILE_ROUNDS` (3): rounds; compare medians
//! - `HFS_TEST_COLUMNAR_PROFILE_DIR` (`$HOME/tmp/hfs-columnar-profile`): SQLite
//!   file location. Never `/tmp`: it is a RAM disk on the dev box.
//!
//! Output lines start with `COLUMNAR_PROFILE` so they can be grepped.
//! Gate: if stages 1-3 minus the driver floor are under ~25% of the total and
//! do not dominate peak RSS, stop and post the numbers on the issue.

use std::time::{Duration, Instant};

use futures::StreamExt;
use helios_persistence::core::sof_runner::{SofRunner, ViewFilters};
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};
use serde_json::{Value, json};

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_usize(name: &str, default: usize) -> usize {
    env_or(name, &default.to_string())
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be an unsigned integer"))
}

/// One synthetic workload: how to build resource `i`, the ViewDefinition over
/// it, and the equivalent plain `SELECT` expressions for the driver floor.
struct Workload {
    name: &'static str,
    resource_type: &'static str,
    view: Value,
    sqlite_exprs: &'static [&'static str],
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    pg_exprs: &'static [&'static str],
    make: fn(usize) -> Value,
}

fn workloads() -> Vec<Workload> {
    vec![
        Workload {
            name: "patient",
            resource_type: "Patient",
            view: json!({
                "resourceType": "ViewDefinition",
                "name": "profile_patient",
                "resource": "Patient",
                "status": "active",
                "select": [{"column": [
                    {"name": "id", "path": "id", "type": "id"},
                    {"name": "gender", "path": "gender", "type": "code"},
                    {"name": "birth_date", "path": "birthDate", "type": "date"},
                    {"name": "active", "path": "active", "type": "boolean"},
                    {"name": "family", "path": "name.first().family", "type": "string"},
                    {"name": "given", "path": "name.first().given.first()", "type": "string"}
                ]}]
            }),
            sqlite_exprs: &[
                "json_extract(CAST(data AS TEXT),'$.id')",
                "json_extract(CAST(data AS TEXT),'$.gender')",
                "json_extract(CAST(data AS TEXT),'$.birthDate')",
                "json_extract(CAST(data AS TEXT),'$.active')",
                "json_extract(CAST(data AS TEXT),'$.name[0].family')",
                "json_extract(CAST(data AS TEXT),'$.name[0].given[0]')",
            ],
            pg_exprs: &[
                "data->>'id'",
                "data->>'gender'",
                "data->>'birthDate'",
                "data->>'active'",
                "data#>>'{name,0,family}'",
                "data#>>'{name,0,given,0}'",
            ],
            make: |i| {
                json!({
                    "resourceType": "Patient",
                    "id": format!("p{i}"),
                    "active": i % 7 != 0,
                    "gender": if i % 2 == 0 { "female" } else { "male" },
                    "birthDate": format!("{}-{:02}-{:02}", 1930 + i % 80, 1 + i % 12, 1 + i % 28),
                    "name": [{"use": "official", "family": format!("Family-{i}"),
                              "given": [format!("Given-{i}"), "Middle"]}]
                })
            },
        },
        Workload {
            name: "observation",
            resource_type: "Observation",
            view: json!({
                "resourceType": "ViewDefinition",
                "name": "profile_observation",
                "resource": "Observation",
                "status": "active",
                "select": [{"column": [
                    {"name": "id", "path": "id", "type": "id"},
                    {"name": "status", "path": "status", "type": "code"},
                    {"name": "code", "path": "code.coding.first().code", "type": "code"},
                    {"name": "subject", "path": "subject.reference", "type": "string"},
                    {"name": "value", "path": "value.ofType(Quantity).value", "type": "decimal"},
                    {"name": "unit", "path": "value.ofType(Quantity).unit", "type": "string"},
                    {"name": "seq", "path": "extension.first().value.ofType(integer)", "type": "integer"}
                ]}]
            }),
            sqlite_exprs: &[
                "json_extract(CAST(data AS TEXT),'$.id')",
                "json_extract(CAST(data AS TEXT),'$.status')",
                "json_extract(CAST(data AS TEXT),'$.code.coding[0].code')",
                "json_extract(CAST(data AS TEXT),'$.subject.reference')",
                "json_extract(CAST(data AS TEXT),'$.valueQuantity.value')",
                "json_extract(CAST(data AS TEXT),'$.valueQuantity.unit')",
                "json_extract(CAST(data AS TEXT),'$.extension[0].valueInteger')",
            ],
            pg_exprs: &[
                "data->>'id'",
                "data->>'status'",
                "data#>>'{code,coding,0,code}'",
                "data#>>'{subject,reference}'",
                "data#>>'{valueQuantity,value}'",
                "data#>>'{valueQuantity,unit}'",
                "data#>>'{extension,0,valueInteger}'",
            ],
            make: |i| {
                json!({
                    "resourceType": "Observation",
                    "id": format!("o{i}"),
                    "status": "final",
                    "code": {"coding": [{"system": "http://loinc.org", "code": format!("{}-{}", 1000 + i % 500, i % 10)}]},
                    "subject": {"reference": format!("Patient/p{}", i % 1000)},
                    "valueQuantity": {"value": (i % 1000) as f64 / 10.0 + 0.5, "unit": "mg"},
                    "extension": [{"url": "http://example.org/seq", "valueInteger": i as i64}]
                })
            },
        },
    ]
}

fn tenant(id: &str) -> TenantContext {
    TenantContext::new(TenantId::new(id), TenantPermissions::full_access())
}

// --- peak RSS ---------------------------------------------------------------

/// Reset the kernel's peak-RSS watermark (`VmHWM`) to the current RSS.
/// Returns false when `/proc` is not writable, in which case the peaks are
/// cumulative over the process.
fn reset_peak_rss() -> bool {
    std::fs::write("/proc/self/clear_refs", "5").is_ok()
}

fn status_kb(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(0)
}

fn mb(kb: u64) -> f64 {
    kb as f64 / 1024.0
}

// --- stages -------------------------------------------------------------------

#[derive(Default, Clone)]
struct Round {
    floor: Duration,
    runner_rows: Duration,
    to_processed: Duration,
    arrow: Duration,
    format_total: Duration,
    rss_floor_mb: f64,
    rss_runner_mb: f64,
    rss_processed_mb: f64,
    rss_arrow_mb: f64,
    rss_format_mb: f64,
    json_bytes: usize,
    parquet_bytes: usize,
    rows: usize,
}

/// Times stages 1-4 over one runner. `floor` is run by the caller, since it
/// is backend specific.
async fn run_stages(
    runner: &dyn SofRunner,
    tenant: &TenantContext,
    view: &Value,
    mut round: Round,
) -> Round {
    // Stage 1: run_view drained to Vec<Value>.
    reset_peak_rss();
    let t = Instant::now();
    let mut stream = runner
        .run_view(tenant, view.clone(), ViewFilters::default())
        .await
        .expect("run_view");
    let mut rows: Vec<Value> = Vec::new();
    while let Some(row) = stream.next().await {
        rows.push(row.expect("row"));
    }
    drop(stream);
    round.runner_rows = t.elapsed();
    round.rss_runner_mb = mb(status_kb("VmHWM:"));
    round.rows = rows.len();
    round.json_bytes = rows.iter().map(|r| r.to_string().len()).sum();

    // Stage 2: copy into ProcessedRows (consumes the JSON rows).
    reset_peak_rss();
    let t = Instant::now();
    let result = helios_sof::rows_to_processed_result(rows);
    round.to_processed = t.elapsed();
    round.rss_processed_mb = mb(status_kb("VmHWM:"));

    // Stage 3: schema + Arrow arrays, as one batch.
    reset_peak_rss();
    let t = Instant::now();
    let schema = helios_sof::parquet_schema::create_arrow_schema(&result.columns, &result.rows)
        .expect("schema");
    let arrays =
        helios_sof::parquet_schema::process_to_arrow_arrays(&schema, &result.columns, &result.rows)
            .expect("arrays");
    round.arrow = t.elapsed();
    round.rss_arrow_mb = mb(status_kb("VmHWM:"));
    drop(arrays);

    // Stage 4: whole format_output (includes stage 3 again; the write is the
    // difference).
    reset_peak_rss();
    let t = Instant::now();
    let bytes = helios_sof::format_output(result, helios_sof::ContentType::Parquet, None)
        .expect("format_output");
    round.format_total = t.elapsed();
    round.rss_format_mb = mb(status_kb("VmHWM:"));
    round.parquet_bytes = bytes.len();
    round
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn report(backend: &str, workload: &str, rounds: &[Round]) {
    let ms = |f: fn(&Round) -> Duration| {
        median(rounds.iter().map(|r| f(r).as_secs_f64() * 1e3).collect())
    };
    let mbf = |f: fn(&Round) -> f64| median(rounds.iter().map(f).collect());
    let floor = ms(|r| r.floor);
    let runner = ms(|r| r.runner_rows);
    let processed = ms(|r| r.to_processed);
    let arrow = ms(|r| r.arrow);
    let format_total = ms(|r| r.format_total);
    let write = (format_total - arrow).max(0.0);
    let json_step = (runner - floor).max(0.0);
    // Stages that disappear or shrink with direct-to-Arrow appends: the JSON
    // row build, the ProcessedRow copy and the Value-to-array pass.
    let removable = json_step + processed + arrow;
    let total = runner + processed + format_total;
    let r0 = &rounds[0];
    println!(
        "COLUMNAR_PROFILE backend={backend} view={workload} rows={} rounds={} \
         json_bytes={} parquet_bytes={}",
        r0.rows,
        rounds.len(),
        r0.json_bytes,
        r0.parquet_bytes
    );
    println!(
        "COLUMNAR_PROFILE backend={backend} view={workload} ms \
         driver_floor={floor:.0} runner_to_json={runner:.0} (json_step~{json_step:.0}) \
         to_processed={processed:.0} arrow_arrays={arrow:.0} parquet_write={write:.0} \
         total={total:.0}"
    );
    println!(
        "COLUMNAR_PROFILE backend={backend} view={workload} peak_rss_mb \
         floor={:.0} runner={:.0} processed={:.0} arrow={:.0} format={:.0}",
        mbf(|r| r.rss_floor_mb),
        mbf(|r| r.rss_runner_mb),
        mbf(|r| r.rss_processed_mb),
        mbf(|r| r.rss_arrow_mb),
        mbf(|r| r.rss_format_mb),
    );
    println!(
        "COLUMNAR_PROFILE backend={backend} view={workload} removable_share={:.1}% \
         (json_step+to_processed+arrow_arrays over total; gate ~25%)",
        100.0 * removable / total.max(f64::EPSILON)
    );
}

// --- SQLite ---------------------------------------------------------------------

#[cfg(all(feature = "sqlite", feature = "R4"))]
mod sqlite {
    use super::*;
    use helios_persistence::backends::sqlite::SqliteBackend;
    use helios_persistence::core::ResourceStorage;

    const TENANT: &str = "profile";

    fn seed(path: &std::path::Path, w: &Workload, n: usize) {
        let backend = SqliteBackend::with_config(path, Default::default()).expect("backend");
        backend.init_schema().expect("schema");
        drop(backend);
        let mut conn = rusqlite::Connection::open(path).expect("open");
        let tx = conn.transaction().expect("tx");
        let at = chrono::Utc::now().to_rfc3339();
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO resources (tenant_id,resource_type,id,version_id,data,\
                     last_updated,is_deleted) VALUES (?1,?2,?3,'1',?4,?5,0)",
                )
                .expect("prepare");
            for i in 0..n {
                let data = (w.make)(i);
                stmt.execute(rusqlite::params![
                    TENANT,
                    w.resource_type,
                    data["id"].as_str().unwrap(),
                    serde_json::to_vec(&data).unwrap(),
                    at
                ])
                .expect("insert");
            }
        }
        tx.commit().expect("commit");
    }

    fn floor(path: &std::path::Path, w: &Workload) -> (Duration, f64) {
        let conn = rusqlite::Connection::open(path).expect("open");
        let sql = format!(
            "SELECT {} FROM resources WHERE tenant_id=?1 AND resource_type=?2 AND is_deleted=0",
            w.sqlite_exprs.join(",")
        );
        reset_peak_rss();
        let t = Instant::now();
        let mut stmt = conn.prepare(&sql).expect("prepare floor");
        let mut rows = stmt
            .query(rusqlite::params![TENANT, w.resource_type])
            .expect("query");
        let mut cells = 0usize;
        while let Some(row) = rows.next().expect("row") {
            for i in 0..w.sqlite_exprs.len() {
                // Touch the value the way the driver hands it over.
                if !matches!(row.get_ref(i).unwrap(), rusqlite::types::ValueRef::Null) {
                    cells += 1;
                }
            }
        }
        std::hint::black_box(cells);
        (t.elapsed(), mb(status_kb("VmHWM:")))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "#1813 measurement harness; run with --ignored"]
    async fn sqlite_columnar_profile() {
        let n = env_usize("HFS_TEST_COLUMNAR_PROFILE_ROWS", 200_000);
        let rounds_n = env_usize("HFS_TEST_COLUMNAR_PROFILE_ROUNDS", 3);
        let default_dir = format!(
            "{}/tmp/hfs-columnar-profile",
            std::env::var("HOME").expect("HOME")
        );
        let dir = std::path::PathBuf::from(env_or("HFS_TEST_COLUMNAR_PROFILE_DIR", &default_dir));
        std::fs::create_dir_all(&dir).expect("create dir");
        let dir = tempfile::tempdir_in(&dir).expect("tempdir");

        for w in workloads() {
            let path = dir.path().join(format!("{}.db", w.name));
            seed(&path, &w, n);
            let backend = SqliteBackend::with_config(&path, Default::default()).expect("backend");
            let runner = backend.sof_runner().expect("runner");
            let tenant = tenant(TENANT);
            let mut rounds = Vec::new();
            for _ in 0..rounds_n {
                let (floor_t, floor_rss) = floor(&path, &w);
                let round = Round {
                    floor: floor_t,
                    rss_floor_mb: floor_rss,
                    ..Default::default()
                };
                rounds.push(run_stages(runner.as_ref(), &tenant, &w.view, round).await);
            }
            assert_eq!(rounds[0].rows, n, "view should return one row per resource");
            report("sqlite", w.name, &rounds);
        }
        // `dir` is a TempDir and removes the database files on drop.
    }
}

// --- PostgreSQL -------------------------------------------------------------------

#[cfg(all(feature = "postgres", feature = "R4"))]
mod postgres {
    use super::*;
    use helios_persistence::backends::postgres::{PostgresBackend, PostgresConfig};
    use helios_persistence::core::ResourceStorage;
    use testcontainers::ImageExt;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    async fn seed(client: &tokio_postgres::Client, tenant_id: &str, w: &Workload, n: usize) {
        let at = chrono::Utc::now();
        for start in (0..n).step_by(10_000) {
            let end = (start + 10_000).min(n);
            let data: Vec<Value> = (start..end).map(w.make).collect();
            let ids: Vec<String> = data
                .iter()
                .map(|d| d["id"].as_str().unwrap().to_string())
                .collect();
            client
                .execute(
                    "INSERT INTO resources (tenant_id,resource_type,id,version_id,data,\
                     last_updated,is_deleted) \
                     SELECT $1,$2,i,'1',d,$3,false FROM unnest($4::text[],$5::jsonb[]) u(i,d)",
                    &[&tenant_id, &w.resource_type, &at, &ids, &data],
                )
                .await
                .expect("seed");
        }
    }

    async fn floor(
        client: &tokio_postgres::Client,
        tenant_id: &str,
        w: &Workload,
    ) -> (Duration, f64) {
        let sql = format!(
            "SELECT {} FROM resources WHERE tenant_id=$1 AND resource_type=$2 AND NOT is_deleted",
            w.pg_exprs.join(",")
        );
        reset_peak_rss();
        let t = Instant::now();
        let stmt = client.prepare(&sql).await.expect("prepare floor");
        let rows = client
            .query_raw(
                &stmt,
                [
                    &tenant_id as &(dyn tokio_postgres::types::ToSql + Sync),
                    &w.resource_type,
                ],
            )
            .await
            .expect("query");
        futures::pin_mut!(rows);
        let mut cells = 0usize;
        while let Some(row) = rows.next().await {
            let row = row.expect("row");
            for i in 0..w.pg_exprs.len() {
                if row.get::<_, Option<&str>>(i).is_some() {
                    cells += 1;
                }
            }
        }
        std::hint::black_box(cells);
        (t.elapsed(), mb(status_kb("VmHWM:")))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "#1813 measurement harness; run with --ignored (needs Docker)"]
    async fn postgres_columnar_profile() {
        let n = env_usize("HFS_TEST_COLUMNAR_PROFILE_ROWS", 200_000);
        let rounds_n = env_usize("HFS_TEST_COLUMNAR_PROFILE_ROUNDS", 3);

        let container = match Postgres::default().with_tag("16-alpine").start().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skip: cannot start PostgreSQL container: {e}");
                return;
            }
        };
        let host = container.get_host().await.expect("host").to_string();
        let port = container.get_host_port_ipv4(5432).await.expect("port");

        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("data"));
        let backend = PostgresBackend::new(PostgresConfig {
            host: host.clone(),
            port,
            dbname: "postgres".to_string(),
            user: "postgres".to_string(),
            password: Some("postgres".to_string()),
            max_connections: 5,
            data_dir,
            ..Default::default()
        })
        .await
        .expect("backend");
        backend.init_schema().await.expect("schema");

        let (client, conn) = tokio_postgres::connect(
            &format!("host={host} port={port} user=postgres password=postgres dbname=postgres"),
            tokio_postgres::NoTls,
        )
        .await
        .expect("connect");
        tokio::spawn(conn);

        let runner = backend.sof_runner().expect("runner");
        for w in workloads() {
            let tenant_id = format!("profile_{}", w.name);
            seed(&client, &tenant_id, &w, n).await;
            client
                .execute("ANALYZE resources", &[])
                .await
                .expect("analyze");
            let tenant = tenant(&tenant_id);
            let mut rounds = Vec::new();
            for _ in 0..rounds_n {
                let (floor_t, floor_rss) = floor(&client, &tenant_id, &w).await;
                let round = Round {
                    floor: floor_t,
                    rss_floor_mb: floor_rss,
                    ..Default::default()
                };
                rounds.push(run_stages(runner.as_ref(), &tenant, &w.view, round).await);
            }
            assert_eq!(rounds[0].rows, n, "view should return one row per resource");
            report("postgres", w.name, &rounds);
        }
        // `container` drops here and stops the PostgreSQL container.
    }
}
