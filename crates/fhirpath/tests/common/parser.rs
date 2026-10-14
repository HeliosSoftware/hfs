use roxmltree::{Document, Node};

// Struct to hold test information from XML
#[derive(Debug)]
pub struct TestInfo {
    pub name: String,
    #[allow(dead_code)]
    pub description: String,
    pub input_file: String,
    pub invalid: String,
    pub predicate: String,
    pub mode: String,
    pub check_ordered_functions: String,
    /// `context` attribute: an expression whose results each become `$this`.
    pub context: String,
    /// `contextType` attribute: the declared type(s) of that focus, `|`-separated.
    pub context_type: String,
    pub expression: String,
    pub outputs: Vec<(String, String)>, // (type, value)
}

pub fn find_test_groups(root: &Node) -> Vec<(String, Vec<TestInfo>)> {
    let mut groups = Vec::new();

    // Find all group elements
    for group in root.descendants().filter(|n| n.has_tag_name("group")) {
        let name = group.attribute("name").unwrap_or("unnamed").to_string();
        let mut tests = Vec::new();

        // Find all test elements within this group and collect their info
        for test in group.children().filter(|n| n.has_tag_name("test")) {
            let test_name = test.attribute("name").unwrap_or("unnamed").to_string();
            let description = test.attribute("description").unwrap_or("").to_string();
            let input_file = test.attribute("inputfile").unwrap_or("").to_string();
            let mode = test.attribute("mode").unwrap_or("").to_string();
            let predicate = test.attribute("predicate").unwrap_or("").to_string();
            let context = test.attribute("context").unwrap_or("").to_string();
            let context_type = test.attribute("contextType").unwrap_or("").to_string();
            let check_ordered_functions = test
                .attribute("checkOrderedFunctions")
                .unwrap_or("")
                .to_string();

            // Find the expression node to get its text and 'invalid' attribute
            let expression_node_opt = test.children().find(|n| n.has_tag_name("expression"));

            let expression_text = expression_node_opt
                .as_ref()
                .and_then(|n| n.text())
                .unwrap_or("")
                .to_string();

            // Try to read 'invalid' attribute from <expression> tag first
            let mut invalid_attr_val = expression_node_opt
                .and_then(|n| n.attribute("invalid"))
                .unwrap_or("")
                .to_string();

            // If not found on <expression>, try to read from <test> tag
            if invalid_attr_val.is_empty() {
                invalid_attr_val = test.attribute("invalid").unwrap_or("").to_string();
            }

            // Find expected outputs
            let mut outputs = Vec::new();
            for output in test.children().filter(|n| n.has_tag_name("output")) {
                let output_type = output.attribute("type").unwrap_or("").to_string();
                let output_value = output.text().unwrap_or("").to_string();
                outputs.push((output_type, output_value));
            }

            tests.push(TestInfo {
                name: test_name,
                description,
                input_file,
                invalid: invalid_attr_val,
                predicate,
                mode,
                check_ordered_functions,
                context,
                context_type,
                expression: expression_text,
                outputs,
            });
        }

        if !tests.is_empty() {
            groups.push((name, tests));
        }
    }

    groups
}

pub fn parse_test_xml(contents: &str) -> Result<Document<'_>, String> {
    // Parse the XML with relaxed parsing options
    Document::parse_with_options(
        contents,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| format!("XML parsing failed: {:?}", e))
}
