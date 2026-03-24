# Code Generation

The `helios-fhir` crate contains auto-generated Rust types for all FHIR resources and data types across R4, R4B, R5, and R6. These types are produced by `helios-fhir-gen` from official HL7 StructureDefinition JSON schemas.

You only need to run the generator if you are **updating the FHIR spec version** or **changing how models are generated**. Normal development does not require it.

---

## When and Why to Regenerate Models

Regenerate the FHIR models when:

- A new FHIR specification release is available (e.g., a new R6 ballot or R5 patch)
- You change code generation logic in `helios-fhir-gen`
- The R6 schema files have been updated on the HL7 build server

Do **not** regenerate just to apply a patch or change application logic. The generated files are checked into the repository.

---

## Running helios-fhir-gen

Regeneration is a three-step process:

### Step 1 — Build the generator

Use `R4,R4B,R5,R6` feature flags explicitly. Do **not** use `--all-features` here (it enables `skip-r6-download`, which would prevent the R6 spec from being fetched).

```bash
cargo build -p helios-fhir-gen --features R4,R4B,R5,R6
```

> **Build time:** This can take 5–10 minutes because `helios-fhir` itself is a large generated crate.

### Step 2 — Run the generator

```bash
./target/debug/helios-fhir-gen --all
```

This reads HL7 StructureDefinition JSON files for each version and writes:
- `crates/fhir/src/r4.rs`
- `crates/fhir/src/r4b.rs`
- `crates/fhir/src/r5.rs`
- `crates/fhir/src/r6.rs`

> **R6 auto-download:** When `R6` is enabled, the generator automatically downloads the R6 StructureDefinition files from `https://build.fhir.org/` at build time. Internet access is required.

### Step 3 — Format the generated files

```bash
cargo fmt --all
```

Generated files are not pre-formatted. Always run `cargo fmt` after generation before committing.

---

## Formatting Generated Code

After running the generator, always format:

```bash
cargo fmt --all
```

Then verify the build still compiles and tests pass:

```bash
cargo build --features R4,R4B,R5,R6
cargo test --features R4,R4B,R5,R6
```

---

## How the Generator Works

1. **Reads StructureDefinition JSON** for each requested FHIR version from the `data/` directory (R4, R4B, R5) or downloaded from `build.fhir.org` (R6)
2. **Parses the schema** to extract resource types, data types, primitive types, and choice element patterns
3. **Generates Rust structs** with appropriate field types, option wrapping, and serde attributes
4. **Writes output files** to `crates/fhir/src/` — one file per FHIR version
5. **Feature flags** in `crates/fhir/Cargo.toml` control which generated files are compiled

### What is generated vs. hand-coded

| Source | Content |
|--------|---------|
| Generated (95%) | All resource structs, data type structs, primitive type wrappers, choice type enums |
| Hand-coded (5%) | `Element<T>`, `DecimalElement`, `PreciseDecimal`, `FhirVersion` enum, `Resource` collection enum |

### The `skip-r6-download` feature

When you run `cargo clippy --all-features`, the `skip-r6-download` feature is activated automatically. This prevents `helios-fhir-gen` from attempting to download R6 specs during the clippy run, making CI linting safe without network access or a long build.
