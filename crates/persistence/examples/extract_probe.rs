//! Search-parameter extraction probe.
//!
//! Times `SearchParameterExtractor::extract` per resource type over the
//! first N resources of each NDJSON file, prices the other halves of the
//! SQLite `prepare_index` step (contained resources, FTS content, SqlValue
//! marshal), measures rayon scaling of the whole prepare step, and can dump
//! every extracted value in a canonical sorted form so two builds of the
//! extractor can be diffed for byte-identical output.
//!
//! ```text
//! extract_probe --data-dir data --limit 5000 --threads 1,4,8 \
//!     Observation.ndjson Condition.ndjson Patient.ndjson Encounter.ndjson
//! extract_probe --data-dir data --limit 20000 --dump /tmp/values.txt FILES…
//! ```
//!
//! Uses only the extractor's public API, so the same probe builds against
//! any revision of the crate.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use helios_fhir::FhirVersion;
use helios_fhir::search::SearchParameterLoader;
use helios_persistence::backends::sqlite::search::fts::extract_searchable_content;
use helios_persistence::backends::sqlite::search::writer::SqliteSearchIndexWriter;
use helios_persistence::search::extractor::{
    ExtractedValue, SearchParameterExtractor, drop_incomplete_composites,
};
use helios_persistence::search::registry::SearchParameterRegistry;
use parking_lot::RwLock;
use serde_json::Value;

struct Args {
    dump: Option<PathBuf>,
    data_dir: PathBuf,
    limit: usize,
    rounds: usize,
    threads: Vec<usize>,
    files: Vec<PathBuf>,
}

fn parse_args() -> Args {
    let mut args = Args {
        dump: None,
        data_dir: PathBuf::from("data"),
        limit: 5000,
        rounds: 3,
        threads: vec![1, 4, 8],
        files: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--data-dir" => args.data_dir = PathBuf::from(it.next().expect("--data-dir")),
            "--dump" => args.dump = Some(PathBuf::from(it.next().expect("--dump"))),
            "--limit" => args.limit = it.next().expect("--limit").parse().expect("limit"),
            "--rounds" => args.rounds = it.next().expect("--rounds").parse().expect("rounds"),
            "--threads" => {
                args.threads = it
                    .next()
                    .expect("--threads")
                    .split(',')
                    .map(|s| s.parse().expect("thread count"))
                    .collect()
            }
            other => args.files.push(PathBuf::from(other)),
        }
    }
    args
}

fn build_registry(data_dir: &Path) -> Arc<RwLock<SearchParameterRegistry>> {
    let loader = SearchParameterLoader::new(FhirVersion::R4);
    let mut registry = SearchParameterRegistry::new();
    let mut n_emb = 0;
    let mut n_spec = 0;
    for p in loader.load_embedded().expect("embedded") {
        if registry.register(p).is_ok() {
            n_emb += 1;
        }
    }
    for p in loader.load_from_spec_file(data_dir).expect("spec file") {
        if registry.register(p).is_ok() {
            n_spec += 1;
        }
    }
    println!("registry: {n_emb} embedded + {n_spec} spec params");
    Arc::new(RwLock::new(registry))
}

fn read_resources(path: &Path, limit: usize) -> Vec<Value> {
    let f = std::fs::File::open(path).expect("open ndjson");
    let r = BufReader::with_capacity(1 << 20, f);
    let mut out = Vec::with_capacity(limit);
    for line in r.lines() {
        let line = line.expect("line");
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line).expect("json"));
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn marshal(values: Vec<ExtractedValue>) -> usize {
    let values = drop_incomplete_composites(values);
    let mut n = 0;
    for v in values {
        let row = SqliteSearchIndexWriter::to_sql_params("t", "T", "id", 0, &v);
        n += row.len();
    }
    n
}

fn prepare_like(ex: &SearchParameterExtractor, rt: &str, r: &Value) -> usize {
    let mut rows = 0;
    if let Ok(values) = ex.extract(r, rt) {
        rows += marshal(values);
    }
    for c in ex.extract_contained(r) {
        rows += marshal(c.values);
    }
    let fts = extract_searchable_content(r);
    rows += fts.full_content.len() % 2;
    rows
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// Best-of-`rounds` wall time of `f` over the resources, with the per-round
/// median alongside.
fn timed<F: FnMut()>(rounds: usize, mut f: F) -> (Duration, Duration) {
    let mut samples = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let t = Instant::now();
        f();
        samples.push(t.elapsed());
    }
    samples.sort();
    (samples[0], samples[samples.len() / 2])
}

