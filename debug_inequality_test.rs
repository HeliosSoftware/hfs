use fhirpath::evaluator::{evaluate, EvaluationContext, EvaluationResult};
use fhirpath::parser::parser;
use serde_json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create test data - Observation with valueInteger: 12
    let observation_json = serde_json::json!({
        "resourceType": "Observation",
        "id": "o1",
        "valueInteger": 12
    });
    
    // Convert to EvaluationResult::Object
    let observation_map = if let serde_json::Value::Object(map) = observation_json {
        let mut eval_map = std::collections::HashMap::new();
        for (k, v) in map {
            eval_map.insert(k, convert_json_value(v));
        }
        EvaluationResult::Object { 
            map: eval_map, 
            resource_type: Some("Observation".to_string()) 
        }
    } else {
        panic!("Expected JSON object");
    };
    
    // Create empty context
    let context = EvaluationContext::new();
    
    // Test parsing and evaluating the expressions
    let parser = parser();
    
    // Test expression: value.ofType(integer) > 11
    let expr_str = "value.ofType(integer) > 11";
    println!("Testing expression: {}", expr_str);
    
    match parser.parse(expr_str) {
        Ok(ast) => {
            println!("AST: {:?}", ast);
            
            match evaluate(&ast, &context, &observation_map) {
                Ok(result) => {
                    println!("Evaluation result: {:?}", result);
                },
                Err(e) => {
                    println!("Evaluation error: {:?}", e);
                }
            }
        },
        Err(e) => {
            println!("Parse error: {:?}", e);
        }
    }
    
    // Also test simpler expressions
    println!("\n--- Testing simpler expressions ---");
    
    // Test: value
    test_expression("value", &observation_map, &context);
    
    // Test: value.ofType(integer)
    test_expression("value.ofType(integer)", &observation_map, &context);
    
    // Test: valueInteger
    test_expression("valueInteger", &observation_map, &context);
    
    // Test: valueInteger > 11
    test_expression("valueInteger > 11", &observation_map, &context);
    
    Ok(())
}

fn test_expression(expr_str: &str, item: &EvaluationResult, context: &EvaluationContext) {
    let parser = parser();
    println!("\nTesting: {}", expr_str);
    
    match parser.parse(expr_str) {
        Ok(ast) => {
            match evaluate(&ast, context, item) {
                Ok(result) => {
                    println!("  Result: {:?}", result);
                },
                Err(e) => {
                    println!("  Error: {:?}", e);
                }
            }
        },
        Err(e) => {
            println!("  Parse error: {:?}", e);
        }
    }
}

fn convert_json_value(value: serde_json::Value) -> EvaluationResult {
    match value {
        serde_json::Value::Null => EvaluationResult::Empty,
        serde_json::Value::Bool(b) => EvaluationResult::boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                EvaluationResult::Integer(i, None)
            } else if let Some(f) = n.as_f64() {
                EvaluationResult::Decimal(rust_decimal::Decimal::try_from(f).unwrap_or_default(), None)
            } else {
                EvaluationResult::Empty
            }
        },
        serde_json::Value::String(s) => EvaluationResult::String(s, None),
        serde_json::Value::Array(arr) => {
            let items: Vec<EvaluationResult> = arr.into_iter().map(convert_json_value).collect();
            EvaluationResult::Collection(items)
        },
        serde_json::Value::Object(obj) => {
            let mut eval_map = std::collections::HashMap::new();
            for (k, v) in obj {
                eval_map.insert(k, convert_json_value(v));
            }
            EvaluationResult::Object { 
                map: eval_map, 
                resource_type: None 
            }
        }
    }
}