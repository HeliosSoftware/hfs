# FHIRPath Implementation Plan

This document provides a detailed implementation plan for fixing the identified issues in the FHIRPath implementation.

## 1. Type System and Type Operations

### 1.1. Unified Type System Architecture

**Goal**: Create a consistent type system that properly handles FHIR types, System types, and namespaces.

**Implementation Steps**:

1. Enhance `fhir_type_hierarchy.rs`:
   - Complete the type hierarchy maps for all FHIR resources
   - Add proper namespace qualification support
   - Implement better case-insensitive type name handling
   - Add comprehensive complex type hierarchy support

2. Create a `TypeInfo` structure:
   ```rust
   pub struct TypeInfo {
       namespace: String,
       name: String,
       is_primitive: bool,
       parent_type: Option<String>,
   }
   ```

3. Implement robust type resolution mechanism:
   - Handle both qualified and unqualified type names
   - Properly handle backticks in type names (`FHIR.`Patient``)
   - Support case-insensitive type matching while preserving canonical forms

### 1.2. Fix `is()` Function Implementation

**Goal**: Make the `is()` function work correctly for all types.

**Implementation Steps**:

1. Fix type specifier extraction:
   ```rust
   fn extract_type_specifier(expr: &Expression) -> Result<TypeSpecifier, EvaluationError> {
       match expr {
           Expression::Term(Term::Invocation(Invocation::Member(type_name))) => {
               Ok(TypeSpecifier::Identifier(type_name.clone()))
           },
           Expression::Invocation(base, Invocation::Member(member)) => {
               // Handle namespace qualification
               if let Expression::Term(Term::Invocation(Invocation::Member(namespace))) = &**base {
                   Ok(TypeSpecifier::QualifiedIdentifier(member.clone(), Some(namespace.clone())))
               } else {
                   Err(EvaluationError::InvalidTypeSpecifier(format!("Invalid type specifier: {}.{}", base, member)))
               }
           },
           // Add handling for backtick names
           _ => Err(EvaluationError::InvalidTypeSpecifier("Expected identifier or qualified identifier".to_string()))
       }
   }
   ```

2. Update `apply_type_operation.rs` to unify type checking logic:
   - Remove duplicate type checking code
   - Use the enhanced type hierarchy for all type checks
   - Handle all edge cases in one central location

3. Fix error messages to be more descriptive:
   - Replace "is() function requires a string type name, got Empty" with proper guidance

### 1.3. Fix `as()` Function Implementation 

**Goal**: Make the `as()` function work correctly for type casting.

**Implementation Steps**:

1. Use the same type resolution logic as `is()` to ensure consistency
2. Properly handle resource types vs. primitive types
3. Implement proper casting for complex types
4. Handle polymorphic fields correctly in type casting

### 1.4. Type Reflection Enhancements

**Goal**: Fix the `type()` function to correctly return type information.

**Implementation Steps**:

1. Enhance type reflection to work with all FHIR types:
   ```rust
   fn get_type_info(result: &EvaluationResult) -> Result<(String, String), EvaluationError> {
       match result {
           EvaluationResult::Boolean(_) => Ok((SYSTEM_NAMESPACE.to_string(), "Boolean".to_string())),
           // Add handling for FHIR primitives
           EvaluationResult::Object(obj) => {
               // Extract resource type from the object
               if let Some(EvaluationResult::String(resource_type)) = obj.get("resourceType") {
                   Ok((FHIR_NAMESPACE.to_string(), resource_type.clone()))
               } else {
                   // Handle FHIR complex types
                   Ok((FHIR_NAMESPACE.to_string(), "Element".to_string()))
               }
           },
           // Handle all other types
       }
   }
   ```

2. Add explicit support for resolving FHIR primitive types to the FHIR namespace
3. Correctly handle BackboneElement and Element types in the hierarchy

## 2. Polymorphic Access / Choice Elements

### 2.1. Enhanced Polymorphic Field Detection

**Goal**: Improve detection and handling of choice elements.

**Implementation Steps**:

1. Create a more robust mechanism for detecting choice elements:
   ```rust
   fn is_choice_element(obj: &HashMap<String, EvaluationResult>, field_name: &str) -> bool {
       // Look for patterns like valueString, valueQuantity, etc.
       for key in obj.keys() {
           if key.starts_with(field_name) && 
              key.len() > field_name.len() && 
              key.chars().nth(field_name.len()).unwrap().is_uppercase() {
               return true;
           }
       }
       // Check other known choice elements (field[x])
       CHOICE_ELEMENTS.contains(&field_name)
   }
   ```

2. Stop using hard-coded lists for choice elements when possible

### 2.2. Fix Polymorphic Property Access

**Goal**: Make polymorphic property access work correctly.

**Implementation Steps**:

