# MongoDB contained-rows collection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A standard MongoDB search no longer matches a container through its same-type contained resources, without changing any standard-search query or rebuilding any value index.

**Architecture:** Contained index rows move from `search_index` to a new `search_index_contained` collection with the same document shape minus `is_contained`. The document builder returns own and contained rows separately; every insert and delete path acts on both collections; only the `_contained` pipeline reads the new one. A boot step moves existing contained rows, and index generation 3 drops the old partial `idx_search_contained` from `search_index`.

**Tech Stack:** Rust (edition 2024), `mongodb` driver 3.x, `bson` `doc!`, testcontainers integration suite `crates/persistence/tests/mongodb_tests.rs`.

**Spec:** `docs/superpowers/specs/2026-09-16-mongodb-contained-rows-collection-design.md`

## Global Constraints

- Every `search_index` read keeps a value-field predicate; no standard-search envelope changes in this plan. The three `*_is_a_covered_v2_scan` integration tests must pass unchanged.
- `search_offloaded` deployments write nothing to either collection.
- Generation-2 value index specs (`_v2` names, `idx_search_composite`, `idx_search_resource`) do not change keys or partial filters.
- Commit trailer on every commit: `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Uo4p2nXn3U3j54MyW4Aqd`.
- CI clippy for the crate: `cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation`; `cargo fmt --all -- --check` clean.
- Integration tests run with `cargo test -p helios-persistence --features mongodb --test mongodb_tests <filter>`; the whole suite must stay green at the end (158 tests before this plan).
- Never touch the host container `hfs-mongo` or set `HFS_TEST_MONGODB_URL`.

## File map

| File | Responsibility in this plan |
|---|---|
| `crates/persistence/src/backends/mongodb/search_index_catalog.rs` | Collection constants, generation 3, `current_specs()`, `contained_specs()`, superseded contained spec, script generators parameterised by collection |
| `crates/persistence/src/backends/mongodb/backend.rs` | `SEARCH_INDEX_CONTAINED_COLLECTION` constant |
| `crates/persistence/src/backends/mongodb/schema.rs` | Inline index creation on both collections; `contained_rows_moved` flag helpers |
| `crates/persistence/src/backends/mongodb/storage.rs` | Split document builder; insert and delete paths on both collections; purge paths; reindex page writer |
| `crates/persistence/src/backends/mongodb/bulk_ingest.rs` | Bulk ingest writes and clears both collections |
| `crates/persistence/src/backends/mongodb/search_impl.rs` | `matching_contained` reads the new collection |
| `crates/persistence/src/backends/mongodb/search_index_builder.rs` | Row-move step; superseded contained index dropped; generation 3 recorded |
| `docs/mongodb/search-index-v2.mongosh.js`, `docs/mongodb/search-index-contained.mongosh.js`, `docs/mongodb/search-index-contained-rollback.mongosh.js`, `docs/mongodb/search-indexes.md` | Operator scripts (generated, pinned by tests) and notes |
| `crates/persistence/tests/mongodb_tests.rs` | Integration tests |

---

### Task 1: Catalog, constants, scripts and operator notes

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/search_index_catalog.rs`
- Modify: `crates/persistence/src/backends/mongodb/backend.rs:234` (constants block)
- Modify: `crates/persistence/src/backends/mongodb/schema.rs:13,251-258` (rename callers only)
- Modify: `crates/persistence/src/backends/mongodb/search_index_builder.rs` (rename callers only; behaviour changes are Task 6)
- Modify: `docs/mongodb/search-index-v2.mongosh.js` (regenerate), create `docs/mongodb/search-index-contained.mongosh.js`, create `docs/mongodb/search-index-contained-rollback.mongosh.js`, modify `docs/mongodb/search-indexes.md`

**Interfaces:**
- Produces: `pub(crate) const SEARCH_INDEX_CONTAINED_COLLECTION: &str = "search_index_contained"` (catalog) and `MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION` (backend.rs, same value); `pub(crate) const SEARCH_INDEX_GENERATION: i32 = 3`; `pub(crate) fn current_specs() -> Vec<SearchIndexSpec>` (was `generation2_specs`, minus the contained spec); `pub(crate) fn contained_specs() -> Vec<SearchIndexSpec>` (two inline specs for the new collection); `pub(crate) fn superseded_contained_spec() -> SearchIndexSpec`; `pub(crate) fn create_indexes_command_for(collection: &str, specs: &[&SearchIndexSpec]) -> Document`; `pub(crate) fn mongosh_script_for(collection: &str, specs: &[SearchIndexSpec]) -> String`; `pub(crate) fn contained_rollback_script() -> String`.

- [ ] **Step 1: Write the failing catalog tests**

In the `tests` module of `search_index_catalog.rs`, replace `generation2_has_nine_value_specs_plus_contained_plus_two_unchanged` and `contained_spec_is_partial_on_is_contained_true` with:

```rust
    #[test]
    fn current_specs_are_nine_value_specs_plus_two_unchanged_and_no_contained() {
        let names: Vec<&str> = current_specs().iter().map(|s| s.name).collect();
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
                "idx_search_composite",
                "idx_search_resource",
            ]
        );
        assert_eq!(SEARCH_INDEX_GENERATION, 3);
    }

    #[test]
    fn contained_specs_are_two_inline_plain_indexes_on_the_contained_collection() {
        let specs = contained_specs();
        assert_eq!(specs.len(), 2);
        assert!(specs.iter().all(|s| s.build == IndexBuild::Inline && s.partial.is_none()));
        let by_name = |n: &str| specs.iter().find(|s| s.name == n).expect(n).clone();
        assert_eq!(
            by_name("idx_search_contained").keys.keys().collect::<Vec<_>>(),
            vec![
                "tenant_id",
                "contained_type",
                "param_name",
                "resource_type",
                "resource_id",
                "contained_local_id"
            ]
        );
        assert_eq!(
            by_name("idx_search_contained_resource").keys.keys().collect::<Vec<_>>(),
            vec!["tenant_id", "resource_type", "resource_id"]
        );
        assert_eq!(SEARCH_INDEX_CONTAINED_COLLECTION, "search_index_contained");
    }

    #[test]
    fn superseded_contained_spec_is_the_generation2_partial_index() {
        let c = superseded_contained_spec();
        assert_eq!(c.name, "idx_search_contained");
        assert_eq!(
            c.keys.keys().collect::<Vec<_>>(),
            vec![
                "tenant_id",
                "contained_type",
                "is_contained",
                "param_name",
                "resource_type",
                "resource_id",
                "contained_local_id"
            ]
        );
        assert_eq!(c.partial, Some(doc! { "is_contained": true }));
    }

    #[test]
    fn create_indexes_command_for_targets_the_named_collection() {
        let specs = contained_specs();
        let refs: Vec<&SearchIndexSpec> = specs.iter().collect();
        let cmd = create_indexes_command_for(SEARCH_INDEX_CONTAINED_COLLECTION, &refs);
        assert_eq!(cmd.get_str("createIndexes"), Ok("search_index_contained"));
        assert_eq!(cmd.get_array("indexes").unwrap().len(), 2);
    }

    #[test]
    fn contained_prebuild_script_in_docs_matches_the_catalog() {
        let expected = mongosh_script_for(SEARCH_INDEX_CONTAINED_COLLECTION, &contained_specs());
        let on_disk =
            include_str!("../../../../../docs/mongodb/search-index-contained.mongosh.js");
        assert_eq!(on_disk, expected, "docs/mongodb/search-index-contained.mongosh.js is stale");
    }

    #[test]
    fn contained_rollback_script_in_docs_matches_the_catalog() {
        let on_disk = include_str!(
            "../../../../../docs/mongodb/search-index-contained-rollback.mongosh.js"
        );
        assert_eq!(on_disk, contained_rollback_script());
    }
