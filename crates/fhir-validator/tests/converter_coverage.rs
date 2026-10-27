//! Converter input-coverage guard.
//!
//! Sweeps every `ElementDefinition` in the vendored FHIR spec bundles and
//! asserts that each key is either read by the SD→schema converter or listed
//! here as deliberately ignored. The same sweep runs over the sub-objects of
//! the fields the converter does read (`base`, `type`, `slicing`, `binding`,
//! `constraint`).
//!
//! This is the other half of the #364/#429 guard: `schema_coverage.rs` catches
//! an IR field nothing fills, this catches an ElementDefinition field nothing
//! reads — including fields a future spec version introduces, which otherwise
//! arrive in total silence (the converter's `Ed` model absorbs everything it
//! does not name into `#[serde(flatten)] rest`).
//!
//! The corpus is the same one `generate-schema-packs` converts, so a key that
//! appears here is a key that reaches the converter for real. It is *core*
//! only, though — no IG is vendored, so profile-driven constructs
//! (`sliceIsConstraining`, re-slicing) are not exercised. See the note on
//! `expected_absent` in `schema_coverage.rs`.
//!
//! `CONSUMED` / `IGNORED` are hand-maintained: the converter's `Ed` model is
//! crate-private, so this test cannot derive them. That is the point — moving
//! a key between the two lists is the deliberate act of deciding what the
//! converter does with it.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

// ---------------------------------------------------------------------
// What the converter reads, and what it deliberately does not.
// ---------------------------------------------------------------------

/// `ElementDefinition` fields read by `converter/mod.rs` (`Ed`) and applied in
/// `converter/tree.rs`.
const CONSUMED: &[&str] = &[
    "id",
    "path",
    "sliceName",
    "min",
    "max",
    "base",
    "type",
    "contentReference",
    "slicing",
    "binding",
    "constraint",
    // Informational mirrors carried into the IR (#429).
    "mustSupport",
    "isModifier",
    "isSummary",
    "short",
];

/// `fixed[x]` / `pattern[x]` are matched by prefix in `apply_value_keywords`.
const CONSUMED_PREFIXES: &[&str] = &["fixed", "pattern"];

/// Fields present in the corpus that the converter deliberately does not read.
const IGNORED: &[(&str, &str)] = &[
    (
        "definition",
        "long-form documentation; not carried (pack size — `short` is the label)",
    ),
    ("comment", "documentation"),
    ("requirements", "documentation"),
    ("alias", "documentation"),
    ("mapping", "cross-standard mappings; no FHIR Schema keyword"),
    (
        "condition",
        "back-links to constraint keys; the constraints themselves are carried",
    ),
    ("isModifierReason", "documentation for `isModifier`"),
    (
        "representation",
        "XML/JSON serialization hints; not a validation rule",
    ),
    ("meaningWhenMissing", "documentation"),
    ("orderMeaning", "documentation"),
    ("example", "illustrative values; not a validation rule"),
    (
        "extension",
        "ED-level extensions; the two we need are read off `type[].extension`",
    ),
    (
        "maxLength",
        "validation-bearing, but FHIR Schema defines no keyword for it",
    ),
];

/// `minValue[x]` / `maxValue[x]`, like `maxLength`, are real constraints with
/// no FHIR Schema keyword to carry them.
const IGNORED_PREFIXES: &[&str] = &["minValue", "maxValue"];

/// Sub-object keys, per parent field: `(parent, key, consumed)`.
const SUB_KEYS: &[(&str, &str, bool)] = &[
    ("base", "max", true),
    ("base", "path", false),
    ("base", "min", false),
    ("type", "code", true),
    ("type", "profile", true),
    ("type", "targetProfile", true),
    ("type", "extension", true),
    ("slicing", "discriminator", true),
    ("slicing", "rules", true),
    ("slicing", "ordered", true),
    ("slicing", "description", false),
    ("binding", "strength", true),
    ("binding", "valueSet", true),
    ("binding", "description", false),
    ("binding", "extension", false),
    ("binding", "additional", false),
    ("constraint", "key", true),
    ("constraint", "severity", true),
    ("constraint", "human", true),
    ("constraint", "expression", true),
    ("constraint", "source", false),
    ("constraint", "xpath", false),
    ("constraint", "extension", false),
    ("constraint", "requirements", false),
];

/// Parents whose sub-objects are swept.
const SUB_PARENTS: &[&str] = &["base", "type", "slicing", "binding", "constraint"];

fn is_known(key: &str) -> bool {
    // `_field` carries the primitive sidecar of `field`; it follows its base.
    let base = key.strip_prefix('_').unwrap_or(key);
    CONSUMED.contains(&base)
        || IGNORED.iter().any(|(k, _)| *k == base)
        || CONSUMED_PREFIXES
            .iter()
            .chain(IGNORED_PREFIXES)
            .any(|p| base.len() > p.len() && base.starts_with(p))
}

// ---------------------------------------------------------------------
// Corpus sweep.
// ---------------------------------------------------------------------

/// A narrow view of a spec bundle: only the element lists, so the bulk of each
/// file (narrative, ValueSets, CodeSystems) is never materialized.
#[derive(Deserialize)]
struct Bundle {
    #[serde(default)]
    entry: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    resource: Option<Resource>,
}

#[derive(Deserialize)]
struct Resource {
    #[serde(rename = "resourceType")]
    resource_type: Option<String>,
    snapshot: Option<ElementList>,
    differential: Option<ElementList>,
}

