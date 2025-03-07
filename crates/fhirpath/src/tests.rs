use crate::parser::parser;
use chumsky::Parser;
use roxmltree::{Document, Node};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

#[test]
fn test_date_formats() {
    // Test just the simplest case first
    let expr = "@2015";
    println!("Attempting to parse: '{}'", expr);
    let result = parser().parse(expr);
    
    // Print detailed error information if it fails
    if let Err(ref err) = result {
        println!("Error parsing '@2015': {:?}", err);
        for e in err {
            println!("  Span: {:?}, Reason: {:?}", 
                     e.span(), e.reason());
        }
    }
    
    assert!(
        result.is_ok(),
        "Failed to parse date expression: '{}', error: {:?}",
        expr,
        result.err()
    );
    
    if let Ok(parsed) = result {
        println!("Successfully parsed '@2015': {:?}", parsed);
    }
    
    // Now test the other date formats
    let date_expressions = vec![
        "@2015.is(Date)",
        "@2015-01",
        "@2015-01-01",
        "@T12:00",
        "@2015T12:00",
    ];

    for expr in date_expressions {
        let result = parser().parse(expr);
        assert!(
            result.is_ok(),
            "Failed to parse date expression: '{}', error: {:?}",
            expr,
            result.err()
        );
    }
}

#[test]
fn test_simple_date_parsing() {
    // Test year-only format
    let result = parser().parse("@2015");
    assert!(
        result.is_ok(),
        "Failed to parse simple date '@2015', error: {:?}",
        result.err()
    );
    
    // Test year-month format
    let result = parser().parse("@2015-01");
    assert!(
        result.is_ok(),
        "Failed to parse date '@2015-01', error: {:?}",
        result.err()
    );
    
    // Test full date format
    let result = parser().parse("@2015-01-01");
    assert!(
        result.is_ok(),
        "Failed to parse date '@2015-01-01', error: {:?}",
        result.err()
    );
}

#[test]
fn test_parse_simple_expressions() {
    let test_cases = vec![
        "true",
        "birthDate",
        "Patient.name.given",
        "Patient.name.where(given = 'Jim')",
        "1 + 2 * 3",
        "true and false",
        "Patient.name.exists()",
        "name.take(2).given",
        "@2015",
        "@2015.is(Date)",
        "@2015-01",
        "@2015-01-01",
    ];

    for expr in test_cases {
        let result = parser().parse(expr);
        assert!(
            result.is_ok(),
            "Failed to parse expression: '{}', error: {:?}",
            expr,
            result.err()
        );
        println!("Successfully parsed '{}': {:?}", expr, result.unwrap());
    }
}

#[test]
fn test_multiple_expressions_from_file() {
    // Get the path to the test file
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("resources/r4/tests-fhir-r4.xml");

    // Load the test file or use a fallback if it doesn't exist
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(e) => {
            println!("Warning: Could not open test file: {:?}.", e);
            return;
        }
    };
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .expect("Failed to read test file");

    // Parse the XML with relaxed parsing options
    let doc = match Document::parse_with_options(
        &contents,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    ) {
        Ok(doc) => doc,
        Err(e) => {
            println!("Warning: XML parsing failed: {:?}.", e);
            panic!("XML parsing failed, cannot continue test");
        }
    };

    // Find all test expressions
    let expressions = find_test_expressions(&doc.root_element());

    // Make sure we found some expressions
    assert!(!expressions.is_empty(), "No test expressions found");
    println!("Found {} test expressions", expressions.len());

    // Try to parse each expression
    let mut success_count = 0;
    let mut failure_count = 0;

    for expr in expressions.iter() {
        let result = parser().parse(expr.clone());
        if result.is_ok() {
            success_count += 1;
        } else {
            failure_count += 1;
            println!("Failed to parse: '{}', error: {:?}", expr, result.err());
        }
    }

    // Use eprintln! instead of println! to ensure output appears during cargo test
    eprintln!(
        "Successfully parsed {}/{} expressions",
        success_count,
        success_count + failure_count
    );

    assert!(failure_count == 0, "There are test failures.");
}

fn find_test_expressions(root: &Node) -> Vec<String> {
    let mut expressions = Vec::new();
    collect_expressions(root, &mut expressions);
    expressions
}

fn collect_expressions(node: &Node, expressions: &mut Vec<String>) {
    // Process this node if it's an expression
    if node.has_tag_name("expression") {
        if let Some(text) = node.text() {
            expressions.push(text.to_string());
        }
    }

    // Process all children
    for child in node.children() {
        if child.is_element() {
            collect_expressions(&child, expressions);
        }
    }
}
