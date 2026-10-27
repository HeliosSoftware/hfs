# Embedding HFS as a Library

HFS crates can be used as libraries in your own Rust projects. This chapter covers adding crates as dependencies, browsing the API docs, embedding the REST layer, and integrating the CDS Hooks types.

---

## Adding Crates as Dependencies

All HFS crates are published to [crates.io](https://crates.io). Add them to your `Cargo.toml`:

```toml
[dependencies]
# FHIRPath evaluation
helios-fhirpath = "0.1"

# SQL-on-FHIR transforms
helios-sof = "0.1"

# FHIR REST API layer
helios-rest = "0.1"

# Core FHIR data models (R4 by default)
helios-fhir = "0.1"

# CDS Hooks protocol types
helios-cds-hooks = "0.1"
```

Enable additional FHIR versions with feature flags:

```toml
[dependencies]
helios-fhir = { version = "0.1", features = ["R4", "R5"] }
helios-fhirpath = { version = "0.1", features = ["R4", "R5"] }
```

---

## API Documentation with cargo doc

Generate and open the local API documentation:

```bash
cargo doc --no-deps --open
```

This opens a browser with documentation for all workspace crates. Each public function, struct, trait, and enum is documented.

---

## Embedding helios-fhirpath

The FHIRPath evaluator can be used standalone in any Rust application:

```rust
use helios_fhirpath::evaluator::{EvaluationContext, evaluate_expression};
use helios_fhir::FhirVersion;

fn main() {
    // Parse a FHIR resource from JSON
    let resource_json = r#"{
        "resourceType": "Patient",
        "id": "p1",
        "name": [{"family": "Smith", "given": ["John"]}]
    }"#;

    // Create an evaluation context
    let context = EvaluationContext::new_empty(FhirVersion::R4);

    // Evaluate a FHIRPath expression
    let result = evaluate_expression("Patient.name.family", resource_json, &context)
        .expect("evaluation failed");

    println!("{:?}", result);  // ["Smith"]
}
```

---

## Embedding helios-sof

Run ViewDefinition transforms in your Rust application:

```rust
use helios_sof::run_view_definition;

let view_def_json = std::fs::read_to_string("patient-view.json").unwrap();
let bundle_json   = std::fs::read_to_string("patients.json").unwrap();

let csv_output = run_view_definition(&view_def_json, &bundle_json, "csv")
    .expect("transform failed");

println!("{}", csv_output);
```

---

## Embedding helios-rest

The `helios-rest` crate provides a complete FHIR REST API layer built on [Axum](https://github.com/tokio-rs/axum). You can wire it into your own Axum application:

```rust
use helios_rest::router::FhirRouter;
use helios_persistence::sqlite::SqliteStorage;

#[tokio::main]
async fn main() {
    let storage = SqliteStorage::new("fhir.db").await.unwrap();
    let app = FhirRouter::new(storage).into_router();

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

`helios-rest` handles:
- All FHIR CRUD, search, history, and batch/transaction interactions
- Multi-tenancy routing (header, URL path, or both)
- ETag versioning and conditional updates
- Request ID tracking and CORS
- FHIR `OperationOutcome` error responses

Any `ResourceStorage` implementation can be plugged in.

---

## CDS Hooks Integration

The `helios-cds-hooks` crate provides complete protocol types and an async trait for building CDS Hooks services:

```toml
[dependencies]
helios-cds-hooks = "0.1"
```

Implement `CdsHooksService` and wire it to your web framework:

```rust
use helios_cds_hooks::{CdsHooksService, CdsRequest, CdsResponse, Card};

struct MyService;

#[async_trait::async_trait]
impl CdsHooksService for MyService {
    async fn handle(&self, request: CdsRequest) -> CdsResponse {
        // Evaluate clinical logic here
        CdsResponse {
            cards: vec![
                Card::info("Consider reviewing medications")
                    .with_summary("Potential drug interaction detected"),
            ],
        }
    }
}
```

The crate supports all 10 hooks in the CDS Hooks Library with strongly-typed context structs (e.g., `PatientViewContext`, `OrderSignContext`). It is FHIR version agnostic.

Full API documentation: `cargo doc -p helios-cds-hooks --no-deps --open`
