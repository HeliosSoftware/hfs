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

The root Dockerfile serves every server binary via the `BINARY_NAME` build
arg. It compiles nothing: build the binary, stage it in a context folder, and
build against that folder (not the repo root).

```bash
# 1. Build the binary
cargo build --release -p helios-hfs --features postgres,s3

# 2. Stage the binary, plus data/ for hfs only
mkdir -p docker-context
cp target/release/hfs docker-context/
cp -r data docker-context/data

# 3. Build against the staged context
docker build -f Dockerfile --build-arg BINARY_NAME=hfs -t hfs docker-context
```

Other servers: same steps, no `data/`, e.g. `--build-arg BINARY_NAME=sof-server`.

> **Details:** Runtime base is `gcr.io/distroless/cc-debian13`, pinned by
> digest: no shell, no package manager, no `HEALTHCHECK`. The binary is
> `/app/server`, run as uid/gid 1000. Default exposed port is 8080.
> Host-binding variables (`HFS_SERVER_HOST`, `SOF_SERVER_HOST`,
> `FHIRPATH_SERVER_HOST`, `HTS_SERVER_HOST`) are set to `0.0.0.0`. `/data` is
> the only writable directory; the image already puts the HTS database and
> export output there. To get a shell, build with the
> `gcr.io/distroless/cc-debian13:debug` base and run `--entrypoint sh`.

Set `HFS_BASE_URL` to the address used outside the container. For a TLS proxy
that publishes HFS below `/fhir`, use
`-e HFS_BASE_URL=https://fhir.example.com/fhir`. HFS uses that configured value
for response links and does not trust forwarding headers to infer it.

## Environment Variables

All [environment variables](../configuration/environment-variables.md) work identically inside Docker — pass them with `-e KEY=VALUE`.
