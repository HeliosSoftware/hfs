//! # Version-Agnostic FHIR Abstraction Traits
//!
//! This module provides trait abstractions that enable the SOF crate to work
//! with ViewDefinitions and Bundles across multiple FHIR versions without
//! duplicating transformation logic. Each FHIR version implements these traits
//! to provide uniform access to their specific data structures.
//!
//! ## Architecture
//!
//! The trait system follows a hierarchical pattern:
//! - Top-level container traits ([`ViewDefinitionTrait`], [`BundleTrait`])
//! - Component traits ([`ViewDefinitionSelectTrait`], [`ViewDefinitionColumnTrait`], etc.)
//! - Version-specific implementations for R4, R4B, R5, and R6
//!
//! ## Design Benefits
//!
//! - **Version Independence**: Core processing logic works with any FHIR version
//! - **Type Safety**: Compile-time verification of trait implementations
//! - **Extensibility**: Easy addition of new FHIR versions or features
//! - **Code Reuse**: Single implementation handles all supported versions

use crate::SofError;
use fhir::FhirResource;
use fhirpath::EvaluationResult;

/// Trait for abstracting ViewDefinition across FHIR versions.
///
/// This trait provides version-agnostic access to ViewDefinition components,
/// enabling the core processing logic to work uniformly across R4, R4B, R5,
/// and R6 specifications. Each FHIR version implements this trait to expose
/// its ViewDefinition structure through a common interface.
///
/// # Associated Types
///
/// - [`Select`](Self::Select): The select statement type for this FHIR version
/// - [`Where`](Self::Where): The where clause type for this FHIR version  
/// - [`Constant`](Self::Constant): The constant definition type for this FHIR version
///
/// # Examples
///
/// ```rust
/// use sof::traits::ViewDefinitionTrait;
///
/// fn process_any_version<T: ViewDefinitionTrait>(vd: &T) {
///     if let Some(resource_type) = vd.resource() {
///         println!("Processing {} resources", resource_type);
///     }
///     
///     if let Some(selects) = vd.select() {
///         println!("Found {} select statements", selects.len());
///     }
/// }
/// ```
pub trait ViewDefinitionTrait {
    /// The select statement type for this FHIR version
    type Select: ViewDefinitionSelectTrait;
    /// The where clause type for this FHIR version
    type Where: ViewDefinitionWhereTrait;
    /// The constant definition type for this FHIR version
    type Constant: ViewDefinitionConstantTrait;

    /// Returns the FHIR resource type this ViewDefinition processes
    fn resource(&self) -> Option<&str>;
    /// Returns the select statements that define output columns and structure
    fn select(&self) -> Option<&[Self::Select]>;
    /// Returns the where clauses that filter resources before processing
    fn where_clauses(&self) -> Option<&[Self::Where]>;
    /// Returns the constants/variables available for use in expressions
    fn constants(&self) -> Option<&[Self::Constant]>;
}

/// Trait for abstracting ViewDefinitionSelect across FHIR versions.
///
/// This trait provides version-agnostic access to select statement components,
/// including columns, nested selects, iteration constructs, and union operations.
/// Select statements define the structure and content of the output table.
///
/// # Associated Types
///
/// - [`Column`](Self::Column): The column definition type for this FHIR version
/// - [`Select`](Self::Select): Recursive select type for nested structures
///
/// # Key Features
///
/// - **Column Definitions**: Direct column mappings from FHIRPath to output
/// - **Nested Selects**: Hierarchical select structures for complex transformations
/// - **Iteration**: `forEach` and `forEachOrNull` for processing collections
/// - **Union Operations**: `unionAll` for combining multiple select results
///
/// # Examples
///
/// ```rust
/// use sof::traits::ViewDefinitionSelectTrait;
///
/// fn analyze_select<T: ViewDefinitionSelectTrait>(select: &T) {
///     if let Some(columns) = select.column() {
///         println!("Found {} columns", columns.len());
///     }
///     
///     if let Some(for_each) = select.for_each() {
///         println!("Iterating over: {}", for_each);
///     }
///     
///     if let Some(union_selects) = select.union_all() {
///         println!("Union with {} other selects", union_selects.len());
///     }
/// }
/// ```
pub trait ViewDefinitionSelectTrait {
    /// The column definition type for this FHIR version
    type Column: ViewDefinitionColumnTrait;
    /// Recursive select type for nested structures
    type Select: ViewDefinitionSelectTrait;

