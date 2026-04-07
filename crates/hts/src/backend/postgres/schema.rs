//! PostgreSQL DDL for the HTS schema.
//!
//! Uses `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` throughout
//! so this is safe to re-apply on every startup (idempotent).
//!
//! Key differences from the SQLite schema:
//! - `INTEGER PRIMARY KEY` → `BIGSERIAL PRIMARY KEY` for auto-increment tables
//! - `resource_json TEXT` → `resource_json JSONB`
//! - `title TEXT` and `resource_json JSONB` are part of the base schema (no migrations)
//! - `name TEXT` is included in `concept_maps` from the start
//! - Trigram GIN index on `concepts.display` for fast substring search

#[cfg(feature = "postgres")]
/// Full PostgreSQL DDL for the HTS normalized schema.
pub const SCHEMA: &str = "
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ── Code Systems ──────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS code_systems (
    id           TEXT PRIMARY KEY,
    url          TEXT NOT NULL UNIQUE,
    version      TEXT,
    name         TEXT,
    title        TEXT,
    status       TEXT NOT NULL DEFAULT 'active',
    content      TEXT NOT NULL DEFAULT 'complete',
    resource_json JSONB,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

-- ── Concepts ───────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS concepts (
    id         BIGSERIAL PRIMARY KEY,
    system_id  TEXT NOT NULL REFERENCES code_systems(id) ON DELETE CASCADE,
    code       TEXT NOT NULL,
    display    TEXT,
    definition TEXT,
    UNIQUE (system_id, code)
);
CREATE INDEX IF NOT EXISTS idx_concepts_system_code ON concepts(system_id, code);
CREATE INDEX IF NOT EXISTS idx_concepts_display_trgm ON concepts USING gin(display gin_trgm_ops);

-- ── Hierarchy (pre-materialized parent-child links) ───────────────────────────
CREATE TABLE IF NOT EXISTS concept_hierarchy (
    system_id   TEXT NOT NULL REFERENCES code_systems(id) ON DELETE CASCADE,
    parent_code TEXT NOT NULL,
    child_code  TEXT NOT NULL,
    PRIMARY KEY (system_id, parent_code, child_code)
);
CREATE INDEX IF NOT EXISTS idx_hierarchy_child ON concept_hierarchy(system_id, child_code);

-- ── Concept Properties (arbitrary FHIR properties) ────────────────────────────
CREATE TABLE IF NOT EXISTS concept_properties (
    id         BIGSERIAL PRIMARY KEY,
    concept_id BIGINT NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    property   TEXT NOT NULL,
    value_type TEXT NOT NULL,
    value      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_props_concept ON concept_properties(concept_id);

-- ── Designations (alternate names / translations) ─────────────────────────────
CREATE TABLE IF NOT EXISTS concept_designations (
    id         BIGSERIAL PRIMARY KEY,
    concept_id BIGINT NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    language   TEXT,
    use_system TEXT,
    use_code   TEXT,
    value      TEXT NOT NULL
);

-- ── Value Sets ─────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS value_sets (
    id            TEXT PRIMARY KEY,
    url           TEXT NOT NULL UNIQUE,
    version       TEXT,
    name          TEXT,
    title         TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    compose_json  TEXT,
    resource_json JSONB,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

-- ── Value Set Expansions (materialized cache) ─────────────────────────────────
CREATE TABLE IF NOT EXISTS value_set_expansions (
    value_set_id TEXT NOT NULL REFERENCES value_sets(id) ON DELETE CASCADE,
    system_url   TEXT NOT NULL,
    code         TEXT NOT NULL,
    display      TEXT,
    PRIMARY KEY (value_set_id, system_url, code)
);

-- ── Concept Maps ───────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS concept_maps (
    id            TEXT PRIMARY KEY,
    url           TEXT NOT NULL UNIQUE,
    version       TEXT,
    name          TEXT,
    title         TEXT,
    source_uri    TEXT,
    target_uri    TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    resource_json JSONB,
    created_at    TEXT NOT NULL
);

-- ── Concept Map Elements ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS concept_map_elements (
    id            BIGSERIAL PRIMARY KEY,
    map_id        TEXT NOT NULL REFERENCES concept_maps(id) ON DELETE CASCADE,
    source_system TEXT NOT NULL,
    source_code   TEXT NOT NULL,
    target_system TEXT NOT NULL,
    target_code   TEXT NOT NULL,
    equivalence   TEXT NOT NULL DEFAULT 'equivalent'
);
CREATE INDEX IF NOT EXISTS idx_map_source
    ON concept_map_elements(map_id, source_system, source_code);
";

/// Apply the HTS PostgreSQL schema to the given client connection.
///
/// Safe to call on every startup — all statements are idempotent.
#[cfg(feature = "postgres")]
pub async fn apply(client: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    client.batch_execute(SCHEMA).await
}