```

Update `value_specs()`, `superseded_v1_names_are_exactly_the_nine_old_value_indexes_and_do_not_collide`, `composite_and_resource_are_unchanged_and_inline`, `every_value_spec_...`, `token_v2_is_code_first`, `create_indexes_command_carries_name_key_and_partial_filter` and `prebuild_script_in_docs_matches_the_catalog` to call `current_specs()` instead of `generation2_specs()`; the last one's expected value is `mongosh_script(&current_specs().into_iter().filter(|s| s.build == IndexBuild::Background).collect::<Vec<_>>())` exactly as it filters today.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p helios-persistence --features mongodb --lib search_index_catalog`
Expected: compile errors for `current_specs`, `contained_specs`, `superseded_contained_spec`, `create_indexes_command_for`, `mongosh_script_for`, `contained_rollback_script`, `SEARCH_INDEX_CONTAINED_COLLECTION`.

- [ ] **Step 3: Implement the catalog**

In `search_index_catalog.rs`:

```rust
/// The collection every value spec in this catalog belongs to.
pub(crate) const SEARCH_INDEX_COLLECTION: &str = "search_index";
/// Where contained-resource rows live since generation 3 (#1160). The
/// standard search never reads it, which is what excludes contained rows
/// from a standard search by construction.
pub(crate) const SEARCH_INDEX_CONTAINED_COLLECTION: &str = "search_index_contained";

/// Recorded in the `schema_version` document as `search_indexes.generation`
/// once every [`IndexBuild::Background`] spec is present and the superseded
/// indexes are gone. Generation 3 = generation 2 minus `idx_search_contained`
/// on `search_index` (contained rows moved to their own collection, #1160).
pub(crate) const SEARCH_INDEX_GENERATION: i32 = 3;
```

Rename `generation2_specs` to `current_specs`, delete the `idx_search_contained` entry from it (keep the nine `value_v2` entries and the two inline specs exactly as they are), and update the doc comment to "Every index `search_index` should have once the current generation is complete."

Add:

```rust
/// The indexes of `search_index_contained`. Created inline at boot: contained
/// resources are rare, so the collection is small on every deployment.
pub(crate) fn contained_specs() -> Vec<SearchIndexSpec> {
    vec![
        // Same keys as the generation-2 partial index minus `is_contained`
        // (every row here is contained), so the contained pipeline's `$group`
        // still reads its key from the index.
        SearchIndexSpec {
            name: "idx_search_contained",
            keys: doc! {
                "tenant_id": 1_i32,
                "contained_type": 1_i32,
                "param_name": 1_i32,
                "resource_type": 1_i32,
                "resource_id": 1_i32,
                "contained_local_id": 1_i32,
            },
            partial: None,
            build: IndexBuild::Inline,
        },
        // Delete-by-container paths, mirroring `idx_search_resource`.
        SearchIndexSpec {
            name: "idx_search_contained_resource",
            keys: doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "resource_id": 1_i32 },
            partial: None,
            build: IndexBuild::Inline,
        },
    ]
}

/// The generation-2 partial index over contained rows on `search_index`,
/// dropped by the builder once the rows have moved. Exact keys, so the
/// rollback script recreates what existed.
pub(crate) fn superseded_contained_spec() -> SearchIndexSpec {
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
    }
}

/// A raw `createIndexes` command for `specs` on `collection`.
pub(crate) fn create_indexes_command_for(collection: &str, specs: &[&SearchIndexSpec]) -> Document {
    let indexes: Vec<Bson> = specs
        .iter()
        .map(|s| Bson::Document(s.create_indexes_entry()))
        .collect();
    doc! { "createIndexes": collection, "indexes": indexes }
}

pub(crate) fn create_indexes_command(specs: &[&SearchIndexSpec]) -> Document {
    create_indexes_command_for(SEARCH_INDEX_COLLECTION, specs)
}

#[allow(dead_code)]
pub(crate) fn mongosh_script_for(collection: &str, specs: &[SearchIndexSpec]) -> String {
    let refs: Vec<&SearchIndexSpec> = specs.iter().collect();
    let cmd = Bson::Document(create_indexes_command_for(collection, &refs)).into_relaxed_extjson();
    let json = serde_json::to_string_pretty(&cmd).expect("createIndexes command serializes");
    format!(
        "// Generated from crates/persistence/src/backends/mongodb/search_index_catalog.rs.\n\
         // Do not edit by hand: a unit test compares this file to the catalog.\n\
         // Usage: mongosh \"$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE\" <this file>\n\
         // Builds every index in one collection scan; reads and writes continue meanwhile.\n\
         db.runCommand({json});\n"
    )
}

#[allow(dead_code)]
pub(crate) fn mongosh_script(specs: &[SearchIndexSpec]) -> String {
    mongosh_script_for(SEARCH_INDEX_COLLECTION, specs)
}

/// Downgrade helper: copies contained rows back into `search_index` with
/// `is_contained: true` and recreates the generation-2 partial index, so a
/// pre-generation-3 binary serves `_contained` searches again.
#[allow(dead_code)]
pub(crate) fn contained_rollback_script() -> String {
    let spec = superseded_contained_spec();
    let refs = [&spec];
    let cmd = Bson::Document(create_indexes_command(&refs)).into_relaxed_extjson();
    let json = serde_json::to_string_pretty(&cmd).expect("createIndexes command serializes");
    format!(
        "// Generated from crates/persistence/src/backends/mongodb/search_index_catalog.rs.\n\
         // Do not edit by hand: a unit test compares this file to the catalog.\n\
         // Usage: mongosh \"$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE\" <this file>\n\
         // Copies contained rows back into search_index (with is_contained: true) and\n\
         // recreates the generation-2 partial index, for a rollback to a binary that\n\
         // predates search_index_contained. Idempotent: rows already present are skipped.\n\
         db.search_index_contained.find().forEach(function (row) {{\n\
         \x20 row.is_contained = true;\n\
         \x20 try {{ db.search_index.insertOne(row); }} catch (e) {{ if (e.code !== 11000) throw e; }}\n\
         }});\n\
         db.runCommand({json});\n"
    )
}
```

