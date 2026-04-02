# Code Generation

The `helios-fhir` crate contains auto-generated Rust types for FHIR resources. These types are produced by `helios-fhir-gen` from official HL7 StructureDefinition JSON schemas.

You only need to run this if you are updating the FHIR spec version or changing how models are generated.

## Regenerating FHIR Models

```bash
# 1. Build the generator
#    (Note: use --features R4,R4B,R5,R6 here — not --all-features)
cargo build -p helios-fhir-gen --features R4,R4B,R5,R6

# 2. Run the generator
./target/debug/helios-fhir-gen --all

# 3. Format the generated files
cargo fmt --all
```

> **R6 specs** are automatically downloaded from the HL7 build server (`https://build.fhir.org/`) during the build. This requires internet access.

> **Build time:** Building `helios-fhir-gen` can take 5–10 minutes due to the large generated files.

## How It Works

1. The generator reads FHIR StructureDefinition JSON files for each version
2. It transforms them into Rust struct and enum definitions
3. The output files (`r4.rs`, `r4b.rs`, `r5.rs`, `r6.rs`) are written to `crates/fhir/src/`
4. Feature flags (`R4`, `R4B`, `R5`, `R6`) gate which versions are compiled

## The `skip-r6-download` Feature

When running `cargo clippy --all-features`, the `skip-r6-download` feature is activated, which prevents `helios-fhir-gen` from attempting to download R6 specs during the build. This makes CI linting safe to run without network access or a long build.
