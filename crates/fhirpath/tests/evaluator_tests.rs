use chumsky::Parser;
// Removed duplicate Parser import
// use chumsky::Parser; // Removed duplicate
use fhir::r4::{self, Boolean, Code, Date, Extension, ExtensionValue, String as FhirString};
use fhir::FhirResource;
use fhirpath::evaluator::{evaluate, EvaluationContext};
use fhirpath::parser::parser;
use fhirpath_support::EvaluationResult;
// use rust_decimal::Decimal; // Removed unused import
use rust_decimal_macros::dec;

// Helper function to parse and evaluate
fn eval(input: &str, context: &EvaluationContext) -> EvaluationResult {
    let expr = parser().parse(input).unwrap_or_else(|e| {
        panic!("Parser error for input '{}': {:?}", input, e);
    });
    // Pass the original context and None for current_item for top-level evaluation
    evaluate(&expr, context, None)
}

// Helper to create a collection result
fn collection(items: Vec<EvaluationResult>) -> EvaluationResult {
    EvaluationResult::Collection(items) // Removed normalize() call
}

// Removed internal date/time parsing helpers. Use eval() with literals instead.

// --- Expressions ---
// Spec: https://hl7.org/fhirpath/2025Jan/#literals
#[test]
fn test_expression_literals() {
    let context = EvaluationContext::new_empty();
    // Boolean
    assert_eq!(eval("true", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("false", &context), EvaluationResult::Boolean(false));
    // String
    assert_eq!(
        eval("'hello'", &context),
        EvaluationResult::String("hello".to_string())
    );
    assert_eq!(
        eval("'urn:oid:1.2.3'", &context),
        EvaluationResult::String("urn:oid:1.2.3".to_string())
    );
    // Integer - Should now be parsed as Integer
    assert_eq!(eval("123", &context), EvaluationResult::Integer(123));
    assert_eq!(eval("0", &context), EvaluationResult::Integer(0));
    assert_eq!(eval("-5", &context), EvaluationResult::Integer(-5));
    // Decimal - Requires a decimal point
    assert_eq!(
        eval("123.45", &context),
        EvaluationResult::Decimal(dec!(123.45)) // Use Decimal
    );
    assert_eq!(eval("0.0", &context), EvaluationResult::Decimal(dec!(0.0)));
    // Date
    assert_eq!(
        eval("@2015-02-04", &context),
        EvaluationResult::Date("2015-02-04".to_string())
    );
    assert_eq!(
        eval("@2015-02", &context),
        EvaluationResult::Date("2015-02".to_string())
    ); // Test partial date parsing
    assert_eq!(
        eval("@2015", &context),
        EvaluationResult::Date("2015".to_string())
    ); // Test partial date parsing
    // DateTime - Use eval directly
    assert_eq!(
        eval("@2015-02-04T14:34:28+09:00", &context),
        EvaluationResult::DateTime("2015-02-04T14:34:28+09:00".to_string())
    );
    assert_eq!(
        eval("@2015-02-04T14:34:28Z", &context),
        EvaluationResult::DateTime("2015-02-04T14:34:28Z".to_string())
    );
    // Time - Use eval directly
    assert_eq!(
        eval("@T14:34:28", &context),
        EvaluationResult::Time("14:34:28".to_string())
    );
    assert_eq!(
        eval("@T14:30", &context),
        EvaluationResult::Time("14:30".to_string())
    );
    // Quantity - Parser returns Decimal or Integer based on number format
    assert_eq!(
        eval("10 'mg'", &context), // 10 is parsed as Integer
        EvaluationResult::Integer(10) // Evaluator ignores unit for now
    );
    assert_eq!(
        eval("4.5 'km'", &context), // 4.5 is parsed as Number (Decimal)
        EvaluationResult::Decimal(dec!(4.5)) // Evaluator ignores unit for now
    );
    // Quantity with date/time unit - Parser returns Decimal or Integer
    assert_eq!(
        eval("100 days", &context), // 100 is parsed as Integer
        EvaluationResult::Integer(100) // Evaluator ignores unit for now
    );

    // Empty Collection (Null literal)
    assert_eq!(eval("{}", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#singleton-evaluation-of-collections
#[test]
fn test_expression_singleton_evaluation() {
    let context = EvaluationContext::new_empty();
    // Single item collection evaluates to the item
    assert_eq!(
        eval("('hello')", &context),
        EvaluationResult::String("hello".to_string())
    );
    // Empty collection evaluates to empty
    assert_eq!(eval("({}).first()", &context), EvaluationResult::Empty); // Example needing singleton eval
    // Multiple items cause error (cannot directly test error signalling here easily)
    // Need a way to check for panics or a custom error result type.
}

// --- Functions ---

// --- Existence ---
// Spec: https://hl7.org/fhirpath/2025Jan/#empty--boolean
#[test]
fn test_function_existence_empty() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.empty()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'test'.empty()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1 | 2).empty()", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#existscriteria--expression--boolean
#[test]
fn test_function_existence_exists() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.exists()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'test'.exists()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).exists()", &context),
        EvaluationResult::Boolean(true)
    );
    // With criteria
    assert_eq!(
        eval("(1 | 2 | 3).exists($this > 2)", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3).exists($this > 5)", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("{}.exists($this > 5)", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#allcriteria--expression--boolean
#[test]
fn test_function_existence_all() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.all($this > 1)", &context),
        EvaluationResult::Boolean(true)
    ); // Empty collection is true
    assert_eq!(
        eval("(1 | 2 | 3).all($this > 0)", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3).all($this > 1)", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1 | 2 | 3).all($this.toString() = '1')", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#alltrue--boolean
#[test]
fn test_function_existence_all_true() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.allTrue()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true).allTrue()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | true).allTrue()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | false).allTrue()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(false | false).allTrue()", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#anytrue--boolean
#[test]
fn test_function_existence_any_true() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.anyTrue()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(true).anyTrue()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | true).anyTrue()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | false).anyTrue()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(false | false).anyTrue()", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#allfalse--boolean
#[test]
fn test_function_existence_all_false() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.allFalse()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(false).allFalse()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(false | false).allFalse()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | false).allFalse()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(true | true).allFalse()", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#anyfalse--boolean
#[test]
fn test_function_existence_any_false() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.anyFalse()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(false).anyFalse()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(false | false).anyFalse()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | false).anyFalse()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | true).anyFalse()", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#subsetofother--collection--boolean
#[test]
fn test_function_existence_subset_of() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.subsetOf({})", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("{}.subsetOf({1, 2})", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1).subsetOf({1, 2})", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).subsetOf({1, 2})", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3).subsetOf({1, 2})", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1 | 2).subsetOf({})", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1 | 2).subsetOf({1})", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#supersetofother--collection--boolean
#[test]
fn test_function_existence_superset_of() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.supersetOf({})", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).supersetOf({})", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).supersetOf({1})", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).supersetOf({1, 2})", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).supersetOf({1, 2, 3})", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("{}.supersetOf({1, 2})", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1).supersetOf({1, 2})", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#count--integer
#[test]
fn test_function_existence_count() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.count()", &context), EvaluationResult::Integer(0));
    assert_eq!(
        eval("'test'.count()", &context),
        EvaluationResult::Integer(1)
    );
    assert_eq!(
        eval("(1 | 2 | 3).count()", &context),
        EvaluationResult::Integer(3)
    );
    assert_eq!(
        eval("(1 | 2 | 1).count()", &context),
        EvaluationResult::Integer(3)
    ); // Duplicates are counted
}

// Spec: https://hl7.org/fhirpath/2025Jan/#distinct--collection
#[test]
fn test_function_existence_distinct() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.distinct()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(1).distinct()", &context),
        EvaluationResult::Integer(1)
    );
    // Order not guaranteed, so check contents
    let result = eval("(1 | 2 | 1 | 3 | 2).distinct()", &context);
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers, got {:?}", item), // Improved panic message
            })
            .collect();
        actual_items.sort();
        assert_eq!(actual_items, vec![1, 2, 3]);
    } else {
        panic!("Expected collection result from distinct");
    }
}

// Spec: https://hl7.org/fhirpath/2025Jan/#isdistinct--boolean
#[test]
fn test_function_existence_is_distinct() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.isDistinct()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1).isDistinct()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3).isDistinct()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 1).isDistinct()", &context),
        EvaluationResult::Boolean(false)
    );
}

// --- Filtering and Projection ---
// Spec: https://hl7.org/fhirpath/2025Jan/#wherecriteria--expression--collection
#[test]
fn test_function_filtering_where() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.where($this > 1)", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(1 | 2 | 3 | 4).where($this > 2)", &context),
        collection(vec![
            EvaluationResult::Integer(3),
            EvaluationResult::Integer(4)
        ])
    );
    assert_eq!(
        eval("(1 | 2 | 3 | 4).where($this > 5)", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("('a' | 'b' | 'c').where($this = 'b')", &context),
        EvaluationResult::String("b".to_string())
    );
    // Test empty result from criteria is ignored
    assert_eq!(
        eval("(1 | 2 | {}).where($this > 1)", &context),
        EvaluationResult::Integer(2)
    );
    // Test criteria evaluating to non-boolean (should be ignored)
    assert_eq!(
        eval("(1 | 2 | 3).where($this)", &context),
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3)
        ])
    ); // All items are truthy
    assert_eq!(
        eval("(0 | 1 | 2).where($this)", &context),
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2)
        ])
    ); // 0 is falsy
}