1. Enhance polymorphic property resolution in `access_polymorphic_element`:
   ```rust
   fn access_polymorphic_element(obj: &HashMap<String, EvaluationResult>, field_name: &str) -> EvaluationResult {
       // First try exact match
       if let Some(value) = obj.get(field_name) {
           return value.clone();
       }
       
       // Look for choice element patterns
       for (key, value) in obj.iter() {
           if key.starts_with(field_name) && 
              key.len() > field_name.len() && 
              key.chars().nth(field_name.len()).unwrap().is_uppercase() {
               return value.clone();
           }
       }
       
       // Not found
       EvaluationResult::Empty
   }
   ```

2. Fix nested property access for choice elements:
   ```rust
   fn resolve_polymorphic_property(obj: &HashMap<String, EvaluationResult>, field_path: &[&str]) -> EvaluationResult {
       if field_path.is_empty() {
           return EvaluationResult::Empty;
       }
       
       let field_name = field_path[0];
       let value = access_polymorphic_element(obj, field_name);
       
       if field_path.len() == 1 {
           return value;
       }
       
       // Handle nested property access
       match value {
           EvaluationResult::Object(nested_obj) => {
               resolve_polymorphic_property(&nested_obj, &field_path[1..])
           },
           _ => EvaluationResult::Empty,
       }
   }
   ```

### 2.3. Integrate Type Checking with Polymorphic Fields

**Goal**: Fix type checking for polymorphic fields.

**Implementation Steps**:

1. Update `apply_polymorphic_type_operation` to determine the actual type of a choice element:
   ```rust
   fn get_polymorphic_field_type(obj: &HashMap<String, EvaluationResult>, field_name: &str) -> Option<String> {
       for key in obj.keys() {
           if key.starts_with(field_name) && key.len() > field_name.len() {
               let type_suffix = &key[field_name.len()..];
               return Some(type_suffix.to_string());
           }
       }
       None
   }
   ```

2. Connect this function to the type checking mechanism:
   ```rust
   fn is_polymorphic_field_of_type(obj: &HashMap<String, EvaluationResult>, field_name: &str, type_name: &str) -> bool {
       if let Some(actual_type) = get_polymorphic_field_type(obj, field_name) {
           // Normalize and compare types
           return actual_type.eq_ignore_ascii_case(type_name);
       }
       false
   }
   ```

## 3. Date/Time Operations

### 3.1. Improved Date/Time Types

**Goal**: Fix date/time type checking and representation.

**Implementation Steps**:

1. Enhance date/time storage to preserve precision and timezone information:
   ```rust
   pub struct DateTimeInfo {
       value: String,
       precision: DateTimePrecision,
       timezone: Option<String>,
   }
   
   enum DateTimePrecision {
       Year,
       Month,
       Day,
       Hour,
       Minute,
       Second,
       Millisecond,
   }
   ```

2. Fix date literal parsing to extract and preserve precision:
   ```rust
   fn parse_date_literal(literal: &str) -> Result<DateTimeInfo, EvaluationError> {
       // Extract precision and timezone information
       // Handle @2015, @2015-02, @2015-02-04 formats
       // Handle @2015-02-04T14:30:00.123+01:00 formats
   }
   ```

3. Fix type checking for date/time literals

### 3.2. Date/Time Comparison Fixes

**Goal**: Fix date/time comparison operations.

**Implementation Steps**:

1. Implement proper timezone-aware comparison:
   ```rust
   fn compare_datetimes(dt1: &DateTimeInfo, dt2: &DateTimeInfo) -> Ordering {
       // Normalize timezones before comparison
       let normalized_dt1 = normalize_to_utc(dt1);
       let normalized_dt2 = normalize_to_utc(dt2);
       
       // Compare with precision awareness
       compare_with_precision(&normalized_dt1, &normalized_dt2)
   }
   ```

2. Fix millisecond precision comparisons:
   ```rust
   fn compare_with_precision(dt1: &DateTimeInfo, dt2: &DateTimeInfo) -> Ordering {
       // Compare up to the minimum precision of both datetimes
       // Treat 2018-03-01T10:30:00 as equal to 2018-03-01T10:30:00.0
   }
   ```

3. Implement cross-type comparisons (Date vs DateTime):
   ```rust
   fn compare_date_to_datetime(date: &DateTimeInfo, datetime: &DateTimeInfo) -> Ordering {
       // Convert date to datetime with 00:00:00.000 time component
       // Then compare as datetimes
   }
   ```

### 3.3. Date/Time Arithmetic Enhancements

**Goal**: Fix date/time arithmetic operations.

**Implementation Steps**:

1. Implement proper date addition with different units:
   ```rust
   fn add_date_time_quantity(date: &DateTimeInfo, quantity: &Quantity) -> Result<DateTimeInfo, EvaluationError> {
       // Handle different units (year, month, day, hour, minute, second, millisecond)
       // Properly handle timezone when adding time components
       // Handle edge cases like month boundaries
   }
   ```