In `backend.rs` constants block add `pub(crate) const SEARCH_INDEX_CONTAINED_COLLECTION: &'static str = super::search_index_catalog::SEARCH_INDEX_CONTAINED_COLLECTION;`.

Rename every `generation2_specs` caller (`schema.rs` import and `ensure_search_indexes`, `search_index_builder.rs` import, `inspect`, and its unit tests) to `current_specs`. The builder's behaviour changes are Task 6; in this task only the name changes.

Regenerate the scripts: write a throwaway test or use `cargo test` output? Simpler: add a temporary `#[test] #[ignore] fn print_scripts()` that prints the three strings, run it with `--ignored --nocapture`, paste each into its file, delete the temporary test. `docs/mongodb/search-index-v2.mongosh.js` loses the `idx_search_contained` entry; the two new files are created.

Append to `docs/mongodb/search-indexes.md`, before "## Downgrading":

```markdown
## Contained rows (generation 3)

Rows extracted from a resource's `contained` entries live in their own collection, `search_index_contained` (#1160). A standard search reads only `search_index`, so it can no longer match a container through a same-type contained resource; `_contained=true|both` searches read `search_index_contained`. Its two indexes are created inline at every boot:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-contained.mongosh.js

On the first boot of a generation-3 binary HFS moves any contained rows still in `search_index` into the new collection, in pages of 1,000, in every `HFS_MONGODB_INDEX_BUILD` mode (it is a correctness fix, not an index build), records `search_indexes.contained_rows_moved: true`, and then drops the old partial `idx_search_contained` from `search_index` under the usual mode rules (in `off` mode it warns and names the `dropIndex` command). Most deployments have no contained rows, so the move is one empty find.
```

And under "## Downgrading" add a paragraph:

```markdown
A binary from before generation 3 reads contained matches from `search_index` only, so `_contained` searches return nothing for rows that were moved (standard searches are unaffected). Before rolling back, copy the rows back and recreate the partial index:

    mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" docs/mongodb/search-index-contained-rollback.mongosh.js
```

Update the first bullet list at the top of the doc: "Generation-2 indexes (nine partial value indexes named `idx_search_*_v2`)" (drop "and `idx_search_contained`"), and "records `search_indexes.generation: 3`".

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p helios-persistence --features mongodb --lib search_index_catalog` and `cargo check -p helios-persistence --features mongodb --tests`
Expected: all catalog tests pass; the crate compiles (builder tests may reference `expected_generation2_names`-style helpers; they are updated in Task 6, so only compile is required here).

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/search_index_catalog.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/src/backends/mongodb/schema.rs crates/persistence/src/backends/mongodb/search_index_builder.rs docs/mongodb/
git commit -m "feat(mongodb): catalog generation 3 with a contained-rows collection (#1160)"
```

---

### Task 2: Inline indexes on the contained collection at boot

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/schema.rs:247-258` (`ensure_search_indexes`)
- Test: `crates/persistence/tests/mongodb_tests.rs` (`mongodb_integration_boot_creates_only_inline_search_indexes_and_keeps_generation_record`, ~line 11510)

**Interfaces:**
- Consumes: `contained_specs()`, `SEARCH_INDEX_CONTAINED_COLLECTION` from Task 1.
- Produces: at boot, `search_index_contained` has `_id_`, `idx_search_contained`, `idx_search_contained_resource`.

- [ ] **Step 1: Extend the boot integration test**

After the existing `assert_eq!(names, vec!["_id_", "idx_search_composite", "idx_search_resource"])` add:

```rust
    // Generation 3: the contained collection gets its two indexes inline too.
    let contained_names = index_names(&db, "search_index_contained").await;
    assert_eq!(
        contained_names,
        vec!["_id_", "idx_search_contained", "idx_search_contained_resource"]
    );
```

and generalise the helper `search_index_names(db)` (~line 11559) into `index_names(db, collection)` with `search_index_names(db)` delegating to `index_names(db, "search_index")`.

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests boot_creates_only_inline`
Expected: FAIL, the contained collection has only `_id_` (or does not exist: make `index_names` return `vec!["_id_"]`-less empty vec on NamespaceNotFound — it should return whatever `list_index_names` returns, which errors on a missing collection; handle with `.unwrap_or_default()` and expect the assertion to fail on an empty vec).