    /// Returns the column definitions for this select statement
    fn column(&self) -> Option<&[Self::Column]>;
    /// Returns nested select statements for hierarchical processing
    fn select(&self) -> Option<&[Self::Select]>;
    /// Returns the FHIRPath expression for forEach iteration (filters out empty collections)
    fn for_each(&self) -> Option<&str>;
    /// Returns the FHIRPath expression for forEachOrNull iteration (includes null rows for empty collections)
    fn for_each_or_null(&self) -> Option<&str>;
    /// Returns select statements to union with this one (all results combined)
    fn union_all(&self) -> Option<&[Self::Select]>;
}

/// Trait for abstracting ViewDefinitionColumn across FHIR versions.
///
/// This trait provides version-agnostic access to column definitions,
/// which specify how to extract data from FHIR resources and map it
/// to output table columns. Columns are the fundamental building blocks
/// of ViewDefinition output structure.
///
/// # Key Properties
///
/// - **Name**: The output column name in the result table
/// - **Path**: The FHIRPath expression to extract the value
/// - **Collection**: Whether this column contains array/collection values
///
/// # Examples
///
/// ```rust
/// use sof::traits::ViewDefinitionColumnTrait;
///
/// fn describe_column<T: ViewDefinitionColumnTrait>(col: &T) {
///     if let Some(name) = col.name() {
///         print!("Column '{}'", name);
///         
///         if let Some(path) = col.path() {
///             print!(" from path '{}'", path);
///         }
///         
///         if col.collection() == Some(true) {
///             print!(" (collection)");
///         }
///         
///         println!();
///     }
/// }
/// ```
pub trait ViewDefinitionColumnTrait {
    /// Returns the name of this column in the output table
    fn name(&self) -> Option<&str>;
    /// Returns the FHIRPath expression to extract the column value
    fn path(&self) -> Option<&str>;
    /// Returns whether this column should contain collection/array values
    fn collection(&self) -> Option<bool>;
}

/// Trait for abstracting ViewDefinitionWhere across FHIR versions.
///
/// This trait provides version-agnostic access to where clause definitions,
/// which filter resources before processing. Where clauses use FHIRPath
/// expressions that must evaluate to boolean or boolean-coercible values.
///
/// # Filtering Logic
///
/// Resources are included in processing only if ALL where clauses evaluate to:
/// - `true` (boolean)
/// - Non-empty collections
/// - Any other "truthy" value
///
/// Resources are excluded if ANY where clause evaluates to:
/// - `false` (boolean)
/// - Empty collections
/// - Empty/null results
///
/// # Examples
///
/// ```rust
/// use sof::traits::ViewDefinitionWhereTrait;
///
/// fn check_where_clause<T: ViewDefinitionWhereTrait>(where_clause: &T) {
///     if let Some(path) = where_clause.path() {
///         println!("Filter condition: {}", path);
///         
///         // Example paths:
///         // "active = true"                  // Boolean condition
///         // "name.exists()"                 // Existence check  
///         // "birthDate >= @1990-01-01"      // Date comparison
///         // "telecom.where(system='email')" // Collection filtering
///     }
/// }
/// ```
pub trait ViewDefinitionWhereTrait {
    /// Returns the FHIRPath expression that must evaluate to true for resource inclusion
    fn path(&self) -> Option<&str>;
}

/// Trait for abstracting ViewDefinitionConstant across FHIR versions.
///
/// This trait provides version-agnostic access to constant definitions,
/// which define reusable values that can be referenced in FHIRPath expressions
/// throughout the ViewDefinition. Constants improve maintainability and
/// readability of complex transformations.
///
/// # Constant Usage
///
/// Constants are referenced in FHIRPath expressions using the `%` prefix:
/// ```fhirpath
/// // Define constant: name="baseUrl", valueString="http://example.org"
/// // Use in path: "identifier.where(system = %baseUrl)"
/// ```
///
/// # Supported Types
///
/// Constants can hold various FHIR primitive types:
/// - String values
/// - Boolean values  
/// - Integer and decimal numbers
/// - Date, dateTime, and time values
/// - Coded values and URIs
///
/// # Examples
///
/// ```rust
/// use sof::traits::ViewDefinitionConstantTrait;
/// use fhirpath::EvaluationResult;
///
/// fn process_constant<T: ViewDefinitionConstantTrait>(constant: &T) -> Result<(), Box<dyn std::error::Error>> {
///     if let Some(name) = constant.name() {
///         let eval_result = constant.to_evaluation_result()?;
///         
///         match eval_result {
///             EvaluationResult::String(s, _) => {
///                 println!("String constant '{}' = '{}'", name, s);
///             },
///             EvaluationResult::Integer(i, _) => {
///                 println!("Integer constant '{}' = {}", name, i);
///             },
///             EvaluationResult::Boolean(b, _) => {
///                 println!("Boolean constant '{}' = {}", name, b);
///             },
///             _ => {
///                 println!("Other constant '{}'", name);
///             }
///         }
///     }
///     Ok(())
/// }
/// ```
pub trait ViewDefinitionConstantTrait {
    /// Returns the name of this constant for use in FHIRPath expressions (referenced as %name)
    fn name(&self) -> Option<&str>;
    /// Converts this constant to an EvaluationResult for use in FHIRPath evaluation
    fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError>;
}

