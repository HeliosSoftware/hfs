//! Schema resolution: how the engine turns a name or canonical URL into a
//! [`FhirSchema`].
//!
//! Mirrors the SearchParameter registry pattern in `crates/fhir/src/search/`:
//! an `Arc`-backed in-memory registry, composable so tenant-uploaded profiles
//! can overlay the embedded core packs.

use crate::schema::FhirSchema;
use std::collections::HashMap;
use std::sync::Arc;

/// Resolves a schema reference — a bare name (`"Patient"`) or a canonical URL
/// (`"http://hl7.org/fhir/StructureDefinition/Patient"`) — to a schema.
pub trait SchemaResolver: Send + Sync {
    /// Returns the schema, or `None` if the reference is unknown.
    fn resolve(&self, reference: &str) -> Option<Arc<FhirSchema>>;
}

/// In-memory schema registry.
///
/// Schemas are indexed under every identity they carry: the explicit key they
/// were inserted with, their `url`, and their `name`. All aliases map to the
/// same `Arc`, so identity-based deduplication in the engine treats them as
/// one schema.
#[derive(Debug, Default)]
pub struct SchemaRegistry {
    map: HashMap<String, Arc<FhirSchema>>,
}

impl SchemaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Extension definitions applicable at any of the given element
    /// contexts (#363): schemas of kind `extension` whose declared context
    /// expressions intersect `contexts`. Deduplicated by canonical URL and
    /// sorted by name for a stable picker order.
    pub fn extensions_applicable(&self, contexts: &[&str]) -> Vec<Arc<FhirSchema>> {
        let mut out: Vec<Arc<FhirSchema>> = self
            .map
            .values()
            .filter(|schema| schema.kind.as_deref() == Some(crate::schema::kind::EXTENSION))
            .filter(|schema| {
                schema
                    .context
                    .as_deref()
                    .is_some_and(|cs| cs.iter().any(|c| contexts.contains(&c.as_str())))
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out.dedup_by(|a, b| a.url == b.url);
        out
    }

    /// Number of distinct keys (aliases included).
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Insert a schema under an explicit key, plus its `url`/`name` aliases.
    ///
    /// The explicit key wins on collision (later inserts overwrite earlier
    /// ones for the same key — last write wins, like the JS fixture maps).
    pub fn insert_named(&mut self, key: impl Into<String>, schema: FhirSchema) {
        let schema = Arc::new(schema);
        let key = key.into();
        if let Some(url) = schema.url.clone()
            && url != key
        {
            self.map.insert(url, Arc::clone(&schema));
        }
        if let Some(name) = schema.name.clone()
            && name != key
        {
            self.map.insert(name, Arc::clone(&schema));
        }
        self.map.insert(key, schema);
    }

    /// Insert a schema under its own `url` and `name`.
    ///
    /// Returns `false` (and stores nothing) if the schema carries neither.
    pub fn insert(&mut self, schema: FhirSchema) -> bool {
        match (schema.url.clone(), schema.name.clone()) {
            (Some(url), _) => {
                self.insert_named(url, schema);
                true
            }
            (None, Some(name)) => {
                self.insert_named(name, schema);
                true
            }
            (None, None) => false,
        }
    }

    /// Build a registry from `(key, schema)` pairs.
    pub fn from_named(iter: impl IntoIterator<Item = (String, FhirSchema)>) -> Self {
        let mut reg = Self::new();
        for (k, s) in iter {
            reg.insert_named(k, s);
        }
        reg
    }
}

impl SchemaResolver for SchemaRegistry {
    fn resolve(&self, reference: &str) -> Option<Arc<FhirSchema>> {
        self.map.get(reference).cloned()
    }
}

/// Layered resolver: earlier layers win. Used to overlay tenant-uploaded
/// profiles on top of an embedded core pack.
#[derive(Clone, Default)]
pub struct CompositeResolver {
    layers: Vec<Arc<dyn SchemaResolver>>,
}

impl CompositeResolver {
    pub fn new(layers: Vec<Arc<dyn SchemaResolver>>) -> Self {
        Self { layers }
    }

    /// Add a layer with lower precedence than all existing layers.
    pub fn push(&mut self, layer: Arc<dyn SchemaResolver>) {
        self.layers.push(layer);
    }
}

impl SchemaResolver for CompositeResolver {
    fn resolve(&self, reference: &str) -> Option<Arc<FhirSchema>> {
        self.layers.iter().find_map(|l| l.resolve(reference))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> FhirSchema {
        FhirSchema {
            name: Some(name.to_string()),
            url: Some(format!("http://example.org/{name}")),
            ..Default::default()
        }
    }

    #[test]
    fn resolves_by_key_url_and_name() {
        let mut reg = SchemaRegistry::new();
        reg.insert_named("PatKey", named("Patient"));
        let by_key = reg.resolve("PatKey").unwrap();
        let by_url = reg.resolve("http://example.org/Patient").unwrap();
        let by_name = reg.resolve("Patient").unwrap();
        assert!(Arc::ptr_eq(&by_key, &by_url));
        assert!(Arc::ptr_eq(&by_key, &by_name));
        assert!(reg.resolve("nope").is_none());
    }

    #[test]
    fn composite_earlier_layer_wins() {
        let mut base = SchemaRegistry::new();
        base.insert_named(
            "Patient",
            FhirSchema {
                kind: Some("resource".into()),
                ..Default::default()
            },
        );
        let mut overlay = SchemaRegistry::new();
        overlay.insert_named(
            "Patient",
            FhirSchema {
                kind: Some("constraint".into()),
                ..Default::default()
            },
        );

        let composite = CompositeResolver::new(vec![Arc::new(overlay), Arc::new(base)]);
        assert_eq!(
            composite.resolve("Patient").unwrap().kind.as_deref(),
            Some("constraint")
        );
    }
}

#[cfg(test)]
mod extension_catalogue_tests {
    use crate::packs::core_registry;
    use helios_fhir::FhirVersion;

    /// #363: the R4 pack now embeds the core extension definitions with
    /// their applicability contexts, queryable by element context.
    #[test]
    fn r4_pack_offers_profiled_extensions_by_context() {
        let registry = core_registry(FhirVersion::R4);
        let for_patient = registry.extensions_applicable(&["Patient"]);
        assert!(
            for_patient.iter().any(|s| s.url.as_deref()
                == Some("http://hl7.org/fhir/StructureDefinition/patient-birthPlace")),
            "birthPlace applies to Patient: {:?}",
            for_patient
                .iter()
                .filter_map(|s| s.url.clone())
                .take(8)
                .collect::<Vec<_>>()
        );
        let for_name = registry.extensions_applicable(&["HumanName.family"]);
        assert!(
            for_name.iter().any(|s| s.url.as_deref()
                == Some("http://hl7.org/fhir/StructureDefinition/humanname-own-name")),
            "own-name applies to HumanName"
        );
        assert!(
            !for_name.iter().any(|s| s.url.as_deref()
                == Some("http://hl7.org/fhir/StructureDefinition/patient-birthPlace")),
            "contexts scope the offer"
        );
    }

    /// #488: the R4B pack carries its own core extension catalogue, same
    /// contract as R4. (R5/R6 stay empty until the Extensions Pack IG is
    /// sourced — the decision is recorded on the issue.)
    #[cfg(feature = "R4B")]
    #[test]
    fn r4b_pack_offers_profiled_extensions_by_context() {
        let registry = core_registry(FhirVersion::R4B);
        let for_patient = registry.extensions_applicable(&["Patient"]);
        assert!(
            for_patient.iter().any(|s| s.url.as_deref()
                == Some("http://hl7.org/fhir/StructureDefinition/patient-birthPlace")),
            "birthPlace applies to Patient in R4B"
        );
        assert!(
            !registry
                .extensions_applicable(&["HumanName.family"])
                .is_empty(),
            "element-scoped contexts populated"
        );
    }
}
