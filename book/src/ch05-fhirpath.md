# FHIRPath Expressions

The `helios-fhirpath` crate is a complete implementation of the [FHIRPath 3.0.0-ballot specification](https://hl7.org/fhirpath/2025Jan/). It ships two executables (`fhirpath-cli` and `fhirpath-server`) and can be embedded as a library.

---

## What Is FHIRPath?

**FHIRPath** is a path-based navigation and extraction language for healthcare data. It is used throughout the FHIR ecosystem for:

- **Resource validation** — expressing invariants and co-occurrence rules
- **Search parameter definitions** — specifying which elements a search parameter indexes
- **Implementation guides** — encoding profile constraints and slicing discriminators
- **Clinical decision support** — writing rules in CDS Hooks services
- **SQL-on-FHIR** — defining column expressions in ViewDefinitions
- **Terminology integration** — filtering and validating coded values

### Validation example

```fhirpath
reference.startsWith('#').not() or
($context.reference.substring(1) in $resource.contained.id)
```
This invariant ensures that a local reference points to a contained resource that actually exists.

### Search parameter example

```fhirpath
Patient.name.given
```
Defines which FHIR element a search parameter indexes (the patient's given names).

### Clinical rule example

```fhirpath
Observation.where(
  code.coding.system = 'http://loinc.org' and
  code.coding.code = '8480-6'
).value.quantity > 140
```
Identifies systolic blood pressure observations above 140.

---

## Basic Syntax and Examples

FHIRPath expressions navigate resource elements using dot notation, function calls, and operators.

### Path navigation

```fhirpath
# Navigate nested elements
Patient.name.family

# Access the first element
Patient.name.first()

# Access by index
Patient.name[0].given[0]
```

### Filtering with `where()`

```fhirpath
# Filter by system code
Patient.telecom.where(system = 'phone')

# Multiple conditions
Patient.telecom.where(system = 'phone' and use = 'mobile').value

# Filter observations to a specific LOINC code
Observation.where(code.coding.code = '8480-6')
```

### Testing existence

```fhirpath
# Does the patient have a phone number?
Patient.telecom.where(system = 'phone').exists()

# Does every name have a family name?
Patient.name.all(family.exists())
```

### String functions

```fhirpath
# Concatenate given names
Patient.name.given.join(' ')

# Check format
Patient.identifier.value.matches('^[0-9]{9}$')
```

### Variables

```fhirpath
# Built-in variable %threshold passed via CLI
value > %threshold

# Built-in context variables
$this    # current item in iteration
$index   # current position in a collection
$total   # total collection count
```

---

## Using fhirpath-cli

```
fhirpath-cli [OPTIONS] -e <EXPRESSION>
```

| Flag | Description |
|------|-------------|
| `-e, --expression <EXPR>` | FHIRPath expression to evaluate (required) |
| `-r, --resource <FILE>` | Path to FHIR resource JSON file, or `-` for stdin |
| `-c, --context <EXPR>` | Context expression to scope the root |
| `--var <NAME=VALUE>` | Define an environment variable (repeatable) |
| `--fhir-version <VER>` | FHIR version: `R4`, `R4B`, `R5`, `R6` (default: `R4`) |
| `--parse-debug-tree` | Print the parse tree and exit (no resource needed) |
| `--terminology-server <URL>` | Override the default terminology server URL |

**Examples:**

```bash
# Basic evaluation
fhirpath-cli -e "Patient.name.family" -r patient.json

# Context expression
fhirpath-cli -c "Patient.name" -e "family" -r patient.json

# Variable injection
fhirpath-cli -e "value > %threshold" -r observation.json --var threshold=5.0

# Stdin input
cat patient.json | fhirpath-cli -e "Patient.name.family" -r -

# Debug parse tree (no resource file needed)
fhirpath-cli -e "Patient.name.given.first()" --parse-debug-tree

# Explicit FHIR version
fhirpath-cli --fhir-version R5 -e "Patient.name.family" -r patient.json
```

---

## Using fhirpath-server

The HTTP server is compatible with [FHIRPath Lab](https://fhirpath-lab.com/).

### Starting the server

```bash
# Defaults: port 3000, host 127.0.0.1
fhirpath-server

# Custom configuration
FHIRPATH_SERVER_PORT=8080 FHIRPATH_SERVER_HOST=0.0.0.0 fhirpath-server
```

### Endpoints

| Method | URL | Description |
|--------|-----|-------------|
| `POST` | `/` | Evaluate FHIRPath (auto-detects FHIR version) |
| `POST` | `/r4` | R4-specific evaluation |
| `POST` | `/r4b` | R4B-specific evaluation |
| `POST` | `/r5` | R5-specific evaluation |
| `POST` | `/r6` | R6-specific evaluation |
| `GET` | `/health` | Health check |

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `FHIRPATH_SERVER_PORT` | `3000` | Server port |
| `FHIRPATH_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `FHIRPATH_LOG_LEVEL` | `info` | Log level |
| `FHIRPATH_ENABLE_CORS` | `true` | Enable CORS |
| `FHIRPATH_CORS_ORIGINS` | `*` | Allowed CORS origins |
| `FHIRPATH_TERMINOLOGY_SERVER` | *(none)* | Terminology server URL |

---

## Terminology Server Integration

FHIRPath provides access to terminology services via the `%terminologies` environment variable.

> **Warning:** By default, the implementation uses public test servers (`https://tx.fhir.org/r4/` for R4/R4B, `https://tx.fhir.org/r5/` for R5). **Do not use these in production** — they are test servers with no SLA.

Configure a production server:

```bash
# Via environment variable
export FHIRPATH_TERMINOLOGY_SERVER=https://your-terminology-server.com/fhir

# Via CLI flag
fhirpath-cli --terminology-server https://your-terminology-server.com/fhir ...

# Via server environment variable
FHIRPATH_TERMINOLOGY_SERVER=https://your-server.com/fhir fhirpath-server
```

Available `%terminologies` functions:

```fhirpath
# Expand a ValueSet
%terminologies.expand('http://hl7.org/fhir/ValueSet/administrative-gender')

# Lookup code details
%terminologies.lookup(Observation.code.coding.first())

# Validate against a ValueSet
%terminologies.validateVS(
  'http://hl7.org/fhir/ValueSet/observation-vitalsignresult',
  Observation.code.coding.first()
)

# Validate against a CodeSystem
%terminologies.validateCS('http://loinc.org', Observation.code.coding.first())

# Check code subsumption
%terminologies.subsumes('http://snomed.info/sct', '73211009', '5935008')

# Translate using a ConceptMap
%terminologies.translate(
  'http://hl7.org/fhir/ConceptMap/cm-address-use-v2',
  Patient.address.use
)

# Check if a coding is a member of a ValueSet
Observation.code.coding.where(
  memberOf('http://hl7.org/fhir/ValueSet/observation-vitalsignresult')
)
```

---

## Built-in Functions Reference

The following categories are fully implemented. See [Appendix B](appendix-b-fhirpath-functions.md) for the complete implementation matrix with status indicators.

| Category | Example functions |
|----------|------------------|
| Existence | `empty()`, `exists()`, `all()`, `count()`, `distinct()` |
| Filtering & Projection | `where()`, `select()`, `ofType()`, `sort()`, `repeat()` |
| Subsetting | `first()`, `last()`, `tail()`, `skip()`, `take()`, `single()` |
| Combining | `union()`, `combine()`, `intersect()`, `exclude()` |
| Conversion | `toBoolean()`, `toInteger()`, `toDecimal()`, `toString()`, `toDate()`, `toDateTime()` |
| String | `indexOf()`, `substring()`, `startsWith()`, `endsWith()`, `contains()`, `matches()`, `replace()`, `upper()`, `lower()`, `trim()`, `split()`, `join()`, `encode()`, `decode()` |
| Math | `abs()`, `ceiling()`, `floor()`, `round()`, `sqrt()`, `exp()`, `ln()`, `log()`, `power()`, `truncate()` |
| Date/Time | `today()`, `now()`, `timeOfDay()`, `duration()`, `difference()`, component extractors (`yearOf()`, `monthOf()`, etc.) |
| Tree navigation | `children()`, `descendants()`, `extension()` |
| Utility | `trace()`, `defineVariable()`, `iif()`, `lowBoundary()`, `highBoundary()`, `precision()` |
| Aggregates | `aggregate()`, `sum()`, `min()`, `max()`, `avg()` |
| Type | `is`, `as`, `ofType()`, `type()` |
| FHIR-specific | `hasValue()`, `memberOf()`, `comparable()` |
