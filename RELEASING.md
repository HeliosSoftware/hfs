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

- Consider downloading the latest FHIR Specification (R6) and test files, [generating](README.md#code-generation), then running a [comprehensive test](README.md#running-tests), and checking in the latest into GitHub.

  Three build scripts download from `build.fhir.org`, and each one skips the download when its marker file is less than 24 hours old. A fresh `git clone`, `git worktree add` or checkout sets every file's mtime to "now", so without the steps below nothing is downloaded and the generator reruns on the old specs. Refresh all three together: `build.fhir.org` is a rolling build, and refreshing only some of them mixes builds (models generated from one build, test examples from another), which shows up as `test_r6_json_examples` failures.
```bash
rm -f crates/fhir-gen/resources/R6/download_metadata.json   # spec definitions (helios-fhir-gen)
rm -f crates/fhir/tests/data/json/R6/.download_marker      # R6 JSON/XML test examples (helios-fhir)
cargo build -p helios-fhir-gen --features R4,R4B,R5,R6     # downloads both of the above
./target/debug/helios-fhir-gen --all
# data/search-parameters-r6.json is downloaded by helios-hfs's build script, which
# `--all-features` turns off (skip-r6-download). It is the same file as the
# spec's search-parameters.json, so copy it over:
cp crates/fhir-gen/resources/R6/search-parameters.json data/search-parameters-r6.json
cargo fmt --all
cat crates/fhir-gen/resources/R6/version.info               # confirm the new buildId
cargo test --workspace --all-features
```
  After `cargo fmt`, the R4, R4B and R5 generated files should show no diff; only R6 files should change.
- Regenerate the validator's embedded R6 schema and terminology packs from the same spec files. These packs ([`crates/fhir-validator/packs/`](crates/fhir-validator/packs/)) drive resource validation and the UI's resource editor and guided form. They are committed and embedded at compile time, so they do not pick up spec updates on their own. Run the schema pack first, because `generate-terminology` reads it:
```bash
cargo run -p helios-fhir-validator --features gen-packs --bin generate-schema-packs -- R6
cargo run -p helios-fhir-validator --features gen-packs --bin generate-terminology -- R6
cargo test -p helios-fhir-validator --features R4,R4B,R5,R6
```
  Expect a large R6 pack diff whenever the R6 spec version moves (for example, ballot to snapshot).
- Check in these changes.
- Refresh the bundled HTS terminology data. These files live in [`crates/hts/terminology-data/`](crates/hts/terminology-data/) and ship inside every HTS release archive and Docker image, so customers get a working server with no extra downloads. The bundled terminologies have their own release cadences (ICD-10-CM each June for the fiscal year starting October 1, NCIt monthly, NUCC twice a year, MeSH each winter for the following year, THO and the FHIR IG packages irregularly, etc.), so check each upstream source before tagging:
  1. Review the upstream landing pages referenced alongside each block in [`crates/hts/scripts/download-bundled-terminologies.sh`](crates/hts/scripts/download-bundled-terminologies.sh). If newer versions are available, bump the `*_VERSION` / `*_YEAR` variables at the top of the script (and the matching `$Xxx` variables in the `.ps1` companion). For the FHIR packages, `curl -s https://packages.fhir.org/<name>` returns the latest version under `dist-tags`. Upstream file names can change between releases (ICD-10-CM's FY2027 zip uses hyphens where FY2026's used spaces), so if a download fails, check the upstream directory listing and update the URL in both scripts.
  2. Refresh the checked-in data by re-running the script against the repo's data directory:
     ```bash
     ./crates/hts/scripts/download-bundled-terminologies.sh ./crates/hts/terminology-data
     ```
     The script skips files that already exist, so `git rm` the files for any version you bumped first (or pass a clean output dir) to force a re-download. NDC has no version in its file name (`ndctext-current.zip`), so delete it on every refresh to pick up the current FDA file. The script fails a download when the server returns an HTML page instead of the file, which is how some hosts answer for a file that has moved or isn't published yet (for example, the next year's MeSH before its release).
  3. `git add crates/hts/terminology-data/ && git commit` the refreshed files. Plain git works (no LFS) as long as every file stays under GitHub's 100 MB per-file limit — MeSH in particular must remain in its `.zip` form. If a new terminology exceeds the limit, either add a compressed variant the importer can read or introduce Git LFS.
  4. Push and let CI verify the archive + Docker builds pick up the new files before cutting the release.
- Ensure that your build in GitHub Actions has succeeded fully.  These are found in [ci.yml](.github/workflows/ci.yml).
- Check for an open [`ci-toolchain` issue](https://github.com/HeliosSoftware/hfs/issues?q=is%3Aissue+is%3Aopen+label%3Aci-toolchain). [Extended CI](.github/workflows/ci-extended.yml) runs the beta and nightly Rust toolchains and an MSRV check daily, and keeps that issue open while it is red. It does not gate the release, because releases are built with stable. So an open issue does not block tagging, but read it first:
  - A **beta** failure becomes a stable failure within six weeks. Know whether it is our code or a rustc regression, and that it is being fixed.
  - A **nightly** failure is usually a rustc bug (v0.2.7 hit one). Make sure it is reported upstream, then tag.
  - Fix an **MSRV** failure before tagging. `rust-version` in the root `Cargo.toml` is published to crates.io with every crate, and the failure means it is wrong. Usually that's a dependency update that now needs a newer rustc. Raise `rust-version`, plus the docs that quote it (`CONTRIBUTING.md`, `book/`, `CLAUDE.md`/`AGENTS.md`), or pin the dependency back.

### 2. Create a Release

To create a new release, use cargo-release with the appropriate version bump:

```bash
# Without --execute, cargo-release only does a dry run.
# For a patch release (0.1.3 -> 0.1.4)
cargo release patch

# For a minor release (0.1.3 -> 0.2.0)
cargo release minor

# For a major release (0.1.3 -> 1.0.0)
cargo release major
```

Review the dry-run output, then run it again with `--execute`:

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