2. Fix date/time difference calculations:
   ```rust
   fn date_time_difference(dt1: &DateTimeInfo, dt2: &DateTimeInfo, unit: &str) -> Result<Quantity, EvaluationError> {
       // Calculate difference in the specified unit
       // Handle different precisions correctly
       // Properly account for timezone differences
   }
   ```

## 4. Collection Functions and Navigation

### 4.1. Fix Collection Navigation

**Goal**: Fix collection navigation and path traversal.

**Implementation Steps**:

1. Enhance the recursive flattening logic:
   ```rust
   fn flatten_collections_recursive(result: EvaluationResult) -> Vec<EvaluationResult> {
       // Current implementation is mostly correct but needs enhancement for nested path traversal
   }
   ```

2. Fix chained navigation after collection operations:
   ```rust
   fn evaluate_invocation(invocation_base: &EvaluationResult, invocation: &Invocation, context: &EvaluationContext) -> Result<EvaluationResult, EvaluationError> {
       // Update collection handling to preserve context through operations
       // Fix context propagation in skip(), take(), first(), last(), etc.
   }
   ```

### 4.2. Fix Collection Functions

**Goal**: Fix where(), select(), and other collection functions.

**Implementation Steps**:

1. Fix the `where()` function to handle nested collections properly:
   ```rust
   fn evaluate_where(collection: &EvaluationResult, criteria_expr: &Expression, context: &EvaluationContext) -> Result<EvaluationResult, EvaluationError> {
       // Enhance to better handle nested collections
       // Fix $this context propagation
   }
   ```

2. Fix the `select()` function similarly:
   ```rust
   fn evaluate_select(collection: &EvaluationResult, projection_expr: &Expression, context: &EvaluationContext) -> Result<EvaluationResult, EvaluationError> {
       // Enhance to better handle nested collections
       // Fix context propagation
   }
   ```

3. Fix children() and descendants() functions:
   ```rust
   fn evaluate_children(obj: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
       // Implement proper children result counting
   }
   
   fn evaluate_descendants(obj: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
       // Fix descendants recursion and counting
   }
   ```

### 4.3. Fix Variable Context

**Goal**: Fix issues with variable resolution, especially $this.

**Implementation Steps**:

1. Fix `$this` handling in expression evaluation:
   ```rust
   fn evaluate(expr: &Expression, context: &EvaluationContext, current_item: Option<&EvaluationResult>) -> Result<EvaluationResult, EvaluationError> {
       // Ensure proper $this propagation through all expression types
   }
   ```

2. Enhance variable resolution in collection functions:
   ```rust
   fn evaluate_with_variable_context(expr: &Expression, context: &EvaluationContext, item: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
       // Create a context with the item as $this
       let mut new_context = context.clone();
       new_context.set_variable_result("this", item.clone());
       
       // Evaluate expression in this context
       evaluate(expr, &new_context, Some(item))
   }
   ```

## 5. Quantity Handling and Unit Conversion

### 5.1. Unit Conversion System

**Goal**: Implement proper unit conversion for quantities.

**Implementation Steps**:

1. Create a unit conversion registry:
   ```rust
   struct UnitConversionRegistry {
       // Map of unit categories (mass, length, time, etc.)
       categories: HashMap<String, UnitCategory>,
   }
   
   struct UnitCategory {
       // Base unit for this category
       base_unit: String,
       // Conversion factors to the base unit
       conversion_factors: HashMap<String, Decimal>,
   }
   ```

2. Add standard unit conversions for UCUM units:
   ```rust
   fn initialize_unit_conversions() -> UnitConversionRegistry {
       let mut registry = UnitConversionRegistry::new();
       
       // Mass units
       registry.add_category("mass", "g");
       registry.add_conversion("mass", "kg", Decimal::from(1000));
       registry.add_conversion("mass", "mg", Decimal::from_f64(0.001).unwrap());
       
       // Time units
       registry.add_category("time", "s");
       registry.add_conversion("time", "min", Decimal::from(60));
       registry.add_conversion("time", "h", Decimal::from(3600));
       registry.add_conversion("time", "d", Decimal::from(86400));
       registry.add_conversion("time", "wk", Decimal::from(604800));
       
       // Add more categories and units
       
       registry
   }
   ```

3. Implement conversion functions:
   ```rust
   fn convert_quantity(quantity: &Quantity, target_unit: &str) -> Result<Quantity, EvaluationError> {
       // Look up units in the registry
       // Perform conversion if they're in the same category
       // Return error for incompatible units
   }
   ```

### 5.2. Quantity Comparison Enhancements

**Goal**: Fix quantity comparison operations.

**Implementation Steps**:

