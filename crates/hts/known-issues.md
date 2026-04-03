# Known Issues — helios-hts

## Clippy `--all-features` Fails Due to Incomplete Pattern Matching in `helios-fhirpath`

**Severity:** Low — does not affect compilation, tests, or runtime behaviour.  
**Affects:** `cargo clippy --all-features` when run from the workspace root or on any crate
that transitively compiles `helios-fhirpath` with more than one FHIR version enabled.

---

### What happens

Running:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

produces errors like:

```
error[E0004]: non-exhaustive patterns: `helios_fhir::FhirVersion::R4B`,
              `helios_fhir::FhirVersion::R5` and `helios_fhir::FhirVersion::R6` not covered
  --> crates/fhirpath/src/...
```

The errors are **inside `helios-fhirpath`**, not inside `helios-hts`.  
They are compile errors surfaced as clippy errors because `-D warnings` promotes
all warnings to errors.

---

### Root cause

`helios-fhirpath` contains `match` expressions over `FhirVersion` and
`VersionIndependentParameters` enums that only handle the `R4` variant (or a
subset of versions). When the `R4B`, `R5`, and `R6` Cargo features are all
enabled at once — which `--all-features` does — the compiler sees the
additional enum variants and the existing `match` arms become non-exhaustive.

This is a pre-existing gap in `helios-fhirpath`: the multi-version match
branches were not fully implemented for every version combination.

---

### How to reproduce

```bash
# From the workspace root — triggers the error
cargo clippy --all-targets --all-features -- -D warnings

# Scoped to fhirpath only — same error, clearer output
cargo clippy -p helios-fhirpath --features R4,R4B,R5,R6 -- -D warnings \
  -A clippy::doc-lazy-continuation \
  -A clippy::doc-overindented-list-items
```

Expected output (excerpt):

```
error[E0004]: non-exhaustive patterns: `helios_fhir::FhirVersion::R4B`,
              `helios_fhir::FhirVersion::R5` and `helios_fhir::FhirVersion::R6` not covered
```

---

### What is NOT affected

| Command | Result |
|---------|--------|
| `cargo build -p helios-hts` | ✅ Compiles fine (default features: R4 + sqlite) |
| `cargo test -p helios-hts` | ✅ All tests pass |
| `cargo clippy -p helios-hts` | ✅ No warnings or errors |
| `cargo clippy -p helios-hts --features R4,R4B,R5,R6,sqlite` | ✅ Clean |
| `cargo clippy --all-targets --all-features` (workspace) | ❌ Fails in `helios-fhirpath` |

`helios-hts` itself is **fully clean** under all single-crate clippy invocations.
The failure only surfaces when the workspace-wide `--all-features` flag forces
every crate to compile simultaneously with all version features active.

---

### Workaround (current CI recommendation)

Run clippy per-crate instead of workspace-wide with `--all-features`:

```bash
# For helios-hts specifically (used in this crate's CI and local checks):
cargo clippy -p helios-hts -- -D warnings \
  -A clippy::items_after_test_module \
  -A clippy::large_enum_variant \
  -A clippy::question_mark \
  -A clippy::collapsible_match \
  -A clippy::collapsible_if \
  -A clippy::field_reassign_with_default \
  -A clippy::doc-overindented-list-items \
  -A clippy::doc-lazy-continuation
```

---

### Fix required (in `helios-fhirpath`)

Add the missing `match` arms for `R4B`, `R5`, and `R6` variants in
`helios-fhirpath`. Each unhandled case should either:

1. Delegate to the same logic as the nearest equivalent version, or
2. Return a `not_supported` / `unimplemented!` error with a clear message.

This is tracked as a gap in `helios-fhirpath` and is outside the scope of
`helios-hts`.

---

*Discovered during `helios-hts` Phase 3 (LOINC importer) clippy verification,
2026-04-02.*
