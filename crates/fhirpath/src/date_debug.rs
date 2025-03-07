use crate::debug::{debug_parse, trace_parse};
use crate::parser::{Literal, Term, Expression};
use chumsky::prelude::*;

/// Run a series of debug tests to diagnose date parsing issues
pub fn run_date_debug_tests() {
    println!("\n=== RUNNING DATE PARSING DEBUG TESTS ===\n");
    
    // Test 1: Try parsing the full expression
    println!("Test 1: Full expression '@2015.is(Date)'");
    let result = debug_parse("@2015.is(Date)");
    println!("Result: {:?}\n", result);
    
    // Test 2: Try parsing just the date part
    println!("Test 2: Just the date '@2015'");
    let result = debug_parse("@2015");
    println!("Result: {:?}\n", result);
    
    // Test 3: Test the date format parser directly
    println!("Test 3: Testing year-only date format");
    let year_only = text::int::<char, Simple<char>>(10);
    
    let result = trace_parse("2015", "year_only", year_only);
    println!("Year only result: {:?}\n", result);
    
    // Test 4: Test date with @ symbol
    println!("Test 4: Testing '@' + year format");
    let date_with_at = just('@')
        .ignore_then(text::int::<char, Simple<char>>(10));
    
    let result = trace_parse("@2015", "date_with_at", date_with_at);
    println!("Date with @ result: {:?}\n", result);
    
    // Test 5: Test date followed by dot
    println!("Test 5: Testing '@2015.'");
    let date_with_dot = just('@')
        .ignore_then(text::int::<char, Simple<char>>(10))
        .then_ignore(just('.'));
    
    let result = trace_parse("@2015.", "date_with_dot", date_with_dot);
    println!("Date with dot result: {:?}\n", result);
    
    // Test 6: Test with different date formats
    println!("Test 6: Testing different date formats");
    let formats = ["@2015", "@2015-01", "@2015-01-01"];
    
    for format in formats {
        println!("Testing format: {}", format);
        let result = debug_parse(format);
        println!("Result: {:?}", result);
    }
    
    // Test 6b: Test specific date format parsers
    println!("\nTest 6b: Testing specific date format parsers");
    
    // Year-month format
    let year_month = text::int::<char, Simple<char>>(10)
        .then(just('-'))
        .then(text::int::<char, Simple<char>>(10))
        .map(|((year, _), month)| format!("{}-{}", year, month));
    
    let result = trace_parse("2015-01", "year_month", year_month);
    println!("Year-month result: {:?}", result);
    
    // Full date format
    let full_date = text::int::<char, Simple<char>>(10)
        .then(just('-'))
        .then(text::int::<char, Simple<char>>(10))
        .then(just('-'))
        .then(text::int::<char, Simple<char>>(10))
        .map(|((((year, _), month), _), day)| format!("{}-{}-{}", year, month, day));
    
    let result = trace_parse("2015-01-01", "full_date", full_date);
    println!("Full date result: {:?}\n", result);
    
    // Test 7: Test with different date formats followed by .is(Date)
    println!("\nTest 7: Testing different date formats with .is(Date)");
    let formats_with_is = ["@2015.is(Date)", "@2015-01.is(Date)", "@2015-01-01.is(Date)"];
    
    for format in formats_with_is {
        println!("Testing format: {}", format);
        let result = debug_parse(format);
        println!("Result: {:?}", result);
    }
    
    // Test 8: Test a custom parser for the specific expression
    println!("\nTest 8: Custom parser for '@2015.is(Date)'");
    let date_literal = just('@')
        .ignore_then(text::int::<char, Simple<char>>(10))
        .map(Literal::Date)
        .map(Term::Literal)
        .map(Expression::Term);
    
    let is_date_expr = date_literal
        .then_ignore(just('.'))
        .then(
            just("is")
                .then_ignore(just('('))
                .then(just("Date"))
                .then_ignore(just(')'))
                .map(|(_, type_name)| type_name)
        )
        .map(|(expr, _)| expr);
    
    let result = trace_parse("@2015.is(Date)", "custom_date_is_expr", is_date_expr);
    println!("Custom parser result: {:?}\n", result);
    
    // Test 9: Test hyphenated date formats with custom parsers
    println!("\nTest 9: Testing hyphenated date formats with custom parsers");
    
    // Custom date parser for @YYYY-MM
    let year_month_date = just('@')
        .ignore_then(
            text::int::<char, Simple<char>>(10)
                .then(just('-'))
                .then(text::int::<char, Simple<char>>(10))
                .map(|((year, _), month)| format!("{}-{}", year, month))
        )
        .map(Literal::Date)
        .map(Term::Literal)
        .map(Expression::Term);
    
    let result = trace_parse("@2015-01", "custom_year_month", year_month_date);
    println!("Custom year-month result: {:?}", result);
    
    // Custom date parser for @YYYY-MM-DD
    let full_date_parser = just('@')
        .ignore_then(
            text::int::<char, Simple<char>>(10)
                .then(just('-'))
                .then(text::int::<char, Simple<char>>(10))
                .then(just('-'))
                .then(text::int::<char, Simple<char>>(10))
                .map(|((((year, _), month), _), day)| format!("{}-{}-{}", year, month, day))
        )
        .map(Literal::Date)
        .map(Term::Literal)
        .map(Expression::Term);
    
    let result = trace_parse("@2015-01-01", "custom_full_date", full_date_parser);
    println!("Custom full date result: {:?}\n", result);
    
    println!("\n=== DATE PARSING DEBUG TESTS COMPLETE ===\n");
}
