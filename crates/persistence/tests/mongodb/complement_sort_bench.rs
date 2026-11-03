//! #2285 measurement harness: `:not` / `:missing=true` complements and
//! unfiltered `_sort` by a search parameter on a large type. Reports p50 and
//! p95 latency per shape and the process peak resident set (`VmHWM`), so the
//! same data can be measured by a baseline build and a candidate build.
//!
//! Seeds once into a fixed database and reuses it. The peak RSS is the one of
//! this test process, which runs the backend in-process; it is reset before
//! each shape when the kernel allows it.
//!
//! ```text
//! HFS_TEST_COMPLEMENT_BENCH_MONGODB_URL=mongodb://127.0.0.1:27117/?directConnection=true \
//! HFS_TEST_COMPLEMENT_BENCH_ROWS=500000 \
//!   cargo test -p helios-persistence --features mongodb --test mongodb_tests \
//!   complement_sort_bench -- --ignored --nocapture
//! ```

use super::*;
use futures::StreamExt;
use std::time::Instant;

const DATABASE: &str = "hfs_bench_2285";

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// `status` is `final` for 3 in 4, so `status:not=final` is a small
/// complement and `status:not=amended` (nobody) is the whole type. `method`
/// is recorded for 1 in 2, and `effective` carries a value for all but 1 in
/// 10, so `_sort=date` has a keyless tail. 1 in 20 belongs to `Patient/p-0`.
fn observation(number: usize) -> serde_json::Value {
    let status = if number % 4 == 3 {
        "preliminary"
    } else {
        "final"
    };
    let day = number % 365;
    let mut resource = json!({
        "resourceType": "Observation", "id": format!("bench-{number:08}"),
        "status": status,
        "code": {"coding": [{"system": "urn:bench", "code": "base"}]},
        "subject": {"reference": format!("Patient/p-{}", number % 20)},
    });
    if number.is_multiple_of(2) {
        resource["method"] = json!({"coding": [{"system": "urn:bench", "code": "m"}]});
    }
    if number % 10 != 9 {
        resource["effectiveDateTime"] = json!(format!(
            "2020-{:02}-{:02}T00:00:00Z",
            day / 31 % 12 + 1,
            day % 28 + 1
        ));
    }
    resource
}

fn token_with(name: &str, modifier: SearchModifier, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.to_owned(),
        param_type: SearchParamType::Token,
        modifier: Some(modifier),
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    }
}

async fn seed(backend: &MongoBackend, tenant: &TenantContext, rows: usize) {
    let db = backend.get_database().await.unwrap();
    let existing = db
        .collection::<Document>("resources")
        .count_documents(doc! {
            "tenant_id": tenant.tenant_id().as_str(), "resource_type": "Observation",
        })
        .await
        .unwrap() as usize;
    if existing >= rows {
        eprintln!("reusing {existing} seeded Observations");
        return;
    }
    let started = Instant::now();
    futures::stream::iter(existing..rows)
        .map(|number| async move {
            backend
                .create(
                    tenant,
                    "Observation",
                    observation(number),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
            if number % 100_000 == 0 {
                eprintln!("seeded {number} after {:?}", started.elapsed());
            }
        })
        .buffer_unordered(env_usize("HFS_TEST_COMPLEMENT_BENCH_SEED_CONCURRENCY", 64))
        .collect::<Vec<_>>()
        .await;
    eprintln!("seeded {rows} in {:?}", started.elapsed());
}

/// Peak resident set of this process in kB, from `/proc/self/status`.
fn peak_rss_kb() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
}

/// Resets the kernel's peak-RSS watermark; a no-op where that is not allowed.
fn reset_peak_rss() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let index = ((sorted.len() as f64 * fraction).ceil() as usize).clamp(1, sorted.len());
    sorted[index - 1]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "#2285 measurement harness; run with --ignored"]
