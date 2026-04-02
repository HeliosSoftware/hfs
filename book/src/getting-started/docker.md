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

## Building Custom Images

A generic Dockerfile at the repo root supports all server binaries via the `BINARY_NAME` build arg:

```bash
# Build the HFS server image
docker build --build-arg BINARY_NAME=hfs -t hfs .

# Build the SOF server image
docker build --build-arg BINARY_NAME=sof-server -t sof-server .

# Build the FHIRPath server image
docker build --build-arg BINARY_NAME=fhirpath-server -t fhirpath-server .
```

The Dockerfile expects the binary and `data/` files to be pre-staged in the build context. In CI, the binary is built separately and copied in before the Docker build step.

> **Details:** Base image is `debian:bookworm-slim`. The server runs as non-root user `hfs`. Default exposed port is 8080. Host-binding environment variables (`HFS_SERVER_HOST`, `SOF_SERVER_HOST`, `FHIRPATH_SERVER_HOST`) are automatically set to `0.0.0.0` inside the container.

## Environment Variables

All [environment variables](../configuration/environment-variables.md) work identically inside Docker — pass them with `-e KEY=VALUE`.
