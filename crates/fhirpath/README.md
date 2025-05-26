# FHIRPath 

This is an implementation of HL7's [FHIRPath Specification - 3.0.0-ballot](https://hl7.org/fhirpath/2025Jan/) written in Rust.

## Table of Contents
 - [About FHIRPath](#about-fhirpath)
 - [Features Implemented](#features-implemented)
 - [Architecture](#architecture)
 - [Performance](#performance)

## About FHIRPath

FHIRPath is a path-based navigation and extraction language for healthcare data that is used in many different contexts within healthcare IT systems. Here are the main places where FHIRPath is implemented and used:

### FHIR Specification and Resource Validation

FHIRPath is used to define and express constraints and co-occurrence rules in FHIR resources within the FHIR specification.

**Example (Validation Invariant):**
```fhirpath
reference.startsWith('#').not() or 
($context.reference.substring(1) in $resource.contained.id)
```

This invariant ensures that a local reference in a resource actually points to a contained resource that exists, checking that the reference (if it starts with "#") points to a valid contained resource ID.

**Relevant Specification Link:**
- [FHIR Validation](https://www.hl7.org/fhir/validation.html)
- [FHIRPath in FHIR R4](https://www.hl7.org/fhir/R4/fhirpath.html)

### FHIR Search Parameter Definitions

FHIRPath defines what contents a search parameter refers to in FHIR resources.

**Example (Search Parameter Path):**
```fhirpath
Patient.name.given
```

This path is used in a search parameter definition to specify that the search parameter applies to a patient's given names.

**More Complex Example:**
```fhirpath
Patient.extension('http://example.org/myExtension').value
```

This path is used to create a search parameter that indexes values from a specific extension.

**Relevant Specification Link:**
- [Search Parameter Resource](https://www.hl7.org/fhir/searchparameter.html)
- [FHIRPath Expressions](https://smilecdr.com/docs/fhir_standard/fhirpath_expressions.html)

### FHIR Implementation Guides

FHIRPath is used to express constraints in implementation guides, particularly for profile definitions.

**Example (Profile Constraint):**
```fhirpath
telecom.where(system='phone').exists() or telecom.where(system='email').exists()
```

This constraint requires that a resource has at least one telecom with either a phone or email system.

**Example (Slicing Discriminator):**
```fhirpath
Observation.category
```

This path is used as a discriminator for slicing, meaning the category element will define uniqueness in sliced arrays.

**Relevant Specification Link:**
- [Profiling FHIR](https://www.hl7.org/fhir/profiling.html)
- [StructureDefinition Resource](https://www.hl7.org/fhir/structuredefinition.html)

### Clinical Decision Support

FHIRPath is used in clinical decision support systems, particularly within CDS Hooks and smart apps.

**Example (CDS Hook Prefetch Template):**
```json
"prefetch": {
  "patient": "Patient/{{context.patientId}}",
  "medications": "MedicationRequest?patient={{context.patientId}}&status=active",
  "conditions": "Condition?patient={{context.patientId}}&clinicalStatus=active&_fhirpath=code.memberOf('http://example.org/ValueSet/ChronicConditions')"
}
```

This prefetch template uses FHIRPath to filter conditions to only those with codes in a specific value set.

**Example (Clinical Rule):**
```fhirpath
Observation.where(code.coding.system='http://loinc.org' and code.coding.code='8480-6')
  .value.quantity > 140
```

This expression identifies systolic blood pressure observations with values above 140.

**Relevant Specification Link:**
- [CDS Hooks](https://cds-hooks.hl7.org/)
- [FHIR Clinical Reasoning Module](https://www.hl7.org/fhir/clinicalreasoning-module.html)
- [CDS on FHIR](https://build.fhir.org/clinicalreasoning-cds-on-fhir.html)

### Terminology Service Integration

FHIRPath provides access to terminology services through a %terminologies object.

**Example (Terminology Service Call):**
```fhirpath
%terminologies.validateVS('http://hl7.org/fhir/ValueSet/observation-vitalsignresult', %context)
```

This expression validates that a code is in the vital sign's value set.

**Example (Member Check):**
```fhirpath
Observation.code.coding.where(memberOf('http://hl7.org/fhir/ValueSet/observation-vitalsignresult'))
```

This expression filters coding elements to only those that are members of the specified value set.

**Relevant Specification Link:**
- [FHIRPath Terminology Services](https://www.hl7.org/fhir/fhirpath.html#txapi)
- [FHIR Terminology Service](https://www.hl7.org/fhir/terminology-service.html)

###  FHIR Resource Mapping and Transformation

FHIRPath is used to map between different FHIR versions or between FHIR and other formats.

**Example (Mapping Rule):**
```fhirpath
source.telecom.where(system='phone').value
```

This expression might be used in a mapping language to extract phone numbers from a source resource.

**Relevant Specification Link:**
- [FHIR Mapping Language](https://www.hl7.org/fhir/mapping-language.html)

### SQL on FHIR

The SQL on FHIR specification leverages FHIRPath to define flattened tabular views of FHIR data that can be queried using standard SQL.

**Example ViewDefinition:**
```json
{
  "resourceType": "ViewDefinition",
  "id": "patient-demographics",
  "name": "PatientDemographics",
  "title": "Basic Patient Demographics",
  "description": "A flattened view of key patient demographic information",
  "from": {
    "resourceType": "Patient"
  },
  "select": [
    {
      "column": [
        {"name": "id", "path": "getResourceKey()"},
        {"name": "birth_date", "path": "birthDate"},
        {"name": "gender", "path": "gender"},
        {"name": "first_name", "path": "name.where(use='official').given.first()"},
        {"name": "last_name", "path": "name.where(use='official').family"},
        {"name": "ssn", "path": "identifier.where(system='http://hl7.org/fhir/sid/us-ssn').value"},
        {"name": "email", "path": "telecom.where(system='email').value"},
        {"name": "phone", "path": "telecom.where(system='phone' and use='mobile').value"},
        {"name": "address_line", "path": "address.where(use='home').line.join(', ')"},
        {"name": "city", "path": "address.where(use='home').city"},
        {"name": "state", "path": "address.where(use='home').state"},
        {"name": "postal_code", "path": "address.where(use='home').postalCode"}
      ]
    }
  ]
}
```

**Relevant Specification Link:**
- [SQL on FHIR](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/)

## Features Implemented

**Legend:**
*   ✅ Implemented
*   🟡 Partially Implemented (Basic functionality, known limitations)
*   ❌ Not Implemented
*   🚧 In Progress
*   (STU) - Standard for Trial Use in the specification
    
### [Expressions](https://hl7.org/fhirpath/2025Jan/#expressions)
    
*   [Literals](https://hl7.org/fhirpath/2025Jan/#literals)
    *   [Boolean](https://hl7.org/fhirpath/2025Jan/#boolean): ✅
    *   [String](https://hl7.org/fhirpath/2025Jan/#string): ✅
    *   [Integer](https://hl7.org/fhirpath/2025Jan/#integer): ✅
    *   [Long](https://hl7.org/fhirpath/2025Jan/#long) (STU): ✅
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
    *   [all()](https://hl7.org/fhirpath/2025Jan/#allcriteria--expression--boolean): ✅ (Some limitations with complex type operations and variable indexing)
    *   [allTrue()](https://hl7.org/fhirpath/2025Jan/#alltrue--boolean): ✅
    *   [anyTrue()](https://hl7.org/fhirpath/2025Jan/#anytrue--boolean): ✅
    *   [allFalse()](https://hl7.org/fhirpath/2025Jan/#allfalse--boolean): ✅
    *   [anyFalse()](https://hl7.org/fhirpath/2025Jan/#anyfalse--boolean): ✅
    *   [subsetOf()](https://hl7.org/fhirpath/2025Jan/#subsetofother--collection--boolean): 🟡 (Issues with complex variables)
    *   [supersetOf()](https://hl7.org/fhirpath/2025Jan/#supersetofother--collection--boolean): 🟡 (Issues with complex variables)
    *   [count()](https://hl7.org/fhirpath/2025Jan/#count--integer): ✅
    *   [distinct()](https://hl7.org/fhirpath/2025Jan/#distinct--collection): ✅ (Order not preserved)
    *   [isDistinct()](https://hl7.org/fhirpath/2025Jan/#isdistinct--boolean): ✅
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
    *   [iif()](https://hl7.org/fhirpath/2025Jan/#iifcriterion-expression-true-result-collection--otherwise-result-collection--collection): ✅
    *   [toBoolean()](https://hl7.org/fhirpath/2025Jan/#toboolean--boolean): ✅
    *   [convertsToBoolean()](https://hl7.org/fhirpath/2025Jan/#convertstoboolean--boolean): ✅
    *   [toInteger()](https://hl7.org/fhirpath/2025Jan/#tointeger--integer): ✅
    *   [convertsToInteger()](https://hl7.org/fhirpath/2025Jan/#convertstointeger--boolean): ✅
    *   [toLong()](https://hl7.org/fhirpath/2025Jan/#tolong--long) (STU): ✅
    *   [convertsToLong()](https://hl7.org/fhirpath/2025Jan/#convertstolong--boolean) (STU): ✅
    *   [toDate()](https://hl7.org/fhirpath/2025Jan/#todate--date): 🟡 (Comparison issues)
    *   [convertsToDate()](https://hl7.org/fhirpath/2025Jan/#convertstodate--boolean): 🟡 (Comparison issues)
    *   [toDateTime()](https://hl7.org/fhirpath/2025Jan/#todatetime--datetime): 🟡 (Comparison issues)
    *   [convertsToDateTime()](https://hl7.org/fhirpath/2025Jan/#convertstodatetime--boolean): 🟡 (Comparison issues)
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
*   [Math](https://hl7.org/fhirpath/2025Jan/#math) (STU): ✅
    *   [round()](https://hl7.org/fhirpath/2025Jan/#round-precision--integer--decimal): ✅
    *   [sqrt()](https://hl7.org/fhirpath/2025Jan/#sqrt--decimal): ✅
    *   [abs()](https://hl7.org/fhirpath/2025Jan/#abs--decimal): ✅
    *   [ceiling()](https://hl7.org/fhirpath/2025Jan/#ceiling--decimal): ✅
    *   [exp()](https://hl7.org/fhirpath/2025Jan/#exp--decimal): ✅
    *   [floor()](https://hl7.org/fhirpath/2025Jan/#floor--decimal): ✅
    *   [ln()](https://hl7.org/fhirpath/2025Jan/#ln--decimal): ✅
    *   [log()](https://hl7.org/fhirpath/2025Jan/#log-base--decimal--decimal): ✅
    *   [power()](https://hl7.org/fhirpath/2025Jan/#power-exponent--decimal--decimal): ✅
    *   [truncate()](https://hl7.org/fhirpath/2025Jan/#truncate--decimal): ✅
*   [Tree Navigation](https://hl7.org/fhirpath/2025Jan/#tree-navigation)
    *   [children()](https://hl7.org/fhirpath/2025Jan/#children--collection): ✅ (Order not preserved)
    *   [descendants()](https://hl7.org/fhirpath/2025Jan/#descendants--collection): ✅ (Order not preserved) 
    *   [extension()](https://hl7.org/fhirpath/2025Jan/#extensionurl--url-string--collection): ✅ (Supports both object and primitive extensions)
*   [Utility Functions](https://hl7.org/fhirpath/2025Jan/#utility-functions)
    *   [trace()](https://hl7.org/fhirpath/2025Jan/#tracename--string--projection-expression--collection): ✅
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
    *   [`*` (Multiplication)](https://hl7.org/fhirpath/2025Jan/#-multiplication): ✅
    *   [`/` (Division)](https://hl7.org/fhirpath/2025Jan/#-division): ✅
    *   [`+` (Addition)](https://hl7.org/fhirpath/2025Jan/#-addition): ✅ (Numeric, String)
    *   [`-` (Subtraction)](https://hl7.org/fhirpath/2025Jan/#--subtraction): ✅
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

## Architecture

### Overview

This FHIRPath implementation is built using a modular architecture with clear separation of concerns:

- **Parser** (`parser.rs`): Converts FHIRPath expressions into an Abstract Syntax Tree (AST)
- **Evaluator** (`evaluator.rs`): Evaluates AST nodes against FHIR resources with context management
- **Type System** (`fhir_type_hierarchy.rs`): Manages FHIR and System type hierarchies with version-aware resource type checking
- **Function Modules**: Specialized modules for individual FHIRPath functions and operations

### FHIR Version Support

The implementation supports multiple FHIR versions (R4, R4B, R5, R6) through:

- **Feature flags**: Each FHIR version is enabled via Cargo features
- **Version-aware type checking**: Resource type validation uses the appropriate FHIR version's Resource enum
- **Dynamic resource type discovery**: The `FhirResourceTypeProvider` trait automatically extracts resource types from generated Resource enums

### Evaluation Context

The `EvaluationContext` provides the runtime environment for FHIRPath evaluation:

```rust
use fhirpath::evaluator::EvaluationContext;
use fhir::FhirVersion;

// Create context with explicit FHIR version
let context = EvaluationContext::new_empty(FhirVersion::R4);

// Create context with resources (version auto-detected)
let context = EvaluationContext::new(fhir_resources);

// Create context with specific version and resources
let context = EvaluationContext::new_with_version(fhir_resources, FhirVersion::R5);
```

The context includes:
- **FHIR Version**: Used for version-specific type checking and resource validation
- **Resources**: Available FHIR resources for evaluation
- **Variables**: Environment variables (including `$this`, `$index`, `$total`)
- **Configuration**: Strict mode, ordered function checking, etc.

### Type System and Namespace Resolution

The type system handles both FHIR and System namespaces:

#### FHIR Namespace
- **Primitive types**: `boolean`, `string`, `integer`, `decimal`, `date`, `dateTime`, `time`, etc.
- **Complex types**: `Quantity`, `HumanName`, `CodeableConcept`, `Reference`, etc.
- **Resource types**: Version-specific types like `Patient`, `Observation`, `Condition`, etc.

#### System Namespace  
- **Primitive types**: `Boolean`, `String`, `Integer`, `Decimal`, `Date`, `DateTime`, `Time`, `Quantity`

#### Version-Aware Resource Type Checking

```rust
use fhir::FhirVersion;
use fhirpath::fhir_type_hierarchy::{is_fhir_resource_type, determine_type_namespace_with_version};

// Check if "Patient" is a resource type in R4
let is_resource = is_fhir_resource_type("Patient", FhirVersion::R4); // true

// Determine namespace for a type with version context
let namespace = determine_type_namespace_with_version("Patient", FhirVersion::R4); // "FHIR"
```

### Code Generation Integration

The implementation leverages procedural macros to automatically generate type information:

- **FhirPath Macro**: Automatically generates `IntoEvaluationResult` implementations for all FHIR types
- **Resource Type Provider**: Automatically generates `FhirResourceTypeProvider` trait implementations for Resource enums
- **Dynamic Resource Discovery**: Resource type information is extracted at compile time from the actual FHIR specification

This approach ensures that:
- Resource type lists are never hardcoded
- Each FHIR version gets accurate resource type information
- Type information stays in sync with the generated FHIR models

### Function Module Architecture

Each FHIRPath function is implemented in its own module:

- `aggregate_function.rs`: Implementation of `aggregate()` with accumulator support
- `date_arithmetic.rs`: Date/time operations and comparisons  
- `extension_function.rs`: Extension access functions
- `resource_type.rs`: Type checking operations (`is`, `as`, `ofType`)
- `trace_function.rs`: Implementation of `trace()` function

This modular approach enables:
- Clear separation of concerns
- Independent testing of each function
- Easy addition of new functions
- Maintainable code organization

## Performance

Performance results go here...

    


