//! #2004 measurement harness: positive filters of varied selectivity combined
//! with metadata-backed `:missing=true`, at a volume where whole-type plans
//! show. Seeds once into a fixed database and reuses it, so the same data can
//! be measured by different builds.
//!
//! ```text
//! HFS_TEST_MISSING_BENCH_MONGODB_URL=mongodb://127.0.0.1:27117/?directConnection=true \
//! HFS_TEST_MISSING_BENCH_ROWS=1000000 \
//!   cargo test -p helios-persistence --features mongodb --test mongodb_tests \
//!   missing_positive_bench -- --ignored --nocapture
//! ```

use super::*;
use futures::StreamExt;
use std::time::Instant;

const DATABASE: &str = "hfs_bench_2004";

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// Every resource has a `code`; `method` is recorded for 1 in 100 and
/// `specimen` is absent for 1 in 100, so `method:missing=true` is a broad
/// absence and `specimen:missing=true` a sparse one.
fn observation(number: usize) -> serde_json::Value {
    let code = match number % 1000 {
        0 => "rare",         // 0.1%
        1..=50 => "mid",     // 5%
        51..=550 => "broad", // 50%
        _ => "rest",
    };
    let day = number % 365;
    let mut resource = json!({
        "resourceType": "Observation", "id": format!("bench-{number:08}"),
        "status": "final",
        "code": {"coding": [{"system": "urn:bench", "code": code}]},
        "effectiveDateTime": format!(
            "2020-{:02}-{:02}T00:00:00Z", day / 31 % 12 + 1, day % 28 + 1
        ),
    });
    if number % 100 == 7 {
        resource["method"] = json!({"coding": [{"system": "urn:bench", "code": "m"}]});
    }
    if number % 100 != 3 {
        resource["specimen"] = json!({"reference": format!("Specimen/s-{}", number % 50)});
    }
    resource
}

fn token(name: &str, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.to_owned(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq(value)],
        chain: vec![],
        components: vec![],
    }
}

fn missing(name: &str, param_type: SearchParamType) -> SearchParameter {
    SearchParameter {
        name: name.to_owned(),
        param_type,
        modifier: Some(SearchModifier::Missing),
        values: vec![SearchValue::eq("true")],
        chain: vec![],
        components: vec![],
    }
}

/// One week of 2020, as two ANDed bounds (values within one parameter are ORed).
fn date_range() -> Vec<SearchParameter> {
    ["ge2020-03-01", "lt2020-03-08"]
        .into_iter()
        .map(|value| SearchParameter {
            name: "date".to_owned(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::parse(value)],
            chain: vec![],
            components: vec![],
        })
        .collect()
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
    let concurrency = env_usize("HFS_TEST_MISSING_BENCH_SEED_CONCURRENCY", 64);
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
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;
    eprintln!("seeded {rows} in {:?}", started.elapsed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "#2004 measurement harness; run with --ignored"]
async fn missing_positive_bench() {
    let Ok(url) = std::env::var("HFS_TEST_MISSING_BENCH_MONGODB_URL") else {
        eprintln!("skip: HFS_TEST_MISSING_BENCH_MONGODB_URL is not set");
        return;
    };
    let rows = env_usize("HFS_TEST_MISSING_BENCH_ROWS", 1_000_000);
    let runs = env_usize("HFS_TEST_MISSING_BENCH_RUNS", 5);
    let filter = std::env::var("HFS_TEST_MISSING_BENCH_FILTER").unwrap_or_default();
    let backend = build_backend(MongoBackendConfig {
        connection_string: url,
        database_name: DATABASE.to_owned(),
        data_dir: Some(repo_data_dir()),
        ..Default::default()
    })
    .await
    .expect("benchmark MongoDB");
    let tenant = create_tenant("bench-2004");
    seed(&backend, &tenant, rows).await;

    let positives = [
        ("rare", vec![token("code", "rare")]),
        ("mid", vec![token("code", "mid")]),
        ("broad", vec![token("code", "broad")]),
        ("date-week", date_range()),
    ];
    let absences = [
        ("method", missing("method", SearchParamType::Token)),
        ("specimen", missing("specimen", SearchParamType::Reference)),
    ];
    println!("| positive | missing | shape | matches | median ms | min ms | max ms |");
    println!("|---|---|---|---:|---:|---:|---:|");
    for (positive_name, positive) in &positives {
        for (missing_name, absence) in &absences {
            let mut base = SearchQuery::new("Observation");
            for parameter in positive {
                base = base.with_parameter(parameter.clone());
            }
            let base = base.with_parameter(absence.clone());
            let mut shapes: Vec<(&str, SearchQuery)> = Vec::new();
            shapes.push(("count", base.clone()));
            let mut summary = base.clone();
            summary.summary = Some(helios_persistence::types::SummaryMode::Count);
            summary.total = Some(TotalMode::Accurate);
            shapes.push(("_summary=count", summary));
            let mut accurate = base.clone().with_count(20);
            accurate.total = Some(TotalMode::Accurate);
            shapes.push(("_total=accurate", accurate));
            let mut updated = base
                .clone()
                .with_count(20)
                .with_sort(SortDirective::parse("_lastUpdated"));
            updated.total = Some(TotalMode::None);
            shapes.push(("_sort=_lastUpdated", updated));
            let mut id_desc = base
                .clone()
                .with_count(20)
                .with_sort(SortDirective::parse("-_id"));
            id_desc.total = Some(TotalMode::None);
            shapes.push(("_sort=-_id", id_desc));
            for (shape, query) in shapes {
                let label = format!("{positive_name}/{missing_name}/{shape}");
                if !filter.is_empty() && !label.contains(&filter) {
                    continue;
                }
                let mut timings = Vec::with_capacity(runs);
                let mut matches = String::new();
                // One untimed warm-up, then `runs` timed executions.
                for run in 0..=runs {
                    let started = Instant::now();
                    let observed = if shape == "count" {
                        backend
                            .search_count(&tenant, &query)
                            .await
                            .map(|count| count.to_string())
                    } else {
                        backend
                            .search(&tenant, &query)
                            .await
                            .map(|result| match result.total {
                                Some(total) => total.to_string(),
                                None => format!("page {}", result.resources.items.len()),
                            })
                    };
                    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                    matches = match observed {
                        Ok(observed) => observed,
                        Err(error) => format!("error: {error}"),
                    };
                    if run > 0 {
                        timings.push(elapsed);
                    }
                }
                timings.sort_by(f64::total_cmp);
                println!(
                    "| {positive_name} | {missing_name} | {shape} | {matches} | {:.1} | {:.1} | {:.1} |",
                    timings[timings.len() / 2],
                    timings[0],
                    timings[timings.len() - 1],
                );
            }
        }
    }
}