- [ ] **Step 3: Implement**

```rust
async fn ensure_search_indexes(database: &Database) -> StorageResult<()> {
    let search_index = database.collection::<Document>(SEARCH_INDEX_COLLECTION);
    for spec in current_specs()
        .iter()
        .filter(|s| s.build == IndexBuild::Inline)
    {
        search_index.create_index(spec.index_model()).await?;
    }
    // Contained rows live in their own, small collection (#1160); both of
    // its indexes are cheap enough to await at boot.
    let contained = database.collection::<Document>(SEARCH_INDEX_CONTAINED_COLLECTION);
    for spec in contained_specs() {
        contained.create_index(spec.index_model()).await?;
    }
    Ok(())
}
```

Import `SEARCH_INDEX_CONTAINED_COLLECTION` and `contained_specs` in `schema.rs`. Update the function doc comment ("...and the contained index" wording goes).

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests boot_creates_only_inline`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/schema.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "feat(mongodb): create the contained collection's indexes at boot (#1160)"
```

---

### Task 3: Split the document builder and write to both collections

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` (`search_index_documents` ~2093, `search_index_documents_checked` ~2113, `build_contained_index_document` ~2360, `index_resource` ~2180, `index_resource_in_bundle_transaction` ~3965, `write_search_entries_page` ~4412)
- Modify: `crates/persistence/src/backends/mongodb/bulk_ingest.rs` (`write_search_index` ~905)
- Test: `crates/persistence/tests/mongodb_tests.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Default)]
  pub(super) struct SearchIndexDocuments {
      /// The resource's own rows, for `search_index`.
      pub own: Vec<Document>,
      /// Rows extracted from `contained` entries, for `search_index_contained`.
      pub contained: Vec<Document>,
  }
  impl SearchIndexDocuments { pub fn is_empty(&self) -> bool { self.own.is_empty() && self.contained.is_empty() } }
  pub(super) fn search_index_documents(&self, tenant_id, resource_type, resource_id, resource) -> SearchIndexDocuments
  pub(super) fn search_index_documents_checked(&self, ...) -> (SearchIndexDocuments, Option<String>)
  ```
  and a private helper used by every insert path:
  ```rust
  async fn insert_search_index_documents(&self, db: &mongodb::Database, docs: SearchIndexDocuments, session: Option<&mut ClientSession>) -> StorageResult<()>
  ```
  which does `insert_many` on `search_index` for `own` (when non-empty) and on `search_index_contained` for `contained` (when non-empty), through the session when given, mapping errors to `internal_error("Failed to insert search index entries: {e}")` / `"Failed to insert search_index_contained entries: {e}"`.

- [ ] **Step 1: Write the failing integration test**

Next to `mongodb_integration_contained_search` (~line 3368):

```rust
#[tokio::test]
async fn mongodb_integration_contained_rows_are_written_to_their_own_collection() {
    let Some(backend) = create_backend_with_full_registry("contained_rows_split").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-contained-split");
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation", "id": "holder", "status": "final",
                "code": { "coding": [{ "system": "http://loinc.org", "code": "OWN" }] },
                "subject": { "reference": "#p" },
                "contained": [{ "resourceType": "Patient", "id": "p", "name": [{ "family": "Inner" }] }]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let db = raw_test_client(&backend.config().connection_string)
        .await
        .unwrap()
        .database(&backend.config().database_name);
    let own = db.collection::<Document>("search_index");
    let contained = db.collection::<Document>("search_index_contained");
    let key = doc! { "tenant_id": "tenant-contained-split", "resource_type": "Observation", "resource_id": "holder" };
    assert!(own.count_documents(key.clone()).await.unwrap() > 0, "own rows in search_index");
    assert_eq!(
        own.count_documents(doc! { "is_contained": true }).await.unwrap(),
        0,
        "no contained row may land in search_index"
    );
    let inner = contained
        .find_one(doc! { "tenant_id": "tenant-contained-split", "contained_type": "Patient", "param_name": "name" })
        .await
        .unwrap()
        .expect("contained row in search_index_contained");
    assert_eq!(inner.get_str("resource_type"), Ok("Observation"));
    assert_eq!(inner.get_str("resource_id"), Ok("holder"));
    assert_eq!(inner.get_str("contained_local_id"), Ok("p"));
    assert!(!inner.contains_key("is_contained"), "is_contained is implied by the collection");
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests contained_rows_are_written_to_their_own_collection`
Expected: FAIL at "no contained row may land in search_index" (count is > 0).

- [ ] **Step 3: Implement**

In `storage.rs`:

1. Add `SearchIndexDocuments` (above `search_index_documents`).
2. `build_contained_index_document`: delete the line `doc.insert("is_contained", true);` and update its doc comment ("...carrying the contained resource's type and local id; written to `search_index_contained`").
3. `search_index_documents_checked`: build `own` exactly as today (`index_docs` becomes `own`), build `contained` from the `extract_contained` loop into a separate `Vec`, return `(SearchIndexDocuments { own, contained }, failure)`. `search_index_documents` returns `.0`.
4. `insert_search_index_documents` as specified in Interfaces. Use it in `index_resource` (replace the `if index_docs.is_empty() { return Ok(()) }` + `insert_many` block; keep the early return when both are empty).
5. `index_resource_in_bundle_transaction`: replace the inline `extract` / fallback block with `let (docs, _failure) = self.search_index_documents_checked(tenant_id, resource_type, resource_id, resource);` then `self.insert_search_index_documents(db, docs, Some(session)).await`. This is the same own-row output by construction and adds the contained rows this path never wrote before; note it in the report.
6. `write_search_entries_page`: `Prepared { docs: SearchIndexDocuments, failure }`; the delete stays on `search_index` for now (Task 4 adds the contained delete); the flattening/chunked insert writes `own` docs to `search_index` and `contained` docs to `search_index_contained`, keeping the existing chunk size and error fan-out (a failed contained insert fans out to the page exactly like a failed own insert). Keep whatever count the existing test `mongodb_integration_reindex_page_counts_contained_entries` (~line 6172) asserts: read that test first; it counts rows with `is_contained: true` in `search_index` and must be changed to count rows in `search_index_contained` instead (the count itself is unchanged).

In `bulk_ingest.rs` `write_search_index`: keep `documents` for own rows and add `contained_documents`; the insert closure inserts each non-empty vec into its collection with the same `ordered(false)` chunking; the replay delete (Task 4) covers both. Bind `let contained_collection = db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION);` next to `collection`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests contained` and `cargo test -p helios-persistence --features mongodb --test mongodb_tests reindex_page`
Expected: the new test passes; `reindex_page_counts_contained_entries` passes after its collection change; the other `contained` tests still pass because `matching_contained` still reads `search_index`... they will FAIL now (rows moved, reader not yet moved). That is expected between Task 3 and Task 5: run only the new test and the reindex tests here, and note the temporary red in the ledger.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/bulk_ingest.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "feat(mongodb): write contained index rows to search_index_contained (#1160)"
```

---

### Task 4: Delete paths cover both collections

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/storage.rs` (`delete_search_index` ~2222, `delete_search_index_in_bundle_transaction` ~4031, `purge` ~4100-4150, `purge_tenant_data` ~1817, `write_search_entries_page` delete ~4462)
- Modify: `crates/persistence/src/backends/mongodb/bulk_ingest.rs` (stale clearing ~920 and replay delete ~990)
- Test: `crates/persistence/tests/mongodb_tests.rs`

