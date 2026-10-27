# MongoDB `search_index` Generation 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the nine full value indexes on MongoDB `search_index` with partial, `resource_id`-carrying generation-2 indexes built in the background after boot, add a partial contained index, and rewrite `_contained` search to page on the server.

**Architecture:** A declarative index catalog (`search_index_catalog.rs`) is the single source of truth for names, keys, partial filters and build class. Boot creates only inline-class indexes; a spawned `SearchIndexBuilder` task inspects `listIndexes`, issues one `createIndexes` for every missing generation-2 spec, drops the superseded v1 names once all are ready, and records the generation in the `schema_version` document. `_contained` search becomes one aggregation on the new partial index with `$skip`/`$limit` and a single batched container fetch.

**Tech Stack:** Rust 2024, `mongodb` driver 3.x (`Collection::create_index`, `Database::run_command`, `list_indexes`), `tokio::spawn`, `tracing`, `serde`. Tests: `cargo test -p helios-persistence --features mongodb`, testcontainers (skip silently without Docker: a skipped test is not a pass, quote the `ok` line).

**Spec:** `docs/superpowers/specs/2026-09-15-mongodb-search-index-v2-design.md`

## Global Constraints

- Never change the key spec or options of an existing index name. MongoDB answers `IndexKeySpecsConflict` (86) and boot fails on every deployed database. Generation 2 lives under new names (`_v2` suffix, plus `idx_search_contained`).
- `idx_search_composite` and `idx_search_resource` keep their exact keys and names; they are hinted by name (`search_impl.rs`, `Hint::Name("idx_search_composite")`) and used by reindex deletes.
- Every generation-2 value spec is partial on its leading value field (`{field: {$exists: true}}`) and ends in `resource_id`. The contained spec is partial on `{is_contained: true}`.
- `HFS_MONGODB_INDEX_BUILD` accepts exactly `background` (default), `inline`, `off`. Tests run in `inline`.
- The builder never touches indexes on collections other than `search_index`, and never drops anything unless every generation-2 spec is present and ready.
- `set_schema_version` must stop deleting the `schema_version` document (it currently does delete-then-insert, which would erase the `search_indexes` record on every boot).
- TDD per task: write the failing test, run it, see it fail for the expected reason, implement, run it green, run the file's full suite, `cargo fmt -p helios-persistence`, then the CI clippy command:
  `cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation`
