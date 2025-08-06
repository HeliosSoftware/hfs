#[cfg(test)]
mod tests {
    use chumsky::Parser;
    use helios_fhirpath::evaluator::{EvaluationContext, evaluate};
    use helios_fhirpath::parser::parser;
    use helios_fhirpath_support::EvaluationResult;
    use std::collections::HashMap;

    // Helper function to create a test object
    fn create_test_resource() -> EvaluationResult {
        let mut patient = HashMap::new();

        // Add resourceType
        patient.insert(
            "resourceType".to_string(),
            EvaluationResult::string("Patient".to_string()),
        );

        // Add id
        patient.insert(
            "id".to_string(),
            EvaluationResult::string("123".to_string()),
        );

        // Add simple property
        patient.insert("active".to_string(), EvaluationResult::boolean(true));

        // Add a complex property (name)
        let mut name = HashMap::new();
        name.insert(
            "use".to_string(),
            EvaluationResult::string("official".to_string()),
        );
        name.insert(
            "family".to_string(),
            EvaluationResult::string("Doe".to_string()),
        );

        // Add name to patient
        patient.insert("name".to_string(), EvaluationResult::object(name));

        // Return as an object
        EvaluationResult::object(patient)
    }

    // Helper function to create a generic object (non-FHIR resource)
    fn create_generic_object() -> EvaluationResult {
        let mut obj = HashMap::new();
        obj.insert(
            "key1".to_string(),
            EvaluationResult::string("value1".to_string()),
        );
        obj.insert("key2".to_string(), EvaluationResult::integer(42));
        EvaluationResult::object(obj)
    }

    #[test]
    fn test_type_function_with_primitives() {
        let context = EvaluationContext::new_empty_with_default_version();

        // Test type() on various primitive types
        let test_cases = vec![
            // (Expression, Expected namespace, Expected name)
            ("true.type()", "System", "Boolean"),
            ("42.type()", "System", "Integer"),
            ("3.14.type()", "System", "Decimal"),
            ("'test'.type()", "System", "String"),
            ("@2021-01-01.type()", "System", "Date"),
            ("@T12:00:00.type()", "System", "Time"),
            ("@2021-01-01T12:00:00.type()", "System", "DateTime"),
            ("10 'mg'.type()", "System", "Quantity"),
        ];

        for (expr, expected_namespace, expected_name) in test_cases {
            let result = evaluate(&parser().parse(expr).unwrap(), &context, None).unwrap();

            match result {
                EvaluationResult::Collection { items, .. } => {
                    assert_eq!(items.len(), 1, "Expected single item for {}", expr);
                    match &items[0] {
                        EvaluationResult::Object { map, .. } => {
                            let namespace = map.get("namespace").unwrap();
                            let name = map.get("name").unwrap();
                            
                            assert_eq!(namespace, &EvaluationResult::String(expected_namespace.to_string(), None), 
                                      "Wrong namespace for {}", expr);
                            assert_eq!(name, &EvaluationResult::String(expected_name.to_string(), None), 
                                      "Wrong name for {}", expr);
                        }
                        _ => panic!("Expected Object in collection for {}, got {:?}", expr, items[0]),
                    }
                }
                _ => panic!("Expected Collection for {}, got {:?}", expr, result),
            }
        }

        // Test empty collection
        let result = evaluate(&parser().parse("{}.type()").unwrap(), &context, None).unwrap();
        assert_eq!(result, EvaluationResult::Empty);
    }

    #[test]
    fn test_type_function_with_collections() {
        let context = EvaluationContext::new_empty_with_default_version();

        // Test type() on collections

        // Empty collection
        let result = evaluate(&parser().parse("{}.type()").unwrap(), &context, None).unwrap();
        assert_eq!(result, EvaluationResult::Empty);

        // Single-item collection (returns type of the item)
        let result = evaluate(&parser().parse("(42).type()").unwrap(), &context, None).unwrap();
        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    EvaluationResult::Object { map, .. } => {
                        assert_eq!(map.get("namespace").unwrap(), &EvaluationResult::String("System".to_string(), None));
                        assert_eq!(map.get("name").unwrap(), &EvaluationResult::String("Integer".to_string(), None));
                    }
                    _ => panic!("Expected Object in collection, got {:?}", items[0]),
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }

        // Multi-item collection (returns collection of types)
        let result = evaluate(
            &parser().parse("(1 | 'test' | true).type()").unwrap(),
            &context,
            None,
        )
        .unwrap();

