# helios-fhir-validator

FHIR resource validation for [Helios FHIR Server](https://github.com/HeliosSoftware/hfs), built on the [FHIR Schema](https://fhir-schema.github.io/fhir-schema/) approach: StructureDefinitions compile to differential JSON-schema-like forms and validate via **cooperative schema sets** (no snapshot flattening).

## Features

- Structural validation: unknown elements, cardinality, choices, fixed/pattern, `maxLength` / `minValue` / `maxValue`, primitives
- Profile layering: `meta.profile`, caller profiles, slicing (pattern / type / profile / binding + reslices), extension sugar
- Deferred effects: FHIRPath invariants (`fhirpath` feature) and terminology bindings (required; optional extensible warnings)
- Embedded core schema + terminology packs for R4 / R4B / R5 / R6 (feature-gated)
- FHIR NPM / IG package cache, offline dependency resolution, `fhirVersions` checks
- QuestionnaireResponse validation against a Questionnaire definition
- Authoring helpers (`editor`) for “what can I add here?” UIs

## Quick start

```rust
use helios_fhir_validator::{SchemaRegistry, ValidationOptions, Validator};
use std::sync::Arc;

let mut registry = SchemaRegistry::new();
// …insert schemas or use packs::core_registry(FhirVersion::R4)
let validator = Validator::new(Arc::new(registry));
let outcome = validator.validate_sync(&resource, &ValidationOptions::default());
```

## Packages

See [docs/packages.md](docs/packages.md) for cache layout, `HFS_FHIR_PACKAGE_*` operator config, and the bundled sample IG under `tests/fixtures/packages/`.

## Tests

```bash
cargo test -p helios-fhir-validator
cargo test -p helios-fhir-validator -- --ignored   # whole-pack smoke
```
