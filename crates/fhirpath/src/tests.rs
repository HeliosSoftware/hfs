use crate::parser::{parser, Expression};
use std::fs::File;
use std::io::Read;
use roxmltree::{Document, Node};

#[test]
fn test_parse_first_expression() {
    // Parse the first expression from the test file
    let expr = "birthDate";
    let result = parser().parse(expr);
    
    assert!(result.is_ok(), "Failed to parse expression: {:?}", result.err());
    println!("Successfully parsed: {:?}", result.unwrap());
}

#[test]
fn test_load_test_file() {
    // Load the test file
    let mut file = File::open("resources/r4/tests-fhir-r4.xml").expect("Failed to open test file");
    let mut contents = String::new();
    file.read_to_string(&mut contents).expect("Failed to read test file");
    
    // Parse the XML
    let doc = Document::parse(&contents).expect("Failed to parse XML");
    
    // Find the first test expression
    let first_test = find_first_test(&doc.root_element());
    assert!(first_test.is_some(), "Failed to find first test");
    
    let first_test = first_test.unwrap();
    println!("First test: {}", first_test);
    
    // Parse the expression
    let result = parser().parse(&first_test);
    assert!(result.is_ok(), "Failed to parse expression: {:?}", result.err());
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
                            return expr_node.text();
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
