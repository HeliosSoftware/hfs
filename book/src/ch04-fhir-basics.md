# FHIR Basics

This chapter covers the minimum FHIR knowledge needed to work with HFS tools. For a full introduction to the FHIR standard, see [hl7.org/fhir](https://hl7.org/fhir).

---

## Resources and Resource Types

A **FHIR resource** is a typed, self-describing JSON document representing a clinical or administrative entity. Every resource has a `resourceType` field that identifies its type.

Common resource types:

| Resource type | Represents |
|---------------|-----------|
| `Patient` | A person receiving care |
| `Observation` | A measurement or assessment (vitals, lab results) |
| `Condition` | A diagnosis or clinical problem |
| `MedicationRequest` | A medication prescription |
| `Encounter` | A clinical visit or episode |
| `Practitioner` | A healthcare provider |
| `Organization` | A care delivery organization |

Each resource type defines a specific set of fields, data types, and cardinality rules in the FHIR specification.

---

## FHIR Versions: R4, R4B, R5, R6

FHIR has evolved through several versions. HFS supports all four currently relevant versions:

| Version | Release | Key characteristics |
|---------|---------|---------------------|
| **R4** (4.0.1) | 2019 | Normative; most widely deployed |
| **R4B** (4.3.0) | 2022 | Normative; introduces clinical reasoning changes |
| **R5** (5.0.0) | 2023 | Current standard; significant model refinements |
| **R6** (6.0.0-ballot2) | In progress | Latest ballot; cutting-edge features |

Most production systems use R4 or R5. R4B is common in the US for specific profiles. R6 is for early adopters and testing.

HFS defaults to **R4** when no feature flags are specified. See [Multi-Version FHIR Support](ch08-versions.md) for how to enable multiple versions.

---

## JSON Representation of FHIR Data

FHIR resources are serialized as JSON objects. Here is a minimal `Patient` resource:

```json
{
  "resourceType": "Patient",
  "id": "example-patient",
  "name": [
    {
      "use": "official",
      "family": "Smith",
      "given": ["John", "Robert"]
    }
  ],
  "gender": "male",
  "birthDate": "1980-04-14",
  "telecom": [
    {
      "system": "phone",
      "value": "555-867-5309",
      "use": "mobile"
    },
    {
      "system": "email",
      "value": "john.smith@example.com"
    }
  ],
  "address": [
    {
      "use": "home",
      "line": ["123 Main St"],
      "city": "Springfield",
      "state": "IL",
      "postalCode": "62701"
    }
  ]
}
```

Key structural features:
- **`resourceType`** — mandatory field identifying the type
- **`id`** — server-assigned or client-provided identifier
- **Arrays** — many fields (name, telecom, address) are arrays to accommodate multiple values
- **Coded fields** — fields like `gender`, `system`, and `use` use defined value sets
- **Extensions** — any field can carry extensions for data not in the base spec

---

## Bundles and NDJSON

### Bundles

A **Bundle** is a FHIR resource that wraps a collection of other resources. It is the standard container for exchanging multiple resources in a single HTTP request or file.

```json
{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "Patient",
        "id": "p1",
        "name": [{"family": "Smith"}]
      }
    },
    {
      "resource": {
        "resourceType": "Patient",
        "id": "p2",
        "name": [{"family": "Jones"}]
      }
    }
  ]
}
```

Bundle `type` values commonly seen with HFS:
- `collection` — an unordered set of resources
- `transaction` — resources to process atomically
- `batch` — resources to process independently
- `searchset` — the result of a search operation

### NDJSON (Newline-Delimited JSON)

**NDJSON** (`.ndjson`) is an alternative to Bundle for large exports. Each line is a complete, self-contained JSON resource with no wrapper:

```ndjson
{"resourceType":"Patient","id":"p1","name":[{"family":"Smith"}]}
{"resourceType":"Patient","id":"p2","name":[{"family":"Jones"}]}
{"resourceType":"Observation","id":"o1","subject":{"reference":"Patient/p1"}}
```

NDJSON is preferred for bulk data exports (e.g., the FHIR `$export` operation) because it is memory-efficient to stream. `sof-cli` auto-detects NDJSON by file extension (`.ndjson`) or by inspecting file content.

### When to use each

| Format | Use for |
|--------|---------|
| Bundle (JSON) | REST API payloads, transactions, small datasets |
| NDJSON | Bulk exports, large datasets, streaming pipelines |

Both are accepted by `sof-cli` with `--bundle` and by `fhirpath-cli` with `-r`.
