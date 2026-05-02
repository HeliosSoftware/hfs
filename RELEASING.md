# Release Process for HFS

This document describes the release process for the Helios FHIR Server (HFS) project.

## Overview

HFS is a multi-crate Rust workspace where all crates share the same version number. This ensures compatibility and simplifies dependency management.

## Prerequisites

1. Install `cargo-release`:
   ```bash
   cargo install cargo-release
   ```

2. Ensure you have publishing rights on crates.io for all HFS crates

3. Set up your crates.io token:
   ```bash
   cargo login
   ```

## Release Steps

### 1. Prepare for Release

- Consider downloading the latest FHIR Specification (R6) and test files, [generating](readme.md#code-generation), then running a [comprehensive test](readme.md#running-tests), and checking in the latest into GitHub. 
```bash
cargo build -p helios-fhir-gen --features R4,R4B,R5,R6
./target/debug/helios-fhir-gen --all
cargo fmt --all 
cargo test --workspace --all-features
```
- Check in these changes.
- Refresh the bundled HTS terminology data. These files live in [`crates/hts/terminology-data/`](crates/hts/terminology-data/) and ship inside every HTS release archive and Docker image, so customers get a working server with no extra downloads. The bundled terminologies have their own release cadences (ICD-10-CM annually in October, THO periodically, MeSH annually, etc.), so check each upstream source before tagging:
  1. Review the upstream landing pages referenced alongside each block in [`crates/hts/scripts/download-bundled-terminologies.sh`](crates/hts/scripts/download-bundled-terminologies.sh). If newer versions are available, bump the `*_VERSION` / `*_YEAR` variables at the top of the script (and the matching `$Xxx` variables in the `.ps1` companion).
  2. Refresh the checked-in data by re-running the script against the repo's data directory:
     ```bash
     ./crates/hts/scripts/download-bundled-terminologies.sh ./crates/hts/terminology-data
     ```
     The script skips files that already exist, so delete any stale pins first (or pass a clean output dir) to force a re-download.
  3. `git add crates/hts/terminology-data/ && git commit` the refreshed files. Plain git works (no LFS) as long as every file stays under GitHub's 100 MB per-file limit — MeSH in particular must remain in its `.zip` form. If a new terminology exceeds the limit, either add a compressed variant the importer can read or introduce Git LFS.
  4. Push and let CI verify the archive + Docker builds pick up the new files before cutting the release.
- Ensure that your build in GitHub Actions has succeeded fully.  These are found in [ci.yml](.github/workflows/ci.yml).

### 2. Create a Release

To create a new release, use cargo-release with the appropriate version bump:

```bash
# For a patch release (0.1.3 -> 0.1.4)
cargo release patch --dry-run

# For a minor release (0.1.3 -> 0.2.0)
cargo release minor --dry-run

# For a major release (0.1.3 -> 1.0.0)
cargo release major --dry-run
```

Review the dry-run output, then execute without `--dry-run`:

```bash
cargo release patch --execute
```

This will:
- Update version numbers in all Cargo.toml files
- Update internal dependency versions
- Create a git commit with the version bump
- Create a git tag
- Publish to crates.io
- Push these changes to GitHub

After the tag is pushed, GitHub Actions will automatically:
- Build release artifacts
- Create a [GitHub Release](https://github.com/HeliosSoftware/hfs/releases) with the artifacts
- Build pysof wheels for multiple platforms (Linux, Windows, macOS)
- Publish pysof to PyPI (if `PYPI_API_TOKEN` secret is configured)


