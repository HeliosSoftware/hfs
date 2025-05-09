# FHIRPath 

This is an implementation of HL7's [FHIRPath Specification - 3.0.0-ballot](https://hl7.org/fhirpath/2025Jan/) written in Rust.

## Table of Contents
 - [About](#-about)
 - [Implementation](#-implementation)
 - [Performance](#-performance)
 - [Features](#-features)


## About
FHIRPath is a path based navigation and extraction language, somewhat like XPath.

### Where is FHIRPath Used? 
FHIRPath is used extensively throughout the FHIR Specification, although, despite it's name, FHIRPath 
is designed to be model-independent and works with data as an abstract model, allowing it to be used with any model.

#### Invariants

#### Search Parameters

#### View Definitions

## Implementation

The FHIRPath implementation consists of:

1. **Parser** - Uses the chumsky parser library to parse FHIRPath expressions into an AST.
2. **Evaluator** - Evaluates the AST against FHIR resources to produce results.
3. **Type System** - Handles different data types and conversions between them.

### Current Implementation Status

We have been working on key FHIRPath features, with the following status:

1. ✅ **Test Framework** - Enhanced to properly fail on unimplemented features
   - ✅ Tests now accurately report implementation gaps
   - ✅ Removed special case handling that artificially made tests pass
   - ✅ Test suite checks conformance with FHIRPath specification

2. ✅ **Core Functionality** - Solid foundation of basic operations
   - ✅ All basic math operations (addition, subtraction, multiplication, division)
   - ✅ Boolean logic (and, or, xor, not)
   - ✅ Basic equality and comparison operators
   - ✅ Existence functions (empty, exists, all, etc.)
   - 🟡 Collection functions (where, select, first, last, etc.) - Basic functionality, but issues with nested collections
   - 🟡 Collection navigation and path traversal - Issues with nested collections and context chaining
   - ✅ Basic type conversion functions
   - ✅ String manipulation functions

3. 🟡 **Advanced Functions**
   - ✅ **Truncate Function** - Implemented to handle numeric values correctly
   - 🟡 **Aggregate Function** - Basic implementation, issues with variable resolution in complex cases
   - ✅ **Trace Function** - Implemented with projection support
   - 🟡 **Repeat Function** - Implemented with cycle detection, needs refinement
   - ✅ **Math Functions** - Support for round, sqrt, abs, ceiling, floor, etc.
   - 🟡 **Extension Support** - Basic implementation, issues with primitive type extensions
   - 🟡 **conformsTo Function** - Basic profile conformance checking, needs refinement

4. 🟡 **Type System and Reflection**
   - ✅ **Basic Type Handling** - Type determination and reflection basics
   - ✅ **Namespace Support** - Framework for System and FHIR namespaces
   - ✅ **Qualified Types** - Support for qualified types (System.Boolean, FHIR.Patient)
   - ✅ **Type Operators** - Improved `is` and `as` operators with namespace qualification 
   - 🟡 **Resource Type Checking** - Basic FHIR type hierarchy support
   - 🟡 **FHIR Type Hierarchy** - Type hierarchy model in `fhir_type_hierarchy.rs` needs refinement

5. 🟡 **Polymorphic Access** - Support for FHIR choice elements
   - 🟡 Basic choice element resolution (like `value` accessing `valueQuantity`) - Needs better implementation
   - 🟡 Polymorphic path resolution (like `value.unit` resolving to `valueQuantity.unit`) - Only partially working
   - ✅ Type operators with polymorphic properties (`value.is(Quantity)`) - Fixed implementation
   - 🟡 Resource references need better resolution and type checking

6. 🟡 **Date/Time Functionality**
   - 🟡 **Date/Time Arithmetic** - Basic operations between dates and durations (needs improvement)
   - ✅ **Component Extraction** - Extracting year, month, day, etc. from dates/times
   - 🟡 **Basic Date/Time Support** - Parsing working, issues with comparison operations
   - 🟡 **Period and Timing** - Needs better handling of FHIR period and timing types

7. ❌ **STU Features**
   - ❌ **STU Functions** - matchesFull(), lastIndexOf(), etc.
   - ❌ **Long Integer Type** - Support for Long values and operations
   - ❌ **Additional String Functions** - STU-defined string operations
   - ❌ **Variable Definition** - Support for defineVariable() function

## Implementation Priorities

Based on our progress and test results, our current implementation priorities are:

1. ✅ **Type System and Is/As Operations** - Completed
   - ✅ Framework for FHIR type hierarchy with resource inheritance model
   - ✅ Support for FHIR.Patient, FHIR.boolean and other qualified types
   - ✅ Namespace-qualified type support (System and FHIR)
   - ✅ Fixed `.is()` and `.as()` functions with proper type handling

2. ✅ **Extension Resolution** - Completed
   - ✅ Fixed extension() function implementation to handle extensions properly
   - ✅ Added support for primitive type extension access
   - ✅ Improved URL variable resolution for standard extensions
   - ✅ Implemented proper extension function dispatch

3. 🟡 **Polymorphic Access for Choice Elements** - In progress
   - Fix polymorphic property access for choice elements
   - Fix value[x] resolution for paths like value.unit
   - ✅ Fixed type checks on choice elements (value.is(Quantity))

4. 🟡 **Collection Navigation and Nested Path Traversal** - In progress
   - Fix recursive flattening for nested collections
   - Fix handling of paths like Patient.name.given to properly collect all nested values
   - Enhance collection operations (where, select) to properly handle nested collections
   - Improve collection path traversal to match the FHIRPath specification

5. 🟡 **Date/Time Arithmetic Operations** - In progress
   - Basic implementation in place for date arithmetic
   - Need to ensure all date arithmetic operations work correctly
   - Need to add support for duration arithmetic and timezone handling

6. ❌ **STU Functions** - Not yet implemented
   - matchesFull(), lastIndexOf(), defineVariable(), etc.
   - Need to add parser and runtime support for these functions

7. ❌ **Long Integer Type** - Not yet implemented
   - Need to add parser and runtime support for Long type

8. 🟡 **Quantity Handling and Unit Conversion** - In progress
   - Fix quantity comparisons with unit conversion
   - Implement time-valued quantity conversion
   - Fix quantity arithmetic operations

The test suite now strictly enforces that all FHIRPath features are properly implemented, and we have identified specific implementation gaps that need to be addressed.

## Performance

Performance results go here...

## Features


**Legend:**
*   ✅ Implemented
*   🟡 Partially Implemented (Basic functionality, known limitations)
*   ❌ Not Implemented
*   🚧 In Progress
*   (STU) - Standard for Trial Use in the specification
    
### [Expressions](https://hl7.org/fhirpath/2025Jan/#expressions)
    
*   [Literals](https://hl7.org/fhirpath/2025Jan/#literals)
    *   [Boolean](https://hl7.org/fhirpath/2025Jan/#boolean): ✅
    *   [String](https://hl7.org/fhirpath/2025Jan/#string) (including escapes): ✅
    *   [Integer](https://hl7.org/fhirpath/2025Jan/#integer): ✅
    *   [Long](https://hl7.org/fhirpath/2025Jan/#long) (STU): ❌ (Parser support needed)
    *   [Decimal](https://hl7.org/fhirpath/2025Jan/#decimal): ✅
    *   [Date](https://hl7.org/fhirpath/2025Jan/#date): 🟡 (Stored as String, type checking issues)
    *   [Time](https://hl7.org/fhirpath/2025Jan/#time): 🟡 (Stored as String, type checking issues)
    *   [DateTime](https://hl7.org/fhirpath/2025Jan/#datetime): 🟡 (Stored as String, comparison issues)
    *   [Quantity](https://hl7.org/fhirpath/2025Jan/#quantity): 🟡 (Basic value/unit storage, comparison issues)
        *   [Time-valued Quantities](https://hl7.org/fhirpath/2025Jan/#time-valued-quantities): 🟡 (Keywords parsed, conversion issues)
    
### [Functions](https://hl7.org/fhirpath/2025Jan/#functions)
    
*   [Existence](https://hl7.org/fhirpath/2025Jan/#existence)
    *   [empty()](https://hl7.org/fhirpath/2025Jan/#empty--boolean): ✅
    *   [exists()](https://hl7.org/fhirpath/2025Jan/#existscriteria--expression--boolean): ✅
    *   [all()](https://hl7.org/fhirpath/2025Jan/#allcriteria--expression--boolean): 🟡 (Issues with complex expressions)
    *   [allTrue()](https://hl7.org/fhirpath/2025Jan/#alltrue--boolean): 🟡 (Issues with nested collections)
    *   [anyTrue()](https://hl7.org/fhirpath/2025Jan/#anytrue--boolean): ✅
    *   [allFalse()](https://hl7.org/fhirpath/2025Jan/#allfalse--boolean): ✅
    *   [anyFalse()](https://hl7.org/fhirpath/2025Jan/#anyfalse--boolean): ✅
    *   [subsetOf()](https://hl7.org/fhirpath/2025Jan/#subsetofother--collection--boolean): 🟡 (Issues with complex variables)
    *   [supersetOf()](https://hl7.org/fhirpath/2025Jan/#supersetofother--collection--boolean): 🟡 (Issues with complex variables)
    *   [count()](https://hl7.org/fhirpath/2025Jan/#count--integer): 🟡 (Issues with nested collections)
    *   [distinct()](https://hl7.org/fhirpath/2025Jan/#distinct--collection): 🟡 (Issues with nested collections)
    *   [isDistinct()](https://hl7.org/fhirpath/2025Jan/#isdistinct--boolean): 🟡 (Issues with complex expressions)
*   [Filtering and Projection](https://hl7.org/fhirpath/2025Jan/#filtering-and-projection)
    *   [where()](https://hl7.org/fhirpath/2025Jan/#wherecriteria--expression--collection): 🟡 (Issues with nested collections)
    *   [select()](https://hl7.org/fhirpath/2025Jan/#selectprojection-expression--collection): 🟡 (Issues with nested collections)
    *   [repeat()](https://hl7.org/fhirpath/2025Jan/#repeatprojection-expression--collection): 🟡 (Needs refinement)
    *   [ofType()](https://hl7.org/fhirpath/2025Jan/#oftypetype--type-specifier--collection): ✅ (Namespace qualification support)
*   [Subsetting](https://hl7.org/fhirpath/2025Jan/#subsetting)
    *   [Indexer `[]`](https://hl7.org/fhirpath/2025Jan/#-index--integer---collection): 🟡 (Issues with nested paths)
    *   [single()](https://hl7.org/fhirpath/2025Jan/#single--collection): 🟡 (Issues in complex paths)
    *   [first()](https://hl7.org/fhirpath/2025Jan/#first--collection): 🟡 (Issues with nested paths)
    *   [last()](https://hl7.org/fhirpath/2025Jan/#last--collection): 🟡 (Issues with nested paths)
    *   [tail()](https://hl7.org/fhirpath/2025Jan/#tail--collection): 🟡 (Issues with nested paths)
    *   [skip()](https://hl7.org/fhirpath/2025Jan/#skipnum--integer--collection): 🟡 (Issues with chained navigations)
    *   [take()](https://hl7.org/fhirpath/2025Jan/#takenum--integer--collection): 🟡 (Issues with chained navigations)
    *   [intersect()](https://hl7.org/fhirpath/2025Jan/#intersectother-collection--collection): ✅ (Order not preserved)
    *   [exclude()](https://hl7.org/fhirpath/2025Jan/#excludeother-collection--collection): ✅
*   [Combining](https://hl7.org/fhirpath/2025Jan/#combining)
    *   [union()](https://hl7.org/fhirpath/2025Jan/#unionother--collection): ✅ (Order not preserved)
    *   [combine()](https://hl7.org/fhirpath/2025Jan/#combineother--collection--collection): ✅ (Order not preserved)
*   [Conversion](https://hl7.org/fhirpath/2025Jan/#conversion)
    *   [Implicit Conversions](https://hl7.org/fhirpath/2025Jan/#conversion): ✅ (Integer/Decimal)
    *   [iif()](https://hl7.org/fhirpath/2025Jan/#iifcriterion-expression-true-result-collection--otherwise-result-collection--collection): 🟡 (Issues with complex expressions)
    *   [toBoolean()](https://hl7.org/fhirpath/2025Jan/#toboolean--boolean): ✅
    *   [convertsToBoolean()](https://hl7.org/fhirpath/2025Jan/#convertstoboolean--boolean): ✅
    *   [toInteger()](https://hl7.org/fhirpath/2025Jan/#tointeger--integer): ✅
    *   [convertsToInteger()](https://hl7.org/fhirpath/2025Jan/#convertstointeger--boolean): ✅
    *   [toLong()](https://hl7.org/fhirpath/2025Jan/#tolong--long) (STU): ❌
    *   [convertsToLong()](https://hl7.org/fhirpath/2025Jan/#convertstolong--boolean) (STU): ❌
    *   [toDate()](https://hl7.org/fhirpath/2025Jan/#todate--date): 🟡 (Comparison issues)
    *   [convertsToDate()](https://hl7.org/fhirpath/2025Jan/#convertstodate--boolean): 🟡 (Comparison issues)
    *   [toDateTime()](https://hl7.org/fhirpath/2025Jan/#todatetime--datetime): 🟡 (Comparison issues)
    *   [convertsToDateTime()](https://hl7.org/fhirpath/2025Jan/#convertstodatetime--boolean): 🟡 (Comparison issues)
    *   [toDecimal()](https://hl7.org/fhirpath/2025Jan/#todecimal--decimal): ✅
    *   [convertsToDecimal()](https://hl7.org/fhirpath/2025Jan/#convertstodecimal--boolean): ✅
    *   [toQuantity()](https://hl7.org/fhirpath/2025Jan/#toquantityunit--string--quantity): 🟡 (Basic types, no unit conversion)
    *   [convertsToQuantity()](https://hl7.org/fhirpath/2025Jan/#convertstoquantityunit--string--boolean): 🟡 (Basic types, no unit conversion)
    *   [toString()](https://hl7.org/fhirpath/2025Jan/#tostring--string): 🟡 (Issues with quantities)
    *   [convertsToString()](https://hl7.org/fhirpath/2025Jan/#convertstostring--string): ✅
    *   [toTime()](https://hl7.org/fhirpath/2025Jan/#totime--time): 🟡 (String to Time only)
    *   [convertsToTime()](https://hl7.org/fhirpath/2025Jan/#convertstotime--boolean): 🟡 (String to Time only)
*   [String Manipulation](https://hl7.org/fhirpath/2025Jan/#string-manipulation)
    *   [indexOf()](https://hl7.org/fhirpath/2025Jan/#indexofsubstring--string--integer): ✅
    *   [lastIndexOf()](https://hl7.org/fhirpath/2025Jan/#lastindexofsubstring--string--integer) (STU): ❌
    *   [substring()](https://hl7.org/fhirpath/2025Jan/#substringstart--integer--length--integer--string): 🟡 (Edge case issues)
    *   [startsWith()](https://hl7.org/fhirpath/2025Jan/#startswithprefix--string--boolean): ✅
    *   [endsWith()](https://hl7.org/fhirpath/2025Jan/#endswithsuffix--string--boolean): ✅
    *   [contains()](https://hl7.org/fhirpath/2025Jan/#containssubstring--string--boolean): ✅
    *   [upper()](https://hl7.org/fhirpath/2025Jan/#upper--string): ✅
    *   [lower()](https://hl7.org/fhirpath/2025Jan/#lower--string): ✅
    *   [replace()](https://hl7.org/fhirpath/2025Jan/#replacepattern--string-substitution--string--string): ✅
    *   [matches()](https://hl7.org/fhirpath/2025Jan/#matchesregex--string--boolean): ✅
    *   [matchesFull()](https://hl7.org/fhirpath/2025Jan/#matchesfullregex--string--boolean) (STU): ❌
    *   [replaceMatches()](https://hl7.org/fhirpath/2025Jan/#replacematchesregex--string-substitution-string--string): ✅
    *   [length()](https://hl7.org/fhirpath/2025Jan/#length--integer): ✅
    *   [toChars()](https://hl7.org/fhirpath/2025Jan/#tochars--collection): ✅
*   [Additional String Functions](https://hl7.org/fhirpath/2025Jan/#additional-string-functions) (STU): ❌ (All)
*   [Math](https://hl7.org/fhirpath/2025Jan/#math) (STU): 🟡 (Basic implementations)
    *   [round()](https://hl7.org/fhirpath/2025Jan/#round-precision--integer--decimal): ✅
    *   [sqrt()](https://hl7.org/fhirpath/2025Jan/#sqrt--decimal): 🟡 (Comparison issues)
    *   [abs()](https://hl7.org/fhirpath/2025Jan/#abs--decimal): 🟡 (Issues with quantities)
    *   [ceiling()](https://hl7.org/fhirpath/2025Jan/#ceiling--decimal): ✅
    *   [exp()](https://hl7.org/fhirpath/2025Jan/#exp--decimal): ✅
    *   [floor()](https://hl7.org/fhirpath/2025Jan/#floor--decimal): ✅
    *   [ln()](https://hl7.org/fhirpath/2025Jan/#ln--decimal): ✅
    *   [log()](https://hl7.org/fhirpath/2025Jan/#log-base--decimal--decimal): ✅
    *   [power()](https://hl7.org/fhirpath/2025Jan/#power-exponent--decimal--decimal): ✅
    *   [truncate()](https://hl7.org/fhirpath/2025Jan/#truncate--decimal): ✅
*   [Tree Navigation](https://hl7.org/fhirpath/2025Jan/#tree-navigation)
    *   [children()](https://hl7.org/fhirpath/2025Jan/#children--collection): 🟡 (Count issues)
    *   [descendants()](https://hl7.org/fhirpath/2025Jan/#descendants--collection): 🟡 (Count issues) 
    *   [extension()](https://hl7.org/fhirpath/2025Jan/#extensionurl--url-string--collection): ✅ (Supports both object and primitive extensions)
*   [Utility Functions](https://hl7.org/fhirpath/2025Jan/#utility-functions)
    *   [trace()](https://hl7.org/fhirpath/2025Jan/#tracename--string--projection-expression--collection): 🟡 (Issues with complex projections)
    *   [now()](https://hl7.org/fhirpath/2025Jan/#now--datetime): 🟡 (Comparison issues)
    *   [timeOfDay()](https://hl7.org/fhirpath/2025Jan/#timeofday--time): 🟡 (Comparison issues)
    *   [today()](https://hl7.org/fhirpath/2025Jan/#today--date): 🟡 (Comparison issues)
    *   [defineVariable()](https://hl7.org/fhirpath/2025Jan/#definevariablename-string--expr-expression) (STU): ❌
    *   [lowBoundary()](https://hl7.org/fhirpath/2025Jan/#lowboundaryprecision-integer-decimal--date--datetime--time) (STU): ❌
    *   [highBoundary()](https://hl7.org/fhirpath/2025Jan/#highboundaryprecision-integer-decimal--date--datetime--time) (STU): ❌
    *   [precision()](https://hl7.org/fhirpath/2025Jan/#precision--integer) (STU): ❌
*   [Date/DateTime/Time Component Extraction](https://hl7.org/fhirpath/2025Jan/#extract-datedatetimetime-components) (STU): ✅ (All component functions implemented: yearOf, monthOf, dayOf, hourOf, minuteOf, secondOf, millisecondOf)
    
### [Operations](https://hl7.org/fhirpath/2025Jan/#operations)
    
*   [Equality](https://hl7.org/fhirpath/2025Jan/#equality)
    *   [`=` (Equals)](https://hl7.org/fhirpath/2025Jan/#-equals): 🟡 (Issues with complex types, dates)
    *   [`~` (Equivalent)](https://hl7.org/fhirpath/2025Jan/#-equivalent): 🟡 (Issues with quantities and dates) 
    *   [`!=` (Not Equals)](https://hl7.org/fhirpath/2025Jan/#-not-equals): 🟡 (Same issues as equals)
    *   [`!~` (Not Equivalent)](https://hl7.org/fhirpath/2025Jan/#-not-equivalent): 🟡 (Same issues as equivalent)
*   [Comparison](https://hl7.org/fhirpath/2025Jan/#comparison)
    *   [`>` (Greater Than)](https://hl7.org/fhirpath/2025Jan/#-greater-than): 🟡 (Issues with dates, quantities)
    *   [`<` (Less Than)](https://hl7.org/fhirpath/2025Jan/#-less-than): 🟡 (Issues with dates, quantities)
    *   [`<=` (Less or Equal)](https://hl7.org/fhirpath/2025Jan/#-less-or-equal): 🟡 (Issues with dates, quantities)
    *   [`>=` (Greater or Equal)](https://hl7.org/fhirpath/2025Jan/#-greater-or-equal): 🟡 (Issues with dates, quantities)
*   [Types](https://hl7.org/fhirpath/2025Jan/#types)
    *   [`is`](https://hl7.org/fhirpath/2025Jan/#is-type-specifier): ✅ (Improved with namespace qualification)
    *   [`as`](https://hl7.org/fhirpath/2025Jan/#as-type-specifier): ✅ (Improved with namespace qualification)
*   [Collections](https://hl7.org/fhirpath/2025Jan/#collections-1)
    *   [`|` (Union)](https://hl7.org/fhirpath/2025Jan/#-union-collections): ✅ (Order not preserved)
    *   [`in` (Membership)](https://hl7.org/fhirpath/2025Jan/#in-membership): ✅
    *   [`contains` (Containership)](https://hl7.org/fhirpath/2025Jan/#contains-containership): ✅
    *   [Collection Navigation](https://hl7.org/fhirpath/2025Jan/#path-selection): 🟡 (Issues with nested collections)
*   [Boolean Logic](https://hl7.org/fhirpath/2025Jan/#boolean-logic)
    *   [`and`](https://hl7.org/fhirpath/2025Jan/#and): ✅
    *   [`or`](https://hl7.org/fhirpath/2025Jan/#or): ✅
    *   [`xor`](https://hl7.org/fhirpath/2025Jan/#xor): ✅
    *   [`implies`](https://hl7.org/fhirpath/2025Jan/#implies): ✅
    *   [`not()`](https://hl7.org/fhirpath/2025Jan/#not--boolean): ✅
*   [Math](https://hl7.org/fhirpath/2025Jan/#math-1)
    *   [`*` (Multiplication)](https://hl7.org/fhirpath/2025Jan/#-multiplication): 🟡 (Issues with quantities)
    *   [`/` (Division)](https://hl7.org/fhirpath/2025Jan/#-division): 🟡 (Precision issues)
    *   [`+` (Addition)](https://hl7.org/fhirpath/2025Jan/#-addition): ✅ (Numeric, String)
    *   [`-` (Subtraction)](https://hl7.org/fhirpath/2025Jan/#--subtraction): 🟡 (Issues with strings)
    *   [`div` (Integer Division)](https://hl7.org/fhirpath/2025Jan/#div): ✅ (Numeric)
    *   [`mod` (Modulo)](https://hl7.org/fhirpath/2025Jan/#mod): ✅ (Numeric)
    *   [`&` (String Concatenation)](https://hl7.org/fhirpath/2025Jan/#-string-concatenation): ✅
*   [Date/Time Arithmetic](https://hl7.org/fhirpath/2025Jan/#datetime-arithmetic): 🟡 (Implementation issues with timezone and precision)
*   [Operator Precedence](https://hl7.org/fhirpath/2025Jan/#operator-precedence): 🟡 (Issues with complex cases)
    
### [Aggregates](https://hl7.org/fhirpath/2025Jan/#aggregates)
    
*   [aggregate()](https://hl7.org/fhirpath/2025Jan/#aggregateaggregator--expression--init--value--value) (STU): 🟡 (Issues with complex expressions)

### [Lexical Elements](https://hl7.org/fhirpath/2025Jan/#lexical-elements)

*   [Lexical Elements](https://hl7.org/fhirpath/2025Jan/#lexical-elements): ✅ (Handled by parser)
    
### [Environment Variables](https://hl7.org/fhirpath/2025Jan/#environment-variables)
    
*   [`%variable`](https://hl7.org/fhirpath/2025Jan/#environment-variables): 🟡 (Some issues with extension resolution)
*   [`%context`](https://hl7.org/fhirpath/2025Jan/#environment-variables): 🟡 (Basic implementation)
    
### [Types and Reflection](https://hl7.org/fhirpath/2025Jan/#types-and-reflection)
    
*   [Models](https://hl7.org/fhirpath/2025Jan/#models): 🟡 (Issues with namespace qualification)
*   [Reflection (`type()`)](https://hl7.org/fhirpath/2025Jan/#reflection) (STU): ✅ (Enhanced with namespace support)
    
### [Type Safety and Strict Evaluation](https://hl7.org/fhirpath/2025Jan/#type-safety-and-strict-evaluation)
    
*   [Type Safety / Strict Evaluation](https://hl7.org/fhirpath/2025Jan/#type-safety-and-strict-evaluation): 🟡 (Runtime checks, errors signaled via Empty/panic)
    
    
    


