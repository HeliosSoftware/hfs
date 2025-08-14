use criterion::{Criterion, black_box, criterion_group, criterion_main};
use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, evaluate_expression};
use serde_json::json;

fn create_simple_patient() -> FhirResource {
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
            {
                "system": "phone",
                "value": "(03) 5555 6473",
                "use": "work"
            },
            {
                "system": "email",
                "value": "peter@example.com",
                "use": "home"
            }
        ]
    });

    #[cfg(feature = "R4")]
    {
        let resource: helios_fhir::r4::Resource = serde_json::from_value(patient_json).unwrap();
        FhirResource::R4(Box::new(resource))
    }
    #[cfg(not(feature = "R4"))]
    panic!("R4 feature not enabled")
}

fn create_complex_patient() -> FhirResource {
    let patient_json = json!({
        "resourceType": "Patient",
        "id": "complex",
        "active": true,
        "name": [
            {
                "use": "official",
                "family": "Chalmers",
                "given": ["Peter", "James"]
            },
            {
                "use": "usual",
                "family": "Chalmers",
                "given": ["Jim"]
            },
            {
                "use": "maiden",
                "family": "Windsor",
                "given": ["Peter", "James"]
            }
        ],
        "telecom": [
            {"system": "phone", "value": "(03) 5555 6473", "use": "work"},
            {"system": "phone", "value": "(03) 4444 6473", "use": "mobile"},
            {"system": "email", "value": "peter@example.com", "use": "home"},
            {"system": "email", "value": "peter.chalmers@work.com", "use": "work"}
        ],
        "address": [
            {
                "use": "home",
                "line": ["534 Erewhon St"],
                "city": "PleasantVille",
                "state": "Vic",
                "postalCode": "3999"
            }
        ],
        "extension": [
            {
                "url": "http://example.org/birthPlace",
                "valueAddress": {
                    "city": "Seattle",
                    "state": "WA",
                    "country": "USA"
                }
            }
        ]
    });

    #[cfg(feature = "R4")]
    {
        let resource: helios_fhir::r4::Resource = serde_json::from_value(patient_json).unwrap();
        FhirResource::R4(Box::new(resource))
    }
    #[cfg(not(feature = "R4"))]
    panic!("R4 feature not enabled")
}

fn create_observation() -> FhirResource {
    let obs_json = json!({
        "resourceType": "Observation",
        "id": "blood-pressure",
        "status": "final",
        "code": {
            "coding": [{
                "system": "http://loinc.org",
                "code": "85354-9",
                "display": "Blood pressure panel"
            }]
        },
        "subject": {
            "reference": "Patient/example"
        },
        "effectiveDateTime": "2023-01-15T10:30:00Z",
        "component": [
            {
                "code": {
                    "coding": [{
                        "system": "http://loinc.org",
                        "code": "8480-6",
                        "display": "Systolic blood pressure"
                    }]
                },
                "valueQuantity": {
                    "value": 140,
                    "unit": "mm[Hg]",
                    "system": "http://unitsofmeasure.org"
                }
            },
            {
                "code": {
                    "coding": [{
                        "system": "http://loinc.org",
                        "code": "8462-4",
                        "display": "Diastolic blood pressure"
                    }]
                },
                "valueQuantity": {
                    "value": 90,
                    "unit": "mm[Hg]",
                    "system": "http://unitsofmeasure.org"
                }
            }
        ]
    });

    #[cfg(feature = "R4")]
    {
        let resource: helios_fhir::r4::Resource = serde_json::from_value(obs_json).unwrap();
        FhirResource::R4(Box::new(resource))
    }
    #[cfg(not(feature = "R4"))]
    panic!("R4 feature not enabled")
}

fn bench_simple_navigation(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluator/navigation");
    let patient = create_simple_patient();
    let context = EvaluationContext::new(vec![patient]);

    group.bench_function("single_field", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.active"), &context))
    });

    group.bench_function("nested_field", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name.family"), &context))
    });

    group.bench_function("deep_nested", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name.given.first()"), &context))
    });

    group.bench_function("indexed_access", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name[0].given[1]"), &context))
    });

    group.finish();
}

