# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

### Building
```bash
# Build all crates
cargo build

# Build with release optimizations
cargo build --release

# Build specific crate
cargo build -p fhir
cargo build -p fhirpath

# Build all FHIR versions at once
cargo build --features R4,R4B,R5,R6

# Run the specific binary
cargo run -p hfs
cargo run -p fhir_gen -- --help
```

### Testing
```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p fhirpath

# Run specific test module
cargo test -p fhirpath truncate_tests
cargo test -p fhirpath r4_tests
cargo test -p fhirpath date_operation_tests
cargo test -p fhirpath extension_tests
cargo test -p fhirpath polymorphic_tests
cargo test -p fhirpath polymorphic_r4_tests
cargo test -p fhirpath trace_tests
cargo test -p fhirpath tree_navigation_tests
cargo test -p fhirpath type_operation_tests
cargo test -p fhirpath type_reflection_tests
cargo test -p fhirpath enhanced_variable_tests
cargo test -p fhirpath parser_tests
cargo test -p fhirpath is_as_method_tests

# Run tests with filter
cargo test -p fhirpath -- truncate
cargo test -p fhirpath -- extension

# Run with verbose output to see test results even if passing
cargo test -p fhirpath -- --nocapture

# Run only the failing tests (tests marked with #[ignore])
cargo test -p fhirpath -- --ignored
```

### FHIR Version Selection
The codebase supports multiple FHIR versions through Cargo features:

```bash
# R4 is the default version
cargo build

# For other versions:
cargo build --features R4B
cargo build --features R5
cargo build --features R6

# Multiple versions can be built together
cargo build --features "R4,R5"
```

### Debugging
```bash
# Enable full stack traces when crashes occur
export RUST_BACKTRACE=1

# Enable debug logging (if implemented with env_logger)
export RUST_LOG=debug
cargo run -p fhirpath

# Run with specific test with debug output
RUST_LOG=debug cargo test -p fhirpath date_operation_tests -- --nocapture

# Increase stack size for complex operations (important for deeply nested expressions)
export RUST_MIN_STACK=8388608
```

### Complete Build Process
For a full build and test of all components:

```bash
# Set increased stack size for complex operations
export RUST_MIN_STACK=8388608

# Build all FHIR versions
cargo build --features R4,R4B,R5,R6

# Generate code for all versions
./target/debug/fhir_gen --all

# Run tests across all versions
cargo test --features R4,R4B,R5,R6
```

## Code Architecture

### Project Structure
- **crates/fhir**: Core FHIR model types and operations
  - Supports multiple FHIR versions (R4, R4B, R5, R6) via feature flags
  - Includes JSON serialization/deserialization
  
- **crates/fhir_gen**: Code generation for FHIR resources
  - Contains FHIR definition files for each version
  - Used to generate Rust models

- **crates/fhir_macro**: Procedural macros supporting FHIR operations

- **crates/fhirpath**: Implementation of the FHIRPath specification
  - Evaluates FHIRPath expressions against FHIR resources
  - Parser using chumsky
  - Evaluator for FHIRPath operations
  - Individual modules for specific FHIRPath functions

- **crates/fhirpath_support**: Common support types for FHIRPath

- **crates/hfs**: Main Helios FHIR Server application (WIP)

### FHIRPath Implementation

The project implements HL7's [FHIRPath Specification - 3.0.0-ballot](https://hl7.org/fhirpath/2025Jan/) in Rust.

FHIRPath implementation is divided into:

1. **Parser** (`crates/fhirpath/src/parser.rs`): Parses FHIRPath expressions into AST
   - Uses chumsky parser combinator library
   - Handles expression syntax and precedence rules
   - Supports all expression types including literals, function calls, operators

2. **Evaluator** (`crates/fhirpath/src/evaluator.rs`): Evaluates parsed expressions against FHIR resources
   - Processes the AST from the parser
   - Handles context and variable resolution
   - Dispatches to specialized function modules

