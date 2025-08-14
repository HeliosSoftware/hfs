use chumsky::Parser;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use helios_fhirpath::parser::parser;

fn bench_simple_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/simple");

    group.bench_function("single_path", |b| {
        b.iter(|| parser().parse(black_box("Patient.name")))
    });

    group.bench_function("nested_path", |b| {
        b.iter(|| parser().parse(black_box("Patient.name.family")))
    });

    group.bench_function("indexed_access", |b| {
        b.iter(|| parser().parse(black_box("Patient.name[0].given[1]")))
    });

    group.bench_function("boolean_literal", |b| {
        b.iter(|| parser().parse(black_box("true")))
    });

    group.bench_function("integer_literal", |b| {
        b.iter(|| parser().parse(black_box("42")))
    });

    group.bench_function("string_literal", |b| {
        b.iter(|| parser().parse(black_box("'hello world'")))
    });

    group.finish();
}

fn bench_function_calls(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/functions");

    group.bench_function("single_function", |b| {
        b.iter(|| parser().parse(black_box("Patient.name.first()")))
    });

    group.bench_function("function_with_args", |b| {
        b.iter(|| parser().parse(black_box("Patient.telecom.where(system = 'email')")))
    });

    group.bench_function("chained_functions", |b| {
        b.iter(|| parser().parse(black_box("Patient.name.given.first().upper()")))
    });

    group.bench_function("nested_function_calls", |b| {
        b.iter(|| {
            parser().parse(black_box(
                "Patient.name.where(use = 'official').given.select(substring(0, 1))",
            ))
        })
    });

    group.finish();
}

fn bench_operators(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/operators");

    group.bench_function("comparison", |b| {
        b.iter(|| parser().parse(black_box("value > 5")))
    });

    group.bench_function("equality", |b| {
        b.iter(|| parser().parse(black_box("status = 'active'")))
    });

    group.bench_function("arithmetic", |b| {
        b.iter(|| parser().parse(black_box("value + 10 * 2")))
    });

    group.bench_function("boolean_logic", |b| {
        b.iter(|| {
            parser().parse(black_box(
                "active and (status = 'final' or status = 'amended')",
            ))
        })
    });

    group.bench_function("union", |b| {
        b.iter(|| parser().parse(black_box("name.given | name.family")))
    });

    group.finish();
}

fn bench_complex_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/complex");

    group.bench_function("complex_filter", |b| {
        b.iter(|| {
            parser().parse(black_box(
                "Patient.telecom.where(system = 'phone' and use = 'mobile').value",
            ))
        })
    });

    group.bench_function("type_checking", |b| {
        b.iter(|| {
            parser().parse(black_box(
                "value.ofType(Quantity).where(value > 140 and unit = 'mm[Hg]')",
            ))
        })
    });

    group.bench_function("extension_access", |b| {
        b.iter(|| {
            parser().parse(black_box(
                "Patient.extension('http://example.org/birthPlace').value.ofType(Address).city",
            ))
        })
    });

    group.bench_function("aggregate_expression", |b| {
        b.iter(|| {
            parser().parse(black_box(
                "Bundle.entry.resource.ofType(Observation).value.aggregate($total + $this, 0)",
            ))
        })
    });

    group.bench_function("date_arithmetic", |b| {
        b.iter(|| parser().parse(black_box("Patient.birthDate + 1 year <= today()")))
    });

    group.bench_function("nested_where_select", |b| {
        b.iter(|| {
            parser().parse(black_box("Bundle.entry.resource.ofType(Patient).where(birthDate > @1990-01-01).name.where(use = 'official').select(given.join(' ') + ' ' + family)"))
        })
    });

    group.finish();
}

fn bench_large_expressions(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser/large");

    // Expression with many OR conditions
    let many_ors = (0..20)
        .map(|i| format!("code = 'CODE{}'", i))
        .collect::<Vec<_>>()
        .join(" or ");

    group.bench_function("many_or_conditions", |b| {
        b.iter(|| parser().parse(black_box(many_ors.as_str())))
    });

    // Expression with deep nesting
    let deep_nesting = "Patient.contact.where(relationship.coding.where(system = 'http://terminology.hl7.org/CodeSystem/v2-0131').code = 'E').name.given.first()";

    group.bench_function("deep_nesting", |b| {
        b.iter(|| parser().parse(black_box(deep_nesting)))
    });

    // Expression with many function calls
    let many_functions =
        "Patient.name.given.first().upper().substring(0, 3).replace('A', 'B').trim()";

    group.bench_function("many_functions", |b| {
        b.iter(|| parser().parse(black_box(many_functions)))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_simple_expressions,
    bench_function_calls,
    bench_operators,
    bench_complex_expressions,
    bench_large_expressions
);
criterion_main!(benches);
