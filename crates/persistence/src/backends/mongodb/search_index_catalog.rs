//! Declarative catalog of the `search_index` indexes (#1059, #1084).
//!
//! Generation 2 replaces the nine full value indexes with partial indexes that
//! carry `resource_id` as their trailing key, so a value-filtered scan can be
//! covered, and adds a partial index over contained rows. New names, never
//! changed keys: MongoDB refuses a different key spec under an existing name
//! (`IndexKeySpecsConflict`, 86), which would fail every deployed boot.

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
    SearchIndexSpec {
        name,
        keys,
        partial: None,
        build: IndexBuild::Background,
    }
}

/// Every index `search_index` should have once generation 2 is complete.
pub(crate) fn generation2_specs() -> Vec<SearchIndexSpec> {
    vec![
        value_v2("idx_search_string_v2", &["value_string"]),
        // Code first: the commonest token predicate (`status=final`,
        // `code=8302-2`) carries no system, and a system-first index leaves
        // `value_token_system` as an unbounded middle key for it.
        value_v2(
            "idx_search_token_v2",
            &["value_token_code", "value_token_system"],
        ),
        value_v2("idx_search_date_v2", &["value_date"]),
        value_v2("idx_search_number_v2", &["value_number"]),
        value_v2(
            "idx_search_quantity_v2",
            &["value_quantity_value", "value_quantity_unit"],
        ),
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
        value_v1(
            "idx_search_token",
            &["value_token_system", "value_token_code"],
        ),
        value_v1("idx_search_date", &["value_date"]),
        value_v1("idx_search_number", &["value_number"]),
        value_v1(
            "idx_search_quantity",
            &["value_quantity_value", "value_quantity_unit"],
        ),
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
// Only this module's tests call it (to check the checked-in mongosh scripts
// are up to date); a normal build never generates the scripts at runtime.
#[allow(dead_code)]
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
            assert_eq!(
                &keys[..3],
                &["tenant_id", "resource_type", "param_name"],
                "{}",
                spec.name
            );
            assert_eq!(keys.last(), Some(&"resource_id"), "{}", spec.name);
            let leading_value = keys[3];
            let partial = spec
                .partial
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no partial filter", spec.name));
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
        let token = generation2_specs()
            .into_iter()
            .find(|s| s.name == "idx_search_token_v2")
            .unwrap();
        let keys: Vec<&str> = token.keys.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "tenant_id",
                "resource_type",
                "param_name",
                "value_token_code",
                "value_token_system",
                "resource_id"
            ]
        );
    }

    #[test]
    fn contained_spec_is_partial_on_is_contained_true() {
        let c = generation2_specs()
            .into_iter()
            .find(|s| s.name == "idx_search_contained")
            .unwrap();
        let keys: Vec<&str> = c.keys.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
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
        assert_eq!(c.build, IndexBuild::Background);
    }

    #[test]
    fn composite_and_resource_are_unchanged_and_inline() {
        let specs = generation2_specs();
        let composite = specs
            .iter()
            .find(|s| s.name == "idx_search_composite")
            .unwrap();
        assert_eq!(
            composite.keys,
            doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "resource_id": 1_i32, "param_name": 1_i32, "composite_group": 1_i32 }
        );
        assert_eq!(composite.partial, None);
        assert_eq!(composite.build, IndexBuild::Inline);
        let resource = specs
            .iter()
            .find(|s| s.name == "idx_search_resource")
            .unwrap();
        assert_eq!(
            resource.keys,
            doc! { "tenant_id": 1_i32, "resource_type": 1_i32, "resource_id": 1_i32 }
        );
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
            assert!(
                !g2.contains(name),
                "{name} is both superseded and generation 2"
            );
        }
        // The old token index is system-first; pin it so the rollback script is faithful.
        let token = superseded_v1_specs()
            .into_iter()
            .find(|s| s.name == "idx_search_token")
            .unwrap();
        let keys: Vec<&str> = token.keys.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "tenant_id",
                "resource_type",
                "param_name",
                "value_token_system",
                "value_token_code"
            ]
        );
    }

    #[test]
    fn create_indexes_command_carries_name_key_and_partial_filter() {
        let specs = generation2_specs();
        let background: Vec<&SearchIndexSpec> = specs
            .iter()
            .filter(|s| s.build == IndexBuild::Background)
            .collect();
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
        let expected = mongosh_script(
            &generation2_specs()
                .into_iter()
                .filter(|s| s.build == IndexBuild::Background)
                .collect::<Vec<_>>(),
        );
        let on_disk = include_str!("../../../../../docs/mongodb/search-index-v2.mongosh.js");
        assert_eq!(
            on_disk, expected,
            "docs/mongodb/search-index-v2.mongosh.js is stale; regenerate it from mongosh_script()"
        );
    }

    #[test]
    fn rollback_script_in_docs_matches_the_catalog() {
        let expected = mongosh_script(&superseded_v1_specs());
        let on_disk =
            include_str!("../../../../../docs/mongodb/search-index-v1-rollback.mongosh.js");
        assert_eq!(
            on_disk, expected,
            "docs/mongodb/search-index-v1-rollback.mongosh.js is stale; regenerate it from mongosh_script()"
        );
    }
}