// Spec: https://hl7.org/fhirpath/2025Jan/#selectprojection-expression--collection
#[test]
fn test_function_filtering_select() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.select($this + 1)", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(1 | 2 | 3).select($this * 2)", &context),
        collection(vec![
            EvaluationResult::Integer(2), // Result should be Integer
            EvaluationResult::Integer(4), // Result should be Integer
            EvaluationResult::Integer(6)  // Result should be Integer
        ])
    );
    // Test flattening
    assert_eq!(
        eval("( (1|2) | (3|4) ).select($this)", &context),
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3),
            EvaluationResult::Integer(4)
        ])
    );
    // Test empty result from projection is skipped
    assert_eq!(
        eval("(1 | 2 | 3).select(iif($this > 2, $this, {}))", &context),
        EvaluationResult::Integer(3)
    );
    // Test projection resulting in collection
    assert_eq!(
        eval("(1 | 2).select( ( $this ) | ( $this + 1 ) )", &context),
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3)
        ])
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#oftypetype--type-specifier--collection
#[test]
fn test_function_filtering_of_type() {
    let context = EvaluationContext::new_empty();
    // Simple types
    assert_eq!(
        eval("(1 | 'a' | true).ofType(Integer)", &context),
        EvaluationResult::Integer(1)
    );
    assert_eq!(
        eval("(1 | 'a' | true).ofType(String)", &context),
        EvaluationResult::String("a".to_string())
    );
    assert_eq!(
        eval("(1 | 'a' | true).ofType(Boolean)", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 'a' | true | 1.5).ofType(Decimal)", &context),
        EvaluationResult::Decimal(dec!(1.5))
    );
    assert_eq!(
        eval("{}.ofType(Integer)", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(1 | 'a' | true).ofType(System.Integer)", &context),
        EvaluationResult::Integer(1)
    );

    // Complex types (requires resource context and proper object representation)
    let patient = r4::Patient {
        id: Some("p1".to_string().into()), // Use .to_string().into()
        active: Some(true.into()),
        ..Default::default()
    };
    let observation = r4::Observation {
        id: Some("o1".to_string().into()), // Use .to_string().into()
        status: r4::Code {
            value: Some("final".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    let resources = vec![
        FhirResource::R4(Box::new(r4::Resource::Patient(patient))),
        FhirResource::R4(Box::new(r4::Resource::Observation(observation))),
    ];
    let ctx_res = EvaluationContext::new(resources);

    // Evaluate against the implicit %context which is the collection of resources
    let result_patient = eval("%context.ofType(Patient)", &ctx_res);
    assert!(
        matches!(&result_patient, EvaluationResult::Object(_)),
        "Expected Object, got {:?}",
        result_patient
    );
    if let EvaluationResult::Object(fields) = result_patient {
        assert_eq!(
            fields.get("id"),
            Some(&EvaluationResult::String("p1".to_string()))
        );
        assert_eq!(fields.get("active"), Some(&EvaluationResult::Boolean(true)));
    }

    let result_obs = eval("%context.ofType(Observation)", &ctx_res);
    assert!(
        matches!(&result_obs, EvaluationResult::Object(_)),
        "Expected Object, got {:?}",
        result_obs
    );
    if let EvaluationResult::Object(fields) = result_obs {
        assert_eq!(
            fields.get("id"),
            Some(&EvaluationResult::String("o1".to_string()))
        );
        // Check nested status field
        match fields.get("status") {
            Some(EvaluationResult::Object(status_fields)) => {
                assert_eq!(
                    status_fields.get("value"),
                    Some(&EvaluationResult::String("final".to_string()))
                );
            }
            _ => panic!("Expected status object"),
        }
    }

    assert_eq!(
        eval("%context.ofType(Practitioner)", &ctx_res),
        EvaluationResult::Empty
    );
}

// --- Subsetting ---
// Spec: https://hl7.org/fhirpath/2025Jan/#-index--integer---collection
#[test]
fn test_function_subsetting_indexer() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}[0]", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(10 | 20 | 30)[0]", &context),
        EvaluationResult::Integer(10)
    );
    assert_eq!(
        eval("(10 | 20 | 30)[1]", &context),
        EvaluationResult::Integer(20)
    );
    assert_eq!(
        eval("(10 | 20 | 30)[2]", &context),
        EvaluationResult::Integer(30)
    );
    assert_eq!(eval("(10 | 20 | 30)[3]", &context), EvaluationResult::Empty); // Index out of bounds
    assert_eq!(
        eval("(10 | 20 | 30)[-1]", &context),
        EvaluationResult::Empty
    ); // Index out of bounds
}

// Spec: https://hl7.org/fhirpath/2025Jan/#single--collection
#[test]
fn test_function_subsetting_single() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.single()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(10).single()", &context),
        EvaluationResult::Integer(10)
    );
    // Multiple items should error - cannot test directly here easily.
    // eval("(10 | 20).single()", &context); // Expect panic or error result
}

// Spec: https://hl7.org/fhirpath/2025Jan/#first--collection
#[test]
fn test_function_subsetting_first() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.first()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(10).first()", &context),
        EvaluationResult::Integer(10)
    );
    assert_eq!(
        eval("(10 | 20 | 30).first()", &context),
        EvaluationResult::Integer(10)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#last--collection
#[test]
fn test_function_subsetting_last() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.last()", &context), EvaluationResult::Empty);
    assert_eq!(eval("(10).last()", &context), EvaluationResult::Integer(10));
    assert_eq!(
        eval("(10 | 20 | 30).last()", &context),
        EvaluationResult::Integer(30)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#tail--collection
#[test]
fn test_function_subsetting_tail() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.tail()", &context), EvaluationResult::Empty);
    assert_eq!(eval("(10).tail()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(10 | 20 | 30).tail()", &context),
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#skipnum--integer--collection
#[test]
fn test_function_subsetting_skip() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.skip(1)", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(10 | 20 | 30).skip(0)", &context),
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
    assert_eq!(
        eval("(10 | 20 | 30).skip(1)", &context),
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
    assert_eq!(
        eval("(10 | 20 | 30).skip(3)", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(10 | 20 | 30).skip(4)", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(10 | 20 | 30).skip(-1)", &context),
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#takenum--integer--collection
#[test]
fn test_function_subsetting_take() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.take(1)", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(10 | 20 | 30).take(0)", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(10 | 20 | 30).take(1)", &context),
        EvaluationResult::Integer(10)
    );
    assert_eq!(
        eval("(10 | 20 | 30).take(2)", &context),
        collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20)
        ])
    );
    assert_eq!(
        eval("(10 | 20 | 30).take(3)", &context),
        collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
    assert_eq!(
        eval("(10 | 20 | 30).take(4)", &context),
        collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
    assert_eq!(
        eval("(10 | 20 | 30).take(-1)", &context),
        EvaluationResult::Empty
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#intersectother-collection--collection
#[test]
fn test_function_subsetting_intersect() {
    // Note: HashSet used internally, order is not guaranteed in output
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.intersect({})", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(1 | 2 | 3).intersect({})", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("{}.intersect({1 | 2 | 3})", &context),
        EvaluationResult::Empty
    );
    // Order not guaranteed, check contents
    let result = eval("(1 | 2 | 3).intersect({2 | 3 | 4})", &context);
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers, got {:?}", item), // Improved panic message
            })
            .collect();
        actual_items.sort();
        assert_eq!(actual_items, vec![2, 3]);
    } else {
        panic!("Expected collection result from intersect");
    }
    let result = eval("(1 | 2 | 1).intersect({1 | 3 | 1})", &context);
    if let EvaluationResult::Collection(items) = result {
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], EvaluationResult::Integer(1)));
    } else {
        panic!("Expected collection result from intersect");
    }
}

// Spec: https://hl7.org/fhirpath/2025Jan/#excludeother-collection--collection
#[test]
fn test_function_subsetting_exclude() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.exclude({})", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(1 | 2 | 3).exclude({})", &context),
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3)
        ])
    );
    assert_eq!(
        eval("{}.exclude({1 | 2 | 3})", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(1 | 2 | 3).exclude({2 | 4})", &context),
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(3)
        ])
    );
    // Preserves duplicates and order
    assert_eq!(
        eval("(1 | 2 | 1 | 3 | 2).exclude({1 | 4})", &context),
        collection(vec![
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3),
            EvaluationResult::Integer(2)
        ])
    );
}

// --- Combining ---
// Spec: https://hl7.org/fhirpath/2025Jan/#unionother--collection
#[test]
fn test_function_combining_union() {
    // Note: HashSet used internally, order is not guaranteed in output
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.union({})", &context), EvaluationResult::Empty);

    let r1 = eval("(1 | 2).union({})", &context);
    assert!(matches!(&r1, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(v) = r1 {
        assert_eq!(v.len(), 2); /* Check items if needed */
    }

    let r2 = eval("{}.union({1 | 2})", &context);
    assert!(matches!(&r2, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(v) = r2 {
        assert_eq!(v.len(), 2); /* Check items if needed */
    }

    // Order not guaranteed, check contents
    let result = eval("(1 | 2 | 3).union({2 | 3 | 4})", &context);
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers, got {:?}", item), // Use pattern matching
            })
            .collect();
        actual_items.sort();
        assert_eq!(actual_items, vec![1, 2, 3, 4]);
    } else {
        panic!("Expected collection result from union");
    }
    let result = eval("(1 | 2 | 1).union({1 | 3 | 1})", &context);
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers, got {:?}", item), // Use pattern matching
            })
            .collect();
        actual_items.sort();
        assert_eq!(actual_items, vec![1, 2, 3]);
    } else {
        panic!("Expected collection result from union");
    }
}

// Spec: https://hl7.org/fhirpath/2025Jan/#combineother--collection--collection
#[test]
fn test_function_combining_combine() {
    // Note: Order not guaranteed in output
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.combine({})", &context), EvaluationResult::Empty);

    let r1 = eval("(1 | 2).combine({})", &context);
    assert!(matches!(&r1, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(v) = r1 {
        assert_eq!(v.len(), 2); /* Check items if needed */
    }

    let r2 = eval("{}.combine({1 | 2})", &context);
    assert!(matches!(&r2, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(v) = r2 {
        assert_eq!(v.len(), 2); /* Check items if needed */
    }

    // Order not guaranteed, check contents, duplicates preserved
    let result = eval("(1 | 2 | 3).combine({2 | 3 | 4})", &context);
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers, got {:?}", item), // Use pattern matching
            })
            .collect();
        actual_items.sort();
        assert_eq!(actual_items, vec![1, 2, 2, 3, 3, 4]);
    } else {
        panic!("Expected collection result from combine");
    }
    let result = eval("(1 | 2 | 1).combine({1 | 3 | 1})", &context);
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers, got {:?}", item), // Use pattern matching
            })
            .collect();
        actual_items.sort();
        assert_eq!(actual_items, vec![1, 1, 1, 1, 2, 3]);
    } else {
        panic!("Expected collection result from combine");
    }
}

// --- Conversion ---
// Spec: https://hl7.org/fhirpath/2025Jan/#iifcriterion-expression-true-result-collection--otherwise-result-collection--collection
#[test]
fn test_function_conversion_iif() {
    let context = EvaluationContext::new_empty();
    // Requires expression passing
    assert_eq!(
        eval("iif(true, 'a', 'b')", &context),
        EvaluationResult::String("a".to_string())
    );
    assert_eq!(
        eval("iif(false, 'a', 'b')", &context),
        EvaluationResult::String("b".to_string())
    );
    assert_eq!(
        eval("iif({}, 'a', 'b')", &context),
        EvaluationResult::String("b".to_string())
    ); // Empty condition is false
    assert_eq!(
        eval("iif(true, 'a')", &context),
        EvaluationResult::String("a".to_string())
    ); // Omitted otherwise
    assert_eq!(eval("iif(false, 'a')", &context), EvaluationResult::Empty); // Omitted otherwise
    assert_eq!(eval("iif({}, 'a')", &context), EvaluationResult::Empty); // Omitted otherwise
    // Test collection results
    assert_eq!(
        eval("iif(true, (1|2), (3|4))", &context),
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2)
        ])
    );
    assert_eq!(
        eval("iif(false, (1|2), (3|4))", &context),
        collection(vec![
            EvaluationResult::Integer(3),
            EvaluationResult::Integer(4)
        ])
    );
    // Test short-circuiting (cannot test directly, assume implementation detail)
    // Example: iif(true, 1, $this) should not fail even if $this is invalid in outer scope
}