**Interfaces:**
- Produces: private `async fn delete_search_index_rows_for(&self, db, tenant_id, resource_type, id_filter: Bson, session: Option<&mut ClientSession>) -> StorageResult<()>` that runs the same `delete_many({tenant_id, resource_type, resource_id: <id_filter>})` on `search_index` and then on `search_index_contained` (id_filter is either a string id or `{ "$in": ids }`). `delete_search_index` and `delete_search_index_in_bundle_transaction` call it.

- [ ] **Step 1: Write the failing integration test**

```rust
#[tokio::test]
async fn mongodb_integration_update_and_delete_leave_no_orphan_contained_rows() {
    let Some(backend) = create_backend_with_full_registry("contained_orphans").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-contained-orphans");
    let with_contained = |family: &str| json!({
        "resourceType": "Observation", "id": "holder", "status": "final",
        "subject": { "reference": "#p" },
        "contained": [{ "resourceType": "Patient", "id": "p", "name": [{ "family": family }] }]
    });
    backend.create(&tenant, "Observation", with_contained("First"), FhirVersion::default()).await.unwrap();
    let db = raw_test_client(&backend.config().connection_string).await.unwrap().database(&backend.config().database_name);
    let contained = db.collection::<Document>("search_index_contained");
    let key = doc! { "tenant_id": "tenant-contained-orphans", "resource_type": "Observation", "resource_id": "holder" };
    let names = |c: &mongodb::Collection<Document>| { let c = c.clone(); let key = key.clone(); async move {
        use futures::TryStreamExt;
        let rows: Vec<Document> = c.find(doc! { "param_name": "name" }).await.unwrap().try_collect().await.unwrap();
        rows.into_iter().filter(|r| r.get_str("resource_id") == Ok("holder")).filter_map(|r| r.get_str("value_string").ok().map(str::to_string)).collect::<Vec<_>>()
    }};
    assert_eq!(names(&contained).await, vec!["first"]);

    // Update: the old contained rows are gone, the new ones are there.
    backend.update(&tenant, "Observation", "holder", with_contained("Second"), None, FhirVersion::default()).await.unwrap();
    assert_eq!(names(&contained).await, vec!["second"]);

    // Delete: nothing left in either collection.
    backend.delete(&tenant, "Observation", "holder").await.unwrap();
    assert_eq!(contained.count_documents(key.clone()).await.unwrap(), 0);
    assert_eq!(db.collection::<Document>("search_index").count_documents(key).await.unwrap(), 0);
}
```

Adjust `update`/`delete` call shapes to the backend's real signatures (read a neighbouring test that updates and deletes, e.g. `mongodb_integration_update_with_match_and_delete_with_match`). `value_string` is stored folded/lower-cased; if the stored form differs, assert on the field the string handler actually writes (read `build_search_index_document` for the string case).

Add two more tests in the same shape: `mongodb_integration_transaction_bundle_indexes_and_clears_contained_rows` (create the holder through a transaction bundle, assert the contained row exists, delete it through a second transaction bundle, assert both collections empty for the id; model the bundle calls on `mongodb_integration_transaction_bundle_mixed_operations_and_idempotent_delete`), and `mongodb_integration_purge_tenant_clears_contained_rows` (create the holder, call `purge_tenant_data` the way `mongodb_integration_purge_tenant_data` does, assert `search_index_contained` has no rows for the tenant). If a test in this file already exercises the admin resource purge (`purge` at ~line 4100; grep for `.purge(`), add a sibling `mongodb_integration_purge_resource_clears_contained_rows` in the same shape; if none exists, say so in the report and cover it in the update/delete test by calling the purge path the same way the REST layer does.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests orphan` (and the two others by name)
Expected: FAIL on the update assertion (`["first", "second"]` present) or the delete count.

- [ ] **Step 3: Implement**

- `delete_search_index_rows_for` as in Interfaces; `delete_search_index` and `delete_search_index_in_bundle_transaction` delegate to it with `Bson::String(resource_id.into())`.
- `purge` (~4100): after the `search_index.delete_many` add the same `delete_many` on `search_index_contained` with `.or_query_error("Failed to purge contained search index")`.
- `purge_tenant_data`: add `MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION` to the collection array (after `SEARCH_INDEX_COLLECTION`).
- `write_search_entries_page`: the per-type delete runs on both collections (same filter); either failure fans out as today.
- `bulk_ingest.rs`: the stale-row clearing and the replay delete each run their `delete_many` on both collections.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests orphan`, `transaction_bundle_indexes_and_clears_contained`, `purge_tenant_clears_contained`, `reindex_page`, `bulk_submit` (whatever bulk-ingest integration tests exist in this file: `grep -n "bulk_submit\|bulk_ingest" ` and run those names)
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/storage.rs crates/persistence/src/backends/mongodb/bulk_ingest.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "feat(mongodb): delete contained index rows alongside own rows everywhere (#1160)"
```

---

### Task 5: The `_contained` pipeline reads the new collection; the bug's tests

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/search_impl.rs:842` (`matching_contained`) and the `$match` at ~875
- Test: `crates/persistence/tests/mongodb_tests.rs` (new tests; rewrite of `mongodb_integration_contained_both_dedupes_a_container_that_is_also_a_top_level_match` ~3879)