fn main() {
    let args = parse_args();
    let registry = build_registry(&args.data_dir);
    let ex = SearchParameterExtractor::new(registry);

    let mut corpus: Vec<(String, Vec<Value>)> = Vec::new();
    for f in &args.files {
        let t = Instant::now();
        let rs = read_resources(f, args.limit);
        let rt = rs[0]["resourceType"].as_str().unwrap().to_string();
        println!(
            "loaded {:>6} {:<14} from {} in {:.2}s",
            rs.len(),
            rt,
            f.display(),
            t.elapsed().as_secs_f64()
        );
        corpus.push((rt, rs));
    }

    // `--dump FILE`: write every extracted row (incl. contained) per resource,
    // sorted, and exit. Diff two dumps to prove two extractor builds agree.
    if let Some(path) = &args.dump {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(path).expect("dump file"));
        let mut n = 0usize;
        for (rt, rs) in &corpus {
            for r in rs {
                let id = r["id"].as_str().unwrap_or("?");
                let mut lines: Vec<String> = Vec::new();
                for v in drop_incomplete_composites(ex.extract(r, rt).unwrap_or_default()) {
                    lines.push(format!(
                        "{} {:?} {:?} {:?} {:?}",
                        v.param_name, v.param_type, v.composite_group, v.composite_slot, v.value
                    ));
                }
                for c in ex.extract_contained(r) {
                    for v in drop_incomplete_composites(c.values) {
                        lines.push(format!(
                            "contained:{}/{} {} {:?} {:?} {:?} {:?}",
                            c.contained_type,
                            c.local_id,
                            v.param_name,
                            v.param_type,
                            v.composite_group,
                            v.composite_slot,
                            v.value
                        ));
                    }
                }
                lines.sort();
                n += lines.len();
                writeln!(w, "## {rt}/{id}").unwrap();
                for l in lines {
                    writeln!(w, "{l}").unwrap();
                }
            }
        }
        w.flush().unwrap();
        println!("dumped {n} rows to {}", path.display());
        return;
    }

    // Warm the prepared-expression caches.
    for (rt, rs) in &corpus {
        for r in rs.iter().take(100) {
            let _ = ex.extract(r, rt);
        }
    }

    let mut mix_extract = Duration::ZERO;
    let mut mix_prepare = Duration::ZERO;
    let mut mix_n = 0usize;

    println!();
    println!(
        "{:<14} {:>6} {:>9} {:>9} {:>8} {:>8} {:>8} {:>9} {:>9} {:>7}",
        "type",
        "n",
        "extract",
        "(median)",
        "contain",
        "fts",
        "marshal",
        "prepare",
        "(median)",
        "rows"
    );
    for (rt, rs) in &corpus {
        let n = rs.len();
        let nf = n as f64;

        // 1. plain extract (what the backends call), best of `rounds`.
        let mut rows = 0usize;
        let (t_extract, t_extract_med) = timed(args.rounds, || {
            rows = 0;
            for r in rs {
                rows += ex.extract(r, rt).map(|v| v.len()).unwrap_or(0);
            }
        });

        // 2. contained
        let (t_contained, _) = timed(args.rounds, || {
            for r in rs {
                for c in ex.extract_contained(r) {
                    std::hint::black_box(c.values.len());
                }
            }
        });

        // 3. fts content
        let (t_fts, _) = timed(args.rounds, || {
            for r in rs {
                let c = extract_searchable_content(r);
                std::hint::black_box(c.full_content.len() + c.narrative.len());
            }
        });

        // 4. marshal (extraction redone outside the clock)
        let extracted: Vec<Vec<ExtractedValue>> = rs
            .iter()
            .map(|r| ex.extract(r, rt).unwrap_or_default())
            .collect();
        let (t_marshal, _) = timed(args.rounds, || {
            for v in &extracted {
                std::hint::black_box(marshal(v.clone()));
            }
        });

        // 5. whole prepare_index equivalent, single thread
        let (t_prepare, t_prepare_med) = timed(args.rounds, || {
            for r in rs {
                std::hint::black_box(prepare_like(&ex, rt, r));
            }
        });

        println!(
            "{:<14} {:>6} {:>8.1}u {:>8.1}u {:>7.1}u {:>7.1}u {:>7.1}u {:>8.1}u {:>8.1}u {:>7.2}",
            rt,
            n,
            us(t_extract) / nf,
            us(t_extract_med) / nf,
            us(t_contained) / nf,
            us(t_fts) / nf,
            us(t_marshal) / nf,
            us(t_prepare) / nf,
            us(t_prepare_med) / nf,
            rows as f64 / nf,
        );
        mix_extract += t_extract;
        mix_prepare += t_prepare;
        mix_n += n;
    }

    println!();
    println!(
        "mix: {} resources, extract {:.3}s = {:.1} us/resource single-thread; prepare {:.3}s = {:.1} us/resource",
        mix_n,
        mix_extract.as_secs_f64(),
        us(mix_extract) / mix_n as f64,
        mix_prepare.as_secs_f64(),
        us(mix_prepare) / mix_n as f64
    );

    // Parallel scaling of the whole prepare step over the mix.
    println!();
    println!("rayon scaling of prepare (extract+contained+fts+marshal) over the mix:");
    let items: Vec<(&str, &Value)> = corpus
        .iter()
        .flat_map(|(rt, rs)| rs.iter().map(move |r| (rt.as_str(), r)))
        .collect();
    for &w in &args.threads {
        use rayon::prelude::*;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(w)
            .build()
            .unwrap();
        let (best, med) = timed(args.rounds, || {
            let n: usize = pool.install(|| {
                items
                    .par_iter()
                    .map(|(rt, r)| prepare_like(&ex, rt, r))
                    .sum()
            });
            std::hint::black_box(n);
        });
        println!(
            "   threads={:>2}  best {:.3}s  median {:.3}s  {:>7.0} res/s  {:>6.1} us/res/core",
            w,
            best.as_secs_f64(),
            med.as_secs_f64(),
            items.len() as f64 / best.as_secs_f64(),
            best.as_secs_f64() * 1e6 * w as f64 / items.len() as f64
        );
    }
}
