# Docker

Pre-built multi-arch Docker images (amd64/arm64) are published to GitHub Container Registry.

## FHIR Server

```bash
# Default: R4, in-memory SQLite, port 8080
docker run -p 8080:8080 ghcr.io/heliossoftware/hfs:latest

# With persistent SQLite storage
docker run -p 8080:8080 \
  -v hfs-data:/data \
  -e HFS_DATABASE_URL=/data/fhir.db \
  -e HFS_BASE_URL=http://localhost:8080 \
  ghcr.io/heliossoftware/hfs:latest

# With PostgreSQL
docker run -p 8080:8080 \
  -e HFS_STORAGE_BACKEND=postgres \
  -e HFS_DATABASE_URL="postgresql://user:pass@host:5432/fhir" \
  ghcr.io/heliossoftware/hfs:latest
```

## FHIRPath Server

```bash
docker run -p 3000:3000 ghcr.io/heliossoftware/fhirpath-server:latest
```

## SQL-on-FHIR Server

```bash
docker run -p 8080:8080 ghcr.io/heliossoftware/sof-server:latest
```

## Terminology Server

```bash
# First start imports the bundled terminology into /data/hts.db (about a minute)
docker run -p 8090:8090 -v hts-data:/data ghcr.io/heliossoftware/hts:latest
```

## Building Custom Images

A generic Dockerfile at the repo root supports all server binaries via the
`BINARY_NAME` build arg. It is runtime-only: it compiles nothing and copies an
already-built binary out of the build context. So build the binary first, stage
a context folder that holds it, and build against that folder. Building against
the repo root fails.

```bash
# 1. Build the binary, with the features your deployment needs
cargo build --release -p helios-hfs --features postgres,s3

# 2. Stage the context: the binary, plus data/ for the hfs image only
mkdir -p docker-context
cp target/release/hfs docker-context/
cp -r data docker-context/data

# 3. Build with the root Dockerfile against the staged context
docker build -f Dockerfile --build-arg BINARY_NAME=hfs -t hfs docker-context
```

The other servers use the same steps with their own binary and no `data/`, for
example `--build-arg BINARY_NAME=sof-server` or
`--build-arg BINARY_NAME=fhirpath-server --build-arg EXPOSE_PORT=3000`.

> **Details:** Base image is `debian:13.7-slim` (trixie), pinned by digest. The
> server runs as the non-root user `helios` (uid/gid 1000). Default exposed
> port is 8080. Host-binding environment variables (`HFS_SERVER_HOST`,
> `SOF_SERVER_HOST`, `FHIRPATH_SERVER_HOST`, `HTS_SERVER_HOST`) are set to
> `0.0.0.0` inside the container. `/data` is the only writable directory: keep
> databases and export output there (the image already points the HTS database
> and the bulk export and submit output there). The image has a `HEALTHCHECK`
> on `GET /health`.

Set `HFS_BASE_URL` to the address used outside the container. For a TLS proxy
that publishes HFS below `/fhir`, use
`-e HFS_BASE_URL=https://fhir.example.com/fhir`. HFS uses that configured value
for response links and does not trust forwarding headers to infer it.

## Environment Variables

All [environment variables](../configuration/environment-variables.md) work identically inside Docker — pass them with `-e KEY=VALUE`.
