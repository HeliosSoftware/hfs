# Components

The Helios FHIR Server is a Rust workspace made up of independent, composable crates. You can use each component as a standalone tool or combine them.

| Component | Crate | Binaries |
|-----------|-------|----------|
| [HFS Server](hfs-server.md) | `helios-hfs` | `hfs` |
| [FHIRPath](fhirpath.md) | `helios-fhirpath` | `fhirpath-cli`, `fhirpath-server` |
| [SQL-on-FHIR](sql-on-fhir.md) | `helios-sof` | `sof-cli`, `sof-server` |
| [pysof](pysof.md) | `pysof` | Python package |
| [CDS Hooks](cds-hooks.md) | `helios-cds-hooks` | Library only |

Internal crates (`helios-fhir`, `helios-fhir-gen`, `helios-fhir-macro`, `helios-fhirpath-support`, `helios-serde`, `helios-serde-support`, `helios-persistence`) are covered in the [Architecture](../architecture.md) chapter.