**Interfaces:**
- Consumes: rows in `search_index_contained` (Task 3) with `tenant_id`, `contained_type`, `param_name`, `resource_type`, `resource_id`, `contained_local_id`, value fields.

- [ ] **Step 1: Write the failing tests**

```rust
/// #1160: a standard search must not match a container through a same-type
/// contained resource; `_contained=true` must, and `both` must return it once.
#[tokio::test]
async fn mongodb_integration_standard_search_ignores_same_type_contained_values() {
    use helios_persistence::types::{ContainedMode, ContainedReturn};
    let Some(backend) = create_backend_with_full_registry("same_type_contained").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-same-type-contained");
    // The holder's only `code = X` lives inside a contained Observation.
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation", "id": "holder", "status": "final",
                "code": { "coding": [{ "system": "http://loinc.org", "code": "OUTER" }] },
                "contained": [{
                    "resourceType": "Observation", "id": "inner", "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "X" }] }
                }]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let mut q = SearchQuery::new("Observation").with_parameter(SearchParameter {
        name: "code".into(),
        param_type: SearchParamType::Token,
        modifier: None,
        values: vec![SearchValue::eq("X")],
        chain: vec![],
        components: vec![],
    });
    let r = backend.search(&tenant, &q).await.unwrap();
    assert!(r.resources.items.is_empty(), "standard search matched through a contained value: {:?}", r.resources.items.iter().map(|x| x.url()).collect::<Vec<_>>());
    assert_eq!(backend.search_count(&tenant, &q).await.unwrap(), 0);

    q.contained = ContainedMode::True;
    q.contained_return = ContainedReturn::Container;
    let r = backend.search(&tenant, &q).await.unwrap();
    assert_eq!(r.resources.items.iter().map(|x| x.url()).collect::<Vec<_>>(), vec!["Observation/holder"]);

    q.contained = ContainedMode::Both;
    let r = backend.search(&tenant, &q).await.unwrap();
    assert_eq!(r.resources.items.iter().map(|x| x.url()).collect::<Vec<_>>(), vec!["Observation/holder"]);
}

/// Cross-type containment is unchanged: the Observation is found as the
/// container of a Patient match only under `_contained`.
#[tokio::test]
async fn mongodb_integration_cross_type_contained_search_still_returns_the_container() {
    use helios_persistence::types::{ContainedMode, ContainedReturn};
    let Some(backend) = create_backend_with_full_registry("cross_type_contained").await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let tenant = create_tenant("tenant-cross-type-contained");
    backend
        .create(
            &tenant,
            "Observation",
            json!({
                "resourceType": "Observation", "id": "obs", "status": "final",
                "subject": { "reference": "#p" },
                "contained": [{ "resourceType": "Patient", "id": "p", "name": [{ "family": "Crosstype" }] }]
            }),
            FhirVersion::default(),
        )
        .await
        .unwrap();
    let mut q = SearchQuery::new("Patient").with_parameter(SearchParameter {
        name: "name".into(),
        param_type: SearchParamType::String,
        modifier: None,
        values: vec![SearchValue::eq("Crosstype")],
        chain: vec![],
        components: vec![],
    });
    assert!(backend.search(&tenant, &q).await.unwrap().resources.items.is_empty());
    q.contained = ContainedMode::True;
    q.contained_return = ContainedReturn::Container;
    let r = backend.search(&tenant, &q).await.unwrap();
    assert_eq!(r.resources.items.iter().map(|x| x.url()).collect::<Vec<_>>(), vec!["Observation/obs"]);
}
```

Check `ContainedMode`'s variant names and `search_count`'s signature against `crates/persistence/src/types/search_params.rs` and the backend before using them; the neighbouring `_contained` tests show the exact spelling.

Rewrite the dedupe test's fixture (a): `dual` keeps its own `name = Smith` **and** its contained Patient `inner` named Smith, so it matches top-level through its own value and through the contained path. The comment now reads: "A top-level Patient match that is ALSO the container of a contained match: its own name is Smith and it contains another Patient named Smith. Before #1160 every same-type container was a top-level match through its contained rows, so this case was not a real dedupe." The assertions (`["Patient/dual", "Observation/obs-holder"]`, `total == Some(3)`) stay.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests contained`
Expected: `standard_search_ignores_same_type_contained_values` fails at the `_contained=true` step (the pipeline still reads `search_index`, which no longer holds contained rows → empty); the older `_contained` tests fail the same way.

- [ ] **Step 3: Implement**

In `matching_contained`: `let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_CONTAINED_COLLECTION);` (rename the binding to `contained_rows`), and in the `$match` remove `"is_contained": true,`. Update the function's doc comment to name the collection. Grep `search_impl.rs` for any other `is_contained` and remove it if it only served this pipeline (there is none today).

- [ ] **Step 4: Run to verify**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests contained`
Expected: every `contained` test passes, including the two new ones and the rewritten dedupe.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/search_impl.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "fix(mongodb): standard search no longer matches through same-type contained rows (#1160)"
```

---

### Task 6: Migration step and generation 3 in the builder

**Files:**
- Modify: `crates/persistence/src/backends/mongodb/schema.rs` (flag helpers next to `get_search_index_generation` ~484)
- Modify: `crates/persistence/src/backends/mongodb/search_index_builder.rs` (`run_inner` ~207, `inspect` ~346, `record_if_needed` ~334, unit tests)
- Modify: `crates/persistence/src/backends/mongodb/backend.rs:501-525` (doc comment only)
- Test: `crates/persistence/tests/mongodb_tests.rs` (builder tests ~11510-11900)

