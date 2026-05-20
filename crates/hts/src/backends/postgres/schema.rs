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

/// Full PostgreSQL DDL for the HTS normalized schema.
pub const SCHEMA: &str = "
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ── Code Systems ──────────────────────────────────────────────────────────────
-- Multi-version: a canonical URL may have multiple rows so long as each row has
-- a distinct `version`. Uniqueness is enforced via the composite expression
-- index `idx_code_systems_url_version` (the column-level UNIQUE on `url` was
-- dropped). Rows with NULL version are coalesced to the empty string by the
-- index so two un-versioned imports of the same URL still collide.
CREATE TABLE IF NOT EXISTS code_systems (
    id           TEXT PRIMARY KEY,
    url          TEXT NOT NULL,
    version      TEXT,
    name         TEXT,
    title        TEXT,
    status       TEXT NOT NULL DEFAULT 'active',
    content      TEXT NOT NULL DEFAULT 'complete',
    resource_json JSONB,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_code_systems_url_version
    ON code_systems(url, COALESCE(version, ''));
CREATE INDEX IF NOT EXISTS idx_code_systems_url ON code_systems(url);

-- Legacy installs created code_systems with `url TEXT NOT NULL UNIQUE`, which
-- bakes uniqueness into a hidden constraint that blocks multi-version imports.
-- Drop both the standalone constraint name and the auto-named one so subsequent
-- (url, version) duplicates can coexist; both forms are silently ignored when
-- absent (legacy index is unnamed across PG versions).
DO $$
DECLARE
    cons_name text;
BEGIN
    FOR cons_name IN
        SELECT conname FROM pg_constraint c
        JOIN pg_class t ON t.oid = c.conrelid
        WHERE t.relname = 'code_systems'
          AND c.contype = 'u'
          AND pg_get_constraintdef(c.oid) = 'UNIQUE (url)'
    LOOP
        EXECUTE format('ALTER TABLE code_systems DROP CONSTRAINT %I', cons_name);
    END LOOP;
END $$;

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
-- Forward lookup: concepts carrying a given (property, value) pair. Drives
-- compose `filter[]` property-equality expansion (EX05's `=` / `in` filters).
-- Without this index `pg_filter_property_eq` had to scan every concept in the
-- system (~350k for SNOMED) and probe `concept_properties` per row. The
-- trailing `concept_id` lets the (property, value) probe stay index-only.
CREATE INDEX IF NOT EXISTS idx_props_property_value
    ON concept_properties(property, value, concept_id);

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
    url           TEXT NOT NULL,
    version       TEXT,
    name          TEXT,
    title         TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    compose_json  TEXT,
    resource_json JSONB,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_value_sets_url_version
    ON value_sets(url, COALESCE(version, ''));
CREATE INDEX IF NOT EXISTS idx_value_sets_url ON value_sets(url);

-- Drop any legacy column-level UNIQUE on value_sets.url for the same reason
-- code_systems above had it removed: tx-ecosystem ships per-version VS
-- fixtures sharing a canonical URL.
DO $$
DECLARE
    cons_name text;
BEGIN
    FOR cons_name IN
        SELECT conname FROM pg_constraint c
        JOIN pg_class t ON t.oid = c.conrelid
        WHERE t.relname = 'value_sets'
          AND c.contype = 'u'
          AND pg_get_constraintdef(c.oid) = 'UNIQUE (url)'
    LOOP
        EXECUTE format('ALTER TABLE value_sets DROP CONSTRAINT %I', cons_name);
    END LOOP;
END $$;

-- ── Value Set Expansions (materialized cache) ─────────────────────────────────
-- `version` carries the resolved CodeSystem version for each cached entry so
-- that callers reading from the cache can echo the right CS version on
-- validate-code and so the version-pinned overload candidate-selection branch
-- has the data it needs to discriminate. The PRIMARY KEY excludes `version`
-- because PG PK columns must be NOT NULL — multi-version overload composes
-- bypass the cache (`compose_has_multi_version_pins`) instead of relying on PK
-- dedupe behaviour.
CREATE TABLE IF NOT EXISTS value_set_expansions (
    value_set_id TEXT NOT NULL REFERENCES value_sets(id) ON DELETE CASCADE,
    system_url   TEXT NOT NULL,
    code         TEXT NOT NULL,
    display      TEXT,
    version      TEXT,
    PRIMARY KEY (value_set_id, system_url, code)
);
-- Legacy installs may have a value_set_expansions table without the `version`
-- column; add it idempotently so upgrades round-trip.
ALTER TABLE value_set_expansions ADD COLUMN IF NOT EXISTS version TEXT;

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

-- ── Transitive ancestor closure ────────────────────────────────────────────────
-- Precomputed (ancestor, descendant) pairs for every code system, including
-- self-links (code, code). Populated at import time for each code system so
-- is-a, descendent-of, generalizes, and $subsumes queries are O(1) index
-- lookups rather than O(depth) recursive CTEs. Mirrors the SQLite
-- concept_closure table at backends/sqlite/schema.rs.
CREATE TABLE IF NOT EXISTS concept_closure (
    system_id       TEXT NOT NULL REFERENCES code_systems(id) ON DELETE CASCADE,
    ancestor_code   TEXT NOT NULL,
    descendant_code TEXT NOT NULL,
    PRIMARY KEY (system_id, ancestor_code, descendant_code)
);
-- Reverse lookup: all ancestors of a given descendant code.
CREATE INDEX IF NOT EXISTS idx_closure_descendant
    ON concept_closure(system_id, descendant_code);
";

/// Apply the HTS PostgreSQL schema to the given client connection.
///
/// Safe to call on every startup — all statements are idempotent. Concurrent
/// callers are serialized via a session-scoped advisory lock so that racing
/// `CREATE EXTENSION IF NOT EXISTS pg_trgm` calls cannot collide on
/// `pg_extension_name_index` (Postgres's `IF NOT EXISTS` guard is not
/// transactional with concurrent writers).
pub async fn apply(client: &tokio_postgres::Client) -> Result<(), tokio_postgres::Error> {
    // Arbitrary constant — must match across all callers that apply this
    // schema. Use an application-scoped value that won't collide with
    // user-chosen advisory locks.
    const HTS_SCHEMA_LOCK: i64 = 0x4854_535f_5343_484du64 as i64; // "HTS_SCHM"

    client
        .execute("SELECT pg_advisory_lock($1)", &[&HTS_SCHEMA_LOCK])
        .await?;
    let result = client.batch_execute(SCHEMA).await;
    // Always release the lock, even if schema apply failed.
    let _ = client
        .execute("SELECT pg_advisory_unlock($1)", &[&HTS_SCHEMA_LOCK])
        .await;
    result
}

/// Build (or rebuild) the transitive ancestor closure for one code system.
///
/// Deletes any existing closure rows for `system_id`, then recomputes the
/// full set of `(ancestor, descendant)` pairs — including self-links
/// `(code, code)`.
///
/// Uses a Rust-based BFS rather than a recursive SQL CTE. The CTE approach
/// requires Postgres to maintain a working-table that grows to O(closure_size)
/// rows — for SNOMED CT (~20M pairs) the CTE alone takes minutes. The BFS
/// processes each concept once using an integer generation counter for O(1)
/// visited-reset, completing the same work in tens of seconds.
///
/// Inserts are batched via UNNEST (parallel arrays) to amortise per-statement
/// roundtrip cost. All inserts share a single transaction so the ~20M rows for
/// SNOMED CT commit in one WAL flush.
///
/// Mirrors `backends::sqlite::schema::build_concept_closure`.
pub async fn build_concept_closure_pg(
    client: &mut tokio_postgres::Client,
    system_id: &str,
) -> Result<(), tokio_postgres::Error> {
    use std::collections::{HashMap, VecDeque};

    // Load all concept codes for this system.
    let concepts: Vec<String> = client
        .query(
            "SELECT code FROM concepts WHERE system_id = $1",
            &[&system_id],
        )
        .await?
        .into_iter()
        .map(|r| r.get::<_, String>(0))
        .collect();

    if concepts.is_empty() {
        client
            .execute(
                "DELETE FROM concept_closure WHERE system_id = $1",
                &[&system_id],
            )
            .await?;
        return Ok(());
    }

    // Map code string → index so we work with usize everywhere (no string
    // clones inside the hot BFS loop).
    let code_to_idx: HashMap<&str, usize> = concepts
        .iter()
        .enumerate()
        .map(|(i, c)| (c.as_str(), i))
        .collect();

    // Build per-node children lists (index-based).
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); concepts.len()];
    let rows = client
        .query(
            "SELECT parent_code, child_code FROM concept_hierarchy WHERE system_id = $1",
            &[&system_id],
        )
        .await?;
    for row in &rows {
        let parent: &str = row.get(0);
        let child: &str = row.get(1);
        if let (Some(&pi), Some(&ci)) = (code_to_idx.get(parent), code_to_idx.get(child)) {
            children[pi].push(ci);
        }
    }
    drop(rows);

    // BFS from every concept to enumerate descendants. Batch the resulting
    // pairs in chunks of `BATCH` before flushing to PG via UNNEST.
    const BATCH: usize = 50_000;
    let mut anc_batch: Vec<&str> = Vec::with_capacity(BATCH);
    let mut des_batch: Vec<&str> = Vec::with_capacity(BATCH);

    let tx = client.transaction().await?;
    tx.execute(
        "DELETE FROM concept_closure WHERE system_id = $1",
        &[&system_id],
    )
    .await?;

    let insert_sql =
        "INSERT INTO concept_closure (system_id, ancestor_code, descendant_code)
         SELECT $1, anc, des
         FROM UNNEST($2::text[], $3::text[]) AS t(anc, des)
         ON CONFLICT DO NOTHING";

    let mut visit_gen: Vec<u32> = vec![0; concepts.len()];
    let mut queue: VecDeque<usize> = VecDeque::new();

    for anc_idx in 0..concepts.len() {
        let g = (anc_idx as u32) + 1;

        queue.clear();
        queue.push_back(anc_idx);

        while let Some(idx) = queue.pop_front() {
            if visit_gen[idx] == g {
                continue;
            }
            visit_gen[idx] = g;

            anc_batch.push(concepts[anc_idx].as_str());
            des_batch.push(concepts[idx].as_str());

            if anc_batch.len() >= BATCH {
                tx.execute(insert_sql, &[&system_id, &anc_batch, &des_batch])
                    .await?;
                anc_batch.clear();
                des_batch.clear();
            }

            for &ci in &children[idx] {
                if visit_gen[ci] != g {
                    queue.push_back(ci);
                }
            }
        }
    }

    if !anc_batch.is_empty() {
        tx.execute(insert_sql, &[&system_id, &anc_batch, &des_batch])
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Populate `concept_closure` for every code system that has hierarchy edges
/// but no closure rows yet. Mirrors `migrate_concept_closure` for SQLite.
///
/// Called once at server startup so existing databases (imported before the
/// closure table was introduced) are migrated automatically, and at the end of
/// CLI imports so the server's first request doesn't pay the build cost.
///
/// Drives the search off the small `code_systems` table (typically <500 rows)
/// with index-friendly EXISTS probes. A previous version used `SELECT DISTINCT
/// h.system_id FROM concept_hierarchy ...`, which the PG planner converted to
/// a parallel sequential scan of the 1.2M-row SNOMED hierarchy and exhausted
/// the dynamic-shared-memory segment on CI containers with a 64 MB `/dev/shm`
/// — even when no closure work was actually needed.
pub async fn migrate_concept_closure_pg(
    client: &mut tokio_postgres::Client,
) -> Result<(), tokio_postgres::Error> {
    let systems_needing_closure: Vec<String> = client
        .query(
            "SELECT cs.id FROM code_systems cs
             WHERE EXISTS (
                       SELECT 1 FROM concept_hierarchy h
                       WHERE  h.system_id = cs.id
                       LIMIT  1
                   )
               AND NOT EXISTS (
                       SELECT 1 FROM concept_closure c
                       WHERE  c.system_id = cs.id
                       LIMIT  1
                   )",
            &[],
        )
        .await?
        .into_iter()
        .map(|r| r.get::<_, String>(0))
        .collect();

    for sid in &systems_needing_closure {
        build_concept_closure_pg(client, sid).await?;
    }
    Ok(())
}
