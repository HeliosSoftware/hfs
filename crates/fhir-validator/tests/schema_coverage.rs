//! IR keyword-coverage guards.
//!
//! The FHIR Schema IR is deserialized tolerantly — unknown keys are ignored
//! (see `schema.rs`) — and the converter is free to leave any IR field unset.
//! Both are correct at runtime and both hide the same failure mode: a keyword
//! the format defines that we silently drop, or an IR field nothing ever
//! fills. #364/#429 were the latter (`mustSupport`/`isModifier`/`isSummary`
//! existed on the struct and stayed empty in every pack, and no test noticed).
//!
//! Two directions, both mechanical:
//!
//! 1. [`no_unknown_keywords_in_packs_or_fixtures`] — every key observed in the
//!    committed packs and the conformance fixtures must map to an IR field.
//! 2. [`every_ir_field_is_emitted_or_documented_absent`] — every IR field must
//!    appear at least once across the packs, or be listed in `EXPECTED_ABSENT`
//!    with the reason. The assertion is set *equality*, so both a regression
//!    (field stops being emitted) and a fix (field starts being emitted) trip
//!    it and force the list to be updated.
//!
//! The field names are not hand-written: each context serializes a struct
//! literal that names every field, so adding a field to `schema.rs` without
//! accounting for it here is a compile error in this file.

use helios_fhir_validator::{Binding, Constraint, FhirSchema, Match, Slice, Slicing};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------
// The IR field sets, derived from the types themselves.
// ---------------------------------------------------------------------

/// Where in the IR a JSON object sits. Determines which field set applies.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Ctx {
    Schema,
    Binding,
    Constraint,
    Slicing,
    Slice,
    Match,
}

impl Ctx {
    fn name(self) -> &'static str {
        match self {
            Ctx::Schema => "FhirSchema",
            Ctx::Binding => "Binding",
            Ctx::Constraint => "Constraint",
            Ctx::Slicing => "Slicing",
            Ctx::Slice => "Slice",
            Ctx::Match => "Match",
        }
    }

    const ALL: [Ctx; 6] = [
        Ctx::Schema,
        Ctx::Binding,
        Ctx::Constraint,
        Ctx::Slicing,
        Ctx::Slice,
        Ctx::Match,
    ];
}

/// The serialized key names of a fully-populated value.
fn keys_of<T: serde::Serialize>(value: &T) -> BTreeSet<String> {
    serde_json::to_value(value)
        .expect("IR type serializes")
        .as_object()
        .expect("IR type serializes to a JSON object")
        .keys()
        .cloned()
        .collect()
}

/// Every field the IR declares, per context.
///
/// The struct literals below are deliberately exhaustive — no
/// `..Default::default()` — so a new field in `schema.rs` fails to compile
/// here until it is added.
fn declared_fields(ctx: Ctx) -> BTreeSet<String> {
    match ctx {
        Ctx::Schema => keys_of(&FhirSchema {
            url: Some(String::new()),
            name: Some(String::new()),
            base: Some(String::new()),
            kind: Some(String::new()),
            derivation: Some(String::new()),
            type_: Some(String::new()),
            array: Some(true),
            scalar: Some(true),
            min: Some(0),
            max: Some(0),
            elements: Some(Default::default()),
            required: Some(Vec::new()),
            excluded: Some(Vec::new()),
            element_reference: Some(Vec::new()),
            choices: Some(Vec::new()),
            choice_of: Some(String::new()),
            fixed: Some(Value::Null),
            pattern: Some(Value::Null),
            binding: Some(Binding {
                value_set: String::new(),
                strength: None,
            }),
            constraints: Some(Default::default()),
            refers: Some(Vec::new()),
            slicing: Some(Slicing {
                slices: Default::default(),
                rules: None,
                ordered: None,
            }),
            extensions: Some(Default::default()),
            modifier: Some(true),
            must_support: Some(true),
            summary: Some(true),
            short: Some(String::new()),
            regex: Some(String::new()),
            context: Some(Vec::new()),
        }),
        Ctx::Binding => keys_of(&Binding {
            value_set: String::new(),
            strength: Some(String::new()),
        }),
        Ctx::Constraint => keys_of(&Constraint {
            expression: String::new(),
            severity: Some(String::new()),
            human: Some(String::new()),
        }),
        Ctx::Slicing => keys_of(&Slicing {
            slices: Default::default(),
            rules: Some(String::new()),
            ordered: Some(true),
        }),
        Ctx::Slice => keys_of(&Slice {
            match_: Some(Match {
                type_: None,
                value: None,
                resolve_ref: None,
            }),
            min: Some(0),
            max: Some(0),
            order: Some(0),
            reslice: Some(String::new()),
            slice_is_constraining: Some(true),
            schema: Some(Arc::new(FhirSchema::default())),
        }),
        Ctx::Match => keys_of(&Match {
            type_: Some(String::new()),
            value: Some(Value::Null),
            resolve_ref: Some(true),
        }),
    }
}

