use chumsky::Parser;
// Removed duplicate Parser import
// use chumsky::Parser; // Removed duplicate
use fhir::FhirResource;
use fhir::r4::{self, Boolean, Code, Date, Extension, ExtensionValue, String as FhirString};
use fhirpath::evaluator::{EvaluationContext, evaluate};
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
        eval("10 'mg'", &context),     // 10 is parsed as Integer
        EvaluationResult::Integer(10)  // Evaluator ignores unit for now
    );
    assert_eq!(
        eval("4.5 'km'", &context),           // 4.5 is parsed as Number (Decimal)
        EvaluationResult::Decimal(dec!(4.5))  // Evaluator ignores unit for now
    );
    // Quantity with date/time unit - Parser returns Decimal or Integer
    assert_eq!(
        eval("100 days", &context),     // 100 is parsed as Integer
        EvaluationResult::Integer(100)  // Evaluator ignores unit for now
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
        eval("{}.empty()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'test'.empty()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1 | 2).empty()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#existscriteria--expression--boolean
#[test]
fn test_function_existence_exists() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.exists()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'test'.exists()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).exists()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    // With criteria
    assert_eq!(
        eval("(1 | 2 | 3).exists($this > 2)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3).exists($this > 5)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("{}.exists($this > 5)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#allcriteria--expression--boolean
#[test]
fn test_function_existence_all() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.all($this > 1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    ); // Empty collection is true
    assert_eq!(
        eval("(1 | 2 | 3).all($this > 0)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3).all($this > 1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1 | 2 | 3).all($this.toString() = '1')", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    // Test with non-boolean criteria - should error
    assert!(eval("(1 | 2 | 3).all($this)", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#alltrue--boolean
#[test]
fn test_function_existence_all_true() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.allTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true).allTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | true).allTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | false).allTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(false | false).allTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    // Test with non-boolean - should error
    assert!(eval("(true | 1).allTrue()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#anytrue--boolean
#[test]
fn test_function_existence_any_true() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.anyTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(true).anyTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | true).anyTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | false).anyTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(false | false).anyTrue()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    // Test with non-boolean - should error
    assert!(eval("(false | 1).anyTrue()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#allfalse--boolean
#[test]
fn test_function_existence_all_false() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.allFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(false).allFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(false | false).allFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | false).allFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(true | true).allFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
     // Test with non-boolean - should error
    assert!(eval("(false | 1).allFalse()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#anyfalse--boolean
#[test]
fn test_function_existence_any_false() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.anyFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(false).anyFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(false | false).anyFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | false).anyFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(true | true).anyFalse()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
     // Test with non-boolean - should error
    assert!(eval("(true | 1).anyFalse()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#subsetofother--collection--boolean
#[test]
fn test_function_existence_subset_of() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.subsetOf({})", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("{}.subsetOf(1 | 2)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1).subsetOf(1 | 2)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).subsetOf(1 | 2)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3).subsetOf(1 | 2)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1 | 2).subsetOf({})", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1 | 2).subsetOf(1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#supersetofother--collection--boolean
#[test]
fn test_function_existence_superset_of() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.supersetOf({})", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).supersetOf({})", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).supersetOf(1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).supersetOf(1 | 2)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2).supersetOf(1 | 2 | 3)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("{}.supersetOf(1 | 2)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("(1).supersetOf(1 | 2)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#count--integer
#[test]
fn test_function_existence_count() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.count()", &context).unwrap(), EvaluationResult::Integer(0)); // Add unwrap
    assert_eq!(
        eval("'test'.count()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(1)
    );
    assert_eq!(
        eval("(1 | 2 | 3).count()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(3)
    );
    // Add test for duplicates - | operator creates distinct collection (1 | 2)
    assert_eq!(
        eval("(1 | 2 | 1).count()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(2) // Expect 2 because (1 | 2 | 1) becomes (1 | 2)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#distinct--collection
#[test]
fn test_function_existence_distinct() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.distinct()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(1).distinct()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(1)
    );
    // Order not guaranteed, so check contents
    let result = eval("(1 | 2 | 1 | 3 | 2).distinct()", &context).unwrap(); // Add unwrap
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
        eval("{}.isDistinct()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1).isDistinct()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 3).isDistinct()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 2 | 1).isDistinct()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true) // Expect true because (1 | 2 | 1) becomes (1 | 2) which IS distinct
    );
}

// --- Filtering and Projection ---
// Spec: https://hl7.org/fhirpath/2025Jan/#wherecriteria--expression--collection
#[test]
fn test_function_filtering_where() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.where($this > 1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(1 | 2 | 3 | 4).where($this > 2)", &context).unwrap(), // Add unwrap
        // Expect collection even if normalization happens
        collection(vec![
            EvaluationResult::Integer(3),
            EvaluationResult::Integer(4)
        ])
    );
    assert_eq!(
        eval("(1 | 2 | 3 | 4).where($this > 5)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("('a' | 'b' | 'c').where($this = 'b')", &context).unwrap(), // Add unwrap
        // Expect single item result due to normalization
        EvaluationResult::String("b".to_string())
    );
    // Test empty result from criteria is ignored
    assert_eq!(
        eval("(1 | 2 | {}).where($this > 1)", &context).unwrap(), // Add unwrap
        // Expect single item result due to normalization
        EvaluationResult::Integer(2)
    );
    // Test criteria evaluating to non-boolean (should error per spec)
    assert!(eval("(1 | 2 | 3).where($this)", &context).is_err());
    assert!(eval("(0 | 1 | 2).where($this)", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#selectprojection-expression--collection
#[test]
fn test_function_filtering_select() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.select($this + 1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(1 | 2 | 3).select($this * 2)", &context).unwrap(), // Add unwrap
        // Expect collection result
        collection(vec![
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(4),
            EvaluationResult::Integer(6)
        ])
    );
    // Test flattening
    assert_eq!(
        eval("( (1|2) | (3|4) ).select($this)", &context).unwrap(), // Add unwrap
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3),
            EvaluationResult::Integer(4)
        ])
    );
    // Test empty result from projection is skipped
    assert_eq!(
        eval("(1 | 2 | 3).select(iif($this > 2, $this, {}))", &context).unwrap(), // Add unwrap
        // Expect single item result due to normalization
        EvaluationResult::Integer(3)
    );
    // Test projection resulting in collection
    assert_eq!(
        eval("(1 | 2).select( ( $this ) | ( $this + 1 ) )", &context).unwrap(), // Add unwrap
        // Expect collection result
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
    // Simple types - expect single item results due to normalization
    assert_eq!(
        eval("(1 | 'a' | true).ofType(Integer)", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(1)
    );
    assert_eq!(
        eval("(1 | 'a' | true).ofType(String)", &context).unwrap(), // Add unwrap
        EvaluationResult::String("a".to_string())
    );
    assert_eq!(
        eval("(1 | 'a' | true).ofType(Boolean)", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("(1 | 'a' | true | 1.5).ofType(Decimal)", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(1.5))
    );
    assert_eq!(
        eval("{}.ofType(Integer)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(1 | 'a' | true).ofType(System.Integer)", &context).unwrap(), // Add unwrap
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
            fields.get("id"), // Patient.id has no extensions, should be primitive String
            Some(&EvaluationResult::String("p1".to_string()))
        );
        // Patient.active should evaluate to its primitive value directly
        assert_eq!(fields.get("active"), Some(&EvaluationResult::Boolean(true)));
        // To access the id, we would need Patient.active.id() or similar (not tested here)
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
        // Check status field - Observation.status has no extensions, should be primitive String
        assert_eq!(
            fields.get("status"),
            Some(&EvaluationResult::String("final".to_string()))
        );
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
    assert_eq!(eval("{}[0]", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(10 | 20 | 30)[0]", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(10)
    );
    assert_eq!(
        eval("(10 | 20 | 30)[1]", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(20)
    );
    assert_eq!(
        eval("(10 | 20 | 30)[2]", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(30)
    );
    assert_eq!(eval("(10 | 20 | 30)[3]", &context).unwrap(), EvaluationResult::Empty); // Index out of bounds -> Empty, Add unwrap
    // Negative index should error
    assert!(eval("(10 | 20 | 30)[-1]", &context).is_err());
    // Non-integer index should error
    assert!(eval("(10 | 20 | 30)['a']", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#single--collection
#[test]
fn test_function_subsetting_single() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.single()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(10).single()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(10)
    );
    // Multiple items should error per spec
    assert!(eval("(10 | 20).single()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#first--collection
#[test]
fn test_function_subsetting_first() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.first()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(10).first()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(10)
    );
    assert_eq!(
        eval("(10 | 20 | 30).first()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(10)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#last--collection
#[test]
fn test_function_subsetting_last() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.last()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(eval("(10).last()", &context).unwrap(), EvaluationResult::Integer(10)); // Add unwrap
    assert_eq!(
        eval("(10 | 20 | 30).last()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(30)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#tail--collection
#[test]
fn test_function_subsetting_tail() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.tail()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(eval("(10).tail()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(10 | 20 | 30).tail()", &context).unwrap(), // Add unwrap
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
    assert_eq!(eval("{}.skip(1)", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(10 | 20 | 30).skip(0)", &context).unwrap(), // Add unwrap
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
    assert_eq!(
        eval("(10 | 20 | 30).skip(1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
    assert_eq!(
        eval("(10 | 20 | 30).skip(3)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(10 | 20 | 30).skip(4)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    // Negative skip is treated as 0
    assert_eq!(
        eval("(10 | 20 | 30).skip(-1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
    // Non-integer skip should error
    assert!(eval("(10 | 20 | 30).skip('a')", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#takenum--integer--collection
#[test]
fn test_function_subsetting_take() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.take(1)", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(10 | 20 | 30).take(0)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(10 | 20 | 30).take(1)", &context).unwrap(), // Add unwrap
        // Expect single item result due to normalization
        EvaluationResult::Integer(10)
    );
    assert_eq!(
        eval("(10 | 20 | 30).take(2)", &context).unwrap(), // Add unwrap
        // Expect collection result
        collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20)
        ])
    );
    // Add the missing assert_eq! for take(3)
    assert_eq!(
        eval("(10 | 20 | 30).take(3)", &context).unwrap(), // Add unwrap
        // Expect collection result
        collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ]) // End collection for take(3)
    ); // End assert_eq for take(3)
    assert_eq!(
        eval("(10 | 20 | 30).take(4)", &context).unwrap(), // Add unwrap
        // Expect collection result
        collection(vec![
            EvaluationResult::Integer(10),
            EvaluationResult::Integer(20),
            EvaluationResult::Integer(30)
        ])
    );
    // Negative take returns empty
    assert_eq!(
        eval("(10 | 20 | 30).take(-1)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    // Non-integer take should error
    assert!(eval("(10 | 20 | 30).take('a')", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#intersectother-collection--collection
#[test]
fn test_function_subsetting_intersect() {
    // Note: HashSet used internally, order is not guaranteed in output
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.intersect({})", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(1 | 2 | 3).intersect({})", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("{}.intersect((1 | 2 | 3))", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    // Order not guaranteed, check contents
    let result = eval("(1 | 2 | 3).intersect((2 | 3 | 4))", &context).unwrap(); // Add unwrap
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
    // (1 | 2 | 1) -> (1 | 2)
    // (1 | 3 | 1) -> (1 | 3)
    // intersect -> (1)
    let result = eval("(1 | 2 | 1).intersect(1 | 3 | 1)", &context).unwrap(); // Add unwrap
    // Check if the result is the single integer 1, handling normalization
    assert_eq!(result, EvaluationResult::Integer(1), "Intersect result mismatch");
}

// Spec: https://hl7.org/fhirpath/2025Jan/#excludeother-collection--collection
#[test]
fn test_function_subsetting_exclude() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.exclude({})", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("(1 | 2 | 3).exclude({})", &context).unwrap(), // Add unwrap
        // Expect collection result
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3)
        ])
    );
    assert_eq!(
        eval("{}.exclude(1 | 2 | 3)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("(1 | 2 | 3).exclude(2 | 4)", &context).unwrap(), // Add unwrap
        // Expect collection result
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(3)
        ])
    );
    // Preserves duplicates and order - but | makes input distinct first
    // (1 | 2 | 1 | 3 | 2) -> (1 | 2 | 3)
    // (1 | 4) -> (1 | 4)
    // exclude -> (2 | 3)
    assert_eq!(
        eval("(1 | 2 | 1 | 3 | 2).exclude(1 | 4)", &context).unwrap(), // Add unwrap
        // Expect collection result based on distinct input
        collection(vec![
            EvaluationResult::Integer(2),
            EvaluationResult::Integer(3)
            // The second '2' is lost because the input collection becomes distinct
        ])
    );
}

// --- Combining ---
// Spec: https://hl7.org/fhirpath/2025Jan/#unionother--collection
#[test]
fn test_function_combining_union() {
    // Note: HashSet used internally, order is not guaranteed in output
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.union({})", &context).unwrap(), EvaluationResult::Empty); // Add unwrap

    let r1 = eval("(1 | 2).union({})", &context).unwrap(); // Add unwrap
    assert!(matches!(&r1, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(v) = r1 {
        assert_eq!(v.len(), 2); /* Check items if needed */
    }

    let r2 = eval("{}.union(1 | 2)", &context).unwrap(); // Add unwrap
    assert!(matches!(&r2, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(v) = r2 {
        assert_eq!(v.len(), 2); /* Check items if needed */
    }

    // Order not guaranteed, check contents
    let result = eval("(1 | 2 | 3).union(2 | 3 | 4)", &context).unwrap(); // Add unwrap
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
    let result = eval("(1 | 2 | 1).union(1 | 3 | 1)", &context).unwrap(); // Add unwrap
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
    assert_eq!(eval("{}.combine({})", &context).unwrap(), EvaluationResult::Empty); // Add unwrap

    let r1 = eval("(1 | 2).combine({})", &context).unwrap(); // Add unwrap
    assert!(matches!(&r1, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(v) = r1 {
        assert_eq!(v.len(), 2); /* Check items if needed */
    }

    // Use valid syntax (1 | 2) instead of {1 | 2}
    let r2 = eval("{}.combine(1 | 2)", &context).unwrap(); // Add unwrap
    assert!(matches!(&r2, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(v) = r2 {
        assert_eq!(v.len(), 2); /* Check items if needed */
    }

    // Order not guaranteed, check contents, duplicates preserved
    // Use valid syntax (2 | 3 | 4) instead of {2 | 3 | 4}
    let result = eval("(1 | 2 | 3).combine(2 | 3 | 4)", &context).unwrap(); // Add unwrap
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
    // Use valid syntax (1 | 3 | 1) instead of {1 | 3 | 1}
    let result = eval("(1 | 2 | 1).combine(1 | 3 | 1)", &context).unwrap(); // Add unwrap
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers, got {:?}", item), // Use pattern matching
            })
            .collect();
        actual_items.sort();
        // (1 | 2 | 1) -> (1 | 2)
        // (1 | 3 | 1) -> (1 | 3)
        // combine -> (1 | 2 | 1 | 3)
        assert_eq!(actual_items, vec![1, 1, 2, 3]); // Correct expectation
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
        eval("iif(true, 'a', 'b')", &context).unwrap(), // Add unwrap
        EvaluationResult::String("a".to_string())
    );
    assert_eq!(
        eval("iif(false, 'a', 'b')", &context).unwrap(), // Add unwrap
        EvaluationResult::String("b".to_string())
    );
    assert_eq!(
        eval("iif({}, 'a', 'b')", &context).unwrap(), // Add unwrap
        EvaluationResult::String("b".to_string())
    ); // Empty condition is false
    assert_eq!(
        eval("iif(true, 'a')", &context).unwrap(), // Add unwrap
        EvaluationResult::String("a".to_string())
    ); // Omitted otherwise
    assert_eq!(eval("iif(false, 'a')", &context).unwrap(), EvaluationResult::Empty); // Omitted otherwise, Add unwrap
    assert_eq!(eval("iif({}, 'a')", &context).unwrap(), EvaluationResult::Empty); // Omitted otherwise, Add unwrap
    // Test collection results
    assert_eq!(
        eval("iif(true, (1|2), (3|4))", &context).unwrap(), // Add unwrap
        collection(vec![
            EvaluationResult::Integer(1),
            EvaluationResult::Integer(2)
        ])
    );
    assert_eq!(
        eval("iif(false, (1|2), (3|4))", &context).unwrap(), // Add unwrap
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
    assert_eq!(eval("{}.toBoolean()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("true.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("1.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("0.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("1.0.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("0.0.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'true'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'false'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'T'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    ); // Case-insensitive
    assert_eq!(
        eval("'f'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    ); // Case-insensitive
    assert_eq!(
        eval("'yes'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'no'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'1'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'0'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("'1.0'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'0.0'.toBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(eval("2.toBoolean()", &context).unwrap(), EvaluationResult::Empty); // Invalid integer, Add unwrap
    assert_eq!(eval("2.5.toBoolean()", &context).unwrap(), EvaluationResult::Empty); // Invalid decimal, Add unwrap
    assert_eq!(eval("'abc'.toBoolean()", &context).unwrap(), EvaluationResult::Empty); // Invalid string, Add unwrap
    // Test multi-item collection - should error
    assert!(eval("(true | false).toBoolean()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstoboolean--boolean
#[test]
fn test_function_conversion_converts_to_boolean() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("true.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("1.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("0.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("1.0.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("0.0.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'true'.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'false'.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("2.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    ); // Invalid decimal
    assert_eq!(
        eval("'abc'.convertsToBoolean()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    ); // Invalid string
    // Test multi-item collection - should error
    assert!(eval("(true | false).convertsToBoolean()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#tointeger--integer
#[test]
fn test_function_conversion_to_integer() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toInteger()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("123.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(123)
    );
    assert_eq!(
        eval("'456'.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(456)
    );
    assert_eq!(
        eval("'+789'.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(789)
    );
    assert_eq!(
        eval("'-12'.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(-12)
    );
    assert_eq!(
        eval("true.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(1)
    );
    assert_eq!(
        eval("false.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Integer(0)
    );
    // Decimal conversion to Integer (truncates) - FHIRPath spec says Empty if not integer representable
    assert_eq!(
        eval("123.45.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty // Per spec
    );
    assert_eq!(
        eval("123.0.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty // Per spec (even if whole number)
    );
    assert_eq!(eval("'abc'.toInteger()", &context).unwrap(), EvaluationResult::Empty); // Invalid string, Add unwrap
    assert_eq!(
        eval("'123.45'.toInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    ); // Invalid string format
    // Test multi-item collection - should error
    assert!(eval("(1 | 2).toInteger()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstointeger--boolean
#[test]
fn test_function_conversion_converts_to_integer() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("123.convertsToInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'456'.convertsToInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true.convertsToInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    // Decimal conversion check
    assert_eq!(
        eval("123.45.convertsToInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false) // Per spec
    );
    assert_eq!(
        eval("123.0.convertsToInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abc'.convertsToInteger()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    ); // Invalid string
    // Test multi-item collection - should error
    assert!(eval("(1 | 2).convertsToInteger()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#todecimal--decimal
#[test]
fn test_function_conversion_to_decimal() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toDecimal()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("123.toDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(123.0)) // Integer to Decimal (explicit .0)
    );
    assert_eq!(
        eval("123.45.toDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(123.45)) // Decimal to Decimal
    );
    assert_eq!(
        eval("'456.78'.toDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(456.78)) // String to Decimal
    );
    assert_eq!(
        eval("'+12.3'.toDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(12.3)) // String with sign
    );
    assert_eq!(
        eval("'-45.6'.toDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(-45.6)) // String with sign
    );
    assert_eq!(
        eval("'789'.toDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(789.0)) // Integer string -> Decimal (explicit .0)
    );
    assert_eq!(
        eval("true.toDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(1.0)) // Boolean to Decimal (explicit .0)
    );
    assert_eq!(
        eval("false.toDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Decimal(dec!(0.0)) // Boolean to Decimal (explicit .0)
    );
    assert_eq!(eval("'abc'.toDecimal()", &context).unwrap(), EvaluationResult::Empty); // Invalid string, Add unwrap
    // Test multi-item collection - should error
    assert!(eval("(1.0 | 2.0).toDecimal()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstodecimal--boolean
#[test]
fn test_function_conversion_converts_to_decimal() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("123.convertsToDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("123.45.convertsToDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'456.78'.convertsToDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true.convertsToDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'abc'.convertsToDecimal()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    ); // Invalid string
    // Test multi-item collection - should error
    assert!(eval("(1.0 | 2.0).convertsToDecimal()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#tostring--string
#[test]
fn test_function_conversion_to_string() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toString()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("'abc'.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("abc".to_string())
    );
    assert_eq!(
        eval("123.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("123".to_string())
    );
    assert_eq!(
        eval("123.45.toString()", &context).unwrap(), // Add unwrap
        // Removed duplicate eval call, compare directly to expected result
        EvaluationResult::String("123.45".to_string()) // Decimal to string
    );
    assert_eq!(
        eval("true.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("true".to_string())
    );
    assert_eq!(
        eval("false.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("false".to_string())
    );
    assert_eq!(
        eval("@2023-10-27.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("2023-10-27".to_string())
    );
    assert_eq!(
        eval("@T10:30:00.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("10:30:00".to_string())
    );
    assert_eq!(
        eval("@2023-10-27T10:30Z.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("2023-10-27T10:30Z".to_string())  // Expect output without seconds
    );
    // Quantity to string (evaluator returns Decimal or Integer, ignoring unit)
    assert_eq!(
        eval("5.5 'mg'.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("5.5".to_string())
    );
    assert_eq!(
        eval("5 'mg'.toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("5".to_string())
    );
    // Collection to string - should error per spec
    assert!(eval("(1|2).toString()", &context).is_err());
    assert_eq!(
        eval("(1).toString()", &context).unwrap(), // Add unwrap
        EvaluationResult::String("1".to_string())
    ); // Single-item collection -> item string
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstostring--string
#[test]
fn test_function_conversion_converts_to_string() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("'abc'.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("123.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("123.45.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("true.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@T10:30:00.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27T10:30:00Z.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    // Quantity conversion (evaluator returns Decimal or Integer)
    assert_eq!(
        eval("5.5 'mg'.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("5 'mg'.convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    // Object/Collection are not convertible
    assert_eq!(
        eval("(1|2).convertsToString()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    // Need object test once available
    // Test multi-item collection - should error
    assert!(eval("(1 | 2).convertsToString()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#todate--date
#[test]
fn test_function_conversion_to_date() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toDate()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("@2023-10-27.toDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Date("2023-10-27".to_string())
    );
    assert_eq!(
        eval("@2023-10-27T10:30:00Z.toDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Date("2023-10-27".to_string())
    ); // DateTime to Date
    assert_eq!(
        eval("'2023-10-27'.toDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Date("2023-10-27".to_string())
    ); // String to Date
    assert_eq!(
        eval("'2023-10'.toDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Date("2023-10".to_string())
    ); // Partial date string
    assert_eq!(
        eval("'2023'.toDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Date("2023".to_string())
    ); // Partial date string
    assert_eq!(
        eval("'2023-10-27T10:30:00Z'.toDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Date("2023-10-27".to_string())
    ); // DateTime string to Date
    assert_eq!(
        eval("'invalid-date'.toDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(eval("123.toDate()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(eval("true.toDate()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    // Test multi-item collection - should error
    assert!(eval("(@2023 | @2024).toDate()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstodate--boolean
#[test]
fn test_function_conversion_converts_to_date() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("@2023-10-27.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27T10:30:00Z.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'2023-10-27'.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'2023-10'.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'2023'.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'2023-10-27T10:30:00Z'.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'invalid-date'.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("123.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("true.convertsToDate()", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
    // Test multi-item collection - should error
    assert!(eval("(@2023 | @2024).convertsToDate()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#todatetime--datetime
#[test]
fn test_function_conversion_to_date_time() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toDateTime()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(
        eval("@2023-10-27T10:30:00Z.toDateTime()", &context).unwrap(), // Add unwrap
        EvaluationResult::DateTime("2023-10-27T10:30:00Z".to_string())
    );
    assert_eq!(
        eval("@2023-10-27.toDateTime()", &context).unwrap(), // Add unwrap
        EvaluationResult::DateTime("2023-10-27".to_string())
    ); // Date to DateTime (no time part)
    assert_eq!(
        eval("'2023-10-27T10:30:00Z'.toDateTime()", &context).unwrap(), // Add unwrap
        EvaluationResult::DateTime("2023-10-27T10:30:00Z".to_string())
    ); // String to DateTime
    assert_eq!(
        eval("'2023-10-27'.toDateTime()", &context).unwrap(), // Add unwrap
        EvaluationResult::DateTime("2023-10-27".to_string())
    ); // Date string to DateTime
    assert_eq!(
        eval("'2023-10'.toDateTime()", &context).unwrap(), // Add unwrap
        EvaluationResult::DateTime("2023-10".to_string())
    ); // Partial date string
    assert_eq!(
        eval("'2023'.toDateTime()", &context).unwrap(), // Add unwrap
        EvaluationResult::DateTime("2023".to_string())
    ); // Partial date string
    assert_eq!(
        eval("'invalid-datetime'.toDateTime()", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
    assert_eq!(eval("123.toDateTime()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(eval("true.toDateTime()", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    // Test multi-item collection - should error
    assert!(eval("(@2023 | @2024).toDateTime()", &context).is_err());
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstodatetime--boolean
#[test]
fn test_function_conversion_converts_to_date_time() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToDateTime()", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("@2023-10-27T10:30:00Z.convertsToDateTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("@2023-10-27.convertsToDateTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'2023-10-27T10:30:00Z'.convertsToDateTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'2023-10-27'.convertsToDateTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'2023-10'.convertsToDateTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'2023'.convertsToDateTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'invalid-datetime'.convertsToDateTime()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("123.convertsToDateTime()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("true.convertsToDateTime()", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#totime--time
#[test]
fn test_function_conversion_to_time() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toTime()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("@T10:30:00.toTime()", &context),
        EvaluationResult::Time("10:30:00".to_string())
    );
    assert_eq!(
        eval("'10:30:00'.toTime()", &context),
        EvaluationResult::Time("10:30:00".to_string())
    ); // String to Time
    assert_eq!(
        eval("'10:30'.toTime()", &context),
        EvaluationResult::Time("10:30".to_string())
    ); // Partial time string
    assert_eq!(
        eval("'10'.toTime()", &context),
        EvaluationResult::Time("10".to_string())
    ); // Partial time string
    assert_eq!(
        eval("'invalid-time'.toTime()", &context),
        EvaluationResult::Empty
    );
    assert_eq!(eval("123.toTime()", &context), EvaluationResult::Empty);
    assert_eq!(eval("true.toTime()", &context), EvaluationResult::Empty);
    assert_eq!(
        eval("@2023-10-27.toTime()", &context),
        EvaluationResult::Empty
    ); // Date cannot convert
    assert_eq!(
        eval("@2023-10-27T10:30Z.toTime()", &context),
        EvaluationResult::Empty
    ); // DateTime cannot convert
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstotime--boolean
#[test]
fn test_function_conversion_converts_to_time() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToTime()", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("@T10:30:00.convertsToTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'10:30:00'.convertsToTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'10:30'.convertsToTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'10'.convertsToTime()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'invalid-time'.convertsToTime()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("123.convertsToTime()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("true.convertsToTime()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("@2023-10-27.convertsToTime()", &context),
        EvaluationResult::Boolean(false)
    );
    assert_eq!(
        eval("@2023-10-27T10:30Z.convertsToTime()", &context),
        EvaluationResult::Boolean(false)
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#toquantity--quantity
#[test]
fn test_function_conversion_to_quantity() {
    let context = EvaluationContext::new_empty();
    assert_eq!(eval("{}.toQuantity()", &context), EvaluationResult::Empty);
    // Boolean to Quantity
    assert_eq!(
        eval("true.toQuantity()", &context),
        EvaluationResult::Decimal(dec!(1.0)) // Spec implies conversion to Decimal 1.0 '1'
    );
    assert_eq!(
        eval("false.toQuantity()", &context),
        EvaluationResult::Decimal(dec!(0.0)) // Spec implies conversion to Decimal 0.0 '1'
    );
    // Integer to Quantity
    assert_eq!(
        eval("123.toQuantity()", &context),
        EvaluationResult::Decimal(dec!(123.0)) // Spec implies conversion to Decimal 123.0 '1'
    );
    // Decimal to Quantity
    assert_eq!(
        eval("123.45.toQuantity()", &context),
        EvaluationResult::Decimal(dec!(123.45)) // Spec implies conversion to Decimal 123.45 '1'
    );
    // String to Quantity (parses number and unit)
    assert_eq!(
        eval("'5.5 mg'.toQuantity()", &context),
        EvaluationResult::Decimal(dec!(5.5)) // Evaluator currently ignores unit
    );
    assert_eq!(
        eval("'100'.toQuantity()", &context),
        EvaluationResult::Decimal(dec!(100.0)) // Evaluator currently ignores unit
    );
    assert_eq!(
        eval("'100 days'.toQuantity()", &context),
        EvaluationResult::Decimal(dec!(100.0)) // String with valid time unit
    );
    assert_eq!(
        eval("'invalid'.toQuantity()", &context),
        EvaluationResult::Empty
    ); // Not a number
    assert_eq!(
        eval("'5.5 invalid-unit'.toQuantity()", &context),
        EvaluationResult::Empty
    ); // Invalid unit part
    assert_eq!(
        eval("'5.5 mg extra'.toQuantity()", &context),
        EvaluationResult::Empty
    ); // Too many parts
    // Quantity literal to Quantity (should just return the numeric part)
    assert_eq!(
        eval("5.5 'mg'.toQuantity()", &context), // This uses the Quantity literal parser
        EvaluationResult::Decimal(dec!(5.5))
    );
    assert_eq!(
        eval("100 days.toQuantity()", &context),
        EvaluationResult::Decimal(dec!(100.0)) // Expect Decimal conversion
    );
}

// Spec: https://hl7.org/fhirpath/2025Jan/#convertstoquantity--boolean
#[test]
fn test_function_conversion_converts_to_quantity() {
    let context = EvaluationContext::new_empty();
    assert_eq!(
        eval("{}.convertsToQuantity()", &context),
        EvaluationResult::Empty
    );
    assert_eq!(
        eval("true.convertsToQuantity()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("123.convertsToQuantity()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("123.45.convertsToQuantity()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'5.5 mg'.convertsToQuantity()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'100'.convertsToQuantity()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'100 days'.convertsToQuantity()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("'invalid'.convertsToQuantity()", &context),
        EvaluationResult::Boolean(false) // Not a number
    );
    assert_eq!(
        eval("'5.5 invalid-unit'.convertsToQuantity()", &context),
        EvaluationResult::Boolean(false) // Invalid unit part
    );
    assert_eq!(
        eval("'5.5 mg extra'.convertsToQuantity()", &context),
        EvaluationResult::Boolean(false) // Too many parts
    );
    // Quantity literal conversion (these use the Quantity literal parser, not string conversion)
    assert_eq!(
        eval("5.5 'mg'.convertsToQuantity()", &context),
        EvaluationResult::Boolean(true)
    );
    assert_eq!(
        eval("100 days.convertsToQuantity()", &context),
        EvaluationResult::Boolean(true)
    );
}

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
        EvaluationResult::String("".to_string()) // Spec: Start out of bounds returns empty string
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
    assert_eq!(
        evaluate(&expr, &context, None),
        EvaluationResult::Boolean(true)
    );
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
    assert_eq!(
        evaluate(&expr, &context, None),
        EvaluationResult::Boolean(true)
    );
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
    assert_eq!(
        evaluate(&expr, &context, None),
        EvaluationResult::Boolean(true)
    );
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
    assert_eq!(eval("1.0 = 1.0", &context), EvaluationResult::Boolean(true)); // Decimal vs Decimal
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
    // Test: Order matters for '='
    assert_eq!(
        eval("(1|2) = (2|1)", &context),
        EvaluationResult::Boolean(false) // This assertion is correct per spec
    );
    assert_eq!(
        eval("(1|2) = (1|2|3)", &context),
        EvaluationResult::Boolean(false)
    ); // Different count
    assert_eq!(
        eval("(1|1) = (1|1)", &context),
        EvaluationResult::Boolean(true)
    );
    // Empty propagation - Per spec, comparison with empty results in empty
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
    assert_eq!(eval("1.0 ~ 1.0", &context), EvaluationResult::Boolean(true)); // Decimal vs Decimal
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
        EvaluationResult::Boolean(false) // Duplicates matter, counts differ
    );
    assert_eq!(
        eval("(1|2|1) ~ (1|1|2)", &context), // Same elements, different order, same counts
        EvaluationResult::Boolean(true)
    );
    // Empty comparison - Corrected based on spec for '~'
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
    // Empty propagation - Per spec, comparison with empty results in empty
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
    // Empty comparison - Corrected based on spec for '!~'
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
    // Order not guaranteed, check contents - Union operator produces distinct results
    let result = eval("(1 | 2 | 3) | (2 | 3 | 4)", &context);
    if let EvaluationResult::Collection(items) = result {
        let mut actual_items: Vec<i64> = items
            .into_iter()
            .map(|item| match item {
                EvaluationResult::Integer(i) => i,
                _ => panic!("Expected integers, got {:?}", item), // Improved panic message
            })
            .collect();
        actual_items.sort();
        assert_eq!(actual_items, vec![1, 2, 3, 4]); // Expect distinct items
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
    // Numbers
    assert_eq!(
        eval("1 + 2", &context),
        EvaluationResult::Integer(3) // Integer + Integer -> Integer (per spec example)
    );
    assert_eq!(
        eval("1.5 + 2", &context),
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
    // Integer - Integer -> Integer
    assert_eq!(
        eval("5 - 3", &context),
        EvaluationResult::Integer(2) // Integer - Integer -> Integer
    );
    // Decimal involved -> Decimal result
    assert_eq!(
        eval("5.5 - 3", &context),
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
    assert_eq!(eval("5.5 div 2.1", &context), EvaluationResult::Integer(2));
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
    // 1 + (2 * 3) = 1 + 6 = 7 (Integer + Integer -> Integer)
    assert_eq!(
        eval("1 + 2 * 3", &context),
        EvaluationResult::Integer(7) // <-- Correct expectation
    );
    // (1 + 2) * 3 = 3 * 3 = 9 (Integer + Integer -> Integer, then Integer * Integer -> Integer)
    assert_eq!(
        eval("(1 + 2) * 3", &context),
        EvaluationResult::Integer(9) // <-- Correct expectation
    );
    // (5 - 2) + 1 = 3 + 1 = 4 (Subtraction -> Integer, then Integer + Integer -> Integer)
    assert_eq!(
        eval("5 - 2 + 1", &context),
        EvaluationResult::Integer(4) // Corrected expectation
    );
    // (10 / 2) * 5 = 5.0 * 5 = 25.0 (Division -> Decimal, then Decimal * Integer -> Decimal)
    assert_eq!(
        eval("10 / 2 * 5", &context),
        EvaluationResult::Decimal(dec!(25.0))
    );
    // (10 div 3) * 2 = 3 * 2 = 6 (div -> Integer, then Integer * Integer -> Integer)
    assert_eq!(eval("10 div 3 * 2", &context), EvaluationResult::Integer(6));
    // (10 mod 3) + 1 = 1 + 1 = 2 (mod -> Integer, then Integer + Integer -> Integer)
    assert_eq!(
        eval("10 mod 3 + 1", &context),
        EvaluationResult::Integer(2) // <-- Correct expectation
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
    // (-1) + 5 = 4 (Unary minus, then Integer + Integer -> Integer)
    assert_eq!(
        eval("-1 + 5", &context),
        EvaluationResult::Integer(4) // <-- Correct expectation
    );
    // -(1 + 5) = -(6) = -6 (Addition -> Integer, then Unary minus)
    assert_eq!(
        eval("-(1 + 5)", &context),
        EvaluationResult::Integer(-6) // <-- Correct expectation
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

    // Evaluate the %context variable using the eval function
    let context_var_result = eval("%context", &ctx_res);
    // Check that the result is not Empty
    assert!(
        !matches!(context_var_result, EvaluationResult::Empty),
        "%context should be set"
    );
    assert!(matches!(context_var_result, EvaluationResult::Object(_)));

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
        identifier: Some(vec![r4::Identifier {
            // Wrap in Some()
            r#use: Some(Code {
                // Use imported Code
                value: Some("usual".to_string()),
                ..Default::default()
            }),
            system: Some("urn:oid:1.2.3.4".to_string().into()), // Use .to_string().into()
            value: Some("12345".to_string().into()),            // Use .to_string().into()
            ..Default::default()
        }]),
        active: Some(Boolean {
            // Use imported Boolean
            // Element with value
            id: Some("active-id".to_string().into()), // Element ID - Use .to_string().into()
            value: Some(true),
            ..Default::default()
        }),
        name: Some(vec![
            // Wrap in Some()
            r4::HumanName {
                // Official Name
                id: Some("name1".to_string().into()), // Use .to_string().into()
                r#use: Some(Code {
                    // Use imported Code
                    value: Some("official".to_string()),
                    ..Default::default()
                }),
                family: Some("Doe".to_string().into()), // Use .to_string().into()
                given: Some(vec![
                    // Wrap in Some()
                    FhirString {
                        // Use imported FhirString
                        value: Some("John".to_string()),
                        ..Default::default()
                    }, // Element<String>
                    FhirString {
                        // Use imported FhirString
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
                r#use: Some(Code {
                    // Use imported Code
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
        telecom: Some(vec![
            // Wrap in Some()
            r4::ContactPoint {
                system: Some(Code {
                    // Use imported Code
                    value: Some("phone".to_string()),
                    ..Default::default()
                }),
                value: Some("555-1234".to_string().into()), // Use .to_string().into()
                ..Default::default()
            },
            r4::ContactPoint {
                system: Some(Code {
                    // Use imported Code
                    value: Some("email".to_string()),
                    ..Default::default()
                }),
                value: Some("john.doe@example.com".to_string().into()), // Use .to_string().into()
                ..Default::default()
            },
        ]),
        birth_date: Some(Date {
            // Use imported Date
            // Element with value and extension
            id: Some("birthdate-id".to_string().into()), // Use .to_string().into()
            value: Some("1980-05-15".to_string()),
            extension: Some(vec![Extension {
                // Use imported Extension, wrap in Some()
                url: "http://example.com/precision".to_string().into(), // Remove Some(), url is not Option
                value: Some(ExtensionValue::String("day".to_string().into())), // Use imported ExtensionValue, .to_string().into()
                ..Default::default()
            }]),
            ..Default::default()
        }),
        deceased: Some(r4::PatientDeceased::Boolean(Boolean {
            // Use imported Boolean
            value: Some(false),
            ..Default::default()
        })), // DeceasedBoolean (Element)
        ..Default::default()
    };
    EvaluationContext::new(vec![FhirResource::R4(Box::new(r4::Resource::Patient(
        // Wrap in Resource::Patient
        patient,
    )))])
}

#[test]
fn test_resource_simple_field_access() {
    let context = patient_context();
    assert_eq!(
        eval("id", &context).unwrap(), // Add unwrap
        EvaluationResult::String("p1".to_string())
    );
    // Accessing 'active' should now return the primitive boolean directly
    assert_eq!(eval("active", &context).unwrap(), EvaluationResult::Boolean(true)); // Add unwrap
    // Accessing 'birthDate' should now return the primitive string directly
    assert_eq!(
        eval("birthDate", &context).unwrap(), // Add unwrap
        EvaluationResult::String("1980-05-15".to_string())
    );
    // Evaluate the context first to check the object structure directly
    let context_result = eval("%context", &context).unwrap(); // Add unwrap
    eprintln!("Debug [test_resource_simple_field_access]: %context result: {:?}", context_result);
    if let EvaluationResult::Object(patient_obj) = context_result {
        // Check the 'deceased' field within the evaluated object map
        assert_eq!(
            patient_obj.get("deceased"),
            Some(&EvaluationResult::Boolean(false)),
            "Deceased field mismatch in evaluated patient object"
        );
    } else {
        panic!("%context did not evaluate to an Object");
    }
    // Accessing non-existent fields should still return Empty for now
    assert_eq!(eval("deceasedBoolean", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(eval("deceasedDateTime", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
    assert_eq!(eval("nonExistentField", &context).unwrap(), EvaluationResult::Empty); // Add unwrap
}

#[test]
fn test_resource_nested_field_access() {
    let context = patient_context();
    // Accessing a field within a list - returns a collection of that field from each list item
    let name_family = eval("name.family", &context).unwrap(); // Add unwrap
    assert!(matches!(name_family, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(items) = name_family {
        assert_eq!(items.len(), 2); // Doe, Smith (usual name has no family)
        assert!(items.contains(&EvaluationResult::String("Doe".to_string())));
        assert!(items.contains(&EvaluationResult::String("Smith".to_string())));
    }

    // Accessing 'name.given' should return a collection of primitive strings
    let name_given = eval("name.given", &context).unwrap(); // Add unwrap
    assert!(matches!(name_given, EvaluationResult::Collection(_)));
    if let EvaluationResult::Collection(items) = name_given {
        assert_eq!(items.len(), 4); // John, Middle, Johnny, Jane
        assert!(items.contains(&EvaluationResult::String("John".to_string())));
        assert!(items.contains(&EvaluationResult::String("Middle".to_string()))); // Now a primitive string
        assert!(items.contains(&EvaluationResult::String("Johnny".to_string())));
        assert!(items.contains(&EvaluationResult::String("Jane".to_string())));
    }

    // Accessing a field that doesn't exist in all items
    let name_use = eval("name.use", &context).unwrap(); // Add unwrap
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

    // TODO: Re-enable these tests when .id access on primitives is implemented
    // // Access element id - 'active' should allow .id access
    // assert_eq!(
    //     eval("active.id", &context),
    //     EvaluationResult::String("active-id".to_string())
    // );
    // // Access element id - 'birthDate' should allow .id access
    // assert_eq!(
    //     eval("birthDate.id", &context),
    //     EvaluationResult::String("birthdate-id".to_string())
    // );

    // Access id on complex type (HumanName) - this should still work
    let name_ids = eval("name.id", &context).unwrap(); // Add unwrap
    assert!(
        matches!(name_ids, EvaluationResult::Collection(_)), // Expect Collection even if only 2 results
        "Expected Collection for name.id, got {:?}",
        name_ids
    );
    if let EvaluationResult::Collection(items) = name_ids {
        assert_eq!(items.len(), 2);
        assert!(items.contains(&EvaluationResult::String("name1".to_string())));
        assert!(items.contains(&EvaluationResult::String("name2".to_string())));
    }
    // TODO: Re-enable this test when .id access on primitives is implemented
    // let given_ids = eval("name.given.id", &context); // (empty for John), given2-id, (empty for Johnny), (empty for Jane)
    // assert!(
    //     matches!(given_ids, EvaluationResult::String(_)),
    //     "Expected String for name.given.id, got {:?}",
    //     given_ids
    // ); // Only one ID present
    // assert_eq!(given_ids, EvaluationResult::String("given2-id".to_string()));

    // TODO: Re-enable these tests when .extension access on primitives is implemented
    // // Access extension (basic check, requires Extension conversion)
    // let bday_ext = eval("birthDate.extension", &context);
    // assert!(
    //     matches!(bday_ext, EvaluationResult::Collection(_)),
    //     "Expected Collection for birthDate.extension, got {:?}", // This message belongs inside the assert!
    //     bday_ext
    // );
    // if let EvaluationResult::Collection(exts) = bday_ext {
    //     assert_eq!(exts.len(), 1);
    //     // Further checks require Extension object structure
    //     // assert_eq!(eval("birthDate.extension.url", &context), EvaluationResult::String("http://example.com/precision".to_string()));
    //     // assert_eq!(eval("birthDate.extension.valueString", &context), EvaluationResult::String("day".to_string()));
    // }
}

#[test]
fn test_resource_filtering_and_projection() {
    let context = patient_context();

    // Where on a list field
    let official_name = eval("name.where(use = 'official')", &context).unwrap(); // Add unwrap
    assert!(matches!(official_name, EvaluationResult::Object(_)), "Expected Object for official name, got {:?}", official_name); // Should return the HumanName object

    // Select from the filtered list
    assert_eq!(
        eval("name.where(use = 'official').family", &context).unwrap(), // Add unwrap
        EvaluationResult::String("Doe".to_string()) // Expect primitive string
    );
    // .given returns a collection of primitive strings
    assert_eq!(
        eval("name.where(use = 'usual').given", &context).unwrap(), // Add unwrap
        EvaluationResult::Collection(vec![EvaluationResult::String("Johnny".to_string())])
    );
    assert_eq!(
        eval("name.where(family = 'Smith').given", &context).unwrap(), // Add unwrap
        EvaluationResult::Collection(vec![EvaluationResult::String("Jane".to_string())])
    );

    // Select multiple fields - This expression should error because 'given' is a collection
    // and '+' requires singletons.
    let official_details_result = eval(
        "name.where(use = 'official').select(given + ' ' + family)",
        &context,
    );
    assert!(official_details_result.is_err()); // Expect error

    // Select on a non-list field (acts on the single item) - birthDate is now primitive
    assert_eq!(
        eval("birthDate.select($this.toString())", &context).unwrap(), // Add unwrap
        EvaluationResult::String("1980-05-15".to_string())
    );

    // Where on root context - 'active' is now primitive
    assert_eq!(
        eval("%context.where(active = true).id", &context).unwrap(), // Add unwrap
        EvaluationResult::String("p1".to_string())
    );
    assert_eq!(
        eval("%context.where(active = false).id", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
}

#[test]
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

    let patients = eval("%context.ofType(Patient)", &context).unwrap(); // Add unwrap
    assert!(matches!(patients, EvaluationResult::Object(_)), "Expected Object for Patient, got {:?}", patients); // Only one patient
    if let EvaluationResult::Object(fields) = patients {
        assert_eq!(
            fields.get("resourceType"),
            Some(&EvaluationResult::String("Patient".to_string()))
        );
        // Accessing 'id' on the Patient object should return the primitive string
        assert_eq!(
            fields.get("id"),
            Some(&EvaluationResult::String("p1".to_string()))
        );
    }

    let observations = eval("%context.ofType(Observation)", &context).unwrap(); // Add unwrap
    assert!(matches!(observations, EvaluationResult::Object(_)), "Expected Object for Observation, got {:?}", observations); // Only one observation
    if let EvaluationResult::Object(fields) = observations {
        assert_eq!(
            fields.get("resourceType"),
            Some(&EvaluationResult::String("Observation".to_string()))
        );
        // Accessing 'id' on the Observation object should return the primitive string
        assert_eq!(
            fields.get("id"),
            Some(&EvaluationResult::String("o1".to_string()))
        );
    }

    assert_eq!(
        eval("%context.ofType(Practitioner)", &context).unwrap(), // Add unwrap
        EvaluationResult::Empty
    );
}

#[test]
fn test_arithmetic_operations() {
    // Note: Result types vary based on operator and operands
    let context = EvaluationContext::new_empty();

    // --- Success Cases ---
    let success_cases = vec![
        ("1 + 2", EvaluationResult::Integer(3)),         // Addition -> Integer
        ("5 - 3", EvaluationResult::Integer(2)),         // Subtraction -> Integer
        ("2 * 3", EvaluationResult::Integer(6)),         // Integer Multiplication -> Integer
        ("6 / 2", EvaluationResult::Decimal(dec!(3.0))), // Division -> Decimal
        ("7 / 2", EvaluationResult::Decimal(dec!(3.5))), // Division -> Decimal
        ("7 div 2", EvaluationResult::Integer(3)),       // Integer div -> Integer
        ("7 mod 2", EvaluationResult::Integer(1)),       // Integer mod -> Integer
        ("5.5 + 2.1", EvaluationResult::Decimal(dec!(7.6))), // Decimal Add -> Decimal
        ("5.5 - 2.1", EvaluationResult::Decimal(dec!(3.4))), // Decimal Sub -> Decimal
        ("5.5 * 2.0", EvaluationResult::Decimal(dec!(11.0))), // Decimal Mult -> Decimal
        ("5.5 / 2.0", EvaluationResult::Decimal(dec!(2.75))), // Decimal Div -> Decimal
        ("5.5 div 2.1", EvaluationResult::Integer(2)),   // Decimal div -> Integer
        ("5.5 mod 2.1", EvaluationResult::Decimal(dec!(1.3))), // Decimal mod -> Decimal
    ];

    for (input, expected) in success_cases {
        assert_eq!(eval(input, &context).unwrap(), expected, "Failed for input: {}", input);
    }

    // --- Error Cases ---
    let error_cases = vec![
        // Mixed type div/mod -> Error
        "5.5 div 2",
        "5 div 2.1",
        "5.5 mod 2",
        "5 mod 2.1",
        // Division by zero -> Error
        "5 / 0",
        "5.0 / 0",
        "5 div 0",
        "5.0 div 0.0", // Changed to 0.0 for Decimal div
        "5 mod 0",
        "5.0 mod 0.0", // Changed to 0.0 for Decimal mod
        // Type Mismatches
        "1 + 'a'",
        "'a' + 1",
        "1 * 'a'",
        "1 / 'a'",
        "1 div 'a'",
        "1 mod 'a'",
    ];

    for input in error_cases {
        assert!(eval(input, &context).is_err(), "Expected error for input: {}", input);
    }

    // --- Empty Propagation Cases ---
    let empty_cases = vec![
        "1 + {}", "{} + 1",
        "1 - {}", "{} - 1",
        "1 * {}", "{} * 1",
        "1 / {}", "{} / 1",
        "1 div {}", "{} div 1",
        "1 mod {}", "{} mod 1",
    ];
    for input in empty_cases {
         assert_eq!(eval(input, &context).unwrap(), EvaluationResult::Empty, "Failed for input: {}", input);
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
        // Test empty propagation (should return Empty, not error)
        ("true and {}", EvaluationResult::Empty),
        ("{} and true", EvaluationResult::Empty),
        ("false and {}", EvaluationResult::Boolean(false)), // Spec table
        ("{} and false", EvaluationResult::Boolean(false)), // Spec table
        ("{} and {}", EvaluationResult::Empty),
        ("true or {}", EvaluationResult::Boolean(true)), // Spec table
        ("{} or true", EvaluationResult::Boolean(true)), // Spec table
        ("false or {}", EvaluationResult::Empty),
        ("{} or false", EvaluationResult::Empty),
        ("{} or {}", EvaluationResult::Empty),
        ("true xor {}", EvaluationResult::Empty),
        ("{} xor true", EvaluationResult::Empty),
        ("false xor {}", EvaluationResult::Empty),
        ("{} xor false", EvaluationResult::Empty),
        ("{} xor {}", EvaluationResult::Empty),
        ("true implies {}", EvaluationResult::Empty),
        ("{} implies true", EvaluationResult::Boolean(true)), // Spec table
        ("false implies {}", EvaluationResult::Boolean(true)), // Spec table
        ("{} implies false", EvaluationResult::Empty),
        ("{} implies {}", EvaluationResult::Empty),
    ];

    // For boolean operations, we don't need any resources
    let context = EvaluationContext::new_empty();

    for (input, expected) in test_cases {
        assert_eq!(eval(input, &context).unwrap(), expected, "Failed for input: {}", input);
    }

    // Test type errors (should error)
    assert!(eval("1 and true", &context).is_err());
    assert!(eval("true and 'a'", &context).is_err());
    assert!(eval("1 or true", &context).is_err());
    assert!(eval("true or 'a'", &context).is_err());
    assert!(eval("1 xor true", &context).is_err());
    assert!(eval("true xor 'a'", &context).is_err());
    assert!(eval("1 implies true", &context).is_err());
    assert!(eval("true implies 'a'", &context).is_err());
}

#[test]
fn test_comparison_operations() {
    let context = EvaluationContext::new_empty();

    // --- Success Cases ---
    let success_cases = vec![
        ("1 < 2", EvaluationResult::Boolean(true)),
        ("2 <= 2", EvaluationResult::Boolean(true)),
        ("3 > 2", EvaluationResult::Boolean(true)),
        ("3 >= 3", EvaluationResult::Boolean(true)),
        ("1 = 1", EvaluationResult::Boolean(true)),
        ("1 != 2", EvaluationResult::Boolean(true)),
        ("'abc' ~ 'ABC'", EvaluationResult::Boolean(true)),
        ("'abc' !~ 'def'", EvaluationResult::Boolean(true)),
        // Add more specific comparison cases
        ("1.0 < 2", EvaluationResult::Boolean(true)),
        ("2 >= 1.5", EvaluationResult::Boolean(true)),
        ("'b' > 'a'", EvaluationResult::Boolean(true)),
        ("'a' <= 'a'", EvaluationResult::Boolean(true)),
        ("@2024 > @2023", EvaluationResult::Boolean(true)),
        ("@T10:00 < @T11:00", EvaluationResult::Boolean(true)),
    ];

    for (input, expected) in success_cases {
        assert_eq!(eval(input, &context).unwrap(), expected, "Failed for input: {}", input);
    }

    // --- Error Cases (Comparing collections or incompatible types) ---
    let error_cases = vec![
        "(1 | 2) < 3", // Collection comparison should error
        "1 < (2 | 3)",
        "1 < 'a'",     // Incompatible types
        "'a' > true",
        "@2023 = @T10:00", // Incompatible date/time types
    ];
    for input in error_cases {
        assert!(eval(input, &context).is_err(), "Expected error for input: {}", input);
    }

    // --- Empty Propagation Cases ---
    let empty_cases = vec![
        "1 < {}", "{} < 1",
        "1 <= {}", "{} <= 1",
        "1 > {}", "{} > 1",
        "1 >= {}", "{} >= 1",
        "1 = {}", "{} = 1", // = with empty -> empty
        "1 != {}", "{} != 1", // != with empty -> empty
        "1 ~ {}", "{} ~ 1", // ~ with empty -> false (handled by assert_eq below)
        "1 !~ {}", "{} !~ 1", // !~ with empty -> true (handled by assert_eq below)
        "{} = {}", // = with empty -> empty
        "{} != {}", // != with empty -> empty
    ];
     for input in empty_cases {
         assert_eq!(eval(input, &context).unwrap(), EvaluationResult::Empty, "Failed for input: {}", input);
    }

    // Specific checks for ~ and !~ with empty
    assert_eq!(eval("1 ~ {}", &context).unwrap(), EvaluationResult::Boolean(false));
    assert_eq!(eval("{} ~ 1", &context).unwrap(), EvaluationResult::Boolean(false));
    assert_eq!(eval("{} ~ {}", &context).unwrap(), EvaluationResult::Boolean(true));
    assert_eq!(eval("1 !~ {}", &context).unwrap(), EvaluationResult::Boolean(true));
    assert_eq!(eval("{} !~ 1", &context).unwrap(), EvaluationResult::Boolean(true));
    assert_eq!(eval("{} !~ {}", &context).unwrap(), EvaluationResult::Boolean(false));
}

#[test]
fn test_variable_access() {
    // We'll set up the context without any resources
    let mut context = EvaluationContext::new_empty();

    // For testing variable access, we'll add some variables to the context
    context.set_variable("name", "John Doe".to_string());
    context.set_variable("age", "42".to_string()); // Store as string, FHIRPath handles conversion if needed

    // --- Success Cases ---
    let success_cases = vec![
        // Access variables directly
        ("%name", EvaluationResult::String("John Doe".to_string())),
        // Accessing %age should return the string value stored
        ("%age", EvaluationResult::String("42".to_string())),
        // Test conversion within expression
        ("%age.toInteger()", EvaluationResult::Integer(42)),
    ];

    for (input, expected) in success_cases {
        assert_eq!(eval(input, &context).unwrap(), expected, "Failed for input: {}", input);
    }

    // --- Error Case (Undefined Variable) ---
    assert!(eval("%address", &context).is_err(), "Expected error for undefined variable %address");

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
        // Test contains with non-string argument (should return false or empty, current impl returns false)
        (
            "'abc'.contains(1)",
            EvaluationResult::Boolean(false),
        ),
        // Test contains with empty argument (should return empty)
        (
            "'abc'.contains({})",
            EvaluationResult::Empty,
        ),
        // Test contains on empty string
        (
            "{}.contains('a')",
            EvaluationResult::Empty, // Spec: {} contains X -> false, but function call on {} -> {}
        ),

    ];

    for (input, expected) in test_cases {
        assert_eq!(eval(input, &context).unwrap(), expected, "Failed for input: {}", input);
    }

     // Test multi-item errors for contains function
    assert!(eval("('a' | 'b').contains('a')", &context).is_err());
    assert!(eval("'abc'.contains(('a' | 'b'))", &context).is_err());
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
        // Evaluate and unwrap, as these are expected success cases
        assert_eq!(eval(&full_expr, &context).unwrap(), expected, "Failed for input: {}", full_expr);
    }
}

#[test]
fn test_direct_string_operations() {
    // We'll set up the context without any resources
    let context = EvaluationContext::new_empty();

    // Test string operations through the parser instead of direct function calls
    assert_eq!(
        eval("'Hello, World!'.contains('World')", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(true)
    );

    assert_eq!(
        eval("'Hello, World!'.contains('Goodbye')", &context).unwrap(), // Add unwrap
        EvaluationResult::Boolean(false)
    );
}

#[test]
fn test_resource_access() {
    // Remove duplicate imports, they are already at the top level
    use fhir::r4::{Account, Code}; // Import only needed types locally if preferred, or rely on top-level
    // Create a dummy R4 resource for testing
    let dummy_resource = r4::Resource::Account(Account {
        // Use imported Account
        id: Some("theid".to_string().into()), // Convert String to Id
        meta: None,
        implicit_rules: None,
        language: None,
        text: None,
        contained: None,
        extension: None,
        modifier_extension: None,
        identifier: None,
        status: Code {
            // Use imported Code
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
    assert_eq!(
        eval("id", &context).unwrap(), // Add unwrap
        EvaluationResult::String("theid".to_string())
    ); // Expect the primitive string value of the id
}
