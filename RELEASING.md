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
cargo test --all-features
```

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