- MongoDB integration tests: `cargo test -p helios-persistence --features mongodb --test mongodb_tests <filter>`. Docker must be running; the suite starts its own container. Never set `HFS_TEST_MONGODB_URL` to the `hfs-mongo` corpus container on port 27017.
- Every commit message ends with:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_013Uo4p2nXn3U3j54MyW4Aqd
  ```
- Work on branch `feat/1059-1084-search-index-v2`, cut from `origin/main`. Do not push; do not open a PR (the controller does).

---

## File structure

| file | responsibility |
|---|---|
| Create `crates/persistence/src/backends/mongodb/search_index_catalog.rs` | The catalog: `SearchIndexSpec`, `IndexBuild`, generation-2 specs, superseded v1 specs, `createIndexes` command generation, mongosh script text. Pure data, unit-tested without a server. |
| Create `crates/persistence/src/backends/mongodb/search_index_builder.rs` | `IndexBuildMode`, `SearchIndexBuilder` (inspect → build → drop → record), `BuildOutcome`. Talks to the server. |
| Modify `crates/persistence/src/backends/mongodb/schema.rs` | `ensure_search_indexes` creates only inline specs from the catalog; `set_schema_version` becomes an upsert; new `get/set_search_index_generation`; `drop_index_if_present` becomes `pub(super)`. |
| Modify `crates/persistence/src/backends/mongodb/backend.rs` | `MongoBackendConfig::index_build`, env parsing, `init_schema` spawns the builder, `wait_for_search_index_build`. |
| Modify `crates/persistence/src/backends/mongodb/mod.rs` | Declare the two new modules; re-export `IndexBuildMode`. |
| Modify `crates/persistence/src/backends/mongodb/storage.rs` | `document_to_stored_resource` becomes `pub(super)`. |
| Modify `crates/persistence/src/backends/mongodb/search_impl.rs` | `matching_contained` and `search_contained` rewritten. |
| Modify `crates/persistence/tests/mongodb_tests.rs` | Test helper sets `inline`; builder, explain and contained tests. |
| Create `docs/mongodb/search-index-v2.mongosh.js` | Operator pre-build script, byte-equal to the catalog output (unit-tested). |
| Create `docs/mongodb/search-index-v1-rollback.mongosh.js` | Recreates the v1 set before a downgrade, byte-equal to the catalog output. |
| Create `docs/mongodb/search-indexes.md` | Upgrade, modes, pre-build, downgrade. |
| Modify `README.md:324-328` | New env var row. |

---

### Task 1: Index catalog

**Files:**
- Create: `crates/persistence/src/backends/mongodb/search_index_catalog.rs`
- Create: `docs/mongodb/search-index-v2.mongosh.js`
- Create: `docs/mongodb/search-index-v1-rollback.mongosh.js`
- Modify: `crates/persistence/src/backends/mongodb/mod.rs`

**Interfaces:**
- Produces:
  - `pub(crate) enum IndexBuild { Inline, Background }`
  - `pub(crate) struct SearchIndexSpec { name: &'static str, keys: Document, partial: Option<Document>, build: IndexBuild }` with `fn index_model(&self) -> IndexModel` and `fn create_indexes_entry(&self) -> Document`
  - `pub(crate) const SEARCH_INDEX_COLLECTION: &str = "search_index"`
  - `pub(crate) const SEARCH_INDEX_GENERATION: i32 = 2`
  - `pub(crate) fn generation2_specs() -> Vec<SearchIndexSpec>` (12 specs: 9 value `_v2`, `idx_search_contained`, `idx_search_composite`, `idx_search_resource`)
  - `pub(crate) fn superseded_v1_specs() -> Vec<SearchIndexSpec>` (the 9 old value indexes, exact keys)
  - `pub(crate) fn create_indexes_command(specs: &[&SearchIndexSpec]) -> Document`
  - `pub(crate) fn mongosh_script(specs: &[SearchIndexSpec]) -> String`

- [ ] **Step 1: Write the failing unit tests**

Create `crates/persistence/src/backends/mongodb/search_index_catalog.rs` containing only the test module for now:

```rust
//! Declarative catalog of the `search_index` indexes (#1059, #1084).
//!
//! Generation 2 replaces the nine full value indexes with partial indexes that
//! carry `resource_id` as their trailing key, so a value-filtered scan can be
//! covered, and adds a partial index over contained rows. New names, never
//! changed keys: MongoDB refuses a different key spec under an existing name
//! (`IndexKeySpecsConflict`, 86), which would fail every deployed boot.

#[cfg(test)]
mod tests {
    use super::*;

    fn value_specs() -> Vec<SearchIndexSpec> {
        generation2_specs()
            .into_iter()
            .filter(|s| s.name.ends_with("_v2"))
            .collect()
    }

    #[test]
    fn generation2_has_nine_value_specs_plus_contained_plus_two_unchanged() {
        let names: Vec<&str> = generation2_specs().iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "idx_search_string_v2",
                "idx_search_token_v2",
                "idx_search_date_v2",
                "idx_search_number_v2",
                "idx_search_quantity_v2",
                "idx_search_reference_v2",
                "idx_search_uri_v2",
                "idx_search_token_display_v2",
                "idx_search_identifier_type_v2",
                "idx_search_contained",
                "idx_search_composite",
                "idx_search_resource",
            ]
        );
    }

    #[test]
    fn every_value_spec_is_partial_on_its_leading_value_key_and_ends_in_resource_id() {
        for spec in value_specs() {
            let keys: Vec<&str> = spec.keys.keys().map(String::as_str).collect();
            assert_eq!(&keys[..3], &["tenant_id", "resource_type", "param_name"], "{}", spec.name);
            assert_eq!(keys.last(), Some(&"resource_id"), "{}", spec.name);
            let leading_value = keys[3];
            let partial = spec.partial.as_ref().unwrap_or_else(|| panic!("{} has no partial filter", spec.name));
            assert_eq!(
                partial,
                &doc! { leading_value: { "$exists": true } },
                "{} partial filter must be on its leading value key",
                spec.name
            );
            assert_eq!(spec.build, IndexBuild::Background, "{}", spec.name);
        }
    }

    #[test]
    fn token_v2_is_code_first() {
        let token = generation2_specs().into_iter().find(|s| s.name == "idx_search_token_v2").unwrap();
        let keys: Vec<&str> = token.keys.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["tenant_id", "resource_type", "param_name", "value_token_code", "value_token_system", "resource_id"]
        );
    }

    #[test]
    fn contained_spec_is_partial_on_is_contained_true() {
        let c = generation2_specs().into_iter().find(|s| s.name == "idx_search_contained").unwrap();
        let keys: Vec<&str> = c.keys.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["tenant_id", "contained_type", "is_contained", "param_name", "resource_type", "resource_id", "contained_local_id"]
        );
        assert_eq!(c.partial, Some(doc! { "is_contained": true }));
        assert_eq!(c.build, IndexBuild::Background);
    }

    #[test]
    fn composite_and_resource_are_unchanged_and_inline() {
        let specs = generation2_specs();
        let composite = specs.iter().find(|s| s.name == "idx_search_composite").unwrap();
        assert_eq!(
            composite.keys,
            doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "resource_id": 1_i32, "param_name": 1_i32, "composite_group": 1_i32 }
        );
        assert_eq!(composite.partial, None);
        assert_eq!(composite.build, IndexBuild::Inline);
        let resource = specs.iter().find(|s| s.name == "idx_search_resource").unwrap();
        assert_eq!(resource.keys, doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "resource_id": 1_i32 });
        assert_eq!(resource.partial, None);
        assert_eq!(resource.build, IndexBuild::Inline);
    }

    #[test]
    fn superseded_v1_names_are_exactly_the_nine_old_value_indexes_and_do_not_collide() {
        let v1: Vec<&str> = superseded_v1_specs().iter().map(|s| s.name).collect();
        assert_eq!(
            v1,
            vec![
                "idx_search_string",
                "idx_search_token",
                "idx_search_date",
                "idx_search_number",
                "idx_search_quantity",
                "idx_search_reference",
                "idx_search_uri",
                "idx_search_token_display",
                "idx_search_identifier_type",
            ]
        );
        let g2: Vec<&str> = generation2_specs().iter().map(|s| s.name).collect();
        for name in &v1 {
            assert!(!g2.contains(name), "{name} is both superseded and generation 2");
        }
        // The old token index is system-first; pin it so the rollback script is faithful.
        let token = superseded_v1_specs().into_iter().find(|s| s.name == "idx_search_token").unwrap();
        let keys: Vec<&str> = token.keys.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["tenant_id", "resource_type", "param_name", "value_token_system", "value_token_code"]);
    }

    #[test]
    fn create_indexes_command_carries_name_key_and_partial_filter() {
        let specs = generation2_specs();
        let background: Vec<&SearchIndexSpec> = specs.iter().filter(|s| s.build == IndexBuild::Background).collect();
        let cmd = create_indexes_command(&background);
        assert_eq!(cmd.get_str("createIndexes"), Ok("search_index"));
        let indexes = cmd.get_array("indexes").unwrap();
        assert_eq!(indexes.len(), 10);
        let first = indexes[0].as_document().unwrap();
        assert_eq!(first.get_str("name"), Ok("idx_search_string_v2"));
        assert_eq!(first.get_document("key").unwrap(), &background[0].keys);
        assert_eq!(
            first.get_document("partialFilterExpression").unwrap(),
            &doc! { "value_string": { "$exists": true } }
        );
        let composite_entry = generation2_specs()
            .into_iter()
            .find(|s| s.name == "idx_search_composite")
            .unwrap()
            .create_indexes_entry();
        assert!(!composite_entry.contains_key("partialFilterExpression"));
    }

    #[test]
    fn prebuild_script_in_docs_matches_the_catalog() {
        let expected = mongosh_script(&generation2_specs().into_iter().filter(|s| s.build == IndexBuild::Background).collect::<Vec<_>>());
        let on_disk = include_str!("../../../../../docs/mongodb/search-index-v2.mongosh.js");
        assert_eq!(on_disk, expected, "docs/mongodb/search-index-v2.mongosh.js is stale; regenerate it from mongosh_script()");
    }

    #[test]
    fn rollback_script_in_docs_matches_the_catalog() {
        let expected = mongosh_script(&superseded_v1_specs());
        let on_disk = include_str!("../../../../../docs/mongodb/search-index-v1-rollback.mongosh.js");
        assert_eq!(on_disk, expected, "docs/mongodb/search-index-v1-rollback.mongosh.js is stale; regenerate it from mongosh_script()");
    }
}
```

Declare the module in `crates/persistence/src/backends/mongodb/mod.rs` next to the existing `mod schema;` line:

```rust
pub(crate) mod search_index_catalog;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p helios-persistence --features mongodb --lib search_index_catalog`
Expected: compile error, `generation2_specs`, `SearchIndexSpec`, etc. not found.

- [ ] **Step 3: Implement the catalog**

Replace the top of the file (above the test module) with:

```rust
//! (keep the module doc comment from Step 1)

use mongodb::{
    IndexModel,
    bson::{Bson, Document, doc},
    options::IndexOptions,
};

/// The collection every spec in this catalog belongs to.
pub(crate) const SEARCH_INDEX_COLLECTION: &str = "search_index";

/// Recorded in the `schema_version` document as `search_indexes.generation`
/// once every [`IndexBuild::Background`] spec is present and the superseded
/// generation-1 indexes are gone.
pub(crate) const SEARCH_INDEX_GENERATION: i32 = 2;

/// When an index is created relative to boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexBuild {
    /// Created by `initialize_schema_async`, awaited before the server serves.
    /// Only for indexes whose build is cheap on every deployment.
    Inline,
    /// Created by the post-boot `SearchIndexBuilder` in one `createIndexes`
    /// command, so a multi-hour build on a large `search_index` never holds
    /// startup.
    Background,
}

/// One `search_index` index.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SearchIndexSpec {
    pub name: &'static str,
    /// Insertion order is key order.
    pub keys: Document,
    /// `partialFilterExpression`, when the index covers only rows that carry
    /// its value field. Every value row populates exactly one value field, so
    /// a partial index holds one entry per matching row instead of one null
    /// entry per row of the whole collection.
    pub partial: Option<Document>,
    pub build: IndexBuild,
}

impl SearchIndexSpec {
    /// The driver model, for `Collection::create_index`.
    pub(crate) fn index_model(&self) -> IndexModel {
        let options = IndexOptions::builder()
            .name(Some(self.name.to_string()))
            .partial_filter_expression(self.partial.clone())
            .build();
        IndexModel::builder()
            .keys(self.keys.clone())
            .options(Some(options))
            .build()
    }

    /// One entry of a raw `createIndexes` command's `indexes` array.
    pub(crate) fn create_indexes_entry(&self) -> Document {
        let mut entry = doc! { "key": self.keys.clone(), "name": self.name };
        if let Some(partial) = &self.partial {
            entry.insert("partialFilterExpression", partial.clone());
        }
        entry
    }
}

const LEADING: [&str; 3] = ["tenant_id", "resource_type", "param_name"];

/// A generation-2 value index: leading triple, the value keys, `resource_id`,
/// partial on the first value key existing.
fn value_v2(name: &'static str, value_keys: &[&str]) -> SearchIndexSpec {
    let mut keys = Document::new();
    for k in LEADING.iter().chain(value_keys.iter()) {
        keys.insert(*k, 1_i32);
    }
    keys.insert("resource_id", 1_i32);
    SearchIndexSpec {
        name,
        keys,
        partial: Some(doc! { value_keys[0]: { "$exists": true } }),
        build: IndexBuild::Background,
    }
}

/// A generation-1 value index, kept only so the rollback script and the
/// builder's drop list are derived from one place.
fn value_v1(name: &'static str, value_keys: &[&str]) -> SearchIndexSpec {
    let mut keys = Document::new();
    for k in LEADING.iter().chain(value_keys.iter()) {
        keys.insert(*k, 1_i32);
    }
    SearchIndexSpec { name, keys, partial: None, build: IndexBuild::Background }
}

/// Every index `search_index` should have once generation 2 is complete.
pub(crate) fn generation2_specs() -> Vec<SearchIndexSpec> {
    vec![
        value_v2("idx_search_string_v2", &["value_string"]),
        // Code first: the commonest token predicate (`status=final`,
        // `code=8302-2`) carries no system, and a system-first index leaves
        // `value_token_system` as an unbounded middle key for it.
        value_v2("idx_search_token_v2", &["value_token_code", "value_token_system"]),
        value_v2("idx_search_date_v2", &["value_date"]),
        value_v2("idx_search_number_v2", &["value_number"]),
        value_v2("idx_search_quantity_v2", &["value_quantity_value", "value_quantity_unit"]),
        value_v2("idx_search_reference_v2", &["value_reference"]),
        value_v2("idx_search_uri_v2", &["value_uri"]),
        value_v2("idx_search_token_display_v2", &["value_token_display"]),
        value_v2(
            "idx_search_identifier_type_v2",
            &["value_identifier_type_system", "value_identifier_type_code"],
        ),
        // Only contained rows carry `is_contained` (see
        // `build_contained_index_document`), so this partial index holds only
        // them; the trailing keys let the contained pipeline's `$group` read
        // its key from the index.
        SearchIndexSpec {
            name: "idx_search_contained",
            keys: doc! {
                "tenant_id": 1_i32,
                "contained_type": 1_i32,
                "is_contained": 1_i32,
                "param_name": 1_i32,
                "resource_type": 1_i32,
                "resource_id": 1_i32,
                "contained_local_id": 1_i32,
            },
            partial: Some(doc! { "is_contained": true }),
            build: IndexBuild::Background,
        },
        // Unchanged from generation 1. Both lead with `resource_id`;
        // `idx_search_composite` is hinted by name in the id-materialisation
        // path and `idx_search_resource` serves reindex deletes.
        SearchIndexSpec {
            name: "idx_search_composite",
            keys: doc! {
                "tenant_id": 1_i32,
                "resource_type": 1_i32,
                "resource_id": 1_i32,
                "param_name": 1_i32,
                "composite_group": 1_i32,
            },
            partial: None,
            build: IndexBuild::Inline,
        },
        SearchIndexSpec {
            name: "idx_search_resource",
            keys: doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "resource_id": 1_i32 },
            partial: None,
            build: IndexBuild::Inline,
        },
    ]
}

/// The generation-1 value indexes the builder drops once their `_v2` twins
/// are ready. Exact keys, so the rollback script recreates what existed.
pub(crate) fn superseded_v1_specs() -> Vec<SearchIndexSpec> {
    vec![
        value_v1("idx_search_string", &["value_string"]),
        value_v1("idx_search_token", &["value_token_system", "value_token_code"]),
        value_v1("idx_search_date", &["value_date"]),
        value_v1("idx_search_number", &["value_number"]),
        value_v1("idx_search_quantity", &["value_quantity_value", "value_quantity_unit"]),
        value_v1("idx_search_reference", &["value_reference"]),
        value_v1("idx_search_uri", &["value_uri"]),
        value_v1("idx_search_token_display", &["value_token_display"]),
        value_v1(
            "idx_search_identifier_type",
            &["value_identifier_type_system", "value_identifier_type_code"],
        ),
    ]
}

/// A raw `createIndexes` command for `specs`. One command builds every index
/// in a single collection scan, which is why the builder issues one command
/// rather than one per index.
pub(crate) fn create_indexes_command(specs: &[&SearchIndexSpec]) -> Document {
    let indexes: Vec<Bson> = specs
        .iter()
        .map(|s| Bson::Document(s.create_indexes_entry()))
        .collect();
    doc! { "createIndexes": SEARCH_INDEX_COLLECTION, "indexes": indexes }
}

/// The operator script: the same `createIndexes` command as relaxed extended
/// JSON, wrapped for `mongosh`. `docs/mongodb/*.mongosh.js` are generated
/// from this and a unit test keeps them equal.
pub(crate) fn mongosh_script(specs: &[SearchIndexSpec]) -> String {
    let refs: Vec<&SearchIndexSpec> = specs.iter().collect();
    let cmd = Bson::Document(create_indexes_command(&refs)).into_relaxed_extjson();
    let json = serde_json::to_string_pretty(&cmd).expect("createIndexes command serializes");
    format!(
        "// Generated from crates/persistence/src/backends/mongodb/search_index_catalog.rs.\n\
         // Do not edit by hand: a unit test compares this file to the catalog.\n\
         // Usage: mongosh \"$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE\" <this file>\n\
         // Builds every index in one collection scan; reads and writes continue meanwhile.\n\
         db.runCommand({json});\n"
    )
}
```

`serde_json` is already a dependency of `helios-persistence` (check `crates/persistence/Cargo.toml`; if it is only a dev-dependency, move it to `[dependencies]`).

- [ ] **Step 4: Generate the two docs scripts**

Add a temporary `#[test] #[ignore] fn print_scripts()` that prints `mongosh_script(...)` for both spec lists, run it with `-- --ignored --nocapture`, and write the output byte-for-byte to `docs/mongodb/search-index-v2.mongosh.js` (background generation-2 specs) and `docs/mongodb/search-index-v1-rollback.mongosh.js` (superseded v1 specs), LF line endings. Then delete the temporary test. Both files must end with a single trailing newline.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p helios-persistence --features mongodb --lib search_index_catalog`
Expected: `test result: ok. 9 passed`

- [ ] **Step 6: fmt, clippy, commit**

```bash
cargo fmt -p helios-persistence
# CI clippy command from Global Constraints
git add crates/persistence/src/backends/mongodb/search_index_catalog.rs crates/persistence/src/backends/mongodb/mod.rs docs/mongodb/search-index-v2.mongosh.js docs/mongodb/search-index-v1-rollback.mongosh.js
git commit -m "feat(mongodb): declarative search_index catalog with generation-2 partial specs (#1059, #1084)"
```

---

### Task 2: `IndexBuildMode` config and env var

**Files:**
- Create: `crates/persistence/src/backends/mongodb/search_index_builder.rs` (only the mode type in this task)
- Modify: `crates/persistence/src/backends/mongodb/backend.rs` (`MongoBackendConfig`, `Default`, `from_env` around lines 101-160, 196-210, 300-345)
- Modify: `crates/persistence/src/backends/mongodb/mod.rs`
- Modify: `crates/persistence/tests/mongodb_tests.rs:627-646` (`build_backend`)
- Modify: `README.md:324-328`

**Interfaces:**
- Produces: `pub enum IndexBuildMode { Background, Inline, Off }` (`Copy`, `Default = Background`, `Serialize`/`Deserialize` lowercase, `FromStr`), `MongoBackendConfig::index_build: IndexBuildMode`, env var `HFS_MONGODB_INDEX_BUILD`.

- [ ] **Step 1: Write the failing tests**

In `search_index_builder.rs`:

```rust
//! Post-boot builder for the generation-2 `search_index` indexes (#1059, #1084).

use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// How `SearchIndexBuilder` runs relative to boot. Read from
/// `HFS_MONGODB_INDEX_BUILD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexBuildMode {
    /// Spawn the builder after boot and return immediately (default).
    #[default]
    Background,
    /// Await the builder before boot completes. Tests and small databases.
    Inline,
    /// Inspect and warn only. The operator builds out of band.
    Off,
}

impl FromStr for IndexBuildMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "background" => Ok(Self::Background),
            "inline" => Ok(Self::Inline),
            "off" => Ok(Self::Off),
            other => Err(format!(
                "HFS_MONGODB_INDEX_BUILD must be one of background, inline, off; got {other:?}"
            )),
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn parses_the_three_modes_case_insensitively() {
        assert_eq!("background".parse::<IndexBuildMode>(), Ok(IndexBuildMode::Background));
        assert_eq!(" Inline ".parse::<IndexBuildMode>(), Ok(IndexBuildMode::Inline));
        assert_eq!("OFF".parse::<IndexBuildMode>(), Ok(IndexBuildMode::Off));
        assert!("sometimes".parse::<IndexBuildMode>().is_err());
    }

    #[test]
    fn default_is_background() {
        assert_eq!(IndexBuildMode::default(), IndexBuildMode::Background);
    }
}
```

In `backend.rs`, inside the existing `#[cfg(test)] mod tests` (find it with `grep -n "mod tests" crates/persistence/src/backends/mongodb/backend.rs`; if there is none, add one at the end of the file):

```rust
    #[test]
    fn config_index_build_defaults_to_background_and_reads_env() {
        assert_eq!(MongoBackendConfig::default().index_build, IndexBuildMode::Background);
        // from_env is process-global; guard the variable.
        unsafe { std::env::set_var("HFS_MONGODB_INDEX_BUILD", "inline") };
        let backend = MongoBackend::from_env().expect("from_env");
        assert_eq!(backend.config().index_build, IndexBuildMode::Inline);
        unsafe { std::env::set_var("HFS_MONGODB_INDEX_BUILD", "nonsense") };
        let err = MongoBackend::from_env().expect_err("invalid mode must be rejected");
        assert!(format!("{err}").contains("HFS_MONGODB_INDEX_BUILD"));
        unsafe { std::env::remove_var("HFS_MONGODB_INDEX_BUILD") };
    }
```

(`std::env::set_var` is `unsafe` in edition 2024; the existing tests in this crate that set env vars show the local convention, follow it. If other `from_env` tests already run in parallel and share env, mark this test `#[serial]` only if the crate already depends on `serial_test`; otherwise keep it single-variable as written.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p helios-persistence --features mongodb --lib index_build`
Expected: compile error, `index_build` field / `IndexBuildMode` not found in `backend.rs`.

- [ ] **Step 3: Implement**

`mod.rs`: add `pub(crate) mod search_index_builder;` and `pub use search_index_builder::IndexBuildMode;`.

`backend.rs`:
- `use super::search_index_builder::IndexBuildMode;`
- Add to `MongoBackendConfig` after `app_name`:
  ```rust
      /// When the generation-2 `search_index` indexes are built relative to
      /// boot: `background` (default) spawns the builder and serves at once,
      /// `inline` awaits it, `off` only warns about missing indexes so an
      /// operator can build them out of band (`HFS_MONGODB_INDEX_BUILD`).
      #[serde(default)]
      pub index_build: IndexBuildMode,
  ```
- `Default`: `index_build: IndexBuildMode::default(),`
- `from_env`, after `max_included_resources`:
  ```rust
          let index_build = match std::env::var("HFS_MONGODB_INDEX_BUILD") {
              Ok(raw) => raw.parse::<IndexBuildMode>().map_err(|message| {
                  StorageError::Backend(BackendError::Internal {
                      backend_name: "mongodb".to_string(),
                      message,
                      source: None,
                  })
              })?,
              Err(_) => IndexBuildMode::default(),
          };
  ```
  and `index_build,` in the `MongoBackendConfig { .. }` literal. Add `- \`HFS_MONGODB_INDEX_BUILD\` (default: \`background\`; \`inline\` | \`off\`)` to the doc comment list.

`mongodb_tests.rs` `build_backend`, after the `max_connections` line:
```rust
    // Generation-2 indexes are built after boot by default; tests assert
    // winning plans right after boot, so they wait for the build.
    config.index_build = IndexBuildMode::Inline;
```
and add `IndexBuildMode` to the `use helios_persistence::backends::mongodb::{...}` import.

`README.md`, after the `HFS_MONGODB_SERVER_SELECTION_TIMEOUT_MS` row:
```
| `HFS_MONGODB_INDEX_BUILD` | `background` | When the generation-2 `search_index` indexes are built: `background` serves immediately and builds after boot, `inline` waits for the build before serving, `off` only warns so an operator can pre-build (see `docs/mongodb/search-indexes.md`). |
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p helios-persistence --features mongodb --lib index_build`
Expected: 3 passed.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt -p helios-persistence
git add crates/persistence/src/backends/mongodb/search_index_builder.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/src/backends/mongodb/mod.rs crates/persistence/tests/mongodb_tests.rs README.md
git commit -m "feat(mongodb): HFS_MONGODB_INDEX_BUILD mode (background|inline|off)"
```

---

### Task 3: Schema: inline specs only, upsert schema version, generation record

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/schema.rs:239-325` (`ensure_search_indexes`), `:160` (`drop_index_if_present` visibility), `:531-552` (version helpers)
- Test: `crates/persistence/tests/mongodb_tests.rs`

**Interfaces:**
- Consumes: `search_index_catalog::{generation2_specs, IndexBuild, SearchIndexSpec::index_model}`
- Produces:
  - `pub(super) async fn drop_index_if_present(collection: &Collection<Document>, name: &str) -> StorageResult<()>` (existing, visibility widened)
  - `pub(super) async fn get_search_index_generation(database: &Database) -> StorageResult<Option<i32>>`
  - `pub(super) async fn set_search_index_generation(database: &Database, generation: i32) -> StorageResult<()>`
  - `set_schema_version` no longer deletes the document.

- [ ] **Step 1: Write the failing integration test**

Append to `mongodb_tests.rs`:

```rust
/// Task 3 of the generation-2 plan: boot creates only the inline
/// `search_index` specs, and the schema-version document survives a second
/// boot with its `search_indexes` record intact.
#[tokio::test]
async fn mongodb_integration_boot_creates_only_inline_search_indexes_and_keeps_generation_record() {
    let Some(connection_string) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let config = MongoBackendConfig {
        connection_string: connection_string.clone(),
        database_name: build_test_database_name("inline_specs_only"),
        // `off` so this test sees exactly what initialize_schema_async does.
        index_build: IndexBuildMode::Off,
        ..Default::default()
    };
    let backend = MongoBackend::new(config.clone()).unwrap();
    backend.initialize().await.expect("first boot");

    let client = raw_test_client(&connection_string).await.unwrap();
    let db = client.database(&config.database_name);
    let names = search_index_names(&db).await;
    assert_eq!(names, vec!["_id_", "idx_search_composite", "idx_search_resource"]);

    // A record written by the builder must survive the next boot.
    db.collection::<Document>("schema_version")
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": { "search_indexes": { "generation": 2_i32 } } },
        )
        .await
        .unwrap();
    let backend2 = MongoBackend::new(config.clone()).unwrap();
    backend2.initialize().await.expect("second boot");
    let doc = db
        .collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await
        .unwrap()
        .expect("schema_version document");
    assert!(doc.get_i32("version").unwrap() >= 10);
    assert_eq!(
        doc.get_document("search_indexes").unwrap().get_i32("generation"),
        Ok(2)
    );
}

/// Sorted index names on `search_index`, from a raw `listIndexes`.
async fn search_index_names(db: &mongodb::Database) -> Vec<String> {
    let reply = db
        .run_command(doc! { "listIndexes": "search_index" })
        .await
        .expect("listIndexes");
    let mut names: Vec<String> = reply
        .get_document("cursor")
        .unwrap()
        .get_array("firstBatch")
        .unwrap()
        .iter()
        .map(|b| b.as_document().unwrap().get_str("name").unwrap().to_string())
        .collect();
    names.sort();
    names
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests boot_creates_only_inline`
Expected: FAIL. The first assertion lists the nine v1 names as well; if it somehow passes, the second assertion fails because `set_schema_version` deleted the record.

- [ ] **Step 3: Implement**

`schema.rs`:

```rust
use super::search_index_catalog::{IndexBuild, SEARCH_INDEX_COLLECTION, generation2_specs};

/// Creates the `search_index` indexes whose build is cheap enough to await at
/// boot. Everything else (the generation-2 value indexes and the contained
/// index) is built by `SearchIndexBuilder` after boot; see the catalog.
async fn ensure_search_indexes(database: &Database) -> StorageResult<()> {
    let search_index = database.collection::<Document>(SEARCH_INDEX_COLLECTION);
    for spec in generation2_specs()
        .iter()
        .filter(|s| s.build == IndexBuild::Inline)
    {
        search_index.create_index(spec.index_model()).await?;
    }
    Ok(())
}
```

Delete the nine inline `create_index` calls that used to be in that function. `drop_index_if_present` becomes `pub(super) async fn`.

Replace `set_schema_version`:

```rust
/// Upserts `version` on the singleton document. Uses `$set` rather than
/// delete-and-insert so sibling fields written by other bootstrap steps
/// (`search_indexes`, see `set_search_index_generation`) survive every boot.
async fn set_schema_version(database: &Database, version: i32) -> StorageResult<()> {
    let collection = database.collection::<Document>("schema_version");
    collection
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": { "version": version } },
        )
        .upsert(true)
        .await?;
    Ok(())
}

/// The recorded `search_index` generation, `None` before the builder has
/// ever completed on this database.
pub(super) async fn get_search_index_generation(database: &Database) -> StorageResult<Option<i32>> {
    let doc = database
        .collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await?;
    Ok(doc
        .as_ref()
        .and_then(|d| d.get_document("search_indexes").ok())
        .and_then(|s| s.get_i32("generation").ok()))
}

/// Records that every background spec of `generation` is present and the
/// superseded indexes are gone.
pub(super) async fn set_search_index_generation(database: &Database, generation: i32) -> StorageResult<()> {
    database
        .collection::<Document>("schema_version")
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": { "search_indexes": {
                "generation": generation,
                "completed_at": mongodb::bson::DateTime::now(),
            } } },
        )
        .upsert(true)
        .await?;
    Ok(())
}
```

Check the driver version's `update_one` builder: in `mongodb` 3.x, `.upsert(true)` is a method on the returned action. If the crate is on 2.x, pass `UpdateOptions::builder().upsert(true).build()` as the third argument instead. Look at an existing `update_one` call in `storage.rs` and match its style.

- [ ] **Step 4: Run the new test and the existing schema-related tests**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests boot_creates_only_inline`
Expected: PASS.

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests schema`
Expected: every pre-existing schema/index test still passes. If a pre-existing test asserts one of the nine v1 names exists after boot, it is asserting generation 1 and must be updated to the `_v2` name once Task 4 lands; for this task, note it in the report rather than editing it.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt -p helios-persistence
git add crates/persistence/src/backends/mongodb/schema.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "feat(mongodb): boot creates only inline search_index specs; schema_version upsert keeps the generation record"
```

---

### Task 4: `SearchIndexBuilder` and boot wiring

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/search_index_builder.rs`
- Modify: `crates/persistence/src/backends/mongodb/backend.rs` (`MongoBackend` struct ~line 60, `new`, `init_schema` ~line 466)
- Test: `crates/persistence/tests/mongodb_tests.rs`

**Interfaces:**
- Consumes: catalog (`generation2_specs`, `superseded_v1_specs`, `create_indexes_command`, `SEARCH_INDEX_COLLECTION`, `SEARCH_INDEX_GENERATION`), schema (`drop_index_if_present`, `get/set_search_index_generation`), `IndexBuildMode`.
- Produces:
  - `pub(super) struct SearchIndexBuilder` with `pub(super) fn new(database: Database, mode: IndexBuildMode) -> Self` and `pub(super) async fn run(self) -> BuildOutcome`
  - `pub enum BuildOutcome { UpToDate, Built { created: Vec<String>, dropped: Vec<String> }, Skipped { missing: Vec<String> }, Failed { message: String } }` (`Clone`, `Debug`, `PartialEq`)
  - `MongoBackend::wait_for_search_index_build(&self) -> Option<BuildOutcome>` (public; awaits a running build, returns the stored outcome)

- [ ] **Step 1: Write the failing integration tests**

Append to `mongodb_tests.rs`. Helper first:

```rust
/// The nine generation-1 value indexes, created the way pre-generation-2
/// binaries created them, so a test can stage an "upgraded from v1" database.
async fn seed_generation1_indexes(db: &mongodb::Database) {
    let v1 = [
        ("idx_search_string", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_string": 1 }),
        ("idx_search_token", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_token_system": 1, "value_token_code": 1 }),
        ("idx_search_date", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_date": 1 }),
        ("idx_search_number", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_number": 1 }),
        ("idx_search_quantity", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_quantity_value": 1, "value_quantity_unit": 1 }),
        ("idx_search_reference", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_reference": 1 }),
        ("idx_search_uri", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_uri": 1 }),
        ("idx_search_token_display", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_token_display": 1 }),
        ("idx_search_identifier_type", doc! { "tenant_id": 1, "resource_type": 1, "param_name": 1, "value_identifier_type_system": 1, "value_identifier_type_code": 1 }),
    ];
    let indexes: Vec<Document> = v1
        .iter()
        .map(|(name, key)| doc! { "key": key.clone(), "name": *name })
        .collect();
    db.run_command(doc! { "createIndexes": "search_index", "indexes": indexes })
        .await
        .expect("seed v1 indexes");
}

const GENERATION2_BACKGROUND_NAMES: [&str; 10] = [
    "idx_search_contained",
    "idx_search_date_v2",
    "idx_search_identifier_type_v2",
    "idx_search_number_v2",
    "idx_search_quantity_v2",
    "idx_search_reference_v2",
    "idx_search_string_v2",
    "idx_search_token_display_v2",
    "idx_search_token_v2",
    "idx_search_uri_v2",
];

fn expected_generation2_names() -> Vec<String> {
    let mut all: Vec<String> = GENERATION2_BACKGROUND_NAMES.iter().map(|s| s.to_string()).collect();
    all.extend(["_id_", "idx_search_composite", "idx_search_resource"].map(String::from));
    all.sort();
    all
}

async fn boot_with_mode(connection_string: &str, database_name: &str, mode: IndexBuildMode) -> MongoBackend {
    let backend = MongoBackend::new(MongoBackendConfig {
        connection_string: connection_string.to_string(),
        database_name: database_name.to_string(),
        index_build: mode,
        max_connections: TEST_BACKEND_MAX_POOL,
        ..Default::default()
    })
    .unwrap();
    backend.initialize().await.expect("boot");
    backend
}
```

Tests:

```rust
#[tokio::test]
async fn mongodb_integration_builder_fresh_database_ends_with_generation2_set() {
    use helios_persistence::backends::mongodb::BuildOutcome;
    let Some(cs) = shared_mongo::connection_string().await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let db_name = build_test_database_name("builder_fresh");
    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    let outcome = backend.wait_for_search_index_build().await.expect("builder ran");
    match outcome {
        BuildOutcome::Built { created, dropped } => {
            let mut created = created; created.sort();
            assert_eq!(created, GENERATION2_BACKGROUND_NAMES.map(String::from).to_vec());
            assert!(dropped.is_empty(), "nothing to drop on a fresh database: {dropped:?}");
        }
        other => panic!("expected Built, got {other:?}"),
    }
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    assert_eq!(search_index_names(&db).await, expected_generation2_names());
    let record = db.collection::<Document>("schema_version").find_one(doc! { "_id": "schema_version" }).await.unwrap().unwrap();
    assert_eq!(record.get_document("search_indexes").unwrap().get_i32("generation"), Ok(2));
}

#[tokio::test]
async fn mongodb_integration_builder_upgrades_a_generation1_database_and_drops_v1() {
    use helios_persistence::backends::mongodb::BuildOutcome;
    let Some(cs) = shared_mongo::connection_string().await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let db_name = build_test_database_name("builder_upgrade");
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    seed_generation1_indexes(&db).await;
    // Some data, so the build has rows to index.
    let staged = boot_with_mode(&cs, &db_name, IndexBuildMode::Off).await;
    let tenant = create_tenant("tenant-builder");
    for i in 0..5 {
        staged.create(&tenant, "Patient", json!({ "resourceType": "Patient", "id": format!("p{i}"), "gender": "female" }), FhirVersion::default()).await.unwrap();
    }
    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    let outcome = backend.wait_for_search_index_build().await.expect("builder ran");
    let BuildOutcome::Built { dropped, .. } = outcome else { panic!("expected Built, got {outcome:?}") };
    let mut dropped = dropped; dropped.sort();
    assert_eq!(dropped, vec![
        "idx_search_date", "idx_search_identifier_type", "idx_search_number", "idx_search_quantity",
        "idx_search_reference", "idx_search_string", "idx_search_token", "idx_search_token_display", "idx_search_uri",
    ]);
    assert_eq!(search_index_names(&db).await, expected_generation2_names());
    // Data still searchable on the new indexes.
    let q = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "gender".into(), param_type: SearchParamType::Token, modifier: None,
        values: vec![SearchValue::eq("female")], chain: vec![], components: vec![],
    });
    assert_eq!(backend.search(&tenant, &q).await.unwrap().resources.items.len(), 5);
}

#[tokio::test]
async fn mongodb_integration_builder_refuses_to_touch_a_conflicting_v2_name() {
    use helios_persistence::backends::mongodb::BuildOutcome;
    let Some(cs) = shared_mongo::connection_string().await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let db_name = build_test_database_name("builder_conflict");
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    seed_generation1_indexes(&db).await;
    // A person built something under our name with different keys.
    db.run_command(doc! { "createIndexes": "search_index", "indexes": [
        { "key": { "tenant_id": 1, "value_date": 1 }, "name": "idx_search_date_v2" }
    ]}).await.unwrap();
    // In `inline` mode a failed build fails boot, so the conflict surfaces as
    // the initialize() error rather than through wait_for_search_index_build.
    let backend = MongoBackend::new(MongoBackendConfig {
        connection_string: cs.clone(),
        database_name: db_name.clone(),
        index_build: IndexBuildMode::Inline,
        max_connections: TEST_BACKEND_MAX_POOL,
        ..Default::default()
    })
    .unwrap();
    let err = backend.initialize().await.expect_err("a conflicting v2 index must fail inline boot");
    let message = format!("{err}");
    assert!(message.contains("idx_search_date_v2"), "{message}");
    let names = search_index_names(&db).await;
    assert!(names.contains(&"idx_search_string".to_string()), "v1 must be untouched: {names:?}");
    assert!(!names.contains(&"idx_search_string_v2".to_string()), "nothing must be built: {names:?}");
}

#[tokio::test]
async fn mongodb_integration_builder_off_mode_warns_and_changes_nothing() {
    use helios_persistence::backends::mongodb::BuildOutcome;
    let Some(cs) = shared_mongo::connection_string().await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let db_name = build_test_database_name("builder_off");
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    seed_generation1_indexes(&db).await;
    let before = search_index_names(&db).await;
    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Off).await;
    let outcome = backend.wait_for_search_index_build().await.expect("builder ran");
    let BuildOutcome::Skipped { missing } = outcome else { panic!("expected Skipped, got {outcome:?}") };
    let mut missing = missing; missing.sort();
    assert_eq!(missing, GENERATION2_BACKGROUND_NAMES.map(String::from).to_vec());
    assert_eq!(search_index_names(&db).await, before);
}

#[tokio::test]
async fn mongodb_integration_builder_second_boot_issues_no_create_indexes() {
    use helios_persistence::backends::mongodb::BuildOutcome;
    let Some(cs) = shared_mongo::connection_string().await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let db_name = build_test_database_name("builder_second_boot");
    let first = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    assert!(matches!(first.wait_for_search_index_build().await, Some(BuildOutcome::Built { .. })));
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    // Profile every command, boot again, then look for createIndexes.
    if db.run_command(doc! { "profile": 2_i32 }).await.is_err() {
        eprintln!("Skipping second-boot assertion: profiling not permitted");
        return;
    }
    let second = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    let outcome = second.wait_for_search_index_build().await.expect("builder ran");
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;
    assert_eq!(outcome, BuildOutcome::UpToDate);
    let created = db
        .collection::<Document>("system.profile")
        .count_documents(doc! { "command.createIndexes": "search_index" })
        .await
        .unwrap();
    assert_eq!(created, 0, "second boot must not issue createIndexes on search_index");
}
```

Add `BuildOutcome` to the `mod.rs` re-exports (`pub use search_index_builder::{BuildOutcome, IndexBuildMode};`) and to the test import.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests builder_`
Expected: compile error, `wait_for_search_index_build` and `BuildOutcome` not found.

- [ ] **Step 3: Implement the builder**

Append to `search_index_builder.rs`:

```rust
use std::time::Duration;

use mongodb::{
    Database,
    bson::{Document, doc},
};

use crate::error::StorageResult;

use super::schema::{drop_index_if_present, get_search_index_generation, set_search_index_generation};
use super::search_index_catalog::{
    IndexBuild, SEARCH_INDEX_COLLECTION, SEARCH_INDEX_GENERATION, SearchIndexSpec,
    create_indexes_command, generation2_specs, superseded_v1_specs,
};

/// What one run of the builder did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildOutcome {
    /// Every background spec present and ready, no superseded index left.
    UpToDate,
    /// Built the named indexes and dropped the named superseded ones.
    Built { created: Vec<String>, dropped: Vec<String> },
    /// `IndexBuildMode::Off`: these background specs are missing; nothing changed.
    Skipped { missing: Vec<String> },
    /// The run stopped. Nothing was dropped. The message is what was logged.
    Failed { message: String },
}

/// How long to wait between `listIndexes` polls while another process's
/// build of one of our names is in progress.
const IN_PROGRESS_POLL: Duration = Duration::from_secs(30);

/// One `listIndexes` reading, classified against the catalog.
#[derive(Debug, Default)]
struct Inspection {
    /// Background specs absent from the collection.
    missing: Vec<SearchIndexSpec>,
    /// Our names that exist but carry a `buildUUID`: someone else is building them.
    in_progress: Vec<String>,
    /// Our names that exist with a different key or partial filter.
    conflicting: Vec<(String, Document)>,
    /// Superseded generation-1 names still present.
    superseded_present: Vec<String>,
}

pub(super) struct SearchIndexBuilder {
    database: Database,
    mode: IndexBuildMode,
}

impl SearchIndexBuilder {
    pub(super) fn new(database: Database, mode: IndexBuildMode) -> Self {
        Self { database, mode }
    }

    /// Inspect, build, drop, record. Never returns an `Err`: every failure is
    /// logged and reported as `BuildOutcome::Failed`, because this runs
    /// detached from boot and nothing is waiting to handle an error.
    pub(super) async fn run(self) -> BuildOutcome {
        match self.run_inner().await {
            Ok(outcome) => outcome,
            Err(e) => {
                let message = format!("search_index generation-{SEARCH_INDEX_GENERATION} build failed: {e}");
                tracing::error!(error = %e, "{message}");
                BuildOutcome::Failed { message }
            }
        }
    }

    async fn run_inner(&self) -> StorageResult<BuildOutcome> {
        let mut inspection = self.inspect().await?;

        if !inspection.conflicting.is_empty() {
            let names: Vec<String> = inspection.conflicting.iter().map(|(n, _)| n.clone()).collect();
            for (name, actual) in &inspection.conflicting {
                let expected = generation2_specs().into_iter().find(|s| s.name == name).map(|s| s.keys);
                tracing::error!(
                    index = %name,
                    expected_keys = ?expected,
                    actual = ?actual,
                    "search_index index exists under a generation-2 name with a different spec; \
                     refusing to build or drop anything. Drop or rename it by hand."
                );
            }
            let message = format!("conflicting index spec under generation-2 name(s): {}", names.join(", "));
            return Ok(BuildOutcome::Failed { message });
        }

        if self.mode == IndexBuildMode::Off {
            let missing: Vec<String> = inspection.missing.iter().map(|s| s.name.to_string()).collect();
            if missing.is_empty() && inspection.superseded_present.is_empty() {
                self.record_if_needed().await?;
                return Ok(BuildOutcome::UpToDate);
            }
            for spec in &inspection.missing {
                tracing::warn!(
                    index = spec.name,
                    "HFS_MONGODB_INDEX_BUILD=off: generation-2 search_index index is missing; \
                     build it with docs/mongodb/search-index-v2.mongosh.js"
                );
            }
            return Ok(BuildOutcome::Skipped { missing });
        }

        // Someone else (another HFS process, or an operator's mongosh) is
        // building one of our names: wait for it rather than issue a second
        // build of the same index.
        while !inspection.in_progress.is_empty() {
            tracing::info!(indexes = ?inspection.in_progress, "waiting for in-progress search_index builds");
            tokio::time::sleep(IN_PROGRESS_POLL).await;
            inspection = self.inspect().await?;
        }

        let mut created = Vec::new();
        if !inspection.missing.is_empty() {
            let refs: Vec<&SearchIndexSpec> = inspection.missing.iter().collect();
            let names: Vec<&str> = refs.iter().map(|s| s.name).collect();
            tracing::info!(indexes = ?names, "building generation-2 search_index indexes in one collection scan");
            let started = std::time::Instant::now();
            self.database.run_command(create_indexes_command(&refs)).await?;
            tracing::info!(indexes = ?names, elapsed_s = started.elapsed().as_secs(), "generation-2 search_index build complete");
            created = names.into_iter().map(String::from).collect();
            inspection = self.inspect().await?;
            if !inspection.missing.is_empty() || !inspection.in_progress.is_empty() {
                let message = format!(
                    "createIndexes returned but generation-2 indexes are still missing or in progress: {:?} / {:?}",
                    inspection.missing.iter().map(|s| s.name).collect::<Vec<_>>(),
                    inspection.in_progress
                );
                tracing::error!("{message}");
                return Ok(BuildOutcome::Failed { message });
            }
        }

        let mut dropped = Vec::new();
        let collection = self.database.collection::<Document>(SEARCH_INDEX_COLLECTION);
        for name in &inspection.superseded_present {
            drop_index_if_present(&collection, name).await?;
            dropped.push(name.clone());
        }

        self.record_if_needed().await?;

        if created.is_empty() && dropped.is_empty() {
            tracing::info!("search_index indexes are at generation {SEARCH_INDEX_GENERATION}; nothing to do");
            Ok(BuildOutcome::UpToDate)
        } else {
            Ok(BuildOutcome::Built { created, dropped })
        }
    }

    async fn record_if_needed(&self) -> StorageResult<()> {
        if get_search_index_generation(&self.database).await? != Some(SEARCH_INDEX_GENERATION) {
            set_search_index_generation(&self.database, SEARCH_INDEX_GENERATION).await?;
        }
        Ok(())
    }

    /// Raw `listIndexes`, because the driver's `IndexModel` does not expose
    /// `buildUUID`, which is how an in-progress build is recognised.
    async fn inspect(&self) -> StorageResult<Inspection> {
        let reply = match self.database.run_command(doc! { "listIndexes": SEARCH_INDEX_COLLECTION }).await {
            Ok(reply) => reply,
            // NamespaceNotFound (26): the collection has never been written.
            // Every background spec is then "missing" and the build is instant.
            Err(e) if is_namespace_not_found(&e) => {
                return Ok(Inspection {
                    missing: generation2_specs().into_iter().filter(|s| s.build == IndexBuild::Background).collect(),
                    ..Default::default()
                });
            }
            Err(e) => return Err(e.into()),
        };
        let existing: Vec<Document> = reply
            .get_document("cursor")
            .ok()
            .and_then(|c| c.get_array("firstBatch").ok())
            .map(|batch| batch.iter().filter_map(|b| b.as_document().cloned()).collect())
            .unwrap_or_default();

        let mut inspection = Inspection::default();
        for spec in generation2_specs().into_iter().filter(|s| s.build == IndexBuild::Background) {
            match existing.iter().find(|d| d.get_str("name") == Ok(spec.name)) {
                None => inspection.missing.push(spec),
                Some(actual) => {
                    let same_keys = actual.get_document("key").ok() == Some(&spec.keys);
                    let same_partial = actual.get_document("partialFilterExpression").ok().cloned() == spec.partial;
                    if !(same_keys && same_partial) {
                        inspection.conflicting.push((spec.name.to_string(), actual.clone()));
                    } else if actual.contains_key("buildUUID") {
                        inspection.in_progress.push(spec.name.to_string());
                    }
                }
            }
        }
        for v1 in superseded_v1_specs() {
            if existing.iter().any(|d| d.get_str("name") == Ok(v1.name)) {
                inspection.superseded_present.push(v1.name.to_string());
            }
        }
        Ok(inspection)
    }
}

fn is_namespace_not_found(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(c) if c.code == 26)
}
```

`listIndexes` may page for collections with very many indexes; ours has at most 21, well under the default batch, so `firstBatch` is complete. Note that in a comment.

- [ ] **Step 4: Wire the builder into the backend**

`backend.rs`:

```rust
use super::search_index_builder::{BuildOutcome, IndexBuildMode, SearchIndexBuilder};

/// Where the post-boot search_index build is, for `wait_for_search_index_build`.
#[derive(Debug)]
enum SearchIndexBuildState {
    NotStarted,
    Running(tokio::task::JoinHandle<BuildOutcome>),
    Done(BuildOutcome),
}
```

Add to `MongoBackend`:
```rust
    /// Post-boot generation-2 index build, spawned by `init_schema`.
    search_index_build: Arc<tokio::sync::Mutex<SearchIndexBuildState>>,
```
Initialise it in `MongoBackend::new` (and any other constructor that builds the struct: grep `MongoBackend {` to find them all) with `Arc::new(tokio::sync::Mutex::new(SearchIndexBuildState::NotStarted))`.

Replace `init_schema`:
```rust
    /// Initializes the MongoDB schema/index bootstrap for this backend.
    ///
    /// Inline-class indexes are created before this returns. The
    /// generation-2 `search_index` indexes are built by `SearchIndexBuilder`:
    /// spawned and left running in `background` mode, awaited in `inline`
    /// mode, and only inspected in `off` mode (see `IndexBuildMode`).
    pub async fn init_schema(&self) -> StorageResult<()> {
        let db = self.get_database().await?;
        schema::initialize_schema_async(&db).await?;

        let builder = SearchIndexBuilder::new(db.clone(), self.config.index_build);
        let handle = tokio::spawn(builder.run());
        match self.config.index_build {
            IndexBuildMode::Inline => {
                let outcome = handle.await.map_err(|e| {
                    StorageError::Backend(BackendError::Internal {
                        backend_name: "mongodb".to_string(),
                        message: format!("search_index build task panicked: {e}"),
                        source: None,
                    })
                })?;
                if let BuildOutcome::Failed { message } = &outcome {
                    return Err(StorageError::Backend(BackendError::Internal {
                        backend_name: "mongodb".to_string(),
                        message: message.clone(),
                        source: None,
                    }));
                }
                *self.search_index_build.lock().await = SearchIndexBuildState::Done(outcome);
            }
            IndexBuildMode::Background | IndexBuildMode::Off => {
                *self.search_index_build.lock().await = SearchIndexBuildState::Running(handle);
            }
        }

        // Populate the per-tenant stored-param cache so the registries can build
        // each tenant's overlay lazily.
        self.reload_stored_cache().await?;
        Ok(())
    }

    /// Waits for the post-boot `search_index` build started by `init_schema`
    /// and returns its outcome; `None` if `init_schema` has not run. Safe to
    /// call repeatedly: the outcome is kept.
    pub async fn wait_for_search_index_build(&self) -> Option<BuildOutcome> {
        let mut state = self.search_index_build.lock().await;
        match std::mem::replace(&mut *state, SearchIndexBuildState::NotStarted) {
            SearchIndexBuildState::NotStarted => None,
            SearchIndexBuildState::Done(outcome) => {
                *state = SearchIndexBuildState::Done(outcome.clone());
                Some(outcome)
            }
            SearchIndexBuildState::Running(handle) => {
                let outcome = match handle.await {
                    Ok(outcome) => outcome,
                    Err(e) => BuildOutcome::Failed { message: format!("search_index build task panicked: {e}") },
                };
                *state = SearchIndexBuildState::Done(outcome.clone());
                Some(outcome)
            }
        }
    }
```

In `Inline` mode a `Failed` outcome fails boot, which is why the conflict test in Step 1 asserts on the `initialize()` error rather than on `wait_for_search_index_build`.

- [ ] **Step 5: Run the builder tests**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests builder_ -- --nocapture`
Expected: 5 `ok` lines, none skipped. Quote them in the report.

- [ ] **Step 6: Run the whole MongoDB suite once**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests`
Expected: all pass. Any pre-existing test that asserted a v1 index name now fails and must be updated to the `_v2` name (this is the follow-up noted in Task 3). Every existing explain-based test must still pass: if one now reports a different winning index, that is expected only if the new index is a `_v2` name; report anything else.

- [ ] **Step 7: fmt, clippy, commit**

```bash
cargo fmt -p helios-persistence
git add crates/persistence/src/backends/mongodb/search_index_builder.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/src/backends/mongodb/mod.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "feat(mongodb): post-boot SearchIndexBuilder builds generation-2 indexes and drops v1 (#1084)"
```

---

### Task 5: Explain tests for covered plans

**Files:**
- Test: `crates/persistence/tests/mongodb_tests.rs`

**Interfaces:**
- Consumes: the profiler-capture pattern in `mongodb_integration_history_providers` (`db.run_command(doc!{"profile": 2})`, query `system.profile`, `contains_stage_named`).

- [ ] **Step 1: Write the tests**

```rust
/// Every `search_index` operation a value-filtered search issues must be a
/// covered index scan on a generation-2 index: `docsExamined == 0`.
async fn assert_search_index_ops_are_covered(
    db: &mongodb::Database,
    search: impl std::future::Future<Output = ()>,
    expected_index_fragment: &str,
) {
    if db.run_command(doc! { "profile": 2_i32 }).await.is_err() {
        eprintln!("Skipping covered-plan assertion: profiling not permitted");
        search.await;
        return;
    }
    search.await;
    let _ = db.run_command(doc! { "profile": 0_i32 }).await;
    let ns = format!("{}.search_index", db.name());
    let ops: Vec<Document> = db
        .collection::<Document>("system.profile")
        .find(doc! { "ns": &ns, "op": { "$in": ["query", "command"] } })
        .await
        .unwrap()
        .try_collect::<Vec<Document>>()
        .await
        .unwrap();
    assert!(!ops.is_empty(), "expected at least one profiled operation on {ns}");
    for op in &ops {
        let docs_examined = op.get_i64("docsExamined").or_else(|_| op.get_i32("docsExamined").map(i64::from)).unwrap_or(0);
        let plan = op.get_str("planSummary").unwrap_or_default().to_string();
        assert_eq!(docs_examined, 0, "not covered: planSummary={plan} op={op:?}");
        assert!(plan.contains(expected_index_fragment), "wrong index: planSummary={plan}");
    }
}

#[tokio::test]
async fn mongodb_integration_date_range_search_is_a_covered_v2_scan() {
    let Some(backend) = create_backend("covered_date").await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let tenant = create_tenant("tenant-covered-date");
    for i in 0..20 {
        backend.create(&tenant, "Observation", json!({
            "resourceType": "Observation", "id": format!("o{i}"), "status": "final",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "8302-2" }] },
            "effectiveDateTime": format!("2016-01-{:02}", i + 1)
        }), FhirVersion::default()).await.unwrap();
    }
    let db = raw_test_client(&backend.config().connection_string).await.unwrap().database(&backend.config().database_name);
    let q = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "date".into(), param_type: SearchParamType::Date, modifier: None,
        values: vec![SearchValue::parse("ge2016-01-10")], chain: vec![], components: vec![],
    });
    assert_search_index_ops_are_covered(&db, async {
        let r = backend.search(&tenant, &q).await.unwrap();
        assert_eq!(r.resources.items.len(), 11);
    }, "idx_search_date_v2").await;
}

#[tokio::test]
async fn mongodb_integration_bare_token_search_is_a_covered_v2_scan() {
    let Some(backend) = create_backend("covered_token").await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let tenant = create_tenant("tenant-covered-token");
    for i in 0..20 {
        backend.create(&tenant, "Observation", json!({
            "resourceType": "Observation", "id": format!("o{i}"), "status": if i % 2 == 0 { "final" } else { "preliminary" },
            "code": { "coding": [{ "system": "http://loinc.org", "code": "8302-2" }] }
        }), FhirVersion::default()).await.unwrap();
    }
    let db = raw_test_client(&backend.config().connection_string).await.unwrap().database(&backend.config().database_name);
    let q = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "status".into(), param_type: SearchParamType::Token, modifier: None,
        values: vec![SearchValue::eq("final")], chain: vec![], components: vec![],
    });
    assert_search_index_ops_are_covered(&db, async {
        let r = backend.search(&tenant, &q).await.unwrap();
        assert_eq!(r.resources.items.len(), 10);
    }, "idx_search_token_v2").await;
}
```

`try_collect` needs `use futures::TryStreamExt;` (check the file's existing imports; `collect_documents`-style loops are fine instead).

- [ ] **Step 2: Run them**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests covered_ -- --nocapture`
Expected: PASS. If an operation on `search_index` reports `docsExamined > 0`, do not relax the assertion: report the operation's `planSummary` and command shape as DONE_WITH_CONCERNS. The controller decides whether that operation needs a projection change or is legitimately uncovered.

- [ ] **Step 3: Commit**

```bash
git add crates/persistence/tests/mongodb_tests.rs
git commit -m "test(mongodb): value-filtered searches are covered scans on generation-2 indexes"
```

---

### Task 6: Contained search on the partial index, paged on the server

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/search_impl.rs:617-760` (`search_contained`, `matching_contained`)
- Modify: `crates/persistence/src/backends/mongodb/storage.rs:435` (`document_to_stored_resource` visibility)
- Test: `crates/persistence/tests/mongodb_tests.rs`

**Interfaces:**
- Consumes: `build_search_index_filter("", "", param)` (unchanged), `extract_contained_resource`, `build_contained_stored`, `collect_documents`, `query.wants_total()`, `SearchResult::new/with_total`, `Page::new(items, PageInfo::end())`.
- Produces: `pub(super) fn document_to_stored_resource(doc: &Document, tenant: &TenantContext, fallback_resource_type: &str) -> StorageResult<StoredResource>` in `storage.rs` (visibility only).

- [ ] **Step 1: Write the failing integration tests**

```rust
/// Seeds `n` Observations, each containing a Patient named Smith, under ids
/// `obs-<i>` with contained local id `p`.
async fn seed_contained_smiths(backend: &MongoBackend, tenant: &TenantContext, n: usize) {
    for i in 0..n {
        backend.create(tenant, "Observation", json!({
            "resourceType": "Observation", "id": format!("obs-{i:02}"), "status": "final",
            "code": { "coding": [{ "code": "1234-5" }] },
            "subject": { "reference": "#p" },
            "contained": [{ "resourceType": "Patient", "id": "p", "name": [{ "family": "Smith" }] }]
        }), FhirVersion::default()).await.unwrap();
    }
}

fn contained_name_query(mode: helios_persistence::types::ContainedMode, count: u32, offset: u32, total: bool) -> SearchQuery {
    use helios_persistence::types::ContainedReturn;
    let mut q = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "name".into(), param_type: SearchParamType::String, modifier: None,
        values: vec![SearchValue::eq("Smith")], chain: vec![], components: vec![],
    });
    q.contained = mode;
    q.contained_return = ContainedReturn::Container;
    q.count = Some(count);
    q.offset = Some(offset);
    q.total = if total { Some(TotalMode::Accurate) } else { None };
    q
}

#[tokio::test]
async fn mongodb_integration_contained_search_pages_on_the_server() {
    use helios_persistence::types::ContainedMode;
    let Some(backend) = create_backend_with_full_registry("contained_paging").await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let tenant = create_tenant("tenant-contained-paging");
    seed_contained_smiths(&backend, &tenant, 7).await;

    let page = |offset: u32| contained_name_query(ContainedMode::On, 3, offset, true);
    let p0 = backend.search(&tenant, &page(0)).await.unwrap();
    let p1 = backend.search(&tenant, &page(3)).await.unwrap();
    let p2 = backend.search(&tenant, &page(6)).await.unwrap();
    let ids = |r: &helios_persistence::core::SearchResult| r.resources.items.iter().map(|x| x.id().to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&p0), vec!["obs-00", "obs-01", "obs-02"]);
    assert_eq!(ids(&p1), vec!["obs-03", "obs-04", "obs-05"]);
    assert_eq!(ids(&p2), vec!["obs-06"]);
    assert_eq!(p0.total, Some(7));
    assert_eq!(p2.total, Some(7));
    // Without _total, no count is computed.
    let no_total = backend.search(&tenant, &contained_name_query(ContainedMode::On, 3, 0, false)).await.unwrap();
    assert_eq!(no_total.total, None);
}

#[tokio::test]
async fn mongodb_integration_contained_both_pages_across_the_top_level_boundary() {
    use helios_persistence::types::ContainedMode;
    let Some(backend) = create_backend_with_full_registry("contained_both_paging").await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let tenant = create_tenant("tenant-contained-both");
    for i in 0..2 {
        backend.create(&tenant, "Patient", json!({ "resourceType": "Patient", "id": format!("top-{i}"), "name": [{ "family": "Smith" }] }), FhirVersion::default()).await.unwrap();
    }
    seed_contained_smiths(&backend, &tenant, 7).await;

    let ids = |r: &helios_persistence::core::SearchResult| r.resources.items.iter().map(|x| x.url()).collect::<Vec<_>>();
    // Page of 5 from offset 0: both top-level Patients, then the first three containers.
    let r = backend.search(&tenant, &contained_name_query(ContainedMode::Both, 5, 0, true)).await.unwrap();
    assert_eq!(ids(&r), vec!["Patient/top-0", "Patient/top-1", "Observation/obs-00", "Observation/obs-01", "Observation/obs-02"]);
    assert_eq!(r.total, Some(9));
    // Offset 5 lands inside the contained set: contained offset 3.
    let r = backend.search(&tenant, &contained_name_query(ContainedMode::Both, 5, 5, true)).await.unwrap();
    assert_eq!(ids(&r), vec!["Observation/obs-03", "Observation/obs-04", "Observation/obs-05", "Observation/obs-06"]);
    // Offset 1 straddles: one top-level, four containers.
    let r = backend.search(&tenant, &contained_name_query(ContainedMode::Both, 5, 1, false)).await.unwrap();
    assert_eq!(ids(&r), vec!["Patient/top-1", "Observation/obs-00", "Observation/obs-01", "Observation/obs-02", "Observation/obs-03"]);
}

#[tokio::test]
async fn mongodb_integration_contained_both_dedupes_a_container_that_is_also_a_top_level_match() {
    use helios_persistence::types::{ContainedMode, ContainedReturn};
    let Some(backend) = create_backend_with_full_registry("contained_both_dedupe").await else { eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)"); return; };
    let tenant = create_tenant("tenant-contained-dedupe");
    // An Observation with code X that also contains an Observation with code X:
    // it is a top-level match AND the container of a contained match.
    backend.create(&tenant, "Observation", json!({
        "resourceType": "Observation", "id": "outer", "status": "final",
        "code": { "coding": [{ "system": "http://loinc.org", "code": "X" }] },
        "contained": [{ "resourceType": "Observation", "id": "inner", "status": "final",
                        "code": { "coding": [{ "system": "http://loinc.org", "code": "X" }] } }]
    }), FhirVersion::default()).await.unwrap();
    // A second container whose only match is contained.
    backend.create(&tenant, "Observation", json!({
        "resourceType": "Observation", "id": "holder", "status": "final",
        "code": { "coding": [{ "system": "http://loinc.org", "code": "Y" }] },
        "contained": [{ "resourceType": "Observation", "id": "inner2", "status": "final",
                        "code": { "coding": [{ "system": "http://loinc.org", "code": "X" }] } }]
    }), FhirVersion::default()).await.unwrap();

    let mut q = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".into(), param_type: SearchParamType::Token, modifier: None,
        values: vec![SearchValue::eq("X")], chain: vec![], components: vec![],
    });
    q.contained = ContainedMode::Both;
    q.contained_return = ContainedReturn::Container;
    q.count = Some(10);
    q.total = Some(TotalMode::Accurate);
    let r = backend.search(&tenant, &q).await.unwrap();
    let urls: Vec<String> = r.resources.items.iter().map(|x| x.url()).collect();
    assert_eq!(urls, vec!["Observation/outer", "Observation/holder"], "outer must appear once");
}
```

Keep the existing `mongodb_integration_contained_search` test as is; it must still pass.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests contained_ -- --nocapture`
Expected: the page-content and `total` assertions PASS against today's code, because client-side `skip/take` over the full materialised set yields the same pages and totals. That is intended: those assertions pin behaviour that must not change. The RED for this task comes from how the work is done, so add the following profiler assertions to the paging test, before the page assertions. They fail today (three `read()` calls per page, and no `idx_search_contained` in the plan):

```rust
    let db = raw_test_client(&backend.config().connection_string).await.unwrap().database(&backend.config().database_name);
    if db.run_command(doc! { "profile": 2_i32 }).await.is_ok() {
        let _ = backend.search(&tenant, &page(0)).await.unwrap();
        let _ = db.run_command(doc! { "profile": 0_i32 }).await;
        let reads = db.collection::<Document>("system.profile")
            .count_documents(doc! { "ns": format!("{}.resources", db.name()), "op": "query" })
            .await.unwrap();
        assert_eq!(reads, 1, "containers for one page must be fetched in a single find, not one read() per match");
        let agg = db.collection::<Document>("system.profile")
            .find(doc! { "ns": format!("{}.search_index", db.name()), "command.aggregate": "search_index" })
            .await.unwrap().try_collect::<Vec<Document>>().await.unwrap();
        assert!(!agg.is_empty(), "the contained pipeline must run as an aggregate on search_index");
        for op in &agg {
            let plan = op.get_str("planSummary").unwrap_or_default();
            assert!(plan.contains("idx_search_contained"), "contained pipeline must use idx_search_contained: {plan}");
        }
    }
```

With that, RED: `reads` is 3 (one `read()` per container) and the plan is a COLLSCAN or `_id_` scan. Record the failing assertion verbatim.

- [ ] **Step 3: Implement**

`storage.rs:435`: `pub(super) fn document_to_stored_resource(`.

`search_impl.rs`: replace `search_contained` and `matching_contained` with:

```rust
    /// One contained match: the container and, for `_containedType=contained`,
    /// the local id of the contained entity.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct ContainedKey {
        rtype: String,
        rid: String,
        lid: Option<String>,
    }

    /// A server-side page of contained matches.
    struct ContainedPage {
        keys: Vec<ContainedKey>,
        /// Only when `_total` was requested.
        total: Option<u64>,
    }

    async fn search_contained(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        use crate::types::{ContainedMode, ContainedReturn, TotalMode};

        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let contained_type = query.resource_type.as_str();
        let count = query.count.unwrap_or(100).max(1) as usize;
        let offset = query.offset.unwrap_or(0) as usize;
        let want_total = query.wants_total();

        let (mut items, total) = match query.contained {
            ContainedMode::Both => {
                // Top-level matches come first, contained matches second. The
                // standard search is asked for its total so the boundary is
                // known, and each source is paged on the server.
                let mut top_query = query.clone();
                top_query.contained = ContainedMode::Off;
                top_query.contained_return = ContainedReturn::Container;
                top_query.total = Some(TotalMode::Accurate);
                let top = self.search(tenant, &top_query).await?;
                let top_total = top.total.unwrap_or(top.resources.items.len() as u64) as usize;
                let mut items = top.resources.items;
                let top_urls: HashSet<String> = items.iter().map(|r| r.url()).collect();

                let (c_offset, c_limit) = if offset < top_total {
                    (0, count.saturating_sub(items.len()))
                } else {
                    (offset - top_total, count)
                };
                let mut contained_total = None;
                if c_limit > 0 || want_total {
                    let page = self
                        .matching_contained(&db, tenant_id, contained_type, query, c_offset, c_limit.max(1), want_total)
                        .await?;
                    contained_total = page.total;
                    let mut contained = self
                        .materialize_contained(tenant, contained_type, query.contained_return, &page.keys)
                        .await?;
                    contained.retain(|r| !top_urls.contains(&r.url()));
                    let dropped = page.keys.len().saturating_sub(contained.len());
                    // Containers already on the top-level page were removed;
                    // fetch that many more, once, so the page stays full.
                    if dropped > 0 && page.keys.len() == c_limit.max(1) {
                        let more = self
                            .matching_contained(&db, tenant_id, contained_type, query, c_offset + page.keys.len(), dropped, false)
                            .await?;
                        let mut extra = self
                            .materialize_contained(tenant, contained_type, query.contained_return, &more.keys)
                            .await?;
                        extra.retain(|r| !top_urls.contains(&r.url()));
                        contained.extend(extra);
                    }
                    if c_limit > 0 {
                        items.extend(contained);
                    }
                }
                let total = if want_total {
                    Some(top_total as u64 + contained_total.unwrap_or(0))
                } else {
                    None
                };
                (items, total)
            }
            _ => {
                let page = self
                    .matching_contained(&db, tenant_id, contained_type, query, offset, count, want_total)
                    .await?;
                let items = self
                    .materialize_contained(tenant, contained_type, query.contained_return, &page.keys)
                    .await?;
                (items, page.total)
            }
        };

        items.truncate(count);
        let page = Page::new(items, PageInfo::end());
        let mut result = SearchResult::new(page);
        if let Some(t) = total {
            result = result.with_total(t);
        }
        Ok(result)
    }

    /// Resolves one server-side page of `_contained` matches over
    /// `idx_search_contained` (#1059): `$match` in the index's key order,
    /// `$group` by container and local id (read from the index keys),
    /// `$sort` for a stable page order, then `$skip`/`$limit`, with a
    /// `$facet` count alongside when `_total` is requested.
    async fn matching_contained(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        contained_type: &str,
        query: &SearchQuery,
        offset: usize,
        limit: usize,
        want_total: bool,
    ) -> StorageResult<ContainedPage> {
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);

        let mut branches: Vec<Bson> = Vec::new();
        let mut distinct_names: Vec<String> = Vec::new();
        for param in &query.parameters {
            if param.name.starts_with('_')
                || matches!(param.param_type, SearchParamType::Composite | SearchParamType::Special)
            {
                continue;
            }
            let mut branch = self.build_search_index_filter("", "", param)?;
            branch.remove("tenant_id");
            branch.remove("resource_type");
            branches.push(Bson::Document(branch));
            if !distinct_names.contains(&param.name) {
                distinct_names.push(param.name.clone());
            }
        }
        if branches.is_empty() {
            return Ok(ContainedPage { keys: Vec::new(), total: want_total.then_some(0) });
        }

        let mut pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "contained_type": contained_type,
                "is_contained": true,
                "$or": branches,
            }},
            doc! { "$group": {
                "_id": { "rtype": "$resource_type", "rid": "$resource_id", "lid": "$contained_local_id" },
                "names": { "$addToSet": "$param_name" },
            }},
        ];
        if distinct_names.len() > 1 {
            pipeline.push(doc! { "$match": { "names": { "$all": distinct_names } } });
        }
        pipeline.push(doc! { "$sort": { "_id.rtype": 1, "_id.rid": 1, "_id.lid": 1 } });
        let page_stages = vec![doc! { "$skip": offset as i64 }, doc! { "$limit": limit as i64 }];
        if want_total {
            pipeline.push(doc! { "$facet": { "page": page_stages, "total": [ { "$count": "n" } ] } });
        } else {
            pipeline.extend(page_stages);
        }

        let cursor = search_index
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to aggregate contained search")?;
        let docs = collect_documents(cursor).await?;

        let (page_docs, total): (Vec<Document>, Option<u64>) = if want_total {
            let facet = docs.into_iter().next().unwrap_or_default();
            let page = facet
                .get_array("page")
                .map(|a| a.iter().filter_map(|b| b.as_document().cloned()).collect())
                .unwrap_or_default();
            let n = facet
                .get_array("total")
                .ok()
                .and_then(|a| a.first())
                .and_then(|b| b.as_document())
                .and_then(|d| d.get_i64("n").ok().or_else(|| d.get_i32("n").ok().map(i64::from)))
                .unwrap_or(0);
            (page, Some(n.max(0) as u64))
        } else {
            (docs, None)
        };

        let mut keys = Vec::with_capacity(page_docs.len());
        for doc in page_docs {
            let Ok(id) = doc.get_document("_id") else { continue };
            let rtype = id.get_str("rtype").unwrap_or_default().to_string();
            let rid = id.get_str("rid").unwrap_or_default().to_string();
            if rtype.is_empty() || rid.is_empty() {
                continue;
            }
            let lid = id.get_str("lid").ok().map(ToString::to_string);
            keys.push(ContainedKey { rtype, rid, lid });
        }
        Ok(ContainedPage { keys, total })
    }

    /// Fetches the containers for `keys` in one `find` on `resources`, in
    /// `keys` order, and shapes them per `contained_return`. Deleted or
    /// missing containers are skipped.
    async fn materialize_contained(
        &self,
        tenant: &TenantContext,
        contained_type: &str,
        contained_return: crate::types::ContainedReturn,
        keys: &[ContainedKey],
    ) -> StorageResult<Vec<StoredResource>> {
        use crate::types::ContainedReturn;
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);

        let mut pairs: Vec<(String, String)> = keys.iter().map(|k| (k.rtype.clone(), k.rid.clone())).collect();
        pairs.sort();
        pairs.dedup();
        let or: Vec<Bson> = pairs
            .iter()
            .map(|(t, i)| Bson::Document(doc! { "resource_type": t, "id": i }))
            .collect();
        let cursor = resources
            .find(doc! {
                "tenant_id": tenant.tenant_id().as_str(),
                "is_deleted": { "$ne": true },
                "$or": or,
            })
            .hint(mongodb::options::Hint::Name("idx_resources_identity".to_string()))
            .await
            .or_query_error("Failed to fetch contained-search containers")?;
        let mut by_key: HashMap<(String, String), StoredResource> = HashMap::new();
        for doc in collect_documents(cursor).await? {
            let rtype = doc.get_str("resource_type").unwrap_or_default().to_string();
            let stored = super::storage::document_to_stored_resource(&doc, tenant, &rtype)?;
            by_key.insert((stored.resource_type().to_string(), stored.id().to_string()), stored);
        }

        let mut items = Vec::with_capacity(keys.len());
        let mut seen: HashSet<String> = HashSet::new();
        for key in keys {
            let Some(container) = by_key.get(&(key.rtype.clone(), key.rid.clone())) else { continue };
            match contained_return {
                ContainedReturn::Container => {
                    if seen.insert(format!("{}/{}", key.rtype, key.rid)) {
                        items.push(container.clone());
                    }
                }
                ContainedReturn::Contained => {
                    let Some(local_id) = &key.lid else { continue };
                    if !seen.insert(format!("{}/{}#{}", key.rtype, key.rid, local_id)) {
                        continue;
                    }
                    if let Some(c) = extract_contained_resource(container.content(), local_id) {
                        items.push(build_contained_stored(container, contained_type, local_id, c));
                    }
                }
            }
        }
        Ok(items)
    }
```

Check the exact names of `MongoBackend::RESOURCES_COLLECTION` and `SEARCH_INDEX_COLLECTION` (both exist as `pub(crate) const` in `backend.rs`), the `.hint()` builder method on `find` for the driver version in use (`search_impl.rs:1123` already uses `Hint::Name` on a `find`), `is_deleted` (read as `get_bool("is_deleted").unwrap_or(false)` in `read()`, so absent means live; hence `$ne: true`), and that `ContainedReturn` is `Copy` (if not, take it by reference). `#[derive]` on the nested structs needs them at module level, not inside `impl`; put `ContainedKey` and `ContainedPage` next to `extract_contained_resource`.

- [ ] **Step 4: Run the contained tests and the existing contained test**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests contained -- --nocapture`
Expected: all `ok`, including the pre-existing `mongodb_integration_contained_search`.

- [ ] **Step 5: Run the whole MongoDB suite once, fmt, clippy, commit**

```bash
cargo test -p helios-persistence --features mongodb --test mongodb_tests
cargo fmt -p helios-persistence
git add crates/persistence/src/backends/mongodb/search_impl.rs crates/persistence/src/backends/mongodb/storage.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "fix(mongodb): _contained search pages on the server over idx_search_contained (#1059)"
```

---

### Task 7: Operator documentation

**Files:**
- Create: `docs/mongodb/search-indexes.md`
- Modify: `crates/persistence/src/backends/mongodb/schema.rs:15-23` (`SCHEMA_VERSION` doc comment)

**Interfaces:**
- Consumes: the two scripts from Task 1, `HFS_MONGODB_INDEX_BUILD` from Task 2.

- [ ] **Step 1: Write the doc**

`docs/mongodb/search-indexes.md`:

```markdown
# MongoDB `search_index` indexes

HFS keeps two kinds of index on the `search_index` collection.

- **Inline** indexes (`idx_search_composite`, `idx_search_resource`) are created at every boot before the server serves. They are cheap to build.
- **Generation-2** indexes (nine partial value indexes named `idx_search_*_v2`, and `idx_search_contained`) are built by HFS **after** boot, in one `createIndexes` command that scans the collection once. MongoDB 4.2 and later do not block reads or writes during the build. When every generation-2 index is ready, HFS drops the nine generation-1 value indexes (`idx_search_string`, `idx_search_token`, ...) and records `search_indexes.generation: 2` in the `schema_version` document.

Why: a generation-1 value index carried one entry for every row of the collection, even rows that had no value of that type, and no value index carried `resource_id`, so every search fetched one document per matching key. Generation-2 indexes are partial (one entry per row that has the value) and end in `resource_id`, so a value-filtered scan is covered. Issues #1059 and #1084 have the measurements.

## `HFS_MONGODB_INDEX_BUILD`

| value | behaviour |
|---|---|
| `background` (default) | Boot returns immediately. The build runs in the background and is logged at `info` when it starts and finishes. |
| `inline` | Boot waits for the build. Use for tests, developer databases and small deployments. |
| `off` | Nothing is built or dropped. Each missing generation-2 index is logged at `warn`. Use when you pre-build in a maintenance window. |

## Upgrading a large deployment

Deploy the binary. The server serves on generation 1 while the build runs. Disk peaks at the size of both generations, then drops when the old ones are removed. Writes are slower during the build by the cost of maintaining both sets. Nothing needs to be scheduled.

To build in a window of your choosing instead, run the pre-build script first; a database that already has every generation-2 index makes the builder a no-op at boot:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-v2.mongosh.js

## If the build fails

The log carries the server's error at `error` level. Nothing has been dropped; searches keep using generation 1. Fix the cause (usually disk) and restart: every step is idempotent, and a completed index is skipped.

If a generation-2 name exists with a different key spec, HFS refuses to build or drop anything and logs both specs. Drop or rename that index by hand.

## Downgrading

A binary from before generation 2 creates the nine generation-1 indexes at boot, inline, and will not serve until they exist. If they have been dropped, that boot rebuilds all nine before serving. Before rolling back, recreate them in the background:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-v1-rollback.mongosh.js

Both scripts are generated from `crates/persistence/src/backends/mongodb/search_index_catalog.rs`; a unit test fails if they drift.
```

Extend the `SCHEMA_VERSION` doc comment in `schema.rs` with one sentence: "`search_index` indexes are versioned separately by `search_indexes.generation` on the same document (see `search_index_catalog.rs`); `SCHEMA_VERSION` does not change for them."

- [ ] **Step 2: Verify the doc paths exist and the catalog test still passes**

Run: `cargo test -p helios-persistence --features mongodb --lib search_index_catalog`
Expected: 9 passed (the two script-equality tests confirm the files referenced by the doc exist).

- [ ] **Step 3: Commit**

```bash
git add docs/mongodb/search-indexes.md crates/persistence/src/backends/mongodb/schema.rs
git commit -m "docs(mongodb): search_index generation-2 build, modes, pre-build and rollback"
```

---

### Task 8: Corpus measurement (controller-run, not a subagent task)

This task runs against the local `hfs-mongo` corpus container (database `helios`, tenant `default`, 228M `search_index` rows) and records numbers for the PR description. It changes no source files.

- [ ] **Step 1: Record the before numbers** (already measured 2026-09-15, reuse): `totalIndexSize` 64.3 GB; `Observation?code=8302-2` 175,358 keys / 175,355 docs / 8 to 12 s; `Observation?status=final` 7,699,979 keys / 7,699,978 docs / 67 s; contained count exceeds 900 s.

- [ ] **Step 2: Build generation 2 on the corpus with the operator script**

```bash
docker cp docs/mongodb/search-index-v2.mongosh.js hfs-mongo:/tmp/v2.js
MSYS_NO_PATHCONV=1 docker exec hfs-mongo mongosh --quiet helios /tmp/v2.js
```

Expect one collection scan; note wall time. Then drop v1 by hand (the builder would, but the corpus is not booted by HFS): for each of the nine names, `db.search_index.dropIndex("<name>")`.

- [ ] **Step 3: Record the after numbers**

```javascript
const st = db.search_index.stats({scale: 1024*1024*1024});
print("totalIndexSize " + st.totalIndexSize.toFixed(1) + " GB");
for (const [k,v] of Object.entries(st.indexSizes)) print("  " + k + " " + v.toFixed(2) + " GB");
function ex(name, filter) {
  const e = db.search_index.find(filter, {resource_id: 1, _id: 0}).explain("executionStats");
  const s = e.executionStats;
  print([name, "keys=" + s.totalKeysExamined, "docs=" + s.totalDocsExamined, "n=" + s.nReturned, "ms=" + s.executionTimeMillis].join("\t"));
}
ex("code bare", {tenant_id: "default", resource_type: "Observation", param_name: "code", value_token_code: "8302-2"});
ex("status final", {tenant_id: "default", resource_type: "Observation", param_name: "status", value_token_code: "final"});
const t0 = Date.now();
print("contained rows: " + db.search_index.countDocuments({tenant_id: "default", is_contained: true}) + " in " + (Date.now() - t0) + " ms");
```

Expected: `docs=0` on both explains, contained count in milliseconds, `totalIndexSize` in the region of 15 GB. Put the table in the PR body.

---

## Self-review

**Spec coverage.** §3 catalog: Task 1. §4.1 boot and JoinHandle: Task 4. §4.2 steps 1 to 4 including the in-progress wait and namespace-not-found: Task 4. §4.3 modes and env var: Task 2, with `off` behaviour in Task 4. §4.4 failures: Task 4 (`Failed` outcome, conflict refusal, nothing dropped on error). §4.5 no query changes: no task edits filter builders. §5.1 pipeline, §5.2 batched fetch with `idx_resources_identity` hint, §5.3 `both` arithmetic with the one extra round trip, §5.4 unchanged pieces: Task 6. §6 rollout, pre-build script, downgrade: Tasks 1 and 7. §7 tests: unit (Task 1), builder integration (Task 4, five tests), explain-based (Task 5 and the contained plan assertion in Task 6), contained behaviour (Task 6), corpus (Task 8). The `set_schema_version` delete-then-insert hazard, not in the spec but fatal to §4.2 step 4, is Task 3.

**Placeholders.** None. Every code step carries the code. The two "check the driver version" notes point at existing call sites to copy from.

**Type consistency.** `SearchIndexSpec { name: &'static str, keys: Document, partial: Option<Document>, build: IndexBuild }` in Task 1 is what Tasks 3 and 4 consume. `BuildOutcome` variants in Task 4's implementation match the five tests. `wait_for_search_index_build(&self) -> Option<BuildOutcome>` is used identically in Tasks 4 and 5. `matching_contained(&self, db, tenant_id, contained_type, query, offset, limit, want_total) -> StorageResult<ContainedPage>` is called with that shape in both branches of `search_contained`. `document_to_stored_resource(doc, tenant, fallback)` matches `storage.rs:435`.