#[derive(Deserialize)]
struct ElementList {
    #[serde(default)]
    element: Vec<Map<String, Value>>,
}

const VERSIONS: [&str; 4] = ["R4", "R4B", "R5", "R6"];
const BUNDLES: [&str; 3] = [
    "profiles-types.json",
    "profiles-resources.json",
    "profiles-others.json",
];

fn resources_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fhir-gen/resources")
}

/// Observed key → the version it was first seen in.
#[derive(Default)]
struct Sweep {
    ed_keys: BTreeMap<String, String>,
    sub_keys: BTreeMap<(String, String), String>,
    elements: usize,
}

fn sweep() -> Sweep {
    let mut out = Sweep::default();
    for version in VERSIONS {
        for bundle_name in BUNDLES {
            let path = resources_dir().join(version).join(bundle_name);
            let Ok(bytes) = std::fs::read(&path) else {
                continue; // e.g. R6 before fhir-gen's download has run
            };
            let bundle: Bundle = serde_json::from_slice(&bytes)
                .unwrap_or_else(|e| panic!("{}: parses as a Bundle: {e}", path.display()));
            for entry in &bundle.entry {
                let Some(resource) = &entry.resource else {
                    continue;
                };
                if resource.resource_type.as_deref() != Some("StructureDefinition") {
                    continue;
                }
                for list in [&resource.snapshot, &resource.differential]
                    .into_iter()
                    .flatten()
                {
                    for element in &list.element {
                        out.elements += 1;
                        for (key, value) in element {
                            out.ed_keys
                                .entry(key.clone())
                                .or_insert_with(|| version.to_string());
                            if !SUB_PARENTS.contains(&key.as_str()) {
                                continue;
                            }
                            let items: &[Value] = match value {
                                Value::Array(items) => items,
                                other => std::slice::from_ref(other),
                            };
                            for item in items {
                                for sub in item.as_object().into_iter().flatten().map(|(k, _)| k) {
                                    out.sub_keys
                                        .entry((key.clone(), sub.clone()))
                                        .or_insert_with(|| version.to_string());
                                }
                            }
                        }
                    }
                }
            }
            // Free the bundle before the next (they are tens of MB each).
            drop(bundle);
        }
    }
    out
}

// ---------------------------------------------------------------------
// Guards.
// ---------------------------------------------------------------------

/// Every ElementDefinition field in the spec corpus is accounted for.
#[test]
fn every_element_definition_field_is_consumed_or_documented() {
    let sweep = sweep();
    if sweep.elements == 0 {
        eprintln!(
            "skipping: no spec bundles under {} — run fhir-gen's resource download first",
            resources_dir().display()
        );
        return;
    }

    let unaccounted: Vec<String> = sweep
        .ed_keys
        .iter()
        .filter(|(key, _)| !is_known(key))
        .map(|(key, version)| format!("  ElementDefinition.{key}  (first seen in {version})"))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "ElementDefinition fields reaching the converter that it neither reads nor \
         documents as ignored — decide which, and add them to CONSUMED or IGNORED:\n{}",
        unaccounted.join("\n")
    );
}

/// Sub-objects of the fields the converter *does* read are accounted for too —
/// this is where a new spec version quietly adds something like
/// `binding.additional`.
#[test]
fn every_sub_object_field_is_consumed_or_documented() {
    let sweep = sweep();
    if sweep.elements == 0 {
        eprintln!("skipping: no spec bundles found (see the sibling test)");
        return;
    }

    let known: BTreeSet<(&str, &str)> = SUB_KEYS.iter().map(|(p, k, _)| (*p, *k)).collect();
    let unaccounted: Vec<String> = sweep
        .sub_keys
        .iter()
        .filter(|((parent, key), _)| !known.contains(&(parent.as_str(), key.as_str())))
        .map(|((parent, key), version)| {
            format!("  ElementDefinition.{parent}.{key}  (first seen in {version})")
        })
        .collect();
    assert!(
        unaccounted.is_empty(),
        "sub-object fields the converter neither reads nor documents as ignored — \
         add them to SUB_KEYS:\n{}",
        unaccounted.join("\n")
    );
}

/// The allowlists describe the corpus, not wishful thinking: an entry that no
/// longer appears anywhere is stale and should be deleted.
#[test]
fn allowlists_have_no_stale_entries() {
    let sweep = sweep();
    if sweep.elements == 0 {
        eprintln!("skipping: no spec bundles found (see the sibling test)");
        return;
    }
    // R6 is downloaded on demand; without it, absence proves nothing.
    if !resources_dir().join("R6").join(BUNDLES[1]).exists() {
        eprintln!("skipping: R6 bundles absent, cannot distinguish stale from R6-only");
        return;
    }

    let mut stale: Vec<String> = IGNORED
        .iter()
        .filter(|(key, _)| !sweep.ed_keys.contains_key(*key))
        .map(|(key, _)| format!("  IGNORED: ElementDefinition.{key}"))
        .collect();
    stale.extend(
        SUB_KEYS
            .iter()
            .filter(|(parent, key, _)| {
                !sweep
                    .sub_keys
                    .contains_key(&(parent.to_string(), key.to_string()))
            })
            .map(|(parent, key, _)| format!("  SUB_KEYS: ElementDefinition.{parent}.{key}")),
    );
    assert!(
        stale.is_empty(),
        "allowlist entries that no longer appear in the spec corpus — delete them:\n{}",
        stale.join("\n")
    );
}
