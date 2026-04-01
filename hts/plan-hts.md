# HTS — Helios Terminology Service: Implementation Plan

> **Source:** [Discussion #54](https://github.com/HeliosSoftware/hfs/discussions/54)
> **Scope:** MVP — single-tenant, SQLite backend, all 6 FHIR terminology operations
> **Scalability:** Trait-based design, feature-gated backends, ready for multi-tenancy and PostgreSQL/Oxigraph

---

## Context

FHIR applications need a dedicated service to work with coded clinical data — checking whether a code is valid, expanding value sets, mapping codes across systems, and testing hierarchical relationships (e.g., "is diabetes a subtype of metabolic disease?"). HFS currently has no terminology layer. This plan creates `helios-hts`, a standalone FHIR Terminology Service (HTS) that:

- Implements the six FHIR terminology operations
- Starts simple (SQLite, single-tenant) but is architecturally ready to scale
- Integrates back into HFS so FHIRPath functions (`memberOf()`, `subsumes()`) and validation can delegate here

---

## Phase 1: Scaffold (New Crate + HTTP Shell)

### 1.1 New crate: `helios-hts`

Add to workspace `Cargo.toml` members list:
- `crates/hts`

Create `crates/hts/Cargo.toml`:

```toml
[package]
name = "helios-hts"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[[bin]]
name = "hts"
path = "src/main.rs"

[features]
default = ["sqlite"]
sqlite = ["dep:rusqlite", "dep:r2d2", "dep:r2d2_sqlite"]
postgres = ["dep:tokio-postgres", "dep:deadpool-postgres"]
R4 = ["helios-fhir/R4"]
R4B = ["helios-fhir/R4B"]
R5 = ["helios-fhir/R5"]
R6 = ["helios-fhir/R6"]

[dependencies]
helios-fhir = { workspace = true }
helios-persistence = { workspace = true, features = ["sqlite"] }
async-trait.workspace = true
axum = { workspace = true, features = ["json", "query"] }
tower.workspace = true
tower-http = { workspace = true, features = ["cors", "trace", "timeout"] }
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
clap = { workspace = true, features = ["derive", "env"] }
uuid = { workspace = true, features = ["v4"] }

# SQLite (default)
rusqlite = { version = "0.32", features = ["bundled"], optional = true }
r2d2 = { version = "0.8", optional = true }
r2d2_sqlite = { version = "0.25", optional = true }

# PostgreSQL (future)
tokio-postgres = { version = "0.7", optional = true }
deadpool-postgres = { version = "0.14", optional = true }
```

### 1.2 Module layout

```
crates/hts/src/
├── main.rs                  # Binary entry: parse config, start server
├── config.rs                # HtsConfig (clap + env vars)
├── server.rs                # create_app(), Axum router setup
├── state.rs                 # AppState<B: TerminologyBackend>
│
├── traits/
│   ├── mod.rs
│   ├── code_system.rs       # CodeSystemOperations trait
│   ├── value_set.rs         # ValueSetOperations trait
│   ├── concept_map.rs       # ConceptMapOperations trait
│   └── metadata.rs          # TerminologyMetadata trait
│
├── operations/
│   ├── mod.rs
│   ├── lookup.rs            # $lookup handler
│   ├── validate_code.rs     # $validate-code handler
│   ├── subsumes.rs          # $subsumes handler
│   ├── expand.rs            # $expand handler
│   ├── translate.rs         # $translate handler
│   └── closure.rs           # $closure handler
│
├── import/
│   ├── mod.rs               # ImportPipeline trait
│   └── fhir_bundle.rs       # FHIR Bundle importer (P0)
│
├── backend/
│   ├── mod.rs
│   └── sqlite/
│       ├── mod.rs           # SqliteTerminologyBackend
│       ├── schema.rs        # CREATE TABLE statements + migrations
│       ├── code_system.rs   # CodeSystemOperations impl
│       ├── value_set.rs     # ValueSetOperations impl
│       └── concept_map.rs   # ConceptMapOperations impl
│
└── error.rs                 # HtsError enum
```

---

## Phase 2: Core Traits

All traits are `#[async_trait]` and take `&self`. Multi-tenancy is stubbed via a `TenantContext` parameter (same type from `helios-persistence`) but ignored in single-tenant mode — this ensures the API never needs to change when multi-tenancy is added.

```rust
// traits/code_system.rs
#[async_trait]
pub trait CodeSystemOperations: Send + Sync {
    async fn lookup(
        &self,
        ctx: &TenantContext,
        req: LookupRequest,
    ) -> Result<LookupResponse, HtsError>;

    async fn validate_code(
        &self,
        ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError>;

    async fn subsumes(
        &self,
        ctx: &TenantContext,
        req: SubsumesRequest,
    ) -> Result<SubsumesResponse, HtsError>;
}

// traits/value_set.rs
#[async_trait]
pub trait ValueSetOperations: Send + Sync {
    async fn expand(
        &self,
        ctx: &TenantContext,
        req: ExpandRequest,
    ) -> Result<ExpandResponse, HtsError>;

    async fn validate_code(
        &self,
        ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError>;
}

// traits/concept_map.rs
#[async_trait]
pub trait ConceptMapOperations: Send + Sync {
    async fn translate(
        &self,
        ctx: &TenantContext,
        req: TranslateRequest,
    ) -> Result<TranslateResponse, HtsError>;

    async fn closure(
        &self,
        ctx: &TenantContext,
        req: ClosureRequest,
    ) -> Result<ClosureResponse, HtsError>;
}

// traits/metadata.rs
pub trait TerminologyMetadata: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn supported_systems(&self) -> Vec<String>;
    fn supports_subsumption(&self) -> bool;
}

// Combined backend bound (what the Axum state requires)
pub trait TerminologyBackend:
    CodeSystemOperations
    + ValueSetOperations
    + ConceptMapOperations
    + TerminologyMetadata
    + Send + Sync + 'static
{}
```

---

## Phase 3: SQLite Data Model

### Schema (normalized, not JSON blobs)

```sql
-- Code systems
CREATE TABLE code_systems (
    id          TEXT PRIMARY KEY,           -- CodeSystem.id
    url         TEXT NOT NULL UNIQUE,       -- CodeSystem.url
    version     TEXT,
    name        TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    content     TEXT NOT NULL DEFAULT 'complete', -- complete|fragment|not-present
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

-- Concepts
CREATE TABLE concepts (
    id          INTEGER PRIMARY KEY,
    system_id   TEXT NOT NULL REFERENCES code_systems(id) ON DELETE CASCADE,
    code        TEXT NOT NULL,
    display     TEXT,
    definition  TEXT,
    UNIQUE(system_id, code)
);
CREATE INDEX idx_concepts_system_code ON concepts(system_id, code);

-- Hierarchy (pre-materialized parent-child links)
CREATE TABLE concept_hierarchy (
    system_id   TEXT NOT NULL REFERENCES code_systems(id) ON DELETE CASCADE,
    parent_code TEXT NOT NULL,
    child_code  TEXT NOT NULL,
    PRIMARY KEY (system_id, parent_code, child_code)
);
CREATE INDEX idx_hierarchy_child ON concept_hierarchy(system_id, child_code);

-- Concept properties (arbitrary FHIR properties)
CREATE TABLE concept_properties (
    id          INTEGER PRIMARY KEY,
    concept_id  INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    property    TEXT NOT NULL,
    value_type  TEXT NOT NULL,  -- code|string|boolean|integer|decimal|dateTime
    value       TEXT NOT NULL
);

-- Designations (alternate names / translations)
CREATE TABLE concept_designations (
    id          INTEGER PRIMARY KEY,
    concept_id  INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    language    TEXT,
    use_system  TEXT,
    use_code    TEXT,
    value       TEXT NOT NULL
);

-- Value sets
CREATE TABLE value_sets (
    id          TEXT PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,
    version     TEXT,
    name        TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    compose_json TEXT,                      -- raw compose element (FHIR JSON)
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

-- Pre-expanded value sets (materialized cache)
CREATE TABLE value_set_expansions (
    value_set_id TEXT NOT NULL REFERENCES value_sets(id) ON DELETE CASCADE,
    system_url   TEXT NOT NULL,
    code         TEXT NOT NULL,
    display      TEXT,
    PRIMARY KEY (value_set_id, system_url, code)
);

-- Concept maps
CREATE TABLE concept_maps (
    id          TEXT PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,
    version     TEXT,
    source_uri  TEXT,
    target_uri  TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    created_at  TEXT NOT NULL
);

-- Concept map groups
CREATE TABLE concept_map_elements (
    id          INTEGER PRIMARY KEY,
    map_id      TEXT NOT NULL REFERENCES concept_maps(id) ON DELETE CASCADE,
    source_system TEXT NOT NULL,
    source_code TEXT NOT NULL,
    target_system TEXT NOT NULL,
    target_code TEXT NOT NULL,
    equivalence TEXT NOT NULL DEFAULT 'equivalent'
);
CREATE INDEX idx_map_source ON concept_map_elements(map_id, source_system, source_code);
```

**Notes:**
- Hierarchy is pre-materialized at import time — no recursive CTE needed for `$subsumes` in MVP
- Value set expansions are computed lazily on first `$expand` call and cached
- PostgreSQL upgrade path: same schema, swap `TEXT` PKs and GIN indexes

---

## Phase 4: HTTP API

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `HTS_SERVER_PORT` | `8090` | Server port |
| `HTS_SERVER_HOST` | `127.0.0.1` | Bind host |
| `HTS_LOG_LEVEL` | `info` | Log level |
| `HTS_DATABASE_URL` | `./data/hts.db` | SQLite path (or PG connection string) |
| `HTS_STORAGE_BACKEND` | `sqlite` | `sqlite` \| `postgres` (future) |
| `HTS_ENABLE_CORS` | `true` | CORS |
| `HTS_CORS_ORIGINS` | `*` | Allowed origins |

### Routes (Axum)

```
GET  /health                                    → health check
GET  /metadata                                  → TerminologyCapabilities

POST /CodeSystem/$lookup                        → $lookup
POST /CodeSystem/$validate-code                 → $validate-code (url)
POST /CodeSystem/$subsumes                      → $subsumes

POST /ValueSet/$expand                          → $expand
POST /ValueSet/$validate-code                   → $validate-code (value set)

POST /ConceptMap/$translate                     → $translate
POST /ConceptMap/$closure                       → $closure

# Resource CRUD (delegates to helios-persistence ResourceStorage)
GET    /CodeSystem/:id
POST   /CodeSystem
PUT    /CodeSystem/:id
DELETE /CodeSystem/:id

GET    /ValueSet/:id
POST   /ValueSet
PUT    /ValueSet/:id
DELETE /ValueSet/:id

GET    /ConceptMap/:id
POST   /ConceptMap
PUT    /ConceptMap/:id
DELETE /ConceptMap/:id
```

All operation endpoints accept `application/fhir+json` request bodies (FHIR Parameters resource).

---

## Phase 5: Import Pipeline

```rust
#[async_trait]
pub trait Importer: Send + Sync {
    fn name(&self) -> &'static str;
    fn can_handle(&self, content_type: &str) -> bool;
    async fn import(
        &self,
        backend: &dyn TerminologyBackend,
        ctx: &TenantContext,
        data: &[u8],
    ) -> Result<ImportStats, HtsError>;
}

pub struct ImportStats {
    pub code_systems: u32,
    pub value_sets: u32,
    pub concept_maps: u32,
    pub concepts: u32,
    pub errors: Vec<String>,
}
```

**Priority:**
- **P0 (MVP):** `FhirBundleImporter` — handles `application/fhir+json` bundles containing CodeSystem/ValueSet/ConceptMap resources
- **P1 (post-MVP):** SNOMED CT RF2 (`SnomedRf2Importer`), LOINC CSV (`LoincCsvImporter`)

**Import endpoint:**
```
POST /import
Content-Type: application/fhir+json   → FHIR Bundle
Content-Type: application/zip          → SNOMED RF2 or LOINC zip (P1)
```

---

## Phase 6: HFS Integration

Configure via `HFS_TERMINOLOGY_SERVER=http://localhost:8090` in HFS.

Integration points (deferred to post-MVP):
- FHIRPath `memberOf()` → `POST /ValueSet/$validate-code`
- FHIRPath `subsumes()` → `POST /CodeSystem/$subsumes`
- FHIR search `:in`, `:not-in`, `:below`, `:above` → `POST /ValueSet/$expand`

---

## Implementation Order

| Phase | Deliverable | Key files |
|---|---|---|
| 1 | Crate scaffold, config, health endpoint | `main.rs`, `config.rs`, `server.rs` |
| 2 | SQLite schema + migrations | `backend/sqlite/schema.rs` |
| 3 | `$lookup` + `$validate-code` (CodeSystem) | `operations/lookup.rs`, `validate_code.rs` |
| 4 | `$subsumes` | `operations/subsumes.rs`, `backend/sqlite/code_system.rs` |
| 5 | FHIR Bundle importer | `import/fhir_bundle.rs` |
| 6 | `$expand` + ValueSet `$validate-code` | `operations/expand.rs`, `backend/sqlite/value_set.rs` |
| 7 | `$translate` + `$closure` | `operations/translate.rs`, `closure.rs`, `backend/sqlite/concept_map.rs` |
| 8 | Resource CRUD (`/CodeSystem`, `/ValueSet`, `/ConceptMap`) | delegates to `ResourceStorage` |
| 9 | `/metadata` TerminologyCapabilities | `operations/` |
| 10 | HFS integration wiring | `helios-hfs` + `helios-fhirpath` |

---

## Scalability Hooks (Pre-wired, Not Implemented)

These are designed in but not built for MVP:

| Hook | How it's pre-wired |
|---|---|
| Multi-tenancy | `TenantContext` param on every trait method (currently always `default`) |
| PostgreSQL backend | Feature flag `postgres`, same trait impl, recursive CTEs replace pre-materialized hierarchy |
| Oxigraph/RDF backend | Implement `TerminologyBackend` for `OxigraphBackend` |
| Background expansion scheduler | `ValueSetOperations::schedule_expand()` stub in trait, noop default |
| SNOMED expression support | `LookupRequest.expression: Option<String>` field, returns `NotSupported` in SQLite MVP |
| Composite backend | Same `CompositeStorage` pattern as `helios-persistence` |

---

## Decision Report

### D1 — Single crate `helios-hts` vs. splitting traits into `helios-hts-core`

**Decision:** Single crate for MVP.
**Why:** The existing codebase keeps traits and implementations together in the same crate (e.g., `helios-persistence` has both traits and SQLite/PG/ES impls). Splitting prematurely adds workspace complexity for no current consumer. When a second backend appears, the split is straightforward.

---

### D2 — Reuse `ResourceStorage` from `helios-persistence` vs. new CRUD traits

**Decision:** Reuse `ResourceStorage` (via `helios-persistence` dependency) for CodeSystem/ValueSet/ConceptMap CRUD.
**Why:** The existing trait already handles create/read/update/delete, versioning, and soft deletes. Reinventing it would diverge behavior. The normalized terminology schema lives in the new crate; `ResourceStorage` is only used for resource-level CRUD, not for terminology-specific operations.

---

### D3 — SQLite as the only MVP backend

**Decision:** SQLite only for MVP.
**Why:** Every other HFS server starts with SQLite as default for zero-config operation. The trait design ensures PostgreSQL can be dropped in without changing the HTTP layer. Recursive CTEs (needed for SNOMED's polyhierarchy at scale) are available in SQLite too — they're just not needed until SNOMED RF2 import is implemented.

---

### D4 — Pre-materialized hierarchy vs. recursive CTE

**Decision:** Pre-materialized `concept_hierarchy` table (parent/child pairs stored at import time).
**Why:** For MVP, imported code systems are small (FHIR spec examples, custom systems). Pre-materializing at import time makes `$subsumes` O(1) lookups with no recursive query. When SNOMED (350,000+ concepts, polyhierarchy) is imported in a later phase, the trade-off can be re-evaluated — either keep pre-materialization (fast reads, expensive import) or switch to recursive CTEs (slower reads, fast import).

---

### D5 — Lazy value set expansion vs. background scheduler

**Decision:** Lazy (expand on first `$expand` call, cache result in `value_set_expansions`).
**Why:** The discussion proposed a threshold-based background scheduler at 10,000 concepts. That adds scheduler complexity (tokio task, cancellation, status tracking) with no benefit for MVP where code systems are small. The cache table is already in the schema — upgrading to a background scheduler later just means populating the same table proactively.

---

### D6 — Single-tenant for now

**Decision:** `TenantContext` is always `TenantContext::default()` in MVP; all trait methods accept it as a parameter.
**Why:** The user explicitly requested single-tenant. Passing `TenantContext` through every method ensures no API surface needs to change when multi-tenancy is introduced — it's a pure implementation detail. This mirrors how `helios-persistence` enforces tenant isolation at the type level.

---

### D7 — SNOMED post-coordination out of MVP scope

**Decision:** `$lookup` accepts an `expression` field but returns `HtsError::NotSupported` if present.
**Why:** Post-coordinated SNOMED expressions (e.g., `128045006:{363698007=56459004}`) require an expression parser and compositional subsumption algorithm. This is a significant scope addition. Stubbing it with `NotSupported` keeps the door open without blocking MVP delivery.

---

### D8 — Write API (CREATE/UPDATE/DELETE) included in MVP

**Decision:** Expose resource CRUD for CodeSystem, ValueSet, ConceptMap.
**Why:** Without a write API, the only way to populate HTS is via the `/import` endpoint. Custom code systems (e.g., a hospital's internal procedure codes) are a primary use case and cannot wait for a future release.

---

### D9 — Generic `AppState<B: TerminologyBackend>` vs `dyn TerminologyBackend`

**Decision:** Generic bound (`AppState<B: TerminologyBackend>`), monomorphized at compile time.
**Why:** Zero runtime overhead; matches the pattern used in `helios-rest` and `helios-persistence`. Runtime backend selection (SQLite vs. PostgreSQL) is handled at the `main.rs` dispatch level rather than through a boxed trait object. Switching to `dyn` later would require changing every handler signature — staying generic keeps that door open without forcing the choice now.

---

### D10 — `url` only (no `system` alias) in `CodeSystem/$validate-code`

**Decision:** The `url` parameter is required, per FHIR R4/R5 spec. The `system` parameter is rejected with HTTP 400.
**Why:** `system` is not a defined parameter for `CodeSystem/$validate-code` in either R4 or R5. Accepting it silently would mask client bugs and create an undocumented divergence from spec. Clients that send `system` receive a 400 with an error message directing them to use `url`. (Note: `system` *is* a valid parameter in `ValueSet/$validate-code`, which is a different operation.)

---

### D11 — FHIR Bundle importer (Phase 5) before `$expand`/`$translate` (Phases 6–7)

**Decision:** Move the import pipeline to Phase 5, immediately after `$subsumes`.
**Why:** Without the `POST /import` endpoint, Phases 6 and 7 can only be tested by manually seeding SQLite. Moving import up makes the service self-contained for end-to-end testing from Phase 6 onward, reducing development friction.

**Decision:** Expose resource CRUD for CodeSystem, ValueSet, ConceptMap.
**Why:** Without a write API, the only way to populate HTS is via the `/import` endpoint. Custom code systems (e.g., a hospital's internal procedure codes) are a primary use case and cannot wait for a future release.

---

## Verification

```bash
# Build
cargo build -p helios-hts

# Run
cargo run --bin hts

# Health check
curl http://localhost:8090/health

# Import a small FHIR Bundle with a CodeSystem
curl -X POST http://localhost:8090/import \
  -H "Content-Type: application/fhir+json" \
  -d @test-bundle.json

# Lookup a concept
curl -X POST http://localhost:8090/CodeSystem/\$lookup \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters","parameter":[{"name":"url","valueUri":"http://example.org/cs"},{"name":"code","valueCode":"ABC"}]}'

# Validate a code
curl -X POST http://localhost:8090/CodeSystem/\$validate-code \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters","parameter":[{"name":"url","valueUri":"http://example.org/cs"},{"name":"code","valueCode":"ABC"}]}'

# Expand a value set
curl -X POST http://localhost:8090/ValueSet/\$expand \
  -H "Content-Type: application/fhir+json" \
  -d '{"resourceType":"Parameters","parameter":[{"name":"url","valueUri":"http://example.org/vs"}]}'

# Tests
cargo test -p helios-hts
```
