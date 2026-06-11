---
name: docker-and-release
description: Work on Docker images or release workflows for this project. Use for the generic Dockerfile, BINARY_NAME builds, container runtime assumptions, cargo-release, workspace version bumps, GitHub Releases, release artifacts, and pysof wheel/PyPI publishing.
---

# Docker and Release

Use this for Docker packaging and release process work.

## Docker

The generic Dockerfile supports all server binaries through the `BINARY_NAME` build argument:

```bash
# Build HFS server image
docker build --build-arg BINARY_NAME=hfs -t hfs .

# Build SOF server image
docker build --build-arg BINARY_NAME=sof-server -t sof-server .

# Build FHIRPath server image
docker build --build-arg BINARY_NAME=fhirpath-server -t fhirpath-server .
```

Container assumptions:

- Base image is `debian:bookworm-slim`.
- Runtime user is non-root user `hfs`.
- Default exposed port is `8080`.
- Server host variables are set to `0.0.0.0` inside the container: `HFS_SERVER_HOST`, `SOF_SERVER_HOST`, and `FHIRPATH_SERVER_HOST`.
- The Dockerfile expects binaries and data files to be pre-staged in the build context. CI builds the binary separately and copies it in.

## Release Process

The workspace uses `cargo-release` for workspace-wide version bumps. All crates share the same version.

```bash
# Dry run
cargo release patch --dry-run

# Execute: bumps versions, commits, tags, publishes to crates.io, and pushes
cargo release patch --execute
```

After the tag is pushed, GitHub Actions:

- Builds release artifacts.
- Creates a GitHub Release.
- Builds pysof wheels for Linux, Windows, and macOS.
- Publishes pysof to PyPI.

See `RELEASING.md` for complete release details.
