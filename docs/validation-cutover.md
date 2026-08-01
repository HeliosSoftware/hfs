# Single-engine validation cutover

HFS now uses **only** `helios-fhir-validator` for `$validate` and write-path
enforcement. The Atrius `fhir-validation` crate and `HFS_PROFILE_MANIFEST` /
`HFS_PROFILE_VALIDATION_MODE` path have been removed.

## Operator config

| Variable | Role |
|----------|------|
| `HFS_FHIR_PACKAGE_CACHE` | Cache root for expanded packages |
| `HFS_FHIR_PACKAGE_SOURCES` | Local `.tgz` / dir / IG `output/`, and/or `https://…/*.tgz` — seeded at boot |
| `HFS_FHIR_PACKAGES` | Optional `name@version` roots (defaults to packages from sources) |
| `HFS_VALIDATION_MODE` | `off` / `log` / `enforce` on create/update/patch/batch/**transaction** |

See [crates/fhir-validator/docs/packages.md](../crates/fhir-validator/docs/packages.md).

### Atrius IG on disk (publisher)

```bash
export HFS_FHIR_PACKAGE_CACHE=$PWD/fhir-package-cache
export HFS_FHIR_PACKAGE_SOURCES=/Users/sandhu/AtriusIGDraft/output/atrius.fhir.r4.india.en.tgz
# or: .../output/package.tgz   or   .../output  (uses package.tgz when unique)
export HFS_VALIDATION_MODE=enforce
```

Do **not** expect the whole HTML `output/` tree to be scanned as a package unless
it contains `package.tgz` (or a single `*.tgz`). Prefer the `.tgz` path.

## Staging checklist

1. Set cache + sources (and optional package roots) as above.
2. Smoke `$validate` and HIS write paths (including transaction Bundles).
3. Compare issues against prior dual-engine baselines for Patient / Encounter / Appointment.
4. Seed dependency packages into the same cache if `package.json` deps must resolve.
