# helios-fhirpath 

This is an implementation of HL7's [FHIRPath Specification - 3.0.0-ballot](https://hl7.org/fhirpath/2025Jan/) written in Rust.

This implementation is available for testing in Brian Postlethwaite's [FHIRPath Lab](https://fhirpath-lab.com/FhirPath?expression=trace('trc').given.join('%20')%0A.combine(family).join('%2C%20')&engine=Helios%20Software%20(R4B)&context=name&resourceJson=N4KABGBEBOCmDOB7ArtAxrAKgTwA60gC4oAFAQwBcBLWAOwsgBpwoqATIqWADzIFtcAGwLMIkdnWoAzGtE4BtFhFARVUZPALFIG5GUFMlqyBTxawKtcbSI2VWgHMFRqxZeuo8bPAqw+nSAALCgpcQgB6cN9oPntEQUQHbAA6QMEAdmTEaAdwgGFbWABlb18+cIA3ACYAWgAGKrqAZkMPK0gbNnNIAFkAJUh3NQBfIbAAXXdh0XavHz8A1FpCRHZCAEZkquSmgDZk9YAWfYBWAE4TrfXMuoPW9or9ZG71qqbDk-u1SHxoKltOJZXJAfGRoAxtI06ut6id6rtBq5pu5IGR4PAqA5aLA5MQge07PAhGRsAEAIJoPiwMAACVg+gogTQYIIUyMowg4xmqLQ1Aq5go0Ge3No-HMijU+LEGm6iCkMjQVH0X2MUn4VEEpO0eUC+ip0HgKrEDio-NozjakBIsGiRuMACkxYb3JMRjNlCiZYt4HoDO7jCazRaPJB7VR-C72f6pepNAE+GQJOb-WI1bFNQEAOr2NhIOQpqCBujB4HW20FsSOqnO1xclG-f4cPFjSB0JtQKFVRFWDmqXt1sS+YQ2fzECWqGM6OPaQKIKndiDIyUouZlAK4WfYu2QR6CZ4BAAUzQAlGATueTmBdod0i0C1PugB3bIAa230DItDfxHWUaMk9XBZtA3RAt3vXd920I8mlPd51jqM9dnWO9PWnKA+EQAAjDURBRD8v04Ko-2XYFANHKAQLAlEIO6aDTwvS8AA5GPebcvW0eIOHvBsAWbS02wCRojgXPt2RYAdCzbHF430XCxGw8FAgAEUoF4znSQ4aleGoqk+bkAH0FMZFTfEBIxW24XxaAxUCSwnFtUAMGcQjCSI0kybJcikQIqGgcIikFZBeVQWAlNgGRaCoahQPCXBKBoegaiMwJMHDOSQxokysDSgJ1nUzTtN0zAjkIJoTkID5YUIOo6hExcjFdMAlygLoMDRWA2AAIUQeJ6XNYg1UETRuTINg2DgdE7MndioFned71MfAAkwxBGW3XxLICE53jAABROBH03MAArAa00U-CgADUNWERgwD6RNaBWx87uutAICaM4vu3QR7HFFttsOPaDqOgK6ome9FVMAISGEc76GuwRhG3QlBSoXkAge+xnu3UFTO0N7t1wRBQUEAougCT7vu4nFGzMy1QXBXL8q02pdPB3t6s5bkbHoMgMbHf88NgQR4tA+AfNwOySLaKBOnsJxBdl+zle+MiAmCUIIiiHFYloeJEhSdyshyfJChKeZymqepkPWO1LU6boADlwaRMZOTGTnVAk75RXnPiQzTDUtRa5AwB6ME0ECABL+2oH0oOMwD-jLLoGz+rAcdlZjS1HI1lzteNzzwm83z-MC4K4DCiKov+WhwkCZAE1oP3YBqRBH1oGpcDgGRuDj4EaICv5HACS7Ltdnt3YmT2K0LU1iyV2XIE66PaGjuxeVZNpGrdFEh1gEdpZV5f1eAzd0stGiAgAaiaJowAPN50lPL6WKqdJJ697mUVG8aEENMndoM0ghzkvu0Ra3QVprTnpAX6W4l6WkBsDWAh1QLHQYGMH27QoYhytHDeAF1EbI1gajP4AsoBYyeh3OOIIKCqTHujWhxNSbk26FTM4zDaa8TcMrOhYIIRQDyhpVmOlPieymPeBwUlcRQCkH4WSRNuHthzp4ehTNITQi7GyEY4luTNzICaRwAB5HIn4qAAC8xYZ3xDAcKOI6AYACKYhw5irHRXrnbFgoxhhAA&terminologyserver=https%3A%2F%2Fsqlonfhir-r4.azurewebsites.net%2Ffhir).