// Spec: https://hl7.org/fhirpath/2025Jan/#toboolean--boolean
#[test]
fn test_function_conversion_to_boolean() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toBoolean()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("true.toBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("1.toBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("0.toBoolean()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("1.0.toBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("0.0.toBoolean()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'true'.toBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'false'.toBoolean()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'T'.toBoolean()", &context),
        EvaluationResult::Boolean(true)
    ); // Case-insensitive
    assert_eq!(
        eval("'f'.toBoolean()", &context),
        EvaluationResult::Boolean(false)
    ); // Case-insensitive
    assert_eq!(
        eval("'yes'.toBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'no'.toBoolean()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'1'.toBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'0'.toBoolean()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'1.0'.toBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'0.0'.toBoolean()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(eval("2.toBoolean()", &context), EvaluationResult::Empty); // Invalid integer
    assert_eq!(
        eval("2.5.toBoolean()", &context),
        EvaluationResult::Empty
    ); // Invalid decimal
    assert_eq!(eval("'abc'.toBoolean()", &context), EvaluationResult::Empty); // Invalid string
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstoboolean--boolean
#[test]
fn test_function_conversion_converts_to_boolean() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToBoolean()", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("true.convertsToBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("1.convertsToBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("0.convertsToBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("1.0.convertsToBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("0.0.convertsToBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'true'.convertsToBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'false'.convertsToBoolean()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("2.convertsToBoolean()", &context),
        EvaluationResult::Boolean(false)
    ); // Invalid decimal
    assert_eq!(
        eval("'abc'.convertsToBoolean()", &context),
        EvaluationResult::Boolean(false)
    ); // Invalid string
}

// Spec: https://hl7.org/fhirpath/2025Jan/#tointeger--integer
#[test]
fn test_function_conversion_to_integer() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toInteger()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("123.toInteger()", &context),
        EvaluationResult::Integer(123)
    );
    assert_eq!(
        eval("'456'.toInteger()", &context),
        EvaluationResult::Integer(456)
    );
    assert_eq!(
        eval("'+789'.toInteger()", &context),
        EvaluationResult::Integer(789)
    );
    assert_eq!(
        eval("'-12'.toInteger()", &context),
        EvaluationResult::Integer(-12)
    );
    assert_eq!(
        eval("true.toInteger()", &context),
        EvaluationResult::Integer(1)
    );
    assert_eq!(
        eval("false.toInteger()", &context),
        EvaluationResult::Integer(0)
    );
    // Decimal conversion to Integer (truncates) - FHIRPath spec says Empty if not integer representable
    assert_eq!(
        eval("123.45.toInteger()", &context),
        EvaluationResult::Empty // Per spec
    );
    assert_eq!(
        eval("123.0.toInteger()", &context),
        EvaluationResult::Empty // Per spec (even if whole number)
    );
    assert_eq!(eval("'abc'.toInteger()", &context), EvaluationResult::Empty); // Invalid string
    assert_eq!(
        eval("'123.45'.toInteger()", &context),
        EvaluationResult::Empty
    ); // Invalid string format
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstointeger--boolean
#[test]
fn test_function_conversion_converts_to_integer() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToInteger()", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("123.convertsToInteger()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'456'.convertsToInteger()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true.convertsToInteger()", &context),
        EvaluationResult::Boolean(true)
    );
    // Decimal conversion check
    assert_eq!(
        eval("123.45.convertsToInteger()", &context),
        EvaluationResult::Boolean(false) // Per spec
    );
    assert_eq!(
        eval("123.0.convertsToInteger()", &context),
        EvaluationResult::Boolean(false) // Per spec
    );
    assert_eq!(
        eval("'abc'.convertsToInteger()", &context),
        EvaluationResult::Boolean(false)
    ); // Invalid string
}

// Spec: https://hl7.org/fhirpath/2025Jan/#todecimal--decimal
#[test]
fn test_function_conversion_to_decimal() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toDecimal()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("123.toDecimal()", &context),
        EvaluationResult::Decimal(dec!(123.0)) // Integer to Decimal (explicit .0)
    );
    assert_eq!(
        eval("123.45.toDecimal()", &context),
        EvaluationResult::Decimal(dec!(123.45)) // Decimal to Decimal
    );
    assert_eq!(
        eval("'456.78'.toDecimal()", &context),
        EvaluationResult::Decimal(dec!(456.78)) // String to Decimal
    );
    assert_eq!(
        eval("'+12.3'.toDecimal()", &context),
        EvaluationResult::Decimal(dec!(12.3)) // String with sign
    );
    assert_eq!(
        eval("'-45.6'.toDecimal()", &context),
        EvaluationResult::Decimal(dec!(-45.6)) // String with sign
    );
    assert_eq!(
        eval("'789'.toDecimal()", &context),
        EvaluationResult::Decimal(dec!(789.0)) // Integer string -> Decimal (explicit .0)
    );
    assert_eq!(
        eval("true.toDecimal()", &context),
        EvaluationResult::Decimal(dec!(1.0)) // Boolean to Decimal (explicit .0)
    );
    assert_eq!(
        eval("false.toDecimal()", &context),
        EvaluationResult::Decimal(dec!(0.0)) // Boolean to Decimal (explicit .0)
    );
    assert_eq!(eval("'abc'.toDecimal()", &context), EvaluationResult::Empty); // Invalid string
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstodecimal--boolean
#[test]
fn test_function_conversion_converts_to_decimal() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToDecimal()", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("123.convertsToDecimal()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("123.45.convertsToDecimal()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'456.78'.convertsToDecimal()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true.convertsToDecimal()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abc'.convertsToDecimal()", &context),
        EvaluationResult::Boolean(false)
    ); // Invalid string
}

// Spec: https://hl7.org/fhirpath/2025Jan/#tostring--string
#[test]
fn test_function_conversion_to_string() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toString()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("'abc'.toString()", &context),
        EvaluationResult::String("abc".to_string())
    );
    assert_eq!(
        eval("123.toString()", &context),
        EvaluationResult::String("123".to_string())
    );
    assert_eq!(
        eval("123.45.toString()", &context),
        // Removed duplicate eval call, compare directly to expected result
        EvaluationResult::String("123.45".to_string()) // Decimal to string
    );
    assert_eq!(
        eval("true.toString()", &context),
        EvaluationResult::String("true".to_string())
    );
    assert_eq!(
        eval("false.toString()", &context),
        EvaluationResult::String("false".to_string())
    );
    assert_eq!(
        eval("@2023-10-27.toString()", &context),
        EvaluationResult::String("2023-10-27".to_string())
    );
    assert_eq!(
        eval("@T10:30:00.toString()", &context),
        EvaluationResult::String("10:30:00.000".to_string())
    ); // Note: .000 added by chrono format
    assert_eq!(
        eval("@2023-10-27T10:30:00Z.toString()", &context),
        EvaluationResult::String("2023-10-27T10:30:00+00:00".to_string()) // RFC3339 format
    );
    // Quantity to string (evaluator returns Decimal or Integer, ignoring unit)
    assert_eq!(
        eval("5.5 'mg'.toString()", &context), // Decimal quantity
        EvaluationResult::String("5.5".to_string())
    );
     assert_eq!(
        eval("5 'mg'.toString()", &context), // Integer quantity
        EvaluationResult::String("5".to_string())
    );
    // Collection to string
    assert_eq!(
        eval("(1|2).toString()", &context),
        EvaluationResult::String("".to_string())
    ); // Multi-item collection -> empty string
    assert_eq!(
        eval("(1).toString()", &context),
        EvaluationResult::String("1".to_string())
    ); // Single-item collection -> item string
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstostring--string
#[test]
fn test_function_conversion_converts_to_string() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToString()", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("'abc'.convertsToString()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("123.convertsToString()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("123.45.convertsToString()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true.convertsToString()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27.convertsToString()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@T10:30:00.convertsToString()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27T10:30:00Z.convertsToString()", &context),
        EvaluationResult::Boolean(true)
    );
    // Quantity conversion (evaluator returns Decimal or Integer)
    assert_eq!(
        eval("5.5 'mg'.convertsToString()", &context), // Decimal quantity
        EvaluationResult::Boolean(true)
    );
     assert_eq!(
        eval("5 'mg'.convertsToString()", &context), // Integer quantity
        EvaluationResult::Boolean(true)
    );
    // Object/Collection are not convertible
    assert_eq!(
        eval("(1|2).convertsToString()", &context),
        EvaluationResult::Boolean(false)
    );
    // Need object test once available
}

// TODO: Add tests for toDate, convertsToDate, toDateTime, convertsToDateTime, toTime, convertsToTime, toQuantity, convertsToQuantity

