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
| `VERSION`, `REVISION`, `CREATED` | `dev`, `unknown`, `""` | OCI labels `org.opencontainers.image.version/revision/created`; the tag jobs in `ci.yml` do not pass them yet |

Container assumptions:

- Base image is `debian:13.7-slim` (trixie), pinned by tag **and** by the digest of
  its multi-platform index. Dependabot (`.github/dependabot.yml`, ecosystem
  `docker`) proposes the next tag + digest weekly; do not hand-edit to a floating
  tag. The build also runs `apt-get upgrade`, so trixie-security fixes newer than
  the pinned base still land.
- Runtime user is `helios`, uid/gid 1000, set as numeric `USER 1000:1000` — not `hfs`.
- Default exposed port is `8080` (overridable via `EXPOSE_PORT`).
- Server host variables are set to `0.0.0.0` inside the container: `HFS_SERVER_HOST`, `SOF_SERVER_HOST`, `FHIRPATH_SERVER_HOST`, and `HTS_SERVER_HOST`.
- `HFS_DATABASE_URL` defaults to `:memory:`, and `/data` is a `VOLUME` owned by
  uid 1000 — the only writable directory (`/app` is root-owned). So the image sets
  `HTS_DATABASE_URL=/data/hts.db`, `HFS_BULK_EXPORT_OUTPUT_DIR=/data/bulk-export`,
  `HFS_BULK_SUBMIT_OUTPUT_DIR=/data/bulk-submit` and `HFS_EXPORT_DIR=/data/exports`;
  the binaries' own defaults are relative to `/app` and fail with "unable to open
  database file" (hts) or "Permission denied" (hfs `$export`). `HFS_DATA_DIR` is
  left unset on purpose: hfs reads the shipped search parameters from `/app/data`.
- `HEALTHCHECK` probes `GET /health` with bash's `/dev/tcp` (the image has no
  curl or wget) on the port from the binary's own `*_SERVER_PORT` variable;
  start period 120 s covers the hts first-boot import (~55 s).
- Only the `hfs` image ships `data/` (the search-parameter definitions); CI gates this behind `include_data`. Without it the server falls back to a handful of embedded search parameters.

Linting: `.github/workflows/docker-lint.yml` runs on PRs touching `Dockerfile`,
`.dockerignore` or `.hadolint.yaml`. It fails on hadolint warnings
(`.hadolint.yaml`; every ignore needs a reason comment), `docker build --check`,
and `trivy config` HIGH/CRITICAL; `trivy image` is report-only. Same checks
locally:

```bash
docker run --rm -i -v "$PWD/.hadolint.yaml:/.hadolint.yaml:ro" \
  hadolint/hadolint:v2.15.1 hadolint --config /.hadolint.yaml - < Dockerfile
docker build --check -f Dockerfile --build-arg BINARY_NAME=hfs docker-context
docker run --rm -v "$PWD:/repo:ro" aquasec/trivy:0.76.2 \
  config --severity HIGH,CRITICAL --exit-code 1 /repo/Dockerfile
docker save hfs -o hfs.tar && docker run --rm -v "$PWD:/w:ro" aquasec/trivy:0.76.2 \
  image --input /w/hfs.tar --severity HIGH,CRITICAL
```

Trivy sees only the OS packages: the release binaries are not built with
`cargo-auditable`, so Rust dependency CVEs are not in its report.

The compose files under `docker/` (bulk-export, bulk-submit, keycloak) are
local developer and test stacks, not shipped; they are not linted or covered by
Dependabot.

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
