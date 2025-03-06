use crate::parser::{Expression, parser};
use chumsky::Parser;
use chumsky::error::Simple;
use std::fmt::Debug;

/// A debug wrapper for the FHIRPath parser that logs parsing steps
pub fn debug_parse(input: &str) -> Result<Expression, Vec<String>> {
    println!("Parsing input: {}", input);

    // Use a non-recursive approach to avoid stack overflow
    let result = parser().parse(input);

    match &result {
        Ok(_expr) => {
            // Don't print the full expression to avoid deep recursion in Debug formatting
            println!("Successfully parsed expression");
        }
        Err(errors) => {
            println!("Parse errors:");
            for err in errors {
                println!("  - {:?}", err);
            }
        }
    }

    result.map_err(|errors| errors.iter().map(|e| format!("{:?}", e)).collect())
}

/// Trace execution of a parser with detailed logging
pub fn trace_parse<T: Debug>(
    input: &str,
    parser_name: &str,
    parser_fn: impl Parser<char, T, Error = Simple<char>>,
) -> Result<T, Vec<String>> {
    println!("Tracing parser '{}' with input: {}", parser_name, input);

    let result = parser_fn.parse(input);

    match &result {
        Ok(value) => {
            println!(
                "Parser '{}' succeeded with result: {:?}",
                parser_name, value
            );
        }
        Err(errors) => {
            println!("Parser '{}' failed with errors:", parser_name);
            for err in errors {
                println!("  - {:?}", err);
            }
        }
    }

    result.map_err(|errors| errors.iter().map(|e| format!("{:?}", e)).collect())
}
