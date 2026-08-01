# FHIR NPM package materialization

Package overlays use the same `SchemaRegistry` + `CompositeResolver` path as
core packs and tenant-uploaded StructureDefinitions (#232). This document
covers **materialization proper**: cache layout, sources, dependency
resolution, and operator configuration.

## Cache vs sources

| Concept | Role |
|---------|------|
| **Cache** (`HFS_FHIR_PACKAGE_CACHE`) | Durable expanded packages: `{cache}/{name}/{version}/` |
| **Sources** (`HFS_FHIR_PACKAGE_SOURCES`) | Where packages are **installed from** at boot (local or URL) |
| **Roots** (`HFS_FHIR_PACKAGES`) | Which `name@version` layers to load; defaults to packages installed from sources |

`.staging/` and `.downloads/` under the cache root are **internal** (temp unpack /
HTTP fetch). They are not package sources.

## Accepted local sources (`PackageCache::ensure_from_path`)

- FHIR NPM `.tgz` / `.tar.gz` file (any path, e.g. IG publisher
  `output/atrius.fhir.r4.india.en.tgz` or `output/package.tgz`)
- Expanded package directory with `package.json` or `package/package.json`
- IG publisher **`output/`** directory: prefers `package.tgz`; if several
  `*.tgz` exist, pass one file explicitly (do **not** treat the whole HTML
  tree as a package)

## Configuration

| Variable | Purpose |
|----------|---------|
| `HFS_FHIR_PACKAGE_CACHE` | Cache root (required when sources/packages are set) |
| `HFS_FHIR_PACKAGE_SOURCES` | Comma-separated local paths and/or `http(s)://…/*.tgz` URLs |
| `HFS_FHIR_PACKAGES` | Optional `name@version` roots; if omitted, uses ids from sources |

### Examples

Bundled test fixture (check out of tree):

```bash
export HFS_FHIR_PACKAGE_CACHE=$PWD/fhir-package-cache
export HFS_FHIR_PACKAGE_SOURCES=crates/fhir-validator/tests/fixtures/packages/sample.tgz
export HFS_VALIDATION_MODE=enforce
# defaults to example.fhir.r4.sample@0.1.0 from the tarball
```

Publisher tarball or `output/` on disk:

```bash
export HFS_FHIR_PACKAGE_SOURCES=/path/to/ig/output/package.tgz
# or: /path/to/ig/output   (picks package.tgz when unique)
```

Published URL:

```bash
export HFS_FHIR_PACKAGE_SOURCES=https://example.org/fhir/r4/sample/package.tgz
```

See `tests/fixtures/packages/README.md` for the sample IG contents and rebuild script.

## Resolver order

`CompositeResolver` (earlier wins):

1. Tenant stored-StructureDefinition overlay (optional)
2. Package layers — dependents before transitive deps
3. Embedded core schema pack

## What is loaded

Only **StructureDefinition** resources become schemas. Abstract infrastructure
roots (`Element`, `BackboneElement`, `Resource`, `DomainResource`) are skipped.
CodeSystem / ValueSet files are discovered for operators but must be imported
via HTS, not the schema registry.

## Library API

See `helios_fhir_validator::packages`: `PackageCache`, `ensure_from_path`,
`resolve_packages`, `materialize_package`, `materialize_package_layers`.