async fn complement_sort_bench() {
    let Ok(url) = std::env::var("HFS_TEST_COMPLEMENT_BENCH_MONGODB_URL") else {
        eprintln!("skip: HFS_TEST_COMPLEMENT_BENCH_MONGODB_URL is not set");
        return;
    };
    let rows = env_usize("HFS_TEST_COMPLEMENT_BENCH_ROWS", 500_000);
    let runs = env_usize("HFS_TEST_COMPLEMENT_BENCH_RUNS", 10);
    let deep_offset = env_usize("HFS_TEST_COMPLEMENT_BENCH_DEEP_OFFSET", rows / 2);
    let filter = std::env::var("HFS_TEST_COMPLEMENT_BENCH_FILTER").unwrap_or_default();
    let backend = build_backend(MongoBackendConfig {
        connection_string: url,
        database_name: DATABASE.to_owned(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    })
    .await
    .expect("benchmark MongoDB");
    let tenant = create_tenant("bench-2285");
    seed(&backend, &tenant, rows).await;

    let page = |parameters: Vec<SearchParameter>| {
        let mut query = SearchQuery::new("Observation").with_count(20);
        for parameter in parameters {
            query = query.with_parameter(parameter);
        }
        query.total = Some(TotalMode::None);
        query
    };
    let mut shapes: Vec<(&str, SearchQuery)> = vec![
        (
            "status:not=final (small complement)",
            page(vec![token_with("status", SearchModifier::Not, "final")]),
        ),
        (
            "status:not=amended (whole type)",
            page(vec![token_with("status", SearchModifier::Not, "amended")]),
        ),
        (
            "method:missing=true",
            page(vec![token_with("method", SearchModifier::Missing, "true")]),
        ),
    ];
    let mut by_date = page(vec![]).with_sort(SortDirective::parse("date"));
    shapes.push(("_sort=date page 1", by_date.clone()));
    by_date.offset = Some(deep_offset as u32);
    shapes.push(("_sort=date deep offset", by_date));
    // Typed like the server resolves it, so the sort finds the date rows and
    // only the tenth of the type without a date falls in the keyless tail.
    let mut typed = SortDirective::parse("date");
    typed.param_type = Some(SearchParamType::Date);
    let mut by_typed_date = page(vec![]).with_sort(typed);
    shapes.push(("_sort=date typed page 1", by_typed_date.clone()));
    by_typed_date.offset = Some(deep_offset as u32);
    shapes.push(("_sort=date typed deep offset", by_typed_date));
    let mut compartment = page(vec![token_with("status", SearchModifier::Not, "final")]);
    compartment.compartment = Some(helios_persistence::types::CompartmentMembership {
        params: vec!["subject".into()],
        reference: "Patient/p-0".into(),
    });
    shapes.push(("compartment + status:not=final", compartment));

    println!("| shape | outcome | p50 ms | p95 ms | peak RSS MB |");
    println!("|---|---|---:|---:|---:|");
    for (label, query) in shapes {
        if !filter.is_empty() && !label.contains(&filter) {
            continue;
        }
        reset_peak_rss();
        let mut timings = Vec::with_capacity(runs);
        let mut outcome = String::new();
        // One untimed warm-up, then `runs` timed executions.
        for run in 0..=runs {
            let started = Instant::now();
            let observed = backend.search(&tenant, &query).await;
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            outcome = match observed {
                Ok(result) => format!("page {}", result.resources.items.len()),
                Err(error) => format!("error: {error}"),
            };
            if run > 0 {
                timings.push(elapsed);
            }
        }
        timings.sort_by(f64::total_cmp);
        println!(
            "| {label} | {outcome} | {:.1} | {:.1} | {} |",
            percentile(&timings, 0.50),
            percentile(&timings, 0.95),
            peak_rss_kb().map_or_else(|| "n/a".into(), |kb| format!("{:.0}", kb as f64 / 1024.0)),
        );
    }
}
