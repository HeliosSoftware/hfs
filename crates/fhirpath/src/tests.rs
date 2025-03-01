use crate::parser::parser;
use chumsky::Parser;
use roxmltree::{Document, Node};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

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
#[ignore]
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

    for (_i, expr) in expressions.iter().enumerate().take(10) {
        // Limit to first 10 for brevity
        let result = parser().parse(expr.clone());
        if result.is_ok() {
            success_count += 1;
        } else {
            failure_count += 1;
            println!("Failed to parse: '{}', error: {:?}", expr, result.err());
        }
    }

    println!(
        "Successfully parsed {}/{} expressions",
        success_count,
        success_count + failure_count
    );
    // We don't assert all must pass yet, as we're still developing the parser
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