3. **Result Types** (`crates/fhirpath_support`): Types for evaluation results
   - Defines the EvaluationResult type and operations
   - Handles conversions between different result types
   - Implements comparisons and operations on results

4. **Function Modules**: Specialized modules for individual FHIRPath functions:
   - `aggregate_function.rs`: Implementation of aggregate() function with accumulator support
   - `date_arithmetic.rs`: Date/time addition, subtraction, and comparative operations
   - `date_components.rs`: Extraction of components from dates/times (year, month, day, etc)
   - `date_operation.rs`: Date/time operations and comparisons
   - `datetime_impl.rs`: Date/time implementation utilities and parsing
   - `extension_function.rs`: Implementation of extension() function for FHIR extensions
   - `extension_helpers.rs`: Helper functions for resolving and working with extensions
   - `fhir_type_hierarchy.rs`: Type system and type hierarchy handling
   - `polymorphic_access.rs`: Support for FHIR choice elements (e.g., value[x])
   - `repeat_function.rs`: Implementation of repeat() function with cycle detection
   - `resource_type.rs`: Resource type reflection for is/as/ofType operators
   - `trace_function.rs`: Implementation of trace() function with projection support
   - `truncate_function.rs`: Implementation of truncate() function for decimal values
   - `truncate_impl.rs`: Implementation details for truncate function
   - `truncate_final.rs`: Final handling of truncate expressions
   - `type_function.rs`: Type checks and operations with namespace qualification support

The FHIRPath README in `crates/fhirpath/README.md` provides a comprehensive list of implemented features and implementation status.

### Implementation Status

The FHIRPath implementation is actively being developed with:
- Core functionality (basic operations, boolean logic, equality) largely complete
- Many advanced functions implemented:
  - Collection functions (where, select, first, last, etc.)
  - String manipulation functions
  - Math functions (truncate, round, sqrt, abs, etc.)
  - Existence functions (empty, exists, all, etc.)
  - Advanced functions (truncate, aggregate, trace, repeat)
  - Basic type conversion functions
  - Type checking with `is` and `as` operators
  - Date/time component extraction (year, month, day, etc.)
  - Extension access functions

- Areas still needing work:
  - Advanced type system with namespace qualification
  - Improved polymorphic property access
  - Resource type checking refinements
  - STU (Standard for Trial Use) functions
  - Additional string functions (STU)
  - Long integer type support (parser and runtime)
  - Date/time arithmetic edge cases
  - Unit conversion for quantities

## Current Development

Active development is focused on:

1. Implementing the remaining FHIRPath functions and operations
2. Fixing test failures (currently ~180 failing tests representing implementation gaps)
3. Improving type handling and reflection capabilities
4. Enhancing polymorphic property access
5. Implementing remaining STU functions

### Priority Implementation Areas

Based on test failure analysis, these are the highest priority implementation areas:

1. **Type System and Type Operations** (~40 failures)
   - `.is()`, `.as()`, `.ofType()` operations
   - Namespace qualification (System vs FHIR) handling
   - Type reflection with `type()` function

2. **Polymorphic Access / Choice Elements** (~15 failures)  
   - Choice element handling (e.g., Observation.value[x])
   - Access to choice properties (Observation.value.unit)
   - Polymorphic casting with .is() and .as()

3. **Date/Time Operations** (~30 failures)
   - Date/time comparisons and equality
   - Timezone handling
   - Date/time arithmetic
   - Different precision date/time comparisons

4. **Collection Functions** (~20 failures)
   - where, select, skip, take functions
   - Collection comparison operations
   - repeat, descendants, children operations

5. **Quantity Handling** (~15 failures)
   - Unit conversion and comparison
   - Quantity arithmetic
   - Complex quantity operations

### Specialized Test Files