// --- String Manipulation ---
// Spec: https://hl7.org/fhirpath/2025Jan/#indexofsubstring--string--integer
#[test]
fn test_function_string_index_of() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abcdefg'.indexOf('bc')", &context),
        EvaluationResult::Integer(1)
    );
    assert_eq!(
        eval("'abcdefg'.indexOf('x')", &context),
        EvaluationResult::Integer(-1)
    );
    assert_eq!(
        eval("'abcdefg'.indexOf('abc')", &context),
        EvaluationResult::Integer(0)
    );
    assert_eq!(
        eval("'abcabc'.indexOf('bc')", &context),
        EvaluationResult::Integer(1)
    ); // First occurrence
    assert_eq!(
        eval("'abcdefg'.indexOf('')", &context),
        EvaluationResult::Integer(0)
    );
    assert_eq!(
        eval("''.indexOf('a')", &context),
        EvaluationResult::Integer(-1)
    );
    assert_eq!(
        eval("''.indexOf('')", &context),
        EvaluationResult::Integer(0)
    );
    assert_eq!(eval("{}.indexOf('a')", &context), EvaluationResult::Empty);
    assert_eq!(eval("'abc'.indexOf({})", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#substringstart--integer--length--integer--string
#[test]
fn test_function_string_substring() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abcdefg'.substring(0)", &context),
        EvaluationResult::String("abcdefg".to_string())
    );
    assert_eq!(
        eval("'abcdefg'.substring(3)", &context),
        EvaluationResult::String("defg".to_string())
    );
    assert_eq!(
        eval("'abcdefg'.substring(1, 2)", &context),
        EvaluationResult::String("bc".to_string())
    );
    assert_eq!(
        eval("'abcdefg'.substring(6, 2)", &context),
        EvaluationResult::String("g".to_string())
    );
    assert_eq!(
        eval("'abcdefg'.substring(7, 1)", &context),
        EvaluationResult::Empty
    ); // Start out of bounds
    assert_eq!(
        eval("'abcdefg'.substring(-1, 1)", &context),
        EvaluationResult::Empty
    ); // Start out of bounds
    assert_eq!(
        eval("'abcdefg'.substring(3, 0)", &context),
        EvaluationResult::String("".to_string())
    ); // Zero length
    assert_eq!(
        eval("'abcdefg'.substring(3, -1)", &context),
        EvaluationResult::String("".to_string())
    ); // Negative length
    assert_eq!(
        eval("''.substring(0)", &context),
        EvaluationResult::String("".to_string())
    );
    assert_eq!(eval("{}.substring(0)", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("'abc'.substring({})", &context),
        EvaluationResult::Empty
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#startswithprefix--string--boolean
#[test]
fn test_function_string_starts_with() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abcdefg'.startsWith('abc')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.startsWith('ab')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.startsWith('a')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.startsWith('bc')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'abcdefg'.startsWith('abcdefg')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.startsWith('')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("''.startsWith('a')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("''.startsWith('')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("{}.startsWith('a')", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("'abc'.startsWith({})", &context),
        EvaluationResult::Empty
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#endswithsuffix--string--boolean
#[test]
fn test_function_string_ends_with() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abcdefg'.endsWith('efg')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.endsWith('fg')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.endsWith('g')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.endsWith('ef')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'abcdefg'.endsWith('abcdefg')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.endsWith('')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("''.endsWith('a')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("''.endsWith('')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(eval("{}.endsWith('a')", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("'abc'.endsWith({})", &context),
        EvaluationResult::Empty
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#containssubstring--string--boolean
#[test]
fn test_function_string_contains() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abcdefg'.contains('cde')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.contains('abc')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.contains('efg')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abcdefg'.contains('ace')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'abcdefg'.contains('x')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'abcdefg'.contains('')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("''.contains('a')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("''.contains('')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(eval("{}.contains('a')", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("'abc'.contains({})", &context),
        EvaluationResult::Empty
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#upper--string
#[test]
fn test_function_string_upper() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abcdefg'.upper()", &context),
        EvaluationResult::String("ABCDEFG".to_string())
    );
    assert_eq!(
        eval("'AbCdEfG'.upper()", &context),
        EvaluationResult::String("ABCDEFG".to_string())
    );
    assert_eq!(
        eval("'123'.upper()", &context),
        EvaluationResult::String("123".to_string())
    );
    assert_eq!(
        eval("''.upper()", &context),
        EvaluationResult::String("".to_string())
    );
    assert_eq!(eval("{}.upper()", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#lower--string
#[test]
fn test_function_string_lower() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'ABCDEFG'.lower()", &context),
        EvaluationResult::String("abcdefg".to_string())
    );
    assert_eq!(
        eval("'aBcDeFg'.lower()", &context),
        EvaluationResult::String("abcdefg".to_string())
    );
    assert_eq!(
        eval("'123'.lower()", &context),
        EvaluationResult::String("123".to_string())
    );
    assert_eq!(
        eval("''.lower()", &context),
        EvaluationResult::String("".to_string())
    );
    assert_eq!(eval("{}.lower()", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#replacepattern--string-substitution--string--string
#[test]
fn test_function_string_replace() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abcdefg'.replace('cde', '123')", &context),
        EvaluationResult::String("ab123fg".to_string())
    );
    assert_eq!(
        eval("'abcabc'.replace('bc', 'XY')", &context),
        EvaluationResult::String("aXYaXY".to_string())
    ); // All instances
    assert_eq!(
        eval("'abcdefg'.replace('xyz', '123')", &context),
        EvaluationResult::String("abcdefg".to_string())
    ); // Pattern not found
    assert_eq!(
        eval("'abcdefg'.replace('cde', '')", &context),
        EvaluationResult::String("abfg".to_string())
    ); // Empty substitution
    assert_eq!(
        eval("'abc'.replace('', 'x')", &context),
        EvaluationResult::String("xaxbxcx".to_string())
    ); // Empty pattern
    assert_eq!(
        eval("''.replace('a', 'b')", &context),
        EvaluationResult::String("".to_string())
    );
    assert_eq!(
        eval("'abc'.replace('', '')", &context),
        EvaluationResult::String("abc".to_string())
    );
    assert_eq!(
        eval("{}.replace('a', 'b')", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("'abc'.replace({}, 'b')", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("'abc'.replace('a', {})", &context),
        EvaluationResult::Empty
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#matchesregex--string--boolean
#[test]
fn test_function_string_matches() {
    let context = EvaluationContext::new_empty();
    // Basic matching
    assert_eq!(
        eval("'abc'.matches('b')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abc'.matches('^b')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'abc'.matches('bc$')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abc'.matches('^abc$')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abc'.matches('x')", &context),
        EvaluationResult::Boolean(false)
    );
    // Regex features (basic)
    assert_eq!(
        eval("'123'.matches('\\\\d+')", &context),
        EvaluationResult::Boolean(true)
    ); // Need double escape for Rust string literal then FHIRPath string literal
    assert_eq!(
        eval("'abc'.matches('\\\\d+')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'a.c'.matches('a.c')", &context),
        EvaluationResult::Boolean(true)
    ); // '.' matches any char
    assert_eq!(
        eval("'axc'.matches('a.c')", &context),
        EvaluationResult::Boolean(true)
    );
    // Empty cases
    assert_eq!(
        eval("'abc'.matches('')", &context),
        EvaluationResult::Boolean(true)
    ); // Empty regex matches
    assert_eq!(
        eval("''.matches('a')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("''.matches('')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(eval("{}.matches('a')", &context), EvaluationResult::Empty);
    assert_eq!(eval("'abc'.matches({})", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#replacematchesregex--string-substitution-string--string
#[test]
fn test_function_string_replace_matches() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abc123def'.replaceMatches('\\\\d+', '#')", &context),
        EvaluationResult::String("abc#def".to_string())
    );
    assert_eq!(
        eval("'abc123def456'.replaceMatches('\\\\d+', '#')", &context),
        EvaluationResult::String("abc#def#".to_string())
    ); // All matches
    assert_eq!(
        eval("'abc'.replaceMatches('\\\\d+', '#')", &context),
        EvaluationResult::String("abc".to_string())
    ); // No match
    // Groups (example from spec)
    let expr = "'11/30/1972'.replaceMatches('\\\\b(?<month>\\\\d{1,2})/(?<day>\\\\d{1,2})/(?<year>\\\\d{2,4})\\\\b', '${day}-${month}-${year}')";
    assert_eq!(
        eval(expr, &context),
        EvaluationResult::String("30-11-1972".to_string())
    );
    // Empty cases
    assert_eq!(
        eval("'abc'.replaceMatches('', '#')", &context),
        EvaluationResult::String("#a#b#c#".to_string())
    ); // Empty regex matches everywhere
    assert_eq!(
        eval("''.replaceMatches('a', '#')", &context),
        EvaluationResult::String("".to_string())
    );
    assert_eq!(
        eval("{}.replaceMatches('a', '#')", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("'abc'.replaceMatches({}, '#')", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("'abc'.replaceMatches('a', {})", &context),
        EvaluationResult::Empty
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#length--integer
#[test]
fn test_function_string_length() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abcdefg'.length()", &context),
        EvaluationResult::Integer(7)
    );
    assert_eq!(eval("''.length()", &context), EvaluationResult::Integer(0));
    assert_eq!(eval("{}.length()", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#tochars--collection
#[test]
fn test_function_string_to_chars() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'abc'.toChars()", &context),
        EvaluationResult::Collection(vec![
            EvaluationResult::String("a".to_string()),
            EvaluationResult::String("b".to_string()),
            EvaluationResult::String("c".to_string()),
        ])
    );
    assert_eq!(eval("''.toChars()", &context), EvaluationResult::Empty);
    assert_eq!(eval("{}.toChars()", &context), EvaluationResult::Empty);
}

// --- Utility Functions ---
// Spec: https://hl7.org/fhirpath/2025Jan/#now--datetime
#[test]
fn test_function_utility_now() {
    let context = EvaluationContext::new_empty();
    let result = eval("now()", &context);
    // Check it's a DateTime, format might vary slightly
    assert!(matches!(result, EvaluationResult::DateTime(_)));
    // Check determinism (calling twice gives same result)
    let expr = parser().parse("now() = now()").unwrap();
    assert_eq!(evaluate(&expr, &context, None), EvaluationResult::Boolean(true));
}

// Spec: https://hl7.org/fhirpath/2025Jan/#timeofday--time
#[test]
fn test_function_utility_time_of_day() {
    let context = EvaluationContext::new_empty();
    let result = eval("timeOfDay()", &context);
    // Check it's a Time
    assert!(matches!(result, EvaluationResult::Time(_)));
    // Check determinism
    let expr = parser().parse("timeOfDay() = timeOfDay()").unwrap();
    assert_eq!(evaluate(&expr, &context, None), EvaluationResult::Boolean(true));
}

// Spec: https://hl7.org/fhirpath/2025Jan/#today--date
#[test]
fn test_function_utility_today() {
    let context = EvaluationContext::new_empty();
    let result = eval("today()", &context);
    // Check it's a Date
    assert!(matches!(result, EvaluationResult::Date(_)));
    // Check determinism
    let expr = parser().parse("today() = today()").unwrap();
    assert_eq!(evaluate(&expr, &context, None), EvaluationResult::Boolean(true));
}

// --- Operations ---

// --- Equality ---
// Spec: https://hl7.org/fhirpath/2025Jan/#-equals
#[test]
fn test_operator_equality_equals() {
    let context = EvaluationContext::new_empty();
    // Primitives
    assert_eq!(eval("1 = 1", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 = 2", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("1 = 1.0", &context), EvaluationResult::Boolean(true)); // Integer vs Decimal equality
    assert_eq!(eval("1.0 = 1", &context), EvaluationResult::Boolean(true)); // Decimal vs Integer equality
    assert_eq!(
        eval("1.0 = 1.0", &context),
        EvaluationResult::Boolean(true)
    ); // Decimal vs Decimal
    assert_eq!(
        eval("1.0 = 2.0", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(eval("'a' = 'a'", &context), EvaluationResult::Boolean(true));
    assert_eq!(
        eval("'a' = 'b'", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("true = true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true = false", &context),
        EvaluationResult::Boolean(false)
    );
    // Dates/Times (assuming string representation for now)
    assert_eq!(
        eval("@2023-10-27 = @2023-10-27", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27 = @2023-10-28", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("@T10:30 = @T10:30", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@T10:30 = @T11:00", &context),
        EvaluationResult::Boolean(false)
    );
    // Collections
    assert_eq!(
        eval("(1|2) = (1|2)", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1|2) = (2|1)", &context),
        EvaluationResult::Boolean(false)
    ); // Order matters
    assert_eq!(
        eval("(1|2) = (1|2|3)", &context),
        EvaluationResult::Boolean(false)
    ); // Different count
    assert_eq!(
        eval("(1|1) = (1|1)", &context),
        EvaluationResult::Boolean(true)
    );
    // Empty propagation
    assert_eq!(eval("{} = {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("1 = {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} = 1", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#-equivalent
#[test]
fn test_operator_equality_equivalent() {
    let context = EvaluationContext::new_empty();
    // Primitives
    assert_eq!(eval("1 ~ 1", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 ~ 2", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("1 ~ 1.0", &context), EvaluationResult::Boolean(true)); // Integer vs Decimal equivalence
    assert_eq!(eval("1.0 ~ 1", &context), EvaluationResult::Boolean(true)); // Decimal vs Integer equivalence
    assert_eq!(
        eval("1.0 ~ 1.0", &context),
        EvaluationResult::Boolean(true)
    ); // Decimal vs Decimal
    assert_eq!(
        eval("1.0 ~ 2.0", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(eval("'a' ~ 'a'", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("'a' ~ 'A'", &context), EvaluationResult::Boolean(true)); // Case-insensitive
    assert_eq!(
        eval("'a' ~ 'b'", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'a b' ~ 'a   b'", &context),
        EvaluationResult::Boolean(true)
    ); // Whitespace normalized
    assert_eq!(
        eval("true ~ true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true ~ false", &context),
        EvaluationResult::Boolean(false)
    );
    // Dates/Times (assuming string representation for now)
    assert_eq!(
        eval("@2023-10-27 ~ @2023-10-27", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27 ~ @2023-10-28", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("@T10:30 ~ @T10:30", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@T10:30 ~ @T11:00", &context),
        EvaluationResult::Boolean(false)
    );
    // Collections
    assert_eq!(
        eval("(1|2) ~ (1|2)", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1|2) ~ (2|1)", &context),
        EvaluationResult::Boolean(true)
    ); // Order doesn't matter
    assert_eq!(
        eval("(1|2) ~ (1|2|3)", &context),
        EvaluationResult::Boolean(false)
    ); // Different count
    assert_eq!(
        eval("(1|1) ~ (1)", &context),
        EvaluationResult::Boolean(false)
    ); // Duplicates matter for count
    assert_eq!(
        eval("(1|2|1) ~ (1|1|2)", &context),
        EvaluationResult::Boolean(true)
    );
    // Empty comparison
    assert_eq!(eval("{} ~ {}", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 ~ {}", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("{} ~ 1", &context), EvaluationResult::Boolean(false));
}

// Spec: https://hl7.org/fhirpath/2025Jan/#-not-equals
#[test]
fn test_operator_equality_not_equals() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("1 != 2", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 != 1", &context), EvaluationResult::Boolean(false));
    assert_eq!(
        eval("(1|2) != (1|3)", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1|2) != (1|2)", &context),
        EvaluationResult::Boolean(false)
    );
    // Empty propagation
    assert_eq!(eval("{} != {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("1 != {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} != 1", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#-not-equivalent
#[test]
fn test_operator_equality_not_equivalent() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("1 !~ 2", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 !~ 1", &context), EvaluationResult::Boolean(false));
    assert_eq!(
        eval("'a' !~ 'A'", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1|2) !~ (1|3)", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1|2) !~ (2|1)", &context),
        EvaluationResult::Boolean(false)
    );
    // Empty comparison
    assert_eq!(eval("{} !~ {}", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("1 !~ {}", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("{} !~ 1", &context), EvaluationResult::Boolean(true));
}

// --- Comparison ---
// Spec: https://hl7.org/fhirpath/2025Jan/#comparison
#[test]
fn test_operator_comparison() {
    let context = EvaluationContext::new_empty();
    // >, <, >=, <=
    assert_eq!(eval("2 > 1", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 > 1", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("1 > 2", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("1 < 2", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 < 1", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("2 < 1", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("1 >= 1", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("2 >= 1", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 >= 2", &context), EvaluationResult::Boolean(false));
    assert_eq!(eval("1 <= 1", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 <= 2", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("2 <= 1", &context), EvaluationResult::Boolean(false));
    // String comparison
    assert_eq!(eval("'b' > 'a'", &context), EvaluationResult::Boolean(true));
    assert_eq!(
        eval("'a' > 'a'", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'a' > 'b'", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(eval("'a' < 'b'", &context), EvaluationResult::Boolean(true));
    // Implicit conversion
    assert_eq!(eval("2 > 1.5", &context), EvaluationResult::Boolean(true));
    assert_eq!(eval("1.5 < 2", &context), EvaluationResult::Boolean(true)); // Decimal < Integer
    assert_eq!(eval("2 > 1.5", &context), EvaluationResult::Boolean(true)); // Integer > Decimal
    assert_eq!(eval("1 <= 1.0", &context), EvaluationResult::Boolean(true)); // Integer <= Decimal
    assert_eq!(eval("1.0 >= 1", &context), EvaluationResult::Boolean(true)); // Decimal >= Integer
    // Empty propagation
    assert_eq!(eval("1 > {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} > 1", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} > {}", &context), EvaluationResult::Empty);
    // Date/Time (assuming string representation)
    assert_eq!(
        eval("@2023-10-27 > @2023-10-26", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27 < @2023-10-28", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@T10:30 >= @T10:30", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@T10:30 <= @T11:00", &context),
        EvaluationResult::Boolean(true)
    );
}

// --- Types ---
// Spec: https://hl7.org/fhirpath/2025Jan/#is-type-specifier
#[test]
fn test_operator_types_is() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("1 is Integer", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("1 is String", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'a' is String", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'a' is Integer", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("true is Boolean", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("1.0 is Decimal", &context),
        EvaluationResult::Boolean(true) // Check Decimal type
    );
    assert_eq!(
        eval("@2023 is Date", &context),
        EvaluationResult::Boolean(true)
    ); // Assuming parser tags type
    assert_eq!(
        eval("{} is Integer", &context),
        EvaluationResult::Boolean(false)
    ); // Empty is not Integer
    // Test 'System' namespace explicitly if needed by implementation
    assert_eq!(
        eval("1 is System.Integer", &context),
        EvaluationResult::Boolean(true)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#as-type-specifier
#[test]
fn test_operator_types_as() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("1 as Integer", &context), EvaluationResult::Integer(1));
    assert_eq!(
        eval("'a' as String", &context),
        EvaluationResult::String("a".to_string())
    );
    assert_eq!(
        eval("1.0 as Decimal", &context),
        EvaluationResult::Decimal(dec!(1.0))
    ); // 'as' Decimal
    assert_eq!(eval("1 as String", &context), EvaluationResult::Empty); // Cannot cast Integer to String
    assert_eq!(eval("'a' as Integer", &context), EvaluationResult::Empty); // Cannot cast String to Integer
    assert_eq!(eval("1 as Decimal", &context), EvaluationResult::Empty); // Cannot cast Integer to Decimal via 'as' (per spec)
    assert_eq!(eval("1.0 as Integer", &context), EvaluationResult::Empty); // Cannot cast Decimal to Integer via 'as'
    assert_eq!(eval("{} as Integer", &context), EvaluationResult::Empty);
    // Test 'System' namespace explicitly
    assert_eq!(
        eval("1 as System.Integer", &context),
        EvaluationResult::Integer(1)
    );
}

// --- Collections ---
// Spec: https://hl7.org/fhirpath/2025Jan/#-union-collections
#[test]
fn test_operator_collections_union() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{} | {}", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("(1 | 2) | {}", &context),
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2)
        ])
    ); // Order not guaranteed
    assert_eq!(
        eval("{} | (1 | 2)", &context),
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2)
        ])
    ); // Order not guaranteed
    // Order not guaranteed, check contents
    let result = eval("(1 | 2 | 3) | (2 | 3 | 4)", &context);
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers"),
            })
            .collect();
        actual_items.sort();
        assert_eq!(actual_items, vec![1, 2, 3, 4]);
    } else {
        panic!("Expected collection result from union operator");
    }
}

// Spec: https://hl7.org/fhirpath/2025Jan/#in-membership
#[test]
fn test_operator_collections_in() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("1 in (1 | 2 | 3)", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("4 in (1 | 2 | 3)", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'a' in ('a' | 'b')", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'c' in ('a' | 'b')", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(eval("1 in {}", &context), EvaluationResult::Boolean(false)); // Item in empty collection
    assert_eq!(eval("{} in (1 | 2)", &context), EvaluationResult::Empty); // Empty item
    assert_eq!(eval("{} in {}", &context), EvaluationResult::Empty); // Empty item
}

// Spec: https://hl7.org/fhirpath/2025Jan/#contains-containership
#[test]
fn test_operator_collections_contains() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("(1 | 2 | 3) contains 1", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3) contains 4", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("('a' | 'b') contains 'a'", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("('a' | 'b') contains 'c'", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("{} contains 1", &context),
        EvaluationResult::Boolean(false)
    ); // Empty collection contains item
    assert_eq!(
        eval("(1 | 2) contains {}", &context),
        EvaluationResult::Empty
    ); // Contains empty item
    assert_eq!(eval("{} contains {}", &context), EvaluationResult::Empty); // Contains empty item
}

// --- Boolean Logic ---
// Spec: https://hl7.org/fhirpath/2025Jan/#and
#[test]
fn test_operator_boolean_and() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("true and true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true and false", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("false and true", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("false and false", &context),
        EvaluationResult::Boolean(false)
    );
    // Empty propagation
    assert_eq!(eval("true and {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} and true", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("false and {}", &context),
        EvaluationResult::Boolean(false)
    ); // Short circuit? Spec says no guarantee, but table shows false.
    assert_eq!(
        eval("{} and false", &context),
        EvaluationResult::Boolean(false)
    ); // Table shows false.
    assert_eq!(eval("{} and {}", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#or
#[test]
fn test_operator_boolean_or() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("true or true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true or false", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("false or true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("false or false", &context),
        EvaluationResult::Boolean(false)
    );
    // Empty propagation
    assert_eq!(
        eval("true or {}", &context),
        EvaluationResult::Boolean(true)
    ); // Table shows true.
    assert_eq!(
        eval("{} or true", &context),
        EvaluationResult::Boolean(true)
    ); // Table shows true.
    assert_eq!(eval("false or {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} or false", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} or {}", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#xor
#[test]
fn test_operator_boolean_xor() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("true xor true", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("true xor false", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("false xor true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("false xor false", &context),
        EvaluationResult::Boolean(false)
    );
    // Empty propagation
    assert_eq!(eval("true xor {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} xor true", &context), EvaluationResult::Empty);
    assert_eq!(eval("false xor {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} xor false", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} xor {}", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#implies
#[test]
fn test_operator_boolean_implies() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("true implies true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true implies false", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("false implies true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("false implies false", &context),
        EvaluationResult::Boolean(true)
    );
    // Empty propagation
    assert_eq!(eval("true implies {}", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("{} implies true", &context),
        EvaluationResult::Boolean(true)
    ); // Table shows true
    assert_eq!(
        eval("false implies {}", &context),
        EvaluationResult::Boolean(true)
    ); // Short circuit
    assert_eq!(eval("{} implies false", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} implies {}", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#not--boolean (Function, but often used like operator)
#[test]
fn test_function_boolean_not() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("true.not()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("false.not()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(eval("{}.not()", &context), EvaluationResult::Empty);
}

// --- Math ---
// Spec: https://hl7.org/fhirpath/2025Jan/#-multiplication
#[test]
fn test_operator_math_multiply() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("2 * 3", &context),
        EvaluationResult::Integer(6) // Result is Integer
    );
    assert_eq!(
        eval("2.5 * 2", &context), // Decimal * Integer -> Decimal
        EvaluationResult::Decimal(dec!(5.0))
    ); // Decimal * Integer -> Decimal
    assert_eq!(
        eval("2 * 2.5", &context),
        EvaluationResult::Decimal(dec!(5.0))
    ); // Integer * Decimal -> Decimal
    assert_eq!(
        eval("2.5 * 2.0", &context),
        EvaluationResult::Decimal(dec!(5.0))
    ); // Decimal * Decimal -> Decimal
    // Empty propagation
    assert_eq!(eval("2 * {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} * 3", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#-division
#[test]
fn test_operator_math_divide() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("6 / 2", &context),
        EvaluationResult::Decimal(dec!(3.0)) // Integer / Integer -> Decimal (explicit .0)
    );
    assert_eq!(
        eval("7 / 2", &context),
        EvaluationResult::Decimal(dec!(3.5))
    ); // Integer / Integer -> Decimal
    assert_eq!(
        eval("5.0 / 2", &context),
        EvaluationResult::Decimal(dec!(2.5))
    ); // Decimal / Integer -> Decimal
    assert_eq!(
        eval("5 / 2.0", &context),
        EvaluationResult::Decimal(dec!(2.5))
    ); // Integer / Decimal -> Decimal
    assert_eq!(
        eval("5.0 / 2.0", &context),
        EvaluationResult::Decimal(dec!(2.5))
    ); // Decimal / Decimal -> Decimal
    // Divide by zero
    assert_eq!(eval("5 / 0", &context), EvaluationResult::Empty); // Integer / 0
    assert_eq!(eval("5.0 / 0", &context), EvaluationResult::Empty);
    assert_eq!(eval("5 / 0.0", &context), EvaluationResult::Empty);
    // Empty propagation
    assert_eq!(eval("6 / {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} / 2", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#-addition
#[test]
fn test_operator_math_add() {
    let context = EvaluationContext::new_empty();
    // Numbers - All result in Decimal
    assert_eq!(
        eval("1 + 2", &context),
        EvaluationResult::Decimal(dec!(3.0)) // Integer + Integer -> Decimal
    );
    assert_eq!(
        eval("1.5 + 2", &context), // Decimal + Integer -> Decimal
        EvaluationResult::Decimal(dec!(3.5))
    ); // Decimal + Integer -> Decimal
    assert_eq!(
        eval("1 + 2.5", &context),
        EvaluationResult::Decimal(dec!(3.5))
    ); // Integer + Decimal -> Decimal
    assert_eq!(
        eval("1.5 + 2.0", &context),
        EvaluationResult::Decimal(dec!(3.5))
    ); // Decimal + Decimal -> Decimal
    // Strings
    assert_eq!(
        eval("'a' + 'b'", &context),
        EvaluationResult::String("ab".to_string())
    );
    assert_eq!(
        eval("'a' + ' ' + 'b'", &context),
        EvaluationResult::String("a b".to_string())
    );
    // Empty propagation
    assert_eq!(eval("1 + {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} + 2", &context), EvaluationResult::Empty);
    assert_eq!(eval("'a' + {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} + 'b'", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#--subtraction
#[test]
fn test_operator_math_subtract() {
    let context = EvaluationContext::new_empty();
    // All result in Decimal
    assert_eq!(
        eval("5 - 3", &context),
        EvaluationResult::Decimal(dec!(2.0)) // Integer - Integer -> Decimal
    );
    assert_eq!(
        eval("5.5 - 3", &context), // Decimal - Integer -> Decimal
        EvaluationResult::Decimal(dec!(2.5))
    ); // Decimal - Integer -> Decimal
    assert_eq!(
        eval("5 - 3.5", &context),
        EvaluationResult::Decimal(dec!(1.5))
    ); // Integer - Decimal -> Decimal
    assert_eq!(
        eval("5.5 - 3.0", &context),
        EvaluationResult::Decimal(dec!(2.5))
    ); // Decimal - Decimal -> Decimal
    // Empty propagation
    assert_eq!(eval("5 - {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} - 3", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#div
#[test]
fn test_operator_math_div() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("5 div 2", &context), EvaluationResult::Integer(2)); // Integer div Integer -> Integer
    assert_eq!(eval("6 div 2", &context), EvaluationResult::Integer(3));
    assert_eq!(eval("-5 div 2", &context), EvaluationResult::Integer(-2));
    // Decimal div Decimal -> Integer (truncates)
    assert_eq!(
        eval("5.5 div 2.1", &context),
        EvaluationResult::Integer(2)
    );
    assert_eq!(
        eval("-5.5 div 2.1", &context),
        EvaluationResult::Integer(-2)
    );
    // Mixed types for div -> Empty
    assert_eq!(eval("5.5 div 2", &context), EvaluationResult::Empty);
    assert_eq!(eval("5 div 2.1", &context), EvaluationResult::Empty);
    // Divide by zero
    assert_eq!(eval("5 div 0", &context), EvaluationResult::Empty); // Integer div 0
    assert_eq!(eval("5.0 div 0", &context), EvaluationResult::Empty);
    // Empty propagation
    assert_eq!(eval("5 div {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} div 2", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#mod
#[test]
fn test_operator_math_mod() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("5 mod 2", &context), EvaluationResult::Integer(1)); // Integer mod Integer -> Integer
    assert_eq!(eval("6 mod 2", &context), EvaluationResult::Integer(0));
    assert_eq!(eval("-5 mod 2", &context), EvaluationResult::Integer(-1)); // Result has sign of dividend
    // Decimal mod Decimal -> Decimal
    assert_eq!(
        eval("5.5 mod 2.1", &context),
        EvaluationResult::Decimal(dec!(1.3))
    );
    assert_eq!(
        eval("-5.5 mod 2.1", &context),
        EvaluationResult::Decimal(dec!(-1.3)) // Result has sign of dividend
    );
     // Mixed types for mod -> Empty
    assert_eq!(eval("5.5 mod 2", &context), EvaluationResult::Empty);
    assert_eq!(eval("5 mod 2.1", &context), EvaluationResult::Empty);
    // Modulo zero
    assert_eq!(eval("5 mod 0", &context), EvaluationResult::Empty); // Integer mod 0
    assert_eq!(eval("5.0 mod 0", &context), EvaluationResult::Empty);
    // Empty propagation
    assert_eq!(eval("5 mod {}", &context), EvaluationResult::Empty);
    assert_eq!(eval("{} mod 2", &context), EvaluationResult::Empty);
}

// Spec: https://hl7.org/fhirpath/2025Jan/#-string-concatenation
#[test]
fn test_operator_math_string_concat() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("'a' & 'b'", &context),
        EvaluationResult::String("ab".to_string())
    );
    assert_eq!(
        eval("'a' & ' ' & 'b'", &context),
        EvaluationResult::String("a b".to_string())
    );
    // Empty treated as empty string
    assert_eq!(
        eval("'a' & {}", &context),
        EvaluationResult::String("a".to_string())
    );
    assert_eq!(
        eval("{} & 'b'", &context),
        EvaluationResult::String("b".to_string())
    );
    assert_eq!(
        eval("{} & {}", &context),
        EvaluationResult::String("".to_string())
    );
    assert_eq!(
        eval("'a' & {} & 'c'", &context),
        EvaluationResult::String("ac".to_string())
    );
}

// --- Operator Precedence ---
#[test]
fn test_operator_precedence() {
    let context = EvaluationContext::new_empty();
    // Results depend on operators
    // 1 + (2 * 3) = 1 + 6 = 7.0 (Addition -> Decimal)
    assert_eq!(
        eval("1 + 2 * 3", &context),
        EvaluationResult::Decimal(dec!(7.0))
    );
    // (1 + 2) * 3 = 3.0 * 3 = 9.0 (Addition -> Decimal, then Decimal * Integer -> Decimal)
    assert_eq!(
        eval("(1 + 2) * 3", &context),
        EvaluationResult::Decimal(dec!(9.0))
    );
    // (5 - 2) + 1 = 3.0 + 1 = 4.0 (Subtraction -> Decimal, then Decimal + Integer -> Decimal)
    assert_eq!(
        eval("5 - 2 + 1", &context),
        EvaluationResult::Decimal(dec!(4.0))
    );
    // (10 / 2) * 5 = 5.0 * 5 = 25.0 (Division -> Decimal, then Decimal * Integer -> Decimal)
    assert_eq!(
        eval("10 / 2 * 5", &context),
        EvaluationResult::Decimal(dec!(25.0))
    );
     // (10 div 3) * 2 = 3 * 2 = 6 (div -> Integer, then Integer * Integer -> Integer)
    assert_eq!(
        eval("10 div 3 * 2", &context),
        EvaluationResult::Integer(6)
    );
     // (10 mod 3) + 1 = 1 + 1 = 2.0 (mod -> Integer, then Integer + Integer -> Decimal)
    assert_eq!(
        eval("10 mod 3 + 1", &context),
        EvaluationResult::Decimal(dec!(2.0))
    );
    assert_eq!(
        eval("true or false and false", &context), // 'and' before 'or'
        EvaluationResult::Boolean(true)
    ); // 'and' before 'or'
    assert_eq!(
        eval("(true or false) and false", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(true or false) and false", &context), // Parentheses
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("1 < 2 and 3 > 2", &context), // Comparison before 'and'
        EvaluationResult::Boolean(true)
    );
    // (-1) + 5 = 4.0 (Unary minus, then Integer + Integer -> Decimal)
    assert_eq!(
        eval("-1 + 5", &context),
        EvaluationResult::Decimal(dec!(4.0))
    );
     // -(1 + 5) = -(6.0) = -6.0 (Addition -> Decimal, then Unary minus)
    assert_eq!(
        eval("-(1 + 5)", &context),
        EvaluationResult::Decimal(dec!(-6.0))
    );
    // assert_eq!(eval("Patient.name[0].given", &context), EvaluationResult::Empty); // Indexer before path (needs context)
    // Add more complex precedence tests as needed
}

// --- Environment Variables ---
// Spec: https://hl7.org/fhirpath/2025Jan/#environment-variables
#[test]
fn test_environment_variables() {
    let mut context = EvaluationContext::new_empty();
    context.set_variable("name", "John Doe".to_string()); // Pass &str for name
    context.set_variable("age", "42".to_string()); // Pass &str for name, String for value
    context.set_variable("myVar", "true".to_string()); // Pass &str for name, String for value
    // Delimited variable name - parser handles this, stores as "my-Var"
    context.set_variable("my-Var", "special".to_string()); // Pass &str for name, String for value

    assert_eq!(
        eval("%name", &context),
        EvaluationResult::String("John Doe".to_string())
    );
    assert_eq!(eval("%age + 1", &context), EvaluationResult::Integer(43));
    assert_eq!(
        eval("%myVar and true", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("%`my-Var`", &context),
        EvaluationResult::String("special".to_string())
    );

    // Accessing undefined variable should return empty
    assert_eq!(eval("%undefinedVar", &context), EvaluationResult::Empty);

    // %context (needs resource context)
    let patient = r4::Patient {
        id: Some("p1".to_string().into()), // Use .to_string().into()
        ..Default::default()
    };
    let ctx_res = EvaluationContext::new(vec![FhirResource::R4(Box::new(
        r4::Resource::Patient(patient.clone()), // Wrap in Resource enum
    ))]); // Pass resource vec

    // %context should be set automatically by evaluate()
    let context_var = ctx_res.get_variable_as_result("context"); // Use get_variable_as_result
    // Remove .expect(), check the result directly
    assert!(
        !matches!(context_var, EvaluationResult::Empty),
        "%context should be set"
    );
    assert!(matches!(context_var, EvaluationResult::Object(_)));

    // Test accessing %context implicitly at start of path
    // assert_eq!(eval("id", &ctx_res), EvaluationResult::String("p1".to_string())); // Requires member access

    // Test accessing %context explicitly
    // assert_eq!(eval("%context.id", &ctx_res), EvaluationResult::String("p1".to_string())); // Requires member access
}

// --- Resource Access Tests ---
// These depend heavily on the fhir crate's IntoEvaluationResult implementation

// Removed unused HashMap import

// Helper to create a patient context
fn patient_context() -> EvaluationContext {
    let patient = r4::Patient {
        id: Some("p1".to_string().into()), // Resource ID - Use .to_string().into()
        identifier: Some(vec![r4::Identifier { // Wrap in Some()
            r#use: Some(Code { // Use imported Code
                value: Some("usual".to_string()),
                ..Default::default()
            }),
            system: Some("urn:oid:1.2.3.4".to_string().into()), // Use .to_string().into()
            value: Some("12345".to_string().into()), // Use .to_string().into()
            ..Default::default()
        }]),
        active: Some(Boolean { // Use imported Boolean
            // Element with value
            id: Some("active-id".to_string().into()), // Element ID - Use .to_string().into()
            value: Some(true),
            ..Default::default()
        }),
        name: Some(vec![ // Wrap in Some()
            r4::HumanName {
                // Official Name
                id: Some("name1".to_string().into()), // Use .to_string().into()
                r#use: Some(Code { // Use imported Code
                    value: Some("official".to_string()),
                    ..Default::default()
                }),
                family: Some("Doe".to_string().into()), // Use .to_string().into()
                given: Some(vec![ // Wrap in Some()
                    FhirString { // Use imported FhirString
                        value: Some("John".to_string()),
                        ..Default::default()
                    }, // Element<String>
                    FhirString { // Use imported FhirString
                        id: Some("given2-id".to_string().into()), // Use .to_string().into()
                        value: Some("Middle".to_string()),
                        ..Default::default()
                    }, // Element with ID
                ]),
                ..Default::default()
            },
            r4::HumanName {
                // Usual Name (no family)
                id: Some("name2".to_string().into()), // Use .to_string().into()
                r#use: Some(Code { // Use imported Code
                    value: Some("usual".to_string()),
                    ..Default::default()
                }),
                given: Some(vec!["Johnny".to_string().into()]), // Wrap in Some(), use .to_string().into()
                ..Default::default()
            },
            r4::HumanName {
                // Anonymous Name (no use, no id)
                family: Some("Smith".to_string().into()), // Use .to_string().into()
                given: Some(vec!["Jane".to_string().into()]), // Wrap in Some(), use .to_string().into()
                ..Default::default()
            },
        ]),
        telecom: Some(vec![ // Wrap in Some()
            r4::ContactPoint {
                system: Some(Code { // Use imported Code
                    value: Some("phone".to_string()),
                    ..Default::default()
                }),
                value: Some("555-1234".to_string().into()), // Use .to_string().into()
                ..Default::default()
            },
            r4::ContactPoint {
                system: Some(Code { // Use imported Code
                    value: Some("email".to_string()),
                    ..Default::default()
                }),
                value: Some("john.doe@example.com".to_string().into()), // Use .to_string().into()
                ..Default::default()
            },
        ]),
        birth_date: Some(Date { // Use imported Date
            // Element with value and extension
            id: Some("birthdate-id".to_string().into()), // Use .to_string().into()
            value: Some("1980-05-15".to_string()),
            extension: Some(vec![Extension { // Use imported Extension, wrap in Some()
                url: "http://example.com/precision".to_string().into(), // Remove Some(), url is not Option
                value: Some(ExtensionValue::String("day".to_string().into())), // Use imported ExtensionValue, .to_string().into()
                ..Default::default()
            }]),
            ..Default::default()
        }),
        deceased: Some(r4::PatientDeceased::Boolean(Boolean { // Use imported Boolean
            value: Some(false),
            ..Default::default()
        })), // DeceasedBoolean (Element)
        ..Default::default()
    };
    EvaluationContext::new(vec![FhirResource::R4(Box::new(r4::Resource::Patient( // Wrap in Resource::Patient
        patient,
    )))])
}

#[test]
#[ignore = "Requires full IntoEvaluationResult for Patient"] // Re-enable
fn test_resource_simple_field_access() {
    let context = patient_context();
    assert_eq!(
        eval("id", &context),
        EvaluationResult::String("p1".to_string())
    );
    assert_eq!(eval("active", &context), EvaluationResult::Boolean(true));
    assert_eq!(
        eval("birthDate", &context),
        EvaluationResult::Date("1980-05-15".to_string())
    ); // Use eval directly
    assert_eq!(eval("deceased", &context), EvaluationResult::Boolean(false)); // Accessing choice type value
    assert_eq!(
        eval("deceasedBoolean", &context),
        EvaluationResult::Boolean(false)
    ); // Accessing specific choice type name
    assert_eq!(eval("deceasedDateTime", &context), EvaluationResult::Empty); // Accessing non-existent choice type name
    assert_eq!(eval("nonExistentField", &context), EvaluationResult::Empty);
}

#[test]
#[ignore = "Requires full IntoEvaluationResult for Patient and list handling"] // Re-enable
fn test_resource_nested_field_access() {
    let context = patient_context();
    // Accessing a field within a list - returns a collection of that field from each list item
    let name_family = eval("name.family", &context);
    assert!(matches!(name_family, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(items) = name_family {
        assert_eq!(items.len(), 2); // Doe, Smith (usual name has no family)
        assert!(items.contains(&EvaluationResult::String("Doe".to_string())));
        assert!(items.contains(&EvaluationResult::String("Smith".to_string())));
    }

    let name_given = eval("name.given", &context);
    assert!(matches!(name_given, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(items) = name_given {
        assert_eq!(items.len(), 3); // John, Johnny, Jane
        assert!(items.contains(&EvaluationResult::String("John".to_string())));
        assert!(items.contains(&EvaluationResult::String("Johnny".to_string())));
        assert!(items.contains(&EvaluationResult::String("Jane".to_string())));
    }

    // Accessing a field that doesn't exist in all items
    let name_use = eval("name.use", &context); // official, usual, (empty for Smith)
    assert!(
        matches!(name_use, EvaluationResult::Collection(_)),
        "Expected Collection for name.use, got {:?}",
        name_use
    );
    if let EvaluationResult::Collection(items) = name_use {
        assert_eq!(items.len(), 2, "Expected 2 'use' values, got {:?}", items); // Only official and usual have 'use'
        assert!(items.contains(&EvaluationResult::String("official".to_string())));
        assert!(items.contains(&EvaluationResult::String("usual".to_string())));
    }

    // Access element id
    assert_eq!(
        eval("active.id", &context),
        EvaluationResult::String("active-id".to_string())
    );
    assert_eq!(
        eval("birthDate.id", &context),
        EvaluationResult::String("birthdate-id".to_string())
    );
    let name_ids = eval("name.id", &context); // name1, name2, (empty for Smith)
    assert!(
        matches!(name_ids, EvaluationResult::Collection(_)),
        "Expected Collection for name.id, got {:?}",
        name_ids
    );
    if let EvaluationResult::Collection(items) = name_ids {
        assert_eq!(items.len(), 2);
        assert!(items.contains(&EvaluationResult::String("name1".to_string())));
        assert!(items.contains(&EvaluationResult::String("name2".to_string())));
    }
    let given_ids = eval("name.given.id", &context); // (empty for John), given2-id, (empty for Johnny), (empty for Jane)
    assert!(
        matches!(given_ids, EvaluationResult::String(_)),
        "Expected String for name.given.id, got {:?}",
        given_ids
    ); // Only one ID present
    assert_eq!(given_ids, EvaluationResult::String("given2-id".to_string()));

    // Access extension (basic check, requires Extension conversion)
    let bday_ext = eval("birthDate.extension", &context);
    assert!(
        matches!(bday_ext, EvaluationResult::Collection(_)),
        "Expected Collection for birthDate.extension, got {:?}",
        bday_ext
    );
    if let EvaluationResult::Collection(exts) = bday_ext {
        assert_eq!(exts.len(), 1);
        // Further checks require Extension object structure
        // assert_eq!(eval("birthDate.extension.url", &context), EvaluationResult::String("http://example.com/precision".to_string()));
        // assert_eq!(eval("birthDate.extension.valueString", &context), EvaluationResult::String("day".to_string()));
    }
}

#[test]
#[ignore = "Requires full IntoEvaluationResult, expression passing, and list handling"] // Re-enable
fn test_resource_filtering_and_projection() {
    let context = patient_context();

    // Where on a list field
    let official_name = eval("name.where(use = 'official')", &context);
    assert!(matches!(official_name, EvaluationResult::Object(_))); // Should return the HumanName object

    // Select from the filtered list
    assert_eq!(
        eval("name.where(use = 'official').family", &context),
        EvaluationResult::String("Doe".to_string())
    );
    assert_eq!(
        eval("name.where(use = 'usual').given", &context),
        EvaluationResult::String("Johnny".to_string())
    );
    assert_eq!(
        eval("name.where(family = 'Smith').given", &context),
        EvaluationResult::String("Jane".to_string())
    );

    // Select multiple fields
    let official_details = eval(
        "name.where(use = 'official').select(given + ' ' + family)",
        &context,
    );
    assert_eq!(
        official_details,
        EvaluationResult::String("John Doe".to_string())
    );

    // Select on a non-list field (acts on the single item)
    assert_eq!(
        eval("birthDate.select($this.toString())", &context),
        EvaluationResult::String("1980-05-15".to_string())
    );

    // Where on root context
    assert_eq!(
        eval("%context.where(active = true).id", &context),
        EvaluationResult::String("p1".to_string())
    );
    assert_eq!(
        eval("%context.where(active = false).id", &context),
        EvaluationResult::Empty
    );
}

#[test]
#[ignore = "Requires full IntoEvaluationResult and TypeSpecifier passing"] // Re-enable
fn test_resource_oftype() {
    let patient = r4::Patient {
        id: Some("p1".to_string().into()), // Use .to_string().into()
        ..Default::default()
    };
    let observation = r4::Observation {
        id: Some("o1".to_string().into()), // Use .to_string().into()
        ..Default::default()
    };
    let resources = vec![
        FhirResource::R4(Box::new(r4::Resource::Patient(patient))),
        FhirResource::R4(Box::new(r4::Resource::Observation(observation))),
    ];
    let context = EvaluationContext::new(resources);

    let patients = eval("%context.ofType(Patient)", &context);
    assert!(matches!(patients, EvaluationResult::Object(_))); // Only one patient
    if let EvaluationResult::Object(fields) = patients {
        assert_eq!(
            fields.get("resourceType"),
            Some(&EvaluationResult::String("Patient".to_string()))
        );
        assert_eq!(
            fields.get("id"),
            Some(&EvaluationResult::String("p1".to_string()))
        );
    }

    let observations = eval("%context.ofType(Observation)", &context);
    assert!(matches!(observations, EvaluationResult::Object(_))); // Only one observation
    if let EvaluationResult::Object(fields) = observations {
        assert_eq!(
            fields.get("resourceType"),
            Some(&EvaluationResult::String("Observation".to_string()))
        );
        assert_eq!(
            fields.get("id"),
            Some(&EvaluationResult::String("o1".to_string()))
        );
    }

    assert_eq!(
        eval("%context.ofType(Practitioner)", &context),
        EvaluationResult::Empty
    );
}

#[test]
fn test_arithmetic_operations() {
    // Note: Result types vary based on operator and operands
    let test_cases = vec![
        ("1 + 2", EvaluationResult::Decimal(dec!(3.0))), // Addition -> Decimal
        ("5 - 3", EvaluationResult::Decimal(dec!(2.0))), // Subtraction -> Decimal
        ("2 * 3", EvaluationResult::Integer(6)),         // Integer Multiplication -> Integer
        ("6 / 2", EvaluationResult::Decimal(dec!(3.0))), // Division -> Decimal
        ("7 / 2", EvaluationResult::Decimal(dec!(3.5))), // Division -> Decimal
        ("7 div 2", EvaluationResult::Integer(3)),       // Integer div -> Integer
        ("7 mod 2", EvaluationResult::Integer(1)),       // Integer mod -> Integer
        ("5.5 + 2.1", EvaluationResult::Decimal(dec!(7.6))), // Decimal Add -> Decimal
        ("5.5 - 2.1", EvaluationResult::Decimal(dec!(3.4))), // Decimal Sub -> Decimal
        ("5.5 * 2.0", EvaluationResult::Decimal(dec!(11.0))), // Decimal Mult -> Decimal
        ("5.5 / 2.0", EvaluationResult::Decimal(dec!(2.75))), // Decimal Div -> Decimal
        ("5.5 div 2.1", EvaluationResult::Integer(2)),       // Decimal div -> Integer
        ("5.5 mod 2.1", EvaluationResult::Decimal(dec!(1.3))), // Decimal mod -> Decimal
        // Mixed type div/mod -> Empty
        ("5.5 div 2", EvaluationResult::Empty), // Decimal div Integer -> Empty
        ("5 div 2.1", EvaluationResult::Empty),
        ("5.5 mod 2", EvaluationResult::Empty),
        ("5 mod 2.1", EvaluationResult::Empty),
        // Division by zero -> Empty
        ("5 / 0", EvaluationResult::Empty),
        ("5.0 / 0", EvaluationResult::Empty),
        ("5 div 0", EvaluationResult::Empty),
        ("5.0 div 0", EvaluationResult::Empty),
        ("5 mod 0", EvaluationResult::Empty),
        ("5.0 mod 0", EvaluationResult::Empty),
    ];

    // For arithmetic operations, we don't need any resources
    let context = EvaluationContext::new_empty();

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context, None);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_boolean_operations() {
    let test_cases = vec![
        ("true and true", EvaluationResult::Boolean(true)),
        ("true and false", EvaluationResult::Boolean(false)),
        ("true or false", EvaluationResult::Boolean(true)),
        ("false or false", EvaluationResult::Boolean(false)),
        ("true xor false", EvaluationResult::Boolean(true)),
        ("true xor true", EvaluationResult::Boolean(false)),
        ("false implies true", EvaluationResult::Boolean(true)),
        ("true implies false", EvaluationResult::Boolean(false)),
    ];

    // For boolean operations, we don't need any resources
    let context = EvaluationContext::new_empty();

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        println!("Boolean op parsed expression: {:?}", expr);
        let result = evaluate(&expr, &context, None);
        println!("Boolean op result: {:?}, Expected: {:?}", result, expected);
        assert_eq!(result, expected, "Failed for input: {}", input);
    }
}

#[test]
fn test_comparison_operations() {
    let test_cases = vec![
        ("1 < 2", EvaluationResult::Boolean(true)),
        ("2 <= 2", EvaluationResult::Boolean(true)),
        ("3 > 2", EvaluationResult::Boolean(true)),
        ("3 >= 3", EvaluationResult::Boolean(true)),
        ("1 = 1", EvaluationResult::Boolean(true)),
        ("1 != 2", EvaluationResult::Boolean(true)),
        ("'abc' ~ 'ABC'", EvaluationResult::Boolean(true)),
        ("'abc' !~ 'def'", EvaluationResult::Boolean(true)),
    ];

    // For comparison operations, we don't need any resources
    let context = EvaluationContext::new_empty();

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        let result = evaluate(&expr, &context, None);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_variable_access() {
    // We'll set up the context without any resources
    let mut context = EvaluationContext::new_empty();

    // For testing variable access, we'll add some variables to the context
    context.set_variable("name", "John Doe".to_string());
    context.set_variable("age", "42".to_string()); // Store as string, FHIRPath handles conversion if needed

    let test_cases = vec![
        // Access variables directly
        ("%name", EvaluationResult::String("John Doe".to_string())),
        // Accessing %age should return the string value stored
        ("%age", EvaluationResult::String("42".to_string())),
        // Test conversion within expression
        ("%age.toInteger()", EvaluationResult::Integer(42)),
        ("%address", EvaluationResult::Empty), // Non-existent variable
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        println!("Variable access parsed expression: {:?}", expr);
        let result = evaluate(&expr, &context, None);
        println!(
            "Variable access result: {:?}, Expected: {:?}",
            result, expected
        );
        assert_eq!(result, expected, "Failed for input: {}", input);
    }
}

#[test]
fn test_string_operations() {
    // We'll set up the context without any resources
    let mut context = EvaluationContext::new_empty();

    // For testing string operations, we'll add a string variable
    context.set_variable("message", "Hello, World!".to_string());

    let test_cases = vec![
        // String contains operation with function call syntax
        (
            "'Hello, World!'.contains('World')",
            EvaluationResult::Boolean(true),
        ),
        (
            "'Hello, World!'.contains('Goodbye')",
            EvaluationResult::Boolean(false),
        ),
        (
            "%message.contains('World')",
            EvaluationResult::Boolean(true),
        ),
    ];

    for (input, expected) in test_cases {
        let expr = parser().parse(input).unwrap();
        println!("String operation parsed expression: {:?}", expr);
        let result = evaluate(&expr, &context, None);
        println!(
            "String operation result: {:?}, Expected: {:?}",
            result, expected
        );
        assert_eq!(result, expected, "Failed for input: {}", input);
    }
}

#[test]
fn test_functions() {
    // We'll set up the context without any resources
    let context = EvaluationContext::new_empty();

    // Test collection functions
    let test_cases = vec![
        // Empty collection
        (
            "{}".to_string(),
            "count()".to_string(),
            EvaluationResult::Integer(0),
        ),
        (
            "{}".to_string(),
            "empty()".to_string(),
            EvaluationResult::Boolean(true),
        ),
        (
            "{}".to_string(),
            "exists()".to_string(),
            EvaluationResult::Boolean(false),
        ),
        // Single item
        (
            "'test'".to_string(),
            "count()".to_string(),
            EvaluationResult::Integer(1),
        ),
        (
            "'test'".to_string(),
            "empty()".to_string(),
            EvaluationResult::Boolean(false),
        ),
        (
            "'test'".to_string(),
            "exists()".to_string(),
            EvaluationResult::Boolean(true),
        ),
        // String functions
        (
            "'Hello'".to_string(),
            "count()".to_string(),
            EvaluationResult::Integer(1),
        ),
        (
            "'Hello'".to_string(),
            "length()".to_string(),
            EvaluationResult::Integer(5),
        ),
        (
            "'Hello, World!'".to_string(),
            "contains('World')".to_string(),
            EvaluationResult::Boolean(true),
        ),
        (
            "'Hello, World!'".to_string(),
            "contains('Goodbye')".to_string(),
            EvaluationResult::Boolean(false),
        ),
    ];

    for (base, func, expected) in test_cases {
        let full_expr = if base == "{}" {
            func.clone()
        } else {
            format!("{}.{}", base, func)
        };

        println!("Testing expression: {}", full_expr);
        let expr = parser().parse(&*full_expr).unwrap();
        let result = evaluate(&expr, &context, None);
        println!("Function result: {:?}, Expected: {:?}", result, expected);
        assert_eq!(result, expected, "Failed for input: {}", full_expr);
    }
}

#[test]
fn test_direct_string_operations() {
    // We'll set up the context without any resources
    let context = EvaluationContext::new_empty();

    // Test string operations through the parser instead of direct function calls
    let expr = parser().parse("'Hello, World!'.contains('World')").unwrap();
    let result = evaluate(&expr, &context, None);
    assert_eq!(result, EvaluationResult::Boolean(true));

    let expr = parser()
        .parse("'Hello, World!'.contains('Goodbye')")
        .unwrap();
    let result = evaluate(&expr, &context, None);
    assert_eq!(result, EvaluationResult::Boolean(false));
}

#[test]
fn test_resource_access() {
    // Remove duplicate imports, they are already at the top level
    use fhir::r4::{Account, Code}; // Import only needed types locally if preferred, or rely on top-level
    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(Account { // Use imported Account
        id: Some("theid".to_string().into()), // Convert String to Id
        meta: None,
        implicit_rules: None,
        language: None,
        text: None,
        contained: None,
        extension: None,
        modifier_extension: None,
        identifier: None,
        status: Code { // Use imported Code
            id: None,
            extension: None,
            value: None,
        },
        r#type: None,
        name: None,
        subject: None,
        service_period: None,
        coverage: None,
        owner: None,
        description: None,
        guarantor: None,
        part_of: None,
    });

    // Create a context with a resource
    let resources = vec![FhirResource::R4(Box::new(dummy_resource))]; // No need for mut
    let context = EvaluationContext::new(resources);
    // Test accessing the resource id
    let expr = parser().parse("id").unwrap(); // Use 'id' to access the field
    let result = evaluate(&expr, &context, None);
    assert_eq!(result, EvaluationResult::String("theid".to_string())); // Expect the string value of the id
}