/// Trait for abstracting Bundle across FHIR versions.
///
/// This trait provides version-agnostic access to Bundle contents,
/// specifically the collection of resources contained within bundle entries.
/// Bundles serve as the input data source for ViewDefinition processing.
///
/// # Bundle Structure
///
/// FHIR Bundles contain:
/// - Bundle metadata (type, id, etc.)
/// - Array of bundle entries
/// - Each entry optionally contains a resource
///
/// This trait focuses on extracting the resources for processing,
/// filtering out entries that don't contain resources.
///
/// # Associated Types
///
/// - [`Resource`](Self::Resource): The resource type for this FHIR version
///
/// # Examples
///
/// ```rust
/// use sof::traits::{BundleTrait, ResourceTrait};
///
/// fn analyze_bundle<B: BundleTrait>(bundle: &B)
/// where
///     B::Resource: ResourceTrait
/// {
///     let resources = bundle.entries();
///     println!("Bundle contains {} resources", resources.len());
///     
///     for resource in resources {
///         println!("- {} resource", resource.resource_name());
///     }
/// }
/// ```
pub trait BundleTrait {
    /// The resource type for this FHIR version
    type Resource: ResourceTrait;

    /// Returns references to all resources contained in this bundle's entries
    fn entries(&self) -> Vec<&Self::Resource>;
}

/// Trait for abstracting Resource across FHIR versions.
///
/// This trait provides version-agnostic access to FHIR resource functionality,
/// enabling the core processing logic to work with resources from any supported
/// FHIR version. Resources are the primary data objects processed by ViewDefinitions.
///
/// # Key Functionality
///
/// - **Type Identification**: Determine the resource type (Patient, Observation, etc.)
/// - **Version Wrapping**: Convert to version-agnostic containers for FHIRPath evaluation
///
/// # Examples
///
/// ```rust
/// use sof::traits::ResourceTrait;
/// use fhir::FhirResource;
///
/// fn process_resource<R: ResourceTrait>(resource: &R) {
///     println!("Processing {} resource", resource.resource_name());
///     
///     // Convert to FhirResource for FHIRPath evaluation
///     let fhir_resource = resource.to_fhir_resource();
///     
///     // Now can be used with FHIRPath evaluation context
///     // let context = EvaluationContext::new(vec![fhir_resource]);
/// }
/// ```
pub trait ResourceTrait: Clone {
    /// Returns the FHIR resource type name (e.g., "Patient", "Observation")
    fn resource_name(&self) -> &str;
    /// Converts this resource to a version-agnostic FhirResource for FHIRPath evaluation
    fn to_fhir_resource(&self) -> FhirResource;
    /// Returns the lastUpdated timestamp from the resource's metadata if available
    fn get_last_updated(&self) -> Option<chrono::DateTime<chrono::Utc>>;
}

// ===== FHIR Version Implementations =====
//
// The following modules provide concrete implementations of the abstraction
// traits for each supported FHIR version. Each implementation maps the
// version-specific FHIR structures to the common trait interface.

/// R4 (FHIR 4.0.1) trait implementations.
///
/// This module implements all abstraction traits for FHIR R4 resources,
/// providing the mapping between R4-specific ViewDefinition structures
/// and the version-agnostic trait interfaces.
#[cfg(feature = "R4")]
mod r4_impl {
    use super::*;
    use fhir::r4::*;

