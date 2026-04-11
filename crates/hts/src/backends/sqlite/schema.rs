/// SQL DDL for the HTS SQLite schema.
///
/// All statements use `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS`
/// so this can be applied safely on every startup without error.
pub const SCHEMA: &str = "
-- ── Code Systems ──────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS code_systems (
    id          TEXT PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,
    version     TEXT,
    name        TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    content     TEXT NOT NULL DEFAULT 'complete',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

-- ── Concepts ───────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS concepts (
    id          INTEGER PRIMARY KEY,
    system_id   TEXT NOT NULL REFERENCES code_systems(id) ON DELETE CASCADE,
    code        TEXT NOT NULL,
    display     TEXT,
    definition  TEXT,
    UNIQUE(system_id, code)
);
CREATE INDEX IF NOT EXISTS idx_concepts_system_code ON concepts(system_id, code);

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
    id          INTEGER PRIMARY KEY,
    concept_id  INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    property    TEXT NOT NULL,
    value_type  TEXT NOT NULL,
    value       TEXT NOT NULL
);

-- ── Designations (alternate names / translations) ─────────────────────────────
CREATE TABLE IF NOT EXISTS concept_designations (
    id          INTEGER PRIMARY KEY,
    concept_id  INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    language    TEXT,
    use_system  TEXT,
    use_code    TEXT,
    value       TEXT NOT NULL
);

-- ── Value Sets ─────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS value_sets (
    id          TEXT PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,
    version     TEXT,
    name        TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    compose_json TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
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
    id          TEXT PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,
    version     TEXT,
    source_uri  TEXT,
    target_uri  TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    created_at  TEXT NOT NULL
);

-- ── Concept Map Elements ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS concept_map_elements (
    id            INTEGER PRIMARY KEY,
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

/// Apply the HTS schema to the given database connection.
///
/// Safe to call on every startup — all statements are idempotent.
pub fn apply(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

/// Add search-related columns to the existing tables.
///
/// `title` and `resource_json` are added to all three resource tables.
/// `name` is added to `concept_maps` (it was absent from the original schema).
///
/// Uses `ALTER TABLE … ADD COLUMN` and silently ignores
/// "duplicate column name" errors so this is safe to run on every startup.
pub fn migrate_search_columns(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let migrations = [
        "ALTER TABLE code_systems ADD COLUMN title TEXT",
        "ALTER TABLE code_systems ADD COLUMN resource_json TEXT",
        "ALTER TABLE value_sets ADD COLUMN title TEXT",
        "ALTER TABLE value_sets ADD COLUMN resource_json TEXT",
        "ALTER TABLE concept_maps ADD COLUMN name TEXT",
        "ALTER TABLE concept_maps ADD COLUMN title TEXT",
        "ALTER TABLE concept_maps ADD COLUMN resource_json TEXT",
    ];
    for sql in &migrations {
        match conn.execute_batch(sql) {
            Ok(_) => {}
            // SQLite error 1 with "duplicate column name" means the column already
            // exists — skip silently so this migration is idempotent.
            Err(e) if e.to_string().contains("duplicate column name") => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_applies_to_in_memory_db() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply(&conn).expect("schema should apply without error");
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply(&conn).expect("first application should succeed");
        apply(&conn).expect("second application should also succeed (idempotent)");
    }

    #[test]
    fn all_nine_tables_exist_after_migration() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();

        let expected_tables = [
            "code_systems",
            "concepts",
            "concept_hierarchy",
            "concept_properties",
            "concept_designations",
            "value_sets",
            "value_set_expansions",
            "concept_maps",
            "concept_map_elements",
        ];

        for table in &expected_tables {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    rusqlite::params![table],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            assert_eq!(count, 1, "table '{table}' should exist after migration");
        }
    }

    #[test]
    fn foreign_key_cascade_works() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        // Insert a code system
        conn.execute(
            "INSERT INTO code_systems (id, url, status, content, created_at, updated_at)
             VALUES ('cs1', 'http://example.org/cs', 'active', 'complete', '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();

        // Insert a concept in that system
        conn.execute(
            "INSERT INTO concepts (system_id, code, display) VALUES ('cs1', 'A', 'Alpha')",
            [],
        )
        .unwrap();

        // Deleting the code system should cascade to concepts
        conn.execute("DELETE FROM code_systems WHERE id='cs1'", [])
            .unwrap();

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE system_id='cs1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "cascade delete should remove child concepts");
    }
}