fn bench_collection_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluator/collections");
    let patient = create_complex_patient();
    let context = EvaluationContext::new(vec![patient]);

    group.bench_function("where_simple", |b| {
        b.iter(|| {
            evaluate_expression(
                black_box("Patient.telecom.where(system = 'email')"),
                &context,
            )
        })
    });

    group.bench_function("where_complex", |b| {
        b.iter(|| {
            evaluate_expression(
                black_box("Patient.telecom.where(system = 'phone' and use = 'mobile')"),
                &context,
            )
        })
    });

    group.bench_function("select", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name.select(given.first())"), &context))
    });

    group.bench_function("exists", |b| {
        b.iter(|| {
            evaluate_expression(
                black_box("Patient.telecom.where(system = 'email').exists()"),
                &context,
            )
        })
    });

    group.bench_function("count", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.telecom.count()"), &context))
    });

    group.bench_function("distinct", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name.given.distinct()"), &context))
    });

    group.finish();
}

fn bench_string_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluator/strings");
    let patient = create_simple_patient();
    let context = EvaluationContext::new(vec![patient]);

    group.bench_function("string_concat", |b| {
        b.iter(|| {
            evaluate_expression(
                black_box("Patient.name.given.first() + ' ' + Patient.name.family"),
                &context,
            )
        })
    });

    group.bench_function("upper", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name.family.upper()"), &context))
    });

    group.bench_function("substring", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name.family.substring(0, 3)"), &context))
    });

    group.bench_function("matches", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name.family.matches('^Ch')"), &context))
    });

    group.bench_function("replace", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.name.family.replace('a', 'A')"), &context))
    });

    group.finish();
}

fn bench_type_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluator/types");
    let obs = create_observation();
    let context = EvaluationContext::new(vec![obs]);

    group.bench_function("is_type", |b| {
        b.iter(|| evaluate_expression(black_box("Observation.is(Observation)"), &context))
    });

    group.bench_function("ofType", |b| {
        b.iter(|| {
            evaluate_expression(
                black_box("Observation.component.value.ofType(Quantity)"),
                &context,
            )
        })
    });

    group.bench_function("as_type", |b| {
        b.iter(|| evaluate_expression(black_box("Observation.as(DomainResource)"), &context))
    });

    group.bench_function("type_reflection", |b| {
        b.iter(|| evaluate_expression(black_box("Observation.type()"), &context))
    });

    group.finish();
}

fn bench_date_time_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluator/datetime");
    let patient = create_simple_patient();
    let context = EvaluationContext::new(vec![patient]);

    group.bench_function("date_comparison", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.birthDate > @1970-01-01"), &context))
    });

    group.bench_function("today", |b| {
        b.iter(|| evaluate_expression(black_box("today()"), &context))
    });

    group.bench_function("now", |b| {
        b.iter(|| evaluate_expression(black_box("now()"), &context))
    });

    group.bench_function("date_arithmetic", |b| {
        b.iter(|| evaluate_expression(black_box("Patient.birthDate + 1 year"), &context))
    });

    group.finish();
}

fn bench_extension_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluator/extensions");
    let patient = create_complex_patient();
    let context = EvaluationContext::new(vec![patient]);

    group.bench_function("extension_by_url", |b| {
        b.iter(|| {
            evaluate_expression(
                black_box("Patient.extension('http://example.org/birthPlace')"),
                &context,
            )
        })
    });

    group.bench_function("extension_value", |b| {
        b.iter(|| {
            evaluate_expression(
                black_box("Patient.extension('http://example.org/birthPlace').value"),
                &context,
            )
        })
    });

    group.bench_function("extension_typed_value", |b| {
        b.iter(|| {
            evaluate_expression(
                black_box(
                    "Patient.extension('http://example.org/birthPlace').value.ofType(Address).city",
                ),
                &context,
            )
        })
    });

    group.finish();
}

fn bench_complex_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("evaluator/complex");

    group.bench_function("complex_filter_select", |b| {
        let patient = create_complex_patient();
        let patient_context = EvaluationContext::new(vec![patient]);

        b.iter(|| {
            evaluate_expression(
                black_box("Patient.name.where(use = 'official').given.select($this + ' ')"),
                &patient_context,
            )
        })
    });

    group.bench_function("quantity_comparison", |b| {
        let obs = create_observation();
        let obs_context = EvaluationContext::new(vec![obs]);

        b.iter(|| {
            evaluate_expression(
                black_box("Observation.component.where(value.ofType(Quantity).value > 100)"),
                &obs_context,
            )
        })
    });

    group.bench_function("union_operation", |b| {
        let patient = create_complex_patient();
        let patient_context = EvaluationContext::new(vec![patient]);

        b.iter(|| {
            evaluate_expression(
                black_box("Patient.name.given | Patient.name.family"),
                &patient_context,
            )
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_simple_navigation,
    bench_collection_operations,
    bench_string_operations,
    bench_type_operations,
    bench_date_time_operations,
    bench_extension_access,
    bench_complex_expressions
);
criterion_main!(benches);
