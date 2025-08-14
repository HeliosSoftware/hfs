use criterion::{Criterion, black_box, criterion_group, criterion_main};
use helios_fhirpath::cli::{Args, run_cli};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

struct TestFixture {
    _temp_dir: TempDir,
    patient_file: PathBuf,
    observation_file: PathBuf,
    bundle_file: PathBuf,
    variables_file: PathBuf,
    output_file: PathBuf,
}

impl TestFixture {
    fn new() -> Self {
        let temp_dir = TempDir::new().unwrap();

        // Create patient resource
        let patient_json = json!({
            "resourceType": "Patient",
            "id": "example",
            "active": true,
            "name": [{
                "use": "official",
                "family": "Chalmers",
                "given": ["Peter", "James"]
            }],
            "gender": "male",
            "birthDate": "1974-12-25",
            "telecom": [
                {"system": "phone", "value": "(03) 5555 6473", "use": "work"},
                {"system": "email", "value": "peter@example.com", "use": "home"}
            ]
        });

        let patient_file = temp_dir.path().join("patient.json");
        fs::write(&patient_file, patient_json.to_string()).unwrap();

        // Create observation resource
        let observation_json = json!({
            "resourceType": "Observation",
            "id": "blood-pressure",
            "status": "final",
            "code": {
                "coding": [{
                    "system": "http://loinc.org",
                    "code": "85354-9"
                }]
            },
            "valueQuantity": {
                "value": 140,
                "unit": "mm[Hg]"
            }
        });

        let observation_file = temp_dir.path().join("observation.json");
        fs::write(&observation_file, observation_json.to_string()).unwrap();

        // Create bundle with multiple resources
        let mut entries = vec![];
        for i in 0..10 {
            entries.push(json!({
                "resource": {
                    "resourceType": "Patient",
                    "id": format!("patient-{}", i),
                    "active": i % 2 == 0,
                    "name": [{
                        "family": format!("Family{}", i),
                        "given": [format!("Given{}", i)]
                    }],
                    "birthDate": format!("19{:02}-01-01", 70 + i)
                }
            }));
        }

        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": entries
        });

        let bundle_file = temp_dir.path().join("bundle.json");
        fs::write(&bundle_file, bundle_json.to_string()).unwrap();

        // Create variables file
        let variables_json = json!({
            "threshold": 100,
            "targetSystem": "email",
            "cutoffDate": "1975-01-01"
        });

        let variables_file = temp_dir.path().join("variables.json");
        fs::write(&variables_file, variables_json.to_string()).unwrap();

        let output_file = temp_dir.path().join("output.json");

        Self {
            _temp_dir: temp_dir,
            patient_file,
            observation_file,
            bundle_file,
            variables_file,
            output_file,
        }
    }
}

fn bench_simple_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("cli/simple");
    let fixture = TestFixture::new();

    group.bench_function("basic_navigation", |b| {
        b.iter(|| {
            let args = Args {
                expression: "Patient.name.family".to_string(),
                resource: fixture.patient_file.clone(),
                context: None,
                variables: None,
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.bench_function("with_function", |b| {
        b.iter(|| {
            let args = Args {
                expression: "Patient.name.given.first()".to_string(),
                resource: fixture.patient_file.clone(),
                context: None,
                variables: None,
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.finish();
}

fn bench_context_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("cli/context");
    let fixture = TestFixture::new();

    group.bench_function("with_context", |b| {
        b.iter(|| {
            let args = Args {
                expression: "given.first()".to_string(),
                resource: fixture.patient_file.clone(),
                context: Some("Patient.name".to_string()),
                variables: None,
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.bench_function("complex_context", |b| {
        b.iter(|| {
            let args = Args {
                expression: "value".to_string(),
                resource: fixture.patient_file.clone(),
                context: Some("Patient.telecom.where(system = 'email')".to_string()),
                variables: None,
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.finish();
}

fn bench_variable_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("cli/variables");
    let fixture = TestFixture::new();

    group.bench_function("inline_variable", |b| {
        b.iter(|| {
            let args = Args {
                expression: "Observation.value.value > %threshold".to_string(),
                resource: fixture.observation_file.clone(),
                context: None,
                variables: None,
                var: vec![("threshold".to_string(), "100".to_string())],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.bench_function("file_variables", |b| {
        b.iter(|| {
            let args = Args {
                expression: "Patient.telecom.where(system = %targetSystem)".to_string(),
                resource: fixture.patient_file.clone(),
                context: None,
                variables: Some(fixture.variables_file.clone()),
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.finish();
}

fn bench_bundle_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("cli/bundle");
    let fixture = TestFixture::new();

    group.bench_function("bundle_filter", |b| {
        b.iter(|| {
            let args = Args {
                expression: "Bundle.entry.resource.ofType(Patient).where(active)".to_string(),
                resource: fixture.bundle_file.clone(),
                context: None,
                variables: None,
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.bench_function("bundle_aggregate", |b| {
        b.iter(|| {
            let args = Args {
                expression: "Bundle.entry.resource.ofType(Patient).count()".to_string(),
                resource: fixture.bundle_file.clone(),
                context: None,
                variables: None,
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.finish();
}

fn bench_debug_features(c: &mut Criterion) {
    let mut group = c.benchmark_group("cli/debug");
    let fixture = TestFixture::new();

    group.bench_function("parse_debug_tree", |b| {
        b.iter(|| {
            let args = Args {
                expression: "Patient.name.where(use = 'official').given.first()".to_string(),
                resource: fixture.patient_file.clone(),
                context: None,
                variables: None,
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: true,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: false,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.bench_function("validation", |b| {
        b.iter(|| {
            let args = Args {
                expression: "Patient.name.family".to_string(),
                resource: fixture.patient_file.clone(),
                context: None,
                variables: None,
                var: vec![],
                output: Some(fixture.output_file.clone()),
                parse_debug_tree: false,
                parse_debug: false,
                trace: false,
                fhir_version: helios_fhir::FhirVersion::R4,
                validate: true,
                terminology_server: None,
            };
            run_cli(black_box(args))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_simple_expressions,
    bench_context_expressions,
    bench_variable_expressions,
    bench_bundle_operations,
    bench_debug_features
);
criterion_main!(benches);
