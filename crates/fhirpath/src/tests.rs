use crate::parser::{parser, Expression};
use chumsky::Parser;
use roxmltree::{Document, Node};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

#[test]
fn test_parse_simple_expressions() {
    let test_cases = vec![
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
fn test_load_test_file() {
    // Get the path to the test file
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("resources/r4/tests-fhir-r4.xml");

    // Load the test file
    let mut file = File::open(path).expect("Failed to open test file");
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .expect("Failed to read test file");

    // Parse the XML
    let doc = Document::parse(&contents).expect("Failed to parse XML");

    // Find the first test expression
    let first_test = find_first_test(&doc.root_element());
    assert!(first_test.is_some(), "Failed to find first test");

    let first_test = first_test.unwrap();
    println!("First test: {}", first_test);

    // Replace any XML-escaped characters
    let processed_test = first_test
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&");
    
    // Parse the expression
    let result = parser().parse(&processed_test);
    assert!(
        result.is_ok(),
        "Failed to parse expression: {:?}",
        result.err()
    );
    println!("Successfully parsed: {:?}", result.unwrap());
}

fn find_first_test(node: &Node) -> Option<String> {
    // Look for the first expression element
    for child in node.children() {
        if child.has_tag_name("group") {
            for test_node in child.children() {
                if test_node.has_tag_name("test") {
                    for expr_node in test_node.children() {
                        if expr_node.has_tag_name("expression") {
                            return expr_node.text().map(|s| s.to_string());
                        }
                    }
                }
            }
        }

        // Recursively search
        let result = find_first_test(&child);
        if result.is_some() {
            return result;
        }
    }

    None
}
