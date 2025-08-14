use criterion::{Criterion, black_box, criterion_group, criterion_main};
use serde_json::json;

fn bench_request_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("server/parsing");

    group.bench_function("simple_parameters", |b| {
        let json = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "expression",
                    "valueString": "Patient.name.family"
                },
                {
                    "name": "resource",
                    "resource": {
                        "resourceType": "Patient",
                        "name": [{
                            "family": "Chalmers"
                        }]
                    }
                }
            ]
        });

        b.iter(|| {
            let value = black_box(&json);
            // Simulate parsing by accessing fields
            let _ = value["resourceType"].as_str();
            let _ = value["parameter"][0]["name"].as_str();
            let _ = value["parameter"][0]["valueString"].as_str();
            let _ = value["parameter"][1]["resource"]["resourceType"].as_str();
        })
    });

    group.bench_function("complex_parameters", |b| {
        let json = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "context",
                    "valueString": "Patient.name"
                },
                {
                    "name": "expression",
                    "valueString": "given.first() + ' ' + family"
                },
                {
                    "name": "variables",
                    "part": [
                        {
                            "name": "threshold",
                            "valueString": "140"
                        }
                    ]
                },
                {
                    "name": "validate",
                    "valueBoolean": true
                },
                {
                    "name": "resource",
                    "resource": {
                        "resourceType": "Patient",
                        "name": [{
                            "family": "Chalmers",
                            "given": ["Peter", "James"]
                        }]
                    }
                }
            ]
        });

        b.iter(|| {
            let value = black_box(&json);
            // Simulate parsing by accessing various fields
            for param in value["parameter"].as_array().unwrap() {
                let _ = param["name"].as_str();
                if let Some(vs) = param["valueString"].as_str() {
                    let _ = vs;
                }
                if let Some(vb) = param["valueBoolean"].as_bool() {
                    let _ = vb;
                }
                if let Some(parts) = param["part"].as_array() {
                    for part in parts {
                        let _ = part["name"].as_str();
                        let _ = part["valueString"].as_str();
                    }
                }
            }
        })
    });

    group.finish();
}

fn bench_response_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("server/serialization");

    group.bench_function("simple_response", |b| {
        let response = json!({
            "resourceType": "Parameters",
            "id": "fhirpath",
            "parameter": [
                {
                    "name": "parameters",
                    "part": [
                        {
                            "name": "evaluator",
                            "valueString": "Helios FHIRPath-0.1.0"
                        },
                        {
                            "name": "expression",
                            "valueString": "Patient.name.family"
                        }
                    ]
                },
                {
                    "name": "result",
                    "valueString": "Resource",
                    "part": [
                        {
                            "name": "string",
                            "valueString": "Chalmers"
                        }
                    ]
                }
            ]
        });

        b.iter(|| {
            let json_str = serde_json::to_string(black_box(&response)).unwrap();
            black_box(json_str)
        })
    });

    group.bench_function("complex_response", |b| {
        let mut result_parts = vec![];
        for i in 0..10 {
            result_parts.push(json!({
                "name": "string",
                "valueString": format!("Result{}", i)
            }));
        }

        let response = json!({
            "resourceType": "Parameters",
            "id": "fhirpath",
            "parameter": [
                {
                    "name": "parameters",
                    "part": [
                        {
                            "name": "evaluator",
                            "valueString": "Helios FHIRPath-0.1.0"
                        },
                        {
                            "name": "expression",
                            "valueString": "Bundle.entry.resource.ofType(Patient).name.family"
                        },
                        {
                            "name": "validate",
                            "valueBoolean": true
                        }
                    ]
                },
                {
                    "name": "parseDebugTree",
                    "valueString": "{\"type\":\"expression\",\"children\":[...]}"
                },
                {
                    "name": "result",
                    "valueString": "Collection",
                    "part": result_parts
                }
            ]
        });

        b.iter(|| {
            let json_str = serde_json::to_string(black_box(&response)).unwrap();
            black_box(json_str)
        })
    });

    group.finish();
}

fn bench_large_bundle_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("server/bundle");

    // Create a bundle with many resources
    let mut entries = vec![];
    for i in 0..50 {
        entries.push(json!({
            "resource": {
                "resourceType": "Patient",
                "id": format!("patient-{}", i),
                "active": i % 2 == 0,
                "name": [{
                    "family": format!("Family{}", i),
                    "given": [format!("Given{}", i)]
                }],
                "telecom": [
                    {"system": "phone", "value": format!("555-{:04}", i)},
                    {"system": "email", "value": format!("patient{}@example.com", i)}
                ]
            }
        }));
    }

    let large_bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": entries
    });

    group.bench_function("parse_bundle", |b| {
        b.iter(|| {
            let bundle = black_box(&large_bundle);
            // Simulate processing each resource
            if let Some(entries) = bundle["entry"].as_array() {
                for entry in entries {
                    let _ = entry["resource"]["resourceType"].as_str();
                    let _ = entry["resource"]["id"].as_str();
                    let _ = entry["resource"]["active"].as_bool();
                }
            }
        })
    });

    group.bench_function("serialize_bundle", |b| {
        b.iter(|| {
            let json_str = serde_json::to_string(black_box(&large_bundle)).unwrap();
            black_box(json_str)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_request_parsing,
    bench_response_serialization,
    bench_large_bundle_processing
);
criterion_main!(benches);
