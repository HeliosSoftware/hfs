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

### Sample package (tests / smoke)

```bash
export HFS_FHIR_PACKAGE_CACHE=$PWD/fhir-package-cache
export HFS_FHIR_PACKAGE_SOURCES=crates/fhir-validator/tests/fixtures/packages/sample.tgz
export HFS_VALIDATION_MODE=enforce
```

For a full IG publisher tree, prefer `output/package.tgz` (or a single `*.tgz`).
Do **not** treat the whole HTML `output/` tree as a package unless it contains
that tarball.

## Staging checklist

1. Set cache + sources (and optional package roots) as above.
2. Smoke `$validate` and HIS write paths (including transaction Bundles).
3. Compare issues against prior dual-engine baselines for Patient / Encounter / Appointment.
4. Seed dependency packages into the same cache if `package.json` deps must resolve.
