# FHIR NPM package materialization

Package overlays use the same `SchemaRegistry` + `CompositeResolver` path as
core packs and tenant-uploaded StructureDefinitions (#232). This document
covers **materialization proper**: cache layout, dependency resolution, and
operator configuration.

## Cache layout

```text
{HFS_FHIR_PACKAGE_CACHE}/{package-name}/{version}/
  package.json
  StructureDefinition-….json
  …
  .sha256          # optional integrity of the source .tgz
```

Populate with `PackageCache::ensure_from_tgz` / `ensure_from_dir`, or any
external seed that expands a FHIR NPM `.tgz` (with the `package/` prefix
stripped) into that directory. **Validation never fetches from the network.**

## Configuration

| Variable | Purpose |
|----------|---------|
| `HFS_FHIR_PACKAGE_CACHE` | Cache root |
| `HFS_FHIR_PACKAGES` | Comma-separated `name@version` roots |

If `HFS_FHIR_PACKAGES` is set, boot **fails** when resolution or
materialization fails (no silent empty overlay).

## Resolver order

`CompositeResolver` (earlier wins):

1. Tenant stored-StructureDefinition overlay (optional)
2. Package layers — configured roots / dependents before transitive deps
3. Embedded core schema pack

## What is loaded

Only **StructureDefinition** resources become schemas. Abstract infrastructure
roots (`Element`, `BackboneElement`, `Resource`, `DomainResource`) are skipped.
CodeSystem / ValueSet files are discovered for operators but must be imported
via HTS, not the schema registry.

## Library API

See `helios_fhir_validator::packages`: `PackageCache`, `resolve_packages`,
`materialize_package`, `materialize_package_layers`.
