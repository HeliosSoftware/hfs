# Appendix B — FHIRPath Function Reference

Implementation status for [FHIRPath 3.0.0-ballot](https://hl7.org/fhirpath/2025Jan/) in `helios-fhirpath`.

**Legend:**
- ✅ Fully implemented
- 🟡 Partially implemented (known limitations noted)
- ❌ Not implemented
- 🚧 In progress
- *(STU)* — Standard for Trial Use in the specification

---

## Literals

| Literal | Status | Notes |
|---------|--------|-------|
| Boolean | ✅ | |
| String | ✅ | |
| Integer | ✅ | |
| Long *(STU)* | 🟡 | Parser support; runtime implementation gaps |
| Decimal | ✅ | |
| Date | ✅ | Full parsing and arithmetic support |
| Time | ✅ | Full parsing and comparison support |
| DateTime | ✅ | Full parsing, timezone and arithmetic support |
| Quantity | 🟡 | Basic value/unit storage; limited unit conversion |
| Time-valued Quantities | 🟡 | Keywords parsed; conversion implementation needed |

---

## Existence Functions

| Function | Status |
|----------|--------|
| `empty()` | ✅ |
| `exists([criteria])` | ✅ |
| `all(criteria)` | ✅ |
| `allTrue()` | ✅ |
| `anyTrue()` | ✅ |
| `allFalse()` | ✅ |
| `anyFalse()` | ✅ |
| `subsetOf(other)` | ✅ |
| `supersetOf(other)` | ✅ |
| `count()` | ✅ |
| `distinct()` | ✅ |
| `isDistinct()` | ✅ |

---

## Filtering and Projection

| Function | Status | Notes |
|----------|--------|-------|
| `where(criteria)` | ✅ | |
| `select(projection)` | ✅ | |
| `sort([keySelector, asc\|desc])` *(STU)* | ✅ | Sort with optional key selector |
| `repeat(projection)` | ✅ | Cycle detection included |
| `repeatAll(projection)` *(STU)* | ✅ | |
| `ofType(type)` | ✅ | Full namespace qualification support |
| `coalesce(value, ...)` *(STU)* | ✅ | |

---

## Subsetting

| Function | Status |
|----------|--------|
| `[index]` indexer | ✅ |
| `single()` | ✅ |
| `first()` | ✅ |
| `last()` | ✅ |
| `tail()` | ✅ |
| `skip(num)` | ✅ |
| `take(num)` | ✅ |
| `intersect(other)` | ✅ |
| `exclude(other)` | ✅ |

---

## Combining

| Function | Status | Notes |
|----------|--------|-------|
| `union(other)` | ✅ | |
| `combine(other[, preserveOrder])` | ✅ | Optional `preserveOrder` parameter supported |

---

## Conversion Functions

| Function | Status | Notes |
|----------|--------|-------|
| Implicit conversions (Integer → Decimal) | ✅ | |
| `iif(criterion, trueResult[, otherwise])` | ✅ | |
| `toBoolean()` / `convertsToBoolean()` | ✅ | |
| `toInteger()` / `convertsToInteger()` | ✅ | |
| `toLong()` / `convertsToLong()` *(STU)* | ✅ | |
| `toDate([format])` / `convertsToDate()` | ✅ | Optional `format` parameter |
| `toDateTime([format])` / `convertsToDateTime()` | ✅ | Optional `format` parameter |
| `toDecimal()` / `convertsToDecimal()` | ✅ | |
| `toQuantity([unit])` / `convertsToQuantity([unit])` | 🟡 | Basic types; no unit conversion |
| `toString([format])` / `convertsToString()` | ✅ | Optional `format` parameter |
| `toTime()` / `convertsToTime()` | ✅ | |
| Date/DateTime/Time format codes (`yyyy`, `MM`, `dd`, ...) *(STU)* | ✅ | |

---

## String Manipulation

| Function | Status | Notes |
|----------|--------|-------|
| `indexOf(substring)` | ✅ | |
| `lastIndexOf(substring)` *(STU)* | ✅ | |
| `substring(start[, length])` | ✅ | |
| `startsWith(prefix)` | ✅ | |
| `endsWith(suffix)` | ✅ | |
| `contains(substring)` | ✅ | |
| `upper()` | ✅ | |
| `lower()` | ✅ | |
| `replace(pattern, substitution)` | ✅ | |
| `matches(regex[, flags])` | ✅ | Flags: `s`, `m`, `i`, `x` |
| `matchesFull(regex[, flags])` *(STU)* | ✅ | |
| `replaceMatches(regex, substitution[, flags])` | ✅ | |
| `length()` | ✅ | |
| `toChars()` | ✅ | |
| `encode(format)` | ✅ | |
| `decode(format)` | ✅ | |
| `escape(target)` *(STU)* | ✅ | `html`, `json` targets |
| `unescape(target)` *(STU)* | ✅ | `html`, `json` targets |
| `split(separator)` | ✅ | |
| `join([separator])` | ✅ | |
| `trim()` | ✅ | |

---

## Math Functions *(STU)*

| Function | Status |
|----------|--------|
| `round([precision])` | ✅ |
| `sqrt()` | ✅ |
| `abs()` | ✅ |
| `ceiling()` | ✅ |
| `exp()` | ✅ |
| `floor()` | ✅ |
| `ln()` | ✅ |
| `log(base)` | ✅ |
| `power(exponent)` | ✅ |
| `truncate()` | ✅ |

---

## Tree Navigation

| Function | Status | Notes |
|----------|--------|-------|
| `children()` | ✅ | |
| `descendants()` | ✅ | |
| `extension(url)` | ✅ | Full support for object and primitive extensions with variable resolution |

---

## Utility Functions

| Function | Status | Notes |
|----------|--------|-------|
| `trace([name][, projection])` | ✅ | Projection support included |
| `now()` | ✅ | |
| `timeOfDay()` | ✅ | |
| `today()` | ✅ | |
| `defineVariable(name[, expr])` *(STU)* | ✅ | |
| `lowBoundary([precision])` *(STU)* | ✅ | Decimal, Date, DateTime, Time |
| `highBoundary([precision])` *(STU)* | ✅ | Decimal, Date, DateTime, Time |
| `precision()` *(STU)* | ✅ | See precision limitation note |

---

## Date/Time Component Extraction *(STU)*

All component functions implemented: `yearOf()`, `monthOf()`, `dayOf()`, `hourOf()`, `minuteOf()`, `secondOf()`, `millisecondOf()` — all ✅

---

## Date and Time Interval Functions *(STU)*

| Function | Status |
|----------|--------|
| `duration(value, precision)` | ✅ |
| `difference(value, precision)` | ✅ |

---

## Operations

### Equality

| Operator | Status | Notes |
|----------|--------|-------|
| `=` (equals) | ✅ | All types including dates and quantities |
| `~` (equivalent) | ✅ | Full equivalence checking |
| `!=` (not equals) | ✅ | |
| `!~` (not equivalent) | ✅ | |

### Comparison

| Operator | Status |
|----------|--------|
| `>` | ✅ |
| `<` | ✅ |
| `<=` | ✅ |
| `>=` | ✅ |

### Type operators

| Operator | Status | Notes |
|----------|--------|-------|
| `is` | ✅ | Full namespace qualification and FHIR type hierarchy |
| `as` | ✅ | Full namespace qualification and type casting |

### Collection operators

| Operator | Status |
|----------|--------|
| `\|` (union) | ✅ |
| `in` (membership) | ✅ |
| `contains` (containership) | ✅ |
| Collection navigation | ✅ |

### Boolean logic

| Operator | Status |
|----------|--------|
| `and` | ✅ |
| `or` | ✅ |
| `xor` | ✅ |
| `implies` | ✅ |
| `not()` | ✅ |

### Math operators

| Operator | Status | Notes |
|----------|--------|-------|
| `*` | ✅ | |
| `/` | ✅ | |
| `+` | ✅ | Numeric and String |
| `-` | ✅ | |
| `div` | ✅ | Integer division |
| `mod` | ✅ | Modulo |
| `&` | ✅ | String concatenation |
| Date/Time arithmetic | ✅ | Full timezone and precision handling |
| Unary `+` and `-` | ✅ | |

---

## Aggregates *(STU)*

| Function | Status | Notes |
|----------|--------|-------|
| `aggregate(aggregator[, init])` | ✅ | Full accumulator support |
| `sum()` | ✅ | |
| `min()` | ✅ | |
| `max()` | ✅ | |
| `avg()` | ✅ | |

---

## Environment Variables

| Variable | Status |
|----------|--------|
| `%variable` | ✅ |
| `%context`, `$this`, `$index`, `$total` | ✅ |

---

## Types and Reflection

| Feature | Status | Notes |
|---------|--------|-------|
| Models (namespace qualification) | ✅ | Full FHIR type hierarchy |
| `type()` reflection *(STU)* | ✅ | Enhanced with namespace support |
| Type Safety / Strict Evaluation | ✅ | Configurable strict mode |

---

## Instance Selector *(STU)*

Object creation syntax (`typename { element: value, ... }`): ✅

---

## FHIR-Specific Functions

| Function | Status | Notes |
|----------|--------|-------|
| `extension(url)` | ✅ | Full support with variable URL resolution |
| `hasValue()` | ✅ | Tests if primitive has a value beyond extensions |
| `getValue()` | ❌ | Not implemented |
| `resolve()` | ✅ | In-scope resolution: `contained` resources and resources available to the evaluation context (matched by `Type/id`, including the trailing `Type/id` of an absolute URL); unresolved `Type/id` references return a typed stub so `resolve() is X` works. In SQL-on-FHIR the resolution scope is the **whole input bundle**, so a ViewDefinition can follow a `Reference` to a sibling resource (e.g. `Encounter.subject.resolve()` → `Patient`). Remote resolution against **explicitly allowlisted** servers is available opt-in (`SOF_RESOLVE_*`); see [SQL-on-FHIR → Resolving References](ch06-sql-on-fhir.md). Storage-backed resolution remains tracked in [#167](https://github.com/HeliosSoftware/hfs/issues/167). |
| `ofType()` (FHIR form) | ✅ | Full FHIR type support |
| `elementDefinition()` | ❌ | Not implemented |
| `slice()` | ❌ | Not implemented |
| `checkModifiers()` | ❌ | Not implemented |
| `conformsTo()` | ❌ | Requires profile validation |
| `memberOf()` | ✅ | Via `%terminologies` integration |
| `subsumes()` (function form) | ❌ | Use `%terminologies.subsumes()` instead |
| `subsumedBy()` | ❌ | Not implemented |
| `htmlChecks()` | ❌ | XHTML narrative validation; not implemented |
| `comparable()` | ✅ | UCUM unit comparison |
| `weight()` | ❌ | Not implemented |

---

## Document Functions (HL7 Java extension)

These functions navigate FHIR documents: a `Bundle` of type `document` whose first entry is a `Composition`. They are **not** part of FHIRPath, nor of the FHIR specification's additional functions. They are an extension of the HL7 Java reference implementation (`org.hl7.fhir.core`), written for "clinical profiles" of IPS documents, whose invariants say what a document must contain, e.g. `Bundle.section('http://loinc.org', '11450-4').exists()`. `helios-fhirpath` follows the Java implementation and passes the `documentFunctions` conformance group, except for one static type check ([#2245](https://github.com/HeliosSoftware/hfs/issues/2245)).

| Function | Status | Notes |
|----------|--------|-------|
| `section(system, code [, min, max])` | ✅ | On documents or Compositions: the sections, nested ones too, whose `code` has a matching coding. `min`/`max` default to 1 and 1 |
| `entry(system, code [, min, max])` | ✅ | On sections: their entries, resolved in the document, whose code has a matching coding. `min`/`max` default to 0 and `*` |
| `entryIn(valueSet [, min, max])` | ✅ | As `entry()`, with a code in the value set. Needs a terminology server; an unknown value set is an error |
| `resources(system, code [, type])` | ✅ | On documents, or anything in one: every resource in the document with a matching code, optionally of one type |
| `resourcesIn(valueSet [, type])` | ✅ | As `resources()`, with a code in the value set. Needs a terminology server |
| `sameSubject([other])` | ✅ | Whether the focus resource's `subject`/`patient` is the same as `other`'s, or the document Composition's. References are compared as strings, identifiers by system and value |
| `hasCoding(system, code [, version])` | 🟡 | On Codings and CodeableConcepts: whether any coding matches. The Java engine also rejects a call on any other type before evaluation; this engine has no static type check and returns `false` |

A resource's code is its `code`, `vaccineCode` or `medication` (a CodeableConcept, or a CodeableReference's `concept`). A parameter given as `{}` is ignored. `min > max`, a `max` other than an integer or `'*'`, or a type that isn't a resource type name is an error. References resolve as the validator resolves them in a Bundle: absolute ones by `fullUrl`, relative ones only against a RESTful Composition `fullUrl`, and `#id` against the Composition's contained resources. A reference with more than one match doesn't resolve. A Composition evaluated outside a Bundle resolves only `#id` references.

---

## Terminology Functions (`%terminologies`)

| Function | Status |
|----------|--------|
| `%terminologies.expand(url[, params])` | ✅ |
| `%terminologies.lookup(coding[, params])` | ✅ |
| `%terminologies.validateVS(url, coding[, params])` | ✅ |
| `%terminologies.validateCS(system, coding[, params])` | ✅ |
| `%terminologies.subsumes(system, codeA, codeB)` | ✅ |
| `%terminologies.translate(url, coding[, params])` | ✅ |

---

## Type Factory (`%factory`) and Server API (`%server`)

All `%factory.*` and `%server.*` functions are **❌ Not Implemented**.
