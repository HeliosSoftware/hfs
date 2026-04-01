# HTS — Helios Terminology Service: Implementation To-Do List

> Source: [plan-hts.md](./plan-hts.md) | [Discussion #54](https://github.com/HeliosSoftware/hfs/discussions/54)
> Phases 0–10: SQLite MVP (standalone binary, shared terminology, single-tenant)
> Phases 11–14: Future work (PostgreSQL, Oxigraph, advanced importers, production hardening)
> Phases are independently completable and can be modified at any time.

---

## Phase 0: Branch, Workspace & CI Setup

- [✅] Create branch `feat/hts-terminology-service` from `main`
- [✅] Add `crates/hts` to workspace `Cargo.toml` members list
- [✅] Create `crates/hts/Cargo.toml` (per plan-hts.md §1.1 spec)
- [✅] Create empty module skeleton matching the planned layout:
  `main.rs`, `config.rs`, `server.rs`, `state.rs`, `error.rs`,
  `traits/mod.rs`, `operations/mod.rs`, `import/mod.rs`, `backend/mod.rs`
- [✅] Add `.github/workflows/hts.yml` — GitHub Actions workflow:
  - Trigger on push/PR affecting `crates/hts/**`
  - Jobs: `cargo check -p helios-hts`, `cargo clippy -p helios-hts`, `cargo test -p helios-hts`
- [✅] Verify `cargo check -p helios-hts` passes with empty stubs

---

## Phase 1: Config, Server Shell & Health Endpoint

- [✅] `config.rs` — `HtsConfig` struct via `clap` + env vars:
  - `HTS_SERVER_PORT` (default 8090)
  - `HTS_SERVER_HOST` (default 127.0.0.1)
  - `HTS_LOG_LEVEL` (default info)
  - `HTS_DATABASE_URL` (default ./hts.db)
  - `HTS_STORAGE_BACKEND` (default sqlite)
  - `HTS_ENABLE_CORS` (default true)
  - `HTS_CORS_ORIGINS` (default *)
- [✅] `server.rs` — `create_app()` returning Axum router with CORS, timeout, tracing middleware
- [✅] `main.rs` — parse config, init logging, start server on configured host:port
- [✅] `GET /health` handler returning `{"status":"ok","service":"hts"}`
- [✅] `error.rs` — `HtsError` enum: `NotFound`, `NotSupported`, `InvalidRequest`, `Internal`, `StorageError`; impl `IntoResponse` mapping to FHIR OperationOutcome
- [✅] **Verify:** `cargo run --bin hts` starts; `curl http://localhost:8090/health` returns 200

---

## Phase 2: Core Traits & Request/Response Types

- [✅] `traits/code_system.rs` — `CodeSystemOperations` trait:
  `lookup()`, `validate_code()`, `subsumes()` — all `async`, all take `&TenantContext`
- [✅] `traits/value_set.rs` — `ValueSetOperations` trait:
  `expand()`, `validate_code()`
- [✅] `traits/concept_map.rs` — `ConceptMapOperations` trait:
  `translate()`, `closure()`
- [✅] `traits/metadata.rs` — `TerminologyMetadata` trait:
  `backend_name()`, `supported_systems()`, `supports_subsumption()`
- [✅] `traits/mod.rs` — `TerminologyBackend` supertrait combining all four + blanket impl
- [✅] `state.rs` — `AppState<B: TerminologyBackend>` (Arc-wrapped for Axum)
- [✅] Define all request/response structs (FHIR Parameters in/out) in `types.rs`:
  - `LookupRequest` / `LookupResponse`
  - `ValidateCodeRequest` / `ValidateCodeResponse`
  - `SubsumesRequest` / `SubsumesResponse`
  - `ExpandRequest` / `ExpandResponse`
  - `TranslateRequest` / `TranslateResponse`
  - `ClosureRequest` / `ClosureResponse`

---

## Phase 3: SQLite Schema & Backend Infrastructure

- [✅] `backend/sqlite/schema.rs` — all 9 `CREATE TABLE` statements + indexes:
  - `code_systems` (id, url, version, name, status, content, created_at, updated_at)
  - `concepts` (id, system_id→code_systems, code, display, definition) + index on (system_id, code)
  - `concept_hierarchy` (system_id, parent_code, child_code) + index on (system_id, child_code)
  - `concept_properties` (id, concept_id→concepts, property, value_type, value)
  - `concept_designations` (id, concept_id→concepts, language, use_system, use_code, value)
  - `value_sets` (id, url, version, name, status, compose_json, created_at, updated_at)
  - `value_set_expansions` (value_set_id, system_url, code, display) — materialized cache
  - `concept_maps` (id, url, version, source_uri, target_uri, status, created_at)
  - `concept_map_elements` (id, map_id→concept_maps, source_system, source_code, target_system, target_code, equivalence) + index on (map_id, source_system, source_code)
- [✅] Migration runner: apply schema on first startup if tables don't exist
- [✅] `backend/sqlite/mod.rs` — `SqliteTerminologyBackend` struct (r2d2 connection pool); `new(db_path)` constructor; impl `TerminologyMetadata`
- [✅] Wire `SqliteTerminologyBackend` into `AppState` in `main.rs`
- [✅] **Verify:** server starts, DB file created, tables visible via `sqlite3 hts.db .tables`

---

## Phase 4: $lookup & $validate-code (CodeSystem)

- [✅] `backend/sqlite/code_system.rs` — impl `CodeSystemOperations`:
  - `lookup()`: query `concepts` JOIN `concept_properties` JOIN `concept_designations` by (system_url, code); return display + properties + designations
  - `validate_code()`: existence check in `concepts`; return `result=true/false` + display
  - `subsumes()`: stub returning `HtsError::NotSupported` (implemented in Phase 5)
- [✅] `operations/lookup.rs` — Axum handler: parse FHIR Parameters → `LookupRequest` → backend → FHIR Parameters response
- [✅] `operations/validate_code.rs` — handler for `POST /CodeSystem/$validate-code`
- [✅] Register both routes in `server.rs`
- [✅] Unit tests with in-memory SQLite (seed concepts, verify lookup returns correct display + properties)
- [✅] **Verify:** seed a concept manually, `curl -X POST /CodeSystem/$lookup ...` returns populated Parameters

---

## Phase 5: $subsumes (Hierarchy)

- [✅] `backend/sqlite/code_system.rs` — implement `subsumes()`:
  - Query `concept_hierarchy` to test A→B parent-child relationship (direct link in pre-materialized table)
  - Handle both directions: `subsumes` (A is ancestor of B) and `subsumed-by` (A is descendant of B)
  - Return `outcome`: `equivalent` | `subsumes` | `subsumed-by` | `not-subsumed`
- [✅] `operations/subsumes.rs` — Axum handler for `POST /CodeSystem/$subsumes`
- [✅] Register route
- [✅] Unit tests covering all four outcome values + multi-level hierarchy (A→B→C)
- [✅] **Verify:** import hierarchy A→B→C; `$subsumes(A, C)` returns `subsumes`; `$subsumes(C, A)` returns `subsumed-by`

---

## Phase 6: FHIR Bundle Import Pipeline

> Moved before $expand/$translate so the service can be tested end-to-end with real data
> from Phase 7 onward. Until Phase 6 is complete, use manual SQLite seeding for tests.

- [✅] `import/mod.rs` — `Importer` trait + `ImportStats` struct (code_systems, value_sets, concept_maps, concepts, errors)
- [✅] `import/fhir_bundle.rs` — `FhirBundleImporter`:
  - Parse FHIR Bundle entries; detect resource type (CodeSystem | ValueSet | ConceptMap)
  - **CodeSystem:** extract concepts, designations, properties, hierarchy (parent property links) → insert normalized rows; pre-materialize `concept_hierarchy` from parent-child concept properties
  - **ValueSet:** extract compose JSON → insert `value_sets` row (expansion deferred to first $expand call)
  - **ConceptMap:** extract groups + elements → insert `concept_maps` + `concept_map_elements` rows
  - Respect import order: CodeSystems first → ValueSets → ConceptMaps
  - Return `ImportStats` with counts + any errors
- [✅] `POST /import` endpoint (Content-Type: `application/fhir+json` → FHIR Bundle)
- [✅] Unit tests with small synthetic Bundles (1 CodeSystem + 1 ValueSet + 1 ConceptMap)
- [✅] Integration test: import Bundle → run $lookup end-to-end (using Phase 4 operations)
- [✅] **Verify:** `curl -X POST /import -d @test-bundle.json` populates DB; $lookup queries succeed

---

## Phase 7: $expand & ValueSet $validate-code

- [✅] `backend/sqlite/value_set.rs` — impl `ValueSetOperations`:
  - `expand()`: check `value_set_expansions` cache; on miss, parse `compose_json`, query `concepts` for matching codes, populate cache, return expansion; support `count` + `offset` pagination
  - `validate_code()`: trigger expansion if needed; check code in expanded set; return `result=true/false` + display
- [✅] `operations/expand.rs` — Axum handler for `POST /ValueSet/$expand`
- [✅] `operations/validate_code.rs` — extend to handle `POST /ValueSet/$validate-code` (detect VS vs CS from parameters)
- [✅] Register routes
- [✅] Unit tests: expand VS referencing a CodeSystem include; validate code in/out of set; pagination
- [✅] Integration test: import Bundle with ValueSet → $expand → $validate-code end-to-end
- [✅] **Verify:** import Bundle with ValueSet, expand it, validate a code, confirm cache hit on second expand call

---

## Phase 8: $translate & $closure (ConceptMap)

- [✅] `backend/sqlite/concept_map.rs` — impl `ConceptMapOperations`:
  - `translate()`: query `concept_map_elements` by (map_id or source_system, source_code); return target codes, equivalence, display
  - `closure()`: given a set of codes, return their transitive closure (union of hierarchy traversal results and ConceptMap forward translations)
- [✅] `operations/translate.rs` — Axum handler for `POST /ConceptMap/$translate`
- [✅] `operations/closure.rs` — Axum handler for `POST /ConceptMap/$closure`
- [✅] Register routes
- [✅] Unit tests: direct mapping, no-match case, multiple targets; closure for a 3-code hierarchy
- [✅] Integration test: import Bundle with ConceptMap → $translate end-to-end
- [✅] **Verify:** import Bundle with ConceptMap, translate a code, verify correct target code and equivalence returned

---

## Phase 9: Resource CRUD API

- [✅] Integrate `helios-persistence::ResourceStorage` (sqlite feature) into `AppState` for resource-level CRUD
- [✅] Handlers for all 3 resource types × 4 verbs:
  - `GET    /CodeSystem/{id}` — read stored FHIR JSON
  - `POST   /CodeSystem` — create; also triggers indexing into normalized terminology tables
  - `PUT    /CodeSystem/{id}` — update; re-index terminology tables
  - `DELETE /CodeSystem/{id}` — delete; cascade to normalized tables
  - Same pattern for `/ValueSet/{id}` and `/ConceptMap/{id}`
- [✅] Versioning via ETag (delegate to `ResourceStorage::update()` versioning)
- [✅] Unit tests for CRUD round-trips (POST → GET → PUT → GET → DELETE → GET returns 404)
- [✅] **Verify:** POST a CodeSystem JSON → GET it back → $lookup a concept from it → DELETE → GET returns 404

---

## Phase 10: /metadata TerminologyCapabilities

- [✅] `operations/metadata.rs` — generate FHIR `TerminologyCapabilities` resource:
  - List supported operations ($lookup, $validate-code, $subsumes, $expand, $translate, $closure)
  - List known CodeSystem URLs (query `code_systems` table)
  - Report `codeSystem[].subsumption = true`
  - Include software name/version
- [✅] `GET /metadata` handler returning TerminologyCapabilities JSON
- [✅] Unit test: returned resource validates as TerminologyCapabilities; lists all 6 operations
- [✅] **Verify:** `curl http://localhost:8090/metadata` returns valid TerminologyCapabilities resource

---

## Phase 10.5: HFS Integration (Post-MVP, Deferred)

> Wire HTS into helios-hfs so FHIRPath functions and FHIR search can delegate to HTS.

- [✅] Add `HFS_TERMINOLOGY_SERVER` env var to `helios-hfs` config (default: none/disabled)
- [✅] HTTP client (reqwest) in `helios-hfs` that POSTs to HTS endpoints
- [✅] FHIRPath `memberOf()` → `POST /ValueSet/$validate-code`
- [✅] FHIRPath `subsumes()` → `POST /CodeSystem/$subsumes`
- [✅] FHIR search `:in`, `:not-in` modifiers → `POST /ValueSet/$expand`
- [✅] Integration tests: start HFS + HTS together, run FHIRPath expression using `memberOf()`, verify delegation

---

## Phase 11: PostgreSQL Backend (Future)

- [ ] Add `tokio-postgres` + `deadpool-postgres` as optional dependencies (feature `postgres`)
- [ ] `backend/postgres/` — implement `TerminologyBackend`:
  - Same logical schema as SQLite; use `BIGSERIAL` PKs, `GIN` indexes on `concept_properties`
  - Replace pre-materialized hierarchy with **recursive CTEs** for `$subsumes` (better for SNOMED polyhierarchy at scale)
  - Add `pg_trgm` GIN index on `concepts.display` for full-text search
- [ ] `HTS_STORAGE_BACKEND=postgres` wiring in `main.rs`
- [ ] Separate migration runner for PostgreSQL (idempotent, `CREATE TABLE IF NOT EXISTS`)
- [ ] Integration tests via `testcontainers` (matching `helios-persistence` testing pattern)
- [ ] **Verify:** all operations pass the same test suite as the SQLite backend

---

## Phase 12: Oxigraph / RDF Triplestore Backend (Future)

- [ ] Evaluate `oxigraph` crate as SPARQL-capable in-process triplestore
- [ ] `backend/oxigraph/` — implement `TerminologyBackend`:
  - Represent CodeSystem concepts as RDF triples (SKOS / OWL vocabulary)
  - `$subsumes` via SPARQL `rdfs:subClassOf*` transitive property path query
  - `$expand` via SPARQL CONSTRUCT query against SKOS concept scheme
- [ ] `HTS_STORAGE_BACKEND=oxigraph` wiring
- [ ] SNOMED CT OWL/RDF import (convert RF2 to OWL using `snomed-owl-toolkit` or pre-converted OWL files)
- [ ] Performance benchmarks vs. SQLite and PostgreSQL for SNOMED-scale queries
- [ ] **Verify:** load SNOMED CT OWL; `$subsumes` query on two SNOMED codes returns correct result

---

## Phase 13: Advanced Import Formats (Future)

> **Licensing notes are explicit tasks — all three terminologies require license verification before distribution.**

- [ ] **SNOMED CT license check:** Verify IHTSDO/NRC license coverage before importing or shipping SNOMED data
- [ ] **LOINC license check:** Verify Regenstrief Institute license coverage before importing or shipping LOINC data
- [ ] **RxNorm license check:** Verify NLM license coverage before importing or shipping RxNorm data
- [ ] `import/snomed_rf2.rs` — SNOMED CT RF2 importer:
  - Parse `Concept_Full_INT_*.txt`, `Description_Full_INT_*.txt`, `Relationship_Full_INT_*.txt`
  - Build transitive closure of `Is-a` (116680003) relationships into `concept_hierarchy`
  - Handle versioned snapshots; report import progress
- [ ] `import/loinc_csv.rs` — LOINC CSV importer (LoincTable.csv + MultiAxialHierarchy.csv)
- [ ] `import/icd10_cm.rs` — ICD-10-CM XML importer (CMS tabular XML format)
- [ ] `import/rxnorm_rrf.rs` — RxNorm RRF importer (RXNCONSO.RRF + RXNREL.RRF)
- [ ] CLI subcommand: `hts import --source ./file.zip --format snomed-rf2|loinc|icd10-cm|rxnorm`
- [ ] Progress bar / streaming log during large imports (SNOMED: ~350,000 concepts)
- [ ] **Verify:** import SNOMED CT snapshot; $lookup a known SNOMED code; $subsumes returns correct hierarchy result

---

## Phase 14: Production Hardening (Future)

- [ ] Background expansion scheduler (tokio task): ValueSets > `HTS_MAX_EXPANSION_SIZE` (default 10,000 codes) are expanded asynchronously
- [ ] Expansion status tracking: `pending | in-progress | complete | too-costly`; expose status in `$expand` response
- [ ] SNOMED post-coordination: basic expression parser stub → return `HtsError::NotSupported` until implemented
- [ ] Multi-tenancy activation: scope `code_systems`, `value_sets`, `concept_maps` queries by `tenant_id` when `TenantContext` is non-default
- [ ] Full HL7 FHIR Terminology Service conformance test suite validation
- [ ] Docker image: extend root `Dockerfile` with `BINARY_NAME=hts` build arg
- [ ] Performance benchmarks under realistic SNOMED + LOINC combined load
- [ ] Update `CLAUDE.md` with HTS build/run commands and environment variable table

---

## Key Files

| File | Purpose |
|------|---------|
| `Cargo.toml` (root) | Add `crates/hts` to members |
| `crates/hts/Cargo.toml` | Crate manifest (per plan-hts.md §1.1) |
| `crates/hts/src/main.rs` | Binary entry point |
| `crates/hts/src/config.rs` | HtsConfig (clap + env) |
| `crates/hts/src/server.rs` | Axum router factory |
| `crates/hts/src/state.rs` | AppState\<B: TerminologyBackend\> |
| `crates/hts/src/error.rs` | HtsError → OperationOutcome |
| `crates/hts/src/traits/` | 4 operation traits + TerminologyBackend supertrait |
| `crates/hts/src/operations/` | 6 operation handlers + /metadata |
| `crates/hts/src/import/` | Importer trait + FhirBundleImporter |
| `crates/hts/src/backend/sqlite/` | SQLite implementation of all traits |
| `.github/workflows/hts.yml` | CI workflow for helios-hts |

## Dependencies to Reuse (already in workspace)

| Dependency | Use |
|------------|-----|
| `helios-persistence::TenantContext` | Tenant param on all trait methods |
| `helios-persistence::ResourceStorage` | CRUD for CodeSystem/ValueSet/ConceptMap |
| `helios-fhir` | FHIR R4 types (CodeSystem, ValueSet, ConceptMap, Parameters) |
| `rusqlite` / `r2d2` / `r2d2_sqlite` | SQLite pool (already in helios-persistence) |
| `axum` / `tokio` / `tower-http` | Web framework (workspace deps) |
| `async-trait`, `serde`, `serde_json`, `thiserror`, `tracing`, `clap`, `uuid` | All workspace |