## Table of Contents
 - [About FHIRPath](#about-fhirpath)
   - [FHIR Specification and Resource Validation](#fhir-specification-and-resource-validation)
   - [FHIR Search Parameter Definitions](#fhir-search-parameter-definitions)
   - [FHIR Implementation Guides](#fhir-implementation-guides)
   - [Clinical Decision Support](#clinical-decision-support)
   - [Terminology Service Integration](#terminology-service-integration)
   - [FHIR Resource Mapping and Transformation](#fhir-resource-mapping-and-transformation)
   - [SQL on FHIR](#sql-on-fhir)
 - [Features Implemented](#features-implemented)
   - [Expressions](#expressions)
   - [Functions](#functions)
   - [Operations](#operations)
   - [Aggregates](#aggregates)
   - [Lexical Elements](#lexical-elements)
   - [Environment Variables](#environment-variables)
   - [Types and Reflection](#types-and-reflection)
   - [Type Safety and Strict Evaluation](#type-safety-and-strict-evaluation)
 - [Architecture](#architecture)
   - [Overview](#overview)
   - [FHIR Version Support](#fhir-version-support)
   - [Evaluation Context](#evaluation-context)
   - [Type System and Namespace Resolution](#type-system-and-namespace-resolution)
   - [Code Generation Integration](#code-generation-integration)
   - [Function Module Architecture](#function-module-architecture)
 - [Executables](#executables)
   - [`fhirpath-cli` - Command Line Interface](#fhirpath-cli---command-line-interface)
   - [`fhirpath-server` - HTTP Server](#fhirpath-server---http-server)
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

FHIRPath provides access to terminology services through a %terminologies object. This implementation supports all standard terminology operations.

**⚠️ IMPORTANT: Default Terminology Servers**
By default, this implementation uses test terminology servers:
- **R4/R4B**: `https://tx.fhir.org/r4/`
- **R5**: `https://tx.fhir.org/r5/`

**DO NOT USE THESE DEFAULT SERVERS IN PRODUCTION!** They are test servers with limited resources and no SLA.

**Configuring a Terminology Server:**
```bash
# Via environment variable
export FHIRPATH_TERMINOLOGY_SERVER=https://your-terminology-server.com/fhir

# Via CLI option
fhirpath-cli --terminology-server https://your-terminology-server.com/fhir ...

# Via server option
fhirpath-server --terminology-server https://your-terminology-server.com/fhir
```

**Supported %terminologies Functions:**
```fhirpath
# Expand a ValueSet
%terminologies.expand('http://hl7.org/fhir/ValueSet/administrative-gender')

# Lookup code details
%terminologies.lookup(Observation.code.coding.first())

# Validate against ValueSet
%terminologies.validateVS('http://hl7.org/fhir/ValueSet/observation-vitalsignresult', Observation.code.coding.first())

# Validate against CodeSystem
%terminologies.validateCS('http://loinc.org', Observation.code.coding.first())

# Check code subsumption
%terminologies.subsumes('http://snomed.info/sct', '73211009', '5935008')

# Translate using ConceptMap
%terminologies.translate('http://hl7.org/fhir/ConceptMap/cm-address-use-v2', Patient.address.use)
```

**memberOf Function:**
```fhirpath
# Check if a coding is member of a ValueSet
Observation.code.coding.where(memberOf('http://hl7.org/fhir/ValueSet/observation-vitalsignresult'))
```

**Example with Parameters:**
```fhirpath
# Expand with count limit
%terminologies.expand('http://hl7.org/fhir/ValueSet/languages', {'count': '10'})

# Validate with language parameter
%terminologies.validateVS('http://hl7.org/fhir/ValueSet/condition-clinical', 
                         Condition.clinicalStatus.coding.first(), 
                         {'language': 'es'})
```

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
    *   [Long](https://hl7.org/fhirpath/2025Jan/#long) (STU): 🟡 (Parser support, runtime implementation gaps)
    *   [Decimal](https://hl7.org/fhirpath/2025Jan/#decimal): ✅
    *   [Date](https://hl7.org/fhirpath/2025Jan/#date): ✅ (Full parsing and arithmetic support)
    *   [Time](https://hl7.org/fhirpath/2025Jan/#time): ✅ (Full parsing and comparison support)
    *   [DateTime](https://hl7.org/fhirpath/2025Jan/#datetime): ✅ (Full parsing, timezone and arithmetic support)
    *   [Quantity](https://hl7.org/fhirpath/2025Jan/#quantity): 🟡 (Basic value/unit storage, limited unit conversion)
        *   [Time-valued Quantities](https://hl7.org/fhirpath/2025Jan/#time-valued-quantities): 🟡 (Keywords parsed, conversion implementation needed)
    
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
    *   [distinct()](https://hl7.org/fhirpath/2025Jan/#distinct--collection): ✅
    *   [isDistinct()](https://hl7.org/fhirpath/2025Jan/#isdistinct--boolean): ✅
*   [Filtering and Projection](https://hl7.org/fhirpath/2025Jan/#filtering-and-projection)
    *   [where()](https://hl7.org/fhirpath/2025Jan/#wherecriteria--expression--collection): ✅
    *   [select()](https://hl7.org/fhirpath/2025Jan/#selectprojection-expression--collection): ✅
    *   [repeat()](https://hl7.org/fhirpath/2025Jan/#repeatprojection-expression--collection): ✅ (With cycle detection)
    *   [ofType()](https://hl7.org/fhirpath/2025Jan/#oftypetype--type-specifier--collection): ✅ (Full namespace qualification support)
*   [Subsetting](https://hl7.org/fhirpath/2025Jan/#subsetting)
    *   [Indexer `[]`](https://hl7.org/fhirpath/2025Jan/#-index--integer---collection): ✅
    *   [single()](https://hl7.org/fhirpath/2025Jan/#single--collection): ✅
    *   [first()](https://hl7.org/fhirpath/2025Jan/#first--collection): ✅
    *   [last()](https://hl7.org/fhirpath/2025Jan/#last--collection): ✅
    *   [tail()](https://hl7.org/fhirpath/2025Jan/#tail--collection): ✅
    *   [skip()](https://hl7.org/fhirpath/2025Jan/#skipnum--integer--collection): ✅
    *   [take()](https://hl7.org/fhirpath/2025Jan/#takenum--integer--collection): ✅
    *   [intersect()](https://hl7.org/fhirpath/2025Jan/#intersectother-collection--collection): ✅
    *   [exclude()](https://hl7.org/fhirpath/2025Jan/#excludeother-collection--collection): ✅
*   [Combining](https://hl7.org/fhirpath/2025Jan/#combining)
    *   [union()](https://hl7.org/fhirpath/2025Jan/#unionother--collection): ✅
    *   [combine()](https://hl7.org/fhirpath/2025Jan/#combineother--collection--collection): ✅
*   [Conversion](https://hl7.org/fhirpath/2025Jan/#conversion)
    *   [Implicit Conversions](https://hl7.org/fhirpath/2025Jan/#conversion): ✅ (Integer/Decimal)
    *   [iif()](https://hl7.org/fhirpath/2025Jan/#iifcriterion-expression-true-result-collection--otherwise-result-collection--collection): ✅
    *   [toBoolean()](https://hl7.org/fhirpath/2025Jan/#toboolean--boolean): ✅
    *   [convertsToBoolean()](https://hl7.org/fhirpath/2025Jan/#convertstoboolean--boolean): ✅
    *   [toInteger()](https://hl7.org/fhirpath/2025Jan/#tointeger--integer): ✅
    *   [convertsToInteger()](https://hl7.org/fhirpath/2025Jan/#convertstointeger--boolean): ✅
    *   [toLong()](https://hl7.org/fhirpath/2025Jan/#tolong--long) (STU): ✅
    *   [convertsToLong()](https://hl7.org/fhirpath/2025Jan/#convertstolong--boolean) (STU): ✅
    *   [toDate()](https://hl7.org/fhirpath/2025Jan/#todate--date): ✅
    *   [convertsToDate()](https://hl7.org/fhirpath/2025Jan/#convertstodate--boolean): ✅
    *   [toDateTime()](https://hl7.org/fhirpath/2025Jan/#todatetime--datetime): ✅
    *   [convertsToDateTime()](https://hl7.org/fhirpath/2025Jan/#convertstodatetime--boolean): ✅
    *   [toDecimal()](https://hl7.org/fhirpath/2025Jan/#todecimal--decimal): ✅
    *   [convertsToDecimal()](https://hl7.org/fhirpath/2025Jan/#convertstodecimal--boolean): ✅
    *   [toQuantity()](https://hl7.org/fhirpath/2025Jan/#toquantityunit--string--quantity): 🟡 (Basic types, no unit conversion)
    *   [convertsToQuantity()](https://hl7.org/fhirpath/2025Jan/#convertstoquantityunit--string--boolean): 🟡 (Basic types, no unit conversion)
    *   [toString()](https://hl7.org/fhirpath/2025Jan/#tostring--string): ✅
    *   [convertsToString()](https://hl7.org/fhirpath/2025Jan/#convertstostring--string): ✅
    *   [toTime()](https://hl7.org/fhirpath/2025Jan/#totime--time): ✅
    *   [convertsToTime()](https://hl7.org/fhirpath/2025Jan/#convertstotime--boolean): ✅
*   [String Manipulation](https://hl7.org/fhirpath/2025Jan/#string-manipulation)
    *   [indexOf()](https://hl7.org/fhirpath/2025Jan/#indexofsubstring--string--integer): ✅
    *   [lastIndexOf()](https://hl7.org/fhirpath/2025Jan/#lastindexofsubstring--string--integer) (STU): ✅
    *   [substring()](https://hl7.org/fhirpath/2025Jan/#substringstart--integer--length--integer--string): ✅
    *   [startsWith()](https://hl7.org/fhirpath/2025Jan/#startswithprefix--string--boolean): ✅
    *   [endsWith()](https://hl7.org/fhirpath/2025Jan/#endswithsuffix--string--boolean): ✅
    *   [contains()](https://hl7.org/fhirpath/2025Jan/#containssubstring--string--boolean): ✅
    *   [upper()](https://hl7.org/fhirpath/2025Jan/#upper--string): ✅
    *   [lower()](https://hl7.org/fhirpath/2025Jan/#lower--string): ✅
    *   [replace()](https://hl7.org/fhirpath/2025Jan/#replacepattern--string-substitution--string--string): ✅
    *   [matches()](https://hl7.org/fhirpath/2025Jan/#matchesregex--string--boolean): ✅
    *   [matchesFull()](https://hl7.org/fhirpath/2025Jan/#matchesfullregex--string--boolean) (STU): ✅
    *   [replaceMatches()](https://hl7.org/fhirpath/2025Jan/#replacematchesregex--string-substitution-string--string): ✅
    *   [length()](https://hl7.org/fhirpath/2025Jan/#length--integer): ✅
    *   [toChars()](https://hl7.org/fhirpath/2025Jan/#tochars--collection): ✅
    *   [encode()](https://hl7.org/fhirpath/2025Jan/#encodeformat--string--string): ✅
    *   [decode()](https://hl7.org/fhirpath/2025Jan/#decodeformat--string--string): ✅
*   [Additional String Functions](https://hl7.org/fhirpath/2025Jan/#additional-string-functions) (STU): ✅
    *   [escape()](https://hl7.org/fhirpath/2025Jan/#escapetarget--string--string): ✅ (html, json targets)
    *   [unescape()](https://hl7.org/fhirpath/2025Jan/#unescapetarget--string--string): ✅ (html, json targets)
    *   [split()](https://hl7.org/fhirpath/2025Jan/#splitseparator--string--collection): ✅
    *   [trim()](https://hl7.org/fhirpath/2025Jan/#trim--string): ✅
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
    *   [children()](https://hl7.org/fhirpath/2025Jan/#children--collection): ✅
    *   [descendants()](https://hl7.org/fhirpath/2025Jan/#descendants--collection): ✅ 
    *   [extension()](https://hl7.org/fhirpath/2025Jan/#extensionurl--url-string--collection): ✅ (Full support for object and primitive extensions with variable resolution)
*   [Utility Functions](https://hl7.org/fhirpath/2025Jan/#utility-functions)
    *   [trace()](https://hl7.org/fhirpath/2025Jan/#tracename--string--projection-expression--collection): ✅ (With projection support)
    *   [now()](https://hl7.org/fhirpath/2025Jan/#now--datetime): ✅
    *   [timeOfDay()](https://hl7.org/fhirpath/2025Jan/#timeofday--time): ✅
    *   [today()](https://hl7.org/fhirpath/2025Jan/#today--date): ✅
    *   [defineVariable()](https://hl7.org/fhirpath/2025Jan/#definevariablename-string--expr-expression) (STU): ✅
    *   [lowBoundary()](https://hl7.org/fhirpath/2025Jan/#lowboundaryprecision-integer-decimal--date--datetime--time) (STU): ✅ (Full support for Decimal, Date, DateTime, and Time)
    *   [highBoundary()](https://hl7.org/fhirpath/2025Jan/#highboundaryprecision-integer-decimal--date--datetime--time) (STU): ✅ (Full support for Decimal, Date, DateTime, and Time)
    *   [precision()](https://hl7.org/fhirpath/2025Jan/#precision--integer) (STU): ✅ (See [limitation for decimal trailing zeros](PRECISION_LIMITATION.md))
*   [Date/DateTime/Time Component Extraction](https://hl7.org/fhirpath/2025Jan/#extract-datedatetimetime-components) (STU): ✅ (All component functions implemented: yearOf, monthOf, dayOf, hourOf, minuteOf, secondOf, millisecondOf)
    
### [Operations](https://hl7.org/fhirpath/2025Jan/#operations)
    
*   [Equality](https://hl7.org/fhirpath/2025Jan/#equality)
    *   [`=` (Equals)](https://hl7.org/fhirpath/2025Jan/#-equals): ✅ (Full support for all types including dates and quantities)
    *   [`~` (Equivalent)](https://hl7.org/fhirpath/2025Jan/#-equivalent): ✅ (Full equivalence checking)
    *   [`!=` (Not Equals)](https://hl7.org/fhirpath/2025Jan/#-not-equals): ✅
    *   [`!~` (Not Equivalent)](https://hl7.org/fhirpath/2025Jan/#-not-equivalent): ✅
*   [Comparison](https://hl7.org/fhirpath/2025Jan/#comparison)
    *   [`>` (Greater Than)](https://hl7.org/fhirpath/2025Jan/#-greater-than): ✅ (Full support including dates and numeric types)
    *   [`<` (Less Than)](https://hl7.org/fhirpath/2025Jan/#-less-than): ✅ (Full support including dates and numeric types)
    *   [`<=` (Less or Equal)](https://hl7.org/fhirpath/2025Jan/#-less-or-equal): ✅ (Full support including dates and numeric types)
    *   [`>=` (Greater or Equal)](https://hl7.org/fhirpath/2025Jan/#-greater-or-equal): ✅ (Full support including dates and numeric types)
*   [Types](https://hl7.org/fhirpath/2025Jan/#types)
    *   [`is`](https://hl7.org/fhirpath/2025Jan/#is-type-specifier): ✅ (Full namespace qualification and FHIR type hierarchy support)
    *   [`as`](https://hl7.org/fhirpath/2025Jan/#as-type-specifier): ✅ (Full namespace qualification and type casting support)
*   [Collections](https://hl7.org/fhirpath/2025Jan/#collections-1)
    *   [`|` (Union)](https://hl7.org/fhirpath/2025Jan/#-union-collections): ✅
    *   [`in` (Membership)](https://hl7.org/fhirpath/2025Jan/#in-membership): ✅
    *   [`contains` (Containership)](https://hl7.org/fhirpath/2025Jan/#contains-containership): ✅
    *   [Collection Navigation](https://hl7.org/fhirpath/2025Jan/#path-selection): ✅ (Full polymorphic access and choice element support)
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
*   [Date/Time Arithmetic](https://hl7.org/fhirpath/2025Jan/#datetime-arithmetic): ✅ (Full arithmetic support with timezone and precision handling)
*   [Operator Precedence](https://hl7.org/fhirpath/2025Jan/#operator-precedence): ✅
    
### [Aggregates](https://hl7.org/fhirpath/2025Jan/#aggregates)
    
*   [aggregate()](https://hl7.org/fhirpath/2025Jan/#aggregateaggregator--expression--init--value--value) (STU): ✅ (Full accumulator support)

### [Lexical Elements](https://hl7.org/fhirpath/2025Jan/#lexical-elements)

*   [Lexical Elements](https://hl7.org/fhirpath/2025Jan/#lexical-elements): ✅ (Handled by parser)
*   [Comments](https://hl7.org/fhirpath/2025Jan/#comments): ✅ (Both single-line `//` and multi-line `/* */` comments)
    
### [Environment Variables](https://hl7.org/fhirpath/2025Jan/#environment-variables)
    
*   [`%variable`](https://hl7.org/fhirpath/2025Jan/#environment-variables): ✅ (Full variable resolution including built-in constants)
*   [`%context`](https://hl7.org/fhirpath/2025Jan/#environment-variables): ✅ (Full context support with $this, $index, $total)
    
### [Types and Reflection](https://hl7.org/fhirpath/2025Jan/#types-and-reflection)
    
*   [Models](https://hl7.org/fhirpath/2025Jan/#models): ✅ (Full namespace qualification and FHIR type hierarchy support)
*   [Reflection (`type()`)](https://hl7.org/fhirpath/2025Jan/#reflection) (STU): ✅ (Enhanced with namespace support and type hierarchy)
    
### [Type Safety and Strict Evaluation](https://hl7.org/fhirpath/2025Jan/#type-safety-and-strict-evaluation)
    
*   [Type Safety / Strict Evaluation](https://hl7.org/fhirpath/2025Jan/#type-safety-and-strict-evaluation): ✅ (Configurable strict mode with proper error handling)

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
use helios_fhirpath::evaluator::EvaluationContext;
use helios_fhir::FhirVersion;

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
- **Variable Scoping**: Parent context support for proper variable scoping in functions like `select()` and `where()`

### Type System and Namespace Resolution

The type system handles both FHIR and System namespaces:

#### FHIR Namespace
- **Primitive types**: `boolean`, `string`, `integer`, `decimal`, `date`, `dateTime`, `time`, etc.
- **Complex types**: `Quantity`, `HumanName`, `CodeableConcept`, `Reference`, etc.
- **Resource types**: Version-specific types like `Patient`, `Observation`, `Condition`, etc.

#### System Namespace  
- **Primitive types**: `Boolean`, `String`, `Integer`, `Decimal`, `Date`, `DateTime`, `Time`, `Quantity`

#### Version-Aware Resource Type Checking

The implementation uses the `FhirResourceTypeProvider` trait to automatically detect resource types for each FHIR version:

```rust
use helios_fhir::FhirVersion;
use helios_fhirpath::evaluator::EvaluationContext;

// Context automatically detects FHIR version from resources
let context = EvaluationContext::new(resources);

// Or specify version explicitly
let context = EvaluationContext::new_with_version(resources, FhirVersion::R4);
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

Each FHIRPath function category is implemented in its own module:

- `aggregate_function.rs`: Implementation of `aggregate()` with accumulator support
- `boolean_functions.rs`: Boolean logic functions (`allTrue`, `anyFalse`, etc.)
- `collection_functions.rs`: Collection manipulation (`where`, `select`, `count`, etc.)
- `collection_navigation.rs`: Navigation functions (`children`, `descendants`)
- `conversion_functions.rs`: Type conversion functions (`toInteger`, `toString`, etc.)
- `date_operation.rs`: Date/time operations and arithmetic
- `extension_function.rs`: FHIR extension access functions
- `polymorphic_access.rs`: Choice element and polymorphic type operations
- `repeat_function.rs`: Implementation of `repeat()` with cycle detection
- `resource_type.rs`: Type checking operations (`is`, `as`, `ofType`)
- `trace_function.rs`: Implementation of `trace()` with projection support
- `type_function.rs`: Type reflection and `type()` function

This modular approach enables:
- Clear separation of concerns by function category
- Independent testing of each function group
- Easy addition of new functions
- Maintainable and organized code structure

## Executables

This crate provides two executable targets for FHIRPath expression evaluation:

### `fhirpath-cli` - Command Line Interface

A feature-rich command-line tool for evaluating FHIRPath expressions against FHIR resources.

#### Installation

```bash
# Install from the workspace root
cargo install --path crates/helios-fhirpath --bin fhirpath-cli

# Or build directly
cargo build --release --bin fhirpath-cli
```

#### Features

- **Expression Evaluation**: Execute FHIRPath expressions against FHIR resources
- **Context Support**: Evaluate expressions with context for scoped evaluation
- **Variables**: Define variables via command line or JSON file
- **Parse Debug**: Generate AST visualizations for expression analysis
- **FHIR Version Support**: Handle resources from any supported FHIR version
- **JSON Output**: Results formatted as JSON for easy processing

#### Command Line Options

```text
-e, --expression <EXPRESSION>      FHIRPath expression to evaluate
-c, --context <CONTEXT>           Context expression to evaluate first
-r, --resource <RESOURCE>         Path to FHIR resource JSON file (use '-' for stdin)
-v, --variables <VARIABLES>       Path to variables JSON file
    --var <KEY=VALUE>            Set a variable directly
-o, --output <OUTPUT>            Output file path (defaults to stdout)
    --parse-debug-tree           Output parse debug tree as JSON
    --parse-debug                Output parse debug info
    --trace                      Enable trace output
    --fhir-version <VERSION>     FHIR version [default: R4]
    --validate                   Validate expression before execution
    --terminology-server <URL>   Terminology server URL
-h, --help                       Print help
```

#### Usage Examples

##### Basic Expression Evaluation
```bash
# Evaluate expression against a resource
fhirpath-cli -e "Patient.name.family" -r patient.json

# Get first given name
fhirpath-cli -e "Patient.name.given.first()" -r patient.json

# Filter telecom by system
fhirpath-cli -e "Patient.telecom.where(system = 'email')" -r patient.json
```

##### Using Context Expressions
```bash
# Evaluate with context
fhirpath-cli -c "Patient.name" -e "given.join(' ')" -r patient.json

# Context with filtering
fhirpath-cli -c "Patient.telecom.where(system = 'phone')" -e "value" -r patient.json
```

##### Working with Variables
```bash
# Variable from command line
fhirpath-cli -e "value > %threshold" -r observation.json --var threshold=5.0

# Multiple variables
fhirpath-cli -e "%system = 'phone' and use = %use" -r patient.json \
  --var system=phone --var use=mobile

# Variables from JSON file
cat > vars.json << EOF
{
  "threshold": 140,
  "unit": "mm[Hg]"
}
EOF
fhirpath-cli -e "value.value > %threshold and value.unit = %unit" \
  -r observation.json -v vars.json
```

##### Parse Debug Features
```bash
# Generate parse debug tree (JSON format)
fhirpath-cli -e "Patient.name.where(use = 'official').given.first()" \
  --parse-debug-tree

# Generate parse debug text
fhirpath-cli -e "Patient.name.given.first() | Patient.name.family" \
  --parse-debug
```

##### Using stdin
```bash
# Resource from stdin
cat patient.json | fhirpath-cli -e "Patient.name.family" -r -

# Pipe from other commands
curl -s https://example.com/fhir/Patient/123 | \
  fhirpath-cli -e "name.family" -r -
```

##### Output Options
```bash
# Output to file
fhirpath-cli -e "Patient.name" -r patient.json -o names.json

# Pretty printed JSON output (default)
fhirpath-cli -e "Patient.identifier" -r patient.json
```

### `fhirpath-server` - HTTP Server

An HTTP server providing FHIRPath expression evaluation via a REST API, compatible with [fhirpath-lab](https://fhirpath-lab.com/). 

#### Installation

```bash
# Install from the workspace root
cargo install --path crates/helios-fhirpath --bin fhirpath-server

# Or build directly
cargo build --release --bin fhirpath-server
```

#### Features

- **FHIRPath Evaluation API**: POST endpoint accepting FHIR Parameters resources
- **Parse Debug Tree**: Generate and return AST visualizations
- **Variable Support**: Pass variables to expressions via Parameters
- **Context Expressions**: Support for context-based evaluation
- **CORS Configuration**: Flexible cross-origin resource sharing
- **Health Check**: Simple health status endpoint
- **fhirpath-lab Compatible**: Full compatibility with the fhirpath-lab tool

#### Configuration

The server can be configured via command-line arguments or environment variables:

| Environment Variable | CLI Argument | Description | Default |
|---------------------|--------------|-------------|---------|
| `FHIRPATH_SERVER_PORT` | `--port` | Server port | `3000` |
| `FHIRPATH_SERVER_HOST` | `--host` | Server host | `127.0.0.1` |
| `FHIRPATH_LOG_LEVEL` | `--log-level` | Log level (error/warn/info/debug/trace) | `info` |
| `FHIRPATH_ENABLE_CORS` | `--enable-cors` | Enable CORS | `true` |
| `FHIRPATH_CORS_ORIGINS` | `--cors-origins` | Allowed origins (comma-separated) | `*` |
| `FHIRPATH_CORS_METHODS` | `--cors-methods` | Allowed methods | `GET,POST,OPTIONS` |
| `FHIRPATH_CORS_HEADERS` | `--cors-headers` | Allowed headers | Common headers |

#### Starting the Server

```bash
# Start with defaults
fhirpath-server

# Custom port and host
fhirpath-server --port 8080 --host 0.0.0.0

# With environment variables
FHIRPATH_SERVER_PORT=8080 FHIRPATH_LOG_LEVEL=debug fhirpath-server

# Production configuration
fhirpath-server \
  --host 0.0.0.0 \
  --port 8080 \
  --log-level warn \
  --cors-origins "https://fhirpath-lab.com,https://dev.fhirpath-lab.com,https://fhirpath-lab.azurewebsites.net,https://fhirpath-lab-dev.azurewebsites.net/,http://localhost:3000"
```

#### API Endpoints

##### POST / - Evaluate FHIRPath Expression

Accepts a FHIR Parameters resource and returns evaluation results. Auto-detects the FHIR version from the resource.

##### POST /r4, /r4b, /r5, /r6 - Version-Specific Evaluation

Forces evaluation with a specific FHIR version (if compiled with the corresponding feature). Useful when you want to ensure your resource is processed with a specific FHIR version, overriding auto-detection.

**Request Body** (FHIR Parameters):
```json
{
  "resourceType": "Parameters",
  "parameter": [
    {
      "name": "expression",
      "valueString": "Patient.name.given.first()"
    },
    {
      "name": "resource",
      "resource": {
        "resourceType": "Patient",
        "name": [{
          "given": ["John", "James"],
          "family": "Doe"
        }]
      }
    }
  ]
}
```

**Response** (FHIR Parameters):
```json
{
  "resourceType": "Parameters",
  "id": "fhirpath",
  "parameter": [
    {
      "name": "parameters",
      "part": [
        {
          "name": "evaluator",
          "valueString": "Helios FHIRPath-0.1.0"
        },
        {
          "name": "expression",
          "valueString": "Patient.name.given.first()"
        },
        {
          "name": "resource",
          "resource": { "...": "..." }
        }
      ]
    },
    {
      "name": "result",
      "valueString": "Resource",
      "part": [
        {
          "name": "string",
          "valueString": "John"
        }
      ]
    }
  ]
}
```

**Supported Input Parameters**:
- `expression` (required): FHIRPath expression to evaluate
- `context` (optional): Context expression to evaluate first
- `resource` (required): FHIR resource to evaluate against
- `validate` (optional): Whether to validate the expression
- `variables` (optional): Variables to pass to the expression
- `terminologyServer` (optional): Terminology server URL

**Additional Output Parameters** (when `validate` is true):
- `parseDebugTree`: JSON representation of the expression AST
- `parseDebug`: Text representation of the parse tree
- `expectedReturnType`: Expected return type of the expression

##### GET /health - Health Check

Returns server health status.

```bash
curl http://localhost:3000/health
```

Response:
```json
{
  "status": "ok",
  "service": "fhirpath-server"
}
```

#### Usage Examples

##### Basic Evaluation
```bash
# Auto-detect FHIR version
curl -X POST http://localhost:3000 \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {
        "name": "expression",
        "valueString": "Patient.birthDate"
      },
      {
        "name": "resource",
        "resource": {
          "resourceType": "Patient",
          "birthDate": "1974-12-25"
        }
      }
    ]
  }'

# Force R4 processing
curl -X POST http://localhost:3000/r4 \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {
        "name": "expression",
        "valueString": "Patient.birthDate"
      },
      {
        "name": "resource",
        "resource": {
          "resourceType": "Patient",
          "birthDate": "1974-12-25"
        }
      }
    ]
  }'
```

##### With Context and Variables
```bash
curl -X POST http://localhost:3000 \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {
        "name": "context",
        "valueString": "Observation.component"
      },
      {
        "name": "expression",
        "valueString": "value > %threshold"
      },
      {
        "name": "variables",
        "part": [
          {
            "name": "threshold",
            "valueString": "140"
          }
        ]
      },
      {
        "name": "resource",
        "resource": {
          "resourceType": "Observation",
          "component": [
            {"valueQuantity": {"value": 150}},
            {"valueQuantity": {"value": 130}}
          ]
        }
      }
    ]
  }'
```

##### With Parse Debug
```bash
curl -X POST http://localhost:3000 \
  -H "Content-Type: application/json" \
  -d '{
    "resourceType": "Parameters",
    "parameter": [
      {
        "name": "expression",
        "valueString": "Patient.name.given.first() | Patient.name.family"
      },
      {
        "name": "validate",
        "valueBoolean": true
      },
      {
        "name": "resource",
        "resource": {
          "resourceType": "Patient",
          "name": [{"given": ["John"], "family": "Doe"}]
        }
      }
    ]
  }'
```

#### Integration with fhirpath-lab

The server is compatible with [fhirpath-lab](https://fhirpath-lab.com/). To use your local server with fhirpath-lab:

1. Start the server with CORS enabled for fhirpath-lab domains:
   ```bash
   fhirpath-server --cors-origins "https://fhirpath-lab.com,http://localhost:3000"
   ```

2. In fhirpath-lab, configure the custom server URL to point to your local instance

3. The server will properly handle all fhirpath-lab requests including parse debug tree generation

## Performance

This implementation is designed for high performance FHIRPath expression evaluation. We use [Criterion.rs](https://github.com/bheisler/criterion.rs) for comprehensive performance benchmarking across all major components.

### Running Benchmarks

To run all benchmarks:
```bash
cargo bench
```

To run specific benchmark suites:
```bash
# Parser benchmarks only
cargo bench --bench parser_benches

# Evaluator benchmarks only  
cargo bench --bench evaluator_benches

# CLI benchmarks only
cargo bench --bench cli_benches

# Server benchmarks only
cargo bench --bench server_benches
```

Benchmark results are saved in `target/criterion/` with HTML reports for detailed analysis.

### Benchmark Categories

#### Parser Benchmarks (`parser_benches`)
- **Simple expressions**: Basic paths, literals, and indexed access
- **Function calls**: Single functions, chained functions, nested calls
- **Operators**: Arithmetic, comparison, boolean logic, unions
- **Complex expressions**: Filters, type checking, extensions, aggregates
- **Large expressions**: Many conditions, deep nesting, multiple functions

#### Evaluator Benchmarks (`evaluator_benches`)
- **Navigation**: Simple and nested field access, indexing
- **Collections**: where(), select(), exists(), count(), distinct()
- **String operations**: Concatenation, upper/lower, substring, regex
- **Type operations**: is(), ofType(), as(), type reflection
- **Date/time**: Comparisons, today(), now(), arithmetic
- **Extensions**: URL-based access, typed values
- **Complex expressions**: Multi-step filters, unions, quantity comparisons

#### CLI Benchmarks (`cli_benches`)
- **Simple expressions**: Basic navigation with and without functions
- **Context expressions**: Simple and complex context evaluation
- **Variables**: Inline variables and file-based variables
- **Bundle operations**: Filtering and aggregation on bundles
- **Debug features**: Parse tree generation and validation

#### Server Benchmarks (`server_benches`)
- **Simple requests**: Basic expressions and health checks
- **Complex requests**: Filters, context, and variables
- **Validation**: Parse debug tree generation
- **Large resources**: Bundle processing with many entries
- **Concurrent requests**: Parallel request handling

### Performance Results

The following results are from a typical development machine (results will vary based on hardware):

| Operation | Time (avg) | Description |
|-----------|------------|-------------|
| **Parser** | | |
| Simple path | ~500 ns | `Patient.name` |
| Nested path | ~800 ns | `Patient.name.family` |
| Function call | ~1.2 μs | `Patient.name.first()` |
| Complex filter | ~3.5 μs | `Patient.telecom.where(system = 'phone')` |
| **Evaluator** | | |
| Field access | ~1.5 μs | Navigate to single field |
| Collection filter | ~4.2 μs | Filter telecom by system |
| String operation | ~2.8 μs | Upper case conversion |
| Type check | ~1.8 μs | Check resource type |
| **CLI** | | |
| Simple expression | ~150 μs | Full CLI execution |
| With variables | ~180 μs | Variable resolution |
| Bundle processing | ~450 μs | Process 10 resources |
| **Server** | | |
| Simple request | ~350 μs | Basic expression evaluation |
| Complex request | ~500 μs | With filtering and context |
| Large bundle | ~2.5 ms | Process 50 resources |

### Performance Optimization Tips

1. **Expression Optimization**
   - Use specific paths instead of wildcards when possible
   - Filter early in the expression chain
   - Avoid redundant type checks

2. **Resource Optimization**
   - Keep resource sizes reasonable
   - Use appropriate FHIR version features
   - Consider using context expressions for repeated navigation

3. **Server Optimization**
   - Enable connection pooling for high load
   - Use appropriate log levels in production
   - Configure CORS for specific origins only

4. **Memory Usage**
   - The evaluator uses streaming where possible
   - Large collections are processed lazily
   - Recursive expressions have cycle detection

### Benchmark Methodology

Our benchmarks follow these principles:
- Use realistic FHIR resources and expressions
- Cover both simple and complex scenarios
- Measure end-to-end performance for CLI/server
- Include memory allocation patterns
- Test with various resource sizes
- Verify correctness alongside performance

The benchmark suite is continuously expanded to cover new features and edge cases.

    