1. Update quantity equality check to handle unit conversion:
   ```rust
   fn compare_quantities(q1: &Quantity, q2: &Quantity) -> Result<Ordering, EvaluationError> {
       // Check if units are compatible
       if let Ok(converted_q2) = convert_quantity(q2, &q1.unit) {
           // Compare values after conversion
           q1.value.cmp(&converted_q2.value)
       } else {
           // Units are incompatible
           Err(EvaluationError::IncompatibleUnits(format!(
               "Cannot compare quantities with incompatible units: '{}' and '{}'",
               q1.unit, q2.unit
           )))
       }
   }
   ```

2. Fix quantity equivalent operator (~) for approximate comparisons:
   ```rust
   fn are_quantities_equivalent(q1: &Quantity, q2: &Quantity) -> Result<bool, EvaluationError> {
       // Allow for small differences (e.g., 4g ~ 4040mg with 1% tolerance)
       if let Ok(converted_q2) = convert_quantity(q2, &q1.unit) {
           let difference = (q1.value - converted_q2.value).abs();
           let tolerance = q1.value * Decimal::from_f64(0.01).unwrap(); // 1% tolerance
           Ok(difference <= tolerance)
       } else {
           // Units are incompatible
           Err(EvaluationError::IncompatibleUnits(format!(
               "Cannot compare quantities with incompatible units: '{}' and '{}'",
               q1.unit, q2.unit
           )))
       }
   }
   ```

### 5.3. Quantity Arithmetic

**Goal**: Fix quantity arithmetic operations.

**Implementation Steps**:

1. Implement proper quantity arithmetic:
   ```rust
   fn multiply_quantities(q1: &Quantity, q2: &Quantity) -> Result<Quantity, EvaluationError> {
       // Multiply values
       let result_value = q1.value * q2.value;
       
       // Handle unit multiplication (e.g., cm * m -> cm*m)
       let result_unit = format!("{}*{}", q1.unit, q2.unit);
       
       // Return combined result
       Ok(Quantity {
           value: result_value,
           unit: result_unit,
       })
   }
   
   fn divide_quantities(q1: &Quantity, q2: &Quantity) -> Result<Quantity, EvaluationError> {
       // Similar to multiply but with division and unit division
   }
   ```

2. Fix quantity toString() formatting:
   ```rust
   fn quantity_to_string(q: &Quantity) -> String {
       // Format according to FHIRPath rules
       format!("{} '{}'", q.value, q.unit)
   }
   ```

## 6. Extension Support

### 6.1. Fix Extension Resolution

**Goal**: Fix extension access for primitive types.

**Implementation Steps**:

1. Enhance extension resolution for primitive types:
   ```rust
   fn resolve_extension(value: &EvaluationResult, url: &str) -> Result<EvaluationResult, EvaluationError> {
       match value {
           EvaluationResult::Object(obj) => {
               // Handle object extensions
               if let Some(EvaluationResult::Collection(extensions)) = obj.get("extension") {
                   // Look for extension with matching url
                   // ...
               }
               Ok(EvaluationResult::Empty)
           },
           // Handle primitive types
           EvaluationResult::String(_) |
           EvaluationResult::Boolean(_) |
           EvaluationResult::Integer(_) |
           EvaluationResult::Decimal(_) |
           EvaluationResult::Date(_) |
           EvaluationResult::DateTime(_) |
           EvaluationResult::Time(_) => {
               // Need to check if this primitive comes from a FHIR Element
               // and look up its _<name> extension container
               // For now, return Empty as a placeholder
               Ok(EvaluationResult::Empty)
           },
           _ => Ok(EvaluationResult::Empty),
       }
   }
   ```

2. Fix environment variable resolution with extensions:
   ```rust
   fn evaluate_extension_with_variable(value: &EvaluationResult, context: &EvaluationContext, var_name: &str) -> Result<EvaluationResult, EvaluationError> {
       // Get the variable value
       let url = context.get_variable_as_string(var_name)
           .ok_or_else(|| EvaluationError::UndefinedVariable(var_name.to_string()))?;
       
       // Resolve extension with this URL
       resolve_extension(value, &url)
   }
   ```

## Implementation Approach

For each of these areas:

1. **Start with the critical functionality**:
   - Type system and is/as operations
   - Polymorphic access for choice elements
   - Collection navigation

2. **Test-driven approach**:
   - Run failing tests to identify specific issues
   - Fix the identified issues
   - Verify the fix with tests
   - Update the README.md to reflect the current status

3. **Refactor for sustainability**:
   - Consolidate duplicated code
   - Ensure consistent error handling
   - Add comprehensive documentation
   - Improve code organization

4. **Update tests**:
   - Add specific tests for each fixed issue
   - Ensure tests cover edge cases
   - Mark fixed tests as passing

This implementation plan provides a roadmap for systematically addressing the issues in the current FHIRPath implementation.