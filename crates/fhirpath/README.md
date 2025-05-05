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

Implementation notes go here...

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
    *   [Date](https://hl7.org/fhirpath/2025Jan/#date): ✅ (Stored as String)
    *   [Time](https://hl7.org/fhirpath/2025Jan/#time): ✅ (Stored as String)
    *   [DateTime](https://hl7.org/fhirpath/2025Jan/#datetime): ✅ (Stored as String)
    *   [Quantity](https://hl7.org/fhirpath/2025Jan/#quantity): ✅ (Basic value/unit storage)
        *   [Time-valued Quantities](https://hl7.org/fhirpath/2025Jan/#time-valued-quantities): ✅ (Keywords parsed)
    
### [Functions](https://hl7.org/fhirpath/2025Jan/#functions)
    
*   [Existence](https://hl7.org/fhirpath/2025Jan/#existence)
    *   [empty()](https://hl7.org/fhirpath/2025Jan/#empty--boolean): ✅
    *   [exists()](https://hl7.org/fhirpath/2025Jan/#existscriteria--expression--boolean): ✅
    *   [all()](https://hl7.org/fhirpath/2025Jan/#allcriteria--expression--boolean): ✅
    *   [allTrue()](https://hl7.org/fhirpath/2025Jan/#alltrue--boolean): ✅
    *   [anyTrue()](https://hl7.org/fhirpath/2025Jan/#anytrue--boolean): ✅
    *   [allFalse()](https://hl7.org/fhirpath/2025Jan/#allfalse--boolean): ✅
    *   [anyFalse()](https://hl7.org/fhirpath/2025Jan/#anyfalse--boolean): ✅
    *   [subsetOf()](https://hl7.org/fhirpath/2025Jan/#subsetofother--collection--boolean): ✅
    *   [supersetOf()](https://hl7.org/fhirpath/2025Jan/#supersetofother--collection--boolean): ✅
    *   [count()](https://hl7.org/fhirpath/2025Jan/#count--integer): ✅
    *   [distinct()](https://hl7.org/fhirpath/2025Jan/#distinct--collection): ✅ (Order not preserved)
    *   [isDistinct()](https://hl7.org/fhirpath/2025Jan/#isdistinct--boolean): ✅
*   [Filtering and Projection](https://hl7.org/fhirpath/2025Jan/#filtering-and-projection)
    *   [where()](https://hl7.org/fhirpath/2025Jan/#wherecriteria--expression--collection): ✅ (Requires expression passing mechanism)
    *   [select()](https://hl7.org/fhirpath/2025Jan/#selectprojection-expression--collection): ✅ (Requires expression passing mechanism)
    *   [repeat()](https://hl7.org/fhirpath/2025Jan/#repeatprojection-expression--collection): ❌
    *   [ofType()](https://hl7.org/fhirpath/2025Jan/#oftypetype--type-specifier--collection): ✅ (Basic type check, no inheritance)
*   [Subsetting](https://hl7.org/fhirpath/2025Jan/#subsetting)
    *   [Indexer `[]`](https://hl7.org/fhirpath/2025Jan/#-index--integer---collection): ✅
    *   [single()](https://hl7.org/fhirpath/2025Jan/#single--collection): ✅ (Signals error for multiple items)
    *   [first()](https://hl7.org/fhirpath/2025Jan/#first--collection): ✅
    *   [last()](https://hl7.org/fhirpath/2025Jan/#last--collection): ✅
    *   [tail()](https://hl7.org/fhirpath/2025Jan/#tail--collection): ✅
    *   [skip()](https://hl7.org/fhirpath/2025Jan/#skipnum--integer--collection): ✅
    *   [take()](https://hl7.org/fhirpath/2025Jan/#takenum--integer--collection): ✅
    *   [intersect()](https://hl7.org/fhirpath/2025Jan/#intersectother-collection--collection): ✅ (Order not preserved)
    *   [exclude()](https://hl7.org/fhirpath/2025Jan/#excludeother-collection--collection): ✅
*   [Combining](https://hl7.org/fhirpath/2025Jan/#combining)
    *   [union()](https://hl7.org/fhirpath/2025Jan/#unionother--collection): ✅ (Order not preserved)
    *   [combine()](https://hl7.org/fhirpath/2025Jan/#combineother--collection--collection): ✅ (Order not preserved)
*   [Conversion](https://hl7.org/fhirpath/2025Jan/#conversion)
    *   [Implicit Conversions](https://hl7.org/fhirpath/2025Jan/#conversion): ✅ (Integer/Decimal)
    *   [iif()](https://hl7.org/fhirpath/2025Jan/#iifcriterion-expression-true-result-collection--otherwise-result-collection--collection): ✅ (Requires expression passing)
    *   [toBoolean()](https://hl7.org/fhirpath/2025Jan/#toboolean--boolean): ✅
    *   [convertsToBoolean()](https://hl7.org/fhirpath/2025Jan/#convertstoboolean--boolean): ✅
    *   [toInteger()](https://hl7.org/fhirpath/2025Jan/#tointeger--integer): ✅
    *   [convertsToInteger()](https://hl7.org/fhirpath/2025Jan/#convertstointeger--boolean): ✅
    *   [toLong()](https://hl7.org/fhirpath/2025Jan/#tolong--long) (STU): ❌
    *   [convertsToLong()](https://hl7.org/fhirpath/2025Jan/#convertstolong--boolean) (STU): ❌
    *   [toDate()](https://hl7.org/fhirpath/2025Jan/#todate--date): ✅ (Handles String/DateTime input)
    *   [convertsToDate()](https://hl7.org/fhirpath/2025Jan/#convertstodate--boolean): ✅ (Handles String/DateTime input)
    *   [toDateTime()](https://hl7.org/fhirpath/2025Jan/#todatetime--datetime): ✅ (Handles String/Date input)
    *   [convertsToDateTime()](https://hl7.org/fhirpath/2025Jan/#convertstodatetime--boolean): ✅ (Handles String/Date input)
    *   [toDecimal()](https://hl7.org/fhirpath/2025Jan/#todecimal--decimal): ✅
    *   [convertsToDecimal()](https://hl7.org/fhirpath/2025Jan/#convertstodecimal--boolean): ✅
    *   [toQuantity()](https://hl7.org/fhirpath/2025Jan/#toquantityunit--string--quantity): 🟡 (Basic types, no unit conversion)
    *   [convertsToQuantity()](https://hl7.org/fhirpath/2025Jan/#convertstoquantityunit--string--boolean): 🟡 (Basic types, no unit conversion)
    *   [toString()](https://hl7.org/fhirpath/2025Jan/#tostring--string): ✅
    *   [convertsToString()](https://hl7.org/fhirpath/2025Jan/#convertstostring--string): ✅
    *   [toTime()](https://hl7.org/fhirpath/2025Jan/#totime--time): 🟡 (String to Time only)
    *   [convertsToTime()](https://hl7.org/fhirpath/2025Jan/#convertstotime--boolean): 🟡 (String to Time only)
*   [String Manipulation](https://hl7.org/fhirpath/2025Jan/#string-manipulation)
    *   [indexOf()](https://hl7.org/fhirpath/2025Jan/#indexofsubstring--string--integer): ✅
    *   [lastIndexOf()](https://hl7.org/fhirpath/2025Jan/#lastindexofsubstring--string--integer) (STU): ❌
    *   [substring()](https://hl7.org/fhirpath/2025Jan/#substringstart--integer--length--integer--string): ✅
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
*   [Math](https://hl7.org/fhirpath/2025Jan/#math) (STU): ❌ (All)
*   [Tree Navigation](https://hl7.org/fhirpath/2025Jan/#tree-navigation)
    *   [children()](https://hl7.org/fhirpath/2025Jan/#children--collection): ❌
    *   [descendants()](https://hl7.org/fhirpath/2025Jan/#descendants--collection): ❌
*   [Utility Functions](https://hl7.org/fhirpath/2025Jan/#utility-functions)
    *   [trace()](https://hl7.org/fhirpath/2025Jan/#tracename--string--projection-expression--collection): ❌
    *   [now()](https://hl7.org/fhirpath/2025Jan/#now--datetime): ✅ (Uses local time, determinism TBD)
    *   [timeOfDay()](https://hl7.org/fhirpath/2025Jan/#timeofday--time): ✅ (Uses local time, determinism TBD)
    *   [today()](https://hl7.org/fhirpath/2025Jan/#today--date): ✅ (Uses local time, determinism TBD)
    *   [defineVariable()](https://hl7.org/fhirpath/2025Jan/#definevariablename-string--expr-expression) (STU): ❌
    *   [lowBoundary()](https://hl7.org/fhirpath/2025Jan/#lowboundaryprecision-integer-decimal--date--datetime--time) (STU): ❌
    *   [highBoundary()](https://hl7.org/fhirpath/2025Jan/#highboundaryprecision-integer-decimal--date--datetime--time) (STU): ❌
    *   [precision()](https://hl7.org/fhirpath/2025Jan/#precision--integer) (STU): ❌
*   [Date/DateTime/Time Component Extraction](https://hl7.org/fhirpath/2025Jan/#extract-datedatetimetime-components) (STU): ❌ (All)
    
### [Operations](https://hl7.org/fhirpath/2025Jan/#operations)
    
*   [Equality](https://hl7.org/fhirpath/2025Jan/#equality)
    *   [`=` (Equals)](https://hl7.org/fhirpath/2025Jan/#-equals): ✅ (Basic types, collection order matters)
    *   [`~` (Equivalent)](https://hl7.org/fhirpath/2025Jan/#-equivalent): ✅ (Basic types, string normalization basic, collection order ignored)
    *   [`!=` (Not Equals)](https://hl7.org/fhirpath/2025Jan/#-not-equals): ✅
    *   [`!~` (Not Equivalent)](https://hl7.org/fhirpath/2025Jan/#-not-equivalent): ✅
*   [Comparison](https://hl7.org/fhirpath/2025Jan/#comparison)
    *   [`>` (Greater Than)](https://hl7.org/fhirpath/2025Jan/#-greater-than): ✅ (Numeric, String)
    *   [`<` (Less Than)](https://hl7.org/fhirpath/2025Jan/#-less-than): ✅ (Numeric, String)
    *   [`<=` (Less or Equal)](https://hl7.org/fhirpath/2025Jan/#-less-or-equal): ✅ (Numeric, String)
    *   [`>=` (Greater or Equal)](https://hl7.org/fhirpath/2025Jan/#-greater-or-equal): ✅ (Numeric, String)
*   [Types](https://hl7.org/fhirpath/2025Jan/#types)
    *   [`is`](https://hl7.org/fhirpath/2025Jan/#is-type-specifier): ✅ (Basic types)
    *   [`as`](https://hl7.org/fhirpath/2025Jan/#as-type-specifier): ✅ (Basic types)
*   [Collections](https://hl7.org/fhirpath/2025Jan/#collections-1)
    *   [`|` (Union)](https://hl7.org/fhirpath/2025Jan/#-union-collections): ✅ (Order not preserved)
    *   [`in` (Membership)](https://hl7.org/fhirpath/2025Jan/#in-membership): ✅
    *   [`contains` (Containership)](https://hl7.org/fhirpath/2025Jan/#contains-containership): ✅
*   [Boolean Logic](https://hl7.org/fhirpath/2025Jan/#boolean-logic)
    *   [`and`](https://hl7.org/fhirpath/2025Jan/#and): ✅
    *   [`or`](https://hl7.org/fhirpath/2025Jan/#or): ✅
    *   [`xor`](https://hl7.org/fhirpath/2025Jan/#xor): ✅
    *   [`implies`](https://hl7.org/fhirpath/2025Jan/#implies): ✅
    *   [`not()`](https://hl7.org/fhirpath/2025Jan/#not--boolean): ✅
*   [Math](https://hl7.org/fhirpath/2025Jan/#math-1)
    *   [`*` (Multiplication)](https://hl7.org/fhirpath/2025Jan/#-multiplication): ✅ (Numeric)
    *   [`/` (Division)](https://hl7.org/fhirpath/2025Jan/#-division): ✅ (Numeric, always Decimal result)
    *   [`+` (Addition)](https://hl7.org/fhirpath/2025Jan/#-addition): ✅ (Numeric, String)
    *   [`-` (Subtraction)](https://hl7.org/fhirpath/2025Jan/#--subtraction): ✅ (Numeric)
    *   [`div` (Integer Division)](https://hl7.org/fhirpath/2025Jan/#div): ✅ (Numeric)
    *   [`mod` (Modulo)](https://hl7.org/fhirpath/2025Jan/#mod): ✅ (Numeric)
    *   [`&` (String Concatenation)](https://hl7.org/fhirpath/2025Jan/#-string-concatenation): ✅
*   [Date/Time Arithmetic](https://hl7.org/fhirpath/2025Jan/#datetime-arithmetic): ❌
*   [Operator Precedence](https://hl7.org/fhirpath/2025Jan/#operator-precedence): ✅
    
### [Aggregates](https://hl7.org/fhirpath/2025Jan/#aggregates)
    
*   [aggregate()](https://hl7.org/fhirpath/2025Jan/#aggregateaggregator--expression--init--value--value) (STU): ❌

### [Lexical Elements](https://hl7.org/fhirpath/2025Jan/#lexical-elements)

*   [Lexical Elements](https://hl7.org/fhirpath/2025Jan/#lexical-elements): ✅ (Handled by parser)
    
### [Environment Variables](https://hl7.org/fhirpath/2025Jan/#environment-variables)
    
*   [`%variable`](https://hl7.org/fhirpath/2025Jan/#environment-variables): ✅
*   [`%context`](https://hl7.org/fhirpath/2025Jan/#environment-variables): ✅ (Basic implementation)
    
### [Types and Reflection](https://hl7.org/fhirpath/2025Jan/#types-and-reflection)
    
*   [Models](https://hl7.org/fhirpath/2025Jan/#models): ✅ (Implicit System, FHIR namespaces assumed)
*   [Reflection (`type()`)](https://hl7.org/fhirpath/2025Jan/#reflection) (STU): ❌
    
### [Type Safety and Strict Evaluation](https://hl7.org/fhirpath/2025Jan/#type-safety-and-strict-evaluation)
    
*   [Type Safety / Strict Evaluation](https://hl7.org/fhirpath/2025Jan/#type-safety-and-strict-evaluation): 🟡 (Runtime checks, errors signaled via Empty/panic)
    
    
    



