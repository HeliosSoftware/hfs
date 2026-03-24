# Contributing Guidelines

---

## Code Style and Conventions

- **Rust edition:** 2024
- **Minimum supported Rust version (MSRV):** 1.90
- **Version:** All crates share the same version number, bumped together at release time
- **Formatting:** `cargo fmt --all` before every commit
- **Linting:** Zero clippy warnings with the CI flags (see [Development Setup](ch13-dev-setup.md#linting-and-formatting))
- **Doc-only commits:** Include `[skip ci]` in the commit message when only documentation files (README, CLAUDE.md, `.md` files) have changed, to avoid unnecessary CI runs

---

## Opening Issues and Pull Requests

- **Issues and bug reports:** [github.com/HeliosSoftware/hfs/issues](https://github.com/HeliosSoftware/hfs/issues)
- **Discussions (design, questions, roadmap):** [github.com/HeliosSoftware/hfs/discussions](https://github.com/HeliosSoftware/hfs/discussions)
- **Weekly developer meeting:** Open to all. Details in [this GitHub Discussion](https://github.com/HeliosSoftware/hfs/discussions/40)

### How to get involved

1. **Comment on a discussion document** — help shape the design of upcoming features
2. **Claim a roadmap item** — open an issue or comment on an existing one to signal interest
3. **Join the weekly call** — introduce yourself and find out where help is needed
4. **Contribute code** — fork, branch, implement, open a PR

---

## Common Extension Points

### Adding a new FHIRPath function

1. Add the function implementation in the appropriate module under `crates/fhirpath/src/`
2. Update the parser in `parser.rs` if the function requires new syntax
3. Add test cases covering the function (both unit and against the official FHIR test suite if applicable)
4. Update the feature matrix in the crate README

### Adding a new REST endpoint

1. Add the handler in `crates/rest/src/handlers/`
2. Register the route in `crates/rest/src/routes.rs`
3. Add integration tests covering the endpoint

### Implementing a new storage backend

1. Implement `ResourceStorage` as the baseline (CRUD operations, TenantContext required for every call)
2. Optionally implement `VersionedStorage`, `SearchProvider`, `TransactionProvider` for additional capabilities
3. Register the backend in `CompositeStorage` if it can be combined with other backends
4. Implement `CapabilityProvider` to advertise the supported FHIR interactions in the CapabilityStatement

---

## Release Process

HFS uses [`cargo-release`](https://github.com/crate-ci/cargo-release) for workspace-wide version bumps. All crates are bumped together.

### Prerequisites

```bash
cargo install cargo-release
cargo login   # set up your crates.io token
```

### Pre-release checklist

Before cutting a release:

```bash
# 1. Optionally regenerate FHIR models from the latest spec
cargo build -p helios-fhir-gen --features R4,R4B,R5,R6
./target/debug/helios-fhir-gen --all
cargo fmt --all

# 2. Run the full test suite
cargo test --workspace --all-features

# 3. Ensure CI is green on main
```

### Creating a release

```bash
# Dry run first — review the output carefully
cargo release patch --dry-run   # 0.1.47 → 0.1.48
cargo release minor --dry-run   # 0.1.47 → 0.2.0
cargo release major --dry-run   # 0.1.47 → 1.0.0

# Execute when satisfied
cargo release patch --execute
```

`cargo release` will:
1. Update all `Cargo.toml` version fields
2. Update internal dependency version references
3. Create a git commit with the version bump
4. Create a git tag
5. Publish all crates to crates.io
6. Push the commit and tag to GitHub

### Automatic post-release actions (GitHub Actions)

After the tag is pushed, CI automatically:
- Builds release binary artifacts for Linux, Windows, and macOS
- Creates a [GitHub Release](https://github.com/HeliosSoftware/hfs/releases) with the artifacts attached
- Builds `pysof` wheels for all platforms
- Publishes `pysof` to PyPI (if `PYPI_API_TOKEN` is configured)
