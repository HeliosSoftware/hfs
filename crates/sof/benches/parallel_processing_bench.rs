use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

fn create_large_bundle(num_patients: usize) -> String {
    let mut entries = Vec::new();

    for i in 0..num_patients {
        entries.push(format!(
            r#"{{
            "resource": {{
                "resourceType": "Patient",
                "id": "patient-{}",
                "identifier": [{{
                    "system": "http://example.org",
                    "value": "ID-{}"
                }}],
                "name": [{{
                    "given": ["John-{}"],
                    "family": "Doe-{}"
                }}],
                "gender": "{}",
                "birthDate": "1970-01-{:02}"
            }}
        }}"#,
            i,
            i,
            i,
            i,
            if i % 2 == 0 { "male" } else { "female" },
            (i % 28) + 1
        ));
    }

    format!(
        r#"{{
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{}]
    }}"#,
        entries.join(",")
    )
}

fn create_complex_view_definition() -> &'static str {
    r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {"name": "patient_id", "path": "id"},
                    {"name": "identifier_value", "path": "identifier[0].value"},
                    {"name": "first_name", "path": "name[0].given[0]"},
                    {"name": "last_name", "path": "name[0].family"},
                    {"name": "gender", "path": "gender"},
                    {"name": "birth_date", "path": "birthDate"},
                    {"name": "identifier_system", "path": "identifier[0].system"},
                    {"name": "name_use", "path": "name[0].use"},
                    {"name": "has_identifier", "path": "identifier.exists()"},
                    {"name": "name_count", "path": "name.count()"}
                ]
            }
        ]
    }"#
}

fn benchmark_parallel_processing(c: &mut Criterion) {
    let view_definition_json = create_complex_view_definition();
    let view_def_r4: helios_fhir::r4::ViewDefinition =
        serde_json::from_str(view_definition_json).expect("Failed to parse ViewDefinition");
    let view_definition = SofViewDefinition::R4(view_def_r4);

    let mut group = c.benchmark_group("parallel_resource_processing");

    for num_patients in [10, 50, 100, 200, 500].iter() {
        let bundle_json = create_large_bundle(*num_patients);
        let bundle_r4: helios_fhir::r4::Bundle =
            serde_json::from_str(&bundle_json).expect("Failed to parse Bundle");
        let bundle = SofBundle::R4(bundle_r4);

        group.bench_with_input(
            BenchmarkId::from_parameter(num_patients),
            num_patients,
            |b, _| {
                b.iter(|| {
                    let result = run_view_definition(
                        black_box(view_definition.clone()),
                        black_box(bundle.clone()),
                        black_box(ContentType::Csv),
                    );

                    match result {
                        Ok(_) => {}
                        Err(e) => panic!("Error running view definition: {}", e),
                    }
                });
            },
        );
    }

    group.finish();
}

fn benchmark_with_foreach(c: &mut Criterion) {
    let view_definition_with_foreach = r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [
            {
                "forEach": "identifier",
                "column": [
                    {"name": "patient_id", "path": "%parent.id"},
                    {"name": "identifier_system", "path": "system"},
                    {"name": "identifier_value", "path": "value"}
                ]
            }
        ]
    }"#;

    let view_def_r4: helios_fhir::r4::ViewDefinition =
        serde_json::from_str(view_definition_with_foreach).expect("Failed to parse ViewDefinition");
    let view_definition = SofViewDefinition::R4(view_def_r4);

    let mut group = c.benchmark_group("parallel_foreach_processing");

    for num_patients in [10, 50, 100, 200].iter() {
        let bundle_json = create_large_bundle(*num_patients);
        let bundle_r4: helios_fhir::r4::Bundle =
            serde_json::from_str(&bundle_json).expect("Failed to parse Bundle");
        let bundle = SofBundle::R4(bundle_r4);

        group.bench_with_input(
            BenchmarkId::from_parameter(num_patients),
            num_patients,
            |b, _| {
                b.iter(|| {
                    let result = run_view_definition(
                        black_box(view_definition.clone()),
                        black_box(bundle.clone()),
                        black_box(ContentType::Csv),
                    );

                    match result {
                        Ok(_) => {}
                        Err(e) => panic!("Error running view definition: {}", e),
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    benchmark_parallel_processing,
    benchmark_with_foreach
);
criterion_main!(benches);
