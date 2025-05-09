fn apply_type_operation(
    value: &EvaluationResult,
    op: &str,
    type_spec: &TypeSpecifier,
) -> Result<EvaluationResult, EvaluationError> {
    match op {
        "is" => {
            // Extract the namespace and name from the type specifier
            let (namespace, type_name) = match type_spec {
                TypeSpecifier::QualifiedIdentifier(ns_or_name, Some(name)) => {
                    // Qualified name: System.Integer or FHIR.Patient
                    (Some(ns_or_name.as_str()), name.as_str())
                },
                TypeSpecifier::QualifiedIdentifier(name, None) => {
                    // Unqualified name: Integer or Patient
                    // Determine the appropriate default namespace
                    let default_ns = crate::fhir_type_hierarchy::determine_type_namespace(name);
                    (Some(default_ns), name.as_str())
                },
            };

            // Handle singleton evaluation: 'is' errors on multi-item collections
            if value.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "'is' operator requires a singleton input".to_string(),
                ));
            }

            let is_type = match value {
                // Empty is not of any type
                EvaluationResult::Empty => false,
                
                // Collections should be handled by initial check, but handle anyway
                EvaluationResult::Collection(_) => false,
                
                // Handle primitive types - first handle System namespace
                EvaluationResult::Boolean(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Boolean") || 
                                      type_name.eq_ignore_ascii_case("boolean");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("boolean"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::String(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("String") || 
                                      type_name.eq_ignore_ascii_case("string");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("string"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Integer(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Integer") || 
                                      type_name.eq_ignore_ascii_case("integer");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("integer"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Decimal(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Decimal") || 
                                      type_name.eq_ignore_ascii_case("decimal");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("decimal"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Date(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Date") || 
                                      type_name.eq_ignore_ascii_case("date");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("date"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::DateTime(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("DateTime") || 
                                      type_name.eq_ignore_ascii_case("dateTime");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("dateTime"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Time(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Time") || 
                                      type_name.eq_ignore_ascii_case("time");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("time"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Quantity(_, _) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Quantity") || 
                                      type_name.eq_ignore_ascii_case("quantity");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true);
                    matches_type && matches_ns
                },
                
                // Special handling for FHIR objects and resources
                EvaluationResult::Object(obj) => {
                    // Check if this is a FHIR resource by looking for resourceType property
                    if let Some(resource_type) = obj.get("resourceType") {
                        if let EvaluationResult::String(resource_type_str) = resource_type {
                            // Check if looking for a specific resource type
                            let type_matches = type_name.eq_ignore_ascii_case(resource_type_str);
                            
                            // Check if the type is a parent type like Resource or DomainResource
                            let type_is_parent = crate::fhir_type_hierarchy::is_derived_from(
                                resource_type_str, type_name
                            );
                            
                            // For resource types, we always require the FHIR namespace
                            let namespace_matches = namespace.map(|ns| {
                                ns.eq_ignore_ascii_case("FHIR") || 
                                ns.eq_ignore_ascii_case("http://hl7.org/fhir")
                            }).unwrap_or(true);
                            
                            (type_matches || type_is_parent) && namespace_matches
                        } else {
                            false
                        }
                    } else {
                        // This could be a FHIR primitive type (boolean, string, etc.)
                        // Let's do a more generic check
                        
                        // Check for FHIR primitive types with .value
                        if obj.contains_key("value") {
                            // This is likely a FHIR primitive
                            if let Some(value) = obj.get("value") {
                                match value {
                                    EvaluationResult::Boolean(_) => 
                                        type_name.eq_ignore_ascii_case("boolean"),
                                    EvaluationResult::String(_) => 
                                        type_name.eq_ignore_ascii_case("string"),
                                    EvaluationResult::Integer(_) => 
                                        type_name.eq_ignore_ascii_case("integer"),
                                    EvaluationResult::Decimal(_) => 
                                        type_name.eq_ignore_ascii_case("decimal"),
                                    EvaluationResult::Date(_) => 
                                        type_name.eq_ignore_ascii_case("date"),
                                    EvaluationResult::DateTime(_) => 
                                        type_name.eq_ignore_ascii_case("dateTime"),
                                    EvaluationResult::Time(_) => 
                                        type_name.eq_ignore_ascii_case("time"),
                                    _ => false
                                }
                            } else {
                                false
                            }
                        } else if type_name.eq_ignore_ascii_case("Quantity") && 
                                 obj.contains_key("value") && 
                                 obj.contains_key("unit") {
                            // This is a FHIR Quantity
                            true
                        } else {
                            // Check for FHIR complex types
                            let is_complex_type = crate::fhir_type_hierarchy::is_fhir_complex_type(type_name);
                            if is_complex_type {
                                // Hard to determine if an object is specifically a HumanName, Address, etc.
                                // without custom code for each type - for now, return false
                                // This would require more comprehensive property checking
                                false
                            } else {
                                // Not a recognized type
                                false
                            }
                        }
                    }
                },
            };

            Ok(EvaluationResult::Boolean(is_type))
        }
        "as" => {
            // Extract the namespace and name from the type specifier
            let (namespace, type_name) = match type_spec {
                TypeSpecifier::QualifiedIdentifier(ns_or_name, Some(name)) => {
                    // Qualified name: System.Integer or FHIR.Patient
                    (Some(ns_or_name.as_str()), name.as_str())
                },
                TypeSpecifier::QualifiedIdentifier(name, None) => {
                    // Unqualified name: Integer or Patient
                    // Determine the appropriate default namespace
                    let default_ns = crate::fhir_type_hierarchy::determine_type_namespace(name);
                    (Some(default_ns), name.as_str())
                },
            };

            // Handle singleton evaluation: 'as' errors on multi-item collections
            if value.count() > 1 {
                return Err(EvaluationError::SingletonEvaluationError(
                    "'as' operator requires a singleton input".to_string(),
                ));
            }

            // First check if the value is of the specified type using our 'is' logic
            // We'll reuse the same logic that we use for 'is', but instead of returning
            // a boolean result, we'll return the value or Empty
            let is_type = match value {
                // Empty is not of any type, so return Empty
                EvaluationResult::Empty => false,
                
                // Collections should be handled by initial check, but handle anyway
                EvaluationResult::Collection(_) => false,
                
                // Handle primitive types
                EvaluationResult::Boolean(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Boolean") || 
                                      type_name.eq_ignore_ascii_case("boolean");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("boolean"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::String(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("String") || 
                                      type_name.eq_ignore_ascii_case("string");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("string"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Integer(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Integer") || 
                                      type_name.eq_ignore_ascii_case("integer");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("integer"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Decimal(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Decimal") || 
                                      type_name.eq_ignore_ascii_case("decimal");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("decimal"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Date(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Date") || 
                                      type_name.eq_ignore_ascii_case("date");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("date"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::DateTime(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("DateTime") || 
                                      type_name.eq_ignore_ascii_case("dateTime");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("dateTime"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Time(_) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Time") || 
                                      type_name.eq_ignore_ascii_case("time");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true) ||
                                    type_name.eq_ignore_ascii_case("time"); // FHIR lowercase primitive
                    matches_type && matches_ns
                },
                EvaluationResult::Quantity(_, _) => {
                    let matches_type = type_name.eq_ignore_ascii_case("Quantity") || 
                                      type_name.eq_ignore_ascii_case("quantity");
                    let matches_ns = namespace.map(|ns| ns.eq_ignore_ascii_case("System")).unwrap_or(true);
                    matches_type && matches_ns
                },
                
                // Special handling for FHIR objects and resources
                EvaluationResult::Object(obj) => {
                    // Check if this is a FHIR resource by looking for resourceType property
                    if let Some(resource_type) = obj.get("resourceType") {
                        if let EvaluationResult::String(resource_type_str) = resource_type {
                            // Check if looking for a specific resource type
                            let type_matches = type_name.eq_ignore_ascii_case(resource_type_str);
                            
                            // Check if the type is a parent type like Resource or DomainResource
                            let type_is_parent = crate::fhir_type_hierarchy::is_derived_from(
                                resource_type_str, type_name
                            );
                            
                            // For resource types, we always require the FHIR namespace
                            let namespace_matches = namespace.map(|ns| {
                                ns.eq_ignore_ascii_case("FHIR") || 
                                ns.eq_ignore_ascii_case("http://hl7.org/fhir")
                            }).unwrap_or(true);
                            
                            (type_matches || type_is_parent) && namespace_matches
                        } else {
                            false
                        }
                    } else {
                        // This could be a FHIR primitive type or complex type
                        
                        // Check for FHIR primitive types with .value
                        if obj.contains_key("value") {
                            if let Some(value) = obj.get("value") {
                                match value {
                                    EvaluationResult::Boolean(_) => 
                                        type_name.eq_ignore_ascii_case("boolean"),
                                    EvaluationResult::String(_) => 
                                        type_name.eq_ignore_ascii_case("string"),
                                    EvaluationResult::Integer(_) => 
                                        type_name.eq_ignore_ascii_case("integer"),
                                    EvaluationResult::Decimal(_) => 
                                        type_name.eq_ignore_ascii_case("decimal"),
                                    EvaluationResult::Date(_) => 
                                        type_name.eq_ignore_ascii_case("date"),
                                    EvaluationResult::DateTime(_) => 
                                        type_name.eq_ignore_ascii_case("dateTime"),
                                    EvaluationResult::Time(_) => 
                                        type_name.eq_ignore_ascii_case("time"),
                                    _ => false
                                }
                            } else {
                                false
                            }
                        } else if type_name.eq_ignore_ascii_case("Quantity") && 
                                 obj.contains_key("value") && 
                                 obj.contains_key("unit") {
                            // This is a FHIR Quantity
                            true
                        } else {
                            // Check for FHIR complex types
                            let is_complex_type = crate::fhir_type_hierarchy::is_fhir_complex_type(type_name);
                            if is_complex_type {
                                // Hard to determine if an object is specifically a HumanName, Address, etc.
                                // For now, do a heuristic check based on property presence
                                match type_name.to_lowercase().as_str() {
                                    "humanname" => obj.contains_key("given") || obj.contains_key("family"),
                                    "address" => obj.contains_key("city") || obj.contains_key("postalCode"),
                                    "contactpoint" => obj.contains_key("system") || obj.contains_key("value"),
                                    "period" => obj.contains_key("start") || obj.contains_key("end"),
                                    "codeableconcept" => obj.contains_key("coding") || obj.contains_key("text"),
                                    "coding" => obj.contains_key("system") || obj.contains_key("code"),
                                    _ => false
                                }
                            } else {
                                // Not a recognized type
                                false
                            }
                        }
                    }
                },
            };
            
            // If the value is of the specified type, return it, otherwise return Empty
            Ok(if is_type {
                value.clone()
            } else {
                EvaluationResult::Empty
            })
        }
        _ => Err(EvaluationError::InvalidOperation(format!(
            "Unknown type operator: {}",
            op
        ))),
    }
}