The project uses specialized test files for different FHIRPath features:
- `evaluator_tests.rs`: Core evaluator functionality (operators, literals, basic functions)
- `r4_tests.rs`: Tests against R4 format examples from the FHIR specification
- `date_operation_tests.rs`: Date/time arithmetic and operations
- `extension_tests.rs`: Extension function and extension access 
- `polymorphic_tests.rs`, `polymorphic_r4_tests.rs`: Choice element handling and polymorphic access
- `trace_tests.rs`: Tests for the trace() function with various projections
- `tree_navigation_tests.rs`: Tree traversal functions like children() and descendants()
- `type_operation_tests.rs`: Type checks and operations with `is` and `as` operators
- `type_reflection_tests.rs`: Type system reflection and type() function
- `enhanced_variable_tests.rs`: Tests for variable handling and resolution
- `parser_tests.rs`: Tests for the FHIRPath parser
- `is_as_method_tests.rs`: Tests for the is() and as() methods

### Development Workflow

When implementing a new FHIRPath feature:

1. **Understand the specification**: Read the relevant parts of the FHIRPath specification
   - Online: [FHIRPath specification](https://hl7.org/fhirpath/2025Jan/)
   - Local: See `crates/fhirpath/docs/fhirpath-spec.html` (if available)

2. **Create a dedicated module** if appropriate:
   - For significant functions, create a new module in `crates/fhirpath/src/`
   - For smaller functions, add to an existing related module

3. **Implement the function in the evaluator**:
   - Connect to function implementation in `evaluator.rs`
   - Handle type checking, error cases, and context properly
   - Use existing helper functions where appropriate

4. **Add comprehensive tests**:
   - Create or extend test file in `crates/fhirpath/tests/`
   - Cover edge cases, error conditions, and expected behavior
   - Consider adding the feature to existing R4 tests if appropriate

5. **Update the README.md**:
   - Update the implementation status in `crates/fhirpath/README.md`
   - Mark the feature as ✅ (implemented), 🟡 (partially implemented), or 🚧 (in progress)
   - Document any known limitations

6. **Test across FHIR versions**:
   - Ensure the feature works with different FHIR versions (R4, R4B, R5)

7. **Run failing tests**:
   - Check if your implementation fixes any previously failing tests:
   - `cargo test -p fhirpath -- --ignored`

8. **Verify test failures**:
   - The failing_tests.txt file contains a comprehensive list of test failures
   - Check if your implementation resolves any specific failures listed there

### Common Issues and Solutions

- **Stack overflows**: When evaluating complex expressions, stack overflows can occur. Use:
  - `export RUST_MIN_STACK=8388608` to increase stack size
  - Consider refactoring recursive functions to be more stack-efficient

- **Test failures**: When fixing failing tests:
  - Check for dependencies on other unimplemented features
  - Focus on one category of related tests at a time
  - Look for common patterns in failing tests

- **Type system challenges**:
  - Namespace qualification requires careful handling (System vs FHIR)
  - Resource type checking needs to respect FHIR type hierarchy
  - Choice element polymorphic access requires special handling

- **Date/time handling**:
  - Precision matters for date/time operations
  - Account for timezone considerations
  - Partial dates and times may need special handling

- **Quantity comparisons**:
  - Unit conversion is required for proper comparison
  - Need to handle compatible units (e.g., 'g' and 'mg')
  - Time-valued quantities require special handling

### Repository Organization and Conventions

- **Feature Flags**: The codebase uses feature flags (`R4`, `R4B`, `R5`, `R6`) to conditionally compile for different FHIR versions
- **Module Organization**: Each FHIRPath function typically gets its own module with implementation, helper functions, and internal types
- **Test Structure**: Tests are organized in specialized modules to focus on specific functionality areas
- **Error Handling**: Error handling follows patterns established in the evaluator, with well-typed error messages

### Contribution Tips

When contributing new features:

1. **Start small**: If possible, isolate and implement a specific function or operator first
2. **Look for patterns**: Observe how similar functions are implemented
3. **Focus on failing tests**: Prioritize implementing features that fix specific failing tests
4. **Implementation quality**: Strive to match the specification accurately, including edge cases
5. **Document well**: Add docstrings to functions and update the README for implementation status