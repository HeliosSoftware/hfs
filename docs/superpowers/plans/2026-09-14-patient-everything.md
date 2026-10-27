# Patient `$everything` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `GET|POST /Patient/{id}/$everything` and `GET|POST /Patient/$everything` on every search-capable backend, composed in the REST layer over the existing per-type `SearchProvider::search`.

**Architecture:** A new `crates/rest/src/handlers/everything/` module walks a fixed sequence of *segments* — the Patient itself, then every Patient-compartment member type — running one compartment-scoped `SearchQuery` per segment and carrying a REST-owned opaque cursor `(seg, inner backend cursor)` across pages. Supporting resources (Practitioner, Organization, …) are collected per page from outbound references and emitted as `include` entries. No persistence changes.

**Tech Stack:** Rust 2024 / Axum / `helios_persistence::core::{ResourceStorage, SearchProvider}` / `helios_fhir::get_compartment_params` / `axum_test::TestServer` / testcontainers (Postgres, Mongo).

**Spec:** `docs/superpowers/specs/2026-09-14-patient-everything-design.md`

## Global Constraints

- Handler generic bound is exactly `S: ResourceStorage + SearchProvider + Send + Sync` — no new persistence traits.
- Compartment membership comes only from `helios_fhir::get_compartment_params(version, "Patient", type)`; never a hand-written type list.
- Operation routes are registered **before** the `/{resource_type}/{id}` CRUD catch-alls in `fhir_routes.rs`.
- Every error is a `RestError` (renders an `OperationOutcome`): unknown param / bad value / unknown `_type` → `BadRequest`; missing patient → `NotFound`; backend without search → the existing `From<BackendError>` 501 mapping.
- New env var: `HFS_EVERYTHING_MAX_UNPAGED`, default `10000`.
- Commit after every task; `cargo fmt` and `cargo clippy -p helios-rest --all-targets -- -D warnings` must be clean before each commit.
- All commits end with the attribution lines from the session's system reminder.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/rest/src/config.rs` | `everything_max_unpaged` config field |
| `crates/rest/src/state.rs` | `AppState::everything_max_unpaged()` accessor |
| `crates/rest/src/handlers/bulk_common.rs` | `pairs_from_parameters` gains `valueDate` / `valueInteger` |
| `crates/rest/src/handlers/everything/mod.rs` | Axum handlers, request decoding (GET query / POST `Parameters`), bundle assembly, audit context |
| `crates/rest/src/handlers/everything/params.rs` | `EverythingParams` — validated inputs |
| `crates/rest/src/handlers/everything/cursor.rs` | `EverythingCursor` — encode/decode/fingerprint |
| `crates/rest/src/handlers/everything/scope.rs` | segment list, clinical-date param map, supporting-reference collector, per-segment `SearchQuery` builder |
| `crates/rest/src/handlers/everything/walk.rs` | the paging walk (instance + type level) |
| `crates/rest/src/handlers/mod.rs` | re-exports |
| `crates/rest/src/routing/fhir_routes.rs` | routes |
| `crates/rest/src/handlers/capabilities.rs` | CapabilityStatement entry |
| `crates/rest/tests/patient_everything.rs` | router tests (SQLite in-memory) |
| `crates/rest/tests/patient_everything_postgres.rs` | Postgres testcontainer |
| `crates/rest/tests/patient_everything_mongodb.rs` | Mongo testcontainer |
| `crates/persistence/tests/elasticsearch_compartment_paging.rs` | ES primitives test |
| docs: `README.md`, `crates/rest/README.md`, `book/src/configuration/environment-variables.md`, `.claude/skills/run-hfs-server/SKILL.md`, `.agents/skills/run-hfs-server/SKILL.md` | env var + operation docs |

---

### Task 1: Config field and state accessor

**Files:**
- Modify: `crates/rest/src/config.rs:1089-1091` (after `max_page_size`), and `for_testing()` at `:1612`
- Modify: `crates/rest/src/state.rs:567-574` (next to `max_page_size()`)

**Interfaces:**
- Produces: `ServerConfig.everything_max_unpaged: usize`, `AppState::everything_max_unpaged(&self) -> usize`.

- [ ] **Step 1: Add the config field**

In `crates/rest/src/config.rs`, directly after the `max_page_size` field:

```rust
    /// Ceiling on `match` entries returned by an unpaged `Patient/$everything`
    /// (no `_count`). When reached, the response switches to paged mode and
    /// carries a `next` link plus an informational `OperationOutcome`.
    #[arg(long, env = "HFS_EVERYTHING_MAX_UNPAGED", default_value = "10000")]
    pub everything_max_unpaged: usize,
```

Then in `for_testing()` (`config.rs:1612`, a struct literal), add `everything_max_unpaged: 10000,` next to `max_page_size`. Run `cargo build -p helios-rest` — any other `ServerConfig { .. }` literal that fails to compile also needs the field (grep `ServerConfig {` across `crates/` and add it wherever the compiler complains; `..ServerConfig::for_testing()` sites need nothing).

- [ ] **Step 2: Add the accessor**

In `crates/rest/src/state.rs`, after `max_page_size()`:

```rust
    /// Ceiling on `match` entries for an unpaged `Patient/$everything`.
    pub fn everything_max_unpaged(&self) -> usize {
        self.config.everything_max_unpaged
    }
```

- [ ] **Step 3: Write a config test**

In the existing `#[cfg(test)] mod tests` of `config.rs`, add:

```rust
    #[test]
    fn everything_max_unpaged_defaults_to_10000() {
        let config = ServerConfig::for_testing();
        assert_eq!(config.everything_max_unpaged, 10000);
    }
```

- [ ] **Step 4: Build and test**

