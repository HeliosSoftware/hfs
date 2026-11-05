---
name: docker-and-release
description: Work on Docker images or release workflows for this project. Use for the generic Dockerfile, BINARY_NAME builds, container runtime assumptions, cargo-release, workspace version bumps, GitHub Releases, release artifacts, and pysof wheel/PyPI publishing.
---

# Docker and Release

Use this for Docker packaging and release process work.

## Docker

The generic Dockerfile supports all server binaries through the `BINARY_NAME` build
argument. It is **runtime-only** — it compiles nothing, and copies an
already-built binary out of the build context. So the context must be staged
first; building against the repo root fails with
`chmod: cannot access '/app/<binary>': No such file or directory` (and transfers
the whole workspace, `target/` included, on the way to that error).

```bash
# 1. Build the binary. Add whatever features the image needs — the helios-hfs
#    default set is R4,sqlite,ui, so e.g. a postgres deployment needs more:
cargo build --release -p helios-hfs --features postgres,s3

# 2. Stage a context holding the binary (plus data/ for the hfs image only).
mkdir -p docker-context
cp target/release/hfs docker-context/
cp -r data docker-context/data

# 3. Build against the staged context, keeping the root Dockerfile.
docker build -f Dockerfile --build-arg BINARY_NAME=hfs -t hfs docker-context
```

Other binaries follow the same three steps with a different `BINARY_NAME`,
`EXPOSE_PORT`, and no `data/`:

```bash
docker build -f Dockerfile --build-arg BINARY_NAME=sof-server \
  --build-arg EXPOSE_PORT=8080 -t sof-server docker-context
docker build -f Dockerfile --build-arg BINARY_NAME=fhirpath-server \
  --build-arg EXPOSE_PORT=3000 -t fhirpath-server docker-context
```

`docker/bulk-submit/stage-context.sh` is a worked example of steps 1–2.

Build arguments:

| Arg | Default | Purpose |
|---|---|---|
| `BINARY_NAME` | none (required) | Binary to copy in and exec; build fails if unset |
| `EXPOSE_PORT` | `8080` | `EXPOSE`d port — CI uses `3000` for fhirpath-server, `8090` for hts |
| `BOOTSTRAP_DIR` | `""` | Sets `HTS_BOOTSTRAP_DIR`; the hts image bakes in `crates/hts/terminology-data` and points here for first-boot auto-import |

Container assumptions:

- Base image is `debian:trixie-slim`.
- Runtime user is non-root user `helios` (uid/gid 1000) — not `hfs`.
- Default exposed port is `8080` (overridable via `EXPOSE_PORT`).
- Server host variables are set to `0.0.0.0` inside the container: `HFS_SERVER_HOST`, `SOF_SERVER_HOST`, `FHIRPATH_SERVER_HOST`, and `HTS_SERVER_HOST`.
- `HFS_DATABASE_URL` defaults to `:memory:`, and `/data` is a `VOLUME` owned by `helios` for SQLite and other persistent state.
- Only the `hfs` image ships `data/` (the search-parameter definitions); CI gates this behind `include_data`. Without it the server falls back to a handful of embedded search parameters.

CI builds these images in the `docker-build` job of `ci.yml`, on tags only. It
downloads the binary as an artifact, stages `docker-context/` exactly as above,
and builds with `context: docker-context`. Note that a `.dockerignore` at the
repo root does **not** apply to those builds — Docker reads `.dockerignore` from
the build-context root, so the repo-root file only guards accidental
root-context builds.

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