**Interfaces:**
- Produces in `schema.rs`:
  ```rust
  pub(super) async fn contained_rows_moved(database: &Database) -> StorageResult<bool>   // schema_version.search_indexes.contained_rows_moved == true
  pub(super) async fn set_contained_rows_moved(database: &Database) -> StorageResult<()> // $set search_indexes.contained_rows_moved: true (upsert)
  ```
  Note `set_search_index_generation` replaces the whole `search_indexes` subdocument with `$set: { search_indexes: {...} }`; change it to set the two fields individually (`"search_indexes.generation"`, `"search_indexes.completed_at"`) so the flag survives.
- Produces in the builder: `async fn move_contained_rows(&self) -> StorageResult<u64>` (rows moved), called first in `run_inner` in every mode; `inspect` marks `superseded_contained_spec().name` present in `superseded_present` when a `search_index` index of that name has a `partialFilterExpression` (the generation-2 shape; the new plain index of the same name lives on the other collection and is never listed here); `BuildOutcome::Built.dropped` includes `"idx_search_contained"` when it was dropped.

- [ ] **Step 1: Write the failing tests**

Unit, in the builder's tests module (find how existing tests build `Inspection` / `listed_indexes` fixtures and follow them): `superseded_present_includes_the_generation2_contained_index` — a listed index named `idx_search_contained` with `partialFilterExpression {is_contained: true}` is classified superseded; `off_mode_warns_about_the_contained_index_and_drops_nothing` if the existing `off` test enumerates warned names, extend its expectation.

Integration, next to `mongodb_integration_builder_upgrades_a_generation1_database_and_drops_v1` (~11725):

```rust
#[tokio::test]
async fn mongodb_integration_builder_moves_contained_rows_and_drops_the_old_partial_index() {
    let Some(cs) = shared_mongo::connection_string().await else {
        eprintln!("Skipping (requires Docker or HFS_TEST_MONGODB_URL)");
        return;
    };
    let db_name = build_test_database_name("builder_contained_move");
    let db = raw_test_client(&cs).await.unwrap().database(&db_name);
    // A generation-2 database: the old partial index and two contained rows
    // written the old way, plus one own row that must stay put.
    let old = superseded_contained_spec_model(); // helper: IndexModel with the generation-2 keys and partial filter, mirroring seed_generation1_indexes
    db.collection::<Document>("search_index").create_index(old).await.unwrap();
    db.collection::<Document>("search_index")
        .insert_many(vec![
            doc! { "tenant_id": "t", "resource_type": "Observation", "resource_id": "holder", "param_name": "code", "param_type": "token", "value_token_code": "OUTER" },
            doc! { "tenant_id": "t", "resource_type": "Observation", "resource_id": "holder", "param_name": "name", "param_type": "string", "value_string": "smith", "is_contained": true, "contained_type": "Patient", "contained_local_id": "p" },
            doc! { "tenant_id": "t", "resource_type": "Observation", "resource_id": "holder", "param_name": "gender", "param_type": "token", "value_token_code": "female", "is_contained": true, "contained_type": "Patient", "contained_local_id": "p" },
        ])
        .await
        .unwrap();
    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    let outcome = backend.wait_for_search_index_build().await.expect("builder ran");
    let BuildOutcome::Built { dropped, .. } = outcome else { panic!("expected Built, got {outcome:?}") };
    assert!(dropped.contains(&"idx_search_contained".to_string()), "{dropped:?}");
    let own = db.collection::<Document>("search_index");
    let contained = db.collection::<Document>("search_index_contained");
    assert_eq!(own.count_documents(doc! { "is_contained": true }).await.unwrap(), 0);
    assert_eq!(own.count_documents(doc! { "resource_id": "holder" }).await.unwrap(), 1);
    assert_eq!(contained.count_documents(doc! { "resource_id": "holder" }).await.unwrap(), 2);
    let moved = contained.find_one(doc! { "param_name": "name" }).await.unwrap().unwrap();
    assert!(!moved.contains_key("is_contained"));
    assert_eq!(moved.get_str("contained_local_id"), Ok("p"));
    assert!(!search_index_names(&db).await.contains(&"idx_search_contained".to_string()));
    assert_eq!(index_names(&db, "search_index_contained").await, vec!["_id_", "idx_search_contained", "idx_search_contained_resource"]);
    let sv = db.collection::<Document>("schema_version").find_one(doc! { "_id": "schema_version" }).await.unwrap().unwrap();
    let si = sv.get_document("search_indexes").unwrap();
    assert_eq!(si.get_i32("generation"), Ok(3));
    assert_eq!(si.get_bool("contained_rows_moved"), Ok(true));

    // Second boot: nothing to move, nothing to build.
    let backend = boot_with_mode(&cs, &db_name, IndexBuildMode::Inline).await;
    assert_eq!(backend.wait_for_search_index_build().await, Some(BuildOutcome::UpToDate));
    assert_eq!(contained.count_documents(doc! { "resource_id": "holder" }).await.unwrap(), 2);
}
```

Update the existing builder tests for generation 3: `GENERATION2_BACKGROUND_NAMES` (find its definition near `expected_generation2_names`) loses `idx_search_contained`; rename the two helpers to `CURRENT_BACKGROUND_NAMES` / `expected_current_names`; `mongodb_integration_builder_fresh_database_ends_with_generation2_set` asserts the new set (and that `search_index_contained` has its two indexes); the upgrade-from-v1 test's `dropped` list is unchanged (a v1 database has no `idx_search_contained`); the boot test asserts `generation` is recorded as 3 where it asserted 2. If a test seeds `idx_search_contained` on `search_index` as part of a "generation-2 set", it now expects the builder to drop it.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p helios-persistence --features mongodb --lib search_index_builder` and `cargo test -p helios-persistence --features mongodb --test mongodb_tests builder`
Expected: the new unit test fails (no superseded classification); the integration test fails at `dropped.contains("idx_search_contained")` or at the `is_contained` count.

- [ ] **Step 3: Implement**