Run: `cargo test -p helios-rest --lib config::tests::everything_max_unpaged_defaults_to_10000`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/rest/src/config.rs crates/rest/src/state.rs
git commit -m "feat(rest): add HFS_EVERYTHING_MAX_UNPAGED config for Patient \$everything"
```

---

### Task 2: `pairs_from_parameters` handles `valueDate` and `valueInteger`

`$everything`'s `start`/`end` are `date` (`valueDate`) and `_count` is `integer` (`valueInteger`); the helper currently reads neither.

**Files:**
- Modify: `crates/rest/src/handlers/bulk_common.rs:75-101`
- Test: same file, `mod tests`

**Interfaces:**
- Produces: `pub(crate) fn pairs_from_parameters(body: &serde_json::Value) -> Vec<(String, String)>` (unchanged signature, wider coverage).

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `bulk_common.rs`:

```rust
    #[test]
    fn pairs_from_parameters_reads_value_date_and_value_integer() {
        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                { "name": "start", "valueDate": "2020-01-01" },
                { "name": "_count", "valueInteger": 25 },
                { "name": "_since", "valueInstant": "2021-02-03T04:05:06Z" }
            ]
        });
        let pairs = pairs_from_parameters(&body);
        assert_eq!(
            pairs,
            vec![
                ("start".to_string(), "2020-01-01".to_string()),
                ("_count".to_string(), "25".to_string()),
                ("_since".to_string(), "2021-02-03T04:05:06Z".to_string()),
            ]
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p helios-rest --lib bulk_common::tests::pairs_from_parameters_reads_value_date_and_value_integer`
Expected: FAIL — `start` and `_count` pairs missing.

- [ ] **Step 3: Implement**

Replace the body of the `for p in arr` loop in `pairs_from_parameters` with:

```rust
            let Some(name) = p.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let value = p
                .get("valueString")
                .or_else(|| p.get("valueUri"))
                .or_else(|| p.get("valueUrl"))
                .or_else(|| p.get("valueInstant"))
                .or_else(|| p.get("valueCode"))
                .or_else(|| p.get("valueDateTime"))
                .or_else(|| p.get("valueDate"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| {
                    p.get("valueReference")
                        .and_then(|r| r.get("reference"))
                        .and_then(|r| r.as_str())
                        .map(str::to_string)
                })
                .or_else(|| {
                    p.get("valueInteger")
                        .and_then(|v| v.as_i64())
                        .map(|v| v.to_string())
                });
            if let Some(v) = value {
                pairs.push((name.to_string(), v));
            }
```

- [ ] **Step 4: Run all bulk_common tests**

Run: `cargo test -p helios-rest --lib bulk_common`
Expected: all PASS (existing tests unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/rest/src/handlers/bulk_common.rs
git commit -m "fix(rest): read valueDate and valueInteger in Parameters bodies"
```

---

### Task 3: `EverythingParams` — validated inputs

**Files:**
- Create: `crates/rest/src/handlers/everything/mod.rs` (module skeleton only in this task)
- Create: `crates/rest/src/handlers/everything/params.rs`
- Modify: `crates/rest/src/handlers/mod.rs` — add `pub mod everything;` (alongside the other `mod` lines) — nothing re-exported yet.

**Interfaces:**
- Produces:
  ```rust
  pub(crate) struct EverythingParams {
      pub start: Option<String>,   // validated FHIR date: YYYY | YYYY-MM | YYYY-MM-DD
      pub end: Option<String>,
      pub since: Option<String>,   // validated RFC3339 instant
      pub types: Option<Vec<String>>, // None = all member types; Some = validated, deduped, in request order
      pub count: Option<usize>,    // None = unpaged; Some = clamped to max_page_size, >= 1
      pub cursor: Option<String>,  // raw token, decoded in Task 4
  }
  impl EverythingParams {
      pub fn from_pairs(pairs: &[(String, String)], version: FhirVersion, max_page_size: usize) -> Result<Self, RestError>;
      pub fn fingerprint_input(&self, patient_id: Option<&str>) -> String; // canonical string for the cursor fingerprint
  }
  ```

- [ ] **Step 1: Create the module skeleton**

`crates/rest/src/handlers/everything/mod.rs`:

```rust
//! FHIR `Patient/$everything` operation.
//!
//! Composed in the REST layer over the per-type [`SearchProvider::search`]
//! and the query-time compartment predicate that `GET /Patient/{id}/*` uses.
//! See `docs/superpowers/specs/2026-09-14-patient-everything-design.md`.

pub(crate) mod params;
```

Add `pub mod everything;` to `crates/rest/src/handlers/mod.rs`.

- [ ] **Step 2: Write the failing tests**

`crates/rest/src/handlers/everything/params.rs` — start with the tests:

```rust
use helios_fhir::FhirVersion;

use crate::error::RestError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EverythingParams {
    pub start: Option<String>,
    pub end: Option<String>,
    pub since: Option<String>,
    pub types: Option<Vec<String>>,
    pub count: Option<usize>,
    pub cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn empty_input_is_all_defaults() {
        let p = EverythingParams::from_pairs(&[], FhirVersion::R4, 1000).unwrap();
        assert_eq!(p, EverythingParams { start: None, end: None, since: None, types: None, count: None, cursor: None });
    }

    #[test]
    fn parses_every_parameter() {
        let p = EverythingParams::from_pairs(
            &pairs(&[("start", "2020"), ("end", "2021-06-30"), ("_since", "2021-02-03T04:05:06Z"),
                     ("_type", "Observation,Encounter"), ("_type", "Condition"), ("_count", "50"), ("_cursor", "abc")]),
            FhirVersion::R4, 1000).unwrap();
        assert_eq!(p.start.as_deref(), Some("2020"));
        assert_eq!(p.end.as_deref(), Some("2021-06-30"));
        assert_eq!(p.since.as_deref(), Some("2021-02-03T04:05:06Z"));
        assert_eq!(p.types, Some(vec!["Observation".into(), "Encounter".into(), "Condition".into()]));
        assert_eq!(p.count, Some(50));
        assert_eq!(p.cursor.as_deref(), Some("abc"));
    }

    #[test]
    fn type_list_is_deduped_preserving_order() {
        let p = EverythingParams::from_pairs(&pairs(&[("_type", "Encounter,Observation,Encounter")]), FhirVersion::R4, 1000).unwrap();
        assert_eq!(p.types, Some(vec!["Encounter".into(), "Observation".into()]));
    }

    #[test]
    fn count_is_clamped_to_max_page_size() {
        let p = EverythingParams::from_pairs(&pairs(&[("_count", "5000")]), FhirVersion::R4, 1000).unwrap();
        assert_eq!(p.count, Some(1000));
    }

    #[test]
    fn rejects_unknown_parameter() {
        let e = EverythingParams::from_pairs(&pairs(&[("_include", "x")]), FhirVersion::R4, 1000).unwrap_err();
        assert!(matches!(e, RestError::BadRequest { .. }), "{e:?}");
    }

    #[test]
    fn rejects_unknown_resource_type() {
        let e = EverythingParams::from_pairs(&pairs(&[("_type", "Observation,Bogus")]), FhirVersion::R4, 1000).unwrap_err();
        assert!(matches!(e, RestError::BadRequest { message } if message.contains("Bogus")));
    }

    #[test]
    fn rejects_bad_dates_and_counts() {
        for (k, v) in [("start", "20-01"), ("end", "2020-13-01x"), ("_since", "2020-01-01"), ("_count", "0"), ("_count", "abc")] {
            let e = EverythingParams::from_pairs(&pairs(&[(k, v)]), FhirVersion::R4, 1000).unwrap_err();
            assert!(matches!(e, RestError::BadRequest { .. }), "{k}={v} should be rejected");
        }
    }

    #[test]
    fn fingerprint_input_is_canonical() {
        let a = EverythingParams::from_pairs(&pairs(&[("_type", "Observation"), ("_count", "5")]), FhirVersion::R4, 1000).unwrap();
        let b = EverythingParams::from_pairs(&pairs(&[("_count", "5"), ("_type", "Observation"), ("_cursor", "zzz")]), FhirVersion::R4, 1000).unwrap();
        assert_eq!(a.fingerprint_input(Some("p1")), b.fingerprint_input(Some("p1")));
        assert_ne!(a.fingerprint_input(Some("p1")), a.fingerprint_input(Some("p2")));
        assert_ne!(a.fingerprint_input(Some("p1")), a.fingerprint_input(None));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p helios-rest --lib handlers::everything::params`
Expected: compile error — `from_pairs` / `fingerprint_input` not defined.

- [ ] **Step 4: Implement**

Insert between the struct and `mod tests`:

```rust
fn is_fhir_date(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    match b.len() {
        4 => digits(0..4),
        7 => digits(0..4) && b[4] == b'-' && digits(5..7),
        10 => digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10),
        _ => false,
    }
}

fn bad(param: &str, value: &str, why: &str) -> RestError {
    RestError::BadRequest {
        message: format!("Invalid value '{value}' for parameter '{param}': {why}"),
    }
}

impl EverythingParams {
    pub fn from_pairs(
        pairs: &[(String, String)],
        version: FhirVersion,
        max_page_size: usize,
    ) -> Result<Self, RestError> {
        let mut out = Self { start: None, end: None, since: None, types: None, count: None, cursor: None };
        let known_types = crate::fhir_types::get_resource_type_names_for_version(version);
        let mut types: Vec<String> = Vec::new();
        let mut saw_type = false;

        for (name, value) in pairs {
            match name.as_str() {
                "start" | "end" => {
                    if !is_fhir_date(value) {
                        return Err(bad(name, value, "expected a FHIR date (YYYY, YYYY-MM or YYYY-MM-DD)"));
                    }
                    let slot = if name == "start" { &mut out.start } else { &mut out.end };
                    *slot = Some(value.clone());
                }
                "_since" => {
                    chrono::DateTime::parse_from_rfc3339(value)
                        .map_err(|_| bad(name, value, "expected an RFC 3339 instant"))?;
                    out.since = Some(value.clone());
                }
                "_type" => {
                    saw_type = true;
                    for t in value.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                        if !known_types.contains(&t) {
                            return Err(bad(name, t, "unknown resource type"));
                        }
                        if !types.iter().any(|x| x == t) {
                            types.push(t.to_string());
                        }
                    }
                }
                "_count" => {
                    let n: usize = value.parse().map_err(|_| bad(name, value, "expected a positive integer"))?;
                    if n == 0 {
                        return Err(bad(name, value, "must be at least 1"));
                    }
                    out.count = Some(n.min(max_page_size));
                }
                "_cursor" => out.cursor = Some(value.clone()),
                "_format" | "_pretty" => {}
                other => {
                    return Err(RestError::BadRequest {
                        message: format!("Unknown parameter '{other}' for $everything"),
                    });
                }
            }
        }
        if saw_type {
            out.types = Some(types);
        }
        Ok(out)
    }

    /// Canonical string of the scope-defining inputs; hashed into the cursor.
    pub fn fingerprint_input(&self, patient_id: Option<&str>) -> String {
        format!(
            "pid={}|start={}|end={}|since={}|types={}|count={}",
            patient_id.unwrap_or(""),
            self.start.as_deref().unwrap_or(""),
            self.end.as_deref().unwrap_or(""),
            self.since.as_deref().unwrap_or(""),
            self.types.as_ref().map(|t| t.join(",")).unwrap_or_default(),
            self.count.map(|c| c.to_string()).unwrap_or_default(),
        )
    }
}
```

`chrono` is already a dependency of `helios-rest` (check `crates/rest/Cargo.toml`; if absent, add `chrono = { workspace = true }` or the version other crates use).

- [ ] **Step 5: Run tests**

Run: `cargo test -p helios-rest --lib handlers::everything::params`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/rest/src/handlers/mod.rs crates/rest/src/handlers/everything/
git commit -m "feat(rest): validated input parameters for Patient \$everything"
```

---

### Task 4: `EverythingCursor` — opaque cross-type paging token

**Files:**
- Create: `crates/rest/src/handlers/everything/cursor.rs`
- Modify: `crates/rest/src/handlers/everything/mod.rs` — add `pub(crate) mod cursor;`
- Modify: `crates/rest/Cargo.toml` — ensure `base64 = "0.22"` is under `[dependencies]` (it is already a dev-dependency).

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub(crate) struct EverythingCursor {
      pub v: u8,                    // always 1
      pub pat: Option<String>,      // type-level: backend cursor for the NEXT Patient page
      pub pid: Option<String>,      // type-level: patient currently being walked
      pub seg: usize,               // index into the segment sequence
      pub inner: Option<String>,    // backend PageCursor within the current segment
      pub fp: String,               // fingerprint(EverythingParams::fingerprint_input)
  }
  impl EverythingCursor {
      pub fn new(seg: usize, inner: Option<String>, fp_input: &str) -> Self;
      pub fn encode(&self) -> String;
      pub fn decode(token: &str, fp_input: &str) -> Result<Self, RestError>; // 400 on garbage, version != 1, or fingerprint mismatch
  }
  pub(crate) fn fingerprint(input: &str) -> String; // FNV-1a 64, hex
  ```

- [ ] **Step 1: Write the failing tests**

`cursor.rs`:

```rust
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use crate::error::RestError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EverythingCursor {
    pub v: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<String>,
    pub seg: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner: Option<String>,
    pub fp: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let c = EverythingCursor { v: 1, pat: Some("p".into()), pid: Some("pt1".into()), seg: 3, inner: Some("in".into()), fp: fingerprint("x") };
        let token = c.encode();
        assert!(!token.contains('='), "url-safe, no padding");
        assert_eq!(EverythingCursor::decode(&token, "x").unwrap(), c);
    }

    #[test]
    fn new_sets_version_and_fingerprint() {
        let c = EverythingCursor::new(2, None, "scope");
        assert_eq!(c.v, 1);
        assert_eq!(c.fp, fingerprint("scope"));
        assert_eq!(c.pat, None);
        assert_eq!(c.pid, None);
    }

    #[test]
    fn rejects_fingerprint_mismatch() {
        let token = EverythingCursor::new(0, None, "a").encode();
        let e = EverythingCursor::decode(&token, "b").unwrap_err();
        assert!(matches!(e, RestError::BadRequest { message } if message.contains("different request")));
    }

    #[test]
    fn rejects_garbage_and_wrong_version() {
        assert!(matches!(EverythingCursor::decode("not base64!", "a"), Err(RestError::BadRequest { .. })));
        let mut c = EverythingCursor::new(0, None, "a");
        c.v = 2;
        assert!(matches!(EverythingCursor::decode(&c.encode(), "a"), Err(RestError::BadRequest { .. })));
    }

    #[test]
    fn fingerprint_is_stable_hex() {
        assert_eq!(fingerprint("abc"), fingerprint("abc"));
        assert_ne!(fingerprint("abc"), fingerprint("abd"));
        assert_eq!(fingerprint("").len(), 16);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helios-rest --lib handlers::everything::cursor`
Expected: compile error — `new` / `encode` / `decode` / `fingerprint` missing.

- [ ] **Step 3: Implement**

Insert before `mod tests`:

```rust
/// FNV-1a 64-bit, hex. Deterministic across processes and Rust versions,
/// unlike `DefaultHasher`.
pub(crate) fn fingerprint(input: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in input.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl EverythingCursor {
    pub fn new(seg: usize, inner: Option<String>, fp_input: &str) -> Self {
        Self { v: 1, pat: None, pid: None, seg, inner, fp: fingerprint(fp_input) }
    }

    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("cursor serializes");
        URL_SAFE_NO_PAD.encode(json)
    }

    pub fn decode(token: &str, fp_input: &str) -> Result<Self, RestError> {
        let invalid = || RestError::BadRequest { message: "Invalid _cursor for $everything".to_string() };
        let bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| invalid())?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.v != 1 {
            return Err(invalid());
        }
        if cursor.fp != fingerprint(fp_input) {
            return Err(RestError::BadRequest {
                message: "_cursor was issued for a different request; repeat the original parameters".to_string(),
            });
        }
        Ok(cursor)
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p helios-rest --lib handlers::everything::cursor`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rest/Cargo.toml crates/rest/src/handlers/everything/
git commit -m "feat(rest): opaque cross-type cursor for Patient \$everything"
```

---

### Task 5: Scope — segments, clinical-date map, supporting references, segment query

**Files:**
- Create: `crates/rest/src/handlers/everything/scope.rs`
- Modify: `crates/rest/src/handlers/everything/mod.rs` — add `pub(crate) mod scope;`

**Interfaces:**
- Consumes: `EverythingParams` (Task 3).
- Produces:
  ```rust
  /// Segment 0 is always "Patient" (tier 1, served by `read`); segments 1.. are compartment member types.
  pub(crate) fn build_segments(version: FhirVersion, types: Option<&[String]>) -> Vec<String>;
  pub(crate) fn clinical_date_param(registry: &SearchParameterRegistry, resource_type: &str) -> Option<String>;
  pub(crate) fn build_segment_query(
      registry: &SearchParameterRegistry, version: FhirVersion, resource_type: &str, patient_id: &str,
      params: &EverythingParams, count: u32, cursor: Option<String>,
  ) -> SearchQuery;
  /// (type, id) pairs referenced by `resources` whose type is NOT a Patient-compartment member. Deduped, request order.
  pub(crate) fn collect_supporting_refs(version: FhirVersion, resources: &[StoredResource]) -> Vec<(String, String)>;
  ```

`SearchParameterRegistry` is `helios_fhir::search::registry::SearchParameterRegistry` (`get_param(resource_type, code) -> Option<Arc<SearchParameterDefinition>>`, definition has `.param_type`).

- [ ] **Step 1: Write the failing tests**

`scope.rs`:

```rust
use std::collections::HashSet;

use helios_fhir::FhirVersion;
use helios_fhir::search::registry::SearchParameterRegistry;
use helios_persistence::types::{
    CompartmentMembership, SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue, StoredResource,
};

use super::params::EverythingParams;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registry() -> SearchParameterRegistry {
        // The rest crate's own tests build the spec registry this way; reuse
        // the helper `crate::extractors::search_query_builder` tests use.
        // If that helper is private, load via
        // `helios_persistence::search::load_spec_registry(FhirVersion::R4, &data_dir())`
        // where data_dir() = workspace `data/` (see tests in compartment_definition_endpoint.rs).
        crate::test_support::spec_registry_r4()
    }

    fn params(list: &[(&str, &str)]) -> EverythingParams {
        let pairs: Vec<(String, String)> = list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        EverythingParams::from_pairs(&pairs, FhirVersion::R4, 1000).unwrap()
    }

    fn stored(rt: &str, id: &str, content: serde_json::Value) -> StoredResource {
        StoredResource::new_for_test(rt, id, content) // see Step 3 note
    }

    #[test]
    fn segments_start_with_patient_then_members_in_table_order() {
        let segs = build_segments(FhirVersion::R4, None);
        assert_eq!(segs[0], "Patient");
        assert!(segs.contains(&"Observation".to_string()));
        assert!(segs.contains(&"Encounter".to_string()));
        assert!(!segs.contains(&"Practitioner".to_string()), "Practitioner is not a Patient-compartment member");
        let dedup: HashSet<_> = segs.iter().collect();
        assert_eq!(dedup.len(), segs.len());
    }

    #[test]
    fn type_filter_restricts_members_but_keeps_patient() {
        let segs = build_segments(FhirVersion::R4, Some(&["Encounter".to_string(), "Observation".to_string()]));
        assert_eq!(segs, vec!["Patient", "Encounter", "Observation"]);
    }

    #[test]
    fn type_filter_ignores_non_members() {
        let segs = build_segments(FhirVersion::R4, Some(&["Practitioner".to_string()]));
        assert_eq!(segs, vec!["Patient"]);
    }

    #[test]
    fn clinical_date_map_prefers_overrides_then_date_param() {
        let reg = registry();
        assert_eq!(clinical_date_param(&reg, "Condition").as_deref(), Some("onset-date"));
        assert_eq!(clinical_date_param(&reg, "MedicationRequest").as_deref(), Some("authoredon"));
        assert_eq!(clinical_date_param(&reg, "Observation").as_deref(), Some("date"));
        assert_eq!(clinical_date_param(&reg, "Encounter").as_deref(), Some("date"));
        assert_eq!(clinical_date_param(&reg, "Coverage"), None);
    }

    #[test]
    fn segment_query_scopes_to_compartment_and_applies_filters() {
        let reg = registry();
        let p = params(&[("_since", "2021-01-01T00:00:00Z"), ("start", "2020"), ("end", "2020-12-31")]);
        let q = build_segment_query(&reg, FhirVersion::R4, "Observation", "p1", &p, 7, Some("cur".into()));
        assert_eq!(q.resource_type, "Observation");
        assert_eq!(q.count, Some(7));
        assert_eq!(q.cursor.as_deref(), Some("cur"));
        assert_eq!(q.offset, None);
        let c = q.compartment.as_ref().unwrap();
        assert_eq!(c.reference, "Patient/p1");
        assert!(c.params.contains(&"subject".to_string()));
        let names: Vec<(&str, SearchPrefix, &str)> = q.parameters.iter()
            .flat_map(|sp| sp.values.iter().map(move |v| (sp.name.as_str(), v.prefix, v.value.as_str()))).collect();
        assert!(names.contains(&("_lastUpdated", SearchPrefix::Ge, "2021-01-01T00:00:00Z")));
        assert!(names.contains(&("date", SearchPrefix::Ge, "2020")));
        assert!(names.contains(&("date", SearchPrefix::Le, "2020-12-31")));
    }

    #[test]
    fn segment_query_skips_clinical_filter_for_types_without_date() {
        let reg = registry();
        let p = params(&[("start", "2020")]);
        let q = build_segment_query(&reg, FhirVersion::R4, "Coverage", "p1", &p, 10, None);
        assert!(q.parameters.is_empty());
    }

    #[test]
    fn supporting_refs_skips_compartment_members_and_dedups() {
        let obs = stored("Observation", "o1", json!({
            "resourceType": "Observation", "id": "o1",
            "subject": { "reference": "Patient/p1" },
            "performer": [{ "reference": "Practitioner/dr1" }, { "reference": "Organization/org1" }],
            "encounter": { "reference": "Encounter/e1" },
            "note": [{ "authorReference": { "reference": "Practitioner/dr1" } }]
        }));
        let enc = stored("Encounter", "e1", json!({
            "resourceType": "Encounter", "id": "e1",
            "serviceProvider": { "reference": "Organization/org1" },
            "location": [{ "location": { "reference": "Location/l1" } }],
            "partOf": { "reference": "http://other.example/fhir/Encounter/abs" }
        }));
        let refs = collect_supporting_refs(FhirVersion::R4, &[obs, enc]);
        assert_eq!(refs, vec![
            ("Practitioner".to_string(), "dr1".to_string()),
            ("Organization".to_string(), "org1".to_string()),
            ("Location".to_string(), "l1".to_string()),
        ]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helios-rest --lib handlers::everything::scope`
Expected: compile errors for the four missing functions (and possibly the two test helpers — resolve in Step 3).

- [ ] **Step 3: Implement**

Test-helper notes before the implementation:
- **Registry in tests:** find how `crates/rest/src/extractors/search_query_builder.rs`'s tests construct a `SearchParameterRegistry` (grep `SearchParameterRegistry::` in that file's `mod tests`). Reuse that exact construction; if it's a private fn, lift it into a `#[cfg(test)] pub(crate) mod test_support` in `crates/rest/src/lib.rs` as `spec_registry_r4()`. Do not invent a new loader.
- **`StoredResource` in tests:** grep `StoredResource::new(` in `crates/persistence/src/types/stored_resource.rs` for the public constructor and use it directly in place of `new_for_test` (adjust the `stored()` helper; a version id of `"1"` and `Utc::now()` are fine).

Implementation, inserted before `mod tests`:

```rust
/// Types whose clinical date search parameter is not named `date`.
const CLINICAL_DATE_OVERRIDES: &[(&str, &str)] = &[
    ("Condition", "onset-date"),
    ("MedicationRequest", "authoredon"),
    ("MedicationStatement", "effective"),
    ("MedicationAdministration", "effective-time"),
    ("MedicationDispense", "whenhandedover"),
    ("Claim", "created"),
    ("ExplanationOfBenefit", "created"),
];

pub(crate) fn build_segments(version: FhirVersion, types: Option<&[String]>) -> Vec<String> {
    let mut segments = vec!["Patient".to_string()];
    let members = crate::fhir_types::get_resource_type_names_for_version(version)
        .iter()
        .filter(|t| **t != "Patient")
        .filter(|t| !helios_fhir::get_compartment_params(version, "Patient", t).is_empty());
    match types {
        None => segments.extend(members.map(|t| t.to_string())),
        Some(wanted) => {
            let members: Vec<&str> = members.copied().collect();
            segments.extend(wanted.iter().filter(|w| members.contains(&w.as_str())).cloned());
        }
    }
    segments
}

pub(crate) fn clinical_date_param(registry: &SearchParameterRegistry, resource_type: &str) -> Option<String> {
    if let Some((_, p)) = CLINICAL_DATE_OVERRIDES.iter().find(|(t, _)| *t == resource_type) {
        return Some((*p).to_string());
    }
    registry
        .get_param(resource_type, "date")
        .filter(|def| def.param_type == SearchParamType::Date)
        .map(|_| "date".to_string())
}

fn date_param(name: &str, prefix: SearchPrefix, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(prefix, value)],
        chain: vec![],
        components: vec![],
    }
}

pub(crate) fn build_segment_query(
    registry: &SearchParameterRegistry,
    version: FhirVersion,
    resource_type: &str,
    patient_id: &str,
    params: &EverythingParams,
    count: u32,
    cursor: Option<String>,
) -> SearchQuery {
    let mut query = SearchQuery::new(resource_type);
    query.compartment = Some(CompartmentMembership {
        params: helios_fhir::get_compartment_params(version, "Patient", resource_type)
            .iter().map(|s| s.to_string()).collect(),
        reference: format!("Patient/{patient_id}"),
    });
    if let Some(since) = &params.since {
        query.parameters.push(date_param("_lastUpdated", SearchPrefix::Ge, since));
    }
    if params.start.is_some() || params.end.is_some() {
        if let Some(date_name) = clinical_date_param(registry, resource_type) {
            if let Some(start) = &params.start {
                query.parameters.push(date_param(&date_name, SearchPrefix::Ge, start));
            }
            if let Some(end) = &params.end {
                query.parameters.push(date_param(&date_name, SearchPrefix::Le, end));
            }
        }
    }
    query.count = Some(count);
    query.cursor = cursor;
    query.offset = None;
    query
}

fn walk_references(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(r)) = map.get("reference") {
                out.push(r.clone());
            }
            for v in map.values() {
                walk_references(v, out);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| walk_references(v, out)),
        _ => {}
    }
}

pub(crate) fn collect_supporting_refs(version: FhirVersion, resources: &[StoredResource]) -> Vec<(String, String)> {
    let known = crate::fhir_types::get_resource_type_names_for_version(version);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for res in resources {
        let mut raw = Vec::new();
        walk_references(res.content(), &mut raw);
        for r in raw {
            // Relative literal references only: "Type/id".
            let Some((rt, id)) = r.split_once('/') else { continue };
            if id.is_empty() || id.contains('/') || !known.contains(&rt) {
                continue;
            }
            if !helios_fhir::get_compartment_params(version, "Patient", rt).is_empty() {
                continue; // a compartment member: it is (or will be) a `match`
            }
            if seen.insert((rt.to_string(), id.to_string())) {
                out.push((rt.to_string(), id.to_string()));
            }
        }
    }
    out
}
```

If `SearchQuery::new(resource_type)` does not exist, construct with `SearchQuery { resource_type: resource_type.to_string(), ..Default::default() }` (check `search_params.rs:623` for a `Default` impl or a constructor and use whichever exists).

- [ ] **Step 4: Run tests**

Run: `cargo test -p helios-rest --lib handlers::everything::scope`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rest/src/handlers/everything/ crates/rest/src/lib.rs
git commit -m "feat(rest): segment plan, clinical-date map and supporting refs for \$everything"
```

---

### Task 6: The walk — instance and type level

**Files:**
- Create: `crates/rest/src/handlers/everything/walk.rs`
- Modify: `crates/rest/src/handlers/everything/mod.rs` — add `pub(crate) mod walk;`

**Interfaces:**
- Consumes: `EverythingParams` (T3), `EverythingCursor` (T4), `build_segments` / `build_segment_query` / `collect_supporting_refs` (T5).
- Produces:
  ```rust
  pub(crate) struct WalkOutput {
      pub matches: Vec<StoredResource>,
      pub included: Vec<StoredResource>,
      pub next: Option<EverythingCursor>,
      pub ceiling_hit: bool,
  }
  pub(crate) struct WalkLimits { pub page: Option<usize>, pub unpaged_ceiling: usize, pub per_query: usize }
  pub(crate) async fn walk_patient<S>(state: &AppState<S>, tenant: &TenantContext, version: FhirVersion,
      patient_id: &str, params: &EverythingParams, resume: Option<EverythingCursor>, limits: WalkLimits)
      -> RestResult<WalkOutput>;
  pub(crate) async fn walk_all_patients<S>(state: &AppState<S>, tenant: &TenantContext, version: FhirVersion,
      params: &EverythingParams, resume: Option<EverythingCursor>, limits: WalkLimits)
      -> RestResult<WalkOutput>;
  ```
  Both bounded `S: ResourceStorage + SearchProvider + Send + Sync`. `limits.page = None` means unpaged (stop at `unpaged_ceiling` and set `ceiling_hit`); `per_query` is the count passed to each backend search when unpaged (`state.max_page_size()`).

This task's logic is verified by the router tests in Task 7 (it needs real storage); no separate unit tests.

- [ ] **Step 1: Implement**

`walk.rs`:

```rust
use helios_fhir::FhirVersion;
use helios_persistence::core::{ResourceStorage, SearchProvider};
use helios_persistence::tenant::TenantContext;
use helios_persistence::types::{SearchQuery, StoredResource};

use super::cursor::EverythingCursor;
use super::params::EverythingParams;
use super::scope::{build_segment_query, build_segments, collect_supporting_refs};
use crate::error::{RestError, RestResult};
use crate::state::AppState;

pub(crate) struct WalkOutput {
    pub matches: Vec<StoredResource>,
    pub included: Vec<StoredResource>,
    pub next: Option<EverythingCursor>,
    pub ceiling_hit: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct WalkLimits {
    pub page: Option<usize>,
    pub unpaged_ceiling: usize,
    pub per_query: usize,
}

impl WalkLimits {
    fn target(&self) -> usize {
        self.page.unwrap_or(self.unpaged_ceiling)
    }
}

/// Where the walk stands inside one patient.
struct Position {
    seg: usize,
    inner: Option<String>,
}

enum Step {
    /// Page (or ceiling) reached; resume here.
    Paused(Position),
    /// Every segment of this patient is exhausted.
    Done,
}

/// Walks one patient's segments from `pos`, appending to `matches` until
/// `matches.len() >= target` or the segments run out.
async fn walk_segments<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    patient_id: &str,
    params: &EverythingParams,
    segments: &[String],
    mut pos: Position,
    target: usize,
    per_query: usize,
    matches: &mut Vec<StoredResource>,
) -> RestResult<Step>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    if pos.seg == 0 {
        let patient = state
            .storage()
            .read(tenant, "Patient", patient_id)
            .await?
            .ok_or_else(|| RestError::NotFound { resource_type: "Patient".to_string(), id: patient_id.to_string() })?;
        matches.push(patient);
        pos = Position { seg: 1, inner: None };
    }

    while pos.seg < segments.len() {
        if matches.len() >= target {
            return Ok(Step::Paused(pos));
        }
        let remaining = (target - matches.len()).min(per_query).max(1) as u32;
        let query: SearchQuery = {
            let reg = state.storage().search_param_registry(tenant);
            let registry = reg.read();
            build_segment_query(&registry, version, &segments[pos.seg], patient_id, params, remaining, pos.inner.take())
        };
        let result = state.storage().search(tenant, &query).await.map_err(RestError::from)?;
        let has_next = result.resources.page_info.has_next;
        let next_cursor = result.resources.page_info.next_cursor.clone();
        matches.extend(result.resources.items);
        if has_next && next_cursor.is_some() {
            pos.inner = next_cursor;
        } else {
            pos = Position { seg: pos.seg + 1, inner: None };
        }
    }
    Ok(Step::Done)
}

async fn resolve_supporting<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    matches: &[StoredResource],
) -> RestResult<Vec<StoredResource>>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let mut included = Vec::new();
    for (rt, id) in collect_supporting_refs(version, matches) {
        if let Some(res) = state.storage().read(tenant, &rt, &id).await? {
            included.push(res);
        }
    }
    Ok(included)
}

pub(crate) async fn walk_patient<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    patient_id: &str,
    params: &EverythingParams,
    resume: Option<EverythingCursor>,
    limits: WalkLimits,
) -> RestResult<WalkOutput>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let segments = build_segments(version, params.types.as_deref());
    let fp_input = params.fingerprint_input(Some(patient_id));
    let pos = match resume {
        Some(c) => Position { seg: c.seg, inner: c.inner },
        None => Position { seg: 0, inner: None },
    };
    let mut matches = Vec::new();
    let step = walk_segments(state, tenant, version, patient_id, params, &segments, pos, limits.target(), limits.per_query, &mut matches).await?;
    let next = match step {
        Step::Paused(p) => Some(EverythingCursor::new(p.seg, p.inner, &fp_input)),
        Step::Done => None,
    };
    let ceiling_hit = limits.page.is_none() && next.is_some();
    let included = resolve_supporting(state, tenant, version, &matches).await?;
    Ok(WalkOutput { matches, included, next, ceiling_hit })
}

pub(crate) async fn walk_all_patients<S>(
    state: &AppState<S>,
    tenant: &TenantContext,
    version: FhirVersion,
    params: &EverythingParams,
    resume: Option<EverythingCursor>,
    limits: WalkLimits,
) -> RestResult<WalkOutput>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let segments = build_segments(version, params.types.as_deref());
    let fp_input = params.fingerprint_input(None);
    let target = limits.target();

    // `pat` is the backend cursor that yields the NEXT patient; `pid`/`pos`
    // describe the patient currently being walked, if any.
    let (mut pat, mut pid, mut pos) = match resume {
        Some(c) => (c.pat, c.pid, Position { seg: c.seg, inner: c.inner }),
        None => (None, None, Position { seg: 0, inner: None }),
    };
    let mut matches = Vec::new();
    let mut exhausted = false;

    loop {
        if pid.is_none() {
            let mut q = SearchQuery::new("Patient");
            q.count = Some(1);
            q.cursor = pat.take();
            let page = state.storage().search(tenant, &q).await.map_err(RestError::from)?;
            let Some(next_patient) = page.resources.items.into_iter().next() else {
                exhausted = true;
                break;
            };
            pat = page.resources.page_info.next_cursor.clone().filter(|_| page.resources.page_info.has_next);
            pid = Some(next_patient.id().to_string());
            pos = Position { seg: 0, inner: None };
        }
        let current = pid.clone().expect("set above");
        match walk_segments(state, tenant, version, &current, params, &segments, pos, target, limits.per_query, &mut matches).await? {
            Step::Paused(p) => {
                pos = p;
                break;
            }
            Step::Done => {
                pid = None;
                pos = Position { seg: 0, inner: None };
                if pat.is_none() {
                    exhausted = true;
                    break;
                }
            }
        }
    }

    let next = if exhausted {
        None
    } else {
        let mut c = EverythingCursor::new(pos.seg, pos.inner, &fp_input);
        c.pat = pat;
        c.pid = pid;
        Some(c)
    };
    let ceiling_hit = limits.page.is_none() && next.is_some();
    let included = resolve_supporting(state, tenant, version, &matches).await?;
    Ok(WalkOutput { matches, included, next, ceiling_hit })
}
```

Notes for the implementer:
- `StoredResource::id()` exists (`stored_resource.rs`); `search_param_registry(tenant)` and `.read()` are used exactly as in `compartment.rs:97-99`.
- `Position` is moved into `walk_segments` and returned in `Step::Paused`; that is deliberate.
- A patient walk that finds *no* patient (deleted between pages) surfaces `NotFound` from `walk_segments` at `seg == 0`; for type-level this only happens if a patient vanished mid-walk, and a 404 is acceptable there.

- [ ] **Step 2: Build**

Run: `cargo build -p helios-rest`
Expected: compiles with no warnings from `walk.rs` (`dead_code` warnings are expected until Task 7 wires the handlers — allow them with `#![allow(dead_code)]` at the top of `walk.rs` **only until Task 7**, then remove it).

- [ ] **Step 3: Commit**

```bash
git add crates/rest/src/handlers/everything/
git commit -m "feat(rest): paging walk for Patient \$everything (instance and type level)"
```

---

### Task 7: Handlers, routes, bundle assembly, router tests

**Files:**
- Modify: `crates/rest/src/handlers/everything/mod.rs` — handlers + assembly
- Modify: `crates/rest/src/handlers/mod.rs` — `pub use everything::{patient_everything_instance_handler, patient_everything_type_handler};`
- Modify: `crates/rest/src/routing/fhir_routes.rs` — two routes next to `/Patient/$export` (`:258-262`)
- Create: `crates/rest/tests/patient_everything.rs`

**Interfaces:**
- Consumes: everything from Tasks 3–6; `pairs_from_parameters` / `parse_query_pairs` from `handlers::bulk_common`; `SearchResult`, `Page`, `PageInfo`, `BundleEntry::outcome_entry`; `crate::responses::bundle::searchset_to_json`; `crate::public_url::rewrite_bundle_full_urls`; `helios_audit::AuditResponseContext`.
- Produces: the two public handlers.

- [ ] **Step 1: Write the failing router tests**

`crates/rest/tests/patient_everything.rs`:

```rust
//! Router-level tests for `Patient/$everything` over SQLite in-memory.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum_test::TestServer;
use helios_persistence::backends::sqlite::{SqliteBackend, SqliteBackendConfig};
use helios_rest::ServerConfig;
use serde_json::{Value, json};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

async fn server_with(max_unpaged: usize) -> TestServer {
    let backend = SqliteBackend::with_config(
        ":memory:",
        SqliteBackendConfig { data_dir: Some(data_dir()), ..Default::default() },
    )
    .expect("create SQLite backend");
    backend.init_schema().expect("init schema");
    let config = ServerConfig {
        base_url: "http://localhost:8080".to_string(),
        default_tenant: "default".to_string(),
        everything_max_unpaged: max_unpaged,
        ..ServerConfig::for_testing()
    };
    let state = helios_rest::AppState::new(Arc::new(backend), config);
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    TestServer::new(app).expect("create test server")
}

async fn put(server: &TestServer, resource: Value) {
    let rt = resource["resourceType"].as_str().unwrap();
    let id = resource["id"].as_str().unwrap();
    let resp = server.put(&format!("/{rt}/{id}")).json(&resource).await;
    assert!(resp.status_code().is_success(), "PUT {rt}/{id}: {}", resp.text());
}

/// Seeds patient `p1` with 3 Observations, 2 Encounters, 1 Condition, a
/// Practitioner and an Organization they reference; and a control patient
/// `p2` with one Observation. Returns nothing; ids are fixed.
async fn seed(server: &TestServer) {
    put(server, json!({"resourceType": "Organization", "id": "org1", "name": "Org"})).await;
    put(server, json!({"resourceType": "Practitioner", "id": "dr1", "name": [{"family": "Who"}]})).await;
    put(server, json!({"resourceType": "Patient", "id": "p1", "managingOrganization": {"reference": "Organization/org1"}})).await;
    put(server, json!({"resourceType": "Patient", "id": "p2"})).await;
    for (i, date) in [(1, "2019-05-01"), (2, "2020-05-01"), (3, "2021-05-01")] {
        put(server, json!({"resourceType": "Observation", "id": format!("o{i}"), "status": "final",
            "code": {"text": "x"}, "subject": {"reference": "Patient/p1"}, "effectiveDateTime": date,
            "performer": [{"reference": "Practitioner/dr1"}]})).await;
    }
    for (i, date) in [(1, "2019-06-01"), (2, "2021-06-01")] {
        put(server, json!({"resourceType": "Encounter", "id": format!("e{i}"), "status": "finished",
            "class": {"code": "AMB"}, "subject": {"reference": "Patient/p1"},
            "period": {"start": date}, "serviceProvider": {"reference": "Organization/org1"}})).await;
    }
    put(server, json!({"resourceType": "Condition", "id": "c1", "subject": {"reference": "Patient/p1"},
        "onsetDateTime": "2020-01-15"})).await;
    put(server, json!({"resourceType": "Observation", "id": "other", "status": "final",
        "code": {"text": "x"}, "subject": {"reference": "Patient/p2"}})).await;
}

fn entries(bundle: &Value, mode: &str) -> Vec<String> {
    bundle["entry"].as_array().unwrap().iter()
        .filter(|e| e["search"]["mode"] == mode)
        .map(|e| format!("{}/{}", e["resource"]["resourceType"].as_str().unwrap(), e["resource"]["id"].as_str().unwrap()))
        .collect()
}

fn next_link(bundle: &Value) -> Option<String> {
    bundle["link"].as_array()?.iter().find(|l| l["relation"] == "next")?["url"].as_str().map(str::to_string)
}

fn path_of(url: &str) -> String {
    url.strip_prefix("http://localhost:8080").unwrap().to_string()
}

async fn walk(server: &TestServer, first: &str) -> (Vec<String>, Vec<Value>) {
    let mut path = first.to_string();
    let mut matches = Vec::new();
    let mut pages = Vec::new();
    loop {
        let resp = server.get(&path).await;
        assert_eq!(resp.status_code(), StatusCode::OK, "{path}: {}", resp.text());
        let b: Value = resp.json();
        matches.extend(entries(&b, "match"));
        let next = next_link(&b);
        pages.push(b);
        match next { Some(n) => path = path_of(&n), None => break }
        assert!(pages.len() < 50, "runaway paging");
    }
    (matches, pages)
}

#[tokio::test]
async fn instance_level_returns_patient_members_and_supporting_resources() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let resp = server.get("/Patient/p1/$everything").await;
    assert_eq!(resp.status_code(), StatusCode::OK, "{}", resp.text());
    let b: Value = resp.json();
    assert_eq!(b["resourceType"], "Bundle");
    assert_eq!(b["type"], "searchset");
    let m = entries(&b, "match");
    assert_eq!(m[0], "Patient/p1", "Patient is first");
    for id in ["Observation/o1", "Observation/o2", "Observation/o3", "Encounter/e1", "Encounter/e2", "Condition/c1"] {
        assert!(m.contains(&id.to_string()), "missing {id} in {m:?}");
    }
    assert!(!m.iter().any(|x| x == "Observation/other" || x == "Patient/p2"));
    let inc = entries(&b, "include");
    assert!(inc.contains(&"Practitioner/dr1".to_string()), "{inc:?}");
    assert!(inc.contains(&"Organization/org1".to_string()), "{inc:?}");
    assert_eq!(b["total"].as_u64(), Some(m.len() as u64), "unpaged total is exact");
    assert!(next_link(&b).is_none());
}

#[tokio::test]
async fn type_filter_restricts_members_but_keeps_patient() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let b: Value = server.get("/Patient/p1/$everything?_type=Encounter").await.json();
    let m = entries(&b, "match");
    assert_eq!(m[0], "Patient/p1");
    assert_eq!(m.len(), 3, "{m:?}");
    assert!(m.iter().skip(1).all(|x| x.starts_with("Encounter/")));
}

#[tokio::test]
async fn since_and_clinical_dates_filter_members_only() {
    let server = server_with(10_000).await;
    seed(&server).await;

    let b: Value = server.get("/Patient/p1/$everything?start=2020-01-01&end=2020-12-31").await.json();
    let m = entries(&b, "match");
    assert!(m.contains(&"Patient/p1".to_string()));
    assert!(m.contains(&"Observation/o2".to_string()));
    assert!(!m.contains(&"Observation/o1".to_string()));
    assert!(!m.contains(&"Observation/o3".to_string()));
    assert!(m.contains(&"Condition/c1".to_string()), "onset-date 2020-01-15 in range");
    assert!(!m.iter().any(|x| x.starts_with("Encounter/")), "both encounters outside range");

    // _since: everything was created "now", so a far-future instant excludes all members.
    let b: Value = server.get("/Patient/p1/$everything?_since=2999-01-01T00:00:00Z").await.json();
    assert_eq!(entries(&b, "match"), vec!["Patient/p1"]);
    let b: Value = server.get("/Patient/p1/$everything?_since=2000-01-01T00:00:00Z").await.json();
    assert!(entries(&b, "match").len() > 1);
}

#[tokio::test]
async fn paging_walks_to_exhaustion_without_gaps_or_duplicates() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let (unpaged, _) = walk(&server, "/Patient/p1/$everything").await;
    let (paged, pages) = walk(&server, "/Patient/p1/$everything?_count=2").await;
    assert!(pages.len() >= 4, "expected several pages, got {}", pages.len());
    for p in &pages {
        assert!(p["total"].is_null(), "paged responses omit total");
        assert!(entries(p, "match").len() <= 2);
    }
    let mut sorted_a = unpaged.clone(); sorted_a.sort();
    let mut sorted_b = paged.clone(); sorted_b.sort();
    assert_eq!(sorted_a, sorted_b);
    assert_eq!(paged.len(), unpaged.len(), "no duplicates: {paged:?}");
}

#[tokio::test]
async fn cursor_from_a_different_request_is_rejected() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let b: Value = server.get("/Patient/p1/$everything?_count=2").await.json();
    let next = path_of(&next_link(&b).unwrap());
    let tampered = next.replace("_count=2", "_count=3");
    let resp = server.get(&tampered).await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.json::<Value>()["resourceType"], "OperationOutcome");
}

#[tokio::test]
async fn unpaged_ceiling_switches_to_paging_with_information_outcome() {
    let server = server_with(4).await;
    seed(&server).await;
    let b: Value = server.get("/Patient/p1/$everything").await.json();
    assert_eq!(entries(&b, "match").len(), 4);
    assert!(next_link(&b).is_some());
    let outcome = b["entry"].as_array().unwrap().iter()
        .find(|e| e["resource"]["resourceType"] == "OperationOutcome")
        .expect("information outcome present");
    assert_eq!(outcome["resource"]["issue"][0]["severity"], "information");
    assert_eq!(outcome["search"]["mode"], "outcome");
    let (all, _) = walk(&server, "/Patient/p1/$everything").await;
    assert_eq!(all.len(), 7);
}

#[tokio::test]
async fn type_level_walks_every_patient_and_resumes_mid_patient() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let (all, _) = walk(&server, "/Patient/$everything?_count=3").await;
    assert!(all.contains(&"Patient/p1".to_string()));
    assert!(all.contains(&"Patient/p2".to_string()));
    assert!(all.contains(&"Observation/other".to_string()));
    assert!(all.contains(&"Observation/o1".to_string()));
    assert_eq!(all.len(), 9, "{all:?}");
    let mut s = all.clone(); s.sort(); s.dedup();
    assert_eq!(s.len(), all.len(), "no duplicates across patients");
}

#[tokio::test]
async fn post_parameters_matches_get() {
    let server = server_with(10_000).await;
    seed(&server).await;
    let get: Value = server.get("/Patient/p1/$everything?_type=Observation&start=2020-01-01").await.json();
    let post: Value = server.post("/Patient/p1/$everything").json(&json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "_type", "valueCode": "Observation"},
            {"name": "start", "valueDate": "2020-01-01"}
        ]
    })).await.json();
    assert_eq!(entries(&get, "match"), entries(&post, "match"));
}

#[tokio::test]
async fn errors() {
    let server = server_with(10_000).await;
    seed(&server).await;
    assert_eq!(server.get("/Patient/nope/$everything").await.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(server.get("/Patient/p1/$everything?_type=Bogus").await.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(server.get("/Patient/p1/$everything?_include=x").await.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(server.get("/Patient/p1/$everything?_since=2020").await.status_code(), StatusCode::BAD_REQUEST);
    let resp = server.post("/Patient/p1/$everything").json(&json!({"resourceType": "Patient"})).await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST, "POST body must be Parameters");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p helios-rest --test patient_everything`
Expected: FAIL — routes return 404/405 (handlers not registered) or compile error on `everything_max_unpaged` if Task 1 was skipped.

- [ ] **Step 3: Implement the handlers**

Replace `crates/rest/src/handlers/everything/mod.rs` with:

```rust
//! FHIR `Patient/$everything` operation.
//!
//! Composed in the REST layer over the per-type [`SearchProvider::search`]
//! and the query-time compartment predicate that `GET /Patient/{id}/*` uses.
//! See `docs/superpowers/specs/2026-09-14-patient-everything-design.md`.

pub(crate) mod cursor;
pub(crate) mod params;
pub(crate) mod scope;
pub(crate) mod walk;

use axum::{
    Json,
    extract::{Path, Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use helios_persistence::core::{ResourceStorage, SearchProvider, SearchResult};
use helios_persistence::types::{BundleEntry, Page, PageInfo};
use tracing::debug;

use crate::error::{RestError, RestResult};
use crate::extractors::{FhirVersionExtractor, TenantExtractor};
use crate::handlers::bulk_common::{pairs_from_parameters, parse_query_pairs};
use crate::state::AppState;
use cursor::EverythingCursor;
use params::EverythingParams;
use walk::{WalkLimits, WalkOutput};

/// `GET|POST /Patient/{id}/$everything`
pub async fn patient_everything_instance_handler<S>(
    State(state): State<AppState<S>>,
    Path(id): Path<String>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    run(state, Some(id), tenant, version, request).await
}

/// `GET|POST /Patient/$everything`
pub async fn patient_everything_type_handler<S>(
    State(state): State<AppState<S>>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    run(state, None, tenant, version, request).await
}

async fn decode_pairs(request: Request) -> RestResult<Vec<(String, String)>> {
    let method = request.method().clone();
    let mut pairs = parse_query_pairs(request.uri().query());
    if method == Method::POST {
        let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
            .await
            .map_err(|e| RestError::BadRequest { message: format!("Unreadable request body: {e}") })?;
        if !bytes.is_empty() {
            let body: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| RestError::BadRequest { message: format!("Invalid JSON body: {e}") })?;
            if body.get("resourceType").and_then(|v| v.as_str()) != Some("Parameters") {
                return Err(RestError::BadRequest {
                    message: "POST $everything requires a Parameters resource body".to_string(),
                });
            }
            pairs.extend(pairs_from_parameters(&body));
        }
    }
    Ok(pairs)
}

async fn run<S>(
    state: AppState<S>,
    patient_id: Option<String>,
    tenant: TenantExtractor,
    version: FhirVersionExtractor,
    request: Request,
) -> RestResult<Response>
where
    S: ResourceStorage + SearchProvider + Send + Sync,
{
    let fhir_version = version.storage_version_or(state.config().default_fhir_version);
    let pairs = decode_pairs(request).await?;
    let params = EverythingParams::from_pairs(&pairs, fhir_version, state.max_page_size())?;
    let fp_input = params.fingerprint_input(patient_id.as_deref());
    let resume = match &params.cursor {
        Some(token) => Some(EverythingCursor::decode(token, &fp_input)?),
        None => None,
    };
    let limits = WalkLimits {
        page: params.count,
        unpaged_ceiling: state.everything_max_unpaged(),
        per_query: state.max_page_size(),
    };
    debug!(patient = ?patient_id, tenant = %tenant.tenant_id(), params = ?params, "Processing $everything");

    let out: WalkOutput = match &patient_id {
        Some(pid) => walk::walk_patient(&state, tenant.context(), fhir_version, pid, &params, resume, limits).await?,
        None => walk::walk_all_patients(&state, tenant.context(), fhir_version, &params, resume, limits).await?,
    };

    let public_base = state.public_base_url_for_request(&tenant);
    let self_link = build_self_link(&public_base, patient_id.as_deref(), &pairs);
    let paged = params.count.is_some() || out.ceiling_hit;
    let total = if paged { None } else { Some(out.matches.len() as u64) };
    let next_token = out.next.as_ref().map(EverythingCursor::encode);
    let page_info = PageInfo {
        next_cursor: next_token.clone(),
        previous_cursor: None,
        total,
        has_next: next_token.is_some(),
        has_previous: false,
    };
    let result = SearchResult {
        resources: Page::new(out.matches, page_info),
        included: out.included,
        total,
        scores: Default::default(),
    };
    let mut bundle = result.into_bundle(&public_base, &self_link);
    if out.ceiling_hit {
        bundle.entry.push(BundleEntry::outcome_entry(serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "information",
                "code": "informational",
                "diagnostics": format!(
                    "Result exceeded the server's unpaged limit of {} entries and was paged; follow the 'next' link for the remainder",
                    state.everything_max_unpaged()
                )
            }]
        })));
    }
    crate::public_url::rewrite_bundle_full_urls(&mut bundle, |resource_type, id| {
        state.public_url_for_request(&tenant, [resource_type, id])
    });

    let mut response = (
        StatusCode::OK,
        Json(crate::responses::bundle::searchset_to_json(bundle, |resource| resource)),
    )
        .into_response();
    response.extensions_mut().insert(helios_audit::AuditResponseContext {
        resource_type: Some("Patient".to_string()),
        resource_id: patient_id.clone(),
        patient_reference: patient_id.as_ref().map(|id| format!("Patient/{id}")),
    });
    Ok(response)
}

fn build_self_link(base_url: &str, patient_id: Option<&str>, pairs: &[(String, String)]) -> String {
    let query: String = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()))
        .collect::<Vec<_>>()
        .join("&");
    let url = crate::public_url::PublicUrl::parse(base_url)
        .expect("request public base was built from validated configuration");
    match patient_id {
        Some(id) => url.with_segments_and_query(["Patient", id, "$everything"], &query),
        None => url.with_segments_and_query(["Patient", "$everything"], &query),
    }
}
```

Checks while wiring:
- `BundleEntry::outcome_entry` (`pagination.rs:522`) — confirm it sets `search.mode = outcome`; if it does not, set `entry.search = Some(SearchInfo { mode: Some("outcome".into()), score: None })` on the pushed entry using whatever the struct in `pagination.rs` is called.
- `into_bundle` builds `next` by replacing `_cursor` on `self_link` (`core/search.rs:271-276`); confirm the emitted URL contains `_cursor=<token>` in the first router test run and adjust nothing else.
- `SearchResult`'s fields are `pub` (`core/search.rs:92-107`). If `scores` is not `HashMap<String, f64>`, use its actual type's `Default`.
- Remove the temporary `#![allow(dead_code)]` from `walk.rs`.

- [ ] **Step 4: Register routes and re-exports**

`crates/rest/src/handlers/mod.rs` — next to `pub use compartment::compartment_search_handler;`:

```rust
pub use everything::{patient_everything_instance_handler, patient_everything_type_handler};
```

`crates/rest/src/routing/fhir_routes.rs` — immediately after the `/Patient/$export` route (`:258-262`):

```rust
        .route(
            "/Patient/$everything",
            get(handlers::patient_everything_type_handler::<S>)
                .post(handlers::patient_everything_type_handler::<S>),
        )
        .route(
            "/Patient/{id}/$everything",
            get(handlers::patient_everything_instance_handler::<S>)
                .post(handlers::patient_everything_instance_handler::<S>),
        )
```

- [ ] **Step 5: Run the router tests**

Run: `cargo test -p helios-rest --test patient_everything`
Expected: all 9 PASS. If `since_and_clinical_dates_filter_members_only` fails on `Condition/c1`, check that the SQLite registry maps `Condition.onset-date` to `onsetDateTime` (it should — it's a spec param); if `Encounter` leaks in, the `period.start` date is being indexed under `date` — that is correct behaviour and the seed dates keep both encounters outside 2020.

- [ ] **Step 6: Run the whole rest crate + clippy**

Run: `cargo test -p helios-rest && cargo clippy -p helios-rest --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/rest/src/handlers/everything/ crates/rest/src/handlers/mod.rs crates/rest/src/routing/fhir_routes.rs crates/rest/tests/patient_everything.rs
git commit -m "feat(rest): Patient \$everything operation (#966)"
```

---

### Task 8: CapabilityStatement entry

**Files:**
- Modify: `crates/rest/src/handlers/capabilities.rs:389-406`
- Test: `crates/rest/tests/patient_everything.rs` (append)

- [ ] **Step 1: Write the failing test**

Append to `patient_everything.rs`:

```rust
#[tokio::test]
async fn capability_statement_declares_everything_on_patient() {
    let server = server_with(10_000).await;
    let b: Value = server.get("/metadata").await.json();
    let patient = b["rest"][0]["resource"].as_array().unwrap().iter()
        .find(|r| r["type"] == "Patient").expect("Patient resource entry");
    let ops = patient["operation"].as_array().expect("operation array");
    assert!(ops.iter().any(|o| o["name"] == "everything"
        && o["definition"] == "http://hl7.org/fhir/OperationDefinition/Patient-everything"), "{ops:?}");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p helios-rest --test patient_everything capability_statement_declares_everything_on_patient`
Expected: FAIL — no `everything` operation.

- [ ] **Step 3: Implement**

In `build_resource_capability`, change the `"Patient"` arm to:

```rust
        "Patient" => {
            entry["operation"] = serde_json::json!([
                {
                    "name": "export",
                    "definition": "http://hl7.org/fhir/uv/bulkdata/OperationDefinition/patient-export"
                },
                {
                    "name": "everything",
                    "definition": "http://hl7.org/fhir/OperationDefinition/Patient-everything"
                }
            ]);
        }
```

If `capabilities.rs` has unit tests asserting the exact Patient `operation` array length, update them to expect two entries.

- [ ] **Step 4: Run tests**

Run: `cargo test -p helios-rest capabilit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/rest/src/handlers/capabilities.rs crates/rest/tests/patient_everything.rs
git commit -m "feat(rest): advertise Patient \$everything in CapabilityStatement"
```

---

### Task 9: PostgreSQL and MongoDB router tests

**Files:**
- Create: `crates/rest/tests/patient_everything_postgres.rs`
- Create: `crates/rest/tests/patient_everything_mongodb.rs`

**Interfaces:**
- Consumes: the seed/walk helpers from Task 7 (copy them — integration test files cannot share modules without a `common` mod; put the copies at the top of each file).

- [ ] **Step 1: Postgres test**

Create `patient_everything_postgres.rs`:
- `#![cfg(feature = "postgres")]`, one `mod`.
- Copy `SharedPg`, `shared_pg()` and the `create_test_server`-style builder **verbatim** from `crates/rest/tests/sof_conformance_postgres.rs` (lines 43–~130; keep the `16-alpine` tag and `github.run_id` label). Give each test a unique tenant id the way that file does, and set `everything_max_unpaged: 10_000` in the `ServerConfig` literal.
- Copy `put`, `seed`, `entries`, `next_link`, `path_of`, `walk` from Task 7's file (pass the tenant header the fixture requires, as `sof_conformance_postgres.rs` does).
- One test:

```rust
    #[tokio::test]
    async fn postgres_everything_paged_walk_matches_unpaged() {
        let Some(server) = create_test_server("everything-pg").await else { return };
        seed(&server).await;
        let (unpaged, _) = walk(&server, "/Patient/p1/$everything").await;
        let (paged, pages) = walk(&server, "/Patient/p1/$everything?_count=2").await;
        assert!(pages.len() >= 4);
        let mut a = unpaged.clone(); a.sort();
        let mut b = paged.clone(); b.sort();
        assert_eq!(a, b);
        assert_eq!(paged.len(), unpaged.len());
        assert_eq!(unpaged[0], "Patient/p1");
        assert_eq!(unpaged.len(), 7, "{unpaged:?}");
    }
```

(If the copied builder returns a `TestServer` directly rather than `Option`, drop the `let Some(..) else`.)

- [ ] **Step 2: Mongo test**

Create `patient_everything_mongodb.rs` the same way, copying `SharedMongo`, `shared_mongo()` and `create_test_server` from `crates/rest/tests/mongodb_include_iterate.rs` (honour `HFS_TEST_MONGODB_URL` as that file does), with the identical test body named `mongodb_everything_paged_walk_matches_unpaged`.

- [ ] **Step 3: Run both (Docker required)**

Run: `cargo test -p helios-rest --features postgres --test patient_everything_postgres`
Run: `cargo test -p helios-rest --features mongodb --test patient_everything_mongodb`
Expected: PASS (or a clean skip when Docker is absent, matching the copied fixture's behaviour).

- [ ] **Step 4: Commit**

```bash
git add crates/rest/tests/patient_everything_postgres.rs crates/rest/tests/patient_everything_mongodb.rs
git commit -m "test(rest): Patient \$everything paged walk on PostgreSQL and MongoDB"
```

---

### Task 10: Elasticsearch primitives test

The handler composes three backend primitives: compartment predicate, `_lastUpdated ge`, and cursor pass-through. ES has no REST-level fixture, so pin them at the persistence level.

**Files:**
- Create: `crates/persistence/tests/elasticsearch_compartment_paging.rs`

- [ ] **Step 1: Locate the ES fixture**

Run: `Glob crates/persistence/tests/*elastic*` and `Grep "GenericImage|elasticsearch" crates/persistence/tests/*.rs -l`. Copy the container/backend builder from the primary ES suite verbatim (same image tag, same startup wait, same `github.run_id` label). If the suite gates with a feature (`#![cfg(feature = "elasticsearch")]`), do the same.

- [ ] **Step 2: Write the test**

```rust
#[tokio::test]
async fn es_compartment_query_pages_with_cursor_and_since() {
    let Some((backend, tenant)) = create_backend().await else { return };
    // 5 Observations for p1, 1 for p2
    for i in 1..=5 {
        backend.create(&tenant, "Observation", &json!({
            "resourceType": "Observation", "status": "final", "code": {"text": "x"},
            "id": format!("o{i}"), "subject": {"reference": "Patient/p1"}
        })).await.unwrap();
    }
    backend.create(&tenant, "Observation", &json!({
        "resourceType": "Observation", "status": "final", "code": {"text": "x"},
        "id": "other", "subject": {"reference": "Patient/p2"}
    })).await.unwrap();
    refresh(&backend).await; // whatever the ES suite calls to make writes searchable

    let mut q = SearchQuery::new("Observation");
    q.compartment = Some(CompartmentMembership {
        params: vec!["subject".into(), "performer".into()],
        reference: "Patient/p1".into(),
    });
    q.count = Some(2);

    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        q.cursor = cursor.take();
        let page = backend.search(&tenant, &q).await.unwrap();
        seen.extend(page.resources.items.iter().map(|r| r.id().to_string()));
        if page.resources.page_info.has_next {
            cursor = page.resources.page_info.next_cursor.clone();
            assert!(cursor.is_some());
        } else {
            break;
        }
    }
    seen.sort();
    assert_eq!(seen, vec!["o1", "o2", "o3", "o4", "o5"]);

    q.cursor = None;
    q.parameters.push(SearchParameter {
        name: "_lastUpdated".into(), param_type: SearchParamType::Date, modifier: None,
        values: vec![SearchValue::new(SearchPrefix::Ge, "2999-01-01T00:00:00Z")], chain: vec![], components: vec![],
    });
    let page = backend.search(&tenant, &q).await.unwrap();
    assert!(page.resources.items.is_empty());
}
```

Use the create/tenant/refresh helpers exactly as the existing ES suite spells them (`create` may take a `serde_json::Value` by value; match it).

- [ ] **Step 3: Run (Docker required)**

Run: `cargo test -p helios-persistence --test elasticsearch_compartment_paging`
Expected: PASS or clean skip.

- [ ] **Step 4: Commit**

```bash
git add crates/persistence/tests/elasticsearch_compartment_paging.rs
git commit -m "test(persistence): pin ES compartment paging primitives used by \$everything"
```

---

### Task 11: Documentation

**Files:**
- Modify: `README.md` — env var table row after `HFS_MAX_PAGE_SIZE`
- Modify: `crates/rest/README.md` — operations list + env var
- Modify: `book/src/configuration/environment-variables.md` — row after `HFS_MAX_PAGE_SIZE`
- Modify: `.claude/skills/run-hfs-server/SKILL.md` and `.agents/skills/run-hfs-server/SKILL.md` — env var + endpoint line (keep both files identical for the changed lines)

- [ ] **Step 1: Env var rows**

In each env-var table, after the `HFS_MAX_PAGE_SIZE` row, add (match the table's column format):

```
| `HFS_EVERYTHING_MAX_UNPAGED` | `10000` | Ceiling on `match` entries for an unpaged `Patient/$everything`; when reached the response is paged and carries a `next` link. |
```

- [ ] **Step 2: Operation docs in `crates/rest/README.md`**

Find the section listing operations (`$export`, `$validate`, `$purge`, …) and add:

```markdown
### `Patient/$everything`

`GET|POST /Patient/{id}/$everything` and `GET|POST /Patient/$everything`.

Returns a `searchset` Bundle with the Patient, every resource in the
patient's compartment (membership from the spec `CompartmentDefinition`, the
same table `GET /Patient/{id}/*` uses), and the supporting resources those
reference (Practitioner, Organization, Location, Medication, …) as
`search.mode = include`.

Parameters: `start`, `end` (clinical dates, applied to each member type's
clinical date search parameter), `_since` (`meta.lastUpdated`), `_type`
(comma-separated, repeatable), `_count`, `_cursor` (server-issued). Without
`_count` the whole result is returned in one bundle up to
`HFS_EVERYTHING_MAX_UNPAGED`, after which it is paged. Paging is forward-only.

Supported on every backend that supports search (SQLite, PostgreSQL,
MongoDB, Elasticsearch, and composites); S3 standalone returns 501.
```

- [ ] **Step 3: Commit**

```bash
git add README.md crates/rest/README.md book/src/configuration/environment-variables.md .claude/skills/run-hfs-server/SKILL.md .agents/skills/run-hfs-server/SKILL.md
git commit -m "docs: Patient \$everything operation and HFS_EVERYTHING_MAX_UNPAGED"
```

---

### Task 12: Final verification

- [ ] **Step 1: Workspace build, fmt, clippy**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 2: Full rest + persistence unit suites**

Run: `cargo test -p helios-rest && cargo test -p helios-persistence --lib`
Expected: PASS.

- [ ] **Step 3: Manual smoke on a running server**

Run `cargo run --bin hfs` (SQLite default), then:

```bash
curl -s -XPUT localhost:8080/Patient/p1 -H 'content-type: application/fhir+json' -d '{"resourceType":"Patient","id":"p1"}'
curl -s -XPUT localhost:8080/Observation/o1 -H 'content-type: application/fhir+json' -d '{"resourceType":"Observation","id":"o1","status":"final","code":{"text":"x"},"subject":{"reference":"Patient/p1"}}'
curl -s 'localhost:8080/Patient/p1/$everything' | jq '.total, [.entry[].resource.resourceType]'
curl -s 'localhost:8080/Patient/p1/$everything?_count=1' | jq '[.link[].relation]'
curl -s localhost:8080/metadata | jq '.rest[0].resource[] | select(.type=="Patient") | .operation'
```

Expected: `2`, `["Patient","Observation"]`; `["self","next"]`; two operations including `everything`.

- [ ] **Step 4: S3 standalone returns 501 (manual, optional if MinIO is not at hand)**

With `HFS_STORAGE_BACKEND=s3` and the MinIO settings from `/run-hfs-server`, `curl -si 'localhost:8080/Patient/p1/$everything' | head -1` → `HTTP/1.1 501`.

- [ ] **Step 5: Push and open the PR**

```bash
git push -u origin feat/966-patient-everything
gh pr create --title "feat(rest): Patient \$everything operation (#966)" --body "..."
```

PR body: link the spec and plan, summarise the A-vs-B decision in two sentences, list the backend matrix, and note the two deferred items (SMART patient-context narrowing; native per-backend path if fan-out proves slow). Close #966 via `Closes #966`.
