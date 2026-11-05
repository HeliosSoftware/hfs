//! Throughput probe for FHIR JSON serialization / deserialization.
//!
//! Exists to quantify the runtime cost of routing the `FhirSerde` derive
//! through a type-erased `Deserializer` / `Serializer` (#510): the generated
//! bodies are compiled once instead of once per `(FHIR type, Deserializer)`
//! pair, at the price of virtual dispatch on every field.
//!
//! Run the same command on both sides of the change and compare:
//!
//! ```text
//! cargo run --release -p helios-serde --example serde_throughput -- [iterations]
//! ```
//!
//! Reads the R4 spec examples under `crates/fhir/tests/data/json/R4`, keeps the
//! ones that round-trip, and reports MB/s for parse and for write.

use helios_fhir::r4::Resource;
use std::time::Instant;

fn main() {
    let iterations: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(3);

    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../fhir/tests/data/json/R4");
    let mut corpus: Vec<(String, String)> = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {dir}: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    entries.sort();

    for path in entries {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Only keep documents the typed model actually accepts, so the timing
        // measures successful work rather than error paths.
        if serde_json::from_str::<Resource>(&text).is_ok() {
            corpus.push((path.file_name().unwrap().to_string_lossy().into(), text));
        }
    }

    let total_bytes: usize = corpus.iter().map(|(_, t)| t.len()).sum();
    println!(
        "corpus: {} resources, {:.2} MB, {} iterations",
        corpus.len(),
        total_bytes as f64 / 1e6,
        iterations
    );

    // --- deserialize ---
    let mut parsed: Vec<Resource> = Vec::new();
    let mut best_de = f64::MAX;
    for i in 0..iterations {
        let start = Instant::now();
        let round: Vec<Resource> = corpus
            .iter()
            .map(|(_, t)| serde_json::from_str::<Resource>(t).unwrap())
            .collect();
        let secs = start.elapsed().as_secs_f64();
        best_de = best_de.min(secs);
        if i == 0 {
            parsed = round;
        } else {
            std::hint::black_box(&round);
        }
    }

    // --- serialize ---
    let mut best_ser = f64::MAX;
    let mut out_bytes = 0usize;
    for _ in 0..iterations {
        let start = Instant::now();
        let mut n = 0usize;
        for r in &parsed {
            n += serde_json::to_string(r).unwrap().len();
        }
        let secs = start.elapsed().as_secs_f64();
        best_ser = best_ser.min(secs);
        out_bytes = n;
    }

    println!(
        "deserialize: {:.3} s  {:.1} MB/s",
        best_de,
        total_bytes as f64 / 1e6 / best_de
    );
    println!(
        "serialize:   {:.3} s  {:.1} MB/s",
        best_ser,
        out_bytes as f64 / 1e6 / best_ser
    );
}