`schema.rs`:

```rust
/// Whether the one-time move of contained rows out of `search_index` has
/// completed on this database (#1160).
pub(super) async fn contained_rows_moved(database: &Database) -> StorageResult<bool> {
    let doc = database
        .collection::<Document>("schema_version")
        .find_one(doc! { "_id": "schema_version" })
        .await?;
    Ok(doc
        .as_ref()
        .and_then(|d| d.get_document("search_indexes").ok())
        .and_then(|s| s.get_bool("contained_rows_moved").ok())
        .unwrap_or(false))
}

pub(super) async fn set_contained_rows_moved(database: &Database) -> StorageResult<()> {
    database
        .collection::<Document>("schema_version")
        .update_one(
            doc! { "_id": "schema_version" },
            doc! { "$set": { "search_indexes.contained_rows_moved": true } },
        )
        .upsert(true)
        .await?;
    Ok(())
}
```

and `set_search_index_generation` uses `doc! { "$set": { "search_indexes.generation": generation, "search_indexes.completed_at": DateTime::now() } }`.

Builder:

```rust
    /// One-time move of contained rows out of `search_index` (#1160). Runs
    /// in every mode: it is a correctness fix for the standard search, not
    /// an index build, and it is bounded by the number of contained rows,
    /// which is small on every deployment. Idempotent page by page, so a
    /// crash mid-way resumes on the next boot.
    async fn move_contained_rows(&self) -> StorageResult<u64> {
        if contained_rows_moved(&self.database).await? {
            return Ok(0);
        }
        let source = self.database.collection::<Document>(SEARCH_INDEX_COLLECTION);
        let target = self.database.collection::<Document>(SEARCH_INDEX_CONTAINED_COLLECTION);
        let mut moved = 0u64;
        loop {
            let mut cursor = source
                .find(doc! { "is_contained": true })
                .limit(MOVE_PAGE as i64)
                .await?;
            let mut page = Vec::with_capacity(MOVE_PAGE);
            while let Some(row) = cursor.try_next().await? {
                page.push(row);
            }
            if page.is_empty() {
                break;
            }
            let ids: Vec<Bson> = page.iter().filter_map(|r| r.get("_id").cloned()).collect();
            let mut rows = page;
            for row in &mut rows {
                row.remove("is_contained");
            }
            // Keep `_id`, so a replayed page is a duplicate-key no-op.
            if let Err(e) = target.insert_many(&rows).ordered(false).await {
                if !is_only_duplicate_keys(&e) {
                    return Err(e.into());
                }
            }
            source.delete_many(doc! { "_id": { "$in": ids } }).await?;
            moved += rows.len() as u64;
            tracing::info!(moved, "moving contained search_index rows to search_index_contained");
        }
        set_contained_rows_moved(&self.database).await?;
        Ok(moved)
    }
```

with `const MOVE_PAGE: usize = 1_000;` and a small `fn is_only_duplicate_keys(e: &mongodb::error::Error) -> bool` that returns true when the error is a `BulkWrite`/`InsertMany` error whose every write error has code 11000 (look at how `bulk_ingest.rs` or `storage.rs` already inspects driver error kinds and copy that shape). Call `let moved = self.move_contained_rows().await?;` as the first statement of `run_inner` (before `inspect`), and log at `info` when `moved > 0`.

`inspect`: after the v1 loop:

```rust
        let contained = superseded_contained_spec();
        if existing
            .iter()
            .any(|d| d.name == contained.name && d.partial.is_some())
        {
            inspection.superseded_present.push(contained.name.to_string());
        }
```

The drop loop already drops every `superseded_present` name from `search_index`, and the `off`-mode warning already names each. `record_if_needed` records `SEARCH_INDEX_GENERATION` (now 3) — unchanged code. Update the log strings that say "generation-2" to use `SEARCH_INDEX_GENERATION`. Update the `backend.rs` doc comment on the builder spawn (lines ~501-505) to say "generation-3 ... and moves any contained rows out of `search_index` first, in every mode".

- [ ] **Step 4: Run to verify**

Run: `cargo test -p helios-persistence --features mongodb --lib search_index_builder`, `cargo test -p helios-persistence --features mongodb --test mongodb_tests builder`, and `cargo test -p helios-persistence --features mongodb --test mongodb_tests boot_creates`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/persistence/src/backends/mongodb/schema.rs crates/persistence/src/backends/mongodb/search_index_builder.rs crates/persistence/src/backends/mongodb/backend.rs crates/persistence/tests/mongodb_tests.rs
git commit -m "feat(mongodb): move existing contained rows at boot and record generation 3 (#1160)"
```

---

### Task 7: Whole-suite verification

**Files:** none new.

- [ ] **Step 1: Covered scans unchanged**

Run: `cargo test -p helios-persistence --features mongodb --test mongodb_tests covered_v2_scan`
Expected: the three covered-scan tests pass with no edits to them.

- [ ] **Step 2: Whole MongoDB suite, lib tests, fmt, clippy**

Run:
```
cargo test -p helios-persistence --features mongodb --test mongodb_tests
cargo test -p helios-persistence --features mongodb --lib backends::mongodb
cargo fmt --all -- --check
cargo clippy -p helios-persistence --all-targets --all-features -- -D warnings -A clippy::items_after_test_module -A clippy::large_enum_variant -A clippy::question_mark -A clippy::collapsible_match -A clippy::collapsible_if -A clippy::field_reassign_with_default -A clippy::doc-overindented-list-items -A clippy::doc-lazy-continuation
```
Expected: the suite passes with the five new tests on top of the previous count; lib tests pass; fmt and clippy clean.

- [ ] **Step 3: Grep for leftovers**

`grep -rn "is_contained" crates/persistence/src/backends/mongodb/` must show only: `superseded_contained_spec` (catalog), the rollback script generator, the migration step's `find`/`remove`, and the builder's inspection. Anything else is a missed reader or writer.

- [ ] **Step 4: Commit any fix-ups**

One commit if steps 1-3 needed changes: `git commit -m "chore(mongodb): tidy after the contained-rows move (#1160)"`.