    impl ViewDefinitionTrait for ViewDefinition {
        type Select = ViewDefinitionSelect;
        type Where = ViewDefinitionWhere;
        type Constant = ViewDefinitionConstant;

        fn resource(&self) -> Option<&str> {
            self.resource.value.as_deref()
        }

        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }

        fn where_clauses(&self) -> Option<&[Self::Where]> {
            self.r#where.as_deref()
        }

        fn constants(&self) -> Option<&[Self::Constant]> {
            self.constant.as_deref()
        }
    }

    impl ViewDefinitionSelectTrait for ViewDefinitionSelect {
        type Column = ViewDefinitionSelectColumn;
        type Select = ViewDefinitionSelect;

        fn column(&self) -> Option<&[Self::Column]> {
            self.column.as_deref()
        }

        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }

        fn for_each(&self) -> Option<&str> {
            self.for_each.as_ref()?.value.as_deref()
        }

        fn for_each_or_null(&self) -> Option<&str> {
            self.for_each_or_null.as_ref()?.value.as_deref()
        }

        fn union_all(&self) -> Option<&[Self::Select]> {
            self.union_all.as_deref()
        }
    }

    impl ViewDefinitionColumnTrait for ViewDefinitionSelectColumn {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }

        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }

        fn collection(&self) -> Option<bool> {
            self.collection.as_ref()?.value
        }
    }

    impl ViewDefinitionWhereTrait for ViewDefinitionWhere {
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
    }

    impl ViewDefinitionConstantTrait for ViewDefinitionConstant {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }

        fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
            let name = self.name().unwrap_or("unknown");

            if let Some(value) = &self.value {
                let eval_result = match value {
                    ViewDefinitionConstantValue::String(s) => {
                        EvaluationResult::String(s.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Boolean(b) => {
                        EvaluationResult::Boolean(b.value.unwrap_or(false), None)
                    }
                    ViewDefinitionConstantValue::Integer(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Decimal(d) => {
                        if let Some(precise_decimal) = &d.value {
                            match precise_decimal.original_string().parse() {
                                Ok(decimal_value) => EvaluationResult::Decimal(decimal_value, None),
                                Err(_) => {
                                    return Err(SofError::InvalidViewDefinition(format!(
                                        "Invalid decimal value for constant '{}'",
                                        name
                                    )));
                                }
                            }
                        } else {
                            EvaluationResult::Decimal("0".parse().unwrap(), None)
                        }
                    }
                    ViewDefinitionConstantValue::Date(d) => {
                        EvaluationResult::Date(d.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::DateTime(dt) => {
                        EvaluationResult::DateTime(dt.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Time(t) => {
                        EvaluationResult::Time(t.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Code(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Base64Binary(b) => {
                        EvaluationResult::String(b.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Id(i) => {
                        EvaluationResult::String(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Instant(i) => {
                        EvaluationResult::DateTime(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Oid(o) => {
                        EvaluationResult::String(o.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::PositiveInt(p) => {
                        EvaluationResult::Integer(p.value.unwrap_or(1) as i64, None)
                    }
                    ViewDefinitionConstantValue::UnsignedInt(u) => {
                        EvaluationResult::Integer(u.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Uri(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Url(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Uuid(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Canonical(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                };

                Ok(eval_result)
            } else {
                Err(SofError::InvalidViewDefinition(format!(
                    "Constant '{}' must have a value",
                    name
                )))
            }
        }
    }

    impl BundleTrait for Bundle {
        type Resource = Resource;

        fn entries(&self) -> Vec<&Self::Resource> {
            self.entry
                .as_ref()
                .map(|entries| entries.iter().filter_map(|e| e.resource.as_ref()).collect())
                .unwrap_or_default()
        }
    }

    impl ResourceTrait for Resource {
        fn resource_name(&self) -> &str {
            self.resource_name()
        }

        fn to_fhir_resource(&self) -> FhirResource {
            FhirResource::R4(Box::new(self.clone()))
        }
        
        fn get_last_updated(&self) -> Option<chrono::DateTime<chrono::Utc>> {
            self.get_last_updated()
        }
    }
}

/// R4B (FHIR 4.3.0) trait implementations.
///
/// This module implements all abstraction traits for FHIR R4B resources,
/// providing the mapping between R4B-specific ViewDefinition structures
/// and the version-agnostic trait interfaces.
#[cfg(feature = "R4B")]
mod r4b_impl {
    use super::*;
    use fhir::r4b::*;

    impl ViewDefinitionTrait for ViewDefinition {
        type Select = ViewDefinitionSelect;
        type Where = ViewDefinitionWhere;
        type Constant = ViewDefinitionConstant;

        fn resource(&self) -> Option<&str> {
            self.resource.value.as_deref()
        }

        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }

        fn where_clauses(&self) -> Option<&[Self::Where]> {
            self.r#where.as_deref()
        }

        fn constants(&self) -> Option<&[Self::Constant]> {
            self.constant.as_deref()
        }
    }

    impl ViewDefinitionSelectTrait for ViewDefinitionSelect {
        type Column = ViewDefinitionSelectColumn;
        type Select = ViewDefinitionSelect;

        fn column(&self) -> Option<&[Self::Column]> {
            self.column.as_deref()
        }

        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }

        fn for_each(&self) -> Option<&str> {
            self.for_each.as_ref()?.value.as_deref()
        }

        fn for_each_or_null(&self) -> Option<&str> {
            self.for_each_or_null.as_ref()?.value.as_deref()
        }

        fn union_all(&self) -> Option<&[Self::Select]> {
            self.union_all.as_deref()
        }
    }

    impl ViewDefinitionColumnTrait for ViewDefinitionSelectColumn {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }

        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }

        fn collection(&self) -> Option<bool> {
            self.collection.as_ref()?.value
        }
    }

    impl ViewDefinitionWhereTrait for ViewDefinitionWhere {
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
    }

    impl ViewDefinitionConstantTrait for ViewDefinitionConstant {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }

        fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
            let name = self.name().unwrap_or("unknown");

            if let Some(value) = &self.value {
                let eval_result = match value {
                    ViewDefinitionConstantValue::String(s) => {
                        EvaluationResult::String(s.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Boolean(b) => {
                        EvaluationResult::Boolean(b.value.unwrap_or(false), None)
                    }
                    ViewDefinitionConstantValue::Integer(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Decimal(d) => {
                        if let Some(precise_decimal) = &d.value {
                            match precise_decimal.original_string().parse() {
                                Ok(decimal_value) => EvaluationResult::Decimal(decimal_value, None),
                                Err(_) => {
                                    return Err(SofError::InvalidViewDefinition(format!(
                                        "Invalid decimal value for constant '{}'",
                                        name
                                    )));
                                }
                            }
                        } else {
                            EvaluationResult::Decimal("0".parse().unwrap(), None)
                        }
                    }
                    ViewDefinitionConstantValue::Date(d) => {
                        EvaluationResult::Date(d.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::DateTime(dt) => {
                        EvaluationResult::DateTime(dt.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Time(t) => {
                        EvaluationResult::Time(t.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Code(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Base64Binary(b) => {
                        EvaluationResult::String(b.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Id(i) => {
                        EvaluationResult::String(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Instant(i) => {
                        EvaluationResult::DateTime(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Oid(o) => {
                        EvaluationResult::String(o.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::PositiveInt(p) => {
                        EvaluationResult::Integer(p.value.unwrap_or(1) as i64, None)
                    }
                    ViewDefinitionConstantValue::UnsignedInt(u) => {
                        EvaluationResult::Integer(u.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Uri(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Url(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Uuid(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Canonical(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                };

                Ok(eval_result)
            } else {
                Err(SofError::InvalidViewDefinition(format!(
                    "Constant '{}' must have a value",
                    name
                )))
            }
        }
    }

    impl BundleTrait for Bundle {
        type Resource = Resource;

        fn entries(&self) -> Vec<&Self::Resource> {
            self.entry
                .as_ref()
                .map(|entries| entries.iter().filter_map(|e| e.resource.as_ref()).collect())
                .unwrap_or_default()
        }
    }

    impl ResourceTrait for Resource {
        fn resource_name(&self) -> &str {
            self.resource_name()
        }

        fn to_fhir_resource(&self) -> FhirResource {
            FhirResource::R4B(Box::new(self.clone()))
        }
        
        fn get_last_updated(&self) -> Option<chrono::DateTime<chrono::Utc>> {
            self.get_last_updated()
        }
    }
}

/// R5 (FHIR 5.0.0) trait implementations.
///
/// This module implements all abstraction traits for FHIR R5 resources,
/// providing the mapping between R5-specific ViewDefinition structures
/// and the version-agnostic trait interfaces. R5 introduces the Integer64
/// data type for constant values.
#[cfg(feature = "R5")]
mod r5_impl {
    use super::*;
    use fhir::r5::*;

    impl ViewDefinitionTrait for ViewDefinition {
        type Select = ViewDefinitionSelect;
        type Where = ViewDefinitionWhere;
        type Constant = ViewDefinitionConstant;

        fn resource(&self) -> Option<&str> {
            self.resource.value.as_deref()
        }

        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }

        fn where_clauses(&self) -> Option<&[Self::Where]> {
            self.r#where.as_deref()
        }

        fn constants(&self) -> Option<&[Self::Constant]> {
            self.constant.as_deref()
        }
    }

    impl ViewDefinitionSelectTrait for ViewDefinitionSelect {
        type Column = ViewDefinitionSelectColumn;
        type Select = ViewDefinitionSelect;

        fn column(&self) -> Option<&[Self::Column]> {
            self.column.as_deref()
        }

        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }

        fn for_each(&self) -> Option<&str> {
            self.for_each.as_ref()?.value.as_deref()
        }

        fn for_each_or_null(&self) -> Option<&str> {
            self.for_each_or_null.as_ref()?.value.as_deref()
        }

        fn union_all(&self) -> Option<&[Self::Select]> {
            self.union_all.as_deref()
        }
    }

    impl ViewDefinitionColumnTrait for ViewDefinitionSelectColumn {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }

        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }

        fn collection(&self) -> Option<bool> {
            self.collection.as_ref()?.value
        }
    }

    impl ViewDefinitionWhereTrait for ViewDefinitionWhere {
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
    }

    impl ViewDefinitionConstantTrait for ViewDefinitionConstant {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }

        fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
            let name = self.name().unwrap_or("unknown");

            if let Some(value) = &self.value {
                // R5 implementation identical to R4
                let eval_result = match value {
                    ViewDefinitionConstantValue::String(s) => {
                        EvaluationResult::String(s.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Boolean(b) => {
                        EvaluationResult::Boolean(b.value.unwrap_or(false), None)
                    }
                    ViewDefinitionConstantValue::Integer(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Decimal(d) => {
                        if let Some(precise_decimal) = &d.value {
                            match precise_decimal.original_string().parse() {
                                Ok(decimal_value) => EvaluationResult::Decimal(decimal_value, None),
                                Err(_) => {
                                    return Err(SofError::InvalidViewDefinition(format!(
                                        "Invalid decimal value for constant '{}'",
                                        name
                                    )));
                                }
                            }
                        } else {
                            EvaluationResult::Decimal("0".parse().unwrap(), None)
                        }
                    }
                    ViewDefinitionConstantValue::Date(d) => {
                        EvaluationResult::Date(d.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::DateTime(dt) => {
                        EvaluationResult::DateTime(dt.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Time(t) => {
                        EvaluationResult::Time(t.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Code(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Base64Binary(b) => {
                        EvaluationResult::String(b.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Id(i) => {
                        EvaluationResult::String(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Instant(i) => {
                        EvaluationResult::DateTime(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Oid(o) => {
                        EvaluationResult::String(o.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::PositiveInt(p) => {
                        EvaluationResult::Integer(p.value.unwrap_or(1) as i64, None)
                    }
                    ViewDefinitionConstantValue::UnsignedInt(u) => {
                        EvaluationResult::Integer(u.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Uri(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Url(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Uuid(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Canonical(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Integer64(i) => {
                        EvaluationResult::Integer64(i.value.unwrap_or(0), None)
                    }
                };

                Ok(eval_result)
            } else {
                Err(SofError::InvalidViewDefinition(format!(
                    "Constant '{}' must have a value",
                    name
                )))
            }
        }
    }

    impl BundleTrait for Bundle {
        type Resource = Resource;

        fn entries(&self) -> Vec<&Self::Resource> {
            self.entry
                .as_ref()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e.resource.as_deref()) // Note: R5 uses Box<Resource>
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    impl ResourceTrait for Resource {
        fn resource_name(&self) -> &str {
            self.resource_name()
        }

        fn to_fhir_resource(&self) -> FhirResource {
            FhirResource::R5(Box::new(self.clone()))
        }
        
        fn get_last_updated(&self) -> Option<chrono::DateTime<chrono::Utc>> {
            self.get_last_updated()
        }
    }
}

/// R6 (FHIR 6.0.0) trait implementations.
///
/// This module implements all abstraction traits for FHIR R6 resources,
/// providing the mapping between R6-specific ViewDefinition structures
/// and the version-agnostic trait interfaces. R6 continues to support
/// the Integer64 data type introduced in R5.
#[cfg(feature = "R6")]
mod r6_impl {
    use super::*;
    use fhir::r6::*;

    impl ViewDefinitionTrait for ViewDefinition {
        type Select = ViewDefinitionSelect;
        type Where = ViewDefinitionWhere;
        type Constant = ViewDefinitionConstant;

        fn resource(&self) -> Option<&str> {
            self.resource.value.as_deref()
        }

        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }

        fn where_clauses(&self) -> Option<&[Self::Where]> {
            self.r#where.as_deref()
        }

        fn constants(&self) -> Option<&[Self::Constant]> {
            self.constant.as_deref()
        }
    }

    impl ViewDefinitionSelectTrait for ViewDefinitionSelect {
        type Column = ViewDefinitionSelectColumn;
        type Select = ViewDefinitionSelect;

        fn column(&self) -> Option<&[Self::Column]> {
            self.column.as_deref()
        }

        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }

        fn for_each(&self) -> Option<&str> {
            self.for_each.as_ref()?.value.as_deref()
        }

        fn for_each_or_null(&self) -> Option<&str> {
            self.for_each_or_null.as_ref()?.value.as_deref()
        }

        fn union_all(&self) -> Option<&[Self::Select]> {
            self.union_all.as_deref()
        }
    }

    impl ViewDefinitionColumnTrait for ViewDefinitionSelectColumn {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }

        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }

        fn collection(&self) -> Option<bool> {
            self.collection.as_ref()?.value
        }
    }

    impl ViewDefinitionWhereTrait for ViewDefinitionWhere {
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
    }

    impl ViewDefinitionConstantTrait for ViewDefinitionConstant {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }

        fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
            let name = self.name().unwrap_or("unknown");

            if let Some(value) = &self.value {
                // R5 implementation identical to R4
                let eval_result = match value {
                    ViewDefinitionConstantValue::String(s) => {
                        EvaluationResult::String(s.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Boolean(b) => {
                        EvaluationResult::Boolean(b.value.unwrap_or(false), None)
                    }
                    ViewDefinitionConstantValue::Integer(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Decimal(d) => {
                        if let Some(precise_decimal) = &d.value {
                            match precise_decimal.original_string().parse() {
                                Ok(decimal_value) => EvaluationResult::Decimal(decimal_value, None),
                                Err(_) => {
                                    return Err(SofError::InvalidViewDefinition(format!(
                                        "Invalid decimal value for constant '{}'",
                                        name
                                    )));
                                }
                            }
                        } else {
                            EvaluationResult::Decimal("0".parse().unwrap(), None)
                        }
                    }
                    ViewDefinitionConstantValue::Date(d) => {
                        EvaluationResult::Date(d.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::DateTime(dt) => {
                        EvaluationResult::DateTime(dt.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Time(t) => {
                        EvaluationResult::Time(t.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Code(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Base64Binary(b) => {
                        EvaluationResult::String(b.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Id(i) => {
                        EvaluationResult::String(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Instant(i) => {
                        EvaluationResult::DateTime(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Oid(o) => {
                        EvaluationResult::String(o.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::PositiveInt(p) => {
                        EvaluationResult::Integer(p.value.unwrap_or(1) as i64, None)
                    }
                    ViewDefinitionConstantValue::UnsignedInt(u) => {
                        EvaluationResult::Integer(u.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Uri(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Url(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Uuid(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Canonical(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Integer64(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0), None)
                    }
                };

                Ok(eval_result)
            } else {
                Err(SofError::InvalidViewDefinition(format!(
                    "Constant '{}' must have a value",
                    name
                )))
            }
        }
    }

    impl BundleTrait for Bundle {
        type Resource = Resource;

        fn entries(&self) -> Vec<&Self::Resource> {
            self.entry
                .as_ref()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e.resource.as_deref()) // Note: R6 uses Box<Resource>
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    impl ResourceTrait for Resource {
        fn resource_name(&self) -> &str {
            self.resource_name()
        }

        fn to_fhir_resource(&self) -> FhirResource {
            FhirResource::R6(Box::new(self.clone()))
        }
        
        fn get_last_updated(&self) -> Option<chrono::DateTime<chrono::Utc>> {
            self.get_last_updated()
        }
    }
}