/// IR fields the converter never emits into the core packs, with the reason.
///
/// This is a statement about *our converter*, not about the format: every key
/// here is a valid FHIR Schema keyword we can parse but do not produce.
fn expected_absent(ctx: Ctx) -> BTreeSet<String> {
    let names: &[&str] = match ctx {
        // Upstream's generated schemas mark every singular element
        // `scalar: true`. Our converter emits only `array`, because the engine
        // treats "not an array" as "must be singular" (`engine/walk.rs`) —
        // stricter than the format's tri-state, so nothing is lost on packs we
        // generate ourselves.
        Ctx::Schema => &["scalar"],
        // Unexercised rather than unimplemented: the core spec bundles contain
        // no re-slicing, so an IG corpus would be needed to cover these.
        // (`order` used to live here — the converter now emits it under
        // `ordered: true`.)
        Ctx::Slice => &["reslice", "sliceIsConstraining"],
        // Only reachable via a `resolve()`-style discriminator, which
        // `build_match` does not translate.
        Ctx::Match => &["resolve-ref"],
        _ => &[],
    };
    names.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------
// Corpus walk.
// ---------------------------------------------------------------------

/// Observed keys per context, with one example location for the failure text.
#[derive(Default)]
struct Observed {
    keys: BTreeMap<Ctx, BTreeMap<String, String>>,
}

impl Observed {
    fn record(&mut self, ctx: Ctx, key: &str, at: &str) {
        self.keys
            .entry(ctx)
            .or_default()
            .entry(key.to_string())
            .or_insert_with(|| at.to_string());
    }

    /// Walk a FHIR-Schema-shaped object, recursing only through *structural*
    /// keys — `fixed` / `pattern` / `match.value` payloads are arbitrary FHIR
    /// data and must not be read as schemas.
    fn walk(&mut self, ctx: Ctx, value: &Value, at: &str) {
        let Some(object) = value.as_object() else {
            return;
        };
        for (key, child) in object {
            self.record(ctx, key, at);
            let here = format!("{at}.{key}");
            match (ctx, key.as_str()) {
                (Ctx::Schema, "elements" | "extensions") => {
                    for (name, schema) in child.as_object().into_iter().flatten() {
                        self.walk(Ctx::Schema, schema, &format!("{here}[{name}]"));
                    }
                }
                (Ctx::Schema, "binding") => self.walk(Ctx::Binding, child, &here),
                (Ctx::Schema, "constraints") => {
                    for (key, constraint) in child.as_object().into_iter().flatten() {
                        self.walk(Ctx::Constraint, constraint, &format!("{here}[{key}]"));
                    }
                }
                (Ctx::Schema, "slicing") => self.walk(Ctx::Slicing, child, &here),
                (Ctx::Slicing, "slices") => {
                    for (name, slice) in child.as_object().into_iter().flatten() {
                        self.walk(Ctx::Slice, slice, &format!("{here}[{name}]"));
                    }
                }
                (Ctx::Slice, "match") => self.walk(Ctx::Match, child, &here),
                (Ctx::Slice, "schema") => self.walk(Ctx::Schema, child, &here),
                _ => {}
            }
        }
    }
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Walk every committed pack. Returns the number of packs read.
fn walk_packs(observed: &mut Observed) -> usize {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let mut count = 0;
    for version in ["r4", "r4b", "r5", "r6"] {
        let path = crate_dir().join(format!("packs/fhir_schemas_{version}.json.gz"));
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut json = Vec::new();
        GzDecoder::new(&bytes[..])
            .read_to_end(&mut json)
            .unwrap_or_else(|e| panic!("{}: decompresses: {e}", path.display()));
        let schemas: Vec<Value> = serde_json::from_slice(&json)
            .unwrap_or_else(|e| panic!("{}: parses as a schema array: {e}", path.display()));
        for schema in &schemas {
            let name = schema
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("<unnamed>");
            observed.walk(Ctx::Schema, schema, &format!("{version}:{name}"));
        }
        count += 1;
    }
    count
}

/// Walk the inline schemas of the conformance fixtures (upstream + extended).
fn walk_fixtures(observed: &mut Observed) -> usize {
    let mut count = 0;
    for dir in ["tests/fixtures/upstream", "tests/fixtures/extended"] {
        let Ok(entries) = std::fs::read_dir(crate_dir().join(dir)) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        paths.sort();
        for path in paths {
            let doc: Value = serde_json::from_slice(&std::fs::read(&path).expect("fixture reads"))
                .unwrap_or_else(|e| panic!("{}: parses: {e}", path.display()));
            let file = path.file_name().unwrap_or_default().to_string_lossy();
            // Fixtures carry a top-level `schemas` map, and may repeat it per
            // test case.
            let inline = std::iter::once(doc.get("schemas")).chain(
                doc.get("tests")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|t| t.get("schemas")),
            );
            for schemas in inline.flatten() {
                for (name, schema) in schemas.as_object().into_iter().flatten() {
                    observed.walk(Ctx::Schema, schema, &format!("{file}:{name}"));
                    count += 1;
                }
            }
        }
    }
    count
}

// ---------------------------------------------------------------------
// Guards.
// ---------------------------------------------------------------------

/// Direction 1: nothing in the corpus uses a keyword the IR cannot hold.
///
/// Deserialization is tolerant by design, so an unmodelled keyword is dropped
/// in silence at runtime. This is where it becomes loud.
#[test]
fn no_unknown_keywords_in_packs_or_fixtures() {
    let mut observed = Observed::default();
    let packs = walk_packs(&mut observed);
    let fixtures = walk_fixtures(&mut observed);
    assert!(packs > 0, "no committed packs found under packs/");
    assert!(
        fixtures > 0,
        "no fixture schemas found under tests/fixtures/"
    );

    let mut unknown: Vec<String> = Vec::new();
    for ctx in Ctx::ALL {
        let declared = declared_fields(ctx);
        for (key, at) in observed.keys.get(&ctx).into_iter().flatten() {
            if !declared.contains(key) {
                unknown.push(format!("  {}.{key}  (first seen at {at})", ctx.name()));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "FHIR Schema keywords observed in the corpus that the IR does not model \
         (they are silently dropped on deserialization — add them to schema.rs \
         or document why they are ignored):\n{}",
        unknown.join("\n")
    );
}

/// Direction 2: every IR field is actually produced by the converter.
///
/// This is the #364/#429 guard — a field that exists on the struct but that no
/// pack ever carries. `expected_absent` is the escape hatch, and it is checked
/// for equality so it cannot go stale in either direction.
#[test]
fn every_ir_field_is_emitted_or_documented_absent() {
    let mut observed = Observed::default();
    assert!(
        walk_packs(&mut observed) > 0,
        "no committed packs found under packs/"
    );

    let mut problems: Vec<String> = Vec::new();
    for ctx in Ctx::ALL {
        let seen: BTreeSet<String> = observed
            .keys
            .get(&ctx)
            .into_iter()
            .flatten()
            .map(|(k, _)| k.clone())
            .collect();
        // A context with no instances at all in the packs says nothing about
        // its fields; only `Slicing`/`Slice`/`Match` could be empty, and they
        // are not.
        if seen.is_empty() {
            problems.push(format!(
                "{}: no instance of this context appears in any pack",
                ctx.name()
            ));
            continue;
        }
        let absent: BTreeSet<String> = declared_fields(ctx).difference(&seen).cloned().collect();
        let expected = expected_absent(ctx);
        for field in absent.difference(&expected) {
            problems.push(format!(
                "{}.{field}: declared in the IR but never emitted into any pack — \
                 wire it up in the converter, or add it to `expected_absent` with the reason",
                ctx.name()
            ));
        }
        for field in expected.difference(&absent) {
            problems.push(format!(
                "{}.{field}: listed in `expected_absent` but the converter now emits it — \
                 remove it from the list",
                ctx.name()
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "IR emission gaps:\n  {}",
        problems.join("\n  ")
    );
}
