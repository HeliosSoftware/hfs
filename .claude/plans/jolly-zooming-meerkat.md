# Plan: Add HTS to CI Docker builds and deploy

## Context
The `feat/hts-terminology-service` branch added the `helios-hts` crate with an `hts` binary. The CI workflow already builds the binary (via `cargo build --workspace`), but the Docker image, manifest, and deploy steps don't include it yet.

## Changes

### 1. `Dockerfile` — add `HTS_SERVER_HOST` env var
**File:** `Dockerfile` (line ~35)

Add `ENV HTS_SERVER_HOST=0.0.0.0` alongside the existing server host vars so the HTS container binds to all interfaces.

### 2. `.github/workflows/ci.yml` — Docker build matrix (line ~888)
Add `hts` to the `image` list and its `include` entry:

```yaml
image:
  - hfs
  - fhirpath-server
  - sof-server
  - hts          # <-- add
```

```yaml
- image: hts
  binary: hts
  port: 8090
  include_data: false
```

### 3. `.github/workflows/ci.yml` — Docker manifests matrix (line ~974)
Add `hts` to the manifest image list:

```yaml
matrix:
  image:
    - hfs
    - fhirpath-server
    - sof-server
    - hts          # <-- add
```

### 4. `.github/workflows/ci.yml` — Deploy matrix (line ~1031)
Add `hts` to the deploy server list and update the deploy step's env block with HTS-specific secrets:

```yaml
matrix:
  server:
    - fhirpath-server
    - sof-server
    - hfs
    - hts          # <-- add
```

Update the `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_PORT` ternary chains (line ~1087) to include `hts` mappings using `secrets.HTS_DEPLOY_HOST`, `secrets.HTS_DEPLOY_USER`, `secrets.HTS_DEPLOY_PORT`.

**Note:** The deploy secrets (`HTS_DEPLOY_HOST`, etc.) must be configured in GitHub repo settings separately — this plan only adds the CI references.

## Files to modify
- `Dockerfile`
- `.github/workflows/ci.yml`

## Verification
- Review the diff to confirm matrix entries are correct
- The `hts` binary is already built by `cargo build --workspace` in the build job, so no build step changes needed
- Port 8090 matches `HTS_SERVER_PORT` default from CLAUDE.md
