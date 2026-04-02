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
| `resolve()` | ❌ | Requires resource resolver integration |
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
