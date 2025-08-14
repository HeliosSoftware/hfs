use helios_fhirpath_support::{EvaluationError, EvaluationResult, TypeInfoResult};
use std::collections::HashMap;

/// FHIRPath type() function implementation
///
/// Returns a Object with namespace and name properties representing the type of each item
/// in the input collection.
pub fn type_function(
    invocation_base: &EvaluationResult,
    _args: &[EvaluationResult],
) -> Result<EvaluationResult, EvaluationError> {
    match invocation_base {
        EvaluationResult::Empty => Ok(EvaluationResult::Empty),
        EvaluationResult::Collection { items, .. } => {
            let type_objects: Vec<EvaluationResult> =
                items.iter().map(create_type_object).collect();

            Ok(EvaluationResult::Collection {
                items: type_objects,
                has_undefined_order: false,
                type_info: None,
            })
        }
        _ => {
            // Single item, return its type as a collection with one object
            let type_object = create_type_object(invocation_base);
            Ok(EvaluationResult::Collection {
                items: vec![type_object],
                has_undefined_order: false,
                type_info: None,
            })
        }
    }
}

/// Creates a type object with namespace and name properties
fn create_type_object(value: &EvaluationResult) -> EvaluationResult {
    let (namespace, name) = get_type_info(value);

    let mut map = HashMap::new();
    map.insert(
        "namespace".to_string(),
        EvaluationResult::String(namespace, None),
    );
    map.insert("name".to_string(), EvaluationResult::String(name, None));

    EvaluationResult::Object {
        map,
        type_info: Some(TypeInfoResult {
            namespace: "System".to_string(),
            name: "Type".to_string(),
        }),
    }
}

/// Gets the type information (namespace, name) for an EvaluationResult
fn get_type_info(value: &EvaluationResult) -> (String, String) {
    match value {
        EvaluationResult::Boolean(_, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Boolean".to_string())
            }
        }
        EvaluationResult::Integer(_, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Integer".to_string())
            }
        }
        EvaluationResult::Decimal(_, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Decimal".to_string())
            }
        }
        EvaluationResult::String(_, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "String".to_string())
            }
        }
        EvaluationResult::Date(_, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Date".to_string())
            }
        }
        EvaluationResult::DateTime(_, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "DateTime".to_string())
            }
        }
        EvaluationResult::Time(_, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Time".to_string())
            }
        }
        EvaluationResult::Quantity(_, _, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Quantity".to_string())
            }
        }
        EvaluationResult::Object { type_info, .. } => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Object".to_string())
            }
        }
        EvaluationResult::Collection { type_info, .. } => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Collection".to_string())
            }
        }
        EvaluationResult::Empty => ("System".to_string(), "Empty".to_string()),
        #[cfg(not(any(feature = "R4", feature = "R4B")))]
        EvaluationResult::Integer64(_, type_info) => {
            if let Some(type_info) = type_info {
                (type_info.namespace.clone(), type_info.name.clone())
            } else {
                ("System".to_string(), "Integer64".to_string())
            }
        }
        #[cfg(any(feature = "R4", feature = "R4B"))]
        EvaluationResult::Integer64(_, _) => {
            // In R4 and R4B, Integer64 should be treated as Integer
            ("System".to_string(), "Integer".to_string())
        }
    }
}
