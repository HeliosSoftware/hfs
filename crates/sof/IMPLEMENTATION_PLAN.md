# SQL-on-FHIR $run Operation Implementation Plan

## Overview
Implementation of the SQL-on-FHIR $run operation for transforming FHIR resources using ViewDefinitions. The implementation will be built in phases, starting with a CLI tool and progressing to an HTTP server.

## Specifications
- [$run Operation API](https://sql-on-fhir.org/ig/latest/api.html#run-viewdefinition)
- [ViewDefinition Processing Algorithm](https://sql-on-fhir.org/ig/latest/StructureDefinition-ViewDefinition.html#processing-algorithm)
- [SQL-on-FHIR Test Framework](https://github.com/FHIR/sql-on-fhir-v2?tab=readme-ov-file#registering-your-implementation)
- [Test Suite Documentation](https://sql-on-fhir.org/extra/tests.html)

## Architecture

### Core Library (`lib.rs`)
- Main processing engine with public function:
  ```rust
  pub fn run_view_definition(
      view_definition: ViewDefinition,
      bundle: Bundle,
      content_type: ContentType
  ) -> Result<Vec<u8>, Error>
  ```

### Processing Algorithm
1. **Resource Selection**: Filter resources by type and optional profile
2. **Filtering**: Apply `where` clauses using FHIRPath expressions
3. **Projection**: Generate columns via `select` structures
4. **Row Generation**: Handle `forEach`/`forEachOrNull` logic
5. **Output Formatting**: Transform to requested format (CSV, JSON, etc.)

## Implementation Phases

### Phase 1: Core Data Structures
- ✅ Add the ViewDefinition StructureDefinition from prior project to all versions of the FHIR models, and generate
- Integrate fhirpath crate for expression evaluation
- Set up basic error handling and result types

### Phase 2: Core Processing Algorithm
- Implement resource filtering with where clauses
- Implement column projection with select structures
- Handle forEach/forEachOrNull row generation
- Support constants and nested selects

### Phase 3: Output Formatters
- CSV formatter with header support
- JSON/NDJSON formatters
- Parquet formatter (lower priority)

### Phase 4: CLI Implementation
- Parse ViewDefinition from file/stdin
- Parse FHIR Bundle input
- Support output format selection via command-line args
- Example usage:
  ```bash
  sof-cli --view view.json --bundle bundle.json --format csv
  ```

### Phase 5: Testing & Compliance
- Download SQL-on-FHIR test suite
- Run basic ViewDefinition tests
- Fix issues and achieve full test suite compliance
- Register implementation with official test framework

### Phase 6: HTTP Server Implementation
- POST /ViewDefinition/$run endpoint
- GET /ViewDefinition/{id}/$run endpoint
- Query parameter support (patient, group, _format, etc.)
- Pagination support (_count, _page)
- Content negotiation for output formats

## Technical Considerations

### Dependencies
- `fhir` crate for FHIR types
- `fhirpath` crate for expression evaluation
- `serde_json` for JSON handling
- `csv` for CSV output
- `arrow` and `parquet` for Parquet support
- `axum` or `actix-web` for HTTP server

### Error Handling
- Invalid ViewDefinition structure
- FHIRPath expression evaluation errors
- Type mismatches in column projections
- Resource filtering edge cases

### Performance
- Efficient resource filtering
- Streaming output for large datasets
- Memory-efficient processing of bundles

## Success Criteria
1. Pass all SQL-on-FHIR test suite tests
2. Support all required output formats
3. Handle edge cases gracefully
4. Performance suitable for real-world datasets
5. Clean, maintainable code architecture