        match result {
            EvaluationResult::Collection { items: types, .. } => {
                assert_eq!(types.len(), 3);
                
                // Check each type object
                for type_obj in &types {
                    match type_obj {
                        EvaluationResult::Object { map, .. } => {
                            let namespace = map.get("namespace").unwrap();
                            let name = map.get("name").unwrap();
                            
                            assert_eq!(namespace, &EvaluationResult::String("System".to_string(), None));
                            
                            let name_str = match name {
                                EvaluationResult::String(s, _) => s,
                                _ => panic!("Expected string name"),
                            };
                            
                            assert!(["Integer", "String", "Boolean"].contains(&name_str.as_str()));
                        }
                        _ => panic!("Expected Object in collection, got {:?}", type_obj),
                    }
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }
    }

    #[test]
    fn test_type_function_with_fhir_resources() {
        let mut context = EvaluationContext::new_empty_with_default_version();

        // Set up resource in context
        let resource = create_test_resource();
        context.set_this(resource.clone());

        // Test type() on FHIR resource
        let result = evaluate(&parser().parse("$this.type()").unwrap(), &context, None).unwrap();
        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    EvaluationResult::Object { map, .. } => {
                        assert_eq!(map.get("namespace").unwrap(), &EvaluationResult::String("System".to_string(), None));
                        assert_eq!(map.get("name").unwrap(), &EvaluationResult::String("Object".to_string(), None));
                    }
                    _ => panic!("Expected Object in collection, got {:?}", items[0]),
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }

        // Test type() on nested object within FHIR resource
        let result = evaluate(
            &parser().parse("$this.name.type()").unwrap(),
            &context,
            None,
        )
        .unwrap();
        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    EvaluationResult::Object { map, .. } => {
                        assert_eq!(map.get("namespace").unwrap(), &EvaluationResult::String("System".to_string(), None));
                        assert_eq!(map.get("name").unwrap(), &EvaluationResult::String("Object".to_string(), None));
                    }
                    _ => panic!("Expected Object in collection, got {:?}", items[0]),
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }

        // Test type() on property within FHIR resource
        let result = evaluate(&parser().parse("$this.id.type()").unwrap(), &context, None).unwrap();
        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    EvaluationResult::Object { map, .. } => {
                        assert_eq!(map.get("namespace").unwrap(), &EvaluationResult::String("System".to_string(), None));
                        assert_eq!(map.get("name").unwrap(), &EvaluationResult::String("String".to_string(), None));
                    }
                    _ => panic!("Expected Object in collection, got {:?}", items[0]),
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }
    }

    #[test]
    fn test_type_function_with_generic_objects() {
        let mut context = EvaluationContext::new_empty_with_default_version();

        // Set up generic object in context
        let obj = create_generic_object();
        context.set_this(obj.clone());

        // Test type() on generic (non-FHIR) object
        let result = evaluate(&parser().parse("$this.type()").unwrap(), &context, None).unwrap();
        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    EvaluationResult::Object { map, .. } => {
                        assert_eq!(map.get("namespace").unwrap(), &EvaluationResult::String("System".to_string(), None));
                        assert_eq!(map.get("name").unwrap(), &EvaluationResult::String("Object".to_string(), None));
                    }
                    _ => panic!("Expected Object in collection, got {:?}", items[0]),
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }

        // Test type() on property within generic object
        let result = evaluate(
            &parser().parse("$this.key1.type()").unwrap(),
            &context,
            None,
        )
        .unwrap();
        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    EvaluationResult::Object { map, .. } => {
                        assert_eq!(map.get("namespace").unwrap(), &EvaluationResult::String("System".to_string(), None));
                        assert_eq!(map.get("name").unwrap(), &EvaluationResult::String("String".to_string(), None));
                    }
                    _ => panic!("Expected Object in collection, got {:?}", items[0]),
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }

        let result = evaluate(
            &parser().parse("$this.key2.type()").unwrap(),
            &context,
            None,
        )
        .unwrap();
        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    EvaluationResult::Object { map, .. } => {
                        assert_eq!(map.get("namespace").unwrap(), &EvaluationResult::String("System".to_string(), None));
                        assert_eq!(map.get("name").unwrap(), &EvaluationResult::String("Integer".to_string(), None));
                    }
                    _ => panic!("Expected Object in collection, got {:?}", items[0]),
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }
    }

    #[test]
    fn test_type_function_chaining() {
        let mut context = EvaluationContext::new_empty_with_default_version();
        context.set_this(create_test_resource());

        // Test chaining type() with other operations

        // Type of the type result (should be Type object)
        let result = evaluate(
            &parser().parse("$this.type().type()").unwrap(),
            &context,
            None,
        )
        .unwrap();
        match result {
            EvaluationResult::Collection { items, .. } => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    EvaluationResult::Object { map, .. } => {
                        assert_eq!(map.get("namespace").unwrap(), &EvaluationResult::String("System".to_string(), None));
                        assert_eq!(map.get("name").unwrap(), &EvaluationResult::String("Type".to_string(), None));
                    }
                    _ => panic!("Expected Object in collection, got {:?}", items[0]),
                }
            }
            _ => panic!("Expected Collection, got {:?}", result),
        }

        // Type used in conditional - accessing the name property
        let result = evaluate(
            &parser().parse("$this.type().name = 'Object' implies $this.id.exists()").unwrap(),
            &context,
            None,
        )
        .unwrap();
        assert_eq!(result, EvaluationResult::boolean(true));
    }
}

