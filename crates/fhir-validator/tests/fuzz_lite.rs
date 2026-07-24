//! Fuzz-lite robustness sweep: the validator must never panic, whatever
//! JSON it is fed. Deterministic (seeded xorshift, no clock/rand deps) so
//! failures reproduce.

#![cfg(feature = "R4")]

use helios_fhir::FhirVersion;
use helios_fhir_validator::packs::core_registry;
use helios_fhir_validator::{ValidationOptions, Validator};
use serde_json::{Value, json};

/// Tiny deterministic PRNG (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A pool of pathological values to splice into resources.
fn junk(rng: &mut Rng) -> Value {
    match rng.below(12) {
        0 => Value::Null,
        1 => json!(true),
        2 => json!(-1),
        3 => json!(1e308),
        4 => json!(""),
        5 => json!("\u{0}\u{ffff}"),
        6 => json!([]),
        7 => json!({}),
        8 => json!([null, [null, [null]]]),
        9 => json!({"resourceType": null}),
        10 => json!({"resourceType": "Patient", "resourceType2": {"a": [{}]}}),
        _ => json!({"_": {"__": {"___": []}}}),
    }
}

/// Mutate a node in place: replace, delete a key, insert junk, or recurse.
fn mutate(value: &mut Value, rng: &mut Rng, depth: usize) {
    if depth > 6 {
        return;
    }
    match rng.below(5) {
        0 => *value = junk(rng),
        1 => {
            if let Some(obj) = value.as_object_mut() {
                if let Some(key) = obj.keys().nth(rng.below(obj.len().max(1))).cloned() {
                    obj.remove(&key);
                }
            } else {
                *value = junk(rng);
            }
        }
        2 => {
            if let Some(obj) = value.as_object_mut() {
                let j = junk(rng);
                let names = [
                    "x",
                    "_x",
                    "extension",
                    "value",
                    "resourceType",
                    "contained",
                    "9",
                ];
                obj.insert(names[rng.below(names.len())].to_string(), j);
            } else if let Some(arr) = value.as_array_mut() {
                let j = junk(rng);
                arr.push(j);
            }
        }
        _ => {
            let target = match value {
                Value::Object(obj) if !obj.is_empty() => {
                    let idx = rng.below(obj.len());
                    obj.values_mut().nth(idx)
                }
                Value::Array(arr) if !arr.is_empty() => {
                    let idx = rng.below(arr.len());
                    arr.get_mut(idx)
                }
                _ => None,
            };
            match target {
                Some(inner) => mutate(inner, rng, depth + 1),
                None => *value = junk(rng),
            }
        }
    }
}

fn seed_patient() -> Value {
    json!({
        "resourceType": "Patient",
        "id": "fuzz",
        "meta": { "profile": ["http://example.org/nope"] },
        "extension": [{ "url": "http://x", "valueString": "v" }],
        "identifier": [{ "system": "http://example.org", "value": "1" }],
        "active": true,
        "name": [{ "family": "F", "given": ["G", null] }],
        "_birthDate": { "id": "b" },
        "birthDate": "1980-01-01",
        "deceasedBoolean": false,
        "contained": [{ "resourceType": "Organization", "id": "o", "name": "N" }],
        "link": [{ "other": { "reference": "#o" }, "type": "seealso" }]
    })
}

#[test]
fn never_panics_on_mutated_resources() {
    let validator = Validator::new(core_registry(FhirVersion::R4));
    let opts = ValidationOptions::default();
    let mut rng = Rng(0x5EED_CAFE_F00D_D00D);

    for round in 0..2000 {
        let mut resource = seed_patient();
        // Escalate mutation aggressiveness with the round number.
        for _ in 0..(1 + round % 7) {
            mutate(&mut resource, &mut rng, 0);
        }
        // Must return (errors are fine, panics are not).
        let _ = validator.validate_sync(&resource, &opts);
    }
}

#[test]
fn never_panics_on_pure_junk_roots() {
    let validator = Validator::new(core_registry(FhirVersion::R4));
    let opts = ValidationOptions::default();
    let mut rng = Rng(0xBAD_5EED);

    for _ in 0..500 {
        let mut root = junk(&mut rng);
        mutate(&mut root, &mut rng, 0);
        let _ = validator.validate_sync(&root, &opts);
    }
}
